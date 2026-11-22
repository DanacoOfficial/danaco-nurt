// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Hunk identifiers.
//!
//! Grammar: `id = path "#" number [":" fingerprint] ["@" rows]`,
//! `rows = range ("," range)*`, `range = n ["-" m]`. The path is relative to
//! the repository root and may contain `#`; the last `#` followed by a valid
//! suffix ends it. The number counts the hunks of the file from 1 in the
//! canonical difference (three context lines, no rename detection), whatever
//! the options of the call that listed them. Rows count the lines of the hunk
//! from 1, including `\ No newline at end of file` lines.

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use nurt_contracts::error::{Error, ErrorCode};
use regex::Regex;

use super::hunk::FINGERPRINT_LEN;

/// Highest row number accepted in a selection.
pub const MAX_ROW: u32 = 5000;

/// Inclusive range of rows of a hunk, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowRange {
    /// First row.
    pub first: u32,
    /// Last row, not lower than `first`.
    pub last: u32,
}

impl RowRange {
    /// The range contains `row`.
    #[must_use]
    pub fn contains(&self, row: u32) -> bool {
        (self.first..=self.last).contains(&row)
    }
}

/// Identifier of a hunk, optionally with a fingerprint and a row selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkId {
    /// File relative to the repository root.
    pub path: String,
    /// Position of the hunk in the file, from 1.
    pub number: u32,
    /// Six lower-case hexadecimal digits that the hunk must still have.
    pub fingerprint: Option<String>,
    /// Selected rows; `None` selects the whole hunk.
    pub rows: Option<Vec<RowRange>>,
}

/// Problem with a hunk identifier or selection, reported in the `problem`
/// field of the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkProblem {
    /// The identifier does not follow the grammar.
    InvalidId,
    /// The file has no hunk with this number.
    NotFound,
    /// The hunk no longer has the given fingerprint.
    Changed,
    /// A row is out of the hunk or out of the accepted range.
    InvalidRows,
    /// The selected rows contain no added or removed line.
    NoChangeRows,
    /// Rows were selected in an added or deleted file.
    RowsOnAddedOrDeleted,
    /// The base text does not match the hunk.
    ApplyFailed,
}

impl HunkProblem {
    /// Name of the problem.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidId => "invalid_id",
            Self::NotFound => "not_found",
            Self::Changed => "changed",
            Self::InvalidRows => "invalid_rows",
            Self::NoChangeRows => "no_change_rows",
            Self::RowsOnAddedOrDeleted => "rows_on_added_or_deleted",
            Self::ApplyFailed => "apply_failed",
        }
    }

    fn code(self) -> ErrorCode {
        match self {
            Self::NotFound => ErrorCode::NotFound,
            Self::Changed => ErrorCode::Stale,
            Self::ApplyFailed => ErrorCode::StageFailed,
            Self::InvalidId
            | Self::InvalidRows
            | Self::NoChangeRows
            | Self::RowsOnAddedOrDeleted => ErrorCode::Arg,
        }
    }
}

/// Error of a hunk identifier or selection with its problem name.
#[must_use]
pub fn hunk_error(problem: HunkProblem, message: impl Into<String>) -> Error {
    Error::new(problem.code(), message).with_problem(problem.as_str())
}

fn grammar() -> &'static Regex {
    static GRAMMAR: OnceLock<Regex> = OnceLock::new();
    GRAMMAR.get_or_init(|| {
        Regex::new(
            r"^(.+)#([0-9]+)(?::([0-9a-f]{6}))?(?:@([0-9]+(?:-[0-9]+)?(?:,[0-9]+(?:-[0-9]+)?)*))?$",
        )
        .unwrap_or_else(|_| unreachable!("the hunk identifier grammar is a valid expression"))
    })
}

impl FromStr for HunkId {
    type Err = Error;

    /// Parses an identifier.
    ///
    /// # Errors
    ///
    /// `E_ARG` with problem `invalid_id` when the text does not follow the
    /// grammar or the number is 0, and `invalid_rows` when a range is
    /// reversed, starts at 0 or exceeds [`MAX_ROW`].
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            hunk_error(
                HunkProblem::InvalidId,
                format!(
                    "invalid hunk identifier `{text}`, expected `path#number[:fingerprint][@rows]`"
                ),
            )
        };
        let captures = grammar().captures(text).ok_or_else(invalid)?;
        let number: u32 = captures[2].parse().map_err(|_| invalid())?;
        if number == 0 {
            return Err(invalid());
        }
        let rows = captures
            .get(4)
            .map(|rows| parse_rows(rows.as_str()))
            .transpose()?;
        Ok(Self {
            path: captures[1].to_owned(),
            number,
            fingerprint: captures.get(3).map(|found| found.as_str().to_owned()),
            rows,
        })
    }
}

fn parse_rows(text: &str) -> Result<Vec<RowRange>, Error> {
    let invalid = || {
        hunk_error(
            HunkProblem::InvalidRows,
            format!(
                "invalid rows `{text}`, use rows from 1 to {MAX_ROW} in ascending ranges such as `3-5,8`"
            ),
        )
    };
    text.split(',')
        .map(|range| {
            let (first, last) = range.split_once('-').unwrap_or((range, range));
            let first: u32 = first.parse().map_err(|_| invalid())?;
            let last: u32 = last.parse().map_err(|_| invalid())?;
            if first == 0 || last < first || last > MAX_ROW {
                return Err(invalid());
            }
            Ok(RowRange { first, last })
        })
        .collect()
}

impl fmt::Display for HunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.path, self.number)?;
        if let Some(fingerprint) = &self.fingerprint {
            write!(f, ":{fingerprint}")?;
        }
        if let Some(rows) = &self.rows {
            f.write_str("@")?;
            for (index, range) in rows.iter().enumerate() {
                if index > 0 {
                    f.write_str(",")?;
                }
                if range.first == range.last {
                    write!(f, "{}", range.first)?;
                } else {
                    write!(f, "{}-{}", range.first, range.last)?;
                }
            }
        }
        Ok(())
    }
}

impl HunkId {
    /// The identifier with the fingerprint of `fingerprint`.
    #[must_use]
    pub fn with_fingerprint(mut self, fingerprint: &str) -> Self {
        let digits: String = fingerprint.chars().take(FINGERPRINT_LEN).collect();
        self.fingerprint = Some(digits);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<HunkId, Error> {
        text.parse()
    }

    #[test]
    fn accepts_valid_identifiers() {
        let plain = parse("a.txt#1").unwrap();
        assert_eq!((plain.path.as_str(), plain.number), ("a.txt", 1));
        assert_eq!(plain.fingerprint, None);
        let full = parse("a.txt#1:abcdef").unwrap();
        assert_eq!(full.fingerprint.as_deref(), Some("abcdef"));
        let rows = parse("a.txt#1@3-5,8").unwrap();
        assert_eq!(
            rows.rows,
            Some(vec![
                RowRange { first: 3, last: 5 },
                RowRange { first: 8, last: 8 }
            ])
        );
        let hash_in_name = parse("a#b.txt#2:012345@1").unwrap();
        assert_eq!(hash_in_name.path, "a#b.txt");
        assert_eq!(hash_in_name.number, 2);
    }

    #[test]
    fn rejects_malformed_identifiers() {
        for text in [
            "a.txt",
            "a.txt#x",
            "a.txt#1:ABCDEF",
            "a.txt#1@",
            "a.txt#0",
            "#1",
        ] {
            let error = parse(text).unwrap_err();
            assert_eq!(
                error.data().problem.as_deref(),
                Some("invalid_id"),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_invalid_rows() {
        for text in ["a.txt#1@0", "a.txt#1@5-3", "a.txt#1@5001"] {
            let error = parse(text).unwrap_err();
            assert_eq!(
                error.data().problem.as_deref(),
                Some("invalid_rows"),
                "{text}"
            );
            assert_eq!(error.code(), ErrorCode::Arg);
        }
    }

    #[test]
    fn writes_identifier_back() {
        for text in ["a.txt#1", "dir/a b.txt#12:0a1b2c@1-2,4"] {
            assert_eq!(parse(text).unwrap().to_string(), text);
        }
        let id = parse("x#3").unwrap().with_fingerprint("abcdef0123");
        assert_eq!(id.to_string(), "x#3:abcdef");
    }

    #[test]
    fn maps_problems_to_codes() {
        assert_eq!(
            hunk_error(HunkProblem::Changed, "x").code(),
            ErrorCode::Stale
        );
        assert_eq!(
            hunk_error(HunkProblem::NotFound, "x").code(),
            ErrorCode::NotFound
        );
        assert_eq!(
            hunk_error(HunkProblem::ApplyFailed, "x").code(),
            ErrorCode::StageFailed
        );
        assert!(RowRange { first: 2, last: 4 }.contains(3));
    }
}
