// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Streamable HTTP transport, against the mock engine on an ephemeral port.

#![cfg(feature = "http")]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use common::{MockBackend, WAIT};
use nurt_mcp::{ENDPOINT_PATH, McpError, McpHttpOptions, serve_http_listener};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const BOTH: &str = "application/json, text/event-stream";

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }

    /// Data of every event of an event stream, in order.
    fn events(&self) -> Vec<Value> {
        assert_eq!(
            self.headers.get("content-type").map(String::as_str),
            Some("text/event-stream")
        );
        self.body
            .split("\n\n")
            .filter(|event| !event.is_empty())
            .map(|event| {
                let data = event
                    .lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .unwrap();
                serde_json::from_str(data).unwrap()
            })
            .collect()
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

    async fn send(&self, method: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Reply {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut request = format!(
            "{method} {ENDPOINT_PATH} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            self.address,
            body.len()
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(&body);
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        tokio::time::timeout(WAIT, stream.read_to_end(&mut raw))
            .await
            .expect("no reply in time")
            .unwrap();
        let raw = String::from_utf8(raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let mut lines = head.lines();
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|line| line.split_once(": "))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
            .collect();
        Reply {
            status,
            headers,
            body: body.to_owned(),
        }
    }

    async fn post(&self, headers: &[(&str, &str)], body: &Value) -> Reply {
        self.send("POST", headers, Some(body)).await
    }

    /// Opens a session and returns its identifier.
    async fn initialize(&self, headers: &[(&str, &str)]) -> String {
        let reply = self
            .post(
                headers,
                &json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "test", "version": "1.0.0"}
                    }
                }),
            )
            .await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert_eq!(reply.json()["result"]["protocolVersion"], "2025-06-18");
        let session = reply.headers.get("mcp-session-id").cloned().unwrap();
        let mut with_session = headers.to_vec();
        with_session.push(("Mcp-Session-Id", session.as_str()));
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

fn call(id: u64, program: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "nurt", "arguments": {"program": program}, "_meta": {"progressToken": id}}
    })
}

#[tokio::test]
async fn session_maps_to_the_engine_session_and_ends_with_delete() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let headers = [
        ("Mcp-Session-Id", session.as_str()),
        ("Accept", "application/json"),
    ];

    let reply = server.post(&headers, &call(2, "session")).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["result"]["content"][0]["text"], session);

    let reply = server.send("DELETE", &headers, None).await;
    assert_eq!(reply.status, 204);
    let reply = server.post(&headers, &call(3, "ok")).await;
    assert_eq!(reply.status, 404);
    let reply = server.send("DELETE", &headers, None).await;
    assert_eq!(reply.status, 404);
}

#[tokio::test]
async fn event_stream_carries_progress_before_the_response() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let reply = server
        .post(
            &[("Mcp-Session-Id", session.as_str()), ("Accept", BOTH)],
            &call(2, "progress"),
        )
        .await;
    assert_eq!(reply.status, 200);
    let events = reply.events();
    let methods: Vec<&str> = events
        .iter()
        .map(|event| event["method"].as_str().unwrap_or("response"))
        .collect();
    assert_eq!(
        methods,
        [
            "notifications/progress",
            "notifications/progress",
            "notifications/progress",
            "notifications/message",
            "response"
        ]
    );
    assert_eq!(events[4]["result"]["content"][0]["text"], "ok");
}

#[tokio::test]
async fn sessions_are_independent() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let first = server.initialize(&[]).await;
    let second = server.initialize(&[]).await;
    assert_ne!(first, second);
    let reply = server
        .post(&[("Mcp-Session-Id", second.as_str())], &call(2, "session"))
        .await;
    assert_eq!(reply.json()["result"]["content"][0]["text"], second);
}

#[tokio::test]
async fn requests_without_a_known_session_are_rejected() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let reply = server.post(&[], &call(1, "ok")).await;
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["error"]["code"], -32600);
    let reply = server
        .post(&[("Mcp-Session-Id", "unknown")], &call(1, "ok"))
        .await;
    assert_eq!(reply.status, 404);
    let reply = server.send("DELETE", &[], None).await;
    assert_eq!(reply.status, 400);
}

#[tokio::test]
async fn foreign_origin_is_rejected_and_loopback_or_listed_origins_pass() {
    let options = McpHttpOptions::new(0).with_allowed_origin("https://example.com");
    let server = Server::start(MockBackend::new(), options).await;
    let reply = server
        .post(
            &[("Origin", "https://attacker.example.net")],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 403);
    let reply = server.send("DELETE", &[("Origin", "null")], None).await;
    assert_eq!(reply.status, 403);
    server
        .initialize(&[("Origin", "http://localhost:6274")])
        .await;
    server
        .initialize(&[("Origin", "https://example.com")])
        .await;
}

#[tokio::test]
async fn bearer_token_is_required_when_configured() {
    let options = McpHttpOptions::new(0).with_bearer_token("s3cret");
    let server = Server::start(MockBackend::new(), options).await;
    let reply = server.post(&[], &call(1, "ok")).await;
    assert_eq!(reply.status, 401);
    assert_eq!(
        reply.headers.get("www-authenticate").map(String::as_str),
        Some("Bearer")
    );
    let reply = server
        .post(&[("Authorization", "Bearer wrong")], &call(1, "ok"))
        .await;
    assert_eq!(reply.status, 401);
    server
        .initialize(&[("Authorization", "Bearer s3cret")])
        .await;
}

#[tokio::test]
async fn malformed_bodies_and_versions_are_rejected() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let session = server.initialize(&[]).await;
    let headers = [("Mcp-Session-Id", session.as_str())];

    let reply = server.send("POST", &headers, None).await;
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["error"]["code"], -32700);
    let reply = server.post(&headers, &json!([call(1, "ok")])).await;
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["error"]["code"], -32600);
    let reply = server
        .post(
            &[
                ("Mcp-Session-Id", session.as_str()),
                ("MCP-Protocol-Version", "1999-01-01"),
            ],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 400);
    let reply = server
        .post(
            &[
                ("Mcp-Session-Id", session.as_str()),
                ("MCP-Protocol-Version", "2025-06-18"),
            ],
            &call(1, "ok"),
        )
        .await;
    assert_eq!(reply.status, 200);
    let reply = server
        .post(
            &[],
            &json!({"jsonrpc": "2.0", "method": "initialize", "params": {}}),
        )
        .await;
    assert_eq!(reply.status, 400);
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
    assert!(
        reply.headers.get("mcp-session-id").is_none(),
        "{}",
        reply.body
    );
    assert!(reply.json().get("error").is_some(), "{}", reply.body);
}

#[tokio::test]
async fn get_is_not_allowed() {
    let server = Server::start(MockBackend::new(), McpHttpOptions::new(0)).await;
    let reply = server
        .send("GET", &[("Accept", "text/event-stream")], None)
        .await;
    assert_eq!(reply.status, 405);
    assert_eq!(
        reply.headers.get("allow").map(String::as_str),
        Some("POST, DELETE")
    );
}

#[tokio::test]
async fn cancelled_request_ends_without_a_response() {
    let backend = MockBackend::new();
    let server = Arc::new(Server::start(backend.clone(), McpHttpOptions::new(0)).await);
    let session = server.initialize(&[]).await;
    let waiting = {
        let server = Arc::clone(&server);
        let session = session.clone();
        tokio::spawn(async move {
            server
                .post(
                    &[("Mcp-Session-Id", session.as_str()), ("Accept", BOTH)],
                    &call(5, "wait"),
                )
                .await
        })
    };
    let started = backend.clone();
    assert!(
        tokio::task::spawn_blocking(move || started.started.wait_for(1))
            .await
            .unwrap()
    );
    let reply = server
        .post(
            &[("Mcp-Session-Id", session.as_str())],
            &json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 5}}),
        )
        .await;
    assert_eq!(reply.status, 202);
    let reply = waiting.await.unwrap();
    assert_eq!(reply.status, 200);
    assert!(reply.events().iter().all(|event| event.get("id").is_none()));
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
