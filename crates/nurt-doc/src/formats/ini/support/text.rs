// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lines, positions and splices of text documents.

use std::ops::Range;

use nurt_contracts::services::{ByteRange, TextPosition, TextSpan};

/// One physical line: `start..content_end` is the content, `content_end..end`
/// the line terminator (`\n` or `\r\n`, empty on the last line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) start: usize,
    pub(crate) content_end: usize,
    pub(crate) end: usize,
}

impl Line {
    /// Content of the line without the terminator.
    pub(crate) fn content<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.content_end]
    }

    /// Returns `true` when the line holds only spaces and tabs.
    pub(crate) fn is_blank(&self, text: &str) -> bool {
        self.content(text).trim_matches([' ', '\t']).is_empty()
    }
}

/// Splits a text into physical lines; a final line without content and
/// terminator is not reported.
pub(crate) fn lines(text: &str) -> Vec<Line> {
    let mut result = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    while start < bytes.len() {
        let (content_end, end) = match bytes[start..].iter().position(|&b| b == b'\n') {
            Some(offset) => {
                let newline = start + offset;
                let content_end = if newline > start && bytes[newline - 1] == b'\r' {
                    newline - 1
                } else {
                    newline
                };
                (content_end, newline + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        result.push(Line {
            start,
            content_end,
            end,
        });
        start = end;
    }
    result
}

/// Line terminator used by most lines of the text: `\r\n` when lines ending
/// in `\r\n` outnumber lines ending in a bare `\n`, otherwise `\n`.
pub(crate) fn dominant_newline(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf { "\r\n" } else { "\n" }
}

/// Text to insert before content appended at the end of `text` so that the
/// appended content starts on a new line.
pub(crate) fn line_break_before_append<'a>(text: &str, newline: &'a str) -> &'a str {
    if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        newline
    }
}

/// Converts byte offsets of a text into line and column positions.
#[derive(Debug, Clone)]
pub(crate) struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    /// Indexes the line starts of `text`.
    pub(crate) fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self { starts }
    }

    /// One-based line and column (in Unicode scalar values) of `offset`.
    pub(crate) fn position(&self, text: &str, offset: usize) -> TextPosition {
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        let start = self.starts[line - 1];
        let column = text
            .get(start..offset)
            .map_or(offset - start, |prefix| prefix.chars().count());
        TextPosition {
            line: to_u32(line),
            column: to_u32(column + 1),
        }
    }

    /// Region of `range` as bytes and positions.
    pub(crate) fn span(&self, text: &str, range: Range<usize>) -> TextSpan {
        TextSpan {
            bytes: ByteRange::new(to_u64(range.start), to_u64(range.end)),
            start: self.position(text, range.start),
            end: self.position(text, range.end),
        }
    }
}

/// Saturating conversion for positions; documents larger than `u32::MAX`
/// lines are rejected by the size limit of the documents group.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Lossless conversion of a byte offset.
pub(crate) fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Replacement of the bytes `range` of a text by `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Splice {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

impl Splice {
    /// Replacement of `range` by `text`.
    pub(crate) fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }

    /// Insertion of `text` at `offset`.
    pub(crate) fn insert(offset: usize, text: impl Into<String>) -> Self {
        Self::new(offset..offset, text)
    }
}

/// Applies splices to a text.
///
/// Returns `None` when two splices overlap, or when two insertions share an
/// offset, since their order would be ambiguous.
pub(crate) fn apply_splices(text: &str, splices: &[Splice]) -> Option<String> {
    let mut order: Vec<&Splice> = splices.iter().collect();
    order.sort_by_key(|s| (s.range.start, s.range.end));
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut last_insert = None;
    for splice in order {
        if splice.range.start < cursor || splice.range.end > text.len() {
            return None;
        }
        if splice.range.is_empty() {
            if last_insert == Some(splice.range.start) {
                return None;
            }
            last_insert = Some(splice.range.start);
        }
        result.push_str(text.get(cursor..splice.range.start)?);
        result.push_str(&splice.text);
        cursor = splice.range.end;
    }
    result.push_str(text.get(cursor..)?);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_with_mixed_terminators() {
        let text = "a\r\nb\nc";
        let found: Vec<&str> = lines(text).iter().map(|l| l.content(text)).collect();
        assert_eq!(found, ["a", "b", "c"]);
        assert_eq!(lines(text)[0].end, 3);
        assert!(lines("").is_empty());
    }

    #[test]
    fn picks_the_dominant_line_terminator() {
        assert_eq!(dominant_newline("a\r\nb\r\nc\n"), "\r\n");
        assert_eq!(dominant_newline("a\nb\r\n"), "\n");
        assert_eq!(dominant_newline("a"), "\n");
    }

    #[test]
    fn reports_positions_in_characters() {
        let text = "λé = 1\nb";
        let index = LineIndex::new(text);
        let position = index.position(text, text.find('=').unwrap());
        assert_eq!((position.line, position.column), (1, 4));
        assert_eq!(index.position(text, text.len()).line, 2);
    }

    #[test]
    fn applies_splices_in_offset_order_and_rejects_overlaps() {
        let text = "abcdef";
        let edits = [Splice::new(4..5, "E"), Splice::insert(0, ">")];
        assert_eq!(apply_splices(text, &edits).as_deref(), Some(">abcdEf"));
        let overlapping = [Splice::new(1..3, "x"), Splice::new(2..4, "y")];
        assert_eq!(apply_splices(text, &overlapping), None);
        let same_point = [Splice::insert(2, "x"), Splice::insert(2, "y")];
        assert_eq!(apply_splices(text, &same_point), None);
    }

    #[test]
    fn appends_on_a_new_line_only_when_needed() {
        assert_eq!(line_break_before_append("a", "\n"), "\n");
        assert_eq!(line_break_before_append("a\n", "\n"), "");
        assert_eq!(line_break_before_append("", "\r\n"), "");
    }
}
