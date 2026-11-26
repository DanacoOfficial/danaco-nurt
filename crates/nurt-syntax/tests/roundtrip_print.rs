// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Printing and parsing are inverse: for every generated tree `T`,
//! `parse(print(T)) = T` apart from spans and metadata, and printing the
//! parse of a printed text gives the same text. Variant C is checked on
//! trees whose arguments are all named.
//!
//! The regular run generates a few hundred trees per property; the nightly run
//! (`NURT_PROPTEST_FULL=1`) generates 100 000 trees for variants A and B and
//! 20 000 for variant C. `PROPTEST_CASES` overrides both numbers.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod roundtrip_support;

use nurt_contracts::ast::AstDocument;
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use roundtrip_support::{Shape, hash_form, program};

/// Generated trees per run for variants A and B in the full run.
const CASES: u32 = 100_000;

/// Generated trees per run for variant C in the full run.
const NAMED_CASES: u32 = 20_000;

/// Generated trees per property in the regular run.
const QUICK_CASES: u32 = 300;

/// Number of cases of a property: `PROPTEST_CASES`, else `full` when
/// `NURT_PROPTEST_FULL=1`, else the quick number.
fn cases_for(full: u32) -> u32 {
    if let Some(cases) = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
    {
        return cases;
    }
    if std::env::var("NURT_PROPTEST_FULL").is_ok_and(|value| value == "1") {
        full
    } else {
        QUICK_CASES
    }
}

fn config(full: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases_for(full),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn round_trip(document: &AstDocument, variant: Variant) -> Result<(), TestCaseError> {
    let text = print::try_print(document, variant)
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    let parsed = parse::parse(&text, variant, &ParseOptions::default())
        .map_err(|error| TestCaseError::fail(format!("{error}\n{text}")))?;
    prop_assert_eq!(hash_form(&parsed), hash_form(document), "{}", text);
    prop_assert_eq!(print::print(&parsed, variant), text);
    Ok(())
}

proptest! {
    #![proptest_config(config(CASES))]

    #[test]
    fn variant_a_text_reads_back_as_the_tree(document in program(Shape::POSITIONAL)) {
        round_trip(&document, Variant::A)?;
    }

    #[test]
    fn variant_b_text_reads_back_as_the_tree(document in program(Shape::POSITIONAL)) {
        round_trip(&document, Variant::B)?;
    }
}

proptest! {
    #![proptest_config(config(NAMED_CASES))]

    #[test]
    fn variant_c_text_reads_back_as_the_tree(document in program(Shape::NAMED)) {
        round_trip(&document, Variant::C)?;
    }
}
