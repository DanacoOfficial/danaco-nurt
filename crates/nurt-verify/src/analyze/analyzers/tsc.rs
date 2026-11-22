// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of `tsc --pretty false` output.

use std::path::Path;
use std::sync::LazyLock;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};

use crate::analyze::output::{Place, ToolOutput, finding, local_path};

/// Tool name in findings.
pub(crate) const TOOL: &str = "tsc";

#[allow(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests"
)]
static DIAGNOSTIC: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.+)$")
        .expect("constant pattern")
});

/// Diagnostics of the type checker.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    output
        .combined()
        .lines()
        .filter_map(|line| DIAGNOSTIC.captures(line.trim()))
        .map(|found| {
            let severity = if &found[4] == "warning" {
                FindingSeverity::Warning
            } else {
                FindingSeverity::Error
            };
            let place = Place::new(
                local_path(&found[1], dir),
                found[2].parse().unwrap_or(0),
                found[3].parse().unwrap_or(1),
            );
            finding(
                FindingKind::Types,
                TOOL,
                severity,
                place,
                &found[5],
                &found[6],
            )
        })
        .collect()
}
