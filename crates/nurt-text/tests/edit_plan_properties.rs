// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/edit_plan_properties.rs
// Responsibility:   Properties of replacements, line changes and templates.
// Dependencies:     nurt_contracts, proptest
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Properties of replacements, line changes and templates.

// The replacement helper outside `#[test]` functions unwraps plans that the
// generated inputs cannot make fail; a failure there is a defect of the test.
#![allow(clippy::unwrap_used)]

use nurt_contracts::value::Range;
use nurt_text::edit_plan::{
    Change, EditOptions, InsertChange, Pattern, Position, ReplaceChange, plan,
};
use nurt_text::fuzzy_match::Whitespace;
use nurt_text::template::{Groups, Template};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

/// Cases per property.
const CASES: u32 = 10_000;
/// Seed shared by all properties, so a failure reproduces on every run.
const SEED: u64 = 0x6e75_7274;

fn config() -> Config {
    Config {
        cases: CASES,
        rng_seed: RngSeed::Fixed(SEED),
        failure_persistence: None,
        ..Config::default()
    }
}

fn replace_all(text: &str, pattern: &str, with: &str, whitespace: Whitespace) -> String {
    let change = ReplaceChange {
        whitespace: Some(whitespace),
        ..ReplaceChange::new(Pattern::Literal(pattern.to_owned()), with)
    };
    plan(text, &[Change::Replace(change)], &EditOptions::default())
        .unwrap()
        .text
}

fn whitespace() -> impl Strategy<Value = Whitespace> {
    prop::sample::select(vec![Whitespace::Exact, Whitespace::Normalize])
}

proptest! {
    #![proptest_config(config())]

    /// The text uses letters `a`, `b` and white space; the replacement uses
    /// letters `X`, `Y`, `Z`, so it never occurs in the text and its copies
    /// are the only matches of the inverse replacement.
    #[test]
    fn replacement_followed_by_inverse_restores_text(
        text in "[ab \t\n]{0,60}",
        pattern in "[ab]{1,4}",
        with in "[XYZ]{1,4}",
        whitespace in whitespace(),
    ) {
        let replaced = replace_all(&text, &pattern, &with, whitespace);
        prop_assert!(!replaced.contains(&pattern));
        prop_assert_eq!(replace_all(&replaced, &with, &pattern, whitespace), text);
    }

    #[test]
    fn replacement_in_crlf_text_keeps_crlf(
        words in prop::collection::vec("[a-c]{1,3}", 1..8),
        target in "[a-c]{1,3}",
        replacement in prop::collection::vec("[x-z]{1,3}", 1..4),
    ) {
        let text = words.join("\r\n") + "\r\n";
        let result = replace_all(&text, &target, &replacement.join("\n"), Whitespace::Normalize);
        let bare = result
            .match_indices('\n')
            .filter(|(at, _)| !result[..*at].ends_with('\r'))
            .count();
        prop_assert_eq!(bare, 0);
    }

    #[test]
    fn deleting_and_reinserting_lines_restores_text(
        words in prop::collection::vec("[a-z]{1,5}", 1..12),
        terminated in any::<bool>(),
        first in 0usize..12,
        span in 0usize..12,
    ) {
        let mut text = words.join("\n");
        if terminated {
            text.push('\n');
        }
        let count = words.len();
        let first = first % count;
        let last = (first + span) % count;
        let (first, last) = (first.min(last), first.max(last));
        let removed = words[first..=last].join("\n");
        let changes = [
            Change::DeleteLines(
                Range::new(Some(first as i64 + 1), Some(last as i64 + 1)).unwrap(),
            ),
            Change::Insert(InsertChange {
                text: removed,
                line: Some(first as i64 + 1),
                position: Position::Before,
            }),
        ];
        let edit = plan(&text, &changes, &EditOptions::default()).unwrap();
        prop_assert_eq!(edit.text, text);
    }

    #[test]
    fn escaped_dollars_expand_to_the_original_text(text in "[a-z$ {}0-9]{0,40}") {
        let template = Template::parse(&text.replace('$', "$$"), &Groups::default()).unwrap();
        let captures = regex::Regex::new("").unwrap().captures("").unwrap();
        let mut out = String::new();
        template.expand_into(&captures, &mut out);
        prop_assert_eq!(out, text);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
