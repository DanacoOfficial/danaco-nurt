// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Generation of the `nurt-check/1` schema from the result types and the
//! validation of check results of the golden bound trees against it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use nurt_contracts::config::Config;
use nurt_contracts::stage::NetworkTarget;
use nurt_plan::{CheckSummary, NetworkUse, TargetSource};
use serde_json::{Value, json};

/// Identifier of the schema, in the naming of the other contract schemas.
const SCHEMA_ID: &str = "https://nurt.danaco-group.pl/schema/nurt-check-1.schema.json";

/// File name of the generated schema.
const SCHEMA_FILE: &str = "nurt-check-1.schema.json";

/// Variable naming a directory that receives the generated schema file.
const OUTPUT_VARIABLE: &str = "NURT_SCHEMA_OUT";

fn generated_schema() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(CheckSummary)).unwrap();
    let root = schema.as_object_mut().unwrap();
    let mut ordered = serde_json::Map::new();
    ordered.insert(
        "$schema".to_owned(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    ordered.insert("$id".to_owned(), json!(SCHEMA_ID));
    ordered.insert("title".to_owned(), json!(nurt_plan::CHECK_FORMAT));
    root.remove("$schema");
    root.remove("title");
    for (key, value) in std::mem::take(root) {
        ordered.insert(key, value);
    }
    Value::Object(ordered)
}

fn golden_summaries() -> Vec<(String, CheckSummary)> {
    let registry = common::registry();
    let host = common::host();
    let config = Config::default();
    let mut found = Vec::new();
    for entry in std::fs::read_dir(common::golden_dir()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if stem.ends_with(".generated") {
            continue;
        }
        let plan = nurt_plan::plan(&common::golden(stem), &registry).unwrap();
        let summary = nurt_plan::check(&plan, &registry, &host, &config).unwrap();
        found.push((stem.to_owned(), summary));
    }
    assert!(!found.is_empty());
    found
}

#[test]
fn schema_is_generated_from_the_result_types() {
    let schema = generated_schema();
    assert_eq!(schema["$id"], SCHEMA_ID);
    assert_eq!(schema["title"], "nurt-check/1");
    assert_eq!(schema["type"], "object");
    let properties = schema["properties"].as_object().unwrap();
    for field in [
        "format",
        "effect",
        "reversibility",
        "operations",
        "waits",
        "paths",
        "network",
        "programs",
        "groups",
        "corrections",
        "details",
    ] {
        assert!(properties.contains_key(field), "{field}");
    }
    let definitions = schema["$defs"].as_object().unwrap();
    for definition in ["NetworkUse", "ProgramUse", "TargetSource", "StageDetails"] {
        assert!(definitions.contains_key(definition), "{definition}");
    }
    if let Ok(directory) = std::env::var(OUTPUT_VARIABLE) {
        let mut text = serde_json::to_string_pretty(&schema).unwrap();
        text.push('\n');
        std::fs::write(std::path::Path::new(&directory).join(SCHEMA_FILE), text).unwrap();
    }
}

#[test]
fn golden_check_results_validate_against_the_schema() {
    let validator = jsonschema::validator_for(&generated_schema()).unwrap();
    for (name, summary) in golden_summaries() {
        let value = serde_json::to_value(&summary).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        assert_eq!(value["format"], "nurt-check/1", "{name}");
    }
}

#[test]
fn network_targets_validate_against_the_schema() {
    let validator = jsonschema::validator_for(&generated_schema()).unwrap();
    let (_, mut summary) = golden_summaries().into_iter().next().unwrap();
    summary.network.push(NetworkUse {
        command: "http.request".to_owned(),
        param: Some("url".to_owned()),
        source: TargetSource::Literal,
        target: Some(NetworkTarget {
            scheme: "https".to_owned(),
            host: "api.example.com".to_owned(),
            port: Some(8443),
            url: Some("https://api.example.com:8443/v1".to_owned()),
        }),
        remote: None,
    });
    summary.network.push(NetworkUse {
        command: "git.push".to_owned(),
        param: Some("remote".to_owned()),
        source: TargetSource::Remote,
        target: None,
        remote: Some("origin".to_owned()),
    });
    let value = serde_json::to_value(&summary).unwrap();
    assert!(validator.is_valid(&value));
    assert_eq!(value["network"][0]["source"], "literal");
    assert_eq!(value["network"][1]["source"], "remote");
}

#[test]
fn schema_rejects_unknown_effect_class() {
    let validator = jsonschema::validator_for(&generated_schema()).unwrap();
    let (_, summary) = golden_summaries().into_iter().next().unwrap();
    let mut value = serde_json::to_value(&summary).unwrap();
    value["effect"] = json!("unbounded");
    assert!(!validator.is_valid(&value));
}
