// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks around an effect, end to end through the engine: `assert` before
//! the effect (step 3) and after it (step 5a), and `verify` with the rollback
//! of the effect when the check fails.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::ReportStatus;
use nurt_testkit::journal::JournalCall;
use serde_json::{Value, json};
use support::{
    FILES, Programs, exists, find_stage, program, remove_stage, run, standard_programs, world_with,
};

const BEFORE: &str = "find /data |> assert count: 3 |> remove";
const BEFORE_WRONG: &str = "find /data |> assert count: 2 |> remove";
const AFTER: &str = "find /data |> remove |> assert count: 1";
const AFTER_WRONG: &str = "find /data |> remove |> assert count: 2";
const VERIFIED: &str = "find /data |> remove |> verify check: (find /data |> assert count: 0)";
const VERIFIED_WRONG: &str =
    "find /data |> remove |> verify check: (find /data |> assert count: 3)";

fn assert_stage(phase: &str, count: i64) -> Value {
    json!({
        "t": "stage", "name": "assert", "effect": "read", "phase": phase,
        "args": [{"name": "count", "value": {"t": "range", "lo": count, "hi": count}}]
    })
}

fn verify_stage(expected: i64) -> Value {
    let check = json!({
        "t": "sub",
        "p": {"t": "pipeline", "head": find_stage(), "stages": [assert_stage("check", expected)]}
    });
    json!({
        "t": "stage", "name": "verify", "effect": "read", "phase": "verify",
        "args": [{"name": "check", "value": check}]
    })
}

fn programs() -> Programs {
    standard_programs()
        .with(
            BEFORE,
            program(find_stage(), vec![assert_stage("check", 3), remove_stage()]),
        )
        .with(
            BEFORE_WRONG,
            program(find_stage(), vec![assert_stage("check", 2), remove_stage()]),
        )
        .with(
            AFTER,
            program(find_stage(), vec![remove_stage(), assert_stage("after", 1)]),
        )
        .with(
            AFTER_WRONG,
            program(find_stage(), vec![remove_stage(), assert_stage("after", 2)]),
        )
        .with(
            VERIFIED,
            program(find_stage(), vec![remove_stage(), verify_stage(0)]),
        )
        .with(
            VERIFIED_WRONG,
            program(find_stage(), vec![remove_stage(), verify_stage(3)]),
        )
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn assert_before_the_effect_lets_a_met_condition_pass() {
    let world = world_with(programs());
    let response = run(&world, BEFORE, false);
    assert_eq!(
        response.report.status,
        ReportStatus::Ok,
        "{:?}",
        response.report.error
    );
    for (path, _) in FILES {
        assert!(!exists(&world, path), "{path}");
    }
    assert_eq!(world.journal.committed().len(), 1);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn assert_before_the_effect_stops_the_program_without_a_change() {
    let world = world_with(programs());
    let response = run(&world, BEFORE_WRONG, false);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Failed);
    let error = report.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::Assert);
    assert!(!error.effect_applied);
    for (path, _) in FILES {
        assert!(exists(&world, path), "{path}");
    }
    assert!(world.journal.committed().is_empty());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn assert_after_the_effect_lets_a_met_condition_pass() {
    let world = world_with(programs());
    let response = run(&world, AFTER, false);
    assert_eq!(
        response.report.status,
        ReportStatus::Ok,
        "{:?}",
        response.report.error
    );
    assert!(response.report.error.is_none());
    assert!(!exists(&world, "/data/a.log"));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn failed_assert_after_the_effect_keeps_the_effect() {
    let world = world_with(programs());
    let response = run(&world, AFTER_WRONG, false);
    let error = response.report.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::Assert);
    assert!(error.effect_applied);
    assert_ne!(response.report.status, ReportStatus::RolledBack);
    for (path, _) in FILES {
        assert!(!exists(&world, path), "{path}");
    }
    assert_eq!(world.journal.committed().len(), 1);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn verify_with_a_met_check_keeps_the_effect() {
    let world = world_with(programs());
    let response = run(&world, VERIFIED, false);
    assert_eq!(
        response.report.status,
        ReportStatus::Ok,
        "{:?}",
        response.report.error
    );
    assert!(response.report.error.is_none());
    for (path, _) in FILES {
        assert!(!exists(&world, path), "{path}");
    }
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn verify_with_a_failed_check_rolls_the_effect_back() {
    let world = world_with(programs());
    let response = run(&world, VERIFIED_WRONG, false);
    let report = &response.report;
    assert_eq!(
        report.status,
        ReportStatus::RolledBack,
        "{:?}",
        report.error
    );
    assert_eq!(report.error.as_ref().unwrap().code, ErrorCode::RolledBack);
    for (path, _) in FILES {
        assert!(exists(&world, path), "{path}");
    }
    assert!(
        world.journal.calls().iter().any(|call| matches!(
            call,
            JournalCall::Store {
                method: "rollback_committed"
            }
        )),
        "the journal was not asked to roll the operation back"
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn verify_in_preview_does_not_run_the_check_or_change_files() {
    let world = world_with(programs());
    let response = run(&world, VERIFIED_WRONG, true);
    assert_eq!(
        response.report.status,
        ReportStatus::Planned,
        "{:?}",
        response.report.error
    );
    for (path, _) in FILES {
        assert!(exists(&world, path), "{path}");
    }
}
