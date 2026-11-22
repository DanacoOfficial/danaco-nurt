// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Comparison of the reader with `journalctl`.
//!
//! The tests run only where `journalctl` is in `PATH`; elsewhere they end
//! without assertions. The product itself never runs `journalctl`.

#[path = "support/journal_writer.rs"]
mod journal_writer;

use std::path::{Path, PathBuf};
use std::process::Command;

use journal_writer::{Compression, Entry, Options, Writer, sample_entries};
use nurt_contracts::record::LogLevel;
use nurt_syslog::journald::JournaldStore;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogQuery, LogStore};
use serde_json::Value;

const START: u64 = 1_680_000_000_000_000;
const NANOS_PER_MICRO: i64 = 1_000;
const LOCAL_EVENTS: u32 = 200;

fn journalctl() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("journalctl"))
        .find(|candidate| candidate.is_file())
}

fn journalctl_json(program: &Path, args: &[&str]) -> Vec<Value> {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn all_levels(limit: u32) -> LogQuery {
    LogQuery {
        level: LogLevel::Debug,
        limit,
        ..LogQuery::default()
    }
}

#[test]
#[ignore = "requires journalctl in PATH"]
fn reader_matches_journalctl_on_writer_files() {
    let Some(program) = journalctl() else {
        return;
    };
    let mut entries = sample_entries(40, START);
    let long = format!("long {}", "journal ".repeat(100));
    entries.push(Entry::new(
        START + 40_000,
        &[("MESSAGE", long.as_str()), ("PRIORITY", "4")],
    ));
    let directory = tempfile::tempdir().unwrap();
    let variants = [
        ("regular", Options::default()),
        (
            "compact",
            Options {
                compact: true,
                keyed_hash: true,
                ..Options::default()
            },
        ),
        (
            "zstd",
            Options {
                compression: Compression::Zstd,
                compress_threshold: 64,
                compact: true,
                keyed_hash: true,
                ..Options::default()
            },
        ),
        (
            "lz4",
            Options {
                compression: Compression::Lz4,
                compress_threshold: 64,
                ..Options::default()
            },
        ),
        (
            "xz",
            Options {
                compression: Compression::Xz,
                compress_threshold: 64,
                first_array_capacity: 1,
                ..Options::default()
            },
        ),
    ];
    for (name, options) in variants {
        let path = directory.path().join(format!("{name}.journal"));
        std::fs::write(&path, Writer::build(options, &entries)).unwrap();
        let file = path.to_str().unwrap();
        let theirs: Vec<String> = journalctl_json(
            &program,
            &["--file", file, "-o", "json", "-r", "--no-pager"],
        )
        .iter()
        .map(|entry| entry["MESSAGE"].as_str().unwrap_or_default().to_owned())
        .collect();
        let store = JournaldStore::from_files(vec![path.clone()], LogLimits::default());
        let ours: Vec<String> = store
            .query(&all_levels(1_000))
            .unwrap()
            .map(|event| event.unwrap().message)
            .collect();
        assert_eq!(ours, theirs, "{name}");
    }
}

#[test]
#[ignore = "requires journalctl in PATH and a readable system journal"]
fn reader_matches_journalctl_on_the_local_journal() {
    let Some(program) = journalctl() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let until_seconds = i64::try_from(now.as_secs()).unwrap() - 5;
    let until = nurt_contracts::value::Instant::from_unix_nanos(until_seconds * 1_000_000_000);
    let limit = LOCAL_EVENTS.to_string();
    let until_arg = format!("@{until_seconds}");
    let theirs: Vec<i64> = journalctl_json(
        &program,
        &[
            "-o",
            "json",
            "-r",
            "--no-pager",
            "-n",
            &limit,
            "--until",
            &until_arg,
        ],
    )
    .iter()
    .filter_map(|entry| entry["__REALTIME_TIMESTAMP"].as_str()?.parse().ok())
    .collect();
    let store = JournaldStore::new("/", LogLimits::default());
    let query = LogQuery {
        until: Some(until),
        ..all_levels(LOCAL_EVENTS)
    };
    let Ok(stream) = store.query(&query) else {
        return;
    };
    let mut ours: Vec<i64> = stream
        .map(|event| event.unwrap().time.as_unix_nanos() / NANOS_PER_MICRO)
        .collect();
    let mut theirs = theirs;
    ours.sort_unstable();
    theirs.sort_unstable();
    assert_eq!(ours, theirs);
}
