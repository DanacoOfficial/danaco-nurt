// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! [`Backend`] implementation on [`nurt_core::Engine`].
//!
//! A request runs through [`Engine::start`]: the events of the execution
//! reach the server through the [`EventSink`] it gave, and a cancelled
//! [`CancelToken`] cancels the started execution. Language errors stay in the
//! [`Response`]; only a failure of the call itself becomes a [`CallError`],
//! with the status of [`ErrorCode::abi`].

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::envelope::{Language, Request, Response};
use nurt_contracts::error::{AbiStatus, Error, ErrorCode};
use nurt_contracts::report::{Report, ReportStatus};
use nurt_contracts::stage::{CancelToken, EventSink};
use nurt_core::{DeliveredEvent, Engine, EventBody, EventListener};

use crate::backend::{
    Backend, BackendInfo, CallError, CheckOutcome, DescribeSubject, HelpKind, HelpQuery, SchemaName,
};

/// Calls handled at the same time; the engine has no setting of its own.
const MAX_INFLIGHT: usize = 16;
/// Interval at which a running request is checked for cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(20);
/// Report kind of a request cancelled before it started.
const CANCELLED_KIND: &str = "program";

/// Passes the events emitted by stages to a server sink; the start and end
/// of the execution are not part of the progress stream.
struct SinkListener(Arc<dyn EventSink>);

impl EventListener for SinkListener {
    fn deliver(&self, event: DeliveredEvent) {
        forward(self.0.as_ref(), event);
    }
}

fn forward(sink: &dyn EventSink, event: DeliveredEvent) {
    if let EventBody::Stage(event) = event.body {
        sink.emit(event);
    }
}

fn call_error(error: &Error) -> CallError {
    CallError::new(error.code.abi(), error.message.clone())
}

impl Backend for Engine {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            version: Engine::info(self).version,
            max_inflight: MAX_INFLIGHT,
        }
    }

    fn catalog_rev(&self) -> Option<String> {
        Some(Engine::catalog_rev(self).to_owned())
    }

    fn run(
        &self,
        request: &Request,
        events: Arc<dyn EventSink>,
        cancel: CancelToken,
    ) -> Result<Response, CallError> {
        if cancel.is_cancelled() {
            let mut report = Report::new(ReportStatus::Failed, CANCELLED_KIND);
            report.error = Some(Error::new(
                ErrorCode::Cancelled,
                "the request was cancelled",
            ));
            return Ok(Response::new(report));
        }
        let handle = self.start(request, Arc::new(SinkListener(events)));
        let mut cancelled = false;
        loop {
            if let Some(response) = handle.wait(Some(CANCEL_POLL)) {
                return Ok(response);
            }
            if !cancelled && cancel.is_cancelled() {
                handle.cancel();
                cancelled = true;
            }
        }
    }

    fn help(&self, query: &HelpQuery) -> Result<String, CallError> {
        let topic = query.topic.as_deref().unwrap_or("");
        let examples = query
            .examples
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX));
        Engine::help_query(
            self,
            topic,
            query.kind.map(HelpKind::as_str),
            examples,
            query.language,
        )
        .map_err(|error| call_error(&error))
    }

    fn describe(&self, subject: &DescribeSubject) -> Result<String, CallError> {
        let name = match subject {
            DescribeSubject::Core => "core",
            DescribeSubject::Group(name) => name.as_str(),
        };
        let description =
            Engine::describe(self, name, None::<Language>).map_err(|error| call_error(&error))?;
        description
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                CallError::new(
                    AbiStatus::Internal,
                    "the description of the engine has no text",
                )
            })
    }

    fn schema(&self, name: SchemaName) -> Result<String, CallError> {
        Engine::schema(self, name.as_str())
            .map(str::to_owned)
            .map_err(|error| call_error(&error))
    }

    fn check(&self, request: &Request) -> Result<CheckOutcome, CallError> {
        match Engine::check(self, request) {
            Ok(report) => serde_json::to_value(&report)
                .map(CheckOutcome::Report)
                .map_err(|error| CallError::new(AbiStatus::Internal, error.to_string())),
            Err(error) => Ok(CheckOutcome::Failed(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nurt_contracts::env::WallTime;
    use nurt_contracts::stage::{DiscardEvents, Event, LogEvent, LogLevel};
    use nurt_core::config::ConfigSources;
    use nurt_core::fixtures::FixtureFrontend;
    use nurt_core::{EngineConfig, Lifecycle};
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::fixtures::empty_program;
    use nurt_testkit::host::TestHost;

    use super::*;

    const KNOWN_PROGRAM: &str = "known";

    fn open(state: &std::path::Path, frontend: Option<FixtureFrontend>) -> Engine {
        let mut config =
            EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
                .with_sources(ConfigSources::isolated())
                .with_state_dir(state);
        if let Some(frontend) = frontend {
            config = config.with_frontend(Arc::new(frontend));
        }
        Engine::open(config).expect("engine opens in a temporary directory")
    }

    fn open_with(state: &std::path::Path, frontend: Arc<dyn nurt_core::Frontend>) -> Engine {
        let config = EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .with_state_dir(state)
            .with_frontend(frontend);
        Engine::open(config).expect("engine opens in a temporary directory")
    }

    fn run(engine: &Engine, program: &str, cancel: CancelToken) -> Response {
        Backend::run(
            engine,
            &Request::program(program),
            Arc::new(DiscardEvents),
            cancel,
        )
        .expect("run reports language errors in the response")
    }

    #[derive(Default)]
    struct Collected(Mutex<Vec<Event>>);

    impl EventSink for Collected {
        fn emit(&self, event: Event) {
            self.0.lock().expect("lock").push(event);
        }
    }

    #[test]
    fn help_passes_the_kind_and_the_examples_to_the_engine() {
        let temp = tempfile::tempdir().expect("temp dir");
        let frontend = nurt_core::LanguageFrontend::new().expect("embedded front end");
        let engine = open_with(&temp.path().join("state"), Arc::new(frontend));
        let ask = |kind: Option<HelpKind>, examples: Option<u32>| {
            Backend::help(
                &engine,
                &HelpQuery {
                    topic: Some("replace".to_owned()),
                    kind,
                    examples,
                    language: None,
                },
            )
        };
        let plain = ask(None, None).expect("help of a command");
        let without_examples = ask(Some(HelpKind::Command), Some(0)).expect("typed help");
        assert!(without_examples.len() < plain.len());
        assert!(ask(Some(HelpKind::Error), None).is_err());
    }

    #[test]
    fn info_reports_the_engine_version_and_a_positive_limit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let info = Backend::info(&engine);
        assert_eq!(info.version, Engine::info(&engine).version);
        assert!(info.max_inflight >= 1);
    }

    #[test]
    fn run_without_a_front_end_reports_unsupported_in_the_response() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let response = run(&engine, "find \"/srv\"", CancelToken::new());
        let error = response.report.error.clone().expect("error in the report");
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert!(response.validate().is_ok());
    }

    #[test]
    fn run_of_a_bound_program_reaches_the_planner() {
        let temp = tempfile::tempdir().expect("temp dir");
        let frontend = FixtureFrontend::new().with_bound(KNOWN_PROGRAM, empty_program());
        let engine = open(&temp.path().join("state"), Some(frontend));
        let response = run(&engine, KNOWN_PROGRAM, CancelToken::new());
        assert_eq!(response.report.status, ReportStatus::Failed);
        assert_eq!(
            response.report.error.expect("error").code,
            ErrorCode::Unsupported
        );
    }

    #[test]
    fn run_with_an_invalid_envelope_reports_the_error_in_the_response() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let request = Request::cursor("no-such-cursor");
        let response = Backend::run(
            &engine,
            &request,
            Arc::new(DiscardEvents),
            CancelToken::new(),
        )
        .expect("response");
        assert!(response.report.error.is_some());
    }

    #[test]
    fn cancelled_token_yields_a_cancelled_response_without_running() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let cancel = CancelToken::new();
        cancel.cancel(nurt_contracts::stage::CancelReason::Requested);
        let response = run(&engine, "find \"/srv\"", cancel);
        assert_eq!(
            response.report.error.expect("error").code,
            ErrorCode::Cancelled
        );
    }

    #[test]
    fn help_without_a_front_end_maps_the_error_to_a_call_status() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let query = HelpQuery {
            topic: Some("find".to_owned()),
            ..HelpQuery::default()
        };
        let error = Backend::help(&engine, &query).expect_err("no front end");
        assert_eq!(error.status, ErrorCode::Unsupported.abi());
        assert_ne!(error.status, AbiStatus::Ok);
    }

    #[test]
    fn describe_without_a_front_end_maps_the_error_to_a_call_status() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let core = Backend::describe(&engine, &DescribeSubject::Core).expect_err("core");
        let group = Backend::describe(&engine, &DescribeSubject::Group("files".to_owned()))
            .expect_err("group");
        assert_eq!(core.status, ErrorCode::Unsupported.abi());
        assert_eq!(group.status, ErrorCode::Unsupported.abi());
    }

    #[test]
    fn check_without_a_front_end_is_a_language_error_not_a_call_error() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        let outcome = Backend::check(&engine, &Request::program("find \"/srv\"")).expect("outcome");
        let CheckOutcome::Failed(error) = outcome else {
            panic!("expected a language error");
        };
        assert_eq!(error.code, ErrorCode::Unsupported);
    }

    #[test]
    fn check_of_a_bound_program_is_the_check_document() {
        let temp = tempfile::tempdir().expect("temp dir");
        let frontend = FixtureFrontend::new().with_bound(KNOWN_PROGRAM, empty_program());
        let engine = open(&temp.path().join("state"), Some(frontend));
        match Backend::check(&engine, &Request::program(KNOWN_PROGRAM)).expect("outcome") {
            CheckOutcome::Report(document) => {
                assert_eq!(document["format"], "nurt-check/1");
                assert!(document.get("bound").is_some());
            }
            CheckOutcome::Failed(error) => panic!("unexpected error {error:?}"),
        }
    }

    #[test]
    fn every_schema_name_is_served_as_json() {
        let temp = tempfile::tempdir().expect("temp dir");
        let engine = open(&temp.path().join("state"), None);
        for name in SchemaName::ALL {
            let text = Backend::schema(&engine, *name).expect("schema");
            let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
            assert!(parsed.is_object(), "schema {} is an object", name.as_str());
        }
    }

    #[test]
    fn only_stage_events_reach_the_sink() {
        let sink = Collected::default();
        let delivered = |body| DeliveredEvent {
            seq: 1,
            time: WallTime::from_unix_nanos(1),
            request_id: "r1".to_owned(),
            body,
        };
        forward(&sink, delivered(EventBody::Lifecycle(Lifecycle::Started)));
        let log = Event::Log(LogEvent {
            level: LogLevel::Info,
            message: "scanning".to_owned(),
            source: None,
        });
        forward(&sink, delivered(EventBody::Stage(log.clone())));
        assert_eq!(*sink.0.lock().expect("lock"), vec![log]);
    }
}
