// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of binding.

use std::fmt;

use nurt_contracts::ast::Span;
use nurt_contracts::error::{At, Error, ErrorCode, Stage};

/// Error of binding a surface tree: a language error of stage `1`.
///
/// Binding never changes the world, so every error describes a mistake in
/// the program and, where possible, how to correct it: `expected` lists the
/// admissible names or types, `fix` holds ready replacement programs and
/// `next_steps` splits a program with more than one effect.
///
/// # Examples
///
/// ```
/// use nurt_bind::BindError;
/// use nurt_contracts::error::{Error, ErrorCode, Stage};
///
/// let error = BindError::from(Error::new(ErrorCode::UnknownCommand, "unknown command `remvoe`"));
/// assert_eq!(error.code(), ErrorCode::UnknownCommand);
/// assert_eq!(error.error().stage(), Some(Stage::Bind));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BindError(Error);

impl BindError {
    /// Creates an error of stage `1` with the code and the message.
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self(Error::new(code, message).with_stage(Stage::Bind))
    }

    /// Sets the place in the program from the span of a surface node.
    #[must_use]
    pub(crate) fn at(mut self, span: Option<Span>) -> Self {
        if let Some(span) = span {
            self.0 = self.0.with_at(place_of(span));
        }
        self
    }

    /// Sets what was expected: a list of names, types or values.
    #[must_use]
    pub(crate) fn expected(mut self, expected: &[String]) -> Self {
        self.0 = self
            .0
            .with_expected(serde_json::Value::from(expected.to_vec()));
        self
    }

    /// Sets the fragment that was found.
    #[must_use]
    pub(crate) fn found(mut self, found: impl Into<String>) -> Self {
        self.0 = self.0.with_found(found);
        self
    }

    /// Adds a named detail to the error.
    #[must_use]
    pub(crate) fn detail(mut self, name: &str, value: serde_json::Value) -> Self {
        self.0 = self.0.with_detail(name, value);
        self
    }

    /// Sets the help topic.
    #[must_use]
    pub(crate) fn help(mut self, topic: impl Into<String>) -> Self {
        self.0 = self.0.with_help(topic);
        self
    }

    /// Adds a ready replacement program.
    #[must_use]
    pub(crate) fn fix(mut self, program: impl Into<String>) -> Self {
        self.0 = self.0.with_fix(program);
        self
    }

    /// Adds a further call that completes the task.
    #[must_use]
    pub(crate) fn next_step(mut self, program: impl Into<String>) -> Self {
        self.0 = self.0.with_next_step(program);
        self
    }

    /// Returns the error code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.0.code()
    }

    /// Returns the language error.
    #[must_use]
    pub fn error(&self) -> &Error {
        &self.0
    }

    /// Returns the language error, consuming the binding error.
    #[must_use]
    pub fn into_error(self) -> Error {
        self.0
    }
}

impl From<Error> for BindError {
    /// Wraps a language error; the stage is set to `1` when the error has
    /// none.
    fn from(error: Error) -> Self {
        if error.stage().is_some() {
            Self(error)
        } else {
            Self(error.with_stage(Stage::Bind))
        }
    }
}

impl From<BindError> for Error {
    fn from(error: BindError) -> Self {
        error.0
    }
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for BindError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Place of a node in the program; the length is known only for a node on a
/// single line.
fn place_of(span: Span) -> At {
    let length = (span.line == span.end_line)
        .then(|| span.end_column.saturating_sub(span.column))
        .filter(|length| *length > 0);
    At {
        line: Some(span.line),
        column: Some(span.column),
        length,
        stage: Some(Stage::Bind),
    }
}

/// Joins two optional spans into the span that covers both.
pub(crate) fn join_spans(first: Option<Span>, last: Option<Span>) -> Option<Span> {
    match (first, last) {
        (Some(first), Some(last)) => Some(Span {
            line: first.line,
            column: first.column,
            end_line: last.end_line,
            end_column: last.end_column,
        }),
        (first, last) => first.or(last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(line: u32, column: u32, end_line: u32, end_column: u32) -> Span {
        Span {
            line,
            column,
            end_line,
            end_column,
        }
    }

    #[test]
    fn records_the_place_and_the_bind_stage() {
        let error = BindError::new(ErrorCode::Type, "wrong type").at(Some(span(2, 5, 2, 9)));
        let at = error.error().at.unwrap();
        assert_eq!(at.line, Some(2));
        assert_eq!(at.column, Some(5));
        assert_eq!(at.length, Some(4));
        assert_eq!(at.stage, Some(Stage::Bind));
    }

    #[test]
    fn omits_the_length_of_a_node_over_several_lines() {
        let error = BindError::new(ErrorCode::Type, "wrong type").at(Some(span(1, 5, 3, 2)));
        assert_eq!(error.error().at.unwrap().length, None);
    }

    #[test]
    fn omits_the_length_of_an_empty_node() {
        let error = BindError::new(ErrorCode::Type, "wrong type").at(Some(span(3, 7, 3, 7)));
        let at = error.error().at.unwrap();
        assert_eq!((at.line, at.column, at.length), (Some(3), Some(7), None));
    }

    #[test]
    fn leaves_the_place_unset_without_a_span() {
        let error = BindError::new(ErrorCode::Type, "wrong type").at(None);
        let at = error.error().at.unwrap_or_default();
        assert_eq!((at.line, at.column, at.length), (None, None, None));
        assert_eq!(at.stage, Some(Stage::Bind));
        assert_eq!(
            join_spans(Some(span(1, 1, 1, 2)), None),
            Some(span(1, 1, 1, 2))
        );
        assert_eq!(join_spans(None, None), None);
    }

    #[test]
    fn keeps_the_stage_of_a_wrapped_error() {
        let error = BindError::from(Error::new(ErrorCode::Internal, "x").with_stage(Stage::Parse));
        assert_eq!(error.error().stage(), Some(Stage::Parse));
        let error: Error = BindError::new(ErrorCode::Arity, "x").into();
        assert_eq!(error.stage(), Some(Stage::Bind));
    }

    #[test]
    fn joins_spans_from_the_first_to_the_last() {
        let joined = join_spans(Some(span(1, 3, 1, 5)), Some(span(1, 9, 1, 12)));
        assert_eq!(joined, Some(span(1, 3, 1, 12)));
        assert_eq!(
            join_spans(None, Some(span(1, 1, 1, 2))),
            Some(span(1, 1, 1, 2))
        );
    }
}
