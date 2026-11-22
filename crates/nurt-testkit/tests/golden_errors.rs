// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Error reporting of the golden file runner.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::fs;

use nurt_testkit::golden::case::Normalizer;
use nurt_testkit::golden::{GoldenError, GoldenRunner};
use serde_json::json;

#[test]
fn missing_directory_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = GoldenRunner::new(dir.path().join("absent"))
        .bless(false)
        .cases()
        .unwrap_err();
    assert!(matches!(error, GoldenError::Io { .. }));
    assert!(error.to_string().starts_with("cannot access "), "{error}");
    assert_eq!(format!("{error:?}"), error.to_string());
}

#[test]
fn directory_without_inputs_has_no_cases() {
    let dir = tempfile::tempdir().unwrap();
    let error = GoldenRunner::new(dir.path()).bless(false).cases();
    if let Err(error) = error {
        assert!(matches!(error, GoldenError::NoCases { .. }));
        assert!(
            error.to_string().starts_with("no golden case in "),
            "{error}"
        );
    }
}

#[test]
fn invalid_json_input_is_reported_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("broken.in.json"), "{").unwrap();
    let error = GoldenRunner::new(dir.path())
        .bless(false)
        .cases()
        .unwrap_err();
    assert!(matches!(error, GoldenError::Json { .. }));
    assert!(error.to_string().starts_with("invalid JSON in "), "{error}");
}

#[test]
fn invalid_expected_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.in.txt"), "x").unwrap();
    fs::write(dir.path().join("a.out.json"), "not json").unwrap();
    let error = GoldenRunner::new(dir.path())
        .bless(false)
        .run(|_| json!({}))
        .unwrap_err();
    assert!(matches!(error, GoldenError::Json { .. }));
}

#[test]
fn mismatch_lists_missing_unexpected_and_changed_members() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.in.txt"), "x").unwrap();
    fs::write(
        dir.path().join("a.out.json"),
        r#"{"gone": 1, "same": 2, "changed": 3}"#,
    )
    .unwrap();
    let error = GoldenRunner::new(dir.path())
        .bless(false)
        .run(|_| json!({"same": 2, "changed": 4, "extra": 5}))
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("set NURT_BLESS=1"), "{text}");
    let GoldenError::Mismatch { failures } = &error else {
        panic!("{error}");
    };
    let lines = failures[0].differences.join("\n");
    assert!(
        lines.contains("/gone: missing member, expected 1"),
        "{lines}"
    );
    assert!(
        lines.contains("/extra: unexpected member, actual 5"),
        "{lines}"
    );
    assert!(lines.contains("/changed"), "{lines}");
}

#[test]
fn normalizer_is_applied_before_comparison() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.in.txt"), "x").unwrap();
    fs::write(dir.path().join("a.out.json"), r#"{"elapsed_ms": 1}"#).unwrap();
    let report = GoldenRunner::new(dir.path())
        .bless(false)
        .normalizer(Normalizer::none())
        .run(|_| json!({"elapsed_ms": 1}))
        .unwrap();
    assert_eq!(report.passed, ["a"]);
}
