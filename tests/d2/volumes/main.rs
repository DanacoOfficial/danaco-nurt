// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal tests on real volumes.
//!
//! The volumes are prepared outside the tests and named by environment
//! variables; every test works in a new temporary directory on its volume.
//!
//! | Variable | Volume |
//! |---|---|
//! | `NURT_TEST_EXT4_DIR` | ext4 of 1 GiB from a file |
//! | `NURT_TEST_EXT4_SMALL_DIR` | ext4 of 64 MiB from a file |
//! | `NURT_TEST_XFS_DIR` | XFS from a file |
//! | `NURT_TEST_BTRFS_DIR` | btrfs from a file |
//! | `NURT_TEST_OVERLAY_DIR` | overlay mount whose directory `lower` comes from the lower layer |
//! | `NURT_TEST_OVERLAY_SMALL_DIR` | overlay mount with a 64 MiB upper layer and a directory `lower` of 48 MiB from the lower layer |
//! | `NURT_TEST_TMPFS_DIR` | tmpfs |
//!
//! A test run with `--ignored` without its variable fails and names the
//! variable.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions report failures by panicking"
)]

#[allow(
    dead_code,
    reason = "the harness is shared by several test targets, each of which uses a subset"
)]
#[path = "../coverage/harness/mod.rs"]
mod harness;

use std::fs;

use nurt_contracts::config::{SizeOrPercent, SizeSetting};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, InactiveReason, JournalNotable, Mutation, MutationTx, OpHandle, PreserveMode,
    TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::Irreversible;

use harness::fixture::{Fixture, FixtureOptions, mounted_dir};
use harness::request;
use harness::snapshot::TreeSnapshot;

/// Size of the file whose copy does not fit on a 64 MiB volume.
const LARGE_FILE: usize = 40 << 20;

/// Space reserve of the prepared volumes. The default reserve is at least
/// 5 GiB, which exceeds the free space of every prepared volume and would
/// make admission skip the preservation of every change.
const RESERVE_MIB: u64 = 1;

fn fixture_on(variable: &str) -> Fixture {
    let mut options = FixtureOptions {
        mounted: vec![mounted_dir(variable)],
        ..FixtureOptions::default()
    };
    options.space.reserve = SizeOrPercent::Size(SizeSetting::from_mib(RESERVE_MIB));
    Fixture::open(options)
}

fn single(
    fixture: &Fixture,
    command: &str,
    path: &AbsPath,
    action: TargetAction,
    mutation: Mutation,
) -> OpHandle {
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(request::begin(command, None));
    tx.preserve(&[TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }])
    .expect("target is preserved");
    tx.apply(TargetIndex(0), mutation)
        .expect("change is applied");
    tx.commit().expect("transaction commits")
}

fn overwrite_round_trips(fixture: &Fixture, accepted: &[PreserveMode]) {
    let path = fixture.seed_file("work/file.txt", b"before\n");
    let before = fixture.snapshot();
    let handle = single(
        fixture,
        "write",
        &path,
        TargetAction::Modify,
        Mutation::WriteBytes(b"after\n".to_vec()),
    );
    let mode = handle.targets[0].mode;
    assert!(
        accepted.contains(&mode),
        "overwrite kept by {} (irreversible: {:?})",
        mode.as_str(),
        handle.targets[0].irreversible
    );
    let after = fixture.snapshot();
    let undone = fixture
        .journal
        .undo(&request::undo_last())
        .expect("overwrite is undone");
    before.assert_same(&fixture.snapshot(), "after undo");
    fixture
        .journal
        .redo(&request::redo(Some(undone.entry)))
        .expect("undo is redone");
    after.assert_same(&fixture.snapshot(), "after redo");
}

#[test]
#[ignore = "requires a file-backed volume"]
fn ext4_overwrite_is_kept_by_exchange() {
    overwrite_round_trips(&fixture_on("NURT_TEST_EXT4_DIR"), &[PreserveMode::Exchange]);
}

#[test]
#[ignore = "requires a file-backed volume"]
fn xfs_overwrite_is_kept_by_exchange_or_clone() {
    overwrite_round_trips(
        &fixture_on("NURT_TEST_XFS_DIR"),
        &[PreserveMode::Exchange, PreserveMode::Clone],
    );
}

#[test]
#[ignore = "requires a file-backed volume"]
fn btrfs_overwrite_is_kept_by_exchange_or_clone() {
    overwrite_round_trips(
        &fixture_on("NURT_TEST_BTRFS_DIR"),
        &[PreserveMode::Exchange, PreserveMode::Clone],
    );
}

#[test]
#[ignore = "requires a file-backed volume"]
fn small_ext4_volume_without_space_for_copy_marks_overwrite_irreversible() {
    let fixture = fixture_on("NURT_TEST_EXT4_SMALL_DIR");
    let path = fixture.seed_file("work/large.bin", &vec![1u8; LARGE_FILE]);
    let link = fixture.path("work/second-name.bin");
    fs::hard_link(path.as_path(), link.as_path()).expect("second name is created");
    let handle = single(
        &fixture,
        "write",
        &path,
        TargetAction::Modify,
        Mutation::WriteBytes(b"small".to_vec()),
    );
    assert_eq!(
        handle.targets[0].mode,
        PreserveMode::None,
        "a file with two names needs a copy"
    );
    assert_eq!(
        handle.targets[0].irreversible,
        Some(Irreversible::NoSpaceForCopy)
    );
}

/// Directory `lower` of the overlay mount, which comes from the lower layer.
/// The test changes it through the journal and restores it by undo.
fn lower_dir(variable: &str) -> AbsPath {
    let lower = mounted_dir(variable).join("lower");
    assert!(
        lower.is_dir(),
        "{variable} has no directory `lower` from the lower layer"
    );
    AbsPath::new(lower).expect("lower directory is absolute")
}

#[test]
#[ignore = "requires a file-backed volume"]
fn overlay_lower_layer_directory_is_removed_by_copy_and_restored() {
    let fixture = fixture_on("NURT_TEST_OVERLAY_DIR");
    let lower = lower_dir("NURT_TEST_OVERLAY_DIR");
    let before = TreeSnapshot::take(fixture.fs.as_ref(), &lower, &[]);
    let handle = single(
        &fixture,
        "remove",
        &lower,
        TargetAction::Delete,
        Mutation::Remove,
    );
    let target = &handle.targets[0];
    assert_eq!(
        target.mode,
        PreserveMode::Copy,
        "a rename across layers fails, the journal copies (irreversible: {:?})",
        target.irreversible
    );
    assert!(
        target
            .notable
            .iter()
            .any(|notable| matches!(notable, JournalNotable::CrossDeviceCopy { .. }))
    );
    fixture
        .journal
        .undo(&request::undo_last())
        .expect("removal is undone");
    assert!(lower.as_path().is_dir(), "the directory is restored");
    before.assert_same(
        &TreeSnapshot::take(fixture.fs.as_ref(), &lower, &[]),
        "after undo",
    );
}

#[test]
#[ignore = "requires a file-backed volume"]
fn overlay_file_from_lower_layer_round_trips_after_copy_up() {
    let fixture = fixture_on("NURT_TEST_OVERLAY_DIR");
    let lower = lower_dir("NURT_TEST_OVERLAY_DIR");
    let file = fs::read_dir(lower.as_path())
        .expect("lower directory is listed")
        .map(|entry| entry.expect("entry is readable").path())
        .find(|path| path.is_file())
        .expect("lower directory holds a file");
    let path = AbsPath::new(&file).expect("path is absolute");
    let original = fs::read(&file).expect("file is readable");
    single(
        &fixture,
        "write",
        &path,
        TargetAction::Modify,
        Mutation::WriteBytes(b"after\n".to_vec()),
    );
    fixture
        .journal
        .undo(&request::undo_last())
        .expect("overwrite is undone");
    assert_eq!(fs::read(&file).expect("file is readable"), original);
}

#[test]
#[ignore = "requires a file-backed volume"]
fn overlay_without_space_for_copy_marks_removal_irreversible() {
    let fixture = fixture_on("NURT_TEST_OVERLAY_SMALL_DIR");
    let lower = lower_dir("NURT_TEST_OVERLAY_SMALL_DIR");
    let handle = single(
        &fixture,
        "remove",
        &lower,
        TargetAction::Delete,
        Mutation::Remove,
    );
    assert_eq!(handle.targets[0].mode, PreserveMode::None);
    assert_eq!(
        handle.targets[0].irreversible,
        Some(Irreversible::NoSpaceForCopy)
    );
}

#[test]
#[ignore = "requires a file-backed volume"]
fn tmpfs_volume_has_inactive_journal() {
    let fixture = fixture_on("NURT_TEST_TMPFS_DIR");
    assert_eq!(
        fixture.status().inactive,
        Some(InactiveReason::MemoryVolume)
    );
    let path = fixture.seed_file("work/file.txt", b"before\n");
    let handle = single(
        &fixture,
        "write",
        &path,
        TargetAction::Modify,
        Mutation::WriteBytes(b"after\n".to_vec()),
    );
    assert_eq!(handle.targets[0].mode, PreserveMode::None);
    assert_eq!(
        handle.targets[0].irreversible,
        Some(Irreversible::JournalInactive)
    );
}

#[test]
fn volume_tests_name_their_variable_when_it_is_missing() {
    let missing = "NURT_TEST_VOLUME_THAT_IS_NEVER_SET";
    let result = std::panic::catch_unwind(|| mounted_dir(missing));
    let message = result
        .expect_err("a missing variable fails the test")
        .downcast::<String>()
        .map(|message| *message)
        .unwrap_or_default();
    assert!(
        message.contains(missing),
        "message names the variable: {message}"
    );
}
