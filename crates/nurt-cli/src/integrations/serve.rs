// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::integrations::serve
// Responsibility:   The `serve` subcommand: the protocol server on the standard streams.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `serve` subcommand: the protocol server on the standard streams.
//!
//! The command line takes exactly one attachment, `--stdio` or
//! `--socket <path>`, and the options `--framing ndjson|content-length` and
//! `--stub <directory>`. Only `--stdio` is served. `--socket` is reserved: no
//! build contains a socket transport, so it ends with the error "not available
//! in this build" (exit code 3). The recorded cases of `--stub` need a build
//! that contains them and end with the same error. Nothing but protocol messages is
//! written to the standard output.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::app::{CliError, EngineSource};
use crate::serve::{self, Framing, ServeOptions};

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

/// Arguments of `serve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    /// How the server is attached to its client.
    pub attachment: Attachment,
    /// Framing of the messages.
    pub framing: Framing,
    /// Directory of recorded cases that answer the requests instead of the
    /// engine.
    pub stub: Option<PathBuf>,
}

/// Parses the arguments of `serve`.
///
/// # Errors
///
/// Returns a usage error unless the arguments contain exactly one attachment,
/// at most one framing and at most one stub directory, and nothing else.
pub fn parse(args: &[String]) -> Result<Arguments, CliError> {
    let mut attachment = None;
    let mut framing = None;
    let mut stub = None;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--stdio" => set(&mut attachment, Attachment::Stdio, "--stdio")?,
            "--socket" => {
                let path = value(&mut words, "--socket")?;
                set(
                    &mut attachment,
                    Attachment::Socket(PathBuf::from(path)),
                    "--socket",
                )?;
            }
            "--framing" => {
                let name = value(&mut words, "--framing")?;
                let chosen = Framing::from_name(name).ok_or_else(|| {
                    CliError::Usage(format!(
                        "unknown framing `{name}`; use `ndjson` or `content-length`"
                    ))
                })?;
                set(&mut framing, chosen, "--framing")?;
            }
            "--stub" => {
                let path = value(&mut words, "--stub")?;
                set(&mut stub, PathBuf::from(path), "--stub")?;
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown argument `{other}` of `serve`"
                )));
            }
        }
    }
    let attachment = attachment.ok_or_else(|| {
        CliError::Usage("`serve` needs `--stdio` or `--socket <path>`".to_owned())
    })?;
    Ok(Arguments {
        attachment,
        framing: framing.unwrap_or_default(),
        stub,
    })
}

fn set<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), CliError> {
    if slot.replace(value).is_some() {
        return Err(CliError::Usage(format!(
            "`serve` takes `{option}` or an option of its kind only once"
        )));
    }
    Ok(())
}

fn value<'a>(words: &mut std::slice::Iter<'a, String>, option: &str) -> Result<&'a str, CliError> {
    words
        .next()
        .map(String::as_str)
        .ok_or_else(|| CliError::Usage(format!("`{option}` needs a value")))
}

/// Runs `nurt serve` on the given streams and returns the exit code of the
/// server.
///
/// # Errors
///
/// Returns a usage error for invalid arguments, the error "not available in
/// this build" for an attachment or a stub directory the build does not
/// serve, and the error of the engine or of the recorded cases.
pub fn start(
    args: &[String],
    source: &dyn EngineSource,
    input: impl BufRead,
    output: impl Write + Send,
) -> Result<ExitCode, CliError> {
    let arguments = parse(args)?;
    if matches!(arguments.attachment, Attachment::Socket(_)) {
        return Err(super::unavailable(arguments.attachment.form()));
    }
    let options = match &arguments.stub {
        Some(directory) => stub_options(directory)?,
        None => ServeOptions::engine(source.open()?),
    };
    Ok(serve::run(
        options.with_framing(arguments.framing),
        input,
        output,
    ))
}

#[cfg(feature = "stub")]
fn stub_options(directory: &std::path::Path) -> Result<ServeOptions, CliError> {
    ServeOptions::stub(directory)
        .map_err(|error| CliError::Failure(format!("cannot load the recorded cases: {error}")))
}

#[cfg(not(feature = "stub"))]
fn stub_options(_directory: &std::path::Path) -> Result<ServeOptions, CliError> {
    Err(super::unavailable("serve --stub"))
}

/// Runs `nurt serve` where the standard streams of the process are not
/// available.
///
/// # Errors
///
/// Returns a usage error for invalid arguments and a failure otherwise: the
/// server is started only by the executable, on its own streams.
pub fn run(args: &[String]) -> Result<u8, CliError> {
    let arguments = parse(args)?;
    if matches!(arguments.attachment, Attachment::Socket(_)) {
        return Err(super::unavailable(arguments.attachment.form()));
    }
    Err(CliError::Failure(
        "`serve` runs only on the standard streams of the process".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn serve_needs_exactly_one_attachment() {
        assert_eq!(
            parse(&words("--stdio")),
            Ok(Arguments {
                attachment: Attachment::Stdio,
                framing: Framing::Ndjson,
                stub: None,
            })
        );
        assert_eq!(
            parse(&words("--socket /run/nurt.sock")).map(|arguments| arguments.attachment),
            Ok(Attachment::Socket(PathBuf::from("/run/nurt.sock")))
        );
        for bad in [
            "",
            "--socket",
            "--stdio --stdio",
            "--stdio --socket /x",
            "--tcp 1",
            "--framing ndjson",
        ] {
            assert!(parse(&words(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn options_choose_the_framing_and_the_stub_directory() {
        let arguments = parse(&words("--stdio --framing content-length --stub /cases")).unwrap();
        assert_eq!(arguments.framing, Framing::ContentLength);
        assert_eq!(arguments.stub, Some(PathBuf::from("/cases")));
        for bad in [
            "--stdio --framing",
            "--stdio --framing xml",
            "--stdio --framing ndjson --framing ndjson",
            "--stdio --stub",
            "--stdio --stub /a --stub /b",
        ] {
            assert!(
                matches!(parse(&words(bad)), Err(CliError::Usage(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn attachments_name_their_form_of_the_subcommand() {
        assert_eq!(Attachment::Stdio.form(), "serve --stdio");
        assert_eq!(
            Attachment::Socket(PathBuf::from("/run/nurt.sock")).form(),
            "serve --socket"
        );
    }

    #[test]
    fn a_socket_path_with_multi_byte_characters_is_kept() {
        let args = vec![
            "--socket".to_owned(),
            "/run/\u{142}\u{f3}d\u{17a}.sock".to_owned(),
        ];
        assert_eq!(
            parse(&args).unwrap().attachment,
            Attachment::Socket(PathBuf::from("/run/\u{142}\u{f3}d\u{17a}.sock"))
        );
    }

    #[test]
    fn each_option_needs_its_value() {
        for bad in ["--stdio --framing", "--stdio --stub", "--socket"] {
            assert_eq!(
                parse(&words(bad)),
                Err(CliError::Usage(format!(
                    "`{}` needs a value",
                    bad.rsplit(' ').next().unwrap()
                ))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_message_of_an_unknown_argument_names_it() {
        assert_eq!(
            parse(&words("--stdio --tcp")),
            Err(CliError::Usage(
                "unknown argument `--tcp` of `serve`".to_owned()
            ))
        );
        assert_eq!(
            parse(&words("")),
            Err(CliError::Usage(
                "`serve` needs `--stdio` or `--socket <path>`".to_owned()
            ))
        );
    }

    #[test]
    fn the_socket_attachment_is_refused_before_the_engine_is_opened() {
        assert_eq!(
            run(&words("--socket /run/nurt.sock")),
            Err(CliError::Unavailable("serve --socket".to_owned()))
        );
    }

    #[test]
    fn in_process_run_validates_and_fails() {
        assert!(matches!(run(&words("--stdio")), Err(CliError::Failure(_))));
        assert!(matches!(run(&[]), Err(CliError::Usage(_))));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
