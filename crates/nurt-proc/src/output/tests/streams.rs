// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Output of real processes: the `proc_fixture` program writes, the tests
//! pass its pipes to the spool as process control does.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nurt_contracts::stage::OutputStream;

use super::{Bench, fixture_program, view};
use crate::iface::{OutputStore, OutputView, ShowMode};

/// Polish pangram written by the `emit-encoding` mode.
const SAMPLE: &str = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}";

/// Bytes passed to the spool per write, small enough to split characters.
const SPLIT_CHUNK: usize = 3;

/// Feeds everything readable from `pipe` to `store` as `stream`.
fn pump(mut pipe: impl Read, store: &dyn OutputStore, stream: OutputStream, chunk: usize) {
    let mut buffer = vec![0; chunk];
    loop {
        let read = pipe.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        store.write(stream, &buffer[..read]).unwrap();
    }
    store.finish(stream).unwrap();
}

/// Runs the fixture with `args` and passes standard output to a new spool.
fn capture(bench: &Bench, id: &str, args: &[&str]) -> Arc<dyn OutputStore> {
    let output = Command::new(fixture_program()).args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}");
    let store = bench.create(id, false);
    pump(
        output.stdout.as_slice(),
        store.as_ref(),
        OutputStream::Stdout,
        SPLIT_CHUNK,
    );
    pump(
        output.stderr.as_slice(),
        store.as_ref(),
        OutputStream::Stderr,
        SPLIT_CHUNK,
    );
    store
}

#[test]
fn output_merge_order_follows_single_pipe() {
    let bench = Bench::new();
    let store = bench.create("interleave", true);
    let (reader, writer) = std::io::pipe().unwrap();
    let mut child = Command::new(fixture_program())
        .args(["interleave", "40"])
        .stdin(Stdio::null())
        .stdout(writer.try_clone().unwrap())
        .stderr(writer)
        .spawn()
        .unwrap();
    pump(reader, store.as_ref(), OutputStream::Stdout, 4096);
    assert!(child.wait().unwrap().success());
    let all = OutputView {
        head: Some(100),
        ..view()
    };
    let out = store.render(&all, Some(0)).unwrap();
    let expected: Vec<String> = (0..40)
        .flat_map(|n| [format!("out {n}"), format!("err {n}")])
        .collect();
    assert_eq!(out.stdout, expected.join("\n"));
}

#[test]
fn output_decode_program_output_in_each_encoding() {
    let bench = Bench::new();
    for (encoding, decoded) in [
        ("utf-8", "utf-8"),
        ("utf-16le", "utf-16le"),
        ("utf-16be", "utf-16be"),
    ] {
        let store = capture(&bench, encoding, &["emit-encoding", encoding]);
        let out = store.render(&view(), Some(0)).unwrap();
        assert_eq!(out.stdout, SAMPLE, "{encoding}");
        assert_eq!(out.decoded_as, decoded);
        assert_eq!(out.stdout_lines, 1, "{encoding}");
    }
    let store = capture(&bench, "cp852", &["emit-encoding", "cp852"]);
    let requested = OutputView {
        encoding: Some("cp852".to_owned()),
        ..view()
    };
    assert_eq!(store.render(&requested, Some(0)).unwrap().stdout, SAMPLE);
}

#[cfg(not(windows))]
#[test]
fn output_decode_detects_cp1250_program_output() {
    let bench = Bench::new();
    let store = capture(&bench, "cp1250", &["emit-encoding", "cp1250"]);
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout, SAMPLE);
    assert_eq!(out.decoded_as, "windows-1250");
}

#[test]
fn output_decode_binary_program_output() {
    let bench = Bench::new();
    let store = capture(&bench, "binary", &["emit-binary", "4096"]);
    let out = store.render(&show_both(), Some(0)).unwrap();
    assert_eq!(out.stdout, "binary output, 4096 bytes");
    assert_eq!(out.decoded_as, "binary");
    assert_eq!(out.stdout_bytes, 4096);
}

fn show_both() -> OutputView {
    OutputView {
        show: ShowMode::Both,
        ..view()
    }
}

#[test]
fn output_cursor_concurrent_with_paced_program() {
    let bench = Bench::new();
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("output")
        .join("test_report.txt");
    let expected: Vec<String> = std::fs::read_to_string(&corpus)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let store = bench.create("paced", false);
    let mut child = Command::new(fixture_program())
        .arg("emit-file")
        .arg(&corpus)
        .args(["7", "1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let done = AtomicBool::new(false);
    let lines = std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            pump(stdout, store.as_ref(), OutputStream::Stdout, 5);
            store.finish(OutputStream::Stderr).unwrap();
            done.store(true, Ordering::Release);
        });
        let mut lines = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let ended = done.load(Ordering::Acquire);
            let out = store.read_since(cursor.as_deref(), &view()).unwrap();
            let progressed = out.output_cursor != cursor;
            if !out.stdout.is_empty() || progressed {
                lines.extend(out.stdout.split('\n').map(str::to_owned));
            }
            cursor = out.output_cursor;
            if ended && !progressed {
                break;
            }
            std::thread::yield_now();
        }
        writer.join().unwrap();
        lines
    });
    assert!(child.wait().unwrap().success());
    let read: Vec<String> = lines.into_iter().filter(|line| !line.is_empty()).collect();
    let expected: Vec<String> = expected
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(read, expected);
}
