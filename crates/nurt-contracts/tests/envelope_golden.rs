// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden envelopes: requests in `spec/golden/request/` and responses in
//! `spec/golden/request/response/`. Every file validates against the
//! published schema, deserializes, satisfies the envelope rules and survives
//! JSON and CBOR round trips unchanged.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::envelope::{Request, RequestInput, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn spec_dir() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec")
}

fn read_json(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn json_files(dir: &Path) -> Vec<(String, Value)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no golden files in {}", dir.display());
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, read_json(&path))
        })
        .collect()
}

fn requests() -> Vec<(String, Value)> {
    json_files(&spec_dir().join("golden/request"))
}

fn responses() -> Vec<(String, Value)> {
    json_files(&spec_dir().join("golden/request/response"))
}

fn request_schema() -> Value {
    read_json(&spec_dir().join("schemas/nurt-request-1.schema.json"))
}

/// Schema whose root is `$defs/response` of the request schema.
fn response_schema() -> Value {
    let schema = request_schema();
    serde_json::json!({
        "$schema": schema["$schema"],
        "$defs": schema["$defs"],
        "$ref": "#/$defs/response"
    })
}

fn errors(validator: &jsonschema::Validator, value: &Value) -> Vec<String> {
    validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect()
}

fn round_trip<T>(name: &str, value: &Value)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let typed: T =
        serde_json::from_value(value.clone()).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(
        &serde_json::to_value(&typed).unwrap(),
        value,
        "{name}: JSON round trip"
    );
    let mut bytes = Vec::new();
    ciborium::into_writer(&typed, &mut bytes).unwrap();
    let decoded: T =
        ciborium::from_reader(bytes.as_slice()).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(decoded, typed, "{name}: CBOR round trip");
}

#[test]
fn golden_requests_match_the_published_schema() {
    let validator = jsonschema::validator_for(&request_schema()).unwrap();
    for (name, value) in requests() {
        let found = errors(&validator, &value);
        assert!(found.is_empty(), "{name}: {found:#?}");
    }
}

#[test]
fn golden_requests_satisfy_the_envelope_rules_and_round_trip() {
    for (name, value) in requests() {
        let request: Request =
            serde_json::from_value(value.clone()).unwrap_or_else(|error| panic!("{name}: {error}"));
        if let Err(error) = request.validate() {
            panic!("{name}: {error}");
        }
        round_trip::<Request>(&name, &value);
    }
}

#[test]
fn golden_requests_cover_every_input() {
    let mut inputs = BTreeSet::new();
    let mut variants = BTreeSet::new();
    for (name, value) in requests() {
        let request: Request = serde_json::from_value(value).unwrap();
        let input = request
            .input()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let kind = match input {
            RequestInput::Program { variant, .. } => {
                variants.insert(format!("{variant:?}"));
                "program"
            }
            RequestInput::Ast(_) => "ast",
            RequestInput::Cursor(_) => "cursor",
            RequestInput::Accept(_) => "accept",
            _ => "other",
        };
        inputs.insert(kind);
    }
    assert_eq!(
        inputs,
        BTreeSet::from(["program", "ast", "cursor", "accept"])
    );
    assert!(
        variants.len() >= 2,
        "golden programs use a single notation variant"
    );
}

#[test]
fn golden_request_trees_match_the_syntax_tree_schema() {
    let path = spec_dir().join("schemas/nurt-ast-1.schema.json");
    let schema = read_json(&path);
    if schema.as_object().is_none_or(|object| object.is_empty()) {
        return;
    }
    let validator = jsonschema::validator_for(&schema).unwrap();
    for (name, value) in requests() {
        if let Some(tree) = value.get("ast") {
            let found = errors(&validator, tree);
            assert!(found.is_empty(), "{name}: {found:#?}");
        }
    }
}

#[test]
fn golden_responses_match_the_published_schema() {
    let validator = jsonschema::validator_for(&response_schema()).unwrap();
    let report = read_json(&spec_dir().join("schemas/nurt-report-1.schema.json"));
    let report_validator = jsonschema::validator_for(&report).unwrap();
    for (name, value) in responses() {
        let found = errors(&validator, &value);
        assert!(found.is_empty(), "{name}: {found:#?}");
        let found = errors(&report_validator, &value["report"]);
        assert!(found.is_empty(), "{name}: report: {found:#?}");
    }
}

#[test]
fn golden_responses_satisfy_the_envelope_rules_and_round_trip() {
    let mut rendered = 0;
    for (name, value) in responses() {
        let response: Response =
            serde_json::from_value(value.clone()).unwrap_or_else(|error| panic!("{name}: {error}"));
        if let Err(error) = response.validate() {
            panic!("{name}: {error}");
        }
        rendered += usize::from(response.rendered.is_some());
        round_trip::<Response>(&name, &value);
    }
    assert!(rendered > 0, "no golden response with rendered text");
}
