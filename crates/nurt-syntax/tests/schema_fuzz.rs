// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/schema_fuzz.rs
// Responsibility:   Deterministic mutation test of the JSON decoders and the tree invariants.
// Dependencies:     hash_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Deterministic mutation test of the JSON decoders and the tree invariants.
//!
//! The JSON texts of all golden trees are mutated byte by byte; deep nesting
//! is added. No input may panic. Every accepted tree that passes validation
//! must encode to CBOR, decode back to the same tree and have a hash.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use hash_support::{Format, SplitMix, all_trees, decode_cbor, decode_json, mutate};

/// Number of mutated inputs.
const MUTATED_INPUTS: usize = 100_000;

/// Largest number of mutations stacked on one input.
const MAX_STACKED_MUTATIONS: usize = 4;

/// Nesting of the deep inputs.
const DEEP_NESTING: usize = 10_000;

fn check(input: &str) -> bool {
    let mut accepted = false;
    for format in [Format::Surface, Format::Bound] {
        if let Ok(tree) = decode_json(format, input) {
            accepted = true;
            let again = decode_cbor(format, &tree.cbor()).unwrap();
            assert_eq!(again, tree, "{input}");
            assert!(tree.hash().starts_with("sha256:"));
        }
    }
    accepted
}

#[test]
fn mutated_golden_json_never_panics() {
    let seeds: Vec<Vec<u8>> = all_trees()
        .iter()
        .map(|tree| tree.json().to_string().into_bytes())
        .collect();
    assert!(!seeds.is_empty());
    let mut rng = SplitMix::new(0x6e75_7274_6a73_6f6e);
    let mut accepted = 0;
    for _ in 0..MUTATED_INPUTS {
        let mut input = seeds[rng.below(seeds.len())].clone();
        for _ in 0..=rng.below(MAX_STACKED_MUTATIONS) {
            input = mutate(&mut rng, &input);
        }
        if check(&String::from_utf8_lossy(&input)) {
            accepted += 1;
        }
    }
    assert!(accepted < MUTATED_INPUTS);
}

#[test]
fn deep_json_nesting_is_rejected() {
    for (open, close) in [("[", "]"), ("{\"a\":", "}")] {
        let input = format!(
            "{}null{}",
            open.repeat(DEEP_NESTING),
            close.repeat(DEEP_NESTING)
        );
        assert!(!check(&input));
        let wrapped = format!(
            "{{\"nurt\":\"1.0\",\"meta\":{}1{},\"program\":{{\"statements\":[]}}}}",
            open.repeat(DEEP_NESTING),
            close.repeat(DEEP_NESTING)
        );
        assert!(!check(&wrapped));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
