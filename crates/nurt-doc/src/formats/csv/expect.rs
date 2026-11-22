// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Value of a table after a change, computed from the cell texts the change
//! writes, for the meaning guard.

use indexmap::IndexMap;
use nurt_contracts::error::Error;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;

use super::edit;
use super::resolve::Target;
use super::table::{Table, label_of};

/// Cells of a table while a change is applied to them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expected {
    header: Option<Vec<String>>,
    labelled: bool,
    rows: Vec<Option<Vec<String>>>,
    appended: Vec<Vec<String>>,
}

impl Expected {
    /// Cells of `table` before the change.
    pub(crate) fn new(table: &Table) -> Self {
        Self {
            header: table.header_record().map(|record| {
                record
                    .fields
                    .iter()
                    .map(|field| field.value.clone())
                    .collect()
            }),
            labelled: table.header,
            rows: table
                .rows()
                .iter()
                .map(|record| {
                    Some(
                        record
                            .fields
                            .iter()
                            .map(|field| field.value.clone())
                            .collect(),
                    )
                })
                .collect(),
            appended: Vec::new(),
        }
    }

    /// Stores `value` at `target`, as [`edit::set`] writes it.
    ///
    /// # Errors
    ///
    /// Returns the errors of the cell and row conversions of [`edit`].
    pub(crate) fn set(
        &mut self,
        table: &Table,
        key: &KeyPath,
        target: Target,
        value: &Value,
    ) -> Result<(), Error> {
        match target {
            Target::Table => {}
            Target::Cell { row, column } => {
                let text = edit::cell_text(key, value)?;
                if let Some(Some(cells)) = self.rows.get_mut(row) {
                    if cells.len() <= column {
                        cells.resize(column + 1, String::new());
                    }
                    cells[column] = text;
                }
            }
            Target::Row(row) => {
                let cells = non_empty(edit::row_texts(table, key, value, table.rows().get(row))?);
                if let Some(slot) = self.rows.get_mut(row) {
                    *slot = Some(cells);
                }
            }
            Target::NewRow => {
                if let Some(names) = edit::header_for(table, value) {
                    self.header = Some(names);
                }
                self.appended
                    .push(non_empty(edit::row_texts(table, key, value, None)?));
            }
            Target::NewRowCell { column } => {
                let mut cells = vec![String::new(); column + 1];
                cells[column] = edit::cell_text(key, value)?;
                self.appended.push(cells);
            }
        }
        Ok(())
    }

    /// Removes `target`, as [`edit::remove`] does: a cell becomes empty, a
    /// row disappears.
    pub(crate) fn remove(&mut self, target: Target) {
        match target {
            Target::Cell { row, column } => {
                if let Some(Some(cells)) = self.rows.get_mut(row)
                    && let Some(cell) = cells.get_mut(column)
                {
                    cell.clear();
                }
            }
            Target::Row(row) => {
                if let Some(slot) = self.rows.get_mut(row) {
                    *slot = None;
                }
            }
            Target::Table | Target::NewRow | Target::NewRowCell { .. } => {}
        }
    }

    /// Value of the table after the change.
    pub(crate) fn value(&self) -> Value {
        let rows = self
            .rows
            .iter()
            .flatten()
            .chain(&self.appended)
            .map(Vec::as_slice);
        rows_value(self.header.as_deref(), self.labelled, rows)
    }
}

/// A row without cells is written as one empty cell.
fn non_empty(cells: Vec<String>) -> Vec<String> {
    if cells.is_empty() {
        vec![String::new()]
    } else {
        cells
    }
}

/// Value of data rows: maps by column label when the table has a header
/// (`labelled`), lists otherwise.
///
/// Columns are labelled by [`label_of`]; a row shorter than the header has
/// empty cells for the missing columns.
pub(crate) fn rows_value<'a>(
    header: Option<&[String]>,
    labelled: bool,
    rows: impl Iterator<Item = &'a [String]>,
) -> Value {
    let names = header.unwrap_or_default();
    Value::List(
        rows.map(|cells| {
            if !labelled {
                return Value::List(cells.iter().cloned().map(Value::Text).collect());
            }
            let width = names.len().max(cells.len());
            let mut map = IndexMap::new();
            for index in 0..width {
                map.entry(label_of(names, index))
                    .or_insert_with(|| Value::Text(cells.get(index).cloned().unwrap_or_default()));
            }
            Value::Map(map)
        })
        .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str, header: bool) -> Table {
        Table::parse(text.as_bytes(), header, None, b',').unwrap()
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn matches_the_table_value_before_a_change() {
        for (source, header) in [
            ("a,b\n1\n1,2,3\n", true),
            ("a,a\n1,2\n", true),
            ("x,y\n1\n", false),
            ("", true),
        ] {
            let table = table(source, header);
            assert_eq!(Expected::new(&table).value(), table.to_value(), "{source}");
        }
    }

    #[test]
    fn follows_cell_row_and_append_changes() {
        let table = table("a,b\n1,2\n3,4\n", true);
        let key = KeyPath::root();
        let mut expected = Expected::new(&table);
        expected
            .set(&table, &key, Target::Cell { row: 0, column: 3 }, &text("x"))
            .unwrap();
        expected
            .set(&table, &key, Target::Row(1), &Value::List(Vec::new()))
            .unwrap();
        expected
            .set(&table, &key, Target::NewRowCell { column: 1 }, &text("y"))
            .unwrap();
        let Value::List(rows) = expected.value() else {
            panic!("list expected")
        };
        let Value::Map(first) = &rows[0] else {
            panic!("map expected")
        };
        assert_eq!(first.get("3"), Some(&text("x")));
        assert_eq!(first.get("2"), Some(&text("")));
        let Value::Map(second) = &rows[1] else {
            panic!("map expected")
        };
        assert_eq!(second.get("a"), Some(&text("")));
        let Value::Map(third) = &rows[2] else {
            panic!("map expected")
        };
        assert_eq!(third.get("b"), Some(&text("y")));
    }

    #[test]
    fn follows_removals_and_new_headers() {
        let parsed = table("a,b\n1,2\n3,4\n", true);
        let mut expected = Expected::new(&parsed);
        expected.remove(Target::Row(0));
        expected.remove(Target::Cell { row: 1, column: 0 });
        expected.remove(Target::Cell { row: 1, column: 9 });
        expected.remove(Target::NewRow);
        let Value::List(rows) = expected.value() else {
            panic!("list expected")
        };
        assert_eq!(rows.len(), 1);
        let empty = table("", true);
        let mut expected = Expected::new(&empty);
        let mut fields = IndexMap::new();
        fields.insert("id".to_owned(), Value::Int(1));
        expected
            .set(
                &empty,
                &KeyPath::root(),
                Target::NewRow,
                &Value::Map(fields),
            )
            .unwrap();
        let Value::List(rows) = expected.value() else {
            panic!("list expected")
        };
        assert_eq!(
            rows[0],
            Value::Map(IndexMap::from([("id".to_owned(), text("1"))]))
        );
    }
}
