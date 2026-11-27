// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Public interface of the engine: envelope rules and defaults, sessions,
//! candidates, execution handles, events, the execution context and the
//! queries that do not run programs.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::env::{Clock, Os};
use nurt_contracts::envelope::{
    DiffMode, Language, OutputFormat, Request, RequestOptions, Response, Verbosity,
};
use nurt_contracts::error::{AbiStatus, Candidate, Error, ErrorCode};
use nurt_contracts::fs::RegistryAccess;
use nurt_contracts::journal::{HistoryQuery, JournalStore};
use nurt_contracts::kinds::{EffectKind, SyntaxVariant};
use nurt_contracts::report::ReportStatus;
use nurt_contracts::services::ServiceKind;
use nurt_core::config::{ConfigSources, toml_table};
use nurt_core::readcache::ReadCache;
use nurt_core::{
    Compiled, Corrected, DeliveredEvent, Engine, EngineConfig, EventListener, ExecHandle, Frontend,
};
use nurt_sys::registry::RegistryProvider;
use nurt_sys::registry::mem::MemRegistry;
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::host::TestHost;
use nurt_testkit::services::test_services;
use serde_json::json;
use tempfile::TempDir;

const AMBIGUOUS: &str = "fnd \"/srv\"";
const FIND: &str = "find \"/srv\"";
const BLOCKING: &str = "find \"/slow\"";

fn bound_find() -> BoundDocument {
    serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{
            "t": "pipeline",
            "head": {"t": "stage", "name": "find", "effect": "read", "phase": "source", "args": [
                {"name": "path", "value": {"t": "path", "v": nurt_testkit::paths::abs("/srv")}}
            ]},
            "stages": []
        }]}
    }))
    .unwrap()
}

fn ast() -> AstDocument {
    serde_json::from_value(json!({"nurt": "1.0", "program": {"statements": []}})).unwrap()
}

/// Front end that binds every program to `find`, reports [`AMBIGUOUS`] as
/// ambiguous and stops [`BLOCKING`] at a barrier.
#[derive(Default)]
struct ScriptedFrontend {
    compiled: Mutex<Vec<String>>,
    barrier: Option<Arc<Barrier>>,
}

impl ScriptedFrontend {
    fn compiled(&self) -> Vec<String> {
        self.compiled.lock().unwrap().clone()
    }
}

impl Frontend for ScriptedFrontend {
    fn parse(
        &self,
        text: &str,
        _: Option<SyntaxVariant>,
        _: &Config,
    ) -> Result<AstDocument, Error> {
        if text == "list" {
            Ok(ast())
        } else {
            Err(Error::new(ErrorCode::Syntax, "unexpected text"))
        }
    }

    fn correct(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Corrected, Error> {
        Ok(Corrected {
            ast: self.parse(text, variant, config)?,
            corrections: Vec::new(),
            ran: None,
        })
    }

    fn bind(&self, _: &AstDocument, _: &Config) -> Result<BoundDocument, Error> {
        Ok(bound_find())
    }

    fn print(&self, _: &AstDocument, variant: SyntaxVariant) -> Result<String, Error> {
        Ok(format!("list ({variant:?})"))
    }

    fn describe(&self, subject: &str, language: Language) -> Result<serde_json::Value, Error> {
        Ok(json!({"subject": subject, "language": language.as_str()}))
    }

    fn help(&self, topic: &str, language: Language) -> Result<String, Error> {
        Ok(format!("{topic}:{}", language.as_str()))
    }

    fn compile(&self, text: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<Compiled, Error> {
        self.compiled.lock().unwrap().push(text.to_owned());
        if text == AMBIGUOUS {
            let candidate = |rank, program: &str| Candidate {
                rank,
                reference: None,
                program: program.to_owned(),
                why: None,
            };
            return Err(
                Error::new(ErrorCode::Ambiguous, "`fnd` matches several commands")
                    .with_candidate(candidate(1, FIND))
                    .with_candidate(candidate(2, "fold \"/srv\"")),
            );
        }
        if text == BLOCKING
            && let Some(barrier) = &self.barrier
        {
            barrier.wait();
            barrier.wait();
        }
        Ok(Compiled {
            bound: bound_find(),
            corrections: Vec::new(),
            ran: None,
            ast_hash: None,
        })
    }
}

struct Harness {
    temp: TempDir,
    clock: Arc<VirtualClock>,
    frontend: Arc<ScriptedFrontend>,
    engine: Engine,
}

fn harness_with(frontend: ScriptedFrontend, overrides: &str) -> Harness {
    let temp = tempfile::tempdir().unwrap();
    let clock = Arc::new(VirtualClock::new());
    let frontend = Arc::new(frontend);
    let context = TestContext::new();
    let journal: Arc<dyn JournalStore> = context.journal().clone();
    let engine = Engine::open(
        EngineConfig::new(clock.clone() as Arc<dyn Clock>, Arc::new(TestHost::linux()))
            .with_sources(
                ConfigSources::isolated().with_overrides(toml_table(overrides, "test").unwrap()),
            )
            .with_state_dir(temp.path().join("state"))
            .with_frontend(frontend.clone())
            .with_fs(context.fs().clone())
            .with_journal(journal)
            .with_services(test_services()),
    )
    .unwrap();
    Harness {
        temp,
        clock,
        frontend,
        engine,
    }
}

fn harness() -> Harness {
    harness_with(ScriptedFrontend::default(), "")
}

fn error_of(response: &Response) -> &Error {
    assert_eq!(response.report.status, ReportStatus::Failed);
    response.validate().unwrap();
    response.report.error.as_ref().unwrap()
}

fn in_session(request: Request, session: &str) -> Request {
    let options = RequestOptions {
        session: Some(session.to_owned()),
        ..RequestOptions::default()
    };
    request.with_options(options)
}

#[test]
fn engine_and_handles_are_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Engine>();
    shared::<ExecHandle>();
}

#[test]
fn golden_requests_get_valid_responses() {
    let engine = Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir(),
    )
    .unwrap();
    let dir = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/request");
    let mut checked = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let request: Request = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let response = engine.run(&request);
        let error = error_of(&response);
        let expected = if request.accept.is_some() {
            ErrorCode::ExpiredCandidate
        } else if request.cursor.is_some() {
            ErrorCode::Arg
        } else {
            ErrorCode::Unsupported
        };
        assert_eq!(error.code, expected, "{}", path.display());
        checked += 1;
    }
    assert!(checked >= 6);
}

#[test]
fn only_one_input_is_accepted() {
    let h = harness();
    let mut request = Request::program(FIND);
    request.accept = Some("c1".to_owned());
    let response = h.engine.run(&request);
    let error = error_of(&response);
    assert_eq!(error.code, ErrorCode::Arg);
    assert_eq!(error.fix, vec![FIND.to_owned()]);
    assert_eq!(error.details["fields"], json!(["program", "accept"]));
    let mut missing = Request::program(FIND);
    missing.program = None;
    assert_eq!(error_of(&h.engine.run(&missing)).code, ErrorCode::Arg);
}

#[test]
fn envelope_errors_have_the_report_kind_request() {
    let h = harness();
    let mut request = Request::program(FIND);
    request.cursor = Some("c_1".to_owned());
    let response = h.engine.run(&request);
    assert_eq!(error_of(&response).code, ErrorCode::Arg);
    assert_eq!(response.report.kind, "request");
    let relative = RequestOptions {
        base: Some("srv/app".to_owned()),
        ..RequestOptions::default()
    };
    let response = h.engine.run(&Request::cursor("c_1").with_options(relative));
    assert_eq!(error_of(&response).code, ErrorCode::PathRelative);
    assert_eq!(response.report.kind, "request");
}

#[test]
fn relative_paths_are_rejected() {
    let h = harness();
    for options in [
        RequestOptions {
            base: Some("srv/app".to_owned()),
            ..RequestOptions::default()
        },
        RequestOptions {
            save_to: Some("result.json".to_owned()),
            ..RequestOptions::default()
        },
    ] {
        let response = h.engine.run(&Request::program(FIND).with_options(options));
        assert_eq!(error_of(&response).code, ErrorCode::PathRelative);
    }
}

const CONFIGURED: &str =
    "[scope]\nbase = \"/srv/app\"\n[output]\nlanguage = \"pl\"\n[limits]\nmax_records = 1000\n";

#[test]
fn defaults_follow_the_envelope_and_the_configuration() {
    let h = harness_with(ScriptedFrontend::default(), CONFIGURED);
    let ctx = h.engine.context(&Request::program(FIND)).unwrap();
    let expected = RequestOptions {
        preview: Some(false),
        base: Some("/srv/app".to_owned()),
        limit: Some(50),
        fields: None,
        format: Some(OutputFormat::Text),
        verbosity: Some(Verbosity::Normal),
        diff: Some(DiffMode::Unified),
        diff_context: Some(3),
        max_chars: Some(0),
        save_to: None,
        timeout: Some("120s".to_owned()),
        session: None,
        language: Some(Language::Pl),
        script: Some(false),
    };
    assert_eq!(ctx.options(), &expected);
}

#[test]
fn context_carries_the_facilities_of_the_engine() {
    let h = harness_with(ScriptedFrontend::default(), CONFIGURED);
    let ctx = h.engine.context(&Request::program(FIND)).unwrap();
    assert_eq!(ctx.limits().max_records, 1000);
    assert_eq!(ctx.limits().threads, 4);
    assert!(ctx.extensions().get::<ReadCache>().is_some());
    assert!(ctx.services().has(ServiceKind::Doc));
    assert!(ctx.fs().is_some());
    assert!(ctx.journal().is_some());
    assert_eq!(ctx.state_dir(), Some(h.temp.path().join("state").as_path()));
    assert!(ctx.deadline().is_some());
    assert_eq!(ctx.config().limits.max_records, 1000);
}

#[test]
fn context_gives_access_to_the_registry_of_an_engine_that_has_one() {
    #[cfg(not(windows))]
    {
        let without = harness();
        let ctx = without.engine.context(&Request::program(FIND)).unwrap();
        assert!(ctx.extensions().get::<dyn RegistryAccess>().is_none());
    }
    let temp = tempfile::tempdir().unwrap();
    let context = TestContext::new();
    let journal: Arc<dyn JournalStore> = context.journal().clone();
    let engine = Engine::open(
        EngineConfig::new(
            Arc::new(VirtualClock::new()) as Arc<dyn Clock>,
            Arc::new(TestHost::windows()),
        )
        .with_sources(ConfigSources::isolated())
        .with_state_dir(temp.path().join("state"))
        .with_frontend(Arc::new(ScriptedFrontend::default()))
        .with_fs(context.fs().clone())
        .with_journal(journal)
        .with_services(test_services())
        .with_registry(Arc::new(RegistryProvider::new(
            Arc::new(MemRegistry::new()),
        ))),
    )
    .unwrap();
    let ctx = engine.context(&Request::program(FIND)).unwrap();
    let access = ctx.extensions().get::<dyn RegistryAccess>().unwrap();
    assert!(
        access
            .describe("registry:/HKCU/Software/Absent")
            .unwrap()
            .is_none()
    );
}

#[test]
fn request_options_override_the_defaults() {
    let h = harness();
    let options = RequestOptions {
        limit: Some(5),
        timeout: Some("1h30m".to_owned()),
        language: Some(Language::Pl),
        ..RequestOptions::default()
    };
    let ctx = h
        .engine
        .context(&Request::program(FIND).with_options(options))
        .unwrap();
    assert_eq!(ctx.options().limit, Some(5));
    assert_eq!(ctx.options().timeout.as_deref(), Some("1h30m"));
    assert_eq!(ctx.options().language, Some(Language::Pl));
}

#[test]
fn ambiguous_programs_offer_candidates_until_they_expire() {
    let h = harness();
    let response = h
        .engine
        .run(&in_session(Request::program(AMBIGUOUS), "s-1"));
    let error = error_of(&response);
    assert_eq!(error.code, ErrorCode::Ambiguous);
    let references: Vec<_> = error
        .candidates
        .iter()
        .map(|candidate| candidate.reference.clone().unwrap())
        .collect();
    assert_eq!(references, vec!["c1", "c2"]);

    let accepted = h.engine.run(&in_session(Request::accept("c1"), "s-1"));
    assert_eq!(error_of(&accepted).code, ErrorCode::NotFound);
    assert_eq!(accepted.report.kind, "find");
    assert_eq!(h.frontend.compiled(), vec![AMBIGUOUS, FIND]);

    let foreign = h.engine.run(&in_session(Request::accept("c1"), "s-2"));
    assert_eq!(error_of(&foreign).code, ErrorCode::ExpiredCandidate);

    h.clock.advance(Duration::from_secs(10 * 60));
    let expired = h.engine.run(&in_session(Request::accept("c1"), "s-1"));
    assert_eq!(error_of(&expired).code, ErrorCode::ExpiredCandidate);
    assert_eq!(expired.report.kind, "accept");
}

#[test]
fn sessions_are_recorded_and_checked() {
    let h = harness();
    let _ = h.engine.run(&in_session(Request::program(FIND), "s-1"));
    assert_eq!(h.engine.sessions().active(h.clock.now()), 1);
    let invalid = h.engine.run(&in_session(Request::program(FIND), "a\nb"));
    assert_eq!(error_of(&invalid).code, ErrorCode::Arg);
    let error = Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_session(""),
    )
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::Arg);
}

#[derive(Default)]
struct Collect(Mutex<Vec<DeliveredEvent>>);

impl EventListener for Collect {
    fn deliver(&self, event: DeliveredEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn cancelled_request_reports_cancellation_and_events() {
    let barrier = Arc::new(Barrier::new(2));
    let h = harness_with(
        ScriptedFrontend {
            barrier: Some(barrier.clone()),
            ..ScriptedFrontend::default()
        },
        "",
    );
    let events = Arc::new(Collect::default());
    let handle = h.engine.start(&Request::program(BLOCKING), events.clone());
    barrier.wait();
    assert!(handle.wait(Some(Duration::from_millis(10))).is_none());
    assert!(!handle.is_finished());
    handle.cancel();
    barrier.wait();
    let response = handle.wait(None).unwrap();
    assert_eq!(error_of(&response).code, ErrorCode::Cancelled);
    assert!(handle.is_finished());
    handle.cancel();
    let delivered = events.0.lock().unwrap().clone();
    let json: Vec<_> = delivered
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    assert_eq!(json.len(), 2);
    assert_eq!(json[0]["t"], "exec.started");
    assert_eq!(json[0]["seq"], 1);
    assert_eq!(json[1]["t"], "exec.finished");
    assert_eq!(json[1]["status"], "failed");
    assert!(
        delivered
            .iter()
            .all(|event| event.request_id == handle.request_id())
    );
}

#[test]
fn started_requests_complete_without_cancellation() {
    let h = harness();
    let handle = h
        .engine
        .start(&Request::program(FIND), Arc::new(Collect::default()));
    let response = handle.wait(None).unwrap();
    assert_eq!(error_of(&response).code, ErrorCode::NotFound);
    let second = h
        .engine
        .start(&Request::program(FIND), Arc::new(Collect::default()));
    assert_ne!(second.request_id(), handle.request_id());
}

#[test]
fn queries_delegate_to_the_front_end() {
    let h = harness();
    assert_eq!(h.engine.parse("list", None).unwrap(), ast());
    assert_eq!(
        h.engine.parse("?", None).unwrap_err().code,
        ErrorCode::Syntax
    );
    assert_eq!(h.engine.print(&ast(), None).unwrap(), "list (A)");
    assert_eq!(h.engine.correct("list", None).unwrap().ast, ast());
    assert_eq!(h.engine.describe("core", None).unwrap()["language"], "en");
    assert_eq!(
        h.engine.help("find", Some(Language::Pl)).unwrap(),
        "find:pl"
    );
    let catalog = h.engine.catalog().unwrap();
    assert_eq!(catalog["catalog_rev"], h.engine.catalog_rev());
}

#[test]
fn check_plans_the_bound_program() {
    let h = harness();
    let report = h.engine.check(&Request::program(FIND)).unwrap();
    assert_eq!(report.bound, bound_find());
    assert_eq!(report.summary.effect, EffectKind::Read);
    assert_eq!(report.summary.paths, vec![nurt_testkit::paths::abs("/srv")]);
    let from_tree = h.engine.check(&Request::ast(ast())).unwrap();
    assert_eq!(from_tree.bound, report.bound);
    assert_eq!(from_tree.summary, report.summary);
    assert!(from_tree.ast_hash.is_some());
    assert_eq!(
        h.engine.check(&Request::cursor("c_1")).unwrap_err().code,
        ErrorCode::Arg
    );
    assert_eq!(
        h.engine
            .check(&Request::program(AMBIGUOUS))
            .unwrap_err()
            .code,
        ErrorCode::Ambiguous
    );
}

#[test]
fn syntax_trees_are_bound_and_run() {
    let h = harness();
    let response = h.engine.run(&Request::ast(ast()));
    let error = error_of(&response);
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(response.report.kind, "find");
    assert_eq!(response.rendered.unwrap().format, OutputFormat::Text);
}

#[test]
fn unknown_cursor_is_reported() {
    let h = harness();
    let response = h
        .engine
        .page("c_00000000-0000000000000000-0", &RequestOptions::default());
    assert_eq!(error_of(&response).code, ErrorCode::Arg);
    assert_eq!(response.report.kind, "page");
}

#[test]
fn schemas_are_published_by_name() {
    let h = harness();
    assert!(h.engine.schema("report").unwrap().contains("nurt-report"));
    let error = h.engine.schema("reports").unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownValue);
    assert_eq!(error.details["accepted"][0], "ast");
}

#[test]
fn journal_queries_need_a_journal() {
    let h = harness();
    assert!(
        h.engine
            .history(&HistoryQuery::default())
            .unwrap()
            .is_empty()
    );
    let _ = h.engine.space(None);
    let bare = Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir(),
    )
    .unwrap();
    assert_eq!(
        bare.history(&HistoryQuery::default()).unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(bare.space(None).unwrap_err().code, ErrorCode::Unsupported);
    assert!(bare.state_dir().is_none());
}

#[test]
fn info_describes_the_build() {
    let h = harness();
    let info = h.engine.info();
    assert_eq!(info.formats.report, "1.0");
    assert_eq!(info.formats.config, "nurt-config/1");
    assert_eq!(info.os, Os::Linux);
    assert!(info.services.contains(&"doc".to_owned()));
    assert!(info.state_dir.is_some());
    assert!(format!("{:?}", h.engine).contains("Engine"));
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["architecture"], "x86_64");
}

#[test]
fn environment_selects_the_state_directory() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("from-environment");
    let host = TestHost::linux()
        .with_var("NURT_STATE_DIR", state.as_os_str())
        .with_var("NURT_BASE", "/srv/app");
    let engine = Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(host))
            .with_sources(ConfigSources::new().with_files(None, None)),
    )
    .unwrap();
    assert_eq!(engine.state_dir().unwrap().root(), state);
    assert_eq!(engine.config().scope.base.as_deref(), Some("/srv/app"));
    assert!(engine.read_cache().entries() == 0);
}

#[test]
fn each_base_directory_uses_its_project_file() {
    let h = harness();
    let project = h.temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join(".nurt.toml"), "[run]\ntimeout = \"5m\"\n").unwrap();
    let options = RequestOptions {
        base: Some(project.join("src").to_string_lossy().into_owned()),
        ..RequestOptions::default()
    };
    let ctx = h
        .engine
        .context(&Request::program(FIND).with_options(options.clone()))
        .unwrap();
    assert_eq!(ctx.config().run.timeout.to_string(), "5m");
    let again = h
        .engine
        .context(&Request::program(FIND).with_options(options))
        .unwrap();
    assert_eq!(again.config().run.timeout.to_string(), "5m");
    let plain = h.engine.context(&Request::program(FIND)).unwrap();
    assert_eq!(plain.config().run.timeout.to_string(), "2m");
    assert_eq!(
        h.engine.loaded_config().config().run.timeout.to_string(),
        "2m"
    );
}

#[test]
fn invalid_project_file_fails_the_request() {
    let h = harness();
    let project = h.temp.path().join("broken");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join(".nurt.toml"), "[run]\ntimeout = 5\n").unwrap();
    let options = RequestOptions {
        base: Some(project.to_string_lossy().into_owned()),
        ..RequestOptions::default()
    };
    let response = h.engine.run(&Request::program(FIND).with_options(options));
    assert_eq!(error_of(&response).code, ErrorCode::Config);
}
