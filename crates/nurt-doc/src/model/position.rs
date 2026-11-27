// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::model::position
// Responsibility:   Offsets of line starts in a text, for converting byte offsets to lines and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
/// let text = "a: 1\néü: 2\n".as_bytes();
/// let position = LineIndex::new(text).position(text, 10);
/// assert_eq!((position.line, position.column), (2, 4));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
    encoding: Encoding,
}

/// Bytes per entry of the counts of continuation bytes.
const BLOCK: usize = 64;

/// What the index knows about the characters of the text, so that a column
/// does not take a scan of the line: a line of a minified document is as long
/// as the document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Encoding {
    /// Not valid UTF-8, or not indexed: a column counts the characters of the
    /// line up to the offset.
    #[default]
    Unscanned,
    /// Every character is one byte.
    Ascii,
    /// Valid UTF-8 with multi-byte characters: the number of continuation
    /// bytes before the first byte of each block of [`BLOCK`] bytes, and one
    /// more entry for the end of the text.
    Counted(Vec<usize>),
}

const fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

impl Encoding {
    fn of(text: &[u8]) -> Self {
        match std::str::from_utf8(text) {
            Ok(valid) if valid.is_ascii() => Self::Ascii,
            Ok(_) => {
                let mut counts = Vec::with_capacity(text.len() / BLOCK + 2);
                let mut total = 0;
                for block in text.chunks(BLOCK) {
                    counts.push(total);
                    total += block.iter().filter(|byte| is_continuation(**byte)).count();
                }
                counts.push(total);
                Self::Counted(counts)
            }
            Err(_) => Self::Unscanned,
        }
    }

    /// Number of characters from `start`, the first byte of a line, to
    /// `offset`, or `None` when only a scan of the bytes tells.
    fn chars(&self, text: &[u8], start: usize, offset: usize) -> Option<usize> {
        match self {
            Self::Ascii => Some(offset - start),
            Self::Counted(counts)
                if text.get(offset).is_none_or(|byte| !is_continuation(*byte)) =>
            {
                let before = |at: usize| {
                    let block = at / BLOCK;
                    counts[block]
                        + text[block * BLOCK..at]
                            .iter()
                            .filter(|byte| is_continuation(**byte))
                            .count()
                };
                Some(offset - start - (before(offset) - before(start)))
            }
            _ => None,
        }
    }
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
            encoding: Encoding::of(text),
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
        let counted = if text.len() == self.len {
            self.encoding.chars(text, start, offset)
        } else {
            None
        };
        let chars =
            counted.unwrap_or_else(|| count_chars(text.get(start..offset).unwrap_or_default()));
        TextPosition {
            line: to_u32(line + 1),
            column: to_u32(chars + 1),
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
