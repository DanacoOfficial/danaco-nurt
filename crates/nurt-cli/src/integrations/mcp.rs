// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `mcp` subcommand: the tool interface of the Model Context Protocol.
//!
//! The command line takes `--stdio` (the default) or `--http <address>`.
//! The standard output carries nothing but protocol frames, so no banner is
//! written. The server crate is linked by the Cargo feature `mcp`; until the
//! subcommand is connected (see [`super::is_connected`]) it ends with the
//! error "not available in this build".

use std::process::ExitCode;
use std::sync::Arc;

use nurt_core::Engine;

use crate::app::{CliError, EngineSource};

/// How the server is attached to its client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// Frames on the standard input and output.
    Stdio,
    /// Streamable HTTP on the given listening address.
    Http(String),
}

impl Transport {
    /// The form of the subcommand with this transport, as written on the
    /// command line.
    #[must_use]
    pub fn form(&self) -> &'static str {
        match self {
            Self::Stdio => "mcp --stdio",
            Self::Http(_) => "mcp --http",
        }
    }
}

/// Arguments of `mcp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    /// How the server is attached to its client.
    pub transport: Transport,
}

/// Parses the arguments of `mcp`.
///
/// # Errors
///
/// Returns a usage error for an unknown argument, a second transport or a
/// missing address.
pub fn parse(args: &[String]) -> Result<Arguments, CliError> {
    let mut transport = None;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let chosen = match word.as_str() {
            "--stdio" => Transport::Stdio,
            "--http" => {
                let address = words
                    .next()
                    .filter(|address| !address.is_empty() && !address.starts_with("--"))
                    .ok_or_else(|| CliError::Usage("`--http` needs an address".to_owned()))?;
                Transport::Http(address.clone())
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown argument `{other}` of `mcp`"
                )));
            }
        };
        if transport.replace(chosen).is_some() {
            return Err(CliError::Usage(
                "`mcp` takes one of `--stdio` and `--http <address>`".to_owned(),
            ));
        }
    }
    Ok(Arguments {
        transport: transport.unwrap_or(Transport::Stdio),
    })
}

/// Runs `nurt mcp` on the standard streams of the process.
///
/// # Errors
///
/// Returns a usage error for invalid arguments, the error "not available in
/// this build" while the subcommand is not connected, and the error of the
/// engine when it cannot be opened.
pub fn start(args: &[String], source: &dyn EngineSource) -> Result<ExitCode, CliError> {
    let arguments = parse(args)?;
    if !super::is_connected("mcp") {
        return Err(super::unavailable(arguments.transport.form()));
    }
    let engine = Arc::new(source.open()?);
    connect(&engine, &arguments)
}

/// Serves the engine to one client; the only function that touches the
/// server crate.
fn connect(_engine: &Arc<Engine>, arguments: &Arguments) -> Result<ExitCode, CliError> {
    Err(super::unavailable(arguments.transport.form()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn standard_streams_are_the_default_transport() {
        assert_eq!(parse(&[]).unwrap().transport, Transport::Stdio);
        assert_eq!(
            parse(&words("--stdio")).unwrap().transport,
            Transport::Stdio
        );
    }

    #[test]
    fn http_takes_an_address() {
        assert_eq!(
            parse(&words("--http 127.0.0.1:8080")).unwrap().transport,
            Transport::Http("127.0.0.1:8080".to_owned())
        );
        assert!(matches!(parse(&words("--http")), Err(CliError::Usage(_))));
        assert!(matches!(
            parse(&words("--http --stdio")),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn one_transport_and_known_arguments_only() {
        assert!(matches!(
            parse(&words("--stdio --http a:1")),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(parse(&words("--tcp")), Err(CliError::Usage(_))));
    }
}
