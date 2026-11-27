// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/support/http_server.rs
// Responsibility:   HTTP/1.1 server on the loopback interface for tests of the `network`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! HTTP/1.1 server on the loopback interface for tests of the `network`
//! commands, and a name resolver that answers from a table.
//!
//! Every connection is served by its own thread and closed after one
//! response. Responses come from a handler function; the server records
//! every request, the number of connections open at the same time and the
//! connections the client closed before the body was complete.

#![allow(
    dead_code,
    reason = "each test binary includes this module and uses a different part of it"
)]
#![allow(
    clippy::expect_used,
    reason = "a failure of the test server ends the test that uses it"
)]

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nurt_net::client::NameResolver;

/// Interval at which a stalled response checks whether the client left.
const STALL_POLL: Duration = Duration::from_millis(10);

/// A request as received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// Method in upper case.
    pub(crate) method: String,
    /// Path with the query.
    pub(crate) path: String,
    /// Headers, names in lower case.
    pub(crate) headers: Vec<(String, String)>,
    /// Body.
    pub(crate) body: Vec<u8>,
}

impl Request {
    /// First value of a header.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Path without the query.
    pub(crate) fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or_default()
    }
}

/// A response to send.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Reply {
    /// Status code.
    pub(crate) status: u16,
    /// Headers; `content-length` is added unless given.
    pub(crate) headers: Vec<(String, String)>,
    /// Body.
    pub(crate) body: Vec<u8>,
    /// Wait before the head is sent.
    pub(crate) delay: Duration,
    /// Bytes of the body sent before `stall`.
    pub(crate) stall_after: Option<usize>,
    /// Wait after the first `stall_after` bytes of the body.
    pub(crate) stall: Duration,
}

impl Reply {
    /// A response with a status and no body.
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }

    /// A response with a media type and a body.
    pub(crate) fn body(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self::status(status)
            .header("content-type", content_type)
            .with_body(body)
    }

    /// Adds a header.
    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Replaces the body.
    pub(crate) fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// Waits before sending the head.
    pub(crate) fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Sends `bytes` bytes of the body, then waits `stall` before the rest.
    pub(crate) fn stalled(mut self, bytes: usize, stall: Duration) -> Self {
        self.stall_after = Some(bytes);
        self.stall = stall;
        self
    }

    /// A redirect to `location`.
    pub(crate) fn redirect(status: u16, location: &str) -> Self {
        Self::status(status).header("location", location)
    }
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

struct State {
    handler: Box<Handler>,
    requests: Mutex<Vec<Request>>,
    open: AtomicUsize,
    peak: AtomicUsize,
    aborted: AtomicUsize,
    stop: AtomicBool,
}

/// Running test server; stops when dropped.
pub(crate) struct TestServer {
    address: SocketAddr,
    state: Arc<State>,
}

impl TestServer {
    /// Starts a server on `127.0.0.1` with a free port.
    pub(crate) fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
        let address = listener.local_addr().expect("local address");
        let state = Arc::new(State {
            handler: Box::new(handler),
            requests: Mutex::new(Vec::new()),
            open: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            aborted: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        let shared = Arc::clone(&state);
        thread::spawn(move || accept(&listener, &shared));
        Self { address, state }
    }

    /// Address of the server.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// Port of the server.
    pub(crate) fn port(&self) -> u16 {
        self.address.port()
    }

    /// `http://127.0.0.1:<port><path>`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// Requests received so far, in order of arrival.
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.state.requests.lock().expect("requests").clone()
    }

    /// Number of requests received so far.
    pub(crate) fn request_count(&self) -> usize {
        self.state.requests.lock().expect("requests").len()
    }

    /// Largest number of connections served at the same time.
    pub(crate) fn peak_connections(&self) -> usize {
        self.state.peak.load(Ordering::SeqCst)
    }

    /// Connections closed by the client before the whole body was sent.
    pub(crate) fn aborted(&self) -> usize {
        self.state.aborted.load(Ordering::SeqCst)
    }

    /// Waits up to `limit` until `aborted` reaches `count`.
    pub(crate) fn wait_aborted(&self, count: usize, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if self.aborted() >= count {
                return true;
            }
            thread::sleep(STALL_POLL);
        }
        self.aborted() >= count
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
    }
}

fn accept(listener: &TcpListener, state: &Arc<State>) {
    for stream in listener.incoming() {
        if state.stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let state = Arc::clone(state);
        thread::spawn(move || {
            let open = state.open.fetch_add(1, Ordering::SeqCst) + 1;
            state.peak.fetch_max(open, Ordering::SeqCst);
            let _ = serve(stream, &state);
            state.open.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn serve(stream: TcpStream, state: &State) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(request) = read_request(&mut reader)? else {
        return Ok(());
    };
    state
        .requests
        .lock()
        .expect("requests")
        .push(request.clone());
    let reply = (state.handler)(&request);
    write_reply(stream, &request, &reply, state)
}

fn read_request(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(Request {
        method,
        path,
        headers,
        body,
    }))
}

fn write_reply(
    mut stream: TcpStream,
    request: &Request,
    reply: &Reply,
    state: &State,
) -> io::Result<()> {
    thread::sleep(reply.delay);
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reason(reply.status));
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let has_length = reply
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
    if !has_length {
        head.push_str(&format!("content-length: {}\r\n", reply.body.len()));
    }
    head.push_str("connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    if request.method == "HEAD" {
        return stream.flush();
    }
    let split = reply
        .stall_after
        .unwrap_or(reply.body.len())
        .min(reply.body.len());
    stream.write_all(&reply.body[..split])?;
    stream.flush()?;
    if reply.stall_after.is_some() && client_left(&stream, reply.stall)? {
        state.aborted.fetch_add(1, Ordering::SeqCst);
        return Ok(());
    }
    if let Err(error) = stream
        .write_all(&reply.body[split..])
        .and_then(|()| stream.flush())
    {
        state.aborted.fetch_add(1, Ordering::SeqCst);
        return Err(error);
    }
    Ok(())
}

fn client_left(stream: &TcpStream, stall: Duration) -> io::Result<bool> {
    let start = Instant::now();
    stream.set_read_timeout(Some(STALL_POLL))?;
    let mut probe = [0_u8; 1];
    let mut reader = stream;
    while start.elapsed() < stall {
        match reader.read(&mut probe) {
            Ok(0) => return Ok(true),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Ok(true),
        }
    }
    Ok(false)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// Byte range `bytes=N-` or `bytes=N-M` of a request, for a body of `len`
/// bytes: `Some((start, end_exclusive))`.
pub(crate) fn requested_range(request: &Request, len: usize) -> Option<(usize, usize)> {
    let spec = request.header("range")?.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: usize = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => len,
        text => text.parse::<usize>().ok()?.saturating_add(1).min(len),
    };
    (start < end).then_some((start, end))
}

/// Resolver that answers from a table and counts lookups; unknown names
/// fail.
#[derive(Default)]
pub(crate) struct MockDns {
    table: Mutex<HashMap<String, Vec<IpAddr>>>,
    lookups: AtomicUsize,
}

impl MockDns {
    /// A resolver that maps every name in `names` to `127.0.0.1`.
    pub(crate) fn loopback(names: &[&str]) -> Arc<Self> {
        let dns = Self::default();
        for name in names {
            dns.set(name, &[IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        }
        Arc::new(dns)
    }

    /// Sets the answer for `name`.
    pub(crate) fn set(&self, name: &str, addresses: &[IpAddr]) {
        self.table
            .lock()
            .expect("table")
            .insert(name.to_ascii_lowercase(), addresses.to_vec());
    }

    /// Number of lookups made.
    pub(crate) fn lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }
}

impl NameResolver for MockDns {
    fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.table
            .lock()
            .expect("table")
            .get(&host.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown name"))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
