// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/print_perf.rs
// Responsibility:   Printing time of a typical program, measured in release builds.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Printing time of a typical program, measured in release builds:
//! `cargo test --release --test print_perf -- --ignored`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use nurt_syntax::json;
use nurt_syntax::parse::Variant;
use nurt_syntax::print;

/// Largest median time of printing one golden task.
const BUDGET: Duration = Duration::from_micros(50);

/// Repetitions per task.
const ROUNDS: usize = 2_000;

#[test]
#[ignore = "timing test; run in release builds"]
fn prints_a_typical_program_within_budget() {
    let directory = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/ast");
    let mut documents = Vec::new();
    for entry in fs::read_dir(&directory).expect("golden directory") {
        let path = entry.expect("directory entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with("task-") && name.ends_with(".json") {
            let text = fs::read_to_string(&path).expect("tree file");
            documents.push(json::decode_surface(&text).expect("valid tree"));
        }
    }
    assert!(!documents.is_empty());
    let mut samples = Vec::with_capacity(ROUNDS * documents.len());
    let mut printed = 0;
    for _ in 0..ROUNDS {
        for document in &documents {
            let start = Instant::now();
            printed += print::print(document, Variant::A).len();
            samples.push(start.elapsed());
        }
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    println!("print: median {median:?}, p95 {p95:?}, {printed} bytes");
    assert!(median < BUDGET, "median {median:?} exceeds {BUDGET:?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
