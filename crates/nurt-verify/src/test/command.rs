// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Commands of the test frameworks.

use std::path::Path;

use nurt_contracts::record::TestResult;

use super::frameworks::{bun, cargo, custom, go, jest, pytest, vitest};
use super::{CommandPlan, Framework, PlanRequest};

/// Characters that are special in the patterns frameworks accept for test
/// names.
const PATTERN_SPECIAL: &str = ".*+?^${}()|[]\\";

/// Builds the command of a run with a machine-readable report written to
/// the files named in the request.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    match request.framework {
        Framework::Bun => bun::plan(request),
        Framework::Vitest => vitest::plan(request),
        Framework::Jest => jest::plan(request),
        Framework::Pytest => pytest::plan(request),
        Framework::Go => go::plan(request),
        Framework::Cargo => cargo::plan(request),
        Framework::Npm | Framework::Custom => custom::plan(request),
    }
}

/// Escapes the characters of a test name that a name pattern would read as
/// syntax.
pub(crate) fn escape_pattern(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if PATTERN_SPECIAL.contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Pattern that matches the names of the results exactly at the end of a
/// full test name.
pub(crate) fn names_pattern(cases: &[&TestResult]) -> String {
    let mut alternatives: Vec<String> = Vec::new();
    for case in cases {
        let escaped = escape_pattern(&case.name);
        if !alternatives.contains(&escaped) {
            alternatives.push(escaped);
        }
    }
    format!("(?:{})$", alternatives.join("|"))
}

/// Distinct files of the results, in order of first occurrence.
pub(crate) fn files_of(cases: &[TestResult]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for case in cases {
        if let Some(path) = case.path.as_ref().map(|path| path.as_str().to_owned())
            && !path.is_empty()
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    files
}

/// Text of a path for a command line.
pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
