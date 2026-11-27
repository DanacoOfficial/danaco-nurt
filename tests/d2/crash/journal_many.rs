// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/journal_many.rs
// Responsibility:   Crash corpus of an operation that overwrites many small files.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs, nurt_journal
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Crash corpus of an operation that overwrites many small files.
//!
//! An operation with many targets keeps the previous content of small files
//! in its manifest and makes the new content durable with one flush before
//! the commit instead of one flush per file. This corpus overwrites more files
//! than the thresholds of both mechanisms in one operation on the crash model
//! and checks the guarantees of the journal for it: a crash at any protocol
//! point or file system call leaves the tree in the state before or after the
//! operation, never a mixture, with the previous content of every file on the
//! volume at every moment; an operation that returned survives a power loss
//! that follows; and a crash during the flush that precedes the commit rolls
//! the operation back.

use std::io::Write;
use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{Fs, FsError, FsErrorKind, FsResult, FsWrite};
use nurt_contracts::journal::{BeginRequest, FileOp, JournalStore, OpKind, apply_ops};
use nurt_crashfs::crash::{CrashFs, CrashProfile};
use nurt_crashfs::oracle::{
    Change, Expected, OpOutcome, TreeSnapshot, Violation, check_complete, check_explicit,
    check_no_leftovers, check_preserved,
};
use nurt_crashfs::points::{CrashPoint, Harness, Phase, Scenario, Trigger};
use nurt_fs::failpoint::{FailPoint, Failpoints, SeededFailpoints};
use nurt_journal::layout::DIR_NAME;
use nurt_journal::store::Journal;

use crate::corpus::assert_clean;
use crate::journal::{Case, Performed, STATE_DIR, absolute, before, journal, setup};
use crate::journal_prior::{program, relative};
use crate::support::{scope, volume_root};

/// Files overwritten by the operation; above the thresholds at which the
/// journal keeps small previous content in the manifest and flushes together.
const FILES: usize = 72;
const DIR: &str = "data/many";
const DIR_MODE: u32 = 0o755;
const SEED: u64 = 0x6D61_6E79_0000;
/// Random points per profile in an ordinary test run.
const RANDOM_POINTS: u64 = 40;

fn path(index: usize) -> String {
    format!("{DIR}/f{index:02}")
}

fn old(index: usize) -> Vec<u8> {
    format!("file {index} before the operation").into_bytes()
}

fn new(index: usize) -> Vec<u8> {
    format!("file {index} after the operation, longer than before").into_bytes()
}

/// Tree before the operation: the shared initial state and the files.
fn tree_before() -> TreeSnapshot {
    let dir = Change::CreateDir {
        path: relative(DIR),
        mode: DIR_MODE,
    };
    (0..FILES).fold(before().with(&dir), |tree, index| {
        tree.with(&Change::Write {
            path: relative(&path(index)),
            content: old(index),
        })
    })
}

/// Tree after the operation.
fn tree_after() -> TreeSnapshot {
    (0..FILES).fold(tree_before(), |tree, index| {
        tree.with(&Change::Write {
            path: relative(&path(index)),
            content: new(index),
        })
    })
}

fn targets() -> Vec<std::path::PathBuf> {
    (0..FILES).map(|index| relative(&path(index))).collect()
}

fn prepare(fs: &dyn FsWrite) -> FsResult<()> {
    setup(fs)?;
    fs.create_dir(&scope(), &absolute(DIR), DIR_MODE)?;
    for index in 0..FILES {
        let mut file = fs.create_new(&scope(), &absolute(&path(index)))?;
        file.write_all(&old(index))
            .map_err(|_| FsError::new(FsErrorKind::Io, "write", None))?;
    }
    Ok(())
}

fn request() -> BeginRequest {
    BeginRequest {
        kind: OpKind::command("replace"),
        session: None,
        program: program(),
        corrections: Vec::new(),
        bulk: true,
        counts_as_step: true,
        preserve_budget: None,
    }
}

/// Overwrites every file in one transaction.
fn perform(journal: &Journal) -> Performed {
    let ops: Vec<FileOp> = (0..FILES)
        .map(|index| FileOp::write(absolute(&path(index)), new(index)))
        .collect();
    let mut tx = journal.begin(request());
    let result = apply_ops(tx.as_mut(), ops).and_then(|_| tx.commit().map(|_| ()));
    match result {
        Ok(()) => Performed {
            outcome: OpOutcome::Committed {
                irreversible: false,
            },
            reasons: Vec::new(),
            code: None,
        },
        Err(error) => Performed {
            outcome: if error.code() == ErrorCode::Internal {
                OpOutcome::Crashed
            } else {
                OpOutcome::RolledBack
            },
            reasons: Vec::new(),
            code: Some(error.code()),
        },
    }
}

fn check(fs: &dyn Fs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
    check_explicit(outcome)?;
    let volume = TreeSnapshot::capture(fs, &volume_root(), &[])?;
    let tree = TreeSnapshot::capture(
        fs,
        &volume_root(),
        &[
            std::path::Path::new(DIR_NAME),
            std::path::Path::new(STATE_DIR),
        ],
    )?;
    check_preserved(&tree_before(), &targets(), &volume)?;
    let expected = match (phase, outcome) {
        (Phase::AfterRecovery, OpOutcome::Committed { .. }) => Expected::After,
        (Phase::AfterRecovery, OpOutcome::RolledBack) => Expected::Before,
        (Phase::AfterRecovery, _) => Expected::Either,
        (Phase::AfterCrash, OpOutcome::Committed { .. }) => {
            return check_complete(
                &tree.without_temporary(),
                &tree_before(),
                &tree_after(),
                Expected::After,
            )
            .map(|_| ());
        }
        _ => return Ok(()),
    };
    check_no_leftovers(&tree)?;
    check_complete(&tree, &tree_before(), &tree_after(), expected).map(|_| ())
}

/// Overwrite of many small files as a crash scenario.
struct ManyOverwrite;

impl Scenario for ManyOverwrite {
    fn name(&self) -> &str {
        "journal_many_overwrite"
    }

    fn volumes(&self) -> Vec<nurt_crashfs::crash::VolumeSpec> {
        vec![Case::OverwriteByExchange.volume()]
    }

    fn setup(&self, fs: &CrashFs) -> FsResult<()> {
        prepare(fs)
    }

    fn run(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> OpOutcome {
        let registry: Arc<dyn Failpoints> = Arc::clone(failpoints) as Arc<dyn Failpoints>;
        let journal = journal(
            Arc::new(fs.clone()),
            fs,
            Some(registry),
            Case::OverwriteByExchange.strategy(),
        );
        perform(&journal).outcome
    }

    fn recover(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> FsResult<()> {
        let registry: Arc<dyn Failpoints> = Arc::clone(failpoints) as Arc<dyn Failpoints>;
        let journal = journal(
            Arc::new(fs.clone()),
            fs,
            Some(registry),
            Case::OverwriteByExchange.strategy(),
        );
        journal
            .recover()
            .map(|_| ())
            .map_err(|error| FsError::new(FsErrorKind::Io, &format!("recover: {error}"), None))
    }

    fn check(&self, fs: &CrashFs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
        check(fs, phase, outcome)
    }
}

fn prepared(profile: CrashProfile) -> CrashFs {
    let fs = CrashFs::new(profile, SEED, vec![Case::OverwriteByExchange.volume()]).unwrap();
    prepare(&fs).unwrap();
    fs.sync_all();
    fs
}

#[test]
fn many_file_overwrite_commits_keeps_previous_content_and_survives_power_loss() {
    for profile in CrashProfile::ALL {
        let fs = prepared(profile);
        let journal = journal(
            Arc::new(fs.clone()),
            &fs,
            None,
            Case::OverwriteByExchange.strategy(),
        );
        let performed = perform(&journal);
        assert_eq!(
            performed.outcome,
            OpOutcome::Committed {
                irreversible: false
            },
            "{profile:?}: {performed:?}"
        );
        check(&fs, Phase::AfterRecovery, performed.outcome)
            .unwrap_or_else(|violation| panic!("{profile:?}: {violation}"));
        fs.crash();
        check(&fs, Phase::AfterCrash, performed.outcome)
            .unwrap_or_else(|violation| panic!("{profile:?} after power loss: {violation}"));
    }
}

#[test]
fn many_file_overwrite_keeps_content_inline_and_flushes_before_the_commit() {
    let fs = prepared(CrashProfile::PosixWorstCase);
    let journal = journal(
        Arc::new(fs.clone()),
        &fs,
        None,
        Case::OverwriteByExchange.strategy(),
    );
    let performed = perform(&journal);
    assert_eq!(
        performed.outcome,
        OpOutcome::Committed {
            irreversible: false
        }
    );
    let entry = nurt_journal::layout::JournalLayout::new(crate::journal::journal_dir())
        .entry(nurt_contracts::journal::OpId(1))
        .unwrap();
    let blobs = fs.read_dir(&entry.blobs().unwrap()).unwrap().count();
    assert_eq!(blobs, 0, "the previous content is kept in the manifest");
}

#[test]
fn many_file_overwrite_crash_at_every_protocol_point_keeps_previous_content() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let enumeration = harness.enumerate(&ManyOverwrite, SEED).unwrap();
    assert!(!enumeration.protocol.is_empty());
    let report = harness.run_random(
        &ManyOverwrite,
        &enumeration,
        SEED,
        enumeration.protocol.len() as u64,
    );
    assert_clean(&report);
}

#[test]
fn many_file_overwrite_crash_before_the_commit_and_during_recovery_ends_in_a_complete_state() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let apply_done = Trigger::Protocol {
        point: FailPoint::ApplyDone,
        n: None,
        occurrence: 0,
    };
    let applied = Trigger::Protocol {
        point: FailPoint::ApplySwitched,
        n: None,
        occurrence: 0,
    };
    for (seed, run) in (SEED..).zip([apply_done, applied]) {
        for recovery in [
            None,
            Some(Trigger::Protocol {
                point: FailPoint::RecoveryEntry,
                n: None,
                occurrence: 0,
            }),
            Some(Trigger::FsCall { call: 3 }),
        ] {
            let report = harness
                .run_point(&ManyOverwrite, seed, CrashPoint { run, recovery })
                .unwrap_or_else(|failure| panic!("{run:?} {recovery:?}: {failure:?}"));
            assert!(report.crashes >= 1);
        }
    }
}

#[test]
fn many_file_overwrite_crash_at_random_points_on_every_profile_keeps_previous_content() {
    for profile in CrashProfile::ALL {
        let harness = Harness::new(profile);
        let enumeration = harness.enumerate(&ManyOverwrite, SEED).unwrap();
        let report = harness.run_random(&ManyOverwrite, &enumeration, SEED, RANDOM_POINTS);
        assert_clean(&report);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
