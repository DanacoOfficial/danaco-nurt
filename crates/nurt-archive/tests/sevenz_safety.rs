// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-archive/tests/sevenz_safety.rs
// Responsibility:   Containment and decompression limits for 7z archives: members that.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Containment and decompression limits for 7z archives: members that
//! would leave the destination are rejected by the extraction plan and
//! never reach the content stream, and declared sizes above the limits fail
//! with `E_LIMIT`.

// Helper functions outside `#[test]` functions build fixtures; an
// unexpected error there is a test failure, so they unwrap.
#![allow(
    clippy::unwrap_used,
    reason = "fixture helpers fail the test on unexpected errors"
)]

use std::io::Cursor;

use nurt_archive::extract::{ExtractPlan, Limits, plan, stream};
use nurt_archive::list::{ArchiveError, LimitKind};
use nurt_archive::safety::{Rules, Violation};
use nurt_archive::sevenz::{Creator, Metadata};
use nurt_contracts::error::ErrorCode;

/// Member of a hand-built archive, stored without path checks.
enum Fixture<'a> {
    File(&'a str, &'a [u8]),
    Directory(&'a str),
    Symlink(&'a str, &'a str),
}

fn archive(fixtures: &[Fixture<'_>]) -> Vec<u8> {
    let mut creator = Creator::new(Cursor::new(Vec::new())).unwrap();
    for fixture in fixtures {
        match fixture {
            Fixture::File(path, content) => {
                creator
                    .add_file(path, Metadata::default(), *content)
                    .unwrap();
            }
            Fixture::Directory(path) => creator.add_directory(path, Metadata::default()).unwrap(),
            Fixture::Symlink(path, target) => creator
                .add_symlink(path, target, Metadata::default())
                .unwrap(),
        }
    }
    creator.finish().unwrap().into_inner()
}

fn streamed(archive: &[u8], plan: &ExtractPlan) -> Vec<(String, Vec<u8>)> {
    let mut written = Vec::new();
    stream(Cursor::new(archive), plan, |entry, content| {
        let mut bytes = Vec::new();
        content.read_to_end(&mut bytes)?;
        written.push((entry.dest.clone(), bytes));
        Ok(())
    })
    .unwrap();
    written
}

#[test]
fn parent_absolute_drive_and_backslash_paths_are_rejected() {
    let bytes = archive(&[
        Fixture::File("../evil.txt", b"x"),
        Fixture::File("/etc/evil.txt", b"x"),
        Fixture::File("safe/../../evil.txt", b"x"),
        Fixture::File("C:/evil.txt", b"x"),
        Fixture::File("dir\\evil.txt", b"x"),
        Fixture::File("ok.txt", b"fine"),
    ]);
    let plan = plan(Cursor::new(&bytes), Rules::default(), &Limits::default()).unwrap();
    let violations: Vec<(u64, Violation)> = plan
        .rejected
        .iter()
        .map(|rejected| (rejected.ordinal, rejected.violation))
        .collect();
    assert_eq!(
        violations,
        [
            (0, Violation::ParentComponent),
            (1, Violation::Absolute),
            (2, Violation::ParentComponent),
            (3, Violation::DriveLetter),
            (4, Violation::Backslash),
        ]
    );
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(
        streamed(&bytes, &plan),
        [("ok.txt".to_owned(), b"fine".to_vec())]
    );
}

#[test]
fn links_leaving_the_destination_and_paths_through_links_are_rejected() {
    let bytes = archive(&[
        Fixture::Directory("pkg"),
        Fixture::Symlink("pkg/up", "../.."),
        Fixture::Symlink("pkg/inside", "data"),
        Fixture::File("pkg/inside/escape.txt", b"x"),
        Fixture::Symlink("pkg/root", "/etc"),
        Fixture::File("pkg/data.txt", b"kept"),
    ]);
    let plan = plan(Cursor::new(&bytes), Rules::default(), &Limits::default()).unwrap();
    let violations: Vec<(&str, Violation)> = plan
        .rejected
        .iter()
        .map(|rejected| (rejected.path.as_str(), rejected.violation))
        .collect();
    assert_eq!(
        violations,
        [
            ("pkg/up", Violation::LinkOutside),
            ("pkg/inside/escape.txt", Violation::ThroughLink),
            ("pkg/root", Violation::LinkOutside),
        ]
    );
    let dests: Vec<&str> = plan
        .entries
        .iter()
        .map(|entry| entry.dest.as_str())
        .collect();
    assert_eq!(dests, ["pkg", "pkg/inside", "pkg/data.txt"]);
    assert_eq!(
        streamed(&bytes, &plan),
        [("pkg/data.txt".to_owned(), b"kept".to_vec())]
    );
}

#[test]
fn allow_outside_keeps_the_stored_path_and_records_the_waiver() {
    let bytes = archive(&[Fixture::File("../evil.txt", b"x")]);
    let rules = Rules {
        allow_outside: true,
        ..Rules::default()
    };
    let plan = plan(Cursor::new(&bytes), rules, &Limits::default()).unwrap();
    assert!(plan.rejected.is_empty());
    assert_eq!(plan.entries[0].dest, "../evil.txt");
    assert_eq!(plan.entries[0].waived, Some(Violation::ParentComponent));
}

#[test]
fn declared_size_above_the_limit_fails_before_streaming() {
    let bytes = archive(&[Fixture::File("big.bin", &[7; 5_000])]);
    let limits = Limits {
        max_size: 4_999,
        ..Limits::default()
    };
    let error = plan(Cursor::new(&bytes), Rules::default(), &limits).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit);
    assert!(matches!(
        error,
        ArchiveError::Limit {
            limit: LimitKind::Size,
            allowed: 4_999,
            actual: 5_000,
        }
    ));
}

#[test]
fn compression_bomb_fails_on_the_ratio_limit() {
    let zeros = vec![0_u8; 4 << 20];
    let bytes = archive(&[Fixture::File("zeros.bin", &zeros)]);
    let limits = Limits {
        max_ratio: 100,
        ratio_floor: 0,
        ..Limits::default()
    };
    let error = plan(Cursor::new(&bytes), Rules::default(), &limits).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit);
    assert!(
        matches!(
            error,
            ArchiveError::Limit {
                limit: LimitKind::Ratio,
                allowed: 100,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn symbolic_link_with_an_overlong_target_is_a_format_error() {
    let target = "a/".repeat(3_000);
    let bytes = archive(&[Fixture::Symlink("link", &target)]);
    let error = plan(Cursor::new(&bytes), Rules::default(), &Limits::default()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Format);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
