// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::serve::backend
// Responsibility:   Engines behind the server and the delivery of their events.
// Dependencies:     nurt_contracts, nurt_core, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Engines behind the server and the delivery of their events.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::envelope::{Request, RequestOptions};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::HistoryQuery;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{CancelToken, DeliveredEvent, Event, ExecFinished};
use nurt_contracts::value::Instant;
use nurt_core::{Engine, EventBody, EventListener, Lifecycle};
use serde_json::{Map, Value};

use super::call::{
    self, Call, CatalogSelection, DEFAULT_SUBJECT, PageParams, PrintParams, SpaceParams,
};
use super::protocol::{self, EVENT_METHOD, RpcError};

/// Interval at which a running request checks whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(10);

/// What `initialize` reports about an engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Summary {
    pub(crate) version: String,
    /// Version of the C interface built into the server, when it has one.
    pub(crate) abi: Option<u32>,
    pub(crate) features: Vec<String>,
    /// Command groups whose services the engine provides.
    pub(crate) groups: Vec<String>,
}

/// Cancellation, session and event delivery of one request.
pub(crate) struct CallContext<'a> {
    pub(crate) cancel: &'a CancelToken,
    pub(crate) events: &'a Arc<Events>,
    /// Session given in `initialize`, used by requests without their own.
    pub(crate) session: Option<&'a str>,
}

/// Engine answering the methods of the protocol.
pub(crate) trait Backend: Send + Sync {
    /// Version and features reported by `initialize`.
    fn summary(&self) -> Summary;

    /// Runs one call. Language errors are part of the returned result.
    fn call(&self, call: Call, context: &CallContext<'_>) -> Result<Value, RpcError>;
}

/// Sends the events of one request as `nurt/event` notifications, or drops
/// them when the client did not ask for events.
pub(crate) struct Events {
    sender: Option<Sender<Value>>,
    request: Option<String>,
    /// The client cancelled the request; a failed `exec.finished` then
    /// carries `E_CANCELLED`, which the engine event does not state.
    cancelled: AtomicBool,
}

impl Events {
    /// Events of the request `id`; `sender` is `None` when events are off.
    pub(crate) fn new(sender: Option<Sender<Value>>, id: Option<&Value>) -> Self {
        let request = id.and_then(|id| match id {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        });
        Self {
            sender,
            request,
            cancelled: AtomicBool::new(false),
        }
    }

    /// Records that the client cancelled the request.
    pub(crate) fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Identifier of the request carried by its events: the `id` of the
    /// request as text.
    pub(crate) fn request(&self) -> Option<String> {
        self.request.clone()
    }

    /// Sends one delivered event.
    pub(crate) fn send(&self, event: &DeliveredEvent) {
        if let Ok(params) = serde_json::to_value(event) {
            self.send_json(params);
        }
    }

    fn send_json(&self, params: Value) {
        if let Some(sender) = &self.sender {
            // The receiver is gone only when the output of the server failed.
            let _ = sender.send(protocol::notification(EVENT_METHOD, params));
        }
    }
}

impl EventListener for Events {
    fn deliver(&self, delivered: nurt_core::DeliveredEvent) {
        if self.sender.is_none() {
            return;
        }
        let nanos = i64::try_from(delivered.time.unix_nanos()).unwrap_or(i64::MAX);
        let time = Instant::from_unix_nanos(nanos);
        let event = match delivered.body {
            EventBody::Stage(event) => event,
            EventBody::Lifecycle(Lifecycle::Finished { status, error }) => {
                let cancelled = self.cancelled.load(Ordering::Acquire)
                    && matches!(status, ReportStatus::Failed | ReportStatus::RolledBack);
                Event::ExecFinished(ExecFinished {
                    status,
                    error: error.or(cancelled.then_some(ErrorCode::Cancelled)),
                })
            }
            body => {
                // The engine reports the start without the effect class of
                // the program, so the start carries the members it has.
                let Ok(Value::Object(body)) = serde_json::to_value(&body) else {
                    return;
                };
                let mut params = Map::new();
                params.insert("seq".to_owned(), Value::from(delivered.seq));
                params.insert("time".to_owned(), Value::from(time.to_string()));
                params.insert("request".to_owned(), self.request.clone().into());
                params.extend(body);
                self.send_json(Value::Object(params));
                return;
            }
        };
        self.send(&DeliveredEvent {
            seq: delivered.seq,
            time,
            request: self.request(),
            event,
        });
    }
}

/// The engine API.
pub(crate) struct EngineBackend {
    engine: Engine,
}

impl EngineBackend {
    pub(crate) fn new(engine: Engine) -> Self {
        Self { engine }
    }

    fn run(&self, request: &Request, context: &CallContext<'_>) -> Result<Value, RpcError> {
        let listener: Arc<dyn EventListener> = Arc::clone(context.events) as Arc<dyn EventListener>;
        let handle = self.engine.start(request, listener);
        loop {
            if context.cancel.is_cancelled() {
                context.events.mark_cancelled();
                handle.cancel();
            }
            if let Some(response) = handle.wait(Some(CANCEL_POLL)) {
                return call::run_result(call::to_json(&response)?);
            }
        }
    }

    fn check(&self, request: &Request) -> Result<Value, RpcError> {
        match self.engine.check(request) {
            Ok(report) => Ok(call::merge(call::to_json(&report)?, no_diagnostics())),
            Err(error) => call::failure("check", error),
        }
    }

    fn print(&self, params: PrintParams) -> Result<Value, RpcError> {
        let ast: AstDocument = serde_json::from_value(params.ast).map_err(|error| {
            RpcError::invalid_params(format!("`ast` is not a syntax tree nurt-ast/1: {error}"))
        })?;
        let printed = self.engine.print(&ast, params.variant);
        call::answer(
            "print",
            printed.map(|text| call::member("text", text.into())),
        )
    }

    fn page(&self, params: &PageParams, session: Option<&str>) -> Result<Value, RpcError> {
        let options = RequestOptions {
            limit: params.limit.filter(|limit| *limit > 0),
            session: session.map(str::to_owned),
            ..RequestOptions::default()
        };
        call::to_json(&self.engine.page(&params.cursor, &options))
    }

    fn catalog(&self, what: &Value) -> Result<Value, RpcError> {
        let selection = CatalogSelection::from_value(what)?;
        call::answer("catalog", self.engine.catalog_query(&selection.query()))
    }

    fn history(&self, query: Value) -> Result<Value, RpcError> {
        let query: HistoryQuery = if query.is_null() {
            HistoryQuery::default()
        } else {
            serde_json::from_value(query).map_err(|error| {
                RpcError::invalid_params(format!("invalid parameters of `history`: {error}"))
            })?
        };
        let operations = self
            .engine
            .history(&query)
            .and_then(|operations| json_value(&operations));
        call::answer(
            "history",
            operations.map(|operations| call::member("operations", operations)),
        )
    }

    fn space(&self, params: SpaceParams) -> Result<Value, RpcError> {
        let volume = match params.volume {
            None => None,
            Some(volume) => match AbsPath::new(volume.as_str()) {
                Ok(path) => Some(path),
                Err(_) => {
                    let error = Error::new(
                        ErrorCode::PathRelative,
                        format!("`volume` must be an absolute path; given `{volume}`"),
                    )
                    .with_found(volume);
                    return call::failure("space", error);
                }
            },
        };
        let volumes = self
            .engine
            .space(volume.as_ref())
            .and_then(|volumes| json_value(&volumes));
        call::answer(
            "space",
            volumes.map(|volumes| call::member("volumes", volumes)),
        )
    }
}

impl Backend for EngineBackend {
    fn summary(&self) -> Summary {
        let info = self.engine.info();
        Summary {
            version: info.version,
            abi: None,
            features: info.features,
            groups: info.services,
        }
    }

    fn call(&self, call: Call, context: &CallContext<'_>) -> Result<Value, RpcError> {
        let kind = call.method();
        match call {
            Call::Run(request) => self.run(&with_session(*request, context.session), context),
            Call::Check(request) => self.check(&with_session(*request, context.session)),
            Call::Parse(params) => {
                let parsed = self.engine.parse(&params.program, params.variant);
                let parsed = parsed.and_then(|ast| json_value(&ast));
                call::answer(
                    kind,
                    parsed.map(|ast| call::merge(call::member("ast", ast), no_diagnostics())),
                )
            }
            Call::Print(params) => self.print(params),
            Call::Correct(params) => {
                let corrected = self.engine.correct(&params.program, params.variant);
                call::answer(kind, corrected.and_then(|corrected| json_value(&corrected)))
            }
            Call::Page(params) => self.page(&params, context.session),
            Call::Describe(params) => {
                let subject = params.subject.as_deref().unwrap_or(DEFAULT_SUBJECT);
                let described = self.engine.describe(subject, params.language);
                call::answer(
                    kind,
                    described.map(|description| call::member("description", description)),
                )
            }
            Call::Help(params) => {
                let topic = params.topic.as_deref().unwrap_or(DEFAULT_SUBJECT);
                let help = self.engine.help(topic, params.language);
                call::answer(kind, help.map(|text| call::member("text", text.into())))
            }
            Call::Schema(params) => call::schema(&params.name),
            Call::Catalog(what) => self.catalog(&what),
            Call::History(query) => self.history(query),
            Call::Space(params) => self.space(params),
            Call::Info => call::to_json(&self.engine.info()),
            Call::Rejected { method, error } => call::failure(method, *error),
        }
    }
}

/// `request` with the session of the connection when it names none.
fn with_session(mut request: Request, session: Option<&str>) -> Request {
    if request.options.session.is_none() {
        request.options.session = session.map(str::to_owned);
    }
    request
}

/// Diagnostics member of `parse` and `check`: the engine reports problems
/// as errors, so the list is empty.
fn no_diagnostics() -> Value {
    call::member("diagnostics", Value::Array(Vec::new()))
}

/// JSON form of an engine result, as a language error when it fails.
fn json_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|error| Error::new(ErrorCode::Internal, error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use nurt_contracts::env::WallTime;
    use nurt_contracts::stage::{LogEvent, LogLevel};
    use serde_json::json;

    use super::*;

    fn delivered(seq: u64, body: EventBody) -> nurt_core::DeliveredEvent {
        nurt_core::DeliveredEvent {
            seq,
            time: WallTime::from_unix_nanos(1_000_000_000),
            request_id: "r1".to_owned(),
            body,
        }
    }

    #[test]
    fn engine_events_are_sent_in_the_contract_form() {
        let (sender, receiver) = mpsc::channel();
        let events = Events::new(Some(sender), Some(&json!(7)));
        assert_eq!(events.request().as_deref(), Some("7"));
        events.deliver(delivered(1, EventBody::Lifecycle(Lifecycle::Started)));
        events.deliver(delivered(
            2,
            EventBody::Stage(Event::Log(LogEvent {
                level: LogLevel::Info,
                message: "scan".to_owned(),
                source: None,
            })),
        ));
        events.deliver(delivered(
            3,
            EventBody::Lifecycle(Lifecycle::Finished {
                status: ReportStatus::Ok,
                error: None,
            }),
        ));
        let messages: Vec<Value> = receiver.try_iter().collect();
        assert!(
            messages
                .iter()
                .all(|message| message["method"] == EVENT_METHOD)
        );
        let started = &messages[0]["params"];
        assert_eq!(started["t"], "exec.started");
        assert_eq!(started["seq"], 1);
        assert_eq!(started["time"], "1970-01-01T00:00:01Z");
        assert_eq!(started["request"], "7");
        let log: DeliveredEvent = serde_json::from_value(messages[1]["params"].clone()).unwrap();
        assert_eq!(log.seq, 2);
        assert_eq!(
            messages[2]["params"],
            json!({"seq": 3, "time": "1970-01-01T00:00:01Z", "request": "7", "t": "exec.finished", "status": "ok", "error": null})
        );
    }

    #[test]
    fn a_cancelled_request_finishes_with_the_cancelled_error_code() {
        let finished = |cancelled: bool, status: ReportStatus| {
            let (sender, receiver) = mpsc::channel();
            let events = Events::new(Some(sender), Some(&json!(7)));
            if cancelled {
                events.mark_cancelled();
            }
            events.deliver(delivered(
                1,
                EventBody::Lifecycle(Lifecycle::Finished {
                    status,
                    error: None,
                }),
            ));
            receiver.try_iter().next().unwrap()["params"]["error"].clone()
        };
        assert_eq!(finished(true, ReportStatus::Failed), json!("E_CANCELLED"));
        assert_eq!(finished(false, ReportStatus::Failed), Value::Null);
        assert_eq!(finished(true, ReportStatus::Ok), Value::Null);
    }

    #[test]
    fn the_request_of_an_event_is_null_for_a_notification() {
        let (sender, receiver) = mpsc::channel();
        let events = Events::new(Some(sender), None);
        events.deliver(delivered(4, EventBody::Lifecycle(Lifecycle::Started)));
        let message = receiver.try_recv().unwrap();
        assert!(message["params"]["request"].is_null());
        assert_eq!(message["params"]["t"], "exec.started");
        assert_eq!(message["params"]["seq"], 4);
        assert_eq!(message["jsonrpc"], "2.0");
    }

    #[test]
    fn identifiers_that_are_not_text_or_numbers_name_no_request() {
        assert_eq!(Events::new(None, Some(&json!(true))).request(), None);
        assert_eq!(Events::new(None, Some(&json!([1]))).request(), None);
        assert_eq!(
            Events::new(None, Some(&json!(2.5))).request().as_deref(),
            Some("2.5")
        );
        assert_eq!(
            Events::new(None, Some(&json!("\u{142}\u{f3}d\u{17a}")))
                .request()
                .as_deref(),
            Some("\u{142}\u{f3}d\u{17a}")
        );
    }

    #[test]
    fn events_for_a_client_that_is_gone_are_dropped() {
        let (sender, receiver) = mpsc::channel();
        let events = Events::new(Some(sender), Some(&json!(1)));
        drop(receiver);
        events.deliver(delivered(1, EventBody::Lifecycle(Lifecycle::Started)));
        events.deliver(delivered(
            2,
            EventBody::Lifecycle(Lifecycle::Finished {
                status: ReportStatus::Failed,
                error: None,
            }),
        ));
    }

    #[test]
    fn the_end_of_an_execution_names_its_status() {
        let (sender, receiver) = mpsc::channel();
        let events = Events::new(Some(sender), Some(&json!("r")));
        events.deliver(delivered(
            2,
            EventBody::Lifecycle(Lifecycle::Finished {
                status: ReportStatus::Failed,
                error: Some(ErrorCode::Syntax),
            }),
        ));
        let message = receiver.try_recv().unwrap();
        let params = &message["params"];
        assert_eq!(params["t"], "exec.finished");
        assert_eq!(params["status"], "failed");
        assert_eq!(params["error"], "E_SYNTAX");
        assert_eq!(params["request"], "r");
    }

    #[test]
    fn events_are_dropped_when_disabled() {
        let events = Events::new(None, Some(&json!("a")));
        assert_eq!(events.request().as_deref(), Some("a"));
        events.deliver(delivered(1, EventBody::Lifecycle(Lifecycle::Started)));
        assert_eq!(Events::new(None, Some(&Value::Null)).request(), None);
        assert_eq!(Events::new(None, None).request(), None);
    }

    #[test]
    fn the_session_of_the_connection_fills_requests_without_one() {
        let filled = with_session(Request::program("count"), Some("s1"));
        assert_eq!(filled.options.session.as_deref(), Some("s1"));
        let mut own = Request::program("count");
        own.options.session = Some("own".to_owned());
        let kept = with_session(own, Some("s1"));
        assert_eq!(kept.options.session.as_deref(), Some("own"));
        assert_eq!(
            with_session(Request::program("count"), None)
                .options
                .session,
            None
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
