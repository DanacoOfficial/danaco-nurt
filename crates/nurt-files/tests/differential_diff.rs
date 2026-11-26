// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential tests of `nurt_text::diff` against `diff -u`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;

use std::fs;
use std::path::Path;

use nurt_text::diff::{Unified, unified_diff};
use support_corpus::Rng;
use support_diff::{require, run_expect};

const SEEDS: u64 = 60;
const MIN_LINES: u64 = 5;
const LINE_SPREAD: u64 = 40;
const CONTEXTS: [usize; 4] = [0, 1, 3, 5];
const OLD_LABEL: &str = "a/f.txt";
const NEW_LABEL: &str = "b/f.txt";
/// Odds `1/n` that a line is deleted, replaced or followed by an insertion.
const EDIT_ODDS: u64 = 8;
const WORDS: [&str; 6] = ["alpha", "bravo", "report", "index", "stream", "buffer"];
const HEADER_LINES: usize = 2;

/// Old text with unique lines and a new text derived from it by random edits.
fn text_pair(seed: u64) -> (String, String) {
    let mut rng = Rng::new(seed);
    let count = MIN_LINES + rng.below(LINE_SPREAD);
    let mut old = String::new();
    let mut new = String::new();
    for index in 0..count {
        let word = WORDS[rng.below(WORDS.len() as u64) as usize];
        let line = format!("line {index} {word}\n");
        old.push_str(&line);
        match rng.below(EDIT_ODDS * 3) {
            0 if index % 2 == 0 => {}
            1 => new.push_str(&format!("changed {index} {word}\n")),
            2 => {
                new.push_str(&line);
                new.push_str(&format!("added {index}\n"));
            }
            _ => new.push_str(&line),
        }
    }
    (old, new)
}

/// Body of a unified diff after the two header lines, which carry time stamps.
fn body(diff: &str) -> String {
    diff.split_inclusive('\n').skip(HEADER_LINES).collect()
}

#[test]
fn the_body_of_a_diff_keeps_carriage_returns_and_a_missing_final_newline() {
    assert_eq!(body("--- a\n+++ b\n-x\r\n+y\n"), "-x\r\n+y\n");
    assert_eq!(body("--- a\n+++ b\n-x\n\\ No newline"), "-x\n\\ No newline");
}

fn reference_diff(diff: &Path, dir: &Path, old: &str, new: &str, context: usize) -> String {
    fs::create_dir_all(dir.join("a")).unwrap();
    fs::create_dir_all(dir.join("b")).unwrap();
    fs::write(dir.join(OLD_LABEL), old).unwrap();
    fs::write(dir.join(NEW_LABEL), new).unwrap();
    let context = format!("-U{context}");
    let output = run_expect(
        diff,
        &[context.as_str(), OLD_LABEL, NEW_LABEL],
        dir,
        &[0, 1],
    );
    assert_eq!(output.code == Some(0), old == new, "exit code of diff");
    String::from_utf8(output.stdout).unwrap()
}

fn nurt_diff(old: &str, new: &str, context: usize) -> String {
    let hunks = unified_diff(old, new, context, false);
    Unified::new(OLD_LABEL, NEW_LABEL, &hunks).to_string()
}

fn assert_same_diff(diff: &Path, dir: &Path, old: &str, new: &str, context: usize, label: &str) {
    let reference = reference_diff(diff, dir, old, new, context);
    let nurt = nurt_diff(old, new, context);
    assert_eq!(
        body(&nurt),
        body(&reference),
        "{label} with {context} lines of context\nold: {old:?}\nnew: {new:?}"
    );
}

#[test]
fn unified_output_matches_diff_for_random_edits() {
    let Some(diff) = require("diff") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for seed in 0..SEEDS {
        let (old, new) = text_pair(seed);
        for context in CONTEXTS {
            assert_same_diff(
                &diff,
                dir.path(),
                &old,
                &new,
                context,
                &format!("seed {seed}"),
            );
        }
    }
}

#[test]
fn identical_texts_produce_no_hunks() {
    let Some(diff) = require("diff") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (old, _) = text_pair(1);
    assert_same_diff(&diff, dir.path(), &old, &old, 3, "identical texts");
    assert!(unified_diff(&old, &old, 3, false).is_empty());
}

#[test]
fn missing_final_newline_is_marked_like_diff() {
    let Some(diff) = require("diff") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("a\nb\nc", "a\nb\nd\n"),
        ("a\nb\nc\n", "a\nb\nc"),
        ("a\nb\nc", "a\nb\nc\nd"),
        ("a\nb", "a\nb"),
    ];
    for (old, new) in cases {
        assert_same_diff(&diff, dir.path(), old, new, 3, "missing final newline");
    }
}

#[test]
fn carriage_returns_are_part_of_the_compared_lines() {
    let Some(diff) = require("diff") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n"),
        ("a\r\nb\r\nc\r\n", "a\nb\nc\n"),
        ("a\nb\n", "a\r\nb\r\n"),
    ];
    for (old, new) in cases {
        assert_same_diff(&diff, dir.path(), old, new, 3, "carriage returns");
    }
}

#[test]
fn empty_side_is_a_pure_insertion_or_deletion() {
    let Some(diff) = require("diff") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    assert_same_diff(&diff, dir.path(), "", "one\ntwo\n", 3, "empty old text");
    assert_same_diff(&diff, dir.path(), "one\ntwo\n", "", 3, "empty new text");
}

#[test]
fn repeated_lines_give_a_valid_minimal_edit_script() {
    let Some(diff) = require("diff") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let old = "x\ny\nx\ny\nx\ny\nz\n";
    let new = "y\nx\ny\nx\nz\nx\n";
    let reference = reference_diff(&diff, dir.path(), old, new, 3);
    let nurt = nurt_diff(old, new, 3);
    let changed = |text: &str| {
        text.lines()
            .filter(|line| {
                line.starts_with(['+', '-']) && !line.starts_with("+++") && !line.starts_with("---")
            })
            .count()
    };
    assert_eq!(
        changed(&nurt),
        changed(&reference),
        "size of the edit script"
    );
}
