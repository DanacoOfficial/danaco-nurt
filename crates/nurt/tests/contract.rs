// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Envelopes of the client agree with the contract types of
//! `nurt-request/1` and `nurt-response/1`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt::{DiffMode, Format, Language, Request, Response, Variant, Verbosity};
use nurt_contracts::envelope as contract;
use serde_json::{Value, json};

fn golden(directory: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/golden")
        .join(directory);
    let mut files: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no golden files in {}", root.display());
    files
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn accepted_by_contract(request: &Request) -> contract::Request {
    let parsed: contract::Request = serde_json::from_value(request.to_value()).unwrap();
    parsed.validate().unwrap();
    parsed
}

#[test]
fn every_input_is_a_valid_contract_request() {
    let tree = read(&golden("ast")[0]);
    for request in [
        Request::program("find \"/srv/app\""),
        Request::program("find(\"/srv/app\")").variant(Variant::B),
        Request::ast(tree),
        Request::cursor("c_91fe"),
        Request::accept("c1"),
    ] {
        let parsed = accepted_by_contract(&request);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), request.to_value());
    }
}

#[test]
fn every_option_is_a_valid_contract_option() {
    let request = Request::program("find \"/srv/app\"")
        .preview(true)
        .base("/srv/app")
        .limit(5000)
        .fields(["path"])
        .format(Format::Compact)
        .verbosity(Verbosity::Quiet)
        .diff(DiffMode::None)
        .diff_context(0)
        .max_chars(10)
        .save_to("/home/user/out.txt")
        .timeout(Duration::from_millis(250))
        .session("s-1")
        .language(Language::Pl)
        .script(true);
    let parsed = accepted_by_contract(&request);
    assert_eq!(parsed.options.limit(), 5000);
    assert_eq!(parsed.options.timeout.as_deref(), Some("250ms"));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), request.to_value());
}

#[test]
fn golden_requests_survive_the_contract_unchanged() {
    for path in golden("request") {
        let value = read(&path);
        let parsed: contract::Request = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            value,
            "{}",
            path.display()
        );
    }
}

#[test]
fn golden_responses_are_read_by_the_client() {
    for path in golden("request/response") {
        let value = read(&path);
        let parsed: contract::Response = serde_json::from_value(value.clone()).unwrap();
        let response = Response::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
        assert_eq!(response.raw(), &value, "{}", path.display());
        assert_eq!(
            response.text(),
            parsed
                .rendered
                .as_ref()
                .map(|rendered| rendered.text.as_str())
        );
        assert_eq!(response.is_ok(), parsed.report.error.is_none());
        assert_eq!(response.has_more(), parsed.has_more());
    }
}

#[test]
fn golden_reports_expose_error_and_cursor() {
    for path in golden("report") {
        let report = read(&path);
        let parsed: nurt_contracts::report::Report =
            serde_json::from_value(report.clone()).unwrap();
        let response = Response::from_value(json!({"nurt": "1.0", "report": report})).unwrap();
        assert_eq!(response.next_cursor(), parsed.next_cursor.as_deref());
        match (&parsed.error, response.error()) {
            (None, None) => assert!(response.is_ok()),
            (Some(expected), Some(error)) => {
                assert_eq!(error.code(), expected.code().as_str(), "{}", path.display());
                assert_eq!(error.message(), expected.data().message);
                assert_eq!(error.fix(), expected.data().fix);
                assert_eq!(error.help(), expected.data().help.as_deref());
            }
            (expected, found) => panic!(
                "{}: expected error {expected:?}, found {found:?}",
                path.display()
            ),
        }
    }
}
