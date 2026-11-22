// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of compiling patterns and of reading the searched input.

use std::error::Error as StdError;
use std::fmt;
use std::io;

use nurt_contracts::error::ErrorCode;

/// Error of building a [`Searcher`](super::Searcher) or of reading its input.
///
/// [`SearchError::code`] gives the error code of the language:
///
/// | Variant | Code |
/// |---|---|
/// | [`SearchError::Pattern`] | `E_REGEX` |
/// | [`SearchError::TooLarge`] | `E_LIMIT` |
/// | [`SearchError::Io`] | `E_IO` |
///
/// A pattern that reaches the step limit of the backtracking engine while
/// searching is not an error: the text being searched is treated as not
/// matching and [`Summary::backtrack_limit_hits`](super::Summary) counts it.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::ErrorCode;
/// use nurt_text::search::{Pattern, SearchError, SearchOptions, Searcher};
///
/// let error = Searcher::new(&[Pattern::regex("a(b")], SearchOptions::new()).unwrap_err();
/// assert_eq!(error.code(), ErrorCode::Regex);
/// assert!(matches!(error, SearchError::Pattern { pattern: 0, position: Some(2), .. }));
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum SearchError {
    /// A pattern is not a valid regular expression.
    Pattern {
        /// Index of the pattern in the list given to the searcher, from 0.
        pattern: usize,
        /// Position of the error in the pattern, in Unicode scalar values
        /// from 1, when the engine reports it.
        position: Option<usize>,
        /// Description of the error.
        message: String,
    },
    /// A pattern exceeds the size limit of the compiled expression.
    TooLarge {
        /// Index of the pattern in the list given to the searcher, from 0.
        pattern: usize,
        /// Size limit in bytes.
        limit: usize,
    },
    /// Reading the input failed.
    Io(io::Error),
}

impl SearchError {
    /// Error code of the language for this error.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Pattern { .. } => ErrorCode::Regex,
            Self::TooLarge { .. } => ErrorCode::Limit,
            Self::Io(_) => ErrorCode::Io,
        }
    }
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pattern {
                pattern,
                position: Some(position),
                message,
            } => write!(
                f,
                "invalid regular expression in pattern {pattern} at character {position}: {message}"
            ),
            Self::Pattern {
                pattern,
                position: None,
                message,
            } => {
                write!(
                    f,
                    "invalid regular expression in pattern {pattern}: {message}"
                )
            }
            Self::TooLarge { pattern, limit } => write!(
                f,
                "pattern {pattern} exceeds the compiled size limit of {limit} bytes; \
                 use fewer or smaller repetitions"
            ),
            Self::Io(error) => write!(f, "cannot read the searched input: {error}"),
        }
    }
}

impl StdError for SearchError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Pattern { .. } | Self::TooLarge { .. } => None,
        }
    }
}

impl From<io::Error> for SearchError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
