// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-mcp/tests/common/mod.rs
// Responsibility:   Test support: a mock engine and a client on an in-memory stream.
// Dependencies:     nurt_contracts, serde_json, tokio
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test support: a mock engine and a client on an in-memory stream.
//!
//! The mock engine interprets the program text as the name of a scripted
//! behavior:
//!
//! | Program | Behavior |
//! |---|---|
//! | `ok` | report `ok`, rendered text `ok` |
//! | `fail` | report with the language error `E_SYNTAX` |
//! | `call-error` | call error `NURT_E_IO` |
//! | `progress` | emits stage progress, job progress, job output and two log messages, then `ok` |
//! | `wait` | blocks until the call is cancelled, then reports `E_CANCELLED` |
//! | `progress-wait` | emits stage progress, then behaves as `wait` |
//! | `rendezvous` | waits until the configured number of calls run at once |
//! | `session` | rendered text is the engine session of the request |

// Every test crate uses a different subset of these helpers.
#![allow(dead_code)]
// Helpers abort the test on a broken expectation, as test code does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use nurt_contracts::envelope::{OutputFormat, Rendered, Request, Response};
use nurt_contracts::error::{AbiStatus, Error, ErrorCode};
use nurt_contracts::report::{Report, ReportStatus};
use nurt_contracts::stage::{
    CancelToken, Event, EventSink, JobOutput, JobProgress, LogEvent, LogLevel, OutputStream,
    Progress,
};
use nurt_mcp::{
    Backend, BackendInfo, CallError, CheckOutcome, DescribeSubject, HelpQuery, McpError,
    McpOptions, SchemaName, serve_io,
};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio::task::JoinHandle;

/// Version reported by the mock engine.
pub(crate) const ENGINE_VERSION: &str = "1.4.0";

/// Core description returned by the mock engine.
pub(crate) const CORE_DESCRIPTION: &str = "Runs programs of the Nurt language.";

/// Longest wait of a test for a message or a scripted condition.
pub(crate) const WAIT: Duration = Duration::from_secs(10);

/// Counter that tests can wait on.
#[derive(Default)]
pub(crate) struct Counter {
    value: Mutex<usize>,
    changed: Condvar,
}

impl Counter {
    pub(crate) fn increment(&self) {
        *self.value.lock().unwrap() += 1;
        self.changed.notify_all();
    }

    pub(crate) fn get(&self) -> usize {
        *self.value.lock().unwrap()
    }

    /// Waits until the counter reaches `target`; `false` on timeout.
    pub(crate) fn wait_for(&self, target: usize) -> bool {
        let guard = self.value.lock().unwrap();
        let (guard, _) = self
            .changed
            .wait_timeout_while(guard, WAIT, |value| *value < target)
            .unwrap();
        *guard >= target
    }
}

/// Scripted engine.
pub(crate) struct MockBackend {
    pub(crate) max_inflight: usize,
    pub(crate) rendezvous: usize,
    pub(crate) started: Counter,
    pub(crate) cancelled: Counter,
    arrived: Counter,
}

impl MockBackend {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_limits(8, 2)
    }

    pub(crate) fn with_limits(max_inflight: usize, rendezvous: usize) -> Arc<Self> {
        Arc::new(Self {
            max_inflight,
            rendezvous,
            started: Counter::default(),
            cancelled: Counter::default(),
            arrived: Counter::default(),
        })
    }

    /// Waits until `count` calls have started; `false` on timeout.
    pub(crate) async fn wait_started(self: &Arc<Self>, count: usize) -> bool {
        let backend = Arc::clone(self);
        tokio::task::spawn_blocking(move || backend.started.wait_for(count))
            .await
            .unwrap()
    }

    /// Waits until `count` calls have observed their cancellation; `false`
    /// on timeout.
    pub(crate) async fn wait_cancelled(self: &Arc<Self>, count: usize) -> bool {
        let backend = Arc::clone(self);
        tokio::task::spawn_blocking(move || backend.cancelled.wait_for(count))
            .await
            .unwrap()
    }
}

impl MockBackend {
    /// Blocks until the call is cancelled; reports `ok` after [`WAIT`].
    fn wait_for_cancellation(&self, request: &Request, cancel: &CancelToken) -> Response {
        match cancel.wait_timeout(WAIT) {
            Some(reason) => {
                self.cancelled.increment();
                response(
                    request,
                    failed(reason.error_code(), "the execution was cancelled"),
                    "cancelled",
                )
            }
            None => response(request, Report::new(ReportStatus::Ok, "find"), "ok"),
        }
    }
}

fn response(request: &Request, report: Report, text: &str) -> Response {
    let mut response = Response::new(report);
    let format = request.options.format();
    if format != OutputFormat::Json {
        response.rendered = Some(Rendered {
            format,
            text: text.to_owned(),
        });
    }
    response
}

fn failed(code: ErrorCode, message: &str) -> Report {
    let mut report = Report::new(ReportStatus::Failed, "find");
    report.error = Some(Error::new(code, message));
    report
}

impl Backend for MockBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            version: ENGINE_VERSION.to_owned(),
            max_inflight: self.max_inflight,
        }
    }

    fn run(
        &self,
        request: &Request,
        events: Arc<dyn EventSink>,
        cancel: CancelToken,
    ) -> Result<Response, CallError> {
        self.started.increment();
        let ok = || response(request, Report::new(ReportStatus::Ok, "find"), "ok");
        match request.program.as_deref().unwrap_or_default() {
            "ok" => Ok(ok()),
            "fail" => Ok(response(
                request,
                failed(ErrorCode::Syntax, "unexpected token"),
                "error E_SYNTAX: unexpected token",
            )),
            "call-error" => Err(CallError::new(AbiStatus::Io, "journal is not writable")),
            "progress" => {
                events.emit(Event::Progress(Progress {
                    command: "find".to_owned(),
                    phase: "scan".to_owned(),
                    done: 1,
                    total: Some(2),
                    bytes: None,
                    message: None,
                }));
                events.emit(Event::JobProgress(JobProgress {
                    id: "j1".to_owned(),
                    percent: Some(50.0),
                    line: Some("half done".to_owned()),
                }));
                events.emit(Event::JobOutput(JobOutput {
                    id: "j1".to_owned(),
                    stream: OutputStream::Stdout,
                    cursor: "c1".to_owned(),
                    lines: vec!["line one".to_owned()],
                    bytes: 9,
                }));
                events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: "slow volume".to_owned(),
                    source: Some("space".to_owned()),
                }));
                events.emit(Event::Log(LogEvent {
                    level: LogLevel::Info,
                    message: "scan finished".to_owned(),
                    source: None,
                }));
                Ok(ok())
            }
            "wait" => Ok(self.wait_for_cancellation(request, &cancel)),
            "progress-wait" => {
                events.emit(Event::Progress(Progress {
                    command: "find".to_owned(),
                    phase: "scan".to_owned(),
                    done: 1,
                    total: None,
                    bytes: None,
                    message: Some("scanning".to_owned()),
                }));
                Ok(self.wait_for_cancellation(request, &cancel))
            }
            "rendezvous" => {
                self.arrived.increment();
                let text = if self.arrived.wait_for(self.rendezvous) {
                    "met"
                } else {
                    "alone"
                };
                Ok(response(
                    request,
                    Report::new(ReportStatus::Ok, "find"),
                    text,
                ))
            }
            "session" => {
                let session = request.options.session.clone().unwrap_or_default();
                Ok(response(
                    request,
                    Report::new(ReportStatus::Ok, "find"),
                    &session,
                ))
            }
            other => Ok(response(
                request,
                failed(ErrorCode::Syntax, &format!("unknown program {other}")),
                "unknown program",
            )),
        }
    }

    fn help(&self, query: &HelpQuery) -> Result<String, CallError> {
        Ok(format!(
            "help topic={} kind={} examples={} language={}",
            query.topic.as_deref().unwrap_or("index"),
            query.kind.map_or("any", |kind| kind.as_str()),
            query.examples.unwrap_or(3),
            query.language.unwrap_or_default().as_str(),
        ))
    }

    fn describe(&self, subject: &DescribeSubject) -> Result<String, CallError> {
        match subject {
            DescribeSubject::Core => Ok(CORE_DESCRIPTION.to_owned()),
            DescribeSubject::Group(name) if name == "git" => {
                Ok("Group git: repository commands.".to_owned())
            }
            DescribeSubject::Group(name) => Err(CallError::new(
                AbiStatus::Arg,
                format!("unknown group `{name}`"),
            )),
            _ => Err(CallError::new(AbiStatus::Arg, "unknown layer")),
        }
    }

    fn check(&self, request: &Request) -> Result<CheckOutcome, CallError> {
        match request.program.as_deref().or(request.accept.as_deref()) {
            Some("fail") => Ok(CheckOutcome::Failed(Error::new(
                ErrorCode::Syntax,
                "unexpected token",
            ))),
            Some("call-error") => Err(CallError::new(AbiStatus::Io, "journal is not writable")),
            Some(program) => Ok(CheckOutcome::Report(json!({
                "format": "nurt-check/1",
                "effect": "read",
                "reversibility": "read",
                "operations": 1,
                "program": program,
                "session": request.options.session,
            }))),
            None => Err(CallError::new(AbiStatus::Arg, "nothing to check")),
        }
    }

    fn schema(&self, name: SchemaName) -> Result<String, CallError> {
        match name {
            SchemaName::Tool => Ok(r#"{"title":"nurt-tool/1"}"#.to_owned()),
            SchemaName::Config => Err(CallError::new(AbiStatus::Io, "schema store unreadable")),
            other => Ok(nurt_contracts::schema::by_name(other.as_str())
                .unwrap_or("{}")
                .to_owned()),
        }
    }
}

/// Client of a server that runs on an in-memory stream.
pub(crate) struct Client {
    writer: Option<WriteHalf<DuplexStream>>,
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    server: JoinHandle<Result<(), McpError>>,
}

impl Client {
    pub(crate) fn start(backend: Arc<MockBackend>) -> Self {
        Self::start_with(backend, McpOptions::new())
    }

    pub(crate) fn start_with(backend: Arc<MockBackend>, options: McpOptions) -> Self {
        let (client_side, server_side) = tokio::io::duplex(1 << 16);
        let (server_reader, server_writer) = tokio::io::split(server_side);
        let server = tokio::spawn(serve_io(backend, options, server_reader, server_writer));
        let (reader, writer) = tokio::io::split(client_side);
        Self {
            writer: Some(writer),
            lines: BufReader::new(reader).lines(),
            server,
        }
    }

    pub(crate) async fn send_raw(&mut self, text: &str) {
        let writer = self.writer.as_mut().unwrap();
        writer.write_all(text.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
    }

    pub(crate) async fn send(&mut self, message: &Value) {
        self.send_raw(&message.to_string()).await;
    }

    /// Next message of the server; `None` when the server closed the stream.
    pub(crate) async fn next(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(WAIT, self.lines.next_line())
            .await
            .expect("no message from the server in time")
            .unwrap()?;
        Some(serde_json::from_str(&line).expect("server wrote a line that is not JSON"))
    }

    pub(crate) async fn recv(&mut self) -> Value {
        self.next().await.expect("server closed the stream")
    }

    /// Sends a request and returns its response with the notifications
    /// received before it.
    pub(crate) async fn call(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> (Value, Vec<Value>) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        let mut notifications = Vec::new();
        loop {
            let message = self.recv().await;
            if message.get("id") == Some(&json!(id)) && message.get("method").is_none() {
                return (message, notifications);
            }
            notifications.push(message);
        }
    }

    pub(crate) async fn initialize(&mut self) -> Value {
        let (response, _) = self
            .call(
                0,
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1.0.0"}
                }),
            )
            .await;
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        response
    }

    pub(crate) async fn tool(
        &mut self,
        id: u64,
        name: &str,
        arguments: Value,
    ) -> (Value, Vec<Value>) {
        self.call(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
    }

    /// Sends `notifications/cancelled` for the request `id`.
    pub(crate) async fn cancel(&mut self, id: u64) {
        self.send(&json!({
            "jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": id, "reason": "stopped by the client"}
        }))
        .await;
    }

    /// Closes the input of the server and returns what the server ended with.
    pub(crate) async fn close(mut self) -> (Result<(), McpError>, Vec<Value>) {
        if let Some(mut writer) = self.writer.take() {
            writer.shutdown().await.unwrap();
        }
        let mut rest = Vec::new();
        while let Some(message) = self.next().await {
            rest.push(message);
        }
        let result = tokio::time::timeout(WAIT, self.server)
            .await
            .expect("server did not stop in time")
            .unwrap();
        (result, rest)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
