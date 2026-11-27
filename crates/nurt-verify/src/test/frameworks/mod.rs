// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::frameworks
// Responsibility:   Commands and output parsers of the test frameworks.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Commands and output parsers of the test frameworks.

use std::collections::HashMap;

use nurt_contracts::record::TestResult;
use nurt_contracts::value::{Duration, Path};

use super::{Framework, ParseError, ParseInput};

pub(crate) mod bun;
pub(crate) mod cargo;
pub(crate) mod custom;
pub(crate) mod go;
pub(crate) mod jest;
pub(crate) mod junit;
pub(crate) mod pytest;
pub(crate) mod vitest;

/// Separator of the parts of a test identifier.
pub(crate) const ID_SEPARATOR: &str = "::";

/// Separator of nested groups in the suite of a result.
pub(crate) const GROUP_SEPARATOR: &str = " > ";

/// Name of the result of a test file that could not be loaded.
pub(crate) const FILE_MARKER: &str = "(file)";

/// Name of the result of a package that failed as a whole.
pub(crate) const PACKAGE_MARKER: &str = "(package)";

/// Name of the LCOV file that script test runners write into the coverage
/// directory.
const LCOV_FILE_NAME: &str = "lcov.info";

/// Coverage report of a script test runner in its coverage directory.
pub(crate) fn js_coverage_file(directory: &std::path::Path) -> String {
    directory
        .join(LCOV_FILE_NAME)
        .to_string_lossy()
        .into_owned()
}

/// Reads the results of a run from the output of the framework.
///
/// Fails with the results read so far when the output is not complete or
/// not in the expected format.
pub(crate) fn parse(
    framework: Framework,
    input: &ParseInput<'_>,
) -> std::result::Result<Vec<TestResult>, ParseError> {
    match framework {
        Framework::Bun => bun::parse(input),
        Framework::Vitest => vitest::parse(input),
        Framework::Jest => jest::parse(input),
        Framework::Pytest => pytest::parse(input),
        Framework::Go => go::parse(input),
        Framework::Cargo => cargo::parse(input),
        Framework::Npm | Framework::Custom => Ok(custom::parse(input)),
    }
}

/// Stable identifier of a test: the non-empty parts joined.
pub(crate) fn make_id(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(ID_SEPARATOR)
}

/// Gives repeated identifiers a numeric suffix, in order of occurrence.
pub(crate) fn unique_ids(results: &mut [TestResult]) {
    let mut seen: HashMap<String, u64> = HashMap::new();
    for result in results {
        let count = seen.entry(result.id.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            result.id = format!("{}#{}", result.id, count);
        }
    }
}

/// Duration of a time given in seconds.
pub(crate) fn seconds(value: f64) -> Option<Duration> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(Duration::from_millis(millis(value * 1000.0)))
}

/// Duration of a time given in milliseconds.
pub(crate) fn milliseconds(value: f64) -> Option<Duration> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(Duration::from_millis(millis(value)))
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "test durations are far below the range of i64 milliseconds"
)]
fn millis(value: f64) -> i64 {
    value.round() as i64
}

/// Path value of a file name; `None` for an empty name.
pub(crate) fn path_of(file: &str) -> Option<Path> {
    if file.is_empty() {
        None
    } else {
        Some(Path::from(file))
    }
}

/// Text that is `None` when blank.
pub(crate) fn non_blank(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
