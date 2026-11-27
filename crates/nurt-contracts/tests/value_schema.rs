// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/value_schema.rs
// Responsibility:   The published schema `nurt-value/1` equals the schema generated from the.
// Dependencies:     schemars, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Tests assert through `unwrap`, `expect` and `panic!`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The published schema `nurt-value/1` equals the schema generated from the
//! Rust types. Run with `NURT_BLESS=1` to write the generated schema.

use std::path::PathBuf;

use nurt_contracts::error::Error;
use nurt_contracts::record::TypedRecord;
use nurt_contracts::value::{Record, Value};
use schemars::generate::SchemaSettings;
use serde_json::json;

fn schema_path() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/schemas/nurt-value-1.schema.json")
}

fn generated_schema() -> serde_json::Value {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let roots = [
        generator.subschema_for::<Value>(),
        generator.subschema_for::<Record>(),
        generator.subschema_for::<TypedRecord>(),
        generator.subschema_for::<Error>(),
    ];
    let definitions = generator.definitions().clone();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://nurt.danaco-group.pl/schema/nurt-value-1.schema.json",
        "title": "nurt-value/1",
        "description": "Values, dynamic records, typed records and errors of the Nurt language.",
        "anyOf": roots,
        "$defs": definitions,
    })
}

#[test]
fn published_schema_matches_types() {
    let generated = generated_schema();
    let path = schema_path();
    if std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1") {
        let text = serde_json::to_string_pretty(&generated).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let published: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        published == generated,
        "{} differs from the schema generated from the types; run the tests with NURT_BLESS=1 \
         to regenerate it and review the difference",
        path.display()
    );
}

#[test]
fn schema_is_valid_json_schema() {
    let schema = generated_schema();
    jsonschema::validator_for(&schema).unwrap();
    for name in [
        "Value",
        "Record",
        "TypedRecord",
        "Error",
        "ErrorCode",
        "Instant",
        "Range",
    ] {
        assert!(
            schema["$defs"].get(name).is_some(),
            "definition {name} is missing"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
