// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion between positions in program text and protocol positions.
//!
//! Syntax trees and errors count lines and columns from 1 in Unicode scalar
//! values, and only a line feed starts a line; a carriage return is an
//! ordinary character of its line. Protocol positions count lines from 0 and
//! characters in UTF-16 code units or in Unicode scalar values, as negotiated
//! at initialization. A carriage return before a line feed belongs to the line
//! break in the protocol, so a protocol position never points between the two.

use std::ops::Range as Bytes;

use lsp_types::{Position, PositionEncodingKind, Range};

/// Unit in which protocol positions count the characters of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Encoding {
    /// UTF-16 code units, the protocol default.
    #[default]
    Utf16,
    /// Unicode scalar values, the unit of syntax tree columns.
    Utf32,
}

impl Encoding {
    /// Selects UTF-32 when the client offers it and UTF-16 otherwise.
    pub(crate) fn negotiate(offered: Option<&[PositionEncodingKind]>) -> Self {
        let offers_utf32 =
            offered.is_some_and(|kinds| kinds.contains(&PositionEncodingKind::UTF32));
        if offers_utf32 {
            Self::Utf32
        } else {
            Self::Utf16
        }
    }

    /// Returns the protocol name of the encoding.
    pub(crate) fn kind(self) -> PositionEncodingKind {
        match self {
            Self::Utf16 => PositionEncodingKind::UTF16,
            Self::Utf32 => PositionEncodingKind::UTF32,
        }
    }

    fn width(self, character: char) -> u32 {
        match self {
            Self::Utf16 => count(character.len_utf16()),
            Self::Utf32 => 1,
        }
    }

    /// Returns the length of `text` in the units of the encoding.
    pub(crate) fn measure(self, text: &str) -> u32 {
        match self {
            Self::Utf16 => count(text.encode_utf16().count()),
            Self::Utf32 => count(text.chars().count()),
        }
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Line structure of a text, for conversions between byte offsets, syntax
/// tree columns and protocol positions.
#[derive(Debug, Clone)]
pub(crate) struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    /// Indexes the line starts of `text`.
    pub(crate) fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));
        Self { text, starts }
    }

    /// Returns the byte offset of a syntax tree position (line and column
    /// from 1, in Unicode scalar values), clamped to the end of its line.
    pub(crate) fn column_offset(&self, line: u32, column: u32) -> usize {
        let index = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
        let Some((start, end)) = self.bounds(index) else {
            return self.text.len();
        };
        self.advance(start, column.saturating_sub(1)).min(end)
    }

    /// Returns the byte offset `count` Unicode scalar values after `offset`,
    /// clamped to the end of the text.
    pub(crate) fn advance(&self, offset: usize, count: u32) -> usize {
        let offset = self.floor(offset);
        let steps = usize::try_from(count).unwrap_or(usize::MAX);
        self.text
            .get(offset..)
            .and_then(|rest| rest.char_indices().nth(steps))
            .map_or(self.text.len(), |(index, _)| offset + index)
    }

    /// Returns the protocol position of a byte offset.
    pub(crate) fn position(&self, offset: usize, encoding: Encoding) -> Position {
        let offset = self.floor(offset);
        let line = self
            .starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let start = self.starts.get(line).copied().unwrap_or(0);
        let end = offset.min(self.content_end(line));
        let character = self
            .text
            .get(start..end)
            .map_or(0, |prefix| encoding.measure(prefix));
        Position::new(count(line), character)
    }

    /// Returns the byte offset of a protocol position, clamped to the end of
    /// its line; a position inside a UTF-16 surrogate pair moves to the start
    /// of the character.
    pub(crate) fn offset(&self, position: Position, encoding: Encoding) -> usize {
        let line = usize::try_from(position.line).unwrap_or(usize::MAX);
        let Some((start, _)) = self.bounds(line) else {
            return self.text.len();
        };
        let content = self
            .text
            .get(start..self.content_end(line))
            .unwrap_or_default();
        let mut remaining = position.character;
        let mut offset = start;
        for character in content.chars() {
            let width = encoding.width(character);
            if width > remaining {
                break;
            }
            remaining -= width;
            offset += character.len_utf8();
        }
        offset
    }

    /// Returns the protocol range of a byte range.
    pub(crate) fn range(&self, bytes: Bytes<usize>, encoding: Encoding) -> Range {
        Range::new(
            self.position(bytes.start, encoding),
            self.position(bytes.end, encoding),
        )
    }

    /// Returns the start and end byte offsets of line `index` (from 0),
    /// without its line feed.
    fn bounds(&self, index: usize) -> Option<(usize, usize)> {
        let start = *self.starts.get(index)?;
        let end = self
            .starts
            .get(index + 1)
            .map_or(self.text.len(), |next| next - 1);
        Some((start, end))
    }

    /// Returns the end of the content of line `index`, before a carriage
    /// return that precedes its line feed.
    fn content_end(&self, index: usize) -> usize {
        let Some((start, end)) = self.bounds(index) else {
            return self.text.len();
        };
        let ends_with_break = end < self.text.len();
        let line = self.text.get(start..end).unwrap_or_default();
        if ends_with_break && line.ends_with('\r') {
            end - 1
        } else {
            end
        }
    }

    /// Clamps `offset` to the text and moves it back to a character boundary.
    fn floor(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    impl Lines<'_> {
        /// Syntax tree line and column of a byte offset.
        fn column(&self, offset: usize) -> (u32, u32) {
            let line = self.starts.partition_point(|&start| start <= offset) - 1;
            let start = self.starts[line];
            (
                count(line + 1),
                count(self.text[start..offset].chars().count() + 1),
            )
        }
    }

    fn boundaries(text: &str) -> Vec<usize> {
        (0..=text.len())
            .filter(|&offset| text.is_char_boundary(offset))
            .collect()
    }

    fn text_strategy() -> impl Strategy<Value = String> {
        let line_breaks_and_wide_characters = proptest::sample::select(vec![
            'a',
            ' ',
            '\n',
            '\r',
            '\u{e9}',
            '\u{4e2d}',
            '\u{1f600}',
        ]);
        let character = prop_oneof![3 => line_breaks_and_wide_characters, 1 => any::<char>()];
        proptest::collection::vec(character, 0..48)
            .prop_map(|characters| characters.into_iter().collect())
    }

    #[test]
    fn negotiates_utf32_only_when_offered() {
        assert_eq!(Encoding::negotiate(None), Encoding::Utf16);
        assert_eq!(
            Encoding::negotiate(Some(&[PositionEncodingKind::UTF8])),
            Encoding::Utf16
        );
        let offered = [PositionEncodingKind::UTF16, PositionEncodingKind::UTF32];
        assert_eq!(Encoding::negotiate(Some(&offered)), Encoding::Utf32);
        assert_eq!(Encoding::Utf32.kind(), PositionEncodingKind::UTF32);
    }

    #[test]
    fn counts_surrogate_pairs_in_utf16_only() {
        let text = "a\u{1f600}b";
        let lines = Lines::new(text);
        let after_emoji = 1 + '\u{1f600}'.len_utf8();
        assert_eq!(
            lines.position(after_emoji, Encoding::Utf16),
            Position::new(0, 3)
        );
        assert_eq!(
            lines.position(after_emoji, Encoding::Utf32),
            Position::new(0, 2)
        );
        assert_eq!(lines.offset(Position::new(0, 2), Encoding::Utf16), 1);
        assert_eq!(
            lines.offset(Position::new(0, 3), Encoding::Utf16),
            after_emoji
        );
    }

    #[test]
    fn treats_carriage_return_before_line_feed_as_line_break() {
        let text = "ab\r\ncd";
        let lines = Lines::new(text);
        assert_eq!(lines.position(2, Encoding::Utf16), Position::new(0, 2));
        assert_eq!(lines.position(3, Encoding::Utf16), Position::new(0, 2));
        assert_eq!(lines.position(4, Encoding::Utf16), Position::new(1, 0));
        assert_eq!(lines.offset(Position::new(0, 9), Encoding::Utf16), 2);
        assert_eq!(lines.column_offset(1, 3), 2);
        assert_eq!(lines.column_offset(2, 2), 5);
    }

    #[test]
    fn clamps_positions_outside_the_text() {
        let lines = Lines::new("ab\ncd");
        assert_eq!(lines.offset(Position::new(7, 0), Encoding::Utf16), 5);
        assert_eq!(lines.column_offset(9, 1), 5);
        assert_eq!(lines.column_offset(1, 40), 2);
        assert_eq!(lines.advance(4, 10), 5);
        assert_eq!(lines.position(99, Encoding::Utf32), Position::new(1, 2));
    }

    #[test]
    fn converts_byte_ranges_to_protocol_ranges() {
        let lines = Lines::new("x \"\u{e9}\"\ny");
        let range = lines.range(2..6, Encoding::Utf16);
        assert_eq!(range, Range::new(Position::new(0, 2), Position::new(0, 5)));
    }

    proptest! {
        #[test]
        fn protocol_positions_round_trip(text in text_strategy()) {
            let lines = Lines::new(&text);
            for encoding in [Encoding::Utf16, Encoding::Utf32] {
                for offset in boundaries(&text) {
                    let back = lines.offset(lines.position(offset, encoding), encoding);
                    let before_line_feed = text[back..offset] == *"\r" && text[offset..].starts_with('\n');
                    prop_assert!(back == offset || before_line_feed, "{text:?} at {offset}: {back}");
                }
            }
        }

        #[test]
        fn syntax_tree_columns_round_trip(text in text_strategy()) {
            let lines = Lines::new(&text);
            for offset in boundaries(&text) {
                let (line, column) = lines.column(offset);
                prop_assert_eq!(lines.column_offset(line, column), offset);
            }
        }

        #[test]
        fn utf32_characters_equal_columns_outside_line_breaks(text in text_strategy()) {
            let lines = Lines::new(&text);
            for offset in boundaries(&text) {
                let (line, column) = lines.column(offset);
                let position = lines.position(offset, Encoding::Utf32);
                prop_assert_eq!(position.line + 1, line);
                if !text[offset..].starts_with('\n') || !text[..offset].ends_with('\r') {
                    prop_assert_eq!(position.character + 1, column);
                }
            }
        }
    }
}
