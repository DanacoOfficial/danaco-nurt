// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conformance of reports with the schema `nurt-report/1` for every error code.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::OutputFormat;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::Report;
use nurt_report::{render, respond};
use nurt_testkit::golden::{GoldenInput, GoldenRunner};
use serde_json::{Value, json};

use common::{
    assert_valid, full_error_report, minimal_error_report, options, read_json, read_text,
    report_validator, response_validator, spec_dir,
};

fn cbor_round_trip(report: &Report) -> Report {
    let mut bytes = Vec::new();
    ciborium::into_writer(report, &mut bytes).unwrap();
    ciborium::from_reader(bytes.as_slice()).unwrap()
}

fn json_round_trip(report: &Report) -> Report {
    serde_json::from_str(&serde_json::to_string(report).unwrap()).unwrap()
}

#[test]
fn every_error_code_yields_a_report_valid_against_the_schema() {
    let validator = report_validator();
    for code in ErrorCode::ALL {
        for report in [minimal_error_report(*code), full_error_report(*code)] {
            let value = serde_json::to_value(&report).unwrap();
            assert_valid(&validator, &value, code.as_str());
            assert_eq!(report.validate(), Ok(()), "{code}");
        }
    }
}

#[test]
fn every_error_code_survives_json_and_cbor_round_trips() {
    for code in ErrorCode::ALL {
        for report in [minimal_error_report(*code), full_error_report(*code)] {
            assert_eq!(json_round_trip(&report), report, "{code}: JSON");
            assert_eq!(cbor_round_trip(&report), report, "{code}: CBOR");
        }
    }
}

#[test]
fn every_error_code_renders_in_all_formats() {
    for code in ErrorCode::ALL {
        for report in [minimal_error_report(*code), full_error_report(*code)] {
            for format in OutputFormat::ALL {
                let text = render(&report, &options(*format));
                assert!(text.contains(code.as_str()), "{code}: {format:?}: {text}");
            }
        }
    }
}

#[test]
fn every_error_code_yields_a_valid_response() {
    let validator = response_validator();
    for code in ErrorCode::ALL {
        for format in OutputFormat::ALL {
            let response = respond(&full_error_report(*code), &options(*format));
            assert_eq!(response.validate(), Ok(()), "{code}");
            let value = serde_json::to_value(&response).unwrap();
            assert_valid(&validator, &value, code.as_str());
        }
    }
}

#[test]
fn every_error_code_is_listed_in_the_specification_and_the_schema() {
    let specification = read_text(&spec_dir().join("errors.md"));
    let schema = read_json(&spec_dir().join("schemas/nurt-report-1.schema.json"));
    let listed: Vec<&str> = schema["$defs"]["error_code"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect();
    for code in ErrorCode::ALL {
        let row = format!("| `{}` |", code.as_str());
        assert!(specification.contains(&row), "{code} missing in errors.md");
        assert!(listed.contains(&code.as_str()), "{code} missing in schema");
    }
    assert_eq!(listed.len(), ErrorCode::ALL.len());
}

#[test]
fn error_stage_is_written_inside_the_position() {
    for code in ErrorCode::ALL {
        let value = serde_json::to_value(minimal_error_report(*code)).unwrap();
        let stage = &value["error"]["at"]["stage"];
        assert_eq!(stage, &json!(code.stages()[0].as_str()), "{code}");
        assert!(value["error"].get("stage").is_none(), "{code}");
    }
}

fn conformance_dir(code: ErrorCode) -> std::path::PathBuf {
    common::manifest_dir()
        .join("../../conformance/cases/errors")
        .join(code.as_str())
}

fn conformance_output(input: &Value) -> Value {
    let report: Report = serde_json::from_value(input.clone()).unwrap();
    let schema_valid = report_validator().is_valid(input);
    json!({
        "schema_valid": schema_valid,
        "json_round_trip": json_round_trip(&report) == report,
        "cbor_round_trip": cbor_round_trip(&report) == report,
        "text": render(&report, &options(OutputFormat::Text)),
        "markdown": render(&report, &options(OutputFormat::Markdown)),
        "compact": render(&report, &options(OutputFormat::Compact)),
    })
}

#[test]
fn conformance_cases_exist_for_every_error_code_and_pass() {
    for code in ErrorCode::ALL {
        let runner = GoldenRunner::new(conformance_dir(*code)).bless(false);
        let outcome = runner.run(|case| match &case.input {
            GoldenInput::Json(value) => conformance_output(value),
            GoldenInput::Text(_) => Value::Null,
        });
        let report = outcome.unwrap_or_else(|error| panic!("{code}: {error}"));
        assert_eq!(report.passed.len(), 1, "{code}");
    }
}

#[test]
fn conformance_directory_holds_no_case_without_an_error_code() {
    let dir = common::manifest_dir().join("../../conformance/cases/errors");
    let known: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            known.contains(&name.as_str()),
            "unknown case directory {name}"
        );
    }
}
