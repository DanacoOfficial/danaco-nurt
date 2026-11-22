// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Collection and merging of irreversibility reasons.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::report::{Change, ChangeAction, Irreversible, IrreversibleEntry};
use nurt_report::assemble::TargetChange;
use nurt_report::rules::{ENTRIES_PER_CODE, collect_irreversible};

fn change(path: &str, reason: Option<Irreversible>) -> TargetChange {
    TargetChange::file(Change {
        irreversible: reason,
        ..Change::new(path, ChangeAction::Modify)
    })
}

fn entry(reason: Irreversible, path: Option<&str>, count: Option<u64>) -> IrreversibleEntry {
    IrreversibleEntry {
        path: path.map(str::to_owned),
        count,
        ..IrreversibleEntry::new(reason)
    }
}

#[test]
fn reversible_changes_produce_no_entries() {
    let changes = [change("/srv/app/a", None), change("/srv/app/b", None)];
    assert!(collect_irreversible(&changes, Vec::new()).is_empty());
    assert!(collect_irreversible(&[], Vec::new()).is_empty());
}

#[test]
fn each_irreversible_change_is_listed_with_its_path() {
    let changes = [
        change("/srv/app/a", Some(Irreversible::NoSpaceForCopy)),
        change("/srv/app/b", None),
        change("/srv/app/c", Some(Irreversible::NoSpaceForCopy)),
    ];
    assert_eq!(
        collect_irreversible(&changes, Vec::new()),
        vec![
            entry(Irreversible::NoSpaceForCopy, Some("/srv/app/a"), None),
            entry(Irreversible::NoSpaceForCopy, Some("/srv/app/c"), None),
        ]
    );
}

#[test]
fn a_reason_shared_by_many_targets_becomes_one_entry() {
    let changes: Vec<TargetChange> = (0..=ENTRIES_PER_CODE)
        .map(|index| {
            change(
                &format!("/mnt/share/data/{index}.bin"),
                Some(Irreversible::NetworkVolume),
            )
        })
        .collect();
    let total = ENTRIES_PER_CODE as u64 + 1;
    assert_eq!(
        collect_irreversible(&changes, Vec::new()),
        vec![IrreversibleEntry {
            detail: Some(format!("{total} targets")),
            ..entry(
                Irreversible::NetworkVolume,
                Some("/mnt/share/data"),
                Some(total)
            )
        }]
    );
}

#[test]
fn entries_follow_the_declaration_order_of_reasons() {
    let reported = vec![
        entry(
            Irreversible::PreviousContentNotKept,
            Some("/srv/app/go.sum"),
            None,
        ),
        entry(Irreversible::ServiceChanged, Some("service:nginx"), None),
        entry(Irreversible::ExternalEffect, None, None),
    ];
    let reasons: Vec<Irreversible> = collect_irreversible(&[], reported)
        .into_iter()
        .map(|entry| entry.reason)
        .collect();
    assert_eq!(
        reasons,
        vec![
            Irreversible::ExternalEffect,
            Irreversible::ServiceChanged,
            Irreversible::PreviousContentNotKept,
        ]
    );
}

#[test]
fn duplicate_entries_are_merged_and_counts_added() {
    let changes = [change("/srv/app/a", Some(Irreversible::JournalError))];
    let reported = vec![
        entry(Irreversible::JournalError, Some("/srv/app/a"), None),
        entry(Irreversible::JournalEntriesExpired, None, Some(2)),
        entry(Irreversible::JournalEntriesExpired, None, Some(3)),
        entry(Irreversible::ServiceChanged, Some("service:nginx"), None),
        entry(Irreversible::ServiceChanged, Some("service:nginx"), None),
    ];
    assert_eq!(
        collect_irreversible(&changes, reported),
        vec![
            entry(Irreversible::ServiceChanged, Some("service:nginx"), None),
            entry(Irreversible::JournalEntriesExpired, None, Some(5)),
            entry(Irreversible::JournalError, Some("/srv/app/a"), None),
        ]
    );
}

#[test]
fn entries_with_different_details_stay_separate() {
    let reported = vec![
        IrreversibleEntry {
            detail: Some("restart".to_owned()),
            ..entry(Irreversible::ServiceChanged, Some("service:nginx"), None)
        },
        IrreversibleEntry {
            detail: Some("stop".to_owned()),
            ..entry(Irreversible::ServiceChanged, Some("service:nginx"), None)
        },
    ];
    assert_eq!(collect_irreversible(&[], reported).len(), 2);
}
