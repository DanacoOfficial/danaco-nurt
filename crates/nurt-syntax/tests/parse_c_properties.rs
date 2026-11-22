// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Properties of the variant C parser over arbitrary input: it never panics,
//! and every tree it accepts names every stage argument.

use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Statement};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

/// Number of generated inputs per property.
const CASES: u32 = 100_000;

/// Fixed seed, so that every run checks the same inputs.
const SEED: u64 = 0x6e75_7274_0000_000c;

/// Longest token sequence of a generated program.
const MAX_FRAGMENTS: usize = 48;

/// Fragments of variant C programs and of the notations it rejects.
const FRAGMENTS: &[&str] = &[
    "find",
    "filter",
    "verify",
    "count",
    "code.rename",
    "path",
    "condition",
    "check",
    "in",
    "and",
    "or",
    "not",
    "let",
    "true",
    "null",
    "=",
    "==",
    "!=",
    ":",
    ",",
    ".",
    "..",
    "...",
    "|>",
    "|",
    "->",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "@r",
    "@r.path",
    "-",
    "+",
    "*",
    "/",
    "/x\\(/i",
    "\"/a\"",
    "\"\\q\"",
    "r#\"a\"#",
    "\"",
    "1",
    "20",
    "1mb",
    "24h",
    "1.5",
    "0x1f",
    "ago(",
    "#c",
    " ",
    " ",
    "  ",
    "\n",
    "\r\n",
    "\t",
    "\u{a0}",
    "Find",
    "x",
];

fn config() -> Config {
    Config {
        cases: CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(SEED),
        ..Config::default()
    }
}

fn parse_c(text: &str) -> Option<AstDocument> {
    parse(text, Variant::C, &ParseOptions::default()).ok()
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

proptest! {
    #![proptest_config(config())]

    #[test]
    fn arbitrary_text_never_panics(text in any::<String>()) {
        if let Some(document) = parse_c(&text) {
            assert_only_named_arguments(&document);
        }
    }

    #[test]
    fn fragment_sequences_never_panic(
        fragments in prop::collection::vec(prop::sample::select(FRAGMENTS), 0..MAX_FRAGMENTS)
    ) {
        let text = fragments.concat();
        if let Some(document) = parse_c(&text) {
            assert_only_named_arguments(&document);
        }
    }
}
