// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Facts reported by the executor mapped onto report input.

use nurt_contracts::config::ReportConfig;
use nurt_contracts::env::Os;
use nurt_contracts::fs::FileKind;
use nurt_contracts::report::{Change, ChangeAction, Notable, ReportStatus, Severity};
use nurt_contracts::value::Size;
use nurt_report::assemble::{EntryKind, ReportBuilder, ReportContext, TargetChange};
use nurt_report::rules::PathContext;

fn ctx() -> ReportContext {
    ReportContext::new(ReportConfig::default(), PathContext::new(Os::Linux))
}

#[test]
fn maps_journal_kinds_to_entry_kinds() {
    assert_eq!(EntryKind::from(FileKind::File), EntryKind::File);
    assert_eq!(EntryKind::from(FileKind::Directory), EntryKind::Directory);
    assert_eq!(EntryKind::from(FileKind::Symlink), EntryKind::Link);
    assert_eq!(EntryKind::from(FileKind::Junction), EntryKind::Link);
    assert_eq!(EntryKind::from(FileKind::Special), EntryKind::Other);
}

#[test]
fn infers_entry_kind_when_journal_did_not_report_it() {
    let path = Change::new("/srv/a.txt", ChangeAction::Modify);
    assert_eq!(
        TargetChange::observed(path, None, false).entry,
        EntryKind::File
    );
    let provider = Change::new("service:nginx", ChangeAction::Modify);
    assert_eq!(
        TargetChange::observed(provider, None, false).entry,
        EntryKind::Other
    );
}

#[test]
fn replaced_existing_entry_reports_overwrote_existing() {
    let change = Change::new("/srv/a.txt", ChangeAction::Modify);
    let report = ReportBuilder::changing(&ctx(), "write", ReportStatus::Ok)
        .op(7)
        .change(TargetChange::observed(change, Some(FileKind::File), true))
        .build()
        .unwrap();
    let entry = report
        .notable
        .iter()
        .find(|entry| entry.code == Notable::OverwroteExisting)
        .unwrap();
    assert_eq!(entry.severity, Severity::Warning);
}

#[test]
fn shrink_to_half_of_one_kibibyte_is_reported_and_below_is_not() {
    let shrink = |before: u64, after: u64| {
        let change = Change {
            size_before: Some(Size::from_bytes(before)),
            size_after: Some(Size::from_bytes(after)),
            ..Change::new("/srv/a.txt", ChangeAction::Modify)
        };
        ReportBuilder::changing(&ctx(), "write", ReportStatus::Ok)
            .op(1)
            .change(TargetChange::observed(change, Some(FileKind::File), false))
            .build()
            .unwrap()
            .notable
            .iter()
            .any(|entry| entry.code == Notable::SizeShrunk)
    };
    assert!(shrink(2048, 1024));
    assert!(!shrink(2048, 1025));
    assert!(!shrink(1023, 1));
}
