// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixed texts of the program: version lines, banner and help introduction.
//!
//! The version comes from the package metadata. The commit and the date of
//! the build are read from the build environment variables
//! `NURT_BUILD_COMMIT` and `NURT_BUILD_DATE`; a build that does not know them
//! omits the parenthesis of the full version line.

/// Version of the program.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Canonical description of the product. It is a single constant of the
/// command line interface until the shared constant of the contracts replaces
/// it.
const TAGLINE: &str = "Command shell and language where every change can be undone";

const COPYRIGHT: &str = "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.";
const LICENSE: &str = "Apache-2.0";
const BUILD_COMMIT: Option<&str> = option_env!("NURT_BUILD_COMMIT");
const BUILD_DATE: Option<&str> = option_env!("NURT_BUILD_DATE");

/// Output of `nurt -V`.
#[must_use]
pub fn short_version() -> String {
    format!("nurt {VERSION}")
}

/// Output of `nurt --version` and `nurt version`: version with the build
/// data, copyright and license.
#[must_use]
pub fn full_version() -> String {
    let build = match (BUILD_COMMIT, BUILD_DATE) {
        (Some(commit), Some(date)) => format!(" ({commit} {date})"),
        _ => String::new(),
    };
    format!("nurt {VERSION}{build}\n{COPYRIGHT}\nLicense: {LICENSE}")
}

/// Banner of an interactive session.
#[must_use]
pub fn banner() -> String {
    format!("Danaco Nurt {VERSION}\n{COPYRIGHT}\nType 'help' to get started.")
}

/// Introduction printed by `help` without a topic.
#[must_use]
pub fn help_intro() -> String {
    let mut tagline = TAGLINE.to_owned();
    if let Some(first) = tagline.get_mut(..1) {
        first.make_ascii_lowercase();
    }
    format!(
        "Danaco Nurt {VERSION} - {tagline}\n\n\
         Run 'help <topic>' for a command or a topic.\n\
         Run 'catalog' to list the commands."
    )
}

/// Output of `nurt --help`.
#[must_use]
pub fn usage() -> String {
    format!(
        "{TAGLINE}.\n\n\
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
         \x20 -h, --help            print this help"
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
