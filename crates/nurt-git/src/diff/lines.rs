// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Line comparison of two texts.
//!
//! Lines keep their terminator, so a last line without a line feed differs
//! from the same line with one, as in Git. The changes are placed by the
//! slider rules of Git: runs are merged where possible and moved down, and the
//! indentation heuristic (`diff.indentHeuristic`) chooses among equivalent
//! positions.

use std::ops::Range;

use gix_imara_diff::{Algorithm as Engine, Diff, InternedInput};

/// Algorithm of the line comparison, as named by `diff.algorithm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Algorithm {
    /// Myers with the speed heuristics of Git.
    #[default]
    Myers,
    /// Myers without the speed heuristics, giving a minimal difference.
    Minimal,
    /// Histogram.
    Histogram,
}

impl Algorithm {
    /// Parses a value of `diff.algorithm`.
    ///
    /// Returns the algorithm and whether it substitutes the requested one:
    /// `patience` is computed with the histogram algorithm, which shares its
    /// unique-line anchoring.
    #[must_use]
    pub fn from_config(value: &str) -> Option<(Self, bool)> {
        match value.to_ascii_lowercase().as_str() {
            "myers" | "default" => Some((Self::Myers, false)),
            "minimal" => Some((Self::Minimal, false)),
            "histogram" => Some((Self::Histogram, false)),
            "patience" => Some((Self::Histogram, true)),
            _ => None,
        }
    }

    fn engine(self) -> Engine {
        match self {
            Self::Myers => Engine::Myers,
            Self::Minimal => Engine::MyersMinimal,
            Self::Histogram => Engine::Histogram,
        }
    }
}

/// Options of a line comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOptions {
    /// Algorithm.
    pub algorithm: Algorithm,
    /// Lines that differ only in white space are equal (`-w`).
    pub ignore_whitespace: bool,
    /// Place ambiguous changes by indentation (`diff.indentHeuristic`).
    pub indent_heuristic: bool,
}

impl Default for LineOptions {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::Myers,
            ignore_whitespace: false,
            indent_heuristic: true,
        }
    }
}

/// A run of removed lines replaced by a run of added lines; either run may be
/// empty. Ranges are 0-based line indexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Removed lines of the old text.
    pub old: Range<usize>,
    /// Added lines of the new text.
    pub new: Range<usize>,
}

/// White space as `git` classifies it (`sane_ctype`): space, horizontal tab,
/// line feed and carriage return. The vertical tab and the form feed are not
/// white space to `git`.
pub(crate) fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// Splits a text into lines that keep their line feed.
pub(crate) fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    data.split_inclusive(|&byte| byte == b'\n').collect()
}

/// A line without its line feed.
pub(crate) fn line_body(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

/// The line consists of white space only.
pub(crate) fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|&byte| is_space(byte))
}

/// Changes that turn `old` into `new`, in order.
pub(crate) fn changes(old: &[&[u8]], new: &[&[u8]], options: &LineOptions) -> Vec<Change> {
    if options.ignore_whitespace {
        let strip = |line: &&[u8]| -> Vec<u8> {
            line.iter()
                .copied()
                .filter(|&byte| !is_space(byte))
                .collect()
        };
        let old_keys: Vec<Vec<u8>> = old.iter().map(strip).collect();
        let new_keys: Vec<Vec<u8>> = new.iter().map(strip).collect();
        let old_refs: Vec<&[u8]> = old_keys.iter().map(Vec::as_slice).collect();
        let new_refs: Vec<&[u8]> = new_keys.iter().map(Vec::as_slice).collect();
        compute(&old_refs, &new_refs, options)
    } else {
        compute(old, new, options)
    }
}

fn compute(old: &[&[u8]], new: &[&[u8]], options: &LineOptions) -> Vec<Change> {
    let mut input: InternedInput<&[u8]> = InternedInput::default();
    input.update_before(old.iter().copied());
    input.update_after(new.iter().copied());
    let mut diff = Diff::compute(options.algorithm.engine(), &input);
    if options.indent_heuristic {
        diff.postprocess_lines(&input);
    } else {
        diff.postprocess_no_heuristic(&input);
    }
    diff.hunks()
        .map(|hunk| Change {
            old: hunk.before.start as usize..hunk.before.end as usize,
            new: hunk.after.start as usize..hunk.after.end as usize,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(old: &str, new: &str, options: &LineOptions) -> Vec<Change> {
        let old = split_lines(old.as_bytes());
        let new = split_lines(new.as_bytes());
        changes(&old, &new, options)
    }

    #[test]
    fn reports_replaced_line() {
        let found = run("a\nb\nc\n", "a\nB\nc\n", &LineOptions::default());
        assert_eq!(
            found,
            vec![Change {
                old: 1..2,
                new: 1..2
            }]
        );
    }

    #[test]
    fn missing_final_line_feed_is_a_change() {
        let found = run("a\nb", "a\nb\n", &LineOptions::default());
        assert_eq!(
            found,
            vec![Change {
                old: 1..2,
                new: 1..2
            }]
        );
    }

    #[test]
    fn ignores_whitespace_only_changes_when_asked() {
        let options = LineOptions {
            ignore_whitespace: true,
            ..LineOptions::default()
        };
        assert!(run("a b\n\tc\n", "ab\nc  \n", &options).is_empty());
        assert_eq!(run("a\n", "b\n", &options).len(), 1);
    }

    #[test]
    fn indentation_heuristic_places_inserted_block_after_closing_line() {
        let old = "fn a() {\n    x\n}\n\nfn c() {\n    z\n}\n";
        let new = "fn a() {\n    x\n}\n\nfn b() {\n    y\n}\n\nfn c() {\n    z\n}\n";
        let found = run(old, new, &LineOptions::default());
        assert_eq!(
            found,
            vec![Change {
                old: 4..4,
                new: 4..8
            }]
        );
    }

    #[test]
    fn parses_configured_algorithms() {
        assert_eq!(
            Algorithm::from_config("Histogram"),
            Some((Algorithm::Histogram, false))
        );
        assert_eq!(
            Algorithm::from_config("patience"),
            Some((Algorithm::Histogram, true))
        );
        assert_eq!(
            Algorithm::from_config("minimal"),
            Some((Algorithm::Minimal, false))
        );
        assert_eq!(
            Algorithm::from_config("default"),
            Some((Algorithm::Myers, false))
        );
        assert_eq!(Algorithm::from_config("other"), None);
    }

    #[test]
    fn classifies_white_space_like_git() {
        assert!(is_blank(b" \t\r\n"));
        assert!(!is_blank(b" \x0b\n"));
        assert!(!is_blank(b"\x0c\n"));
        assert!(!is_blank(b" x\n"));
        assert_eq!(line_body(b"x\n"), b"x");
        assert_eq!(line_body(b"x"), b"x");
    }
}
