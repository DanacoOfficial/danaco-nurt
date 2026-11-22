// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Properties of the variant A parser on arbitrary text and on text built
//! from language fragments.

mod parse_a_support;

use nurt_syntax::parse::{NoRepair, ParseOptions, Variant, parse, parse_tolerant};
use parse_a_support::check_invariants;
use proptest::prelude::*;

/// Cases per property and run.
const CASES: u32 = 100_000;

/// Fragments of program text; concatenations reach most grammar paths.
const FRAGMENTS: [&str; 48] = [
    "find",
    "x",
    "git.status",
    " ",
    "  ",
    "\n",
    "\"/a\"",
    "r#\"q\"#",
    "|>",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ",",
    ":",
    "glob:",
    "in:",
    "-",
    "+",
    "*",
    "/",
    "/re/i",
    "1",
    "0x1f",
    "2.5",
    "1mb",
    "24h",
    "..",
    "...",
    "@x",
    ".",
    "not",
    "and",
    "or",
    "in",
    "==",
    "<",
    "~",
    "let",
    "=",
    "ago(",
    "true",
    "null",
    "#c\n",
    "-1",
    "\t",
];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS.as_slice()), 0..40)
        .prop_map(|fragments| fragments.concat())
}

fn input() -> impl Strategy<Value = String> {
    prop_oneof![any::<String>(), soup()]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: CASES, ..ProptestConfig::default() })]

    #[test]
    fn strict_results_survive_tolerant_parsing_unchanged(text in input()) {
        let options = ParseOptions::default();
        let strict = parse(&text, Variant::A, &options);
        let tolerant = parse_tolerant(&text, Variant::A, &options, &NoRepair);
        match (strict, tolerant) {
            (Ok(document), Ok(repaired)) => {
                prop_assert_eq!(document, repaired.document);
                prop_assert!(repaired.repairs.is_empty());
            }
            (Err(strict), Err(tolerant)) => prop_assert_eq!(strict, tolerant),
            (strict, tolerant) => prop_assert!(false, "{:?} vs {:?}", strict, tolerant),
        }
    }

    #[test]
    fn trees_keep_spans_nested_ordered_and_invariants_held(text in input()) {
        if let Ok(document) = parse(&text, Variant::A, &ParseOptions::default()) {
            let json = serde_json::to_value(&document).expect("tree serializes");
            prop_assert!(check_invariants(&json).is_ok(), "{:?}: {:?}", text, check_invariants(&json));
        }
    }
}
