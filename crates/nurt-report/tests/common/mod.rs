// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/common/mod.rs
// Responsibility:   Helpers shared by the integration tests of the crate.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Helpers shared by the integration tests of the crate.

// Every test binary includes this module and uses a different subset of it.
#![allow(dead_code)]
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::envelope::OutputFormat;
use nurt_contracts::error::{Candidate, Error, ErrorCode, Location};
use nurt_contracts::report::{Report, ReportStatus};
use nurt_report::RenderOptions;
use serde_json::Value;

/// Words that must not appear in a rendered result unless the report holds them.
pub(crate) const UNDO_WORDS: [&str; 7] = [
    "undo", "redo", "undone", "expire", "window", "revert", "deadline",
];

pub(crate) fn manifest_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
}

pub(crate) fn spec_dir() -> PathBuf {
    manifest_dir().join("../../spec")
}

pub(crate) fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub(crate) fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read_text(path))
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Golden reports of the specification, sorted by file name.
pub(crate) fn golden_reports() -> Vec<(String, Value)> {
    let dir = spec_dir().join("golden/report");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no golden reports in {}", dir.display());
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let value = read_json(&path);
            (name, value)
        })
        .collect()
}

pub(crate) fn golden_report(name: &str) -> Report {
    let path = spec_dir().join(format!("golden/report/{name}.json"));
    serde_json::from_value(read_json(&path)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

pub(crate) fn report_validator() -> jsonschema::Validator {
    let schema = read_json(&spec_dir().join("schemas/nurt-report-1.schema.json"));
    jsonschema::validator_for(&schema).unwrap()
}

/// Validator of the response envelope, which the request schema defines in
/// `$defs/response`.
pub(crate) fn response_validator() -> jsonschema::Validator {
    let schema = read_json(&spec_dir().join("schemas/nurt-request-1.schema.json"));
    let wrapper = serde_json::json!({
        "$schema": schema["$schema"],
        "$id": schema["$id"],
        "$ref": "#/$defs/response",
        "$defs": schema["$defs"],
    });
    jsonschema::validator_for(&wrapper).unwrap()
}

pub(crate) fn assert_valid(validator: &jsonschema::Validator, value: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:#?}");
}

/// Report of a failed program whose error has the given code and only the
/// fields that every error has.
pub(crate) fn minimal_error_report(code: ErrorCode) -> Report {
    let mut report = Report::new(ReportStatus::Failed, "replace");
    let error = Error::new(code, "conformance case");
    report.error = Some(match code.stages().first() {
        Some(stage) => error.with_stage(*stage),
        None => error,
    });
    report
}

/// Report of a failed program whose error has every optional field and a
/// cause.
pub(crate) fn full_error_report(code: ErrorCode) -> Report {
    let cause = Error::new(ErrorCode::Io, "read failed").with_detail("os_error", 13.into());
    let error = Error::new(code, "message of the error")
        .with_at(nurt_contracts::error::At::new(2, 5, 3))
        .with_location(Location {
            path: "/srv/app/data.json".to_owned(),
            line: Some(4),
            column: Some(9),
            end_line: None,
            end_column: None,
            byte: None,
        })
        .with_found("remvoe")
        .with_problem("not_merged")
        .with_fix("remove \"/srv/app/a.log\"")
        .with_help("remove")
        .with_expected(serde_json::json!(12))
        .with_actual(serde_json::json!({"files": 14}))
        .with_sample("/srv/app/a.go")
        .with_candidate(Candidate {
            rank: 1,
            reference: Some("c1".to_owned()),
            program: "remove \"/srv/app/a.log\"".to_owned(),
            why: Some("closest command".to_owned()),
        })
        .with_next_step("find \"/srv/app\"")
        .with_effect_applied(true)
        .with_detail("path", "/srv/app".into())
        .with_cause(cause);
    let mut report = Report::new(ReportStatus::Failed, "replace");
    report.error = Some(match code.stages().first() {
        Some(stage) => error.with_stage(*stage),
        None => error,
    });
    report
}

pub(crate) fn options(format: OutputFormat) -> RenderOptions {
    RenderOptions {
        format,
        ..RenderOptions::default()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
