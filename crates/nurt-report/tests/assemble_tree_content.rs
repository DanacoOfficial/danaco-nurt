// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Totals of changes of directories removed or restored with their content.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::report::{Change, ChangeAction};
use nurt_contracts::value::Size;
use nurt_report::assemble::{TargetChange, count_targets, summarize_changes};

fn tree(action: ChangeAction, files: u64, directories: u64) -> TargetChange {
    TargetChange::directory(Change {
        size_before: Some(Size::from_bytes(6)),
        ..Change::new("/srv/app/cache", action)
    })
    .with_content(files, directories)
}

#[test]
fn deleted_tree_counts_the_entries_inside() {
    let changes = [tree(ChangeAction::Delete, 4, 2)];
    let summary = summarize_changes(&changes).unwrap();
    assert_eq!(summary.files_deleted, Some(4));
    assert_eq!(summary.directories_deleted, Some(3));
    assert_eq!(summary.bytes_freed, Some(6));
    let targets = count_targets(&changes);
    assert_eq!(targets.count, 1);
    assert_eq!(targets.files, Some(4));
    assert_eq!(targets.directories, Some(3));
    assert_eq!(targets.bytes, Some(6));
}

#[test]
fn restored_tree_counts_the_entries_inside() {
    let changes = [tree(ChangeAction::Create, 4, 2)];
    let summary = summarize_changes(&changes).unwrap();
    assert_eq!(summary.files_created, Some(4));
    assert_eq!(summary.directories_created, Some(3));
    assert_eq!(summary.files_deleted, None);
}

#[test]
fn empty_directory_and_directory_without_counts_count_one_directory() {
    let empty = tree(ChangeAction::Delete, 0, 0);
    let unknown = TargetChange::directory(Change::new("/srv/app/other", ChangeAction::Delete));
    let changes = [empty, unknown];
    let summary = summarize_changes(&changes).unwrap();
    assert_eq!(summary.directories_deleted, Some(2));
    assert_eq!(summary.files_deleted, None);
    let targets = count_targets(&changes);
    assert_eq!(targets.directories, Some(2));
    assert_eq!(targets.files, None);
}

#[test]
fn content_counts_set_the_number_of_entries_inside() {
    let target = tree(ChangeAction::Delete, 4, 2);
    assert_eq!(target.entries_inside, Some(6));
    assert_eq!(target.files_inside, Some(4));
    assert_eq!(target.directories_inside, Some(2));
}
