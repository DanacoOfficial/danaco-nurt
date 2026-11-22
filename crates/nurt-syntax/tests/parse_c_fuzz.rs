// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Deterministic mutation test of the variant C parser.
//!
//! Every input is a reference task text of variant C changed by a short
//! sequence of edits drawn from a seeded generator. The parser must return a
//! tree or an error for each input without panicking, and every accepted tree
//! must name all stage arguments.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Statement};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Number of mutated inputs checked in one run.
const INPUTS: usize = 100_000;

/// Seed of the edit generator.
const SEED: u64 = 0x6e75_7274_c0de_0004;

/// Largest number of edits applied to one input.
const MAX_EDITS: u64 = 4;

/// Text inserted by the insertion edits.
const DICTIONARY: &[&str] = &[
    "=",
    "==",
    ":",
    ",",
    "|>",
    "|",
    "->",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "@x",
    ".",
    "..",
    "...",
    "-",
    "+",
    "/",
    "/a/i",
    "\"",
    "\"/a\"",
    "r#\"",
    "\"#",
    "\\",
    "let ",
    "in",
    "in=",
    "and",
    "not",
    " ",
    "\n",
    "#",
    "0x",
    "1_",
    "9007199254740993",
    "1kib",
    "1h30m",
    "path=",
    "check=(",
    "count",
    "\u{feff}",
    "\u{200b}",
    "Ä",
];

/// SplitMix64 generator; deterministic across platforms.
struct Generator(u64);

impl Generator {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        usize::try_from(self.next() % u64::try_from(bound).expect("bound fits"))
            .expect("index fits")
    }
}

/// Byte offsets of the character boundaries of a text, including its end.
fn boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect()
}

fn mutate(text: &str, generator: &mut Generator) -> String {
    let mut out = text.to_owned();
    let edits = 1 + generator.next() % MAX_EDITS;
    for _ in 0..edits {
        let cuts = boundaries(&out);
        let at = cuts[generator.below(cuts.len())];
        let to = cuts[generator.below(cuts.len())];
        let (start, end) = (at.min(to), at.max(to));
        match generator.below(5) {
            0 => out.replace_range(start..end, ""),
            1 => out.insert_str(at, DICTIONARY[generator.below(DICTIONARY.len())]),
            2 => {
                let slice = out[start..end].to_owned();
                out.insert_str(end, &slice);
            }
            3 => out.truncate(at),
            _ => {
                let next = cuts.iter().find(|&&cut| cut > at).copied();
                if let Some(next) = next {
                    out.replace_range(at..next, DICTIONARY[generator.below(DICTIONARY.len())]);
                }
            }
        }
    }
    out
}

fn assert_named(pipeline: &Pipeline) {
    let head = match &pipeline.head {
        Head::Stage(stage) => Some(stage),
        _ => None,
    };
    for stage in head.into_iter().chain(&pipeline.stages) {
        for argument in &stage.args {
            assert!(argument.name.is_some(), "positional argument in {stage:?}");
            if let Expr::Sub { p, .. } = &argument.value {
                assert_named(p);
            }
        }
    }
}

fn assert_only_named_arguments(document: &AstDocument) {
    for statement in &document.program.statements {
        match statement {
            Statement::Let(binding) => assert_named(&binding.value),
            Statement::Pipeline(pipeline) => assert_named(pipeline),
            other => panic!("unexpected statement {other:?}"),
        }
    }
}

fn reference_texts() -> Vec<String> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d1/golden/ast/c");
    let mut paths: Vec<_> = fs::read_dir(&directory)
        .expect("variant C golden directory exists")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "nurt")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| fs::read_to_string(path).expect("golden text is UTF-8"))
        .collect()
}

#[test]
fn mutated_reference_texts_never_panic() {
    let texts = reference_texts();
    assert!(!texts.is_empty());
    let options = ParseOptions::default();
    let mut generator = Generator(SEED);
    let mut accepted = 0_usize;
    for index in 0..INPUTS {
        let input = mutate(&texts[index % texts.len()], &mut generator);
        if let Ok(document) = parse(&input, Variant::C, &options) {
            assert_only_named_arguments(&document);
            accepted += 1;
        }
    }
    assert!(accepted < INPUTS, "every mutated input was accepted");
}
