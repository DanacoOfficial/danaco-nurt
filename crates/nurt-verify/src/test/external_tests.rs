// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::external_tests
// Responsibility:   Parsers on the output of installed test frameworks.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsers on the output of installed test frameworks. Every test needs its
//! framework in `PATH`, is skipped without it and runs only with the
//! `external-tools` feature.

use std::path::Path;
use std::process::Command;

use nurt_contracts::record::{TestResult, TestStatus};

use super::command::plan;
use super::frameworks::parse;
use super::{Framework, ParseInput, PlanRequest};
use crate::stage::test::support::write;

fn installed(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// Runs the framework in `dir` with the command of a plain `test.run` and
/// parses what it wrote.
fn run_and_parse(framework: Framework, dir: &Path) -> Vec<TestResult> {
    let state = tempfile::tempdir().unwrap();
    let report = state.path().join("report");
    let none = |_: &str| None;
    let planned = plan(&PlanRequest {
        framework,
        targets: &[],
        filter: None,
        bail: false,
        only: &[],
        report: &report,
        coverage: None,
        command: None,
        program: &none,
    });
    let output = Command::new(&planned.argv[0])
        .args(&planned.argv[1..])
        .envs(planned.env.iter().map(|(k, v)| (k, v)))
        .current_dir(dir)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let content = planned
        .report
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let input = ParseInput {
        output: &text,
        report: content.as_deref(),
        dir,
    };
    parse(framework, &input).unwrap_or_default()
}

fn status_of<'a>(results: &'a [TestResult], name: &str) -> Option<&'a TestStatus> {
    results
        .iter()
        .find(|result| result.name == name)
        .map(|result| &result.status)
}

#[test]
#[cfg_attr(
    not(feature = "external-tools"),
    ignore = "needs the external-tools feature"
)]
fn go_test_output_is_parsed_into_passed_and_failed_results() {
    if !installed("go") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "go.mod",
        "module example.com/sample\n\ngo 1.21\n",
    );
    write(
        dir.path(),
        "sum_test.go",
        "package sample\n\nimport \"testing\"\n\nfunc TestPasses(t *testing.T) {}\n\nfunc TestFails(t *testing.T) { t.Fatal(\"bad\") }\n",
    );
    let results = run_and_parse(Framework::Go, dir.path());
    assert_eq!(status_of(&results, "TestPasses"), Some(&TestStatus::Passed));
    assert_eq!(status_of(&results, "TestFails"), Some(&TestStatus::Failed));
}

#[test]
#[cfg_attr(
    not(feature = "external-tools"),
    ignore = "needs the external-tools feature"
)]
fn cargo_test_output_is_parsed_into_passed_and_failed_results() {
    if !installed("cargo") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        dir.path(),
        "src/lib.rs",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn passes() {}\n\n    #[test]\n    fn fails() {\n        panic!(\"bad\");\n    }\n}\n",
    );
    let results = run_and_parse(Framework::Cargo, dir.path());
    let status = |name: &str| {
        results
            .iter()
            .find(|result| result.name.ends_with(name))
            .map(|result| result.status)
    };
    assert_eq!(status("passes"), Some(TestStatus::Passed));
    assert_eq!(status("fails"), Some(TestStatus::Failed));
}

#[test]
#[cfg_attr(
    not(feature = "external-tools"),
    ignore = "needs the external-tools feature"
)]
fn pytest_report_is_parsed_into_passed_and_failed_results() {
    if !installed("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "test_sample.py",
        "def test_passes():\n    assert True\n\n\ndef test_fails():\n    assert False\n",
    );
    let results = run_and_parse(Framework::Pytest, dir.path());
    assert_eq!(
        status_of(&results, "test_passes"),
        Some(&TestStatus::Passed)
    );
    assert_eq!(status_of(&results, "test_fails"), Some(&TestStatus::Failed));
}

#[test]
#[cfg_attr(
    not(feature = "external-tools"),
    ignore = "needs the external-tools feature"
)]
fn bun_report_is_parsed_into_passed_and_failed_results() {
    if !installed("bun") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "sum.test.ts",
        "import { test, expect } from \"bun:test\";\ntest(\"passes\", () => expect(1).toBe(1));\ntest(\"fails\", () => expect(1).toBe(2));\n",
    );
    let results = run_and_parse(Framework::Bun, dir.path());
    assert_eq!(status_of(&results, "passes"), Some(&TestStatus::Passed));
    assert_eq!(status_of(&results, "fails"), Some(&TestStatus::Failed));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
