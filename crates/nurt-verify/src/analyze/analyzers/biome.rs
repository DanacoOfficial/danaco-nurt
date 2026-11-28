// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::analyzers::biome
// Responsibility:   Parser of the `github` reporter of `biome lint` and `biome format`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of the `github` reporter of `biome lint` and `biome format`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};

use crate::analyze::output::{Place, ToolOutput, finding, local_path, unformatted};

/// Tool name in findings.
pub(crate) const TOOL: &str = "biome";

#[allow(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests"
)]
static ANNOTATION: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^::(error|warning|notice) (.*?)::(.*)$").expect("constant pattern")
});

/// Properties of an annotation: `title=rule,file=path,line=1,col=2`.
fn properties(text: &str) -> HashMap<&str, &str> {
    text.split(',')
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (key.trim(), value))
        .collect()
}

fn annotations(output: &ToolOutput) -> Vec<(String, String, String)> {
    output
        .combined()
        .lines()
        .filter_map(|line| ANNOTATION.captures(line.trim()))
        .map(|found| {
            (
                found[1].to_owned(),
                found[2].to_owned(),
                found[3].to_owned(),
            )
        })
        .collect()
}

/// Diagnostics of the linter.
pub(crate) fn parse_lint(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    annotations(output)
        .into_iter()
        .map(|(level, props, message)| {
            let fields = properties(&props);
            let severity = match level.as_str() {
                "error" => FindingSeverity::Error,
                "warning" => FindingSeverity::Warning,
                _ => FindingSeverity::Info,
            };
            let place = Place::new(
                local_path(fields.get("file").copied().unwrap_or(""), dir),
                fields.get("line").and_then(|v| v.parse().ok()).unwrap_or(0),
                fields.get("col").and_then(|v| v.parse().ok()).unwrap_or(1),
            );
            finding(
                FindingKind::Lint,
                TOOL,
                severity,
                place,
                fields.get("title").copied().unwrap_or(""),
                &message,
            )
        })
        .collect()
}

/// Files the formatter would change, once per file.
pub(crate) fn parse_format(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    let mut files: Vec<String> = Vec::new();
    for (_, props, _) in annotations(output) {
        let file = local_path(properties(&props).get("file").copied().unwrap_or(""), dir);
        if !file.is_empty() && !files.contains(&file) {
            files.push(file);
        }
    }
    files
        .into_iter()
        .map(|file| unformatted(TOOL, file))
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
