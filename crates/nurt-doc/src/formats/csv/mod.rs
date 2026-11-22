// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! CSV and TSV documents.
//!
//! A [`CsvDocument`] keeps the source text and the byte range of every
//! field. Reading follows the `csv` crate in flexible mode (RFC 4180 quoting
//! with `""` for a quote inside a quoted field, fields over several lines,
//! rows with different numbers of fields, blank lines skipped). Changes are
//! replacements of byte ranges, so the separator, the quote character, the
//! line terminators, the byte order mark and every unchanged field keep
//! their bytes.
//!
//! The separator is detected among `,`, `;`, tab and `|` unless it is given;
//! the quote character is `"`, or `'` when the file quotes fields with it
//! and has no `"`.
//!
//! # Keys
//!
//! | Key | Addresses |
//! |---|---|
//! | `[0]` | first data row: a map by column name with a header, a list without |
//! | `[0].name` | cell of the first data row in column `name` (header only) |
//! | `[0][2]` | third cell of the first data row |
//! | `[-1]` | last data row |
//! | `[*].name`, `[0][*]` | every row, every cell of a row |
//! | `[?id == "7"].name` | rows whose value matches the filter |
//! | `[+]` | row appended after the last one, in the style of the file |
//!
//! The header row is not a data row. Cells are text; a number or a truth
//! value is written in its usual text form. A cell is quoted only when its
//! text contains the separator, the quote character or a line break, or when
//! every field of the file is quoted. Removing a cell empties it; removing a
//! row removes it with its line terminator.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::keypath::KeyPath;
//! use nurt_contracts::services::DocFormatId;
//! use nurt_contracts::value::Value;
//! use nurt_doc::formats::csv::{CsvDocument, CsvOptions};
//!
//! let text = b"id;name\r\n1;Ann\r\n";
//! let document = CsvDocument::parse(text, &DocFormatId::CSV, &CsvOptions::default())?;
//! assert_eq!(document.delimiter(), ';');
//!
//! let items = document.read(&KeyPath::parse("[0].name")?)?;
//! assert_eq!(items[0].value, Value::Text("Ann".to_owned()));
//!
//! let change = document.set(&KeyPath::parse("[0].name")?, &Value::Text("Ann; Bo".to_owned()))?;
//! assert_eq!(change.apply(text), b"id;name\r\n1;\"Ann; Bo\"\r\n");
//!
//! let change = document.set(&KeyPath::parse("[+]")?, &Value::List(vec![Value::Int(2), Value::Text("Cy".to_owned())]))?;
//! assert_eq!(change.apply(text), b"id;name\r\n1;Ann\r\n2;Cy\r\n");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod dialect;
mod edit;
mod errors;
mod resolve;
mod scan;
mod table;

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::record::Row;
use nurt_contracts::services::{DocFormatId, DocItem};
use nurt_contracts::value::{Path, Value};

use crate::model::{DocTree, NodeKind, NodeSpec, Span, TreeBuilder};
use edit::Splice;
use resolve::{Access, Target};
use table::Table;

/// File name extensions of comma-separated files, without the period.
pub const CSV_EXTENSIONS: [&str; 1] = ["csv"];

/// File name extensions of tab-separated files, without the period.
pub const TSV_EXTENSIONS: [&str; 2] = ["tsv", "tab"];

/// Options of [`CsvDocument::parse`], the parameters `header` and
/// `delimiter` of `read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvOptions {
    /// The first row names the columns and is not a data row.
    pub header: bool,
    /// Field separator; `None` detects it.
    pub delimiter: Option<char>,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self { header: true, delimiter: None }
    }
}

/// Replacement of a byte range of a delimited file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvEdit {
    /// Replaced bytes of the source text; empty for an insertion.
    pub range: Range<usize>,
    /// New text.
    pub replacement: String,
}

/// Change of a delimited file: replacements of non-overlapping byte ranges
/// of the source text, ordered by position.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CsvChange {
    /// Replacements in ascending order of their ranges.
    pub edits: Vec<CsvEdit>,
}

impl CsvChange {
    /// `true` when the change leaves the text as it is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Text after the change; `text` is the source text of the document.
    #[must_use]
    pub fn apply(&self, text: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        let mut position = 0;
        for edit in &self.edits {
            out.extend_from_slice(&text[position..edit.range.start]);
            out.extend_from_slice(edit.replacement.as_bytes());
            position = edit.range.end;
        }
        out.extend_from_slice(&text[position..]);
        out
    }
}

/// CSV or TSV document with the byte range of every field.
#[derive(Debug, Clone)]
pub struct CsvDocument {
    format: DocFormatId,
    text: Vec<u8>,
    table: Table,
    tree: DocTree,
}

impl CsvDocument {
    /// Parses a CSV or TSV document from UTF-8 text with an optional byte
    /// order mark.
    ///
    /// # Errors
    ///
    /// Returns `E_ENCODING` for text that is not UTF-8 and `E_INVALID_VALUE`
    /// for a format other than `csv` or `tsv` or a separator that is not a
    /// single ASCII character other than a quote or a line break.
    pub fn parse(text: &[u8], format: &DocFormatId, options: &CsvOptions) -> Result<Self> {
        let default_delimiter = if *format == DocFormatId::CSV {
            b','
        } else if *format == DocFormatId::TSV {
            b'\t'
        } else {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("format {format} is not a delimited format; pass csv or tsv"),
            ));
        };
        let delimiter = options.delimiter.map(delimiter_byte).transpose()?;
        let table = Table::parse(text, options.header, delimiter, default_delimiter)?;
        let tree = build_tree(text, &table);
        Ok(Self { format: format.clone(), text: text.to_vec(), table, tree })
    }

    /// Format of the document, `csv` or `tsv`.
    #[must_use]
    pub const fn format(&self) -> &DocFormatId {
        &self.format
    }

    /// Source text.
    #[must_use]
    pub fn text(&self) -> &[u8] {
        &self.text
    }

    /// Field separator.
    #[must_use]
    pub fn delimiter(&self) -> char {
        char::from(self.table.dialect.delimiter)
    }

    /// Quote character.
    #[must_use]
    pub fn quote(&self) -> char {
        char::from(self.table.dialect.quote)
    }

    /// Column names from the header; empty without a header.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        self.table.column_names().into_iter().map(str::to_owned).collect()
    }

    /// Node tree: a list of rows, each a map of cells by column name with a
    /// header or a list of cells without one.
    #[must_use]
    pub const fn tree(&self) -> &DocTree {
        &self.tree
    }

    /// Value of the whole document: the list of data rows.
    #[must_use]
    pub fn to_value(&self) -> Value {
        self.table.to_value()
    }

    /// Data rows as `row` records for `read as: rows`; `no` counts data rows
    /// from 1 and `cells` keeps the column order.
    #[must_use]
    pub fn rows(&self, path: &Path) -> Vec<Row> {
        self.table
            .rows()
            .iter()
            .zip(1u64..)
            .map(|(record, no)| Row {
                path: path.clone(),
                no,
                cells: self.table.row_cells(record),
                table: None,
            })
            .collect()
    }

    /// Values at `key`, one item per addressed row or cell.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_NOT_FOUND` for a row, a column or a cell that does not
    /// exist and `E_KEY_CONFLICT` for a key that does not address a table
    /// node, for example `a.b`.
    pub fn read(&self, key: &KeyPath) -> Result<Vec<DocItem>> {
        let targets = resolve::resolve(&self.table, &self.tree, key, Access::Read)?;
        Ok(targets.into_iter().map(|target| self.item(target)).collect())
    }

    /// Change that stores `value` at `key`.
    ///
    /// A cell takes a scalar. A row takes a list of cells, a map by column
    /// name or a record; a map written to a table without rows and with a
    /// header also writes the header from its keys. `[+]` and the index equal
    /// to the number of rows append a row. Storing the current value changes
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`CsvDocument::read`], `E_TYPE` for a list or a
    /// map stored in a cell, `E_KEY_NOT_FOUND` for a map with a key that is
    /// not a column and `E_KEY_CONFLICT` for keys that select overlapping
    /// nodes, such as `**`.
    pub fn set(&self, key: &KeyPath, value: &Value) -> Result<CsvChange> {
        let targets = resolve::resolve(&self.table, &self.tree, key, Access::Write)?;
        let mut splices = Vec::new();
        for target in targets {
            splices.extend(edit::set(&self.table, &self.text, key, target, value)?);
        }
        finish(key, splices)
    }

    /// Change that removes the nodes at `key`: a cell is emptied, a row is
    /// removed with its line terminator.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`CsvDocument::read`] and `E_KEY_CONFLICT` for
    /// the whole table.
    pub fn remove(&self, key: &KeyPath) -> Result<CsvChange> {
        let targets = resolve::resolve(&self.table, &self.tree, key, Access::Read)?;
        let mut splices = Vec::new();
        for target in targets {
            splices.extend(edit::remove(&self.table, &self.text, key, target)?);
        }
        finish(key, splices)
    }

    fn item(&self, target: Target) -> DocItem {
        let (value, range) = match target {
            Target::Table => (self.table.to_value(), Some(self.table.scan.bom..self.text.len())),
            Target::Row(row) => {
                let record = &self.table.rows()[row];
                (self.table.row_value(record), Some(record.span.clone()))
            }
            Target::Cell { row, column } => match self.table.rows()[row].fields.get(column) {
                Some(field) => (table::field_value(field), Some(field.raw.clone())),
                None => (Value::Text(String::new()), None),
            },
            Target::NewRow | Target::NewRowCell { .. } => (Value::Null, None),
        };
        DocItem {
            key: resolve::canonical_key(&self.table, target),
            value,
            span: range.map(|range| self.tree.text_span(&self.text, Span::new(range.start, range.end))),
            comment: None,
            document: None,
            anchor: None,
            alias_of: None,
            tag: None,
        }
    }
}

/// Byte of a separator given as a character.
fn delimiter_byte(delimiter: char) -> Result<u8> {
    u8::try_from(delimiter)
        .ok()
        .filter(|byte| byte.is_ascii() && !matches!(byte, b'"' | b'\'' | b'\r' | b'\n'))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("separator {delimiter:?} is not allowed; pass one ASCII character such as , ; | or a tab"),
            )
        })
}

/// Orders the replacements; removals of adjacent or overlapping rows are
/// merged, any other overlap is a conflict.
fn finish(key: &KeyPath, mut splices: Vec<Splice>) -> Result<CsvChange> {
    splices.sort_by_key(|splice| (splice.range.start, splice.range.end));
    let mut edits: Vec<CsvEdit> = Vec::with_capacity(splices.len());
    for splice in splices {
        if let Some(last) = edits.last_mut() {
            let overlaps = splice.range.start < last.range.end
                || (splice.range.start == last.range.start && splice.range.is_empty() && last.range.is_empty());
            if overlaps {
                if !(last.replacement.is_empty() && splice.text.is_empty()) {
                    return Err(errors::key_shape(key, "the key selects rows and cells that overlap; select either rows or cells"));
                }
                last.range.end = last.range.end.max(splice.range.end);
                continue;
            }
        }
        edits.push(CsvEdit { range: splice.range, replacement: splice.text });
    }
    Ok(CsvChange { edits })
}

/// Node tree of a table: a list of rows, each a map of named cells with a
/// header or a list of cells without one.
fn build_tree(text: &[u8], table: &Table) -> DocTree {
    let mut builder = TreeBuilder::new(text);
    let root = builder.node(None, NodeSpec::new(NodeKind::List, Span::new(table.scan.bom, text.len())));
    let row_kind = if table.header { NodeKind::Map } else { NodeKind::List };
    for record in table.rows() {
        let row = builder.node(Some(root), NodeSpec::new(row_kind, Span::new(record.span.start, record.span.end)));
        for (column, field) in record.fields.iter().enumerate() {
            let mut spec = NodeSpec::new(NodeKind::Scalar, Span::new(field.raw.start, field.raw.end))
                .with_value(table::field_value(field));
            if table.header {
                spec = spec.with_name(table.column_label(column));
            }
            builder.node(Some(row), spec);
        }
    }
    builder.finish()
}
