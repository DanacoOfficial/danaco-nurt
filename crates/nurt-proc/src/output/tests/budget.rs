// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::output::tests::budget
// Responsibility:   The byte budget of a response, the cutting of long lines, the short result of a success and the errors of known tools.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The byte budget of a response, the cutting of long lines, the short
//! result of a success, the errors of known tools and the hint that reads
//! the rest from the output file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use nurt_contracts::report::Notable;

use super::{Bench, view};
use crate::iface::{OutputView, ShowMode};

/// `count` distinct lines of about `width` bytes with the prefix `name`.
fn numbered(name: &str, count: usize, width: usize) -> Vec<u8> {
    let mut text = String::new();
    for number in 0..count {
        let line = format!("{name} {number:05} ");
        text.push_str(&line);
        text.push_str(&"x".repeat(width.saturating_sub(line.len())));
        text.push('\n');
    }
    text.into_bytes()
}

fn budgeted(max_bytes: u32) -> OutputView {
    OutputView {
        max_bytes,
        head: Some(1000),
        tail: Some(1000),
        ..view()
    }
}

#[test]
fn text_of_both_streams_fits_the_budget_together() {
    let bench = Bench::new();
    let store = bench.finished("both", &numbered("out", 3000, 60), &numbered("err", 30, 60));
    let both = OutputView {
        show: ShowMode::Both,
        ..budgeted(4096)
    };
    let out = store.render(&both, Some(1)).unwrap();
    assert!(out.stdout.len() + out.stderr.len() <= 4096);
    assert_eq!(
        out.stderr.lines().count(),
        30,
        "standard error is served first"
    );
    assert!(out.stdout.contains("out 02999"));
    assert!(!out.stdout.is_empty());
}

#[test]
fn a_large_standard_error_takes_the_whole_budget_before_standard_output() {
    let bench = Bench::new();
    let store = bench.finished(
        "err-first",
        &numbered("out", 300, 60),
        &numbered("err", 300, 60),
    );
    let both = OutputView {
        show: ShowMode::Both,
        ..budgeted(4096)
    };
    let out = store.render(&both, Some(1)).unwrap();
    assert!(out.stderr.contains("err 00299"));
    assert!(out.stderr.len() <= 4096);
    assert!(out.stderr.len() + out.stdout.len() <= 4096);
    assert!(!out.stdout.contains("out 00299"));
}

#[test]
fn the_end_of_the_output_wins_over_the_beginning() {
    let bench = Bench::new();
    let store = bench.finished("end", &numbered("row", 200, 100), b"");
    let out = store.render(&budgeted(1000), Some(1)).unwrap();
    assert!(out.stdout.len() <= 1000, "{}", out.stdout.len());
    assert!(out.stdout.ends_with(&"x".repeat(50)));
    assert!(out.stdout.contains("row 00199"));
    assert!(out.stdout.contains("row 00198"));
    let shown = out.shown.stdout.unwrap();
    assert!(shown.omitted_lines > 150);
    assert!(shown.tail >= shown.head);
}

#[test]
fn a_budget_of_zero_is_no_limit() {
    let bench = Bench::new();
    let store = bench.finished("zero", &numbered("row", 300, 100), b"");
    let out = store.render(&budgeted(0), Some(1)).unwrap();
    assert_eq!(out.stdout.lines().count(), 300);
    assert_eq!(out.continuation, None);
}

#[test]
fn a_line_longer_than_400_characters_is_cut_and_marked() {
    let bench = Bench::new();
    let long = format!("{}\n", "a".repeat(1000));
    let store = bench.finished("long", long.as_bytes(), b"");
    let out = store.render(&budgeted(4096), Some(0)).unwrap();
    assert_eq!(
        out.stdout,
        format!("{}\u{2026} [+600 characters]", "a".repeat(400))
    );
    let notes = out
        .notes
        .iter()
        .find(|note| note.code == Notable::TruncatedLines)
        .unwrap();
    assert_eq!(notes.count, Some(1));
    let continuation = out.continuation.unwrap();
    assert_eq!(continuation.cut, [1]);
    assert!(continuation.first_line > continuation.last_line);
}

#[test]
fn a_single_line_longer_than_the_budget_is_clipped_to_it() {
    let bench = Bench::new();
    let store = bench.finished("clip", format!("{}\n", "b".repeat(380)).as_bytes(), b"");
    let out = store.render(&budgeted(200), Some(0)).unwrap();
    assert!(out.stdout.len() <= 200, "{}", out.stdout.len());
    assert!(out.stdout.contains("characters]"));
}

#[test]
fn steps_that_differ_only_in_names_collapse_into_one_line() {
    let bench = Bench::new();
    let mut text = String::new();
    for number in 0..300 {
        text.push_str(&format!("   Compiling crate{number} v1.{number}.0\n"));
    }
    text.push_str("    Finished `dev` profile\n");
    let store = bench.finished("steps", text.as_bytes(), b"");
    let collapsing = OutputView {
        dedupe: true,
        ..budgeted(4096)
    };
    let out = store.render(&collapsing, Some(0)).unwrap();
    assert_eq!(
        out.stdout,
        "   Compiling crate0 v1.0.0 (\u{d7}300)\n    Finished `dev` profile"
    );
    assert_eq!(out.shown.stdout.unwrap().omitted_lines, 0);
}

#[test]
fn identical_lines_collapse_and_the_hint_counts_the_lines_of_the_file() {
    let bench = Bench::new();
    let mut text = "hello\n".repeat(50_000);
    text.push_str("last\n");
    let store = bench.finished("same", text.as_bytes(), b"");
    let collapsing = OutputView {
        dedupe: true,
        ..budgeted(4096)
    };
    let out = store.render(&collapsing, Some(0)).unwrap();
    assert_eq!(out.stdout, "hello (\u{d7}50000)\nlast");
    assert_eq!(out.continuation, None);
}

#[test]
fn after_a_success_auto_returns_the_last_five_lines() {
    let bench = Bench::new();
    let store = bench.finished("success", &numbered("row", 200, 30), b"");
    let compact = OutputView {
        compact: true,
        max_bytes: 4096,
        head: Some(20),
        tail: Some(40),
        ..view()
    };
    let out = store.render(&compact, Some(0)).unwrap();
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines.len(), 5);
    assert!(lines[4].starts_with("row 00199"));
    let asked = OutputView {
        compact: false,
        ..compact.clone()
    };
    assert_eq!(
        store
            .render(&asked, Some(0))
            .unwrap()
            .stdout
            .lines()
            .count(),
        60 + 1
    );
    let failed = store.render(&compact, Some(1)).unwrap();
    assert!(failed.stdout.lines().count() > 5);
}

const CARGO_FAILURE: &str = "   Compiling fx v0.1.0 (/src/fx)\n\
warning: unused variable: `a`\n --> src/main.rs:1:5\n  |\nwarning: unused variable: `b`\n --> src/main.rs:2:5\n  |\n\
error[E0308]: mismatched types\n --> src/main.rs:41:21\n  |\n41 | let a: u32 = \"text\";\n  |\n\
error[E0425]: cannot find value `missing` in this scope\n --> src/main.rs:41:60\n  |\n\
warning: `fx` (bin \"fx\") generated 2 warnings\n\
error: could not compile `fx` (bin \"fx\") due to 2 previous errors; 2 warnings emitted\n";

#[test]
fn a_failed_cargo_run_reports_its_errors_instead_of_its_warnings() {
    let bench = Bench::new();
    let mut text = String::new();
    for number in 0..200 {
        text.push_str(&format!(
            "warning: unused variable: `v{number}`\n --> src/lib.rs:{number}:9\n  |\n"
        ));
    }
    text.push_str(CARGO_FAILURE);
    let store = bench.finished("cargo", b"", text.as_bytes());
    let out = store.render(&budgeted(4096), Some(101)).unwrap();
    let found = out.diagnostics.unwrap();
    assert_eq!(found.tool, "cargo");
    assert_eq!((found.errors, found.warnings), (2, 202));
    assert_eq!(found.first.len(), 2);
    assert_eq!(found.first[0].file.as_deref(), Some("src/main.rs"));
    assert_eq!(
        (found.first[0].line, found.first[0].col),
        (Some(41), Some(21))
    );
    assert_eq!(found.first[0].code.as_deref(), Some("E0308"));
    assert!(out.stderr.lines().count() <= 10);
    assert!(out.stderr.contains("could not compile"));
}

#[test]
fn a_successful_run_still_counts_the_findings_of_a_known_tool() {
    let bench = Bench::new();
    let text =
        "warning: unused variable: `a`\n --> src/main.rs:1:5\n    Finished `dev` profile in 1s\n";
    let store = bench.finished("cargo-ok", b"", text.as_bytes());
    let compact = OutputView {
        compact: true,
        ..budgeted(4096)
    };
    let found = store
        .render(&compact, Some(0))
        .unwrap()
        .diagnostics
        .unwrap();
    assert_eq!(
        (found.tool.as_str(), found.errors, found.warnings),
        ("cargo", 0, 1)
    );
}

const PYTEST_FAILURE: &str = "..F.\n=== FAILURES ===\n___ test_bad ___\n\n    def test_bad():\n>       assert 1 == 2\nE       assert 1 == 2\n\ntest_fx.py:64: AssertionError\n=== short test summary info ===\nFAILED test_fx.py::test_bad - assert 1 == 2\n=== 1 failed, 3 passed in 0.17s ===\n";

#[test]
fn a_failed_pytest_run_reports_the_failure_with_file_and_line() {
    let bench = Bench::new();
    let text = "tests/test_fx.py::test_ok PASSED\n".repeat(30) + PYTEST_FAILURE;
    let store = bench.finished("pytest", text.as_bytes(), b"");
    let out = store.render(&budgeted(4096), Some(1)).unwrap();
    let found = out.diagnostics.unwrap();
    assert_eq!((found.tool.as_str(), found.errors), ("pytest", 1));
    assert_eq!(found.first[0].file.as_deref(), Some("test_fx.py"));
    assert_eq!(found.first[0].line, Some(64));
}

#[test]
fn a_short_failed_output_is_shown_whole_without_diagnostics() {
    let bench = Bench::new();
    let store = bench.finished("pytest-short", PYTEST_FAILURE.as_bytes(), b"");
    let out = store.render(&budgeted(4096), Some(1)).unwrap();
    assert_eq!(out.diagnostics, None);
    assert_eq!(out.stdout.lines().count(), 12);
}

#[test]
fn the_budget_keeps_the_errors_before_the_text() {
    let bench = Bench::new();
    let mut text = String::new();
    for number in 0..40 {
        text.push_str(&format!(
            "error[E0308]: mismatched types {}\n --> src/main.rs:{number}:5\n",
            "y".repeat(80)
        ));
        text.push_str(&format!(
            "error[E0599]: no method number{number}\n --> src/b.rs:{number}:1\n"
        ));
    }
    let store = bench.finished("errors-first", b"", text.as_bytes());
    let out = store.render(&budgeted(1200), Some(101)).unwrap();
    let found = out.diagnostics.unwrap();
    assert!(!found.first.is_empty());
    let used: usize = found
        .first
        .iter()
        .map(super::super::formats::entry_bytes)
        .sum();
    assert!(
        used + out.stderr.len() <= 1200,
        "{used} {}",
        out.stderr.len()
    );
}

#[test]
fn the_continuation_names_the_lines_left_out_of_the_file() {
    let bench = Bench::new();
    let store = bench.finished("hint", &numbered("row", 300, 40), b"");
    let narrow = OutputView {
        head: Some(10),
        tail: Some(20),
        ..budgeted(4096)
    };
    let out = store.render(&narrow, Some(1)).unwrap();
    let continuation = out.continuation.unwrap();
    let (path, from, to) = (
        continuation.path.clone(),
        continuation.first_line,
        continuation.last_line,
    );
    assert_eq!(Some(path.clone()), out.stdout_file);
    assert_eq!((from, to), (11, 280));
    assert_eq!(continuation.shown, 30);
    let text = std::fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[from as usize - 1].starts_with("row 00010"));
    assert!(lines[to as usize - 1].starts_with("row 00279"));
}

#[test]
fn the_continuation_counts_lines_of_the_file_when_lines_were_collapsed() {
    let bench = Bench::new();
    let mut text = String::from("start\n");
    text.push_str(&"tick\n".repeat(100));
    for number in 0u8..50 {
        text.push_str(&format!(
            "item {}{}\n",
            char::from(b'a' + number / 26),
            char::from(b'a' + number % 26)
        ));
    }
    let store = bench.finished("hint-collapsed", text.as_bytes(), b"");
    let narrow = OutputView {
        dedupe: true,
        head: Some(2),
        tail: Some(3),
        ..budgeted(4096)
    };
    let out = store.render(&narrow, Some(1)).unwrap();
    assert!(out.stdout.starts_with("start\ntick (\u{d7}100)"));
    let continuation = out.continuation.unwrap();
    let (path, from, to) = (
        continuation.path,
        continuation.first_line,
        continuation.last_line,
    );
    assert_eq!((from, to), (102, 148));
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(text.lines().nth(from as usize - 1), Some("item aa"));
    assert_eq!(text.lines().nth(to as usize - 1), Some("item bu"));
}

#[test]
fn nothing_cut_means_no_continuation() {
    let bench = Bench::new();
    let store = bench.finished("whole", b"a\nb\n", b"");
    let out = store.render(&budgeted(4096), Some(1)).unwrap();
    assert_eq!(out.continuation, None);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
