// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sched/tests/module_host.rs
// Responsibility:   The schedule module under a test host: start and stop, control requests.
// Dependencies:     nurt_contracts, serde_json, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The schedule module under a test host: start and stop, control requests,
//! remote action kinds, busy runs, stopped runs and undo of a removal.
//!
//! The module runs on its own thread with a frozen virtual clock. A test
//! moves the clock and sends `schedule.changed`, which wakes the module, and
//! then waits for the events the module publishes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::report::Irreversible;
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx, Extensions};
use nurt_contracts::value::{Duration as ValueDuration, Instant, Value};
use nurt_sched::action::args_hash;
use nurt_sched::engine::Scheduler;
use nurt_sched::module::{
    EventTarget, ModuleHealth, ModuleRequest, RunContextFactory, SCHEDULE_MODULE, ScheduleModule,
    ServiceCtx, ServiceModule,
};
use nurt_sched::settings::ScheduleSettings;
use nurt_sched::stages::{ChangeStage, Create};
use nurt_sched::store::{Definition, StateFile, Store, encode_definition};
use serde_json::{Value as Json, json};
use support::{
    TestEvents, TestFactory, TestLink, TestRunner, TestTx, VirtualClock, WAIT, apply, args, at,
    create_args, definition, every, exec_ctx, exec_record, minutes, text, with_link,
};

const T0: &str = "2026-11-22T14:30:00Z";
const POLL: Duration = Duration::from_millis(2);

fn time(clock: &str) -> Instant {
    at(&format!("2026-11-22T{clock}Z"))
}

fn ten_minutes(id: &str) -> Definition {
    definition(id, every(minutes(10)), at(T0))
}

fn reminder(id: &str) -> Definition {
    let arguments = json!({ "text": "reminder" });
    let mut reminder = ten_minutes(id);
    reminder.action_kind = "session_message".to_owned();
    reminder.ast_hash = args_hash("session_message", Some(&arguments)).unwrap();
    reminder.pipeline = None;
    reminder.action_args = Some(arguments);
    reminder.action = "session_message".to_owned();
    reminder
}

fn write_definitions(store: &Store, definitions: &[Definition]) {
    std::fs::create_dir_all(store.definitions_dir()).unwrap();
    for definition in definitions {
        std::fs::write(
            store.definition_path(&definition.id),
            encode_definition(definition).unwrap(),
        )
        .unwrap();
    }
}

/// A schedule module started on its own thread over a temporary state
/// directory.
struct Harness {
    dir: tempfile::TempDir,
    clock: Arc<VirtualClock>,
    module: ScheduleModule,
    events: Arc<TestEvents>,
    factory: Arc<TestFactory>,
    stop: CancelToken,
    thread: Option<JoinHandle<Result<()>>>,
}

impl Harness {
    fn start(definitions: Vec<Definition>, runner: Arc<TestRunner>) -> Self {
        Self::start_with(T0, None, runner, |store| {
            write_definitions(store, &definitions)
        })
    }

    fn start_with(
        now: &str,
        settings: Option<ScheduleSettings>,
        runner: Arc<TestRunner>,
        prepare: impl FnOnce(&Store),
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        prepare(&Store::new(dir.path()));
        let clock = VirtualClock::frozen(at(now));
        let factory = TestFactory::new(clock.clone(), runner);
        let mut extensions = Extensions::new();
        extensions.insert::<dyn RunContextFactory>(factory.clone());
        if let Some(settings) = settings {
            extensions.insert(Arc::new(settings));
        }
        let events = TestEvents::new();
        let stop = CancelToken::new();
        let ctx = ServiceCtx {
            clock: clock.clone(),
            config: Arc::new(Config::default()),
            state_dir: dir.path().to_path_buf(),
            events: events.clone(),
            stop: stop.clone(),
            extensions: Arc::new(extensions),
        };
        let module = ScheduleModule::new();
        let running = module.clone();
        let thread = thread::spawn(move || running.start(ctx));
        let harness = Self {
            dir,
            clock,
            module,
            events,
            factory,
            stop,
            thread: Some(thread),
        };
        harness.wait_until_started();
        harness
    }

    fn wait_until_started(&self) {
        let deadline = std::time::Instant::now() + WAIT;
        while self.request("schedule.status", json!({})).is_err() {
            assert!(
                std::time::Instant::now() < deadline,
                "the module did not start"
            );
            thread::sleep(POLL);
        }
    }

    fn request(&self, op: &str, body: Json) -> Result<Json> {
        self.module.request(ModuleRequest::new(op, body))
    }

    fn request_from(&self, client: &str, op: &str, body: Json) -> Result<Json> {
        self.module
            .request(ModuleRequest::new(op, body).from_client(client))
    }

    /// Moves the clock to `clock` and wakes the module.
    fn advance_to(&self, clock: &str) {
        self.clock.set(time(clock));
        self.request("schedule.changed", json!({})).unwrap();
    }

    fn store(&self) -> Store {
        Store::new(self.dir.path())
    }

    /// Execution context of the schedule commands connected to the module.
    fn ctx(&self) -> ExecCtx {
        let link = TestLink::to(self.module.clone());
        exec_ctx(self.dir.path(), self.clock.clone(), with_link(link))
    }

    fn finished(&self, count: usize) -> Vec<Json> {
        let events = self.events.wait_for("schedule.finished", count);
        assert_eq!(events.len(), count, "{events:?}");
        events
            .into_iter()
            .map(|event| event.data["schedule_run"].clone())
            .collect()
    }

    fn shutdown(mut self) -> ModuleHealth {
        self.module.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap().unwrap();
        }
        self.module.health()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.module.stop();
        self.stop.cancel(CancelReason::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Runner whose runs end when `release` is cancelled.
fn gated(release: &CancelToken) -> Arc<TestRunner> {
    let gate = release.clone();
    TestRunner::new(move |_| {
        let _ = gate.wait_timeout(WAIT);
        Ok(vec![exec_record("done\n", 0)])
    })
}

/// Runner whose runs end when the run is cancelled.
fn until_cancelled() -> Arc<TestRunner> {
    TestRunner::new(|ctx| match ctx.cancel().wait_timeout(WAIT) {
        Some(_) => Err(Error::new(ErrorCode::Cancelled, "the run was stopped")),
        None => Ok(Vec::new()),
    })
}

#[test]
fn tc_14_run_busy_then_manual_run_after_the_scheduled_one() {
    let release = CancelToken::new();
    let harness = Harness::start(vec![ten_minutes("s1")], gated(&release));
    harness.advance_to("14:40:00");
    let fired = harness.events.wait_for("schedule.fired", 1);
    assert_eq!(fired[0].data["id"], "s1");
    assert_eq!(fired[0].data["trigger"], "schedule");
    assert_eq!(fired[0].target, EventTarget::All);

    let busy = harness
        .request("schedule.run", json!({ "id": "s1" }))
        .unwrap_err();
    assert_eq!(busy.code(), ErrorCode::JobState);
    assert_eq!(busy.actual, Some(json!("s1#1")));
    let status = harness.request("schedule.status", json!({})).unwrap();
    assert_eq!(status["running"], json!(["s1#1"]));

    release.cancel(CancelReason::Requested);
    assert_eq!(harness.finished(1)[0]["state"], "ok");
    let reply = harness
        .request("schedule.run", json!({ "id": "s1" }))
        .unwrap();
    assert_eq!(reply["run"]["trigger"], "manual");
    assert_eq!(reply["run"]["state"], "ok");
    assert_eq!(reply["output"].as_array().map(Vec::len), Some(1));

    let history = harness.store().read_state().unwrap().history["s1"].clone();
    let triggers: Vec<&str> = history.iter().map(|run| run.trigger.as_str()).collect();
    assert_eq!(triggers, vec!["schedule", "manual"]);
}

#[test]
fn run_busy() {
    let release = CancelToken::new();
    let harness = Harness::start(vec![ten_minutes("s1")], gated(&release));
    harness.advance_to("14:40:00");
    harness.events.wait_for("schedule.fired", 1);
    let ctx = harness.ctx();
    let mut tx = TestTx::default();
    let error = apply(
        &nurt_sched::stages::Run,
        &args("schedule.run", &[("id", Value::Text("s1".to_owned()))]),
        &mut tx,
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::JobState);
    release.cancel(CancelReason::Requested);
}

#[test]
fn tc_20_time_limit_ends_the_run_with_timeout() {
    let runner = TestRunner::new(|_| {
        Err(Error::new(
            ErrorCode::Timeout,
            "the program exceeded its time limit",
        ))
    });
    let mut limited = ten_minutes("s1");
    limited.timeout = ValueDuration::from_millis(30_000);
    let harness = Harness::start(vec![limited], runner);
    harness.advance_to("14:40:00");
    let run = harness.finished(1).remove(0);
    assert_eq!(run["state"], "timeout");
    assert_eq!(run["exit_code"], -1);
    let requests = harness.factory.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Duration::from_secs(30));
    assert_eq!(requests[0].session, "schedule:s1");
    assert_eq!(requests[0].run, 1);
}

#[test]
fn tc_27_remote_action_fired_to_its_client_and_acknowledged() {
    let harness = Harness::start(vec![reminder("s1")], TestRunner::printing("", 0));
    let registered = harness
        .request_from(
            "c1",
            "schedule/register_action",
            json!({ "kind": "session_message" }),
        )
        .unwrap();
    assert_eq!(registered["kind"], "session_message");
    let status = harness.request("schedule.status", json!({})).unwrap();
    assert_eq!(status["action_kinds"], json!(["session_message"]));

    harness.advance_to("14:40:00");
    let fired = harness.events.wait_for("schedule.fired", 1).remove(0);
    assert_eq!(fired.target, EventTarget::Client("c1".to_owned()));
    assert_eq!(fired.data["action_kind"], "session_message");
    assert_eq!(fired.data["action_args"]["text"], "reminder");
    let token = fired.data["ack_token"].as_str().unwrap().to_owned();
    let acknowledged = harness
        .request(
            "schedule/ack",
            json!({ "token": token, "state": "ok", "summary": "delivered" }),
        )
        .unwrap();
    assert_eq!(acknowledged, json!({}));
    let first = harness.finished(1).remove(0);
    assert_eq!(first["state"], "ok");
    assert_eq!(first["message"], "delivered");
    let late = harness
        .request("schedule/ack", json!({ "token": token }))
        .unwrap_err();
    assert_eq!(late.code(), ErrorCode::NotFound);

    harness.module.client_closed("c1");
    harness.advance_to("14:50:00");
    let second = harness.finished(2).remove(1);
    assert_eq!(second["state"], "skipped");
    assert_eq!(second["reason"], "action_unavailable");
}

#[test]
fn an_acknowledgement_accepts_only_the_states_of_a_finished_action() {
    let harness = Harness::start(vec![reminder("s1")], TestRunner::printing("", 0));
    for state in ["skipped", "rejected", "bogus"] {
        let error = harness
            .request("schedule/ack", json!({ "token": "t", "state": state }))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg, "{state}");
    }
}

#[test]
fn remote_action_ack_timeout_skipped() {
    let settings = ScheduleSettings {
        action_ack_timeout: Duration::ZERO,
        ..ScheduleSettings::default()
    };
    let harness = Harness::start_with(T0, Some(settings), TestRunner::printing("", 0), |store| {
        write_definitions(store, &[reminder("s1")]);
    });
    harness
        .request_from(
            "c1",
            "schedule/register_action",
            json!({ "kind": "session_message" }),
        )
        .unwrap();
    harness.advance_to("14:40:00");
    let run = harness.finished(1).remove(0);
    assert_eq!(run["state"], "skipped");
    assert_eq!(run["reason"], "action_unavailable");
    assert_eq!(harness.events.named("schedule.fired").len(), 1);
}

#[test]
fn remove_stop_running_kills_job() {
    let harness = Harness::start(vec![ten_minutes("s1")], until_cancelled());
    harness.advance_to("14:40:00");
    harness.events.wait_for("schedule.fired", 1);

    let ctx = harness.ctx();
    let mut tx = TestTx::default();
    let removed = apply(
        &ChangeStage::remove(),
        &args(
            "schedule.remove",
            &[("id", Value::List(vec![Value::Text("s1".to_owned())]))],
        ),
        &mut tx,
        &ctx,
    )
    .unwrap();
    let after = removed[0].get("after").map(Value::to_plain_json).unwrap();
    assert_eq!(after["stopped_jobs"], json!(["s1#1"]));
    assert_eq!(tx.irreversible.len(), 1);
    assert_eq!(tx.irreversible[0].reason, Irreversible::ProcessStopped);
    assert_eq!(harness.finished(1)[0]["state"], "stopped");
    assert!(!harness.store().definition_path("s1").exists());
    assert!(harness.store().read_state().unwrap().schedules.is_empty());

    tx.undo();
    harness.advance_to("15:30:00");
    let state = harness.store().read_state().unwrap();
    assert_eq!(state.schedules["s1"].next_run, Some(time("15:40:00")));
    assert_eq!(state.schedules["s1"].runs, 1);
}

#[test]
fn tc_13_run_now_through_the_service() {
    let harness = Harness::start(Vec::new(), TestRunner::printing("hello\n", 0));
    let ctx = harness.ctx();
    let mut tx = TestTx::default();
    let values = [
        ("label", Value::Text("t".to_owned())),
        (
            "every",
            Value::Duration(ValueDuration::from_millis(600_000)),
        ),
        ("run_now", Value::Bool(true)),
    ];
    let record = apply(&Create, &create_args(&values), &mut tx, &ctx)
        .unwrap()
        .remove(0);
    assert!(matches!(record.get("active"), Some(Value::Bool(true))));
    assert!(
        record
            .get("reason")
            .is_none_or(|reason| matches!(reason, Value::Null))
    );
    assert_eq!(text(&record, "id"), "s1");

    // The creation does not wait for the first run, so the record of the run
    // appears in the history a moment later.
    let begun = std::time::Instant::now();
    let state = loop {
        let state = harness.store().read_state().unwrap();
        if state
            .history
            .get("s1")
            .is_some_and(|history| !history.is_empty())
        {
            break state;
        }
        assert!(
            begun.elapsed() < std::time::Duration::from_secs(20),
            "the run of the new schedule left no record"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let entry = &state.schedules["s1"];
    assert_eq!(entry.next_run, Some(time("14:40:00")));
    assert_eq!(entry.run_count, 0);
    let history = &state.history["s1"];
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].trigger.as_str(), "run_now");
    assert!(history[0].output_sha256.is_some());
}

#[test]
fn restart_service_recovers_due_run_on_start() {
    let harness = Harness::start_with(
        "2026-11-22T14:41:00Z",
        None,
        TestRunner::printing("", 0),
        |store| {
            let definitions = vec![ten_minutes("s1")];
            let mut before_stop = Scheduler::new(ScheduleSettings::default(), StateFile::default());
            before_stop.load(definitions.clone(), at(T0));
            store.write_state(before_stop.state()).unwrap();
            write_definitions(store, &definitions);
        },
    );
    let fired = harness.events.wait_for("schedule.fired", 1).remove(0);
    assert_eq!(fired.data["trigger"], "schedule");
    assert_eq!(
        fired.data["scheduled_for"],
        serde_json::to_value(time("14:40:00")).unwrap()
    );
    harness.finished(1);
}

#[test]
fn invalid_definition_file_degrades_health() {
    let harness = Harness::start_with(T0, None, TestRunner::printing("", 0), |store| {
        write_definitions(store, &[ten_minutes("s1")]);
        std::fs::write(store.definitions_dir().join("s9.json"), b"{ not json").unwrap();
    });
    match harness.module.health() {
        ModuleHealth::Degraded { reason } => assert!(reason.contains("s9.json"), "{reason}"),
        other => panic!("health is {other:?}"),
    }
    let status = harness.request("schedule.status", json!({})).unwrap();
    assert_eq!(status["schedules"], 1);
}

#[test]
fn control_requests_report_their_errors() {
    let idle = ScheduleModule::new();
    assert_eq!(idle.name(), SCHEDULE_MODULE);
    assert_eq!(idle.health(), ModuleHealth::Starting);
    let not_started = idle
        .request(ModuleRequest::new("schedule.status", json!({})))
        .unwrap_err();
    assert_eq!(not_started.code(), ErrorCode::StageFailed);
    idle.stop();

    let harness = Harness::start(vec![ten_minutes("s1")], TestRunner::printing("", 0));
    assert_eq!(harness.module.health(), ModuleHealth::Running);
    let cases: [(&str, Json, ErrorCode); 5] = [
        ("schedule.unknown", json!({}), ErrorCode::Unsupported),
        (
            "schedule/register_action",
            json!({ "kind": "k" }),
            ErrorCode::Arg,
        ),
        (
            "schedule/ack",
            json!({ "token": "t", "state": "fine" }),
            ErrorCode::Arg,
        ),
        ("schedule.run", json!({ "id": "s9" }), ErrorCode::NotFound),
        ("schedule.run", json!({}), ErrorCode::MissingParam),
    ];
    for (op, body, code) in cases {
        assert_eq!(harness.request(op, body).unwrap_err().code(), code, "{op}");
    }
    assert_eq!(
        harness
            .request("schedule.stop", json!({ "id": "s1" }))
            .unwrap(),
        json!({ "stopped": [] })
    );
    harness.module.reload(&Config::default()).unwrap();
    assert_eq!(
        harness.request("schedule.changed", json!({})).unwrap(),
        json!({ "schedules": 1 })
    );
    assert_eq!(harness.shutdown(), ModuleHealth::Stopped);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
