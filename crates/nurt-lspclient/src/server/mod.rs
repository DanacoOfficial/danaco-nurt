// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-lspclient::server
// Responsibility:   Connection to one running language server.
// Dependencies:     crossbeam_channel, nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Connection to one running language server.
//!
//! The server runs as a helper process started through
//! [`ProcessHost::spawn_service`] with a private temporary directory that is
//! removed once the process has ended. A reader thread decodes the frames of
//! standard output: responses complete pending requests, requests of the
//! server (`workspace/configuration`, `client/registerCapability`,
//! `window/workDoneProgress/create`, `workspace/workspaceFolders`) are
//! answered at once, and notifications update the progress tracker and the
//! diagnostic store. A second thread keeps the last non-empty line of
//! standard error, which is reported when the server ends.
//!
//! Requests block the calling thread. A request that reaches its time limit
//! or whose cancellation token is cancelled is cancelled on the server side
//! with `$/cancelRequest`.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use nurt_contracts::services::{EnvVar, ProcessHost, ServiceChild, ServiceSpawnSpec};
use nurt_contracts::stage::{CancelReason, CancelToken};
use serde_json::{Value, json};

use crate::diagnostics::DiagnosticStore;
use crate::jsonrpc::{METHOD_NOT_FOUND, Message, RequestId, RpcError};
use crate::progress::ProgressTracker;
use crate::sync::{DocumentSet, FileStamp, path_to_uri};
use crate::transport::{Frame, FrameReader, FrameWriter};

/// Longest single wait between checks of the cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(25);

/// Time a server is given to answer `shutdown`; the protocol sends `exit`
/// only after the answer, so that the server can finish its work.
const SHUTDOWN_REPLY_WAIT: Duration = Duration::from_secs(2);

/// Time a server is given to exit after `exit` before its process tree is
/// stopped.
const EXIT_GRACE: Duration = Duration::from_millis(200);

/// Time the reader waits for the standard error thread after the end of
/// standard output, so the last error line is known.
const STDERR_GRACE: Duration = Duration::from_millis(500);

/// References to the connection held by the reader thread.
const READER_REFERENCES: usize = 1;

/// Longest chunk of standard error read at once, in bytes.
const STDERR_CHUNK: u64 = 64 * 1024;

/// Prefix of the private temporary directory of each server.
const TEMP_PREFIX: &str = "nurt-lsp-";

/// Failure of a language server operation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum LspError {
    /// No server is configured or allowed for the language, or its program
    /// is not installed.
    #[error("{message}")]
    Missing {
        /// Server program; empty when no server is configured.
        server: String,
        /// Cause.
        message: String,
    },
    /// The server could not be started, or a recent start failed and the
    /// retry period has not elapsed.
    #[error("{message}")]
    Unavailable {
        /// Server program.
        server: String,
        /// Cause.
        message: String,
    },
    /// The server process ended.
    #[error("language server {server} ended{}", detail.as_ref().map(|line| format!(": {line}")).unwrap_or_default())]
    Closed {
        /// Server program.
        server: String,
        /// Last line of the standard error of the server, when known.
        detail: Option<String>,
    },
    /// The server did not answer within the time limit.
    #[error("language server {server} did not answer within the time limit")]
    Timeout {
        /// Server program.
        server: String,
    },
    /// The operation was cancelled.
    #[error("the request to the language server was cancelled")]
    Cancelled(CancelReason),
    /// The server answered with an error.
    #[error("language server {server} failed: {error}")]
    Rpc {
        /// Server program.
        server: String,
        /// Error of the response.
        error: Box<RpcError>,
    },
}

/// Program, arguments and project root of a server to start.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerSpec {
    /// Name shown in results: the file name of the program.
    pub name: String,
    /// Program to start.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Project root, also the working directory.
    pub root: PathBuf,
    /// `initializationOptions` of the `initialize` request.
    pub init_options: Option<Value>,
    /// Variables set for the server on top of the inherited environment.
    pub env: Vec<EnvVar>,
}

/// Capabilities announced by the client in `initialize`.
#[must_use]
pub fn client_capabilities() -> Value {
    json!({
        "textDocument": {
            "synchronization": {"didSave": false},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "definition": {"linkSupport": true},
            "typeDefinition": {"linkSupport": true},
            "implementation": {"linkSupport": true},
            "references": {},
            "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
            "publishDiagnostics": {"versionSupport": true},
            "diagnostic": {"dynamicRegistration": true},
            "rename": {"prepareSupport": false},
            "callHierarchy": {},
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": {"valueSet": ["quickfix", "refactor", "source"]}
                }
            }
        },
        "workspace": {
            "configuration": true,
            "workspaceFolders": true,
            "symbol": {},
            "workspaceEdit": {
                "documentChanges": true,
                "resourceOperations": ["create", "rename", "delete"]
            }
        },
        "window": {"workDoneProgress": true},
        "experimental": {"serverStatusNotification": true}
    })
}

type Pending = Sender<Result<Value, RpcError>>;

struct Shared {
    name: String,
    root: PathBuf,
    outbox: Mutex<Option<Sender<Value>>>,
    pending: Mutex<HashMap<i64, Pending>>,
    next_id: AtomicI64,
    capabilities: Mutex<Value>,
    registrations: Mutex<HashSet<String>>,
    progress: ProgressTracker,
    diagnostics: DiagnosticStore,
    documents: Mutex<DocumentSet>,
    closed: AtomicBool,
    close_detail: Mutex<Option<String>>,
    stderr_tail: Arc<Mutex<Option<String>>>,
    child: Mutex<Box<dyn ServiceChild>>,
    temp_dir: Mutex<Option<tempfile::TempDir>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Running language server.
///
/// Cloning shares the connection. The server stops with
/// [`LanguageServer::shutdown`] or when its process ends.
#[derive(Clone)]
pub struct LanguageServer {
    shared: Arc<Shared>,
    pid: u32,
}

impl std::fmt::Debug for LanguageServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanguageServer")
            .field("name", &self.shared.name)
            .field("root", &self.shared.root)
            .field("pid", &self.pid)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl LanguageServer {
    /// Starts the server process and its reader threads; the server is not
    /// initialized yet.
    ///
    /// # Errors
    ///
    /// Fails with [`LspError::Unavailable`] when the process cannot be
    /// started or has no piped standard streams.
    pub fn spawn(host: &dyn ProcessHost, spec: &ServerSpec) -> Result<Self, LspError> {
        let unavailable = |message: String| LspError::Unavailable {
            server: spec.name.clone(),
            message,
        };
        let temp_dir = tempfile::Builder::new().prefix(TEMP_PREFIX).tempdir().ok();
        let spawn = ServiceSpawnSpec {
            program: spec.program.clone(),
            args: spec.args.clone(),
            cwd: spec.root.clone(),
            env: spec.env.clone(),
            inherit_env: true,
            temp_dir: temp_dir.as_ref().map(|dir| dir.path().to_path_buf()),
        };
        let mut child = host
            .spawn_service(&spawn)
            .map_err(|error| unavailable(error.data().message.clone()))?;
        let streams = (child.take_stdin(), child.take_stdout(), child.take_stderr());
        let (Some(stdin), Some(stdout), stderr) = streams else {
            let _ = child.kill_tree();
            return Err(unavailable(format!(
                "language server {} has no piped standard streams",
                spec.name
            )));
        };
        let pid = child.pid();
        let (outbox, queue) = unbounded();
        let shared = Arc::new(Shared {
            name: spec.name.clone(),
            root: spec.root.clone(),
            outbox: Mutex::new(Some(outbox)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            capabilities: Mutex::new(Value::Null),
            registrations: Mutex::new(HashSet::new()),
            progress: ProgressTracker::new(),
            diagnostics: DiagnosticStore::new(),
            documents: Mutex::new(DocumentSet::new()),
            closed: AtomicBool::new(false),
            close_detail: Mutex::new(None),
            stderr_tail: Arc::new(Mutex::new(None)),
            child: Mutex::new(child),
            temp_dir: Mutex::new(temp_dir),
        });
        let started = thread::Builder::new()
            .name(format!("lsp-{}-stdin", spec.name))
            .spawn(move || write_loop(stdin, &queue));
        let stderr_done = stderr.map(|stream| spawn_stderr_reader(stream, &shared));
        let reader_shared = Arc::clone(&shared);
        let started = started.and_then(|_| {
            thread::Builder::new()
                .name(format!("lsp-{}-stdout", spec.name))
                .spawn(move || read_loop(&reader_shared, stdout, stderr_done))
        });
        if let Err(error) = started {
            finish(&shared);
            return Err(unavailable(format!(
                "cannot start the threads of language server {}: {error}",
                spec.name
            )));
        }
        Ok(Self { shared, pid })
    }

    /// Sends `initialize` and `initialized`.
    ///
    /// # Errors
    ///
    /// Fails as [`LanguageServer::request`].
    pub fn initialize(
        &self,
        init_options: Option<&Value>,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> Result<(), LspError> {
        let root = &self.shared.root;
        let root_uri = path_to_uri(root);
        let mut params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "rootPath": root.to_string_lossy(),
            "workspaceFolders": [{"uri": root_uri, "name": root.to_string_lossy()}],
            "capabilities": client_capabilities(),
        });
        if let (Some(options), Some(object)) = (init_options, params.as_object_mut()) {
            object.insert("initializationOptions".to_owned(), options.clone());
        }
        let result = self.request("initialize", params, timeout, cancel)?;
        *lock(&self.shared.capabilities) =
            result.get("capabilities").cloned().unwrap_or(Value::Null);
        self.notify("initialized", json!({}));
        Ok(())
    }

    /// Name of the server program.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// Project root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.shared.root
    }

    /// Process identifier.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Capabilities from the `initialize` response.
    #[must_use]
    pub fn capabilities(&self) -> Value {
        lock(&self.shared.capabilities).clone()
    }

    /// Returns `true` when the server announced the capability (any value
    /// other than `null` and `false`) or registered `method` dynamically.
    #[must_use]
    pub fn supports(&self, capability: &str, method: &str) -> bool {
        let announced = lock(&self.shared.capabilities)
            .get(capability)
            .is_some_and(|value| !matches!(value, Value::Null | Value::Bool(false)));
        announced || lock(&self.shared.registrations).contains(method)
    }

    /// Work in progress of the server.
    #[must_use]
    pub fn progress(&self) -> &ProgressTracker {
        &self.shared.progress
    }

    /// Diagnostics published by the server.
    #[must_use]
    pub fn published_diagnostics(&self) -> &DiagnosticStore {
        &self.shared.diagnostics
    }

    /// Returns `true` once the server process ended or was stopped.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// Error describing the end of the server.
    #[must_use]
    pub fn closed_error(&self) -> LspError {
        LspError::Closed {
            server: self.shared.name.clone(),
            detail: lock(&self.shared.close_detail).clone(),
        }
    }

    /// Sends a notification; ignored after the server ended.
    pub fn notify(&self, method: &str, params: Value) {
        send(&self.shared, &Message::notification(method, params));
    }

    /// Sends a request and waits for its response.
    ///
    /// # Errors
    ///
    /// Fails with [`LspError::Timeout`] after `timeout`,
    /// [`LspError::Cancelled`] when `cancel` is cancelled (both cancel the
    /// request on the server), [`LspError::Rpc`] for an error response and
    /// [`LspError::Closed`] when the server ends.
    pub fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> Result<Value, LspError> {
        if let Some(reason) = cancel.reason() {
            return Err(LspError::Cancelled(reason));
        }
        if self.is_closed() {
            return Err(self.closed_error());
        }
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = bounded(1);
        lock(&self.shared.pending).insert(id, sender);
        if self.is_closed() {
            lock(&self.shared.pending).remove(&id);
            return Err(self.closed_error());
        }
        send(
            &self.shared,
            &Message::request(RequestId::Number(id), method, params),
        );
        self.await_response(id, &receiver, timeout, cancel)
    }

    fn await_response(
        &self,
        id: i64,
        receiver: &Receiver<Result<Value, RpcError>>,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> Result<Value, LspError> {
        let started = Instant::now();
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            match receiver.recv_timeout(remaining.min(CANCEL_POLL)) {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) => {
                    return Err(LspError::Rpc {
                        server: self.shared.name.clone(),
                        error: Box::new(error),
                    });
                }
                Err(RecvTimeoutError::Disconnected) => return Err(self.closed_error()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Some(reason) = cancel.reason() {
                self.abandon(id);
                return Err(LspError::Cancelled(reason));
            }
            if started.elapsed() >= timeout {
                self.abandon(id);
                return Err(LspError::Timeout {
                    server: self.shared.name.clone(),
                });
            }
        }
    }

    fn abandon(&self, id: i64) {
        if lock(&self.shared.pending).remove(&id).is_some() {
            self.notify("$/cancelRequest", json!({"id": id}));
        }
    }

    /// Opens the document in the server or sends its new content.
    pub fn sync_document(
        &self,
        path: &Path,
        language_id: &str,
        text: &str,
        stamp: Option<FileStamp>,
    ) {
        // The notification is queued before the lock of the documents is
        // released: the versions of one document reach the server in the
        // order in which they were numbered, whichever thread is faster.
        let mut documents = lock(&self.shared.documents);
        if let Some(notification) = documents.sync(path, language_id, text, stamp) {
            send(&self.shared, &notification);
        }
    }

    /// Closes the document in the server.
    pub fn forget_document(&self, path: &Path) {
        let mut documents = lock(&self.shared.documents);
        if let Some(notification) = documents.forget(path) {
            send(&self.shared, &notification);
            drop(documents);
            if let Some(uri) = path_to_uri(path) {
                self.shared.diagnostics.forget(&uri);
            }
        }
    }

    /// Version of an open document.
    #[must_use]
    pub fn document_version(&self, path: &Path) -> Option<i64> {
        lock(&self.shared.documents).version(path)
    }

    /// Brings the documents opened earlier up to date with the disk.
    pub fn refresh_documents(&self, max_bytes: u64) {
        let notifications = {
            let mut documents = lock(&self.shared.documents);
            let notifications = documents.refresh(max_bytes);
            for notification in &notifications {
                send(&self.shared, notification);
            }
            notifications
        };
        for notification in &notifications {
            if let Message::Notification { method, params } = notification
                && method == "textDocument/didClose"
                && let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str)
            {
                self.shared.diagnostics.forget(uri);
            }
        }
    }

    /// Asks the server to shut down, then stops its process tree and removes
    /// its temporary directory.
    pub fn shutdown(&self) {
        if self.is_closed() {
            return;
        }
        // The answer is not used; waiting for it is what the protocol asks
        // before `exit`, and an unresponsive server is stopped after the wait.
        let _ = self.request(
            "shutdown",
            Value::Null,
            SHUTDOWN_REPLY_WAIT,
            &CancelToken::new(),
        );
        send(&self.shared, &Message::notification("exit", Value::Null));
        mark_closed(&self.shared, None);
        let _ = lock(&self.shared.child).wait_timeout(EXIT_GRACE);
        finish(&self.shared);
    }
}

impl Drop for LanguageServer {
    /// Stops the server when the last handle goes away; the reader thread
    /// holds the one other reference to the connection.
    fn drop(&mut self) {
        if Arc::strong_count(&self.shared) <= READER_REFERENCES + 1 {
            self.shutdown();
        }
    }
}

/// Queues a message for the writer thread, so the thread that reads from the
/// server never blocks on a full input pipe of the server.
fn send(shared: &Shared, message: &Message) {
    if let Some(outbox) = lock(&shared.outbox).as_ref() {
        let _ = outbox.send(message.to_value());
    }
}

fn write_loop(stdin: Box<dyn Write + Send>, queue: &Receiver<Value>) {
    let mut writer = FrameWriter::new(stdin);
    while let Ok(message) = queue.recv() {
        if writer.send(&message).is_err() {
            break;
        }
    }
}

fn spawn_stderr_reader(stream: Box<dyn Read + Send>, shared: &Shared) -> Receiver<()> {
    let tail = Arc::clone(&shared.stderr_tail);
    let (done, finished) = bounded(1);
    let started = thread::Builder::new()
        .name(format!("lsp-{}-stderr", shared.name))
        .spawn(move || {
            // Standard error is drained to its end: a server whose error
            // pipe is closed early may be killed by SIGPIPE.
            let mut reader = BufReader::new(stream);
            let mut raw = Vec::new();
            loop {
                raw.clear();
                match (&mut reader).take(STDERR_CHUNK).read_until(b'\n', &mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let text = String::from_utf8_lossy(&raw);
                let line = text.trim();
                if !line.is_empty() {
                    *lock(&tail) = Some(line.to_owned());
                }
            }
            let _ = done.send(());
        });
    if started.is_err() {
        let (done, finished) = bounded(1);
        let _ = done.send(());
        return finished;
    }
    finished
}

fn read_loop(
    shared: &Arc<Shared>,
    stdout: Box<dyn Read + Send>,
    stderr_done: Option<Receiver<()>>,
) {
    let mut reader = FrameReader::new(stdout);
    while let Ok(Some(frame)) = reader.next_frame() {
        if let Frame::Message(value) = frame
            && let Ok(message) = Message::from_value(value)
        {
            dispatch(shared, message);
        }
    }
    if let Some(done) = stderr_done {
        let _ = done.recv_timeout(STDERR_GRACE);
    }
    let detail = lock(&shared.stderr_tail).clone();
    mark_closed(shared, detail);
    finish(shared);
}

fn dispatch(shared: &Shared, message: Message) {
    match message {
        Message::Response { id, result } => {
            if let RequestId::Number(id) = id
                && let Some(pending) = lock(&shared.pending).remove(&id)
            {
                let _ = pending.send(result);
            }
        }
        Message::Notification { method, params } => on_notification(shared, &method, &params),
        Message::Request { id, method, params } => {
            let result = answer(shared, &method, &params);
            send(shared, &Message::Response { id, result });
        }
    }
}

fn on_notification(shared: &Shared, method: &str, params: &Value) {
    match method {
        "textDocument/publishDiagnostics" => shared.diagnostics.publish(params),
        "$/progress" => shared.progress.on_progress(params),
        "experimental/serverStatus" => shared.progress.on_server_status(params),
        _ => {}
    }
}

/// Answer to a request of the server; servers wait for these answers before
/// they continue loading the project.
fn answer(shared: &Shared, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "workspace/configuration" => {
            let count = params
                .get("items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Ok(Value::Array(vec![Value::Null; count]))
        }
        "client/registerCapability" => {
            let methods = params
                .get("registrations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|registration| registration.get("method").and_then(Value::as_str));
            lock(&shared.registrations).extend(methods.map(str::to_owned));
            Ok(Value::Null)
        }
        "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "window/showMessageRequest"
        | "workspace/diagnostic/refresh"
        | "workspace/codeLens/refresh"
        | "workspace/semanticTokens/refresh"
        | "workspace/inlayHint/refresh" => Ok(Value::Null),
        "workspace/workspaceFolders" => {
            let uri = path_to_uri(&shared.root);
            Ok(json!([{"uri": uri, "name": shared.root.to_string_lossy()}]))
        }
        "workspace/applyEdit" => Ok(json!({"applied": false})),
        _ => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("unsupported method {method}"),
        )),
    }
}

fn mark_closed(shared: &Shared, detail: Option<String>) {
    if shared.closed.swap(true, Ordering::AcqRel) {
        return;
    }
    *lock(&shared.close_detail) = detail;
    lock(&shared.outbox).take();
    lock(&shared.pending).clear();
    shared.progress.close();
    shared.diagnostics.close();
}

/// Stops the process tree and removes the temporary directory once the
/// process has ended.
fn finish(shared: &Shared) {
    mark_closed(shared, None);
    {
        let mut child = lock(&shared.child);
        let _ = child.kill_tree();
        let _ = child.wait_timeout(EXIT_GRACE);
    }
    if let Some(dir) = lock(&shared.temp_dir).take() {
        let _ = dir.close();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
