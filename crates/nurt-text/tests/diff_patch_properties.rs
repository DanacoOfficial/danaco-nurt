// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/diff_patch_properties.rs
// Responsibility:   Properties of differences and patches on generated texts.
// Dependencies:     proptest
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Properties of differences and patches on generated texts.
//!
//! Texts are built from a few short lines, including an empty line, a line
//! with a carriage return and lines that differ only in white space, so that
//! repeated lines and ambiguous alignments are frequent.

use nurt_text::diff::{Hunk, Unified, unified_diff};
use nurt_text::patch::{ApplyOptions, FileChange, Patch, apply_hunks, plan};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

/// Lines texts are built from.
const LINES: [&str; 7] = ["a", "b", "c", "", " a", "a\r", "x y"];

/// Fixed seed so that every run checks the same cases.
const SEED: [u8; 32] = *b"nurt-text diff and patch propert";

const CASES: u32 = 10_000;

/// Largest number of context lines tried.
const MAX_CONTEXT: usize = 4;

fn runner() -> TestRunner {
    let config = Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    };
    TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &SEED))
}

fn text() -> impl Strategy<Value = String> {
    (prop::collection::vec(0..LINES.len(), 0..16), any::<bool>()).prop_map(
        |(indexes, final_newline)| {
            let mut text = indexes
                .iter()
                .map(|&index| LINES[index])
                .collect::<Vec<_>>()
                .join("\n");
            if final_newline && !indexes.is_empty() {
                text.push('\n');
            }
            text
        },
    )
}

fn pair() -> impl Strategy<Value = (String, String, usize)> {
    (text(), text(), 0..=MAX_CONTEXT)
}

fn reversed(hunks: &[Hunk]) -> Vec<Hunk> {
    hunks.iter().map(Hunk::reversed).collect()
}

#[test]
fn applying_difference_gives_new_text() {
    runner()
        .run(&pair(), |(old, new, context)| {
            let hunks = unified_diff(&old, &new, context, false);
            let applied = apply_hunks(&old, &hunks, 0)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(applied.text, new);
            prop_assert!(
                applied
                    .hunks
                    .iter()
                    .all(|outcome| outcome.offset == 0 && outcome.fuzz == 0)
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn applying_reversed_difference_gives_old_text() {
    runner()
        .run(&pair(), |(old, new, context)| {
            let hunks = reversed(&unified_diff(&old, &new, context, false));
            let applied = apply_hunks(&new, &hunks, 0)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(applied.text, old);
            Ok(())
        })
        .unwrap();
}

#[test]
fn rendered_difference_parses_and_applies() {
    runner()
        .run(&pair(), |(old, new, context)| {
            let hunks = unified_diff(&old, &new, context, false);
            prop_assume!(!hunks.is_empty());
            let text = Unified::new("a/file", "b/file", &hunks).to_string();
            let patch =
                Patch::parse(&text).map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(&patch.files[0].hunks, &hunks);

            let options = ApplyOptions {
                strip: 1,
                ..ApplyOptions::default()
            };
            let changes = plan(&patch, &options, |_| Some(old.clone()))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let expected = FileChange::Modify {
                path: "file".to_owned(),
                content: new.clone(),
                mode: None,
            };
            prop_assert_eq!(&changes[0].change, &expected);

            let options = ApplyOptions {
                reverse: true,
                ..options
            };
            let changes = plan(&patch, &options, |_| Some(new.clone()))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let expected = FileChange::Modify {
                path: "file".to_owned(),
                content: old.clone(),
                mode: None,
            };
            prop_assert_eq!(&changes[0].change, &expected);
            Ok(())
        })
        .unwrap();
}

#[test]
fn hunk_ranges_match_their_lines() {
    runner()
        .run(
            &(text(), text(), 0..=MAX_CONTEXT, any::<bool>()),
            |(old, new, context, ignore_whitespace)| {
                for hunk in unified_diff(&old, &new, context, ignore_whitespace) {
                    let old_lines = hunk.lines.iter().filter(|line| line.kind.in_old()).count();
                    let new_lines = hunk.lines.iter().filter(|line| line.kind.in_new()).count();
                    prop_assert_eq!((old_lines, new_lines), (hunk.old_len, hunk.new_len));
                    prop_assert!(hunk.old_len > 0 || hunk.new_len > 0);
                }
                Ok(())
            },
        )
        .unwrap();
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
