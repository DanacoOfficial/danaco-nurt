// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::fmt::Write as _;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{DocFormatId, SerializeOptions};
use nurt_contracts::value::{Fields, Record, Value};

use crate::value::{
    QuoteRules, Syntax, needs_quotes, parse_number, render_scalar, to_document_value,
};

/// Indentation of JSON and YAML output, in spaces.
const INDENT: usize = 2;

/// Writes records as a document of `format`.
pub(super) fn records(
    records: &[Record],
    format: &DocFormatId,
    options: &SerializeOptions,
) -> Result<Vec<u8>> {
    match format.as_str() {
        "json" => Ok(json(records, options.pretty).into_bytes()),
        "yaml" => Ok(yaml(records).into_bytes()),
        "toml" => Ok(toml(records).into_bytes()),
        "csv" => table(
            records,
            delimiter(options, b',')?,
            options.header,
            csv::Terminator::CRLF,
        ),
        "tsv" => table(
            records,
            delimiter(options, b'\t')?,
            options.header,
            csv::Terminator::Any(b'\n'),
        ),
        other => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("records cannot be written as `{other}`; use json, yaml, toml, csv or tsv"),
        )
        .with_found(other)),
    }
}

fn delimiter(options: &SerializeOptions, default: u8) -> Result<u8> {
    match options.delimiter {
        None => Ok(default),
        Some(c) => u8::try_from(c)
            .ok()
            .filter(|byte| byte.is_ascii() && !matches!(byte, b'"' | b'\n' | b'\r'))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "the delimiter {c:?} cannot separate fields; use an ASCII character other than a quote or a line break"
                    ),
                )
            }),
    }
}

fn fields_of(record: &Record) -> Fields {
    match to_document_value(Value::Record(record.clone())) {
        Value::Map(fields) => fields,
        _ => Fields::new(),
    }
}

fn json(records: &[Record], pretty: bool) -> String {
    let mut out = String::new();
    let list = Value::List(
        records
            .iter()
            .map(|record| Value::Map(fields_of(record)))
            .collect(),
    );
    write_json(&mut out, &list, pretty.then_some(0));
    out.push('\n');
    out
}

fn write_json(out: &mut String, value: &Value, level: Option<usize>) {
    let (open, close, items): (char, char, Vec<(Option<&str>, &Value)>) = match value {
        Value::List(items) => ('[', ']', items.iter().map(|item| (None, item)).collect()),
        Value::Map(fields) => (
            '{',
            '}',
            fields
                .iter()
                .map(|(name, item)| (Some(name.as_str()), item))
                .collect(),
        ),
        scalar => {
            out.push_str(&json_scalar(scalar));
            return;
        }
    };
    out.push(open);
    for (position, (name, item)) in items.iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        if let Some(level) = level {
            out.push('\n');
            out.push_str(&" ".repeat(INDENT * (level + 1)));
        }
        if let Some(name) = name {
            out.push_str(&json_string(name));
            out.push_str(if level.is_some() { ": " } else { ":" });
        }
        write_json(out, item, level.map(|level| level + 1));
    }
    if let (Some(level), false) = (level, items.is_empty()) {
        out.push('\n');
        out.push_str(&" ".repeat(INDENT * level));
    }
    out.push(close);
}

fn json_scalar(value: &Value) -> String {
    match value {
        Value::Number(number) if !number.is_finite() => "null".to_owned(),
        Value::Decimal(lexeme) if parse_number(lexeme, Syntax::Json).is_none() => {
            json_string(lexeme)
        }
        Value::Text(text) => json_string(text),
        Value::Date(date) => json_string(&date.text),
        Value::Bytes(bytes) => json_string(&bytes.to_base64()),
        other => render_scalar(other, Syntax::Json).unwrap_or_else(|| "null".to_owned()),
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn yaml(records: &[Record]) -> String {
    if records.is_empty() {
        return "[]\n".to_owned();
    }
    let mut out = String::new();
    let list = Value::List(
        records
            .iter()
            .map(|record| Value::Map(fields_of(record)))
            .collect(),
    );
    write_yaml_block(&mut out, &list, 0);
    out
}

/// Writes a non-empty map or list in block style at `level`; every line
/// ends with a line break.
fn write_yaml_block(out: &mut String, value: &Value, level: usize) {
    let pad = " ".repeat(INDENT * level);
    match value {
        Value::Map(fields) => {
            for (name, item) in fields {
                out.push_str(&pad);
                out.push_str(&yaml_scalar_text(name));
                out.push(':');
                write_yaml_child(out, item, level + 1);
            }
        }
        Value::List(items) => {
            for item in items {
                if is_block(item) && matches!(item, Value::Map(_)) {
                    let mut inner = String::new();
                    write_yaml_block(&mut inner, item, level + 1);
                    out.push_str(&pad);
                    out.push_str("- ");
                    out.push_str(inner.trim_start_matches(' '));
                } else {
                    out.push_str(&pad);
                    out.push('-');
                    write_yaml_child(out, item, level + 1);
                }
            }
        }
        scalar => {
            out.push_str(&pad);
            out.push_str(&yaml_value(scalar));
            out.push('\n');
        }
    }
}

fn write_yaml_child(out: &mut String, value: &Value, level: usize) {
    if is_block(value) {
        out.push('\n');
        write_yaml_block(out, value, level);
    } else {
        out.push(' ');
        out.push_str(&yaml_value(value));
        out.push('\n');
    }
}

fn is_block(value: &Value) -> bool {
    match value {
        Value::Map(fields) => !fields.is_empty(),
        Value::List(items) => !items.is_empty(),
        _ => false,
    }
}

fn yaml_value(value: &Value) -> String {
    match value {
        Value::Map(_) => "{}".to_owned(),
        Value::List(_) => "[]".to_owned(),
        Value::Number(number) if number.is_nan() => ".nan".to_owned(),
        Value::Number(number) if number.is_infinite() => {
            if *number > 0.0 { ".inf" } else { "-.inf" }.to_owned()
        }
        Value::Decimal(lexeme) if parse_number(lexeme, Syntax::Yaml12).is_none() => {
            json_string(lexeme)
        }
        Value::Text(text) => yaml_scalar_text(text),
        Value::Date(date) => yaml_scalar_text(&date.text),
        Value::Bytes(bytes) => format!("!!binary {}", bytes.to_base64()),
        other => render_scalar(other, Syntax::Yaml12).unwrap_or_else(|| "null".to_owned()),
    }
}

/// Text as a plain scalar when YAML 1.1 and 1.2 read it back as the same
/// text, otherwise double-quoted.
fn yaml_scalar_text(text: &str) -> String {
    if text.is_empty() || needs_quotes(text, QuoteRules::Yaml) {
        json_string(text)
    } else {
        text.to_owned()
    }
}

fn toml(records: &[Record]) -> String {
    let mut out = String::new();
    for (position, record) in records.iter().enumerate() {
        if position > 0 {
            out.push('\n');
        }
        let _ = writeln!(out, "[[{}]]", toml_key(record.kind.as_str()));
        for (name, value) in fields_of(record) {
            if value.is_null() {
                continue;
            }
            let _ = writeln!(out, "{} = {}", toml_key(&name), toml_value(&value));
        }
    }
    out
}

fn toml_key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if bare {
        name.to_owned()
    } else {
        json_string(name)
    }
}

fn toml_value(value: &Value) -> String {
    match value {
        Value::Map(fields) if fields.is_empty() => "{}".to_owned(),
        Value::Map(fields) => {
            let members: Vec<String> = fields
                .iter()
                .filter(|(_, item)| !item.is_null())
                .map(|(name, item)| format!("{} = {}", toml_key(name), toml_value(item)))
                .collect();
            format!("{{ {} }}", members.join(", "))
        }
        Value::List(items) => {
            let members: Vec<String> = items.iter().map(toml_value).collect();
            format!("[{}]", members.join(", "))
        }
        Value::Null => json_string(""),
        Value::Number(number) if number.is_nan() => "nan".to_owned(),
        Value::Number(number) if number.is_infinite() => {
            if *number > 0.0 { "inf" } else { "-inf" }.to_owned()
        }
        Value::BigInt(digits) => json_string(digits),
        Value::Decimal(lexeme) if parse_number(lexeme, Syntax::Toml).is_none() => {
            json_string(lexeme)
        }
        Value::Date(date) if is_toml_datetime(&date.text) => date.text.clone(),
        Value::Date(date) => json_string(&date.text),
        Value::Text(text) => json_string(text),
        Value::Bytes(bytes) => json_string(&bytes.to_base64()),
        other => render_scalar(other, Syntax::Toml).unwrap_or_else(|| json_string("")),
    }
}

fn is_toml_datetime(text: &str) -> bool {
    text.parse::<toml_edit::Value>()
        .is_ok_and(|value| value.is_datetime())
}

fn table(
    records: &[Record],
    delimiter: u8,
    header: bool,
    terminator: csv::Terminator,
) -> Result<Vec<u8>> {
    let mut columns: Vec<String> = Vec::new();
    let rows: Vec<Fields> = records.iter().map(fields_of).collect();
    for row in &rows {
        for name in row.keys() {
            if !columns.contains(name) {
                columns.push(name.clone());
            }
        }
    }
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .terminator(terminator)
        .flexible(true)
        .from_writer(Vec::new());
    let failed = |error: csv::Error| {
        Error::new(
            ErrorCode::Internal,
            format!("writing a table failed: {error}"),
        )
    };
    if header && !columns.is_empty() {
        writer.write_record(&columns).map_err(failed)?;
    }
    for row in &rows {
        let cells: Vec<String> = columns
            .iter()
            .map(|name| row.get(name).map(cell).unwrap_or_default())
            .collect();
        writer.write_record(&cells).map_err(failed)?;
    }
    writer.into_inner().map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!("writing a table failed: {error}"),
        )
    })
}

fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Text(text) => text.clone(),
        Value::Date(date) => date.text.clone(),
        Value::Decimal(lexeme) => lexeme.clone(),
        Value::Bytes(bytes) => bytes.to_base64(),
        Value::Map(_) | Value::List(_) => {
            let mut out = String::new();
            write_json(&mut out, value, None);
            out
        }
        other => render_scalar(other, Syntax::Json).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::RecordKind;

    use super::*;

    fn item(pairs: &[(&str, Value)]) -> Record {
        pairs
            .iter()
            .fold(Record::new(RecordKind::Item), |record, (name, value)| {
                record.with(*name, value.clone())
            })
    }

    fn written(records: &[Record], format: &DocFormatId, options: &SerializeOptions) -> String {
        let bytes = super::records(records, format, options).unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn delimiter_rejects_characters_that_break_the_table() {
        for c in ['"', '\n', '\r', 'é'] {
            let options = SerializeOptions {
                delimiter: Some(c),
                ..SerializeOptions::default()
            };
            assert!(delimiter(&options, b',').is_err(), "{c:?}");
        }
        let options = SerializeOptions {
            delimiter: Some(';'),
            ..SerializeOptions::default()
        };
        assert_eq!(delimiter(&options, b',').unwrap(), b';');
        assert_eq!(
            delimiter(&SerializeOptions::default(), b'\t').unwrap(),
            b'\t'
        );
    }

    #[test]
    fn json_strings_escape_controls_and_quotes() {
        assert_eq!(
            json_string("a\"b\\c\n\u{1}\u{7f}é"),
            "\"a\\\"b\\\\c\\n\\u0001\\u007fé\""
        );
    }

    #[test]
    fn json_output_is_compact_or_indented() {
        let records = [item(&[
            ("a", Value::Int(1)),
            ("b", Value::Text("x".to_owned())),
        ])];
        let compact = SerializeOptions {
            pretty: false,
            ..SerializeOptions::default()
        };
        assert_eq!(
            written(&records, &DocFormatId::JSON, &compact),
            "[{\"a\":1,\"b\":\"x\"}]\n"
        );
        let pretty = written(&records, &DocFormatId::JSON, &SerializeOptions::default());
        assert_eq!(pretty, "[\n  {\n    \"a\": 1,\n    \"b\": \"x\"\n  }\n]\n");
        let none = written(&[], &DocFormatId::JSON, &SerializeOptions::default());
        assert_eq!(none, "[]\n");
    }

    #[test]
    fn yaml_output_quotes_text_that_would_change_type() {
        let records = [item(&[
            ("flag", Value::Text("yes".to_owned())),
            ("num", Value::Text("12".to_owned())),
            ("txt", Value::Text("a: b".to_owned())),
            ("empty", Value::Text(String::new())),
            ("list", Value::List(vec![Value::Int(1), Value::Null])),
        ])];
        let out = written(&records, &DocFormatId::YAML, &SerializeOptions::default());
        assert_eq!(
            out,
            "- flag: \"yes\"\n  num: \"12\"\n  txt: \"a: b\"\n  empty: \"\"\n  list:\n    - 1\n    - null\n"
        );
        let none = written(&[], &DocFormatId::YAML, &SerializeOptions::default());
        assert_eq!(none, "[]\n");
    }

    #[test]
    fn table_output_uses_the_union_of_columns_and_quotes_cells() {
        let records = [
            item(&[("a", Value::Text("x,y".to_owned()))]),
            item(&[("b", Value::Int(2)), ("a", Value::Text("q\"r".to_owned()))]),
        ];
        let csv = written(&records, &DocFormatId::CSV, &SerializeOptions::default());
        assert_eq!(csv, "a,b\r\n\"x,y\",\r\n\"q\"\"r\",2\r\n");
        let tsv = written(&records, &DocFormatId::TSV, &SerializeOptions::default());
        assert_eq!(tsv, "a\tb\nx,y\t\n\"q\"\"r\"\t2\n");
    }

    #[test]
    fn toml_output_writes_array_tables_and_skips_null() {
        let records = [item(&[
            ("name", Value::Text("x".to_owned())),
            ("gone", Value::Null),
            (
                "big",
                Value::BigInt("123456789012345678901234567890".to_owned()),
            ),
        ])];
        let out = written(&records, &DocFormatId::TOML, &SerializeOptions::default());
        assert!(out.starts_with("[["), "{out}");
        assert!(out.contains("name = \"x\"\n"), "{out}");
        assert!(!out.contains("gone"), "{out}");
        assert!(
            out.contains("big = \"123456789012345678901234567890\"\n"),
            "{out}"
        );
    }

    #[test]
    fn unsupported_format_is_rejected() {
        let error =
            super::records(&[], &DocFormatId::XML, &SerializeOptions::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }
}
