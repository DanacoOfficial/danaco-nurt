// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::services::{TextPosition, TextSpan};

use super::Span;

/// Offsets of line starts in a text, for converting byte offsets to lines and
/// columns.
///
/// Lines end at `\n`; a `\r` before it belongs to the line. Columns count
/// Unicode scalar values from 1; bytes that are not valid UTF-8 count as one
/// column each.
///
/// ```
/// use nurt_doc::model::LineIndex;
///
/// let text = "a: 1\nżó: 2\n".as_bytes();
/// let position = LineIndex::new(text).position(text, 10);
/// assert_eq!((position.line, position.column), (2, 4));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    /// Index of the lines of `text`.
    #[must_use]
    pub fn new(text: &[u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(offset, _)| offset + 1),
        );
        Self {
            starts,
            len: text.len(),
        }
    }

    /// Number of lines; a text ending with a line break has an empty last
    /// line.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Zero-based line containing `offset`; offsets past the end belong to
    /// the last line.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset.min(self.len)) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1),
        }
    }

    /// Offset of the first byte of the zero-based `line`, or the text length
    /// for a line past the end.
    #[must_use]
    pub fn line_start(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or(self.len)
    }

    /// One-based line and column (in characters) of `offset` in `text`,
    /// which must be the text the index was built from.
    #[must_use]
    pub fn position(&self, text: &[u8], offset: usize) -> TextPosition {
        let offset = offset.min(text.len());
        let line = self.line_of(offset);
        let start = self.line_start(line).min(offset);
        let prefix = text.get(start..offset).unwrap_or_default();
        TextPosition {
            line: to_u32(line + 1),
            column: to_u32(count_chars(prefix) + 1),
        }
    }

    /// Region of `span` in `text` as bytes and character positions.
    #[must_use]
    pub fn text_span(&self, text: &[u8], span: Span) -> TextSpan {
        TextSpan {
            bytes: span.to_byte_range(),
            start: self.position(text, span.start),
            end: self.position(text, span.end),
        }
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Number of characters in `bytes`; bytes outside valid UTF-8 sequences count
/// as one character each.
fn count_chars(bytes: &[u8]) -> usize {
    bytes
        .utf8_chunks()
        .map(|chunk| chunk.valid().chars().count() + chunk.invalid().len())
        .sum()
}
