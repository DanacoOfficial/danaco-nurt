// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::error
// Responsibility:   Syntax errors with source positions and suggested fixes.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Syntax errors with source positions and suggested fixes.

use std::fmt;
use std::ops::{Deref, DerefMut};

use nurt_contracts::error::{At, Error, ErrorCode, Stage};

use crate::lex::LexError;

/// Maximum number of ready replacements in [`SyntaxErrorData::fix`].
pub const MAX_FIXES: usize = 3;

/// Help topic attached to every grammar error.
pub const SYNTAX_HELP: &str = "help syntax";

/// Error of the program text or of a syntax tree document.
///
/// The codes are [`ErrorCode::Lexical`], [`ErrorCode::Escape`] and
/// [`ErrorCode::Syntax`] for program text, and [`ErrorCode::AstSchema`] and
/// [`ErrorCode::AstVersion`] for tree documents. Every error belongs to the
/// parse stage and converts into the language error
/// [`nurt_contracts::error::Error`]. The content is boxed so that results
/// stay small; the fields of [`SyntaxErrorData`] are reachable through
/// dereferencing.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::{At, Error, ErrorCode, Stage};
/// use nurt_syntax::error::SyntaxError;
///
/// let error = SyntaxError::syntax("expected a value after `glob:`")
///     .with_at(At::new(1, 6, 5))
///     .with_found("glob:")
///     .with_expected(["value"]);
/// assert_eq!(error.found.as_deref(), Some("glob:"));
/// let error = Error::from(error);
/// assert_eq!(error.code, ErrorCode::Syntax);
/// assert_eq!(error.stage(), Some(Stage::Parse));
/// assert_eq!(error.help.as_deref(), Some("help syntax"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError(Box<SyntaxErrorData>);

/// Content of a [`SyntaxError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxErrorData {
    /// Stable code.
    pub code: ErrorCode,
    /// Message in English that states what to correct.
    pub message: String,
    /// Position and length of the erroneous fragment.
    pub at: Option<At>,
    /// The fragment as written.
    pub found: Option<String>,
    /// Short name of the problem, for example `chained_comparison`.
    pub problem: Option<String>,
    /// Up to [`MAX_FIXES`] ready replacements, best first. Grammar errors
    /// give complete program texts; lexical errors give replacements of the
    /// erroneous literal.
    pub fix: Vec<String>,
    /// Help topic with the rules that apply.
    pub help: Option<String>,
    /// Kinds of tokens or constructs that would be valid at the position.
    pub expected: Vec<String>,
}

impl SyntaxError {
    /// Error with the code and the message and no further details.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self(Box::new(SyntaxErrorData {
            code,
            message: message.into(),
            at: None,
            found: None,
            problem: None,
            fix: Vec::new(),
            help: None,
            expected: Vec::new(),
        }))
    }

    /// Grammar error, [`ErrorCode::Syntax`], with the help topic
    /// [`SYNTAX_HELP`].
    #[must_use]
    pub fn syntax(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Syntax, message).with_help(SYNTAX_HELP)
    }

    /// Tree document that does not conform to its schema,
    /// [`ErrorCode::AstSchema`].
    #[must_use]
    pub fn ast_schema(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AstSchema, message)
    }

    /// Tree document of an unsupported format version,
    /// [`ErrorCode::AstVersion`].
    #[must_use]
    pub fn ast_version(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AstVersion, message)
    }

    /// The content.
    #[must_use]
    pub fn data(&self) -> &SyntaxErrorData {
        &self.0
    }

    /// Unwraps the content.
    #[must_use]
    pub fn into_data(self) -> SyntaxErrorData {
        *self.0
    }

    /// Sets the position.
    #[must_use]
    pub fn with_at(mut self, at: At) -> Self {
        self.0.at = Some(at);
        self
    }

    /// Sets the fragment as written.
    #[must_use]
    pub fn with_found(mut self, found: impl Into<String>) -> Self {
        self.0.found = Some(found.into());
        self
    }

    /// Sets the short name of the problem.
    #[must_use]
    pub fn with_problem(mut self, problem: impl Into<String>) -> Self {
        self.0.problem = Some(problem.into());
        self
    }

    /// Adds a ready replacement unless it is already listed or
    /// [`MAX_FIXES`] replacements are present.
    #[must_use]
    pub fn with_fix(mut self, program: impl Into<String>) -> Self {
        let program = program.into();
        if self.0.fix.len() < MAX_FIXES && !self.0.fix.contains(&program) {
            self.0.fix.push(program);
        }
        self
    }

    /// Sets the help topic.
    #[must_use]
    pub fn with_help(mut self, topic: impl Into<String>) -> Self {
        self.0.help = Some(topic.into());
        self
    }

    /// Sets the kinds of tokens or constructs expected at the position.
    #[must_use]
    pub fn with_expected<I, S>(mut self, expected: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.0.expected = expected.into_iter().map(Into::into).collect();
        self
    }
}

impl From<SyntaxErrorData> for SyntaxError {
    fn from(data: SyntaxErrorData) -> Self {
        Self(Box::new(data))
    }
}

impl Deref for SyntaxError {
    type Target = SyntaxErrorData;

    fn deref(&self) -> &SyntaxErrorData {
        &self.0
    }
}

impl DerefMut for SyntaxError {
    fn deref_mut(&mut self) -> &mut SyntaxErrorData {
        &mut self.0
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.at {
            Some(At {
                line: Some(line),
                column: Some(column),
                ..
            }) => write!(f, "{}: {} at {line}:{column}", self.code, self.message),
            _ => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for SyntaxError {}

impl From<LexError> for SyntaxError {
    fn from(error: LexError) -> Self {
        let data = error.into_data();
        Self::from(SyntaxErrorData {
            code: data.code,
            message: data.message,
            at: Some(data.at),
            found: Some(data.found),
            problem: Some(data.problem),
            fix: data.fix,
            help: data.help,
            expected: Vec::new(),
        })
    }
}

impl From<SyntaxError> for Error {
    fn from(error: SyntaxError) -> Self {
        let SyntaxErrorData {
            code,
            message,
            at,
            found,
            problem,
            fix,
            help,
            expected,
        } = error.into_data();
        let mut converted = Self::new(code, message).with_stage(Stage::Parse);
        if let Some(at) = at {
            converted = converted.with_at(at);
        }
        if let Some(found) = found {
            converted = converted.with_found(found);
        }
        if let Some(problem) = problem {
            converted = converted.with_problem(problem);
        }
        for program in fix {
            converted = converted.with_fix(program);
        }
        if let Some(help) = help {
            converted = converted.with_help(help);
        }
        if !expected.is_empty() {
            converted = converted.with_expected(serde_json::Value::from(expected));
        }
        converted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_up_to_three_distinct_fixes() {
        let error = SyntaxError::syntax("m")
            .with_fix("a")
            .with_fix("a")
            .with_fix("b")
            .with_fix("c")
            .with_fix("d");
        assert_eq!(error.fix, ["a", "b", "c"]);
    }

    #[test]
    fn converts_every_field_into_the_language_error() {
        let error = SyntaxError::syntax("expected `)`")
            .with_at(At::new(2, 3, 1))
            .with_found("]")
            .with_problem("unexpected_token")
            .with_fix("count")
            .with_expected(["`)`"]);
        let converted = Error::from(error);
        assert_eq!(converted.code, ErrorCode::Syntax);
        assert_eq!(converted.stage(), Some(Stage::Parse));
        assert_eq!(
            converted.at,
            Some(At::new(2, 3, 1).with_stage(Stage::Parse))
        );
        assert_eq!(converted.found.as_deref(), Some("]"));
        assert_eq!(converted.problem.as_deref(), Some("unexpected_token"));
        assert_eq!(converted.fix, ["count"]);
        assert_eq!(converted.help.as_deref(), Some(SYNTAX_HELP));
        assert_eq!(converted.expected, Some(serde_json::json!(["`)`"])));
    }

    #[test]
    fn omits_absent_fields_in_the_language_error() {
        let converted = Error::from(SyntaxError::ast_schema("missing member `program`"));
        assert_eq!(converted.code, ErrorCode::AstSchema);
        assert_eq!(converted.at, Some(At::stage(Stage::Parse)));
        assert_eq!(converted.expected, None);
        assert_eq!(converted.help, None);
        let converted = Error::from(SyntaxError::ast_version("version 2.0 is newer"));
        assert_eq!(converted.code, ErrorCode::AstVersion);
    }

    #[test]
    fn displays_code_message_and_position() {
        let error = SyntaxError::syntax("expected `)`").with_at(At::new(1, 4, 1));
        assert_eq!(error.to_string(), "E_SYNTAX: expected `)` at 1:4");
        let error = SyntaxError::ast_schema("bad");
        assert_eq!(error.to_string(), "E_AST_SCHEMA: bad");
    }

    #[test]
    fn exposes_and_releases_its_content() {
        let mut error = SyntaxError::syntax("m");
        error.problem = Some("p".to_owned());
        assert_eq!(error.data().problem.as_deref(), Some("p"));
        let data = error.clone().into_data();
        assert_eq!(SyntaxError::from(data), error);
    }

    #[test]
    fn keeps_the_fields_of_a_lexical_error() {
        let lexical = crate::lex::decode_text(r#""\d""#).expect_err("invalid escape");
        let error = SyntaxError::from(lexical);
        assert_eq!(error.code, ErrorCode::Escape);
        assert!(error.at.is_some());
        assert!(error.expected.is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
