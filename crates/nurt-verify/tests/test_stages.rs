// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-verify/tests/test_stages.rs
// Responsibility:   The test commands through the public stage list, with the test program standing in for a framework (`go`) and for a custom runner (`tap`).
// Dependencies:     analyze_support, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The test commands through the public stage list, with the test program
//! standing in for a framework (`go`) and for a custom runner (`tap`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed precondition of a test is reported by a panic"
)]

mod analyze_support;

use std::path::Path;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Record, Value};

use analyze_support::{FakeGit, Workspace, apply, args, field, plan, read, text, texts};

/// Recorded output of `go test -json` with passed, failed, skipped and errored tests.
fn go_report() -> String {
    corpus("go/1.26.0/mixed.jsonl")
}

fn corpus(relative: &str) -> String {
    let path = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/data/test")
    .join(relative);
    std::fs::read_to_string(path).unwrap()
}

fn path_value(workspace: &Workspace) -> Value {
    Value::Path(workspace.project.path().to_string_lossy().into_owned())
}

/// Project that the `go` program replays: the report and the exit code of
/// the run.
fn go_project(report: &str, exit_code: u8) -> Workspace {
    let workspace = Workspace::new();
    workspace
        .file("go.mod", "module example.com/p\n")
        .file("go.report", report)
        .file("go.exit", &exit_code.to_string())
        .install("go");
    workspace
}

fn number(record: &Record, name: &str) -> Option<i64> {
    record.get(name).and_then(Value::as_int)
}

fn run_args(workspace: &Workspace, extra: &[(&str, Value)]) -> nurt_contracts::stage::BoundArgs {
    let mut values = vec![("path", path_value(workspace))];
    values.extend(extra.iter().map(|(name, value)| (*name, value.clone())));
    args("test.run", &values)
}

#[test]
fn run_executes_the_framework_and_stores_the_results() {
    let workspace = go_project(&go_report(), 1);
    let ctx = workspace.context();
    let (output, _) = apply(&ctx, "test.run", &run_args(&workspace, &[])).unwrap();
    let header = output.header.unwrap();
    assert_eq!(field(&header, "id"), Some("run-1"));
    assert_eq!(field(&header, "framework"), Some("go"));
    assert_eq!(field(&header, "status"), Some("failed"));
    assert_eq!(number(&header, "passed"), Some(2));
    assert_eq!(number(&header, "failed"), Some(2));
    assert_eq!(output.records.len(), 6);
    let listed = read(&ctx, "test.list", &args("test.list", &[])).unwrap();
    assert_eq!(field(&listed.records[0], "id"), Some("run-1"));
}

#[test]
fn run_without_a_path_uses_the_base_directory_of_the_request() {
    let workspace = go_project(&go_report(), 1);
    let ctx = workspace.context();
    let values = [("framework", text("go"))];
    let (output, _) = apply(&ctx, "test.run", &args("test.run", &values)).unwrap();
    assert_eq!(field(&output.header.unwrap(), "framework"), Some("go"));
}

#[test]
fn show_returns_the_failed_results_of_the_stored_run() {
    let workspace = go_project(&go_report(), 1);
    let ctx = workspace.context();
    apply(&ctx, "test.run", &run_args(&workspace, &[])).unwrap();
    let values = [("status", text("failed"))];
    let shown = read(&ctx, "test.show", &args("test.show", &values)).unwrap();
    assert_eq!(shown.records.len(), 3);
    assert!(
        shown
            .records
            .iter()
            .all(|record| field(record, "status") != Some("passed"))
    );
}

#[test]
fn failed_runs_the_framework_again_for_the_failed_tests_only() {
    let workspace = go_project(&go_report(), 1);
    let ctx = workspace.context();
    apply(&ctx, "test.run", &run_args(&workspace, &[])).unwrap();
    let values = [("path", path_value(&workspace))];
    let (output, _) = apply(&ctx, "test.failed", &args("test.failed", &values)).unwrap();
    let header = output.header.unwrap();
    assert_eq!(field(&header, "id"), Some("run-2"));
    assert_eq!(field(&header, "retry_of"), Some("run-1"));
}

#[test]
fn failed_of_a_run_without_failures_reports_it_and_runs_nothing() {
    let pass = "{\"Action\":\"run\",\"Package\":\"p\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"p\",\"Test\":\"TestA\",\"Elapsed\":0.1}\n";
    let workspace = go_project(pass, 0);
    let ctx = workspace.context();
    apply(&ctx, "test.run", &run_args(&workspace, &[])).unwrap();
    let values = [("run_id", text("run-1"))];
    let (output, _) = apply(&ctx, "test.failed", &args("test.failed", &values)).unwrap();
    assert!(output.records.is_empty());
    assert_eq!(field(&output.header.unwrap(), "id"), Some("run-1"));
}

#[test]
fn failed_with_an_unknown_run_offers_the_stored_runs() {
    let workspace = go_project(&go_report(), 1);
    let ctx = workspace.context();
    apply(&ctx, "test.run", &run_args(&workspace, &[])).unwrap();
    let values = [("run_id", text("run-9"))];
    let error = plan(&ctx, "test.failed", &args("test.failed", &values))
        .err()
        .unwrap();
    assert_eq!(error.code(), ErrorCode::UnknownRun);
    assert_eq!(error.data().candidates.len(), 1);
}

#[test]
fn run_reads_the_results_of_a_custom_command_in_tap_format() {
    let workspace = Workspace::new();
    workspace
        .file("tap.report", "ok 1 - adds\nnot ok 2 - subtracts\n")
        .file("tap.exit", "1")
        .install("tap");
    let values = [("command", texts(&["tap"]))];
    let ctx = workspace.context();
    let (output, _) = apply(&ctx, "test.run", &run_args(&workspace, &values)).unwrap();
    let header = output.header.unwrap();
    assert_eq!(field(&header, "framework"), Some("custom"));
    assert_eq!(number(&header, "passed"), Some(1));
    assert_eq!(number(&header, "failed"), Some(1));
}

/// Fingerprint reported by a run of the test program in the project.
///
/// A test program built for coverage measurement writes its own profile
/// (`*.profraw`) into the directory it runs in; the files are removed after
/// the run, so that the next fingerprint covers the files of the project only.
fn fingerprint_of(ctx: &nurt_contracts::stage::ExecCtx, workspace: &Workspace) -> String {
    let (output, _) = apply(ctx, "test.run", &run_args(workspace, &[])).unwrap();
    for entry in std::fs::read_dir(workspace.project.path())
        .unwrap()
        .flatten()
    {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "profraw")
        {
            std::fs::remove_file(path).unwrap();
        }
    }
    field(&output.header.unwrap(), "fingerprint")
        .unwrap()
        .to_owned()
}

#[test]
fn run_fingerprints_the_tracked_files_when_the_directory_is_in_a_repository() {
    let workspace = go_project(&go_report(), 1);
    let project = workspace.project.path();
    let name = project.file_name().unwrap().to_string_lossy().into_owned();
    let tracked = FakeGit {
        root: Some(project.parent().unwrap().to_path_buf()),
        tracked: vec![format!("{name}/go.mod"), "elsewhere/other.txt".to_owned()],
        changed: Vec::new(),
    };
    let outside = FakeGit {
        root: None,
        tracked: Vec::new(),
        changed: Vec::new(),
    };
    let walked = fingerprint_of(&workspace.context(), &workspace);
    assert!(walked.starts_with("sha256:"));
    assert_eq!(
        fingerprint_of(&workspace.serviced(Some(outside)), &workspace),
        walked
    );
    assert_ne!(
        fingerprint_of(&workspace.serviced(Some(tracked)), &workspace),
        walked
    );
}

#[test]
fn run_accepts_a_time_limit_as_a_duration() {
    let workspace = go_project(&go_report(), 1);
    let limit = Value::Duration(nurt_contracts::value::Duration::from_millis(60_000));
    let (output, _) = apply(
        &workspace.context(),
        "test.run",
        &run_args(&workspace, &[("timeout", limit)]),
    )
    .unwrap();
    assert_eq!(field(&output.header.unwrap(), "status"), Some("failed"));
}

#[test]
fn run_reports_a_program_that_is_not_installed_with_a_fix() {
    let workspace = Workspace::new();
    let values = [("command", texts(&["no-such-test-program"]))];
    let error = apply(
        &workspace.context(),
        "test.run",
        &run_args(&workspace, &values),
    )
    .err()
    .unwrap();
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    assert!(!error.data().fix.is_empty());
}

#[test]
fn run_attaches_the_coverage_of_the_given_report_file() {
    let workspace = go_project(&go_report(), 1);
    workspace.file("lcov.info", &corpus("coverage/sample.lcov"));
    let report = workspace.project.path().join("lcov.info");
    let values = [(
        "coverage_file",
        Value::Path(report.to_string_lossy().into_owned()),
    )];
    let (output, _) = apply(
        &workspace.context(),
        "test.run",
        &run_args(&workspace, &values),
    )
    .unwrap();
    assert!(output.header.unwrap().get("coverage").is_some());
}

#[test]
fn run_rejects_an_unknown_framework_and_a_custom_one_without_a_command() {
    let workspace = Workspace::new();
    let ctx = workspace.context();
    let unknown = plan(
        &ctx,
        "test.run",
        &run_args(&workspace, &[("framework", text("rspec"))]),
    );
    assert_eq!(unknown.err().unwrap().code(), ErrorCode::UnknownValue);
    let custom = plan(
        &ctx,
        "test.run",
        &run_args(&workspace, &[("framework", text("custom"))]),
    );
    assert_eq!(custom.err().unwrap().code(), ErrorCode::MissingParam);
}

#[test]
fn arguments_of_the_wrong_type_are_rejected_with_the_type_error() {
    let workspace = Workspace::new();
    let ctx = workspace.context();
    for (name, value) in [
        ("filter", Value::Int(1)),
        ("targets", text("one")),
        ("timeout", Value::Int(5)),
        ("coverage_file", Value::Int(1)),
        ("command", Value::Int(1)),
    ] {
        let error = plan(&ctx, "test.run", &run_args(&workspace, &[(name, value)]))
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::Type, "parameter {name}");
    }
}

#[test]
fn negative_limits_are_rejected_with_a_fix() {
    let workspace = go_project(&go_report(), 1);
    let ctx = workspace.context();
    let error = plan(
        &ctx,
        "test.run",
        &run_args(&workspace, &[("failures", Value::Int(-1))]),
    )
    .err()
    .unwrap();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.data().fix, ["failures: 10"]);
    let error = read(
        &ctx,
        "test.list",
        &args("test.list", &[("limit", Value::Int(i64::MAX))]),
    )
    .err()
    .unwrap();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
