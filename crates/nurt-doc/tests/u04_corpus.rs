// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Edits of CSV, TSV and property list documents checked by the harness:
//! every document opens with its bytes kept, setting the current value
//! changes no byte, and every accepted change stays within the lines of its
//! node and reads back with the expected value.
//!
//! Besides the stored corpus, documents are generated: every combination of
//! separator, quoting, line break, byte order mark and final line break for
//! delimited text, and every XML sample written again by the `plist` crate
//! with other indentation and line breaks and as ASCII property lists.
//!
//! Cells, ASCII strings and removed cells read back as text, so the harness
//! runs only the operations whose expected value is text for these forms:
//! storing a number reads back as a string, and removing a cell empties it.
//! A changed type stores an integer, so ASCII property lists skip it.
//! Those behaviours are checked by `u04_csv` and `u04_plist`. Binary
//! property lists are rewritten as a whole and are checked there too.
#![cfg(feature = "testing")]
// Helper functions of the tests report failures by unwrapping and
// panicking, as the test functions themselves do.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use nurt_doc::formats::csv::CsvFormat;
use nurt_doc::formats::plist::{PlistFormat, PlistKind};
use nurt_doc::registry::DocFormat;
use nurt_doc::testing::{HarnessOperation, HarnessOptions, Summary, run_document};

/// Separators of the generated delimited files, with their names.
const SEPARATORS: [(char, &str); 4] = [
    (',', "comma"),
    (';', "semicolon"),
    ('\t', "tab"),
    ('|', "pipe"),
];

/// Byte order mark of UTF-8.
const BOM: &str = "\u{feff}";

fn corpus(format: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/d5/corpus")
        .join(format)
}

fn files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| extensions.iter().any(|known| ext == *known))
        })
        .collect();
    files.sort();
    files
}

fn options(operations: &[HarnessOperation]) -> HarnessOptions {
    HarnessOptions {
        operations: operations.to_vec(),
        ..HarnessOptions::default()
    }
}

/// Operations whose expected value a delimited file holds.
fn table_options() -> HarnessOptions {
    options(&[HarnessOperation::SetSame, HarnessOperation::ReplaceScalar])
}

/// Operations whose expected value an ASCII property list holds. A type
/// change stores a dictionary with an integer, which reads back as a string.
fn ascii_options() -> HarnessOptions {
    options(&[
        HarnessOperation::SetSame,
        HarnessOperation::ReplaceScalar,
        HarnessOperation::Remove,
    ])
}

fn assert_clean(summary: &Summary, what: &str) {
    let violations: Vec<String> = summary
        .violations
        .iter()
        .map(|v| {
            format!(
                "{}: {} {:?} {:?}: {}",
                v.path.display(),
                v.key,
                v.operation,
                v.guard,
                v.detail
            )
        })
        .collect();
    assert!(
        violations.is_empty(),
        "{what}: {} violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
    let not_opened: Vec<String> = summary
        .not_opened
        .iter()
        .map(|(path, message)| format!("{}: {message}", path.display()))
        .collect();
    assert!(
        not_opened.is_empty(),
        "{what}: not opened:\n{}",
        not_opened.join("\n")
    );
    assert!(summary.accepted > 0, "{what}: no change accepted");
}

fn quote_field(field: &str, separator: char, all: bool) -> String {
    let needed = field.contains(separator) || field.contains(['"', '\r', '\n']);
    if needed || all {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// Delimited table in the given style.
fn table(separator: char, style: &str, eol: &str, bom: bool, final_break: bool) -> String {
    let rows: Vec<Vec<String>> = if style == "plain" {
        vec![
            vec!["id".into(), "name".into(), "city".into(), "amount".into()],
            vec!["1".into(), "Ann".into(), "Oslo".into(), "12.50".into()],
            vec!["2".into(), "Bob".into(), "Rome".into(), "7".into()],
        ]
    } else {
        vec![
            vec!["id".into(), "name".into(), "note".into()],
            vec![
                "1".into(),
                format!("Smith{separator} Ann"),
                "said \"hi\"".into(),
            ],
            vec!["2".into(), "Bo".into(), "line one\nline two".into()],
            vec!["3".into(), String::new(), "plain".into()],
        ]
    };
    let lines: Vec<String> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|field| quote_field(field, separator, style == "all"))
                .collect::<Vec<_>>()
                .join(&separator.to_string())
        })
        .collect();
    let mut text = if bom { BOM.to_owned() } else { String::new() };
    text.push_str(&lines.join(eol));
    if final_break {
        text.push_str(eol);
    }
    text
}

/// Every generated delimited file with its name and format.
fn generated_tables() -> Vec<(String, String, CsvFormat)> {
    let mut out = Vec::new();
    for (separator, name) in SEPARATORS {
        let format = if separator == '\t' {
            CsvFormat::TSV
        } else {
            CsvFormat::CSV
        };
        for style in ["plain", "quoted", "all"] {
            for (eol, eol_name) in [("\n", "lf"), ("\r\n", "crlf")] {
                for bom in [false, true] {
                    for final_break in [true, false] {
                        let file = format!("{name}_{style}_{eol_name}_{bom}_{final_break}");
                        out.push((file, table(separator, style, eol, bom, final_break), format));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn stored_tables_keep_their_layout_under_cell_changes() {
    let mut summary = Summary::default();
    for path in files(&corpus("csv"), &["csv", "tsv", "tab"]) {
        let bytes = std::fs::read(&path).unwrap();
        let format = if path.extension().is_some_and(|ext| ext == "csv") {
            CsvFormat::CSV
        } else {
            CsvFormat::TSV
        };
        summary.merge(run_document(&path, &bytes, &format, &table_options()));
    }
    assert_clean(&summary, "stored tables");
    assert!(summary.files >= 40, "{} files", summary.files);
}

#[test]
fn generated_tables_keep_their_layout_under_cell_changes() {
    let tables = generated_tables();
    assert_eq!(tables.len(), 96);
    let mut summary = Summary::default();
    for (name, text, format) in tables {
        summary.merge(run_document(
            Path::new(&name),
            text.as_bytes(),
            &format,
            &table_options(),
        ));
    }
    assert_clean(&summary, "generated tables");
}

/// Sample whose key `a` is repeated. Removing a repeated key removes every
/// entry, also outside the lines of the entry the key selects, so the
/// harness does not remove there; `u04_plist` checks that removal.
const REPEATED_KEYS: &str = "xml_duplicate_keys.plist";

fn plist_options(path: &Path, bytes: &[u8]) -> HarnessOptions {
    if path.ends_with(REPEATED_KEYS) {
        options(&[
            HarnessOperation::SetSame,
            HarnessOperation::ReplaceScalar,
            HarnessOperation::ChangeType,
            HarnessOperation::InsertKey,
            HarnessOperation::Append,
        ])
    } else if PlistKind::of(bytes) == PlistKind::Ascii {
        ascii_options()
    } else {
        HarnessOptions::default()
    }
}

#[test]
fn stored_property_lists_keep_their_layout_under_every_operation() {
    let mut summary = Summary::default();
    for path in files(&corpus("plist"), &["plist", "entitlements"]) {
        let bytes = std::fs::read(&path).unwrap();
        summary.merge(run_document(
            &path,
            &bytes,
            &PlistFormat,
            &plist_options(&path, &bytes),
        ));
    }
    assert_clean(&summary, "stored property lists");
    assert!(summary.files >= 38, "{} files", summary.files);
}

/// XML samples of the stored corpus read by the `plist` crate.
fn samples() -> Vec<(String, plist::Value)> {
    files(&corpus("plist"), &["plist", "entitlements"])
        .into_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            if PlistKind::of(&bytes) != PlistKind::Xml || path.ends_with(REPEATED_KEYS) {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some((
                name,
                plist::Value::from_reader_xml(bytes.as_slice()).unwrap(),
            ))
        })
        .collect()
}

fn xml_text(value: &plist::Value, indent: Option<usize>, crlf: bool) -> String {
    let options = match indent {
        Some(width) => plist::XmlWriteOptions::default().indent(b' ', width),
        None => plist::XmlWriteOptions::default(),
    };
    let mut bytes = Vec::new();
    value
        .to_writer_xml_with_options(&mut bytes, &options)
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

#[test]
fn rewritten_xml_samples_keep_their_layout_under_every_operation() {
    let mut summary = Summary::default();
    for (name, value) in samples() {
        for indent in [None, Some(2), Some(4)] {
            for crlf in [false, true] {
                let text = xml_text(&value, indent, crlf);
                let file = format!("{name}_{indent:?}_{crlf}.plist");
                summary.merge(run_document(
                    Path::new(&file),
                    text.as_bytes(),
                    &PlistFormat,
                    &HarnessOptions::default(),
                ));
            }
        }
    }
    assert_clean(&summary, "rewritten XML samples");
    assert!(summary.files >= 100, "{} files", summary.files);
}

/// ASCII string: unquoted when every character allows it.
fn ascii_string(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_$+/:.-".contains(&byte))
        && !text.starts_with("//")
        && !text.starts_with("/*");
    if plain {
        return text.to_owned();
    }
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

/// ASCII property list text of `value`; `unit` and `eol` empty write one
/// line.
fn ascii_text(value: &plist::Value, indent: &str, unit: &str, eol: &str) -> String {
    let inner = format!("{indent}{unit}");
    let compact = eol.is_empty();
    match value {
        plist::Value::Array(items) if items.is_empty() => "()".to_owned(),
        plist::Value::Array(items) => {
            let rendered: Vec<String> = items
                .iter()
                .map(|item| ascii_text(item, &inner, unit, eol))
                .collect();
            if compact {
                format!("({})", rendered.join(", "))
            } else {
                let body: Vec<String> = rendered
                    .iter()
                    .map(|item| format!("{eol}{inner}{item}"))
                    .collect();
                format!("({}{eol}{indent})", body.join(","))
            }
        }
        plist::Value::Dictionary(dictionary) if dictionary.is_empty() => "{}".to_owned(),
        plist::Value::Dictionary(dictionary) => {
            let mut out = String::from("{");
            for (name, item) in dictionary {
                let rendered = ascii_text(item, &inner, unit, eol);
                if compact {
                    let _ = write!(out, " {} = {rendered};", ascii_string(name));
                } else {
                    let _ = write!(out, "{eol}{inner}{} = {rendered};", ascii_string(name));
                }
            }
            if compact {
                out.push_str(" }")
            } else {
                let _ = write!(out, "{eol}{indent}}}");
            }
            out
        }
        plist::Value::String(text) => ascii_string(text),
        plist::Value::Integer(integer) => integer.to_string(),
        plist::Value::Real(number) => number.to_string(),
        plist::Value::Boolean(flag) => (if *flag { "YES" } else { "NO" }).to_owned(),
        plist::Value::Date(date) => ascii_string(&date.to_xml_format()),
        plist::Value::Data(bytes) => {
            let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("<{hex}>")
        }
        other => panic!("no ASCII form for {other:?}"),
    }
}

#[test]
fn ascii_forms_of_xml_samples_keep_their_layout_under_text_operations() {
    let styles = [
        ("\t", "\n", "tab"),
        ("    ", "\n", "spaces"),
        ("\t", "\r\n", "crlf"),
        ("", "", "compact"),
    ];
    let mut summary = Summary::default();
    for (name, value) in samples() {
        for (unit, eol, style) in styles {
            let text = format!("{}{eol}", ascii_text(&value, "", unit, eol));
            let file = format!("{name}_{style}.plist");
            assert_eq!(PlistKind::of(text.as_bytes()), PlistKind::Ascii);
            summary.merge(run_document(
                Path::new(&file),
                text.as_bytes(),
                &PlistFormat,
                &ascii_options(),
            ));
        }
    }
    assert_clean(&summary, "ASCII forms");
    assert!(summary.files >= 60, "{} files", summary.files);
}

#[test]
fn generated_documents_are_detected_by_content() {
    let format: &dyn DocFormat = &PlistFormat;
    for (_, value) in samples().into_iter().take(3) {
        let text = xml_text(&value, None, false);
        assert_eq!(
            format.sniff(text.as_bytes()),
            nurt_doc::registry::Confidence::High
        );
    }
}
