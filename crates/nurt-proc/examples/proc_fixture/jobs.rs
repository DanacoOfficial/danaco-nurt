// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Modes for the tests of background jobs.

use std::process::ExitCode;

/// Runs `mode` when it belongs to this area.
pub(crate) fn run(_mode: &str, _args: &[String]) -> Option<ExitCode> {
    None
}
