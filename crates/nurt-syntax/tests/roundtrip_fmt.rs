// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/roundtrip_fmt.rs
// Responsibility:   Formatting keeps comments, is idempotent and does not change the tree.
// Dependencies:     nurt_contracts, proptest, roundtrip_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Formatting keeps comments, is idempotent and does not change the tree.
//!
//! Comments are inserted into the canonical text of a generated tree: own
//! lines before statements and stages, and line ends after the last line of
//! a statement element. Such text is already canonical, so formatting must
//! return it unchanged.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod roundtrip_support;

use nurt_contracts::ast::{AstDocument, Program};
use nurt_syntax::fmt;
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print;
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use roundtrip_support::{Shape, hash_form, program};

/// Generated programs per run.
const CASES: u32 = 20_000;

/// Insertion choices drawn per program; lines beyond this count get none.
const CHOICES: usize = 64;

/// Canonical text of `document` with comments inserted where `choices`
/// says so, and the inserted comment texts.
fn with_comments(document: &AstDocument, choices: &[bool]) -> (String, Vec<String>) {
    let mut choice = choices.iter().copied().chain(std::iter::repeat(false));
    let mut text = String::new();
    let mut comments = Vec::new();
    for statement in &document.program.statements {
        let single = AstDocument::new(Program {
            statements: vec![statement.clone()],
        });
        let printed = print::print(&single, Variant::A);
        let lines: Vec<&str> = printed.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let starts = index == 0 || line.starts_with("|> ");
            let ends = index + 1 == lines.len() || lines[index + 1].starts_with("|> ");
            if starts && choice.next().unwrap_or(false) {
                let comment = format!(" before {}", comments.len());
                text.push_str(&format!("#{comment}\n"));
                comments.push(comment);
            }
            text.push_str(line);
            if ends && choice.next().unwrap_or(false) {
                let comment = format!(" after {}", comments.len());
                text.push_str(&format!(" #{comment}"));
                comments.push(comment);
            }
            text.push('\n');
        }
    }
    (text, comments)
}

fn comment_texts(text: &str) -> Result<Vec<String>, TestCaseError> {
    let options = ParseOptions {
        keep_comments: true,
        ..ParseOptions::default()
    };
    let document = parse::parse(text, Variant::A, &options)
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    let traces = document
        .meta
        .as_ref()
        .and_then(|meta| meta.get("comments"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut texts: Vec<String> = traces
        .iter()
        .filter_map(|trace| {
            trace
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    texts.sort();
    Ok(texts)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: CASES, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn formatting_keeps_comments_and_the_tree(
        document in program(Shape::SINGLE_LINE_TEXT),
        choices in vec(any::<bool>(), CHOICES),
    ) {
        let (text, mut inserted) = with_comments(&document, &choices);
        let formatted = fmt::format(&text, Variant::A).map_err(|error| TestCaseError::fail(format!("{error}\n{text}")))?;
        prop_assert_eq!(&formatted, &text);
        let again = fmt::format(&formatted, Variant::A).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(&again, &formatted);
        let parsed = parse::parse(&formatted, Variant::A, &ParseOptions::default())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(hash_form(&parsed), hash_form(&document));
        inserted.sort();
        prop_assert_eq!(comment_texts(&formatted)?, inserted);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
