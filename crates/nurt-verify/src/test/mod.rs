// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test
// Responsibility:   Test framework detection, commands and output parsers.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test framework detection, commands and output parsers.

use std::path::Path;

use nurt_contracts::record::{TestFramework, TestResult};

pub(crate) mod command;
pub(crate) mod detect;
pub(crate) mod frameworks;
pub(crate) mod xml;

/// Test framework a run can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Framework {
    Bun,
    Vitest,
    Jest,
    Npm,
    Pytest,
    Go,
    Cargo,
    Custom,
}

impl Framework {
    /// Frameworks in the order of the `framework` parameter.
    pub(crate) const ALL: [Self; 8] = [
        Self::Bun,
        Self::Vitest,
        Self::Jest,
        Self::Npm,
        Self::Pytest,
        Self::Go,
        Self::Cargo,
        Self::Custom,
    ];

    /// Name used in parameters and in stored runs.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Vitest => "vitest",
            Self::Jest => "jest",
            Self::Npm => "npm",
            Self::Pytest => "pytest",
            Self::Go => "go",
            Self::Cargo => "cargo",
            Self::Custom => "custom",
        }
    }

    /// Framework with the given name.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|framework| framework.name() == name)
    }

    /// Framework as recorded in a `test_run`; scripts of the package manager
    /// have no framework of their own and are recorded as `custom`.
    pub(crate) const fn record(self) -> TestFramework {
        match self {
            Self::Bun => TestFramework::Bun,
            Self::Vitest => TestFramework::Vitest,
            Self::Jest => TestFramework::Jest,
            Self::Pytest => TestFramework::Pytest,
            Self::Go => TestFramework::Go,
            Self::Cargo => TestFramework::Cargo,
            Self::Npm | Self::Custom => TestFramework::Custom,
        }
    }
}

/// Everything a framework command needs to know about a run.
pub(crate) struct PlanRequest<'a> {
    pub(crate) framework: Framework,
    pub(crate) targets: &'a [String],
    pub(crate) filter: Option<&'a str>,
    pub(crate) bail: bool,
    /// Results to run again; empty runs the whole scope.
    pub(crate) only: &'a [TestResult],
    /// File the framework writes its machine-readable report to.
    pub(crate) report: &'a Path,
    /// File or directory the framework writes the coverage report to.
    pub(crate) coverage: Option<&'a Path>,
    /// User command, for `custom`.
    pub(crate) command: Option<&'a [String]>,
    /// Program to start for a tool name: the project-local installation when
    /// there is one.
    pub(crate) program: &'a dyn Fn(&str) -> Option<String>,
}

/// Command of a run and the files it produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandPlan {
    pub(crate) framework: Framework,
    pub(crate) argv: Vec<String>,
    /// Machine-readable report written by the framework.
    pub(crate) report: Option<String>,
    /// Coverage report that exists after the run.
    pub(crate) coverage_file: Option<String>,
    /// Variables set for the framework process.
    pub(crate) env: Vec<(String, String)>,
    /// Facts about the command that the user should know.
    pub(crate) notes: Vec<String>,
}

/// Input of an output parser.
pub(crate) struct ParseInput<'a> {
    /// Standard output followed by standard error.
    pub(crate) output: &'a str,
    /// Content of the machine-readable report, when the framework wrote one.
    pub(crate) report: Option<&'a str>,
    /// Project directory.
    pub(crate) dir: &'a Path,
}

/// Output that could not be read completely.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParseError {
    pub(crate) reason: String,
    /// Results read before the output became unreadable.
    pub(crate) partial: Vec<TestResult>,
}

#[cfg(test)]
mod corpus_tests;
#[cfg(test)]
mod external_tests;
#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
