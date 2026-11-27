// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/perf_walk.rs
// Responsibility:   Performance measurement of the walker against `fd` on a tree of 10^6 entries.
// Dependencies:     support_corpus, support_diff, support_walk
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Performance measurement of the walker against `fd` on a tree of 10^6 entries.
//!
//! Run with `cargo test -p nurt-files --features differential --test perf_walk -- --ignored`.
//! The directory of the tree is taken from `NURT_PERF_DIR` and defaults to the
//! temporary directory of the system.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;
mod support_fs;
#[path = "support_diff/walk.rs"]
mod support_walk;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nurt_files::walk::{KindFilter, WalkOptions};
use support_corpus::generate_empty_tree;
use support_diff::{require, run_expect};
use support_walk::walk_count;

/// Number of files in the tree.
const ENTRIES: u64 = 1_000_000;
/// Files per directory.
const FANOUT: u64 = 1_000;
/// Highest accepted ratio of the Nurt time to the `fd` time.
const MAX_RATIO: f64 = 1.3;
const THREADS: usize = 8;
const ROUNDS: usize = 3;
const PERF_DIR_VARIABLE: &str = "NURT_PERF_DIR";

fn timed<T>(work: impl FnOnce() -> T) -> (Duration, T) {
    let started = Instant::now();
    let result = work();
    (started.elapsed(), result)
}

fn fd_count(fd: &Path, root: &Path) -> u64 {
    let threads = THREADS.to_string();
    let output = run_expect(
        fd,
        &[
            "--no-config",
            "-j",
            threads.as_str(),
            "-H",
            "-I",
            "-t",
            "f",
            "--print0",
        ],
        root,
        &[0],
    );
    output.stdout.iter().filter(|byte| **byte == 0).count() as u64
}

fn nurt_count(root: &Path) -> u64 {
    walk_count(
        root,
        WalkOptions {
            min_depth: 1,
            kind: KindFilter::File,
            threads: THREADS,
            ..WalkOptions::default()
        },
    )
}

// Writing 10^6 files takes minutes and stresses the file system; the measurement
// belongs to the performance run, not to the default test run.
#[test]
#[ignore = "creates 10^6 files and takes minutes; run explicitly for the performance measurement"]
fn walk_over_a_million_entries_is_within_the_ratio_to_fd() {
    let Some(fd) = require("fd") else {
        return;
    };
    let dir = match std::env::var_os(PERF_DIR_VARIABLE).map(PathBuf::from) {
        Some(parent) => tempfile::tempdir_in(parent).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    generate_empty_tree(dir.path(), ENTRIES, FANOUT).unwrap();

    // The first rounds also bring the directory entries into the cache for both programs.
    let mut best_nurt = Duration::MAX;
    let mut best_fd = Duration::MAX;
    for _ in 0..ROUNDS {
        let (elapsed, count) = timed(|| fd_count(&fd, dir.path()));
        assert_eq!(count, ENTRIES, "fd count");
        best_fd = best_fd.min(elapsed);
        let (elapsed, count) = timed(|| nurt_count(dir.path()));
        assert_eq!(count, ENTRIES, "Nurt count");
        best_nurt = best_nurt.min(elapsed);
    }
    let ratio = best_nurt.as_secs_f64() / best_fd.as_secs_f64();
    assert!(
        ratio <= MAX_RATIO,
        "walk takes {ratio:.2} times as long as fd (Nurt {best_nurt:?}, fd {best_fd:?}); limit {MAX_RATIO}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
