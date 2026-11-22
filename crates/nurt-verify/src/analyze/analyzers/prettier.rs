// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of `prettier --list-different` output.

use std::path::Path;

use nurt_contracts::record::Finding;

use crate::analyze::output::{ToolOutput, file_list_findings};

/// Tool name in findings.
pub(crate) const TOOL: &str = "prettier";

/// Files that need formatting, one per output line.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    file_list_findings(TOOL, output, dir, None)
}
