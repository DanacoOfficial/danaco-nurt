// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Structural replacement with templates.
//!
//! A template is replacement text in which `$NAME` and `$$$NAME` stand for
//! the text captured by the metavariables of the pattern. A metavariable the
//! pattern did not capture is replaced by nothing. A `$` that does not start
//! a metavariable name (`$5`, `$x`) is kept.
//!
//! Indentation follows the matched code. Every line of the template after
//! the first is indented by the indentation of the line where the match
//! starts, and every line of a multi-line capture after the first keeps its
//! indentation relative to the line the capture starts on, placed at the
//! indentation of the template line that holds the metavariable.
//!
//! ```
//! use nurt_code::pattern::StructuralPattern;
//! use nurt_code::structural::replacement_edits;
//! use nurt_contracts::services::LanguageId;
//!
//! let js = LanguageId::new("javascript")?;
//! let grammar = tree_sitter::Language::new(tree_sitter_javascript::LANGUAGE);
//! let pattern = StructuralPattern::compile("var $A = $B", &js, &grammar)?;
//! let source = b"var x = 1;";
//! let found = pattern.search(source)?;
//! let edits = replacement_edits(source, &found, "let $A = $B");
//! assert_eq!(edits[0].text, "let x = 1");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use nurt_contracts::services::{ByteRange, SyntaxCapture, SyntaxMatch};

use crate::spans::LineIndex;

/// Number of sigils of a metavariable that matches a sequence of nodes
/// (`$$$NAME`).
const SEQUENCE_SIGILS: usize = 3;

/// Replacement of one byte range of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// Bytes replaced.
    pub bytes: ByteRange,
    /// New text.
    pub text: String,
}

/// Piece of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece<'t> {
    /// Literal text.
    Text(&'t str),
    /// Metavariable with its name.
    Var(&'t str),
}

/// Splits a template into literal text and metavariables.
fn parse_template(template: &str) -> Vec<Piece<'_>> {
    let bytes = template.as_bytes();
    let mut pieces = Vec::new();
    let mut literal_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let dollars = bytes[index..].iter().take_while(|b| **b == b'$').count();
        let name_start = index + dollars;
        let name_len = metavariable_name_len(&bytes[name_start..]);
        if name_len == 0 {
            index = name_start;
            continue;
        }
        let sigils = if dollars >= SEQUENCE_SIGILS {
            SEQUENCE_SIGILS
        } else {
            1
        };
        let var_start = name_start - sigils;
        pieces.push(Piece::Text(&template[literal_start..var_start]));
        pieces.push(Piece::Var(&template[name_start..name_start + name_len]));
        index = name_start + name_len;
        literal_start = index;
    }
    pieces.push(Piece::Text(&template[literal_start..]));
    pieces.retain(|piece| *piece != Piece::Text(""));
    pieces
}

/// Length of the metavariable name at the start of `bytes`: an upper-case
/// letter or `_`, then upper-case letters, digits and `_`.
fn metavariable_name_len(bytes: &[u8]) -> usize {
    match bytes.first() {
        Some(first) if first.is_ascii_uppercase() || *first == b'_' => bytes
            .iter()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || **b == b'_')
            .count(),
        _ => 0,
    }
}

/// Indentation of the lines a match and its captures start on.
trait Indents {
    /// Leading whitespace of the line where the match starts.
    fn of_match(&self) -> String;

    /// Leading whitespace of the line holding `offset` when that line is
    /// not the first line of the match; `None` on the first line.
    fn of_later_line(&self, offset: u64) -> Option<String>;
}

/// Indentation read from the document.
struct SourceIndents<'s> {
    lines: &'s LineIndex<'s>,
    match_start: u64,
    match_line: u32,
}

impl Indents for SourceIndents<'_> {
    fn of_match(&self) -> String {
        self.lines.indentation(to_usize(self.match_start))
    }

    fn of_later_line(&self, offset: u64) -> Option<String> {
        let offset = to_usize(offset);
        (self.lines.position(offset).line != self.match_line)
            .then(|| self.lines.indentation(offset))
    }
}

/// Indentation inferred from the match alone.
///
/// The indentation of the first line is not part of the match. It is taken
/// as the smallest indentation of the later lines of the match, which for
/// a block is the line of its closing bracket, cut to the column of the
/// match; a match on one line is assumed to start after spaces only.
struct MatchIndents<'m> {
    found: &'m SyntaxMatch,
}

impl Indents for MatchIndents<'_> {
    fn of_match(&self) -> String {
        let limit = usize::try_from(self.found.span.start.column.saturating_sub(1)).unwrap_or(0);
        let later = self
            .found
            .text
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(leading_whitespace)
            .min_by_key(|indent| indent.chars().count());
        match later {
            Some(indent) => indent.chars().take(limit).collect(),
            None => " ".repeat(limit),
        }
    }

    fn of_later_line(&self, offset: u64) -> Option<String> {
        let relative = to_usize(offset.checked_sub(self.found.span.bytes.start)?);
        let before = self.found.text.get(..relative)?;
        let line_start = before.rfind('\n')? + 1;
        Some(leading_whitespace(&self.found.text[line_start..]).to_owned())
    }
}

fn leading_whitespace(line: &str) -> &str {
    let end = line
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(line.len());
    &line[..end]
}

fn to_usize(offset: u64) -> usize {
    usize::try_from(offset).unwrap_or(usize::MAX)
}

/// Replacement text for a match, with indentation inferred from the match.
///
/// Prefer [`replacement_edits`] when the document is at hand: it reads the
/// indentation of the matched lines from the document itself.
///
/// # Examples
///
/// ```
/// use nurt_code::structural::render_template;
/// use nurt_contracts::services::{ByteRange, SyntaxCapture, SyntaxMatch, TextPosition, TextSpan};
///
/// let span = TextSpan {
///     bytes: ByteRange::new(0, 6),
///     start: TextPosition { line: 1, column: 1 },
///     end: TextPosition { line: 1, column: 7 },
/// };
/// let found = SyntaxMatch {
///     span,
///     node_kind: "call_expression".to_owned(),
///     text: "f(1)".to_owned(),
///     captures: vec![SyntaxCapture { name: "A".to_owned(), spans: vec![], text: "1".to_owned() }],
/// };
/// assert_eq!(render_template(&found, "g($A)"), "g(1)");
/// ```
#[must_use]
pub fn render_template(found: &SyntaxMatch, template: &str) -> String {
    render(found, template, &MatchIndents { found })
}

/// Replacements of the matches by the template, in source order.
///
/// Matches nested in an earlier match are left out, so the replacements do
/// not overlap and can be applied together.
#[must_use]
pub fn replacement_edits(
    source: &[u8],
    matches: &[SyntaxMatch],
    template: &str,
) -> Vec<Replacement> {
    let lines = LineIndex::new(source);
    let crlf = dominant_line_ending_is_crlf(source);
    let mut sorted: Vec<&SyntaxMatch> = matches.iter().collect();
    sorted.sort_by_key(|found| {
        (
            found.span.bytes.start,
            std::cmp::Reverse(found.span.bytes.end),
        )
    });
    let mut edits: Vec<Replacement> = Vec::new();
    for found in sorted {
        if edits
            .last()
            .is_some_and(|last| found.span.bytes.start < last.bytes.end)
        {
            continue;
        }
        let indents = SourceIndents {
            lines: &lines,
            match_start: found.span.bytes.start,
            match_line: found.span.start.line,
        };
        let text = render(found, template, &indents);
        edits.push(Replacement {
            bytes: found.span.bytes,
            text: if crlf { to_crlf(&text) } else { text },
        });
    }
    edits
}

/// Returns `true` when the document has more `CRLF` than bare `LF` line
/// endings.
fn dominant_line_ending_is_crlf(source: &[u8]) -> bool {
    let (mut crlf, mut lf) = (0_usize, 0_usize);
    for (index, byte) in source.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && source[index - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
    }
    crlf > lf
}

/// Text with every line ending written as `CRLF`.
fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn render(found: &SyntaxMatch, template: &str, indents: &dyn Indents) -> String {
    let mut match_indent: Option<String> = None;
    let mut out = String::new();
    let mut template_line_indent = leading_whitespace(template).to_owned();
    for piece in parse_template(template) {
        match piece {
            Piece::Text(text) => {
                let mut lines = text.split('\n');
                if let Some(first) = lines.next() {
                    out.push_str(first);
                }
                for line in lines {
                    out.push('\n');
                    if !line.is_empty() {
                        out.push_str(match_indent.get_or_insert_with(|| indents.of_match()));
                    }
                    out.push_str(line);
                    template_line_indent = leading_whitespace(line).to_owned();
                }
            }
            Piece::Var(name) => {
                if let Some(capture) = found.captures.iter().find(|c| c.name == name) {
                    let target = Target {
                        template_indent: &template_line_indent,
                        match_indent: &mut match_indent,
                    };
                    insert_capture(&mut out, capture, target, indents);
                }
            }
        }
    }
    out
}

/// Where a capture is inserted: the indentation of its template line, and
/// the indentation of the match, computed on first use.
struct Target<'a> {
    template_indent: &'a str,
    match_indent: &'a mut Option<String>,
}

fn insert_capture(
    out: &mut String,
    capture: &SyntaxCapture,
    target: Target<'_>,
    indents: &dyn Indents,
) {
    let mut lines = capture.text.split('\n');
    if let Some(first) = lines.next() {
        out.push_str(first);
    }
    let start = capture.spans.first().map(|span| span.bytes.start);
    let original = start.and_then(|offset| indents.of_later_line(offset));
    for line in lines {
        out.push('\n');
        if line.trim().is_empty() {
            out.push_str(line.trim_start_matches([' ', '\t']));
            continue;
        }
        match &original {
            None => {
                out.push_str(target.template_indent);
                out.push_str(line);
            }
            Some(capture_indent) => {
                let base = target
                    .match_indent
                    .get_or_insert_with(|| indents.of_match());
                out.push_str(base);
                out.push_str(target.template_indent);
                out.push_str(strip_indent(line, capture_indent));
            }
        }
    }
}

/// Removes `indent` from the start of `line`, or as much leading whitespace
/// as the line has when it is indented less.
fn strip_indent<'l>(line: &'l str, indent: &str) -> &'l str {
    if let Some(rest) = line.strip_prefix(indent) {
        return rest;
    }
    let width = indent.chars().count();
    let cut = line
        .char_indices()
        .take(width)
        .take_while(|(_, c)| *c == ' ' || *c == '\t')
        .last()
        .map_or(0, |(index, c)| index + c.len_utf8());
    &line[cut..]
}

#[cfg(test)]
mod tests {
    use nurt_contracts::services::{TextPosition, TextSpan};

    use super::*;

    fn span(start: u64, end: u64, line: u32, column: u32) -> TextSpan {
        TextSpan {
            bytes: ByteRange::new(start, end),
            start: TextPosition { line, column },
            end: TextPosition { line, column },
        }
    }

    #[test]
    fn templates_split_into_text_and_metavariables() {
        assert_eq!(
            parse_template("f($A, $$$REST) $5 $x"),
            [
                Piece::Text("f("),
                Piece::Var("A"),
                Piece::Text(", "),
                Piece::Var("REST"),
                Piece::Text(") $5 $x"),
            ]
        );
        assert_eq!(parse_template("$$A"), [Piece::Text("$"), Piece::Var("A")]);
        assert_eq!(parse_template("$$$$A"), [Piece::Text("$"), Piece::Var("A")]);
        assert_eq!(parse_template("$"), [Piece::Text("$")]);
    }

    #[test]
    fn indentation_is_stripped_only_as_far_as_the_line_has_it() {
        assert_eq!(strip_indent("        x", "    "), "    x");
        assert_eq!(strip_indent("  x", "    "), "x");
        assert_eq!(strip_indent("\tx", "\t"), "x");
    }

    #[test]
    fn undefined_metavariables_are_replaced_by_nothing() {
        let found = SyntaxMatch {
            span: span(0, 1, 1, 1),
            node_kind: "identifier".to_owned(),
            text: "a".to_owned(),
            captures: Vec::new(),
        };
        assert_eq!(render_template(&found, "[$A]"), "[]");
    }

    #[test]
    fn inferred_indentation_uses_the_closing_line_of_the_match() {
        let found = SyntaxMatch {
            span: span(4, 20, 2, 5),
            node_kind: "block".to_owned(),
            text: "{\n        x;\n    }".to_owned(),
            captures: Vec::new(),
        };
        let indents = MatchIndents { found: &found };
        assert_eq!(indents.of_match(), "    ");
        assert_eq!(indents.of_later_line(7), Some("        ".to_owned()));
        assert_eq!(indents.of_later_line(4), None);
        assert_eq!(render_template(&found, "a\nb"), "a\n    b");
    }

    fn found_at(source: &str, start: usize, end: usize, line: u32) -> SyntaxMatch {
        SyntaxMatch {
            span: span(start as u64, end as u64, line, 1),
            node_kind: "node".to_owned(),
            text: source[start..end].to_owned(),
            captures: Vec::new(),
        }
    }

    #[test]
    fn matches_inside_or_overlapping_an_earlier_match_are_skipped() {
        let source = "abcdefgh";
        let found = [
            found_at(source, 4, 7, 1),
            found_at(source, 0, 5, 1),
            found_at(source, 1, 2, 1),
            found_at(source, 5, 8, 1),
        ];
        let edits = replacement_edits(source.as_bytes(), &found, "_");
        let ranges: Vec<(u64, u64)> = edits
            .iter()
            .map(|edit| (edit.bytes.start, edit.bytes.end))
            .collect();
        assert_eq!(ranges, [(0, 5), (5, 8)]);
        assert!(replacement_edits(source.as_bytes(), &[], "_").is_empty());
    }

    #[test]
    fn captured_lines_keep_their_tab_indentation_relative_to_the_capture() {
        let source = "\tcall(\n\t\tfirst,\n\t\tsecond)\n";
        let start = 1;
        let end = source.find(')').unwrap() + 1;
        let arg_start = source.find("first").unwrap();
        let arg_end = source.find("second").unwrap() + "second".len();
        let mut found = found_at(source, start, end, 1);
        found.captures.push(SyntaxCapture {
            name: "ARGS".to_owned(),
            spans: vec![span(arg_start as u64, arg_end as u64, 2, 3)],
            text: source[arg_start..arg_end].to_owned(),
        });
        let edits = replacement_edits(source.as_bytes(), &[found], "wrapped(\n\t$ARGS\n)");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].bytes, ByteRange::new(1, end as u64));
        assert_eq!(edits[0].text, "wrapped(\n\t\tfirst,\n\t\tsecond\n\t)");
    }

    #[test]
    fn dominant_line_ending_decides_the_endings_of_the_replacement() {
        assert!(dominant_line_ending_is_crlf(b"a\r\nb\r\nc\n"));
        assert!(!dominant_line_ending_is_crlf(b"a\r\nb\nc\n"));
        assert!(!dominant_line_ending_is_crlf(b"a\nb\n"));
        assert!(!dominant_line_ending_is_crlf(b""));
        assert!(!dominant_line_ending_is_crlf(b"\n\r\n"));
        assert_eq!(to_crlf("a\nb\r\nc"), "a\r\nb\r\nc");
        let source = "x\r\ny\r\nabcdefgh\r\n";
        let start = source.find("abc").unwrap();
        let found = found_at(source, start, start + 3, 3);
        let edits = replacement_edits(source.as_bytes(), &[found], "p\nq");
        assert_eq!(edits[0].text, "p\r\nq");
        let lf = "x\ny\nabcdefgh\n";
        let found = found_at(lf, start - 2, start + 1, 3);
        let edits = replacement_edits(lf.as_bytes(), &[found], "p\nq");
        assert_eq!(edits[0].text, "p\nq");
    }

    #[test]
    fn captured_text_keeps_multibyte_characters_and_carriage_returns() {
        let source = "  f(\"é\",\r\n    \"λ\")\r\n";
        let start = 2;
        let end = source.find(')').unwrap() + 1;
        let arg_start = source.find('"').unwrap();
        let arg_end = end - 1;
        let mut found = found_at(source, start, end, 1);
        found.captures.push(SyntaxCapture {
            name: "ARGS".to_owned(),
            spans: vec![span(arg_start as u64, arg_end as u64, 1, 5)],
            text: source[arg_start..arg_end].to_owned(),
        });
        let edits = replacement_edits(source.as_bytes(), &[found], "g($ARGS)");
        assert_eq!(edits[0].text, "g(\"é\",\r\n    \"λ\")");
    }
}
