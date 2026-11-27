// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-notebook
// Responsibility:   Jupyter notebooks for Danaco Nurt.
// Dependencies:     address, cell, changes, content, error, export, find, notebook, ops
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Jupyter notebooks for Danaco Nurt.
//!
//! The crate reads and edits cells of `.ipynb` notebooks (nbformat 4) and
//! exports notebooks to scripts, Markdown and plain text. Edits keep the
//! text of unchanged cells and members byte for byte, so a difference of the
//! file shows only the changed cells.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod address;
mod cell;
mod changes;
mod content;
mod error;
mod export;
mod find;
mod json;
mod notebook;
mod ops;
mod source;

pub use address::{CellRef, Position, Selection};
pub use cell::{CellData, OutputsDetail, View};
pub use changes::{Change, ChangeOutcome, PatternFactory, literal_pattern};
pub use content::{NotebookContent, cells_as_content};
pub use error::NotebookError;
pub use export::ExportFormat;
pub use find::{Literal, Pattern};
pub use notebook::{IdGenerator, Notebook};
pub use ops::{CellQuery, Inserted, Normalized, Replace, Replaced, Scope};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
