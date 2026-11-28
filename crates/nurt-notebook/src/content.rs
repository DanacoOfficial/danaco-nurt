// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-notebook::content
// Responsibility:   Cells of a notebook read as file content.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Cells of a notebook read as file content.

use crate::cell::{CellData, OutputsDetail, View};
use crate::error::NotebookError;
use crate::notebook::Notebook;

/// Notebook read as file content: format version, language and cells.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct NotebookContent {
    /// Major version of nbformat.
    pub nbformat: u64,
    /// Minor version of nbformat.
    pub nbformat_minor: u64,
    /// Programming language of the kernel, lowercase.
    pub language: Option<String>,
    /// Every cell with its full source, metadata and an outputs summary.
    pub cells: Vec<CellData>,
}

/// Reads the cells of an `.ipynb` file for the content of `read`.
///
/// # Errors
///
/// Returns the errors of [`Notebook::parse`], all with code `E_FORMAT`.
///
/// # Examples
///
/// ```
/// let text = br#"{"cells": [{"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": "1 + 1"}],
///   "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#;
/// let content = nurt_notebook::cells_as_content(text)?;
/// assert_eq!(content.cells[0].source, "1 + 1");
/// assert_eq!(content.cells[0].outputs_summary.as_deref(), Some("no outputs"));
/// # Ok::<(), nurt_notebook::NotebookError>(())
/// ```
pub fn cells_as_content(bytes: &[u8]) -> Result<NotebookContent, NotebookError> {
    let notebook = Notebook::parse(bytes)?;
    let (nbformat, nbformat_minor) = notebook.nbformat();
    let cells = (0..notebook.len())
        .filter_map(|index| notebook.stored(index))
        .map(|cell| cell.data(View::Full, OutputsDetail::Summary))
        .collect();
    Ok(NotebookContent {
        nbformat,
        nbformat_minor,
        language: notebook.language(),
        cells,
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
