// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::analyzers::eslint
// Responsibility:   Parser of `eslint -f json` output.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of `eslint -f json` output.

use std::path::Path;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use serde::Deserialize;

use crate::analyze::output::{Place, ToolOutput, finding, local_path};

/// Tool name in findings.
pub(crate) const TOOL: &str = "eslint";

/// Severity number of an error in the report.
const ERROR_LEVEL: u8 = 2;

#[derive(Deserialize)]
struct FileReport {
    #[serde(rename = "filePath", default)]
    file_path: String,
    #[serde(default)]
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(rename = "ruleId", default)]
    rule_id: Option<String>,
    #[serde(default)]
    severity: u8,
    #[serde(default)]
    message: String,
    #[serde(default)]
    line: u64,
    #[serde(default)]
    column: u64,
}

/// Messages of the report; empty when the output is not a JSON report.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    let Ok(report) = serde_json::from_str::<Vec<FileReport>>(&output.stdout) else {
        return Vec::new();
    };
    report
        .into_iter()
        .flat_map(|file| {
            let path = local_path(&file.file_path, dir);
            file.messages.into_iter().map(move |item| {
                let severity = if item.severity == ERROR_LEVEL {
                    FindingSeverity::Error
                } else {
                    FindingSeverity::Warning
                };
                finding(
                    FindingKind::Lint,
                    TOOL,
                    severity,
                    Place::new(path.clone(), item.line, item.column),
                    item.rule_id.as_deref().unwrap_or(""),
                    &item.message,
                )
            })
        })
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
