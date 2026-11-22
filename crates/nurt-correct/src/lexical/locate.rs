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
        Self {
            index: LineIndex::new(text),
            map: PositionMap::identity(text.len()),
        }
    }

    /// Position in the text as written of a byte range of the corrected text.
    pub(crate) fn at(&self, range: Range<usize>) -> TextPosition {
        self.index.position(self.map.range_to_original(range))
    }
}
