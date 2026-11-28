// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-notebook::ops
// Responsibility:   Operations of the command group `notebook`.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Operations of the command group `notebook`.
//!
//! Each operation changes only the cells it names. Members it does not
//! change keep their original text, so a difference of the file shows only
//! the changed cells.

use std::collections::HashSet;

use nurt_contracts::record::CellType;
use nurt_contracts::value::Range;
use serde_json::{Value, json};

use crate::address::{CellRef, Position, Selection};
use crate::cell::{CellData, OutputsDetail, View};
use crate::error::NotebookError;
use crate::find::{Pattern, replace_all};
use crate::json::JsonObject;
use crate::notebook::{MINOR_WITH_IDS, Notebook, is_valid_id};
use crate::source::{self, TextForm};

/// Part of a cell searched by [`CellQuery::find`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scope {
    /// The source of the cell.
    #[default]
    Source,
    /// The text of the outputs of the cell.
    Outputs,
}

/// Filters and view of a cell listing.
///
/// Without `index`, `ids` and `find`, every cell is listed as an outline;
/// with any of them, the selected cells are listed in full. `view`
/// overrides that choice.
#[derive(Debug, Clone, Default)]
pub struct CellQuery<'a> {
    /// Cells by index; an end past the last cell is allowed.
    pub index: Option<Range>,
    /// Cells by identifier, in addition to `index`.
    pub ids: Vec<String>,
    /// Only cells of this type.
    pub cell_type: Option<CellType>,
    /// Detail of the outputs of code cells.
    pub outputs: OutputsDetail,
    /// Only cells in which this pattern occurs.
    pub find: Option<&'a dyn Pattern>,
    /// Part of the cell searched by `find`.
    pub scope: Scope,
    /// Content of the listed cells.
    pub view: Option<View>,
}

/// Change of one cell or of text within cells.
///
/// `source` replaces the whole source; `find` with `with` replaces text
/// within the source of the target cell, or of every cell when there is no
/// target. `cell_type` and `reset_outputs` need a target or `find`.
#[derive(Debug, Clone, Default)]
pub struct Replace<'a> {
    /// Cell to change.
    pub target: Option<CellRef>,
    /// New source of the target cell.
    pub source: Option<String>,
    /// New type of the target cell.
    pub cell_type: Option<CellType>,
    /// Text to replace within sources.
    pub find: Option<&'a dyn Pattern>,
    /// Replacement for each match of `find`.
    pub with: Option<String>,
    /// Clear the outputs and execution counter of changed code cells.
    pub reset_outputs: bool,
}

/// Result of [`Notebook::insert`] and [`Notebook::copy_cell`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Inserted {
    /// Index of the new cell.
    pub index: usize,
    /// Identifier of the new cell, when the notebook uses identifiers.
    pub id: Option<String>,
}

/// Result of [`Notebook::replace`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Replaced {
    /// Indexes of the changed cells.
    pub cells: Vec<usize>,
    /// Number of replaced matches of `find`.
    pub replacements: usize,
}

/// Result of [`Notebook::normalize`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Normalized {
    /// Indexes of the changed cells.
    pub cells: Vec<usize>,
    /// Whether members of the notebook itself changed: `nbformat_minor` or
    /// `metadata`.
    pub notebook: bool,
}

impl Notebook {
    /// Cells matching the query (`notebook.cells`).
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::IndexOutOfRange`] when the range starts past
    /// the last cell, and [`NotebookError::UnknownId`] or
    /// [`NotebookError::DuplicateId`] for an identifier that names no cell
    /// or several cells.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_notebook::{CellQuery, Literal, Notebook};
    ///
    /// let text = r##"{"cells": [
    ///   {"cell_type": "markdown", "metadata": {}, "source": "# Load\nData"},
    ///   {"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": "df = load()"}
    /// ], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"##;
    /// let notebook = Notebook::parse(text.as_bytes())?;
    ///
    /// let outline = notebook.cells(&CellQuery::default())?;
    /// assert_eq!(outline[0].source, "# Load");
    ///
    /// let pattern = Literal::new("df", true);
    /// let found = notebook.cells(&CellQuery { find: Some(&pattern), ..CellQuery::default() })?;
    /// assert_eq!(found.len(), 1);
    /// assert_eq!(found[0].index, 1);
    /// # Ok::<(), nurt_notebook::NotebookError>(())
    /// ```
    pub fn cells(&self, query: &CellQuery<'_>) -> Result<Vec<CellData>, NotebookError> {
        let selected = query.index.is_some() || !query.ids.is_empty();
        let mut indexes: Vec<usize> = if selected {
            let mut set = Vec::new();
            if let Some(range) = query.index {
                set.extend(self.resolve_selection(&Selection::Range(range), true)?);
            }
            if !query.ids.is_empty() {
                set.extend(self.resolve_selection(&Selection::Ids(query.ids.clone()), false)?);
            }
            set
        } else {
            (0..self.len()).collect()
        };
        indexes.sort_unstable();
        indexes.dedup();
        let view = query.view.unwrap_or(if selected || query.find.is_some() {
            View::Full
        } else {
            View::Outline
        });
        let mut cells = Vec::new();
        for index in indexes {
            let Some(cell) = self.stored(index) else {
                continue;
            };
            if query
                .cell_type
                .is_some_and(|wanted| wanted != cell.cell_type)
            {
                continue;
            }
            if let Some(pattern) = query.find {
                let haystack = match query.scope {
                    Scope::Source => cell.source(),
                    Scope::Outputs => cell.outputs_text(),
                };
                if pattern.find_all(&haystack).is_empty() {
                    continue;
                }
            }
            cells.push(cell.data(view, query.outputs));
        }
        Ok(cells)
    }

    /// Inserts a new cell (`notebook.insert`).
    ///
    /// The cell gets a new identifier when the notebook uses identifiers
    /// (nbformat 4.5 and later). Its `source` takes the form used by the
    /// other cells: a list of lines or a single string.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::IndexOutOfRange`] for an index greater than
    /// the number of cells, and [`NotebookError::UnknownId`] for an unknown
    /// `after_id`.
    pub fn insert(
        &mut self,
        position: &Position,
        cell_type: CellType,
        source: &str,
    ) -> Result<Inserted, NotebookError> {
        let index = match position {
            Position::End => self.len(),
            Position::Index(index) => self.check_position(*index)?,
            Position::AfterId(id) => self.index_of_id(id)? + 1,
        };
        let cell = self.new_cell(cell_type, source);
        let id = cell.get("id").and_then(Value::as_str).map(str::to_owned);
        self.insert_cell(index, cell, cell_type);
        Ok(Inserted { index, id })
    }

    /// Replaces the source or type of a cell, or text within cell sources
    /// (`notebook.replace`).
    ///
    /// Outputs stay unless `reset_outputs` is set. A changed type adds or
    /// removes the members that nbformat requires for it: `outputs` and
    /// `execution_count` exist only in code cells, `attachments` only in
    /// Markdown and raw cells.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidChange`] for inconsistent arguments,
    /// [`NotebookError::NoMatch`] when `find` occurs in no selected cell, and
    /// the addressing errors of [`CellRef`].
    pub fn replace(&mut self, change: &Replace<'_>) -> Result<Replaced, NotebookError> {
        let target = validate_replace(change)?;
        let target = target.map(|t| self.resolve(t)).transpose()?;
        let mut outcome = Replaced::default();
        let mut changed = HashSet::new();
        if let (Some(pattern), Some(with)) = (change.find, &change.with) {
            let scope: Vec<usize> = target.map_or_else(|| (0..self.len()).collect(), |t| vec![t]);
            for index in scope {
                let Some(cell) = self.stored(index) else {
                    continue;
                };
                let (text, count) = replace_all(pattern, &cell.source(), with);
                if count > 0 {
                    outcome.replacements += count;
                    if self.set_source(index, &text) {
                        changed.insert(index);
                    }
                }
            }
            if outcome.replacements == 0 {
                return Err(NotebookError::NoMatch {
                    pattern: pattern.display(),
                });
            }
        }
        if let Some(index) = target {
            if let Some(text) = &change.source
                && self.set_source(index, text)
            {
                changed.insert(index);
            }
            if let Some(cell_type) = change.cell_type
                && self.change_type(index, cell_type)
            {
                changed.insert(index);
            }
        }
        if change.reset_outputs {
            let reset: Vec<usize> = match (change.find, target) {
                (Some(_), _) => changed.iter().copied().collect(),
                (None, Some(index)) => vec![index],
                (None, None) => Vec::new(),
            };
            for index in reset {
                if self.clear_cell_outputs(index) {
                    changed.insert(index);
                }
            }
        }
        outcome.cells = changed.into_iter().collect();
        outcome.cells.sort_unstable();
        Ok(outcome)
    }

    /// Moves a cell so that it ends at index `to` (`notebook.move`).
    /// Returns the previous index of the cell.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::IndexOutOfRange`] when `to` is not the index
    /// of an existing cell, and the addressing errors of [`CellRef`].
    pub fn move_cell(&mut self, from: &CellRef, to: i64) -> Result<usize, NotebookError> {
        let from = self.resolve(from)?;
        let to = self.check_index(to)?;
        if from != to {
            let (cell, cell_type) = self.remove_cell(from);
            self.insert_cell(to, cell, cell_type);
        }
        Ok(from)
    }

    /// Copies a cell to index `to`, by default directly after the original
    /// (`notebook.copy`).
    ///
    /// The copy repeats the text of the original exactly, except for a new
    /// identifier when the notebook uses identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::IndexOutOfRange`] when `to` is greater than
    /// the number of cells, and the addressing errors of [`CellRef`].
    pub fn copy_cell(
        &mut self,
        from: &CellRef,
        to: Option<i64>,
    ) -> Result<Inserted, NotebookError> {
        let from = self.resolve(from)?;
        let index = match to {
            Some(to) => self.check_position(to)?,
            None => from + 1,
        };
        let mut cell = self.cells[from].clone();
        let cell_type = self.cell_types[from];
        let id = if self.uses_ids() {
            let id = self.fresh_id();
            cell.set("id", json!(id));
            Some(id)
        } else {
            None
        };
        self.insert_cell(index, cell, cell_type);
        Ok(Inserted { index, id })
    }

    /// Removes the selected cells (`notebook.remove`). Returns their former
    /// indexes in ascending order.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::IndexOutOfRange`] when a range bound names
    /// no cell, and [`NotebookError::UnknownId`] or
    /// [`NotebookError::DuplicateId`] for an identifier that names no cell
    /// or several cells.
    pub fn remove(&mut self, selection: &Selection) -> Result<Vec<usize>, NotebookError> {
        let indexes = self.resolve_selection(selection, false)?;
        for index in indexes.iter().rev() {
            self.remove_cell(*index);
        }
        Ok(indexes)
    }

    /// Clears outputs and execution counters of the selected code cells
    /// (`notebook.clear_outputs`). Returns the indexes of the cells that
    /// changed; cells without outputs stay byte for byte.
    ///
    /// # Errors
    ///
    /// Returns the selection errors of [`Notebook::remove`].
    pub fn clear_outputs(&mut self, selection: &Selection) -> Result<Vec<usize>, NotebookError> {
        let indexes = self.resolve_selection(selection, false)?;
        Ok(indexes
            .into_iter()
            .filter(|index| self.clear_cell_outputs(*index))
            .collect())
    }

    /// Brings the notebook to nbformat 4.5 (`notebook.normalize`).
    ///
    /// Sets `nbformat_minor` to at least 5, adds a missing notebook
    /// `metadata`, gives every cell without a valid unique identifier a new
    /// one, adds missing `metadata` and `source` to cells and `outputs` and
    /// `execution_count` to code cells, and removes members that nbformat
    /// does not allow for the cell type. A second call changes nothing.
    pub fn normalize(&mut self) -> Normalized {
        let mut outcome = Normalized::default();
        if self.minor() < MINOR_WITH_IDS {
            outcome.notebook |= self.set_root("nbformat_minor", json!(MINOR_WITH_IDS));
        }
        if !self.root_has("metadata") {
            outcome.notebook |= self.set_root("metadata", json!({}));
        }
        let form = self.source_form();
        let mut seen = HashSet::new();
        for index in 0..self.len() {
            let current = self.cell_id(index).map(str::to_owned);
            let mut changed = false;
            let id = match current {
                Some(id) if is_valid_id(&id) && !seen.contains(&id) => id,
                _ => {
                    let id = self.fresh_id();
                    changed |= self.with_cell(index, |cell| cell.set("id", json!(id)));
                    id
                }
            };
            seen.insert(id);
            let is_code = self.cell_types.get(index) == Some(&CellType::Code);
            changed |= self.with_cell(index, |cell| {
                let mut changed = false;
                if !cell.get("metadata").is_some_and(Value::is_object) {
                    changed |= cell.set("metadata", json!({}));
                }
                if cell.get("source").is_none() {
                    changed |= cell.set("source", form.store(""));
                }
                if is_code {
                    if cell.get("outputs").is_none() {
                        changed |= cell.set("outputs", json!([]));
                    }
                    if cell.get("execution_count").is_none() {
                        changed |= cell.set("execution_count", Value::Null);
                    }
                    changed |= cell.remove("attachments");
                } else {
                    changed |= cell.remove("outputs");
                    changed |= cell.remove("execution_count");
                }
                changed
            });
            if changed {
                outcome.cells.push(index);
            }
        }
        outcome
    }

    fn with_cell(&mut self, index: usize, change: impl FnOnce(&mut JsonObject) -> bool) -> bool {
        self.cell_mut(index).is_some_and(change)
    }

    /// Sets the source text of a cell in the cell's own form. Returns
    /// whether the cell changed.
    fn set_source(&mut self, index: usize, text: &str) -> bool {
        let form = match self.cells.get(index) {
            Some(cell) => {
                let stored = cell.get("source");
                if stored.and_then(source::join).as_deref() == Some(text) {
                    return false;
                }
                stored.and_then(TextForm::of)
            }
            None => return false,
        };
        let form = form.unwrap_or_else(|| self.source_form());
        self.with_cell(index, |cell| cell.set("source", form.store(text)))
    }

    /// Changes the type of a cell with the members nbformat requires for
    /// the new type. Returns whether the cell changed.
    fn change_type(&mut self, index: usize, cell_type: CellType) -> bool {
        if self.cell_types.get(index) == Some(&cell_type) {
            return false;
        }
        let changed = self.with_cell(index, |cell| {
            cell.set("cell_type", json!(cell_type.as_str()));
            if cell_type == CellType::Code {
                cell.remove("attachments");
                if cell.get("execution_count").is_none() {
                    cell.set("execution_count", Value::Null);
                }
                if cell.get("outputs").is_none() {
                    cell.set("outputs", json!([]));
                }
            } else {
                cell.remove("outputs");
                cell.remove("execution_count");
            }
            true
        });
        self.set_cell_type(index, cell_type);
        changed
    }

    /// Clears the outputs and execution counter of a code cell. Returns
    /// whether the cell changed.
    fn clear_cell_outputs(&mut self, index: usize) -> bool {
        if self.cell_types.get(index) != Some(&CellType::Code) {
            return false;
        }
        self.with_cell(index, |cell| {
            let outputs = cell.set("outputs", json!([]));
            let count = cell.set("execution_count", Value::Null);
            outputs || count
        })
    }
}

fn validate_replace<'r>(change: &'r Replace<'_>) -> Result<Option<&'r CellRef>, NotebookError> {
    match (&change.find, &change.with, &change.source) {
        (Some(_), _, Some(_)) => {
            return Err(NotebookError::invalid(
                "give either source or find with with, not both",
            ));
        }
        (Some(_), None, None) => {
            return Err(NotebookError::invalid(
                "find needs with, the replacement text",
            ));
        }
        (None, Some(_), _) => {
            return Err(NotebookError::invalid(
                "with needs find, the text to replace",
            ));
        }
        _ => {}
    }
    if change.find.is_none()
        && change.source.is_none()
        && change.cell_type.is_none()
        && !change.reset_outputs
    {
        return Err(NotebookError::invalid(
            "nothing to change; give source, cell_type, find with with, or reset_outputs",
        ));
    }
    if change.target.is_none() && change.find.is_none() {
        return Err(NotebookError::invalid(
            "give index or id of the cell to change",
        ));
    }
    if change.target.is_none() && change.cell_type.is_some() {
        return Err(NotebookError::invalid(
            "cell_type needs index or id of the cell to change",
        ));
    }
    Ok(change.target.as_ref())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
