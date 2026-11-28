// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::frameworks::cargo
// Responsibility:   Cargo tests.
// Dependencies:     nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Cargo tests.

use std::collections::HashMap;
use std::sync::LazyLock;

use nurt_contracts::record::{TestResult, TestStatus};
use regex::Regex;

use super::{non_blank, unique_ids};
use crate::test::command::path_text;
use crate::test::{CommandPlan, Framework, ParseError, ParseInput, PlanRequest};

/// Note on `bail`: the test runner of cargo has no per-test equivalent.
const BAIL_NOTE: &str = "cargo stops after the first failing test target by itself";

/// Note on names that cannot be selected: the harness reads them as options.
const OPTION_NAME_NOTE: &str =
    "tests whose names start with `-` cannot be selected by name and were not run again";

/// Suffix the test harness appends to the name of a `should_panic` test in
/// its result line; it is not a part of the name the harness selects by.
const SHOULD_PANIC_SUFFIX: &str = " - should panic";

static RESULT_LINE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^test (.+?) \.\.\. (ok|FAILED|ignored)(?:, .*)?$").ok());
static BLOCK_START: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^---- (.+?) stdout ----$").ok());

/// Command of a run. Coverage replaces `cargo test` by `cargo llvm-cov`.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    if let Some(file) = request.coverage {
        let file = path_text(file);
        return CommandPlan {
            framework: Framework::Cargo,
            argv: ["cargo", "llvm-cov", "--lcov", "--output-path", &file]
                .iter()
                .map(|part| (*part).to_owned())
                .collect(),
            report: None,
            coverage_file: Some(file),
            env: Vec::new(),
            notes: Vec::new(),
        };
    }
    let mut argv = vec!["cargo".to_owned(), "test".to_owned()];
    argv.extend(request.targets.iter().cloned());
    let mut notes = Vec::new();
    if !request.only.is_empty() {
        let (names, skipped) = rerun_names(request);
        argv.push("--".to_owned());
        argv.extend(names);
        argv.push("--exact".to_owned());
        if skipped {
            notes.push(OPTION_NAME_NOTE.to_owned());
        }
    } else if let Some(filter) = request.filter {
        if filter.starts_with('-') {
            argv.push("--".to_owned());
        }
        argv.push(filter.to_owned());
    }
    if request.bail {
        notes.push(BAIL_NOTE.to_owned());
    }
    CommandPlan {
        framework: Framework::Cargo,
        argv,
        report: None,
        coverage_file: None,
        env: Vec::new(),
        notes,
    }
}

/// Names the harness selects the results of a rerun by, and whether a name
/// was left out because it would be read as an option of the harness. When
/// no name is left, the empty name selects no test, never all of them.
fn rerun_names(request: &PlanRequest<'_>) -> (Vec<String>, bool) {
    let mut names: Vec<String> = request
        .only
        .iter()
        .map(|case| {
            case.name
                .strip_suffix(SHOULD_PANIC_SUFFIX)
                .unwrap_or(&case.name)
        })
        .filter(|name| !name.starts_with('-'))
        .map(str::to_owned)
        .collect();
    let skipped = names.len() < request.only.len();
    if names.is_empty() {
        names.push(String::new());
    }
    (names, skipped)
}

/// Reads the text output of the test harness.
pub(crate) fn parse(input: &ParseInput<'_>) -> Result<Vec<TestResult>, ParseError> {
    let (Some(result_line), Some(block_start)) = (RESULT_LINE.as_ref(), BLOCK_START.as_ref())
    else {
        return Ok(Vec::new());
    };
    let blocks = failure_blocks(input.output, block_start);
    let mut results = Vec::new();
    for line in input.output.lines() {
        let Some(captures) = result_line.captures(line) else {
            continue;
        };
        let name = captures[1].to_owned();
        let status = match &captures[2] {
            "ok" => TestStatus::Passed,
            "FAILED" => TestStatus::Failed,
            _ => TestStatus::Skipped,
        };
        let message = if status == TestStatus::Failed {
            blocks.get(&name).and_then(|text| non_blank(text))
        } else {
            None
        };
        results.push(TestResult {
            id: name.clone(),
            name,
            status,
            message,
            ..TestResult::default()
        });
    }
    unique_ids(&mut results);
    Ok(results)
}

/// Captured output of the failed tests, keyed by test name.
fn failure_blocks(output: &str, block_start: &Regex) -> HashMap<String, String> {
    let mut blocks = HashMap::new();
    let mut current: Option<String> = None;
    let mut body = String::new();
    for line in output.lines() {
        if let Some(captures) = block_start.captures(line) {
            store(&mut blocks, current.take(), &mut body);
            current = Some(captures[1].to_owned());
        } else if line.starts_with("failures:") || line.starts_with("test result:") {
            store(&mut blocks, current.take(), &mut body);
        } else if current.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    store(&mut blocks, current.take(), &mut body);
    blocks
}

fn store(blocks: &mut HashMap<String, String>, name: Option<String>, body: &mut String) {
    let text = std::mem::take(body);
    if let Some(name) = name {
        blocks.entry(name).or_insert(text);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
