// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the test commands on recorded results.

use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{Exec, TestFramework, TestResult, TestRun, TestStatus};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectStage, ExecCtx, ReadStage, StageData, StageInput, StageOutput,
};
use nurt_contracts::value::{Duration, Path as PathValue, Record, Value};
use nurt_proc::{JobFinisher, ToolResult};

use super::failed::TestFailed;
use super::finish::{TestFinisher, finish};
use super::launch::{RunSpec, Shape};
use super::list::TestList;
use super::run::TestRun as RunStage;
use super::show::TestShow;
use super::support::{corpus, ctx, write};
use crate::iface::{EntryKind, StoredEntry};
use crate::store::DirStore;

const SHAPE: Shape = Shape {
    failures: 10,
    message_lines: 5,
    slowest: 0,
};

fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values.iter().fold(BoundArgs::new(command), |args, (name, value)| {
        args.with(*name, ArgValue::Value(value.clone()), false)
    })
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn records(output: StageOutput<'_>) -> Vec<Record> {
    match output.data {
        StageData::Records(stream) => stream.collect_bounded(10_000).unwrap(),
        _ => Vec::new(),
    }
}

fn field<'a>(record: &'a Record, name: &str) -> Option<&'a str> {
    record.get(name).and_then(Value::as_str)
}

fn number(record: &Record, name: &str) -> Option<i64> {
    record.get(name).and_then(Value::as_int)
}

fn result(run: &str, id: &str, status: TestStatus, message: Option<&str>) -> TestResult {
    TestResult {
        run: run.to_owned(),
        id: id.to_owned(),
        name: id.to_owned(),
        status,
        suite: Some("example.com/p".to_owned()),
        message: message.map(str::to_owned),
        ..TestResult::default()
    }
}

/// Stores a run of the Go framework in the project `cwd`.
fn seed(state: &Path, id: &str, cwd: &str, retry_of: Option<&str>, items: Vec<TestResult>) {
    let failed = items.iter().filter(|r| r.status == TestStatus::Failed).count() as u64;
    let header = Record::from(TestRun {
        id: id.to_owned(),
        framework: TestFramework::Go,
        failed,
        passed: items.len() as u64 - failed,
        cwd: PathValue::from(cwd),
        command: vec!["go".to_owned(), "test".to_owned()],
        retry_of: retry_of.map(str::to_owned),
        ..TestRun::default()
    });
    let entry = StoredEntry {
        id: id.to_owned(),
        header,
        items: items.into_iter().map(Record::from).collect(),
    };
    DirStore::new(state, 50).save(EntryKind::TestRun, &entry).unwrap();
}

fn mixed_items(run: &str) -> Vec<TestResult> {
    vec![
        result(run, "TestA", TestStatus::Passed, None),
        result(run, "TestB", TestStatus::Failed, Some("line 1\nline 2\nline 3")),
        result(run, "TestC", TestStatus::Error, Some("boom")),
        result(run, "TestD", TestStatus::Skipped, None),
    ]
}

fn show(ctx: &ExecCtx, values: &[(&str, Value)]) -> nurt_contracts::error::Result<Vec<Record>> {
    let args = args("test.show", values);
    let output = TestShow.execute(&args, StageInput::none(), ctx)?;
    Ok(records(output))
}

#[test]
fn show_returns_every_result_without_a_status_filter() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    let shown = show(&ctx(state.path()), &[]).unwrap();
    assert_eq!(shown.len(), 4);
}

#[test]
fn show_selects_failed_results_including_errors() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    let shown = show(&ctx(state.path()), &[("status", text("failed"))]).unwrap();
    let ids: Vec<&str> = shown.iter().filter_map(|r| field(r, "id")).collect();
    assert_eq!(ids, ["TestB", "TestC"]);
    let passed = show(&ctx(state.path()), &[("status", text("passed"))]).unwrap();
    assert_eq!(passed.len(), 1);
}

#[test]
fn show_filters_by_part_of_the_identifier_ignoring_case() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    let shown = show(&ctx(state.path()), &[("test", text("testc"))]).unwrap();
    assert_eq!(shown.len(), 1);
}

#[test]
fn show_cuts_messages_to_the_line_limit_and_hides_failures_beyond_the_detail_limit() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    let shown = show(
        &ctx(state.path()),
        &[
            ("status", text("failed")),
            ("failures", Value::Int(1)),
            ("message_lines", Value::Int(2)),
        ],
    )
    .unwrap();
    assert_eq!(field(&shown[0], "message"), Some("line 1\nline 2"));
    assert_eq!(field(&shown[1], "message"), None);
}

#[test]
fn show_selects_the_named_run_and_lists_candidates_for_an_unknown_one() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    seed(state.path(), "run-2", "/p", None, vec![result("run-2", "TestA", TestStatus::Passed, None)]);
    let shown = show(&ctx(state.path()), &[("run_id", text("run-1"))]).unwrap();
    assert_eq!(shown.len(), 4);
    let error = show(&ctx(state.path()), &[("run_id", text("run-9"))]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownRun);
    assert_eq!(error.data().candidates.len(), 2);
}

#[test]
fn show_without_any_run_fails_with_unknown_run() {
    let state = tempfile::tempdir().unwrap();
    let error = show(&ctx(state.path()), &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownRun);
}

#[test]
fn list_returns_the_newest_runs_first_up_to_the_limit() {
    let state = tempfile::tempdir().unwrap();
    for number in 1..=3 {
        seed(state.path(), &format!("run-{number}"), "/p", None, Vec::new());
    }
    let args = args("test.list", &[("limit", Value::Int(2))]);
    let context = ctx(state.path());
    let output = TestList.execute(&args, StageInput::none(), &context).unwrap();
    let headers = records(output);
    let ids: Vec<&str> = headers.iter().filter_map(|r| field(r, "id")).collect();
    assert_eq!(ids, ["run-3", "run-2"]);
}

fn rerun_command(ctx: &ExecCtx, values: &[(&str, Value)]) -> nurt_contracts::error::Result<Vec<String>> {
    let args = args("test.failed", values);
    let plan = TestFailed.plan(&args, StageInput::none(), ctx)?;
    let command = plan.preview.first().and_then(|r| r.get("command")).and_then(Value::as_list);
    Ok(command
        .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default())
}

#[test]
fn failed_plans_a_rerun_of_only_failed_and_errored_tests() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    let command = rerun_command(&ctx(state.path()), &[]).unwrap();
    assert_eq!(command[..3], ["go", "test", "-json"]);
    assert!(command.contains(&"^(TestB|TestC)$".to_owned()));
    assert_eq!(command.last().map(String::as_str), Some("example.com/p"));
}

#[test]
fn failed_skips_a_run_that_a_later_run_repeated() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, mixed_items("run-1"));
    seed(state.path(), "run-2", "/p", Some("run-1"), vec![result("run-2", "TestB", TestStatus::Passed, None)]);
    let error = rerun_command(&ctx(state.path()), &[]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownRun);
}

#[test]
fn failed_with_a_project_path_ignores_runs_of_other_projects() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/one", None, mixed_items("run-1"));
    seed(state.path(), "run-2", "/two", None, vec![result("run-2", "TestZ", TestStatus::Failed, None)]);
    let command = rerun_command(&ctx(state.path()), &[("path", Value::Path("/one".to_owned()))]).unwrap();
    assert!(command.contains(&"^(TestB|TestC)$".to_owned()));
}

#[test]
fn failed_of_a_run_without_failures_plans_no_command() {
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), "run-1", "/p", None, vec![result("run-1", "TestA", TestStatus::Passed, None)]);
    let command = rerun_command(&ctx(state.path()), &[("run_id", text("run-1"))]).unwrap();
    assert!(command.is_empty());
}

#[test]
fn run_without_a_recognised_framework_offers_candidates_and_a_fix() {
    let project = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let args = args("test.run", &[("path", Value::Path(project.path().to_string_lossy().into_owned()))]);
    let error = RunStage.plan(&args, StageInput::none(), &ctx(state.path())).unwrap_err();
    assert_eq!(error.code(), ErrorCode::TestNoFramework);
    assert_eq!(error.data().candidates.len(), 7);
    assert!(!error.data().fix.is_empty());
}

#[test]
fn run_rejects_a_target_that_looks_like_an_option() {
    let project = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let args = args(
        "test.run",
        &[
            ("path", Value::Path(project.path().to_string_lossy().into_owned())),
            ("targets", Value::List(vec![text("--watch")])),
        ],
    );
    let error = RunStage.plan(&args, StageInput::none(), &ctx(state.path())).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn run_previews_the_command_of_the_detected_framework() {
    let project = tempfile::tempdir().unwrap();
    write(project.path(), "go.mod", "module example.com/p\n");
    let state = tempfile::tempdir().unwrap();
    let args = args(
        "test.run",
        &[
            ("path", Value::Path(project.path().to_string_lossy().into_owned())),
            ("filter", text("TestA")),
            ("bail", Value::Bool(true)),
        ],
    );
    let plan = RunStage.plan(&args, StageInput::none(), &ctx(state.path())).unwrap();
    let preview = &plan.preview[0];
    assert_eq!(field(preview, "framework"), Some("go"));
    let command = preview.get("command").and_then(Value::as_list).unwrap();
    assert!(command.contains(&text("-failfast")));
}

#[test]
fn run_with_a_command_and_no_framework_uses_the_custom_runner() {
    let project = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let args = args(
        "test.run",
        &[
            ("path", Value::Path(project.path().to_string_lossy().into_owned())),
            ("command", Value::List(vec![text("make"), text("test")])),
        ],
    );
    let plan = RunStage.plan(&args, StageInput::none(), &ctx(state.path())).unwrap();
    assert_eq!(field(&plan.preview[0], "framework"), Some("custom"));
}

fn tool_result(dir: &Path, exit_code: i64, stdout: &str) -> ToolResult {
    let file = write(dir, "stdout.txt", stdout);
    ToolResult {
        exec: Exec {
            exit_code,
            elapsed: Duration::from_millis(1500),
            ..Exec::default()
        },
        stdout_file: Some(file),
        stderr_file: None,
        op: None,
    }
}

fn spec(run_id: &str, framework: &str, dir: &Path) -> RunSpec {
    RunSpec {
        run_id: run_id.to_owned(),
        framework: framework.to_owned(),
        dir: dir.to_string_lossy().into_owned(),
        argv: vec![framework.to_owned()],
        report: None,
        coverage_file: None,
        targets: Vec::new(),
        filter: None,
        retry_of: None,
        fingerprint: Some("sha256:abc".to_owned()),
        only_ids: Vec::new(),
        shape: SHAPE,
    }
}

fn go_output(status: &str) -> String {
    format!(
        "{{\"Action\":\"run\",\"Package\":\"p\",\"Test\":\"TestA\"}}\n{{\"Action\":\"{status}\",\"Package\":\"p\",\"Test\":\"TestA\",\"Elapsed\":0.5}}\n"
    )
}

#[test]
fn finish_counts_results_and_stores_the_run() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outcome = tool_result(project.path(), 1, &corpus("go/1.26.0/mixed.jsonl"));
    let finished = finish(&ctx(state.path()), &spec("run-1", "go", project.path()), &outcome).unwrap();
    let header = &finished.header;
    assert_eq!(field(header, "status"), Some("failed"));
    assert_eq!(number(header, "passed"), Some(2));
    assert_eq!(number(header, "failed"), Some(2));
    assert_eq!(number(header, "skipped"), Some(1));
    assert_eq!(number(header, "errors"), Some(1));
    assert_eq!(field(header, "fingerprint"), Some("sha256:abc"));
    assert_eq!(number(header, "exit_code"), Some(1));
    assert_eq!(finished.items.len(), 6);
    let stored = DirStore::new(state.path(), 50).load(EntryKind::TestRun, "run-1").unwrap().unwrap();
    assert_eq!(stored.items.len(), 6);
    assert_eq!(field(&stored.items[0], "run"), Some("run-1"));
}

#[test]
fn finish_compares_with_earlier_runs_of_the_same_project() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let ctx = ctx(state.path());
    for (run, status) in [("run-1", "pass"), ("run-2", "fail"), ("run-3", "pass")] {
        let outcome = tool_result(project.path(), 0, &go_output(status));
        let finished = finish(&ctx, &spec(run, "go", project.path()), &outcome).unwrap();
        let header = finished.header;
        match run {
            "run-1" => assert!(header.get("baseline").is_none()),
            "run-2" => {
                assert_eq!(field(&header, "baseline"), Some("run-1"));
                let regressions = header.get("regressions").and_then(Value::as_list).unwrap();
                assert_eq!(regressions, [text("p::TestA")]);
            }
            _ => {
                assert_eq!(field(&header, "baseline"), Some("run-2"));
                assert!(header.get("regressions").is_none());
                assert_eq!(header.get("fixed").and_then(Value::as_list).unwrap(), [text("p::TestA")]);
            }
        }
    }
}

#[test]
fn finish_does_not_compare_with_runs_of_another_project() {
    let state = tempfile::tempdir().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let ctx = ctx(state.path());
    finish(&ctx, &spec("run-1", "go", first.path()), &tool_result(first.path(), 0, &go_output("pass"))).unwrap();
    let finished = finish(&ctx, &spec("run-2", "go", second.path()), &tool_result(second.path(), 1, &go_output("fail"))).unwrap();
    assert!(finished.header.get("baseline").is_none());
    assert!(finished.header.get("regressions").is_none());
}

#[test]
fn finish_reports_a_time_limit_with_the_results_read_so_far() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let mut outcome = tool_result(project.path(), -1, &corpus("go/1.26.0/truncated.jsonl"));
    outcome.exec.timed_out = true;
    let finished = finish(&ctx(state.path()), &spec("run-1", "go", project.path()), &outcome).unwrap();
    assert_eq!(field(&finished.header, "status"), Some("timed_out"));
    assert_eq!(number(&finished.header, "passed"), Some(1));
    assert_eq!(number(&finished.header, "errors"), Some(1));
}

#[test]
fn finish_of_a_failed_start_keeps_the_end_of_the_output_as_an_error_result() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outcome = tool_result(project.path(), 101, &corpus("cargo/1.98.1/build_error.txt"));
    let finished = finish(&ctx(state.path()), &spec("run-1", "cargo", project.path()), &outcome).unwrap();
    assert_eq!(field(&finished.header, "status"), Some("error"));
    assert_eq!(number(&finished.header, "errors"), Some(1));
    assert_eq!(field(&finished.items[0], "id"), Some("(run)"));
    assert!(field(&finished.items[0], "message").unwrap().contains("could not compile"));
}

#[test]
fn finish_without_tests_and_with_exit_code_zero_is_no_tests() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outcome = tool_result(project.path(), 0, &corpus("cargo/1.98.1/no_tests.txt"));
    let finished = finish(&ctx(state.path()), &spec("run-1", "cargo", project.path()), &outcome).unwrap();
    assert_eq!(field(&finished.header, "status"), Some("no_tests"));
    assert!(finished.items.is_empty());
}

#[test]
fn finish_with_passing_tests_and_a_failing_exit_code_is_failed() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outcome = tool_result(project.path(), 2, &go_output("pass"));
    let finished = finish(&ctx(state.path()), &spec("run-1", "go", project.path()), &outcome).unwrap();
    assert_eq!(field(&finished.header, "status"), Some("failed"));
}

#[test]
fn finish_reads_the_report_file_of_the_framework() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let report = write(project.path(), "report.xml", &corpus("vitest/5.0.3/mixed.xml"));
    let mut run = spec("run-1", "vitest", project.path());
    run.report = Some(report.to_string_lossy().into_owned());
    let finished = finish(&ctx(state.path()), &run, &tool_result(project.path(), 1, "")).unwrap();
    assert_eq!(number(&finished.header, "failed"), Some(3));
    assert_eq!(number(&finished.header, "skipped"), Some(2));
}

#[test]
fn finish_rejects_an_unreadable_report_with_a_sample_and_the_execution() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let report = write(project.path(), "report.json", &corpus("jest/30.5.2/truncated.json"));
    let mut run = spec("run-1", "jest", project.path());
    run.report = Some(report.to_string_lossy().into_owned());
    let outcome = tool_result(project.path(), 1, "jest printed this\nand this\n");
    let error = match finish(&ctx(state.path()), &run, &outcome) {
        Ok(_) => panic!("the report must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::OutputParse);
    assert_eq!(error.data().sample, ["jest printed this", "and this"]);
    assert!(error.data().details.contains_key("exec"));
}

#[test]
fn finish_keeps_partial_results_of_an_unreadable_report_after_a_time_limit() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let report = write(project.path(), "report.xml", &corpus("bun/1.4.2/truncated.xml"));
    let mut run = spec("run-1", "bun", project.path());
    run.report = Some(report.to_string_lossy().into_owned());
    let mut outcome = tool_result(project.path(), -1, "");
    outcome.exec.timed_out = true;
    let finished = finish(&ctx(state.path()), &run, &outcome).unwrap();
    assert_eq!(field(&finished.header, "status"), Some("timed_out"));
    assert_eq!(number(&finished.header, "passed"), Some(2));
}

#[test]
fn finish_attaches_the_coverage_read_from_the_report_file() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let lcov = write(project.path(), "lcov.info", &corpus("coverage/sample.lcov"));
    let mut run = spec("run-1", "go", project.path());
    run.coverage_file = Some(lcov.to_string_lossy().into_owned());
    let finished = finish(&ctx(state.path()), &run, &tool_result(project.path(), 0, &go_output("pass"))).unwrap();
    let coverage = TestRun::try_from(finished.header).unwrap().coverage.unwrap();
    assert!((coverage.lines_pct - 50.0).abs() < f64::EPSILON);
    assert_eq!(coverage.report_file.unwrap().as_str(), lcov.to_string_lossy());
}

#[test]
fn finish_ignores_a_coverage_report_that_is_not_readable() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let broken = write(project.path(), "lcov.info", "not coverage");
    let mut run = spec("run-1", "go", project.path());
    run.coverage_file = Some(broken.to_string_lossy().into_owned());
    let finished = finish(&ctx(state.path()), &run, &tool_result(project.path(), 0, &go_output("pass"))).unwrap();
    assert!(finished.header.get("coverage").is_none());
}

#[test]
fn finish_lists_the_slowest_tests_when_asked() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let mut run = spec("run-1", "go", project.path());
    run.shape.slowest = 1;
    let finished = finish(&ctx(state.path()), &run, &tool_result(project.path(), 0, &go_output("pass"))).unwrap();
    let slowest = finished.header.get("slowest").and_then(Value::as_list).unwrap();
    assert_eq!(slowest, [text("p::TestA: 500ms")]);
}

#[test]
fn finish_of_a_repeat_drops_skipped_results_outside_the_selection() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let output = format!(
        "{}{{\"Action\":\"skip\",\"Package\":\"p\",\"Test\":\"TestOther\",\"Elapsed\":0}}\n",
        go_output("fail")
    );
    let mut run = spec("run-2", "go", project.path());
    run.only_ids = vec!["p::TestA".to_owned()];
    run.retry_of = Some("run-1".to_owned());
    let finished = finish(&ctx(state.path()), &run, &tool_result(project.path(), 1, &output)).unwrap();
    assert_eq!(finished.items.len(), 1);
    assert_eq!(field(&finished.header, "retry_of"), Some("run-1"));
}

#[test]
fn job_finisher_returns_the_header_of_the_stored_run() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let data = serde_json::to_value(spec("run-1", "go", project.path())).unwrap();
    let header = TestFinisher
        .finish(&ctx(state.path()), &tool_result(project.path(), 0, &go_output("pass")), Some(&data))
        .unwrap();
    assert_eq!(field(&header, "id"), Some("run-1"));
    assert!(TestFinisher.finish(&ctx(state.path()), &tool_result(project.path(), 0, ""), None).is_err());
}

#[test]
fn stages_cover_the_four_test_commands() {
    let names: Vec<String> = super::stages().iter().map(|stage| stage.descriptor().command).collect();
    assert_eq!(names, ["test.run", "test.failed", "test.show", "test.list"]);
}

#[test]
fn project_directories_are_kept_as_given() {
    let path: PathBuf = Path::new("/p").to_path_buf();
    assert_eq!(path.to_string_lossy(), "/p");
}
