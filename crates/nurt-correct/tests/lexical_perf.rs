// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/lexical_perf.rs
// Responsibility:   Time of the front part of correction, measured in a release build.
// Dependencies:     lexical_support, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Time of the front part of correction, measured in a release build:
//! `cargo test -p nurt-correct --release --test lexical_perf -- --ignored`.

mod lexical_support;

use std::time::{Duration, Instant};

use lexical_support::{Catalog, correction, syntax};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::lexical::front;
use nurt_correct::tables::load_embedded;

/// Runs of each program.
const RUNS: usize = 2_000;

/// Limit of the median time of a typical program.
const TYPICAL_LIMIT: Duration = Duration::from_millis(1);

/// Limit of the median time of a program that needs three repairs.
const REPAIRED_LIMIT: Duration = Duration::from_millis(5);

/// Typical programs: valid, or with token corrections.
const TYPICAL: [&str; 4] = [
    "find \"/srv/app\" glob: \"**/*.go\"\n|> filter size > 1mb and modified > ago(24h)\n|> first 20",
    "Find /srv/app glob: '*.go' | search /Pending/ | count",
    "list \"/var/log\" hidden: true |> sort name |> first 5",
    "find \"/srv\" glob=\"*.rs\" -> count",
];

/// A program that needs three repairs of one statement.
const REPAIRED: &str =
    "find \"/a\", glob: \"*.go\", hidden: true |> verify (run [\"go\", \"build\"";

fn median(text: &str) -> Duration {
    let tables = load_embedded().expect("embedded tables");
    let catalog = Catalog::load();
    let syntax = syntax(SyntaxVariant::A);
    let correction = correction();
    let mut times: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            let result = front(text, &syntax, &correction, &tables, &catalog);
            let elapsed = start.elapsed();
            assert!(result.is_ok(), "{text}");
            elapsed
        })
        .collect();
    times.sort();
    times[times.len() / 2]
}

#[test]
#[ignore = "timing; run in a release build"]
fn typical_programs_take_less_than_a_millisecond() {
    for text in TYPICAL {
        let time = median(text);
        assert!(time < TYPICAL_LIMIT, "{text}: {time:?}");
    }
}

#[test]
#[ignore = "timing; run in a release build"]
fn a_program_with_three_repairs_takes_less_than_five_milliseconds() {
    let time = median(REPAIRED);
    assert!(time < REPAIRED_LIMIT, "{time:?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
