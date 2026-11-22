// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Incremental reads with output cursors.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::stage::OutputStream;

use super::{Bench, numbered, view};
use crate::iface::{OutputStore, OutputView, ShowMode};

/// Deterministic sequence of chunk lengths from 1 to `max`.
struct ChunkSizes {
    state: u64,
    max: u64,
}

impl Iterator for ChunkSizes {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
        const INCREMENT: u64 = 1_442_695_040_888_963_407;
        self.state = self.state.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
        usize::try_from((self.state >> 33) % self.max + 1).ok()
    }
}

/// Reads with `cursor` until the output ends, returning the lines and the
/// last cursor.
fn read_all(
    store: &dyn OutputStore,
    view: &OutputView,
    mut cursor: Option<String>,
    done: &AtomicBool,
) -> (Vec<String>, Option<String>) {
    let mut lines = Vec::new();
    loop {
        let ended = done.load(Ordering::Acquire);
        let out = store.read_since(cursor.as_deref(), view).unwrap();
        let progressed = out.output_cursor != cursor;
        lines.extend(
            out.stdout
                .lines()
                .map(str::to_owned)
                .filter(|line| !line.is_empty()),
        );
        cursor = out.output_cursor;
        if ended && !progressed {
            return (lines, cursor);
        }
        std::thread::yield_now();
    }
}

#[test]
fn output_cursor_concurrent_reads_lose_and_repeat_nothing() {
    let bench = Bench::new();
    let store = bench.create("concurrent", false);
    let text = numbered("record", 5_000);
    let writer = {
        let store = Arc::clone(&store);
        let bytes = text.clone().into_bytes();
        std::thread::spawn(move || {
            let mut rest = bytes.as_slice();
            let sizes = ChunkSizes { state: 7, max: 97 };
            for size in sizes {
                if rest.is_empty() {
                    break;
                }
                let (chunk, tail) = rest.split_at(size.min(rest.len()));
                store.write(OutputStream::Stdout, chunk).unwrap();
                rest = tail;
            }
            store.finish(OutputStream::Stdout).unwrap();
            store.finish(OutputStream::Stderr).unwrap();
        })
    };
    let done = AtomicBool::new(false);
    let reader = std::thread::scope(|scope| {
        let handle = scope.spawn(|| read_all(store.as_ref(), &view(), None, &done));
        writer.join().unwrap();
        done.store(true, Ordering::Release);
        handle.join().unwrap()
    });
    let expected: Vec<String> = text.lines().map(str::to_owned).collect();
    assert_eq!(reader.0, expected);
}

#[test]
fn holds_back_partial_line_while_program_runs() {
    let bench = Bench::new();
    let store = bench.create("partial", false);
    store.write(OutputStream::Stdout, b"first\nsec").unwrap();
    let first = store.read_since(None, &view()).unwrap();
    assert_eq!(first.stdout, "first");
    let cursor = first.output_cursor.unwrap();
    let again = store.read_since(Some(&cursor), &view()).unwrap();
    assert_eq!(again.stdout, "");
    assert_eq!(again.output_cursor.as_deref(), Some(cursor.as_str()));
    store.write(OutputStream::Stdout, b"ond").unwrap();
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let last = store.read_since(Some(&cursor), &view()).unwrap();
    assert_eq!(last.stdout, "second");
}

#[test]
fn continues_long_output_over_several_reads() {
    let bench = Bench::new();
    let text = numbered("line with some padding to fill the budget", 60_000);
    let store = bench.finished("long", text.as_bytes(), b"");
    let done = AtomicBool::new(true);
    let (lines, _) = read_all(store.as_ref(), &view(), None, &done);
    assert_eq!(lines.len(), 60_000);
    let first = store.read_since(None, &view()).unwrap();
    assert!(first.stdout.lines().count() < 60_000);
}

#[test]
fn reads_selected_stream_and_applies_filters() {
    let bench = Bench::new();
    let store = bench.finished("select", b"a\nkeep 1\nb\n", b"e\nkeep 2\n");
    let stderr = OutputView {
        show: ShowMode::Stderr,
        ..view()
    };
    let out = store.read_since(None, &stderr).unwrap();
    assert_eq!(
        (out.stdout.as_str(), out.stderr.as_str()),
        ("", "e\nkeep 2")
    );
    let cursor = out.output_cursor.unwrap();
    let rest = store.read_since(Some(&cursor), &view()).unwrap();
    assert_eq!(
        (rest.stdout.as_str(), rest.stderr.as_str()),
        ("a\nkeep 1\nb", "")
    );
    let filtered = OutputView {
        filter: Some("keep".to_owned()),
        ..view()
    };
    let out = store.read_since(None, &filtered).unwrap();
    assert_eq!(
        (out.stdout.as_str(), out.stderr.as_str()),
        ("keep 1", "keep 2")
    );
    let limited = OutputView {
        tail: Some(1),
        ..view()
    };
    let out = store.read_since(None, &limited).unwrap();
    assert_eq!(out.stdout, "b");
}

#[test]
fn render_cursor_points_after_whole_output() {
    let bench = Bench::new();
    let store = bench.create("after", true);
    store.write(OutputStream::Stdout, b"one\n").unwrap();
    store.write(OutputStream::Stderr, b"two\n").unwrap();
    let rendered = store.render(&view(), None).unwrap();
    assert_eq!(rendered.stdout, "one\ntwo");
    store.write(OutputStream::Stdout, b"three\n").unwrap();
    let next = store
        .read_since(rendered.output_cursor.as_deref(), &view())
        .unwrap();
    assert_eq!(next.stdout, "three");
}

#[test]
fn rejects_cursor_of_other_spool_and_past_end() {
    let bench = Bench::new();
    let first = bench.finished("first", b"a\n", b"");
    let second = bench.finished("second", b"b\n", b"");
    let cursor = first
        .render(&view(), Some(0))
        .unwrap()
        .output_cursor
        .unwrap();
    let error = second.read_since(Some(&cursor), &view()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    let tag = cursor.split('.').next().unwrap();
    let past = format!("{tag}.999.1.0.0");
    let error = first.read_since(Some(&past), &view()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}
