// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::frameworks::vitest
// Responsibility:   The command plan of a test run of Vitest and the parsing of its JUnit report.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Vitest.

use nurt_contracts::record::TestResult;

use super::junit::{self, Flavor};
use super::{custom, js_coverage_file};
use crate::test::command::{files_of, names_pattern, path_text};
use crate::test::{CommandPlan, Framework, ParseError, ParseInput, PlanRequest};

/// Command of a run: JUnit report, name pattern, bail and LCOV coverage.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    let program = (request.program)("vitest").unwrap_or_else(|| "vitest".to_owned());
    let mut argv = vec![program, "run".to_owned()];
    if request.only.is_empty() {
        argv.extend(request.targets.iter().cloned());
    } else {
        argv.extend(files_of(request.only));
    }
    argv.push("--reporter=junit".to_owned());
    argv.push(format!("--outputFile={}", path_text(request.report)));
    if !request.only.is_empty() {
        let only: Vec<&TestResult> = request.only.iter().collect();
        argv.push("-t".to_owned());
        argv.push(names_pattern(&only));
    } else if let Some(filter) = request.filter {
        argv.push("-t".to_owned());
        argv.push(filter.to_owned());
    }
    if request.bail {
        argv.push("--bail=1".to_owned());
    }
    if let Some(directory) = request.coverage {
        argv.push("--coverage".to_owned());
        argv.push("--coverage.reporter=lcov".to_owned());
        argv.push(format!(
            "--coverage.reportsDirectory={}",
            path_text(directory)
        ));
    }
    CommandPlan {
        framework: Framework::Vitest,
        argv,
        report: Some(path_text(request.report)),
        coverage_file: request.coverage.map(js_coverage_file),
        env: Vec::new(),
        notes: Vec::new(),
    }
}

/// Reads the JUnit report; without a report, the lines the runner prints.
pub(crate) fn parse(input: &ParseInput<'_>) -> Result<Vec<TestResult>, ParseError> {
    match input.report {
        Some(report) => junit::parse(report, Flavor::Vitest, None),
        None => Ok(custom::parse(input)),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
