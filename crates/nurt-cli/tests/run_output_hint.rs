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

/// The command of the hint that starts with `word`: from the word to the end
/// of the quoted path that follows it.
fn command_of<'a>(hint: &'a str, word: &str) -> &'a str {
    let start = hint.find(word).unwrap_or_else(|| panic!("no `{word}` in {hint}"));
    let rest = &hint[start..];
    let open = rest.find('"').unwrap();
    let close = open + 1 + rest[open + 1..].find('"').unwrap();
    &rest[..=close]
}

/// Text of the `content` record a `read` returned.
fn read_text(response: &Value) -> String {
    let rows = &response["report"]["records"]["rows"];
    field(&rows[0], "text").as_str().unwrap().to_owned()
}

fn run_program(home: &Path, work: &Path, script: &str) -> Value {
    nurt(
        home,
        &format!(
            r#"run ["sh", "-c", "{script}"] cwd: "{}""#,
            work.display()
        ),
    )
}

#[test]
fn the_read_command_of_the_hint_returns_the_lines_the_response_left_out() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let response = run_program(home.path(), work.path(), "seq 1 3000; exit 1");
    let record = exec(&response);
    let hint = field(record, "read_hint").as_str().unwrap().to_owned();
    assert!(hint.starts_with("Output cut"), "{hint}");
    let text = field(record, "stdout").as_str().unwrap();
    assert!(text.starts_with("1\n2\n"), "{text}");
    assert!(text.len() <= 4096);
    let read = command_of(&hint, "read ");
    assert!(read.contains(" lines: 21..2960"), "{read}");
    let rest = nurt(home.path(), read);
    let lines = read_text(&rest);
    assert_eq!(lines.lines().next(), Some("21"));
    assert_eq!(lines.lines().last(), Some("2960"));
    assert_eq!(lines.lines().count(), 2940);
}

#[test]
fn a_range_taken_from_the_hint_and_a_search_of_the_hint_both_work() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let response = run_program(
        home.path(),
        work.path(),
        "seq 1 500; echo FAILED test_middle; seq 501 1000; exit 1",
    );
    let hint = field(exec(&response), "read_hint")
        .as_str()
        .unwrap()
        .to_owned();
    let read = command_of(&hint, "read ");
    let path = read.split('"').nth(1).unwrap();
    let narrow = read.replace(" lines: 21..960", " lines: 499..503");
    assert_ne!(narrow, read, "{read}");
    assert_eq!(
        read_text(&nurt(home.path(), &narrow)),
        "499\n500\nFAILED test_middle\n501\n502\n"
    );
    let search = command_of(&hint, "search ");
    let found = nurt(home.path(), search);
    let rows = found["report"]["records"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{found}");
    assert_eq!(field(&rows[0], "no").as_i64(), Some(501));
    assert!(field(&rows[0], "path").as_str().unwrap().ends_with(path.rsplit('/').next().unwrap()));
}
