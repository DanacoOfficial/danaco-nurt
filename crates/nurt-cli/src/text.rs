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
         \x20 -V, --version         print the version\n\
         \x20 -h, --help            print this help",
        product::help_summary()
    )
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
        assert!(banner.contains(&format!("Danaco Nurt {VERSION}")));
        assert!(banner.ends_with("Type 'help' to get started."));
    }

    #[test]
    fn full_version_has_the_copyright_and_the_license() {
        let lines: Vec<String> = full_version().lines().map(str::to_owned).collect();
        assert!(lines[1].starts_with("Copyright (c) 2026"));
        assert_eq!(lines[2], "License: Apache-2.0");
    }

    #[test]
    fn product_lines_are_ascii() {
        for text in [
            short_version(),
            full_version(),
            banner(),
            help_intro(),
            usage(),
        ] {
            assert!(text.is_ascii());
        }
    }
}
