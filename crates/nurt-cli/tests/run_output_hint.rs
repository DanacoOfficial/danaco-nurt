// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! End-to-end test of the hint that `run` gives for reading the rest of a
//! cut output: the commands of `exec.read_hint` run as given and return the
//! lines the response left out, without running the program again.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_nurt");

/// Runs one program of the language with a state directory shared by the
/// calls of a test, and returns the JSON of the response.
fn nurt(home: &Path, program: &str) -> Value {
    let output = Command::new(BIN)
        .args(["--json", program])
        .env_clear()
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn field<'a>(record: &'a Value, name: &str) -> &'a Value {
    &record["fields"][name]["v"]
}

/// The `exec` record of a `run` response.
fn exec(response: &Value) -> &Value {
    &response["report"]["exec"]
}

/// The command of the hint that starts with `word` (`read "` or `search /`):
/// up to the next ` or `, the next `; ` or the final full stop.
fn command_of<'a>(hint: &'a str, word: &str) -> &'a str {
    let start = hint
        .find(word)
        .unwrap_or_else(|| panic!("no `{word}` in {hint}"));
    let rest = &hint[start..];
    let end = [" or search /", "; "]
        .iter()
        .filter_map(|mark| rest.find(mark))
        .min()
        .unwrap_or(rest.len());
    rest[..end].strip_suffix('.').unwrap_or(&rest[..end])
}

/// Text of the `content` record a `read` returned.
fn read_text(response: &Value) -> String {
    let rows = &response["report"]["records"]["rows"];
    field(&rows[0], "text").as_str().unwrap().to_owned()
}

/// The matches of a `search` response as (line, text) pairs.
fn matches(response: &Value) -> Vec<(i64, String)> {
    response["report"]["records"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                field(row, "no").as_i64().unwrap(),
                field(row, "text").as_str().unwrap().trim_end().to_owned(),
            )
        })
        .collect()
}

fn run_program(home: &Path, work: &Path, script: &str) -> Value {
    nurt(
        home,
        &format!(r#"run ["sh", "-c", "{script}"] cwd: "{}""#, work.display()),
    )
}

fn hint_of(response: &Value) -> String {
    field(exec(response), "read_hint")
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn the_read_command_of_the_hint_returns_the_lines_the_response_left_out() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let response = run_program(home.path(), work.path(), "seq 1 3000; exit 1");
    let hint = hint_of(&response);
    assert!(hint.starts_with("Output cut"), "{hint}");
    let text = field(exec(&response), "stdout").as_str().unwrap();
    assert!(text.starts_with("1\n2\n"), "{text}");
    assert!(text.len() <= 4096);
    let read = command_of(&hint, "read \"");
    assert!(
        read.ends_with(" lines: 21..2960 \"") || read.contains(" lines: 21..2960"),
        "{read}"
    );
    let rest = nurt(home.path(), read);
    let lines = read_text(&rest);
    assert_eq!(lines.lines().next(), Some("21"));
    assert_eq!(lines.lines().last(), Some("2960"));
    assert_eq!(lines.lines().count(), 2940);
}

#[test]
fn a_range_and_a_filter_taken_from_the_hint_both_return_the_left_out_lines() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let response = run_program(
        home.path(),
        work.path(),
        "seq 1 500; echo FAILED test_middle; seq 501 1000",
    );
    let hint = hint_of(&response);
    let read = command_of(&hint, "read \"");
    assert!(read.contains(" lines: 1..996"), "{read}");
    let narrow = read.replace(" lines: 1..996", " lines: 499..503");
    assert_eq!(
        read_text(&nurt(home.path(), &narrow)),
        "499\n500\nFAILED test_middle\n501\n502\n"
    );
    let found = nurt(home.path(), command_of(&hint, "search /"));
    assert_eq!(matches(&found), [(501, "FAILED test_middle".to_owned())]);
}

#[test]
fn the_hint_of_a_selection_of_error_lines_names_the_search() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let response = run_program(
        home.path(),
        work.path(),
        "seq 1 100; echo FAILED test_x; seq 101 200; exit 1",
    );
    let hint = hint_of(&response);
    assert!(!hint.contains(" or search"), "{hint}");
    let found = nurt(home.path(), command_of(&hint, "search /"));
    assert_eq!(matches(&found), [(101, "FAILED test_x".to_owned())]);
}

#[test]
fn a_short_output_has_no_hint() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let response = run_program(home.path(), work.path(), "echo hello");
    assert!(exec(&response)["fields"].get("read_hint").is_none());
    assert_eq!(field(exec(&response), "stdout").as_str(), Some("hello"));
}
