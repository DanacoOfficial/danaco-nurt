// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connection of the server: reading, dispatch, execution and writing.
//!
//! The calling thread reads and dispatches messages. Each engine method
//! runs on its own scoped thread; one writer thread owns the output, so
//! replies and event notifications are written whole and in the order they
//! were produced. `initialize`, `cancel` and `shutdown` are answered by the
//! reading thread, so a cancellation is never queued behind the requests it
//! cancels.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
#[cfg(feature = "stub")]
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{Scope, ScopedJoinHandle};

use nurt_contracts::ast::AST_VERSION;
use nurt_contracts::error::{AbiStatus, Error, ErrorCode};
use nurt_contracts::report::REPORT_VERSION;
use nurt_contracts::stage::{CancelReason, CancelToken};
use nurt_core::Engine;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::backend::{Backend, CallContext, EngineBackend, Events};
use super::call::{self, CANCEL, Call, INITIALIZE, METHODS, SHUTDOWN};
use super::framing::{self, Frame, Framing};
use super::protocol::{self, Incoming, Message, PROTOCOL, RpcError};

/// Default limit of requests executed at the same time.
pub const DEFAULT_MAX_INFLIGHT: usize = 16;

/// Default limit of the size of one received message, in bytes (16 MiB).
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Prefix of the protocol name before its version.
const PROTOCOL_FAMILY: &str = "nurt-rpc/";

/// Configuration of one connection of the server.
///
/// A configuration names what answers the requests (an engine, or with the
/// feature `stub` the recorded cases of the library stub), the framing of
/// messages and the limits reported by `initialize`.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_cli::serve::{Framing, ServeOptions};
/// use nurt_core::config::ConfigSources;
/// use nurt_core::{Engine, EngineConfig};
/// use nurt_testkit::clock::VirtualClock;
/// use nurt_testkit::host::TestHost;
///
/// let engine = Engine::open(
///     EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
///         .with_sources(ConfigSources::isolated())
///         .without_state_dir(),
/// )?;
/// let options = ServeOptions::engine(engine)
///     .with_framing(Framing::ContentLength)
///     .with_max_inflight(4);
/// assert_eq!(options.framing(), Framing::ContentLength);
/// assert_eq!(options.max_inflight(), 4);
/// # Ok::<(), nurt_core::EngineError>(())
/// ```
pub struct ServeOptions {
    backend: Box<dyn Backend>,
    framing: Framing,
    max_inflight: usize,
    max_message_bytes: usize,
}

impl ServeOptions {
    /// Configuration that executes requests through `engine`, with the
    /// framing [`Framing::Ndjson`] and the default limits.
    #[must_use]
    pub fn engine(engine: Engine) -> Self {
        Self::with_backend(Box::new(EngineBackend::new(engine)))
    }

    /// Configuration that answers requests from the recorded cases in
    /// `cases`, the case directory format of `nurt_ffi::stub`.
    ///
    /// # Errors
    ///
    /// Returns the error of the first case file that cannot be read or is
    /// not a valid case.
    #[cfg(feature = "stub")]
    pub fn stub(cases: &Path) -> Result<Self, nurt_ffi::stub::CatalogError> {
        super::stub::StubBackend::load(cases).map(|backend| Self::with_backend(Box::new(backend)))
    }

    fn with_backend(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            framing: Framing::default(),
            max_inflight: DEFAULT_MAX_INFLIGHT,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }

    /// Sets the framing of messages on both streams.
    #[must_use]
    pub fn with_framing(mut self, framing: Framing) -> Self {
        self.framing = framing;
        self
    }

    /// Sets the number of requests executed at the same time; at least 1.
    /// A request above the limit is rejected with `-32600`.
    #[must_use]
    pub fn with_max_inflight(mut self, max_inflight: usize) -> Self {
        self.max_inflight = max_inflight.max(1);
        self
    }

    /// Sets the size limit of one received message in bytes; at least 1.
    /// A longer message is skipped and rejected with `-32600`.
    #[must_use]
    pub fn with_max_message_bytes(mut self, max_message_bytes: usize) -> Self {
        self.max_message_bytes = max_message_bytes.max(1);
        self
    }

    /// Framing of messages.
    #[must_use]
    pub fn framing(&self) -> Framing {
        self.framing
    }

    /// Number of requests executed at the same time.
    #[must_use]
    pub fn max_inflight(&self) -> usize {
        self.max_inflight
    }

    /// Size limit of one received message in bytes.
    #[must_use]
    pub fn max_message_bytes(&self) -> usize {
        self.max_message_bytes
    }
}

impl fmt::Debug for ServeOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServeOptions")
            .field("framing", &self.framing)
            .field("max_inflight", &self.max_inflight)
            .field("max_message_bytes", &self.max_message_bytes)
            .finish_non_exhaustive()
    }
}

/// Serves one connection of `nurt-rpc/1` on `input` and `output`.
///
/// The function returns after `shutdown` or at the end of the input, once
/// every request in flight has been answered. Messages after `shutdown` are
/// not read.
///
/// The exit code is [`ExitCode::SUCCESS`] after `shutdown` or the end of the
/// input, and [`ExitCode::FAILURE`] when the input cannot be read, its
/// framing is broken, or the output cannot be written.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
/// use std::sync::Arc;
///
/// use nurt_cli::serve::{self, ServeOptions};
/// use nurt_core::config::ConfigSources;
/// use nurt_core::{Engine, EngineConfig};
/// use nurt_testkit::clock::VirtualClock;
/// use nurt_testkit::host::TestHost;
/// use serde_json::Value;
///
/// let engine = Engine::open(
///     EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
///         .with_sources(ConfigSources::isolated())
///         .without_state_dir(),
/// )?;
/// let input = Cursor::new(concat!(
///     r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":"nurt-rpc/1"}}"#,
///     "\n",
///     r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
///     "\n",
/// ));
/// let mut output = Vec::new();
/// serve::run(ServeOptions::engine(engine), input, &mut output);
/// let replies: Vec<Value> = output
///     .split(|byte| *byte == b'\n')
///     .filter(|line| !line.is_empty())
///     .map(serde_json::from_slice)
///     .collect::<Result<_, _>>()?;
/// assert_eq!(replies[0]["result"]["protocol"], "nurt-rpc/1");
/// assert_eq!(replies[1], serde_json::json!({"jsonrpc": "2.0", "id": 2, "result": null}));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn run(options: ServeOptions, input: impl BufRead, output: impl Write + Send) -> ExitCode {
    let (sender, receiver) = mpsc::channel();
    let shared = Mutex::new(Inflight::default());
    let framing = options.framing;
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || write_messages(output, framing, receiver));
        let mut connection = Connection {
            options: &options,
            scope,
            sender,
            shared: &shared,
            workers: Vec::new(),
            initialized: None,
        };
        let end = connection.serve(input);
        connection.finish(&end);
        let written = writer.join().is_ok_and(|written| written.is_ok());
        if written && end.is_clean() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    })
}

/// Writes every message received from the connection until all senders are
/// gone.
fn write_messages(
    mut output: impl Write,
    framing: Framing,
    receiver: Receiver<Value>,
) -> io::Result<()> {
    for message in receiver {
        let body = serde_json::to_vec(&message).map_err(io::Error::other)?;
        framing::write_frame(&mut output, framing, &body)?;
    }
    Ok(())
}

/// Why the connection stopped reading.
enum End {
    /// The input ended.
    Input,
    /// `shutdown` was received; the identifier of the request, `None` for a
    /// notification.
    Shutdown(Option<Value>),
    /// The input could not be read.
    InputFailed,
    /// The framing of the input is broken.
    Framing,
}

impl End {
    fn is_clean(&self) -> bool {
        matches!(self, Self::Input | Self::Shutdown(_))
    }
}

/// Requests in flight, shared with their threads.
#[derive(Default)]
struct Inflight {
    /// Cancellation of each request in flight, by the JSON text of its `id`.
    tokens: HashMap<String, CancelToken>,
    /// Number of requests and notifications being executed.
    running: usize,
}

/// State negotiated by `initialize`.
struct Session {
    events: bool,
    session: Option<String>,
}

/// Parameters of `initialize`. Members not listed, such as `client` and
/// `language`, are accepted and ignored.
#[derive(Debug, Deserialize)]
struct InitializeParams {
    protocol: String,
    #[serde(default)]
    abi: Option<u32>,
    #[serde(default)]
    ast: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    config_overrides: Option<Map<String, Value>>,
    #[serde(default)]
    capabilities: Capabilities,
}

/// Capabilities of the client given in `initialize`.
#[derive(Debug, Default, Deserialize)]
struct Capabilities {
    #[serde(default)]
    events: bool,
}

/// Parameters of `cancel`: exactly one of the two members.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelParams {
    #[serde(default)]
    request_id: Option<Value>,
    #[serde(default)]
    job_id: Option<String>,
}

struct Connection<'scope, 'env> {
    options: &'env ServeOptions,
    scope: &'scope Scope<'scope, 'env>,
    sender: Sender<Value>,
    shared: &'env Mutex<Inflight>,
    workers: Vec<ScopedJoinHandle<'scope, ()>>,
    initialized: Option<Session>,
}

impl<'scope, 'env> Connection<'scope, 'env> {
    fn serve(&mut self, mut input: impl BufRead) -> End {
        let limit = self.options.max_message_bytes;
        loop {
            let frame = match framing::read_frame(&mut input, self.options.framing, limit) {
                Ok(frame) => frame,
                Err(_) => return End::InputFailed,
            };
            match frame {
                Frame::End => return End::Input,
                Frame::TooLarge => self.send(protocol::error_message(
                    Value::Null,
                    &RpcError::invalid_request(format!(
                        "the message is longer than {limit} bytes; send a shorter message"
                    )),
                )),
                Frame::Malformed(reason) => {
                    self.send(protocol::error_message(
                        Value::Null,
                        &RpcError::parse(format!("the framing of the input is broken: {reason}")),
                    ));
                    return End::Framing;
                }
                Frame::Message(bytes) => match protocol::parse_message(&bytes) {
                    Incoming::Message(message) => {
                        if let Some(end) = self.dispatch(message) {
                            return end;
                        }
                    }
                    Incoming::Ignored => {}
                    Incoming::Rejected(reply) => self.send(reply),
                },
            }
        }
    }

    /// Handles one message; `Some` when the connection stops reading.
    fn dispatch(&mut self, message: Message) -> Option<End> {
        let Message { id, method, params } = message;
        match method.as_str() {
            SHUTDOWN => return Some(End::Shutdown(id)),
            INITIALIZE => {
                let outcome = self.initialize(params);
                self.reply(id, outcome);
            }
            name if !METHODS.contains(&name) => {
                self.reply(id, Err(RpcError::method_not_found(name, METHODS)));
            }
            _ if self.initialized.is_none() => self.reply(
                id,
                Err(RpcError::invalid_request(
                    "the connection is not initialized; send `initialize` first",
                )),
            ),
            CANCEL => {
                let outcome = self.cancel(params);
                self.reply(id, outcome);
            }
            name => match Call::parse(name, params) {
                Ok(call) => self.start(id, call),
                Err(error) => self.reply(id, Err(error)),
            },
        }
        None
    }

    fn initialize(&mut self, params: Option<Value>) -> Result<Value, RpcError> {
        if self.initialized.is_some() {
            return Err(RpcError::invalid_request(
                "the connection is already initialized",
            ));
        }
        let params: InitializeParams = call::object(INITIALIZE, params)?;
        let summary = self.options.backend.summary();
        if major(params.protocol.strip_prefix(PROTOCOL_FAMILY))
            != major(PROTOCOL.strip_prefix(PROTOCOL_FAMILY))
        {
            return Err(RpcError::status(
                AbiStatus::Abi,
                format!(
                    "unsupported protocol `{}`; this server speaks {PROTOCOL}",
                    params.protocol
                ),
            )
            .with_data("supported", PROTOCOL.into()));
        }
        if let (Some(requested), Some(provided)) = (params.abi, summary.abi)
            && abi_major(requested) != abi_major(provided)
        {
            return Err(RpcError::status(
                AbiStatus::Abi,
                format!(
                    "unsupported C interface version {requested:#x}; this server provides {provided:#x}"
                ),
            )
            .with_data("supported", provided.into()));
        }
        if let Some(ast) = &params.ast
            && major(Some(ast)) != major(Some(AST_VERSION))
        {
            return Err(RpcError::status(
                AbiStatus::Abi,
                format!(
                    "unsupported syntax tree version `{ast}`; this server speaks {AST_VERSION}"
                ),
            )
            .with_data("supported", AST_VERSION.into()));
        }
        if params
            .config_overrides
            .as_ref()
            .is_some_and(|overrides| !overrides.is_empty())
        {
            return Err(RpcError::invalid_params(
                "`config_overrides` is not supported by this server; configure the engine before it starts",
            ));
        }
        self.initialized = Some(Session {
            events: params.capabilities.events,
            session: params.session,
        });
        Ok(json!({
            "protocol": PROTOCOL,
            "version": summary.version,
            "abi": summary.abi,
            "language": Value::Null,
            "ast": AST_VERSION,
            "report": REPORT_VERSION,
            "catalog_rev": Value::Null,
            "features": summary.features,
            "groups": summary.groups,
            "limits": {
                "max_inflight": self.options.max_inflight,
                "max_message_bytes": self.options.max_message_bytes,
            },
        }))
    }

    fn cancel(&self, params: Option<Value>) -> Result<Value, RpcError> {
        let params: CancelParams = call::object(CANCEL, params)?;
        match (params.request_id, params.job_id) {
            (Some(request), None) => {
                let token = self.lock().tokens.get(&id_key(&request)).cloned();
                if let Some(token) = &token {
                    token.cancel(CancelReason::Requested);
                }
                Ok(json!({ "cancelled": token.is_some() }))
            }
            (None, Some(job)) => call::failure(
                CANCEL,
                Error::new(
                    ErrorCode::Unsupported,
                    "cancelling background jobs is not available in this server; cancel the request instead",
                )
                .with_found(job),
            ),
            _ => Err(RpcError::invalid_params(
                "`cancel` needs exactly one of `request_id` and `job_id`",
            )),
        }
    }

    /// Starts an engine method on its own thread.
    fn start(&mut self, id: Option<Value>, call: Call) {
        let key = id.as_ref().map(id_key);
        let token = CancelToken::new();
        {
            let mut shared = self.lock();
            if shared.running >= self.options.max_inflight {
                drop(shared);
                let limit = self.options.max_inflight;
                return self.reply(
                    id,
                    Err(RpcError::invalid_request(format!(
                        "too many requests in flight; wait for a reply, the limit is {limit}"
                    ))),
                );
            }
            if let Some(key) = &key {
                if shared.tokens.contains_key(key) {
                    drop(shared);
                    return self.reply(
                        id,
                        Err(RpcError::invalid_request(format!(
                            "a request with the id {key} is in flight; use another id"
                        ))),
                    );
                }
                shared.tokens.insert(key.clone(), token.clone());
            }
            shared.running += 1;
        }
        let (events, session) = match &self.initialized {
            Some(state) => (state.events, state.session.clone()),
            None => (false, None),
        };
        let events = Arc::new(Events::new(
            events.then(|| self.sender.clone()),
            id.as_ref(),
        ));
        let backend: &'env dyn Backend = &*self.options.backend;
        let shared: &'env Mutex<Inflight> = self.shared;
        let sender = self.sender.clone();
        self.workers.retain(|worker| !worker.is_finished());
        self.workers.push(self.scope.spawn(move || {
            let context = CallContext {
                cancel: &token,
                events: &events,
                session: session.as_deref(),
            };
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| backend.call(call, &context)))
                .unwrap_or_else(|payload| Err(panicked(payload.as_ref())));
            {
                let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(key) = &key {
                    shared.tokens.remove(key);
                }
                shared.running = shared.running.saturating_sub(1);
            }
            if let Some(id) = id {
                // The receiver is gone only when the output of the server failed.
                let _ = sender.send(protocol::reply(id, outcome));
            }
        }));
    }

    /// Waits for the requests in flight and answers `shutdown`.
    fn finish(mut self, end: &End) {
        for worker in self.workers.drain(..) {
            // The thread of a request catches the panics of the engine.
            let _ = worker.join();
        }
        if let End::Shutdown(Some(id)) = end {
            self.send(protocol::result_message(id.clone(), Value::Null));
        }
    }

    fn reply(&self, id: Option<Value>, outcome: Result<Value, RpcError>) {
        if let Some(id) = id {
            self.send(protocol::reply(id, outcome));
        }
    }

    fn send(&self, message: Value) {
        // The receiver is gone only when the output of the server failed.
        let _ = self.sender.send(message);
    }

    fn lock(&self) -> MutexGuard<'env, Inflight> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Key of a request identifier: its JSON text, so `1` and `"1"` differ.
fn id_key(id: &Value) -> String {
    id.to_string()
}

/// Major version of a version text such as `1` or `1.2`.
fn major(version: Option<&str>) -> Option<&str> {
    version.map(|version| version.split('.').next().unwrap_or(version))
}

/// Major version of a C interface version `(major << 16) | minor`.
const fn abi_major(version: u32) -> u32 {
    version >> 16
}

/// Error reply of a request whose execution panicked.
fn panicked(payload: &(dyn Any + Send)) -> RpcError {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    RpcError::status(
        AbiStatus::Internal,
        "the server failed while answering the request",
    )
    .with_data("panic", detail.into())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn versions_are_compared_by_their_major_part() {
        assert_eq!(major(Some("1.4")), Some("1"));
        assert_eq!(major(Some("2")), Some("2"));
        assert_eq!(major(None), None);
        assert_eq!(major(PROTOCOL.strip_prefix(PROTOCOL_FAMILY)), Some("1"));
        assert_eq!(abi_major(0x0001_0003), 1);
    }

    #[test]
    fn major_versions_ignore_the_minor_and_patch_parts() {
        assert_eq!(major(Some("1.2.3")), Some("1"));
        assert_eq!(major(Some("")), Some(""));
        assert_eq!(major(Some(".5")), Some(""));
        assert_eq!(major(Some("10.0")), Some("10"));
        assert_ne!(major(Some("1")), major(Some("10")));
        assert_eq!(PROTOCOL.strip_prefix(PROTOCOL_FAMILY), Some("1"));
        assert_eq!("lsp/1".strip_prefix(PROTOCOL_FAMILY), None);
    }

    #[test]
    fn interface_versions_are_compared_by_their_upper_half() {
        assert_eq!(abi_major(0), 0);
        assert_eq!(abi_major(0x0000_ffff), 0);
        assert_eq!(abi_major(0x0001_0000), 1);
        assert_eq!(abi_major(u32::MAX), 0xffff);
        assert_ne!(abi_major(0x0001_ffff), abi_major(0x0002_0000));
    }

    #[test]
    fn identifiers_of_every_json_type_have_distinct_keys() {
        let keys = [
            id_key(&json!(1)),
            id_key(&json!(1.5)),
            id_key(&json!("1")),
            id_key(&Value::Null),
            id_key(&json!("null")),
        ];
        for (index, key) in keys.iter().enumerate() {
            assert!(!keys[..index].contains(key), "{key}");
        }
        assert_eq!(id_key(&json!("\u{142}")), "\"\u{142}\"");
    }

    #[test]
    fn a_panic_payload_of_another_type_has_no_detail() {
        let error = panicked(&42_i32);
        assert_eq!(error.code, protocol::application_code(AbiStatus::Internal));
        assert_eq!(error.data["panic"], "");
    }

    #[test]
    fn the_limits_have_the_documented_defaults() {
        assert_eq!(DEFAULT_MAX_INFLIGHT, 16);
        assert_eq!(DEFAULT_MAX_MESSAGE_BYTES, 16 * 1024 * 1024);
        let options = ServeOptions::with_backend(Box::new(NoBackend));
        assert_eq!(options.max_inflight(), DEFAULT_MAX_INFLIGHT);
        assert_eq!(options.max_message_bytes(), DEFAULT_MAX_MESSAGE_BYTES);
        let options = options
            .with_max_inflight(usize::MAX)
            .with_max_message_bytes(usize::MAX);
        assert_eq!(options.max_inflight(), usize::MAX);
        assert_eq!(options.max_message_bytes(), usize::MAX);
        assert_eq!(
            options.with_framing(Framing::ContentLength).framing(),
            Framing::ContentLength
        );
    }

    #[test]
    fn identifiers_keep_their_json_type() {
        assert_ne!(id_key(&json!(1)), id_key(&json!("1")));
        assert_eq!(id_key(&json!("a")), "\"a\"");
    }

    #[test]
    fn panics_carry_their_message() {
        let text = panicked(&"broken");
        assert_eq!(text.data["panic"], "broken");
        assert_eq!(text.data["status"], "NURT_E_INTERNAL");
        let owned = panicked(&String::from("lost"));
        assert_eq!(owned.data["panic"], "lost");
        assert_eq!(panicked(&7_u8).data["panic"], "");
    }

    #[test]
    fn options_keep_their_limits_above_zero() {
        let options = ServeOptions::with_backend(Box::new(NoBackend))
            .with_max_inflight(0)
            .with_max_message_bytes(0);
        assert_eq!(options.max_inflight(), 1);
        assert_eq!(options.max_message_bytes(), 1);
        assert_eq!(options.framing(), Framing::Ndjson);
        assert!(format!("{options:?}").contains("max_inflight: 1"));
    }

    /// Longest time a stub call waits for its release before the test fails.
    const RELEASE_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

    /// Backend whose `run` is slow: it ends only when a `help` has been
    /// answered or when its request is cancelled.
    struct SlowBackend {
        helped: AtomicBool,
    }

    impl SlowBackend {
        fn new() -> Self {
            Self {
                helped: AtomicBool::new(false),
            }
        }
    }

    impl Backend for SlowBackend {
        fn summary(&self) -> super::super::backend::Summary {
            NoBackend.summary()
        }

        fn call(&self, call: Call, context: &CallContext<'_>) -> Result<Value, RpcError> {
            match call {
                Call::Run(_) => {
                    let deadline = std::time::Instant::now() + RELEASE_LIMIT;
                    while !self.helped.load(Ordering::SeqCst) && !context.cancel.is_cancelled() {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "the slow call was never released"
                        );
                        std::thread::yield_now();
                    }
                    if context.cancel.is_cancelled() {
                        Err(RpcError::cancelled(json!({"stopped": true})))
                    } else {
                        Ok(json!({"slow": "done"}))
                    }
                }
                Call::Help(_) => {
                    self.helped.store(true, Ordering::SeqCst);
                    Ok(json!({"text": "help"}))
                }
                _ => Err(RpcError::internal("unexpected call")),
            }
        }
    }

    /// Serves `messages` on `backend` to the end of the input and returns
    /// every message the server wrote.
    fn converse(backend: SlowBackend, messages: &[Value]) -> Vec<Value> {
        let mut input = Vec::new();
        for message in messages {
            input.extend_from_slice(message.to_string().as_bytes());
            input.push(b'\n');
        }
        let mut output = Vec::new();
        let options = ServeOptions::with_backend(Box::new(backend));
        run(options, std::io::Cursor::new(input), &mut output);
        output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    fn call_message(id: u64, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    fn reply_to(messages: &[Value], id: u64) -> &Value {
        messages
            .iter()
            .find(|message| message["id"] == id && message.get("method").is_none())
            .unwrap_or_else(|| panic!("no reply to {id}: {messages:?}"))
    }

    fn initialize_message() -> Value {
        call_message(1, "initialize", json!({"protocol": "nurt-rpc/1"}))
    }

    #[test]
    fn a_slow_run_does_not_hold_back_a_help_request_behind_it() {
        let messages = converse(
            SlowBackend::new(),
            &[
                initialize_message(),
                call_message(2, "run", json!({"nurt": "1.0", "program": "count"})),
                call_message(3, "help", json!({"topic": "find"})),
            ],
        );
        assert_eq!(reply_to(&messages, 3)["result"], json!({"text": "help"}));
        assert_eq!(reply_to(&messages, 2)["result"], json!({"slow": "done"}));
    }

    #[test]
    fn cancelling_a_request_stops_the_slow_call_with_its_cancellation_error() {
        let messages = converse(
            SlowBackend::new(),
            &[
                initialize_message(),
                call_message(2, "run", json!({"nurt": "1.0", "program": "count"})),
                call_message(3, "cancel", json!({"request_id": 2})),
            ],
        );
        assert_eq!(reply_to(&messages, 3)["result"], json!({"cancelled": true}));
        let stopped = &reply_to(&messages, 2)["error"];
        assert_eq!(
            stopped["code"],
            protocol::application_code(AbiStatus::Cancelled)
        );
        assert_eq!(stopped["data"]["response"], json!({"stopped": true}));
    }

    #[test]
    fn cancelling_one_request_leaves_the_others_running() {
        let messages = converse(
            SlowBackend::new(),
            &[
                initialize_message(),
                call_message(2, "run", json!({"nurt": "1.0", "program": "count"})),
                call_message(3, "run", json!({"nurt": "1.0", "program": "count"})),
                call_message(4, "cancel", json!({"request_id": 2})),
                call_message(5, "help", json!({})),
            ],
        );
        assert_eq!(reply_to(&messages, 4)["result"], json!({"cancelled": true}));
        assert!(reply_to(&messages, 2)["error"].is_object());
        assert_eq!(reply_to(&messages, 3)["result"], json!({"slow": "done"}));
        assert_eq!(reply_to(&messages, 5)["result"], json!({"text": "help"}));
    }

    struct NoBackend;

    impl Backend for NoBackend {
        fn summary(&self) -> super::super::backend::Summary {
            super::super::backend::Summary {
                version: String::new(),
                abi: None,
                features: Vec::new(),
                groups: Vec::new(),
            }
        }

        fn call(&self, _call: Call, _context: &CallContext<'_>) -> Result<Value, RpcError> {
            Err(RpcError::internal("no backend"))
        }
    }
}
