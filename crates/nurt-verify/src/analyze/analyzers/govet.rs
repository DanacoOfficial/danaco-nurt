// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of `go vet` output.

use std::path::Path;

use nurt_contracts::record::{Finding, FindingKind};

use crate::analyze::output::{ToolOutput, colon_line_findings};

/// Tool name in findings.
pub(crate) const TOOL: &str = "go vet";

/// Diagnostics written to standard error, without the `vet: ` prefix.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    let text: String = output
        .stderr
        .lines()
        .map(|line| line.strip_prefix("vet: ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    colon_line_findings(&text, dir, FindingKind::Lint, TOOL)
}
