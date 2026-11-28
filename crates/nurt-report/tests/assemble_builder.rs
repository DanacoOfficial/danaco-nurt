// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/assemble_builder.rs
// Responsibility:   Assembly of reports: targets, summary, operation number, listing, notable effects and irreversibility.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Assembly of reports: targets, summary, operation number, listing,
//! notable effects and irreversibility.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::config::ReportConfig;
use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::record::Operation;
use nurt_contracts::report::{
    Change, ChangeAction, Hunk, Irreversible, IrreversibleEntry, Notable, NotableEntry,
    RecordsPage, Report, ReportStatus, ReportViolation, Severity, Summary, Targets,
};
use nurt_contracts::value::{Path, Record, RecordKind, Size};
use nurt_report::assemble::{
    EntryKind, ReportBuilder, ReportContext, TargetChange, count_targets, summarize_changes,
};
use nurt_report::rules::PathContext;
use serde_json::json;

fn ctx() -> ReportContext {
    ReportContext::new(
        ReportConfig::default(),
        PathContext::new(Os::Linux).with_home("/home/user"),
    )
}

fn hunk(path: &str, line: u64) -> Hunk {
    Hunk {
        path: Path::new(path),
        old_start: line,
        old_len: 1,
        new_start: line,
        new_len: 1,
        lines: vec!["-old".to_owned(), "+new".to_owned()],
        ..Hunk::default()
    }
}

fn modified(path: &str, before: u64, after: u64, replacements: u64) -> Change {
    Change {
        size_before: Some(Size::from_bytes(before)),
        size_after: Some(Size::from_bytes(after)),
        replacements: Some(replacements),
        added: Some(replacements),
        removed: Some(replacements),
        diff: Some(vec![hunk(path, 12)]),
        ..Change::new(path, ChangeAction::Modify)
    }
}

fn replace_changes() -> Vec<Change> {
    vec![
        modified("/srv/app/src/a.go", 1127, 1131, 3),
        modified("/srv/app/src/b.go", 8192, 8194, 2),
        modified("/srv/app/src/c/d.go", 32870, 32966, 2),
    ]
}

#[test]
fn replace_report_carries_targets_summary_changes_and_diff() {
    let ctx = ctx();
    let report = ReportBuilder::changing(&ctx, "replace", ReportStatus::Ok)
        .op(18)
        .changes(replace_changes())
        .build()
        .unwrap();
    assert_eq!(report.op, Some(18));
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value["targets"],
        json!({"count": 3, "files": 3, "bytes": 42291, "listed": 3})
    );
    assert_eq!(
        value["summary"],
        json!({
            "files_modified": 3,
            "replacements": 7,
            "lines_added": 7,
            "lines_removed": 7,
            "bytes_before": 42189,
            "bytes_after": 42291
        })
    );
    assert_eq!(report.changes.len(), 3);
    assert!(report.changes.iter().all(|change| change.diff.is_none()));
    assert_eq!(report.diff.len(), 3);
    assert_eq!(report.diff[2].path.as_str(), "/srv/app/src/c/d.go");
    assert!(report.notable.is_empty());
    assert!(report.irreversible.is_empty());
    assert!(report.bulk.is_none());
    assert!(value.get("irreversible").is_none());
    assert!(value.get("notable").is_none());
}

#[test]
fn preview_drops_the_operation_number_and_keeps_notable_effects() {
    let ctx = ctx();
    let emptied = Change {
        size_before: Some(Size::from_bytes(10)),
        size_after: Some(Size::ZERO),
        ..Change::new("/srv/app/a.txt", ChangeAction::Modify)
    };
    let report = ReportBuilder::changing(&ctx, "write", ReportStatus::Planned)
        .op(5)
        .change(emptied)
        .build()
        .unwrap();
    assert_eq!(report.status, ReportStatus::Planned);
    assert_eq!(report.op, None);
    assert_eq!(report.notable.len(), 1);
    assert_eq!(report.notable[0].code, Notable::FileEmptied);
}

#[test]
fn changing_program_without_targets_reports_nothing_matched() {
    let ctx = ctx();
    let report = ReportBuilder::changing(&ctx, "remove", ReportStatus::Ok)
        .op(4)
        .note(NotableEntry::new(Notable::NothingMatched))
        .build()
        .unwrap();
    assert_eq!(report.op, None);
    assert_eq!(report.targets, Some(Targets::default()));
    assert_eq!(report.summary, None);
    assert_eq!(
        report.notable,
        vec![NotableEntry::new(Notable::NothingMatched)]
    );

    let planned = ReportBuilder::changing(&ctx, "remove", ReportStatus::Planned)
        .build()
        .unwrap();
    assert_eq!(planned.notable[0].code, Notable::NothingMatched);
    assert_eq!(planned.notable[0].severity, Severity::Notice);
}

#[test]
fn programs_with_a_process_effect_do_not_report_nothing_matched() {
    let ctx = ctx();
    let run = ReportBuilder::changing(&ctx, "run", ReportStatus::Ok)
        .effect(EffectKind::Process)
        .build()
        .unwrap();
    assert!(run.notable.is_empty());
    assert_eq!(run.targets, Some(Targets::default()));
    for effect in [EffectKind::Local, EffectKind::External] {
        let report = ReportBuilder::changing(&ctx, "start", ReportStatus::Ok)
            .effect(effect)
            .build()
            .unwrap();
        assert_eq!(
            report.notable[0].code,
            Notable::NothingMatched,
            "{effect:?}"
        );
    }
}

#[test]
fn a_program_that_ran_a_process_reports_no_nothing_matched() {
    let ctx = ctx();
    let report = ReportBuilder::changing(&ctx, "run", ReportStatus::Ok)
        .exec(Record::new(RecordKind::Exec))
        .build()
        .unwrap();
    assert!(report.notable.is_empty());
}

#[test]
fn failed_program_needs_an_error_and_reports_no_nothing_matched() {
    let ctx = ctx();
    assert_eq!(
        ReportBuilder::changing(&ctx, "remove", ReportStatus::Failed).build(),
        Err(ReportViolation::MissingError {
            status: ReportStatus::Failed
        })
    );
    let report = ReportBuilder::changing(&ctx, "remove", ReportStatus::Failed)
        .error(Error::new(ErrorCode::Internal, "the program stopped"))
        .build()
        .unwrap();
    assert!(report.notable.is_empty());
    assert_eq!(report.error.unwrap().code(), ErrorCode::Internal);
}

#[test]
fn running_program_needs_a_job_record() {
    let ctx = ctx();
    assert_eq!(
        ReportBuilder::changing(&ctx, "run", ReportStatus::Running).build(),
        Err(ReportViolation::MissingJob)
    );
    let report = ReportBuilder::changing(&ctx, "run", ReportStatus::Running)
        .job(Record::new(RecordKind::Job))
        .build()
        .unwrap();
    assert!(report.job.is_some());
}

#[test]
fn rename_without_previous_path_is_rejected() {
    let ctx = ctx();
    let result = ReportBuilder::changing(&ctx, "move", ReportStatus::Ok)
        .op(2)
        .change(Change::new("/srv/app/b.txt", ChangeAction::Rename))
        .build();
    assert_eq!(
        result,
        Err(ReportViolation::RenameWithoutFrom {
            path: "/srv/app/b.txt".to_owned()
        })
    );
}

#[test]
fn reading_program_has_no_operation_number_and_keeps_records() {
    let ctx = ctx();
    let page = RecordsPage {
        record_type: RecordKind::ALL[0],
        matched: Some(148),
        header: None,
        rows: Vec::new(),
    };
    let report = ReportBuilder::reading(&ctx, "find", ReportStatus::Ok)
        .op(3)
        .records(page.clone())
        .note(NotableEntry::new(Notable::PermissionDeniedSkipped))
        .next_cursor("c_91fe")
        .build()
        .unwrap();
    assert_eq!(report.op, None);
    assert_eq!(report.targets, None);
    assert_eq!(report.summary, None);
    assert_eq!(report.records, Some(page));
    assert_eq!(report.notable[0].code, Notable::PermissionDeniedSkipped);
    assert_eq!(report.next_cursor.as_deref(), Some("c_91fe"));
    assert!(report.has_more());
}

#[test]
fn list_limit_bounds_the_listed_changes_but_not_the_totals() {
    let ctx = ctx();
    let changes: Vec<Change> = (0..5)
        .map(|index| modified(&format!("/srv/app/{index}.go"), 100, 100, 1))
        .collect();
    let report = ReportBuilder::changing(&ctx, "replace", ReportStatus::Ok)
        .op(9)
        .changes(changes)
        .list_limit(2)
        .next_cursor("c_2")
        .build()
        .unwrap();
    assert_eq!(report.changes.len(), 2);
    assert_eq!(report.diff.len(), 2);
    let targets = report.targets.unwrap();
    assert_eq!(targets.count, 5);
    assert_eq!(targets.listed, Some(2));
    assert_eq!(report.summary.unwrap().files_modified, Some(5));

    let quiet = ReportBuilder::changing(&ctx, "replace", ReportStatus::Ok)
        .op(9)
        .changes(replace_changes())
        .list_limit(0)
        .build()
        .unwrap();
    assert!(quiet.changes.is_empty());
    assert!(quiet.diff.is_empty());
    assert_eq!(quiet.targets.unwrap().listed, None);
}

#[test]
fn explicit_targets_replace_the_counted_ones() {
    let ctx = ctx();
    let report = ReportBuilder::changing(&ctx, "replace", ReportStatus::Ok)
        .op(1)
        .targets(Targets {
            count: 148,
            files: Some(148),
            listed: Some(99),
            ..Targets::default()
        })
        .change(modified("/srv/app/a.go", 10, 10, 1))
        .build()
        .unwrap();
    let targets = report.targets.unwrap();
    assert_eq!(targets.count, 148);
    assert_eq!(targets.listed, Some(1));
}

#[test]
fn irreversible_is_present_only_for_irreversible_effects() {
    let ctx = ctx();
    let reversible = ReportBuilder::changing(&ctx, "write", ReportStatus::Ok)
        .op(1)
        .change(modified("/srv/app/a.go", 10, 12, 1))
        .build()
        .unwrap();
    let value = serde_json::to_value(&reversible).unwrap();
    assert!(value.get("irreversible").is_none());

    let irreversible = ReportBuilder::changing(&ctx, "write", ReportStatus::Ok)
        .op(2)
        .change(Change {
            irreversible: Some(Irreversible::NoSpaceForCopy),
            ..modified("/srv/app/a.go", 10, 12, 1)
        })
        .build()
        .unwrap();
    let value = serde_json::to_value(&irreversible).unwrap();
    assert_eq!(
        value["irreversible"],
        json!([{"reason": "no_space_for_copy", "path": "/srv/app/a.go"}])
    );
}

#[test]
fn operation_record_gives_number_and_journal_reasons() {
    let ctx = ctx();
    let operation = Operation {
        id: 33,
        irreversible: vec![IrreversibleEntry {
            count: Some(3),
            ..IrreversibleEntry::new(Irreversible::JournalEntriesExpired)
        }],
        ..Operation::default()
    };
    let report = ReportBuilder::changing(&ctx, "write", ReportStatus::Ok)
        .operation(&operation)
        .irreversible(IrreversibleEntry::new(Irreversible::ExternalEffect))
        .irreversible_entries([IrreversibleEntry::new(Irreversible::ExternalEffect)])
        .change(modified("/srv/app/.nurt.toml", 410, 388, 1))
        .build()
        .unwrap();
    assert_eq!(report.op, Some(33));
    let reasons: Vec<Irreversible> = report.irreversible.iter().map(|e| e.reason).collect();
    assert_eq!(
        reasons,
        vec![
            Irreversible::ExternalEffect,
            Irreversible::JournalEntriesExpired
        ]
    );
}

#[test]
fn plain_fields_are_carried_into_the_report() {
    let ctx = ctx();
    let hash = format!("sha256:{}", "a".repeat(64));
    let report = ReportBuilder::changing(&ctx, "run", ReportStatus::Ok)
        .op(20)
        .ast_hash(hash.clone())
        .result_file("/srv/out/result.json")
        .exec(Record::new(RecordKind::Exec))
        .test(Record::new(RecordKind::Exec))
        .analysis(Record::new(RecordKind::Exec))
        .git(Record::new(RecordKind::Exec))
        .timing(nurt_contracts::report::Timing {
            parse_ms: Some(1),
            ..nurt_contracts::report::Timing::default()
        })
        .notes([NotableEntry::new(Notable::DetectionScope)])
        .change(Change {
            size_before: Some(Size::ZERO),
            size_after: Some(Size::from_bytes(2150)),
            ..Change::new("/srv/app/.cache/go-test.json", ChangeAction::Create)
        })
        .build()
        .unwrap();
    assert_eq!(report.ast_hash, Some(hash));
    assert_eq!(report.result_file.as_deref(), Some("/srv/out/result.json"));
    assert!(report.exec.is_some() && report.test.is_some());
    assert!(report.analysis.is_some() && report.git.is_some());
    assert_eq!(report.timing.unwrap().parse_ms, Some(1));
    assert_eq!(report.notable[0].code, Notable::DetectionScope);
    assert_eq!(report.summary.unwrap().files_created, Some(1));
}

#[test]
fn corrections_require_the_canonical_text() {
    use nurt_contracts::report::{Correction, CorrectionClass};
    let ctx = ctx();
    let correction = Correction {
        id: "C1".to_owned(),
        rule: "L01".to_owned(),
        class: CorrectionClass::Fuzzy,
        at: None,
        from: "remvoe".to_owned(),
        to: "remove".to_owned(),
        kind: None,
        alternatives: Vec::new(),
    };
    let report = ReportBuilder::changing(&ctx, "remove", ReportStatus::Ok)
        .op(21)
        .ran("remove \"/srv/app/build/old.log\"")
        .corrections(vec![correction])
        .change(TargetChange::file(Change {
            size_before: Some(Size::from_bytes(3277)),
            ..Change::new("/srv/app/build/old.log", ChangeAction::Delete)
        }))
        .build()
        .unwrap();
    assert_eq!(report.corrections.len(), 1);
    assert_eq!(report.summary.unwrap().bytes_freed, Some(3277));
    assert_eq!(
        ReportBuilder::changing(&ctx, "remove", ReportStatus::Ok)
            .ran("remove x")
            .build(),
        Err(ReportViolation::RanWithoutCorrections)
    );
}

#[test]
fn reports_carry_nothing_about_undo() {
    let ctx = ctx();
    let reports = [
        ReportBuilder::changing(&ctx, "replace", ReportStatus::Ok)
            .op(18)
            .changes(replace_changes())
            .build()
            .unwrap(),
        ReportBuilder::changing(&ctx, "remove", ReportStatus::Ok)
            .op(19)
            .changes((0..250).map(|index| {
                TargetChange::file(Change {
                    size_before: Some(Size::from_bytes(10)),
                    irreversible: Some(Irreversible::JournalInactive),
                    ..Change::new(
                        format!("/srv/app/build/{index}.o").as_str(),
                        ChangeAction::Delete,
                    )
                })
            }))
            .build()
            .unwrap(),
    ];
    for report in reports {
        let text = serde_json::to_string(&report).unwrap().to_lowercase();
        for word in ["undo", "expir", "until", "deadline", "ttl", "revert"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
    }
}

/// Deterministic generator of pseudo-random numbers (xorshift64).
struct Sequence(u64);

impl Sequence {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn maybe(&mut self, bound: u64) -> Option<u64> {
        (self.below(3) != 0).then(|| self.below(bound))
    }
}

const ACTIONS: [ChangeAction; 8] = [
    ChangeAction::Create,
    ChangeAction::Modify,
    ChangeAction::Delete,
    ChangeAction::Rename,
    ChangeAction::Attributes,
    ChangeAction::Link,
    ChangeAction::Truncate,
    ChangeAction::State,
];

const ENTRIES: [EntryKind; 4] = [
    EntryKind::File,
    EntryKind::Directory,
    EntryKind::Link,
    EntryKind::Other,
];

fn random_change(sequence: &mut Sequence, index: u64) -> TargetChange {
    let action = ACTIONS[sequence.below(ACTIONS.len() as u64) as usize];
    let entry = ENTRIES[sequence.below(ENTRIES.len() as u64) as usize];
    let path = format!("/srv/p{}/f{index}", sequence.below(7));
    let change = Change {
        from: (action == ChangeAction::Rename).then(|| Path::new(format!("/srv/old/{index}"))),
        size_before: sequence.maybe(1 << 20).map(Size::from_bytes),
        size_after: sequence.maybe(1 << 20).map(Size::from_bytes),
        replacements: sequence.maybe(300),
        added: sequence.maybe(50),
        removed: sequence.maybe(50),
        ..Change::new(path.as_str(), action)
    };
    TargetChange::new(change, entry)
}

fn counted(changes: &[TargetChange], keep: impl Fn(&TargetChange) -> bool) -> Option<u64> {
    let count = changes.iter().filter(|target| keep(target)).count() as u64;
    (count > 0).then_some(count)
}

fn summed(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.fold(None, |total, value| match value {
        Some(value) => Some(total.unwrap_or(0) + value),
        None => total,
    })
}

fn reference_summary(changes: &[TargetChange]) -> Summary {
    let file = |target: &TargetChange| matches!(target.entry, EntryKind::File | EntryKind::Link);
    let directory = |target: &TargetChange| target.entry == EntryKind::Directory;
    let action =
        |target: &TargetChange, wanted: &[ChangeAction]| wanted.contains(&target.change.action);
    let deleted = |target: &&TargetChange| target.change.action == ChangeAction::Delete;
    let kept = |target: &&TargetChange| target.change.action != ChangeAction::Delete;
    let total = |field: fn(&Change) -> Option<u64>| {
        let sum: u64 = changes
            .iter()
            .filter_map(|target| field(&target.change))
            .sum();
        (sum > 0).then_some(sum)
    };
    Summary {
        files_created: counted(changes, |t| {
            action(t, &[ChangeAction::Create, ChangeAction::Link]) && file(t)
        }),
        files_modified: counted(changes, |t| {
            action(t, &[ChangeAction::Modify, ChangeAction::Truncate]) && file(t)
        }),
        files_deleted: counted(changes, |t| action(t, &[ChangeAction::Delete]) && file(t)),
        files_renamed: counted(changes, |t| action(t, &[ChangeAction::Rename])),
        directories_created: counted(changes, |t| {
            action(t, &[ChangeAction::Create]) && directory(t)
        }),
        directories_deleted: counted(changes, |t| {
            action(t, &[ChangeAction::Delete]) && directory(t)
        }),
        replacements: total(|change| change.replacements),
        lines_added: total(|change| change.added),
        lines_removed: total(|change| change.removed),
        bytes_before: summed(
            changes
                .iter()
                .filter(kept)
                .map(|t| t.change.size_before.map(Size::bytes)),
        ),
        bytes_after: summed(
            changes
                .iter()
                .filter(kept)
                .map(|t| t.change.size_after.map(Size::bytes)),
        ),
        bytes_freed: summed(
            changes
                .iter()
                .filter(deleted)
                .map(|t| t.change.size_before.map(Size::bytes)),
        ),
    }
}

#[test]
fn summary_equals_the_totals_of_the_changes() {
    let ctx = ctx();
    for seed in 1..=64_u64 {
        let mut sequence = Sequence(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let size = sequence.below(400);
        let changes: Vec<TargetChange> = (0..size)
            .map(|index| random_change(&mut sequence, index))
            .collect();
        let expected = reference_summary(&changes);
        let summary = summarize_changes(&changes);
        assert_eq!(
            summary.unwrap_or_default(),
            expected,
            "seed {seed}, {size} changes"
        );
        assert_eq!(summary.is_none(), expected == Summary::default());

        let report = ReportBuilder::changing(&ctx, "apply", ReportStatus::Ok)
            .op(1)
            .changes(changes.clone())
            .build()
            .unwrap();
        assert_eq!(report.summary, summary);
        let targets = report.targets.unwrap();
        assert_eq!(targets.count, size);
        assert_eq!(targets.files, counted(&changes, |t| t.entry.is_file()));
        assert_eq!(
            targets.directories,
            counted(&changes, |t| t.entry == EntryKind::Directory)
        );
    }
}

#[test]
fn target_bytes_use_the_size_after_or_the_deleted_size() {
    let changes = [
        TargetChange::file(Change {
            size_before: Some(Size::from_bytes(100)),
            ..Change::new("/srv/a", ChangeAction::Delete)
        }),
        TargetChange::file(Change {
            size_before: Some(Size::from_bytes(5)),
            size_after: Some(Size::from_bytes(7)),
            ..Change::new("/srv/b", ChangeAction::Modify)
        }),
        TargetChange::from(Change::new("service:nginx", ChangeAction::State)),
    ];
    let targets = count_targets(&changes);
    assert_eq!(targets.count, 3);
    assert_eq!(targets.files, Some(2));
    assert_eq!(targets.directories, None);
    assert_eq!(targets.bytes, Some(107));
    assert_eq!(count_targets(&changes[2..]).bytes, None);
}

#[test]
fn service_state_change_counts_as_a_target_without_files() {
    let ctx = ctx();
    let report = ReportBuilder::changing(&ctx, "restart", ReportStatus::Ok)
        .op(27)
        .change(Change {
            before: Some("running".to_owned()),
            after: Some("running".to_owned()),
            ..Change::new("service:nginx", ChangeAction::State)
        })
        .irreversible(IrreversibleEntry {
            path: Some("service:nginx".to_owned()),
            detail: Some("restart".to_owned()),
            ..IrreversibleEntry::new(Irreversible::ServiceChanged)
        })
        .build()
        .unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["targets"], json!({"count": 1, "listed": 1}));
    assert!(value.get("summary").is_none());
    assert_eq!(value["irreversible"][0]["reason"], "service_changed");
    assert!(Report::validate(&report).is_ok());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
