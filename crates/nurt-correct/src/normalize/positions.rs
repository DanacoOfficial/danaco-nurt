// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Positions in corrected text and their origin in the text as written.

use std::ops::Range;

use nurt_contracts::report::TextPosition;

/// Byte order mark that the lexer skips at the start of a program.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// Map from byte offsets of corrected text to offsets of the text as written.
///
/// The map is a sequence of segments. A copied segment maps byte for byte; a
/// replaced segment maps every offset inside it to the start of the replaced
/// source range, and its end to the end of that range. Maps compose, so a text
/// corrected in several passes keeps one map to the original text.
///
/// # Examples
///
/// ```
/// use nurt_correct::normalize::PositionMap;
///
/// let map = PositionMap::identity(5);
/// assert_eq!(map.to_original(3), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionMap {
    segments: Vec<Segment>,
    corrected_len: usize,
    original_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    corrected: Range<usize>,
    original: Range<usize>,
}

impl Segment {
    fn is_copy(&self) -> bool {
        self.corrected.len() == self.original.len()
    }
}

impl PositionMap {
    /// Map of a text that was not changed.
    #[must_use]
    pub fn identity(len: usize) -> Self {
        Self {
            segments: vec![Segment {
                corrected: 0..len,
                original: 0..len,
            }],
            corrected_len: len,
            original_len: len,
        }
    }

    /// Length in bytes of the corrected text.
    #[must_use]
    pub fn corrected_len(&self) -> usize {
        self.corrected_len
    }

    /// Length in bytes of the text as written.
    #[must_use]
    pub fn original_len(&self) -> usize {
        self.original_len
    }

    /// Returns the byte offset in the text as written that corresponds to a
    /// byte offset of the corrected text.
    ///
    /// Offsets past the end map to the end of the text as written.
    #[must_use]
    pub fn to_original(&self, offset: usize) -> usize {
        let index = self
            .segments
            .partition_point(|segment| segment.corrected.end <= offset);
        match self.segments.get(index) {
            Some(segment) if segment.is_copy() => {
                segment.original.start + offset.saturating_sub(segment.corrected.start)
            }
            Some(segment) => segment.original.start,
            None => self.original_len,
        }
    }

    /// Returns the range of the text as written that corresponds to a range of
    /// the corrected text.
    #[must_use]
    pub fn range_to_original(&self, range: Range<usize>) -> Range<usize> {
        let start = self.to_original(range.start);
        let end = if range.is_empty() {
            start
        } else {
            self.end_to_original(range.end).max(start)
        };
        start..end
    }

    fn end_to_original(&self, end: usize) -> usize {
        let index = self
            .segments
            .partition_point(|segment| segment.corrected.end < end);
        match self.segments.get(index) {
            Some(segment) if segment.is_copy() => {
                segment.original.start + end.saturating_sub(segment.corrected.start)
            }
            Some(segment) if end > segment.corrected.start => segment.original.end,
            Some(segment) => segment.original.start,
            None => self.original_len,
        }
    }

    /// Composes this map, from an intermediate text to the text as written,
    /// with `next`, from a later text to the intermediate text.
    #[must_use]
    pub fn then(&self, next: &PositionMap) -> PositionMap {
        let mut segments = Vec::with_capacity(next.segments.len() + self.segments.len());
        for segment in &next.segments {
            if segment.is_copy() {
                self.split_copy(segment, &mut segments);
            } else {
                segments.push(Segment {
                    corrected: segment.corrected.clone(),
                    original: self.range_to_original(segment.original.clone()),
                });
            }
        }
        PositionMap {
            segments,
            corrected_len: next.corrected_len,
            original_len: self.original_len,
        }
    }

    /// Splits a copied segment of a later map at the segment boundaries of
    /// this map, so that offsets inside it stay exact.
    fn split_copy(&self, copy: &Segment, out: &mut Vec<Segment>) {
        let range = &copy.original;
        let first = self
            .segments
            .partition_point(|segment| segment.corrected.end < range.start);
        for own in self.segments.iter().skip(first) {
            let past_end = own.corrected.start > range.end
                || (own.corrected.start == range.end && !range.is_empty());
            if past_end {
                break;
            }
            let from = own.corrected.start.max(range.start);
            let to = own.corrected.end.min(range.end);
            if !own.corrected.is_empty() && to <= from {
                continue;
            }
            let corrected_start = copy.corrected.start + (from - range.start);
            let original = if own.is_copy() {
                let offset = own.original.start + (from - own.corrected.start);
                offset..offset + (to - from)
            } else {
                own.original.clone()
            };
            out.push(Segment {
                corrected: corrected_start..corrected_start + (to - from),
                original,
            });
        }
    }
}

/// Replacement of a byte range of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    pub(crate) range: Range<usize>,
    pub(crate) replacement: String,
}

impl Edit {
    pub(crate) fn new(range: Range<usize>, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

/// Applies edits sorted by position and not overlapping.
///
/// Returns the edited text and the map from the edited text to `text`.
pub(crate) fn apply_edits(text: &str, edits: &[Edit]) -> (String, PositionMap) {
    let mut out = String::with_capacity(text.len());
    let mut segments = Vec::with_capacity(edits.len() * 2 + 1);
    let mut cursor = 0;
    for edit in edits {
        let start = edit.range.start.max(cursor);
        let end = edit.range.end.max(start);
        if start > cursor {
            push_segment(&mut segments, out.len(), cursor..start);
            out.push_str(&text[cursor..start]);
        }
        let corrected_start = out.len();
        out.push_str(&edit.replacement);
        segments.push(Segment {
            corrected: corrected_start..out.len(),
            original: start..end,
        });
        cursor = end;
    }
    if cursor < text.len() {
        push_segment(&mut segments, out.len(), cursor..text.len());
        out.push_str(&text[cursor..]);
    }
    let map = PositionMap {
        segments,
        corrected_len: out.len(),
        original_len: text.len(),
    };
    (out, map)
}

fn push_segment(segments: &mut Vec<Segment>, corrected_start: usize, original: Range<usize>) {
    segments.push(Segment {
        corrected: corrected_start..corrected_start + original.len(),
        original,
    });
}

/// Line and column lookup in a program text.
///
/// Lines and columns start at 1 and columns count Unicode scalar values. A
/// byte order mark at the start of the text is not counted.
#[derive(Debug, Clone)]
pub(crate) struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        let first = if text.starts_with(BYTE_ORDER_MARK) {
            BYTE_ORDER_MARK.len_utf8()
        } else {
            0
        };
        let mut line_starts = vec![first];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, byte)| byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        Self { text, line_starts }
    }

    /// Position of a byte range of the text.
    pub(crate) fn position(&self, range: Range<usize>) -> TextPosition {
        let start = floor_char_boundary(self.text, range.start.min(self.text.len()));
        let end = floor_char_boundary(self.text, range.end.clamp(start, self.text.len()));
        let line_index = self
            .line_starts
            .partition_point(|&line_start| line_start <= start);
        let line_start = self
            .line_starts
            .get(line_index.saturating_sub(1))
            .copied()
            .unwrap_or(0)
            .min(start);
        let column = self.text[line_start..start].chars().count() + 1;
        let length = self.text[start..end].chars().count();
        TextPosition {
            line: saturate(line_index.max(1)),
            column: saturate(column),
            length: Some(saturate(length)),
        }
    }
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn saturate(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_every_offset_to_itself() {
        let map = PositionMap::identity(4);
        for offset in 0..=4 {
            assert_eq!(map.to_original(offset), offset);
        }
    }

    #[test]
    fn replacement_maps_inner_offsets_to_its_source_start() {
        let (text, map) = apply_edits("ab\u{a0}cd", &[Edit::new(2..4, " ")]);
        assert_eq!(text, "ab cd");
        assert_eq!(map.to_original(2), 2);
        assert_eq!(map.to_original(3), 4);
        assert_eq!(map.to_original(4), 5);
        assert_eq!(map.range_to_original(2..3), 2..4);
    }

    #[test]
    fn deletion_keeps_following_offsets_in_place() {
        let (text, map) = apply_edits("fi\u{200b}nd", &[Edit::new(2..5, "")]);
        assert_eq!(text, "find");
        assert_eq!(map.to_original(2), 5);
        assert_eq!(map.range_to_original(0..4), 0..7);
    }

    #[test]
    fn composed_maps_reach_the_text_as_written() {
        let (first, first_map) = apply_edits("a\u{a0}'x'", &[Edit::new(1..3, " ")]);
        assert_eq!(first, "a 'x'");
        let (second, second_map) = apply_edits(&first, &[Edit::new(2..5, "\"x\"")]);
        assert_eq!(second, "a \"x\"");
        let map = first_map.then(&second_map);
        assert_eq!(map.range_to_original(2..5), 3..6);
        assert_eq!(map.original_len(), 6);
        assert_eq!(map.corrected_len(), 5);
    }

    #[test]
    fn insertion_maps_to_an_empty_source_range() {
        let (text, map) = apply_edits("(a", &[Edit::new(2..2, ")")]);
        assert_eq!(text, "(a)");
        assert_eq!(map.range_to_original(2..3), 2..2);
    }

    #[test]
    fn positions_count_characters_and_skip_byte_order_mark() {
        let text = "\u{feff}αβγδεζ x\nb";
        let index = LineIndex::new(text);
        let x = text.find('x').unwrap_or_default();
        assert_eq!(
            index.position(x..x + 1),
            TextPosition {
                line: 1,
                column: 8,
                length: Some(1)
            }
        );
        let b = text.len() - 1;
        let position = index.position(b..b + 1);
        assert_eq!((position.line, position.column), (2, 1));
    }
}
