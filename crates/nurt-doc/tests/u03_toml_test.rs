// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u03_toml_test.rs
// Responsibility:   Conformance of the TOML reader with the `toml-test` suite.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conformance of the TOML reader with the `toml-test` suite.
//!
//! Every document under `valid/` must read as the typed JSON next to it;
//! every document under `invalid/` must be rejected, except the cases that
//! TOML 1.1 made valid.
// Helpers outside `#[test]` functions fail the test on an unreadable suite.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{DateKind, Value};
use nurt_doc::formats::toml::{TomlDocument, TomlFormat};
use nurt_doc::registry::{DocOptions, open_with};
use serde_json::Value as Json;

/// Root of the suite in the repository.
fn suite() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/third_party/toml/toml-test")
}

/// Invalid cases of TOML 1.0 that TOML 1.1 accepts: times without seconds,
/// `\x` escapes, trailing commas and line breaks in inline tables. The
/// reader implements TOML 1.1.
const VALID_IN_TOML_1_1: [&str; 9] = [
    "invalid/datetime/no-secs.toml",
    "invalid/local-time/no-secs.toml",
    "invalid/local-datetime/no-secs.toml",
    "invalid/string/basic-byte-escapes.toml",
    "invalid/inline-table/trailing-comma.toml",
    "invalid/inline-table/linebreak-1.toml",
    "invalid/inline-table/linebreak-2.toml",
    "invalid/inline-table/linebreak-3.toml",
    "invalid/inline-table/linebreak-4.toml",
];

fn files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, extension, out);
        } else if path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(suite())
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn float_of(text: &str) -> Option<f64> {
    match text.trim_start_matches('+') {
        "inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" | "-nan" => Some(f64::NAN),
        other => other.parse().ok(),
    }
}

fn same_datetime(actual: &str, expected: &str) -> bool {
    let parse = |text: &str| {
        text.parse::<toml_edit::Datetime>()
            .map(|d| with_seconds(&d.to_string()))
    };
    matches!((parse(actual), parse(expected)), (Ok(a), Ok(b)) if a == b)
}

/// Canonical text with `:00` seconds added to a time written without them
/// (TOML 1.1), as `toml-test` writes it.
fn with_seconds(text: &str) -> String {
    let start = text.find(['T', 't', ' ']).map_or(0, |at| at + 1);
    let bytes = text.as_bytes();
    let time = bytes.get(start + 2) == Some(&b':');
    if time && bytes.get(start + 5) != Some(&b':') {
        format!("{}:00{}", &text[..start + 5], &text[start + 5..])
    } else {
        text.to_owned()
    }
}

/// Compares a value with the typed JSON of `toml-test`.
fn matches(actual: &Value, expected: &Json) -> bool {
    match (actual, expected) {
        (Value::List(items), Json::Array(expected)) => {
            items.len() == expected.len() && items.iter().zip(expected).all(|(a, e)| matches(a, e))
        }
        (_, Json::Object(object)) if is_typed(object) => scalar_matches(actual, object),
        (Value::Map(fields), Json::Object(object)) => {
            fields.len() == object.len()
                && fields
                    .iter()
                    .all(|(k, v)| object.get(k).is_some_and(|e| matches(v, e)))
        }
        _ => false,
    }
}

fn is_typed(object: &serde_json::Map<String, Json>) -> bool {
    object.len() == 2
        && object.get("type").is_some_and(Json::is_string)
        && object.get("value").is_some_and(Json::is_string)
}

fn scalar_matches(actual: &Value, object: &serde_json::Map<String, Json>) -> bool {
    let kind = object["type"].as_str().unwrap_or_default();
    let text = object["value"].as_str().unwrap_or_default();
    match (kind, actual) {
        ("string", Value::Text(value)) => value == text,
        ("integer", Value::Int(value)) => text.parse() == Ok(*value),
        ("bool", Value::Bool(value)) => text.parse() == Ok(*value),
        ("float", Value::Number(value)) => {
            float_of(text).is_some_and(|e| e == *value || (e.is_nan() && value.is_nan()))
        }
        ("datetime", Value::Date(date)) => {
            date.kind == DateKind::OffsetDateTime && same_datetime(&date.text, text)
        }
        ("datetime-local", Value::Date(date)) => {
            date.kind == DateKind::LocalDateTime && same_datetime(&date.text, text)
        }
        ("date-local", Value::Date(date)) => {
            date.kind == DateKind::LocalDate && same_datetime(&date.text, text)
        }
        ("time-local", Value::Date(date)) => {
            date.kind == DateKind::LocalTime && same_datetime(&date.text, text)
        }
        _ => false,
    }
}

#[test]
fn reads_every_valid_document_as_its_typed_json() {
    let mut documents = Vec::new();
    files(&suite().join("valid"), "toml", &mut documents);
    assert!(
        documents.len() > 150,
        "the suite has {} valid documents",
        documents.len()
    );
    let mut failures = Vec::new();
    for path in &documents {
        let expected: Json =
            serde_json::from_slice(&fs::read(path.with_extension("json")).unwrap()).unwrap();
        let text = String::from_utf8(fs::read(path).unwrap()).unwrap();
        match TomlDocument::parse(&text) {
            Ok(doc) if matches(&doc.to_value(), &expected) => {}
            Ok(doc) => failures.push(format!("{}: read {:?}", relative(path), doc.to_value())),
            Err(error) => failures.push(format!("{}: {error}", relative(path))),
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn rejects_every_invalid_document() {
    let mut documents = Vec::new();
    files(&suite().join("invalid"), "toml", &mut documents);
    assert!(
        documents.len() > 300,
        "the suite has {} invalid documents",
        documents.len()
    );
    let mut accepted = Vec::new();
    let mut not_utf8 = 0;
    for path in &documents {
        let name = relative(path);
        let bytes = fs::read(path).unwrap();
        let Ok(text) = String::from_utf8(bytes.clone()) else {
            // The reader takes text, so a document that is not UTF-8 is
            // refused when it is opened, as an encoding error, unless it is
            // UTF-16 text, which is decoded and read as text.
            if let Err(error) = open_with(&TomlFormat, &bytes, &DocOptions::default()) {
                assert_eq!(error.code, ErrorCode::Encoding, "{name}");
            }
            not_utf8 += 1;
            continue;
        };
        match TomlDocument::parse(&text) {
            Err(error) => assert_eq!(error.code(), ErrorCode::DocParse, "{name}"),
            Ok(_) if VALID_IN_TOML_1_1.contains(&name.as_str()) => {}
            Ok(_) => accepted.push(name),
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted invalid documents:\n{}",
        accepted.join("\n")
    );
    assert!(
        not_utf8 * 20 < documents.len(),
        "{not_utf8} of {} invalid documents are not UTF-8 text",
        documents.len()
    );
}

#[test]
fn toml_1_1_cases_are_read() {
    for name in VALID_IN_TOML_1_1 {
        let text = fs::read_to_string(suite().join(name)).unwrap();
        assert!(TomlDocument::parse(&text).is_ok(), "{name}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
