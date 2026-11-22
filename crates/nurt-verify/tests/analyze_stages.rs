// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The analysis commands through the public stage list, with the test
//! program standing in for the analyzers and formatters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed precondition of a test is reported by a panic"
)]

mod analyze_support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::TargetPath;
use nurt_contracts::value::Value;
use nurt_testkit::journal::JournalCall;

use analyze_support::{Workspace, apply, args, field, plan, read, text, texts};

const TSC_REPORT: &str = "\
src/types.ts(1,14): error TS2322: Type 'string' is not assignable to type 'number'.
src/util/parse.ts(12,5): error TS2345: Argument of type 'number' is not assignable to parameter of type 'string'.
src/old.ts(3,1): warning TS6133: 'unused' is declared but its value is never read.
";

const WARNINGS_ONLY: &str =
    "src/old.ts(3,1): warning TS6133: 'unused' is declared but its value is never read.\n";

fn typed_project() -> Workspace {
    let workspace = Workspace::new();
    workspace
        .file("tsconfig.json", "{}")
        .file("src/types.ts", "export const a = 1;\n")
        .install("tsc");
    workspace
}

fn count(record: &nurt_contracts::value::Record, name: &str) -> Option<i64> {
    record.get(name).and_then(Value::as_int)
}

#[test]
fn lists_detected_tools_with_version_and_availability() {
    let workspace = Workspace::new();
    workspace
        .file(".prettierrc", "{}")
        .file("tsconfig.json", "{}")
        .file("src/a.ts", "export const a = 1;\n")
        .install("prettier");
    let output = read(
        &workspace.context(),
        "analyze.tools",
        &args("analyze.tools", &[]),
    )
    .unwrap();
    let find = |tool: &str| {
        output
            .records
            .iter()
            .find(|record| field(record, "tool") == Some(tool))
            .unwrap()
    };
    let prettier = find("prettier");
    assert_eq!(prettier.get("found").and_then(Value::as_bool), Some(true));
    assert_eq!(field(prettier, "version"), Some("prettier 9.9.9"));
    assert_eq!(field(prettier, "config"), Some(".prettierrc"));
    let tsc = find("tsc");
    assert_eq!(tsc.get("found").and_then(Value::as_bool), Some(false));
    assert_eq!(field(tsc, "version"), None);
}

#[test]
fn check_returns_the_verdict_and_stores_the_analysis() {
    let workspace = typed_project();
    workspace.file("tsc.report", TSC_REPORT);
    let ctx = workspace.context();
    let values = [("checks", texts(&["types"]))];
    let (output, _) = apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();
    let header = output.header.unwrap();
    assert_eq!(field(&header, "id"), Some("an-1"));
    assert_eq!(header.get("passed").and_then(Value::as_bool), Some(false));
    assert_eq!(count(&header, "errors"), Some(2));
    assert_eq!(count(&header, "warnings"), Some(1));
    assert_eq!(output.records.len(), 3);
    assert_eq!(field(&output.records[0], "severity"), Some("error"));
    assert_eq!(field(&output.records[2], "severity"), Some("warning"));
}

#[test]
fn min_severity_and_max_findings_limit_the_response_not_the_verdict() {
    let workspace = typed_project();
    workspace.file("tsc.report", TSC_REPORT);
    let ctx = workspace.context();
    let values = [
        ("checks", texts(&["types"])),
        ("min_severity", Value::Enum("error".to_owned())),
        ("max_findings", Value::Int(1)),
    ];
    let (output, _) = apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();
    let header = output.header.unwrap();
    assert_eq!(output.records.len(), 1);
    assert_eq!(count(&header, "errors"), Some(2));
    assert_eq!(count(&header, "warnings"), Some(1));
    assert_eq!(header.get("passed").and_then(Value::as_bool), Some(false));
}

#[test]
fn fail_on_selects_the_severity_that_fails_the_verdict() {
    let workspace = typed_project();
    workspace.file("tsc.report", WARNINGS_ONLY);
    let ctx = workspace.context();
    let passed = |fail_on: &str| {
        let values = [
            ("checks", texts(&["types"])),
            ("fail_on", Value::Enum(fail_on.to_owned())),
        ];
        let (output, _) = apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();
        output
            .header
            .unwrap()
            .get("passed")
            .and_then(Value::as_bool)
    };
    assert_eq!(passed("error"), Some(true));
    assert_eq!(passed("warning"), Some(false));
}

#[test]
fn check_reports_a_missing_tool_without_failing() {
    let workspace = Workspace::new();
    workspace.file("tsconfig.json", "{}").file("a.ts", "");
    let ctx = workspace.context();
    let values = [("checks", texts(&["types"]))];
    let (output, summary) = apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();
    let header = output.header.unwrap();
    assert!(format!("{:?}", header.get("checks")).contains("missing"));
    assert!(summary.notable.iter().any(|code| code == "tool_missing"));
    assert_eq!(header.get("passed").and_then(Value::as_bool), Some(true));
}

#[test]
fn check_of_changed_files_needs_the_repository_service() {
    let workspace = typed_project();
    let values = [("changed_only", Value::Bool(true))];
    let error = apply(
        &workspace.context(),
        "analyze.check",
        &args("analyze.check", &values),
    )
    .err()
    .unwrap();
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn show_filters_the_stored_findings() {
    let workspace = typed_project();
    workspace.file("tsc.report", TSC_REPORT);
    let ctx = workspace.context();
    let values = [("checks", texts(&["types"]))];
    apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();

    let all = read(&ctx, "analyze.show", &args("analyze.show", &[])).unwrap();
    assert_eq!(all.records.len(), 3);
    assert_eq!(field(&all.header.unwrap(), "id"), Some("an-1"));

    let by_check = [("check", text("types")), ("offset", Value::Int(1))];
    let page = read(&ctx, "analyze.show", &args("analyze.show", &by_check)).unwrap();
    assert_eq!(page.records.len(), 2);

    let by_file = [("file", text("parse.ts"))];
    let one = read(&ctx, "analyze.show", &args("analyze.show", &by_file)).unwrap();
    assert_eq!(one.records.len(), 1);
    assert_eq!(field(&one.records[0], "path"), Some("src/util/parse.ts"));

    let severe = [
        ("min_severity", Value::Enum("error".to_owned())),
        ("max_findings", Value::Int(1)),
    ];
    let first = read(&ctx, "analyze.show", &args("analyze.show", &severe)).unwrap();
    assert_eq!(first.records.len(), 1);
}

#[test]
fn show_rejects_an_unknown_analysis_and_offers_the_stored_ones() {
    let workspace = typed_project();
    let ctx = workspace.context();
    let values = [("checks", texts(&["types"]))];
    apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();
    let unknown = [("analysis_id", text("an-99"))];
    let error = read(&ctx, "analyze.show", &args("analyze.show", &unknown))
        .err()
        .unwrap();
    assert_eq!(error.code(), ErrorCode::UnknownAnalysis);
    assert_eq!(error.data().candidates.len(), 1);
}

fn formatter_project() -> Workspace {
    let workspace = Workspace::new();
    workspace
        .file(".prettierrc", "{}")
        .file(
            "src/loose.ts",
            "let a = 1;   \nlet b = 2;\t\nlet c = 3;  \n",
        )
        .file("src/tidy.ts", "let a = 1;\n")
        .install("prettier");
    workspace
}

#[test]
fn format_preview_lists_the_files_and_changes_nothing() {
    let workspace = formatter_project();
    let plan = plan(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &[]),
    )
    .unwrap();
    let paths: Vec<&str> = plan
        .changes
        .iter()
        .map(|change| change.path.as_str())
        .collect();
    assert_eq!(paths, vec!["src/loose.ts"]);
    let sample = plan.changes[0].after.clone().unwrap();
    assert!(sample.contains("-let a = 1;   "));
    assert!(sample.contains("+let a = 1;"));
    assert_eq!(workspace.read("src/loose.ts").matches("  ").count(), 2);
}

#[test]
fn format_preview_limits_the_sample_lines() {
    let workspace = formatter_project();
    let values = [("sample_lines", Value::Int(2))];
    let plan = plan(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &values),
    )
    .unwrap();
    let sample = plan.changes[0].after.clone().unwrap();
    assert_eq!(sample.lines().count(), 2);
}

#[test]
fn format_rewrites_the_listed_files_and_journals_exactly_them() {
    let workspace = formatter_project();
    let (ctx, journal) = workspace.journaled();
    let (_, summary) = apply(&ctx, "analyze.format", &args("analyze.format", &[])).unwrap();
    assert_eq!(summary.changes, vec!["src/loose.ts".to_owned()]);
    assert_eq!(
        workspace.read("src/loose.ts"),
        "let a = 1;\nlet b = 2;\nlet c = 3;\n"
    );
    assert_eq!(workspace.read("src/tidy.ts"), "let a = 1;\n");

    let calls = journal.calls();
    let last_run = calls
        .iter()
        .rposition(|call| matches!(call, JournalCall::Begin { .. }))
        .unwrap();
    let mut preserved: Vec<String> = calls
        .into_iter()
        .skip(last_run)
        .filter_map(|call| match call {
            JournalCall::Preserve { targets, .. } => Some(targets),
            _ => None,
        })
        .flatten()
        .filter_map(|target| match target {
            TargetPath::Fs(path) => path
                .as_path()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    preserved.sort();
    preserved.dedup();
    assert_eq!(preserved, vec!["loose.ts".to_owned()]);
}

#[test]
fn format_reports_that_nothing_needs_formatting() {
    let workspace = Workspace::new();
    workspace
        .file(".prettierrc", "{}")
        .file("a.ts", "let a = 1;\n")
        .install("prettier");
    let (_, summary) = apply(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &[]),
    )
    .unwrap();
    assert!(summary.changes.is_empty());
    assert!(summary.notable.iter().any(|code| code == "nothing_matched"));
}

#[test]
fn format_is_limited_to_the_given_paths() {
    let workspace = formatter_project();
    workspace.file("lib/other.ts", "let z = 0;  \n");
    let values = [("path", texts(&["lib"]))];
    let (_, summary) = apply(
        &workspace.context(),
        "analyze.format",
        &args("analyze.format", &values),
    )
    .unwrap();
    assert_eq!(summary.changes, vec!["lib/other.ts".to_owned()]);
    assert!(workspace.read("src/loose.ts").contains("1;   "));
}
