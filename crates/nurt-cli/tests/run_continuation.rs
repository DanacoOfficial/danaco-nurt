// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/run_continuation.rs
// Responsibility:   End-to-end tests of the continuation of shortened results.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! End-to-end tests of the continuation of shortened results: the field
//! `continuation`, the text `more: next` and the command `next` with its
//! short forms, its sources (the output file of `run`, the kept records of a
//! reading command), several open continuations and the limits of
//! `[continuation]`. Every call is a separate process; the calls of a test
//! share one state directory.

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

/// Output of one call of the program `nurt` with its own state directory.
fn call(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// JSON of the response to a program.
fn nurt(home: &Path, program: &str) -> Value {
    let output = call(home, &["--json", "--limit", "1000", program]);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// JSON of the response to a program with the default size of a page.
fn nurt_page(home: &Path, program: &str) -> Value {
    let output = call(home, &["--json", program]);
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Text of the response to a program.
fn text(home: &Path, program: &str) -> String {
    String::from_utf8_lossy(&call(home, &[program]).stdout).into_owned()
}

fn report(response: &Value) -> &Value {
    &response["report"]
}

fn continuation(response: &Value) -> Option<&Value> {
    report(response).get("continuation")
}

fn field<'a>(record: &'a Value, name: &str) -> &'a Value {
    &record["fields"][name]["v"]
}

fn rows(response: &Value) -> &[Value] {
    report(response)["records"]["rows"]
        .as_array()
        .map_or(&[], Vec::as_slice)
}

/// The texts of the `line` records of a part.
fn line_texts(response: &Value) -> Vec<String> {
    rows(response)
        .iter()
        .map(|row| field(row, "text").as_str().unwrap().to_owned())
        .collect()
}

/// The values of one field of the records of a part.
fn values(response: &Value, name: &str) -> Vec<String> {
    rows(response)
        .iter()
        .map(|row| field(row, name).to_string())
        .collect()
}

fn run_sh(home: &Path, script: &str) -> Value {
    nurt(
        home,
        &format!(r#"run ["sh", "-c", "{script}"] cwd: "{}""#, home.display()),
    )
}

/// A directory with `count` empty files `f1`, `f2`, …
fn files(home: &Path, count: u32) -> String {
    let dir = home.join("files");
    std::fs::create_dir_all(&dir).unwrap();
    for number in 1..=count {
        std::fs::write(dir.join(format!("f{number}")), format!("n{number}\n")).unwrap();
    }
    dir.display().to_string()
}

/// Writes the configuration file of the account.
fn configure(home: &Path, text: &str) {
    let dir = home.join("config").join("nurt");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("nurt.toml"), text).unwrap();
}

/// Reads parts with `next` until none remains and returns the parts.
fn read_all(home: &Path, call_text: &str) -> Vec<Value> {
    let mut parts = Vec::new();
    loop {
        let part = nurt_page(home, call_text);
        let more = continuation(&part).is_some_and(|c| c["remaining"] != 0);
        parts.push(part);
        if !more {
            return parts;
        }
        assert!(parts.len() < 400, "the continuation does not end");
    }
}

#[test]
fn a_shortened_run_carries_the_continuation_and_the_hint() {
    let home = tempfile::tempdir().unwrap();
    let response = run_sh(home.path(), "seq 1 3000; exit 1");
    let c = continuation(&response).unwrap();
    assert_eq!(c["id"], "c1");
    assert_eq!(c["unit"], "lines");
    assert_eq!(c["shown"], 60);
    assert_eq!(c["remaining"], 2940);
    assert_eq!(c["cut_lines"], 0);
    assert!(
        response["report"]["exec"]["fields"]
            .get("read_hint")
            .is_none()
    );
    let shown = text(
        home.path(),
        &format!(
            r#"run ["sh", "-c", "seq 1 3000; exit 1"] cwd: "{}""#,
            home.path().display()
        ),
    );
    assert_eq!(shown.lines().last(), Some("more: next"));
}

#[test]
fn next_reads_the_lines_left_out_until_nothing_remains() {
    let home = tempfile::tempdir().unwrap();
    run_sh(home.path(), "seq 1 3000; exit 1");
    let parts = read_all(home.path(), "next");
    let mut texts = Vec::new();
    for part in &parts {
        texts.extend(line_texts(part));
    }
    let expected: Vec<String> = (21..=2960).map(|number| number.to_string()).collect();
    assert_eq!(texts, expected);
    assert!(parts.len() > 2);
    let last = parts.last().unwrap();
    assert_eq!(continuation(last).unwrap()["remaining"], 0);
    let first = &parts[0];
    assert_eq!(continuation(first).unwrap()["id"], "c1");
    assert_eq!(continuation(first).unwrap()["unit"], "lines");
}

#[test]
fn the_continuation_of_a_part_keeps_the_id_and_counts_what_was_given() {
    let home = tempfile::tempdir().unwrap();
    run_sh(home.path(), "seq 1 3000; exit 1");
    let part = nurt_page(home.path(), "next");
    let c = continuation(&part).unwrap();
    assert_eq!(c["id"], "c1");
    assert_eq!(c["shown"], 120);
    assert_eq!(c["remaining"], 2880);
    let first = text(home.path(), "next");
    assert!(
        first.lines().next().unwrap().ends_with("more: next"),
        "{first}"
    );
}

#[test]
fn next_all_returns_the_rest_and_the_short_forms_equal_the_long_forms() {
    let long = tempfile::tempdir().unwrap();
    let short = tempfile::tempdir().unwrap();
    for home in [&long, &short] {
        run_sh(home.path(), "seq 1 3000; exit 1");
    }
    let all_long = nurt(long.path(), "next all: true");
    let all_short = nurt(short.path(), "next all");
    assert_eq!(line_texts(&all_long), line_texts(&all_short));
    assert_eq!(line_texts(&all_long).len(), 2940);
    assert_eq!(continuation(&all_long).unwrap()["remaining"], 0);
    assert_eq!(continuation(&all_short).unwrap()["remaining"], 0);
}

#[test]
fn a_count_and_an_identifier_are_positional() {
    let long = tempfile::tempdir().unwrap();
    let short = tempfile::tempdir().unwrap();
    for home in [&long, &short] {
        run_sh(home.path(), "seq 1 3000; exit 1");
        run_sh(home.path(), "seq 5001 8000; exit 1");
    }
    let by_count = nurt(long.path(), "next count: 200");
    let by_short = nurt(short.path(), "next 200");
    assert_eq!(line_texts(&by_count).len(), 200);
    assert_eq!(line_texts(&by_count), line_texts(&by_short));
    assert_eq!(line_texts(&by_short)[0], "5021");
    let named = nurt(long.path(), "next c1 count: 200");
    let named_short = nurt(short.path(), "next c1 200");
    assert_eq!(line_texts(&named).len(), 200);
    assert_eq!(line_texts(&named), line_texts(&named_short));
    assert_eq!(line_texts(&named)[0], "21");
    assert_eq!(continuation(&named).unwrap()["id"], "c1");
}

#[test]
fn full_returns_the_cut_lines_whole() {
    let long = tempfile::tempdir().unwrap();
    let short = tempfile::tempdir().unwrap();
    let script = "head -c 1000 /dev/zero | tr '\\\\0' x; echo; seq 1 3000; exit 1";
    for home in [&long, &short] {
        let response = run_sh(home.path(), script);
        let c = continuation(&response).unwrap();
        assert_eq!(c["cut_lines"], 1, "{c}");
    }
    let by_flag = nurt(long.path(), "next full_lines: true");
    let by_word = nurt(short.path(), "next full");
    let lines = line_texts(&by_word);
    assert_eq!(lines, line_texts(&by_flag));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "x".repeat(1000));
    let after = nurt_page(short.path(), "next");
    assert_eq!(line_texts(&after)[0], "20");
}

#[test]
fn two_continuations_are_used_alternately() {
    let home = tempfile::tempdir().unwrap();
    run_sh(home.path(), "seq 1 3000; exit 1");
    let second = run_sh(home.path(), "seq 5001 8000; exit 1");
    assert_eq!(continuation(&second).unwrap()["id"], "c2");
    let first_part = nurt_page(home.path(), "next c1");
    assert_eq!(line_texts(&first_part)[0], "21");
    let other_part = nurt_page(home.path(), "next c2");
    assert_eq!(line_texts(&other_part)[0], "5021");
    let again = nurt_page(home.path(), "next c1");
    assert_eq!(line_texts(&again)[0], "81");
    let again_other = nurt_page(home.path(), "next c2");
    assert_eq!(line_texts(&again_other)[0], "5081");
    let named = text(home.path(), "next c1");
    assert!(
        named.lines().next().unwrap().ends_with("more: next c1"),
        "{named}"
    );
}

#[test]
fn a_selection_of_lines_continues_from_the_first_line_of_the_file() {
    let home = tempfile::tempdir().unwrap();
    let response = run_sh(
        home.path(),
        "seq 1 100; echo ERROR step FAILED; seq 101 200; exit 1",
    );
    let c = continuation(&response).unwrap();
    assert_eq!(c["unit"], "lines");
    let part = nurt(home.path(), "next all");
    let texts = line_texts(&part);
    assert_eq!(texts.len(), 201);
    assert_eq!(texts[100], "ERROR step FAILED");
}

#[test]
fn a_short_output_has_no_continuation() {
    let home = tempfile::tempdir().unwrap();
    let response = run_sh(home.path(), "seq 1 5");
    assert!(continuation(&response).is_none());
    let error = nurt(home.path(), "next");
    assert_eq!(error["report"]["error"]["code"], "E_CONTINUATION_EXPIRED");
}

#[test]
fn the_parts_of_a_listing_equal_the_listing_without_a_limit() {
    let home = tempfile::tempdir().unwrap();
    let dir = files(home.path(), 130);
    let program = format!(r#"list "{dir}""#);
    let whole = values(&nurt(home.path(), &program), "path");
    assert_eq!(whole.len(), 130);
    let first = nurt_page(home.path(), &program);
    let c = continuation(&first).unwrap();
    assert_eq!(c["id"], "c1");
    assert_eq!(c["unit"], "records");
    assert_eq!(c["shown"], 50);
    assert_eq!(c["remaining"], 80);
    assert!(text(home.path(), &program).contains("more: next"));
    let mut paths = values(&first, "path");
    let parts = read_all(home.path(), "next");
    assert_eq!(parts.len(), 2);
    for part in &parts {
        paths.extend(values(part, "path"));
    }
    assert_eq!(paths, whole);
    let last = parts.last().unwrap();
    assert_eq!(continuation(last).unwrap()["remaining"], 0);
}

#[test]
fn the_parts_of_a_search_and_a_find_equal_the_result_without_a_limit() {
    let home = tempfile::tempdir().unwrap();
    let dir = files(home.path(), 120);
    for (program, name) in [
        (format!(r#"search /n[0-9]+/ "{dir}""#), "text"),
        (format!(r#"find "{dir}" glob: "f*""#), "path"),
    ] {
        let whole = values(&nurt(home.path(), &program), name);
        assert_eq!(whole.len(), 120, "{program}");
        let first = nurt_page(home.path(), &program);
        assert!(continuation(&first).is_some(), "{program}");
        let mut found = values(&first, name);
        for part in read_all(home.path(), "next") {
            found.extend(values(&part, name));
        }
        assert_eq!(found, whole, "{program}");
    }
}

#[test]
fn the_parts_of_the_lines_of_a_file_equal_the_lines_without_a_limit() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("big.txt");
    let content: String = (1..=130).map(|number| format!("row {number}\n")).collect();
    std::fs::write(&file, &content).unwrap();
    let program = format!(r#"lines "{}""#, file.display());
    let whole = line_texts(&nurt(home.path(), &program));
    assert_eq!(whole.len(), 130);
    let first = nurt_page(home.path(), &program);
    assert_eq!(continuation(&first).unwrap()["unit"], "records");
    let mut found = line_texts(&first);
    for part in read_all(home.path(), "next") {
        found.extend(line_texts(&part));
    }
    assert_eq!(found, whole);
}

#[test]
fn the_part_is_a_result_that_a_pipeline_continues() {
    let home = tempfile::tempdir().unwrap();
    let dir = files(home.path(), 130);
    nurt_page(home.path(), &format!(r#"list "{dir}""#));
    let counted = nurt_page(home.path(), "next 40 |> count");
    assert_eq!(rows(&counted)[0]["fields"]["value"]["v"], 40);
}

#[test]
fn next_sizes_follow_the_count_and_the_default_is_the_first_response() {
    let home = tempfile::tempdir().unwrap();
    let dir = files(home.path(), 300);
    nurt_page(home.path(), &format!(r#"list "{dir}""#));
    assert_eq!(rows(&nurt_page(home.path(), "next 200")).len(), 200);
    assert_eq!(rows(&nurt_page(home.path(), "next")).len(), 50);
    assert_eq!(rows(&nurt_page(home.path(), "next c1 all")).len(), 0);
}

#[test]
fn a_continuation_expires_by_count() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path(), "[continuation]\nmax_count = 1\n");
    let dir = files(home.path(), 130);
    let program = format!(r#"list "{dir}""#);
    nurt_page(home.path(), &program);
    nurt_page(home.path(), &format!(r#"find "{dir}" glob: "f*""#));
    let gone = nurt_page(home.path(), "next c1");
    let error = &gone["report"]["error"];
    assert_eq!(error["code"], "E_CONTINUATION_EXPIRED");
    assert_eq!(error["next_steps"][0], program);
    assert!(continuation(&nurt_page(home.path(), "next c2")).is_some());
}

#[test]
fn a_continuation_expires_by_age() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path(), "[continuation]\nmax_age = \"1ms\"\n");
    let dir = files(home.path(), 130);
    let program = format!(r#"list "{dir}""#);
    nurt_page(home.path(), &program);
    let gone = nurt_page(home.path(), "next");
    let error = &gone["report"]["error"];
    assert_eq!(error["code"], "E_CONTINUATION_EXPIRED");
    assert_eq!(error["next_steps"][0], program);
}

#[test]
fn a_result_above_the_size_limit_is_not_announced_and_says_why() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path(), "[continuation]\nmax_size = \"1kib\"\n");
    let dir = files(home.path(), 130);
    let program = format!(r#"list "{dir}""#);
    let first = nurt_page(home.path(), &program);
    assert!(continuation(&first).is_none());
    let notable = report(&first)["notable"].as_array().unwrap();
    assert!(
        notable.iter().any(|entry| entry["detail"]
            .as_str()
            .is_some_and(|text| text.contains("max_size"))),
        "{notable:?}"
    );
    assert!(!text(home.path(), &program).contains("more: next"));
    let gone = nurt_page(home.path(), "next");
    assert_eq!(gone["report"]["error"]["code"], "E_CONTINUATION_EXPIRED");
}

#[test]
fn the_default_sizes_of_a_run_are_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let response = run_sh(home.path(), "seq 1 3000; exit 1");
    let stdout = response["report"]["exec"]["fields"]["stdout"]["v"]
        .as_str()
        .unwrap();
    assert!(stdout.starts_with("1\n2\n"));
    assert!(stdout.len() <= 4096);
}

#[test]
fn an_identifier_followed_by_a_word_equals_the_long_form() {
    let long = tempfile::tempdir().unwrap();
    let short = tempfile::tempdir().unwrap();
    for home in [&long, &short] {
        run_sh(home.path(), "seq 1 3000; exit 1");
        run_sh(home.path(), "seq 5001 8000; exit 1");
    }
    let all_long = nurt(long.path(), "next id: \"c1\" all: true");
    let all_short = nurt(short.path(), "next c1 all");
    assert_eq!(line_texts(&all_long), line_texts(&all_short));
    assert_eq!(line_texts(&all_short).len(), 2940);
    assert_eq!(line_texts(&all_short)[0], "21");
}

#[test]
fn an_identifier_followed_by_full_returns_the_cut_lines_of_that_continuation() {
    let long = tempfile::tempdir().unwrap();
    let short = tempfile::tempdir().unwrap();
    let script = "head -c 1000 /dev/zero | tr '\\\\0' x; echo; seq 1 3000; exit 1";
    for home in [&long, &short] {
        run_sh(home.path(), script);
        run_sh(home.path(), "seq 5001 8000; exit 1");
    }
    let by_flag = nurt(long.path(), "next id: \"c1\" full_lines: true");
    let by_word = nurt(short.path(), "next c1 full");
    assert_eq!(line_texts(&by_word), line_texts(&by_flag));
    assert_eq!(line_texts(&by_word), ["x".repeat(1000)]);
}

#[test]
fn the_notices_of_a_shortened_response_name_the_continuation() {
    let home = tempfile::tempdir().unwrap();
    let script = "head -c 1000 /dev/zero | tr '\\\\0' x; echo; seq 1 3000; exit 1";
    let response = run_sh(home.path(), script);
    let notable = report(&response)["notable"].as_array().unwrap();
    let truncated = notable
        .iter()
        .find(|entry| entry["code"] == "truncated_lines")
        .expect("a notice of cut lines");
    assert_eq!(truncated["continuation"], "c1");
    for entry in notable
        .iter()
        .filter(|entry| entry["code"] != "truncated_lines")
    {
        assert!(entry.get("continuation").is_none(), "{entry}");
    }
    let quiet = run_sh(home.path(), "seq 1 5");
    let none = report(&quiet)["notable"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(none.iter().all(|entry| entry.get("continuation").is_none()));
}

/// A file with `count` lines, of which every tenth is `width` characters long.
fn long_lines_file(home: &Path, count: u32, width: usize) -> std::path::PathBuf {
    let file = home.join("long.txt");
    let content: String = (1..=count)
        .map(|number| {
            if number % 10 == 0 {
                format!("hit{number}{}\n", "z".repeat(width))
            } else {
                format!("hit{number}\n")
            }
        })
        .collect();
    std::fs::write(&file, content).unwrap();
    file
}

#[test]
fn search_cut_lines_have_a_continuation_and_next_full_returns_them_whole() {
    let home = tempfile::tempdir().unwrap();
    let file = long_lines_file(home.path(), 30, 2000);
    let program = format!(r#"search /hit/ "{}""#, file.display());
    let first = nurt_page(home.path(), &program);
    let c = continuation(&first).expect("a response with cut lines continues");
    assert_eq!(c["cut_lines"], 3, "{c}");
    assert_eq!(c["remaining"], 0);
    assert!(
        text(home.path(), &program)
            .lines()
            .next()
            .unwrap()
            .ends_with("more: next")
    );
    let whole = nurt(home.path(), "next full");
    let lines = line_texts(&whole);
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|line| line.len() > 2000), "{lines:?}");
}

#[test]
fn read_cut_lines_have_a_continuation_and_next_full_returns_them_whole() {
    let home = tempfile::tempdir().unwrap();
    let file = long_lines_file(home.path(), 30, 2000);
    let program = format!(r#"read "{}" max_line: 100"#, file.display());
    let first = nurt_page(home.path(), &program);
    let c = continuation(&first).expect("a response with cut lines continues");
    assert_eq!(c["cut_lines"], 3, "{c}");
    let whole = nurt(home.path(), "next full");
    let lines = line_texts(&whole);
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|line| line.len() > 2000), "{lines:?}");
}

#[test]
fn run_continues_both_streams() {
    let home = tempfile::tempdir().unwrap();
    let script = "seq 1 3000; seq 5001 8000 >&2; exit 1";
    let response = nurt(
        home.path(),
        &format!(
            r#"run ["sh", "-c", "{script}"] cwd: "{}" merge_stderr: false"#,
            home.path().display()
        ),
    );
    let c = continuation(&response).unwrap();
    assert!(c["remaining"].as_i64().unwrap() > 4000, "{c}");
    let parts = read_all(home.path(), "next 1000");
    let mut texts = Vec::new();
    for part in &parts {
        texts.extend(line_texts(part));
    }
    assert!(texts.iter().any(|line| line == "2000"));
    assert!(texts.iter().any(|line| line == "7000"));
}

#[test]
fn next_after_the_last_part_gives_remaining_zero_and_no_hint() {
    let home = tempfile::tempdir().unwrap();
    let dir = files(home.path(), 60);
    nurt_page(home.path(), &format!(r#"list "{dir}""#));
    let last = nurt_page(home.path(), "next");
    assert_eq!(continuation(&last).unwrap()["remaining"], 0);
    assert!(!text(home.path(), "next").contains("more: next"));
}

#[test]
fn a_continuation_of_another_session_is_refused_and_next_leaves_no_journal_entry() {
    let home = tempfile::tempdir().unwrap();
    let dir = files(home.path(), 130);
    nurt_page(home.path(), &format!(r#"list "{dir}""#));
    let before = nurt(home.path(), "history");
    nurt_page(home.path(), "next");
    let after = nurt(home.path(), "history");
    assert_eq!(rows(&before).len(), rows(&after).len());
}

#[test]
fn an_expired_continuation_exits_with_the_argument_status() {
    let home = tempfile::tempdir().unwrap();
    let output = call(home.path(), &["next"]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_program_with_two_runs_continues_the_first_cut_one() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().display();
    let program = format!(
        r#"run ["sh", "-c", "seq 1 3000"] cwd: "{cwd}" |> run ["sh", "-c", "seq 5001 8000"] cwd: "{cwd}""#
    );
    let first = nurt(home.path(), &program);
    let again = nurt(home.path(), &program);
    assert_eq!(
        continuation(&first).map(|c| c["unit"].clone()),
        continuation(&again).map(|c| c["unit"].clone())
    );
}

#[test]
fn next_full_returns_the_text_of_the_response_even_when_the_file_changed() {
    let home = tempfile::tempdir().unwrap();
    let file = long_lines_file(home.path(), 30, 2000);
    let program = format!(r#"search /hit/ "{}""#, file.display());
    nurt_page(home.path(), &program);
    std::fs::write(&file, "edited\n").unwrap();
    let whole = nurt(home.path(), "next full");
    let lines = line_texts(&whole);
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|line| line.len() > 2000), "{lines:?}");
}

#[test]
fn next_full_of_a_utf16_file_returns_the_decoded_text() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("wide.txt");
    let long = format!("zażółć{}", "ć".repeat(300));
    let mut bytes = vec![0xff, 0xfe];
    for unit in format!("short\n{long}\nend\n").encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&file, bytes).unwrap();
    let program = format!(r#"read "{}" max_line: 50"#, file.display());
    let first = nurt_page(home.path(), &program);
    assert_eq!(continuation(&first).unwrap()["cut_lines"], 1);
    let whole = line_texts(&nurt(home.path(), "next full"));
    assert_eq!(whole, [long]);
}

#[test]
fn a_new_repl_still_sees_the_operations_made_before_it() {
    use std::io::Write;
    let home = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let file = home.path().join("made.txt");
    let made = nurt(
        home.path(),
        &format!(r#"write "{}" "hello" mode: create"#, file.display()),
    );
    assert_eq!(report(&made)["status"], "ok", "{made}");
    assert!(file.exists());
    let mut child = Command::new(BIN)
        .arg("repl")
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"undo\nexit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        !file.exists(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
