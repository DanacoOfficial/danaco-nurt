// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-bind/tests/corpus_bind.rs
// Responsibility:   Corpus of the type and value rules applied during binding (`tests/d1/bind/corpus`), run through the full correction pipeline.
// Dependencies:     corpus_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Corpus of the type and value rules applied during binding
//! (`tests/d1/bind/corpus`), run through the full correction pipeline.

mod corpus_support;

use corpus_support::{read_cases, run, workspace};

/// Directory of the corpus, relative to the workspace.
const CORPUS: &str = "tests/d1/bind/corpus";

/// Largest share of `E_AMBIGUOUS` among unambiguous programs that change
/// the world.
const MAX_AMBIGUITY: f64 = 0.05;

/// Smallest number of mistake cases.
const MIN_MISTAKES: usize = 90;

/// Smallest number of adversarial cases.
const MIN_ADVERSARIAL: usize = 45;

/// Number of listed problems in a failure message.
const REPORT_LIMIT: usize = 40;

#[test]
fn every_case_of_the_binding_corpus_passes() {
    let cases = read_cases(&workspace().join(CORPUS));
    let measures = run(&cases);
    assert!(
        measures.failures.is_empty()
            && measures.false_corrections.is_empty()
            && measures.violations.is_empty(),
        "{}",
        measures.report(REPORT_LIMIT)
    );
    assert!(
        measures.ambiguity_share() <= MAX_AMBIGUITY,
        "{}",
        measures.summary()
    );
}

#[test]
fn the_binding_corpus_has_mistakes_and_adversarial_cases() {
    let cases = read_cases(&workspace().join(CORPUS));
    let adversarial = cases
        .iter()
        .filter(|case| case.file.contains("adversarial"))
        .count();
    let mistakes = cases.len() - adversarial;
    assert!(mistakes >= MIN_MISTAKES, "{mistakes} mistakes");
    assert!(
        adversarial >= MIN_ADVERSARIAL,
        "{adversarial} adversarial cases"
    );
    let mut ids: Vec<&str> = cases.iter().map(|case| case.id.as_str()).collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "identifiers are not unique");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
