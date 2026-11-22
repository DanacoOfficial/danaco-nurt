// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reference corpus of program output with the expected responses.
//!
//! `tests/data/output/cases.json` lists cases: the input file written to
//! standard output, the exit code, the view parameters and the expected
//! standard output text, `shown` counters (head, tail, omitted, filtered),
//! summary lines and progress.

use std::path::PathBuf;

use nurt_contracts::stage::OutputStream;
use serde_json::Value;

use super::{Bench, view};
use crate::iface::{OutputView, ShowMode};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("output")
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn count(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|number| u32::try_from(number).ok())
}

fn show_mode(name: &str) -> ShowMode {
    match name {
        "auto" => ShowMode::Auto,
        "both" => ShowMode::Both,
        "stdout" => ShowMode::Stdout,
        "stderr" => ShowMode::Stderr,
        "errors" => ShowMode::Errors,
        "summary" => ShowMode::Summary,
        "none" => ShowMode::None,
        other => panic!("unknown show value {other}"),
    }
}

fn case_view(spec: &Value) -> OutputView {
    OutputView {
        show: spec["show"].as_str().map_or(ShowMode::Auto, show_mode),
        head: count(&spec["head"]),
        tail: count(&spec["tail"]),
        filter: text(&spec["filter"]),
        context: count(&spec["context"]).unwrap_or(0),
        dedupe: spec["dedupe"].as_bool().unwrap_or(false),
        quiet_on_success: spec["quiet_on_success"].as_bool().unwrap_or(false),
        summary: spec["summary"]
            .as_array()
            .map(|patterns| patterns.iter().filter_map(text).collect())
            .unwrap_or_default(),
        progress: text(&spec["progress"]),
        ..view()
    }
}

#[test]
fn output_matches_reference_corpus() {
    let dir = corpus_dir();
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let cases = cases.as_array().unwrap();
    assert!(!cases.is_empty());
    let bench = Bench::new();
    for (number, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        let input = std::fs::read(dir.join(case["input"].as_str().unwrap())).unwrap();
        let store = bench.create(&format!("case{number}"), false);
        store.write(OutputStream::Stdout, &input).unwrap();
        store.finish(OutputStream::Stdout).unwrap();
        store.finish(OutputStream::Stderr).unwrap();
        let exit_code = case["exit_code"]
            .as_i64()
            .and_then(|code| i32::try_from(code).ok());
        let out = store.render(&case_view(&case["view"]), exit_code).unwrap();
        assert_eq!(out.stdout, case["stdout"].as_str().unwrap(), "{name}");
        let shown = out.shown.stdout.unwrap();
        let counters: Vec<u64> = case["shown"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        assert_eq!(
            vec![shown.head, shown.tail, shown.omitted_lines, shown.filtered],
            counters,
            "{name}"
        );
        if let Some(lines) = case["summary_lines"].as_array() {
            let expected: Vec<String> = lines.iter().filter_map(text).collect();
            assert_eq!(out.summary_lines, expected, "{name}");
        }
        if let Some(percent) = case["progress"].as_f64() {
            assert_eq!(out.progress.and_then(|progress| progress.percent), Some(percent), "{name}");
        }
    }
}
