// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/journal_faults.rs
// Responsibility:   Fault injection into the journal transaction protocol.
// Dependencies:     nurt_contracts, nurt_crashfs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fault injection into the journal transaction protocol.
//!
//! Each file system call made by an operation receives, one call at a time,
//! every injected error kind. The operation must roll back with an error or
//! commit, with an irreversible marker when previous content was not kept,
//! and the tree must match the reported result. Lack of space inside the
//! journal never fails the operation.

use std::sync::Arc;

use nurt_contracts::fs::{FsErrorKind, FsWrite};
use nurt_contracts::report::Irreversible;
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
use nurt_crashfs::inject::{Call, Fault, INJECTED_KINDS, InjectFs, InjectPlan, InjectRule};
use nurt_crashfs::oracle::OpOutcome;
use nurt_crashfs::points::Phase;

use crate::journal::{
    Case, METADATA_ROOM, NEW, Performed, check, initial_bytes, journal, journal_dir, perform,
    setup, state_dir,
};

const FAULT_SEED: u64 = 0x6661_756C_7473;
/// Error kinds that stand for lack of space.
const SPACE_KINDS: [FsErrorKind; 2] = [FsErrorKind::NoSpace, FsErrorKind::QuotaExceeded];
/// Bytes accepted by one write in the short transfer test.
const SHORT_WRITE: usize = 3;
/// Bytes returned by one read in the short transfer test.
const SHORT_READ: usize = 2;
/// Failures listed in the message of a failed test.
const MAX_REPORTED: usize = 20;
/// Irreversible reasons acceptable when the journal has no space.
const NO_SPACE_REASONS: [Irreversible; 2] =
    [Irreversible::NoSpaceForCopy, Irreversible::JournalError];
/// Cases receiving every error kind at every call: every preservation mode
/// of file content and the bulk operation.
const FAULT_CASES: [Case; 7] = [
    Case::OverwriteByExchange,
    Case::OverwriteByHardLink,
    Case::OverwriteByCopy,
    Case::Append,
    Case::RemoveTree,
    Case::RenameOverDestination,
    Case::Bulk,
];

struct FaultRun {
    fs: Arc<InjectFs<CrashFs>>,
    performed: Performed,
}

fn prepared(spec: VolumeSpec) -> CrashFs {
    let fs = CrashFs::new(CrashProfile::PosixWorstCase, FAULT_SEED, vec![spec]).unwrap();
    setup(&fs).unwrap();
    fs.sync_all();
    fs
}

fn run_on(case: Case, spec: VolumeSpec, plan: InjectPlan) -> FaultRun {
    let model = prepared(spec);
    let fs = Arc::new(InjectFs::new(model.clone(), plan));
    let writer = Arc::clone(&fs) as Arc<dyn FsWrite>;
    let performed = perform(&journal(writer, &model, None, case.strategy()), case);
    FaultRun { fs, performed }
}

fn run(case: Case, plan: InjectPlan) -> FaultRun {
    run_on(case, case.volume(), plan)
}

fn calls_of(case: Case) -> Vec<(Call, u32)> {
    let run = run(case, InjectPlan::default());
    assert_eq!(
        run.performed.outcome,
        OpOutcome::Committed {
            irreversible: false
        },
        "{}: {:?}",
        case.name(),
        run.performed
    );
    Call::ALL
        .into_iter()
        .map(|call| (call, u32::try_from(run.fs.calls(call)).unwrap()))
        .filter(|(_, count)| *count > 0)
        .collect()
}

fn error_plan(call: Call, occurrence: u32, kind: FsErrorKind) -> InjectPlan {
    InjectPlan {
        seed: FAULT_SEED,
        rules: vec![InjectRule::error_at(call, occurrence, kind)],
        ..InjectPlan::default()
    }
}

/// Whether a fault was injected into the journal or account state.
fn hit_journal(run: &FaultRun) -> bool {
    let journal = journal_dir();
    let state = state_dir();
    run.fs.injected().iter().any(|injected| {
        injected.path.starts_with(journal.as_path()) || injected.path.starts_with(state.as_path())
    })
}

/// Failure description of a run that did not end explicitly or left a tree
/// that does not match its result.
fn verdict(case: Case, label: &str, run: &FaultRun) -> Option<String> {
    let outcome = run.performed.outcome;
    if outcome == OpOutcome::Crashed {
        return Some(format!(
            "{label}: stopped with {:?} without a crash",
            run.performed.code
        ));
    }
    check(case, run.fs.inner(), Phase::AfterRecovery, outcome)
        .err()
        .map(|violation| format!("{label}: {violation}"))
}

fn assert_no_failures(failures: &[String], runs: usize) {
    assert!(
        failures.is_empty(),
        "{} of {runs} runs failed:\n{}",
        failures.len(),
        failures
            .iter()
            .take(MAX_REPORTED)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn inject_errors_of_every_kind_at_every_journal_call_end_explicitly() {
    let mut runs = 0;
    let mut failures = Vec::new();
    for case in FAULT_CASES {
        for (call, count) in calls_of(case) {
            for occurrence in 0..count {
                for kind in INJECTED_KINDS {
                    let run = run(case, error_plan(call, occurrence, kind));
                    let label = format!("{} {call:?}#{occurrence} {kind:?}", case.name());
                    failures.extend(verdict(case, &label, &run));
                    runs += 1;
                }
            }
        }
    }
    assert_no_failures(&failures, runs);
    assert!(runs >= INJECTED_KINDS.len() * FAULT_CASES.len());
}

#[test]
fn enospc_at_every_step_never_fails_the_operation_because_of_the_journal() {
    let mut runs = 0;
    let mut failures = Vec::new();
    for case in Case::ALL {
        for (call, count) in calls_of(case) {
            for occurrence in 0..count {
                for kind in SPACE_KINDS {
                    let run = run(case, error_plan(call, occurrence, kind));
                    let label = format!("{} {call:?}#{occurrence} {kind:?}", case.name());
                    failures.extend(verdict(case, &label, &run));
                    let committed = matches!(run.performed.outcome, OpOutcome::Committed { .. });
                    if hit_journal(&run) && !committed {
                        failures.push(format!(
                            "{label}: lack of space in the journal failed the operation with {:?}",
                            run.performed.code
                        ));
                    }
                    runs += 1;
                }
            }
        }
    }
    assert_no_failures(&failures, runs);
}

#[test]
fn full_journal_area_degrades_preservation_and_commits_every_case() {
    for case in Case::ALL {
        let plan = InjectPlan {
            rules: vec![InjectRule {
                call: None,
                below: Some(journal_dir().as_path().to_path_buf()),
                occurrence: None,
                fault: Fault::Error {
                    kind: FsErrorKind::NoSpace,
                },
            }],
            ..InjectPlan::default()
        };
        let run = run(case, plan);
        assert!(
            matches!(run.performed.outcome, OpOutcome::Committed { .. }),
            "{}: {:?}",
            case.name(),
            run.performed
        );
        assert!(
            run.performed
                .reasons
                .iter()
                .all(|reason| NO_SPACE_REASONS.contains(reason)),
            "{}: {:?}",
            case.name(),
            run.performed.reasons
        );
        if let Some(failure) = verdict(case, case.name(), &run) {
            panic!("{failure}");
        }
    }
}

#[test]
fn full_volume_keeps_room_for_the_new_content_and_reports_the_lost_copy() {
    let case = Case::OverwriteByCopy;
    let spec = case.volume();
    let inodes = spec.capacity_inodes;
    let spec = spec.with_capacity(initial_bytes() + NEW.len() as u64 + METADATA_ROOM, inodes);
    let run = run_on(case, spec, InjectPlan::default());
    assert_eq!(
        run.performed.outcome,
        OpOutcome::Committed { irreversible: true },
        "{:?}",
        run.performed
    );
    assert!(
        run.performed
            .reasons
            .iter()
            .all(|reason| NO_SPACE_REASONS.contains(reason)),
        "{:?}",
        run.performed.reasons
    );
    if let Some(failure) = verdict(case, case.name(), &run) {
        panic!("{failure}");
    }
}

#[test]
fn inject_short_writes_and_reads_keep_the_result_of_every_case() {
    for case in Case::ALL {
        let plan = InjectPlan {
            rules: vec![
                InjectRule {
                    call: Some(Call::Write),
                    below: None,
                    occurrence: None,
                    fault: Fault::ShortWrite { max: SHORT_WRITE },
                },
                InjectRule {
                    call: Some(Call::Read),
                    below: None,
                    occurrence: None,
                    fault: Fault::ShortRead { max: SHORT_READ },
                },
            ],
            ..InjectPlan::default()
        };
        let run = run(case, plan);
        assert_eq!(
            run.performed.outcome,
            OpOutcome::Committed {
                irreversible: false
            },
            "{}: {:?}",
            case.name(),
            run.performed
        );
        if let Some(failure) = verdict(case, case.name(), &run) {
            panic!("{failure}");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
