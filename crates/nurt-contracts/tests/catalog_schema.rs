// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/catalog_schema.rs
// Responsibility:   Tests that the published schema of catalog files equals the schema generated from the Rust types.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests that the published schema of catalog files equals the schema
//! generated from the Rust types. With `NURT_BLESS=1` the generated schema is
//! written to the published file.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::spec::{CATALOG_SCHEMA_ID, CatalogFile};

fn schema_path() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/schemas/nurt-catalog-1.schema.json")
}

fn generated_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(CatalogFile);
    let mut value = serde_json::to_value(&schema).unwrap();
    let object = value.as_object_mut().unwrap();
    object.insert(
        "$id".to_owned(),
        serde_json::Value::String(CATALOG_SCHEMA_ID.to_owned()),
    );
    value
}

#[test]
fn published_schema_equals_generated_schema() {
    let generated = generated_schema();
    let path = schema_path();
    if std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1") {
        let mut text = serde_json::to_string_pretty(&generated).unwrap();
        text.push('\n');
        fs::write(&path, text).unwrap();
    }
    let published: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        published == generated,
        "{} differs from the schema generated from the Rust types; run the tests with NURT_BLESS=1 to regenerate it",
        path.display()
    );
}

#[test]
fn published_schema_is_draft_2020_12_with_stable_id() {
    let published: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(schema_path()).unwrap()).unwrap();
    assert_eq!(
        published["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(published["$id"], CATALOG_SCHEMA_ID);
    assert!(jsonschema::validator_for(&published).is_ok());
}

#[test]
fn schema_rejects_unknown_keys_and_invalid_kinds() {
    let published: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(schema_path()).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&published).unwrap();
    let valid = serde_json::json!({
        "format": "nurt-catalog/1",
        "group": "core",
        "summary": "Core commands.",
        "command": [{
            "name": "count",
            "category": "transform",
            "reversibility": "read",
            "effect": "read",
            "implementation": "builtin",
            "input": ["any"],
            "output": ["int"],
            "summary": "Count the records."
        }]
    });
    assert!(validator.is_valid(&valid));
    let mut unknown = valid.clone();
    unknown["command"][0]["aliases"] = serde_json::json!(["wc"]);
    assert!(!validator.is_valid(&unknown));
    let mut category = valid.clone();
    category["command"][0]["category"] = serde_json::json!("filter");
    assert!(!validator.is_valid(&category));
    let mut missing = valid;
    missing.as_object_mut().unwrap().remove("summary");
    assert!(!validator.is_valid(&missing));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
