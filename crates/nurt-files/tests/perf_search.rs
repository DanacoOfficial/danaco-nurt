// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/perf_search.rs
// Responsibility:   Performance measurement of `nurt_text::search` against `rg` on a 1 GiB corpus.
// Dependencies:     nurt_text, support_corpus, support_diff
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Performance measurement of `nurt_text::search` against `rg` on a 1 GiB corpus.
//!
//! Run with `cargo test -p nurt-files --features differential --test perf_search -- --ignored`.
//! The directory of the corpus is taken from `NURT_PERF_DIR` and defaults to the
//! temporary directory of the system.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nurt_text::search::{Pattern, SearchOptions, Searcher};
use support_corpus::{LargeCorpus, NEEDLE, generate_large};
use support_diff::{require, run_expect};

/// Size of the corpus in bytes.
const CORPUS_BYTES: u64 = 1 << 30;
const SEED: u64 = 0x9E_4001;
/// Highest accepted ratio of the Nurt time to the `rg` time.
const MAX_RATIO: f64 = 1.3;
const THREADS: usize = 8;
const ROUNDS: usize = 3;
const PERF_DIR_VARIABLE: &str = "NURT_PERF_DIR";

fn corpus_parent() -> Option<PathBuf> {
    std::env::var_os(PERF_DIR_VARIABLE).map(PathBuf::from)
}

fn timed<T>(work: impl FnOnce() -> T) -> (Duration, T) {
    let started = Instant::now();
    let result = work();
    (started.elapsed(), result)
}

/// Counts the lines containing the needle in all files, with `THREADS` workers.
fn nurt_count(files: &[PathBuf]) -> u64 {
    let searcher = Searcher::new(&[Pattern::literal(NEEDLE)], SearchOptions::new()).unwrap();
    let next = AtomicUsize::new(0);
    let totals: Vec<u64> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    let mut total = 0u64;
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = files.get(index) else {
                            break total;
                        };
                        let data = fs::read(path).unwrap();
                        total += searcher
                            .search_slice(&data, |_| ControlFlow::Continue(()))
                            .hits;
                    }
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    totals.iter().sum()
}

fn rg_count(rg: &Path, root: &Path) -> u64 {
    let threads = THREADS.to_string();
    let output = run_expect(
        rg,
        &[
            "--no-config",
            "-j",
            threads.as_str(),
            "-c",
            "-F",
            NEEDLE,
            ".",
        ],
        root,
        &[0],
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| line.rsplit_once(':').unwrap().1.parse::<u64>().unwrap())
        .sum()
}

// A 1 GiB corpus needs about a minute to write and several GiB of disk space; the
// measurement belongs to the performance run, not to the default test run.
#[test]
#[ignore = "writes a 1 GiB corpus and takes minutes; run explicitly for the performance measurement"]
fn search_over_one_gibibyte_is_within_the_ratio_to_rg() {
    let Some(rg) = require("rg") else {
        return;
    };
    let dir = match corpus_parent() {
        Some(parent) => tempfile::tempdir_in(parent).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    let corpus: LargeCorpus = generate_large(dir.path(), CORPUS_BYTES, SEED).unwrap();
    assert!(corpus.bytes >= CORPUS_BYTES);

    // The first rounds also bring the corpus into the page cache for both programs.
    let mut best_nurt = Duration::MAX;
    let mut best_rg = Duration::MAX;
    for _ in 0..ROUNDS {
        let (elapsed, count) = timed(|| rg_count(&rg, dir.path()));
        assert_eq!(count, corpus.needle_lines, "rg count");
        best_rg = best_rg.min(elapsed);
        let (elapsed, count) = timed(|| nurt_count(&corpus.files));
        assert_eq!(count, corpus.needle_lines, "Nurt count");
        best_nurt = best_nurt.min(elapsed);
    }
    let ratio = best_nurt.as_secs_f64() / best_rg.as_secs_f64();
    assert!(
        ratio <= MAX_RATIO,
        "search takes {ratio:.2} times as long as rg (Nurt {best_nurt:?}, rg {best_rg:?}); limit {MAX_RATIO}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
