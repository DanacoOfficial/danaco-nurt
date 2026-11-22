// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `lsp` subcommand: the language server on the standard streams.
//!
//! The subcommand takes no arguments. The server binds programs to the
//! embedded catalog and describes commands and error codes from the embedded
//! documentation. Nothing but protocol messages is written to the standard
//! output.

use nurt_bind::Catalog as BindCatalog;
use nurt_help::catalog::Catalog as HelpCatalog;
use nurt_help::docs::Docs;
use nurt_langserver::{Options, ServerError, run_stdio};

use crate::app::CliError;

/// Exit code of a session that the client ended with `shutdown` and `exit`.
const EXIT_OK: u8 = 0;

/// Builds the settings of a session from the embedded catalog.
fn options() -> Result<Options, CliError> {
    let (index, files) = nurt_catalog::embedded_sources();
    let texts: Vec<&str> = files.iter().map(|(_, text)| *text).collect();
    let bind = BindCatalog::from_toml(index, &texts)
        .map_err(|error| CliError::Failure(format!("cannot load the embedded catalog: {error}")))?;
    let help = HelpCatalog::from_sources(index, files, Docs::empty())
        .map_err(|error| CliError::Failure(format!("cannot load the embedded catalog: {error}")))?;
    Ok(Options::from_catalog(bind).with_help(help))
}

/// Maps the error that ended a session to a command line error.
fn failure(error: &ServerError) -> CliError {
    CliError::Failure(error.to_string())
}

/// Runs `nurt lsp` on the standard input and output until the client ends
/// the session.
///
/// # Errors
///
/// Returns a usage error for any argument, a failure when the embedded
/// catalog cannot be loaded, and a failure carrying the message of the
/// [`ServerError`] that ended the session before `shutdown`.
pub fn run(args: &[String]) -> Result<u8, CliError> {
    if let Some(argument) = args.first() {
        return Err(CliError::Usage(format!(
            "unknown argument `{argument}` of `lsp`; the subcommand takes no arguments"
        )));
    }
    let options = options()?;
    run_stdio(&options).map_err(|error| failure(&error))?;
    Ok(EXIT_OK)
}
