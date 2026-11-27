// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::journald::index
// Responsibility:   Lists of entries: the global index and the entries of a DATA object.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Lists of entries: the global index and the entries of a DATA object.
//!
//! A list is a chain of ENTRY_ARRAY objects, optionally preceded by one
//! entry stored in the DATA object itself. Positions count from the oldest
//! entry. The chain must have strictly increasing offsets, which also stops
//! loops; its length is bounded by the number of entries the header or the
//! DATA object declares, so items written after the file was opened are not
//! visited.

use super::Fault;
use super::file::JournalFile;
use super::object::DataObject;

#[derive(Debug, Clone, Copy)]
struct Span {
    offset: u64,
    start: u64,
}

/// Entries of one list, addressed by position.
#[derive(Debug, Clone)]
pub(super) struct EntryIndex {
    inline: Option<u64>,
    spans: Vec<Span>,
    len: u64,
}

impl EntryIndex {
    /// Index of all entries of the file.
    pub(super) fn global(file: &mut JournalFile) -> Result<Self, Fault> {
        let first = file.header().entry_array_offset;
        let bound = file.header().n_entries;
        Self::build(file, None, first, bound)
    }

    /// Entries that reference a DATA object.
    pub(super) fn of_data(file: &mut JournalFile, data: &DataObject) -> Result<Self, Fault> {
        let inline = (data.entry_offset != 0).then_some(data.entry_offset);
        let bound = data.n_entries.saturating_sub(u64::from(inline.is_some()));
        Self::build(file, inline, data.entry_array_offset, bound)
    }

    fn build(
        file: &mut JournalFile,
        inline: Option<u64>,
        first: u64,
        bound: u64,
    ) -> Result<Self, Fault> {
        let mut len = u64::from(inline.is_some());
        let mut spans = Vec::new();
        let mut offset = first;
        let mut previous = 0;
        let mut remaining = bound;
        let mut steps = 0;
        while offset != 0 && remaining > 0 {
            steps += 1;
            if offset <= previous || steps > file.max_steps() {
                return Err(Fault::Corrupt);
            }
            let (next, capacity) = match file.entry_array(offset) {
                Ok(array) => array,
                Err(Fault::Beyond) => break,
                Err(fault) => return Err(fault),
            };
            let count = capacity.min(remaining);
            spans.push(Span { offset, start: len });
            len += count;
            remaining -= count;
            previous = offset;
            offset = next;
        }
        Ok(Self { inline, spans, len })
    }

    /// Number of positions.
    pub(super) const fn len(&self) -> u64 {
        self.len
    }

    /// Offset of the entry at `position`; 0 marks an unwritten item.
    pub(super) fn get(&self, file: &mut JournalFile, position: u64) -> Result<u64, Fault> {
        if position >= self.len {
            return Err(Fault::Corrupt);
        }
        if let Some(inline) = self.inline
            && position == 0
        {
            return Ok(inline);
        }
        let index = self.spans.partition_point(|span| span.start <= position);
        let span = index
            .checked_sub(1)
            .and_then(|index| self.spans.get(index))
            .ok_or(Fault::Corrupt)?;
        file.entry_array_item(span.offset, position - span.start)
    }

    /// Number of leading positions whose entries were received before
    /// `realtime`, found by bisection; unwritten items and items beyond the
    /// read boundary count as newer.
    pub(super) fn count_before(&self, file: &mut JournalFile, realtime: u64) -> Result<u64, Fault> {
        let (mut low, mut high) = (0, self.len);
        while low < high {
            let middle = low + (high - low) / 2;
            let offset = self.get(file, middle)?;
            let older = offset != 0
                && match file.entry_realtime(offset) {
                    Ok(time) => time < realtime,
                    Err(Fault::Beyond) => false,
                    Err(fault) => return Err(fault),
                };
            if older {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Ok(low)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
