// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential test of `query` filters against results recorded with `jq`.
//!
//! `tests/d5/query/differential.json` holds named inputs and, for every
//! filter, the outputs of `jq -c` on one of them and whether `jq` reported a
//! compile error or a runtime error. Numbers are compared by value, objects
//! without regard to the order of their keys. Filters whose results differ
//! by design are listed in [`KNOWN_DIFFERENCES`]; the reference
//! documentation of `query` describes each group.

// Test helpers report setup failures by panicking; clippy exempts only `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Value;
use nurt_doc::jq::Query;
use nurt_testkit::ctx::TestContext;
use serde_json::Value as Json;

/// Smallest number of filters of the data file.
const MIN_CASES: usize = 300;

/// Filters whose results differ from `jq` by design, with the reason.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    (".b[5] = 1", "an update does not extend a list past its end"),
    (
        ".b | .[length] = 99",
        "an update does not extend a list past its end",
    ),
    (
        "null | .a.b = 1",
        "an update does not create containers from null",
    ),
    (
        "null | .[2] = 1",
        "an update does not create containers from null",
    ),
    ("del(.[0, 2])", "paths are deleted one after another"),
    (". * 0", "text times zero is null, as in jq 1.7"),
    ("\"a\" * 0", "text times zero is null, as in jq 1.7"),
    (
        "[match(\"(d)(x)?\").captures[].string]",
        "groups that did not take part are omitted",
    ),
    ("[1,2,3] | .[1.5]", "an index must be an integer"),
    (
        "{\"a\":1} | .[0]",
        "a number index of an object yields null",
    ),
    ("null | .[1:]", "a slice of null is an error"),
    ("\"abc\" | @base64d", "invalid Base64 input is an error"),
    (
        "[limit(5; .a | repeat(. * 2))]",
        "repeat emits its input first, as the jq manual specifies",
    ),
    (
        "[limit(3; repeat(1))]",
        "repeat emits its input first, as the jq manual specifies",
    ),
    (
        ".g | @base32",
        "an unknown format fails when the filter is compiled",
    ),
    (
        ".g | @base32 | @base32d",
        "an unknown format fails when the filter is compiled",
    ),
    ("fromstream(tostream)", "fromstream is not provided"),
    (
        "fromstream(([0], 1), ([1], 2), ([1]))",
        "fromstream is not provided",
    ),
];

fn data_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/d5/query/differential.json")
}

fn load() -> Json {
    let text = std::fs::read_to_string(data_file()).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// One recorded case.
struct Case<'d> {
    filter: &'d str,
    input: &'d Json,
    outputs: Vec<Json>,
    error: Option<&'d str>,
}

fn cases(data: &Json) -> Vec<Case<'_>> {
    let inputs = &data["inputs"];
    data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| Case {
            filter: case["filter"].as_str().unwrap(),
            input: &inputs[case["input"].as_str().unwrap()],
            outputs: case["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|text| serde_json::from_str(text.as_str().unwrap()).unwrap())
                .collect(),
            error: case["error"].as_str(),
        })
        .collect()
}

/// Plain JSON of a value with exact numbers turned into JSON numbers.
fn plain(value: &Value) -> Json {
    match value {
        Value::BigInt(digits) | Value::Decimal(digits) => digits
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(Json::Null, Json::Number),
        Value::List(items) => Json::Array(items.iter().map(plain).collect()),
        Value::Map(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), plain(item)))
                .collect(),
        ),
        other => other.to_plain_json(),
    }
}

fn same(expected: &Json, actual: &Json) -> bool {
    match (expected, actual) {
        (Json::Number(left), Json::Number(right)) => match (left.as_i64(), right.as_i64()) {
            (Some(left), Some(right)) => left == right,
            _ => left.as_f64() == right.as_f64(),
        },
        (Json::Array(left), Json::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same(left, right))
        }
        (Json::Object(left), Json::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(name, item)| right.get(name).is_some_and(|other| same(item, other)))
        }
        _ => expected == actual,
    }
}

/// Outcome of one filter: the outputs and how the run ended.
#[derive(Debug)]
struct Run {
    outputs: Vec<Json>,
    error: Option<&'static str>,
}

fn run(filter: &str, input: &Json, test: &TestContext) -> Run {
    let query = match Query::compile(filter) {
        Ok(query) => query,
        Err(error) => {
            assert_eq!(error.code, ErrorCode::Query, "{filter}: {}", error.message);
            return Run {
                outputs: Vec::new(),
                error: Some("compile"),
            };
        }
    };
    let ctx = test.ctx();
    let result = query.run(&Value::from_plain_json(input.clone()), &ctx);
    Run {
        outputs: result
            .outputs
            .iter()
            .map(|output| plain(&output.value))
            .collect(),
        error: result.error.map(|_| "runtime"),
    }
}

fn matches(case: &Case<'_>, run: &Run) -> bool {
    run.error == case.error
        && run.outputs.len() == case.outputs.len()
        && case
            .outputs
            .iter()
            .zip(&run.outputs)
            .all(|(expected, actual)| same(expected, actual))
}

#[test]
fn the_data_file_has_enough_distinct_filters() {
    let data = load();
    let cases = cases(&data);
    let mut filters: Vec<&str> = cases.iter().map(|case| case.filter).collect();
    filters.sort_unstable();
    filters.dedup();
    assert!(filters.len() >= MIN_CASES, "{} filters", filters.len());
    for (filter, _) in KNOWN_DIFFERENCES {
        assert!(
            filters.binary_search(filter).is_ok(),
            "`{filter}` is not in the data file"
        );
    }
}

#[test]
fn query_matches_the_recorded_jq_results() {
    let data = load();
    let test = TestContext::new();
    let mut unexpected = Vec::new();
    for case in cases(&data) {
        let run = run(case.filter, case.input, &test);
        let known = KNOWN_DIFFERENCES
            .iter()
            .any(|(filter, _)| *filter == case.filter);
        if !matches(&case, &run) && !known {
            unexpected.push(format!(
                "{}\n    jq:    {:?} {:?}\n    query: {:?} {:?}",
                case.filter, case.outputs, case.error, run.outputs, run.error
            ));
        }
    }
    assert!(
        unexpected.is_empty(),
        "{} differences:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
}

fn installed_jq() -> Option<PathBuf> {
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search)
        .map(|dir| dir.join("jq"))
        .find(|candidate| candidate.is_file())
}

fn jq_outputs(jq: &PathBuf, filter: &str, input: &Json) -> (Vec<Json>, Option<&'static str>) {
    let mut child = Command::new(jq)
        .arg("-c")
        .arg(filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let outputs = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let error = match output.status.code() {
        Some(0) => None,
        Some(3) => Some("compile"),
        _ => Some("runtime"),
    };
    (outputs, error)
}

/// [U] Runs the recorded filters through the installed `jq` of the same
/// version; returns without checks when `jq` is not in the search path.
#[test]
fn recorded_results_match_installed_jq_skipped_without_jq() {
    let Some(jq) = installed_jq() else {
        return;
    };
    let data = load();
    let version = Command::new(&jq).arg("--version").output().unwrap();
    if String::from_utf8_lossy(&version.stdout).trim() != data["generator"].as_str().unwrap() {
        return;
    }
    for case in cases(&data) {
        let (outputs, error) = jq_outputs(&jq, case.filter, case.input);
        assert_eq!(error, case.error, "{}", case.filter);
        assert_eq!(outputs.len(), case.outputs.len(), "{}", case.filter);
        assert!(
            case.outputs
                .iter()
                .zip(&outputs)
                .all(|(expected, actual)| same(expected, actual)),
            "{}",
            case.filter
        );
    }
}
