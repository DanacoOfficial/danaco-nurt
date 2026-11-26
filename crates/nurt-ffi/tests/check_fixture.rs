// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The recorded answer of `nurt_check` carries the members of the check
//! document `nurt-check/1`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

use std::fs;
use std::path::Path;

use serde_json::Value;

#[test]
fn recorded_check_answer_has_every_required_member() {
    let path = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/fixtures/stub/check-read.json");
    let case: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let document = case["output"][0].as_object().unwrap();
    let schema: Value = serde_json::from_str(nurt_contracts::schema::CHECK).unwrap();
    for name in schema["required"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        assert!(document.contains_key(name), "missing member `{name}`");
    }
    assert_eq!(document["format"], "nurt-check/1");
}

#[test]
fn recorded_syntax_error_points_into_the_program_text() {
    let path = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/fixtures/stub/run-syntax-error.json");
    let case: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let program = case["input"]["program"].as_str().unwrap();
    let error = &case["output"][0]["report"]["error"];
    let chars = program.chars().count();

    assert_eq!(error["found"], "end of program");
    let column = error["at"]["column"].as_u64().unwrap();
    assert_eq!(column, u64::try_from(chars).unwrap() + 1);

    let problem = error["problem"].as_str().unwrap();
    let opened: usize = problem
        .strip_prefix("the list started at column ")
        .and_then(|rest| rest.split(' ').next())
        .and_then(|text| text.parse().ok())
        .unwrap();
    assert_eq!(program.chars().nth(opened - 1), Some('['));
}
