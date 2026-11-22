// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Tests assert through `unwrap`, `expect` and `panic!`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Golden files of `spec/golden/value/`.
//!
//! Every file holds a JSON array; the prefix of the file name gives the type
//! of its items: `values-` for [`Value`], `records-` for [`TypedRecord`],
//! `dynamic-` for [`Record`] and `errors-` for [`Error`]. Every item passes
//! the published schema, deserializes and serializes to the same JSON, and
//! survives a CBOR round trip. Every typed record kind and every record kind
//! used by the command catalog has at least one golden record.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::path::PathBuf;
use std::str::FromStr;

use nurt_contracts::error::Error;
use nurt_contracts::record::TypedRecord;
use nurt_contracts::value::{Record, RecordKind, Value};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn published_schema() -> serde_json::Value {
    let path = root().join("spec/schemas/nurt-value-1.schema.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Validator of one definition of the published schema.
fn validator(schema: &serde_json::Value, definition: &str) -> jsonschema::Validator {
    let mut schema = schema.clone();
    let object = schema.as_object_mut().unwrap();
    object.remove("anyOf");
    object.insert(
        "$ref".to_owned(),
        serde_json::Value::String(format!("#/$defs/{definition}")),
    );
    jsonschema::validator_for(&schema).unwrap()
}

fn golden_files() -> Vec<(String, Vec<serde_json::Value>)> {
    let directory = root().join("spec/golden/value");
    let mut files: Vec<(String, Vec<serde_json::Value>)> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let json: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
            let serde_json::Value::Array(items) = json else {
                panic!("{name}: a golden file holds a JSON array");
            };
            (name, items)
        })
        .collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        !files.is_empty(),
        "no golden files in {}",
        directory.display()
    );
    files
}

fn round_trip<T>(label: &str, json: &serde_json::Value) -> T
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let value: T = serde_json::from_value(json.clone())
        .unwrap_or_else(|error| panic!("{label}: does not deserialize: {error}"));
    assert_eq!(
        &serde_json::to_value(&value).unwrap(),
        json,
        "{label}: JSON round trip"
    );
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&value, &mut bytes).unwrap();
    let back: T = ciborium::de::from_reader(bytes.as_slice())
        .unwrap_or_else(|error| panic!("{label}: CBOR does not deserialize: {error}"));
    assert_eq!(back, value, "{label}: CBOR round trip");
    value
}

fn check_schema(validator: &jsonschema::Validator, label: &str, json: &serde_json::Value) {
    let errors: Vec<String> = validator
        .iter_errors(json)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{label}: schema violations: {errors:#?}");
}

#[test]
fn golden_files_validate_and_round_trip() {
    let schema = published_schema();
    let values = validator(&schema, "Value");
    let records = validator(&schema, "TypedRecord");
    let dynamic = validator(&schema, "Record");
    let errors = validator(&schema, "Error");
    for (name, items) in golden_files() {
        for (index, json) in items.iter().enumerate() {
            let label = format!("{name}[{index}]");
            if name.starts_with("values-") {
                check_schema(&values, &label, json);
                round_trip::<Value>(&label, json);
            } else if name.starts_with("records-") {
                check_schema(&records, &label, json);
                let typed = round_trip::<TypedRecord>(&label, json);
                let record = Record::from(typed.clone());
                assert_eq!(record.kind, typed.kind(), "{label}");
                let back = TypedRecord::try_from(record.clone())
                    .unwrap_or_else(|error| panic!("{label}: {error}"));
                assert_eq!(back, typed, "{label}: dynamic record round trip");
                let tagged = serde_json::to_value(Value::Record(record)).unwrap();
                check_schema(&values, &label, &tagged);
                round_trip::<Value>(&label, &tagged);
            } else if name.starts_with("dynamic-") {
                check_schema(&dynamic, &label, json);
                round_trip::<Record>(&label, json);
            } else if name.starts_with("errors-") {
                check_schema(&errors, &label, json);
                round_trip::<Error>(&label, json);
            } else {
                panic!("{name}: unknown prefix of a golden file");
            }
        }
    }
}

fn golden_record_kinds() -> BTreeSet<RecordKind> {
    golden_files()
        .into_iter()
        .filter(|(name, _)| name.starts_with("records-"))
        .flat_map(|(_, items)| items)
        .map(|json| serde_json::from_value::<TypedRecord>(json).unwrap().kind())
        .collect()
}

#[test]
fn every_typed_kind_has_a_golden_record() {
    let covered = golden_record_kinds();
    let missing: Vec<&str> = TypedRecord::KINDS
        .iter()
        .filter(|kind| !covered.contains(kind))
        .map(|kind| kind.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "record kinds without a golden record: {missing:?}"
    );
}

/// Names in `input` and `output` of the catalog that are not record kinds.
const NOT_RECORD_KINDS: &[&str] = &[
    "none", "any", "same", "text", "int", "number", "bool", "path", "glob", "regex", "size",
    "duration", "instant", "key", "bytes",
];

#[test]
fn catalog_record_kinds_are_defined() {
    let directory = root().join("catalog");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    let mut used = BTreeSet::new();
    for entry in entries {
        let path = entry.unwrap().path();
        let is_group = path
            .extension()
            .is_some_and(|extension| extension == "toml")
            && path.file_name().is_some_and(|name| name != "_index.toml");
        if !is_group {
            continue;
        }
        let table: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let commands = table
            .get("command")
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for command in commands {
            for key in ["input", "output"] {
                let names = command
                    .get(key)
                    .and_then(toml::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for name in names.iter().filter_map(toml::Value::as_str) {
                    if !NOT_RECORD_KINDS.contains(&name) {
                        used.insert(name.to_owned());
                    }
                }
            }
        }
    }
    let covered = golden_record_kinds();
    for name in used {
        let kind = RecordKind::from_str(&name)
            .unwrap_or_else(|_| panic!("record kind `{name}` of the catalog is not a RecordKind"));
        assert!(
            TypedRecord::KINDS.contains(&kind),
            "record kind `{name}` has no typed structure"
        );
        assert!(
            covered.contains(&kind),
            "record kind `{name}` has no golden record"
        );
    }
}
