// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! CSV and TSV documents: identity of every corpus file, reading compared
//! with the `csv` crate, and changes of single cells and rows that leave
//! every other value and byte of the file as it was.
//!
//! The corpus is `tests/d5/corpus/csv`: RFC 4180 quoting, fields over
//! several lines, every separator, CRLF, byte order marks, files without a
//! header and without a final line break.

// Helper functions of the tests report failures by unwrapping and
// panicking, as the test functions themselves do.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{self, Value};
use nurt_doc::formats::csv::{CsvChange, CsvDocument, CsvOptions};
use nurt_doc::registry::{DocOptions, Registry};

/// Rows of every file whose cells are changed one by one.
const ROWS_CHANGED: usize = 12;

/// Byte order mark of UTF-8.
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Cell text that needs quoting in a comma-separated file.
const QUOTED_TEXT: &str = "x,y \"z\"";

fn corpus() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/corpus/csv")
}

fn files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| matches!(extension(path).as_str(), "csv" | "tsv" | "tab"))
        .collect();
    files.sort();
    files
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn format_of(path: &Path) -> DocFormatId {
    if extension(path) == "csv" {
        DocFormatId::CSV
    } else {
        DocFormatId::TSV
    }
}

fn parse(bytes: &[u8], format: &DocFormatId, header: bool) -> CsvDocument {
    CsvDocument::parse(
        bytes,
        format,
        &CsvOptions {
            header,
            delimiter: None,
        },
    )
    .unwrap()
}

fn reparse(bytes: &[u8], original: &CsvDocument, header: bool) -> CsvDocument {
    let options = CsvOptions {
        header,
        delimiter: Some(original.delimiter()),
    };
    CsvDocument::parse(bytes, original.format(), &options).unwrap()
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn rows(value: &Value) -> &[Value] {
    let Value::List(rows) = value else {
        panic!("list of rows expected")
    };
    rows
}

fn cells(row: &Value) -> &[Value] {
    let Value::List(cells) = row else {
        panic!("list of cells expected")
    };
    cells
}

fn set_cell(table: &mut Value, row: usize, column: usize, cell: &str) {
    let Value::List(rows) = table else {
        panic!("list of rows expected")
    };
    let Value::List(cells) = &mut rows[row] else {
        panic!("list of cells expected")
    };
    cells[column] = text(cell);
}

/// Table read by the `csv` crate in flexible mode with the given dialect.
fn reference(bytes: &[u8], delimiter: char, quote: char) -> Value {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(u8::try_from(delimiter).unwrap())
        .quote(u8::try_from(quote).unwrap())
        .from_reader(bytes);
    Value::List(
        reader
            .records()
            .map(|record| Value::List(record.unwrap().iter().map(text).collect()))
            .collect(),
    )
}

/// Bytes before the first and after the last edit of a change are the
/// bytes of the source.
fn assert_outside_unchanged(source: &[u8], change: &CsvChange, after: &[u8], name: &str) {
    let first = change.edits.first().map_or(0, |edit| edit.range.start);
    let last = change
        .edits
        .last()
        .map_or(source.len(), |edit| edit.range.end);
    assert_eq!(
        &after[..first],
        &source[..first],
        "{name}: bytes before the change"
    );
    let tail = source.len() - last;
    assert_eq!(
        &after[after.len() - tail..],
        &source[last..],
        "{name}: bytes after the change"
    );
}

#[test]
fn corpus_has_at_least_forty_files() {
    assert!(files().len() >= 40, "{} files", files().len());
}

#[test]
fn every_file_is_detected_and_opened_with_its_bytes() {
    let registry = Registry::builtin();
    for path in files() {
        let bytes = std::fs::read(&path).unwrap();
        let detected = registry.detect(Some(&path), &bytes);
        assert_eq!(detected, Some(format_of(&path)), "{}", path.display());
        let document = registry
            .open(&format_of(&path), &bytes, &DocOptions::default())
            .unwrap();
        assert_eq!(document.bytes(), bytes, "{}", path.display());
    }
}

#[test]
fn reading_matches_the_csv_crate_on_every_file() {
    for path in files() {
        let bytes = std::fs::read(&path).unwrap();
        let document = parse(&bytes, &format_of(&path), false);
        let expected = reference(&bytes, document.delimiter(), document.quote());
        assert_eq!(document.to_value(), expected, "{}", path.display());
    }
}

#[test]
fn changing_any_cell_changes_only_that_cell() {
    for path in files() {
        let name = path.display().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let document = parse(&bytes, &format_of(&path), false);
        let original = document.to_value();
        for (row, record) in rows(&original).iter().enumerate().take(ROWS_CHANGED) {
            for column in 0..cells(record).len() {
                let change = document
                    .set(&key(&format!("[{row}][{column}]")), &text(QUOTED_TEXT))
                    .unwrap();
                assert_eq!(change.edits.len(), 1, "{name} [{row}][{column}]");
                let after = change.apply(&bytes);
                assert_outside_unchanged(&bytes, &change, &after, &name);
                let mut expected = original.clone();
                set_cell(&mut expected, row, column, QUOTED_TEXT);
                assert_eq!(
                    reparse(&after, &document, false).to_value(),
                    expected,
                    "{name} [{row}][{column}]"
                );
            }
        }
    }
}

#[test]
fn setting_the_current_value_changes_nothing() {
    for path in files() {
        let bytes = std::fs::read(&path).unwrap();
        let document = parse(&bytes, &format_of(&path), false);
        for (row, record) in rows(&document.to_value()).iter().enumerate() {
            for (column, cell) in cells(record).iter().enumerate() {
                let change = document
                    .set(&key(&format!("[{row}][{column}]")), cell)
                    .unwrap();
                assert!(change.is_empty(), "{} [{row}][{column}]", path.display());
            }
            let change = document.set(&key(&format!("[{row}]")), record).unwrap();
            assert!(change.is_empty(), "{} [{row}]", path.display());
        }
    }
}

#[test]
fn removing_each_row_removes_only_that_row() {
    for path in files() {
        let name = path.display().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let document = parse(&bytes, &format_of(&path), false);
        let original = rows(&document.to_value()).to_vec();
        for row in 0..original.len() {
            let change = document.remove(&key(&format!("[{row}]"))).unwrap();
            let after = change.apply(&bytes);
            assert_outside_unchanged(&bytes, &change, &after, &name);
            let mut expected = original.clone();
            expected.remove(row);
            assert_eq!(
                reparse(&after, &document, false).to_value(),
                Value::List(expected),
                "{name} [{row}]"
            );
        }
    }
}

#[test]
fn removing_a_cell_empties_it() {
    let source = b"id,name,city\n1,Ann,Oslo\n2,Bo,Rome\n";
    let document = parse(source, &DocFormatId::CSV, true);
    let change = document.remove(&key("[*].name")).unwrap();
    assert_eq!(change.apply(source), b"id,name,city\n1,,Oslo\n2,,Rome\n");
    let change = document.remove(&key("[0][*]")).unwrap();
    assert_eq!(change.apply(source), b"id,name,city\n,,\n2,Bo,Rome\n");
}

#[test]
fn appended_rows_follow_the_style_of_the_file() {
    let row = Value::List(vec![text("9"), text("new, row")]);
    let cases: [(&str, &[u8]); 6] = [
        ("a,b\r\n1,2\r\n", b"a,b\r\n1,2\r\n9,\"new, row\"\r\n"),
        (
            "\u{feff}a;b\n1;2\n",
            "\u{feff}a;b\n1;2\n9;new, row\n".as_bytes(),
        ),
        ("a,b\n1,2", b"a,b\n1,2\n9,\"new, row\""),
        (
            "\"a\",\"b\"\n\"1\",\"2\"\n",
            b"\"a\",\"b\"\n\"1\",\"2\"\n\"9\",\"new, row\"\n",
        ),
        ("a|b\n1|2\n\n", b"a|b\n1|2\n9|new, row\n\n"),
        ("'a,b',c\n'd',e\n", b"'a,b',c\n'd',e\n9,'new, row'\n"),
    ];
    for (source, expected) in cases {
        let document = parse(source.as_bytes(), &DocFormatId::CSV, true);
        let change = document.set(&key("[+]"), &row).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&change.apply(source.as_bytes())),
            String::from_utf8_lossy(expected),
            "{source:?}"
        );
    }
    let tsv = b"a\tb\n1\t2\n";
    let document = parse(tsv, &DocFormatId::TSV, true);
    let change = document.set(&key("[1]"), &row).unwrap();
    assert_eq!(change.apply(tsv), b"a\tb\n1\t2\n9\tnew, row\n");
}

#[test]
fn keys_address_rows_cells_and_columns() {
    let source = b"id,name\n1,Ann\n2,Bo\n3,Cy\n";
    let document = parse(source, &DocFormatId::CSV, true);
    assert_eq!(document.columns(), ["id", "name"]);
    assert_eq!(
        document.read(&key("[0].name")).unwrap()[0].value,
        text("Ann")
    );
    assert_eq!(document.read(&key("[-1][0]")).unwrap()[0].value, text("3"));
    let ids: Vec<Value> = document
        .read(&key("[*].id"))
        .unwrap()
        .into_iter()
        .map(|item| item.value)
        .collect();
    assert_eq!(ids, [text("1"), text("2"), text("3")]);
    let filtered = document.read(&key("[?id == \"2\"].name")).unwrap();
    assert_eq!(filtered[0].value, text("Bo"));
    assert_eq!(filtered[0].key.to_string(), "[1].name");
    let row = document.read(&key("[1]")).unwrap();
    let Value::Map(fields) = &row[0].value else {
        panic!("map expected")
    };
    assert_eq!(fields.keys().collect::<Vec<_>>(), ["id", "name"]);
    assert!(row[0].span.is_some());
}

#[test]
fn files_without_a_header_are_addressed_by_index() {
    let source = b"1,2,3\n4,5,6\n";
    let document = parse(source, &DocFormatId::CSV, false);
    assert!(document.columns().is_empty());
    assert_eq!(document.read(&key("[1][2]")).unwrap()[0].value, text("6"));
    let error = document.read(&key("[0].name")).unwrap_err();
    assert_eq!(error.code().as_str(), "E_KEY_NOT_FOUND");
    assert_eq!(rows(&document.to_value()).len(), 2);
}

#[test]
fn rows_are_records_with_cells_in_column_order() {
    let source = b"b,a\n1,2\n3\n";
    let document = parse(source, &DocFormatId::CSV, true);
    let records = document.rows(&value::Path::new("data.csv"));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].no, 1);
    assert_eq!(records[0].cells.keys().collect::<Vec<_>>(), ["b", "a"]);
    assert_eq!(records[1].cells.get("a"), Some(&text("")));
}

#[test]
fn explicit_separator_overrides_detection() {
    let source = b"a;b,c\n1;2,3\n";
    let options = CsvOptions {
        header: true,
        delimiter: Some(';'),
    };
    let document = CsvDocument::parse(source, &DocFormatId::CSV, &options).unwrap();
    assert_eq!(document.delimiter(), ';');
    assert_eq!(document.read(&key("[0][1]")).unwrap()[0].value, text("2,3"));
    let options = CsvOptions {
        header: true,
        delimiter: Some('"'),
    };
    let error = CsvDocument::parse(source, &DocFormatId::CSV, &options).unwrap_err();
    assert_eq!(error.code().as_str(), "E_INVALID_VALUE");
}

#[test]
fn invalid_keys_values_and_text_are_rejected() {
    let source = b"id,name\n1,Ann\n";
    let document = parse(source, &DocFormatId::CSV, true);
    let cases = [
        (document.read(&key("[5]")).unwrap_err(), "E_KEY_NOT_FOUND"),
        (
            document.read(&key("[0].missing")).unwrap_err(),
            "E_KEY_NOT_FOUND",
        ),
        (
            document.read(&key("[0].name.deeper")).unwrap_err(),
            "E_KEY_CONFLICT",
        ),
        (document.read(&key("name")).unwrap_err(), "E_KEY_CONFLICT"),
        (
            document
                .set(&key("[0].name"), &Value::List(Vec::new()))
                .unwrap_err(),
            "E_TYPE",
        ),
        (
            document.set(&KeyPath::root(), &text("x")).unwrap_err(),
            "E_KEY_CONFLICT",
        ),
        (document.remove(&key("[+]")).unwrap_err(), "E_KEY_NOT_FOUND"),
        (
            CsvDocument::parse(b"a,\xFF\n", &DocFormatId::CSV, &CsvOptions::default()).unwrap_err(),
            "E_ENCODING",
        ),
        (
            CsvDocument::parse(b"a\n", &DocFormatId::JSON, &CsvOptions::default()).unwrap_err(),
            "E_INVALID_VALUE",
        ),
    ];
    for (error, code) in cases {
        assert_eq!(error.code().as_str(), code, "{}", error.message);
    }
}

#[test]
fn numbers_and_truth_values_are_written_as_text() {
    let source = b"n,flag\n0,false\n";
    let document = parse(source, &DocFormatId::CSV, true);
    let change = document.set(&key("[0].n"), &Value::Number(2.5)).unwrap();
    assert_eq!(change.apply(source), b"n,flag\n2.5,false\n");
    let change = document.set(&key("[0].flag"), &Value::Bool(true)).unwrap();
    assert_eq!(change.apply(source), b"n,flag\n0,true\n");
    let change = document.set(&key("[0].n"), &Value::Int(0)).unwrap();
    assert!(change.is_empty());
}
