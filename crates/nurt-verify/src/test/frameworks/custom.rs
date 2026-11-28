// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::frameworks::custom
// Responsibility:   Package scripts and commands given by the user.
// Dependencies:     nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Package scripts and commands given by the user.

use std::sync::LazyLock;

use nurt_contracts::record::{TestResult, TestStatus};
use regex::Regex;

use super::{milliseconds, unique_ids};
use crate::test::{CommandPlan, Framework, ParseInput, PlanRequest};

const RERUN_NOTE: &str = "this framework does not select single tests: the whole scope runs again";
const COVERAGE_NOTE: &str = "coverage is not measured for this framework: give coverage_file";

/// Line of the Test Anything Protocol.
static TAP_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(not ok|ok)\s+\d+\s*-?\s*(.*?)\s*(#\s*(skip|todo)\b.*)?$").ok()
});

/// Line of a test marked by its outcome, as bun prints them.
static MARKED_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^\((pass|fail|skip|todo)\)\s+(.*?)(?:\s+\[([0-9.]+)ms\])?$").ok()
});

/// Command of a run: the user command as given, or the `test` script of the
/// package.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    let mut notes = Vec::new();
    if !request.only.is_empty() {
        notes.push(RERUN_NOTE.to_owned());
    }
    if request.coverage.is_some() {
        notes.push(COVERAGE_NOTE.to_owned());
    }
    let argv = if request.framework == Framework::Custom {
        request.command.map(<[String]>::to_vec).unwrap_or_default()
    } else {
        let mut argv = vec!["npm".to_owned(), "test".to_owned(), "--silent".to_owned()];
        if !request.targets.is_empty() {
            argv.push("--".to_owned());
            argv.extend(request.targets.iter().cloned());
        }
        argv
    };
    CommandPlan {
        framework: request.framework,
        argv,
        report: None,
        coverage_file: None,
        env: Vec::new(),
        notes,
    }
}

/// Reads TAP lines and lines marked `(pass)`, `(fail)`, `(skip)` or
/// `(todo)`. Other lines are ignored.
///
/// A TAP point of a subtest is indented below the point of its parent, which
/// follows it; only the points of the shallowest indentation are results,
/// so that a test is not counted with its parent.
pub(crate) fn parse(input: &ParseInput<'_>) -> Vec<TestResult> {
    let mut found: Vec<(Option<usize>, TestResult)> = Vec::new();
    for line in input.output.lines() {
        if let Some(result) = tap_result(line) {
            found.push((Some(line.len() - line.trim_start().len()), result));
        } else if let Some(result) = marked_result(line) {
            found.push((None, result));
        }
    }
    let shallowest = found.iter().filter_map(|(indent, _)| *indent).min();
    let mut results: Vec<TestResult> = found
        .into_iter()
        .filter(|(indent, _)| indent.is_none_or(|indent| Some(indent) == shallowest))
        .map(|(_, result)| result)
        .collect();
    unique_ids(&mut results);
    results
}

fn tap_result(line: &str) -> Option<TestResult> {
    let captures = TAP_LINE.as_ref()?.captures(line)?;
    let directive = captures
        .get(4)
        .map(|found| found.as_str().to_ascii_lowercase());
    let status = match directive.as_deref() {
        Some("skip") => TestStatus::Skipped,
        Some(_) => TestStatus::Todo,
        None if captures[1].eq_ignore_ascii_case("ok") => TestStatus::Passed,
        None => TestStatus::Failed,
    };
    Some(result(&captures[2], status, None))
}

fn marked_result(line: &str) -> Option<TestResult> {
    let captures = MARKED_LINE.as_ref()?.captures(line)?;
    let status = match &captures[1] {
        "pass" => TestStatus::Passed,
        "fail" => TestStatus::Failed,
        "skip" => TestStatus::Skipped,
        _ => TestStatus::Todo,
    };
    let time = captures
        .get(3)
        .and_then(|found| found.as_str().parse::<f64>().ok());
    Some(result(&captures[2], status, time))
}

fn result(name: &str, status: TestStatus, millis: Option<f64>) -> TestResult {
    TestResult {
        id: name.to_owned(),
        name: name.to_owned(),
        status,
        duration: millis.and_then(milliseconds),
        ..TestResult::default()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
