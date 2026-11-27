// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-mcp/tests/http.rs
// Responsibility:   Streamable HTTP transport, against the mock engine on an ephemeral port.
// Dependencies:     common, serde_json, tokio
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Streamable HTTP transport, against the mock engine on an ephemeral port.

#![cfg(feature = "http")]
// Helpers abort the test on a broken expectation, as test code does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use common::{MockBackend, WAIT};
use nurt_mcp::{ENDPOINT_PATH, McpError, McpHttpOptions, serve_http_listener};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const BOTH: &str = "application/json, text/event-stream";
const EVENT_STREAM: &str = "text/event-stream";
const SESSION: &str = "Mcp-Session-Id";

/// One event of an event stream.
#[derive(Debug)]
struct SseEvent {
    id: Option<String>,
    /// The JSON-RPC message; `None` for an event without data, such as the
    /// event that opens a stream.
    data: Option<Value>,
}

impl SseEvent {
    /// Parses one event block; `None` for a block of comments only.
    fn parse(block: &str) -> Option<Self> {
        let mut id = None;
        let mut data: Option<String> = None;
        let mut has_field = false;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("id:") {
                id = Some(value.trim_start().to_owned());
                has_field = true;
            } else if let Some(value) = line.strip_prefix("data:") {
                data.get_or_insert_with(String::new)
                    .push_str(value.strip_prefix(' ').unwrap_or(value));
                has_field = true;
            } else if line.starts_with("retry:") || line.starts_with("event:") {
                has_field = true;
            }
        }
        has_field.then(|| Self {
            id,
            data: data
                .filter(|data| !data.is_empty())
                .map(|data| serde_json::from_str(&data).unwrap()),
        })
    }
}

/// Response whose body is read as it arrives.
struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    reader: BufReader<TcpStream>,
    chunked: bool,
    length: Option<usize>,
    buffer: String,
    finished: bool,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Reads the next piece of the body into the buffer; `false` at its end.
    async fn read_more(&mut self) -> bool {
        if self.finished {
            return false;
        }
        let piece = tokio::time::timeout(WAIT, self.read_piece())
            .await
            .expect("no body data in time");
        match piece {
            Some(text) => {
                self.buffer.push_str(&text.replace('\r', ""));
                true
            }
            None => {
                self.finished = true;
                false
            }
        }
    }

    async fn read_piece(&mut self) -> Option<String> {
        let mut data = Vec::new();
        if self.chunked {
            let mut line = String::new();
            self.reader.read_line(&mut line).await.unwrap();
            let size = usize::from_str_radix(line.trim(), 16).unwrap_or(0);
            if size == 0 {
                return None;
            }
            data.resize(size + 2, 0);
            self.reader.read_exact(&mut data).await.unwrap();
            data.truncate(size);
        } else {
            match self.length {
                Some(0) => return None,
                Some(length) => {
                    data.resize(length, 0);
                    self.reader.read_exact(&mut data).await.unwrap();
                    self.length = Some(0);
                }
                None => {
                    self.reader.read_to_end(&mut data).await.unwrap();
                    if data.is_empty() {
                        return None;
                    }
                }
            }
        }
        Some(String::from_utf8(data).unwrap())
    }

    /// Whole body as text.
    async fn text(mut self) -> String {
        while self.read_more().await {}
        self.buffer
    }

    /// Next event of an event stream; `None` at the end of the stream.
    async fn next_event(&mut self) -> Option<SseEvent> {
        assert_eq!(self.header("content-type"), Some(EVENT_STREAM));
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                if let Some(event) = SseEvent::parse(&block) {
                    return Some(event);
                }
                continue;
            }
            if !self.read_more().await {
                return None;
            }
        }
    }

    /// Next JSON-RPC message of an event stream; `None` at the end of the
    /// stream.
    async fn next_message(&mut self) -> Option<Value> {
        loop {
            if let Some(data) = self.next_event().await?.data {
                return Some(data);
            }
        }
    }

    /// Every remaining JSON-RPC message of an event stream.
    async fn messages(mut self) -> Vec<Value> {
        let mut messages = Vec::new();
        while let Some(message) = self.next_message().await {
            messages.push(message);
        }
        messages
    }
}

struct Server {
    address: SocketAddr,
    task: JoinHandle<Result<(), McpError>>,
}

impl Server {
    async fn start(backend: Arc<MockBackend>, options: McpHttpOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(serve_http_listener(backend, options, listener));
        Self { address, task }
    }

    /// Sends a request with exactly the given headers, apart from `Host`
    /// when they name none.
    async fn send(&self, method: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Reply {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut request = format!(
            "{method} {ENDPOINT_PATH} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"))
        {
            request.push_str(&format!("Host: {}\r\n", self.address));
        }
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(&body);
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut reader = BufReader::new(stream);
        let head = tokio::time::timeout(WAIT, async {
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let line = line.trim_end().to_owned();
                if line.is_empty() {
                    return lines;
                }
                lines.push(line);
            }
        })
        .await
        .expect("no response head in time");
        let status = head[0].split(' ').nth(1).unwrap().parse().unwrap();
        let headers: HashMap<String, String> = head[1..]
            .iter()
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let chunked = headers
            .get("transfer-encoding")
            .is_some_and(|value| value.eq_ignore_ascii_case("chunked"));
        let length = headers
            .get("content-length")
            .map(|value| value.parse().unwrap());
        Reply {
            status,
            headers,
            reader,
            chunked,
            length,
            buffer: String::new(),
            finished: false,
        }
    }

    /// Sends a message by `POST`, accepting both response media types unless
    /// the headers say otherwise.
    async fn post(&self, headers: &[(&str, &str)], body: &Value) -> Reply {
        let mut all = headers.to_vec();
        for (name, value) in [("Accept", BOTH), ("Content-Type", "application/json")] {
            if !headers
                .iter()
                .any(|(given, _)| given.eq_ignore_ascii_case(name))
            {
                all.push((name, value));
            }
        }
        self.send("POST", &all, Some(body)).await
    }

    /// Opens a session and returns its identifier.
    async fn initialize(&self, headers: &[(&str, &str)]) -> String {
        let reply = self.post(headers, &initialize_request()).await;
        assert_eq!(reply.status, 200);
        let session = reply.header("mcp-session-id").unwrap().to_owned();
        let messages = reply.messages().await;
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(messages[0]["result"]["protocolVersion"], "2025-06-18");
        let mut with_session = headers.to_vec();
        with_session.push((SESSION, session.as_str()));
        let reply = self
            .post(
                &with_session,
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            )
            .await;
        assert_eq!(reply.status, 202);
        session
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0.0"}
        }
    })
}

fn call(id: u64, program: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "nurt", "arguments": {"program": program}, "_meta": {"progressToken": id}}
    })
}

fn cancelled(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": id}})
}

fn method(message: &Value) -> &str {
    message["method"].as_str().unwrap_or("response")
}

#[tokio::test]
async fn session_maps_to_the_engine_session_and_ends_with_delete() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let headers = [(SESSION, session.as_str())];

    let reply = server.post(&headers, &call(2, "session")).await;
    assert_eq!(reply.status, 200);
    let messages = reply.messages().await;
    assert_eq!(
        messages.last().unwrap()["result"]["content"][0]["text"],
        session
    );

    assert_eq!(server.send("DELETE", &headers, None).await.status, 202);
    assert_eq!(server.post(&headers, &call(3, "ok")).await.status, 404);
    assert_eq!(server.send("DELETE", &headers, None).await.status, 404);
}

#[tokio::test]
async fn request_stream_carries_progress_before_the_response() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let reply = server
        .post(&[(SESSION, session.as_str())], &call(2, "progress"))
        .await;
    assert_eq!(reply.status, 200);
    let messages = reply.messages().await;
    let methods: Vec<&str> = messages.iter().map(method).collect();
    assert_eq!(
        methods,
        [
            "notifications/progress",
            "notifications/progress",
            "notifications/progress",
            "response"
        ]
    );
    assert_eq!(messages[3]["id"], 2);
    assert_eq!(messages[3]["result"]["content"][0]["text"], "ok");
}

#[tokio::test]
async fn progress_is_streamed_while_the_call_runs_and_cancellation_ends_the_stream() {
    let backend = MockBackend::new();
    let server = Server::start(backend.clone(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let headers = [(SESSION, session.as_str())];
    let mut stream = server.post(&headers, &call(5, "progress-wait")).await;
    assert_eq!(stream.status, 200);

    let progress = stream.next_message().await.unwrap();
    assert_eq!(progress["method"], "notifications/progress");
    assert_eq!(progress["params"]["progressToken"], 5);
    assert_eq!(progress["params"]["message"], "scanning");
    assert_eq!(backend.cancelled.get(), 0);

    assert_eq!(server.post(&headers, &cancelled(5)).await.status, 202);
    assert!(backend.wait_cancelled(1).await);
    let rest = stream.messages().await;
    assert!(
        rest.iter().all(|message| message.get("id").is_none()),
        "{rest:?}"
    );
}

#[tokio::test]
async fn get_opens_the_session_stream_that_carries_log_messages() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let mut stream = server
        .send(
            "GET",
            &[(SESSION, session.as_str()), ("Accept", EVENT_STREAM)],
            None,
        )
        .await;
    assert_eq!(stream.status, 200);

    let reply = server
        .post(&[(SESSION, session.as_str())], &call(2, "progress"))
        .await;
    assert_eq!(reply.status, 200);
    let messages = reply.messages().await;
    assert!(
        messages
            .iter()
            .all(|message| method(message) != "notifications/message")
    );

    let log = stream.next_message().await.unwrap();
    assert_eq!(log["method"], "notifications/message");
    assert_eq!(log["params"]["level"], "warning");
    assert_eq!(log["params"]["logger"], "space");
    assert_eq!(log["params"]["data"], "slow volume");
}

#[tokio::test]
async fn get_requires_an_event_stream_and_a_known_session() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let reply = server
        .send(
            "GET",
            &[(SESSION, session.as_str()), ("Accept", "application/json")],
            None,
        )
        .await;
    assert_eq!(reply.status, 406);
    let reply = server.send("GET", &[("Accept", EVENT_STREAM)], None).await;
    assert_eq!(reply.status, 400);
    let reply = server
        .send(
            "GET",
            &[(SESSION, "unknown"), ("Accept", EVENT_STREAM)],
            None,
        )
        .await;
    assert_eq!(reply.status, 404);
}

#[tokio::test]
async fn interrupted_stream_resumes_from_the_last_event_id() {
    let backend = MockBackend::new();
    let server = Server::start(backend.clone(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let headers = [(SESSION, session.as_str())];

    let mut stream = server.post(&headers, &call(7, "progress-wait")).await;
    assert_eq!(stream.status, 200);
    let first = stream.next_event().await.unwrap();
    assert!(first.data.is_none(), "{first:?}");
    let last_event_id = first.id.unwrap();
    let progress = stream.next_message().await.unwrap();
    assert_eq!(progress["method"], "notifications/progress");
    drop(stream);

    let mut resumed = server
        .send(
            "GET",
            &[
                (SESSION, session.as_str()),
                ("Accept", EVENT_STREAM),
                ("Last-Event-ID", last_event_id.as_str()),
            ],
            None,
        )
        .await;
    assert_eq!(resumed.status, 200);
    assert_eq!(resumed.next_message().await.unwrap(), progress);

    assert_eq!(server.post(&headers, &cancelled(7)).await.status, 202);
    assert!(backend.wait_cancelled(1).await);
    let rest = resumed.messages().await;
    assert!(
        rest.iter().all(|message| message.get("id").is_none()),
        "{rest:?}"
    );
}

#[tokio::test]
async fn sessions_are_independent() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let first = server.initialize(&[]).await;
    let second = server.initialize(&[]).await;
    assert_ne!(first, second);
    let messages = server
        .post(&[(SESSION, second.as_str())], &call(2, "session"))
        .await
        .messages()
        .await;
    assert_eq!(
        messages.last().unwrap()["result"]["content"][0]["text"],
        second
    );
}

#[tokio::test]
async fn requests_without_a_known_session_are_rejected() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    assert_eq!(server.post(&[], &call(1, "ok")).await.status, 400);
    let reply = server.post(&[(SESSION, "unknown")], &call(1, "ok")).await;
    assert_eq!(reply.status, 404);
    assert_eq!(server.send("DELETE", &[], None).await.status, 400);
}

#[tokio::test]
async fn foreign_origin_is_rejected_and_loopback_or_listed_origins_pass() {
    let options = McpHttpOptions::new(0).with_allowed_origin("https://example.com:443");
    let server = Server::start(MockBackend::new(), options).await;
    let reply = server
        .post(
            &[("Origin", "https://attacker.example.net")],
            &initialize_request(),
        )
        .await;
    assert_eq!(reply.status, 403);
    assert!(!reply.headers.contains_key("mcp-session-id"));
    let reply = server.send("DELETE", &[("Origin", "null")], None).await;
    assert_eq!(reply.status, 403);
    server
        .initialize(&[("Origin", "http://localhost:6274")])
        .await;
    server.initialize(&[("Origin", "http://[::1]:3000")]).await;
    server
        .initialize(&[("Origin", "https://example.com")])
        .await;
}

#[tokio::test]
async fn foreign_host_is_rejected_and_listed_hosts_pass() {
    let options = McpHttpOptions::new(0).with_allowed_host("nurt.example.com");
    let server = Server::start(MockBackend::new(), options).await;
    let reply = server
        .post(
            &[("Host", "attacker.example.net:8765")],
            &initialize_request(),
        )
        .await;
    assert_eq!(reply.status, 403);
    server.initialize(&[("Host", "localhost:8765")]).await;
    server
        .initialize(&[("Host", "nurt.example.com:8765")])
        .await;
}

#[tokio::test]
async fn bearer_token_is_required_when_configured() {
    let options = McpHttpOptions::new(0).with_bearer_token("s3cret");
    let server = Server::start(MockBackend::new(), options).await;
    let reply = server.post(&[], &initialize_request()).await;
    assert_eq!(reply.status, 401);
    assert_eq!(reply.header("www-authenticate"), Some("Bearer"));
    let reply = server
        .post(&[("Authorization", "Bearer wrong")], &initialize_request())
        .await;
    assert_eq!(reply.status, 401);
    assert!(reply.text().await.contains("Authorization: Bearer"));
    server
        .initialize(&[("Authorization", "Bearer s3cret")])
        .await;
}

#[tokio::test]
async fn malformed_requests_are_rejected() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let session = session.as_str();

    let reply = server
        .send(
            "POST",
            &[
                (SESSION, session),
                ("Accept", BOTH),
                ("Content-Type", "application/json"),
            ],
            None,
        )
        .await;
    assert_eq!(reply.status, 415);
    let reply = server
        .post(&[(SESSION, session)], &json!([call(1, "ok")]))
        .await;
    assert_eq!(reply.status, 415);
    let reply = server
        .post(
            &[(SESSION, session), ("Accept", "application/json")],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 406);
    let reply = server
        .post(
            &[(SESSION, session), ("Content-Type", "text/plain")],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 415);
    let reply = server
        .post(
            &[(SESSION, session), ("MCP-Protocol-Version", "1999-01-01")],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 400);
    let reply = server
        .post(
            &[(SESSION, session), ("MCP-Protocol-Version", "2025-06-18")],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 200);
    let messages = reply.messages().await;
    assert_eq!(messages.last().unwrap()["result"]["isError"], false);
    let reply = server
        .post(
            &[],
            &json!({"jsonrpc": "2.0", "method": "initialize", "params": {}}),
        )
        .await;
    assert_eq!(reply.status, 400);
    assert!(!reply.headers.contains_key("mcp-session-id"));
}

#[tokio::test]
async fn failed_initialization_opens_no_session() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let reply = server
        .post(
            &[],
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}),
        )
        .await;
    assert!((400..500).contains(&reply.status), "{}", reply.status);
    assert!(!reply.headers.contains_key("mcp-session-id"));
}

#[tokio::test]
async fn unsupported_methods_are_not_allowed() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let reply = server.send("PUT", &[], None).await;
    assert_eq!(reply.status, 405);
    assert_eq!(reply.header("allow"), Some("GET, POST, DELETE"));
}

#[tokio::test]
async fn remote_listener_requires_explicit_permission() {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let result = serve_http_listener(MockBackend::new(), McpHttpOptions::new(0), listener).await;
    assert!(
        matches!(result, Err(McpError::InvalidOptions(_))),
        "{result:?}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
