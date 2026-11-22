// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Diagnostics and formatting of a document.

use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range, TextEdit};
use nurt_contracts::error::At;
use nurt_syntax::error::{SyntaxError, SyntaxErrorData};
use nurt_syntax::fmt;
use nurt_syntax::parse::{ParseOptions, Variant, parse};

use crate::position::{Encoding, Lines};

/// Source name of every diagnostic.
pub(crate) const SOURCE: &str = "nurt";

/// Problem of a text without statements; an empty document is not an error
/// while it is being written.
const EMPTY_PROGRAM: &str = "empty_program";

/// Returns the diagnostics of a document: the error of the parser, if any.
pub(crate) fn diagnostics(text: &str, variant: Variant, encoding: Encoding) -> Vec<Diagnostic> {
    match parse(text, variant, &ParseOptions::default()) {
        Ok(_) => Vec::new(),
        Err(error) if is_empty_program(&error) => Vec::new(),
        Err(error) => vec![diagnostic(&Lines::new(text), error.data(), encoding)],
    }
}

/// Formats a document into its canonical text.
///
/// Returns no edit when the text is canonical or empty, and one edit of the
/// whole document otherwise.
///
/// # Errors
///
/// Returns a message with the position of the syntax error when the text is
/// not a valid program.
pub(crate) fn formatting(
    text: &str,
    variant: Variant,
    encoding: Encoding,
) -> Result<Vec<TextEdit>, String> {
    let formatted = match fmt::format(text, variant) {
        Ok(formatted) => formatted,
        Err(error) if is_empty_program(&error) => return Ok(Vec::new()),
        Err(error) => return Err(format_failure(&error)),
    };
    if formatted == text {
        return Ok(Vec::new());
    }
    let range = Lines::new(text).range(0..text.len(), encoding);
    Ok(vec![TextEdit::new(range, formatted)])
}

fn is_empty_program(error: &SyntaxError) -> bool {
    error.problem.as_deref() == Some(EMPTY_PROGRAM)
}

fn diagnostic(lines: &Lines<'_>, error: &SyntaxErrorData, encoding: Encoding) -> Diagnostic {
    let mut message = error.message.clone();
    for fix in &error.fix {
        message.push_str("\nfix: ");
        message.push_str(fix);
    }
    Diagnostic {
        range: error_range(lines, error.at, encoding),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(error.code.as_str().to_owned())),
        source: Some(SOURCE.to_owned()),
        message,
        ..Diagnostic::default()
    }
}

/// Range of the fragment an error points at; the start of the document when
/// the error has no position.
fn error_range(lines: &Lines<'_>, at: Option<At>, encoding: Encoding) -> Range {
    let Some((line, column)) = at.and_then(|at| at.line.zip(at.column)) else {
        return Range::default();
    };
    let start = lines.column_offset(line, column);
    let length = at.and_then(|at| at.length).unwrap_or(0);
    lines.range(start..lines.advance(start, length), encoding)
}

fn format_failure(error: &SyntaxError) -> String {
    match error.at.and_then(|at| at.line.zip(at.column)) {
        Some((line, column)) => {
            format!(
                "cannot format the document: {} at line {line}, column {column}",
                error.message
            )
        }
        None => format!("cannot format the document: {}", error.message),
    }
}

#[cfg(test)]
mod tests {
    use lsp_types::Position;

    use super::*;

    #[test]
    fn reports_no_diagnostics_for_valid_and_empty_programs() {
        assert!(diagnostics("find \"/srv\" |> count\n", Variant::A, Encoding::Utf16).is_empty());
        assert!(diagnostics("", Variant::A, Encoding::Utf16).is_empty());
        assert!(diagnostics("# only a comment\n", Variant::A, Encoding::Utf16).is_empty());
    }

    #[test]
    fn reports_the_parser_error_with_code_range_and_fix() {
        let found = diagnostics("filter a < b < c", Variant::A, Encoding::Utf16);
        let [diagnostic] = found.as_slice() else {
            panic!("expected one diagnostic: {found:?}")
        };
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("E_SYNTAX".to_owned()))
        );
        assert_eq!(diagnostic.source.as_deref(), Some(SOURCE));
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        assert!(
            diagnostic
                .message
                .ends_with("\nfix: filter a < b and b < c")
        );
        assert_eq!(diagnostic.range.start.line, 0);
    }

    #[test]
    fn measures_error_ranges_in_the_negotiated_encoding() {
        let text = "count \"\u{1f600}\" )";
        let utf16 = diagnostics(text, Variant::A, Encoding::Utf16);
        let utf32 = diagnostics(text, Variant::A, Encoding::Utf32);
        assert_eq!(utf16[0].range.start, Position::new(0, 11));
        assert_eq!(utf32[0].range.start, Position::new(0, 10));
    }

    #[test]
    fn formats_into_one_edit_of_the_whole_document() {
        let text = "find \"/srv/app\"   glob: \"*.go\" |> first 0x14\n";
        let edits = formatting(text, Variant::A, Encoding::Utf16).unwrap();
        let [edit] = edits.as_slice() else {
            panic!("expected one edit: {edits:?}")
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 0), Position::new(1, 0))
        );
        assert_eq!(
            edit.new_text,
            "find \"/srv/app\" glob: \"*.go\"\n|> first 20\n"
        );
        let canonical = edit.new_text.clone();
        assert!(
            formatting(&canonical, Variant::A, Encoding::Utf16)
                .unwrap()
                .is_empty()
        );
        assert!(
            formatting("", Variant::A, Encoding::Utf16)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn refuses_to_format_invalid_programs() {
        let failure = formatting("count )", Variant::A, Encoding::Utf16).unwrap_err();
        assert!(
            failure.starts_with("cannot format the document: "),
            "{failure}"
        );
        assert!(failure.ends_with("at line 1, column 7"), "{failure}");
    }
}
