// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Printing and parsing are inverse: for every generated tree `T`,
//! `parse(print(T)) = T` apart from spans and metadata, and printing the
//! parse of a printed text gives the same text.

mod roundtrip_support;

use nurt_contracts::ast::AstDocument;
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use roundtrip_support::{Shape, hash_form, program};

/// Generated trees per run for variants A and B.
const CASES: u32 = 100_000;

fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
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
}
