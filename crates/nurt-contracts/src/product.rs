// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Product texts of Danaco Nurt.
//!
//! Every place that names or describes the product derives its text from
//! this module: the description of the package that builds the `nurt`
//! executable, `nurt -V` and `nurt --version`, the first line of
//! `nurt --help`, the first line of the `help` command, the banner of an
//! interactive session and the `NAME` section of the manual page. The texts
//! contain only ASCII characters; the version is passed in by the caller from
//! the package metadata.

/// Name of the product.
pub const PRODUCT_NAME: &str = "Danaco Nurt";

/// Name of the executable.
pub const PROGRAM_NAME: &str = "nurt";

/// Canonical description of the product, one sentence without a final period.
///
/// The `description` of the package that builds the `nurt` executable is
/// exactly this sentence, and the first paragraph of `README.md` contains it.
pub const TAGLINE: &str = "Command shell and language where every change can be undone";

/// Copyright line printed by `nurt --version` and by the session banner.
pub const COPYRIGHT_NOTICE: &str =
    "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.";

/// License line printed by `nurt --version`.
pub const LICENSE_NOTICE: &str = "License: Apache-2.0";

/// Last line of the session banner.
pub const BANNER_HINT: &str = "Type 'help' to get started.";

/// Name of the product drawn with characters, the first part of the session
/// banner. Each row is at most 28 characters wide and has no trailing space.
pub const BANNER_ART: [&str; 5] = [
    r" _   _  _   _  ____   _____",
    r"| \ | || | | ||  _ \ |_   _|",
    r"|  \| || | | || |_) |  | |",
    r"| |\  || |_| ||  _ <   | |",
    r"|_| \_| \___/ |_| \_\  |_|",
];

/// Returns the output of `nurt -V`, the program name and the version.
///
/// # Examples
///
/// ```
/// assert_eq!(nurt_contracts::product::version_line("0.1.0"), "nurt 0.1.0");
/// ```
#[must_use]
pub fn version_line(version: &str) -> String {
    format!("{PROGRAM_NAME} {version}")
}

/// Returns the output of `nurt --version` as three lines without a final
/// line break.
///
/// The argument `build` holds the commit and the date of the build. A build
/// without repository data, for example from a source package, passes
/// `None`, and the first line then has no parenthesized part.
///
/// # Examples
///
/// ```
/// let text = nurt_contracts::product::long_version("0.1.0", Some(("1a2b3c4", "2026-11-22")));
/// assert!(text.starts_with("nurt 0.1.0 (1a2b3c4 2026-11-22)\n"));
/// ```
#[must_use]
pub fn long_version(version: &str, build: Option<(&str, &str)>) -> String {
    let first = match build {
        Some((commit, date)) => format!("{} ({commit} {date})", version_line(version)),
        None => version_line(version),
    };
    format!("{first}\n{COPYRIGHT_NOTICE}\n{LICENSE_NOTICE}")
}

/// Returns the first line of `nurt --help`, which is the canonical sentence
/// with a final period.
#[must_use]
pub fn help_summary() -> String {
    format!("{TAGLINE}.")
}

/// Returns the first line of the `help` command of a session, which joins
/// the product name, the version and the canonical sentence.
///
/// # Examples
///
/// ```
/// let line = nurt_contracts::product::session_help_heading("0.1.0");
/// assert!(line.starts_with("Danaco Nurt 0.1.0 - command shell"));
/// ```
#[must_use]
pub fn session_help_heading(version: &str) -> String {
    format!("{PRODUCT_NAME} {version} - {}", lowercase_first(TAGLINE))
}

/// Returns the banner of an interactive session without a final line break:
/// the product name drawn with characters, an empty line and three lines of
/// text (the name with the version, the copyright and a hint).
///
/// The banner is written only to an interactive terminal; scripts, pipes and
/// `serve --stdio` receive nothing.
#[must_use]
pub fn banner(version: &str) -> String {
    format!(
        "{}\n\n{PRODUCT_NAME} {version}\n{COPYRIGHT_NOTICE}\n{BANNER_HINT}",
        BANNER_ART.join("\n")
    )
}

/// Returns the line of the `NAME` section of the manual page, which joins
/// the program name and the canonical sentence.
#[must_use]
pub fn man_name() -> String {
    format!("{PROGRAM_NAME} - {}", lowercase_first(TAGLINE))
}

/// Returns the text with its first character in lower case; the product texts
/// are ASCII, so one character is one byte.
fn lowercase_first(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TEXTS: &[&str] = &[
        PRODUCT_NAME,
        PROGRAM_NAME,
        TAGLINE,
        COPYRIGHT_NOTICE,
        LICENSE_NOTICE,
        BANNER_HINT,
    ];

    const BANNER_TEXT: &str = concat!(
        " _   _  _   _  ____   _____\n",
        "| \\ | || | | ||  _ \\ |_   _|\n",
        "|  \\| || | | || |_) |  | |\n",
        "| |\\  || |_| ||  _ <   | |\n",
        "|_| \\_| \\___/ |_| \\_\\  |_|\n",
        "\n",
        "Danaco Nurt 1.2.3\n",
        "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\n",
        "Type 'help' to get started."
    );

    #[test]
    fn canonical_sentence_is_exact() {
        assert_eq!(
            TAGLINE,
            "Command shell and language where every change can be undone"
        );
    }

    #[test]
    fn version_texts_are_exact() {
        assert_eq!(version_line("1.2.3"), "nurt 1.2.3");
        assert_eq!(
            long_version("1.2.3", Some(("0123abc", "2026-11-22"))),
            "nurt 1.2.3 (0123abc 2026-11-22)\n\
             Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\n\
             License: Apache-2.0"
        );
        assert_eq!(
            long_version("1.2.3", None),
            "nurt 1.2.3\n\
             Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\n\
             License: Apache-2.0"
        );
    }

    #[test]
    fn help_lines_are_exact() {
        assert_eq!(
            help_summary(),
            "Command shell and language where every change can be undone."
        );
        assert_eq!(
            session_help_heading("1.2.3"),
            "Danaco Nurt 1.2.3 - command shell and language where every change can be undone"
        );
    }

    #[test]
    fn banner_is_exact() {
        assert_eq!(banner("1.2.3"), BANNER_TEXT);
    }

    #[test]
    fn banner_art_is_narrow_ascii_without_trailing_space() {
        for row in BANNER_ART {
            assert!(row.is_ascii(), "{row}");
            assert!(row.len() <= 28, "{row}");
            assert_eq!(row, row.trim_end(), "{row}");
        }
    }

    #[test]
    fn manual_page_name_is_exact() {
        assert_eq!(
            man_name(),
            "nurt - command shell and language where every change can be undone"
        );
    }

    #[test]
    fn texts_are_ascii_without_audience_suffix() {
        let derived = [
            version_line("1.2.3"),
            long_version("1.2.3", Some(("0123abc", "2026-11-22"))),
            help_summary(),
            session_help_heading("1.2.3"),
            banner("1.2.3"),
            man_name(),
        ];
        for text in ALL_TEXTS
            .iter()
            .copied()
            .chain(derived.iter().map(String::as_str))
        {
            assert!(text.is_ascii(), "{text:?} is not ASCII");
            assert!(
                !text.contains("people and AI agents"),
                "{text:?} carries the audience suffix"
            );
        }
    }

    #[test]
    fn lowercase_first_handles_empty_text() {
        assert_eq!(lowercase_first(""), "");
        assert_eq!(lowercase_first("Ab"), "ab");
    }
}
