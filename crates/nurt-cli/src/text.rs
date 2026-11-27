// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixed texts of the program: version lines, banner and help introduction.
//!
//! The version comes from the package metadata. The commit and the date of
//! the build are read from the build environment variables
//! `NURT_BUILD_COMMIT` and `NURT_BUILD_DATE`; a build that does not know them
//! omits the parenthesis of the full version line.

use nurt_contracts::product;

/// Version of the program.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const BUILD_COMMIT: Option<&str> = option_env!("NURT_BUILD_COMMIT");
const BUILD_DATE: Option<&str> = option_env!("NURT_BUILD_DATE");

/// Output of `nurt -V`.
#[must_use]
pub fn short_version() -> String {
    product::version_line(VERSION)
}

/// Output of `nurt --version` and `nurt version`: version with the build
/// data, copyright and license.
#[must_use]
pub fn full_version() -> String {
    let build = BUILD_COMMIT.zip(BUILD_DATE);
    product::long_version(VERSION, build)
}

/// Banner of an interactive session.
#[must_use]
pub fn banner() -> String {
    product::banner(VERSION)
}

/// Introduction printed by `help` without a topic.
#[must_use]
pub fn help_intro() -> String {
    format!(
        "{}\n\n\
         Run 'help <topic>' for a command or a topic.\n\
         Run 'help \"commands\"' to list the commands.",
        product::session_help_heading(VERSION)
    )
}

/// Output of `nurt --help`.
#[must_use]
pub fn usage() -> String {
    format!(
        "{}\n\n\
         Usage: nurt [OPTIONS] <PROGRAM>\n\
         \x20      nurt <COMMAND> [ARGS]\n\n\
         Commands:\n\
         \x20 exec, script, repl, check, fmt, ast, schema, catalog, help,\n\
         \x20 history, undo, redo, serve, mcp, lsp, migrate, service, journal,\n\
         \x20 ext, doctor, version\n\n\
         Options of a program:\n\
         \x20 --variant a|b|c       notation variant of the program\n\
         \x20 --preview             compute the effect without applying it\n\
         \x20 --json                same as --format json\n\
         \x20 --format F            text, json, markdown or compact\n\
         \x20 --limit N             records per page\n\
         \x20 --cursor C            read a further page of an earlier result\n\
         \x20 --verbosity V         quiet, normal or detailed\n\
         \x20 --privilege P         account, elevated or auto\n\
         \x20 --as-user NAME        run as another account (Linux and macOS)\n\
         \x20 -V, --version         print the version\n\
         \x20 -h, --help            print this help",
        product::help_summary()
    )
}

/// Description and arguments of each subcommand, as in
/// `tables/cli-subcommands.toml`; a test of the crate compares the two.
const SUBCOMMAND_USAGE: &[(&str, &str, &[&str])] = &[
    (
        "exec",
        "Run a program given as arguments.",
        &[
            "[--variant a|b|c]",
            "[--preview]",
            "[--json | --format F]",
            "[--limit N]",
            "[--cursor C]",
            "[--verbosity V]",
            "[--privilege P]",
            "[--as-user NAME]",
            "<program>",
        ],
    ),
    (
        "script",
        "Run a script file, each pipeline as its own operation.",
        &[
            "<file | ->",
            "[--preview]",
            "[--json | --format F]",
            "[--limit N]",
            "[--verbosity V]",
            "[--privilege P]",
            "[--as-user NAME]",
        ],
    ),
    ("repl", "Start an interactive session.", &[]),
    (
        "check",
        "Parse, correct and bind a program without running it.",
        &[
            "<file | -e program>",
            "[--variant a|b|c]",
            "[--json | --format F]",
        ],
    ),
    (
        "fmt",
        "Print a program in canonical form.",
        &["<file | ->", "[--variant a|b|c]"],
    ),
    (
        "ast",
        "Convert a syntax tree between notations.",
        &[
            "[--from a|b|c|json]",
            "[--to json|cbor|a|b|c]",
            "<input | ->",
        ],
    ),
    (
        "schema",
        "Print a published JSON Schema or the tool definition.",
        &[
            "ast|bound|report|request|config|tool",
            "[--format F]",
            "[--language en|pl]",
            "[--groups G]",
        ],
    ),
    (
        "catalog",
        "Describe the command catalog, a group or a command.",
        &["[command | group | check-distance]"],
    ),
    ("help", "Show the built-in help.", &["[topic]"]),
    (
        "history",
        "List the recorded operations.",
        &["[arguments of the history command]"],
    ),
    (
        "undo",
        "Undo recorded operations.",
        &["[arguments of the undo command]"],
    ),
    (
        "redo",
        "Redo undone operations.",
        &["[arguments of the redo command]"],
    ),
    (
        "serve",
        "Serve the request protocol to a client process.",
        &[
            "--stdio | --socket <path>",
            "[--framing ndjson|content-length]",
            "[--stub <directory>]",
        ],
    ),
    (
        "mcp",
        "Serve the tool interface of the Model Context Protocol.",
        &["[--stdio | --http <address>]"],
    ),
    ("lsp", "Serve the language protocol for editors.", &[]),
    (
        "migrate",
        "Translate a shell script to a Nurt script.",
        &["<file.sh | file.ps1>"],
    ),
    (
        "service",
        "Manage the background service.",
        &["install | uninstall | start | stop | status | run"],
    ),
    ("journal", "Maintain the journal.", &["salvage <volume>"]),
    ("ext", "Manage extensions.", &["install | remove | list"]),
    ("doctor", "Check the installation.", &[]),
    (
        "version",
        "Print the full version, the same text as `nurt --version`.",
        &[],
    ),
];

/// Output of `nurt <subcommand> --help`; `None` for a name that is not a
/// subcommand.
#[must_use]
pub fn subcommand_usage(name: &str) -> Option<String> {
    let (_, description, arguments) = SUBCOMMAND_USAGE.iter().find(|(known, ..)| *known == name)?;
    let mut synopsis = format!("nurt {name}");
    for argument in *arguments {
        synopsis.push(' ');
        synopsis.push_str(argument);
    }
    Some(format!(
        "{description}\n\n\
         Usage: {synopsis}\n\
         \x20      nurt {name} -h | --help\n\n\
         Run 'nurt --help' for the options of a program and the other commands."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_start_with_the_package_version() {
        assert_eq!(short_version(), format!("nurt {VERSION}"));
        assert!(full_version().starts_with(&format!("nurt {VERSION}")));
        assert_eq!(full_version().lines().count(), 3);
    }

    #[test]
    fn help_intro_lowers_the_first_letter_of_the_description() {
        let first = help_intro().lines().next().map(str::to_owned);
        assert_eq!(
            first.as_deref(),
            Some(
                format!(
                    "Danaco Nurt {VERSION} - command shell and language where every change can be undone"
                )
                .as_str()
            )
        );
    }

    #[test]
    fn help_introduction_points_to_topics_and_the_catalog() {
        let intro = help_intro();
        assert!(intro.contains("Run 'help <topic>' for a command or a topic."));
        assert!(intro.ends_with("Run 'help \"commands\"' to list the commands."));
    }

    #[test]
    fn usage_names_every_option_of_a_program() {
        let usage = usage();
        for option in [
            "--variant",
            "--preview",
            "--json",
            "--format",
            "--limit",
            "--cursor",
            "--verbosity",
            "-V, --version",
            "-h, --help",
        ] {
            assert!(usage.contains(option), "{option}");
        }
        assert!(usage.contains("Usage: nurt [OPTIONS] <PROGRAM>"));
    }

    #[test]
    fn banner_carries_the_version_and_a_hint() {
        let banner = banner();
        assert!(banner.contains("Danaco Nurt"));
        assert!(banner.contains(&format!("nurt {VERSION}")));
        assert!(banner.contains("Type 'help' to get started."));
        assert!(banner.contains("All rights reserved."));
    }

    #[test]
    fn full_version_has_the_copyright_and_the_license() {
        let lines: Vec<String> = full_version().lines().map(str::to_owned).collect();
        assert!(lines[1].starts_with("Copyright (c) 2026"));
        assert_eq!(lines[2], "License: Apache-2.0");
    }

    #[test]
    fn product_lines_are_ascii() {
        for text in [short_version(), full_version(), help_intro(), usage()] {
            assert!(text.is_ascii());
        }
    }
}
