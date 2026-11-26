// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parsers of `ruff check --output-format=json` and `ruff format --check`.

use std::path::Path;
use std::sync::LazyLock;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use serde::Deserialize;

use crate::analyze::output::{Place, ToolOutput, file_list_findings, finding, local_path};

/// Tool name in findings.
pub(crate) const TOOL: &str = "ruff";

/// Rule code reported for a file that does not parse.
const SYNTAX_CODE: &str = "syntax";

/// Rule code prefixes that are reported as errors.
const ERROR_PREFIXES: [&str; 3] = ["F", "E9", SYNTAX_CODE];

#[allow(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests"
)]
static UNFORMATTED: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^(?:Would reformat:\s*(.+)|-->\s*(.+?):\d+:\d+)$")
        .expect("constant pattern")
});

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    location: Location,
}

#[derive(Deserialize, Default)]
struct Location {
    #[serde(default)]
    row: u64,
    #[serde(default)]
    column: u64,
}

/// Diagnostics of the linter; empty when the output is not a JSON report.
pub(crate) fn parse_lint(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    let Ok(report) = serde_json::from_str::<Vec<Item>>(&output.stdout) else {
        return Vec::new();
    };
    report
        .into_iter()
        .map(|item| {
            let code = item.code.unwrap_or_else(|| SYNTAX_CODE.to_owned());
            let severity = if is_error_code(&code) {
                FindingSeverity::Error
            } else {
                FindingSeverity::Warning
            };
            let place = Place::new(
                local_path(&item.filename, dir),
                item.location.row,
                item.location.column,
            );
            finding(
                FindingKind::Lint,
                TOOL,
                severity,
                place,
                &code,
                &item.message,
            )
        })
        .collect()
}

/// Whether a rule code belongs to a class that is reported as an error: the
/// code is the prefix followed by digits only, so `F401` counts and the
/// `FBT001` rule of another class does not.
fn is_error_code(code: &str) -> bool {
    ERROR_PREFIXES.iter().any(|prefix| {
        code.strip_prefix(prefix)
            .is_some_and(|rest| rest.chars().next().is_none_or(|first| first.is_ascii_digit()))
    })
}

/// Files the formatter would change.
pub(crate) fn parse_format(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    file_list_findings(TOOL, output, dir, Some(&UNFORMATTED))
}
