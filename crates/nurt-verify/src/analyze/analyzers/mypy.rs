// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::analyzers::mypy
// Responsibility:   Parser of `mypy --show-column-numbers --no-error-summary` output.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of `mypy --show-column-numbers --no-error-summary` output.

use std::path::Path;

use nurt_contracts::record::{Finding, FindingKind};

use crate::analyze::output::{ToolOutput, colon_line_findings};

/// Tool name in findings.
pub(crate) const TOOL: &str = "mypy";

/// Diagnostics of the type checker; the error code in brackets becomes the
/// rule.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    colon_line_findings(&output.stdout, dir, FindingKind::Types, TOOL)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
