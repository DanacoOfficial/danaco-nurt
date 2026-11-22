// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Schemas of the effect report.
//!
//! The published schema is maintained by hand and is stricter than the schema
//! generated from the Rust types. The tests check that the generated schema
//! equals its approved snapshot (set `NURT_BLESS=1` to rewrite the snapshot),
//! that the two schemas agree on fields and codes, that every golden report
//! passes both, and that neither schema has a field about undo or journal
//! state.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::{
    ChangeAction, CorrectionClass, Irreversible, Notable, REPORT_SCHEMA_ID, Report, ReportStatus,
    Severity,
};
use serde_json::Value;

/// Property names that would tell whether, until when or how an operation
/// can be undone, or expose the state of a journal entry.
const UNDO_WINDOW_FIELDS: &[&str] = &[
    "undo",
    "undoable",
    "can_undo",
    "undo_available",
    "undo_until",
    "undo_window",
    "redo",
    "reversible",
    "revertible",
    "expires",
    "expires_at",
    "expiry",
    "expired",
    "deadline",
    "deadline_utc",
    "ttl",
    "time_left",
    "remaining",
    "after_ops",
    "after_seconds",
    "journal_state",
    "entry_state",
    "available",
    "undone",
    "retention",
];

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec")
}

fn read_json(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn published() -> Value {
    read_json(&spec_dir().join("schemas/nurt-report-1.schema.json"))
}

fn generated() -> Value {
    serde_json::to_value(schemars::schema_for!(Report)).unwrap()
}

fn snapshot_path() -> PathBuf {
    spec_dir().join("golden/report/schema/report.generated.json")
}

fn golden_reports() -> Vec<(String, Value)> {
    let dir = spec_dir().join("golden/report");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, read_json(&path))
        })
        .collect()
}

/// Collects the string values of an enumeration schema, whether written as
/// `enum` or as `oneOf`/`anyOf` of `const` values.
fn enum_values(schema: &Value) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    if let Some(items) = schema.get("enum").and_then(Value::as_array) {
        values.extend(items.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    if let Some(value) = schema.get("const").and_then(Value::as_str) {
        values.insert(value.to_owned());
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.get(key).and_then(Value::as_array) {
            for branch in branches {
                values.extend(enum_values(branch));
            }
        }
    }
    values
}

fn codes<T: Copy>(all: &[T], as_str: fn(T) -> &'static str) -> BTreeSet<String> {
    all.iter().map(|value| as_str(*value).to_owned()).collect()
}

fn property_names(schema: &Value, names: &mut BTreeSet<String>) {
    match schema {
        Value::Object(object) => {
            for key in ["properties", "patternProperties"] {
                if let Some(Value::Object(properties)) = object.get(key) {
                    names.extend(properties.keys().cloned());
                }
            }
            if let Some(Value::Array(required)) = object.get("required") {
                names.extend(required.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            for value in object.values() {
                property_names(value, names);
            }
        }
        Value::Array(items) => {
            for item in items {
                property_names(item, names);
            }
        }
        _ => {}
    }
}

#[test]
fn generated_schema_equals_the_approved_snapshot() {
    let generated = generated();
    let path = snapshot_path();
    if std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = serde_json::to_string_pretty(&generated).unwrap();
        text.push('\n');
        fs::write(&path, text).unwrap();
    }
    assert!(
        path.is_file(),
        "{} is missing; run the tests with NURT_BLESS=1 to create it",
        path.display()
    );
    assert!(
        read_json(&path) == generated,
        "{} differs from the schema generated from the Rust types; review the change and run the tests with NURT_BLESS=1 to approve it",
        path.display()
    );
}

#[test]
fn published_schema_is_draft_2020_12_with_stable_id() {
    let schema = published();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["$id"], REPORT_SCHEMA_ID);
    assert_eq!(schema["title"], "nurt-report/1");
    assert!(jsonschema::validator_for(&schema).is_ok());
}

#[test]
fn published_and_generated_schemas_have_the_same_report_fields() {
    let published_fields: BTreeSet<String> = published()["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let generated_fields: BTreeSet<String> = generated()["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(published_fields, generated_fields);
    let required = |schema: Value| -> BTreeSet<String> {
        schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect()
    };
    let expected: BTreeSet<String> = ["nurt", "status", "kind"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(required(published()), expected);
    assert_eq!(required(generated()), expected);
}

#[test]
fn published_codes_equal_the_rust_enumerations() {
    let defs = &published()["$defs"];
    assert_eq!(
        enum_values(&defs["status"]),
        codes(ReportStatus::ALL, ReportStatus::as_str)
    );
    assert_eq!(
        enum_values(&defs["severity"]),
        codes(Severity::ALL, Severity::as_str)
    );
    assert_eq!(
        enum_values(&defs["notable_code"]),
        codes(Notable::ALL, Notable::as_str)
    );
    assert_eq!(
        enum_values(&defs["irreversible_reason"]),
        codes(Irreversible::ALL, Irreversible::as_str)
    );
    assert_eq!(
        enum_values(&defs["change"]["properties"]["action"]),
        codes(ChangeAction::ALL, ChangeAction::as_str)
    );
    assert_eq!(
        enum_values(&defs["correction"]["properties"]["class"]),
        codes(CorrectionClass::ALL, CorrectionClass::as_str)
    );
}

#[test]
fn every_error_code_is_listed_in_the_published_schema() {
    let listed = enum_values(&published()["$defs"]["error_code"]);
    let defined = enum_values(&serde_json::to_value(schemars::schema_for!(ErrorCode)).unwrap());
    assert!(
        !defined.is_empty(),
        "the schema of ErrorCode lists no codes"
    );
    let missing: Vec<&String> = defined.difference(&listed).collect();
    let unknown: Vec<&String> = listed.difference(&defined).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "error codes missing from the report schema: {missing:?}; listed but not defined: {unknown:?}"
    );
    assert!(listed.contains("E_JOURNAL_PARTIAL"));
}

#[test]
fn result_file_is_an_optional_string() {
    let schema = published();
    assert_eq!(schema["properties"]["result_file"]["type"], "string");
    let required = schema["required"].as_array().unwrap();
    assert!(!required.iter().any(|name| name == "result_file"));
    assert!(generated()["properties"].get("result_file").is_some());
}

#[test]
fn schemas_have_no_field_about_undo_or_journal_state() {
    for (name, schema) in [("published", published()), ("generated", generated())] {
        let mut names = BTreeSet::new();
        property_names(&schema, &mut names);
        for field in UNDO_WINDOW_FIELDS {
            assert!(
                !names.contains(*field),
                "the {name} report schema has the field `{field}`"
            );
        }
    }
    let statuses = codes(ReportStatus::ALL, ReportStatus::as_str);
    for state in ["available", "expired", "undone", "expiring", "undoable"] {
        assert!(
            !statuses.contains(state),
            "the report status `{state}` describes the journal"
        );
    }
}

#[test]
fn golden_reports_match_the_generated_schema() {
    let validator = jsonschema::validator_for(&generated()).unwrap();
    for (name, value) in golden_reports() {
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:#?}");
    }
}

#[test]
fn published_schema_rejects_malformed_reports() {
    let validator = jsonschema::validator_for(&published()).unwrap();
    let valid = serde_json::json!({ "nurt": "1.0", "status": "ok", "kind": "remove", "op": 3 });
    assert!(validator.is_valid(&valid));
    let cases = [
        ("unknown field", serde_json::json!({ "undoable": true })),
        (
            "empty irreversible",
            serde_json::json!({ "irreversible": [] }),
        ),
        ("journal status", serde_json::json!({ "status": "undone" })),
        ("bad hash", serde_json::json!({ "ast_hash": "sha256:XYZ" })),
        ("bad version", serde_json::json!({ "nurt": "2.0" })),
        ("operation zero", serde_json::json!({ "op": 0 })),
        (
            "unknown notable code",
            serde_json::json!({ "notable": [{ "code": "undo_possible", "severity": "info" }] }),
        ),
        (
            "unknown irreversible reason",
            serde_json::json!({ "irreversible": [{ "reason": "window_closed" }] }),
        ),
        (
            "unknown error code",
            serde_json::json!({ "error": { "code": "E_NOPE", "message": "x" } }),
        ),
    ];
    for (case, patch) in cases {
        let mut report = valid.clone();
        for (key, value) in patch.as_object().unwrap() {
            report[key] = value.clone();
        }
        assert!(
            !validator.is_valid(&report),
            "the schema accepts a report with {case}"
        );
    }
}
