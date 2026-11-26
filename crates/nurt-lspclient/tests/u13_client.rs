// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the language server client against an in-memory server.
//!
//! The mock server runs on a thread and talks to the client through
//! operating system pipes handed out by a mock process host, so the client
//! code under test is the same as with a real server process. Its answers,
//! delays, project loading, published diagnostics and crashes are scripted
//! per test. No network and no external program is used.

// The mock server and the fixtures fail the test on a setup error by
// unwrapping or panicking; clippy exempts only `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{PipeReader, PipeWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nurt_code::semantic::{
    CallDirection, DocumentChange, DocumentPosition, Location, SemanticErrorKind, SemanticProvider,
    SemanticScope,
};
use nurt_contracts::config::{CodeConfig, DurationSetting, LanguageServerConfig, ServerAllow};
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{DiagnosticSeverity, SymbolKind};
use nurt_contracts::services::{
    ChildExit, ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
    TextPosition,
};
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx};
use nurt_lspclient::config::{
    self, ClientSettings, host_tsserver_library, init_options, is_batch_file, npm_shim,
    resolve_command,
};
use nurt_lspclient::diagnostics::{DiagnosticStore, QuietWait};
use nurt_lspclient::jsonrpc::{Message, RequestId};
use nurt_lspclient::pool::{AcquireRequest, ServerPool};
use nurt_lspclient::progress::ProgressTracker;
use nurt_lspclient::provider::{
    LspProvider, from_lsp, hover_text, semantic_error, symbol_kind, to_lsp, word_at,
};
use nurt_lspclient::rename::PositionUnit;
use nurt_lspclient::server::LspError;
use nurt_lspclient::sync::{
    CandidateLimits, DocumentSet, candidate_files, contains_word, path_to_uri,
};
use nurt_lspclient::transport::{
    Frame, FrameError, FrameReader, FrameWriter, MESSAGE_BYTES_MAX, encode,
};
use serde_json::{Value, json};

// Mock language server that answers from a script.

type Handler = Arc<dyn Fn(&Value) -> std::result::Result<Value, String> + Send + Sync>;

/// Behaviour of the mock server.
#[derive(Default)]
struct Script {
    capabilities: Value,
    handlers: HashMap<String, Handler>,
    /// Diagnostics published after `didOpen` and `didChange`.
    publish: Option<Vec<Value>>,
    /// Publish only for the first version of a document.
    publish_once: bool,
    /// Answer nothing but `initialize`.
    mute: bool,
    /// Delay of every answer.
    delay: Duration,
    /// Project loading reported after `initialized`.
    loading: Option<Duration>,
    /// Write an error line and end when this method is requested.
    crash_on: Option<String>,
    fail_initialize: bool,
}

struct Mock {
    script: Mutex<Script>,
    received: Mutex<Vec<Value>>,
    loading: AtomicBool,
}

impl Mock {
    fn new(capabilities: Value) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(Script {
                capabilities,
                ..Script::default()
            }),
            received: Mutex::new(Vec::new()),
            loading: AtomicBool::new(false),
        })
    }

    fn handle(
        &self,
        method: &str,
        handler: impl Fn(&Value) -> std::result::Result<Value, String> + Send + Sync + 'static,
    ) {
        self.script
            .lock()
            .unwrap()
            .handlers
            .insert(method.to_owned(), Arc::new(handler));
    }

    fn methods(&self) -> Vec<String> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .map(|message| {
                message
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            })
            .collect()
    }

    fn messages(&self, method: &str) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message.get("method").and_then(Value::as_str) == Some(method))
            .cloned()
            .collect()
    }

    fn responses(&self) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message.get("method").is_none())
            .cloned()
            .collect()
    }

    fn wait_for(&self, method: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.methods().iter().any(|known| known == method) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

type Shared<T> = Arc<Mutex<Option<T>>>;

fn send(out: &Shared<FrameWriter<PipeWriter>>, message: &Value) {
    if let Some(writer) = out.lock().unwrap().as_mut() {
        let _ = writer.send(message);
    }
}

fn close(out: &Shared<FrameWriter<PipeWriter>>, err: &Shared<PipeWriter>) {
    out.lock().unwrap().take();
    err.lock().unwrap().take();
}

fn serve(
    mock: &Arc<Mock>,
    input: PipeReader,
    out: &Shared<FrameWriter<PipeWriter>>,
    err: &Shared<PipeWriter>,
) {
    let mut reader = FrameReader::new(input);
    while let Ok(Some(frame)) = reader.next_frame() {
        let Frame::Message(message) = frame else {
            continue;
        };
        mock.received.lock().unwrap().push(message.clone());
        if !on_message(mock, &message, out, err) {
            break;
        }
    }
    close(out, err);
}

fn on_message(
    mock: &Arc<Mock>,
    message: &Value,
    out: &Shared<FrameWriter<PipeWriter>>,
    err: &Shared<PipeWriter>,
) -> bool {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match message.get("id").filter(|id| !id.is_null()).cloned() {
        Some(id) if !method.is_empty() => on_request(mock, id, method, &params, out, err),
        Some(_) => true,
        None => on_notification(mock, method, &params, out),
    }
}

fn on_request(
    mock: &Arc<Mock>,
    id: Value,
    method: &str,
    params: &Value,
    out: &Shared<FrameWriter<PipeWriter>>,
    err: &Shared<PipeWriter>,
) -> bool {
    let script = mock.script.lock().unwrap();
    if method == "initialize" {
        let answer = if script.fail_initialize {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "cannot load the workspace"}})
        } else {
            json!({"jsonrpc": "2.0", "id": id, "result": {"capabilities": script.capabilities}})
        };
        drop(script);
        send(out, &answer);
        return true;
    }
    if method == "shutdown" {
        drop(script);
        send(out, &json!({"jsonrpc": "2.0", "id": id, "result": null}));
        return true;
    }
    if script.crash_on.as_deref() == Some(method) {
        drop(script);
        if let Some(writer) = err.lock().unwrap().as_mut() {
            let _ = writeln!(writer, "starting analysis");
            let _ = writeln!(writer, "fatal: index corrupted");
        }
        close(out, err);
        return false;
    }
    if script.mute {
        return true;
    }
    let answer = if mock.loading.load(Ordering::SeqCst) {
        json!({"jsonrpc": "2.0", "id": id, "result": null})
    } else {
        match script.handlers.get(method).map(|handler| handler(params)) {
            Some(Ok(result)) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Some(Err(message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": message}})
            }
            None => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unsupported method {method}")}})
            }
        }
    };
    let delay = script.delay;
    drop(script);
    if delay.is_zero() {
        send(out, &answer);
    } else {
        let out = Arc::clone(out);
        thread::spawn(move || {
            thread::sleep(delay);
            send(&out, &answer);
        });
    }
    true
}

fn on_notification(
    mock: &Arc<Mock>,
    method: &str,
    params: &Value,
    out: &Shared<FrameWriter<PipeWriter>>,
) -> bool {
    match method {
        "initialized" => {
            send(
                out,
                &json!({"jsonrpc": "2.0", "id": "cfg-1", "method": "workspace/configuration", "params": {"items": [{}, {}]}}),
            );
            send(
                out,
                &json!({"jsonrpc": "2.0", "id": "reg-1", "method": "client/registerCapability", "params": {"registrations": [{"id": "1", "method": "workspace/didChangeWatchedFiles"}]}}),
            );
            send(
                out,
                &json!({"jsonrpc": "2.0", "id": "wdp-1", "method": "window/workDoneProgress/create", "params": {"token": "load"}}),
            );
            let loading = mock.script.lock().unwrap().loading;
            if let Some(duration) = loading {
                mock.loading.store(true, Ordering::SeqCst);
                send(
                    out,
                    &json!({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": "load", "value": {"kind": "begin", "title": "indexing"}}}),
                );
                let out = Arc::clone(out);
                let mock = Arc::clone(mock);
                thread::spawn(move || {
                    thread::sleep(duration);
                    mock.loading.store(false, Ordering::SeqCst);
                    send(
                        &out,
                        &json!({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": "load", "value": {"kind": "end"}}}),
                    );
                });
            }
            true
        }
        "textDocument/didOpen" | "textDocument/didChange" => {
            let script = mock.script.lock().unwrap();
            let uri = params
                .pointer("/textDocument/uri")
                .cloned()
                .unwrap_or(Value::Null);
            let version = params
                .pointer("/textDocument/version")
                .and_then(Value::as_i64)
                .unwrap_or(1);
            if let Some(items) = &script.publish
                && (!script.publish_once || version == 1)
            {
                let note = json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "version": version, "diagnostics": items}});
                drop(script);
                send(out, &note);
            }
            true
        }
        "exit" => false,
        _ => true,
    }
}

// Process host double that runs the mock server on in-process pipes.

struct MockChild {
    pid: u32,
    stdin: Option<PipeWriter>,
    stdout: Option<PipeReader>,
    stderr: Option<PipeReader>,
    out: Shared<FrameWriter<PipeWriter>>,
    err: Shared<PipeWriter>,
    done: Arc<AtomicBool>,
    kills: Arc<AtomicUsize>,
}

impl ServiceChild for MockChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn started(&self) -> WallTime {
        WallTime::UNIX_EPOCH
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.stdin
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Write + Send>)
    }

    fn take_stdout(&mut self) -> Option<Box<dyn std::io::Read + Send>> {
        self.stdout
            .take()
            .map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>)
    }

    fn take_stderr(&mut self) -> Option<Box<dyn std::io::Read + Send>> {
        self.stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>)
    }

    fn try_wait(&mut self) -> Result<Option<ChildExit>> {
        Ok(self.done.load(Ordering::SeqCst).then_some(ChildExit {
            exit_code: Some(0),
            signal: None,
        }))
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ChildExit>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.try_wait()? {
                return Ok(Some(exit));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn kill_tree(&mut self) -> Result<()> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        close(&self.out, &self.err);
        self.done.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct MockHost {
    mock: Arc<Mock>,
    programs: Mutex<HashMap<String, PathBuf>>,
    refuse: AtomicBool,
    spawns: AtomicUsize,
    kills: Arc<AtomicUsize>,
    next_pid: AtomicU32,
    specs: Mutex<Vec<ServiceSpawnSpec>>,
}

impl MockHost {
    fn new(mock: &Arc<Mock>, programs: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            mock: Arc::clone(mock),
            programs: Mutex::new(
                programs
                    .iter()
                    .map(|program| {
                        (
                            (*program).to_owned(),
                            PathBuf::from(format!("/opt/bin/{program}")),
                        )
                    })
                    .collect(),
            ),
            refuse: AtomicBool::new(false),
            spawns: AtomicUsize::new(0),
            kills: Arc::new(AtomicUsize::new(0)),
            next_pid: AtomicU32::new(4000),
            specs: Mutex::new(Vec::new()),
        })
    }

    fn spawns(&self) -> usize {
        self.spawns.load(Ordering::SeqCst)
    }

    fn temp_dirs(&self) -> Vec<PathBuf> {
        self.specs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|spec| spec.temp_dir.clone())
            .collect()
    }
}

impl ProcessHost for MockHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        self.programs
            .lock()
            .unwrap()
            .get(program)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ExternalMissing,
                    format!("program {program} is not found in PATH"),
                )
            })
    }

    fn run_program(&self, _spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "the mock host does not run programs",
        ))
    }

    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        self.specs.lock().unwrap().push(spec.clone());
        if self.refuse.load(Ordering::SeqCst) {
            return Err(Error::new(
                ErrorCode::ExternalFailed,
                "the service limit is reached",
            ));
        }
        let (server_in, client_in) = std::io::pipe().unwrap();
        let (client_out, server_out) = std::io::pipe().unwrap();
        let (client_err, server_err) = std::io::pipe().unwrap();
        let out: Shared<FrameWriter<PipeWriter>> =
            Arc::new(Mutex::new(Some(FrameWriter::new(server_out))));
        let err: Shared<PipeWriter> = Arc::new(Mutex::new(Some(server_err)));
        let done = Arc::new(AtomicBool::new(false));
        {
            let (mock, out, err, done) = (
                Arc::clone(&self.mock),
                Arc::clone(&out),
                Arc::clone(&err),
                Arc::clone(&done),
            );
            thread::spawn(move || {
                serve(&mock, server_in, &out, &err);
                done.store(true, Ordering::SeqCst);
            });
        }
        Ok(Box::new(MockChild {
            pid: self.next_pid.fetch_add(1, Ordering::SeqCst),
            stdin: Some(client_in),
            stdout: Some(client_out),
            stderr: Some(client_err),
            out,
            err,
            done,
            kills: Arc::clone(&self.kills),
        }))
    }
}

// Clock, host and execution context of the tests.

struct VirtualClock(AtomicU64);

impl VirtualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(0)))
    }

    fn advance(&self, by: Duration) {
        self.0
            .fetch_add(u64::try_from(by.as_millis()).unwrap(), Ordering::SeqCst);
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        WallTime::UNIX_EPOCH
    }

    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime::UNIX_EPOCH,
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(
        &self,
        duration: Duration,
        _cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        self.advance(duration);
        Ok(())
    }
}

struct TestHost(HostInfo);

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

fn context(cancel: CancelToken) -> ExecCtx {
    let host = TestHost(HostInfo {
        os: Os::Linux,
        os_version: "6".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "test".to_owned(),
        user: "test".to_owned(),
        home: PathBuf::from("/home/test"),
        temp: std::env::temp_dir(),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    });
    ExecCtx::builder(VirtualClock::new(), Arc::new(host))
        .cancel(cancel)
        .build()
}

/// Configuration with the mock servers: `mock-py` for Python, `mock-ts`
/// shared by TypeScript and JavaScript.
fn code_config() -> CodeConfig {
    let mut code = CodeConfig::default();
    code.servers.clear();
    code.servers
        .insert("python".to_owned(), LanguageServerConfig::new(&["mock-py"]));
    code.servers.insert(
        "typescript".to_owned(),
        LanguageServerConfig::new(&["mock-ts", "--stdio"]),
    );
    code.servers.insert(
        "javascript".to_owned(),
        LanguageServerConfig::new(&["mock-ts", "--stdio"]),
    );
    code.servers
        .insert("go".to_owned(), LanguageServerConfig::new(&["gopls"]));
    code.timeout = DurationSetting::from_secs(5);
    code.diagnostics_quiet = DurationSetting::from_millis(100);
    code
}

const CAPABILITIES: &str = r#"{
    "definitionProvider": true, "referencesProvider": true, "hoverProvider": true,
    "implementationProvider": true, "typeDefinitionProvider": true, "callHierarchyProvider": true,
    "codeActionProvider": true, "documentSymbolProvider": true, "workspaceSymbolProvider": true,
    "renameProvider": true
}"#;

const SHAPES: &str = "\"\"\"Shapes.\"\"\"\nLIMIT = 3\n\n\nclass Shape:\n    def describe(self):\n        return \"shape\"\n\n\ndef area(shape):\n    return LIMIT\n";

const USE: &str = "from shapes import area, Shape\n\n\ndef main():\n    return area(Shape())\n";

/// Sample Python project with two modules and a skipped dependency folder.
struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("proj");
        for (name, text) in [
            ("pyproject.toml", "[project]\nname = \"sample\"\n"),
            ("pkg/shapes.py", SHAPES),
            ("pkg/use.py", USE),
            ("node_modules/dep/area.py", "def area():\n    pass\n"),
        ] {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Self { _dir: dir, root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn uri(&self, name: &str) -> String {
        path_to_uri(&self.path(name)).unwrap()
    }

    fn scope(&self) -> SemanticScope {
        SemanticScope::new(self.root.clone())
    }

    fn at(&self, name: &str, line: u32, column: u32) -> DocumentPosition {
        DocumentPosition {
            path: self.path(name),
            position: TextPosition { line, column },
        }
    }
}

fn range(line: u64, start: u64, end: u64) -> Value {
    json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}})
}

struct Setup {
    project: Project,
    mock: Arc<Mock>,
    host: Arc<MockHost>,
    clock: Arc<VirtualClock>,
    provider: LspProvider,
}

fn setup_with(code: &CodeConfig) -> Setup {
    let project = Project::new();
    let mock = Mock::new(serde_json::from_str(CAPABILITIES).unwrap());
    let host = MockHost::new(&mock, &["mock-py", "mock-ts"]);
    let clock = VirtualClock::new();
    let provider = LspProvider::new(host.clone(), clock.clone(), code);
    Setup {
        project,
        mock,
        host,
        clock,
        provider,
    }
}

fn setup() -> Setup {
    setup_with(&code_config())
}

// Transport and JSON-RPC tests.

#[test]
fn frame_reader_reassembles_messages_and_skips_malformed_frames() {
    let mut bytes = encode(&json!({"jsonrpc": "2.0", "id": 1, "result": null}));
    bytes.extend_from_slice(b"Content-Type: application/vscode-jsonrpc\r\n\r\n");
    bytes.extend_from_slice(b"Content-Length: 3\r\n\r\n[1]");
    bytes.extend_from_slice(b"Content-Length: 5\r\n\r\nnot j");
    bytes
        .extend_from_slice(format!("Content-Length: {}\r\n\r\n", MESSAGE_BYTES_MAX + 1).as_bytes());
    bytes.extend_from_slice(&[b'x'; 9000]);
    bytes.extend_from_slice(b"\r\n\r\n");
    bytes.extend_from_slice(&encode(&json!({"jsonrpc": "2.0", "method": "exit"})));
    let mut reader = FrameReader::new(bytes.as_slice());
    let mut frames = Vec::new();
    while let Some(frame) = reader.next_frame().unwrap() {
        frames.push(frame);
    }
    assert!(matches!(frames[0], Frame::Message(_)));
    assert_eq!(frames[1], Frame::Rejected(FrameError::MissingLength));
    assert_eq!(frames[2], Frame::Rejected(FrameError::InvalidBody));
    assert_eq!(frames[3], Frame::Rejected(FrameError::InvalidBody));
    assert_eq!(
        frames[4],
        Frame::Rejected(FrameError::TooLarge(MESSAGE_BYTES_MAX + 1))
    );
    assert!(
        frames
            .iter()
            .any(|frame| *frame == Frame::Rejected(FrameError::HeaderTooLong))
    );
    assert_eq!(
        frames.last(),
        Some(&Frame::Message(json!({"jsonrpc": "2.0", "method": "exit"})))
    );
}

#[test]
fn frame_reader_reads_messages_written_in_small_pieces_through_a_pipe() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let message = json!({"jsonrpc": "2.0", "method": "note", "params": {"text": "café déjà ✓"}});
    let bytes = encode(&message);
    let feeder = thread::spawn(move || {
        for chunk in bytes.chunks(3) {
            writer.write_all(chunk).unwrap();
        }
    });
    let mut frames = FrameReader::new(reader);
    assert_eq!(frames.next_frame().unwrap(), Some(Frame::Message(message)));
    feeder.join().unwrap();
    assert_eq!(frames.next_frame().unwrap(), None);
}

#[test]
fn stream_ending_inside_a_frame_ends_the_reader() {
    let mut reader = FrameReader::new(&b"Content-Length: 40\r\n\r\n{\"jsonrpc\""[..]);
    assert_eq!(reader.next_frame().unwrap(), None);
}

#[test]
fn messages_are_classified_by_their_members() {
    let request = Message::from_value(
        json!({"jsonrpc": "2.0", "id": "cfg", "method": "workspace/configuration"}),
    )
    .unwrap();
    assert_eq!(
        request,
        Message::request(
            RequestId::Text("cfg".to_owned()),
            "workspace/configuration",
            Value::Null
        )
    );
    let note =
        Message::from_value(json!({"method": "$/progress", "params": {"token": 1}})).unwrap();
    assert!(matches!(note, Message::Notification { .. }));
    let failed =
        Message::from_value(json!({"id": 2, "error": {"code": -32601, "message": "unknown"}}))
            .unwrap();
    let Message::Response {
        result: Err(error), ..
    } = failed
    else {
        panic!("expected an error response");
    };
    assert_eq!(error.code, -32601);
    for invalid in [
        json!([1]),
        json!({"method": 3}),
        json!({"id": 1}),
        json!({"result": 1}),
        json!({"id": true, "result": 1}),
    ] {
        assert!(Message::from_value(invalid).is_err());
    }
    let round = Message::from_value(request.to_value()).unwrap();
    assert_eq!(round, request);
}

// Progress, diagnostics and document synchronization tests.

#[test]
fn progress_tracker_counts_work_and_server_status() {
    let tracker = ProgressTracker::new();
    tracker.on_progress(
        &json!({"token": 7, "value": {"kind": "begin", "title": "indexing", "percentage": 10}}),
    );
    assert!(tracker.busy());
    assert_eq!(tracker.last_note().as_deref(), Some("indexing 10%"));
    assert!(!tracker.wait_settled(Duration::from_millis(30), &CancelToken::new()));
    tracker.on_progress(&json!({"token": 7, "value": {"kind": "end"}}));
    tracker.on_server_status(&json!({"quiescent": false}));
    assert!(tracker.busy());
    assert_eq!(tracker.loads(), 2);
    let cancelled = CancelToken::new();
    cancelled.cancel(CancelReason::Requested);
    assert!(!tracker.wait_settled(Duration::from_secs(5), &cancelled));
    tracker.on_server_status(&json!({"quiescent": true}));
    assert!(tracker.wait_settled(Duration::from_millis(10), &CancelToken::new()));
    tracker.on_server_status(&json!({"quiescent": false}));
    tracker.close();
    assert!(!tracker.busy());
}

#[test]
fn diagnostic_wait_ends_after_quiet_period_with_last_publication() {
    let store = DiagnosticStore::new();
    let cancel = CancelToken::new();
    let wait = QuietWait {
        timeout: Duration::from_secs(5),
        quiet: Duration::from_millis(80),
        cancel: &cancel,
    };
    store.publish(&json!({"uri": "file:///a.py", "version": 1, "diagnostics": [{"range": range(0, 0, 1), "message": "old"}]}));
    let started = Instant::now();
    let items = store.wait("file:///a.py", 2, &wait, || false).unwrap();
    assert_eq!(items[0].message, "old");
    assert!(started.elapsed() < Duration::from_secs(2));
    let short = QuietWait {
        timeout: Duration::from_millis(60),
        ..wait
    };
    assert!(store.wait("file:///b.py", 1, &short, || false).is_none());
    assert!(store.wait("file:///a.py", 3, &short, || true).is_none());
    store.forget("file:///a.py");
    store.close();
    assert!(store.wait("file:///a.py", 1, &wait, || false).is_none());
}

#[test]
fn diagnostic_wait_returns_a_newer_publication() {
    let store = Arc::new(DiagnosticStore::new());
    store.publish(&json!({"uri": "file:///a.py", "version": 1, "diagnostics": []}));
    let publisher = Arc::clone(&store);
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        publisher.publish(&json!({"uri": "file:///a.py", "diagnostics": [{"range": range(1, 0, 2), "message": "new", "severity": 2}]}));
    });
    let cancel = CancelToken::new();
    let wait = QuietWait {
        timeout: Duration::from_secs(5),
        quiet: Duration::from_secs(3),
        cancel: &cancel,
    };
    let items = store.wait("file:///a.py", 2, &wait, || false).unwrap();
    handle.join().unwrap();
    assert_eq!(items[0].message, "new");
}

#[test]
fn document_set_sends_changes_and_closes_removed_files() {
    let project = Project::new();
    let path = project.path("pkg/shapes.py");
    let mut documents = DocumentSet::new();
    let stamp = nurt_lspclient::sync::file_stamp(&path);
    assert!(
        matches!(documents.sync(&path, "python", SHAPES, stamp), Some(Message::Notification { ref method, .. }) if method == "textDocument/didOpen")
    );
    assert!(documents.refresh(1 << 20).is_empty());
    std::fs::write(&path, "LIMIT = 4\n").unwrap();
    let changes = documents.refresh(1 << 20);
    assert!(
        matches!(&changes[0], Message::Notification { method, params } if method == "textDocument/didChange" && params["contentChanges"][0]["text"] == "LIMIT = 4\n")
    );
    assert_eq!(documents.version(&path), Some(2));
    std::fs::remove_file(&path).unwrap();
    let closed = documents.refresh(1 << 20);
    assert!(
        matches!(&closed[0], Message::Notification { method, .. } if method == "textDocument/didClose")
    );
    assert!(!documents.is_open(&path));
    assert!(documents.forget(&path).is_none());
}

#[test]
fn candidate_files_contain_the_word_and_skip_dependency_folders() {
    let project = Project::new();
    let limits = CandidateLimits {
        max_files: 60,
        max_scanned: 100,
        max_bytes: 1 << 20,
    };
    let found = candidate_files(
        &project.root,
        "area",
        |path| path.extension().is_some_and(|ext| ext == "py"),
        limits,
    );
    assert_eq!(
        found,
        vec![project.path("pkg/shapes.py"), project.path("pkg/use.py")]
    );
    let one = candidate_files(
        &project.root,
        "area",
        |_| true,
        CandidateLimits {
            max_files: 1,
            ..limits
        },
    );
    assert_eq!(one.len(), 1);
    assert!(contains_word("x = area(1)", "area"));
    assert!(!contains_word("x = areas(1)", "area"));
    assert!(!contains_word("x", ""));
}

// Settings, server command and Windows wrapper tests.

#[test]
fn settings_follow_the_code_configuration() {
    let mut code = code_config();
    code.timeout_max = DurationSetting::from_secs(30);
    code.diagnostics_quiet = DurationSetting::from_millis(1);
    code.servers_allow = ServerAllow::Languages(vec!["Python".to_owned(), "mock-ts".to_owned()]);
    let settings = ClientSettings::from_config(&code);
    assert_eq!(settings.request_timeout(None), Duration::from_secs(5));
    assert_eq!(
        settings.request_timeout(Some(Duration::from_secs(600))),
        Duration::from_secs(30)
    );
    assert_eq!(settings.diagnostics_quiet, config::DIAGNOSTICS_QUIET_MIN);
    assert!(settings.allows("python", "mock-py"));
    assert!(settings.allows("javascript", "/opt/bin/mock-ts.cmd"));
    assert!(!settings.allows("go", "gopls"));
    assert_eq!(settings.command("py").map(<[String]>::len), Some(1));
    assert_eq!(
        settings.languages(),
        vec!["python", "typescript", "javascript", "go"]
    );
    let defaults = ClientSettings::default();
    assert_eq!(defaults.idle, Duration::from_secs(600));
    assert_eq!(defaults.retry, Duration::from_secs(300));
    assert_eq!(defaults.max_servers, 4);
    assert_eq!(
        defaults.command("cpp").map(|command| command[0].as_str()),
        Some("clangd")
    );
}

#[test]
fn languages_roots_and_program_names_are_detected() {
    let project = Project::new();
    assert_eq!(config::language_for_path(Path::new("a.PY")), Some("python"));
    assert_eq!(config::language_for_path(Path::new("Makefile")), None);
    assert_eq!(
        config::language_id(Path::new("a.jsx"), "javascript"),
        "javascriptreact"
    );
    assert_eq!(config::normalize_language("C++"), "cpp");
    assert_eq!(
        config::program_name("C:\\npm\\typescript-language-server.CMD"),
        "typescript-language-server"
    );
    assert_eq!(
        config::project_root(&project.path("pkg/shapes.py")),
        project.root
    );
    assert!(config::install_hint("gopls").is_some());
    assert!(config::install_hint("unknown-ls").is_none());
}

const NPM_SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\typescript-language-server\\lib\\cli.mjs\" %*\r\n";

#[test]
fn npm_wrapper_resolves_to_the_interpreter_and_script() {
    let shim = Path::new("/npm/typescript-language-server.cmd");
    let parsed = npm_shim(shim, NPM_SHIM, |_| false).unwrap();
    assert_eq!(
        parsed.script,
        Path::new("/npm/node_modules/typescript-language-server/lib/cli.mjs")
    );
    assert_eq!(parsed.interpreter, None);
    let local = npm_shim(shim, NPM_SHIM, |path| path.ends_with("node.exe")).unwrap();
    assert_eq!(local.interpreter, Some(PathBuf::from("/npm/node.exe")));
    let old_style = "@IF EXIST \"%~dp0\\node.exe\" (\r\n  \"%~dp0\\node.exe\"  \"%~dp0\\..\\pyright\\langserver.index.js\" %*\r\n)";
    let parsed = npm_shim(shim, old_style, |_| false).unwrap();
    assert_eq!(parsed.script, Path::new("/pyright/langserver.index.js"));
    assert!(npm_shim(shim, "@echo off\r\necho %1\r\n", |_| false).is_none());
    assert!(is_batch_file(Path::new("x.BAT")));
    assert!(!is_batch_file(Path::new("x.exe")));
}

#[test]
fn batch_server_commands_are_replaced_by_a_direct_interpreter_call() {
    let dir = tempfile::tempdir().unwrap();
    let wrapper = dir.path().join("typescript-language-server.cmd");
    std::fs::write(&wrapper, NPM_SHIM).unwrap();
    let other = dir.path().join("custom.bat");
    std::fs::write(&other, "@echo off\r\nserver.exe %1\r\n").unwrap();
    let mock = Mock::new(Value::Null);
    let host = MockHost::new(&mock, &["node"]);
    host.programs
        .lock()
        .unwrap()
        .insert("typescript-language-server".to_owned(), wrapper.clone());
    host.programs
        .lock()
        .unwrap()
        .insert("custom".to_owned(), other);
    let command = vec![
        "typescript-language-server".to_owned(),
        "--stdio".to_owned(),
    ];
    let resolved = resolve_command(host.as_ref(), &command, dir.path()).unwrap();
    assert_eq!(resolved.name, "typescript-language-server");
    assert_eq!(resolved.program, "/opt/bin/node");
    assert_eq!(resolved.args[1], "--stdio");
    assert!(resolved.args[0].ends_with("cli.mjs"));
    let refused = resolve_command(host.as_ref(), &["custom".to_owned()], dir.path()).unwrap_err();
    assert!(matches!(refused, LspError::Unavailable { .. }));
    let missing =
        resolve_command(host.as_ref(), &["absent-ls".to_owned()], dir.path()).unwrap_err();
    assert!(matches!(missing, LspError::Missing { .. }));
    assert!(matches!(
        resolve_command(host.as_ref(), &[], dir.path()),
        Err(LspError::Missing { .. })
    ));
}

#[test]
fn typescript_server_gets_the_host_tsserver_only_without_a_project_typescript() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("ts");
    std::fs::create_dir_all(install.join("bin")).unwrap();
    std::fs::create_dir_all(install.join("lib")).unwrap();
    std::fs::write(install.join("bin/tsserver"), "").unwrap();
    std::fs::write(install.join("lib/tsserver.js"), "").unwrap();
    let mock = Mock::new(Value::Null);
    let host = MockHost::new(&mock, &[]);
    host.programs
        .lock()
        .unwrap()
        .insert("tsserver".to_owned(), install.join("bin/tsserver"));
    let bare = dir.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let own = dir.path().join("own");
    std::fs::create_dir_all(own.join("node_modules/typescript/lib")).unwrap();
    std::fs::write(own.join("node_modules/typescript/lib/tsserver.js"), "").unwrap();
    let settings = ClientSettings::from_config(&code_config());
    let options = init_options(
        &settings,
        "typescript",
        "typescript-language-server",
        &bare,
        host.as_ref(),
    )
    .unwrap();
    assert!(
        options["tsserver"]["path"]
            .as_str()
            .unwrap()
            .ends_with("tsserver.js")
    );
    assert!(
        init_options(
            &settings,
            "typescript",
            "typescript-language-server",
            &own,
            host.as_ref()
        )
        .is_none()
    );
    assert!(
        init_options(
            &settings,
            "python",
            "pyright-langserver",
            &bare,
            host.as_ref()
        )
        .is_none()
    );
    let mut code = code_config();
    code.servers.get_mut("typescript").unwrap().init = Some(json!({"preferences": {}}));
    let configured = ClientSettings::from_config(&code);
    assert_eq!(
        init_options(
            &configured,
            "typescript",
            "typescript-language-server",
            &bare,
            host.as_ref()
        ),
        Some(json!({"preferences": {}}))
    );
    assert!(host_tsserver_library(&dir.path().join("absent")).is_none());
}

// Pool and server lifecycle tests.

fn acquire(
    pool: &ServerPool,
    language: &str,
    root: &Path,
) -> std::result::Result<nurt_lspclient::server::LanguageServer, LspError> {
    pool.acquire(&AcquireRequest {
        language,
        root,
        timeout: Duration::from_secs(5),
        cancel: &CancelToken::new(),
    })
}

#[test]
fn pool_shares_one_start_between_concurrent_commands_and_answers_server_requests() {
    let s = setup();
    let pool = s.provider.pool();
    thread::scope(|scope| {
        let first =
            scope.spawn(|| acquire(pool, "python", &s.project.root).map(|server| server.pid()));
        let second =
            scope.spawn(|| acquire(pool, "python", &s.project.root).map(|server| server.pid()));
        assert_eq!(
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap()
        );
    });
    assert_eq!(s.host.spawns(), 1);
    let initialize = &s.mock.messages("initialize")[0];
    assert_eq!(initialize["params"]["rootUri"], s.project.uri(""));
    assert!(s.mock.wait_for("initialized"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.mock.responses().len() < 3 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let responses = s.mock.responses();
    let configuration = responses.iter().find(|item| item["id"] == "cfg-1").unwrap();
    assert_eq!(configuration["result"], json!([null, null]));
    assert!(
        responses
            .iter()
            .any(|item| item["id"] == "reg-1" && item["result"].is_null())
    );
    assert!(responses.iter().any(|item| item["id"] == "wdp-1"));
}

#[test]
fn typescript_and_javascript_share_one_server() {
    let s = setup();
    let pool = s.provider.pool();
    let typescript = acquire(pool, "typescript", &s.project.root).unwrap();
    let javascript = acquire(pool, "javascript", &s.project.root).unwrap();
    assert_eq!(typescript.pid(), javascript.pid());
    assert_eq!(s.host.spawns(), 1);
    let status = pool.status();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].languages, vec!["javascript", "typescript"]);
    assert_eq!(status[0].server, "mock-ts");
    assert!(status[0].ready);
}

#[test]
fn idle_server_stops_and_its_temporary_directory_is_removed() {
    let s = setup();
    let pool = s.provider.pool();
    acquire(pool, "python", &s.project.root).unwrap();
    let temp = s.host.temp_dirs();
    assert_eq!(temp.len(), 1);
    assert!(temp[0].is_dir());
    assert_eq!(pool.reap_idle(), 0);
    s.clock.advance(Duration::from_secs(601));
    assert_eq!(pool.reap_idle(), 1);
    assert!(pool.status().is_empty());
    assert!(!temp[0].exists());
    assert!(s.mock.wait_for("exit"));
    assert!(s.host.kills.load(Ordering::SeqCst) >= 1);
}

#[test]
fn pool_limit_stops_the_least_recently_used_server() {
    let mut code = code_config();
    code.max_servers = 1;
    let s = setup_with(&code);
    let other = s.project.path("pkg");
    let pool = s.provider.pool();
    let first = acquire(pool, "python", &s.project.root).unwrap();
    s.clock.advance(Duration::from_secs(1));
    acquire(pool, "python", &other).unwrap();
    let status = pool.status();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].root, other);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !first.is_closed() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(first.is_closed());
}

#[test]
fn refused_start_is_not_retried_before_the_retry_period() {
    let s = setup();
    s.host.refuse.store(true, Ordering::SeqCst);
    let pool = s.provider.pool();
    let error = acquire(pool, "python", &s.project.root).unwrap_err();
    assert!(error.to_string().contains("service limit"), "{error}");
    assert_eq!(s.host.spawns(), 1);
    assert!(acquire(pool, "python", &s.project.root).is_err());
    assert_eq!(s.host.spawns(), 1);
    s.host.refuse.store(false, Ordering::SeqCst);
    s.clock.advance(Duration::from_secs(301));
    acquire(pool, "python", &s.project.root).unwrap();
    assert_eq!(s.host.spawns(), 2);
    assert_eq!(pool.stop(Some("mock-py")), 1);
    assert_eq!(pool.stop(None), 0);
}

#[test]
fn failed_initialization_reports_the_server_error() {
    let s = setup();
    s.mock.script.lock().unwrap().fail_initialize = true;
    let error = acquire(s.provider.pool(), "python", &s.project.root).unwrap_err();
    assert!(matches!(error, LspError::Unavailable { .. }));
    assert!(
        error.to_string().contains("cannot load the workspace"),
        "{error}"
    );
}

#[test]
fn servers_outside_the_allowance_or_not_installed_are_missing() {
    let mut code = code_config();
    code.servers_allow = ServerAllow::Languages(vec!["go".to_owned()]);
    let s = setup_with(&code);
    let ctx = context(CancelToken::new());
    let error = s
        .provider
        .definition(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::Unavailable);
    assert!(error.message.contains("servers_allow"));
    let gopls = semantic_error(acquire(s.provider.pool(), "go", &s.project.root).unwrap_err());
    assert_eq!(gopls.kind, SemanticErrorKind::Unavailable);
    assert_eq!(gopls.fix.as_deref(), config::install_hint("gopls"));
    assert_eq!(gopls.to_error().code(), ErrorCode::ExternalMissing);
    let unknown = s.provider.definition(
        &ctx,
        &s.project.scope(),
        &DocumentPosition {
            path: s.project.path("pyproject.toml"),
            position: TextPosition { line: 1, column: 1 },
        },
    );
    assert_eq!(unknown.unwrap_err().kind, SemanticErrorKind::Unavailable);
}

#[test]
fn cancelled_request_is_cancelled_on_the_server() {
    let s = setup();
    s.mock.script.lock().unwrap().mute = true;
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        trigger.cancel(CancelReason::Requested);
    });
    let ctx = context(cancel);
    let error = s
        .provider
        .definition(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap_err();
    canceller.join().unwrap();
    assert_eq!(error.kind, SemanticErrorKind::Cancelled);
    assert!(s.mock.wait_for("$/cancelRequest"));
    let request = s.mock.messages("textDocument/definition")[0]["id"].clone();
    assert_eq!(
        s.mock.messages("$/cancelRequest")[0]["params"]["id"],
        request
    );
}

#[test]
fn slow_request_times_out_and_is_cancelled() {
    let s = setup();
    s.mock.script.lock().unwrap().delay = Duration::from_secs(3);
    s.mock
        .handle("textDocument/definition", |_| Ok(Value::Null));
    let ctx = context(CancelToken::new());
    let mut scope = s.project.scope();
    scope.timeout = Some(Duration::from_millis(300));
    let error = s
        .provider
        .definition(&ctx, &scope, &s.project.at("pkg/use.py", 5, 12))
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::TimedOut);
    assert!(s.mock.wait_for("$/cancelRequest"));
}

#[test]
fn server_end_reports_the_last_error_line() {
    let s = setup();
    s.mock.script.lock().unwrap().crash_on = Some("textDocument/definition".to_owned());
    let ctx = context(CancelToken::new());
    let error = s
        .provider
        .definition(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::Failed);
    assert!(
        error.message.contains("fatal: index corrupted"),
        "{}",
        error.message
    );
    assert_eq!(error.to_error().code(), ErrorCode::ExternalFailed);
    let temp = s.host.temp_dirs();
    let deadline = Instant::now() + Duration::from_secs(5);
    while temp[0].exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!temp[0].exists());
    assert!(s.provider.pool().status().is_empty());
}

#[test]
fn shutdown_stops_every_server() {
    let s = setup();
    acquire(s.provider.pool(), "python", &s.project.root).unwrap();
    acquire(s.provider.pool(), "typescript", &s.project.root).unwrap();
    assert_eq!(s.provider.shutdown(), 2);
    assert!(s.host.temp_dirs().iter().all(|dir| !dir.exists()));
}

// Provider request tests.

#[test]
fn definition_converts_locations_and_location_links() {
    let s = setup();
    let shapes = s.project.uri("pkg/shapes.py");
    s.mock.handle("textDocument/definition", move |params| {
        assert_eq!(params["position"], json!({"line": 4, "character": 11}));
        Ok(json!([
            {"uri": shapes, "range": range(9, 4, 8)},
            {"targetUri": shapes, "targetRange": range(9, 0, 20), "targetSelectionRange": range(9, 4, 8)},
            {"uri": "untitled:1", "range": range(0, 0, 1)}
        ]))
    });
    let ctx = context(CancelToken::new());
    let found = s
        .provider
        .definition(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap();
    assert_eq!(
        found,
        vec![Location {
            path: s.project.path("pkg/shapes.py"),
            start: TextPosition {
                line: 10,
                column: 5
            },
            end: TextPosition {
                line: 10,
                column: 9
            },
        }]
    );
    let opened = s.mock.messages("textDocument/didOpen");
    assert_eq!(opened[0]["params"]["textDocument"]["languageId"], "python");
}

#[test]
fn references_open_project_files_that_contain_the_name() {
    let s = setup();
    let (shapes, usage) = (s.project.uri("pkg/shapes.py"), s.project.uri("pkg/use.py"));
    s.mock.handle("textDocument/references", move |params| {
        assert_eq!(params["context"]["includeDeclaration"], true);
        Ok(json!([{"uri": shapes, "range": range(9, 4, 8)}, {"uri": usage, "range": range(4, 11, 15)}]))
    });
    let ctx = context(CancelToken::new());
    let found = s
        .provider
        .references(
            &ctx,
            &s.project.scope(),
            &s.project.at("pkg/use.py", 5, 12),
            true,
        )
        .unwrap();
    assert_eq!(found.len(), 2);
    let opened: Vec<String> = s
        .mock
        .messages("textDocument/didOpen")
        .iter()
        .map(|message| {
            message["params"]["textDocument"]["uri"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert!(opened.contains(&s.project.uri("pkg/shapes.py")));
    assert!(!opened.iter().any(|uri| uri.contains("node_modules")));
}

#[test]
fn hover_joins_markup_and_marked_strings() {
    let s = setup();
    s.mock.handle("textDocument/hover", |_| {
        Ok(json!({"contents": [{"language": "python", "value": "def area(shape)"}, "Compute the area."], "range": range(4, 11, 15)}))
    });
    let ctx = context(CancelToken::new());
    let hover = s
        .provider
        .hover(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap()
        .unwrap();
    assert_eq!(
        hover.contents,
        "```python\ndef area(shape)\n```\n\nCompute the area."
    );
    assert_eq!(
        hover.range.unwrap().start,
        TextPosition {
            line: 5,
            column: 12
        }
    );
    s.mock.handle("textDocument/hover", |_| {
        Ok(json!({"contents": {"kind": "markdown", "value": "  "}}))
    });
    assert!(
        s.provider
            .hover(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
            .unwrap()
            .is_none()
    );
}

#[test]
fn implementations_type_definitions_and_unsupported_requests() {
    let s = setup();
    let shapes = s.project.uri("pkg/shapes.py");
    let again = shapes.clone();
    s.mock.handle("textDocument/implementation", move |_| {
        Ok(json!({"uri": shapes, "range": range(4, 6, 11)}))
    });
    s.mock.handle("textDocument/typeDefinition", move |_| {
        Ok(json!([{"uri": again, "range": range(4, 6, 11)}]))
    });
    let ctx = context(CancelToken::new());
    let at = s.project.at("pkg/use.py", 5, 17);
    assert_eq!(
        s.provider
            .implementations(&ctx, &s.project.scope(), &at)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.provider
            .type_definition(&ctx, &s.project.scope(), &at)
            .unwrap()[0]
            .start
            .line,
        5
    );
    s.mock.script.lock().unwrap().capabilities["typeDefinitionProvider"] = json!(false);
    s.provider.shutdown();
    let error = s
        .provider
        .type_definition(&ctx, &s.project.scope(), &at)
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::Unsupported);
}

#[test]
fn call_hierarchy_lists_callers_and_callees_with_call_sites() {
    let s = setup();
    let (shapes, usage) = (s.project.uri("pkg/shapes.py"), s.project.uri("pkg/use.py"));
    let item = json!({"name": "area", "kind": 12, "uri": shapes, "range": range(9, 0, 20), "selectionRange": range(9, 4, 8)});
    let prepared = item.clone();
    s.mock
        .handle("textDocument/prepareCallHierarchy", move |_| {
            Ok(json!([prepared]))
        });
    let caller = json!({"name": "main", "kind": 12, "uri": usage, "range": range(3, 0, 20), "selectionRange": range(3, 4, 8)});
    s.mock.handle("callHierarchy/incomingCalls", move |_| {
        Ok(json!([{"from": caller, "fromRanges": [range(4, 11, 15), range(4, 11, 15)]}]))
    });
    let callee = json!({"name": "LIMIT", "kind": 14, "uri": s.project.uri("pkg/shapes.py"), "range": range(1, 0, 5)});
    s.mock.handle("callHierarchy/outgoingCalls", move |_| {
        Ok(json!([{"to": callee, "fromRanges": [range(10, 11, 16)]}]))
    });
    let ctx = context(CancelToken::new());
    let at = s.project.at("pkg/shapes.py", 10, 5);
    let edges = s
        .provider
        .call_hierarchy(&ctx, &s.project.scope(), &at, CallDirection::Both)
        .unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0].direction, CallDirection::Incoming);
    assert_eq!(edges[0].symbol.name, "main");
    assert_eq!(edges[0].calls.len(), 2);
    assert_eq!(edges[0].calls[0].path, s.project.path("pkg/use.py"));
    assert_eq!(edges[1].symbol.kind, SymbolKind::Constant);
    assert_eq!(edges[1].calls[0].path, s.project.path("pkg/shapes.py"));
    let incoming = s
        .provider
        .call_hierarchy(&ctx, &s.project.scope(), &at, CallDirection::Incoming)
        .unwrap();
    assert_eq!(incoming.len(), 1);
}

#[test]
fn code_actions_carry_published_diagnostics_and_edits() {
    let s = setup();
    let diagnostic = json!({"range": range(4, 11, 15), "message": "area is unused", "severity": 2});
    s.mock.script.lock().unwrap().publish = Some(vec![diagnostic.clone()]);
    let usage = s.project.uri("pkg/use.py");
    s.mock.handle("textDocument/codeAction", move |params| {
        assert_eq!(params["context"]["diagnostics"][0]["message"], "area is unused");
        Ok(json!([
            {"title": "Organize imports", "command": "pyright.organizeimports"},
            {"title": "Rename to total", "kind": "quickfix", "isPreferred": true,
             "edit": {"changes": {usage.clone(): [{"range": range(4, 11, 15), "newText": "total"}]}}}
        ]))
    });
    let ctx = context(CancelToken::new());
    let location = Location {
        path: s.project.path("pkg/use.py"),
        start: TextPosition { line: 5, column: 1 },
        end: TextPosition {
            line: 5,
            column: 25,
        },
    };
    let actions = s
        .provider
        .code_actions(&ctx, &s.project.scope(), &location)
        .unwrap();
    assert_eq!(
        actions[0].command.as_deref(),
        Some("pyright.organizeimports")
    );
    assert!(actions[1].preferred);
    let edit = actions[1].edit.as_ref().unwrap();
    assert!(matches!(&edit.changes[0], DocumentChange::Text(change) if change.new_text == "total"));
}

#[test]
fn published_diagnostics_are_converted_by_severity() {
    let s = setup();
    s.mock.script.lock().unwrap().publish = Some(vec![
        json!({"range": range(0, 5, 11), "message": "\"areas\" is\n not  defined", "severity": 1, "code": "reportUndefinedVariable", "source": "pyright"}),
        json!({"range": range(1, 0, 1), "message": "hint", "severity": 4, "code": 7}),
        json!({"message": "no range"}),
    ]);
    let ctx = context(CancelToken::new());
    let items = s
        .provider
        .diagnostics(
            &ctx,
            &s.project.scope(),
            &s.project.path("pkg/use.py"),
            None,
        )
        .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].severity, DiagnosticSeverity::Error);
    assert_eq!(items[0].message, "\"areas\" is not defined");
    assert_eq!(items[0].code.as_deref(), Some("reportUndefinedVariable"));
    assert_eq!(items[0].source.as_deref(), Some("pyright"));
    assert_eq!(items[1].severity, DiagnosticSeverity::Hint);
    assert_eq!(items[1].code.as_deref(), Some("7"));
}

#[test]
fn pull_diagnostics_are_requested_from_servers_that_offer_them() {
    let s = setup();
    s.mock.script.lock().unwrap().capabilities["diagnosticProvider"] =
        json!({"interFileDependencies": false});
    s.mock.handle("textDocument/diagnostic", |_| Ok(json!({"kind": "full", "items": [{"range": range(2, 0, 1), "message": "pulled", "severity": 3}]})));
    let ctx = context(CancelToken::new());
    let items = s
        .provider
        .diagnostics(
            &ctx,
            &s.project.scope(),
            &s.project.path("pkg/use.py"),
            None,
        )
        .unwrap();
    assert_eq!(items[0].message, "pulled");
    assert_eq!(items[0].severity, DiagnosticSeverity::Info);
}

#[test]
fn diagnostics_of_text_in_memory_are_replaced_by_the_disk_content_later() {
    let s = setup();
    s.mock.script.lock().unwrap().publish = Some(Vec::new());
    let ctx = context(CancelToken::new());
    let path = s.project.path("pkg/use.py");
    s.provider
        .diagnostics(&ctx, &s.project.scope(), &path, Some("broken(\n"))
        .unwrap();
    let opened = s.mock.messages("textDocument/didOpen");
    assert_eq!(
        opened.last().unwrap()["params"]["textDocument"]["text"],
        "broken(\n"
    );
    s.provider
        .diagnostics(
            &ctx,
            &s.project.scope(),
            &s.project.path("pkg/shapes.py"),
            None,
        )
        .unwrap();
    let changed = s.mock.messages("textDocument/didChange");
    assert_eq!(
        changed.last().unwrap()["params"]["contentChanges"][0]["text"],
        USE
    );
}

#[test]
fn quiet_server_keeps_its_last_publication() {
    let s = setup();
    {
        let mut script = s.mock.script.lock().unwrap();
        script.publish = Some(vec![
            json!({"range": range(0, 0, 4), "message": "first", "severity": 2}),
        ]);
        script.publish_once = true;
    }
    let ctx = context(CancelToken::new());
    let path = s.project.path("pkg/use.py");
    assert_eq!(
        s.provider
            .diagnostics(&ctx, &s.project.scope(), &path, None)
            .unwrap()[0]
            .message,
        "first"
    );
    std::fs::write(&path, format!("{USE}# changed\n")).unwrap();
    let started = Instant::now();
    let items = s
        .provider
        .diagnostics(&ctx, &s.project.scope(), &path, None)
        .unwrap();
    assert_eq!(items[0].message, "first");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(s.mock.messages("textDocument/didChange").len(), 1);
}

#[test]
fn removed_documents_are_closed_before_the_next_request() {
    let s = setup();
    s.mock.handle("textDocument/definition", |_| Ok(json!([])));
    let ctx = context(CancelToken::new());
    s.provider
        .definition(
            &ctx,
            &s.project.scope(),
            &s.project.at("pkg/shapes.py", 10, 5),
        )
        .unwrap();
    std::fs::remove_file(s.project.path("pkg/shapes.py")).unwrap();
    s.provider
        .definition(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap();
    let closed = s.mock.messages("textDocument/didClose");
    assert_eq!(
        closed[0]["params"]["textDocument"]["uri"],
        s.project.uri("pkg/shapes.py")
    );
}

#[test]
fn answers_during_project_loading_are_repeated_after_loading_ends() {
    let s = setup();
    s.mock.script.lock().unwrap().loading = Some(Duration::from_millis(400));
    let shapes = s.project.uri("pkg/shapes.py");
    s.mock.handle("textDocument/definition", move |_| {
        Ok(json!([{"uri": shapes, "range": range(9, 4, 8)}]))
    });
    let ctx = context(CancelToken::new());
    let found = s
        .provider
        .definition(&ctx, &s.project.scope(), &s.project.at("pkg/use.py", 5, 12))
        .unwrap();
    assert_eq!(found.len(), 1);
    assert!(s.mock.messages("textDocument/definition").len() >= 2);
}

#[test]
fn loading_that_outlasts_the_time_limit_times_out() {
    let s = setup();
    s.mock.script.lock().unwrap().loading = Some(Duration::from_secs(30));
    s.mock.handle("textDocument/definition", |_| Ok(json!([])));
    let ctx = context(CancelToken::new());
    let mut scope = s.project.scope();
    scope.timeout = Some(Duration::from_millis(400));
    let error = s
        .provider
        .definition(&ctx, &scope, &s.project.at("pkg/use.py", 5, 12))
        .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::TimedOut);
    assert!(error.message.contains("loading"));
}

#[test]
fn document_and_workspace_symbols_are_flattened() {
    let s = setup();
    s.mock.handle("textDocument/documentSymbol", |_| {
        Ok(json!([{"name": "Shape", "kind": 5, "range": range(4, 0, 30), "selectionRange": range(4, 6, 11),
                   "children": [{"name": "describe", "kind": 6, "detail": "(self)", "range": range(5, 4, 30), "selectionRange": range(5, 8, 16)}]}]))
    });
    let shapes = s.project.uri("pkg/shapes.py");
    s.mock.handle("workspace/symbol", move |params| {
        assert_eq!(params["query"], "are");
        Ok(json!([{"name": "area", "kind": 12, "location": {"uri": shapes, "range": range(9, 4, 8)}, "containerName": "shapes"},
                  {"name": "areas", "kind": 13, "location": {"uri": shapes}}]))
    });
    let ctx = context(CancelToken::new());
    let symbols = s
        .provider
        .document_symbols(&ctx, &s.project.scope(), &s.project.path("pkg/shapes.py"))
        .unwrap();
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[1].container.as_deref(), Some("Shape"));
    assert_eq!(symbols[1].depth, 1);
    assert_eq!(symbols[1].kind, SymbolKind::Method);
    assert_eq!(symbols[1].detail.as_deref(), Some("(self)"));
    let mut scope = s.project.scope();
    scope.language = Some(nurt_contracts::services::LanguageId::new("python").unwrap());
    let found = s.provider.workspace_symbols(&ctx, &scope, "are").unwrap();
    assert_eq!(found[0].container.as_deref(), Some("shapes"));
    assert_eq!(found[1].location.start, TextPosition { line: 1, column: 1 });
    assert!(
        s.provider
            .workspace_symbols(&ctx, &s.project.scope(), "are")
            .is_err()
    );
}

#[test]
fn rename_returns_the_workspace_change() {
    let s = setup();
    let (shapes, usage) = (s.project.uri("pkg/shapes.py"), s.project.uri("pkg/use.py"));
    s.mock.handle("textDocument/rename", move |params| {
        assert_eq!(params["newName"], "surface");
        Ok(
            json!({"changes": {shapes.clone(): [{"range": range(9, 4, 8), "newText": "surface"}],
                              usage.clone(): [{"range": range(4, 11, 15), "newText": "surface"}]}}),
        )
    });
    let ctx = context(CancelToken::new());
    let change = s
        .provider
        .rename(
            &ctx,
            &s.project.scope(),
            &s.project.at("pkg/use.py", 5, 12),
            "surface",
        )
        .unwrap();
    assert_eq!(change.changes.len(), 2);
}

#[test]
fn server_status_reports_running_and_available_servers() {
    let s = setup();
    let ctx = context(CancelToken::new());
    let before = s.provider.server_status(&ctx, None);
    assert_eq!(before.len(), 4);
    assert!(before.iter().all(|status| !status.running));
    let go = before
        .iter()
        .find(|status| status.language.as_str() == "go")
        .unwrap();
    assert!(!go.found);
    assert!(go.allowed);
    acquire(s.provider.pool(), "typescript", &s.project.root).unwrap();
    let python = nurt_contracts::services::LanguageId::new("javascript").unwrap();
    let after = s.provider.server_status(&ctx, Some(&python));
    assert_eq!(after.len(), 1);
    assert!(after[0].running);
    assert_eq!(after[0].root.as_deref(), Some(s.project.root.as_path()));
    assert!(after[0].pid.is_some());
}

// Position conversion tests.

#[test]
fn columns_are_converted_between_characters_and_code_units() {
    let text = "s = \"é😀\"; area()\n";
    let position = TextPosition {
        line: 1,
        column: 12,
    };
    let utf16 = to_lsp(Some(text), position, PositionUnit::Utf16);
    assert_eq!(utf16, json!({"line": 0, "character": 12}));
    assert_eq!(
        to_lsp(Some(text), position, PositionUnit::Utf8)["character"],
        15
    );
    assert_eq!(
        to_lsp(Some(text), position, PositionUnit::Utf32)["character"],
        11
    );
    assert_eq!(
        from_lsp(Some(text), &utf16, PositionUnit::Utf16),
        Some(position)
    );
    assert_eq!(
        from_lsp(
            None,
            &json!({"line": 2, "character": 3}),
            PositionUnit::Utf16
        ),
        Some(TextPosition { line: 3, column: 4 })
    );
    assert_eq!(
        to_lsp(
            Some("ab"),
            TextPosition { line: 1, column: 6 },
            PositionUnit::Utf16
        )["character"],
        5
    );
    assert!(from_lsp(Some(text), &json!({"line": 0}), PositionUnit::Utf16).is_none());
}

#[test]
fn identifiers_kinds_and_hover_contents_are_read() {
    assert_eq!(
        word_at("x = area(1)", TextPosition { line: 1, column: 6 }).as_deref(),
        Some("area")
    );
    assert_eq!(
        word_at("x = area(1)", TextPosition { line: 1, column: 9 }).as_deref(),
        Some("area")
    );
    assert_eq!(
        word_at("x = area(1)", TextPosition { line: 1, column: 3 }),
        None
    );
    assert_eq!(word_at("x", TextPosition { line: 4, column: 1 }), None);
    assert_eq!(symbol_kind(5), SymbolKind::Class);
    assert_eq!(symbol_kind(23), SymbolKind::Struct);
    assert_eq!(symbol_kind(99), SymbolKind::Other);
    assert_eq!(hover_text(&json!("plain")), "plain");
    assert_eq!(hover_text(&json!({"kind": "plaintext", "value": "v"})), "v");
    assert_eq!(hover_text(&Value::Null), "");
}

#[test]
fn dropping_the_last_handle_stops_the_server_and_removes_its_temporary_directory() {
    use nurt_lspclient::server::{LanguageServer, ServerSpec};

    let s = setup();
    let spec = ServerSpec {
        name: "mock-py".to_owned(),
        program: "/opt/bin/mock-py".to_owned(),
        args: Vec::new(),
        root: s.project.root.clone(),
        init_options: None,
        env: Vec::new(),
    };
    let server = LanguageServer::spawn(s.host.as_ref(), &spec).unwrap();
    server
        .initialize(None, Duration::from_secs(5), &CancelToken::new())
        .unwrap();
    let clone = server.clone();
    drop(server);
    assert!(!clone.is_closed());
    assert!(!s.mock.methods().iter().any(|method| method == "exit"));
    drop(clone);
    assert!(s.mock.wait_for("exit"));
    let temp = s.host.temp_dirs();
    let deadline = Instant::now() + Duration::from_secs(5);
    while temp[0].exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!temp[0].exists());
    assert!(s.host.kills.load(Ordering::SeqCst) >= 1);
}
