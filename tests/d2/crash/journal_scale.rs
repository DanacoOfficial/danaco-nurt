// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Recovery time of a large journal on the crash model.
//!
//! The operation under test is interrupted and the crash applied before the
//! many copied entries are added, so that the model keeps no operation history
//! for them and holds one image of the volume less during the preparation.
//!
//! The journal holds one committed entry made by a real operation and many
//! entry directories that carry its state file, numbered above the sequence
//! so that new operations never meet them. An operation is then interrupted
//! by a crash after it applied its change. Recovery in a new journal
//! instance must roll the operation back, leave every other entry untouched
//! and end within the recovery budget, because its cost follows the
//! interrupted entries and not the size of the journal.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nurt_contracts::fs::FsWrite;
use nurt_contracts::journal::{FileOp, JournalStore, OpId, RecoveryReport};
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
use nurt_crashfs::oracle::{Expected, check_complete};
use nurt_fs::failpoint::{FailAction, FailPlan, FailPoint, FailRule, SeededFailpoints};
use nurt_journal::layout::JournalLayout;

use crate::journal::{NEW, absolute, before, journal_dir, setup};
use crate::journal_prior::{commit, instrumented, user_tree, view, write};
use crate::support::{content, scope, volume_root};

/// Entries of the ordinary test run.
const REDUCED_ENTRIES: u64 = 10_000;
/// Entries of the full-scale run.
const FULL_ENTRIES: u64 = 1_000_000;
/// Longest recovery accepted with the full number of entries in a release
/// build.
const RECOVERY_BUDGET: Duration = Duration::from_secs(5);
/// Number of the first copied entry, far above the sequence.
const FIRST_COPY: u64 = 100_000_000_000;
/// Entries of the volume per copied entry: its directory and state file.
const INODES_PER_ENTRY: u64 = 2;
const ENTRY_MODE: u32 = 0o700;
const SCALE_SEED: u64 = 0x7363_616C_6500;

/// Result of one recovery of a large journal.
struct Measured {
    report: RecoveryReport,
    elapsed: Duration,
    interrupted: OpId,
}

/// Adds `count` entry directories with the state file of entry `source`.
fn copy_entries(fs: &CrashFs, source: OpId, count: u64) {
    let layout = JournalLayout::new(journal_dir());
    let state = content(fs, &layout.entry(source).unwrap().state().unwrap()).unwrap();
    for n in 0..count {
        let entry = layout.entry(OpId(FIRST_COPY + n)).unwrap();
        fs.create_dir(&scope(), entry.dir(), ENTRY_MODE).unwrap();
        let mut file = fs.create_new(&scope(), &entry.state().unwrap()).unwrap();
        file.write_all(&state).unwrap();
    }
}

fn recover_with_entries(entries: u64) -> Measured {
    let spec = VolumeSpec::new(volume_root());
    let bytes = spec.capacity_bytes;
    let inodes = spec.capacity_inodes + entries * INODES_PER_ENTRY;
    let spec = spec.with_capacity(bytes, inodes);
    let fs = CrashFs::new(CrashProfile::PosixWorstCase, SCALE_SEED, vec![spec]).unwrap();
    setup(&fs).unwrap();
    let create = vec![FileOp::create(absolute("data/new"), NEW.to_vec())];
    let committed = commit(&view(&fs), "write", create).unwrap().unwrap();
    fs.sync_all();

    let rule = FailRule {
        point: FailPoint::ApplyDone,
        n: None,
        occurrence: 0,
        action: FailAction::Crash,
    };
    let failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(SCALE_SEED, rule)));
    fs.attach_failpoints(Arc::clone(&failpoints));
    let remove = vec![FileOp::remove(absolute("data/b"))];
    let result = commit(&instrumented(&fs, &failpoints), "remove", remove);
    assert!(result.is_err() && fs.is_halted(), "{result:?}");
    fs.crash();
    copy_entries(&fs, committed, entries);
    fs.sync_all();

    let journal = view(&fs);
    let started = Instant::now();
    let report = journal.recover().unwrap();
    let elapsed = started.elapsed();
    let expected = before().with(&write("data/new", NEW));
    check_complete(
        &user_tree(&fs).unwrap(),
        &expected,
        &expected,
        Expected::After,
    )
    .unwrap();
    Measured {
        report,
        elapsed,
        interrupted: OpId(committed.0 + 1),
    }
}

fn assert_recovered(measured: &Measured) {
    let report = &measured.report;
    assert_eq!(report.rolled_back, [measured.interrupted], "{report:?}");
    assert!(
        report.corrupt.is_empty()
            && report.unresolved.is_empty()
            && report.needs_salvage.is_empty(),
        "recovery touched entries that were not interrupted: {report:?}"
    );
    assert!(
        measured.elapsed <= RECOVERY_BUDGET,
        "recovery took {:?}, longer than {RECOVERY_BUDGET:?}",
        measured.elapsed
    );
}

#[test]
fn journal_recovery_with_ten_thousand_entries_rolls_back_only_the_interrupted_one() {
    assert_recovered(&recover_with_entries(REDUCED_ENTRIES));
}

#[test]
#[ignore = "full-scale recovery; run: cargo test -p nurt-crashfs --release --test crash -- --ignored journal_recovery_with_a_million_entries"]
fn journal_recovery_with_a_million_entries_ends_within_five_seconds() {
    assert_recovered(&recover_with_entries(FULL_ENTRIES));
}
