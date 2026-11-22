// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Jest.

use std::path::Path;
use std::sync::LazyLock;

use nurt_contracts::record::{TestResult, TestStatus};
use regex::Regex;
use serde::Deserialize;

use super::{
    FILE_MARKER, GROUP_SEPARATOR, js_coverage_file, make_id, milliseconds, non_blank, path_of,
    unique_ids,
};
use crate::test::command::{files_of, names_pattern, path_text};
use crate::test::{CommandPlan, Framework, ParseError, ParseInput, PlanRequest};

/// Stack frames of dependencies and of the runtime, which hide the frame of
/// the test.
static NOISE_FRAME: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s+at .*(?:node_modules/|\(node:|<anonymous>\)$)").ok());

/// First line of a stack frame.
static FRAME: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\s+at ").ok());

/// Command of a run: JSON report, name pattern, bail and LCOV coverage.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    let program = (request.program)("jest").unwrap_or_else(|| "jest".to_owned());
    let mut argv = vec![program];
    if request.only.is_empty() {
        argv.extend(request.targets.iter().cloned());
    } else {
        argv.extend(files_of(request.only));
    }
    argv.push("--json".to_owned());
    argv.push(format!("--outputFile={}", path_text(request.report)));
    let named: Vec<&TestResult> = request
        .only
        .iter()
        .filter(|case| case.name != FILE_MARKER)
        .collect();
    if !named.is_empty() {
        argv.push("-t".to_owned());
        argv.push(names_pattern(&named));
    } else if request.only.is_empty()
        && let Some(filter) = request.filter
    {
        argv.push("-t".to_owned());
        argv.push(filter.to_owned());
    }
    if request.bail {
        argv.push("--bail".to_owned());
    }
    if let Some(directory) = request.coverage {
        argv.push("--coverage".to_owned());
        argv.push("--coverageReporters=lcov".to_owned());
        argv.push(format!("--coverageDirectory={}", path_text(directory)));
    }
    CommandPlan {
        framework: Framework::Jest,
        argv,
        report: Some(path_text(request.report)),
        coverage_file: request.coverage.map(js_coverage_file),
        env: Vec::new(),
        notes: Vec::new(),
    }
}

#[derive(Deserialize, Default)]
struct Report {
    #[serde(rename = "testResults", default)]
    test_results: Vec<Suite>,
}

#[derive(Deserialize, Default)]
struct Suite {
    #[serde(default)]
    name: String,
    #[serde(default)]
    message: String,
    #[serde(rename = "assertionResults", default)]
    assertions: Vec<Assertion>,
}

#[derive(Deserialize, Default)]
struct Assertion {
    #[serde(default)]
    title: String,
    #[serde(rename = "fullName", default)]
    full_name: String,
    #[serde(rename = "ancestorTitles", default)]
    ancestors: Vec<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(rename = "failureMessages", default)]
    failures: Vec<String>,
    #[serde(default)]
    location: Option<Location>,
}

#[derive(Deserialize, Default)]
struct Location {
    #[serde(default)]
    line: Option<u64>,
    #[serde(default)]
    column: Option<u64>,
}

/// Reads the JSON report. Without a report there is nothing to read.
pub(crate) fn parse(input: &ParseInput<'_>) -> Result<Vec<TestResult>, ParseError> {
    let Some(text) = input.report else {
        return Ok(Vec::new());
    };
    let report: Report = serde_json::from_str(text).map_err(|failure| ParseError {
        reason: format!("the report is not valid JSON: {failure}"),
        partial: Vec::new(),
    })?;
    let mut results: Vec<TestResult> = report
        .test_results
        .iter()
        .flat_map(|suite| suite_results(suite, input.dir))
        .collect();
    unique_ids(&mut results);
    Ok(results)
}

fn suite_results(suite: &Suite, dir: &Path) -> Vec<TestResult> {
    let file = relative(&suite.name, dir);
    let message = clean(&suite.message, dir);
    if suite.assertions.is_empty() && !message.is_empty() {
        let (text, stack) = split_stack(&message);
        return vec![TestResult {
            id: make_id(&[&file, FILE_MARKER]),
            name: FILE_MARKER.to_owned(),
            status: TestStatus::Error,
            path: path_of(&file),
            message: non_blank(&text),
            stack,
            ..TestResult::default()
        }];
    }
    suite
        .assertions
        .iter()
        .map(|assertion| assertion_result(assertion, &file, dir))
        .collect()
}

fn assertion_result(assertion: &Assertion, file: &str, dir: &Path) -> TestResult {
    let group = assertion.ancestors.join(GROUP_SEPARATOR);
    let name = if assertion.title.is_empty() {
        assertion.full_name.clone()
    } else {
        assertion.title.clone()
    };
    let status = match assertion.status.as_str() {
        "passed" => TestStatus::Passed,
        "failed" => TestStatus::Failed,
        "todo" => TestStatus::Todo,
        _ => TestStatus::Skipped,
    };
    let (message, stack) = if assertion.failures.is_empty() {
        (None, None)
    } else {
        let joined = clean(&assertion.failures.join("\n"), dir);
        let (text, stack) = split_stack(&joined);
        (non_blank(&text), stack)
    };
    TestResult {
        id: make_id(&[file, &group, &name]),
        name,
        status,
        suite: non_blank(&group),
        path: path_of(file),
        duration: assertion.duration.and_then(milliseconds),
        message,
        stack,
        line: assertion
            .location
            .as_ref()
            .and_then(|location| location.line),
        column: assertion
            .location
            .as_ref()
            .and_then(|location| location.column),
        ..TestResult::default()
    }
}

fn relative(name: &str, dir: &Path) -> String {
    let prefix = format!("{}/", dir.to_string_lossy().trim_end_matches('/'));
    name.strip_prefix(&prefix).unwrap_or(name).to_owned()
}

/// Message without the frames of dependencies and with paths relative to the
/// project directory.
fn clean(text: &str, dir: &Path) -> String {
    let noise = NOISE_FRAME.as_ref();
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !noise.is_some_and(|noise| noise.is_match(line)))
        .collect();
    let prefix = format!("{}/", dir.to_string_lossy().trim_end_matches('/'));
    kept.join("\n").replace(&prefix, "").trim().to_owned()
}

/// Splits a failure text into the message and the stack that starts at the
/// first frame.
fn split_stack(text: &str) -> (String, Option<String>) {
    let frame = FRAME.as_ref();
    let lines: Vec<&str> = text.lines().collect();
    let first = lines
        .iter()
        .position(|line| frame.is_some_and(|frame| frame.is_match(line)));
    match first {
        Some(index) => (
            lines[..index].join("\n").trim().to_owned(),
            non_blank(&lines[index..].join("\n")),
        ),
        None => (text.trim().to_owned(), None),
    }
}
