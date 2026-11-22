// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Saving the prepared state for a job and finishing it elsewhere.

use std::fs;
use std::path::Path;
use std::process::Command;

use nurt_contracts::config::Config;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::TargetIndex;
use nurt_contracts::record::{Change, Tracking};
use nurt_contracts::report::ChangeAction;

use super::support::{Harness, RecordingTx, context};
use crate::iface::TrackMode;
use crate::track::{restore_prepared, save_prepared};

/// Variable naming the saved state for the child process.
const STATE_VARIABLE: &str = "NURT_TRACK_TEST_STATE";

/// Variable naming the result file of the child process.
const RESULT_VARIABLE: &str = "NURT_TRACK_TEST_RESULT";

/// Full name of the test that runs again as the child process.
const CHILD_TEST: &str = "track::tests::serialize::finishes_saved_state_in_another_process";

#[test]
fn saved_state_round_trips_and_finishes_by_scan() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    let mut tx = RecordingTx::new();
    let prepared = h.prepare(&h.spec(TrackMode::Events), &mut tx);
    let saved = save_prepared(prepared.as_ref()).unwrap();
    drop(prepared);
    h.write("a.txt", "alpha changed");
    let restored = restore_prepared(saved).unwrap();
    let outcome = restored.finish(&h.ctx, &mut tx).unwrap();
    assert_eq!(outcome.tracking, Tracking::Scan);
    assert_eq!(
        h.actions(&outcome),
        vec![("a.txt".to_owned(), ChangeAction::Modify)]
    );
    assert_eq!(tx.declared_indexes(), vec![Some(TargetIndex(0))]);
    tx.undo();
    assert_eq!(h.read("a.txt"), "alpha");
}

#[test]
fn restore_rejects_foreign_state() {
    let error = restore_prepared(serde_json::json!({ "unrelated": true }))
        .err()
        .unwrap();
    assert_eq!(error.code(), ErrorCode::Internal);
}

#[test]
fn declared_indexes_require_the_same_operation() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    let mut first = RecordingTx::new();
    let prepared = h.prepare(&h.spec(TrackMode::Scan), &mut first);
    h.write("a.txt", "alpha changed");
    let mut other = RecordingTx::other_operation();
    prepared.finish(&h.ctx, &mut other).unwrap();
    assert_eq!(other.declared_indexes(), vec![None]);
}

fn finish_as_child(state: &Path, result: &Path) {
    let (ctx, _) = context(Config::default());
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(state).unwrap()).unwrap();
    let mut tx = RecordingTx::new();
    let outcome = restore_prepared(saved).unwrap().finish(&ctx, &mut tx).unwrap();
    fs::write(result, serde_json::to_vec(&outcome.changes).unwrap()).unwrap();
}

#[test]
fn finishes_saved_state_in_another_process() {
    if let (Some(state), Some(result)) = (
        std::env::var_os(STATE_VARIABLE),
        std::env::var_os(RESULT_VARIABLE),
    ) {
        finish_as_child(Path::new(&state), Path::new(&result));
        return;
    }
    let h = Harness::new();
    h.write("a.txt", "alpha");
    h.write("b.txt", "beta");
    let mut tx = RecordingTx::new();
    let prepared = h.prepare(&h.spec(TrackMode::Scan), &mut tx);
    let state = h.root.with_file_name("state.json");
    let result = h.root.with_file_name("result.json");
    fs::write(&state, serde_json::to_vec(&save_prepared(prepared.as_ref()).unwrap()).unwrap())
        .unwrap();
    drop(prepared);
    h.write("a.txt", "alpha changed");
    fs::remove_file(h.path("b.txt")).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD_TEST, "--test-threads=1"])
        .env(STATE_VARIABLE, &state)
        .env(RESULT_VARIABLE, &result)
        .status()
        .unwrap();
    assert!(status.success());
    let changes: Vec<Change> = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
    let actions: Vec<(String, ChangeAction)> = changes
        .iter()
        .map(|change| (h.relative(change.path.as_str()), change.action))
        .collect();
    assert_eq!(
        actions,
        vec![
            ("a.txt".to_owned(), ChangeAction::Modify),
            ("b.txt".to_owned(), ChangeAction::Delete),
        ]
    );
}
