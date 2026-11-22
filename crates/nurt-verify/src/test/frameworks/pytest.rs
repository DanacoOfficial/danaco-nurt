// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The pytest framework.

use nurt_contracts::record::TestResult;

use super::junit::{self, Flavor};
use super::{custom, make_id};
use crate::test::command::path_text;
use crate::test::{CommandPlan, Framework, ParseError, ParseInput, PlanRequest};

/// Command of a run: JUnit report in the xunit2 family, name expression,
/// bail and LCOV coverage through pytest-cov.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    let mut argv: Vec<String> = match (request.program)("pytest") {
        Some(program) => vec![program],
        None => vec!["python3".to_owned(), "-m".to_owned(), "pytest".to_owned()],
    };
    argv.push("-q".to_owned());
    argv.push(format!("--junitxml={}", path_text(request.report)));
    argv.push("-o".to_owned());
    argv.push("junit_family=xunit2".to_owned());
    if request.only.is_empty() {
        argv.extend(request.targets.iter().cloned());
        if let Some(filter) = request.filter {
            argv.push("-k".to_owned());
            argv.push(filter.to_owned());
        }
    } else {
        for node in nodes(request.only) {
            argv.push(node);
        }
    }
    if request.bail {
        argv.push("-x".to_owned());
    }
    let mut env = Vec::new();
    let mut coverage_file = None;
    if let Some(file) = request.coverage {
        let file = path_text(file);
        argv.push("--cov=.".to_owned());
        argv.push(format!("--cov-report=lcov:{file}"));
        env.push(("COVERAGE_FILE".to_owned(), format!("{file}.data")));
        coverage_file = Some(file);
    }
    CommandPlan {
        framework: Framework::Pytest,
        argv,
        report: Some(path_text(request.report)),
        coverage_file,
        env,
        notes: Vec::new(),
    }
}

/// Node identifiers of the results: file, classes and test name.
fn nodes(only: &[TestResult]) -> Vec<String> {
    let mut nodes: Vec<String> = Vec::new();
    for case in only {
        let path = case.path.as_ref().map_or("", |path| path.as_str());
        let node = make_id(&[path, case.suite.as_deref().unwrap_or_default(), &case.name]);
        if !nodes.contains(&node) {
            nodes.push(node);
        }
    }
    nodes
}

/// Reads the JUnit report.
pub(crate) fn parse(input: &ParseInput<'_>) -> Result<Vec<TestResult>, ParseError> {
    match input.report {
        Some(report) => junit::parse(report, Flavor::Pytest, Some(input.dir)),
        None => Ok(custom::parse(input)),
    }
}
