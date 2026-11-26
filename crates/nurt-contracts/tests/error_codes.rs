// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Tests assert through `unwrap`, `expect` and `panic!`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The list of error codes is the same in the code, in the specification of
//! error codes and in every published schema, and every code used in the
//! specification and the documentation is a known code.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use nurt_contracts::error::{AbiStatus, ErrorCode, Stage};

fn root() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn all_codes() -> Vec<&'static str> {
    ErrorCode::ALL.iter().map(|code| code.as_str()).collect()
}

/// One row of the code table of `spec/errors.md`.
struct Row {
    code: String,
    stages: String,
    status: String,
}

fn cell(text: &str) -> String {
    text.trim().trim_matches('`').to_owned()
}

fn specification_rows() -> Vec<Row> {
    let path = root().join("spec/errors.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .filter(|line| line.starts_with("| `E_"))
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
            assert!(cells.len() >= 4, "malformed row of the code table: {line}");
            Row {
                code: cell(cells[0]),
                stages: cell(cells[1]),
                status: cell(cells[2]),
            }
        })
        .collect()
}

fn schema_codes(schema: &serde_json::Value) -> Option<Vec<String>> {
    let definition = schema.get("$defs")?.get("ErrorCode")?;
    let mut codes = Vec::new();
    if let Some(values) = definition.get("enum").and_then(serde_json::Value::as_array) {
        codes.extend(
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned)),
        );
    }
    if let Some(variants) = definition
        .get("oneOf")
        .and_then(serde_json::Value::as_array)
    {
        for variant in variants {
            if let Some(code) = variant.get("const").and_then(serde_json::Value::as_str) {
                codes.push(code.to_owned());
            }
            if let Some(values) = variant.get("enum").and_then(serde_json::Value::as_array) {
                codes.extend(
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_owned)),
                );
            }
        }
    }
    Some(codes)
}

#[test]
fn codes_are_unique_and_well_formed() {
    let codes = all_codes();
    let unique: BTreeSet<_> = codes.iter().collect();
    assert_eq!(unique.len(), codes.len(), "duplicate error codes");
    for code in codes {
        assert!(code.starts_with("E_"), "{code}");
        assert!(
            code[2..]
                .chars()
                .all(|character| character.is_ascii_uppercase()
                    || character.is_ascii_digit()
                    || character == '_'),
            "{code}"
        );
    }
}

#[test]
fn every_code_round_trips() {
    for &code in ErrorCode::ALL {
        assert_eq!(ErrorCode::from_str(code.as_str()), Ok(code));
        assert_eq!(code.to_string(), code.as_str());
        let json = serde_json::to_value(code).unwrap();
        assert_eq!(json, serde_json::json!(code.as_str()));
        assert_eq!(serde_json::from_value::<ErrorCode>(json).unwrap(), code);
        assert!(!code.stages().is_empty(), "{code} has no stage");
        assert!(code.abi_status() <= 6, "{code}");
        assert_ne!(
            code.abi(),
            AbiStatus::Abi,
            "{code}: no language error maps to NURT_E_ABI"
        );
    }
    assert!(ErrorCode::from_str("E_NO_SUCH_CODE").is_err());
    assert!(ErrorCode::from_str("e_stale").is_err());
}

#[test]
fn specification_lists_every_code_in_order() {
    let rows = specification_rows();
    let listed: Vec<&str> = rows.iter().map(|row| row.code.as_str()).collect();
    assert_eq!(
        listed,
        all_codes(),
        "the code table of spec/errors.md differs from ErrorCode::ALL"
    );
}

#[test]
fn specification_states_stages_and_status() {
    for row in specification_rows() {
        let code = ErrorCode::from_str(&row.code).unwrap();
        let stages: Vec<&str> = code.stages().iter().map(|stage| stage.as_str()).collect();
        let listed: Vec<&str> = row.stages.split(',').map(str::trim).collect();
        assert_eq!(listed, stages, "stages of {} in spec/errors.md", row.code);
        assert_eq!(
            row.status,
            code.abi().name(),
            "C status of {} in spec/errors.md",
            row.code
        );
    }
}

#[test]
fn schemas_list_every_code() {
    let expected: BTreeSet<String> = all_codes().into_iter().map(str::to_owned).collect();
    let directory = root().join("spec/schemas");
    let mut checked = Vec::new();
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let schema: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if let Some(codes) = schema_codes(&schema) {
            let listed: BTreeSet<String> = codes.into_iter().collect();
            assert_eq!(listed, expected, "error codes in {}", path.display());
            checked.push(path);
        }
    }
    assert!(
        checked
            .iter()
            .any(|path| path.ends_with("nurt-value-1.schema.json")),
        "nurt-value-1.schema.json must define ErrorCode"
    );
}

fn markdown_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            markdown_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
}

/// Codes written in a text: `E_` at a word start followed by an upper-case
/// letter and upper-case letters, digits or underscores.
fn mentioned_codes(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut codes = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find("E_") {
        let start = index + offset;
        index = start + 2;
        let at_word_start =
            start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        if !at_word_start || !bytes.get(index).is_some_and(u8::is_ascii_uppercase) {
            continue;
        }
        let end = bytes[index..]
            .iter()
            .position(|byte| !(byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_'))
            .map_or(bytes.len(), |length| index + length);
        let code = text[start..end].trim_end_matches('_');
        codes.push(code.to_owned());
        index = end;
    }
    codes
}

#[test]
fn documentation_uses_known_codes() {
    let known: BTreeSet<&str> = all_codes().into_iter().collect();
    let mut files = Vec::new();
    markdown_files(&root().join("spec"), &mut files);
    markdown_files(&root().join("docs"), &mut files);
    let mut unknown = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        for code in mentioned_codes(&text) {
            if !known.contains(code.as_str()) {
                unknown.push(format!("{}: {code}", path.display()));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "unknown error codes in the documentation: {unknown:#?}"
    );
}

#[test]
fn scanner_finds_codes() {
    assert_eq!(
        mentioned_codes("see `E_STALE`, not NURT_E_IO or E_* or E_"),
        ["E_STALE"]
    );
    assert_eq!(mentioned_codes("E_JOURNAL_PARTIAL."), ["E_JOURNAL_PARTIAL"]);
}

#[test]
fn abi_mapping_follows_the_c_interface() {
    assert_eq!(ErrorCode::Timeout.abi(), AbiStatus::Timeout);
    assert_eq!(ErrorCode::Cancelled.abi(), AbiStatus::Cancelled);
    assert_eq!(ErrorCode::Syntax.abi(), AbiStatus::Arg);
    assert_eq!(ErrorCode::AstVersion.abi(), AbiStatus::Arg);
    assert_eq!(ErrorCode::Io.abi(), AbiStatus::Io);
    assert_eq!(ErrorCode::JournalCorrupt.abi(), AbiStatus::Io);
    assert_eq!(ErrorCode::Internal.abi(), AbiStatus::Internal);
    assert_eq!(ErrorCode::Stale.abi(), AbiStatus::Ok);
    assert_eq!(ErrorCode::JournalPartial.abi(), AbiStatus::Ok);
    let numbers: Vec<u32> = AbiStatus::ALL.iter().map(|status| status.code()).collect();
    assert_eq!(numbers, [0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(AbiStatus::Io.name(), "NURT_E_IO");
}

#[test]
fn stages_round_trip() {
    for &stage in Stage::ALL {
        assert_eq!(Stage::from_str(stage.as_str()), Ok(stage));
        assert_eq!(Stage::from_str(stage.name()), Ok(stage));
        assert_eq!(
            serde_json::to_value(stage).unwrap(),
            serde_json::json!(stage.as_str())
        );
    }
    assert_eq!(Stage::After.to_string(), "5a");
    assert!(Stage::from_str("8").is_err());
    assert!(Stage::Plan.is_before_effect());
    assert!(!Stage::After.is_before_effect());
}

#[test]
fn journal_partial_is_listed() {
    assert!(ErrorCode::ALL.contains(&ErrorCode::JournalPartial));
    assert_eq!(ErrorCode::JournalPartial.stages(), [Stage::Plan]);
}
