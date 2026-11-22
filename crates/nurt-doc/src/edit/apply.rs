// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::{Error, ErrorCode, Result};

use crate::model::Span;

/// Replacement of the bytes of `span` in the original text.
///
/// An empty span inserts at its position; an empty replacement deletes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ByteEdit {
    /// Bytes of the original text that are replaced.
    pub span: Span,
    /// New bytes.
    pub replacement: Vec<u8>,
}

impl ByteEdit {
    /// Replaces `span` with `replacement`.
    #[must_use]
    pub fn replace(span: Span, replacement: impl Into<Vec<u8>>) -> Self {
        Self {
            span,
            replacement: replacement.into(),
        }
    }

    /// Inserts `bytes` at `offset`.
    #[must_use]
    pub fn insert(offset: usize, bytes: impl Into<Vec<u8>>) -> Self {
        Self::replace(Span::at(offset), bytes)
    }

    /// Deletes `span`.
    #[must_use]
    pub fn delete(span: Span) -> Self {
        Self::replace(span, Vec::new())
    }

    /// Returns `true` when applying the edit to `original` changes nothing.
    #[must_use]
    pub fn is_noop(&self, original: &[u8]) -> bool {
        original.get(self.span.start..self.span.end) == Some(self.replacement.as_slice())
    }
}

/// Correspondence between offsets of the original text and of the text
/// after a list of edits.
///
/// ```
/// use nurt_doc::edit::{ByteEdit, apply};
/// use nurt_doc::model::Span;
///
/// let applied = apply(b"abcdef", &[ByteEdit::delete(Span::new(1, 3))])?;
/// assert_eq!(applied.offsets.map(4), 2);
/// assert_eq!(applied.offsets.changed(), &[Span::at(1)]);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OffsetMap {
    /// Original spans of the edits, sorted.
    old: Vec<Span>,
    /// Spans of the replacements in the new text, in the same order.
    new: Vec<Span>,
}

impl OffsetMap {
    /// New offset of the original `offset`. An offset inside a replaced
    /// span maps to the start of its replacement; an offset at the end of a
    /// replaced span maps to the end of the replacement.
    #[must_use]
    pub fn map(&self, offset: usize) -> usize {
        let before = self.old.partition_point(|span| span.end <= offset);
        if let (Some(old), Some(new)) = (self.old.get(before), self.new.get(before))
            && old.start < offset
        {
            return new.start;
        }
        match before.checked_sub(1) {
            Some(last) => offset - self.old[last].end + self.new[last].end,
            None => offset,
        }
    }

    /// Spans of the replacements in the new text, sorted.
    #[must_use]
    pub fn changed(&self) -> &[Span] {
        &self.new
    }

    /// Original spans of the edits, sorted.
    #[must_use]
    pub fn replaced(&self) -> &[Span] {
        &self.old
    }
}

/// Result of [`apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Text after the edits.
    pub bytes: Vec<u8>,
    /// Offsets of the original text in the new text.
    pub offsets: OffsetMap,
}

/// Sorts `edits` by position and checks that no two edits overlap.
///
/// Insertions at the same offset keep their order; an insertion at the
/// boundary of a replaced span is allowed.
///
/// # Errors
///
/// Fails with `E_KEY_CONFLICT` when two edits replace a common byte or an
/// insertion lies strictly inside a replaced span, and with
/// `E_INVALID_VALUE` when an edit reaches past the end of the text of length
/// `len`.
pub fn check_overlaps(edits: &[ByteEdit], len: usize) -> Result<Vec<&ByteEdit>> {
    let mut sorted: Vec<&ByteEdit> = edits.iter().collect();
    sorted.sort_by_key(|edit| (edit.span.start, edit.span.end));
    if let Some(edit) = sorted.iter().find(|edit| edit.span.end > len) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "edit {}..{} reaches past the end of the {len}-byte text",
                edit.span.start, edit.span.end
            ),
        ));
    }
    for pair in sorted.windows(2) {
        let (first, second) = (pair[0].span, pair[1].span);
        if second.start < first.end {
            return Err(Error::new(
                ErrorCode::KeyConflict,
                format!(
                    "changes at bytes {}..{} and {}..{} overlap; change each node once",
                    first.start, first.end, second.start, second.end
                ),
            )
            .with_found("overlap"));
        }
    }
    Ok(sorted)
}

/// Applies non-overlapping `edits` to `original`.
///
/// # Errors
///
/// As for [`check_overlaps`].
pub fn apply(original: &[u8], edits: &[ByteEdit]) -> Result<Applied> {
    let sorted = check_overlaps(edits, original.len())?;
    let added: usize = sorted.iter().map(|edit| edit.replacement.len()).sum();
    let mut bytes = Vec::with_capacity(original.len() + added);
    let mut offsets = OffsetMap::default();
    let mut cursor = 0;
    for edit in sorted {
        bytes.extend_from_slice(original.get(cursor..edit.span.start).unwrap_or_default());
        let start = bytes.len();
        bytes.extend_from_slice(&edit.replacement);
        offsets.old.push(edit.span);
        offsets.new.push(Span::new(start, bytes.len()));
        cursor = edit.span.end;
    }
    bytes.extend_from_slice(original.get(cursor..).unwrap_or_default());
    Ok(Applied { bytes, offsets })
}
