// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Spool files: writing, limits, lifetime, reopening and removal.

use std::fs::File;
use std::time::{Duration, SystemTime};

use nurt_contracts::config::{ByteLimit, Config, SizeSetting};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::Notable;
use nurt_contracts::stage::{ExecCtx, OutputStream};
use sha2::{Digest, Sha256};

use super::{Bench, START_NANOS, numbered, view};
use crate::iface::OutputView;

/// Output written by the large output test.
const LARGE_OUTPUT_BYTES: usize = 1024 * 1024 * 1024;

/// Memory growth allowed while writing and reading the large output.
const LARGE_OUTPUT_MEMORY: u64 = 64 * 1024 * 1024;

/// Resident memory of this process in bytes.
#[cfg(target_os = "linux")]
fn resident_bytes() -> u64 {
    const KIB: u64 = 1024;
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| {
            value
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap()
        * KIB
}

#[cfg(not(target_os = "linux"))]
fn resident_bytes() -> u64 {
    0
}

#[test]
fn output_spool_large_keeps_memory_flat() {
    const LINE_BYTES: usize = 4096;
    const CHUNK_LINES: usize = 256;
    // The memory of a spool without a limit does not grow with the output.
    let mut config = Config::default();
    config.run.spool_max = ByteLimit::Unlimited;
    let bench = Bench::with_config(config);
    let store = bench.create("large", false);
    let mut line = vec![b'x'; LINE_BYTES - 1];
    line.push(b'\n');
    let chunk = line.repeat(CHUNK_LINES);
    let before = resident_bytes();
    for _ in 0..LARGE_OUTPUT_BYTES / chunk.len() {
        store.write(OutputStream::Stdout, &chunk).unwrap();
    }
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let out = store.render(&view(), Some(0)).unwrap();
    let growth = resident_bytes().saturating_sub(before);
    assert!(
        growth <= LARGE_OUTPUT_MEMORY,
        "memory grew by {growth} bytes"
    );
    let lines = (LARGE_OUTPUT_BYTES / LINE_BYTES) as u64;
    assert_eq!(out.stdout_bytes, LARGE_OUTPUT_BYTES as u64);
    assert_eq!(out.stdout_lines, lines);
    assert_eq!(out.shown.stdout.unwrap().omitted_lines, lines - 60);
    let file = out.stdout_file.unwrap();
    assert_eq!(
        std::fs::metadata(file).unwrap().len(),
        LARGE_OUTPUT_BYTES as u64
    );
}

#[test]
fn output_merge_order_keeps_arrival_order() {
    let bench = Bench::new();
    let store = bench.create("merged", true);
    for n in 0..3 {
        store
            .write(OutputStream::Stdout, format!("out {n}\n").as_bytes())
            .unwrap();
        store
            .write(OutputStream::Stderr, format!("err {n}\n").as_bytes())
            .unwrap();
    }
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout, "out 0\nerr 0\nout 1\nerr 1\nout 2\nerr 2");
    assert_eq!(out.stderr, "");
    assert_eq!((out.stdout_bytes, out.stderr_bytes), (18, 18));
    let file = out.output_file.unwrap();
    assert!(file.ends_with("merged-output.txt"));
    assert_eq!(out.stdout_file, None);
}

#[test]
fn hashes_all_bytes_in_arrival_order() {
    let bench = Bench::new();
    let store = bench.create("hash", false);
    assert_eq!(store.render(&view(), None).unwrap().output_sha256, None);
    store.write(OutputStream::Stdout, b"a\n").unwrap();
    store.write(OutputStream::Stderr, b"b\n").unwrap();
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let expected: String = Sha256::digest(b"a\nb\n")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.output_sha256, Some(expected));
    let error = store.write(OutputStream::Stdout, b"late").unwrap_err();
    assert_eq!(error.code(), ErrorCode::Internal);
}

fn limited(bytes: u64) -> Bench {
    let mut config = Config::default();
    config.run.spool_max = ByteLimit::Bytes(SizeSetting::from_bytes(bytes));
    Bench::with_config(config)
}

fn file_text(out: &crate::iface::RenderedOutput) -> String {
    let path = out
        .stdout_file
        .as_ref()
        .or(out.output_file.as_ref())
        .unwrap();
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn output_beyond_the_spool_limit_keeps_the_start_and_the_end() {
    // Limit 40: a head of 30 bytes and, per stream file, a tail of 5 bytes.
    let bench = limited(40);
    let output = "aaaaaaaaa\n".repeat(10);
    let store = bench.finished("limit", output.as_bytes(), b"");
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout_bytes, 100);
    assert_eq!(out.stdout_lines, 10);
    assert_eq!(
        file_text(&out),
        format!(
            "{}[... 65 bytes omitted ...]\naaaa\n",
            "aaaaaaaaa\n".repeat(3)
        )
    );
    let note = out
        .notes
        .iter()
        .find(|note| note.code == Notable::SpoolLimit)
        .unwrap();
    assert_eq!(note.count, Some(65));
    let expected: String = Sha256::digest(output.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        out.output_sha256,
        Some(expected),
        "the hash covers the omitted bytes"
    );
}

#[test]
fn a_cut_in_the_middle_of_a_line_is_marked_on_a_line_of_its_own() {
    // Limit 8: a head of 6 bytes and a tail of 2 bytes of the merged file.
    let bench = limited(8);
    let store = bench.create("cut", true);
    store.write(OutputStream::Stdout, b"abcdefghij").unwrap();
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(file_text(&out), "abcdef\n[... 2 bytes omitted ...]\nij");
}

#[test]
fn output_that_fits_in_the_head_and_the_tail_is_kept_whole() {
    // Limit 40, merged: a head of 30 bytes and a tail of 10 bytes.
    let bench = limited(40);
    let store = bench.create("fits", true);
    let output = "0123456789".repeat(3) + "abcdef";
    store
        .write(OutputStream::Stdout, output.as_bytes())
        .unwrap();
    store.finish(OutputStream::Stdout).unwrap();
    store.finish(OutputStream::Stderr).unwrap();
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(file_text(&out), output);
    assert!(
        out.notes
            .iter()
            .all(|note| note.code != Notable::SpoolLimit)
    );
}

#[test]
fn without_a_limit_the_whole_output_is_kept() {
    let mut config = Config::default();
    config.run.spool_max = ByteLimit::Unlimited;
    let bench = Bench::with_config(config);
    let output = "aaaaaaaaa\n".repeat(1000);
    let store = bench.finished("unlimited", output.as_bytes(), b"");
    let out = store.render(&view(), Some(0)).unwrap();
    assert_eq!(file_text(&out), output);
    assert!(
        out.notes
            .iter()
            .all(|note| note.code != Notable::SpoolLimit)
    );
}

#[test]
fn the_tail_of_a_limit_is_a_quarter_of_it_and_at_most_64_mib() {
    use super::super::spool::Keep;
    assert_eq!(Keep::of(Some(40), true), Keep { head: 30, tail: 10 });
    assert_eq!(Keep::of(Some(40), false), Keep { head: 30, tail: 5 });
    let gib = 1024 * 1024 * 1024;
    let mib = 1024 * 1024;
    assert_eq!(
        Keep::of(Some(gib), true),
        Keep {
            head: gib - 64 * mib,
            tail: 64 * mib as usize
        }
    );
    assert_eq!(Keep::of(Some(0), true), Keep { head: 0, tail: 0 });
    assert_eq!(Keep::of(None, true).head, u64::MAX);
}

#[test]
fn reports_last_activity_from_context_clock() {
    let bench = Bench::new();
    let store = bench.create("activity", false);
    assert_eq!(store.last_activity(), None);
    bench.clock.advance(Duration::from_secs(5));
    store.write(OutputStream::Stdout, b"x").unwrap();
    store.write(OutputStream::Stdout, b"").unwrap();
    assert_eq!(store.last_activity(), Some(Duration::from_secs(5)));
}

#[test]
fn reopens_spool_of_ended_and_running_runs() {
    let bench = Bench::new();
    drop(bench.finished("done", b"kept\n", b""));
    let reopened = bench.factory.open(&bench.ctx, "done").unwrap();
    let out = reopened.render(&view(), Some(0)).unwrap();
    assert_eq!(out.stdout, "kept");
    assert!(out.output_sha256.is_some());
    let running = bench.create("running", false);
    running.write(OutputStream::Stdout, b"x\ny").unwrap();
    let same = bench.factory.open(&bench.ctx, "running").unwrap();
    assert_eq!(same.last_activity(), running.last_activity());
    let other_process = super::super::spool_factory();
    let copy = other_process.open(&bench.ctx, "running").unwrap();
    let out = copy.render(&view(), None).unwrap();
    assert_eq!(out.stdout, "x");
    assert_eq!((out.stdout_bytes, out.stdout_lines), (3, 2));
    let error = copy.write(OutputStream::Stdout, b"z").unwrap_err();
    assert_eq!(error.code(), ErrorCode::Internal);
}

#[test]
fn removes_spool_and_reports_missing_output() {
    let bench = Bench::new();
    drop(bench.finished("gone", b"x\n", b""));
    bench.factory.remove(&bench.ctx, "gone").unwrap();
    bench.factory.remove(&bench.ctx, "gone").unwrap();
    let names: Vec<_> = std::fs::read_dir(bench.spool_dir()).unwrap().collect();
    assert!(names.is_empty());
    let error = bench.factory.open(&bench.ctx, "gone").err().unwrap();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn rejects_names_unusable_as_file_names_and_missing_state() {
    let bench = Bench::new();
    for id in ["", "../x", ".hidden", "a/b", "x".repeat(200).as_str()] {
        let error = bench.factory.create(&bench.ctx, id, false).err().unwrap();
        assert_eq!(error.code(), ErrorCode::Arg, "{id}");
    }
    let error = bench
        .factory
        .create(&bench.ctx, "dup", false)
        .and_then(|_| bench.factory.create(&bench.ctx, "dup", false));
    assert_eq!(error.err().unwrap().code(), ErrorCode::Exists);
    let stateless = ExecCtx::builder(
        bench.clock.clone(),
        std::sync::Arc::new(super::host(bench.dir.path())),
    )
    .build();
    let error = bench.factory.create(&stateless, "x", false).err().unwrap();
    assert_eq!(error.code(), ErrorCode::Config);
}

fn age_files(bench: &Bench, id: &str, age: Duration) {
    let start = SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::try_from(START_NANOS).unwrap());
    for name in [
        format!("{id}-stdout.txt"),
        format!("{id}-stderr.txt"),
        format!("{id}.json"),
    ] {
        let path = bench.spool_dir().join(name);
        if path.exists() {
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(start - age)
                .unwrap();
        }
    }
}

#[test]
fn removes_expired_spools_when_a_run_starts() {
    let bench = Bench::new();
    drop(bench.finished("old", b"x\n", b""));
    drop(bench.finished("fresh", b"x\n", b""));
    let stalled = bench.create("stalled", false);
    drop(bench.create("abandoned", false));
    let hour = Duration::from_secs(60 * 60);
    age_files(&bench, "old", hour * 2);
    age_files(&bench, "stalled", hour * 2);
    age_files(&bench, "abandoned", hour * 24 * 8);
    drop(bench.create("new", false));
    let exists = |name: &str| bench.spool_dir().join(name).exists();
    assert!(!exists("old.json") && !exists("old-stdout.txt"));
    assert!(exists("fresh.json"));
    assert!(exists("stalled.json"));
    assert!(!exists("abandoned.json"));
    drop(stalled);
}

#[test]
fn a_spool_whose_record_cannot_be_written_leaves_no_stream_files() {
    let bench = Bench::new();
    std::fs::create_dir_all(bench.spool_dir().join("broken.json").join("keep")).unwrap();
    let error = bench.factory.create(&bench.ctx, "broken", false).err();
    assert!(error.is_some());
    let leftovers: Vec<String> = std::fs::read_dir(bench.spool_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["broken.json"]);
}

#[cfg(unix)]
#[test]
fn creates_files_readable_by_the_account_only() {
    use std::os::unix::fs::PermissionsExt;
    const MODE_BITS: u32 = 0o777;
    let bench = Bench::new();
    drop(bench.finished("private", b"secret\n", b""));
    let mode = |name: &str| {
        std::fs::metadata(bench.spool_dir().join(name))
            .unwrap()
            .permissions()
            .mode()
            & MODE_BITS
    };
    assert_eq!(mode("private-stdout.txt"), 0o600);
    assert_eq!(mode("private.json"), 0o600);
    let dir = std::fs::metadata(bench.spool_dir())
        .unwrap()
        .permissions()
        .mode()
        & MODE_BITS;
    assert_eq!(dir, 0o700);
}

#[test]
fn summary_lines_use_whole_cleaned_lines() {
    let bench = Bench::new();
    let store = bench.finished("lines", numbered("ok", 3).as_bytes(), b"");
    let summary = OutputView {
        summary: vec!["ok 2".to_owned()],
        ..view()
    };
    let out = store.render(&summary, Some(0)).unwrap();
    assert_eq!(out.summary_lines, ["ok 2"]);
}

#[test]
fn the_default_spool_limit_is_one_gibibyte() {
    let limit = Config::default().run.spool_max;
    assert_eq!(limit, ByteLimit::Bytes(SizeSetting::from_gib(1)));
    let bench = Bench::with_config(Config::default());
    let store = bench.finished("default-limit", b"x\n", b"");
    let out = store.render(&view(), Some(0)).unwrap();
    assert!(
        out.notes
            .iter()
            .all(|note| note.code != Notable::SpoolLimit)
    );
}
