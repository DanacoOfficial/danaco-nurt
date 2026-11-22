// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Crash corpus driven on the reference protocol.
//!
//! The totals of each run are written as JSON to the directory named by the
//! `NURT_CRASH_REPORT_DIR` environment variable when it is set.

use std::path::PathBuf;
use std::time::Instant;

use nurt_crashfs::crash::CrashProfile;
use nurt_crashfs::points::{CorpusReport, CrashPoint, Harness, Trigger};
use nurt_fs::failpoint::FailPoint;

use crate::reference::ReferenceReplace;

/// Base seed of the corpus runs.
const CORPUS_SEED: u64 = 0x6E75_7274;
/// Points of the random corpus in an ordinary test run.
const ORDINARY_POINTS: u64 = 2_000;
/// Points of the long random corpus per profile.
const LONG_POINTS: u64 = 100_000;

const CORRECT: ReferenceReplace = ReferenceReplace { flush_keep: true };
const BROKEN: ReferenceReplace = ReferenceReplace { flush_keep: false };

fn save(report: &CorpusReport) {
    if let Some(dir) = std::env::var_os("NURT_CRASH_REPORT_DIR") {
        let name = format!(
            "{}-{}-{}.json",
            report.scenario,
            report.profile.as_str(),
            report.os
        );
        report.write_json(&PathBuf::from(dir).join(name)).unwrap();
    }
}

fn assert_clean(report: &CorpusReport) {
    save(report);
    assert!(
        report.failures.is_empty(),
        "{} of {} points failed, first: {:?}",
        report.failures.len(),
        report.points,
        report.failures.first()
    );
}

#[test]
fn reference_corpus_every_trigger_on_every_profile() {
    for profile in CrashProfile::ALL {
        let harness = Harness::new(profile);
        let enumeration = harness.enumerate(&CORRECT, CORPUS_SEED).unwrap();
        assert!(enumeration.fs_calls > 0);
        assert!(enumeration.protocol.len() >= 10);
        let report = harness.run_all(&CORRECT, &enumeration, CORPUS_SEED);
        assert_clean(&report);
        assert!(report.crashes >= report.points - 1);
    }
}

#[test]
fn reference_corpus_random_points() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let enumeration = harness.enumerate(&CORRECT, CORPUS_SEED).unwrap();
    let report = harness.run_random(&CORRECT, &enumeration, CORPUS_SEED, ORDINARY_POINTS);
    assert_clean(&report);
    assert!(
        report.crashes > report.points,
        "no crash during recovery was executed"
    );
}

#[test]
#[ignore = "long crash corpus"]
fn reference_corpus_long_random_points() {
    let started = Instant::now();
    for profile in CrashProfile::ALL {
        let harness = Harness::new(profile);
        let enumeration = harness.enumerate(&CORRECT, CORPUS_SEED).unwrap();
        let report = harness.run_random(&CORRECT, &enumeration, CORPUS_SEED, LONG_POINTS);
        assert_clean(&report);
    }
    assert!(started.elapsed().as_secs() > 0);
}

#[test]
fn reference_corpus_detects_a_missing_directory_flush() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let enumeration = harness.enumerate(&BROKEN, CORPUS_SEED).unwrap();
    let report = harness.run_random(&BROKEN, &enumeration, CORPUS_SEED, ORDINARY_POINTS / 4);
    let failure = report
        .failures
        .first()
        .expect("the oracle accepted a protocol that loses previous content");
    let replayed = harness.replay(&BROKEN, failure).unwrap_err();
    assert_eq!(&replayed, failure);
}

#[test]
fn reference_corpus_crashes_during_recovery_and_again() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let run = Trigger::Protocol {
        point: FailPoint::ApplyDone,
        n: None,
        occurrence: 0,
    };
    let recovery_points = [
        Trigger::Protocol {
            point: FailPoint::RecoveryStarted,
            n: None,
            occurrence: 0,
        },
        Trigger::Protocol {
            point: FailPoint::RecoveryEntry,
            n: None,
            occurrence: 0,
        },
        Trigger::FsCall { call: 1 },
        Trigger::FsCall { call: 2 },
        Trigger::FsCall { call: 3 },
    ];
    for (seed, recovery) in (0..).zip(recovery_points) {
        let point = CrashPoint {
            run,
            recovery: Some(recovery),
        };
        let report = harness.run_point(&CORRECT, seed, point).unwrap();
        assert_eq!(report.crashes, 2, "{recovery:?}");
    }
}
