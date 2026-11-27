// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::csv::scan
// Responsibility:   Lossless scanner of delimited text that records the byte range of every.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Lossless scanner of delimited text that records the byte range of every
//! field, record and line terminator.
//!
//! The scanner accepts exactly the input the `csv` crate accepts with
//! `flexible(true)`, `double_quote(true)`, no escape character and the
//! default CRLF terminator, and it yields the same field values: blank lines
//! are skipped, a quote opens a quoted field only at the start of a field,
//! `""` inside a quoted field stands for one quote, and bytes after the
//! closing quote up to the next delimiter belong to the field.

use std::ops::Range;

/// Byte order mark of UTF-8.
pub(crate) const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Field separator and quote character of a delimited file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Dialect {
    /// Byte separating fields.
    pub(crate) delimiter: u8,
    /// Byte enclosing quoted fields.
    pub(crate) quote: u8,
}

/// Field of a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// Bytes of the field as written, including quotes.
    pub(crate) raw: Range<usize>,
    /// The field starts with the quote character.
    pub(crate) quoted: bool,
    /// The field is quoted but has bytes after the closing quote, or the
    /// closing quote is missing.
    pub(crate) irregular: bool,
    /// Field value after removing quotes and doubled quotes.
    pub(crate) value: String,
}

/// Record (one row) of a delimited file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    /// Bytes from the first field to the end of the last field.
    pub(crate) span: Range<usize>,
    /// Fields in order; a record has at least one field.
    pub(crate) fields: Vec<Field>,
    /// Line terminator after the record: `\r\n`, `\n`, `\r` or empty at the
    /// end of the input.
    pub(crate) terminator: Range<usize>,
}

/// Result of scanning a delimited file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scan {
    /// Length of the byte order mark at the start, 0 or 3.
    pub(crate) bom: usize,
    /// Records in order; blank lines are not records.
    pub(crate) records: Vec<Record>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    StartRecord,
    StartField,
    InField,
    InQuoted,
    QuoteInQuoted,
}

struct Scanner<'a> {
    text: &'a [u8],
    dialect: Dialect,
    state: State,
    records: Vec<Record>,
    fields: Vec<Field>,
    record_start: usize,
    field_start: usize,
    value: Vec<u8>,
    quoted: bool,
    irregular: bool,
}

/// Scans `text`, which must not include a byte order mark.
///
/// Field values are UTF-8 because the caller has validated the input as
/// UTF-8 and the scanner splits only at ASCII bytes.
pub(crate) fn scan(text: &[u8], dialect: Dialect) -> Vec<Record> {
    let mut scanner = Scanner {
        text,
        dialect,
        state: State::StartRecord,
        records: Vec::new(),
        fields: Vec::new(),
        record_start: 0,
        field_start: 0,
        value: Vec::new(),
        quoted: false,
        irregular: false,
    };
    let mut position = 0;
    while position < text.len() {
        position = scanner.step(position);
    }
    scanner.finish();
    scanner.records
}

impl Scanner<'_> {
    /// Consumes the byte at `position` and returns the next position.
    fn step(&mut self, position: usize) -> usize {
        let byte = self.text[position];
        match self.state {
            State::StartRecord => {
                if is_line_break(byte) {
                    return position + 1;
                }
                self.record_start = position;
                self.start_field(position);
                self.step(position)
            }
            State::StartField => {
                if byte == self.dialect.quote {
                    self.quoted = true;
                    self.state = State::InQuoted;
                    position + 1
                } else {
                    self.state = State::InField;
                    self.step(position)
                }
            }
            State::InField => self.unquoted_byte(position, byte),
            State::InQuoted => {
                if byte == self.dialect.quote {
                    self.state = State::QuoteInQuoted;
                } else {
                    self.value.push(byte);
                }
                position + 1
            }
            State::QuoteInQuoted => {
                if byte == self.dialect.quote {
                    self.value.push(byte);
                    self.state = State::InQuoted;
                    position + 1
                } else {
                    if byte != self.dialect.delimiter && !is_line_break(byte) {
                        self.irregular = true;
                    }
                    self.state = State::InField;
                    self.unquoted_byte(position, byte)
                }
            }
        }
    }

    fn unquoted_byte(&mut self, position: usize, byte: u8) -> usize {
        if byte == self.dialect.delimiter {
            self.end_field(position);
            self.start_field(position + 1);
            position + 1
        } else if is_line_break(byte) {
            self.end_field(position);
            let end = if byte == b'\r' && self.text.get(position + 1) == Some(&b'\n') {
                position + 2
            } else {
                position + 1
            };
            self.end_record(position, end);
            end
        } else {
            self.value.push(byte);
            position + 1
        }
    }

    fn start_field(&mut self, position: usize) {
        self.field_start = position;
        self.value.clear();
        self.quoted = false;
        self.irregular = false;
        self.state = State::StartField;
    }

    fn end_field(&mut self, position: usize) {
        self.fields.push(Field {
            raw: self.field_start..position,
            quoted: self.quoted,
            irregular: self.irregular,
            value: String::from_utf8_lossy(&self.value).into_owned(),
        });
    }

    fn end_record(&mut self, content_end: usize, terminator_end: usize) {
        self.records.push(Record {
            span: self.record_start..content_end,
            fields: std::mem::take(&mut self.fields),
            terminator: content_end..terminator_end,
        });
        self.state = State::StartRecord;
    }

    fn finish(&mut self) {
        let end = self.text.len();
        match self.state {
            State::StartRecord => {}
            State::InQuoted => {
                self.irregular = true;
                self.end_field(end);
                self.end_record(end, end);
            }
            State::StartField | State::InField | State::QuoteInQuoted => {
                self.end_field(end);
                self.end_record(end, end);
            }
        }
    }
}

const fn is_line_break(byte: u8) -> bool {
    byte == b'\r' || byte == b'\n'
}

/// Scans a whole file, skipping a leading UTF-8 byte order mark; ranges are
/// offsets into `bytes`.
pub(crate) fn scan_file(bytes: &[u8], dialect: Dialect) -> Scan {
    let bom = if bytes.starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    };
    let mut records = scan(&bytes[bom..], dialect);
    if bom > 0 {
        for record in &mut records {
            shift_record(record, bom);
        }
    }
    Scan { bom, records }
}

fn shift_record(record: &mut Record, offset: usize) {
    record.span = shift(&record.span, offset);
    record.terminator = shift(&record.terminator, offset);
    for field in &mut record.fields {
        field.raw = shift(&field.raw, offset);
    }
}

fn shift(range: &Range<usize>, offset: usize) -> Range<usize> {
    range.start + offset..range.end + offset
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMA: Dialect = Dialect {
        delimiter: b',',
        quote: b'"',
    };

    fn values(text: &str) -> Vec<Vec<String>> {
        scan(text.as_bytes(), COMMA)
            .into_iter()
            .map(|record| record.fields.into_iter().map(|field| field.value).collect())
            .collect()
    }

    fn reference(text: &str) -> Vec<Vec<String>> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        reader
            .records()
            .map(|record| record.unwrap().iter().map(str::to_owned).collect())
            .collect()
    }

    #[test]
    fn values_match_csv_crate_on_edge_cases() {
        let cases = [
            "a,b\n1,2\n",
            "a,b\r\n1,2",
            "\n\na\n\n\nb\n",
            "\"a,b\",\"c\"\"d\"\n",
            "\"multi\nline\",x\r\n",
            "\"ab\"cd,e\n",
            "a\"b,c\n",
            "\"unterminated\n",
            ",\n,,\n",
            "\"\"\n",
            "x\ry\rz",
            " \"a\" ,b\n",
            "\"a\"\"\"\"\n",
            "",
        ];
        for case in cases {
            assert_eq!(values(case), reference(case), "input {case:?}");
        }
    }

    #[test]
    fn records_carry_field_and_terminator_ranges() {
        let text = "a,\"b\"\r\nc";
        let records = scan(text.as_bytes(), COMMA);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].span, 0..5);
        assert_eq!(records[0].terminator, 5..7);
        assert_eq!(records[0].fields[1].raw, 2..5);
        assert!(records[0].fields[1].quoted);
        assert_eq!(records[1].terminator, 8..8);
    }

    #[test]
    fn bytes_after_closing_quote_mark_field_irregular() {
        let records = scan(b"\"ab\"cd,e", COMMA);
        assert!(records[0].fields[0].irregular);
        assert!(!records[0].fields[1].irregular);
    }

    #[test]
    fn scan_file_skips_byte_order_mark() {
        let scan = scan_file(b"\xEF\xBB\xBFa,b\n", COMMA);
        assert_eq!(scan.bom, 3);
        assert_eq!(scan.records[0].span, 3..6);
        assert_eq!(scan.records[0].fields[0].value, "a");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
