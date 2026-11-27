// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/rules_notable.rs
// Responsibility:   Thresholds, severities and ordering of the notable effects derived from.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Thresholds, severities and ordering of the notable effects derived from
//! changes.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::config::{NotableThresholds, ReportConfig, SizeSetting};
use nurt_contracts::env::Os;
use nurt_contracts::report::{
    Change, ChangeAction, Notable, NotableEntry, Report, ReportStatus, Severity,
};
use nurt_contracts::value::{Path, Size};
use nurt_report::assemble::{ReportContext, ScopeDirectory, TargetChange};
use nurt_report::rules::{
    ALERT_FACTOR, ENTRIES_PER_CODE, PathContext, SHRINK_MIN_BYTES, derive_notable, merge_notable,
};

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

fn linux() -> PathContext {
    PathContext::new(Os::Linux).with_home("/home/user")
}

fn default_ctx() -> ReportContext {
    ReportContext::new(ReportConfig::default(), linux())
}

fn ctx_with(edit: impl FnOnce(&mut NotableThresholds)) -> ReportContext {
    let mut config = ReportConfig::default();
    edit(&mut config.notable);
    ReportContext::new(config, linux())
}

fn deleted_file(path: &str, size: u64) -> TargetChange {
    TargetChange::file(Change {
        size_before: Some(Size::from_bytes(size)),
        ..Change::new(path, ChangeAction::Delete)
    })
}

fn deleted_files(count: u64) -> Vec<TargetChange> {
    (0..count)
        .map(|index| deleted_file(&format!("/srv/app/build/f{index}.o"), 10))
        .collect()
}

fn resized(path: &str, before: u64, after: u64) -> TargetChange {
    TargetChange::file(Change {
        size_before: Some(Size::from_bytes(before)),
        size_after: Some(Size::from_bytes(after)),
        ..Change::new(path, ChangeAction::Modify)
    })
}

fn replaced(path: &str, replacements: u64) -> TargetChange {
    TargetChange::file(Change {
        replacements: Some(replacements),
        ..Change::new(path, ChangeAction::Modify)
    })
}

fn derive(ctx: &ReportContext, changes: &[TargetChange]) -> Vec<NotableEntry> {
    derive_notable(ctx, changes, changes.len() as u64, &[])
}

fn entries(entries: &[NotableEntry], code: Notable) -> Vec<NotableEntry> {
    entries
        .iter()
        .filter(|entry| entry.code == code)
        .cloned()
        .collect()
}

fn single(found: &[NotableEntry], code: Notable) -> Option<NotableEntry> {
    let matching = entries(found, code);
    assert!(
        matching.len() <= 1,
        "more than one {code} entry: {matching:?}"
    );
    matching.into_iter().next()
}

fn severity(found: &[NotableEntry], code: Notable) -> Option<Severity> {
    single(found, code).map(|entry| entry.severity)
}

#[test]
fn mass_delete_by_file_count_at_default_threshold() {
    let ctx = default_ctx();
    for (files, expected) in [
        (99, None),
        (100, Some(Severity::Warning)),
        (101, Some(Severity::Warning)),
        (999, Some(Severity::Warning)),
        (1000, Some(Severity::Alert)),
        (1001, Some(Severity::Alert)),
    ] {
        let found = derive(&ctx, &deleted_files(files));
        assert_eq!(
            severity(&found, Notable::MassDelete),
            expected,
            "{files} files"
        );
        if let Some(entry) = single(&found, Notable::MassDelete) {
            assert_eq!(entry.count, Some(files));
            assert_eq!(entry.path.as_deref(), Some("/srv/app/build"));
        }
    }
}

#[test]
fn mass_delete_by_file_count_at_configured_threshold() {
    let ctx = ctx_with(|thresholds| thresholds.mass_files = 5);
    for (files, expected) in [
        (4, None),
        (5, Some(Severity::Warning)),
        (6, Some(Severity::Warning)),
        (49, Some(Severity::Warning)),
        (50, Some(Severity::Alert)),
    ] {
        let found = derive(&ctx, &deleted_files(files));
        assert_eq!(
            severity(&found, Notable::MassDelete),
            expected,
            "{files} files"
        );
    }
}

#[test]
fn mass_delete_by_bytes_at_default_threshold() {
    let ctx = default_ctx();
    for (bytes, expected) in [
        (GIB - 1, None),
        (GIB, Some(Severity::Warning)),
        (GIB + 1, Some(Severity::Warning)),
        (ALERT_FACTOR * GIB - 1, Some(Severity::Warning)),
        (ALERT_FACTOR * GIB, Some(Severity::Alert)),
    ] {
        let found = derive(&ctx, &[deleted_file("/srv/data/image.iso", bytes)]);
        assert_eq!(
            severity(&found, Notable::MassDelete),
            expected,
            "{bytes} bytes"
        );
    }
    let found = derive(&ctx, &[deleted_file("/srv/data/image.iso", GIB)]);
    let entry = single(&found, Notable::MassDelete).unwrap();
    assert_eq!(entry.path.as_deref(), Some("/srv/data/image.iso"));
    assert_eq!(entry.detail.as_deref(), Some("1 file, 1.0gib"));
}

#[test]
fn mass_delete_by_bytes_at_configured_threshold() {
    let ctx = ctx_with(|thresholds| thresholds.mass_bytes = SizeSetting::from_mib(1));
    for (bytes, expected) in [
        (MIB - 1, None),
        (MIB, Some(Severity::Warning)),
        (MIB + 1, Some(Severity::Warning)),
        (ALERT_FACTOR * MIB, Some(Severity::Alert)),
    ] {
        let found = derive(&ctx, &[deleted_file("/srv/data/a.bin", bytes)]);
        assert_eq!(
            severity(&found, Notable::MassDelete),
            expected,
            "{bytes} bytes"
        );
    }
}

fn share_of(ctx: &ReportContext, deleted: u64, held: u64) -> Option<NotableEntry> {
    let changes = deleted_files(deleted);
    let scope = [ScopeDirectory::new("/srv/app/build", held)];
    single(
        &derive_notable(ctx, &changes, deleted, &scope),
        Notable::MassDelete,
    )
}

#[test]
fn mass_delete_by_share_of_directory_at_default_ratio() {
    let ctx = default_ctx();
    assert_eq!(share_of(&ctx, 7, 10), None);
    let entry = share_of(&ctx, 8, 10).unwrap();
    assert_eq!(entry.severity, Severity::Warning);
    assert_eq!(entry.path.as_deref(), Some("/srv/app/build"));
    assert_eq!(
        entry.detail.as_deref(),
        Some("8 files, 80b, 80% of /srv/app/build")
    );
    assert_eq!(share_of(&ctx, 9, 10).unwrap().severity, Severity::Warning);
}

#[test]
fn mass_delete_by_share_of_directory_at_configured_ratio() {
    let ctx = ctx_with(|thresholds| thresholds.mass_ratio = 0.5);
    assert_eq!(share_of(&ctx, 4, 10), None);
    assert!(share_of(&ctx, 5, 10).is_some());
    assert!(share_of(&ctx, 6, 10).is_some());
}

#[test]
fn mass_delete_ignores_scope_directories_without_files() {
    let ctx = default_ctx();
    assert_eq!(share_of(&ctx, 3, 0), None);
    let changes = deleted_files(3);
    let elsewhere = [ScopeDirectory::new("/srv/other", 3)];
    assert!(
        single(
            &derive_notable(&ctx, &changes, 3, &elsewhere),
            Notable::MassDelete
        )
        .is_none()
    );
}

#[test]
fn mass_delete_describes_a_bulk_removal() {
    let ctx = default_ctx();
    let files = 18_402_u64;
    let total = 3_650_722_201_u64;
    let each = total / files;
    let mut changes: Vec<TargetChange> = (0..files)
        .map(|index| deleted_file(&format!("/srv/app/build/cache/{index:05}.o"), each))
        .collect();
    changes[0] = deleted_file("/srv/app/build/a.tmp", each + total % files);
    let scope = [ScopeDirectory::new("/srv/app/build", files)];
    let found = derive_notable(&ctx, &changes, files, &scope);
    let entry = single(&found, Notable::MassDelete).unwrap();
    assert_eq!(entry.severity, Severity::Alert);
    assert_eq!(entry.count, Some(files));
    assert_eq!(entry.path.as_deref(), Some("/srv/app/build"));
    assert_eq!(
        entry.detail.as_deref(),
        Some("18402 files, 3.4gib, 100% of /srv/app/build")
    );
}

#[test]
fn mass_delete_counts_files_but_not_directories() {
    let ctx = default_ctx();
    let directories: Vec<TargetChange> = (0..200)
        .map(|index| {
            TargetChange::directory(Change::new(
                format!("/srv/app/d{index}").as_str(),
                ChangeAction::Delete,
            ))
        })
        .collect();
    assert!(single(&derive(&ctx, &directories), Notable::MassDelete).is_none());
}

#[test]
fn file_emptied_only_for_files_that_had_content() {
    let ctx = default_ctx();
    let found = derive(&ctx, &[resized("/srv/app/a.txt", 1, 0)]);
    let entry = single(&found, Notable::FileEmptied).unwrap();
    assert_eq!(entry.severity, Severity::Warning);
    assert_eq!(entry.path.as_deref(), Some("/srv/app/a.txt"));
    assert_eq!(entry.detail.as_deref(), Some("was 1b"));
    assert!(single(&found, Notable::SizeShrunk).is_none());

    let created_empty = TargetChange::file(Change {
        size_after: Some(Size::ZERO),
        ..Change::new("/srv/app/new.txt", ChangeAction::Create)
    });
    let unchanged_empty = resized("/srv/app/b.txt", 0, 0);
    let deleted = deleted_file("/srv/app/c.txt", 10);
    let found = derive(&ctx, &[created_empty, unchanged_empty, deleted]);
    assert!(single(&found, Notable::FileEmptied).is_none());
}

#[test]
fn size_shrunk_at_default_ratio() {
    let ctx = default_ctx();
    for (before, after, expected) in [
        (2048, 1025, false),
        (2048, 1024, true),
        (2048, 1023, true),
        (SHRINK_MIN_BYTES - 1, 1, false),
        (SHRINK_MIN_BYTES, SHRINK_MIN_BYTES / 2, true),
        (SHRINK_MIN_BYTES, SHRINK_MIN_BYTES / 2 + 1, false),
        (2048, 4096, false),
    ] {
        let found = derive(&ctx, &[resized("/srv/app/log.txt", before, after)]);
        assert_eq!(
            single(&found, Notable::SizeShrunk).is_some(),
            expected,
            "{before} -> {after}"
        );
    }
    let found = derive(&ctx, &[resized("/srv/app/log.txt", 4096, 1024)]);
    let entry = single(&found, Notable::SizeShrunk).unwrap();
    assert_eq!(entry.severity, Severity::Warning);
    assert_eq!(entry.detail.as_deref(), Some("from 4.0kib to 1.0kib"));
}

#[test]
fn size_shrunk_at_configured_ratio() {
    let ctx = ctx_with(|thresholds| thresholds.shrink_ratio = 0.25);
    for (after, expected) in [(3073, false), (3072, true), (3071, true)] {
        let found = derive(&ctx, &[resized("/srv/app/log.txt", 4096, after)]);
        assert_eq!(
            single(&found, Notable::SizeShrunk).is_some(),
            expected,
            "{after}"
        );
    }
}

#[test]
fn many_per_file_at_default_threshold() {
    let ctx = default_ctx();
    for (count, expected) in [(99, false), (100, true), (101, true)] {
        let found = derive(&ctx, &[replaced("/srv/app/a.go", count)]);
        let entry = single(&found, Notable::ManyPerFile);
        assert_eq!(entry.is_some(), expected, "{count} replacements");
        if let Some(entry) = entry {
            assert_eq!(entry.severity, Severity::Notice);
            assert_eq!(entry.count, Some(count));
            assert_eq!(entry.path.as_deref(), Some("/srv/app/a.go"));
        }
    }
}

#[test]
fn many_per_file_at_configured_threshold() {
    let ctx = ctx_with(|thresholds| thresholds.per_file_repl = 3);
    for (count, expected) in [(2, false), (3, true), (4, true), (30, true)] {
        let found = derive(&ctx, &[replaced("/srv/app/a.go", count)]);
        assert_eq!(single(&found, Notable::ManyPerFile).is_some(), expected);
    }
    let ctx = ctx_with(|thresholds| thresholds.per_file_repl = 0);
    let found = derive(&ctx, &[replaced("/srv/app/a.go", 0)]);
    assert!(single(&found, Notable::ManyPerFile).is_none());
}

#[test]
fn target_count_high_at_default_and_configured_threshold() {
    let ctx = default_ctx();
    for (targets, expected) in [(499, false), (500, true), (501, true)] {
        let found = derive_notable(&ctx, &[], targets, &[]);
        let entry = single(&found, Notable::TargetCountHigh);
        assert_eq!(entry.is_some(), expected, "{targets} targets");
        if let Some(entry) = entry {
            assert_eq!(entry.severity, Severity::Notice);
            assert_eq!(entry.count, Some(targets));
        }
    }
    let ctx = ctx_with(|thresholds| thresholds.targets_high = 10);
    for (targets, expected) in [(9, false), (10, true), (11, true)] {
        let found = derive_notable(&ctx, &[], targets, &[]);
        assert_eq!(single(&found, Notable::TargetCountHigh).is_some(), expected);
    }
    let ctx = ctx_with(|thresholds| thresholds.targets_high = 0);
    assert!(derive_notable(&ctx, &[], 0, &[]).is_empty());
}

fn deleted_directory(path: &str) -> TargetChange {
    TargetChange::directory(Change::new(path, ChangeAction::Delete))
}

#[test]
fn delete_tree_reports_non_empty_directories_only() {
    let ctx = default_ctx();
    let found = derive(
        &ctx,
        &[
            deleted_directory("/srv/app/full").with_entries_inside(3),
            deleted_directory("/srv/app/empty").with_entries_inside(0),
            deleted_directory("/srv/app/unknown"),
        ],
    );
    let trees = entries(&found, Notable::DeleteTree);
    assert_eq!(trees.len(), 1);
    assert_eq!(trees[0].severity, Severity::Notice);
    assert_eq!(trees[0].path.as_deref(), Some("/srv/app/full"));
    assert_eq!(trees[0].count, Some(3));
}

#[test]
fn delete_tree_reports_the_topmost_directory_of_listed_entries() {
    let ctx = default_ctx();
    let found = derive(
        &ctx,
        &[
            deleted_directory("/srv/app/build"),
            deleted_directory("/srv/app/build/cache"),
            deleted_file("/srv/app/build/cache/1.o", 10),
            deleted_file("/srv/app/build/a.tmp", 10),
            deleted_directory("/srv/app/other"),
            deleted_file("/srv/app/other/x", 1),
        ],
    );
    let trees = entries(&found, Notable::DeleteTree);
    let paths: Vec<(Option<&str>, Option<u64>)> = trees
        .iter()
        .map(|entry| (entry.path.as_deref(), entry.count))
        .collect();
    assert_eq!(
        paths,
        vec![
            (Some("/srv/app/build"), Some(3)),
            (Some("/srv/app/other"), Some(1))
        ]
    );
}

#[test]
fn overwrote_existing_follows_the_change_fact() {
    let ctx = default_ctx();
    let overwritten = TargetChange::file(Change::new("/srv/app/a.txt", ChangeAction::Modify))
        .replacing_existing();
    let plain = TargetChange::file(Change::new("/srv/app/b.txt", ChangeAction::Modify));
    let found = derive(&ctx, &[overwritten, plain]);
    let entry = single(&found, Notable::OverwroteExisting).unwrap();
    assert_eq!(entry.severity, Severity::Warning);
    assert_eq!(entry.path.as_deref(), Some("/srv/app/a.txt"));
}

#[test]
fn outside_project_needs_configured_roots() {
    let change = || TargetChange::file(Change::new("/srv/other/a.txt", ChangeAction::Modify));
    assert!(
        single(
            &derive(&default_ctx(), &[change()]),
            Notable::OutsideProject
        )
        .is_none()
    );

    let ctx = default_ctx().with_project_roots(["/srv/app/", "/srv/lib"]);
    let found = derive(&ctx, &[change()]);
    let entry = single(&found, Notable::OutsideProject).unwrap();
    assert_eq!(entry.severity, Severity::Alert);
    assert_eq!(entry.path.as_deref(), Some("/srv/other/a.txt"));

    let inside = [
        TargetChange::file(Change::new("/srv/app/a.txt", ChangeAction::Modify)),
        TargetChange::file(Change::new("/srv/lib", ChangeAction::Attributes)),
        TargetChange::from(Change::new("service:nginx", ChangeAction::State)),
    ];
    assert!(single(&derive(&ctx, &inside), Notable::OutsideProject).is_none());
}

#[test]
fn outside_project_checks_the_previous_and_new_path_of_a_move() {
    let ctx = default_ctx().with_project_roots(["/srv/app"]);
    let moved_in = TargetChange::file(Change {
        from: Some(Path::new("/tmp/report.txt")),
        ..Change::new("/srv/app/report.txt", ChangeAction::Rename)
    });
    let found = derive(&ctx, &[moved_in]);
    assert_eq!(
        single(&found, Notable::OutsideProject)
            .unwrap()
            .path
            .as_deref(),
        Some("/tmp/report.txt")
    );
    let copied_out = TargetChange::file(Change {
        to: Some(Path::new("/mnt/backup/report.txt")),
        ..Change::new("/srv/app/report.txt", ChangeAction::Create)
    });
    let found = derive(&ctx, &[copied_out]);
    assert_eq!(
        single(&found, Notable::OutsideProject)
            .unwrap()
            .path
            .as_deref(),
        Some("/mnt/backup/report.txt")
    );
}

#[test]
fn outside_project_compares_windows_roots_without_case() {
    let ctx = ReportContext::new(
        ReportConfig::default(),
        PathContext::new(Os::Windows).with_home("C:/Users/user"),
    )
    .with_project_roots(["C:\\Src\\App\\"]);
    let inside = TargetChange::file(Change::new("c:/src/app/main.rs", ChangeAction::Modify));
    let outside = TargetChange::file(Change::new("C:/Src/Other/main.rs", ChangeAction::Modify));
    let found = derive(&ctx, &[inside, outside]);
    let entries = entries(&found, Notable::OutsideProject);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.as_deref(), Some("C:/Src/Other/main.rs"));
}

#[test]
fn system_path_and_home_config_are_derived_from_changes() {
    let ctx = default_ctx();
    let found = derive(
        &ctx,
        &[
            TargetChange::file(Change::new("/etc/hosts", ChangeAction::Modify)),
            TargetChange::file(Change::new("/home/user/.ssh/config", ChangeAction::Modify)),
            TargetChange::file(Change::new("/home/user/notes.txt", ChangeAction::Modify)),
        ],
    );
    let system = single(&found, Notable::SystemPath).unwrap();
    assert_eq!(system.severity, Severity::Alert);
    assert_eq!(system.path.as_deref(), Some("/etc/hosts"));
    let home = single(&found, Notable::HomeConfig).unwrap();
    assert_eq!(home.severity, Severity::Warning);
    assert_eq!(home.path.as_deref(), Some("/home/user/.ssh/config"));
}

#[test]
fn many_targets_of_one_code_become_one_entry() {
    let ctx = default_ctx();
    let listed: Vec<TargetChange> = (0..ENTRIES_PER_CODE)
        .map(|index| resized(&format!("/srv/app/data/{index}.csv"), 100, 0))
        .collect();
    assert_eq!(
        entries(&derive(&ctx, &listed), Notable::FileEmptied).len(),
        ENTRIES_PER_CODE
    );

    let many: Vec<TargetChange> = (0..=ENTRIES_PER_CODE)
        .map(|index| resized(&format!("/srv/app/data/{index}.csv"), 100, 0))
        .collect();
    let found = derive(&ctx, &many);
    let entry = single(&found, Notable::FileEmptied).unwrap();
    let total = ENTRIES_PER_CODE as u64 + 1;
    assert_eq!(entry.count, Some(total));
    assert_eq!(entry.path.as_deref(), Some("/srv/app/data"));
    assert_eq!(entry.detail, Some(format!("{total} targets")));
}

#[test]
fn many_targets_without_common_directory_have_no_path() {
    let ctx = default_ctx();
    let many: Vec<TargetChange> = (0..=ENTRIES_PER_CODE)
        .map(|index| resized(&format!("/srv{index}/a.csv"), 100, 0))
        .collect();
    let entry = single(&derive(&ctx, &many), Notable::FileEmptied).unwrap();
    assert_eq!(entry.path, None);
}

fn emitted(code: Notable, path: Option<&str>) -> NotableEntry {
    NotableEntry {
        path: path.map(str::to_owned),
        ..NotableEntry::new(code)
    }
}

#[test]
fn merge_orders_by_severity_then_code_path_and_detail() {
    let derived = vec![
        emitted(Notable::ManyPerFile, Some("/srv/app/c.go")),
        emitted(Notable::SystemPath, Some("/etc/hosts")),
        emitted(Notable::FileEmptied, Some("/srv/app/b.txt")),
    ];
    let commands = vec![
        emitted(Notable::LineEndingPreserved, Some("/srv/app/c.go")),
        emitted(Notable::EncodingPreserved, Some("/srv/app/c.go")),
        emitted(Notable::BinaryTarget, Some("/srv/app/a.bin")),
        emitted(Notable::FileEmptied, Some("/srv/app/a.txt")),
    ];
    let merged = merge_notable(derived.clone(), commands.clone());
    let order: Vec<(Notable, Option<&str>)> = merged
        .iter()
        .map(|entry| (entry.code, entry.path.as_deref()))
        .collect();
    assert_eq!(
        order,
        vec![
            (Notable::SystemPath, Some("/etc/hosts")),
            (Notable::FileEmptied, Some("/srv/app/a.txt")),
            (Notable::FileEmptied, Some("/srv/app/b.txt")),
            (Notable::BinaryTarget, Some("/srv/app/a.bin")),
            (Notable::ManyPerFile, Some("/srv/app/c.go")),
            (Notable::EncodingPreserved, Some("/srv/app/c.go")),
            (Notable::LineEndingPreserved, Some("/srv/app/c.go")),
        ]
    );
    let reversed = merge_notable(
        derived.into_iter().rev().collect(),
        commands.into_iter().rev(),
    );
    assert_eq!(reversed, merged);

    let mut report = Report::new(ReportStatus::Ok, "replace");
    report.notable = merged;
    assert!(report.validate().is_ok());
}

#[test]
fn merge_prefers_derived_entries_and_removes_duplicates() {
    let derived = vec![NotableEntry {
        count: Some(120),
        ..emitted(Notable::ManyPerFile, Some("/srv/app/a.go"))
    }];
    let commands = vec![
        NotableEntry {
            count: Some(3),
            ..emitted(Notable::ManyPerFile, Some("/srv/app/a.go"))
        },
        NotableEntry {
            count: Some(2),
            ..emitted(Notable::BinarySkipped, None)
        },
        NotableEntry {
            count: Some(5),
            severity: Severity::Warning,
            ..emitted(Notable::BinarySkipped, None)
        },
        emitted(Notable::NothingMatched, None),
        emitted(Notable::NothingMatched, None),
    ];
    let merged = merge_notable(derived, commands);
    assert_eq!(merged.len(), 3);
    let many = single(&merged, Notable::ManyPerFile).unwrap();
    assert_eq!(many.count, Some(120));
    let skipped = single(&merged, Notable::BinarySkipped).unwrap();
    assert_eq!(skipped.count, Some(7));
    assert_eq!(skipped.severity, Severity::Warning);
    let nothing = single(&merged, Notable::NothingMatched).unwrap();
    assert_eq!(nothing.count, None);
}

#[test]
fn derived_entries_use_the_default_severity_of_their_code() {
    let ctx = default_ctx().with_project_roots(["/srv/app"]);
    let changes = [
        resized("/etc/app.conf", 4096, 0),
        replaced("/home/user/.config/app.toml", 100),
        TargetChange::file(Change::new("/srv/other/a", ChangeAction::Modify)).replacing_existing(),
    ];
    for entry in derive(&ctx, &changes) {
        assert_eq!(entry.severity, entry.code.default_severity(), "{entry:?}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
