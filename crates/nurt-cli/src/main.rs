// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli
// Responsibility:   Command line interface of Danaco Nurt.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Command line interface of Danaco Nurt.
//!
//! The `nurt` executable runs programs given as arguments or files, offers an
//! interactive session, and hosts the protocol servers (`serve`, `mcp`, `lsp`)
//! and the background service in the same executable.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    nurt_cli::main_with_args(std::env::args_os().skip(1))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
