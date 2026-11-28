// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::analyzers::clippy
// Responsibility:   Parser of `cargo clippy --message-format=json` output.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of `cargo clippy --message-format=json` output.

use std::path::Path;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use serde::Deserialize;

use crate::analyze::output::{Place, ToolOutput, finding, local_path};

/// Tool name in findings.
pub(crate) const TOOL: &str = "clippy";

/// Reason of the messages that carry compiler diagnostics.
const MESSAGE_REASON: &str = "compiler-message";

#[derive(Deserialize)]
struct Line {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    message: Option<Diagnostic>,
}

#[derive(Deserialize)]
struct Diagnostic {
    #[serde(default)]
    level: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<Code>,
    #[serde(default)]
    spans: Vec<Span>,
}

#[derive(Deserialize)]
struct Code {
    code: String,
}

#[derive(Deserialize)]
struct Span {
    file_name: String,
    line_start: u64,
    column_start: u64,
    #[serde(default)]
    is_primary: bool,
}

/// Diagnostics with a primary span; summaries without a span are skipped.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    output
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Line>(line).ok())
        .filter(|line| line.reason == MESSAGE_REASON)
        .filter_map(|line| line.message)
        .filter_map(|diagnostic| convert(diagnostic, dir))
        .collect()
}

fn convert(diagnostic: Diagnostic, dir: &Path) -> Option<Finding> {
    let severity = match diagnostic.level.as_str() {
        "error" | "error: internal compiler error" => FindingSeverity::Error,
        "warning" => FindingSeverity::Warning,
        _ => return None,
    };
    let span = diagnostic
        .spans
        .iter()
        .find(|span| span.is_primary)
        .or_else(|| diagnostic.spans.first())?;
    let rule = diagnostic.code.map_or(String::new(), |code| code.code);
    Some(finding(
        FindingKind::Lint,
        TOOL,
        severity,
        Place::new(
            local_path(&span.file_name, dir),
            span.line_start,
            span.column_start,
        ),
        &rule,
        &diagnostic.message,
    ))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
