// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/lexical_regressions.rs
// Responsibility:   Inputs that once made the front part of correction run without end. Each.
// Dependencies:     lexical_support, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Inputs that once made the front part of correction run without end. Each
//! input is read in every variant; a regression that does not end is stopped
//! by the test runner, which has a time limit for this test binary.

mod lexical_support;

use lexical_support::{Catalog, correction, syntax};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::lexical::front;
use nurt_correct::tables::load_embedded;

/// Reads `text` in every variant; the call returns only when the reading
/// ends.
fn read_in_every_variant(text: &str) {
    let tables = load_embedded().expect("embedded tables");
    let catalog = Catalog::load();
    for variant in [SyntaxVariant::A, SyntaxVariant::B, SyntaxVariant::C] {
        let _ = front(text, &syntax(variant), &correction(), &tables, &catalog);
    }
}

#[test]
fn reads_a_unicode_space_after_an_escaped_quote() {
    read_in_every_variant("find \"/a\\\"\u{2003}|> count");
}

#[test]
fn reads_unicode_spaces_inside_an_unterminated_text() {
    for text in ["list \"\\\"\u{3000}x", "\"\\\"\u{2009}", "x\u{2002}y"] {
        read_in_every_variant(text);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
