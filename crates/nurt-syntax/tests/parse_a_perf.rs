// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_a_perf.rs
// Responsibility:   Timing of the variant A parser on the golden programs. Run in release.
// Dependencies:     parse_a_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Timing of the variant A parser on the golden programs. Run in release
//! mode with `--ignored`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod parse_a_support;

use std::time::{Duration, Instant};

use nurt_syntax::parse::{ParseOptions, Variant, parse};
use parse_a_support::golden_programs;

/// Timed parses of each program.
const ITERATIONS: usize = 2_000;

/// Untimed parses of each program before timing.
const WARM_UP: usize = 200;

/// Required median of one parse of a typical program.
const MEDIAN_LIMIT: Duration = Duration::from_micros(500);

/// Median and 95th percentile of a set of durations.
#[derive(Debug, Clone, Copy)]
struct Summary {
    median: Duration,
    p95: Duration,
}

/// Times `run` on every text: `warm_up` untimed calls, then `iterations`
/// timed calls per text; returns the median and the 95th percentile of all
/// timed calls.
fn measure(texts: &[&str], warm_up: usize, iterations: usize, run: impl Fn(&str)) -> Summary {
    let mut samples = Vec::with_capacity(texts.len() * iterations);
    for text in texts {
        for _ in 0..warm_up {
            run(text);
        }
        for _ in 0..iterations {
            let start = Instant::now();
            run(text);
            samples.push(start.elapsed());
        }
    }
    samples.sort_unstable();
    let at = |fraction: f64| {
        let last = samples.len().saturating_sub(1);
        let index = (fraction * last as f64).round() as usize;
        samples.get(index).copied().unwrap_or_default()
    };
    Summary {
        median: at(0.5),
        p95: at(0.95),
    }
}

#[test]
#[ignore = "timing; run in release mode"]
fn perf_parse() {
    let programs = golden_programs(&["task-", "example-", "extra-"]);
    let texts: Vec<&str> = programs.iter().map(|(_, text)| text.as_str()).collect();
    let options = ParseOptions::default();
    let summary = measure(&texts, WARM_UP, ITERATIONS, |text| {
        let parsed = parse(text, Variant::A, &options);
        assert!(parsed.is_ok());
    });
    println!(
        "parse, {} programs x {ITERATIONS}: median {:?}, p95 {:?}",
        texts.len(),
        summary.median,
        summary.p95
    );
    assert!(summary.median <= MEDIAN_LIMIT, "{summary:?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
