// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Programs run end to end through the engine on a virtual file system and a
//! recording journal. The commands `find` and `remove` are test stages that
//! work on the virtual file system; the record stages are those of the build.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use std::collections::HashMap;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::Language;
use nurt_contracts::envelope::{Request, RequestOptions, Response, Verbosity};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{FileOp, MutationTx, apply_ops};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::events::{DeliveredEvent, EventListener};
use nurt_core::frontend::{Compiled, Corrected, Frontend};
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::RecordingJournal;
use nurt_testkit::vfs::{VirtualFs, test_path};
use serde_json::json;

const FILES: [(&str, i64); 3] = [
    ("/data/a.log", 10),
    ("/data/b.log", 200),
    ("/data/c.log", 300),
];

/// `find`: one `entry` record for every file of the virtual file system under
/// the directory `/data`.
struct Find;

impl StageImpl for Find {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "find".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Find {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let fs = ctx.require_fs()?;
        let mut records = Vec::new();
        for (path, _) in FILES {
            let abs = AbsPath::new(path)?;
            let size = fs
                .metadata(&abs, nurt_contracts::fs::Follow::NoFollow)
                .ok()
                .map(|stat| i64::try_from(stat.size).unwrap_or(i64::MAX));
            if let Some(size) = size {
                records.push(
                    Record::new(RecordKind::Entry)
                        .with("path", Value::Path(path.to_owned()))
                        .with("size", Value::Int(size)),
                );
            }
        }
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// `remove`: deletes the files of the input records.
struct Remove;

struct Targets(Vec<AbsPath>);

impl StageImpl for Remove {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "remove".to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Remove {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let mut targets = Vec::new();
        let mut changes = Vec::new();
        for item in input.into_records()? {
            let record = item?;
            let text = record
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            targets.push(AbsPath::new(&text)?);
            changes.push(Change::new(text.as_str(), ChangeAction::Delete));
        }
        let mut plan = EffectPlan::new();
        plan.bulk = targets.len() > 1;
        plan.changes = changes;
        Ok(plan.with_state(Targets(targets)))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let Targets(targets) = plan
            .take_state::<Targets>()
            .ok_or_else(|| Error::new(ErrorCode::Internal, "the plan has no targets"))?;
        apply_ops(tx, targets.into_iter().map(FileOp::remove).collect())?;
        let effect = Record::new(RecordKind::Effect)
            .with("status", Value::Enum("ok".to_owned()))
            .with("kind", Value::Text("remove".to_owned()));
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(effect)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}

/// Front end that binds the program texts it was given to fixed trees.
#[derive(Default)]
struct Programs(HashMap<String, BoundDocument>);

impl Programs {
    fn with(mut self, text: &str, bound: BoundDocument) -> Self {
        self.0.insert(text.to_owned(), bound);
        self
    }
}

fn no_language() -> Error {
    Error::new(ErrorCode::Unsupported, "the test front end only compiles")
}

impl Frontend for Programs {
    fn parse(&self, _: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<AstDocument> {
        Err(no_language())
    }

    fn correct(&self, _: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<Corrected> {
        Err(no_language())
    }

    fn bind(&self, _: &AstDocument, _: &Config) -> Result<BoundDocument> {
        Err(no_language())
    }

    fn print(&self, _: &AstDocument, _: SyntaxVariant) -> Result<String> {
        Err(no_language())
    }

    fn describe(&self, _: &str, _: Language) -> Result<serde_json::Value> {
        Err(no_language())
    }

    fn help(&self, _: &str, _: Language) -> Result<String> {
        Err(no_language())
    }

    fn compile(&self, text: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<Compiled> {
        let bound =
            self.0.get(text).cloned().ok_or_else(|| {
                Error::new(ErrorCode::Syntax, format!("unknown program `{text}`"))
            })?;
        Ok(Compiled {
            bound,
            corrections: Vec::new(),
            ran: None,
            ast_hash: Some(PROGRAM_HASH.to_owned()),
        })
    }
}

const PROGRAM_HASH: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Default)]
struct Collect(Mutex<Vec<DeliveredEvent>>);

impl EventListener for Collect {
    fn deliver(&self, event: DeliveredEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn find_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "find", "effect": "read", "phase": "source",
        "args": [{"name": "path", "value": {"t": "path", "v": "/data"}}]
    })
}

fn filter_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "filter", "effect": "read", "phase": "transform",
        "args": [{"name": "condition", "value": {
            "t": "bin", "op": ">",
            "l": {"t": "field", "name": "size"},
            "r": {"t": "int", "v": 150}
        }}]
    })
}

fn count_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "count", "effect": "read", "phase": "transform", "args": []
    })
}

fn remove_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "remove", "effect": "local", "phase": "mutate", "args": []
    })
}

fn program(head: serde_json::Value, stages: Vec<serde_json::Value>) -> BoundDocument {
    serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{"t": "pipeline", "head": head, "stages": stages}]}
    }))
    .unwrap()
}

struct World {
    engine: Engine,
    vfs: Arc<VirtualFs>,
    journal: Arc<RecordingJournal>,
}

fn world() -> World {
    let clock = Arc::new(VirtualClock::new());
    let vfs = Arc::new(VirtualFs::new(clock.clone()));
    vfs.create_dir_all(&test_path("/data").unwrap()).unwrap();
    for (path, size) in FILES {
        let content = vec![b'x'; usize::try_from(size).unwrap()];
        vfs.write_file(&test_path(path).unwrap(), &content).unwrap();
    }
    let journal = Arc::new(RecordingJournal::keeping(vfs.clone()));
    let frontend = Programs::default()
        .with(
            "find /data |> filter size > 150 |> count",
            program(find_stage(), vec![filter_stage(), count_stage()]),
        )
        .with(
            "find /data |> remove",
            program(find_stage(), vec![remove_stage()]),
        )
        .with(
            "find /data |> filter size > 150 |> remove",
            program(find_stage(), vec![filter_stage(), remove_stage()]),
        )
        .with("find /data", program(find_stage(), Vec::new()));
    let engine = Engine::open(
        EngineConfig::new(clock, Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_fs(vfs.clone())
            .with_journal(journal.clone())
            .with_frontend(Arc::new(frontend))
            .with_stage(Arc::new(Find))
            .with_stage(Arc::new(Remove)),
    )
    .unwrap();
    World {
        engine,
        vfs,
        journal,
    }
}

fn exists(world: &World, path: &str) -> bool {
    world.vfs.exists(&test_path(path).unwrap())
}

fn options(preview: bool) -> RequestOptions {
    RequestOptions {
        preview: Some(preview),
        verbosity: Some(Verbosity::Detailed),
        ..RequestOptions::default()
    }
}

fn run(world: &World, text: &str, preview: bool) -> Response {
    let response = world
        .engine
        .run(&Request::program(text).with_options(options(preview)));
    response.validate().unwrap();
    response
}

#[test]
fn counting_filtered_entries_returns_the_count() {
    let world = world();
    let response = run(&world, "find /data |> filter size > 150 |> count", false);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Ok, "{:?}", report.error);
    assert_eq!(report.kind, "find");
    let page = report.records.as_ref().unwrap();
    assert_eq!(page.record_type, RecordKind::Row);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].get("value"), Some(&Value::from(2_u64)));
    assert!(report.changes.is_empty());
    assert!(report.timing.is_some());
    assert!(exists(&world, "/data/a.log"));
}

#[test]
fn reading_returns_the_records_of_the_source() {
    let world = world();
    let response = run(&world, "find /data", false);
    let page = response.report.records.as_ref().unwrap();
    assert_eq!(page.record_type, RecordKind::Entry);
    assert_eq!(page.rows.len(), 3);
    assert_eq!(page.matched, Some(3));
}

#[test]
fn removing_found_entries_deletes_them_and_reports_the_changes() {
    let world = world();
    let response = run(&world, "find /data |> remove", false);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Ok, "{:?}", report.error);
    assert_eq!(report.kind, "remove");
    assert_eq!(report.changes.len(), 3);
    assert!(
        report
            .changes
            .iter()
            .all(|change| change.action == ChangeAction::Delete)
    );
    assert_eq!(report.targets.as_ref().unwrap().count, 3);
    assert_eq!(report.summary.as_ref().unwrap().files_deleted, Some(3));
    assert!(report.op.is_some());
    for (path, _) in FILES {
        assert!(!exists(&world, path), "{path}");
    }
    assert_eq!(world.journal.committed().len(), 1);
}

#[test]
fn removing_filtered_entries_keeps_the_others() {
    let world = world();
    let response = run(&world, "find /data |> filter size > 150 |> remove", false);
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert_eq!(response.report.changes.len(), 2);
    assert!(exists(&world, "/data/a.log"));
    assert!(!exists(&world, "/data/b.log"));
    assert!(!exists(&world, "/data/c.log"));
}

#[test]
fn preview_plans_the_changes_without_applying_them() {
    let world = world();
    let response = run(&world, "find /data |> remove", true);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Planned, "{:?}", report.error);
    assert_eq!(report.changes.len(), 3);
    assert!(report.op.is_none());
    for (path, _) in FILES {
        assert!(exists(&world, path), "{path}");
    }
    assert!(world.journal.committed().is_empty());
}

fn engine_without_journal() -> Engine {
    let clock = Arc::new(VirtualClock::new());
    let vfs = Arc::new(VirtualFs::new(clock.clone()));
    vfs.create_dir_all(&test_path("/data").unwrap()).unwrap();
    vfs.write_file(&test_path("/data/a.log").unwrap(), b"x")
        .unwrap();
    let frontend = Programs::default()
        .with("find /data", program(find_stage(), Vec::new()))
        .with(
            "find /data |> remove",
            program(find_stage(), vec![remove_stage()]),
        );
    Engine::open(
        EngineConfig::new(clock, Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_fs(vfs)
            .with_frontend(Arc::new(frontend))
            .with_stage(Arc::new(Find))
            .with_stage(Arc::new(Remove)),
    )
    .unwrap()
}

#[test]
fn read_only_program_runs_without_journal() {
    let response = engine_without_journal().run(&Request::program("find /data"));
    response.validate().unwrap();
    assert!(response.report.error.is_none());
    assert!(response.report.records.is_some());
    assert_eq!(response.report.ast_hash.as_deref(), Some(PROGRAM_HASH));
}

#[test]
fn program_with_effects_fails_without_journal() {
    let response = engine_without_journal().run(&Request::program("find /data |> remove"));
    response.validate().unwrap();
    let error = response.report.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::StageFailed);
}

#[test]
fn started_requests_deliver_the_events_of_the_executor_once() {
    let world = world();
    let events = Arc::new(Collect::default());
    let handle = world
        .engine
        .start(&Request::program("find /data |> remove"), events.clone());
    let response = handle.wait(None).unwrap();
    assert_eq!(response.report.status, ReportStatus::Ok);
    let delivered = events.0.lock().unwrap().clone();
    let kinds: Vec<String> = delivered
        .iter()
        .map(|event| {
            serde_json::to_value(event).unwrap()["t"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds.first().map(String::as_str), Some("exec.started"));
    assert_eq!(kinds.last().map(String::as_str), Some("exec.finished"));
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "exec.started").count(),
        1
    );
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "exec.finished").count(),
        1
    );
    let seqs: Vec<u64> = delivered.iter().map(|event| event.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
}

#[test]
fn cancelled_execution_reports_the_cancellation() {
    let world = world();
    let events = Arc::new(Collect::default());
    let handle = world
        .engine
        .start(&Request::program("find /data |> remove"), events);
    handle.cancel();
    let response = handle.wait(None).unwrap();
    response.validate().unwrap();
    match response.report.status {
        ReportStatus::Ok => assert!(!exists(&world, "/data/a.log")),
        _ => {
            assert_eq!(
                response.report.error.as_ref().unwrap().code,
                ErrorCode::Cancelled
            );
            assert!(exists(&world, "/data/a.log"));
        }
    }
}
