// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/diff.md`, run on the virtual file system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::{Hunk, HunkStatus};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, ExecCtx, RecordStream, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

fn diff_stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "diff")
        .expect("diff is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn language(text: &str) -> String {
    abs(text).to_language_string()
}

fn with_files(files: &[(&str, &[u8])]) -> TestContext {
    let test = TestContext::new();
    for (path, content) in files {
        test.fs().write_file(&abs(path), content).unwrap();
    }
    test
}

fn args(params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new("diff");
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn path(text: &str) -> Value {
    Value::Path(language(text))
}

fn run(ctx: &ExecCtx, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Hunk>> {
    let stage = diff_stage();
    let output = stage
        .as_read()
        .expect("diff is a read stage")
        .execute(args, input, ctx)?;
    let StageData::Records(stream) = output.data else {
        panic!("expected hunk records");
    };
    stream
        .map(|item| item.map(|record| Hunk::try_from(record).unwrap()))
        .collect()
}

fn entries(paths: &[&str]) -> StageInput<'static> {
    let records = paths
        .iter()
        .map(|item| Record::new(RecordKind::Entry).with("path", path(item)))
        .collect();
    StageInput::records(RecordStream::from_records(records))
}

/// Example `diff "/srv/app/config.old.toml" "/srv/app/config.toml"` returns
/// one hunk with three lines of context.
#[test]
fn compares_two_files() {
    let test = with_files(&[
        (
            "/srv/app/config.old.toml",
            b"[server]\ntimeout = 30\nport = 8080\n",
        ),
        (
            "/srv/app/config.toml",
            b"[server]\ntimeout = 60\nport = 8080\n",
        ),
    ]);
    let given = args(&[
        ("from", path("/srv/app/config.old.toml")),
        ("to", path("/srv/app/config.toml")),
    ]);
    let hunks = run(&test.ctx(), &given, StageInput::none()).unwrap();
    assert_eq!(hunks.len(), 1);
    let hunk = &hunks[0];
    assert_eq!(
        (hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len),
        (1, 3, 1, 3)
    );
    assert_eq!(
        hunk.lines,
        [
            " [server]",
            "-timeout = 30",
            "+timeout = 60",
            " port = 8080"
        ]
    );
    assert_eq!(hunk.path.as_str(), language("/srv/app/config.toml"));
    assert_eq!(
        hunk.old_path.as_ref().map(|old| old.as_str().to_owned()),
        Some(language("/srv/app/config.old.toml"))
    );
    assert_eq!(hunk.status, Some(HunkStatus::Modified));
    assert_eq!(
        hunk.id.as_deref(),
        Some(format!("{}#1", language("/srv/app/config.toml")).as_str())
    );
}

/// Example `diff "/srv/release/1.0" "/srv/release/1.1"` compares two
/// directory trees and reports added, deleted, modified and renamed files.
#[test]
fn compares_two_directory_trees() {
    let test = with_files(&[
        ("/srv/release/1.0/README.md", b"# App\n"),
        (
            "/srv/release/1.0/src/main.rs",
            b"fn main() {\n    run();\n}\n",
        ),
        ("/srv/release/1.0/src/util.rs", b"pub fn run() {}\n"),
        ("/srv/release/1.0/NOTES.txt", b"draft\n"),
        ("/srv/release/1.1/README.md", b"# App\n"),
        (
            "/srv/release/1.1/src/main.rs",
            b"fn main() {\n    run(true);\n}\n",
        ),
        ("/srv/release/1.1/src/run.rs", b"pub fn run() {}\n"),
        ("/srv/release/1.1/CHANGELOG.md", b"## 1.1\n"),
    ]);
    let given = args(&[
        ("from", path("/srv/release/1.0")),
        ("to", path("/srv/release/1.1")),
    ]);
    let hunks = run(&test.ctx(), &given, StageInput::none()).unwrap();
    let summary: Vec<(String, HunkStatus)> = hunks
        .iter()
        .map(|hunk| (hunk.path.as_str().to_owned(), hunk.status.unwrap()))
        .collect();
    assert_eq!(
        summary,
        [
            (language("/srv/release/1.1/CHANGELOG.md"), HunkStatus::Added),
            (language("/srv/release/1.0/NOTES.txt"), HunkStatus::Deleted),
            (
                language("/srv/release/1.1/src/main.rs"),
                HunkStatus::Modified
            ),
            (language("/srv/release/1.1/src/run.rs"), HunkStatus::Renamed),
        ]
    );
    assert_eq!(hunks[0].lines, ["+## 1.1"]);
    assert_eq!(hunks[1].lines, ["-draft"]);
    assert_eq!(
        hunks[2].lines,
        [" fn main() {", "-    run();", "+    run(true);", " }"]
    );
    assert_eq!(
        hunks[3]
            .old_path
            .as_ref()
            .map(|old| old.as_str().to_owned()),
        Some(language("/srv/release/1.0/src/util.rs"))
    );
}

/// Example `find "/srv/app/src" glob: "*.rs" type: file |> diff against:
/// "/srv/app.orig/src" context: 1` compares each listed file with the file of
/// the same name in the other directory.
#[test]
fn compares_listed_files_with_a_directory() {
    let test = with_files(&[
        ("/srv/app.orig/src/lib.rs", b"a\nb\nc\nd\ne\n"),
        ("/srv/app/src/lib.rs", b"a\nb\nC\nd\ne\n"),
        ("/srv/app/src/new.rs", b"pub fn new() {}\n"),
    ]);
    let given = args(&[
        ("against", path("/srv/app.orig/src")),
        ("context", Value::Int(1)),
    ]);
    let hunks = run(
        &test.ctx(),
        &given,
        entries(&["/srv/app/src/lib.rs", "/srv/app/src/new.rs"]),
    )
    .unwrap();
    assert_eq!(hunks.len(), 2);
    assert_eq!(hunks[0].lines, [" b", "-c", "+C", " d"]);
    assert_eq!(hunks[0].status, Some(HunkStatus::Modified));
    assert_eq!(hunks[1].status, Some(HunkStatus::Added));
    assert_eq!(hunks[1].lines, ["+pub fn new() {}"]);
}

/// Example `diff "/srv/app/build.cmd" "/srv/app/build.new.cmd"
/// ignore_whitespace: true` ignores indentation and line ending changes.
#[test]
fn ignores_white_space_on_request() {
    let test = with_files(&[
        ("/srv/app/build.cmd", b"@echo off\nmsbuild /m\n"),
        (
            "/srv/app/build.new.cmd",
            b"@echo off\r\n  msbuild  /m\r\nexit\r\n",
        ),
    ]);
    let given = args(&[
        ("from", path("/srv/app/build.cmd")),
        ("to", path("/srv/app/build.new.cmd")),
        ("ignore_whitespace", Value::Bool(true)),
    ]);
    let hunks = run(&test.ctx(), &given, StageInput::none()).unwrap();
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].lines, [" @echo off", " msbuild /m", "+exit\r"]);
    assert_eq!(hunks[0].ignore_whitespace, Some(true));
}

/// A missing file fails with `E_NOT_FOUND`.
#[test]
fn missing_file_fails_with_not_found() {
    let test = with_files(&[("/srv/app/a.txt", b"a\n")]);
    let given = args(&[
        ("from", path("/srv/app/a.txt")),
        ("to", path("/srv/app/b.txt")),
    ]);
    let error = run(&test.ctx(), &given, StageInput::none()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}
