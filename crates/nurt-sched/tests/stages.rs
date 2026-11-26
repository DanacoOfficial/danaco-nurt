// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `schedule.*` commands without a running service: definitions written
//! through the journal transaction, listings, history, undo and errors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelToken, EffectStage, ExecCtx, Extensions, StageImpl, StageInput,
};
use nurt_contracts::value::{Duration as ValueDuration, Instant, Record, Value};
use nurt_sched::action::{ActionRegistry, ActionRun, Outcome, ScheduleAction};
use nurt_sched::engine::Scheduler;
use nurt_sched::settings::ScheduleSettings;
use nurt_sched::stages::{ChangeStage, Create, History, List, Run, SERVICE_NOT_RUNNING, stages};
use nurt_sched::store::{FORMAT, Overlap, RunRecord, RunState, RunTrigger, StateFile, Store};
use support::{
    TestHost, TestLink, TestRunner, TestTx, VirtualClock, apply, args, at, create_args, exec_ctx,
    exec_ctx_in_session, records, text, with_link,
};

const T0: &str = "2026-11-22T14:30:00Z";

fn time(clock: &str) -> Instant {
    at(&format!("2026-11-22T{clock}Z"))
}

fn duration(minutes: i64) -> Value {
    Value::Duration(ValueDuration::from_millis(minutes * 60_000))
}

fn label(name: &str) -> (&'static str, Value) {
    ("label", Value::Text(name.to_owned()))
}

/// A temporary state directory and a frozen virtual clock at 14:30.
struct Env {
    dir: tempfile::TempDir,
    clock: Arc<VirtualClock>,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            clock: VirtualClock::frozen(at(T0)),
        }
    }

    fn ctx(&self) -> ExecCtx {
        exec_ctx(self.dir.path(), self.clock.clone(), Extensions::new())
    }

    fn ctx_with(&self, extensions: Extensions) -> ExecCtx {
        exec_ctx(self.dir.path(), self.clock.clone(), extensions)
    }

    fn store(&self) -> Store {
        Store::new(self.dir.path())
    }

    fn create(&self, values: &[(&str, Value)]) -> Record {
        create_in(&self.ctx(), values)
    }

    fn change(&self, stage: ChangeStage, values: &[(&str, Value)]) -> (Vec<Record>, TestTx) {
        let command = stage.descriptor().command;
        let mut tx = TestTx::default();
        let output = apply(&stage, &args(&command, values), &mut tx, &self.ctx()).unwrap();
        (output, tx)
    }

    fn list(&self, values: &[(&str, Value)]) -> Vec<Record> {
        list_in(&self.ctx(), values).unwrap()
    }
}

fn create_in(ctx: &ExecCtx, values: &[(&str, Value)]) -> Record {
    let mut tx = TestTx::default();
    apply(&Create, &create_args(values), &mut tx, ctx)
        .unwrap()
        .remove(0)
}

fn create_error(ctx: &ExecCtx, arguments: &BoundArgs) -> Error {
    Create.plan(arguments, StageInput::none(), ctx).unwrap_err()
}

fn list_in(ctx: &ExecCtx, values: &[(&str, Value)]) -> Result<Vec<Record>, Error> {
    let arguments = args("schedule.list", values);
    let output =
        nurt_contracts::stage::ReadStage::execute(&List, &arguments, StageInput::none(), ctx)?;
    records(output.data)
}

fn history(env: &Env, values: &[(&str, Value)]) -> Result<Vec<Record>, Error> {
    let arguments = args("schedule.history", values);
    let ctx = env.ctx();
    let output =
        nurt_contracts::stage::ReadStage::execute(&History, &arguments, StageInput::none(), &ctx)?;
    records(output.data)
}

fn ids(values: &[&str]) -> Value {
    Value::List(
        values
            .iter()
            .map(|id| Value::Text((*id).to_owned()))
            .collect(),
    )
}

fn every_ten(name: &str) -> [(&'static str, Value); 2] {
    [label(name), ("every", duration(10))]
}

fn instant(record: &Record, name: &str) -> Instant {
    match record.get(name) {
        Some(Value::Instant(instant)) => *instant,
        other => panic!("field {name} is {other:?}"),
    }
}

fn int(record: &Record, name: &str) -> i64 {
    match record.get(name) {
        Some(Value::Int(number)) => *number,
        other => panic!("field {name} is {other:?}"),
    }
}

fn json(record: &Record, name: &str) -> serde_json::Value {
    record
        .get(name)
        .map(Value::to_plain_json)
        .unwrap_or_default()
}

/// Local action kind that writes a line; `text` is required.
struct LogLine;

impl ScheduleAction for LogLine {
    fn kind(&self) -> &str {
        "log_line"
    }

    fn validate(&self, args: &serde_json::Value) -> Result<(), Error> {
        if args
            .get("text")
            .and_then(serde_json::Value::as_str)
            .is_some()
        {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::InvalidValue,
                "`log_line` needs `text`",
            ))
        }
    }

    fn describe(&self, args: &serde_json::Value) -> String {
        format!("log line {}", args["text"])
    }

    fn execute(
        &self,
        _run: &ActionRun,
        _args: &serde_json::Value,
        _cancel: &CancelToken,
    ) -> Outcome {
        Outcome::new(RunState::Ok, "written")
    }
}

fn with_registry() -> Extensions {
    let mut registry = ActionRegistry::new();
    registry.register(Arc::new(LogLine));
    let mut extensions = Extensions::new();
    extensions.insert(Arc::new(registry));
    extensions
}

fn kind_args(kind: &str, action_args: Option<serde_json::Value>) -> BoundArgs {
    let mut arguments = args(
        "schedule.create",
        &[
            label("note"),
            ("every", duration(10)),
            ("action_kind", Value::Text(kind.to_owned())),
        ],
    );
    if let Some(action_args) = action_args {
        arguments.insert(
            "action_args",
            ArgValue::Value(Value::from_plain_json(action_args)),
            false,
        );
    }
    arguments
}

fn run_record(schedule: &str, run: u64, started: &str) -> RunRecord {
    RunRecord {
        schedule: schedule.to_owned(),
        run,
        trigger: RunTrigger::Schedule,
        scheduled_for: Some(time(started)),
        started: time(started),
        elapsed: ValueDuration::from_millis(1_500),
        state: RunState::Ok,
        exit_code: Some(0),
        reason: None,
        operation: Some(run + 40),
        job: None,
        output_file: None,
        output_sha256: None,
        changed: Some(true),
        message: None,
    }
}

#[test]
fn tc_01_every_ten_minutes_runs_at_14_40() {
    let env = Env::new();
    let record = env.create(&[label("t"), ("every", duration(10))]);
    assert_eq!(text(&record, "id"), "s1");
    assert_eq!(instant(&record, "next_run"), time("14:40:00"));
    assert_eq!(text(&record, "state"), "active");
    let trigger = json(&record, "trigger");
    assert_eq!(trigger["expression"], "10m");
    assert_eq!(trigger["timezone"], "Europe/Warsaw");
    assert_eq!(text(&record, "action_kind"), "pipeline");
    assert!(text(&record, "action").starts_with("run"));
    let saved = env.store().load_definition("s1").unwrap().unwrap();
    assert_eq!(saved.format, FORMAT);
    assert!(saved.ast_hash.starts_with("sha256:"));
}

#[test]
fn tc_09_single_time_is_created_and_a_past_time_is_rejected() {
    let env = Env::new();
    let record = env.create(&[label("once"), ("at", Value::Instant(time("14:40:00")))]);
    assert_eq!(instant(&record, "next_run"), time("14:40:00"));
    let past = create_args(&[
        label("late"),
        ("at", Value::Instant(at("2026-11-01T00:00:00Z"))),
    ]);
    let error = create_error(&env.ctx(), &past);
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn tc_10_trigger_parameters_exclude_each_other_and_one_is_required() {
    let env = Env::new();
    let both = create_args(&[
        label("t"),
        ("cron", Value::Text("*/10 * * * *".to_owned())),
        ("every", duration(10)),
    ]);
    let conflict = create_error(&env.ctx(), &both);
    assert_eq!(conflict.code(), ErrorCode::ParamConflict);
    assert_eq!(
        conflict.expected,
        Some(serde_json::json!(["cron", "every", "at"]))
    );
    let none = create_args(&[label("t")]);
    assert_eq!(
        create_error(&env.ctx(), &none).code(),
        ErrorCode::MissingParam
    );
}

#[test]
fn tc_21_pause_of_a_paused_schedule_is_unchanged() {
    let env = Env::new();
    env.create(&every_ten("t"));
    let (paused, _) = env.change(ChangeStage::pause(), &[("id", ids(&["s1"]))]);
    assert_eq!(text(&paused[0], "action"), "pause");
    let (again, tx) = env.change(ChangeStage::pause(), &[("id", ids(&["s1"]))]);
    assert_eq!(text(&again[0], "action"), "unchanged");
    assert_eq!(text(&again[0], "before"), "paused");
    assert!(tx.paths().is_empty());
    let (resumed, _) = env.change(ChangeStage::resume(), &[("id", ids(&["s1"]))]);
    assert_eq!(text(&resumed[0], "action"), "resume");
    let (active, _) = env.change(ChangeStage::resume(), &[("id", ids(&["s1"]))]);
    assert_eq!(text(&active[0], "action"), "unchanged");
}

#[test]
fn tc_23_removing_two_schedules_is_one_operation_with_one_undo() {
    let env = Env::new();
    env.create(&every_ten("a"));
    env.create(&every_ten("b"));
    let store = env.store();
    let preview_args = args("schedule.remove", &[("id", ids(&["s1", "s2"]))]);
    let plan = ChangeStage::remove()
        .plan(&preview_args, StageInput::none(), &env.ctx())
        .unwrap();
    assert_eq!(plan.preview.len(), 2);
    assert!(plan.bulk);
    assert!(store.definition_path("s1").exists());

    let (removed, tx) = env.change(ChangeStage::remove(), &[("id", ids(&["s1", "s2"]))]);
    assert_eq!(removed.len(), 2);
    assert!(
        removed
            .iter()
            .all(|record| text(record, "action") == "remove")
    );
    assert!(!store.definition_path("s1").exists());
    assert!(!store.definition_path("s2").exists());
    tx.undo();
    assert!(store.definition_path("s1").exists());
    assert!(store.definition_path("s2").exists());
}

#[test]
fn tc_24_list_filters_by_state_and_shows_details_of_one_schedule() {
    let env = Env::new();
    for name in ["a", "b", "c"] {
        env.create(&every_ten(name));
    }
    env.change(ChangeStage::pause(), &[("id", ids(&["s2"]))]);
    let store = env.store();
    let mut scheduler = Scheduler::new(ScheduleSettings::default(), StateFile::default());
    scheduler.load(store.load_definitions().unwrap().items, at(T0));
    for launch in scheduler.tick(time("14:40:00")) {
        scheduler.finish(
            &launch.id(),
            &Outcome::new(RunState::Ok, "done"),
            time("14:40:02"),
        );
    }
    store.write_state(scheduler.state()).unwrap();

    let paused = env.list(&[("state", Value::Text("paused".to_owned()))]);
    assert_eq!(paused.len(), 1);
    assert_eq!(text(&paused[0], "id"), "s2");
    let detail = env.list(&[("id", Value::Text("s3".to_owned()))]);
    assert_eq!(detail.len(), 1);
    assert_eq!(instant(&detail[0], "last_run"), time("14:40:00"));
    assert_eq!(int(&detail[0], "runs"), 1);
    assert_eq!(text(&detail[0], "last_result"), "ok");
    assert_eq!(instant(&detail[0], "next_run"), time("14:50:00"));
    assert_eq!(env.list(&[]).len(), 3);
}

#[test]
fn tc_25_max_active_counts_paused_schedules() {
    let env = Env::new();
    env.create(&[
        label("paused"),
        ("every", duration(10)),
        ("enabled", Value::Bool(false)),
    ]);
    for index in 1..50 {
        env.create(&every_ten(&format!("t{index}")));
    }
    let error = create_error(&env.ctx(), &create_args(&every_ten("one more")));
    assert_eq!(error.code(), ErrorCode::Limit);
    assert!(
        error.message.contains("schedule.max_active"),
        "{}",
        error.message
    );
}

#[test]
fn tc_26_create_without_service_reports_inactive() {
    let env = Env::new();
    let record = env.create(&every_ten("t"));
    assert!(matches!(record.get("active"), Some(Value::Bool(false))));
    assert_eq!(text(&record, "reason"), SERVICE_NOT_RUNNING);
}

#[test]
fn create_service_not_running_reported() {
    let env = Env::new();
    let link = TestLink::stopped();
    let ctx = env.ctx_with(with_link(link.clone()));
    let record = create_in(
        &ctx,
        &[
            label("t"),
            ("every", duration(10)),
            ("run_now", Value::Bool(true)),
        ],
    );
    assert!(matches!(record.get("active"), Some(Value::Bool(false))));
    assert_eq!(text(&record, "reason"), SERVICE_NOT_RUNNING);
    assert!(link.ops().is_empty());
}

#[test]
fn action_kind_unknown_candidates() {
    let env = Env::new();
    let error = create_error(&env.ctx(), &kind_args("session_message", None));
    assert_eq!(error.code(), ErrorCode::UnknownValue);
    assert_eq!(error.found.as_deref(), Some("session_message"));
    assert_eq!(error.expected, Some(serde_json::json!(["pipeline"])));

    let ctx = env.ctx_with(with_registry());
    let error = create_error(&ctx, &kind_args("session_message", None));
    assert_eq!(
        error.expected,
        Some(serde_json::json!(["pipeline", "log_line"]))
    );
    let invalid = create_error(&ctx, &kind_args("log_line", Some(serde_json::json!({}))));
    assert_eq!(invalid.code(), ErrorCode::InvalidValue);

    let mut tx = TestTx::default();
    let arguments = kind_args("log_line", Some(serde_json::json!({ "text": "hello" })));
    let record = apply(&Create, &arguments, &mut tx, &ctx).unwrap().remove(0);
    assert_eq!(text(&record, "action_kind"), "log_line");
    assert_eq!(text(&record, "action"), "log line \"hello\"");
    let saved = env.store().load_definition("s1").unwrap().unwrap();
    assert_eq!(
        saved.action_args,
        Some(serde_json::json!({ "text": "hello" }))
    );
    assert!(saved.pipeline.is_none());
}

#[test]
fn action_and_action_kind_exclude_each_other() {
    let env = Env::new();
    let mut both = kind_args("log_line", None);
    both.insert(
        "action",
        ArgValue::Pipeline(support::pipeline("true")),
        false,
    );
    assert_eq!(
        create_error(&env.ctx(), &both).code(),
        ErrorCode::ParamConflict
    );
}

#[test]
fn history_limit_and_order() {
    let env = Env::new();
    let mut state = StateFile::default();
    state.history.insert(
        "s1".to_owned(),
        vec![
            run_record("s1", 1, "14:40:00"),
            run_record("s1", 2, "15:00:00"),
        ],
    );
    state.history.insert(
        "s2".to_owned(),
        vec![
            run_record("s2", 1, "14:50:00"),
            run_record("s2", 2, "15:10:00"),
        ],
    );
    env.store().write_state(&state).unwrap();

    let newest = history(&env, &[("limit", Value::Int(3))]).unwrap();
    let order: Vec<(String, Instant)> = newest
        .iter()
        .map(|record| (text(record, "schedule_id"), instant(record, "started")))
        .collect();
    assert_eq!(
        order,
        vec![
            ("s2".to_owned(), time("15:10:00")),
            ("s1".to_owned(), time("15:00:00")),
            ("s2".to_owned(), time("14:50:00"))
        ]
    );
    assert_eq!(int(&newest[0], "operation"), 42);
    assert!(text(&newest[0], "message").contains("#2 schedule ok"));

    let one = history(&env, &[("id", Value::Text("s1".to_owned()))]).unwrap();
    assert_eq!(one.len(), 2);
    assert_eq!(history(&env, &[]).unwrap().len(), 4);
    let zero = history(&env, &[("limit", Value::Int(0))]).unwrap_err();
    assert_eq!(zero.code(), ErrorCode::InvalidValue);
    let unknown = history(&env, &[("id", Value::Text("s7".to_owned()))]).unwrap_err();
    assert_eq!(unknown.code(), ErrorCode::NotFound);
}

#[test]
fn origin_session_filter() {
    let env = Env::new();
    let in_a = exec_ctx_in_session(
        env.dir.path(),
        env.clock.clone(),
        Extensions::new(),
        Some("a"),
    );
    let in_b = exec_ctx_in_session(
        env.dir.path(),
        env.clock.clone(),
        Extensions::new(),
        Some("b"),
    );
    create_in(&in_a, &every_ten("from a"));
    let from_b = create_in(&in_b, &every_ten("from b"));
    assert_eq!(text(&from_b, "session"), "b");

    let listed = env.list(&[("origin_session", Value::Text("a".to_owned()))]);
    assert_eq!(listed.len(), 1);
    assert_eq!(text(&listed[0], "id"), "s1");

    let (paused, _) = env.change(
        ChangeStage::pause(),
        &[("origin_session", Value::Text("b".to_owned()))],
    );
    assert_eq!(paused.len(), 1);
    assert_eq!(text(&paused[0], "target"), "schedule:s2");
    assert!(!env.store().load_definition("s1").unwrap().unwrap().paused);
    assert!(env.store().load_definition("s2").unwrap().unwrap().paused);
}

#[test]
fn state_file_outside_journal() {
    let env = Env::new();
    let store = env.store();
    let mut tx = TestTx::default();
    apply(&Create, &create_args(&every_ten("t")), &mut tx, &env.ctx()).unwrap();
    let (_, pause) = env.change(ChangeStage::pause(), &[("id", ids(&["s1"]))]);
    let (_, remove) = env.change(ChangeStage::remove(), &[("id", ids(&["s1"]))]);
    let declared: Vec<PathBuf> = [tx.paths(), pause.paths(), remove.paths()].concat();
    assert_eq!(
        declared,
        vec![
            store.root().to_path_buf(),
            store.definitions_dir(),
            store.definition_path("s1"),
            store.definition_path("s1"),
            store.definition_path("s1"),
        ]
    );
    assert!(!store.state_path().exists());
}

#[test]
fn remove_undo_restores_next_run_from_now() {
    let env = Env::new();
    env.create(&every_ten("t"));
    env.clock.set(time("15:30:00"));
    let (_, tx) = env.change(ChangeStage::remove(), &[("id", ids(&["s1"]))]);
    assert!(env.list(&[]).is_empty());
    tx.undo();
    let restored = env.list(&[]);
    assert_eq!(restored.len(), 1);
    assert_eq!(instant(&restored[0], "next_run"), time("15:40:00"));
}

#[test]
fn expires_duration() {
    let env = Env::new();
    env.create(&[
        label("t"),
        ("every", duration(10)),
        ("expires", duration(25)),
    ]);
    let saved = env.store().load_definition("s1").unwrap().unwrap();
    assert_eq!(saved.expires, Some(time("14:55:00")));
    let past = create_args(&[
        label("t"),
        ("every", duration(10)),
        ("expires", Value::Instant(time("14:00:00"))),
    ]);
    assert_eq!(
        create_error(&env.ctx(), &past).code(),
        ErrorCode::InvalidValue
    );
}

#[test]
fn create_saves_options_and_rejects_invalid_ones() {
    let env = Env::new();
    let record = env.create(&[
        label("t"),
        ("cron", Value::Text("0 3 * * *".to_owned())),
        ("timezone", Value::Text("UTC".to_owned())),
        ("overlap", Value::Text("queue".to_owned())),
        ("on_missed", Value::Text("run_once".to_owned())),
        ("misfire", duration(5)),
        ("timeout", duration(30)),
        ("max_runs", Value::Int(4)),
        ("enabled", Value::Bool(false)),
    ]);
    assert_eq!(text(&record, "state"), "paused");
    assert_eq!(instant(&record, "next_run"), at("2026-11-23T03:00:00Z"));
    let saved = env.store().load_definition("s1").unwrap().unwrap();
    assert_eq!(saved.overlap, Overlap::Queue);
    assert_eq!(saved.max_runs, Some(4));
    assert_eq!(saved.misfire, ValueDuration::from_millis(300_000));
    assert!(saved.paused);

    let ctx = env.ctx();
    let cases: [(&str, Value, ErrorCode); 5] = [
        ("timeout", duration(25 * 60), ErrorCode::InvalidValue),
        (
            "on_missed",
            Value::Text("always".to_owned()),
            ErrorCode::UnknownValue,
        ),
        ("max_runs", Value::Int(0), ErrorCode::InvalidValue),
        (
            "timezone",
            Value::Text("Europe/Warsw".to_owned()),
            ErrorCode::InvalidValue,
        ),
        (
            "misfire",
            Value::Duration(ValueDuration::from_millis(-1)),
            ErrorCode::InvalidValue,
        ),
    ];
    for (name, value, code) in cases {
        let arguments = create_args(&[label("t"), ("every", duration(10)), (name, value)]);
        assert_eq!(create_error(&ctx, &arguments).code(), code, "{name}");
    }
    let wrong_type = create_args(&[label("t"), ("every", Value::Int(10))]);
    assert_eq!(create_error(&ctx, &wrong_type).code(), ErrorCode::Type);
    let no_label = create_args(&[("every", duration(10))]);
    assert_eq!(
        create_error(&ctx, &no_label).code(),
        ErrorCode::MissingParam
    );
}

#[test]
fn run_without_service_runs_the_pipeline_here() {
    let env = Env::new();
    env.create(&every_ten("t"));
    let runner = TestRunner::printing("hello\n", 0);
    let ctx = ExecCtx::builder(env.clock.clone(), TestHost::new("UTC"))
        .state_dir(env.dir.path().to_path_buf())
        .pipelines(runner.clone())
        .build();
    let mut tx = TestTx::default();
    let output = apply(
        &Run,
        &args("schedule.run", &[("id", Value::Text("s1".to_owned()))]),
        &mut tx,
        &ctx,
    )
    .unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(text(&output[0], "stdout"), "hello\n");
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
    assert!(!env.store().state_path().exists());
}

#[test]
fn run_is_busy_while_another_process_runs_the_schedule() {
    let env = Env::new();
    env.create(&every_ten("t"));
    let _held = env.store().try_run_lock("s1").unwrap().unwrap();
    let ctx = ExecCtx::builder(env.clock.clone(), TestHost::new("UTC"))
        .state_dir(env.dir.path().to_path_buf())
        .pipelines(TestRunner::printing("", 0))
        .build();
    let mut tx = TestTx::default();
    let error = apply(
        &Run,
        &args("schedule.run", &[("id", Value::Text("s1".to_owned()))]),
        &mut tx,
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::JobState);
}

#[test]
fn run_reports_unknown_schedules_and_service_only_kinds() {
    let env = Env::new();
    let ctx = env.ctx_with(with_registry());
    let mut tx = TestTx::default();
    let arguments = kind_args("log_line", Some(serde_json::json!({ "text": "hello" })));
    apply(&Create, &arguments, &mut tx, &ctx).unwrap();
    let run = |id: &str| {
        let mut tx = TestTx::default();
        apply(
            &Run,
            &args("schedule.run", &[("id", Value::Text(id.to_owned()))]),
            &mut tx,
            &ctx,
        )
        .unwrap_err()
    };
    assert_eq!(run("s9").code(), ErrorCode::NotFound);
    assert_eq!(run("s1").code(), ErrorCode::StageFailed);
    assert_eq!(run("../s1").code(), ErrorCode::InvalidValue);
    let mut tx = TestTx::default();
    let missing = apply(&Run, &args("schedule.run", &[]), &mut tx, &ctx).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::MissingParam);
}

#[test]
fn change_selection_errors() {
    let env = Env::new();
    env.create(&every_ten("t"));
    let ctx = env.ctx();
    let plan_error = |values: &[(&str, Value)]| {
        ChangeStage::remove()
            .plan(&args("schedule.remove", values), StageInput::none(), &ctx)
            .unwrap_err()
    };
    let both = plan_error(&[("id", ids(&["s1"])), ("all", Value::Bool(true))]);
    assert_eq!(both.code(), ErrorCode::ParamConflict);
    assert_eq!(plan_error(&[]).code(), ErrorCode::MissingParam);
    let missing = plan_error(&[("id", ids(&["s1", "s9"]))]);
    assert_eq!(missing.code(), ErrorCode::NotFound);
    assert_eq!(missing.actual, Some(serde_json::json!(["s9"])));
    let single = plan_error(&[("id", Value::Text("bad id".to_owned()))]);
    assert_eq!(single.code(), ErrorCode::InvalidValue);
    let (all, _) = env.change(ChangeStage::remove(), &[("all", Value::Bool(true))]);
    assert_eq!(all.len(), 1);
}

#[test]
fn list_errors() {
    let env = Env::new();
    let ctx = env.ctx();
    let unknown = list_in(&ctx, &[("id", Value::Text("s4".to_owned()))]).unwrap_err();
    assert_eq!(unknown.code(), ErrorCode::NotFound);
    let state = list_in(&ctx, &[("state", Value::Text("sleeping".to_owned()))]).unwrap_err();
    assert_eq!(state.code(), ErrorCode::InvalidValue);
    assert!(list_in(&ctx, &[]).unwrap().is_empty());
}

#[test]
fn frozen_clock_sets_the_wall_time_only_and_a_cancelled_sleep_ends_at_once() {
    use nurt_contracts::env::Clock;
    use nurt_contracts::stage::CancelReason;

    let clock = VirtualClock::frozen(at(T0));
    let before = clock.monotonic();
    clock.set(at("2030-01-01T00:00:00Z"));
    assert_eq!(clock.monotonic(), before);
    assert_eq!(
        clock.now().unix_nanos(),
        i128::from(at("2030-01-01T00:00:00Z").as_unix_nanos())
    );
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Requested);
    let slept = clock.sleep(std::time::Duration::from_secs(3600), &cancel);
    assert_eq!(slept, Err(CancelReason::Requested));
    assert_eq!(clock.monotonic(), before);
}

#[test]
fn commands_without_state_directory_fail() {
    let clock = VirtualClock::frozen(at(T0));
    let ctx = ExecCtx::builder(clock, TestHost::new("UTC")).build();
    let error = list_in(&ctx, &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
}

#[test]
fn catalog_lists_every_command() {
    let catalog_path = format!(
        "{}/../../catalog/schedule.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let catalog = std::fs::read_to_string(catalog_path).unwrap();
    let commands: Vec<String> = stages()
        .iter()
        .map(|stage| stage.descriptor().command)
        .collect();
    assert_eq!(commands.len(), 7);
    for command in &commands {
        assert!(
            catalog.contains(&format!("name = \"{command}\"")),
            "{command}"
        );
    }
    let listed = catalog.matches("name = \"schedule.").count();
    assert_eq!(listed, commands.len());
}

#[test]
fn run_without_service_refuses_a_definition_changed_outside_the_commands() {
    let env = Env::new();
    env.create(&every_ten("t"));
    let path = env.store().definition_path("s1");
    let text = std::fs::read_to_string(&path).unwrap();
    let tampered = text.replace(
        "\"ast_hash\": \"sha256:",
        "\"ast_hash\": \"sha256:0000000000000000",
    );
    assert_ne!(tampered, text);
    std::fs::write(&path, tampered).unwrap();
    let runner = TestRunner::printing("hello\n", 0);
    let ctx = ExecCtx::builder(env.clock.clone(), TestHost::new("UTC"))
        .state_dir(env.dir.path().to_path_buf())
        .pipelines(runner.clone())
        .build();
    let mut tx = TestTx::default();
    let error = apply(
        &Run,
        &args("schedule.run", &[("id", Value::Text("s1".to_owned()))]),
        &mut tx,
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(runner.calls.lock().unwrap().is_empty());
}
