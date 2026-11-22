// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Byte-range changes of rows and cells.
//!
//! A cell is quoted only when its text requires it (separator, quote
//! character or line break) or when every field of the file is quoted, with
//! the quote character of the file. A record with a single empty field is
//! written as `""`, because an empty line is not a record. New rows take the
//! most common line terminator of the file.

use std::ops::Range;

use indexmap::IndexMap;
use nurt_contracts::error::Error;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;

use super::errors;
use super::resolve::Target;
use super::scan::Record;
use super::table::Table;

/// Replacement of a byte range of the original document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Splice {
    /// Replaced bytes of the original document; empty for an insertion.
    pub(crate) range: Range<usize>,
    /// New text.
    pub(crate) text: String,
}

/// Changes that store `value` at `target`.
///
/// # Errors
///
/// Returns `E_TYPE` for a value that cannot be stored there, `E_KEY_NOT_FOUND` for a
/// map with a key that is not a column, `E_KEY_CONFLICT` for the whole
/// table.
pub(crate) fn set(
    table: &Table,
    bytes: &[u8],
    key: &KeyPath,
    target: Target,
    value: &Value,
) -> Result<Vec<Splice>, Error> {
    match target {
        Target::Table => Err(whole_table(key)),
        Target::Cell { row, column } => {
            let text = cell_text(key, value)?;
            Ok(set_cell(table, &table.rows()[row], column, &text)
                .into_iter()
                .collect())
        }
        Target::Row(row) => {
            let record = &table.rows()[row];
            let cells = row_texts(table, key, value, Some(record))?;
            Ok(set_row(table, record, &cells))
        }
        Target::NewRow => {
            let cells = row_texts(table, key, value, None)?;
            Ok(vec![append_row(
                table,
                bytes,
                &cells,
                header_for(table, value),
            )])
        }
        Target::NewRowCell { column } => {
            let text = cell_text(key, value)?;
            let mut cells = vec![String::new(); column + 1];
            cells[column] = text;
            Ok(vec![append_row(table, bytes, &cells, None)])
        }
    }
}

/// Changes that remove `target`: a cell becomes empty, a row is removed with
/// its line terminator.
///
/// # Errors
///
/// Returns `E_KEY_CONFLICT` for the whole table and `E_KEY_NOT_FOUND` for a row that
/// does not exist.
pub(crate) fn remove(
    table: &Table,
    bytes: &[u8],
    key: &KeyPath,
    target: Target,
) -> Result<Vec<Splice>, Error> {
    match target {
        Target::Table => Err(whole_table(key)),
        Target::Cell { row, column } => {
            let record = &table.rows()[row];
            if column >= record.fields.len() {
                return Ok(Vec::new());
            }
            Ok(set_cell(table, record, column, "").into_iter().collect())
        }
        Target::Row(row) => Ok(vec![remove_record(bytes, &table.rows()[row])]),
        Target::NewRow | Target::NewRowCell { .. } => {
            Err(errors::key_not_found(key, "the row does not exist"))
        }
    }
}

fn whole_table(key: &KeyPath) -> Error {
    errors::key_shape(
        key,
        "the key addresses the whole table; set rows with [0] or append with [+]",
    )
}

/// Text of a cell for a scalar value.
pub(crate) fn cell_text(key: &KeyPath, value: &Value) -> Result<String, Error> {
    let text = match value {
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Int(number) => number.to_string(),
        Value::Number(number) => number.to_string(),
        Value::BigInt(text) | Value::Decimal(text) | Value::Text(text) | Value::Path(text) => {
            text.clone()
        }
        Value::Glob(text) | Value::Key(text) | Value::Enum(text) => text.clone(),
        Value::Date(date) => date.text.clone(),
        Value::Instant(instant) => instant.to_string(),
        Value::Size(size) => size.bytes().to_string(),
        Value::Duration(duration) => duration.as_millis().to_string(),
        other => return Err(errors::value_type(key, "a scalar", other.type_name())),
    };
    Ok(text)
}

/// Cell texts of a row value: a map by column, a list by position or a
/// scalar as a single cell. `current` is the row the value replaces, `None`
/// for a new row.
pub(crate) fn row_texts(
    table: &Table,
    key: &KeyPath,
    value: &Value,
    current: Option<&Record>,
) -> Result<Vec<String>, Error> {
    match value {
        Value::List(items) => items.iter().map(|item| cell_text(key, item)).collect(),
        Value::Map(fields) => map_texts(table, key, fields, current),
        Value::Record(record) => {
            let fields: IndexMap<String, Value> = record
                .field_names()
                .filter_map(|name| Some((name.to_owned(), record.get(name)?.clone())))
                .collect();
            map_texts(table, key, &fields, current)
        }
        scalar => Ok(vec![cell_text(key, scalar)?]),
    }
}

/// Cells of a map in column order, by the labels of
/// [`Table::column_label`]; a table without rows and without a header row
/// takes the map keys as its header.
///
/// Empty cells past the end of the `current` row are not written, so a row
/// shorter than the header keeps its length when its own value is stored.
fn map_texts(
    table: &Table,
    key: &KeyPath,
    fields: &IndexMap<String, Value>,
    current: Option<&Record>,
) -> Result<Vec<String>, Error> {
    if table.header && table.scan.records.is_empty() {
        return fields.values().map(|value| cell_text(key, value)).collect();
    }
    let mut cells: Vec<String> = vec![String::new(); table.column_names().len()];
    for (name, value) in fields {
        let column = table.label_column(name).ok_or_else(|| {
            let columns = table.column_names().join(", ");
            errors::key_not_found(key, &format!("no column {name:?}; columns: {columns}"))
        })?;
        if column >= cells.len() {
            cells.resize(column + 1, String::new());
        }
        cells[column] = cell_text(key, value)?;
    }
    if let Some(record) = current {
        let kept = record.fields.len().max(1);
        while cells.len() > kept && cells.last().is_some_and(String::is_empty) {
            cells.pop();
        }
    }
    Ok(cells)
}

/// Header row to write before the first row of an empty table read with a
/// header.
pub(crate) fn header_for(table: &Table, value: &Value) -> Option<Vec<String>> {
    if !table.header || !table.scan.records.is_empty() {
        return None;
    }
    match value {
        Value::Map(fields) => Some(fields.keys().cloned().collect()),
        Value::Record(record) => Some(record.field_names().map(str::to_owned).collect()),
        _ => None,
    }
}

/// Change of one cell, or nothing when the cell already holds `text`.
fn set_cell(table: &Table, record: &Record, column: usize, text: &str) -> Option<Splice> {
    let sole = record.fields.len() == 1 && column == 0;
    if let Some(field) = record.fields.get(column) {
        let unchanged = field.value == text && !(sole && text.is_empty() && !field.quoted);
        if unchanged {
            return None;
        }
        return Some(Splice {
            range: field.raw.clone(),
            text: encode(table, text, sole),
        });
    }
    let delimiter = char::from(table.dialect.delimiter);
    let mut inserted = String::new();
    for _ in record.fields.len()..column {
        inserted.push(delimiter);
        inserted.push_str(&encode(table, "", false));
    }
    inserted.push(delimiter);
    inserted.push_str(&encode(table, text, false));
    Some(Splice {
        range: record.span.end..record.span.end,
        text: inserted,
    })
}

/// Changes that make a row hold exactly `cells`.
fn set_row(table: &Table, record: &Record, cells: &[String]) -> Vec<Splice> {
    let cells: Vec<String> = if cells.is_empty() {
        vec![String::new()]
    } else {
        cells.to_vec()
    };
    let old = record.fields.len();
    let mut splices: Vec<Splice> = Vec::new();
    for (column, text) in cells.iter().enumerate().take(old) {
        let sole = cells.len() == 1;
        let field = &record.fields[column];
        let unchanged = field.value == *text && !(sole && text.is_empty() && !field.quoted);
        if !unchanged {
            splices.push(Splice {
                range: field.raw.clone(),
                text: encode(table, text, sole),
            });
        }
    }
    let delimiter = char::from(table.dialect.delimiter);
    if cells.len() > old {
        let mut inserted = String::new();
        for text in &cells[old..] {
            inserted.push(delimiter);
            inserted.push_str(&encode(table, text, false));
        }
        splices.push(Splice {
            range: record.span.end..record.span.end,
            text: inserted,
        });
    } else if cells.len() < old {
        let start = record.fields[cells.len() - 1].raw.end;
        splices.push(Splice {
            range: start..record.span.end,
            text: String::new(),
        });
    }
    splices
}

/// Insertion of a row after the last record, before trailing blank lines.
fn append_row(
    table: &Table,
    bytes: &[u8],
    cells: &[String],
    header: Option<Vec<String>>,
) -> Splice {
    let mut text = String::new();
    if let Some(names) = header {
        text.push_str(&row_text(table, &names));
        text.push_str(&table.terminator);
    }
    text.push_str(&row_text(table, cells));
    match table.scan.records.last() {
        Some(last) if last.terminator.is_empty() => Splice {
            range: last.span.end..last.span.end,
            text: format!("{}{text}", table.terminator),
        },
        Some(last) => {
            text.push_str(&table.terminator);
            Splice {
                range: last.terminator.end..last.terminator.end,
                text,
            }
        }
        None => {
            text.push_str(&table.terminator);
            let start = table.scan.bom.min(bytes.len());
            Splice {
                range: start..start,
                text,
            }
        }
    }
}

/// Text of a whole row without its terminator.
fn row_text(table: &Table, cells: &[String]) -> String {
    let sole = cells.len() <= 1;
    let delimiter = char::from(table.dialect.delimiter).to_string();
    if cells.is_empty() {
        return encode(table, "", true);
    }
    cells
        .iter()
        .map(|text| encode(table, text, sole))
        .collect::<Vec<_>>()
        .join(&delimiter)
}

/// Removal of a record with its line terminator; the last record of a file
/// without a final line break takes the line break before it.
fn remove_record(bytes: &[u8], record: &Record) -> Splice {
    if !record.terminator.is_empty() {
        return Splice {
            range: record.span.start..record.terminator.end,
            text: String::new(),
        };
    }
    let start = record.span.start;
    let before = match start.checked_sub(1).map(|index| bytes[index]) {
        Some(b'\n') if start >= 2 && bytes[start - 2] == b'\r' => 2,
        Some(b'\n' | b'\r') => 1,
        _ => 0,
    };
    Splice {
        range: start - before..record.span.end,
        text: String::new(),
    }
}

/// Field text for `value`, quoted when required or when the file quotes
/// every field. `sole` marks the only field of a record, which is quoted
/// when empty.
pub(crate) fn encode(table: &Table, value: &str, sole: bool) -> String {
    let quote = table.dialect.quote;
    let delimiter = table.dialect.delimiter;
    let required = value
        .bytes()
        .any(|byte| byte == delimiter || byte == quote || byte == b'\r' || byte == b'\n');
    if !(required || table.quote_all || (sole && value.is_empty())) {
        return value.to_owned();
    }
    let quote = char::from(quote);
    let mut text = String::with_capacity(value.len() + 2);
    text.push(quote);
    for character in value.chars() {
        if character == quote {
            text.push(quote);
        }
        text.push(character);
    }
    text.push(quote);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(bytes: &str, splices: &[Splice]) -> String {
        let mut sorted = splices.to_vec();
        sorted.sort_by_key(|splice| std::cmp::Reverse(splice.range.start));
        let mut text = bytes.to_owned();
        for splice in sorted {
            text.replace_range(splice.range, &splice.text);
        }
        text
    }

    fn table(text: &str) -> Table {
        Table::parse(text.as_bytes(), true, None, b',').unwrap()
    }

    fn key() -> KeyPath {
        KeyPath::root()
    }

    #[test]
    fn cell_is_quoted_only_when_required() {
        let text = "a,b\n1,\"x\"\n";
        let table = table(text);
        let target = Target::Cell { row: 0, column: 1 };
        let plain = set(
            &table,
            text.as_bytes(),
            &key(),
            target,
            &Value::Text("y".into()),
        )
        .unwrap();
        assert_eq!(apply(text, &plain), "a,b\n1,y\n");
        let comma = set(
            &table,
            text.as_bytes(),
            &key(),
            target,
            &Value::Text("p,q".into()),
        )
        .unwrap();
        assert_eq!(apply(text, &comma), "a,b\n1,\"p,q\"\n");
        let quote = set(
            &table,
            text.as_bytes(),
            &key(),
            target,
            &Value::Text("say \"hi\"".into()),
        )
        .unwrap();
        assert_eq!(apply(text, &quote), "a,b\n1,\"say \"\"hi\"\"\"\n");
    }

    #[test]
    fn same_value_changes_nothing() {
        let text = "a,b\n1,\"x\"\n";
        let table = table(text);
        let target = Target::Cell { row: 0, column: 1 };
        assert!(
            set(
                &table,
                text.as_bytes(),
                &key(),
                target,
                &Value::Text("x".into())
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn cell_past_the_end_of_a_short_row_extends_it() {
        let text = "a,b,c\n1\n";
        let table = table(text);
        let target = Target::Cell { row: 0, column: 2 };
        let splices = set(&table, text.as_bytes(), &key(), target, &Value::Int(3)).unwrap();
        assert_eq!(apply(text, &splices), "a,b,c\n1,,3\n");
    }

    #[test]
    fn appended_row_follows_terminator_and_final_line_break() {
        let crlf = "a,b\r\n1,2\r\n\r\n";
        let table_crlf = table(crlf);
        let row = Value::List(vec![Value::Int(3), Value::Text("x y".into())]);
        let splices = set(&table_crlf, crlf.as_bytes(), &key(), Target::NewRow, &row).unwrap();
        assert_eq!(apply(crlf, &splices), "a,b\r\n1,2\r\n3,x y\r\n\r\n");
        let open = "a,b\n1,2";
        let table_open = table(open);
        let splices = set(&table_open, open.as_bytes(), &key(), Target::NewRow, &row).unwrap();
        assert_eq!(apply(open, &splices), "a,b\n1,2\n3,x y");
    }

    #[test]
    fn row_map_is_written_in_column_order() {
        let text = "a,b\n1,2\n";
        let table = table(text);
        let mut fields = IndexMap::new();
        fields.insert("b".to_owned(), Value::Int(5));
        fields.insert("a".to_owned(), Value::Int(4));
        let splices = set(
            &table,
            text.as_bytes(),
            &key(),
            Target::NewRow,
            &Value::Map(fields),
        )
        .unwrap();
        assert_eq!(apply(text, &splices), "a,b\n1,2\n4,5\n");
    }

    #[test]
    fn row_map_of_the_current_value_changes_nothing() {
        for text in ["a,b,c\n1\n1,2\n1,2,3,4\n", "a,a,,b\n1,2,3,4\n1\n"] {
            let table = table(text);
            for (row, record) in table.rows().iter().enumerate() {
                let value = table.row_value(record);
                let splices =
                    set(&table, text.as_bytes(), &key(), Target::Row(row), &value).unwrap();
                assert!(splices.is_empty(), "{text:?} [{row}]: {splices:?}");
            }
        }
    }

    #[test]
    fn map_with_unknown_column_is_rejected() {
        let text = "a,b\n1,2\n";
        let table = table(text);
        let mut fields = IndexMap::new();
        fields.insert("c".to_owned(), Value::Int(5));
        let error = set(
            &table,
            text.as_bytes(),
            &key(),
            Target::Row(0),
            &Value::Map(fields),
        )
        .unwrap_err();
        assert_eq!(error.code().as_str(), "E_KEY_NOT_FOUND");
    }

    #[test]
    fn empty_table_with_header_gets_header_from_map() {
        let text = "";
        let table = table(text);
        let mut fields = IndexMap::new();
        fields.insert("id".to_owned(), Value::Int(1));
        let splices = set(
            &table,
            text.as_bytes(),
            &key(),
            Target::NewRow,
            &Value::Map(fields),
        )
        .unwrap();
        assert_eq!(apply(text, &splices), "id\n1\n");
    }

    #[test]
    fn removing_rows_takes_their_terminator() {
        let text = "a\n1\n2";
        let table = table(text);
        let first = remove(&table, text.as_bytes(), &key(), Target::Row(0)).unwrap();
        assert_eq!(apply(text, &first), "a\n2");
        let last = remove(&table, text.as_bytes(), &key(), Target::Row(1)).unwrap();
        assert_eq!(apply(text, &last), "a\n1");
    }

    #[test]
    fn removing_a_cell_empties_it_and_sole_cell_stays_a_record() {
        let text = "a,b\n1,2\nx\n";
        let table = table(text);
        let cell = remove(
            &table,
            text.as_bytes(),
            &key(),
            Target::Cell { row: 0, column: 0 },
        )
        .unwrap();
        assert_eq!(apply(text, &cell), "a,b\n,2\nx\n");
        let sole = remove(
            &table,
            text.as_bytes(),
            &key(),
            Target::Cell { row: 1, column: 0 },
        )
        .unwrap();
        assert_eq!(apply(text, &sole), "a,b\n1,2\n\"\"\n");
    }

    #[test]
    fn row_list_shrinks_and_grows_the_row() {
        let text = "a,b,c\n1,2,3\n";
        let table = table(text);
        let shorter = Value::List(vec![Value::Int(1)]);
        let splices = set(&table, text.as_bytes(), &key(), Target::Row(0), &shorter).unwrap();
        assert_eq!(apply(text, &splices), "a,b,c\n1\n");
        let longer = Value::List((1..=4).map(Value::Int).collect());
        let splices = set(&table, text.as_bytes(), &key(), Target::Row(0), &longer).unwrap();
        assert_eq!(apply(text, &splices), "a,b,c\n1,2,3,4\n");
    }

    #[test]
    fn quote_all_files_quote_new_cells() {
        let text = "\"a\",\"b\"\n\"1\",\"2\"\n";
        let table = table(text);
        let splices = set(
            &table,
            text.as_bytes(),
            &key(),
            Target::Cell { row: 0, column: 0 },
            &Value::Int(7),
        )
        .unwrap();
        assert_eq!(apply(text, &splices), "\"a\",\"b\"\n\"7\",\"2\"\n");
    }

    #[test]
    fn whole_table_and_nested_values_are_rejected() {
        let text = "a\n1\n";
        let table = table(text);
        let error = set(&table, text.as_bytes(), &key(), Target::Table, &Value::Null).unwrap_err();
        assert_eq!(error.code().as_str(), "E_KEY_CONFLICT");
        let nested = Value::List(vec![]);
        let error = set(
            &table,
            text.as_bytes(),
            &key(),
            Target::Cell { row: 0, column: 0 },
            &nested,
        )
        .unwrap_err();
        assert_eq!(error.code().as_str(), "E_TYPE");
    }
}
