// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Seeded failpoint registry and its link to `CrashFs`.

use std::sync::Arc;

use nurt_contracts::fs::{FsErrorKind, FsWrite};
use nurt_crashfs::crash::CrashProfile;
use nurt_fs::failpoint::{
    FailAction, FailPlan, FailPoint, FailRule, Failpoints, RandomFaults, RunRecord,
    SeededFailpoints,
};

use crate::support::{at, new_fs, scope};

fn random_plan(seed: u64) -> FailPlan {
    FailPlan {
        seed,
        rules: vec![FailRule {
            point: FailPoint::CommitWritten,
            n: None,
            occurrence: 3,
            action: FailAction::Error(FsErrorKind::Io),
        }],
        random: RandomFaults {
            crash_per_million: 5_000,
            error_per_million: 100_000,
            error_kinds: vec![FsErrorKind::NoSpace, FsErrorKind::InUse],
        },
    }
}

fn drive(registry: &SeededFailpoints) {
    for round in 0..50_u32 {
        for point in FailPoint::ALL {
            registry.hit(point, point.takes_index().then_some(round % 4));
        }
    }
}

#[test]
fn failpoints_replay_gives_the_same_run() {
    let first = SeededFailpoints::new(random_plan(2024));
    drive(&first);
    let replayed = SeededFailpoints::replay(&first.record());
    drive(&replayed);
    assert_eq!(replayed.record(), first.record());
}

#[test]
fn failpoints_record_round_trips_through_json() {
    let registry = SeededFailpoints::new(random_plan(9));
    drive(&registry);
    let record = registry.record();
    let text = serde_json::to_string(&record).unwrap();
    let parsed: RunRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, record);
    let replayed = SeededFailpoints::replay(&parsed);
    drive(&replayed);
    assert_eq!(replayed.record(), record);
}

#[test]
fn failpoints_serialized_names_are_stable() {
    for point in FailPoint::ALL {
        let text = serde_json::to_string(&point).unwrap();
        assert_eq!(text, format!("\"{}\"", point.as_str()));
    }
    let action = serde_json::to_string(&FailAction::Error(FsErrorKind::NoSpace)).unwrap();
    assert_eq!(action, r#"{"error":"no_space"}"#);
}

#[test]
fn failpoints_crash_decision_stops_crashfs() {
    let fs = new_fs(CrashProfile::PosixWorstCase, 4);
    let plan = FailPlan::at(
        4,
        FailRule {
            point: FailPoint::ManifestWritten,
            n: None,
            occurrence: 0,
            action: FailAction::Crash,
        },
    );
    let registry = Arc::new(SeededFailpoints::new(plan));
    fs.attach_failpoints(Arc::clone(&registry));
    fs.create_dir(&scope(), &at("before"), 0o755).unwrap();
    assert_eq!(
        registry.hit(FailPoint::ManifestWritten, None),
        FailAction::Crash
    );
    assert!(fs.is_halted());
    assert_eq!(
        fs.create_dir(&scope(), &at("after"), 0o755)
            .unwrap_err()
            .kind,
        FsErrorKind::Io
    );
}

#[test]
fn failpoints_crashfs_crash_marks_the_registry() {
    let fs = new_fs(CrashProfile::PosixWorstCase, 6);
    let registry = Arc::new(SeededFailpoints::new(FailPlan::observe(6)));
    fs.attach_failpoints(Arc::clone(&registry));
    fs.arm_crash_at_call(1);
    assert!(fs.create_dir(&scope(), &at("d"), 0o755).is_err());
    assert!(registry.is_crashed());
    assert_eq!(registry.hit(FailPoint::ApplyDone, None), FailAction::Crash);
    fs.detach_failpoints();
    fs.crash();
    fs.create_dir(&scope(), &at("d"), 0o755).unwrap();
}
