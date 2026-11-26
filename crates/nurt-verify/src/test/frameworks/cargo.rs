// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
    if !request.only.is_empty() {
        argv.push("--".to_owned());
        argv.extend(
            request
                .only
                .iter()
                .map(|case| case.name.strip_suffix(SHOULD_PANIC_SUFFIX).unwrap_or(&case.name))
                .map(str::to_owned),
        );
        argv.push("--exact".to_owned());
    } else if let Some(filter) = request.filter {
        if filter.starts_with('-') {
            argv.push("--".to_owned());
        }
        argv.push(filter.to_owned());
    }
    CommandPlan {
        framework: Framework::Cargo,
        argv,
        report: None,
        coverage_file: None,
        env: Vec::new(),
        notes: if request.bail {
            vec![BAIL_NOTE.to_owned()]
        } else {
            Vec::new()
        },
    }
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
