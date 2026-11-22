// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Minimal HTTP/1.1 server on the loopback interface for tests of the
//! backend: every connection is answered by a handler and then closed.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// Request as seen by the handler.
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}

/// Response produced by the handler.
pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    pub(crate) fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self::status(200)
            .with_header("Content-Type", content_type)
            .with_body(body)
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(crate) fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

/// Server running until the test process ends.
pub(crate) struct TestServer {
    port: u16,
}

impl TestServer {
    pub(crate) fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback port");
        let port = listener.local_addr().expect("bound address").port();
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = Arc::clone(&handler);
                std::thread::spawn(move || serve(stream, handler.as_ref()));
            }
        });
        Self { port }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Address of a port with no listener.
    pub(crate) fn closed_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback port");
        let port = listener.local_addr().expect("bound address").port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }
}

fn serve(stream: TcpStream, handler: &Handler) {
    let mut reader = BufReader::new(stream.try_clone().expect("cloned stream"));
    let Some(request) = read_request(&mut reader) else {
        return;
    };
    let reply = handler(&request);
    let mut out = format!(
        "HTTP/1.1 {} X\r\nConnection: close\r\nContent-Length: {}\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let mut stream = stream;
    stream.write_all(out.as_bytes()).ok();
    stream.write_all(&reply.body).ok();
}

fn read_request(reader: &mut impl BufRead) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    let find = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    let mut body = Vec::new();
    if find("transfer-encoding").is_some() {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).ok()?;
            let size = usize::from_str_radix(size.trim(), 16).ok()?;
            let mut chunk = vec![0; size + 2];
            reader.read_exact(&mut chunk).ok()?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
    } else if let Some(length) = find("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(length, 0);
        reader.read_exact(&mut body).ok()?;
    }
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

/// Encodes `lines` as packet lines followed by a flush packet.
pub(crate) fn pkt_lines(lines: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(format!("{:04x}{line}", line.len() + 4).as_bytes());
    }
    out.extend_from_slice(b"0000");
    out
}
