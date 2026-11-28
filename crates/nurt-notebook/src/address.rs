// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-notebook::address
// Responsibility:   Addressing of cells by index and identifier.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Addressing of cells by index and identifier.

use nurt_contracts::value::Range;

/// One cell, by index, by identifier, or by both.
///
/// With both, the index and the identifier must name the same cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CellRef {
    /// Position of the cell, from 0.
    Index(i64),
    /// Identifier of the cell (nbformat 4.5 and later).
    Id(String),
    /// Position and identifier of the same cell.
    IndexAndId(i64, String),
}

impl CellRef {
    /// Reference from optional index and identifier; `None` when both are
    /// missing.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_notebook::CellRef;
    ///
    /// assert_eq!(CellRef::from_parts(Some(2), None), Some(CellRef::Index(2)));
    /// assert_eq!(CellRef::from_parts(None, None), None);
    /// ```
    #[must_use]
    pub fn from_parts(index: Option<i64>, id: Option<String>) -> Option<Self> {
        match (index, id) {
            (Some(index), Some(id)) => Some(Self::IndexAndId(index, id)),
            (Some(index), None) => Some(Self::Index(index)),
            (None, Some(id)) => Some(Self::Id(id)),
            (None, None) => None,
        }
    }
}

/// Set of cells.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Selection {
    /// Every cell.
    All,
    /// Cells whose index lies in the inclusive range.
    Range(Range),
    /// Cells with the listed identifiers.
    Ids(Vec<String>),
}

/// Position of a new cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Position {
    /// After the last cell.
    #[default]
    End,
    /// At this index, from 0; the cell at that index and later cells move
    /// down.
    Index(i64),
    /// Directly after the cell with this identifier.
    AfterId(String),
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
