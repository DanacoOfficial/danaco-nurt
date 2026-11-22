// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Deterministic mutation test of the call notation parser (variant B).
//!
//! Inputs are derived from the variant B golden programs by deleting,
//! inserting, replacing, duplicating and swapping characters and fragments.
//! The parser returns a tree or an error for every input; a returned tree
//! has nested spans and survives a JSON round trip through the contract
//! types.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use nurt_contracts::ast::AstDocument;
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Smallest number of mutated inputs.
const MIN_INPUTS: usize = 100_000;
/// Largest number of mutations applied to one input.
const MAX_MUTATIONS: u64 = 4;
/// Seed of the pseudo-random sequence.
const SEED: u64 = 0x9e37_79b9_7f4a_7c15;

/// Characters inserted by mutations: the punctuation of variant B, the
/// notation of other variants and characters outside the language.
const ALPHABET: &[char] = &[
    '(', ')', '[', ']', '{', '}', ',', ':', '=', '|', '>', '<', '-', '+', '*', '/', '.', '@', '"',
    '#', '\n', ' ', '\t', 'a', 'z', '0', '9', '_', 'r', '\\', '!', '~', '$', ';', '\'', '\u{a0}',
    'é',
];

/// Fragments inserted by mutations.
const FRAGMENTS: &[&str] = &[
    "|>",
    "...",
    "..",
    "let ",
    "not ",
    " and ",
    " in ",
    "()",
    "(",
    ")",
    "r#\"",
    "\"#",
    "1mb",
    "24h",
    "@x",
    "glob: ",
    "glob=",
    ", ",
    "/re/i",
    "-1",
    "\"\\u{41}\"",
    "\"\\q\"",
];

/// xorshift64* generator; enough for reproducible mutations.
struct Sequence(u64);

impl Sequence {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            usize::try_from(self.next() % u64::try_from(bound).unwrap()).unwrap()
        }
    }
}

fn golden_programs() -> Vec<Vec<char>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d1/golden/ast/b");
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "nurt")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| fs::read_to_string(path).unwrap().chars().collect())
        .collect()
}

fn mutate(text: &mut Vec<char>, sequence: &mut Sequence) {
    let length = text.len();
    let at = sequence.below(length + 1);
    match sequence.below(6) {
        0 if at < length => {
            text.remove(at);
        }
        1 => text.insert(at, ALPHABET[sequence.below(ALPHABET.len())]),
        2 if at < length => text[at] = ALPHABET[sequence.below(ALPHABET.len())],
        3 => {
            let fragment = FRAGMENTS[sequence.below(FRAGMENTS.len())];
            for (offset, character) in fragment.chars().enumerate() {
                text.insert(at + offset, character);
            }
        }
        4 if at < length => {
            let end = (at + 1 + sequence.below(12)).min(length);
            if sequence.below(2) == 0 {
                text.drain(at..end);
            } else {
                let copy: Vec<char> = text[at..end].to_vec();
                for (offset, character) in copy.into_iter().enumerate() {
                    text.insert(end + offset, character);
                }
            }
        }
        _ if at + 1 < length => text.swap(at, at + 1),
        _ => {}
    }
}

fn check(input: &str) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        parse(input, Variant::B, &ParseOptions::default())
    }));
    let Ok(result) = outcome else {
        panic!("parser panicked on {input:?}");
    };
    if let Ok(document) = result {
        let json = serde_json::to_value(&document).unwrap();
        let decoded: AstDocument = serde_json::from_value(json)
            .unwrap_or_else(|error| panic!("{input:?}: tree does not decode: {error}"));
        assert_eq!(decoded, document, "{input:?}");
    }
}

#[test]
fn mutated_golden_programs_never_panic() {
    let programs = golden_programs();
    assert!(!programs.is_empty());
    let mut sequence = Sequence(SEED);
    let mut inputs = 0;
    while inputs < MIN_INPUTS {
        for program in &programs {
            let mut text = program.clone();
            let mutations = 1 + sequence.next() % MAX_MUTATIONS;
            for _ in 0..mutations {
                mutate(&mut text, &mut sequence);
            }
            check(&text.iter().collect::<String>());
            inputs += 1;
        }
    }
}

#[test]
fn every_prefix_of_golden_programs_is_handled() {
    for program in golden_programs() {
        for end in 0..=program.len() {
            check(&program[..end].iter().collect::<String>());
        }
    }
}
