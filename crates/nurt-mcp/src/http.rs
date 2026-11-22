// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Streamable HTTP transport.
//!
//! One endpoint, [`ENDPOINT_PATH`], accepts JSON-RPC messages by `POST`. An
//! `initialize` request opens a session whose identifier the server returns
//! in the `Mcp-Session-Id` header; every later message carries the header
//! and `DELETE` ends the session. Each session is served by its own protocol
//! handler, with the session identifier as the default engine session.
//!
//! The response to a request is a JSON document, or, when the client
//! accepts `text/event-stream`, an event stream with the notifications of
//! the request followed by the response. The server opens no stream of its
//! own, so `GET` is answered with `405 Method Not Allowed`.
//!
//! Every request with an `Origin` header is checked against the allowed
//! origins to prevent DNS rebinding: origins on a loopback host and origins
//! given in [`McpHttpOptions::with_allowed_origin`] are accepted.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use rmcp::ServiceExt;
use rmcp::model::ProtocolVersion;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::backend::Backend;
use crate::error::McpError;
use crate::server::{MAX_PROTOCOL_VERSION, NurtServer, Shared};

/// Path of the single endpoint.
pub const ENDPOINT_PATH: &str = "/mcp";

const SESSION_HEADER: &str = "mcp-session-id";
const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const EVENT_STREAM: &str = "text/event-stream";
const JSON_MEDIA: &str = "application/json";
const BEARER_PREFIX: &str = "Bearer ";

/// Bytes buffered in each direction between the HTTP layer and the protocol
/// handler of a session.
const PIPE_CAPACITY: usize = 64 * 1024;

/// JSON-RPC 2.0 error codes used by the transport.
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;

/// Options of the HTTP server.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct McpHttpOptions {
    /// Address to listen on; a loopback address unless
    /// [`allow_remote`](Self::allow_remote) is set.
    pub address: SocketAddr,
    /// Whether an address other than a loopback address is accepted.
    pub allow_remote: bool,
    /// Origins accepted in addition to the origins on a loopback host, for
    /// example `https://example.com`.
    pub allowed_origins: Vec<String>,
    /// Token that every request must carry as `Authorization: Bearer
    /// <token>`; `None` disables the check.
    pub bearer_token: Option<String>,
}

impl std::fmt::Debug for McpHttpOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHttpOptions")
            .field("address", &self.address)
            .field("allow_remote", &self.allow_remote)
            .field("allowed_origins", &self.allowed_origins)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl McpHttpOptions {
    /// Creates options that listen on `127.0.0.1` at the given port; port 0
    /// selects a free port.
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            allow_remote: false,
            allowed_origins: Vec::new(),
            bearer_token: None,
        }
    }

    /// Returns the options with another listening address.
    #[must_use]
    pub fn with_address(mut self, address: SocketAddr) -> Self {
        self.address = address;
        self
    }

    /// Returns the options with listening on a non-loopback address allowed
    /// or forbidden.
    #[must_use]
    pub fn allow_remote(mut self, allow: bool) -> Self {
        self.allow_remote = allow;
        self
    }

    /// Returns the options with one more accepted origin.
    #[must_use]
    pub fn with_allowed_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.push(origin.into());
        self
    }

    /// Returns the options with a bearer token.
    #[must_use]
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    /// Returns the options with the bearer token read from a file; leading
    /// and trailing white space is removed.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Io`] when the file cannot be read and
    /// [`McpError::InvalidOptions`] when it holds no token.
    pub fn with_bearer_token_file(self, path: impl AsRef<Path>) -> Result<Self, McpError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)?;
        let token = non_empty_token(&text).ok_or_else(|| {
            McpError::InvalidOptions(format!(
                "token file `{}` is empty; write the token into it",
                path.display()
            ))
        })?;
        Ok(self.with_bearer_token(token))
    }

    /// Returns the options with the bearer token read from an environment
    /// variable; leading and trailing white space is removed.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::InvalidOptions`] when the variable is not set, is
    /// not valid Unicode or holds no token.
    pub fn with_bearer_token_env(self, name: &str) -> Result<Self, McpError> {
        let token = std::env::var(name)
            .ok()
            .as_deref()
            .and_then(non_empty_token)
            .ok_or_else(|| {
                McpError::InvalidOptions(format!(
                    "environment variable `{name}` holds no token; set it or omit the option"
                ))
            })?;
        Ok(self.with_bearer_token(token))
    }

    fn validate(&self, address: SocketAddr) -> Result<(), McpError> {
        if !address.ip().is_loopback() && !self.allow_remote {
            return Err(McpError::InvalidOptions(format!(
                "address {address} is not a loopback address; allow remote clients explicitly to use it"
            )));
        }
        if self
            .bearer_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty())
        {
            return Err(McpError::InvalidOptions(
                "bearer token is empty; give a token or omit the option".to_owned(),
            ));
        }
        Ok(())
    }
}

fn non_empty_token(text: &str) -> Option<String> {
    let token = text.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// Serves clients over streamable HTTP at the address of the options until
/// the process ends.
///
/// The function starts its own asynchronous runtime and blocks the calling
/// thread.
///
/// # Errors
///
/// Returns [`McpError::InvalidOptions`] for a non-loopback address without
/// [`McpHttpOptions::allow_remote`] or for an empty token, and
/// [`McpError::Io`] when the runtime cannot start or the address cannot be
/// bound.
pub fn serve_http<B: Backend>(backend: Arc<B>, options: McpHttpOptions) -> Result<(), McpError> {
    options.validate(options.address)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = TcpListener::bind(options.address).await?;
        serve_http_listener(backend, options, listener).await
    })
}

/// Serves clients over streamable HTTP on a bound listener; the address of
/// the options is ignored.
///
/// # Errors
///
/// Returns [`McpError::InvalidOptions`] when the listener is bound to a
/// non-loopback address without [`McpHttpOptions::allow_remote`] or the
/// token is empty, and [`McpError::Io`] when accepting connections fails.
pub async fn serve_http_listener<B: Backend>(
    backend: Arc<B>,
    options: McpHttpOptions,
    listener: TcpListener,
) -> Result<(), McpError> {
    options.validate(listener.local_addr()?)?;
    let state = Arc::new(HttpState {
        shared: Shared::new(backend),
        options,
        sessions: Mutex::new(HashMap::new()),
    });
    let router = Router::new()
        .route(
            ENDPOINT_PATH,
            post(post_message::<B>)
                .get(open_stream)
                .delete(end_session::<B>),
        )
        .with_state(state);
    axum::serve(listener, router).await?;
    Ok(())
}

/// State of the HTTP server.
struct HttpState<B> {
    shared: Arc<Shared<B>>,
    options: McpHttpOptions,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl<B: Backend> HttpState<B> {
    fn sessions(&self) -> MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Checks the origin and the bearer token of a request.
    fn authorize(&self, headers: &HeaderMap) -> Result<(), Rejection> {
        if let Some(origin) = headers.get(header::ORIGIN) {
            let accepted = origin.to_str().is_ok_and(|origin| {
                is_loopback_origin(origin)
                    || self
                        .options
                        .allowed_origins
                        .iter()
                        .any(|allowed| allowed == origin)
            });
            if !accepted {
                return Err(Rejection::Origin);
            }
        }
        if let Some(expected) = &self.options.bearer_token {
            let given = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.as_bytes().strip_prefix(BEARER_PREFIX.as_bytes()));
            if !given.is_some_and(|given| same_token(given, expected.as_bytes())) {
                return Err(Rejection::Token);
            }
        }
        Ok(())
    }

    /// Returns the session named by the request headers.
    fn session(&self, headers: &HeaderMap) -> Result<Arc<Session>, Rejection> {
        let id = headers
            .get(SESSION_HEADER)
            .ok_or(Rejection::MissingSession)?;
        id.to_str()
            .ok()
            .and_then(|id| self.sessions().get(id).cloned())
            .ok_or(Rejection::UnknownSession)
    }

    /// Opens a session, sends it the `initialize` request and keeps the
    /// session when the initialization succeeds.
    async fn initialize(self: &Arc<Self>, message: Value, event_stream: bool) -> Response {
        let Some(id) = message.get("id").cloned() else {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                INVALID_REQUEST,
                "initialize must be a request with an id",
            );
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let (http_side, server_side) = tokio::io::duplex(PIPE_CAPACITY);
        let (reader, writer) = tokio::io::split(http_side);
        let session = Arc::new(Session::new(writer));
        self.sessions()
            .insert(session_id.clone(), Arc::clone(&session));

        let server = NurtServer::new(Arc::clone(&self.shared), Some(session_id.clone()));
        tokio::spawn(async move {
            match server.serve(server_side).await {
                Ok(running) => {
                    if let Err(error) = running.waiting().await {
                        tracing::debug!(%error, "session task failed");
                    }
                }
                Err(error) => tracing::debug!(%error, "session not initialized"),
            }
        });
        tokio::spawn(read_server(
            Arc::clone(&session),
            reader,
            Arc::downgrade(self),
            session_id.clone(),
        ));

        let outcome = exchange(&session, &message, id).await;
        let initialized = matches!(
            &outcome,
            Some(Outcome::Response { message, .. }) if message.get("result").is_some()
        );
        let mut response = respond(outcome, event_stream);
        if initialized {
            if let Ok(value) = HeaderValue::from_str(&session_id) {
                response.headers_mut().insert(SESSION_HEADER, value);
            }
        } else {
            self.end(&session_id).await;
        }
        response
    }

    /// Removes a session and closes the input of its protocol handler.
    async fn end(&self, session_id: &str) -> bool {
        let session = self.sessions().remove(session_id);
        match session {
            Some(session) => {
                session.close().await;
                true
            }
            None => false,
        }
    }
}

/// Reason to refuse a request before it reaches a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    /// The `Origin` header names an origin that is not allowed.
    Origin,
    /// The bearer token is missing or wrong.
    Token,
    /// The request has no `Mcp-Session-Id` header.
    MissingSession,
    /// The session is unknown or has ended.
    UnknownSession,
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            Self::Origin => rpc_error(
                StatusCode::FORBIDDEN,
                INVALID_REQUEST,
                "origin not allowed; add it to the allowed origins of the server",
            ),
            Self::Token => {
                let mut response = rpc_error(
                    StatusCode::UNAUTHORIZED,
                    INVALID_REQUEST,
                    "missing or wrong bearer token; send `Authorization: Bearer <token>`",
                );
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                response
            }
            Self::MissingSession => rpc_error(
                StatusCode::BAD_REQUEST,
                INVALID_REQUEST,
                "missing Mcp-Session-Id header; send initialize first",
            ),
            Self::UnknownSession => session_ended(),
        }
    }
}

/// Result of a request forwarded to a session.
enum Outcome {
    /// The response with the notifications sent before it.
    Response {
        notifications: Vec<Value>,
        message: Value,
    },
    /// The client cancelled the request; no response follows.
    Cancelled { notifications: Vec<Value> },
}

/// A request waiting for its response.
struct Pending {
    progress_token: Option<String>,
    notifications: Vec<Value>,
    reply: oneshot::Sender<Outcome>,
}

/// One session: the input of its protocol handler and its pending requests.
struct Session {
    writer: tokio::sync::Mutex<Option<WriteHalf<DuplexStream>>>,
    pending: Mutex<HashMap<String, Pending>>,
}

impl Session {
    fn new(writer: WriteHalf<DuplexStream>) -> Self {
        Self {
            writer: tokio::sync::Mutex::new(Some(writer)),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes one message to the protocol handler; `false` when the session
    /// has ended.
    async fn send(&self, message: &Value) -> bool {
        let mut line = message.to_string().into_bytes();
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        let Some(stream) = writer.as_mut() else {
            return false;
        };
        stream.write_all(&line).await.is_ok() && stream.flush().await.is_ok()
    }

    /// Closes the input of the protocol handler, which then ends.
    async fn close(&self) {
        if let Some(mut writer) = self.writer.lock().await.take()
            && let Err(error) = writer.shutdown().await
        {
            tracing::debug!(%error, "session input not closed cleanly");
        }
    }

    fn register(&self, key: String, progress_token: Option<String>) -> oneshot::Receiver<Outcome> {
        let (reply, receiver) = oneshot::channel();
        self.pending().insert(
            key,
            Pending {
                progress_token,
                notifications: Vec::new(),
                reply,
            },
        );
        receiver
    }

    fn cancel(&self, request_id: &Value) {
        if let Some(pending) = self.pending().remove(&request_id.to_string()) {
            let _ignored = pending.reply.send(Outcome::Cancelled {
                notifications: pending.notifications,
            });
        }
    }

    /// Routes one message of the protocol handler to the request it belongs
    /// to. A response goes to the request with its identifier; an error
    /// without an identifier, sent for a message that could not be read,
    /// goes to the only pending request. A notification goes to the request
    /// with its progress token, or to the only pending request; otherwise it
    /// is dropped, because the server keeps no stream outside requests.
    fn dispatch(&self, message: Value) {
        let mut pending = self.pending();
        let is_response = message.get("method").is_none()
            && (message.get("result").is_some() || message.get("error").is_some());
        if is_response {
            let id = message.get("id").cloned().unwrap_or_default();
            let key = if id.is_null() && pending.len() == 1 {
                pending.keys().next().cloned().unwrap_or_default()
            } else {
                id.to_string()
            };
            if let Some(entry) = pending.remove(&key) {
                let _ignored = entry.reply.send(Outcome::Response {
                    notifications: entry.notifications,
                    message,
                });
            }
            return;
        }
        let token = message
            .pointer("/params/progressToken")
            .map(Value::to_string);
        let target = match token {
            Some(token) => pending
                .values_mut()
                .find(|entry| entry.progress_token.as_deref() == Some(token.as_str())),
            None if pending.len() == 1 => pending.values_mut().next(),
            None => None,
        };
        match target {
            Some(entry) => entry.notifications.push(message),
            None => tracing::debug!("server message without a request stream dropped"),
        }
    }

    /// Ends every pending request without a response.
    fn fail_pending(&self) {
        self.pending().clear();
    }
}

/// Reads the messages of a protocol handler until it ends, then removes the
/// session.
async fn read_server<B: Backend>(
    session: Arc<Session>,
    reader: ReadHalf<DuplexStream>,
    state: Weak<HttpState<B>>,
    session_id: String,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match serde_json::from_str(&line) {
            Ok(message) => session.dispatch(message),
            Err(error) => tracing::debug!(%error, "invalid message of the protocol handler"),
        }
    }
    session.fail_pending();
    if let Some(state) = state.upgrade() {
        state.sessions().remove(&session_id);
    }
}

/// Sends a request to a session and waits for its outcome; `None` when the
/// session ends first.
async fn exchange(session: &Session, message: &Value, id: Value) -> Option<Outcome> {
    let key = id.to_string();
    let token = message
        .pointer("/params/_meta/progressToken")
        .map(Value::to_string);
    let receiver = session.register(key.clone(), token);
    if !session.send(message).await {
        session.pending().remove(&key);
        return None;
    }
    receiver.await.ok()
}

async fn post_message<B: Backend>(
    State(state): State<Arc<HttpState<B>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(response) = state.authorize(&headers) {
        return response.into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(error) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                PARSE_ERROR,
                &format!("body is not valid JSON: {error}"),
            );
        }
    };
    if !message.is_object() {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            INVALID_REQUEST,
            "body must be one JSON-RPC message object; batches are not supported",
        );
    }
    if let Some(version) = headers.get(PROTOCOL_HEADER) {
        let supported = ProtocolVersion::known_up_to(&MAX_PROTOCOL_VERSION);
        if !supported
            .iter()
            .any(|known| version.as_bytes() == known.as_str().as_bytes())
        {
            let names: Vec<&str> = supported.iter().map(ProtocolVersion::as_str).collect();
            return rpc_error(
                StatusCode::BAD_REQUEST,
                INVALID_REQUEST,
                &format!(
                    "unsupported MCP-Protocol-Version header; supported: {}",
                    names.join(", ")
                ),
            );
        }
    }
    let event_stream = accepts_event_stream(&headers);
    let method = message.get("method").and_then(Value::as_str);
    if method == Some("initialize") {
        return state.initialize(message, event_stream).await;
    }
    let session = match state.session(&headers) {
        Ok(session) => session,
        Err(rejection) => return rejection.into_response(),
    };
    match (method.is_some(), message.get("id").cloned()) {
        (true, Some(id)) => respond(exchange(&session, &message, id).await, event_stream),
        _ => {
            if !session.send(&message).await {
                return session_ended();
            }
            if method == Some("notifications/cancelled")
                && let Some(request_id) = message.pointer("/params/requestId")
            {
                session.cancel(request_id);
            }
            StatusCode::ACCEPTED.into_response()
        }
    }
}

async fn open_stream() -> Response {
    let mut response = rpc_error(
        StatusCode::METHOD_NOT_ALLOWED,
        INVALID_REQUEST,
        "the server opens no stream outside requests; send requests by POST",
    );
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
    response
}

async fn end_session<B: Backend>(
    State(state): State<Arc<HttpState<B>>>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = state.authorize(&headers) {
        return response.into_response();
    }
    let session_id = headers
        .get(SESSION_HEADER)
        .and_then(|id| id.to_str().ok())
        .map(str::to_owned);
    let Some(session_id) = session_id else {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            INVALID_REQUEST,
            "missing Mcp-Session-Id header",
        );
    };
    if state.end(&session_id).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        session_ended()
    }
}

fn respond(outcome: Option<Outcome>, event_stream: bool) -> Response {
    match outcome {
        None => session_ended(),
        Some(Outcome::Response {
            notifications,
            message,
        }) => {
            if event_stream {
                let mut messages = notifications;
                messages.push(message);
                sse(&messages)
            } else {
                json_body(StatusCode::OK, &message)
            }
        }
        Some(Outcome::Cancelled { notifications }) => {
            if event_stream {
                sse(&notifications)
            } else {
                StatusCode::ACCEPTED.into_response()
            }
        }
    }
}

/// Event stream with one `message` event per JSON-RPC message.
fn sse(messages: &[Value]) -> Response {
    let body: String = messages
        .iter()
        .map(|message| format!("event: message\ndata: {message}\n\n"))
        .collect();
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(EVENT_STREAM));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn json_body(status: StatusCode, message: &Value) -> Response {
    let mut response = Response::new(Body::from(message.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON_MEDIA));
    response
}

fn rpc_error(status: StatusCode, code: i32, message: &str) -> Response {
    json_body(
        status,
        &json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": code, "message": message }
        }),
    )
}

fn session_ended() -> Response {
    rpc_error(
        StatusCode::NOT_FOUND,
        INVALID_REQUEST,
        "session ended; send initialize to open a new one",
    )
}

fn accepts_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.contains(EVENT_STREAM))
}

/// Whether an origin names a loopback host: `localhost`, `127.0.0.0/8` or
/// `[::1]`.
fn is_loopback_origin(origin: &str) -> bool {
    let Some((_, authority)) = origin.split_once("://") else {
        return false;
    };
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => authority.split([':', '/']).next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Compares tokens in time that does not depend on the position of the
/// first difference.
fn same_token(given: &[u8], expected: &[u8]) -> bool {
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::http::{HeaderMap, HeaderValue, header};

    use super::{McpHttpOptions, accepts_event_stream, is_loopback_origin, same_token};

    #[test]
    fn loopback_origins_are_recognized() {
        for origin in [
            "http://localhost:3000",
            "http://LOCALHOST",
            "http://127.0.0.1:8080",
            "http://127.1.2.3",
            "http://[::1]:9000",
        ] {
            assert!(is_loopback_origin(origin), "{origin}");
        }
        for origin in [
            "http://example.com",
            "http://localhost.example.com",
            "http://10.0.0.1",
            "null",
            "http://[::2]",
        ] {
            assert!(!is_loopback_origin(origin), "{origin}");
        }
    }

    #[test]
    fn tokens_match_only_when_equal() {
        assert!(same_token(b"secret", b"secret"));
        assert!(!same_token(b"secret", b"secreT"));
        assert!(!same_token(b"secret", b"secret2"));
    }

    #[test]
    fn event_stream_is_detected_in_accept() {
        let mut headers = HeaderMap::new();
        assert!(!accepts_event_stream(&headers));
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        assert!(accepts_event_stream(&headers));
    }

    #[test]
    fn remote_address_requires_explicit_permission() {
        let remote: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let options = McpHttpOptions::new(0);
        assert!(options.validate(remote).is_err());
        assert!(options.clone().allow_remote(true).validate(remote).is_ok());
        assert!(options.validate(options.address).is_ok());
        assert!(
            McpHttpOptions::new(0)
                .with_bearer_token(" ")
                .validate("127.0.0.1:1".parse().unwrap())
                .is_err()
        );
    }

    #[test]
    fn debug_output_hides_the_token() {
        let options = McpHttpOptions::new(8080).with_bearer_token("secret");
        let text = format!("{options:?}");
        assert!(!text.contains("secret"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn token_is_read_from_a_file_without_white_space() {
        let directory = std::env::temp_dir().join(format!("nurt-mcp-token-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("token");
        std::fs::write(&path, "  abc123\n").unwrap();
        let options = McpHttpOptions::new(0)
            .with_bearer_token_file(&path)
            .unwrap();
        assert_eq!(options.bearer_token.as_deref(), Some("abc123"));
        std::fs::write(&path, "\n").unwrap();
        assert!(
            McpHttpOptions::new(0)
                .with_bearer_token_file(&path)
                .is_err()
        );
        assert!(
            McpHttpOptions::new(0)
                .with_bearer_token_file(directory.join("missing"))
                .is_err()
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn missing_environment_token_is_rejected() {
        let result =
            McpHttpOptions::new(0).with_bearer_token_env("NURT_MCP_TEST_TOKEN_THAT_IS_NEVER_SET");
        assert!(result.is_err());
    }
}
