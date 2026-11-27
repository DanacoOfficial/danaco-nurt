// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/archive_list_golden.rs
// Responsibility:   The examples of the `archive.list` page of the command documentation, run on the virtual file system.
// Dependencies:     archive_support, nurt_archive, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The examples of the `archive.list` page of the command documentation,
//! run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod archive_support;

use archive_support::{abs, args, build, files, list_archive, path};
use nurt_archive::codec::Algorithm;
use nurt_archive::create::Input;
use nurt_archive::list::Format;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::EntryType;
use nurt_contracts::value::{Size, Value};
use nurt_testkit::ctx::TestContext;

fn paths(entries: &[nurt_contracts::record::ArchiveEntry]) -> Vec<&str> {
    entries
        .iter()
        .map(|entry| entry.path.as_deref().unwrap_or_default())
        .collect()
}

#[test]
fn example_lists_the_members_of_a_zip_archive() {
    let test = TestContext::new();
    let archive = build(
        Format::Zip,
        vec![
            Input::file("index.html", 13).with_mode(0o644),
            Input::directory("css").with_mode(0o755),
            Input::file("css/site.css", 4).with_mode(0o644),
        ],
        &[("index.html", b"<html></html>"), ("css/site.css", b"a{ }")],
    );
    test.fs()
        .write_file(&abs("/home/user/dl/site.zip"), &archive)
        .expect("the archive is stored");

    let entries = list_archive(
        &test,
        &args("archive.list", &[("path", path("/home/user/dl/site.zip"))]),
    )
    .expect("the example runs");
    assert_eq!(paths(&entries), ["css", "css/site.css", "index.html"]);
    assert_eq!(entries[0].kind, EntryType::Directory);
    assert_eq!(entries[2].kind, EntryType::File);
    assert_eq!(entries[2].size, Size::from_bytes(13));
    assert_eq!(entries[2].mode.as_deref(), Some("0644"));
    assert_eq!(entries[2].format.as_deref(), Some("zip"));
}

#[test]
fn example_lists_only_matching_members() {
    let test = TestContext::new();
    let archive = files(
        Format::Tar(Some(Algorithm::Gzip)),
        &[
            ("README.md", b"# src\n"),
            ("src/lib.rs", b"pub fn f() {}\n"),
            ("src/main.rs", b"fn main() {}\n"),
        ],
    );
    test.fs()
        .write_file(&abs("/home/user/dl/src.tar.gz"), &archive)
        .expect("the archive is stored");

    let entries = list_archive(
        &test,
        &args(
            "archive.list",
            &[
                ("path", path("/home/user/dl/src.tar.gz")),
                ("glob", Value::Glob("*.rs".to_owned())),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(paths(&entries), ["src/lib.rs", "src/main.rs"]);
    assert!(
        entries
            .iter()
            .all(|entry| entry.format.as_deref() == Some("tar_gz"))
    );
}

#[test]
fn example_recognises_the_format_from_the_content() {
    let test = TestContext::new();
    let archive = files(
        Format::Tar(Some(Algorithm::Zstd)),
        &[("db/dump.sql", b"select 1;\n")],
    );
    test.fs()
        .write_file(&abs("/home/user/dl/backup.bin"), &archive)
        .expect("the archive is stored");

    let entries = list_archive(
        &test,
        &args(
            "archive.list",
            &[("path", path("/home/user/dl/backup.bin"))],
        ),
    )
    .expect("the example runs");
    assert_eq!(paths(&entries), ["db/dump.sql"]);
    assert_eq!(entries[0].format.as_deref(), Some("tar_zst"));
    assert_eq!(entries[0].name, "dump.sql");
}

#[test]
fn names_are_reported_in_composed_form() {
    let test = TestContext::new();
    let decomposed = "zo\u{301}\u{142}w.txt";
    let archive = files(Format::Zip, &[(decomposed, b"x")]);
    test.fs()
        .write_file(&abs("/data/names.zip"), &archive)
        .expect("the archive is stored");
    let entries = list_archive(
        &test,
        &args("archive.list", &[("path", path("/data/names.zip"))]),
    )
    .expect("the listing runs");
    assert_eq!(entries[0].name, "z\u{f3}\u{142}w.txt");
}

#[test]
fn files_that_are_not_archives_fail_with_format() {
    let test = TestContext::new();
    test.fs()
        .write_file(&abs("/data/report.zip"), b"%PDF-1.7")
        .expect("the file is stored");
    let error = list_archive(
        &test,
        &args("archive.list", &[("path", path("/data/report.zip"))]),
    )
    .expect_err("the content is not an archive");
    assert_eq!(error.code(), ErrorCode::Format);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
