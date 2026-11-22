// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Notebook document: parsing, validation, cell access and lossless output.

use std::collections::HashSet;

use nurt_contracts::record::CellType;
use serde_json::{Map, Value, json};

use crate::address::{CellRef, Selection};
use crate::cell::{CellData, OutputsDetail, StoredCell, View};
use crate::error::{NotebookError, SUPPORTED_MAJOR};
use crate::json::{Document, JsonObject, ObjectArray};
use crate::source::{self, TextForm};

/// Root member holding the cells.
const CELLS: &str = "cells";

/// Nesting level of the `cells` array: a member of the root object.
const CELLS_DEPTH: usize = 1;

/// First minor version of nbformat 4 with cell identifiers.
pub(crate) const MINOR_WITH_IDS: u64 = 5;

/// Length of generated cell identifiers, as in Jupyter.
const GENERATED_ID_LENGTH: usize = 8;

/// Longest valid cell identifier (nbformat 4.5).
const MAX_ID_LENGTH: usize = 64;

/// Source of identifiers for new cells.
///
/// The default generator follows Jupyter: the first 8 hexadecimal digits of
/// a random UUID. The sequential generator gives `00000001`, `00000002` and
/// so on, for reproducible output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdGenerator {
    sequence: Option<u64>,
}

impl IdGenerator {
    /// Generator of random identifiers.
    #[must_use]
    pub fn random() -> Self {
        Self { sequence: None }
    }

    /// Generator of sequential identifiers starting at `00000001`.
    #[must_use]
    pub fn sequential() -> Self {
        Self { sequence: Some(0) }
    }

    fn next_id(&mut self) -> String {
        match &mut self.sequence {
            Some(counter) => {
                *counter += 1;
                format!("{counter:0width$x}", width = GENERATED_ID_LENGTH)
            }
            None => {
                let mut id = uuid::Uuid::new_v4().simple().to_string();
                id.truncate(GENERATED_ID_LENGTH);
                id
            }
        }
    }
}

/// Whether `id` is a valid nbformat cell identifier: 1 to 64 ASCII letters,
/// digits, `-` or `_`.
pub(crate) fn is_valid_id(id: &str) -> bool {
    (1..=MAX_ID_LENGTH).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Jupyter notebook in nbformat 4, edited without losing the original text.
///
/// Rendering a notebook that was not changed gives the original bytes.
/// After a change, cells and members not touched by it keep their exact
/// original text: key order, indentation, number notation and escapes.
/// Values written anew follow the layout detected from the file.
///
/// # Examples
///
/// ```
/// use nurt_notebook::{Notebook, Position};
/// use nurt_contracts::record::CellType;
///
/// let text = "{\n \"cells\": [],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 5\n}\n";
/// let mut notebook = Notebook::parse(text.as_bytes())?;
/// assert_eq!(notebook.to_bytes(), text.as_bytes());
///
/// notebook.insert(&Position::End, CellType::Markdown, "# Title")?;
/// assert_eq!(notebook.len(), 1);
/// # Ok::<(), nurt_notebook::NotebookError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Notebook {
    document: Document,
    cells_array: ObjectArray,
    pub(crate) cells: Vec<JsonObject>,
    pub(crate) cell_types: Vec<CellType>,
    structure_changed: bool,
    ids: IdGenerator,
}

impl Notebook {
    /// Parses a notebook from the bytes of an `.ipynb` file.
    ///
    /// # Errors
    ///
    /// Returns an error with code `E_FORMAT`: [`NotebookError::NotUtf8`],
    /// [`NotebookError::InvalidJson`], [`NotebookError::Version`] for
    /// nbformat other than 4, and [`NotebookError::Structure`] when a
    /// required member is missing or has the wrong type.
    pub fn parse(bytes: &[u8]) -> Result<Self, NotebookError> {
        let document = Document::parse(bytes, &[CELLS])?;
        let nbformat = document
            .root
            .get("nbformat")
            .ok_or_else(|| NotebookError::structure("missing `nbformat`"))?
            .as_u64()
            .ok_or_else(|| NotebookError::structure("`nbformat` is not a non-negative integer"))?;
        if nbformat != SUPPORTED_MAJOR {
            return Err(NotebookError::Version { nbformat });
        }
        if let Some(minor) = document.root.get("nbformat_minor")
            && !minor.is_u64()
        {
            return Err(NotebookError::structure(
                "`nbformat_minor` is not a non-negative integer",
            ));
        }
        let start = document
            .member_value_start(CELLS)
            .ok_or_else(|| NotebookError::structure("missing `cells`"))?;
        if !document.text[start..].starts_with('[') {
            return Err(NotebookError::structure("`cells` is not a list"));
        }
        let (cells_array, cells) =
            ObjectArray::parse(&document.text, start, &document.layout, CELLS_DEPTH, "cell")?;
        let cell_types = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| validate_cell(index, cell))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            document,
            cells_array,
            cells,
            cell_types,
            structure_changed: false,
            ids: IdGenerator::default(),
        })
    }

    /// Bytes of the notebook; the original bytes when nothing changed.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.render().into_bytes()
    }

    /// Text of the notebook; the original text when nothing changed.
    #[must_use]
    pub fn render(&self) -> String {
        if !self.is_modified() {
            return self.document.text.to_string();
        }
        if self.cells_changed() {
            let cells = self
                .cells_array
                .render(&self.cells, &self.document.layout, CELLS_DEPTH);
            self.document.render(&[(CELLS, &cells)])
        } else {
            self.document.render(&[])
        }
    }

    /// Whether the notebook differs from the parsed text.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.cells_changed() || self.document.root.is_changed()
    }

    fn cells_changed(&self) -> bool {
        self.structure_changed || self.cells.iter().any(JsonObject::is_changed)
    }

    /// Replaces the source of identifiers for new cells.
    pub fn set_id_generator(&mut self, generator: IdGenerator) {
        self.ids = generator;
    }

    /// Major and minor version of nbformat.
    #[must_use]
    pub fn nbformat(&self) -> (u64, u64) {
        (SUPPORTED_MAJOR, self.minor())
    }

    pub(crate) fn minor(&self) -> u64 {
        self.document
            .root
            .get("nbformat_minor")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    }

    /// Number of cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the notebook has no cells.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Notebook metadata, `metadata` of the root object.
    #[must_use]
    pub fn metadata(&self) -> Map<String, Value> {
        self.document
            .root
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    /// Programming language of the kernel, lowercase: `language_info.name`,
    /// else `kernelspec.language`.
    #[must_use]
    pub fn language(&self) -> Option<String> {
        let metadata = self.document.root.get("metadata")?;
        metadata
            .pointer("/language_info/name")
            .or_else(|| metadata.pointer("/kernelspec/language"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_lowercase)
    }

    /// Cell at `index` with the full source, metadata and outputs.
    #[must_use]
    pub fn cell(&self, index: usize) -> Option<CellData> {
        self.stored(index)
            .map(|cell| cell.data(View::Full, OutputsDetail::Full))
    }

    pub(crate) fn stored(&self, index: usize) -> Option<StoredCell<'_>> {
        Some(StoredCell {
            index,
            cell_type: *self.cell_types.get(index)?,
            object: self.cells.get(index)?,
        })
    }

    pub(crate) fn cell_id(&self, index: usize) -> Option<&str> {
        self.cells.get(index)?.get("id").and_then(Value::as_str)
    }

    /// Whether new cells get identifiers: nbformat 4.5 and later, or a
    /// notebook whose cells already have them.
    pub(crate) fn uses_ids(&self) -> bool {
        self.minor() >= MINOR_WITH_IDS || (0..self.len()).any(|i| self.cell_id(i).is_some())
    }

    /// New identifier not used by any cell.
    pub(crate) fn fresh_id(&mut self) -> String {
        let used: HashSet<String> = (0..self.len())
            .filter_map(|i| self.cell_id(i).map(str::to_owned))
            .collect();
        loop {
            let id = self.ids.next_id();
            if !used.contains(&id) {
                return id;
            }
        }
    }

    /// Form of `source` for new cells: the form of the first cell, or a
    /// list of lines as written by Jupyter.
    pub(crate) fn source_form(&self) -> TextForm {
        self.cells
            .iter()
            .find_map(|cell| cell.get("source").and_then(TextForm::of))
            .unwrap_or(TextForm::Lines)
    }

    pub(crate) fn cell_mut(&mut self, index: usize) -> Option<&mut JsonObject> {
        self.cells.get_mut(index)
    }

    pub(crate) fn set_root(&mut self, key: &str, value: Value) -> bool {
        self.document.root.set(key, value)
    }

    pub(crate) fn root_has(&self, key: &str) -> bool {
        self.document.root.get(key).is_some()
    }

    pub(crate) fn insert_cell(&mut self, index: usize, cell: JsonObject, cell_type: CellType) {
        self.cells.insert(index, cell);
        self.cell_types.insert(index, cell_type);
        self.structure_changed = true;
    }

    pub(crate) fn remove_cell(&mut self, index: usize) -> (JsonObject, CellType) {
        self.structure_changed = true;
        (self.cells.remove(index), self.cell_types.remove(index))
    }

    pub(crate) fn set_cell_type(&mut self, index: usize, cell_type: CellType) {
        if let Some(slot) = self.cell_types.get_mut(index) {
            *slot = cell_type;
        }
    }

    /// Order of keys for new cells. When the keys of the first cell are
    /// sorted, as Jupyter writes them, or there are no cells, keys are
    /// sorted; otherwise they follow the order of the first cell, then other
    /// keys alphabetically.
    pub(crate) fn order_keys(&self, mut map: Map<String, Value>) -> Map<String, Value> {
        let template: Vec<&str> = self
            .cells
            .first()
            .map(|c| c.keys().collect())
            .unwrap_or_default();
        let sorted = template.windows(2).all(|pair| pair[0] <= pair[1]);
        let mut keys: Vec<String> = map.keys().cloned().collect();
        keys.sort_by_key(|key| {
            let rank = if sorted {
                None
            } else {
                template.iter().position(|known| known == key)
            };
            (rank.unwrap_or(usize::MAX), key.clone())
        });
        let mut ordered = Map::new();
        for key in keys {
            if let Some(value) = map.remove(&key) {
                ordered.insert(key, value);
            }
        }
        ordered
    }

    /// Stored members of a new cell.
    pub(crate) fn new_cell(&mut self, cell_type: CellType, text: &str) -> JsonObject {
        let mut map = Map::new();
        map.insert("cell_type".to_owned(), json!(cell_type.as_str()));
        if self.uses_ids() {
            map.insert("id".to_owned(), json!(self.fresh_id()));
        }
        map.insert("metadata".to_owned(), json!({}));
        map.insert("source".to_owned(), self.source_form().store(text));
        if cell_type == CellType::Code {
            map.insert("execution_count".to_owned(), Value::Null);
            map.insert("outputs".to_owned(), json!([]));
        }
        JsonObject::from_map(self.order_keys(map))
    }

    /// Index of the cell named by `target`.
    pub(crate) fn resolve(&self, target: &CellRef) -> Result<usize, NotebookError> {
        match target {
            CellRef::Index(index) => self.check_index(*index),
            CellRef::Id(id) => self.index_of_id(id),
            CellRef::IndexAndId(index, id) => {
                let by_index = self.check_index(*index)?;
                let by_id = self.index_of_id(id)?;
                if by_index == by_id {
                    Ok(by_index)
                } else {
                    Err(NotebookError::AddressMismatch {
                        index: by_index,
                        id: id.clone(),
                        id_index: by_id,
                        index_id: self.cell_id(by_index).map(str::to_owned),
                    })
                }
            }
        }
    }

    /// Indexes of the selected cells in ascending order. Explicit range
    /// bounds must name existing cells unless `clamp_end` allows an end past
    /// the last cell.
    pub(crate) fn resolve_selection(
        &self,
        selection: &Selection,
        clamp_end: bool,
    ) -> Result<Vec<usize>, NotebookError> {
        match selection {
            Selection::All => Ok((0..self.len()).collect()),
            Selection::Range(range) => {
                let start = match range.start() {
                    Some(start) => self.check_index(start)?,
                    None => 0,
                };
                let end = match range.end() {
                    Some(end) if clamp_end && end >= 0 => usize::try_from(end)
                        .map_or(self.len(), |end| end.saturating_add(1).min(self.len())),
                    Some(end) => self.check_index(end)? + 1,
                    None => self.len(),
                };
                Ok((start..end).collect())
            }
            Selection::Ids(ids) => {
                let mut indexes = ids
                    .iter()
                    .map(|id| self.index_of_id(id))
                    .collect::<Result<Vec<_>, _>>()?;
                indexes.sort_unstable();
                indexes.dedup();
                Ok(indexes)
            }
        }
    }

    pub(crate) fn check_index(&self, index: i64) -> Result<usize, NotebookError> {
        usize::try_from(index)
            .ok()
            .filter(|position| *position < self.len())
            .ok_or(NotebookError::IndexOutOfRange {
                index,
                len: self.len(),
            })
    }

    /// Checks a position for a new cell: from 0 to the number of cells.
    pub(crate) fn check_position(&self, index: i64) -> Result<usize, NotebookError> {
        usize::try_from(index)
            .ok()
            .filter(|position| *position <= self.len())
            .ok_or(NotebookError::IndexOutOfRange {
                index,
                len: self.len() + 1,
            })
    }

    pub(crate) fn index_of_id(&self, id: &str) -> Result<usize, NotebookError> {
        let mut found = (0..self.len()).filter(|i| self.cell_id(*i) == Some(id));
        match (found.next(), found.next()) {
            (Some(index), None) => Ok(index),
            (Some(_), Some(_)) => Err(NotebookError::DuplicateId { id: id.to_owned() }),
            (None, _) => {
                let ids: Vec<&str> = (0..self.len()).filter_map(|i| self.cell_id(i)).collect();
                Err(NotebookError::UnknownId {
                    id: id.to_owned(),
                    candidates: closest(id, &ids),
                    notebook_has_ids: !ids.is_empty(),
                })
            }
        }
    }
}

/// Checks the members of the cell at `index` that every operation reads.
fn validate_cell(index: usize, cell: &JsonObject) -> Result<CellType, NotebookError> {
    let cell_type = match cell.get("cell_type") {
        Some(Value::String(name)) => name.parse::<CellType>().map_err(|_| {
            NotebookError::structure(format!(
                "cell {index} has unknown cell_type `{name}`, expected code, markdown or raw"
            ))
        })?,
        Some(_) => {
            return Err(NotebookError::structure(format!(
                "cell {index} has a cell_type that is not text"
            )));
        }
        None => {
            return Err(NotebookError::structure(format!(
                "cell {index} has no cell_type"
            )));
        }
    };
    if cell
        .get("source")
        .is_some_and(|value| source::join(value).is_none())
    {
        return Err(NotebookError::structure(format!(
            "cell {index} has a source that is neither text nor a list of text"
        )));
    }
    if cell
        .get("outputs")
        .is_some_and(|outputs| !outputs.is_array())
    {
        return Err(NotebookError::structure(format!(
            "cell {index} has outputs that are not a list"
        )));
    }
    Ok(cell_type)
}

/// Identifiers closest to `wanted` by edit distance, no more than
/// [`MAX_CANDIDATES`].
fn closest(wanted: &str, ids: &[&str]) -> Vec<String> {
    let mut ranked: Vec<(usize, &str)> = ids
        .iter()
        .map(|id| (edit_distance(wanted, id), *id))
        .filter(|(distance, id)| *distance <= id.len().max(wanted.len()) / 2)
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, id)| id.to_owned())
        .collect()
}

/// Largest number of identifiers suggested for an unknown identifier.
const MAX_CANDIDATES: usize = 5;

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != *cb);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "{\n \"cells\": [\n  {\n   \"cell_type\": \"code\",\n   \"execution_count\": null,\n   \"id\": \"abc12345\",\n   \"metadata\": {},\n   \"outputs\": [],\n   \"source\": []\n  }\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 5\n}\n";

    #[test]
    fn rejects_old_format_versions() {
        let error = Notebook::parse(b"{\"nbformat\": 3, \"worksheets\": []}").unwrap_err();
        assert_eq!(error, NotebookError::Version { nbformat: 3 });
    }

    #[test]
    fn rejects_documents_without_cells() {
        let error = Notebook::parse(b"{\"nbformat\": 4, \"nbformat_minor\": 5}").unwrap_err();
        assert!(matches!(error, NotebookError::Structure { .. }));
    }

    #[test]
    fn rejects_unknown_cell_types() {
        let text = br#"{"cells": [{"cell_type": "heading", "source": ""}], "nbformat": 4}"#;
        let error = Notebook::parse(text).unwrap_err();
        assert!(error.to_string().contains("heading"), "{error}");
    }

    #[test]
    fn rejects_sources_of_wrong_type() {
        let text = br#"{"cells": [{"cell_type": "code", "source": 5}], "nbformat": 4}"#;
        assert!(matches!(
            Notebook::parse(text).unwrap_err(),
            NotebookError::Structure { .. }
        ));
    }

    #[test]
    fn keeps_unchanged_text() {
        let notebook = Notebook::parse(MINIMAL.as_bytes()).unwrap();
        assert!(!notebook.is_modified());
        assert_eq!(notebook.render(), MINIMAL);
        assert_eq!(notebook.nbformat(), (4, 5));
    }

    #[test]
    fn sequential_ids_are_reproducible() {
        let mut generator = IdGenerator::sequential();
        assert_eq!(generator.next_id(), "00000001");
        assert_eq!(generator.next_id(), "00000002");
    }

    #[test]
    fn random_ids_are_valid_and_eight_characters_long() {
        let id = IdGenerator::random().next_id();
        assert_eq!(id.len(), GENERATED_ID_LENGTH);
        assert!(is_valid_id(&id));
    }

    #[test]
    fn validates_identifier_syntax() {
        assert!(is_valid_id("a-b_C9"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("a b"));
        assert!(!is_valid_id(&"a".repeat(MAX_ID_LENGTH + 1)));
    }

    #[test]
    fn suggests_close_identifiers() {
        assert_eq!(closest("abc12346", &["abc12345", "zzzzzzzz"]), ["abc12345"]);
        assert!(closest("q", &["abc12345"]).is_empty());
    }

    #[test]
    fn reads_language_from_metadata() {
        let text = r#"{"cells": [], "metadata": {"kernelspec": {"language": "R"}}, "nbformat": 4}"#;
        let notebook = Notebook::parse(text.as_bytes()).unwrap();
        assert_eq!(notebook.language().as_deref(), Some("r"));
    }
}
