// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Performance tests of the journal.
//!
//! Latency limits are checked by time on the platform file system of the
//! temporary directory; the cost of bulk removal and of overwrites is
//! checked by counting file system operations, which does not depend on the
//! machine. Tests at full scale run on request with `--ignored`.

#[allow(
    dead_code,
    reason = "the harness is shared by several test targets, each of which uses a subset"
)]
#[path = "../coverage/harness/mod.rs"]
mod harness;

use std::time::{Duration, Instant};

use nurt_contracts::config::StepLimit;
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, OpHandle, PreserveMode, TargetAction, TargetIndex, TargetPath,
    TargetSpec,
};

use harness::fixture::{Fixture, FixtureOptions, GIB};
use harness::request;

/// Upper limit of the median latency of a small journaled write.
const SMALL_WRITE_LIMIT: Duration = Duration::from_millis(1);
/// Writes measured for the median.
const SAMPLES: usize = 201;
/// Files of the bulk removal at full scale.
const FULL_SCALE_FILES: usize = 1_000_000;
/// Files of the bulk removal in the default run.
const DEFAULT_SCALE_FILES: usize = 10_000;
/// Files of the bulk operation whose undo is timed.
const BULK_FILES: usize = 1_000;
/// Undo of a bulk operation takes no longer than this multiple of the
/// operation.
const UNDO_FACTOR: u32 = 3;

fn spec(path: &nurt_contracts::fs::AbsPath, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

fn options() -> FixtureOptions {
    let mut options = FixtureOptions::default();
    options.journal.max_ops = StepLimit::Unlimited;
    options
}

fn single(fixture: &Fixture, command: &str, target: TargetSpec, mutation: Mutation) -> OpHandle {
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(request::begin(command, None));
    tx.preserve(&[target]).expect("target is preserved");
    tx.apply(TargetIndex(0), mutation)
        .expect("change is applied");
    tx.commit().expect("transaction commits")
}

#[test]
#[ignore = "requires the journal store implementation"]
fn small_write_with_journal_takes_within_one_millisecond() {
    let fixture = Fixture::open(options());
    let path = fixture.seed_file("work/small.txt", b"0\n");
    let mut samples: Vec<Duration> = (0..SAMPLES)
        .map(|round| {
            let started = Instant::now();
            single(
                &fixture,
                "write",
                spec(&path, TargetAction::Modify),
                Mutation::WriteBytes(format!("{round}\n").into_bytes()),
            );
            started.elapsed()
        })
        .collect();
    samples.sort();
    let median = samples[SAMPLES / 2];
    assert!(
        median <= SMALL_WRITE_LIMIT,
        "median write latency {median:?}"
    );
}

fn remove_tree_is_one_rename(files: usize) {
    let fixture = Fixture::open(options());
    let dir = fixture.seed_tree("work/tree", files);
    let before = fixture.counters();
    let handle = single(
        &fixture,
        "remove",
        spec(&dir, TargetAction::Delete),
        Mutation::Remove,
    );
    let after = fixture.counters();
    assert_eq!(handle.targets[0].mode, PreserveMode::Moved);
    assert_eq!(
        after.renames - before.renames,
        1,
        "the tree is moved by one rename"
    );
    assert_eq!(
        after.removals, before.removals,
        "no file of the tree is removed"
    );
    assert_eq!(after.bytes_copied, before.bytes_copied, "nothing is copied");
}

#[test]
#[ignore = "requires the journal store implementation"]
fn removing_a_tree_is_one_rename() {
    remove_tree_is_one_rename(DEFAULT_SCALE_FILES);
}

#[test]
#[ignore = "runs at full scale on request"]
fn removing_a_million_files_is_one_rename() {
    remove_tree_is_one_rename(FULL_SCALE_FILES);
}

#[test]
#[ignore = "requires the journal store implementation"]
fn overwriting_one_gib_by_exchange_copies_no_bytes() {
    let fixture = Fixture::open(options());
    let path = fixture.seed_sparse("work/large.bin", GIB);
    let before = fixture.counters();
    let handle = single(
        &fixture,
        "write",
        spec(&path, TargetAction::Modify),
        Mutation::WriteBytes(b"replacement".to_vec()),
    );
    let mode = handle.targets[0].mode;
    assert!(
        matches!(
            mode,
            PreserveMode::Exchange | PreserveMode::ReplaceBackup | PreserveMode::Hardlink
        ),
        "the previous content is kept under another name, not copied: {}",
        mode.as_str()
    );
    assert_eq!(fixture.counters().bytes_copied, before.bytes_copied);
}

#[test]
#[ignore = "requires the journal store implementation"]
fn bulk_undo_takes_no_longer_than_three_times_the_operation() {
    let fixture = Fixture::open(options());
    let paths: Vec<_> = (0..BULK_FILES)
        .map(|index| fixture.seed_file(&format!("work/bulk/{index}.txt"), b"before\n"))
        .collect();
    let mut begin = request::begin("replace", None);
    begin.bulk = true;
    let started = Instant::now();
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(begin);
    let specs: Vec<TargetSpec> = paths
        .iter()
        .map(|path| spec(path, TargetAction::Modify))
        .collect();
    tx.preserve(&specs).expect("targets are preserved");
    for index in 0..specs.len() {
        let index = TargetIndex(u32::try_from(index).expect("index fits"));
        tx.apply(index, Mutation::WriteBytes(b"after\n".to_vec()))
            .expect("change is applied");
    }
    tx.commit().expect("bulk operation commits");
    let operation = started.elapsed();
    let started = Instant::now();
    fixture
        .journal
        .undo(&request::undo_last())
        .expect("bulk operation is undone");
    let undo = started.elapsed();
    assert!(
        undo <= operation * UNDO_FACTOR,
        "undo took {undo:?}, the operation {operation:?}"
    );
}
