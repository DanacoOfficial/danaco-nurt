// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::track::tests::events
// Responsibility:   Detection accelerated by file system events.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Detection accelerated by file system events.

use std::fs;

use nurt_contracts::record::Tracking;
use nurt_contracts::report::{ChangeAction, Notable};

use super::support::{Harness, RecordingTx};
use crate::iface::{PreparedScan, TrackMode};
use crate::track::Tracker;

#[test]
fn events_narrow_the_second_scan_to_touched_paths() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    h.write("gone.txt", "gone");
    h.write("deep/keep.txt", "keep");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Events), &mut tx, |h| {
        h.write("a.txt", "alpha changed");
        fs::remove_file(h.path("gone.txt")).unwrap();
        h.write("fresh/nested/new.txt", "new");
    });
    assert_eq!(outcome.tracking, Tracking::Events);
    assert_eq!(
        h.actions(&outcome),
        vec![
            ("a.txt".to_owned(), ChangeAction::Modify),
            ("fresh".to_owned(), ChangeAction::Create),
            ("fresh/nested".to_owned(), ChangeAction::Create),
            ("fresh/nested/new.txt".to_owned(), ChangeAction::Create),
            ("gone.txt".to_owned(), ChangeAction::Delete),
        ]
    );
}

#[test]
fn lost_events_fall_back_to_a_full_scan() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    let mut tx = RecordingTx::new();
    let prepared = Tracker::default()
        .capture(&h.ctx, &h.spec(TrackMode::Auto), &mut tx)
        .unwrap();
    if let Some(watch) = prepared.watch.as_ref() {
        watch.lose_events();
    }
    h.write("a.txt", "alpha changed");
    h.write("b.txt", "beta");
    let outcome = Box::new(prepared).finish(&h.ctx, &mut tx).unwrap();
    assert_eq!(outcome.tracking, Tracking::Scan);
    assert_eq!(
        h.actions(&outcome),
        vec![
            ("a.txt".to_owned(), ChangeAction::Modify),
            ("b.txt".to_owned(), ChangeAction::Create),
        ]
    );
}

#[test]
fn changes_below_ignored_names_are_counted() {
    let h = Harness::new();
    h.write(".git/HEAD", "ref: main");
    h.write("a.txt", "alpha");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Events), &mut tx, |h| {
        h.write(".git/HEAD", "ref: feature");
        h.write(".git/refs/heads/feature", "0123");
    });
    assert_eq!(outcome.tracking, Tracking::Events);
    assert!(outcome.changes.is_empty());
    let ignored = outcome
        .notes
        .iter()
        .find(|note| note.code == Notable::Ignored)
        .expect("ignored changes are noted");
    assert!(ignored.count.unwrap_or_default() >= 1);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
