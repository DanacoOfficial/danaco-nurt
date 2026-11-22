// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Edge cases of `analyze.format`: tools that do not format, programs that
//! are not installed, formatters without files in scope and very large
//! rewrites.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed precondition of a test is reported by a panic"
)]

mod analyze_support;

use nurt_contracts::value::Value;

use analyze_support::{Workspace, apply, args, plan, text};

/// Lines of a file whose every line a formatter rewrites; the product of the
/// old and the new line count passes the limit of the sample diff.
const REWRITTEN_LINES: usize = 2_100;

/// Sample length asked for in the large rewrite.
const SAMPLE_LINES: usize = 5;

#[test]
fn format_skips_tools_that_do_not_format_and_those_that_are_not_installed() {
    let workspace = Workspace::new();
    workspace
        .file("package.json", "{}\n")
        .file(".prettierrc", "{}\n")
        .file("tsconfig.json", "{}\n")
        .file("go.mod", "module example.com/p\n")
        .file("main.go", "package main  \n")
        .install("prettier");
    let ctx = workspace.context();
    let (output, summary) = apply(&ctx, "analyze.format", &args("analyze.format", &[])).unwrap();
    assert!(output.records.is_empty());
    assert!(summary.changes.is_empty());
    assert!(summary.notable.iter().any(|code| code == "nothing_matched"));
}

#[test]
fn format_accepts_a_single_path_given_as_text() {
    let workspace = Workspace::new();
    workspace
        .file(".prettierrc", "{}")
        .file("src/a.ts", "let a = 1;  \n")
        .file("lib/b.ts", "let b = 1;  \n")
        .install("prettier");
    let values = [("path", text("src"))];
    let (_, summary) = apply(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &values),
    )
    .unwrap();
    assert_eq!(summary.changes, ["src/a.ts"]);
    assert_eq!(workspace.read("lib/b.ts"), "let b = 1;  \n");
}

#[test]
fn format_previews_a_large_rewrite_as_removed_and_added_lines_up_to_the_sample_limit() {
    let workspace = Workspace::new();
    let before = "let a = 1;  \n".repeat(REWRITTEN_LINES);
    workspace
        .file(".prettierrc", "{}")
        .file("src/big.ts", &before)
        .install("prettier");
    let values = [("sample_lines", Value::Int(SAMPLE_LINES as i64))];
    let planned = plan(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &values),
    )
    .unwrap();
    let change = &planned.changes[0];
    assert_eq!(change.removed, Some(SAMPLE_LINES as u64));
    assert_eq!(change.added, Some(0));
    assert!(change.after.as_deref().unwrap().starts_with("-let a = 1;"));
    assert_eq!(workspace.read("src/big.ts"), before);
}

#[test]
fn format_reports_the_changed_lines_of_a_file_in_the_preview() {
    let workspace = Workspace::new();
    workspace
        .file(".prettierrc", "{}")
        .file("a.ts", "keep\nchange  \nkeep\n")
        .install("prettier");
    let planned = plan(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &[]),
    )
    .unwrap();
    let change = &planned.changes[0];
    assert_eq!(change.after.as_deref(), Some("-change  \n+change"));
}
