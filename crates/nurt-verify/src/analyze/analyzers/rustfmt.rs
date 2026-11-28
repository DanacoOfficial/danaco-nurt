// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::analyzers::rustfmt
// Responsibility:   Parser of `cargo fmt --all -- --check -l` output.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of `cargo fmt --all -- --check -l` output.

use std::path::Path;

use nurt_contracts::record::Finding;

use crate::analyze::output::{ToolOutput, file_list_findings};

/// Tool name in findings.
pub(crate) const TOOL: &str = "rustfmt";

/// Files that need formatting, one per output line.
pub(crate) fn parse(output: &ToolOutput, dir: &Path) -> Vec<Finding> {
    file_list_findings(TOOL, output, dir, None)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
