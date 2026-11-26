// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parsers on recorded output of each analyzer and formatter.

use std::path::Path;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};

use super::support::Project;
use crate::analyze::analyzers::{
    biome, clippy, eslint, gofmt, govet, mypy, prettier, ruff, rustfmt, tsc,
};
use crate::analyze::output::ToolOutput;

macro_rules! corpus {
    ($path:literal) => {
        include_str!(concat!("../../../tests/data/analyze/", $path))
    };
}

fn stdout(text: &str) -> ToolOutput {
    ToolOutput {
        stdout: text.to_owned(),
        stderr: String::new(),
        exit_code: Some(1),
    }
}

fn stderr(text: &str) -> ToolOutput {
    ToolOutput {
        stdout: String::new(),
        stderr: text.to_owned(),
        exit_code: Some(1),
    }
}

fn places(findings: &[Finding]) -> Vec<(String, Option<u64>, Option<u64>)> {
    findings
        .iter()
        .map(|item| (item.path.as_str().to_owned(), item.line, item.column))
        .collect()
}

fn paths(findings: &[Finding]) -> Vec<&str> {
    findings.iter().map(|item| item.path.as_str()).collect()
}

#[test]
fn parses_tsc_diagnostics() {
    let found = tsc::parse(
        &stdout(corpus!("tsc/5.9.2/errors.txt")),
        Path::new("/work/project"),
    );
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].kind, FindingKind::Types);
    assert_eq!(found[0].severity, FindingSeverity::Error);
    assert_eq!(found[0].rule.as_deref(), Some("TS2322"));
    assert_eq!(found[0].tool.as_deref(), Some("tsc"));
    assert_eq!(
        places(&found),
        vec![
            ("src/types.ts".to_owned(), Some(1), Some(14)),
            ("src/util/parse.ts".to_owned(), Some(12), Some(5)),
            ("src/old.ts".to_owned(), Some(3), Some(1)),
        ]
    );
    assert_eq!(found[2].severity, FindingSeverity::Warning);
    assert_eq!(
        found[0].message,
        "Type 'string' is not assignable to type 'number'."
    );
}

#[test]
fn tsc_clean_output_has_no_findings() {
    let found = tsc::parse(
        &stdout(corpus!("tsc/5.9.2/clean.txt")),
        Path::new("/work/project"),
    );
    assert!(found.is_empty());
}

#[test]
fn parses_eslint_json_report() {
    let found = eslint::parse(
        &stdout(corpus!("eslint/10.12.0/report.json")),
        Path::new("/work/project"),
    );
    assert_eq!(found.len(), 3);
    assert_eq!(paths(&found), vec!["src/a.js", "src/a.js", "src/b.js"]);
    assert_eq!(found[0].severity, FindingSeverity::Error);
    assert_eq!(found[0].rule.as_deref(), Some("no-unused-vars"));
    assert_eq!(found[1].severity, FindingSeverity::Warning);
    assert_eq!(found[2].rule, None);
    assert_eq!(found[2].line, Some(2));
}

#[test]
fn eslint_ignores_output_that_is_not_a_report() {
    let found = eslint::parse(&stdout("Oops! Something went wrong"), Path::new("/work"));
    assert!(found.is_empty());
}

#[test]
fn parses_biome_lint_annotations() {
    let found = biome::parse_lint(
        &stdout(corpus!("biome/2.5.15/lint.txt")),
        Path::new("/work"),
    );
    assert_eq!(found.len(), 3);
    assert_eq!(
        found.iter().map(|item| item.severity).collect::<Vec<_>>(),
        vec![
            FindingSeverity::Error,
            FindingSeverity::Warning,
            FindingSeverity::Info
        ]
    );
    assert_eq!(
        found[0].rule.as_deref(),
        Some("lint/suspicious/noDoubleEquals")
    );
    assert_eq!(found[0].message, "Use === instead of ==.");
    assert_eq!(found[0].column, Some(7));
}

#[test]
fn parses_biome_format_annotations_once_per_file() {
    let found = biome::parse_format(
        &stdout(corpus!("biome/2.5.15/format.txt")),
        Path::new("/work"),
    );
    assert_eq!(paths(&found), vec!["src/a.ts", "src/c.ts"]);
    assert!(found.iter().all(|item| item.kind == FindingKind::Format));
    assert!(
        found
            .iter()
            .all(|item| item.severity == FindingSeverity::Warning)
    );
}

#[test]
fn parses_prettier_list_of_different_files() {
    let project = Project::with_files(&[("src/ugly.ts", ""), ("src/styles/main.css", "")]);
    let found = prettier::parse(
        &stdout(corpus!("prettier/3.9.9/list-different.txt")),
        project.dir(),
    );
    assert_eq!(paths(&found), vec!["src/ugly.ts", "src/styles/main.css"]);
    assert_eq!(found[0].rule.as_deref(), Some("format"));
    assert_eq!(found[0].line, None);
}

#[test]
fn parses_ruff_json_report_with_error_classes() {
    let found = ruff::parse_lint(
        &stdout(corpus!("ruff/0.16.7/check.json")),
        Path::new("/work/project"),
    );
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].severity, FindingSeverity::Error);
    assert_eq!(found[0].rule.as_deref(), Some("F401"));
    assert_eq!(found[0].path.as_str(), "src/app.py");
    assert_eq!(found[1].severity, FindingSeverity::Warning);
    assert_eq!(found[2].rule.as_deref(), Some("syntax"));
    assert_eq!(found[2].severity, FindingSeverity::Error);
}

#[test]
fn ruff_reports_only_pyflakes_and_runtime_error_codes_as_errors() {
    let item = |code: &str| {
        format!(
            "{{\"code\":\"{code}\",\"message\":\"m\",\"filename\":\"/work/a.py\",\"location\":{{\"row\":1,\"column\":1}}}}"
        )
    };
    let report = format!(
        "[{}]",
        [
            "F401", "E902", "FBT001", "FIX002", "FURB101", "E501", "ERA001"
        ]
        .map(item)
        .join(",")
    );
    let found = ruff::parse_lint(&stdout(&report), Path::new("/work"));
    let severities: Vec<FindingSeverity> = found.iter().map(|item| item.severity).collect();
    assert_eq!(
        severities,
        [
            FindingSeverity::Error,
            FindingSeverity::Error,
            FindingSeverity::Warning,
            FindingSeverity::Warning,
            FindingSeverity::Warning,
            FindingSeverity::Warning,
            FindingSeverity::Warning,
        ]
    );
}

#[test]
fn parses_ruff_format_check_listing() {
    let project = Project::with_files(&[("src/app.py", ""), ("src/helpers.py", "")]);
    let found = ruff::parse_format(
        &stderr(corpus!("ruff/0.16.7/format-check.txt")),
        project.dir(),
    );
    assert_eq!(paths(&found), vec!["src/app.py", "src/helpers.py"]);
}

#[test]
fn parses_mypy_errors_with_codes_and_skips_notes() {
    let found = mypy::parse(
        &stdout(corpus!("mypy/2.3.1/errors.txt")),
        Path::new("/work"),
    );
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].rule.as_deref(), Some("assignment"));
    assert_eq!(
        found[0].message,
        "Incompatible types in assignment (expression has type \"str\", variable has type \"int\")"
    );
    assert_eq!(found[1].rule.as_deref(), Some("import-not-found"));
    assert_eq!(found[2].severity, FindingSeverity::Warning);
    assert_eq!(found[0].kind, FindingKind::Types);
}

#[test]
fn parses_go_vet_diagnostics_from_standard_error() {
    let found = govet::parse(
        &stderr(corpus!("go-vet/1.25.4/stderr.txt")),
        Path::new("/work"),
    );
    assert_eq!(
        places(&found),
        vec![
            ("internal/store/store.go".to_owned(), Some(42), Some(3)),
            ("internal/store/store.go".to_owned(), Some(57), Some(2)),
            ("internal/cache/cache.go".to_owned(), Some(18), Some(9)),
        ]
    );
    assert_eq!(found[0].tool.as_deref(), Some("go vet"));
    assert_eq!(found[0].message, "declared and not used: unused");
}

#[test]
fn parses_gofmt_list() {
    let project = Project::with_files(&[("internal/store/store.go", ""), ("cmd/main.go", "")]);
    let found = gofmt::parse(&stdout(corpus!("gofmt/1.25.4/list.txt")), project.dir());
    assert_eq!(
        paths(&found),
        vec!["internal/store/store.go", "cmd/main.go"]
    );
}

#[test]
fn gofmt_skips_listed_files_that_do_not_exist() {
    let project = Project::with_files(&[("cmd/main.go", "")]);
    let found = gofmt::parse(&stdout(corpus!("gofmt/1.25.4/list.txt")), project.dir());
    assert_eq!(paths(&found), vec!["cmd/main.go"]);
}

#[test]
fn parses_clippy_json_messages_and_skips_summaries() {
    let found = clippy::parse(
        &stdout(corpus!("clippy/1.95.0/messages.jsonl")),
        Path::new("/work/project"),
    );
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].rule.as_deref(), Some("clippy::needless_return"));
    assert_eq!(found[0].severity, FindingSeverity::Warning);
    assert_eq!(found[0].path.as_str(), "src/lib.rs");
    assert_eq!(found[1].severity, FindingSeverity::Error);
    assert_eq!(found[1].line, Some(4));
    assert_eq!(found[1].column, Some(22));
}

#[test]
fn parses_rustfmt_check_listing() {
    let project = Project::with_files(&[("src/lib.rs", ""), ("src/main.rs", "")]);
    let found = rustfmt::parse(&stdout(corpus!("rustfmt/1.95.0/list.txt")), project.dir());
    assert_eq!(paths(&found), vec!["src/lib.rs", "src/main.rs"]);
}

#[test]
fn normalizes_long_messages() {
    let long = format!("a.ts(1,1): error TS1000: {}", "word ".repeat(200));
    let found = tsc::parse(&stdout(&long), Path::new("/work"));
    assert_eq!(found[0].message.chars().count(), 300);
}

#[test]
fn rewrites_windows_separators_and_project_prefix() {
    let found = tsc::parse(
        &stdout("C:\\work\\project\\src\\a.ts(2,3): error TS1005: ';' expected."),
        Path::new("C:\\work\\project"),
    );
    assert_eq!(found[0].path.as_str(), "src/a.ts");
}
