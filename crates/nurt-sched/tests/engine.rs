// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sched/tests/engine.rs
// Responsibility:   Run decisions: missed and late runs, overlapping runs, run limits, expiry, pauses, restarts of the service and changed definitions.
// Dependencies:     nurt_contracts, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Run decisions: missed and late runs, overlapping runs, run limits, expiry,
//! pauses, restarts of the service and changed definitions.
//!
//! Every test drives the scheduler with explicit instants, so no test waits
//! for real time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

mod support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Instant;
use nurt_sched::action::{Outcome, pipeline_hash};
use nurt_sched::engine::{EngineEvent, Launch, Scheduler};
use nurt_sched::settings::ScheduleSettings;
use nurt_sched::store::{
    Definition, OnMissed, Overlap, RunState, RunTrigger, SkipReason, StateFile, Status, Store,
};
use nurt_sched::trigger::TriggerDef;
use support::{at, definition, every, exec_record, minutes, pipeline};

/// Creation time of the test schedules; an interval of ten minutes makes the
/// first run time 14:40.
const T0: &str = "2026-11-22T14:30:00Z";

fn time(clock: &str) -> Instant {
    at(&format!("2026-11-22T{clock}Z"))
}

fn ten_minutes(id: &str) -> Definition {
    definition(id, every(minutes(10)), at(T0))
}

fn scheduler(definitions: Vec<Definition>, now: &str) -> Scheduler {
    scheduler_with(
        ScheduleSettings::default(),
        StateFile::default(),
        definitions,
        now,
    )
}

fn scheduler_with(
    settings: ScheduleSettings,
    state: StateFile,
    definitions: Vec<Definition>,
    now: &str,
) -> Scheduler {
    let mut scheduler = Scheduler::new(settings, state);
    assert!(scheduler.load(definitions, time(now)).is_empty());
    scheduler
}

fn ok() -> Outcome {
    Outcome::new(RunState::Ok, "the action ended")
}

fn tick_one(scheduler: &mut Scheduler, now: &str) -> Option<Launch> {
    let mut launches = scheduler.tick(time(now));
    assert!(launches.len() <= 1, "{launches:?}");
    launches.pop()
}

fn run_at(scheduler: &mut Scheduler, now: &str) -> Launch {
    tick_one(scheduler, now).unwrap_or_else(|| panic!("no run at {now}"))
}

fn next_run(scheduler: &Scheduler, id: &str) -> Option<Instant> {
    scheduler.entry(id).unwrap().next_run
}

fn outcomes(scheduler: &Scheduler, id: &str) -> Vec<(RunTrigger, RunState, Option<SkipReason>)> {
    scheduler
        .history(id)
        .iter()
        .map(|record| (record.trigger, record.state, record.reason))
        .collect()
}

#[test]
fn tc_09_at_runs_once_then_completes() {
    let trigger = TriggerDef::at(time("14:40:00"), "UTC", "UTC", at(T0)).unwrap();
    let mut scheduler = scheduler(vec![definition("s1", trigger, at(T0))], "14:30:00");
    let launch = run_at(&mut scheduler, "14:40:00");
    assert_eq!(launch.scheduled_for, Some(time("14:40:00")));
    assert_eq!(scheduler.entry("s1").unwrap().status, Status::Completed);
    scheduler.finish(&launch.id(), &ok(), time("14:40:02"));
    assert!(scheduler.tick(time("14:50:00")).is_empty());
    assert_eq!(scheduler.entry("s1").unwrap().status, Status::Completed);
}

#[test]
fn tc_11_max_runs_completes_and_manual_runs_do_not_count() {
    let mut limited = ten_minutes("s1");
    limited.max_runs = Some(3);
    let mut scheduler = scheduler(vec![limited], "14:30:00");
    let first = run_at(&mut scheduler, "14:40:00");
    scheduler.finish(&first.id(), &ok(), time("14:40:05"));
    let manual = scheduler
        .run_now("s1", RunTrigger::Manual, time("14:45:00"))
        .unwrap();
    assert_eq!(manual.trigger, RunTrigger::Manual);
    scheduler.finish(&manual.id(), &ok(), time("14:45:05"));
    for clock in ["14:50:00", "15:00:00"] {
        let launch = run_at(&mut scheduler, clock);
        scheduler.finish(&launch.id(), &ok(), time(clock));
    }
    let entry = scheduler.entry("s1").unwrap();
    assert_eq!(entry.status, Status::Completed);
    assert_eq!(entry.run_count, 3);
    assert_eq!(entry.runs, 4);
    assert_eq!(entry.next_run, None);
    assert!(scheduler.tick(time("15:10:00")).is_empty());
}

#[test]
fn max_runs_completes() {
    let mut once = ten_minutes("s1");
    once.max_runs = Some(1);
    let mut scheduler = scheduler(vec![once], "14:30:00");
    let launch = run_at(&mut scheduler, "14:40:00");
    assert_eq!(scheduler.entry("s1").unwrap().status, Status::Completed);
    assert_eq!(next_run(&scheduler, "s1"), None);
    assert_eq!(scheduler.next_wake(), None);
    scheduler.finish(&launch.id(), &ok(), time("14:40:01"));
    assert!(scheduler.tick(time("14:50:00")).is_empty());
}

#[test]
fn tc_12_expires_after_two_runs() {
    let mut expiring = ten_minutes("s1");
    expiring.expires = Some(time("14:55:00"));
    let mut scheduler = scheduler(vec![expiring], "14:30:00");
    for clock in ["14:40:00", "14:50:00"] {
        let launch = run_at(&mut scheduler, clock);
        scheduler.finish(&launch.id(), &ok(), time(clock));
    }
    assert!(scheduler.tick(time("15:00:00")).is_empty());
    let entry = scheduler.entry("s1").unwrap();
    assert_eq!(entry.status, Status::Expired);
    assert_eq!(entry.run_count, 2);
    assert_eq!(entry.next_run, None);
}

#[test]
fn tc_13_run_now_keeps_next_run_and_run_limit() {
    let mut limited = ten_minutes("s1");
    limited.max_runs = Some(1);
    let mut scheduler = scheduler(vec![limited], "14:30:00");
    let launch = scheduler
        .run_now("s1", RunTrigger::RunNow, time("14:30:00"))
        .unwrap();
    assert_eq!(launch.trigger, RunTrigger::RunNow);
    assert_eq!(launch.scheduled_for, None);
    scheduler.finish(&launch.id(), &ok(), time("14:30:01"));
    assert_eq!(next_run(&scheduler, "s1"), Some(time("14:40:00")));
    assert_eq!(scheduler.entry("s1").unwrap().run_count, 0);
    assert_eq!(scheduler.entry("s1").unwrap().status, Status::Active);
    assert_eq!(
        outcomes(&scheduler, "s1"),
        vec![(RunTrigger::RunNow, RunState::Ok, None)]
    );
}

#[test]
fn tc_14_manual_run_is_busy_while_a_run_executes() {
    let mut scheduler = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let scheduled = run_at(&mut scheduler, "14:40:00");
    let error = scheduler
        .run_now("s1", RunTrigger::Manual, time("14:41:00"))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::JobState);
    assert_eq!(error.actual, Some(serde_json::json!("s1#1")));
    assert_eq!(scheduler.running(), vec!["s1#1".to_owned()]);
    scheduler.finish(&scheduled.id(), &ok(), time("14:42:00"));
    let manual = scheduler
        .run_now("s1", RunTrigger::Manual, time("14:43:00"))
        .unwrap();
    scheduler.finish(&manual.id(), &ok(), time("14:43:01"));
    assert_eq!(
        outcomes(&scheduler, "s1"),
        vec![
            (RunTrigger::Schedule, RunState::Ok, None),
            (RunTrigger::Manual, RunState::Ok, None)
        ]
    );
}

#[test]
fn tc_15_overlap_skip_counts_skipped_run_times() {
    let mut scheduler = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let long = run_at(&mut scheduler, "14:40:00");
    assert!(tick_one(&mut scheduler, "14:50:00").is_none());
    assert!(tick_one(&mut scheduler, "15:00:00").is_none());
    assert_eq!(
        scheduler
            .finish(&long.id(), &ok(), time("15:05:00"))
            .map(|l| l.id()),
        None
    );
    assert_eq!(scheduler.entry("s1").unwrap().skipped, 2);
    assert_eq!(next_run(&scheduler, "s1"), Some(time("15:10:00")));
    assert_eq!(
        outcomes(&scheduler, "s1"),
        vec![
            (
                RunTrigger::Schedule,
                RunState::Skipped,
                Some(SkipReason::Overlap)
            ),
            (
                RunTrigger::Schedule,
                RunState::Skipped,
                Some(SkipReason::Overlap)
            ),
            (RunTrigger::Schedule, RunState::Ok, None)
        ]
    );
}

#[test]
fn tc_16_overlap_queue_runs_one_deferred_run() {
    let mut queued = ten_minutes("s1");
    queued.overlap = Overlap::Queue;
    let mut scheduler = scheduler(vec![queued], "14:30:00");
    let long = run_at(&mut scheduler, "14:40:00");
    assert!(tick_one(&mut scheduler, "14:50:00").is_none());
    assert!(scheduler.entry("s1").unwrap().queued);
    assert!(tick_one(&mut scheduler, "15:00:00").is_none());
    let deferred = scheduler
        .finish(&long.id(), &ok(), time("15:05:00"))
        .expect("the queued run starts");
    assert_eq!(deferred.trigger, RunTrigger::Queued);
    assert_eq!(deferred.started, time("15:05:00"));
    assert!(!scheduler.entry("s1").unwrap().queued);
    assert!(
        scheduler
            .finish(&deferred.id(), &ok(), time("15:06:00"))
            .is_none()
    );
    assert_eq!(scheduler.entry("s1").unwrap().skipped, 1);
    assert_eq!(
        outcomes(&scheduler, "s1"),
        vec![
            (
                RunTrigger::Schedule,
                RunState::Skipped,
                Some(SkipReason::Overlap)
            ),
            (RunTrigger::Schedule, RunState::Ok, None),
            (RunTrigger::Queued, RunState::Ok, None)
        ]
    );
}

#[test]
fn a_queued_run_waits_for_a_free_place_among_max_parallel_runs() {
    let settings = ScheduleSettings {
        max_parallel: 1,
        ..ScheduleSettings::default()
    };
    let mut queued = ten_minutes("s1");
    queued.overlap = Overlap::Queue;
    let mut scheduler = scheduler_with(
        settings,
        StateFile::default(),
        vec![queued, ten_minutes("s2")],
        "14:30:00",
    );
    let first = run_at(&mut scheduler, "14:40:00");
    assert_eq!(first.schedule, "s1");
    let manual = scheduler
        .run_now("s2", RunTrigger::Manual, time("14:41:00"))
        .unwrap();
    assert!(tick_one(&mut scheduler, "14:50:00").is_none());
    assert!(scheduler.entry("s1").unwrap().queued);
    assert!(
        scheduler
            .finish(&first.id(), &ok(), time("14:51:00"))
            .is_none()
    );
    assert!(scheduler.entry("s1").unwrap().queued);
    assert_eq!(scheduler.running(), vec![manual.id()]);
    scheduler.finish(&manual.id(), &ok(), time("14:52:00"));
    let deferred = run_at(&mut scheduler, "14:53:00");
    assert_eq!(deferred.schedule, "s1");
    assert_eq!(deferred.trigger, RunTrigger::Queued);
    assert!(!scheduler.entry("s1").unwrap().queued);
}

#[test]
fn overlap_queue_single_slot() {
    let mut queued = ten_minutes("s1");
    queued.overlap = Overlap::Queue;
    let mut scheduler = scheduler(vec![queued], "14:30:00");
    let long = run_at(&mut scheduler, "14:40:00");
    assert!(tick_one(&mut scheduler, "14:50:00").is_none());
    assert_eq!(scheduler.stop("s1"), vec!["s1#1".to_owned()]);
    assert!(!scheduler.entry("s1").unwrap().queued);
    assert!(
        scheduler
            .finish(&long.id(), &ok(), time("14:55:00"))
            .is_none()
    );
    assert!(scheduler.stop("s1").is_empty());
}

#[test]
fn overlap_parallel_cap() {
    let settings = ScheduleSettings {
        max_parallel: 2,
        ..ScheduleSettings::default()
    };
    let mut parallel = ten_minutes("s1");
    parallel.overlap = Overlap::Parallel;
    let mut scheduler = scheduler_with(settings, StateFile::default(), vec![parallel], "14:30:00");
    let first = run_at(&mut scheduler, "14:40:00");
    let second = run_at(&mut scheduler, "14:50:00");
    assert_ne!(first.id(), second.id());
    assert!(tick_one(&mut scheduler, "15:00:00").is_none());
    let entry = scheduler.entry("s1").unwrap();
    assert_eq!(entry.last_refused, Some(time("15:00:00")));
    assert_eq!(entry.next_run, Some(time("15:00:00")));
    scheduler.finish(&first.id(), &ok(), time("15:01:00"));
    let third = run_at(&mut scheduler, "15:01:00");
    assert_eq!(third.scheduled_for, Some(time("15:00:00")));
    assert_eq!(third.trigger, RunTrigger::Schedule);
    assert_eq!(scheduler.running().len(), 2);
}

#[test]
fn capacity_wait_beyond_misfire_is_skipped() {
    let settings = ScheduleSettings {
        max_parallel: 1,
        ..ScheduleSettings::default()
    };
    let mut scheduler = scheduler_with(
        settings,
        StateFile::default(),
        vec![ten_minutes("s1"), ten_minutes("s2")],
        "14:30:00",
    );
    let launches = scheduler.tick(time("14:40:00"));
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].schedule, "s1");
    assert_eq!(
        scheduler.entry("s2").unwrap().last_refused,
        Some(time("14:40:00"))
    );
    assert!(scheduler.tick(time("14:42:30")).is_empty());
    assert_eq!(
        outcomes(&scheduler, "s2"),
        vec![(
            RunTrigger::Schedule,
            RunState::Skipped,
            Some(SkipReason::Capacity)
        )]
    );
    assert_eq!(next_run(&scheduler, "s2"), Some(time("14:50:00")));
}

#[test]
fn tc_17_missed_run_times_are_skipped_after_the_service_stops() {
    let before_stop = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let saved = before_stop.state().clone();
    let mut restarted = scheduler_with(
        ScheduleSettings::default(),
        saved.clone(),
        vec![ten_minutes("s1")],
        "15:10:00",
    );
    assert!(restarted.tick(time("15:10:00")).is_empty());
    assert_eq!(restarted.entry("s1").unwrap().skipped, 1);
    assert_eq!(next_run(&restarted, "s1"), Some(time("15:20:00")));
    assert_eq!(
        outcomes(&restarted, "s1"),
        vec![(
            RunTrigger::Schedule,
            RunState::Skipped,
            Some(SkipReason::Misfire)
        )]
    );

    let mut run_once = ten_minutes("s1");
    run_once.on_missed = OnMissed::RunOnce;
    let mut restarted = scheduler_with(
        ScheduleSettings::default(),
        saved,
        vec![run_once],
        "15:10:00",
    );
    let missed = run_at(&mut restarted, "15:10:00");
    assert_eq!(missed.trigger, RunTrigger::Missed);
    assert_eq!(missed.scheduled_for, Some(time("14:40:00")));
    assert_eq!(next_run(&restarted, "s1"), Some(time("15:20:00")));
    restarted.finish(&missed.id(), &ok(), time("15:10:05"));
    assert!(restarted.tick(time("15:10:10")).is_empty());
}

#[test]
fn restart_service_recovers_due() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path());
    let before_stop = scheduler(vec![ten_minutes("s1")], "14:30:00");
    store.write_state(before_stop.state()).unwrap();
    let saved = store.read_state().unwrap();
    assert_eq!(&saved, before_stop.state());
    let mut restarted = scheduler_with(
        ScheduleSettings::default(),
        saved,
        vec![ten_minutes("s1")],
        "14:41:00",
    );
    let launch = run_at(&mut restarted, "14:41:00");
    assert_eq!(launch.trigger, RunTrigger::Schedule);
    assert_eq!(launch.scheduled_for, Some(time("14:40:00")));
    assert_eq!(next_run(&restarted, "s1"), Some(time("14:50:00")));
}

#[test]
fn on_missed_run_once_after_sleep() {
    let mut run_once = ten_minutes("s1");
    run_once.on_missed = OnMissed::RunOnce;
    let mut scheduler = scheduler(vec![run_once], "14:30:00");
    let first = run_at(&mut scheduler, "14:40:00");
    scheduler.finish(&first.id(), &ok(), time("14:40:05"));
    let after_sleep = run_at(&mut scheduler, "16:00:00");
    assert_eq!(after_sleep.trigger, RunTrigger::Missed);
    assert_eq!(after_sleep.scheduled_for, Some(time("14:50:00")));
    scheduler.finish(&after_sleep.id(), &ok(), time("16:00:05"));
    assert!(scheduler.tick(time("16:00:06")).is_empty());
    assert_eq!(next_run(&scheduler, "s1"), Some(time("16:10:00")));
    assert_eq!(scheduler.entry("s1").unwrap().skipped, 0);
}

#[test]
fn tc_18_late_run_within_misfire_runs_and_beyond_is_skipped() {
    let mut within = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let launch = run_at(&mut within, "14:41:30");
    assert_eq!(launch.scheduled_for, Some(time("14:40:00")));
    assert_eq!(launch.trigger, RunTrigger::Schedule);

    let mut beyond = scheduler(vec![ten_minutes("s1")], "14:30:00");
    assert!(beyond.tick(time("14:42:30")).is_empty());
    assert_eq!(
        outcomes(&beyond, "s1"),
        vec![(
            RunTrigger::Schedule,
            RunState::Skipped,
            Some(SkipReason::Misfire)
        )]
    );
    assert_eq!(
        beyond.history("s1")[0].scheduled_for,
        Some(time("14:40:00"))
    );
}

#[test]
fn misfire_skips_late() {
    let mut strict = ten_minutes("s1");
    strict.misfire = nurt_contracts::value::Duration::from_millis(30_000);
    let mut scheduler = scheduler(vec![strict], "14:30:00");
    assert!(scheduler.tick(time("14:40:45")).is_empty());
    assert_eq!(scheduler.entry("s1").unwrap().skipped, 1);
    assert_eq!(next_run(&scheduler, "s1"), Some(time("14:50:00")));
    let events = scheduler.take_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Skipped(record) if record.reason == Some(SkipReason::Misfire)))
    );
}

#[test]
fn tc_19_single_time_missed_during_a_pause() {
    let single = |on_missed| {
        let trigger = TriggerDef::at(time("14:40:00"), "UTC", "UTC", at(T0)).unwrap();
        let mut definition = definition("s1", trigger, at(T0));
        definition.on_missed = on_missed;
        definition
    };
    let mut skipped = scheduler(vec![single(OnMissed::Skip)], "14:30:00");
    assert!(skipped.tick(time("14:50:00")).is_empty());
    assert_eq!(skipped.entry("s1").unwrap().status, Status::Expired);

    let mut made_up = scheduler(vec![single(OnMissed::RunOnce)], "14:30:00");
    let launch = run_at(&mut made_up, "14:50:00");
    assert_eq!(launch.trigger, RunTrigger::Missed);
    made_up.finish(&launch.id(), &ok(), time("14:50:05"));
    assert_eq!(made_up.entry("s1").unwrap().status, Status::Completed);
}

#[test]
fn tc_21_resume_counts_next_run_from_the_resume() {
    let active = ten_minutes("s1");
    let mut scheduler = scheduler(vec![active.clone()], "14:30:00");
    let mut paused = active.clone();
    paused.paused = true;
    paused.changed = time("14:31:00");
    scheduler.load(vec![paused.clone()], time("14:31:00"));
    for clock in ["14:40:00", "14:50:00", "15:00:00"] {
        assert!(scheduler.tick(time(clock)).is_empty());
    }
    assert_eq!(scheduler.next_wake(), None);
    let mut resumed = paused;
    resumed.paused = false;
    resumed.changed = time("15:07:00");
    scheduler.load(vec![resumed], time("15:07:00"));
    assert_eq!(next_run(&scheduler, "s1"), Some(time("15:17:00")));
    assert!(!scheduler.entry("s1").unwrap().queued);
    assert!(scheduler.history("s1").is_empty());
}

#[test]
fn tc_28_changed_compares_outputs_of_scheduled_runs() {
    let mut scheduler = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let outputs = [
        ("14:40:00", "same\n"),
        ("14:50:00", "same\n"),
        ("15:00:00", "other\n"),
    ];
    for (clock, stdout) in outputs {
        let launch = run_at(&mut scheduler, clock);
        let outcome = Outcome::from_output(&[exec_record(stdout, 0)]);
        scheduler.finish(&launch.id(), &outcome, time(clock));
    }
    let manual = scheduler
        .run_now("s1", RunTrigger::Manual, time("15:01:00"))
        .unwrap();
    scheduler.finish(
        &manual.id(),
        &Outcome::from_output(&[exec_record("x", 0)]),
        time("15:01:00"),
    );
    let history = scheduler.history("s1");
    let changed: Vec<Option<bool>> = history.iter().map(|record| record.changed).collect();
    assert_eq!(changed, vec![Some(true), Some(false), Some(true), None]);
    assert_eq!(history[0].output_sha256, history[1].output_sha256);
    assert_ne!(history[1].output_sha256, history[2].output_sha256);
    assert_eq!(history[0].exit_code, Some(0));
}

#[test]
fn tc_30_definition_edited_outside_the_commands_is_rejected() {
    let mut edited = ten_minutes("s1");
    edited.pipeline = Some(pipeline("false"));
    let mut scheduler = scheduler(vec![edited], "14:30:00");
    assert!(scheduler.tick(time("14:40:00")).is_empty());
    assert_eq!(
        outcomes(&scheduler, "s1"),
        vec![(
            RunTrigger::Schedule,
            RunState::Rejected,
            Some(SkipReason::DefinitionChanged)
        )]
    );
    assert_eq!(
        scheduler.entry("s1").unwrap().last_state,
        Some(RunState::Rejected)
    );
    assert_eq!(next_run(&scheduler, "s1"), Some(time("14:50:00")));
    let manual = scheduler
        .run_now("s1", RunTrigger::Manual, time("14:41:00"))
        .unwrap_err();
    assert_eq!(manual.code(), ErrorCode::StageFailed);
    assert!(scheduler.running().is_empty());
}

#[test]
fn definition_changed_rejected() {
    let original = ten_minutes("s1");
    let mut scheduler = scheduler(vec![original.clone()], "14:30:00");
    let mut replaced = original;
    let action = pipeline("rm");
    replaced.ast_hash = pipeline_hash(&action).unwrap();
    replaced.pipeline = Some(action);
    scheduler.load(vec![replaced], time("14:35:00"));
    assert!(scheduler.tick(time("14:40:00")).is_empty());
    let record = &scheduler.history("s1")[0];
    assert_eq!(record.state, RunState::Rejected);
    assert_eq!(record.reason, Some(SkipReason::DefinitionChanged));
    let events = scheduler.take_events();
    assert!(events.iter().any(
        |event| matches!(event, EngineEvent::Finished(run) if run.state == RunState::Rejected)
    ));
}

#[test]
fn restored_definition_counts_next_run_from_now_and_keeps_numbering() {
    let mut scheduler = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let launch = run_at(&mut scheduler, "14:40:00");
    scheduler.finish(&launch.id(), &ok(), time("14:40:05"));
    scheduler.load(Vec::new(), time("14:45:00"));
    assert!(scheduler.entry("s1").is_none());
    assert!(scheduler.is_empty());
    assert_eq!(scheduler.history("s1").len(), 1);
    scheduler.load(vec![ten_minutes("s1")], time("15:30:00"));
    assert_eq!(next_run(&scheduler, "s1"), Some(time("15:40:00")));
    let again = run_at(&mut scheduler, "15:40:00");
    assert_eq!(again.run, 2);
    assert_eq!(scheduler.len(), 1);
}

#[test]
fn history_keeps_the_configured_number_of_runs() {
    let settings = ScheduleSettings {
        history_keep: 5,
        ..ScheduleSettings::default()
    };
    let mut scheduler = scheduler_with(
        settings,
        StateFile::default(),
        vec![ten_minutes("s1")],
        "14:30:00",
    );
    for _ in 0..7 {
        let launch = scheduler
            .run_now("s1", RunTrigger::Manual, time("14:31:00"))
            .unwrap();
        scheduler.finish(&launch.id(), &ok(), time("14:31:01"));
    }
    let kept: Vec<u64> = scheduler
        .history("s1")
        .iter()
        .map(|record| record.run)
        .collect();
    assert_eq!(kept, vec![3, 4, 5, 6, 7]);
}

#[test]
fn events_and_dirty_flag_follow_the_runs() {
    let mut scheduler = scheduler(vec![ten_minutes("s1")], "14:30:00");
    assert!(scheduler.take_dirty());
    assert!(!scheduler.take_dirty());
    let launch = run_at(&mut scheduler, "14:40:00");
    scheduler.finish(&launch.id(), &ok(), time("14:40:01"));
    let events = scheduler.take_events();
    assert!(matches!(events.first(), Some(EngineEvent::Fired(fired)) if fired.id() == "s1#1"));
    assert!(matches!(events.last(), Some(EngineEvent::Finished(run)) if run.id() == "s1#1"));
    assert!(scheduler.take_events().is_empty());
    assert!(scheduler.take_dirty());
    assert_eq!(scheduler.definition("s1").unwrap().id, "s1");
    assert!(scheduler.definition("s2").is_none());
}

#[test]
fn next_wake_is_the_earliest_active_run_time() {
    let later = definition("s2", every(minutes(20)), at(T0));
    let scheduler = scheduler(vec![ten_minutes("s1"), later], "14:30:00");
    assert_eq!(scheduler.next_wake(), Some(time("14:40:00")));
}

#[test]
fn unknown_schedule_and_invalid_trigger_are_reported() {
    let mut scheduler = scheduler(vec![ten_minutes("s1")], "14:30:00");
    let error = scheduler
        .run_now("s9", RunTrigger::Manual, time("14:31:00"))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(scheduler.finish("s9#1", &ok(), time("14:31:00")).is_none());

    let mut broken = ten_minutes("s2");
    broken.trigger.expression = "not an interval".to_owned();
    let invalid = scheduler.load(vec![ten_minutes("s1"), broken], time("14:31:00"));
    assert_eq!(invalid, vec!["s2".to_owned()]);
    assert!(
        scheduler
            .tick(time("14:40:00"))
            .iter()
            .all(|launch| launch.schedule == "s1")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
