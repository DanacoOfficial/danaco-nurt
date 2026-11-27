// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::model::span
// Responsibility:   Byte range `start..end` in a document text (end exclusive).
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::services::ByteRange;

/// Byte range `start..end` in a document text (end exclusive).
///
/// An empty span (`start == end`) marks a position, for example the place
/// where new content is inserted.
///
/// ```
/// use nurt_doc::model::Span;
///
/// let span = Span::new(2, 5);
/// assert_eq!(span.len(), 3);
/// assert!(span.contains(4));
/// assert!(span.overlaps(Span::new(4, 9)));
/// assert!(!span.overlaps(Span::new(5, 9)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    /// Offset of the first byte.
    pub start: usize,
    /// Offset after the last byte.
    pub end: usize,
}

impl Span {
    /// Span `start..end`; the bounds are swapped when `end < start`.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        if end < start {
            Self {
                start: end,
                end: start,
            }
        } else {
            Self { start, end }
        }
    }

    /// Empty span at `offset`.
    #[must_use]
    pub const fn at(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Number of bytes.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    /// Returns `true` for an empty span.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns `true` when `offset` lies in the span.
    #[must_use]
    pub const fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Returns `true` when `other` lies entirely in the span; an empty
    /// `other` may lie at either end.
    #[must_use]
    pub const fn covers(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Returns `true` when the spans share at least one byte.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Smallest span covering both spans.
    #[must_use]
    pub fn join(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// The span as a contract byte range.
    #[must_use]
    pub const fn to_byte_range(self) -> ByteRange {
        ByteRange::new(self.start as u64, self.end as u64)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
