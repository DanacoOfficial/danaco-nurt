// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-testkit/tests/golden.rs
// Responsibility:   Golden file runner on the sample cases and on temporary directories.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Golden file runner on the sample cases and on temporary directories.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::path::Path;

use nurt_testkit::golden::{GoldenError, GoldenInput, GoldenRunner};
use serde_json::{Value, json};

fn word_count(input: &GoldenInput) -> Value {
    match input {
        GoldenInput::Text(text) => json!({
            "lines": text.lines().count(),
            "words": text.split_whitespace().count(),
        }),
        GoldenInput::Json(value) => json!({
            "items": value.as_array().map_or(0, Vec::len),
        }),
    }
}

#[test]
fn sample_cases_match() {
    let dir = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/golden/sample");
    let report = GoldenRunner::new(dir)
        .run(|case| word_count(&case.input))
        .unwrap();
    assert_eq!(report.passed, ["list", "text"]);
}

#[test]
fn differences_are_listed_by_pointer() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("case.in.txt"), "a b\n").unwrap();
    std::fs::write(
        dir.path().join("case.out.json"),
        "{\"lines\": 1, \"words\": 3}",
    )
    .unwrap();
    let error = GoldenRunner::new(dir.path())
        .bless(false)
        .run(|case| word_count(&case.input))
        .unwrap_err();
    let GoldenError::Mismatch { failures } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(failures[0].differences, ["/words: expected 3, actual 2"]);
    assert!(error.to_string().contains("NURT_BLESS=1"));
}

#[test]
fn blessing_writes_expected_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("case.in.json"), "[1, 2]").unwrap();
    let missing = GoldenRunner::new(dir.path())
        .bless(false)
        .run(|case| word_count(&case.input))
        .unwrap_err();
    assert!(matches!(missing, GoldenError::Mismatch { .. }));

    let report = GoldenRunner::new(dir.path())
        .bless(true)
        .run(|case| word_count(&case.input))
        .unwrap();
    assert_eq!(report.blessed, ["case"]);
    let written = std::fs::read_to_string(dir.path().join("case.out.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&written).unwrap(),
        json!({"items": 2})
    );

    let report = GoldenRunner::new(dir.path())
        .bless(false)
        .run(|case| word_count(&case.input))
        .unwrap();
    assert_eq!(report.passed, ["case"]);
}

#[test]
fn an_empty_directory_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = GoldenRunner::new(dir.path()).cases().unwrap_err();
    assert!(matches!(error, GoldenError::NoCases { .. }));
}

#[test]
fn a_case_with_two_input_files_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("case.in.json"), "[1]").unwrap();
    std::fs::write(dir.path().join("case.in.txt"), "a\n").unwrap();
    let error = GoldenRunner::new(dir.path()).cases().unwrap_err();
    let GoldenError::Io { message, .. } = &error else {
        panic!("unexpected error: {error}");
    };
    assert!(message.contains("`case`"), "{message}");
    assert!(message.contains("more than one input file"), "{message}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
