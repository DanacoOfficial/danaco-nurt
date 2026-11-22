// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The built-in checks of `analyze.check` (complexity and duplication) and the
//! selection of files by the repository, with doubles for the code service,
//! the file listing and the repository queries.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed precondition of a test is reported by a panic"
)]

mod analyze_support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Record, Value};

use analyze_support::{FakeGit, Workspace, apply, args, field, text, texts};

/// Number of `if` lines that give a function a complexity above the default
/// limit of 15.
const BRANCHES: usize = 20;

/// Lines of a block that the duplication check compares.
const BLOCK_LINES: usize = 12;

fn branching() -> String {
    format!("fn main() {{\n{}}}\n", "    if a {}\n".repeat(BRANCHES))
}

fn block() -> String {
    (0..BLOCK_LINES)
        .map(|index| format!("    let v{index} = call(arg, {index});\n"))
        .collect()
}

fn count(record: &Record, name: &str) -> Option<i64> {
    record.get(name).and_then(Value::as_int)
}

fn kinds<'a>(records: &'a [Record], kind: &str) -> Vec<&'a Record> {
    records
        .iter()
        .filter(|record| field(record, "kind") == Some(kind))
        .collect()
}

fn check(
    workspace: &Workspace,
    git: Option<FakeGit>,
    values: &[(&str, Value)],
) -> nurt_contracts::error::Result<(Record, Vec<Record>)> {
    let ctx = workspace.serviced(git);
    let (output, _) = apply(&ctx, "analyze.check", &args("analyze.check", values))?;
    Ok((output.header.unwrap(), output.records))
}

#[test]
fn complexity_above_the_limit_is_an_error_naming_the_function() {
    let workspace = Workspace::new();
    workspace.file("src/a.rs", &branching());
    let (header, records) = check(&workspace, None, &[("checks", texts(&["complexity"]))]).unwrap();
    assert_eq!(count(&header, "errors"), Some(1));
    assert_eq!(header.get("passed").and_then(Value::as_bool), Some(false));
    let finding = &kinds(&records, "complexity")[0];
    assert!(field(finding, "message").unwrap().starts_with("app.main:"));
    assert_eq!(field(finding, "path"), Some("src/a.rs"));
}

#[test]
fn complexity_max_raises_the_accepted_complexity() {
    let workspace = Workspace::new();
    workspace.file("src/a.rs", &branching());
    let values = [
        ("checks", texts(&["complexity"])),
        ("complexity_max", Value::Int(BRANCHES as i64 + 1)),
    ];
    let (header, records) = check(&workspace, None, &values).unwrap();
    assert_eq!(count(&header, "errors"), Some(0));
    assert!(records.is_empty());
    assert_eq!(header.get("passed").and_then(Value::as_bool), Some(true));
}

#[test]
fn duplicated_blocks_in_two_files_are_reported() {
    let workspace = Workspace::new();
    workspace
        .file("src/a.rs", &block())
        .file("src/b.rs", &block());
    let (_, records) = check(&workspace, None, &[("checks", texts(&["duplication"]))]).unwrap();
    assert!(!kinds(&records, "duplication").is_empty());
}

#[test]
fn duplication_includes_test_files_by_default_and_skips_them_when_asked() {
    let workspace = Workspace::new();
    workspace
        .file("tests/one.rs", &block())
        .file("tests/two.rs", &block());
    let included = check(&workspace, None, &[("checks", texts(&["duplication"]))]).unwrap();
    assert!(!kinds(&included.1, "duplication").is_empty());
    let values = [
        ("checks", texts(&["duplication"])),
        ("include_tests", Value::Bool(false)),
    ];
    let skipped = check(&workspace, None, &values).unwrap();
    assert!(kinds(&skipped.1, "duplication").is_empty());
}

#[test]
fn changed_only_checks_the_files_the_repository_reports() {
    let workspace = Workspace::new();
    workspace
        .file("src/a.rs", &branching())
        .file("src/b.rs", &branching());
    let git = FakeGit {
        root: Some(workspace.project.path().to_path_buf()),
        tracked: Vec::new(),
        changed: vec!["src/a.rs".to_owned()],
    };
    let values = [
        ("checks", texts(&["complexity"])),
        ("changed_only", Value::Bool(true)),
    ];
    let (header, records) = check(&workspace, Some(git), &values).unwrap();
    let paths: Vec<_> = records.iter().filter_map(|r| field(r, "path")).collect();
    assert_eq!(paths, ["src/a.rs"]);
    assert_eq!(
        header.get("changed_only").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn changed_only_outside_a_repository_checks_every_file() {
    let workspace = Workspace::new();
    workspace
        .file("src/a.rs", &branching())
        .file("src/b.rs", &branching());
    let git = FakeGit {
        root: None,
        tracked: Vec::new(),
        changed: Vec::new(),
    };
    let values = [
        ("checks", texts(&["complexity"])),
        ("changed_only", Value::Bool(true)),
    ];
    let (_, records) = check(&workspace, Some(git), &values).unwrap();
    assert_eq!(records.len(), 2);
}

#[test]
fn paths_limit_the_check_to_the_named_files_and_directories() {
    let workspace = Workspace::new();
    workspace
        .file("src/a.rs", &branching())
        .file("lib/b.rs", &branching());
    let values = [
        ("checks", texts(&["complexity"])),
        ("path", texts(&["src"])),
    ];
    let (_, records) = check(&workspace, None, &values).unwrap();
    let paths: Vec<_> = records.iter().filter_map(|r| field(r, "path")).collect();
    assert_eq!(paths, ["src/a.rs"]);
}

#[test]
fn an_unknown_check_is_rejected_with_the_valid_names() {
    let workspace = Workspace::new();
    let error = check(&workspace, None, &[("checks", texts(&["style"]))]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Arg);
    assert!(
        error
            .data()
            .fix
            .iter()
            .any(|fix| fix.contains("complexity"))
    );
}

#[test]
fn an_unknown_fail_on_value_is_rejected() {
    let workspace = Workspace::new();
    let error = check(&workspace, None, &[("fail_on", text("info"))]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Arg);
}
