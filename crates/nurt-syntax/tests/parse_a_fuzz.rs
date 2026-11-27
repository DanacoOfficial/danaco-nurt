// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_a_fuzz.rs
// Responsibility:   Deterministic mutation test: golden programs with deleted, duplicated.
// Dependencies:     parse_a_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Deterministic mutation test: golden programs with deleted, duplicated,
//! swapped and inserted tokens never make the parser panic, and every tree
//! it accepts holds the invariants.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod parse_a_support;

use nurt_syntax::lex::{LexOptions, tokenize};
use nurt_syntax::parse::{NoRepair, ParseOptions, Variant, parse, parse_tolerant};
use parse_a_support::{check_invariants, golden_programs};

/// Number of mutated programs.
const MUTANTS: usize = 100_000;

/// Largest number of mutations applied to one program.
const MAX_MUTATIONS: u64 = 3;

/// Fragments inserted by mutations.
const INSERTIONS: [&str; 24] = [
    " ", "\n", "(", ")", "[", "]", "{", "}", ",", ":", "|>", "|", "-", "+", "..", "...", "@", ".",
    "not ", "and ", "\"", "/", "=", "#",
];

/// Linear congruential generator with the constants of Knuth's MMIX.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next()).unwrap_or(0) % bound.max(1)
    }
}

/// Token boundaries of a program: byte ranges of its tokens.
fn token_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    tokenize(text, LexOptions::default())
        .map(|tokens| tokens.into_iter().map(|token| token.range).collect())
        .unwrap_or_default()
}

fn mutate(text: &str, random: &mut Lcg) -> String {
    let mut text = text.to_owned();
    let count = 1 + random.next() % MAX_MUTATIONS;
    for _ in 0..count {
        let ranges = token_ranges(&text);
        if ranges.is_empty() {
            text.push_str(INSERTIONS[random.below(INSERTIONS.len())]);
            continue;
        }
        let range = ranges[random.below(ranges.len())].clone();
        let fragment = text[range.clone()].to_owned();
        text = match random.below(4) {
            0 => format!("{}{}", &text[..range.start], &text[range.end..]),
            1 => format!("{}{fragment}{}", &text[..range.end], &text[range.end..]),
            2 => {
                let other = ranges[random.below(ranges.len())].clone();
                let replacement = text[other].to_owned();
                format!(
                    "{}{replacement}{}",
                    &text[..range.start],
                    &text[range.end..]
                )
            }
            _ => {
                let insertion = INSERTIONS[random.below(INSERTIONS.len())];
                format!(
                    "{}{insertion}{}",
                    &text[..range.start],
                    &text[range.start..]
                )
            }
        };
    }
    text
}

#[test]
fn mutated_golden_programs_never_panic_and_accepted_trees_hold_invariants() {
    let programs = golden_programs(&["task-", "example-", "extra-"]);
    assert!(!programs.is_empty());
    let options = ParseOptions::default();
    let mut random = Lcg(0x5eed);
    let mut accepted = 0_usize;
    for index in 0..MUTANTS {
        let (_, original) = &programs[index % programs.len()];
        let text = mutate(original, &mut random);
        let strict = parse(&text, Variant::A, &options);
        let tolerant = parse_tolerant(&text, Variant::A, &options, &NoRepair);
        assert_eq!(strict.is_ok(), tolerant.is_ok(), "{text:?}");
        if let Ok(document) = strict {
            accepted += 1;
            let json = serde_json::to_value(&document).expect("tree serializes");
            check_invariants(&json).unwrap_or_else(|problem| panic!("{text:?}: {problem}"));
        }
    }
    assert!(accepted > 0, "some mutants stay valid");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
