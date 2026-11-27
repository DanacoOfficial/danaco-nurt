// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::corpus_tests
// Responsibility:   Parsers on stored outputs of every framework in every scenario.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsers on stored outputs of every framework in every scenario.

use std::path::Path;

use nurt_contracts::record::{TestResult, TestStatus};

use super::frameworks::parse;
use super::{Framework, ParseError, ParseInput};
use crate::stage::test::support::corpus;

/// Where a parser reads a stored output from.
#[derive(Clone, Copy)]
enum Source {
    Report,
    Output,
}

/// What a stored output must yield.
#[derive(Clone, Copy, Debug)]
enum Expect {
    /// Only passing tests, at least one.
    Passed,
    /// Passing and failing tests.
    Mixed,
    /// No results.
    Empty,
    /// At least one result with the status `error`.
    Errors,
    /// A parse error that keeps the results read so far.
    Unreadable,
}

struct Case {
    framework: Framework,
    file: &'static str,
    source: Source,
    expect: Expect,
}

const fn case(framework: Framework, file: &'static str, source: Source, expect: Expect) -> Case {
    Case {
        framework,
        file,
        source,
        expect,
    }
}

const CASES: [Case; 28] = [
    case(
        Framework::Bun,
        "bun/1.4.2/passed.xml",
        Source::Report,
        Expect::Passed,
    ),
    case(
        Framework::Bun,
        "bun/1.4.2/mixed.xml",
        Source::Report,
        Expect::Mixed,
    ),
    case(
        Framework::Bun,
        "bun/1.4.2/no_tests.xml",
        Source::Report,
        Expect::Empty,
    ),
    case(
        Framework::Bun,
        "bun/1.4.2/collect_error.txt",
        Source::Output,
        Expect::Empty,
    ),
    case(
        Framework::Bun,
        "bun/1.4.2/truncated.xml",
        Source::Report,
        Expect::Unreadable,
    ),
    case(
        Framework::Vitest,
        "vitest/5.0.3/passed.xml",
        Source::Report,
        Expect::Passed,
    ),
    case(
        Framework::Vitest,
        "vitest/5.0.3/mixed.xml",
        Source::Report,
        Expect::Mixed,
    ),
    case(
        Framework::Vitest,
        "vitest/5.0.3/no_tests.xml",
        Source::Report,
        Expect::Empty,
    ),
    case(
        Framework::Vitest,
        "vitest/5.0.3/collect_error.txt",
        Source::Output,
        Expect::Empty,
    ),
    case(
        Framework::Vitest,
        "vitest/5.0.3/truncated.xml",
        Source::Report,
        Expect::Unreadable,
    ),
    case(
        Framework::Jest,
        "jest/30.5.2/passed.json",
        Source::Report,
        Expect::Passed,
    ),
    case(
        Framework::Jest,
        "jest/30.5.2/mixed.json",
        Source::Report,
        Expect::Mixed,
    ),
    case(
        Framework::Jest,
        "jest/30.5.2/no_tests.json",
        Source::Report,
        Expect::Empty,
    ),
    case(
        Framework::Jest,
        "jest/30.5.2/collect_error.json",
        Source::Report,
        Expect::Errors,
    ),
    case(
        Framework::Jest,
        "jest/30.5.2/truncated.json",
        Source::Report,
        Expect::Unreadable,
    ),
    case(
        Framework::Pytest,
        "pytest/9.1.1/passed.xml",
        Source::Report,
        Expect::Passed,
    ),
    case(
        Framework::Pytest,
        "pytest/9.1.1/mixed.xml",
        Source::Report,
        Expect::Mixed,
    ),
    case(
        Framework::Pytest,
        "pytest/9.1.1/no_tests.xml",
        Source::Report,
        Expect::Empty,
    ),
    case(
        Framework::Pytest,
        "pytest/9.1.1/collect_error.xml",
        Source::Report,
        Expect::Errors,
    ),
    case(
        Framework::Pytest,
        "pytest/9.1.1/truncated.xml",
        Source::Report,
        Expect::Unreadable,
    ),
    case(
        Framework::Go,
        "go/1.26.0/passed.jsonl",
        Source::Output,
        Expect::Passed,
    ),
    case(
        Framework::Go,
        "go/1.26.0/mixed.jsonl",
        Source::Output,
        Expect::Mixed,
    ),
    case(
        Framework::Go,
        "go/1.26.0/no_tests.jsonl",
        Source::Output,
        Expect::Empty,
    ),
    case(
        Framework::Go,
        "go/1.26.0/truncated.jsonl",
        Source::Output,
        Expect::Errors,
    ),
    case(
        Framework::Cargo,
        "cargo/1.98.1/passed.txt",
        Source::Output,
        Expect::Passed,
    ),
    case(
        Framework::Cargo,
        "cargo/1.98.1/mixed.txt",
        Source::Output,
        Expect::Mixed,
    ),
    case(
        Framework::Cargo,
        "cargo/1.98.1/no_tests.txt",
        Source::Output,
        Expect::Empty,
    ),
    case(
        Framework::Cargo,
        "cargo/1.98.1/build_error.txt",
        Source::Output,
        Expect::Empty,
    ),
];

fn count(results: &[TestResult], status: TestStatus) -> usize {
    results
        .iter()
        .filter(|result| result.status == status)
        .count()
}

/// Describes how the parsed output differs from the expectation.
fn mismatch(expect: Expect, outcome: &Result<Vec<TestResult>, ParseError>) -> Option<String> {
    let verdict = |ok: bool, what: &str| (!ok).then(|| what.to_owned());
    match (expect, outcome) {
        (Expect::Unreadable, Err(error)) => verdict(!error.reason.is_empty(), "empty reason"),
        (Expect::Unreadable, Ok(_)) => Some("parsed without error".to_owned()),
        (_, Err(error)) => Some(format!("parse error: {}", error.reason)),
        (Expect::Passed, Ok(results)) => verdict(
            !results.is_empty() && count(results, TestStatus::Passed) == results.len(),
            "not only passed results",
        ),
        (Expect::Mixed, Ok(results)) => verdict(
            count(results, TestStatus::Passed) > 0 && count(results, TestStatus::Failed) > 0,
            "no passed and failed results",
        ),
        (Expect::Empty, Ok(results)) => verdict(results.is_empty(), "unexpected results"),
        (Expect::Errors, Ok(results)) => verdict(
            count(results, TestStatus::Error) > 0,
            "no result with the status error",
        ),
    }
}

#[test]
fn stored_outputs_of_every_framework_and_scenario_parse_as_expected() {
    let project = tempfile::tempdir().unwrap();
    let mut problems = Vec::new();
    for case in &CASES {
        let content = corpus(case.file);
        let input = match case.source {
            Source::Report => ParseInput {
                output: "",
                report: Some(&content),
                dir: project.path(),
            },
            Source::Output => ParseInput {
                output: &content,
                report: None,
                dir: Path::new(project.path()),
            },
        };
        let outcome = parse(case.framework, &input);
        if let Some(problem) = mismatch(case.expect, &outcome) {
            problems.push(format!("{} ({:?}): {problem}", case.file, case.expect));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn stored_outputs_parse_to_the_same_results_every_time() {
    let project = tempfile::tempdir().unwrap();
    for case in CASES
        .iter()
        .filter(|case| matches!(case.expect, Expect::Mixed))
    {
        let content = corpus(case.file);
        let (output, report) = match case.source {
            Source::Report => ("", Some(content.as_str())),
            Source::Output => (content.as_str(), None),
        };
        let input = ParseInput {
            output,
            report,
            dir: project.path(),
        };
        let first = parse(case.framework, &input).unwrap_or_default();
        let second = parse(case.framework, &input).unwrap_or_default();
        assert_eq!(first, second, "{}", case.file);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
