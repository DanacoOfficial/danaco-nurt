// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Positions of corrections in the text as written.

use std::ops::Range;

use nurt_contracts::report::TextPosition;

use crate::normalize::{LineIndex, PositionMap};

/// Finds the position in the text as written of a range of a corrected text.
pub(crate) struct Locator<'a> {
    index: LineIndex<'a>,
    map: PositionMap,
}

impl<'a> Locator<'a> {
    /// Locator for a text that is the text as written.
    pub(crate) fn identity(text: &'a str) -> Self {
        Self::new(text, PositionMap::identity(text.len()))
    }

    /// Locator for a corrected text whose positions `map` leads back to
    /// `original`.
    pub(crate) fn new(original: &'a str, map: PositionMap) -> Self {
        Self {
            index: LineIndex::new(original),
            map,
        }
    }

    /// Position in the text as written of a byte range of the corrected text.
    pub(crate) fn at(&self, range: Range<usize>) -> TextPosition {
        self.index.position(self.map.range_to_original(range))
    }
}

/// Byte offset of a line and a column, both from 1, in `text`; columns count
/// Unicode scalar values. Positions past the end of a line or of the text
/// give the end of the line or of the text.
pub(crate) fn offset_of(text: &str, line: u32, column: u32) -> usize {
    let mut start = 0;
    for _ in 1..line {
        match text.get(start..).and_then(|rest| rest.find('\n')) {
            Some(end) => start += end + 1,
            None => return text.len(),
        }
    }
    let rest = text.get(start..).unwrap_or_default();
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let skip = usize::try_from(column.saturating_sub(1)).unwrap_or(usize::MAX);
    let within = rest
        .char_indices()
        .nth(skip)
        .map_or(line_end, |(offset, _)| offset.min(line_end));
    start + within
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_offsets_of_lines_and_columns() {
        let text = "ab\ncé d\n";
        assert_eq!(offset_of(text, 1, 1), 0);
        assert_eq!(offset_of(text, 2, 1), 3);
        assert_eq!(offset_of(text, 2, 3), 6);
        assert_eq!(offset_of(text, 2, 40), 8);
        assert_eq!(offset_of(text, 9, 1), text.len());
    }
}
