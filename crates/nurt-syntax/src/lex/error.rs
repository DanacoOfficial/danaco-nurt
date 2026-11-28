// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::lex::error
// Responsibility:   Lexical error: `E_LEXICAL` for an invalid token, `E_ESCAPE` for an invalid escape sequence in a text.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::fmt;
use std::ops::{Deref, DerefMut, Range};

use nurt_contracts::error::{At, Error, ErrorCode, Stage};

use super::cursor::Cursor;

/// Lexical error: `E_LEXICAL` for an invalid token, `E_ESCAPE` for an
/// invalid escape sequence in a text.
///
/// The content is boxed so that results stay small; the fields of
/// [`LexErrorData`] are reachable through dereferencing.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::{Error, ErrorCode, Stage};
/// use nurt_syntax::lex::decode_text;
///
/// let error = decode_text(r#""\d+""#).unwrap_err();
/// assert_eq!(error.code, ErrorCode::Escape);
/// assert_eq!((error.at.line, error.at.column), (Some(1), Some(2)));
/// assert_eq!(error.fix[0], r#"r"\d+""#);
/// let error = Error::from(error);
/// assert_eq!(error.stage(), Some(Stage::Parse));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError(Box<LexErrorData>);

/// Content of a [`LexError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexErrorData {
    /// [`ErrorCode::Lexical`] or [`ErrorCode::Escape`].
    pub code: ErrorCode,
    /// Message in English that states what to correct.
    pub message: String,
    /// Position and length, in Unicode scalar values, of the erroneous
    /// fragment.
    pub at: At,
    /// The erroneous fragment as written.
    pub found: String,
    /// Short name of the problem, for example `unknown_escape`.
    pub problem: String,
    /// Up to three ready replacements of the fragment, best first; empty when
    /// no replacement is certain.
    pub fix: Vec<String>,
    /// Help topic with the rules that apply.
    pub help: Option<String>,
}

impl LexError {
    /// The content.
    #[must_use]
    pub fn data(&self) -> &LexErrorData {
        &self.0
    }

    /// Unwraps the content.
    #[must_use]
    pub fn into_data(self) -> LexErrorData {
        *self.0
    }
}

impl From<LexErrorData> for LexError {
    fn from(data: LexErrorData) -> Self {
        Self(Box::new(data))
    }
}

impl Deref for LexError {
    type Target = LexErrorData;

    fn deref(&self) -> &LexErrorData {
        &self.0
    }
}

impl DerefMut for LexError {
    fn deref_mut(&mut self) -> &mut LexErrorData {
        &mut self.0
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.at.line, self.at.column) {
            (Some(line), Some(column)) => {
                write!(f, "{}: {} at {line}:{column}", self.code, self.message)
            }
            _ => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for LexError {}

impl From<LexError> for Error {
    fn from(error: LexError) -> Self {
        let LexErrorData {
            code,
            message,
            at,
            found,
            problem,
            fix,
            help,
        } = error.into_data();
        let mut converted = Self::new(code, message)
            .with_stage(Stage::Parse)
            .with_at(at)
            .with_found(found)
            .with_problem(problem);
        for program in fix {
            converted = converted.with_fix(program);
        }
        match help {
            Some(topic) => converted.with_help(topic),
            None => converted,
        }
    }
}

/// Maximum number of ready replacements in [`LexErrorData::fix`].
pub(crate) const MAX_FIXES: usize = 3;

/// Error found by a scanner, with byte offsets into the scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fault {
    pub(crate) code: ErrorCode,
    /// Erroneous fragment.
    pub(crate) range: Range<usize>,
    /// Offset where strict lexing resumes; at least `range.end`.
    pub(crate) resume: usize,
    pub(crate) problem: &'static str,
    pub(crate) message: String,
    pub(crate) fix: Vec<String>,
    pub(crate) help: &'static str,
}

impl Fault {
    /// Lexical error of the fragment `range`.
    pub(crate) fn lexical(
        range: Range<usize>,
        problem: &'static str,
        message: impl Into<String>,
        help: &'static str,
    ) -> Self {
        Self {
            code: ErrorCode::Lexical,
            resume: range.end,
            range,
            problem,
            message: message.into(),
            fix: Vec::new(),
            help,
        }
    }

    /// Escape error of the fragment `range`.
    pub(crate) fn escape(
        range: Range<usize>,
        problem: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: ErrorCode::Escape,
            resume: range.end,
            range,
            problem,
            message: message.into(),
            fix: Vec::new(),
            help: "text",
        }
    }

    /// Adds a ready replacement unless it is already listed or the list is
    /// full.
    pub(crate) fn with_fix(mut self, fix: impl Into<String>) -> Self {
        let fix = fix.into();
        if self.fix.len() < MAX_FIXES && !self.fix.contains(&fix) {
            self.fix.push(fix);
        }
        self
    }

    /// Adds a ready replacement when one is known.
    pub(crate) fn with_optional_fix(self, fix: Option<String>) -> Self {
        match fix {
            Some(fix) => self.with_fix(fix),
            None => self,
        }
    }

    /// Sets the offset where strict lexing resumes.
    pub(crate) fn resume_at(mut self, offset: usize) -> Self {
        self.resume = offset.max(self.range.end);
        self
    }

    /// Converts the fault into an error; `origin` is a byte offset of `text`
    /// at or before the fragment and `cursor` its position.
    pub(crate) fn into_error(self, text: &str, origin: usize, cursor: Cursor) -> LexError {
        let start = self.range.start.min(text.len());
        let end = self.range.end.clamp(start, text.len());
        let position = cursor.after(text.get(origin..start).unwrap_or_default());
        let found = text.get(start..end).unwrap_or_default();
        LexError::from(LexErrorData {
            code: self.code,
            message: self.message,
            at: At::new(position.line, position.column, Cursor::count(found)),
            found: found.to_owned(),
            problem: self.problem.to_owned(),
            fix: self.fix,
            help: Some(self.help.to_owned()),
        })
    }

    /// Converts a fault of a standalone literal `src` into an error.
    pub(crate) fn into_standalone(self, src: &str) -> LexError {
        self.into_error(src, 0, Cursor::START)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
