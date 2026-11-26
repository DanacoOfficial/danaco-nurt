// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of key paths to rows and cells.
//!
//! The first segment selects rows (`[2]`, `[-1]`, `[*]`, `[+]`, a filter or
//! `**`), the second selects a cell by column name (`.name`, header only) or
//! by index (`[1]`). A path with more segments addresses nothing, because
//! cells are text.

use std::collections::HashMap;

use nurt_contracts::error::Error;
use nurt_contracts::keypath::{KeyPath, KeySegment};

use super::errors;
use super::table::{MAX_COLUMNS, Table};
use crate::model::{DocTree, MapSemantics, NodeId};

/// Node of a table addressed by a key path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// The whole table.
    Table,
    /// Data row by index into [`Table::rows`].
    Row(usize),
    /// Cell of a data row; the column may lie past the end of the row.
    Cell {
        /// Data row index.
        row: usize,
        /// Column index.
        column: usize,
    },
    /// Row appended after the last one.
    NewRow,
    /// Cell of a row appended after the last one, addressed as `[n].name`
    /// where `n` is the number of data rows.
    NewRowCell {
        /// Column index.
        column: usize,
    },
}

/// Purpose of a resolution: reading accepts only existing nodes, writing
/// also accepts new rows and cells past the end of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
}

/// Resolves `key` to the nodes it addresses.
///
/// Filters are evaluated on the rows of `tree`, the node tree of the table.
///
/// # Errors
///
/// Returns `E_KEY_NOT_FOUND` when an index or a column does not exist and
/// `E_KEY_CONFLICT` when the path is deeper than a cell or uses a segment
/// that has no meaning in a table.
pub(crate) fn resolve(
    table: &Table,
    tree: &DocTree,
    key: &KeyPath,
    access: Access,
) -> Result<Vec<Target>, Error> {
    let segments = key.segments();
    let Some((first, rest)) = segments.split_first() else {
        return Ok(vec![Target::Table]);
    };
    if rest.len() > 1 {
        return Err(errors::key_shape(
            key,
            "a cell holds text and has no members",
        ));
    }
    if matches!(first, KeySegment::Recursive) {
        return Ok(descendants(table));
    }
    let rows = select_rows(table, tree, key, first, access)?;
    let Some(column) = rest.first() else {
        return Ok(rows);
    };
    let mut cells = Vec::new();
    for row in rows {
        cells.extend(select_cells(table, key, row, column, access)?);
    }
    Ok(cells)
}

fn select_rows(
    table: &Table,
    tree: &DocTree,
    key: &KeyPath,
    segment: &KeySegment,
    access: Access,
) -> Result<Vec<Target>, Error> {
    let count = table.rows().len();
    match segment {
        KeySegment::Index(index) => row_index(key, *index, count, access).map(|row| vec![row]),
        KeySegment::Append if access == Access::Write => Ok(vec![Target::NewRow]),
        KeySegment::Append => Err(errors::key_not_found(
            key,
            "[+] addresses a row that does not exist yet",
        )),
        KeySegment::Wildcard => Ok((0..count).map(Target::Row).collect()),
        KeySegment::Filter(_) => filtered_rows(tree, key, segment),
        _ => Err(errors::key_shape(
            key,
            "the first segment selects a row; write [0], [-1], [*] or [+]",
        )),
    }
}

fn row_index(key: &KeyPath, index: i64, count: usize, access: Access) -> Result<Target, Error> {
    let resolved = absolute_index(index, count);
    match resolved {
        Some(row) if row < count => Ok(Target::Row(row)),
        Some(row) if row == count && access == Access::Write => Ok(Target::NewRow),
        _ => Err(errors::key_not_found(
            key,
            &format!("the table has {count} data rows"),
        )),
    }
}

/// Index from the start for a non-negative `index`, from the end for a
/// negative one.
fn absolute_index(index: i64, count: usize) -> Option<usize> {
    if index >= 0 {
        usize::try_from(index).ok()
    } else {
        let back = usize::try_from(index.unsigned_abs()).ok()?;
        count.checked_sub(back)
    }
}

/// Rows selected by a filter segment, evaluated on the row values of the
/// node tree.
fn filtered_rows(
    tree: &DocTree,
    key: &KeyPath,
    segment: &KeySegment,
) -> Result<Vec<Target>, Error> {
    let filter = KeyPath::new(None, vec![segment.clone()])
        .map_err(|error| errors::key_shape(key, &error.to_string()))?;
    let rows: &[NodeId] = tree
        .roots()
        .first()
        .map_or(&[], |root| tree[*root].children.as_slice());
    let positions: HashMap<NodeId, usize> = rows
        .iter()
        .enumerate()
        .map(|(position, row)| (*row, position))
        .collect();
    Ok(tree
        .resolve(&filter, &MapSemantics::LAST_WINS)?
        .into_iter()
        .filter_map(|found| positions.get(&found.node).copied())
        .map(Target::Row)
        .collect())
}

fn select_cells(
    table: &Table,
    key: &KeyPath,
    row: Target,
    segment: &KeySegment,
    access: Access,
) -> Result<Vec<Target>, Error> {
    let width = match row {
        Target::Row(index) => table.rows()[index].fields.len().max(header_width(table)),
        _ => header_width(table),
    };
    let columns = match segment {
        KeySegment::Key(name) => vec![column_by_name(table, key, name)?],
        KeySegment::Index(index) => {
            let column = absolute_index(*index, width)
                .ok_or_else(|| errors::key_not_found(key, &format!("the row has {width} cells")))?;
            if column >= width && access == Access::Read {
                return Err(errors::key_not_found(
                    key,
                    &format!("the row has {width} cells"),
                ));
            }
            if column >= width.max(MAX_COLUMNS) {
                return Err(errors::key_not_found(
                    key,
                    &format!("a row can be extended to {MAX_COLUMNS} cells"),
                ));
            }
            vec![column]
        }
        KeySegment::Wildcard => (0..width).collect(),
        _ => {
            return Err(errors::key_shape(
                key,
                "the second segment selects a cell; write .name, [0] or [*]",
            ));
        }
    };
    Ok(columns
        .into_iter()
        .map(|column| match row {
            Target::Row(row) => Target::Cell { row, column },
            _ => Target::NewRowCell { column },
        })
        .collect())
}

fn header_width(table: &Table) -> usize {
    table
        .header_record()
        .map_or(0, |record| record.fields.len())
}

fn column_by_name(table: &Table, key: &KeyPath, name: &str) -> Result<usize, Error> {
    if !table.header {
        return Err(errors::key_not_found(
            key,
            "the table is read without a header; address cells by index, for example [0][1]",
        ));
    }
    table.label_column(name).ok_or_else(|| {
        let columns = table.column_names().join(", ");
        errors::key_not_found(key, &format!("no column {name:?}; columns: {columns}"))
    })
}

/// Every row followed by its cells, for `**`.
fn descendants(table: &Table) -> Vec<Target> {
    let mut targets = Vec::new();
    for (row, record) in table.rows().iter().enumerate() {
        targets.push(Target::Row(row));
        let width = record.fields.len().max(header_width(table));
        targets.extend((0..width).map(|column| Target::Cell { row, column }));
    }
    targets
}

/// Canonical key of a resolved node.
///
/// A cell is named by its column when the header names it once, by index
/// otherwise.
pub(crate) fn canonical_key(table: &Table, target: Target) -> KeyPath {
    let mut segments = Vec::new();
    match target {
        Target::Table => {}
        Target::Row(row) => segments.push(index_segment(row)),
        Target::Cell { row, column } => {
            segments.push(index_segment(row));
            segments.push(column_segment(table, column));
        }
        Target::NewRow => segments.push(index_segment(table.rows().len())),
        Target::NewRowCell { column } => {
            segments.push(index_segment(table.rows().len()));
            segments.push(column_segment(table, column));
        }
    }
    KeyPath::new(None, segments).unwrap_or_else(|_| KeyPath::root())
}

fn index_segment(index: usize) -> KeySegment {
    KeySegment::Index(i64::try_from(index).unwrap_or(i64::MAX))
}

fn column_segment(table: &Table, column: usize) -> KeySegment {
    let names = table.column_names();
    match names.get(column) {
        Some(name)
            if !name.is_empty() && names.iter().filter(|other| *other == name).count() == 1 =>
        {
            KeySegment::Key((*name).to_owned())
        }
        _ => index_segment(column),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::services::DocFormatId;

    use super::*;
    use crate::formats::csv::{CsvDocument, CsvOptions};

    fn document(text: &str, header: bool) -> CsvDocument {
        let options = CsvOptions {
            header,
            delimiter: None,
        };
        CsvDocument::parse(text.as_bytes(), &DocFormatId::CSV, &options).unwrap()
    }

    fn targets(document: &CsvDocument, key: &str, access: Access) -> Result<Vec<Target>, Error> {
        resolve(
            &document.table,
            &document.tree,
            &KeyPath::parse(key).unwrap(),
            access,
        )
    }

    fn code(error: &Error) -> &str {
        error.code().as_str()
    }

    #[test]
    fn root_key_addresses_the_table() {
        let document = document("id,name\n1,ann\n", true);
        let found = resolve(
            &document.table,
            &document.tree,
            &KeyPath::root(),
            Access::Read,
        )
        .unwrap();
        assert_eq!(found, [Target::Table]);
        assert_eq!(
            canonical_key(&document.table, Target::Table),
            KeyPath::root()
        );
    }

    #[test]
    fn recursive_key_lists_rows_followed_by_their_cells() {
        let document = document("a,b\n1,2,3\n4\n", true);
        let found = targets(&document, "**", Access::Read).unwrap();
        assert_eq!(
            found,
            [
                Target::Row(0),
                Target::Cell { row: 0, column: 0 },
                Target::Cell { row: 0, column: 1 },
                Target::Cell { row: 0, column: 2 },
                Target::Row(1),
                Target::Cell { row: 1, column: 0 },
                Target::Cell { row: 1, column: 1 },
            ]
        );
    }

    #[test]
    fn new_row_cells_use_the_header_width() {
        let document = document("id,name\n1,ann\n", true);
        assert_eq!(
            targets(&document, "[1].name", Access::Write).unwrap(),
            [Target::NewRowCell { column: 1 }]
        );
        assert_eq!(
            targets(&document, "[1][*]", Access::Write).unwrap(),
            [
                Target::NewRowCell { column: 0 },
                Target::NewRowCell { column: 1 }
            ]
        );
        let error = targets(&document, "[1]", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
        let error = targets(&document, "[+]", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
    }

    #[test]
    fn cell_indexes_past_the_row_exist_only_for_writing() {
        let document = document("id,name\n1,ann\n", true);
        let error = targets(&document, "[0][5]", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
        assert_eq!(
            targets(&document, "[0][5]", Access::Write).unwrap(),
            [Target::Cell { row: 0, column: 5 }]
        );
        assert_eq!(
            targets(&document, "[0][-1]", Access::Read).unwrap(),
            [Target::Cell { row: 0, column: 1 }]
        );
        let error = targets(&document, "[0][-3]", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
        let error = targets(&document, "[-2]", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
    }

    #[test]
    fn cell_indexes_beyond_the_column_limit_are_rejected() {
        let document = document("id,name\n1,ann\n", true);
        let last = MAX_COLUMNS - 1;
        assert_eq!(
            targets(&document, &format!("[0][{last}]"), Access::Write).unwrap(),
            [Target::Cell {
                row: 0,
                column: last
            }]
        );
        for key in [
            format!("[0][{MAX_COLUMNS}]"),
            "[0][99999999999]".to_owned(),
            "[1][99999999999]".to_owned(),
        ] {
            let error = targets(&document, &key, Access::Write).unwrap_err();
            assert_eq!(code(&error), "E_KEY_NOT_FOUND", "{key}");
        }
    }

    #[test]
    fn filters_select_rows_of_a_large_table() {
        let mut text = String::from("id,name\n");
        for row in 0..2000 {
            text.push_str(&format!("{},n{row}\n", row % 2));
        }
        let document = document(&text, true);
        let found = targets(&document, "[?id == \"1\"]", Access::Read).unwrap();
        assert_eq!(found.len(), 1000);
        assert_eq!(found[0], Target::Row(1));
        assert_eq!(found[999], Target::Row(1999));
    }

    #[test]
    fn rejects_segments_without_meaning_in_a_table() {
        let document = document("id,name\n1,ann\n", true);
        for key in ["[0][+]", "[0].**", "name", "[0].name.first"] {
            let error = targets(&document, key, Access::Write).unwrap_err();
            assert_eq!(code(&error), "E_KEY_CONFLICT", "{key}");
        }
        let error = targets(&document, "[0].missing", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
        assert!(error.to_string().contains("columns: id, name"), "{error}");
    }

    #[test]
    fn names_require_a_header() {
        let document = document("1,ann\n", false);
        let error = targets(&document, "[0].name", Access::Read).unwrap_err();
        assert_eq!(code(&error), "E_KEY_NOT_FOUND");
        assert_eq!(
            targets(&document, "[0][1]", Access::Read).unwrap(),
            [Target::Cell { row: 0, column: 1 }]
        );
    }

    #[test]
    fn filters_select_matching_rows() {
        let document = document("id,name\n1,ann\n2,bob\n1,cy\n", true);
        assert_eq!(
            targets(&document, "[?id == \"1\"].name", Access::Read).unwrap(),
            [
                Target::Cell { row: 0, column: 1 },
                Target::Cell { row: 2, column: 1 }
            ]
        );
    }

    #[test]
    fn canonical_keys_name_unique_columns_and_index_the_rest() {
        let document = document("id,id,\n1,2,3\n", true);
        let table = &document.table;
        let key = |target| canonical_key(table, target).to_string();
        assert_eq!(key(Target::Row(0)), "[0]");
        assert_eq!(key(Target::Cell { row: 0, column: 0 }), "[0][0]");
        assert_eq!(key(Target::Cell { row: 0, column: 2 }), "[0][2]");
        assert_eq!(key(Target::NewRow), "[1]");
        assert_eq!(key(Target::NewRowCell { column: 1 }), "[1][1]");
        let named = CsvDocument::parse(
            b"id,name\n1,ann\n",
            &DocFormatId::CSV,
            &CsvOptions::default(),
        )
        .unwrap();
        assert_eq!(
            canonical_key(&named.table, Target::NewRowCell { column: 1 }).to_string(),
            "[1].name"
        );
    }
}
