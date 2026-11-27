// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/assemble_bulk.rs
// Responsibility:   Bulk summaries: threshold, ranking of directories and extensions.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Bulk summaries: threshold, ranking of directories and extensions,
//! samples and determinism on large operations.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cmp::Reverse;
use std::collections::HashMap;

use nurt_contracts::config::ReportConfig;
use nurt_contracts::env::Os;
use nurt_contracts::report::{
    Bulk, Change, ChangeAction, Notable, Report, ReportStatus, Severity, Summary, Targets,
};
use nurt_contracts::value::Size;
use nurt_report::assemble::{
    BULK_GROUPS, ReportBuilder, ReportContext, SAMPLE_HEAD, SAMPLE_TAIL, ScopeDirectory,
    TargetChange, summarize_bulk,
};
use nurt_report::rules::PathContext;

fn ctx_with_threshold(threshold: u32) -> ReportContext {
    let mut config = ReportConfig::default();
    config.mass_threshold = threshold;
    ReportContext::new(config, PathContext::new(Os::Linux))
}

fn deleted(path: &str, size: u64) -> TargetChange {
    TargetChange::file(Change {
        size_before: Some(Size::from_bytes(size)),
        ..Change::new(path, ChangeAction::Delete)
    })
}

fn deleted_directory(path: &str) -> TargetChange {
    TargetChange::directory(Change::new(path, ChangeAction::Delete))
}

fn modified_files(count: usize) -> Vec<TargetChange> {
    (0..count)
        .map(|index| {
            TargetChange::file(Change::new(
                format!("/srv/app/src/{index}.go").as_str(),
                ChangeAction::Modify,
            ))
        })
        .collect()
}

fn has_bulk(ctx: &ReportContext, count: usize) -> bool {
    ReportBuilder::changing(ctx, "replace", ReportStatus::Ok)
        .op(1)
        .changes(modified_files(count))
        .build()
        .unwrap()
        .bulk
        .is_some()
}

#[test]
fn bulk_summary_starts_at_the_mass_threshold() {
    let ctx = ctx_with_threshold(ReportConfig::default().mass_threshold);
    assert!(!has_bulk(&ctx, 199));
    assert!(has_bulk(&ctx, 200));
    assert!(has_bulk(&ctx, 201));
    let ctx = ctx_with_threshold(3);
    assert!(!has_bulk(&ctx, 2));
    assert!(has_bulk(&ctx, 3));
    assert!(has_bulk(&ctx, 4));
    let explicit = ReportBuilder::changing(&ctx, "replace", ReportStatus::Ok)
        .targets(Targets {
            count: 500,
            ..Targets::default()
        })
        .build()
        .unwrap();
    assert!(explicit.bulk.is_none());
}

/// Removal of `/srv/app/build`: 18 402 files in three directories, 1 206
/// empty subdirectories and the directory itself, children before parents.
fn build_removal() -> Vec<TargetChange> {
    let mut changes = Vec::new();
    for (directory, files, extension, size) in [
        ("/srv/app/build/cache", 15_880, ".o", 196_087),
        ("/srv/app/build/obj", 2_100, ".o", 191_739),
        ("/srv/app/build/tmp", 422, ".tmp", 318_052),
    ] {
        for index in 0..files {
            changes.push(deleted(&format!("{directory}/{index:05}{extension}"), size));
        }
    }
    for index in 0..1_206 {
        changes.push(deleted_directory(&format!(
            "/srv/app/build/cache/sub{index:04}"
        )));
    }
    changes.push(deleted_directory("/srv/app/build"));
    changes
}

fn removal_report(ctx: &ReportContext, changes: Vec<TargetChange>) -> Report {
    ReportBuilder::changing(ctx, "remove", ReportStatus::Ok)
        .op(19)
        .changes(changes)
        .scope_directory(ScopeDirectory::new("/srv/app/build", 18_402))
        .list_limit(0)
        .next_cursor("c_19aa")
        .build()
        .unwrap()
}

#[test]
fn bulk_removal_report_summarizes_eighteen_thousand_files() {
    let ctx = ctx_with_threshold(ReportConfig::default().mass_threshold);
    let changes = build_removal();
    let report = removal_report(&ctx, changes.clone());
    let bytes = 15_880 * 196_087 + 2_100 * 191_739 + 422 * 318_052;
    assert_eq!(
        report.targets,
        Some(Targets {
            count: 19_609,
            files: Some(18_402),
            directories: Some(1_207),
            bytes: Some(bytes),
            listed: None,
        })
    );
    assert_eq!(
        report.summary,
        Some(Summary {
            files_deleted: Some(18_402),
            directories_deleted: Some(1_207),
            bytes_freed: Some(bytes),
            ..Summary::default()
        })
    );
    assert_eq!(report.op, Some(19));
    assert!(report.changes.is_empty());
    assert_removal_groups(report.bulk.as_ref().unwrap());
    assert_removal_notable(&report);
    assert_eq!(removal_report(&ctx, changes), report);
}

fn assert_removal_groups(bulk: &Bulk) {
    let directories: Vec<(&str, u64)> = bulk
        .by_directory
        .iter()
        .map(|group| (group.directory.as_str(), group.count))
        .collect();
    assert_eq!(
        directories,
        vec![
            ("/srv/app/build/cache", 15_880),
            ("/srv/app/build/obj", 2_100),
            ("/srv/app/build/tmp", 422),
        ]
    );
    assert_eq!(bulk.by_directory[0].bytes, Some(15_880 * 196_087));
    let extensions: Vec<(&str, u64)> = bulk
        .by_extension
        .iter()
        .map(|group| (group.extension.as_str(), group.count))
        .collect();
    assert_eq!(extensions, vec![(".o", 17_980), (".tmp", 422)]);
    assert_eq!(bulk.sample_head.len(), SAMPLE_HEAD);
    assert_eq!(bulk.sample_head[0], "/srv/app/build/cache/00000.o");
    assert_eq!(
        bulk.sample_tail,
        vec!["/srv/app/build/cache/sub1205", "/srv/app/build"]
    );
}

fn assert_removal_notable(report: &Report) {
    let codes: Vec<(Notable, Severity)> = report
        .notable
        .iter()
        .map(|entry| (entry.code, entry.severity))
        .collect();
    assert_eq!(
        codes,
        vec![
            (Notable::MassDelete, Severity::Alert),
            (Notable::DeleteTree, Severity::Notice),
            (Notable::TargetCountHigh, Severity::Notice),
        ]
    );
    assert_eq!(report.notable[1].path.as_deref(), Some("/srv/app/build"));
    assert_eq!(report.notable[1].count, Some(19_608));
}

/// One hundred thousand files spread over 523 directories and 37 extensions
/// with uneven sizes.
fn large_operation() -> Vec<TargetChange> {
    (0..100_000_u64)
        .map(|index| {
            let directory = (index * index) % 523;
            let extension = (index / 7) % 37;
            let size = (index * 7_919) % 10_007;
            TargetChange::file(Change {
                size_after: Some(Size::from_bytes(size)),
                ..Change::new(
                    format!("/data/d{directory:03}/f{index}.e{extension:02}").as_str(),
                    ChangeAction::Create,
                )
            })
        })
        .collect()
}

type Ranked = Vec<(String, u64, u64)>;

fn reference_ranking(changes: &[TargetChange], key: fn(&str) -> String) -> Ranked {
    let mut groups: HashMap<String, (u64, u64)> = HashMap::new();
    for target in changes {
        let group = groups.entry(key(target.change.path.as_str())).or_default();
        group.0 += 1;
        group.1 += target.change.size_after.map_or(0, Size::bytes);
    }
    let mut ranked: Ranked = groups
        .into_iter()
        .map(|(name, (count, bytes))| (name, count, bytes))
        .collect();
    ranked.sort_by_key(|(name, count, bytes)| (Reverse(*count), Reverse(*bytes), name.clone()));
    ranked.truncate(BULK_GROUPS);
    ranked
}

fn directory_of(path: &str) -> String {
    path.rsplit_once('/').unwrap().0.to_owned()
}

fn extension_of(path: &str) -> String {
    path[path.rfind('.').unwrap()..].to_owned()
}

#[test]
fn bulk_of_one_hundred_thousand_targets_matches_a_full_ranking() {
    let changes = large_operation();
    let bulk = summarize_bulk(&changes);
    let directories: Ranked = bulk
        .by_directory
        .iter()
        .map(|group| (group.directory.clone(), group.count, group.bytes.unwrap()))
        .collect();
    assert_eq!(directories, reference_ranking(&changes, directory_of));
    let extensions: Ranked = bulk
        .by_extension
        .iter()
        .map(|group| (group.extension.clone(), group.count, group.bytes.unwrap()))
        .collect();
    assert_eq!(extensions, reference_ranking(&changes, extension_of));
    assert_eq!(bulk.sample_head.len(), SAMPLE_HEAD);
    assert_eq!(bulk.sample_tail.len(), SAMPLE_TAIL);
    assert_eq!(bulk.sample_tail[1], changes[99_999].change.path.as_str());

    let mut reversed = changes.clone();
    reversed.reverse();
    let other = summarize_bulk(&reversed);
    assert_eq!(other.by_directory, bulk.by_directory);
    assert_eq!(other.by_extension, bulk.by_extension);
    assert_eq!(summarize_bulk(&changes), bulk);
}

#[test]
fn ties_are_broken_by_bytes_then_by_name() {
    let bulk = summarize_bulk(&[
        deleted("/b/y.txt", 5),
        deleted("/a/x.txt", 5),
        deleted("/c/z.md", 10),
        TargetChange::file(Change::new("/d/no-size", ChangeAction::Modify)),
    ]);
    let directories: Vec<(&str, u64, Option<u64>)> = bulk
        .by_directory
        .iter()
        .map(|group| (group.directory.as_str(), group.count, group.bytes))
        .collect();
    assert_eq!(
        directories,
        vec![
            ("/c", 1, Some(10)),
            ("/a", 1, Some(5)),
            ("/b", 1, Some(5)),
            ("/d", 1, None),
        ]
    );
    let extensions: Vec<(&str, u64)> = bulk
        .by_extension
        .iter()
        .map(|group| (group.extension.as_str(), group.count))
        .collect();
    assert_eq!(extensions, vec![(".txt", 2), (".md", 1), ("", 1)]);
}

#[test]
fn only_the_largest_groups_are_kept() {
    let changes: Vec<TargetChange> = (0..15_u64)
        .flat_map(|directory| {
            (0..=directory).map(move |index| deleted(&format!("/srv/d{directory:02}/{index}"), 1))
        })
        .collect();
    let bulk = summarize_bulk(&changes);
    assert_eq!(bulk.by_directory.len(), BULK_GROUPS);
    assert_eq!(bulk.by_directory[0].directory, "/srv/d14");
    assert_eq!(bulk.by_directory[0].count, 15);
    assert_eq!(bulk.by_directory[BULK_GROUPS - 1].directory, "/srv/d05");
}

#[test]
fn samples_do_not_repeat_targets() {
    let paths = |count: usize| -> Vec<TargetChange> {
        (0..count)
            .map(|index| deleted(&format!("/srv/{index}"), 1))
            .collect()
    };
    for (count, head, tail) in [
        (0, 0, 0),
        (3, 3, 0),
        (5, 5, 0),
        (6, 5, 1),
        (7, 5, 2),
        (8, 5, 2),
    ] {
        let bulk = summarize_bulk(&paths(count));
        assert_eq!(bulk.sample_head.len(), head, "{count} targets");
        assert_eq!(bulk.sample_tail.len(), tail, "{count} targets");
        if count > 0 {
            let last = format!("/srv/{}", count - 1);
            let shown = bulk.sample_tail.last().or(bulk.sample_head.last()).unwrap();
            assert_eq!(*shown, last);
        }
    }
}

#[test]
fn directories_and_provider_targets_are_not_grouped() {
    let bulk = summarize_bulk(&[
        deleted_directory("/srv/app/cache"),
        TargetChange::from(Change::new("service:nginx", ChangeAction::State)),
        deleted("/srv/app/a.log", 3),
    ]);
    assert_eq!(bulk.by_directory.len(), 1);
    assert_eq!(bulk.by_directory[0].directory, "/srv/app");
    assert_eq!(bulk.by_extension.len(), 1);
    assert_eq!(bulk.sample_head.len(), 3);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
