// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden reports in `spec/golden/report/`: every file validates against the
//! published schema, deserializes to `Report`, satisfies `Report::validate`
//! and survives JSON and CBOR round trips unchanged. The set of files covers
//! every status, every group of error codes, bulk operations, unusual and
//! irreversible effects.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::report::{Report, ReportStatus};

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn golden_reports() -> Vec<(String, serde_json::Value)> {
    let dir = spec_dir().join("golden/report");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no golden reports in {}", dir.display());
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, read_json(&path))
        })
        .collect()
}

fn published_validator() -> jsonschema::Validator {
    let schema = read_json(&spec_dir().join("schemas/nurt-report-1.schema.json"));
    jsonschema::validator_for(&schema).unwrap()
}

fn error_codes(value: &serde_json::Value, codes: &mut BTreeSet<String>) {
    if let Some(error) = value.get("error") {
        if let Some(code) = error.get("code").and_then(serde_json::Value::as_str) {
            codes.insert(code.to_owned());
        }
        if let Some(cause) = error.get("cause") {
            let wrapper = serde_json::json!({ "error": cause });
            error_codes(&wrapper, codes);
        }
    }
}

#[test]
fn golden_reports_match_the_published_schema() {
    let validator = published_validator();
    for (name, value) in golden_reports() {
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:#?}");
    }
}

#[test]
fn golden_reports_satisfy_the_report_rules() {
    for (name, value) in golden_reports() {
        let report: Report =
            serde_json::from_value(value).unwrap_or_else(|error| panic!("{name}: {error}"));
        if let Err(violation) = report.validate() {
            panic!("{name}: {violation}");
        }
    }
}

#[test]
fn golden_reports_round_trip_through_json() {
    for (name, value) in golden_reports() {
        let report: Report =
            serde_json::from_value(value.clone()).unwrap_or_else(|error| panic!("{name}: {error}"));
        let back = serde_json::to_value(&report).unwrap();
        assert_eq!(back, value, "{name}: JSON round trip changed the report");
    }
}

#[test]
fn golden_reports_round_trip_through_cbor() {
    for (name, value) in golden_reports() {
        let report: Report =
            serde_json::from_value(value.clone()).unwrap_or_else(|error| panic!("{name}: {error}"));
        let mut bytes = Vec::new();
        ciborium::into_writer(&report, &mut bytes).unwrap();
        let decoded: Report = ciborium::from_reader(bytes.as_slice())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            decoded, report,
            "{name}: CBOR round trip changed the report"
        );
        assert_eq!(serde_json::to_value(&decoded).unwrap(), value, "{name}");
    }
}

#[test]
fn golden_reports_cover_every_status() {
    let found: BTreeSet<String> = golden_reports()
        .iter()
        .map(|(_, value)| value["status"].as_str().unwrap().to_owned())
        .collect();
    for status in ReportStatus::ALL {
        assert!(
            found.contains(status.as_str()),
            "no golden report with status {status}"
        );
    }
}

#[test]
fn golden_reports_cover_every_group_of_error_codes() {
    let groups: &[(&str, &[&str])] = &[
        ("lexical and syntax", &["E_LEXICAL", "E_ESCAPE", "E_SYNTAX"]),
        (
            "correction",
            &["E_AMBIGUOUS", "E_UNCORRECTABLE", "E_EXPIRED_CANDIDATE"],
        ),
        ("syntax tree", &["E_AST_VERSION", "E_AST_SCHEMA"]),
        (
            "binding",
            &[
                "E_UNKNOWN_COMMAND",
                "E_UNKNOWN_PARAM",
                "E_UNKNOWN_NAME",
                "E_UNKNOWN_FIELD",
                "E_UNKNOWN_VALUE",
                "E_MISSING_PARAM",
                "E_TYPE",
                "E_ARITY",
                "E_PATH_RELATIVE",
                "E_PATH_FOREIGN",
                "E_ONE_EFFECT",
                "E_OS_ONLY",
            ],
        ),
        ("external program", &["E_EXTERNAL_MISSING"]),
        ("check", &["E_ASSERT"]),
        (
            "content",
            &["E_BINARY", "E_UNSUPPORTED_CONSTRUCT", "E_ENCODING"],
        ),
        ("interface", &["E_ARG"]),
        ("concurrent change", &["E_STALE"]),
        (
            "file system",
            &[
                "E_IO",
                "E_PERMISSION",
                "E_EXISTS",
                "E_NOT_FOUND",
                "E_IN_USE",
                "E_PRIVILEGE",
                "E_NO_SPACE",
            ],
        ),
        ("limits", &["E_TIMEOUT", "E_CANCELLED", "E_LIMIT"]),
        (
            "stage and rollback",
            &["E_STAGE_FAILED", "E_ROLLED_BACK", "E_ROLLBACK_FAILED"],
        ),
        (
            "journal",
            &[
                "E_JOURNAL_NOOP",
                "E_JOURNAL_EXPIRED",
                "E_JOURNAL_CONFLICT",
                "E_JOURNAL_PARTIAL",
            ],
        ),
    ];
    let mut found = BTreeSet::new();
    for (_, value) in golden_reports() {
        error_codes(&value, &mut found);
    }
    for (group, codes) in groups {
        assert!(
            codes.iter().any(|code| found.contains(*code)),
            "no golden report for the error group {group}"
        );
    }
    assert!(
        found.contains("E_JOURNAL_PARTIAL"),
        "no golden report for E_JOURNAL_PARTIAL"
    );
}

#[test]
fn golden_reports_cover_bulk_notable_irreversible_and_result_file() {
    let reports = golden_reports();
    for field in [
        "bulk",
        "notable",
        "irreversible",
        "result_file",
        "corrections",
        "records",
        "exec",
        "job",
        "diff",
    ] {
        assert!(
            reports.iter().any(|(_, value)| value.get(field).is_some()),
            "no golden report with `{field}`"
        );
    }
    assert!(
        reports
            .iter()
            .any(|(_, value)| value.get("notable").is_some() && value.get("irreversible").is_some()),
        "no golden report with both `notable` and `irreversible`"
    );
}

#[test]
fn golden_reports_never_carry_journal_state() {
    for (name, value) in golden_reports() {
        let text = serde_json::to_string(&value).unwrap();
        for word in [
            "\"expires\"",
            "\"undoable\"",
            "\"undo_available\"",
            "\"deadline\"",
        ] {
            assert!(!text.contains(word), "{name} contains {word}");
        }
    }
}
