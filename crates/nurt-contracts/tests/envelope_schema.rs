// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Schemas of the request envelope and the response.
//!
//! The published schema is maintained by hand and describes the response in
//! `$defs/response`. The tests check that the schemas generated from the Rust
//! types equal their approved snapshots (set `NURT_BLESS=1` to rewrite them),
//! that the published and generated schemas agree on fields and values, and
//! that the published schema rejects malformed envelopes.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::envelope::{
    DiffMode, Language, OutputFormat, REQUEST_SCHEMA_ID, Request, Response, Verbosity,
};
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

fn published() -> Value {
    read_json(&spec_dir().join("schemas/nurt-request-1.schema.json"))
}

fn check_snapshot(file: &str, generated: &Value) {
    let path = spec_dir().join("golden/request/schema").join(file);
    if std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = serde_json::to_string_pretty(generated).unwrap();
        text.push('\n');
        fs::write(&path, text).unwrap();
    }
    assert!(
        path.is_file(),
        "{} is missing; run the tests with NURT_BLESS=1 to create it",
        path.display()
    );
    assert!(
        read_json(&path) == *generated,
        "{} differs from the schema generated from the Rust types; review the change and run the tests with NURT_BLESS=1 to approve it",
        path.display()
    );
}

fn generated_request() -> Value {
    serde_json::to_value(schemars::schema_for!(Request)).unwrap()
}

fn generated_response() -> Value {
    serde_json::to_value(schemars::schema_for!(Response)).unwrap()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

fn codes<T: Copy>(all: &[T], as_str: fn(T) -> &'static str) -> BTreeSet<String> {
    all.iter().map(|value| as_str(*value).to_owned()).collect()
}

#[test]
fn generated_request_schema_equals_the_approved_snapshot() {
    check_snapshot("request.generated.json", &generated_request());
}

#[test]
fn generated_response_schema_equals_the_approved_snapshot() {
    check_snapshot("response.generated.json", &generated_response());
}

#[test]
fn published_schema_is_draft_2020_12_with_stable_id() {
    let schema = published();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["$id"], REQUEST_SCHEMA_ID);
    assert_eq!(schema["title"], "nurt-request/1");
    assert!(jsonschema::validator_for(&schema).is_ok());
}

#[test]
fn published_and_generated_schemas_have_the_same_fields() {
    let schema = published();
    assert_eq!(
        keys(&schema["properties"]),
        keys(&generated_request()["properties"])
    );
    let options = &schema["$defs"]["options"]["properties"];
    let generated_options = generated_request();
    let generated_options = generated_options["$defs"]["RequestOptions"]["properties"].clone();
    assert_eq!(keys(options), keys(&generated_options));
    assert_eq!(
        keys(&schema["$defs"]["response"]["properties"]),
        keys(&generated_response()["properties"])
    );
}

#[test]
fn options_cover_every_shared_call_parameter() {
    let options = keys(&published()["$defs"]["options"]["properties"]);
    let expected: BTreeSet<String> = [
        "preview",
        "base",
        "limit",
        "fields",
        "format",
        "verbosity",
        "diff",
        "diff_context",
        "max_chars",
        "save_to",
        "timeout",
        "session",
        "language",
        "script",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(options, expected);
}

#[test]
fn published_values_equal_the_rust_enumerations() {
    let schema = published();
    let defs = &schema["$defs"];
    let options = &defs["options"]["properties"];
    assert_eq!(
        strings(&defs["format"]["enum"]),
        codes(OutputFormat::ALL, OutputFormat::as_str)
    );
    assert_eq!(
        strings(&options["verbosity"]["enum"]),
        codes(Verbosity::ALL, Verbosity::as_str)
    );
    assert_eq!(
        strings(&options["diff"]["enum"]),
        codes(DiffMode::ALL, DiffMode::as_str)
    );
    assert_eq!(
        strings(&options["language"]["enum"]),
        codes(Language::ALL, Language::as_str)
    );
    assert_eq!(
        strings(&schema["properties"]["variant"]["enum"]),
        codes_of_variants()
    );
    let rendered = strings(&defs["rendered"]["properties"]["format"]["enum"]);
    let mut text_formats = codes(OutputFormat::ALL, OutputFormat::as_str);
    text_formats.remove("json");
    assert_eq!(rendered, text_formats);
}

fn codes_of_variants() -> BTreeSet<String> {
    use nurt_contracts::kinds::SyntaxVariant;
    [SyntaxVariant::A, SyntaxVariant::B, SyntaxVariant::C]
        .iter()
        .map(|variant| {
            serde_json::to_value(variant)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn published_schema_rejects_malformed_requests() {
    let validator = jsonschema::validator_for(&published()).unwrap();
    let valid = serde_json::json!({ "nurt": "1.0", "program": "find \"/srv/app\"" });
    assert!(validator.is_valid(&valid));
    let cases = [
        ("no input", serde_json::json!({ "nurt": "1.0" })),
        (
            "two inputs",
            serde_json::json!({ "nurt": "1.0", "program": "find \"/srv\"", "cursor": "c_1" }),
        ),
        (
            "variant without program",
            serde_json::json!({ "nurt": "1.0", "cursor": "c_1", "variant": "b" }),
        ),
        (
            "unknown option",
            serde_json::json!({ "nurt": "1.0", "program": "x", "options": { "preveiw": true } }),
        ),
        (
            "limit too large",
            serde_json::json!({ "nurt": "1.0", "program": "x", "options": { "limit": 5001 } }),
        ),
        (
            "bad timeout",
            serde_json::json!({ "nurt": "1.0", "program": "x", "options": { "timeout": "soon" } }),
        ),
        (
            "unknown field",
            serde_json::json!({ "nurt": "1.0", "program": "x", "dry_run": true }),
        ),
        (
            "bad version",
            serde_json::json!({ "nurt": "2", "program": "x" }),
        ),
    ];
    for (case, request) in cases {
        assert!(
            !validator.is_valid(&request),
            "the schema accepts a request with {case}"
        );
    }
}
