// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Command line interface of Danaco Nurt.
//!
//! The `nurt` executable runs programs given as arguments or files, offers an
//! interactive session, and hosts the protocol servers (`serve`, `mcp`, `lsp`)
//! and the background service in the same executable.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut out = std::io::stdout().lock();
    match writeln!(
        out,
        "Danaco Nurt {} \u{2014} Danaco Holding Group",
        env!("CARGO_PKG_VERSION")
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
