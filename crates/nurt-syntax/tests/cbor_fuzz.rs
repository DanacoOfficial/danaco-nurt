// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/cbor_fuzz.rs
// Responsibility:   Deterministic mutation test of the CBOR decoders.
// Dependencies:     hash_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Deterministic mutation test of the CBOR decoders.
//!
//! The deterministic encodings of all golden trees are mutated by bit flips,
//! byte replacements, insertions, deletions, truncations and duplicated
//! slices; hostile length headers and deep nesting are added. No input may
//! panic. Every accepted tree must pass validation, encode again to the same
//! tree and have a hash.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use hash_support::{Format, SplitMix, all_trees, decode_cbor, mutate};
use nurt_syntax::cbor::{MAX_DEPTH, read};

/// Number of mutated inputs.
const MUTATED_INPUTS: usize = 100_000;

/// Largest number of mutations stacked on one input.
const MAX_STACKED_MUTATIONS: usize = 4;

/// Nesting of the deep inputs.
const DEEP_NESTING: usize = 10_000;

fn check(input: &[u8]) -> bool {
    let _ = read(input);
    let mut accepted = false;
    for format in [Format::Surface, Format::Bound] {
        if let Ok(tree) = decode_cbor(format, input) {
            accepted = true;
            let bytes = tree.cbor();
            let again = decode_cbor(format, &bytes).unwrap();
            assert_eq!(again, tree, "{input:02x?}");
            assert!(tree.hash().starts_with("sha256:"));
        }
    }
    accepted
}

fn seeds() -> Vec<Vec<u8>> {
    all_trees()
        .iter()
        .map(|tree| tree.decode().cbor())
        .collect()
}

#[test]
fn mutated_golden_encodings_never_panic() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let mut rng = SplitMix::new(0x6e75_7274_6362_6f72);
    let mut accepted = 0;
    for _ in 0..MUTATED_INPUTS {
        let mut input = seeds[rng.below(seeds.len())].clone();
        for _ in 0..=rng.below(MAX_STACKED_MUTATIONS) {
            input = mutate(&mut rng, &input);
        }
        if check(&input) {
            accepted += 1;
        }
    }
    assert!(accepted < MUTATED_INPUTS);
}

#[test]
fn hostile_length_headers_never_allocate_or_panic() {
    let headers: [&[u8]; 6] = [
        &[0x7a, 0xff, 0xff, 0xff, 0xff],
        &[0x7b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[0x9a, 0xff, 0xff, 0xff, 0xff],
        &[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[0xba, 0xff, 0xff, 0xff, 0xff],
        &[0xbb, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
    ];
    let mut rng = SplitMix::new(1);
    for seed in seeds() {
        for header in headers {
            let at = rng.below(seed.len());
            let mut input = seed.clone();
            input.splice(at..at, header.iter().copied());
            check(&input);
            let mut replaced = seed.clone();
            replaced.splice(
                at..(at + header.len()).min(seed.len()),
                header.iter().copied(),
            );
            check(&replaced);
        }
    }
}

#[test]
fn deep_nesting_is_rejected_without_recursion() {
    let shapes: [(&[u8], &[u8]); 4] = [
        (&[0x81], &[0x80]),
        (&[0x9f], &[]),
        (&[0xa1, 0x61, b'k'], &[0xf6]),
        (&[0xbf, 0x61, b'k'], &[]),
    ];
    for (open, close) in shapes {
        let mut input = open.repeat(DEEP_NESTING);
        input.extend(close);
        let error = read(&input).unwrap_err();
        assert!(error.message.contains("nesting"), "{error}");
        assert!(!check(&input));
    }
    let mut input = [0x81].repeat(MAX_DEPTH - 1);
    input.push(0x80);
    assert!(read(&input).is_ok());
}

#[test]
fn every_truncation_of_a_golden_encoding_is_rejected() {
    for seed in seeds().into_iter().take(16) {
        for end in 0..seed.len() {
            assert!(!check(&seed[..end]), "accepted a truncation at {end}");
        }
        assert!(check(&seed));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
