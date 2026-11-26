// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Character normalization of program text.
//!
//! Normalization is the first step of correction. Outside literals it writes
//! no-break spaces and other Unicode spaces as spaces, removes characters of
//! zero width, turns Unicode line separators into line feeds and brings words
//! into Unicode normalization form C. Each of these changes except the last is
//! reported as a correction of rule `L17`; normalization form C only changes
//! words outside ASCII, which are never names of the language.
//!
//! Literals keep every byte: text, raw text, regular expressions, unquoted
//! paths, text in single or typographic quotes and comments. A byte order mark
//! at the start of the text and carriage returns before line feeds are left to
//! the lexer and are not corrections.

mod positions;

use crate::record::{Correction, CorrectionKind};
use unicode_normalization::{UnicodeNormalization, is_nfc};

use crate::lexical::scan::{Piece, PieceKind, scan};
use crate::lexical::{Rule, number_corrections};

pub use positions::PositionMap;
pub(crate) use positions::{Edit, LineIndex, apply_edits};

/// Program text after character normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    /// Normalized text.
    pub text: String,
    /// Corrections of rule `L17`, with positions in the text as written.
    pub corrections: Vec<Correction>,
    /// Map from positions of [`Normalized::text`] to the text as written.
    pub positions: PositionMap,
}

/// Normalizes the characters of a program text outside literals.
///
/// The function is total: it accepts any text and never fails. A text without
/// Unicode spaces, zero-width characters and non-normalized words outside
/// literals is returned unchanged, with no corrections.
///
/// # Examples
///
/// ```
/// use nurt_correct::normalize::normalize;
///
/// let normalized = normalize("find\u{a0}\"/srv\u{a0}app\"");
/// assert_eq!(normalized.text, "find \"/srv\u{a0}app\"");
/// assert_eq!(normalized.corrections.len(), 1);
/// assert_eq!(normalized.corrections[0].rule.to_string(), "L17");
/// ```
#[must_use]
pub fn normalize(text: &str) -> Normalized {
    let pieces = scan(text);
    let index = LineIndex::new(text);
    let mut edits = Vec::new();
    let mut corrections = Vec::new();
    let mut run: Vec<&Piece> = Vec::new();
    for piece in &pieces {
        if is_space_like(piece.kind) {
            run.push(piece);
            continue;
        }
        flush_run(text, &index, &mut run, &mut edits, &mut corrections);
        if !piece.kind.is_literal() {
            let slice = piece.text(text);
            if !slice.is_ascii() && !is_nfc(slice) {
                edits.push(Edit::new(
                    piece.range.clone(),
                    slice.nfc().collect::<String>(),
                ));
            }
        }
    }
    flush_run(text, &index, &mut run, &mut edits, &mut corrections);
    number_corrections(&mut corrections);
    let (normalized, positions) = apply_edits(text, &edits);
    Normalized {
        text: normalized,
        corrections,
        positions,
    }
}

fn is_space_like(kind: PieceKind) -> bool {
    matches!(
        kind,
        PieceKind::ForeignSpace | PieceKind::Invisible | PieceKind::LineSeparator
    )
}

/// Replaces a run of Unicode spaces, zero-width characters and line
/// separators with one correction.
fn flush_run(
    text: &str,
    index: &LineIndex<'_>,
    run: &mut Vec<&Piece>,
    edits: &mut Vec<Edit>,
    corrections: &mut Vec<Correction>,
) {
    let (Some(first), Some(last)) = (run.first(), run.last()) else {
        return;
    };
    let range = first.range.start..last.range.end;
    let line_break = run
        .iter()
        .any(|piece| piece.kind == PieceKind::LineSeparator);
    let space = run
        .iter()
        .any(|piece| piece.kind == PieceKind::ForeignSpace);
    let replacement = if line_break {
        "\n"
    } else if space && !next_to_separator(text, range.start, range.end) {
        " "
    } else {
        ""
    };
    let from = code_points(text.get(range.clone()).unwrap_or_default());
    let to = code_points(replacement);
    corrections.push(Rule::L17.record(
        index.position(range.clone()),
        from,
        to,
        CorrectionKind::Whitespace,
    ));
    edits.push(Edit::new(range, replacement));
    run.clear();
}

/// Whether the text has a space, a tabulator or a line end directly before
/// `start` or after `end`, or `start` is the start of the text, so that a run
/// of Unicode spaces between them is removed rather than written as a space.
fn next_to_separator(text: &str, start: usize, end: usize) -> bool {
    let before = text.get(..start).and_then(|head| head.chars().next_back());
    let after = text.get(end..).and_then(|tail| tail.chars().next());
    let separator = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    before.is_none_or(separator) || after.is_none_or(separator)
}

/// Writes characters as Unicode code points, `U+00A0 U+200B`, because the
/// characters themselves are not visible in a report.
fn code_points(text: &str) -> String {
    text.chars()
        .map(|c| format!("U+{:04X}", u32::from(c)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use nurt_contracts::report::{CorrectionClass, TextPosition};

    use super::*;

    #[test]
    fn leaves_a_valid_program_unchanged() {
        let text = "find \"/srv/app\" glob: \"**/*.go\"\n|> filter size > 1mb # x\u{a0}y";
        let normalized = normalize(text);
        assert_eq!(normalized.text, text);
        assert!(normalized.corrections.is_empty());
        assert_eq!(normalized.positions.to_original(7), 7);
    }

    #[test]
    fn writes_no_break_space_as_space() {
        let normalized = normalize("find\u{a0}\"/a\"");
        assert_eq!(normalized.text, "find \"/a\"");
        let correction = &normalized.corrections[0];
        assert_eq!(correction.id, "C1");
        assert_eq!(correction.rule.to_string(), "L17");
        assert_eq!(correction.class, CorrectionClass::Certain);
        assert_eq!(correction.from, "U+00A0");
        assert_eq!(correction.to, "U+0020");
        assert_eq!(
            correction.at,
            Some(TextPosition {
                line: 1,
                column: 5,
                length: Some(1)
            })
        );
    }

    #[test]
    fn removes_zero_width_characters_inside_names() {
        let normalized = normalize("fi\u{200b}nd \"/a\" |> co\u{feff}unt");
        assert_eq!(normalized.text, "find \"/a\" |> count");
        assert_eq!(normalized.corrections.len(), 2);
        assert_eq!(normalized.corrections[1].to, "");
    }

    #[test]
    fn keeps_literal_bytes_including_decomposed_names() {
        let text = "find \"/a/zoe\u{308}\u{a0}x\" glob: 'e\u{301}\u{a0}*' /a\u{a0}b/ r\"\u{200b}\"";
        let normalized = normalize(text);
        assert_eq!(normalized.text, text);
        assert!(normalized.corrections.is_empty());
    }

    #[test]
    fn merges_a_run_of_unicode_spaces_into_one_correction() {
        let normalized = normalize("find\u{2003}\u{200b}\u{a0}\"/a\"");
        assert_eq!(normalized.text, "find \"/a\"");
        assert_eq!(normalized.corrections.len(), 1);
        assert_eq!(normalized.corrections[0].from, "U+2003 U+200B U+00A0");
        assert_eq!(
            normalized.corrections[0].at.and_then(|at| at.length),
            Some(3)
        );
    }

    #[test]
    fn removes_unicode_space_next_to_an_ascii_space() {
        let normalized = normalize("find \u{a0}\"/a\" |>\u{a0} count\u{a0}");
        assert_eq!(normalized.text, "find \"/a\" |> count");
        assert_eq!(normalized.corrections.len(), 3);
        assert!(
            normalized
                .corrections
                .iter()
                .all(|correction| correction.to.is_empty())
        );
    }

    #[test]
    fn turns_line_separators_into_line_feeds() {
        let normalized = normalize("list \"/a\"\u{2028}count");
        assert_eq!(normalized.text, "list \"/a\"\ncount");
        assert_eq!(normalized.corrections[0].to, "U+000A");
    }

    #[test]
    fn brings_foreign_words_into_composed_form_without_a_correction() {
        let normalized = normalize("e\u{301}cho");
        assert_eq!(normalized.text, "\u{e9}cho");
        assert!(normalized.corrections.is_empty());
        assert_eq!(normalized.positions.to_original(normalized.text.len()), 6);
    }

    #[test]
    fn leaves_empty_and_crlf_text_unchanged() {
        let empty = normalize("");
        assert_eq!(empty.text, "");
        assert!(empty.corrections.is_empty());
        let crlf = normalize("list \"/a\"\r\ncount\r\n");
        assert_eq!(crlf.text, "list \"/a\"\r\ncount\r\n");
        assert!(crlf.corrections.is_empty());
    }

    #[test]
    fn removes_a_unicode_space_at_the_start_of_the_text() {
        let normalized = normalize("\u{a0}list \"/a\"");
        assert_eq!(normalized.text, "list \"/a\"");
        assert_eq!(normalized.corrections.len(), 1);
        assert_eq!(normalized.corrections[0].to, "");
        assert_eq!(normalized.positions.to_original(0), 2);
    }

    #[test]
    fn reports_positions_in_the_text_as_written() {
        let normalized = normalize("\u{feff}find \"/a\"\nlist\u{a0}\"/b\"");
        let at = normalized.corrections[0].at;
        assert_eq!(at.map(|at| (at.line, at.column)), Some((2, 5)));
        assert_eq!(normalized.text, "\u{feff}find \"/a\"\nlist \"/b\"");
    }
}
