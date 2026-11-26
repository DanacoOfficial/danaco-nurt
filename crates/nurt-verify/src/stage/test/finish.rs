// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading the outcome of a finished test run into records.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{
    Coverage, Exec, KilledBy, TestResult, TestRun, TestRunStatus, TestStatus,
};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Path as PathValue, Record, Value};
use nurt_proc::{JobFinisher, ToolResult};

use super::launch::RunSpec;
use crate::coverage;
use crate::iface::{EntryKind, StoredEntry, VerifyStore, verify_store};
use crate::test::frameworks::parse;
use crate::test::{Framework, ParseError, ParseInput};

/// Lines of the end of the output kept when no test result explains a
/// failed run.
const TAIL_LINES: usize = 12;

/// Name and identifier of the result that stands for a run that produced
/// no test results.
pub(super) const RUN_MARKER: &str = "(run)";

/// Runs of the same project that a run is compared with.
const COMPARE_DEPTH: usize = 20;

/// Lines of the output shown in an `E_OUTPUT_PARSE` error.
const SAMPLE_LINES: usize = 5;

/// Records of a finished run.
pub(super) struct Finished {
    pub(super) header: Record,
    pub(super) items: Vec<Record>,
}

/// Finisher of background test runs.
pub(super) struct TestFinisher;

impl JobFinisher for TestFinisher {
    fn finish(
        &self,
        ctx: &ExecCtx,
        result: &ToolResult,
        data: Option<&serde_json::Value>,
    ) -> Result<Record> {
        let data =
            data.ok_or_else(|| Error::new(ErrorCode::Internal, "the test job has no run data"))?;
        let spec: RunSpec = serde_json::from_value(data.clone()).map_err(|failure| {
            Error::new(
                ErrorCode::Internal,
                format!("the test job data is not readable: {failure}"),
            )
        })?;
        Ok(finish(ctx, &spec, result)?.header)
    }
}

/// Reads the output of the framework, stores the run and returns the
/// records to show.
///
/// # Errors
///
/// Fails with `E_OUTPUT_PARSE` when the output cannot be read and the run
/// did not end by a limit, and when the history cannot be written.
pub(super) fn finish(ctx: &ExecCtx, spec: &RunSpec, result: &ToolResult) -> Result<Finished> {
    let store = verify_store();
    let analysis = analyze(ctx, spec, result)?;
    let previous = previous_runs(ctx, store.as_ref(), &spec.dir);
    let comparison = compare(&previous, &analysis.results);
    let header = header_record(spec, result, &analysis, &comparison);
    let mut full: Vec<TestResult> = analysis.results;
    for item in &mut full {
        item.run.clone_from(&spec.run_id);
    }
    let records: Vec<Record> = full.iter().cloned().map(Record::from).collect();
    store.save(
        ctx,
        EntryKind::TestRun,
        &StoredEntry {
            id: spec.run_id.clone(),
            header: header.clone(),
            items: records,
        },
    )?;
    let items = super::output::shape(&full, spec.shape)
        .into_iter()
        .map(Record::from)
        .collect();
    Ok(Finished { header, items })
}

struct Analysis {
    results: Vec<TestResult>,
    status: TestRunStatus,
    coverage: Option<Coverage>,
}

fn analyze(ctx: &ExecCtx, spec: &RunSpec, result: &ToolResult) -> Result<Analysis> {
    let exec = &result.exec;
    let (stdout, stderr) = outputs(result);
    let output = format!("{stdout}\n{stderr}");
    let report = spec
        .report
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok());
    let framework = Framework::from_name(&spec.framework).unwrap_or(Framework::Custom);
    let dir = PathBuf::from(&spec.dir);
    let input = ParseInput {
        output: &output,
        report: report.as_deref(),
        dir: &dir,
    };
    let ended_early = exec.timed_out || exec.killed_by.is_some();
    let mut results = match parse(framework, &input) {
        Ok(results) => results,
        Err(failure) if ended_early => failure.partial,
        Err(failure) => return Err(parse_error(&failure, &output, exec)),
    };
    if let Some(path) = &spec.report {
        // The results are read; an unreadable report stays for diagnosis.
        let _already_gone_is_fine = fs::remove_file(path);
    }
    if !spec.only_ids.is_empty() {
        results
            .retain(|case| case.status != TestStatus::Skipped || spec.only_ids.contains(&case.id));
    }
    let status = run_status(exec, &results);
    if results.is_empty() && status == TestRunStatus::Error {
        results.push(TestResult {
            id: RUN_MARKER.to_owned(),
            name: RUN_MARKER.to_owned(),
            status: TestStatus::Error,
            message: Some(tail(&output)),
            ..TestResult::default()
        });
    }
    let coverage = read_coverage(ctx, spec, &dir);
    Ok(Analysis {
        results,
        status,
        coverage,
    })
}

fn outputs(result: &ToolResult) -> (String, String) {
    let read = |file: &Option<PathBuf>, inline: &str| match file {
        Some(path) => fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_else(|_| inline.to_owned()),
        None => inline.to_owned(),
    };
    (
        read(&result.stdout_file, &result.exec.stdout),
        read(&result.stderr_file, &result.exec.stderr),
    )
}

fn parse_error(failure: &ParseError, output: &str, exec: &Exec) -> Error {
    let mut error = Error::new(
        ErrorCode::OutputParse,
        format!("cannot read the test output: {}", failure.reason),
    )
    .with_fix("run the tests with command: to see the output");
    for line in output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(SAMPLE_LINES)
    {
        error = error.with_sample(line);
    }
    if let Ok(value) = serde_json::to_value(exec) {
        error = error.with_detail("exec", value);
    }
    error
}

fn run_status(exec: &Exec, results: &[TestResult]) -> TestRunStatus {
    if exec.timed_out {
        return TestRunStatus::TimedOut;
    }
    if exec.killed_by.is_some_and(|by| by != KilledBy::Timeout) {
        return TestRunStatus::Error;
    }
    let count = |status: TestStatus| results.iter().filter(|case| case.status == status).count();
    if count(TestStatus::Failed) > 0 {
        return TestRunStatus::Failed;
    }
    if count(TestStatus::Error) > 0 {
        return TestRunStatus::Error;
    }
    if results.is_empty() {
        return if exec.exit_code == 0 {
            TestRunStatus::NoTests
        } else {
            TestRunStatus::Error
        };
    }
    if exec.exit_code == 0 {
        TestRunStatus::Passed
    } else {
        TestRunStatus::Failed
    }
}

fn tail(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..].join("\n")
}

fn read_coverage(ctx: &ExecCtx, spec: &RunSpec, dir: &Path) -> Option<Coverage> {
    let file = spec.coverage_file.as_ref()?;
    let text = fs::read_to_string(file).ok()?;
    let format = coverage::detect_format(&text);
    match coverage::parse(&text, format, dir) {
        Ok(mut found) => {
            found.report_file = Some(PathValue::from(file.as_str()));
            Some(found)
        }
        Err(reason) => {
            ctx.emit(nurt_contracts::stage::Event::Log(
                nurt_contracts::stage::LogEvent {
                    level: nurt_contracts::stage::LogLevel::Warn,
                    message: format!("coverage report not read: {reason}"),
                    source: Some("test".to_owned()),
                },
            ));
            None
        }
    }
}

/// Earlier runs of the same project, newest first.
fn previous_runs(ctx: &ExecCtx, store: &dyn VerifyStore, dir: &str) -> Vec<StoredEntry> {
    let Ok(ids) = store.ids(ctx, EntryKind::TestRun) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for id in ids {
        if runs.len() >= COMPARE_DEPTH {
            break;
        }
        if let Ok(Some(entry)) = store.load(ctx, EntryKind::TestRun, &id)
            && entry.header.get("cwd").and_then(Value::as_str) == Some(dir)
        {
            runs.push(entry);
        }
    }
    runs
}

#[derive(Default)]
struct Comparison {
    baseline: Option<String>,
    regressions: Vec<String>,
    fixed: Vec<String>,
}

/// Compares each result with the latest earlier result of the same test.
/// The baseline is the earlier run that shared the largest number of tests.
fn compare(previous: &[StoredEntry], current: &[TestResult]) -> Comparison {
    let mut comparison = Comparison::default();
    let mut hits: HashMap<&str, usize> = HashMap::new();
    let statuses: Vec<HashMap<&str, Option<&str>>> = previous
        .iter()
        .map(|entry| {
            let mut known: HashMap<&str, Option<&str>> = HashMap::new();
            for record in &entry.items {
                if let Some(id) = record.get("id").and_then(Value::as_str) {
                    known
                        .entry(id)
                        .or_insert_with(|| record.get("status").and_then(Value::as_str));
                }
            }
            known
        })
        .collect();
    for case in current {
        for (entry, known) in previous.iter().zip(&statuses) {
            let Some(&earlier) = known.get(case.id.as_str()) else {
                continue;
            };
            *hits.entry(entry.id.as_str()).or_default() += 1;
            let now_failed = matches!(case.status, TestStatus::Failed | TestStatus::Error);
            if earlier == Some("passed") && now_failed {
                comparison.regressions.push(case.id.clone());
            }
            if matches!(earlier, Some("failed" | "error")) && case.status == TestStatus::Passed {
                comparison.fixed.push(case.id.clone());
            }
            break;
        }
    }
    comparison.baseline = previous
        .iter()
        .map(|entry| {
            (
                entry.id.as_str(),
                hits.get(entry.id.as_str()).copied().unwrap_or(0),
            )
        })
        .filter(|(_, count)| *count > 0)
        .max_by_key(|(_, count)| *count)
        .map(|(id, _)| id.to_owned());
    comparison
}

fn header_record(
    spec: &RunSpec,
    result: &ToolResult,
    analysis: &Analysis,
    comparison: &Comparison,
) -> Record {
    let exec = &result.exec;
    let count = |status: TestStatus| {
        analysis
            .results
            .iter()
            .filter(|case| case.status == status)
            .count() as u64
    };
    let framework = Framework::from_name(&spec.framework).unwrap_or(Framework::Custom);
    let todo = count(TestStatus::Todo);
    let run = TestRun {
        id: spec.run_id.clone(),
        framework: framework.record(),
        status: analysis.status,
        passed: count(TestStatus::Passed),
        failed: count(TestStatus::Failed),
        skipped: count(TestStatus::Skipped),
        errors: count(TestStatus::Error),
        duration: exec.elapsed,
        command: spec.argv.clone(),
        started: exec.started,
        cwd: PathValue::from(spec.dir.as_str()),
        todo: (todo > 0).then_some(todo),
        fingerprint: spec.fingerprint.clone(),
        targets: (!spec.targets.is_empty()).then(|| spec.targets.clone()),
        filter: spec.filter.clone(),
        exit_code: Some(exec.exit_code),
        output_file: result
            .stdout_file
            .as_ref()
            .map(|file| PathValue::from(file.to_string_lossy().as_ref())),
        retry_of: spec.retry_of.clone(),
        op: result.op,
        coverage: analysis.coverage.clone(),
        ..TestRun::default()
    };
    let mut record = Record::from(run);
    if let Some(baseline) = &comparison.baseline {
        record.set("baseline", Value::Text(baseline.clone()));
    }
    if !comparison.regressions.is_empty() {
        record.set("regressions", text_list(&comparison.regressions));
    }
    if !comparison.fixed.is_empty() {
        record.set("fixed", text_list(&comparison.fixed));
    }
    if spec.shape.slowest > 0 {
        let slowest = super::output::slowest(&analysis.results, spec.shape.slowest);
        if !slowest.is_empty() {
            record.set("slowest", text_list(&slowest));
        }
    }
    record
}

fn text_list(items: &[String]) -> Value {
    Value::List(items.iter().cloned().map(Value::Text).collect())
}
