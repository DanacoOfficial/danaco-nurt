// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of the command line and parsing of the options.
//!
//! The first argument selects a subcommand only when it equals a name of
//! [`SUBCOMMANDS`] exactly; any other first argument starts a program of the
//! language. `nurt exec` is always unambiguous.

use nurt_contracts::envelope::{OutputFormat, Verbosity};
use nurt_contracts::kinds::SyntaxVariant;

use crate::app::CliError;

/// Names of the subcommands, in the order of the documentation.
pub const SUBCOMMANDS: &[&str] = &[
    "exec", "script", "repl", "check", "fmt", "ast", "schema", "catalog", "help", "history",
    "undo", "redo", "serve", "mcp", "lsp", "migrate", "service", "journal", "ext", "doctor",
    "version",
];

/// Options of a program: `exec` and the shortcut `nurt <program>`.
pub const EXEC_FLAGS: &[&str] = &[
    "--variant",
    "--preview",
    "--json",
    "--format",
    "--limit",
    "--cursor",
    "--verbosity",
];

/// Options of `script`.
pub const SCRIPT_FLAGS: &[&str] = &["--preview", "--json", "--format", "--limit", "--verbosity"];

/// Name of the form of `run` that runs a program file.
pub const RUN_FILE: &str = "run";

/// Options of `run`.
pub const RUN_FLAGS: &[&str] = &[
    "--variant",
    "--preview",
    "--json",
    "--format",
    "--limit",
    "--verbosity",
];

/// Options of `check`.
pub const CHECK_FLAGS: &[&str] = &["-e", "--variant", "--json", "--format"];

/// Options of `fmt`.
pub const FMT_FLAGS: &[&str] = &["--variant", "--json", "--format"];

/// Options of `ast`.
pub const AST_FLAGS: &[&str] = &["--from", "--to", "--json", "--format"];

/// Options of `schema`.
pub const SCHEMA_FLAGS: &[&str] = &["--json", "--format"];

/// How the command line is interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// The option `-V` (short form) or `--version` (full form).
    Version {
        /// Whether the full form is requested.
        full: bool,
    },
    /// The option `-h` or `--help`.
    Help,
    /// A subcommand with the arguments that follow its name.
    Subcommand {
        /// Name of the subcommand.
        name: &'static str,
        /// Arguments after the name.
        args: Vec<String>,
    },
    /// A program of the language, given as the arguments.
    Program {
        /// All arguments.
        args: Vec<String>,
    },
}

/// Interprets the arguments after the name of the executable.
///
/// No arguments start an interactive session.
#[must_use]
pub fn resolve(args: &[String]) -> Invocation {
    let Some(first) = args.first() else {
        return Invocation::Subcommand {
            name: "repl",
            args: Vec::new(),
        };
    };
    match first.as_str() {
        "-V" => Invocation::Version { full: false },
        "--version" => Invocation::Version { full: true },
        "-h" | "--help" => Invocation::Help,
        // `run` is also a command of the language: `nurt run ["make"] cwd: "/x"` is
        // a program, `nurt run build.nurt` runs a program file with parameters.
        "run" if args.get(1).is_some_and(|file| is_program_file(file)) => Invocation::Subcommand {
            name: RUN_FILE,
            args: args[1..].to_vec(),
        },
        name => match SUBCOMMANDS.iter().find(|known| **known == name) {
            Some(known) => Invocation::Subcommand {
                name: known,
                args: args[1..].to_vec(),
            },
            None => Invocation::Program {
                args: args.to_vec(),
            },
        },
    }
}

/// Whether the argument names a program file for `nurt run`: it ends with
/// `.nurt`.
fn is_program_file(argument: &str) -> bool {
    std::path::Path::new(argument)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("nurt"))
}

/// Whether the arguments after the name of a subcommand ask for its usage: the
/// first one is `-h` or `--help`.
#[must_use]
pub fn wants_help(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("-h" | "--help"))
}

/// Values of the options of a command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    /// Value of the option `--variant`.
    pub variant: Option<SyntaxVariant>,
    /// Value of the option `--preview`.
    pub preview: bool,
    /// Value of the options `--format` and `--json`.
    pub format: Option<OutputFormat>,
    /// Value of the option `--limit`.
    pub limit: Option<u32>,
    /// Value of the option `--cursor`.
    pub cursor: Option<String>,
    /// Value of the option `--verbosity`.
    pub verbosity: Option<Verbosity>,
    /// Program text given with the option `-e`.
    pub program: Option<String>,
    /// Value of the option `--from`.
    pub from: Option<String>,
    /// Value of the option `--to`.
    pub to: Option<String>,
}

/// Splits the arguments into options and positional arguments.
///
/// Options not listed in `allowed` are rejected. With `interleaved` false the
/// first positional argument and everything after it are positional, which
/// keeps the text of a program intact. The argument `--` ends the options.
///
/// # Errors
///
/// Returns a usage error for an unknown option, a missing or an invalid
/// value.
pub fn parse_flags(
    args: &[String],
    allowed: &[&str],
    interleaved: bool,
) -> Result<(Flags, Vec<String>), CliError> {
    let mut flags = Flags::default();
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            positional.extend(iter.by_ref().cloned());
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            positional.push(arg.clone());
            if !interleaved {
                positional.extend(iter.by_ref().cloned());
                break;
            }
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (arg.as_str(), None),
        };
        if !allowed.contains(&name) {
            return Err(CliError::Usage(format!("unknown option `{name}`")));
        }
        apply(&mut flags, name, inline, &mut iter)?;
    }
    Ok((flags, positional))
}

fn apply(
    flags: &mut Flags,
    name: &str,
    inline: Option<String>,
    rest: &mut std::slice::Iter<'_, String>,
) -> Result<(), CliError> {
    match name {
        "--preview" | "--json" => {
            if inline.is_some() {
                return Err(CliError::Usage(format!("option `{name}` takes no value")));
            }
            if name == "--preview" {
                flags.preview = true;
            } else {
                flags.format = Some(OutputFormat::Json);
            }
            return Ok(());
        }
        _ => {}
    }
    let value = match inline {
        Some(value) => value,
        None => rest
            .next()
            .cloned()
            .ok_or_else(|| CliError::Usage(format!("option `{name}` needs a value")))?,
    };
    store(flags, name, value)
}

fn store(flags: &mut Flags, name: &str, value: String) -> Result<(), CliError> {
    match name {
        "--variant" => flags.variant = Some(variant(&value)?),
        "--format" => flags.format = Some(format(&value)?),
        "--verbosity" => flags.verbosity = Some(verbosity(&value)?),
        "--limit" => {
            flags.limit = Some(value.parse().map_err(|_| {
                CliError::Usage(format!("`{value}` is not a valid limit; expected a number"))
            })?);
        }
        "--cursor" => flags.cursor = Some(value),
        "-e" => flags.program = Some(value),
        "--from" => flags.from = Some(value),
        "--to" => flags.to = Some(value),
        _ => return Err(CliError::Usage(format!("unknown option `{name}`"))),
    }
    Ok(())
}

/// Parses the name of a notation variant, which is `a`, `b` or `c`.
///
/// # Errors
///
/// Returns a usage error for another value.
pub fn variant(value: &str) -> Result<SyntaxVariant, CliError> {
    match value {
        "a" => Ok(SyntaxVariant::A),
        "b" => Ok(SyntaxVariant::B),
        "c" => Ok(SyntaxVariant::C),
        _ => Err(CliError::Usage(format!(
            "`{value}` is not a variant; expected a, b or c"
        ))),
    }
}

fn format(value: &str) -> Result<OutputFormat, CliError> {
    OutputFormat::ALL
        .iter()
        .copied()
        .find(|format| format.as_str() == value)
        .ok_or_else(|| {
            CliError::Usage(format!(
                "`{value}` is not a format; expected text, json, markdown or compact"
            ))
        })
}

fn verbosity(value: &str) -> Result<Verbosity, CliError> {
    Verbosity::ALL
        .iter()
        .copied()
        .find(|level| level.as_str() == value)
        .ok_or_else(|| {
            CliError::Usage(format!(
                "`{value}` is not a verbosity; expected quiet, normal or detailed"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn empty_command_line_starts_a_session() {
        assert_eq!(
            resolve(&[]),
            Invocation::Subcommand {
                name: "repl",
                args: Vec::new()
            }
        );
    }

    #[test]
    fn options_stop_at_the_first_program_word() {
        let (flags, rest) =
            parse_flags(&words("--preview find --json"), EXEC_FLAGS, false).unwrap();
        assert!(flags.preview);
        assert_eq!(flags.format, None);
        assert_eq!(rest, words("find --json"));
    }

    #[test]
    fn interleaved_options_follow_positional_arguments() {
        let (flags, rest) =
            parse_flags(&words("file.nurt --preview --limit=7"), SCRIPT_FLAGS, true).unwrap();
        assert!(flags.preview);
        assert_eq!(flags.limit, Some(7));
        assert_eq!(rest, words("file.nurt"));
    }

    #[test]
    fn values_are_validated() {
        for bad in [
            "--limit x",
            "--variant d",
            "--format yaml",
            "--verbosity loud",
        ] {
            let error = parse_flags(&words(bad), EXEC_FLAGS, false).unwrap_err();
            assert!(matches!(error, CliError::Usage(_)), "{bad}");
        }
        let error = parse_flags(&words("--limit"), EXEC_FLAGS, false).unwrap_err();
        assert!(matches!(error, CliError::Usage(_)));
    }

    #[test]
    fn unknown_and_misused_options_are_rejected() {
        assert!(parse_flags(&words("--bogus"), EXEC_FLAGS, false).is_err());
        assert!(parse_flags(&words("--preview=yes"), EXEC_FLAGS, false).is_err());
        assert!(parse_flags(&words("-e x"), EXEC_FLAGS, false).is_err());
    }

    #[test]
    fn double_dash_ends_the_options() {
        let (flags, rest) = parse_flags(&words("-- --preview"), EXEC_FLAGS, true).unwrap();
        assert!(!flags.preview);
        assert_eq!(rest, words("--preview"));
    }

    #[test]
    fn values_follow_an_equals_sign_or_the_next_word() {
        let (flags, rest) = parse_flags(
            &words("--variant=c --format=compact --limit=9 --verbosity=detailed find"),
            EXEC_FLAGS,
            false,
        )
        .unwrap();
        assert_eq!(flags.variant, Some(SyntaxVariant::C));
        assert_eq!(flags.format, Some(OutputFormat::Compact));
        assert_eq!(flags.limit, Some(9));
        assert_eq!(flags.verbosity, Some(Verbosity::Detailed));
        assert_eq!(rest, words("find"));
    }

    #[test]
    fn json_and_format_set_the_same_option_and_the_last_wins() {
        let (flags, _) = parse_flags(&words("--format text --json"), EXEC_FLAGS, false).unwrap();
        assert_eq!(flags.format, Some(OutputFormat::Json));
        let (flags, _) =
            parse_flags(&words("--json --format markdown"), EXEC_FLAGS, false).unwrap();
        assert_eq!(flags.format, Some(OutputFormat::Markdown));
    }

    #[test]
    fn check_options_take_the_program_text_and_a_file() {
        let (flags, rest) =
            parse_flags(&words("-e count --variant b file.nurt"), CHECK_FLAGS, true).unwrap();
        assert_eq!(flags.program.as_deref(), Some("count"));
        assert_eq!(flags.variant, Some(SyntaxVariant::B));
        assert_eq!(rest, words("file.nurt"));
        let (flags, _) = parse_flags(&words("-e=a=b"), CHECK_FLAGS, true).unwrap();
        assert_eq!(flags.program.as_deref(), Some("a=b"));
    }

    #[test]
    fn ast_options_take_the_notations() {
        let (flags, rest) = parse_flags(&words("--from a --to=json -"), AST_FLAGS, true).unwrap();
        assert_eq!(flags.from.as_deref(), Some("a"));
        assert_eq!(flags.to.as_deref(), Some("json"));
        assert_eq!(rest, words("-"));
        assert!(parse_flags(&words("--variant a"), AST_FLAGS, true).is_err());
        assert!(parse_flags(&words("--from"), AST_FLAGS, true).is_err());
    }

    #[test]
    fn options_of_other_subcommands_are_rejected() {
        for (flags, option) in [
            (SCRIPT_FLAGS, "--variant"),
            (SCRIPT_FLAGS, "--cursor"),
            (FMT_FLAGS, "--preview"),
            (CHECK_FLAGS, "--limit"),
        ] {
            let error = parse_flags(&words(&format!("{option} x")), flags, true).unwrap_err();
            assert_eq!(
                error,
                CliError::Usage(format!("unknown option `{option}`")),
                "{option}"
            );
        }
    }

    #[test]
    fn a_lone_dash_and_non_ascii_words_are_positional() {
        let (flags, rest) =
            parse_flags(&words("- \u{142}\u{f3}d\u{17a}"), EXEC_FLAGS, true).unwrap();
        assert_eq!(flags, Flags::default());
        assert_eq!(rest, words("- \u{142}\u{f3}d\u{17a}"));
    }

    #[test]
    fn limit_must_be_a_number_that_fits_the_type() {
        for bad in [
            "--limit -1",
            "--limit 4294967296",
            "--limit 1.5",
            "--limit=",
        ] {
            assert!(
                parse_flags(&words(bad), EXEC_FLAGS, false).is_err(),
                "{bad}"
            );
        }
        let (flags, _) = parse_flags(&words("--limit 4294967295"), EXEC_FLAGS, false).unwrap();
        assert_eq!(flags.limit, Some(u32::MAX));
    }

    #[test]
    fn version_and_help_options_resolve_before_subcommands() {
        assert_eq!(resolve(&words("-V")), Invocation::Version { full: false });
        assert_eq!(
            resolve(&words("--version x")),
            Invocation::Version { full: true }
        );
        assert_eq!(resolve(&words("-h")), Invocation::Help);
        assert_eq!(resolve(&words("--help check")), Invocation::Help);
    }

    #[test]
    fn help_is_asked_by_the_first_argument_after_the_subcommand() {
        assert!(wants_help(&words("--help")));
        assert!(wants_help(&words("-h extra")));
        assert!(!wants_help(&words("")));
        assert!(!wants_help(&words("-e count --help")));
        assert!(!wants_help(&words("help")));
    }

    #[test]
    fn subcommand_arguments_exclude_its_name() {
        assert_eq!(
            resolve(&words("check -e count")),
            Invocation::Subcommand {
                name: "check",
                args: words("-e count")
            }
        );
        assert_eq!(
            resolve(&words("count |> first")),
            Invocation::Program {
                args: words("count |> first")
            }
        );
    }

    #[test]
    fn named_values_are_stored() {
        let (flags, _) = parse_flags(
            &words("--variant b --format markdown --verbosity quiet --cursor c_1"),
            EXEC_FLAGS,
            false,
        )
        .unwrap();
        assert_eq!(flags.variant, Some(SyntaxVariant::B));
        assert_eq!(flags.format, Some(OutputFormat::Markdown));
        assert_eq!(flags.verbosity, Some(Verbosity::Quiet));
        assert_eq!(flags.cursor.as_deref(), Some("c_1"));
    }
}
