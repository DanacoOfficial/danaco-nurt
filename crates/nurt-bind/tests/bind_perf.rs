// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Time of reading, correction and binding of the conformance programs.
//! Run in a release build with
//! `cargo test -p nurt-bind --release --test bind_perf -- --ignored`.

mod bind_support;

use std::fs;
use std::time::{Duration, Instant};

use bind_support::{catalog, workspace};
use nurt_bind::{BindContext, bind_with_correction};
use nurt_contracts::config::Config;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::Os;
use nurt_correct::tables::load_embedded;

/// Largest median time of one program.
const MAX_MEDIAN: Duration = Duration::from_millis(5);

/// Runs of every program.
const RUNS: usize = 21;

/// Word in the name of a program written for a Windows host.
const WINDOWS_MARK: [&str; 3] = ["windows", "tree-transport", "legacy-console"];

#[test]
#[ignore = "measures time; meaningful in a release build"]
fn reading_correction_and_binding_stay_within_five_milliseconds() {
    let tables = load_embedded().unwrap();
    let config = Config::default();
    let dir = workspace().join("spec/golden/ast");
    let mut programs: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".nurt") && !name.ends_with(".b.nurt") && !name.ends_with(".c.nurt")
        })
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect();
    programs.sort();
    assert!(!programs.is_empty());
    let mut times = Vec::with_capacity(programs.len() * RUNS);
    for (name, text) in &programs {
        let os = if WINDOWS_MARK.iter().any(|mark| name.contains(mark)) {
            Os::Windows
        } else {
            Os::Linux
        };
        let context = BindContext::new(os);
        for _ in 0..RUNS {
            let start = Instant::now();
            let result = bind_with_correction(
                text,
                SyntaxVariant::A,
                catalog(),
                &tables,
                &config,
                &context,
            );
            times.push(start.elapsed());
            drop(result);
        }
    }
    times.sort();
    let median = times[times.len() / 2];
    assert!(
        median <= MAX_MEDIAN,
        "median {median:?} over {} programs",
        programs.len()
    );
}
