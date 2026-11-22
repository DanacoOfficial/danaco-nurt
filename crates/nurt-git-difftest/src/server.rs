// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test server of the smart HTTP transport.
//!
//! The server listens on the loopback interface and answers every request
//! with the `git http-backend` program run as a CGI script, with the
//! repositories below one directory. Each connection carries one request and
//! is closed after the response; request bodies may be chunked or compressed
//! (`Content-Encoding: gzip` is passed to the backend). Pushes are accepted,
//! because the backend sees an authenticated user.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::{Result, io_error};
use crate::git::{GitEnv, GitProgram};

/// Address the server listens on; the port is chosen by the system.
const LOOPBACK: &str = "127.0.0.1:0";

/// Longest accepted request line or header line.
const MAX_LINE_BYTES: usize = 16 * 1024;

/// Largest number of header lines accepted in one request.
const MAX_HEADERS: usize = 100;

/// Time after which a silent client is disconnected.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// User name seen by the backend; with it the backend accepts pushes.
const REMOTE_USER: &str = "nurt";

/// Smart HTTP server for the repositories below a directory.
///
/// The server stops when the value is dropped.
#[derive(Debug)]
pub struct HttpServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Starts a server for the repositories below `root`.
    ///
    /// The repository `<root>/<path>` has the address
    /// `<base_url>/<path>`; `env` is the environment of the backend, so its
    /// identity and dates apply to reflog entries written by pushes.
    ///
    /// # Errors
    ///
    /// Fails with [`crate::error::DifftestError::Io`] when the listening socket cannot be
    /// opened.
    pub fn start(git: &GitProgram, env: &GitEnv, root: &Path) -> Result<Self> {
        let listen = Path::new(LOOPBACK);
        let listener = TcpListener::bind(LOOPBACK).map_err(io_error("listen on", listen))?;
        let address = listener
            .local_addr()
            .map_err(io_error("listen on", listen))?;
        let backend = Backend {
            git: git.clone(),
            env: env
                .with_var("GIT_PROJECT_ROOT", root)
                .with_var("GIT_HTTP_EXPORT_ALL", "1")
                .with_var("REMOTE_USER", REMOTE_USER)
                .with_var("REMOTE_ADDR", address.ip().to_string())
                .with_var("GATEWAY_INTERFACE", "CGI/1.1")
                .with_var("SERVER_PROTOCOL", "HTTP/1.1"),
            root: root.to_path_buf(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || serve(&listener, &backend, &flag));
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }

    /// Address of the root directory, `http://127.0.0.1:<port>`, without a
    /// trailing slash.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop wakes up only for a connection.
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The CGI program and its fixed environment.
#[derive(Debug)]
struct Backend {
    git: GitProgram,
    env: GitEnv,
    root: PathBuf,
}

fn serve(listener: &TcpListener, backend: &Backend, stop: &AtomicBool) {
    std::thread::scope(|scope| {
        for stream in listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(stream) = stream {
                scope.spawn(move || {
                    let _ = backend.handle(&stream);
                });
            }
        }
    });
}

/// One parsed HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    method: String,
    path: String,
    query: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// One response before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Response {
    status: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn plain(status: &str, message: &str) -> Self {
        Self {
            status: status.to_owned(),
            headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
            body: message.as_bytes().to_vec(),
        }
    }

    fn write_to(&self, mut out: impl Write) -> io::Result<()> {
        let mut head = format!("HTTP/1.1 {}\r\n", self.status);
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.body.len()
        ));
        out.write_all(head.as_bytes())?;
        out.write_all(&self.body)?;
        out.flush()
    }
}

impl Backend {
    fn handle(&self, stream: &TcpStream) -> io::Result<()> {
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        let mut reader = BufReader::new(stream);
        let response = match read_request(&mut reader) {
            Ok(request) => self.answer(&request),
            Err(error) => Response::plain("400 Bad Request", &error.to_string()),
        };
        response.write_to(stream)
    }

    fn answer(&self, request: &Request) -> Response {
        let mut env = self
            .env
            .with_var("REQUEST_METHOD", &request.method)
            .with_var("PATH_INFO", percent_decode(&request.path))
            .with_var("QUERY_STRING", &request.query)
            .with_var("CONTENT_LENGTH", request.body.len().to_string());
        let forwarded = [
            ("content-type", "CONTENT_TYPE"),
            ("git-protocol", "GIT_PROTOCOL"),
            ("content-encoding", "HTTP_CONTENT_ENCODING"),
        ];
        for (header, variable) in forwarded {
            if let Some(value) = request.headers.get(header) {
                env = env.with_var(variable, value);
            }
        }
        match self
            .git
            .run_with_input(&env, &self.root, &["http-backend"], Some(&request.body))
        {
            Ok(output) => cgi_response(&output.stdout),
            Err(error) => Response::plain("500 Internal Server Error", &error.to_string()),
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Reads one line ending with LF, without the line end.
fn read_line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_LINE_BYTES as u64)
        .read_until(b'\n', &mut line)?;
    if read == 0 {
        return Err(invalid("the connection closed before the request ended"));
    }
    if line.last() != Some(&b'\n') {
        return Err(invalid("a request line is too long"));
    }
    let text = String::from_utf8(line).map_err(|_| invalid("a request line is not valid UTF-8"))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn read_request(reader: &mut impl BufRead) -> io::Result<Request> {
    let request_line = read_line(reader)?;
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(invalid("the request line has no method and target"));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = BTreeMap::new();
    loop {
        let line = read_line(reader)?;
        if line.is_empty() {
            break;
        }
        if headers.len() == MAX_HEADERS {
            return Err(invalid("the request has too many headers"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("a header line has no colon"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let chunked = headers
        .get("transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"));
    let body = if chunked {
        read_chunked(reader)?
    } else {
        let length = match headers.get("content-length") {
            Some(text) => text
                .parse()
                .map_err(|_| invalid("Content-Length is not a number"))?,
            None => 0,
        };
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        body
    };
    Ok(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        headers,
        body,
    })
}

/// Reads a body in chunked transfer coding (RFC 9112, section 7.1).
fn read_chunked(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line = read_line(reader)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| invalid("invalid chunk size"))?;
        if size == 0 {
            while !read_line(reader)?.is_empty() {}
            return Ok(body);
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        if !read_line(reader)?.is_empty() {
            return Err(invalid("a chunk is longer than its size"));
        }
    }
}

/// Splits the output of a CGI script into a response (RFC 3875, section 6).
fn cgi_response(output: &[u8]) -> Response {
    let (head, body) = split_head(output);
    let mut status = "200 OK".to_owned();
    let mut headers = Vec::new();
    for line in String::from_utf8_lossy(head).lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("status") {
            value.trim().clone_into(&mut status);
        } else {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    Response {
        status,
        headers,
        body: body.to_vec(),
    }
}

fn split_head(output: &[u8]) -> (&[u8], &[u8]) {
    let find = |separator: &[u8]| {
        output
            .windows(separator.len())
            .position(|window| window == separator)
            .map(|position| (position, separator.len()))
    };
    let crlf = find(b"\r\n\r\n");
    let lf = find(b"\n\n");
    let first = match (crlf, lf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (found, None) | (None, found) => found,
    };
    match first {
        Some((position, length)) => (&output[..position], &output[position + length..]),
        None => (&[], output),
    }
}

/// Decodes `%XX` escapes of a request path; invalid escapes stay as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let value = text
                .get(index + 1..index + 3)
                .and_then(|digits| u8::from_str_radix(digits, 16).ok());
            if let Some(value) = value {
                decoded.push(value);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &[u8]) -> io::Result<Request> {
        read_request(&mut BufReader::new(text))
    }

    #[test]
    fn reads_a_request_with_a_length_and_a_query() {
        let request = parse(
            b"POST /a/up.git/git-upload-pack?x=1 HTTP/1.1\r\nContent-Type: application/x-git-upload-pack-request\r\nGit-Protocol: version=2\r\nContent-Length: 4\r\n\r\nbodyrest",
        )
        .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/a/up.git/git-upload-pack");
        assert_eq!(request.query, "x=1");
        assert_eq!(request.headers["git-protocol"], "version=2");
        assert_eq!(request.body, b"body");
    }

    #[test]
    fn decodes_chunked_bodies() {
        let request = parse(
            b"POST /r HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4;ext=1\r\nabcd\r\n2\r\nef\r\n0\r\nTrailer: x\r\n\r\n",
        )
        .unwrap();
        assert_eq!(request.body, b"abcdef");
        assert!(parse(b"POST /r HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_err());
    }

    #[test]
    fn rejects_malformed_requests() {
        assert!(parse(b"").is_err());
        assert!(parse(b"GET\r\n\r\n").is_err());
        assert!(parse(b"GET / HTTP/1.1\r\nbroken\r\n\r\n").is_err());
        assert!(parse(b"GET / HTTP/1.1\r\nContent-Length: x\r\n\r\n").is_err());
        assert!(parse(b"POST / HTTP/1.1\r\nContent-Length: 9\r\n\r\nshort").is_err());
    }

    #[test]
    fn converts_cgi_output_to_a_response() {
        let response =
            cgi_response(b"Status: 404 Not Found\r\nContent-Type: text/plain\r\n\r\nmissing");
        assert_eq!(response.status, "404 Not Found");
        assert_eq!(
            response.headers,
            vec![("Content-Type".to_owned(), "text/plain".to_owned())]
        );
        assert_eq!(response.body, b"missing");
        let plain = cgi_response(b"Content-Type: x\n\n\r\n\r\nbody");
        assert_eq!(plain.status, "200 OK");
        assert_eq!(plain.body, b"\r\n\r\nbody");
        assert_eq!(cgi_response(b"no head").body, b"no head");
    }

    #[test]
    fn writes_responses_with_length_and_close() {
        let mut out = Vec::new();
        Response::plain("200 OK", "hi").write_to(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\nConnection: close\r\n\r\nhi"));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("/a%20b/%7Ex"), "/a b/~x");
        assert_eq!(percent_decode("/bad%zz%4"), "/bad%zz%4");
    }
}
