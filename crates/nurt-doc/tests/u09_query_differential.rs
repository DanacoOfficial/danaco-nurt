// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u09_query_differential.rs
// Responsibility:   Differential test of `query` filters against results recorded with `jq`.
// Dependencies:     nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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

/// Filters whose results differ from `jq` 1.8.1, with the reason.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    (
        "[match(\"(d)(x)?\").captures[].string]",
        "groups that did not take part are omitted",
    ),
    (
        ".g | @base32",
        "an unknown format fails when the filter is compiled",
    ),
    (
        ".g | @base32 | @base32d",
        "an unknown format fails when the filter is compiled",
    ),
];

fn data_file() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/query/differential.json")
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

/// A listed difference that `query` no longer shows would hide a later
/// regression of that filter, so every entry must still differ.
#[test]
fn every_known_difference_still_differs_from_the_recorded_jq_result() {
    let data = load();
    let test = TestContext::new();
    let cases = cases(&data);
    let stale: Vec<&str> = KNOWN_DIFFERENCES
        .iter()
        .map(|(filter, _)| *filter)
        .filter(|filter| {
            cases
                .iter()
                .filter(|case| case.filter == *filter)
                .all(|case| matches(case, &run(case.filter, case.input, &test)))
        })
        .collect();
    assert!(stale.is_empty(), "no longer different from jq: {stale:?}");
}

fn installed_jq() -> Option<PathBuf> {
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search)
        .map(|dir| dir.join("jq"))
        .find(|candidate| candidate.is_file())
}

/// Runs `command` with `input` on its standard input and returns its output.
///
/// The input is written from a thread while the output is read: a child that
/// answers before it has read everything would otherwise block on a full
/// output pipe while the harness blocks on a full input pipe. A child that
/// exits without reading its input, as `jq` does after a compile error, closes
/// the pipe; a broken pipe means the end of the input and the exit status
/// tells what happened.
fn run_with_input(mut command: Command, input: Vec<u8>) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || match stdin.write_all(&input) {
        Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => panic!("{error}"),
        Ok(()) | Err(_) => {}
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    output
}

#[cfg(unix)]
#[test]
fn a_child_that_answers_while_it_reads_does_not_block_the_harness() {
    let input = vec![b'x'; 4 * 1024 * 1024];
    let output = run_with_input(Command::new("cat"), input.clone());
    assert_eq!(output.stdout, input);
}

fn jq_outputs(jq: &PathBuf, filter: &str, input: &Json) -> (Vec<Json>, Option<&'static str>) {
    let mut command = Command::new(jq);
    command.arg("-c").arg(filter);
    let output = run_with_input(command, input.to_string().into_bytes());
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

/// `jq` exits at once after a compile error, before it reads its input, so
/// a large input meets a closed pipe; the status tells the outcome.
#[test]
fn a_compile_error_of_jq_is_not_a_broken_pipe_of_the_harness() {
    let Some(jq) = installed_jq() else {
        return;
    };
    let input = Json::Array((0..200_000).map(Json::from).collect());
    let (outputs, error) = jq_outputs(&jq, ".[", &input);
    assert!(outputs.is_empty());
    assert_eq!(error, Some("compile"));
    let (outputs, error) = jq_outputs(
        &jq,
        ".[1]",
        &Json::Array(vec![Json::from(1), Json::from(2)]),
    );
    assert_eq!(outputs, [Json::from(2)]);
    assert_eq!(error, None);
}

/// [U] Runs the recorded filters through the installed `jq` of the same
/// version; returns without checks when `jq` is not in the search path or has
/// another version.
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
