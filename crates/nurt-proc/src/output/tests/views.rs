// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stream selection, filters, head and tail, summary and decoding of
//! responses.

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::OutputShown;
use nurt_contracts::report::Notable;
use nurt_contracts::stage::OutputStream;

use super::{Bench, numbered, view};
use crate::iface::{OutputView, ShowMode};

fn shown(head: u64, tail: u64, omitted_lines: u64, filtered: u64) -> Option<OutputShown> {
    Some(OutputShown {
        head,
        tail,
        omitted_lines,
        filtered,
    })
}

fn show(mode: ShowMode) -> OutputView {
    OutputView {
        show: mode,
        ..view()
    }
}

/// Thirty lines with an error line in the middle.
fn long_failure() -> String {
    numbered("step", 30).replace("step 15\n", "error: broken\n")
}

#[test]
fn output_show_auto_returns_stdout_after_success() {
    let bench = Bench::new();
    let store = bench.finished("auto-ok", b"one\ntwo\n", b"note\n");
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout, "one\ntwo");
    assert_eq!(out.stderr, "");
    assert_eq!(out.shown.stdout, shown(2, 0, 0, 0));
    assert_eq!(out.shown.stderr, None);
    assert_eq!((out.stdout_bytes, out.stderr_bytes), (8, 5));
    assert_eq!((out.stdout_lines, out.stderr_lines), (2, 1));
    assert_eq!(out.decoded_as, "utf-8");
    assert!(out.stdout_file.is_some() && out.stderr_file.is_some());
    assert_eq!(out.output_file, None);
    assert_eq!(out.output_sha256.as_ref().map(String::len), Some(64));
}

#[test]
fn output_show_auto_adds_stderr_after_failure() {
    let bench = Bench::new();
    let store = bench.finished("auto-fail", b"one\n", b"boom\n");
    let out = store.render(&view(), Some(1)).unwrap();
    assert_eq!(out.stdout, "one");
    assert_eq!(out.stderr, "boom");
    let signalled = store.render(&view(), None).unwrap();
    assert_eq!(signalled.stderr, "boom");
}

#[test]
fn output_show_auto_shows_stderr_when_stdout_is_empty() {
    let bench = Bench::new();
    let store = bench.finished("auto-empty", b"", b"only\n");
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "only");
}

#[test]
fn output_show_auto_returns_error_lines_of_long_failed_output() {
    let bench = Bench::new();
    let store = bench.finished("auto-errors", long_failure().as_bytes(), b"");
    let out = store.render(&view(), Some(2)).unwrap();
    assert_eq!(
        out.stdout,
        "step 13\nstep 14\nerror: broken\nstep 16\nstep 17"
    );
    assert_eq!(out.shown.stdout, shown(5, 0, 0, 25));
    let success = store.render(&view(), Some(0)).unwrap();
    assert_eq!(success.stdout.lines().count(), 30);
}

#[test]
fn output_show_auto_returns_short_failed_output_whole() {
    let bench = Bench::new();
    let text = numbered("line", 10).replace("line 5\n", "error: short\n");
    let store = bench.finished("auto-short", text.as_bytes(), b"");
    let out = store.render(&view(), Some(1)).unwrap();
    assert_eq!(out.stdout.lines().count(), 10);
}

#[test]
fn output_show_both_stdout_and_stderr_select_streams() {
    let bench = Bench::new();
    let store = bench.finished("streams", b"out\n", b"err\n");
    let both = store.render(&show(ShowMode::Both), Some(0)).unwrap();
    assert_eq!((both.stdout.as_str(), both.stderr.as_str()), ("out", "err"));
    let stdout = store.render(&show(ShowMode::Stdout), Some(1)).unwrap();
    assert_eq!(
        (stdout.stdout.as_str(), stdout.stderr.as_str()),
        ("out", "")
    );
    assert_eq!(stdout.shown.stderr, None);
    let stderr = store.render(&show(ShowMode::Stderr), Some(0)).unwrap();
    assert_eq!(
        (stderr.stdout.as_str(), stderr.stderr.as_str()),
        ("", "err")
    );
    assert_eq!(stderr.shown.stdout, None);
}

#[test]
fn output_show_errors_returns_error_lines_and_stderr() {
    let bench = Bench::new();
    let store = bench.finished("errors", b"a\nb\nc\nd\ne\nerror: x\nf\n", b"e1\ne2\n");
    let out = store.render(&show(ShowMode::Errors), Some(0)).unwrap();
    assert_eq!(out.stdout, "d\ne\nerror: x\nf");
    assert_eq!(out.stderr, "e1\ne2");
}

#[test]
fn output_show_errors_falls_back_to_last_lines() {
    let bench = Bench::new();
    let store = bench.finished("no-errors", numbered("x", 50).as_bytes(), b"");
    let out = store.render(&show(ShowMode::Errors), Some(1)).unwrap();
    assert!(out.stdout.starts_with("x 11\n"));
    assert!(out.stdout.ends_with("x 50"));
    assert_eq!(out.shown.stdout, shown(0, 40, 10, 0));
}

#[test]
fn output_show_summary_returns_summary_lines_and_last_lines() {
    let bench = Bench::new();
    let text = format!("{}ok: 12 passed\n", numbered("l", 11));
    let store = bench.finished("summary", text.as_bytes(), b"");
    let summary = OutputView {
        summary: vec!["passed".to_owned()],
        ..show(ShowMode::Summary)
    };
    let out = store.render(&summary, Some(0)).unwrap();
    assert_eq!(out.stdout, "l 8\nl 9\nl 10\nl 11\nok: 12 passed");
    assert_eq!(out.summary_lines, ["ok: 12 passed"]);
}

#[test]
fn output_show_none_keeps_counters_and_summary() {
    let bench = Bench::new();
    let store = bench.finished("none", numbered("l", 12).as_bytes(), b"e\n");
    let none = OutputView {
        summary: vec!["l 1[0-9]".to_owned()],
        ..show(ShowMode::None)
    };
    let out = store.render(&none, Some(1)).unwrap();
    assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("", ""));
    assert_eq!(out.shown.stdout, None);
    assert_eq!((out.stdout_lines, out.stderr_lines), (12, 1));
    assert_eq!(out.summary_lines, ["l 12"]);
    assert!(out.output_cursor.is_some());
}

#[test]
fn output_head_tail_marks_omitted_lines() {
    let bench = Bench::new();
    let store = bench.finished("head-tail", numbered("n", 100).as_bytes(), b"");
    let narrow = OutputView {
        head: Some(3),
        tail: Some(2),
        ..view()
    };
    let out = store.render(&narrow, Some(0)).unwrap();
    assert_eq!(out.stdout, "n 1\nn 2\nn 3\n...\nn 99\nn 100");
    assert_eq!(out.shown.stdout, shown(3, 2, 95, 0));
    let defaults = store.render(&view(), Some(0)).unwrap();
    assert_eq!(defaults.shown.stdout, shown(20, 40, 40, 0));
    let head_only = OutputView {
        head: Some(2),
        tail: Some(0),
        ..view()
    };
    let out = store.render(&head_only, Some(0)).unwrap();
    assert_eq!(out.stdout, "n 1\nn 2");
    let whole = bench.finished("short", numbered("n", 60).as_bytes(), b"");
    let out = whole.render(&view(), Some(0)).unwrap();
    assert_eq!(out.shown.stdout, shown(20, 40, 0, 0));
    assert!(!out.stdout.contains("..."));
}

#[test]
fn output_filter_context_merges_ranges_and_separates_groups() {
    let bench = Bench::new();
    let text = "a\nb\nmatch 1\nc\nmatch 2\nd\ne\nf\ng\nmatch 3\nh\n";
    let store = bench.finished("filter", text.as_bytes(), b"");
    let filter = OutputView {
        filter: Some("^match".to_owned()),
        context: 1,
        ..view()
    };
    let out = store.render(&filter, Some(0)).unwrap();
    assert_eq!(out.stdout, "b\nmatch 1\nc\nmatch 2\nd\n--\ng\nmatch 3\nh");
    assert_eq!(out.shown.stdout, shown(8, 0, 0, 3));
    let look_ahead = OutputView {
        filter: Some("match(?= 2)".to_owned()),
        ..view()
    };
    let out = store.render(&look_ahead, Some(0)).unwrap();
    assert_eq!(out.stdout, "match 2");
    let invalid = OutputView {
        filter: Some("(".to_owned()),
        ..view()
    };
    let error = store.render(&invalid, Some(0)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Regex);
}

#[test]
fn output_dedupe_collapses_consecutive_repeats() {
    let bench = Bench::new();
    let store = bench.finished("dedupe", b"x\nx\nx\ny\n\n\ny\n", b"");
    let dedupe = OutputView {
        dedupe: true,
        ..view()
    };
    let out = store.render(&dedupe, Some(0)).unwrap();
    assert_eq!(out.stdout, "x (\u{d7}3)\ny\n\n\ny");
    assert_eq!(out.shown.stdout, shown(5, 0, 0, 2));
    let plain = store.render(&view(), Some(0)).unwrap();
    assert_eq!(plain.stdout, "x\nx\nx\ny\n\n\ny");
}

#[test]
fn output_quiet_hides_output_after_success_only() {
    let bench = Bench::new();
    let store = bench.finished("quiet", b"built\n", b"");
    let quiet = OutputView {
        quiet_on_success: true,
        ..view()
    };
    let success = store.render(&quiet, Some(0)).unwrap();
    assert_eq!(success.stdout, "");
    assert_eq!(success.stdout_lines, 1);
    let failure = store.render(&quiet, Some(2)).unwrap();
    assert_eq!(failure.stdout, "built");
}

#[test]
fn output_summary_progress_reports_last_matches() {
    let bench = Bench::new();
    let store = bench.finished(
        "progress",
        b"Compiling a\n[1/4] 25%\n[3/4] 75%\ntest result: ok. 5 passed\n",
        b"Finished release\n",
    );
    let watch = OutputView {
        summary: vec![
            "passed".to_owned(),
            "^Finished".to_owned(),
            "absent".to_owned(),
        ],
        progress: Some(r"(\d+)%".to_owned()),
        ..view()
    };
    let out = store.render(&watch, Some(0)).unwrap();
    assert_eq!(
        out.summary_lines,
        ["test result: ok. 5 passed", "Finished release"]
    );
    let progress = out.progress.unwrap();
    assert_eq!(progress.percent, Some(75.0));
    assert_eq!(progress.line, "[3/4] 75%");
}

#[test]
fn output_decode_utf8_split_across_writes() {
    let bench = Bench::new();
    let store = bench.create("split", false);
    let bytes = "za\u{17c}\u{f3}\u{142}\u{107}\n".as_bytes();
    for part in bytes.chunks(1) {
        store.write(OutputStream::Stdout, part).unwrap();
    }
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout, "za\u{17c}\u{f3}\u{142}\u{107}");
    assert_eq!(out.decoded_as, "utf-8");
}

#[test]
fn output_decode_utf16_counts_lines_by_code_units() {
    let bench = Bench::new();
    for (id, little, name) in [("le", true, "utf-16le"), ("be", false, "utf-16be")] {
        let bytes: Vec<u8> = "one\n\u{a0a}two\n"
            .encode_utf16()
            .flat_map(|unit| {
                if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                }
            })
            .collect();
        let store = bench.finished(id, &bytes, b"");
        let out = store.render(&view(), Some(0)).unwrap();
        assert_eq!(out.stdout, "one\n\u{a0a}two", "{name}");
        assert_eq!(out.stdout_lines, 2, "{name}");
        assert_eq!(out.decoded_as, name);
    }
}

#[test]
fn output_decode_cp852_and_cp1250_on_request() {
    let bench = Bench::new();
    let cp852 = [0x5a, 0x61, 0xbe, 0xa2, 0x88, 0x86, b'\n'];
    let store = bench.finished("cp852", &cp852, b"");
    let requested = OutputView {
        encoding: Some("cp852".to_owned()),
        ..view()
    };
    let out = store.render(&requested, Some(0)).unwrap();
    assert_eq!(out.stdout, "Za\u{17c}\u{f3}\u{142}\u{107}");
    assert_eq!(out.decoded_as, "ibm852");
    let (cp1250, _, _) = encoding_rs::WINDOWS_1250.encode("\u{15a}wi\u{119}ty\n");
    let store = bench.finished("cp1250", &cp1250, b"");
    let requested = OutputView {
        encoding: Some("cp1250".to_owned()),
        ..view()
    };
    let out = store.render(&requested, Some(0)).unwrap();
    assert_eq!(out.stdout, "\u{15a}wi\u{119}ty");
    assert_eq!(out.decoded_as, "windows-1250");
    let unknown = OutputView {
        encoding: Some("nope".to_owned()),
        ..view()
    };
    let error = store.render(&unknown, Some(0)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn output_decode_binary_describes_bytes() {
    let bench = Bench::new();
    let bytes: Vec<u8> = (0u8..=255).collect();
    let store = bench.finished("binary", &bytes, b"text\n");
    let out = store.render(&show(ShowMode::Both), Some(0)).unwrap();
    assert_eq!(out.stdout, "binary output, 256 bytes");
    assert_eq!(out.stderr, "text");
    assert_eq!(out.decoded_as, "binary");
    let note = out
        .notes
        .iter()
        .find(|note| note.code == Notable::OutputBinary)
        .unwrap();
    assert_eq!(note.count, Some(256));
}

#[test]
fn removes_terminal_sequences_and_shortens_long_lines() {
    let bench = Bench::new();
    let long = "a".repeat(70_000);
    let text = format!("\u{1b}[32mok\u{1b}[0m\n{long}\n");
    let store = bench.finished("clean", text.as_bytes(), b"");
    let out = store.render(&view(), Some(0)).unwrap();
    let mut lines = out.stdout.lines();
    assert_eq!(lines.next(), Some("ok"));
    assert_eq!(
        lines.next(),
        Some(
            format!(
                "{}\u{2026} [+{} znak\u{f3}w]",
                "a".repeat(400),
                64 * 1024 - 400
            )
            .as_str()
        )
    );
    let note = out
        .notes
        .iter()
        .find(|note| note.code == Notable::TruncatedLines)
        .unwrap();
    assert_eq!(note.count, Some(1));
}
