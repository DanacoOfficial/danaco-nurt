// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Operations of the engine in the JSON form used by the Node.js binding.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::envelope::{Language, Request, RequestOptions};
use nurt_contracts::error::{AbiStatus, Error};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::HistoryQuery;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::value::Instant;
use nurt_core::{DeliveredEvent, Engine, EngineConfig, EngineError, EventListener, ExecHandle};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::system::{SystemClock, SystemHost};

/// Subject of `describe` and topic of `help` when none is given.
const DEFAULT_SUBJECT: &str = "core";

/// Failure of a call, as opposed to an error of the Nurt language.
///
/// The status uses the names of `nurt_status`; the JavaScript package maps
/// them to its `NurtError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    status: Status,
    message: String,
}

/// Status of a failed call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `NURT_E_TIMEOUT`.
    Timeout,
    /// `NURT_E_CANCELLED`.
    Cancelled,
    /// `NURT_E_ARG`.
    Arg,
    /// `NURT_E_ABI`.
    Abi,
    /// `NURT_E_IO`.
    Io,
    /// `NURT_E_INTERNAL`.
    Internal,
}

impl Status {
    /// Name of the status in `nurt.h`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Timeout => "NURT_E_TIMEOUT",
            Self::Cancelled => "NURT_E_CANCELLED",
            Self::Arg => "NURT_E_ARG",
            Self::Abi => "NURT_E_ABI",
            Self::Io => "NURT_E_IO",
            Self::Internal => "NURT_E_INTERNAL",
        }
    }
}

impl From<AbiStatus> for Status {
    fn from(status: AbiStatus) -> Self {
        match status {
            AbiStatus::Timeout => Self::Timeout,
            AbiStatus::Cancelled => Self::Cancelled,
            AbiStatus::Arg => Self::Arg,
            AbiStatus::Abi => Self::Abi,
            AbiStatus::Io => Self::Io,
            _ => Self::Internal,
        }
    }
}

impl Failure {
    fn new(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn arg(message: impl Into<String>) -> Self {
        Self::new(Status::Arg, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(Status::Internal, message)
    }

    /// Status of the failure.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// Description of the failure.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<&EngineError> for Failure {
    fn from(error: &EngineError) -> Self {
        Self::new(error.status().into(), error.message())
    }
}

impl fmt::Display for Failure {
    /// Formats the status name, a colon and the description; the JavaScript
    /// package parses this form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status.name(), self.message)
    }
}

impl std::error::Error for Failure {}

/// Options given to [`Native::open`] as a JSON object.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenOptions {
    /// Session of requests that do not name one.
    session: Option<String>,
    /// State directory, instead of the configured one.
    state_dir: Option<String>,
}

#[derive(Deserialize)]
struct ProgramParams {
    program: String,
    variant: Option<SyntaxVariant>,
}

#[derive(Deserialize)]
struct PrintParams {
    ast: AstDocument,
    variant: Option<SyntaxVariant>,
}

#[derive(Deserialize)]
struct PageParams {
    cursor: String,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct SubjectParams {
    subject: Option<String>,
    language: Option<Language>,
}

#[derive(Deserialize)]
struct TopicParams {
    topic: Option<String>,
    language: Option<Language>,
}

#[derive(Deserialize)]
struct SchemaParams {
    name: String,
}

#[derive(Deserialize)]
struct SpaceParams {
    volume: Option<String>,
}

/// An engine and the operations the addon exposes on it.
///
/// Cloning is cheap; clones share the engine.
#[derive(Clone)]
pub struct Native {
    engine: Engine,
}

impl fmt::Debug for Native {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Native").finish_non_exhaustive()
    }
}

/// Name of the thread that waits for a response and reports it.
const RESPONSE_THREAD_NAME: &str = "nurt-node-response";

/// Item of the ordered stream of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// An event as JSON text, in the form of `encode_event`.
    Event(String),
    /// The response as JSON text, or the failure of the wait. Always the last
    /// message of a request.
    Response(Result<String, Failure>),
}

/// Receiver of the ordered stream of a request.
pub type Sink = Arc<dyn Fn(Message) + Send + Sync>;

/// Passes the events of the engine to a [`Sink`].
struct Relay(Sink);

impl EventListener for Relay {
    fn deliver(&self, event: DeliveredEvent) {
        if let Ok(text) = encode_event(&event) {
            (self.0)(Message::Event(text));
        }
    }
}

/// Result of an operation that can fail with a language error.
type Answer = Result<Value, Error>;

impl Native {
    /// Opens an engine with the clock and the host of the operating system.
    ///
    /// `options` is absent or a JSON object with the optional members
    /// `session` (the session of requests that name none) and `state_dir`.
    ///
    /// # Errors
    ///
    /// Returns `NURT_E_ARG` for options that are not valid and the status of
    /// the engine for a configuration or a state directory that cannot be
    /// used.
    pub fn open(options: Option<&str>) -> Result<Self, Failure> {
        let options: OpenOptions = match options {
            Some(text) => parse(text, "the options")?,
            None => OpenOptions::default(),
        };
        let mut config =
            EngineConfig::new(Arc::new(SystemClock::new()), Arc::new(SystemHost::new()));
        if let Some(session) = options.session {
            config = config.with_session(session);
        }
        if let Some(dir) = options.state_dir {
            config = config.with_state_dir(dir);
        }
        Self::open_with(config)
    }

    /// Opens an engine from a configuration assembled by the caller, for
    /// example with a language front end and the services of the command
    /// groups.
    ///
    /// # Errors
    ///
    /// Returns the status of the engine for a configuration or a state
    /// directory that cannot be used.
    pub fn open_with(config: EngineConfig) -> Result<Self, Failure> {
        Engine::open(config)
            .map(Self::from_engine)
            .map_err(|error| Failure::from(&error))
    }

    /// Wraps an engine that is already open.
    #[must_use]
    pub fn from_engine(engine: Engine) -> Self {
        Self { engine }
    }

    /// Description of the engine (`EngineInfo`) as JSON text.
    ///
    /// # Errors
    ///
    /// Returns `NURT_E_INTERNAL` when the description cannot be serialized.
    pub fn info(&self) -> Result<String, Failure> {
        to_text(&self.engine.info())
    }

    /// Starts a request on its own thread.
    ///
    /// `request` is a request envelope (`nurt-request/1`) as JSON text.
    /// Events are delivered to `listener` from `exec.started` to
    /// `exec.finished`.
    ///
    /// # Errors
    ///
    /// Returns `NURT_E_ARG` when the text is not a valid request envelope.
    pub fn start(
        &self,
        request: &str,
        listener: Arc<dyn EventListener>,
    ) -> Result<ExecHandle, Failure> {
        let request: Request = parse(request, "the request")?;
        Ok(self.engine.start(&request, listener))
    }

    /// Starts a request and reports its events and its response, in this
    /// order, to one `sink`.
    ///
    /// The engine delivers `exec.finished` before it makes the response
    /// available, so the response is passed to `sink` only after the sink has
    /// returned from every event of the request. `sink` is called from the
    /// execution thread for events and from a separate thread for the
    /// response, never concurrently; the last call is always
    /// [`Message::Response`].
    ///
    /// # Errors
    ///
    /// Returns `NURT_E_ARG` when the text is not a valid request envelope and
    /// `NURT_E_INTERNAL` when the thread that reports the response cannot be
    /// started.
    pub fn start_ordered(&self, request: &str, sink: Sink) -> Result<ExecHandle, Failure> {
        let handle = self.start(request, Arc::new(Relay(Arc::clone(&sink))))?;
        let waiting = handle.clone();
        std::thread::Builder::new()
            .name(RESPONSE_THREAD_NAME.to_owned())
            .spawn(move || sink(Message::Response(guarded(|| finish(&waiting, None)))))
            .map_err(|error| {
                handle.cancel();
                Failure::internal(format!("cannot start the response thread: {error}"))
            })?;
        Ok(handle)
    }

    /// Runs one of the operations other than `run` and returns the result as
    /// JSON text.
    ///
    /// The operations are `check`, `parse`, `print`, `correct`, `page`,
    /// `describe`, `help`, `catalog`, `schema`, `history`, `space` and `info`;
    /// `input` holds the parameters of the method of the same name in
    /// `nurt-rpc/1`. The result is `{"result": ...}`, or `{"error": ...}` with
    /// the error of the Nurt language. `history` also accepts
    /// `{"query": "<JSON text of a history query>"}`; the results of
    /// `history` and `space` are given as `{"operations": [...]}` and
    /// `{"volumes": [...]}`.
    ///
    /// # Errors
    ///
    /// Returns `NURT_E_ARG` for an unknown operation or parameters that are
    /// not valid.
    pub fn call(&self, operation: &str, input: &str) -> Result<String, Failure> {
        guarded(|| self.dispatch(operation, input))
    }

    fn dispatch(&self, operation: &str, input: &str) -> Result<String, Failure> {
        let input: Value = parse(input, "the parameters")?;
        let engine = &self.engine;
        let outcome: Answer = match operation {
            "check" => answer(engine.check(&from_value::<Request>(&input, "the request")?))?,
            "parse" => {
                let params: ProgramParams = from_value(&input, "the parameters of parse")?;
                answer(engine.parse(&params.program, params.variant))?
            }
            "print" => {
                let params: PrintParams = from_value(&input, "the parameters of print")?;
                answer(engine.print(&params.ast, params.variant))?
            }
            "correct" => {
                let params: ProgramParams = from_value(&input, "the parameters of correct")?;
                answer(engine.correct(&params.program, params.variant))?
            }
            "page" => {
                let params: PageParams = from_value(&input, "the parameters of page")?;
                let options = RequestOptions {
                    limit: params.limit.filter(|limit| *limit > 0),
                    ..RequestOptions::default()
                };
                Ok(to_value(&engine.page(&params.cursor, &options))?)
            }
            "describe" => {
                let params: SubjectParams = from_optional(&input, "the parameters of describe")?;
                let subject = params.subject.as_deref().unwrap_or(DEFAULT_SUBJECT);
                engine.describe(subject, params.language)
            }
            "help" => {
                let params: TopicParams = from_optional(&input, "the parameters of help")?;
                let topic = params.topic.as_deref().unwrap_or(DEFAULT_SUBJECT);
                answer(engine.help(topic, params.language))?
            }
            "catalog" => engine.catalog(),
            "schema" => {
                let params: SchemaParams = from_value(&input, "the parameters of schema")?;
                match engine.schema(&params.name) {
                    Ok(text) => Ok(parse::<Value>(text, "the schema")
                        .map_err(|failure| Failure::internal(failure.message))?),
                    Err(error) => Err(error),
                }
            }
            "history" => {
                let query = history_query(&input)?;
                answer(
                    engine
                        .history(&query)
                        .map(|operations| json!({ "operations": operations })),
                )?
            }
            "space" => {
                let params: SpaceParams = from_optional(&input, "the parameters of space")?;
                let volume = params
                    .volume
                    .map(|path| from_value::<AbsPath>(&Value::String(path), "the volume"))
                    .transpose()?;
                answer(
                    engine
                        .space(volume.as_ref())
                        .map(|volumes| json!({ "volumes": volumes })),
                )?
            }
            "info" => Ok(to_value(&engine.info())?),
            other => return Err(Failure::arg(format!("unknown operation `{other}`"))),
        };
        let document = match outcome {
            Ok(result) => json!({ "result": result }),
            Err(error) => json!({ "error": error }),
        };
        to_text(&document)
    }
}

/// Waits for the response of a started request and returns it as JSON text.
///
/// # Errors
///
/// Returns `NURT_E_TIMEOUT` when the request has not finished within
/// `timeout` and `NURT_E_INTERNAL` when the response cannot be serialized.
pub fn finish(handle: &ExecHandle, timeout: Option<Duration>) -> Result<String, Failure> {
    let response = handle.wait(timeout).ok_or_else(|| {
        Failure::new(
            Status::Timeout,
            "the execution has not finished within the time limit",
        )
    })?;
    to_text(&response)
}

/// Encodes an event as JSON text in the form of `nurt-event/1`: the time as
/// an RFC 3339 instant in UTC and the request identifier as `request`.
///
/// The event of the engine carries the time as a number of nanoseconds and
/// the identifier as `request_id`; the contract type `DeliveredEvent` is not
/// the type the engine delivers.
///
/// # Errors
///
/// Returns `NURT_E_INTERNAL` when the event cannot be serialized.
pub fn encode_event(event: &DeliveredEvent) -> Result<String, Failure> {
    let mut document = to_value(event)?;
    if let Some(members) = document.as_object_mut() {
        let nanos = i64::try_from(event.time.unix_nanos()).unwrap_or(i64::MAX);
        members.insert(
            "time".to_owned(),
            Value::String(Instant::from_unix_nanos(nanos).to_string()),
        );
        if let Some(request) = members.remove("request_id") {
            members.insert("request".to_owned(), request);
        }
    }
    to_text(&document)
}

/// Runs `work` and turns a panic into `NURT_E_INTERNAL`.
///
/// A panic that leaves a worker thread of the addon aborts the Node.js process
/// or, in the thread that reports a response, leaves the request without one.
fn guarded<T>(work: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let cause = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown cause");
        Err(Failure::internal(format!(
            "the operation panicked: {cause}"
        )))
    })
}

fn answer<T: Serialize>(result: Result<T, Error>) -> Result<Answer, Failure> {
    match result {
        Ok(value) => Ok(Ok(to_value(&value)?)),
        Err(error) => Ok(Err(error)),
    }
}

fn history_query(input: &Value) -> Result<HistoryQuery, Failure> {
    if let Some(text) = input.get("query").and_then(Value::as_str) {
        let mut query: Value = parse(text, "the history query")?;
        default_flags(&mut query);
        return from_value(&query, "the history query");
    }
    let mut query = if input.is_null() {
        json!({})
    } else {
        input.clone()
    };
    default_flags(&mut query);
    from_value(&query, "the history query")
}

/// The flag `deleted_only` of a history query has no default in the contract;
/// a query that omits it asks for all operations.
fn default_flags(query: &mut Value) {
    if let Some(members) = query.as_object_mut() {
        members.entry("deleted_only").or_insert(Value::Bool(false));
    }
}

fn parse<T: DeserializeOwned>(text: &str, what: &str) -> Result<T, Failure> {
    serde_json::from_str(text)
        .map_err(|error| Failure::arg(format!("{what} is not valid: {error}")))
}

fn from_value<T: DeserializeOwned>(value: &Value, what: &str) -> Result<T, Failure> {
    serde_json::from_value(value.clone())
        .map_err(|error| Failure::arg(format!("{what} is not valid: {error}")))
}

/// Like [`from_value`], with an absent value read as an empty object.
fn from_optional<T: DeserializeOwned>(value: &Value, what: &str) -> Result<T, Failure> {
    if value.is_null() {
        from_value(&json!({}), what)
    } else {
        from_value(value, what)
    }
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, Failure> {
    serde_json::to_value(value)
        .map_err(|error| Failure::internal(format!("cannot serialize the result: {error}")))
}

fn to_text<T: Serialize>(value: &T) -> Result<String, Failure> {
    serde_json::to_string(value)
        .map_err(|error| Failure::internal(format!("cannot serialize the result: {error}")))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError, mpsc};
    use std::time::Duration;

    use nurt_contracts::envelope::Request;
    use nurt_core::config::ConfigSources;
    use nurt_core::{DeliveredEvent, EngineConfig, EventListener};
    use serde_json::{Value, json};

    use super::{Failure, Message, Native, Sink, Status, encode_event, finish, guarded};
    use crate::system::{SystemClock, SystemHost};

    #[test]
    fn a_panic_becomes_an_internal_failure_with_its_message() {
        let literal: Result<(), Failure> = guarded(|| panic!("boom żółć"));
        let failure = literal.unwrap_err();
        assert_eq!(failure.status(), Status::Internal);
        assert!(failure.message().contains("boom żółć"), "{failure}");
        let owned: Result<(), Failure> = guarded(|| panic!("{}", String::from("owned cause")));
        assert!(owned.unwrap_err().message().contains("owned cause"));
        let opaque: Result<(), Failure> = guarded(|| std::panic::panic_any(7_u8));
        assert!(opaque.unwrap_err().message().contains("unknown cause"));
    }

    #[test]
    fn a_guarded_call_returns_its_result_and_its_failure_unchanged() {
        assert_eq!(guarded(|| Ok::<_, Failure>(5)), Ok(5));
        let failure = Failure::arg("bad");
        assert_eq!(guarded(|| Err::<(), _>(failure.clone())), Err(failure));
    }

    #[test]
    fn encoded_events_keep_their_time_in_utc_and_rename_the_request() {
        let native = native();
        let collector = Arc::new(Collector::default());
        let request = serde_json::to_string(&Request::program("find \"/srv\"")).unwrap();
        let handle = native.start(&request, collector.clone()).unwrap();
        finish(&handle, Some(WAIT)).unwrap();
        let events = collector.0.lock().unwrap_or_else(PoisonError::into_inner);
        for event in events.iter() {
            let document: Value = serde_json::from_str(&encode_event(event).unwrap()).unwrap();
            assert!(document["time"].as_str().unwrap().ends_with('Z'));
            assert_eq!(document["request"], handle.request_id());
            assert!(document.get("request_id").is_none());
        }
    }

    #[test]
    fn history_and_space_reject_non_object_parameters() {
        let native = native();
        for input in ["5", "\"x\"", "true"] {
            for operation in ["history", "space", "describe", "help"] {
                let failure = native.call(operation, input).unwrap_err();
                assert_eq!(failure.status(), Status::Arg, "{operation} {input}");
            }
        }
    }

    const WAIT: Duration = Duration::from_secs(30);

    fn native() -> Native {
        let config = EngineConfig::new(Arc::new(SystemClock::new()), Arc::new(SystemHost::new()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir();
        Native::open_with(config).unwrap()
    }

    fn call(native: &Native, operation: &str, input: &Value) -> Value {
        let text = native.call(operation, &input.to_string()).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[derive(Default)]
    struct Collector(Mutex<Vec<DeliveredEvent>>);

    impl EventListener for Collector {
        fn deliver(&self, event: DeliveredEvent) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(event);
        }
    }

    #[test]
    fn status_names_follow_nurt_h() {
        let names: Vec<&str> = [
            Status::Timeout,
            Status::Cancelled,
            Status::Arg,
            Status::Abi,
            Status::Io,
            Status::Internal,
        ]
        .iter()
        .map(|status| status.name())
        .collect();
        assert_eq!(
            names,
            [
                "NURT_E_TIMEOUT",
                "NURT_E_CANCELLED",
                "NURT_E_ARG",
                "NURT_E_ABI",
                "NURT_E_IO",
                "NURT_E_INTERNAL"
            ]
        );
    }

    #[test]
    fn failure_text_starts_with_the_status_name() {
        let failure = Failure::arg("bad input");
        assert_eq!(failure.to_string(), "NURT_E_ARG: bad input");
        assert_eq!(failure.status(), Status::Arg);
        assert_eq!(failure.message(), "bad input");
    }

    #[test]
    fn rejects_options_that_are_not_valid() {
        assert_eq!(
            Native::open(Some("not json")).unwrap_err().status(),
            Status::Arg
        );
        assert_eq!(
            Native::open(Some(r#"{"unknown": 1}"#))
                .unwrap_err()
                .status(),
            Status::Arg
        );
    }

    #[test]
    fn info_describes_the_engine() {
        let info: Value = serde_json::from_str(&native().info().unwrap()).unwrap();
        assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
        let result = call(&native(), "info", &Value::Null);
        assert_eq!(result["result"]["version"], info["version"]);
    }

    #[test]
    fn rejects_an_unknown_operation() {
        let failure = native().call("explode", "{}").unwrap_err();
        assert_eq!(failure.status(), Status::Arg);
        assert!(failure.message().contains("explode"));
    }

    #[test]
    fn rejects_parameters_that_are_not_valid() {
        let native = native();
        assert_eq!(native.call("parse", "[").unwrap_err().status(), Status::Arg);
        assert_eq!(
            native.call("parse", "{}").unwrap_err().status(),
            Status::Arg
        );
        assert_eq!(
            native
                .call("space", r#"{"volume": "relative/path"}"#)
                .unwrap_err()
                .status(),
            Status::Arg
        );
    }

    #[test]
    fn language_errors_are_returned_as_results() {
        let native = native();
        let parsed = call(&native, "parse", &json!({ "program": "find \"/srv\"" }));
        assert!(parsed.get("error").is_some(), "{parsed}");
        let history = call(&native, "history", &Value::Null);
        assert!(history.get("error").is_some(), "{history}");
        let space = call(&native, "space", &json!({}));
        assert!(space.get("error").is_some(), "{space}");
    }

    #[test]
    fn schema_returns_the_document() {
        let native = native();
        let schema = call(&native, "schema", &json!({ "name": "request" }));
        assert!(schema["result"].is_object(), "{schema}");
        let unknown = call(&native, "schema", &json!({ "name": "nonexistent" }));
        assert!(unknown["error"].is_object(), "{unknown}");
    }

    #[test]
    fn history_accepts_a_query_given_as_text() {
        let native = native();
        let query = json!({ "query": "{\"limit\": 5}" });
        assert!(call(&native, "history", &query).get("error").is_some());
        let invalid = json!({ "query": "{\"limit\": \"many\"}" });
        assert_eq!(
            native
                .call("history", &invalid.to_string())
                .unwrap_err()
                .status(),
            Status::Arg
        );
    }

    #[test]
    fn page_of_an_unknown_cursor_reports_an_error() {
        let page = call(
            &native(),
            "page",
            &json!({ "cursor": "missing", "limit": 0 }),
        );
        assert!(page["result"]["report"]["error"].is_object(), "{page}");
    }

    #[test]
    fn started_request_delivers_events_and_a_response() {
        let native = native();
        let collector = Arc::new(Collector::default());
        let request = serde_json::to_string(&Request::program("find \"/srv\"")).unwrap();
        let handle = native.start(&request, collector.clone()).unwrap();
        let response: Value = serde_json::from_str(&finish(&handle, Some(WAIT)).unwrap()).unwrap();
        assert!(handle.is_finished());
        assert!(response["report"]["error"].is_object(), "{response}");

        let events = collector.0.lock().unwrap_or_else(PoisonError::into_inner);
        let first: Value = serde_json::from_str(&encode_event(&events[0]).unwrap()).unwrap();
        let last: Value =
            serde_json::from_str(&encode_event(events.last().unwrap()).unwrap()).unwrap();
        assert_eq!(first["t"], "exec.started");
        assert_eq!(first["seq"], 1);
        assert_eq!(last["t"], "exec.finished");
        assert_eq!(first["request"], handle.request_id());
        assert!(first.get("request_id").is_none());
        assert!(
            first["time"]
                .as_str()
                .is_some_and(|time| time.ends_with('Z'))
        );
    }

    #[test]
    fn response_follows_every_event_of_the_request() {
        const REQUESTS: usize = 25;
        let native = native();
        let request = serde_json::to_string(&Request::program("find \"/srv\"")).unwrap();
        for _ in 0..REQUESTS {
            let (sender, receiver) = mpsc::channel();
            let sender = Mutex::new(sender);
            let sink: Sink = Arc::new(move |message| {
                let _ = sender
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(message);
            });
            let handle = native.start_ordered(&request, sink).unwrap();
            let mut events = Vec::new();
            let response = loop {
                match receiver.recv_timeout(WAIT).unwrap() {
                    Message::Event(text) => {
                        events.push(serde_json::from_str::<Value>(&text).unwrap())
                    }
                    Message::Response(response) => break response,
                }
            };
            let first = events.first().unwrap();
            let last = events.last().unwrap();
            assert_eq!(first["t"], "exec.started");
            assert_eq!(last["t"], "exec.finished");
            let sequence: Vec<u64> = events
                .iter()
                .map(|event| event["seq"].as_u64().unwrap())
                .collect();
            let expected: Vec<u64> = (1..=sequence.len() as u64).collect();
            assert_eq!(sequence, expected);
            assert!(
                events
                    .iter()
                    .all(|event| event["request"] == handle.request_id())
            );
            let response: Value = serde_json::from_str(&response.unwrap()).unwrap();
            assert!(response["report"]["error"].is_object(), "{response}");
            assert!(receiver.try_recv().is_err());
        }
    }

    #[test]
    fn rejects_a_request_that_is_not_valid() {
        let failure = native()
            .start("{\"program\": 1}", Arc::new(Collector::default()))
            .unwrap_err();
        assert_eq!(failure.status(), Status::Arg);
    }

    #[test]
    fn cancelling_a_finished_request_changes_nothing() {
        let native = native();
        let request = serde_json::to_string(&Request::program("find \"/srv\"")).unwrap();
        let handle = native
            .start(&request, Arc::new(Collector::default()))
            .unwrap();
        let before = finish(&handle, Some(WAIT)).unwrap();
        handle.cancel();
        assert_eq!(finish(&handle, Some(WAIT)).unwrap(), before);
    }
}
