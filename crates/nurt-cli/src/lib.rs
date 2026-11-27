// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli
// Responsibility:   Library of the `nurt` command line interface.
// Dependencies:     app
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Library of the `nurt` command line interface.
//!
//! [`main_with_args`] is the whole executable: it resolves the arguments to a
//! subcommand or to a program of the language, runs it on an engine and
//! returns the exit code ([`exit`]). The [`serve`] module implements the
//! protocol server started by `nurt serve`.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

pub mod app;
pub mod args;
pub mod banner;
pub mod commands;
pub mod complete;
pub mod doctor;
mod elevated;
pub mod exit;
pub mod integrations;
pub mod output;
pub mod params;
pub mod repl;
pub mod serve;
pub mod system;
pub mod text;
pub mod theme;
mod tool_definition;

pub use app::{CliError, Context, EngineSource, Io, SystemEngine, Terminal, run};

/// Runs the command line interface with the arguments that follow the name
/// of the executable and returns the exit code.
///
/// An argument that is not valid UTF-8 is a usage error.
pub fn main_with_args<I: IntoIterator<Item = OsString>>(args: I) -> ExitCode {
    let mut words = Vec::new();
    for argument in args {
        match argument.into_string() {
            Ok(word) => words.push(word),
            Err(_) => {
                let _ = writeln!(io::stderr(), "nurt: an argument is not valid UTF-8");
                return ExitCode::from(exit::USAGE);
            }
        }
    }
    let stdin = io::stdin();
    let terminal = Terminal {
        stdin: stdin.is_terminal(),
        stdout: io::stdout().is_terminal(),
        stderr: io::stderr().is_terminal(),
    };
    // The lock of the standard input is taken per read: the servers of `mcp`
    // and `lsp` read the same stream from their own threads and would block
    // for ever behind a lock held for the whole process.
    let mut input = io::BufReader::new(stdin);
    let mut io = Io {
        out: &mut io::stdout(),
        err: &mut io::stderr(),
        input: &mut input,
        terminal,
    };
    app::run_process(&words, &mut io, &SystemEngine)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
