// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Attachment of `nurt serve` to its client.
//!
//! The subcommand takes exactly one attachment: `--stdio` or
//! `--socket <path>`. This build has no entry point of the protocol server
//! in [`crate::serve`], so a valid command line ends with the error "not
//! available in this build" and writes nothing to the standard output.

use std::path::PathBuf;

use crate::app::CliError;

/// How the protocol server is attached to its client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Messages on the standard input and output.
    Stdio,
    /// A local socket at the given path.
    Socket(PathBuf),
}

impl Attachment {
    /// The form of the subcommand with this attachment, as written on the
    /// command line.
    #[must_use]
    pub fn form(&self) -> &'static str {
        match self {
            Self::Stdio => "serve --stdio",
            Self::Socket(_) => "serve --socket",
        }
    }
}

/// Parses the arguments of `serve`.
///
/// # Errors
///
/// Returns a usage error unless the arguments are `--stdio` or
/// `--socket <path>`.
pub fn attachment(args: &[String]) -> Result<Attachment, CliError> {
    match args {
        [flag] if flag == "--stdio" => Ok(Attachment::Stdio),
        [flag, path] if flag == "--socket" => Ok(Attachment::Socket(PathBuf::from(path))),
        _ => Err(CliError::Usage(
            "`serve` needs `--stdio` or `--socket <path>`".to_owned(),
        )),
    }
}

/// Runs `nurt serve`.
///
/// # Errors
///
/// Returns a usage error for invalid arguments and the error "not available
/// in this build" for a valid attachment.
pub fn run(args: &[String]) -> Result<u8, CliError> {
    let attachment = attachment(args)?;
    Err(super::unavailable(attachment.form()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn serve_needs_exactly_one_attachment() {
        assert_eq!(attachment(&words("--stdio")), Ok(Attachment::Stdio));
        assert_eq!(
            attachment(&words("--socket /run/nurt.sock")),
            Ok(Attachment::Socket(PathBuf::from("/run/nurt.sock")))
        );
        for bad in ["", "--socket", "--stdio --stdio", "--tcp 1"] {
            assert!(attachment(&words(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn valid_attachment_is_not_available_in_this_build() {
        assert_eq!(
            run(&words("--stdio")),
            Err(CliError::Unavailable("serve --stdio".to_owned()))
        );
        assert_eq!(
            run(&words("--socket /run/nurt.sock")),
            Err(CliError::Unavailable("serve --socket".to_owned()))
        );
        assert!(matches!(run(&[]), Err(CliError::Usage(_))));
    }
}
