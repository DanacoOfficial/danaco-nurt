// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Table model of a delimited file: header, data rows and the layout facts
//! that new content follows.

use indexmap::IndexMap;
use nurt_contracts::error::Error;
use nurt_contracts::value::Value;

use super::dialect;
use super::errors;
use super::scan::{Dialect, Field, Record, Scan, scan_file};

/// Line terminator written when the file has none.
const DEFAULT_TERMINATOR: &str = "\n";

/// Largest number of columns a change may create past the end of a row; a
/// column index beyond it would make the new text as long as the index.
pub(crate) const MAX_COLUMNS: usize = 16_384;

/// Parsed delimited file with the byte range of every field.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    /// Separator and quote character.
    pub(crate) dialect: Dialect,
    /// The first record names the columns.
    pub(crate) header: bool,
    /// Scanned records.
    pub(crate) scan: Scan,
    /// Terminator for new rows: the most common one in the file.
    pub(crate) terminator: String,
    /// Every field of the file is quoted, so new fields are quoted too.
    pub(crate) quote_all: bool,
}

impl Table {
    /// Parses `bytes` as UTF-8 delimited text, with an optional byte order
    /// mark.
    ///
    /// `delimiter` is an explicit separator; without it the separator is
    /// detected and `default_delimiter` is kept when detection does not
    /// decide.
    pub(crate) fn parse(
        bytes: &[u8],
        header: bool,
        delimiter: Option<u8>,
        default_delimiter: u8,
    ) -> Result<Self, Error> {
        let bom = if bytes.starts_with(super::scan::UTF8_BOM) {
            super::scan::UTF8_BOM.len()
        } else {
            0
        };
        if let Err(error) = std::str::from_utf8(&bytes[bom..]) {
            return Err(errors::not_utf8(bom + error.valid_up_to()));
        }
        let dialect = dialect::detect(&bytes[bom..], delimiter, default_delimiter);
        let scan = scan_file(bytes, dialect);
        let terminator = dominant_terminator(bytes, &scan.records);
        let quote_all = scan
            .records
            .iter()
            .flat_map(|record| &record.fields)
            .all(|field| field.quoted)
            && !scan.records.is_empty();
        Ok(Self {
            dialect,
            header,
            scan,
            terminator,
            quote_all,
        })
    }

    /// Header record, when the table has one.
    pub(crate) fn header_record(&self) -> Option<&Record> {
        if self.header {
            self.scan.records.first()
        } else {
            None
        }
    }

    /// Records holding data, without the header.
    pub(crate) fn rows(&self) -> &[Record] {
        let skip = usize::from(self.header && !self.scan.records.is_empty());
        &self.scan.records[skip..]
    }

    /// Column names from the header, empty without a header.
    pub(crate) fn column_names(&self) -> Vec<&str> {
        self.header_record()
            .map(|record| {
                record
                    .fields
                    .iter()
                    .map(|field| field.value.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Column named `label` by [`Table::column_label`], or the column with
    /// that decimal index past the header, up to [`MAX_COLUMNS`].
    pub(crate) fn label_column(&self, label: &str) -> Option<usize> {
        let names = self.column_names();
        (0..names.len())
            .find(|index| label_of(&names, *index) == label)
            .or_else(|| {
                label
                    .parse()
                    .ok()
                    .filter(|index| *index >= names.len() && *index < MAX_COLUMNS)
            })
    }

    /// Name of column `index` in rows read as maps; see [`label_of`].
    pub(crate) fn column_label(&self, index: usize) -> String {
        label_of(&self.column_names(), index)
    }

    /// Cells of a row by column label, in column order.
    ///
    /// A row shorter than the header has empty cells for the missing
    /// columns.
    pub(crate) fn row_cells(&self, record: &Record) -> IndexMap<String, Value> {
        let width = self.header_record().map_or(0, |header| header.fields.len());
        let mut cells = IndexMap::new();
        for index in 0..width.max(record.fields.len()) {
            let value = record
                .fields
                .get(index)
                .map_or("", |field| field.value.as_str());
            cells
                .entry(self.column_label(index))
                .or_insert_with(|| Value::Text(value.to_owned()));
        }
        cells
    }

    /// Value of a row: a map with a header, a list without one.
    pub(crate) fn row_value(&self, record: &Record) -> Value {
        if self.header {
            Value::Map(self.row_cells(record))
        } else {
            Value::List(record.fields.iter().map(field_value).collect())
        }
    }

    /// Value of the whole table: a list of rows.
    pub(crate) fn to_value(&self) -> Value {
        Value::List(
            self.rows()
                .iter()
                .map(|record| self.row_value(record))
                .collect(),
        )
    }
}

/// Label of column `index` given the header `names`: the header name when it
/// is not empty and no earlier column has it, the decimal index otherwise,
/// so that every column of a row map has its own label.
pub(crate) fn label_of<S: AsRef<str>>(names: &[S], index: usize) -> String {
    match names.get(index).map(AsRef::as_ref) {
        Some(name)
            if !name.is_empty() && !names[..index].iter().any(|other| other.as_ref() == name) =>
        {
            name.to_owned()
        }
        _ => index.to_string(),
    }
}

/// Text value of a field.
pub(crate) fn field_value(field: &Field) -> Value {
    Value::Text(field.value.clone())
}

/// Most common terminator among records; ties go to the first seen.
fn dominant_terminator(bytes: &[u8], records: &[Record]) -> String {
    let mut counts: Vec<(&[u8], usize)> = Vec::new();
    for record in records {
        let terminator = &bytes[record.terminator.clone()];
        if terminator.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(seen, _)| *seen == terminator) {
            Some((_, count)) => *count += 1,
            None => counts.push((terminator, 1)),
        }
    }
    let mut best: Option<(&[u8], usize)> = None;
    for (terminator, count) in counts {
        if best.is_none_or(|(_, best_count)| count > best_count) {
            best = Some((terminator, count));
        }
    }
    best.map_or_else(
        || DEFAULT_TERMINATOR.to_owned(),
        |(terminator, _)| String::from_utf8_lossy(terminator).into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str, header: bool) -> Table {
        Table::parse(text.as_bytes(), header, None, b',').unwrap()
    }

    #[test]
    fn header_names_columns_and_is_not_a_row() {
        let table = table("id,name\n1,ann\n2,bob\n", true);
        assert_eq!(table.column_names(), ["id", "name"]);
        assert_eq!(table.rows().len(), 2);
        assert_eq!(table.label_column("name"), Some(1));
    }

    #[test]
    fn repeated_and_empty_column_names_are_labelled_by_index() {
        let table = table("a,a,,b\n1,2,3,4\n", true);
        let cells = table.row_cells(&table.rows()[0]);
        assert_eq!(cells.keys().collect::<Vec<_>>(), ["a", "1", "2", "b"]);
        assert_eq!(table.label_column("a"), Some(0));
        assert_eq!(table.label_column("1"), Some(1));
        assert_eq!(table.label_column("7"), Some(7));
        assert_eq!(table.label_column("c"), None);
    }

    #[test]
    fn numeric_labels_past_the_header_stop_at_the_column_limit() {
        let table = table("a,b\n1,2\n", true);
        let last = (MAX_COLUMNS - 1).to_string();
        assert_eq!(table.label_column(&last), Some(MAX_COLUMNS - 1));
        assert_eq!(table.label_column(&MAX_COLUMNS.to_string()), None);
        assert_eq!(table.label_column("99999999999"), None);
    }

    #[test]
    fn rows_without_header_are_lists() {
        let table = table("1,ann\n", false);
        assert_eq!(
            table.to_value(),
            Value::List(vec![Value::List(vec![
                Value::Text("1".to_owned()),
                Value::Text("ann".to_owned())
            ])])
        );
    }

    #[test]
    fn short_and_long_rows_fill_and_extend_columns() {
        let table = table("a,b\n1\n1,2,3\n", true);
        let short = table.row_cells(&table.rows()[0]);
        assert_eq!(short.get("b"), Some(&Value::Text(String::new())));
        let long = table.row_cells(&table.rows()[1]);
        assert_eq!(long.get("2"), Some(&Value::Text("3".to_owned())));
    }

    #[test]
    fn dominant_terminator_follows_the_file() {
        assert_eq!(table("a\r\nb\r\nc\n", false).terminator, "\r\n");
        assert_eq!(table("a", false).terminator, "\n");
    }

    #[test]
    fn rejects_text_that_is_not_utf8() {
        let error = Table::parse(b"a,\xFF\n", true, None, b',').unwrap_err();
        assert_eq!(error.code().as_str(), "E_ENCODING");
    }

    #[test]
    fn quote_all_is_detected() {
        assert!(table("\"a\",\"b\"\n\"1\",\"2\"\n", true).quote_all);
        assert!(!table("\"a\",b\n", true).quote_all);
    }
}
