// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/hash_golden.rs
// Responsibility:   Recorded tree hashes of the golden files.
// Dependencies:     hash_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Recorded tree hashes of the golden files.
//!
//! Every golden tree is decoded with the JSON decoder of the crate and its
//! `ast_hash` is compared with the recorded `.hash` file. With `NURT_BLESS=1`
//! the hashes of `tests/d1/golden` and the index `tests/d1/golden/hashes.tsv`
//! are written instead; the specification files are always compared.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use std::fs;

use hash_support::{
    GoldenFile, HASH_INDEX, HASH_INDEX_HEADER, all_trees, bless, d1_golden, d1_relative, d1_trees,
    read_text, spec_trees,
};

/// Smallest number of specification trees: 14 tasks, 48 examples and the
/// bound trees.
const MIN_SPEC_TREES: usize = 62;

fn recorded(tree: &GoldenFile) -> Option<String> {
    fs::read_to_string(tree.hash_path())
        .ok()
        .map(|text| text.trim_end().to_owned())
}

fn check(tree: &GoldenFile, writable: bool) -> Option<String> {
    let hash = tree.decode().hash();
    if writable && bless() {
        fs::write(tree.hash_path(), format!("{hash}\n"))
            .unwrap_or_else(|error| panic!("{}: {error}", tree.hash_path().display()));
        return None;
    }
    match recorded(tree) {
        Some(expected) if expected == hash => None,
        Some(expected) => Some(format!(
            "{}: recorded {expected}, computed {hash}",
            tree.path.display()
        )),
        None => Some(format!(
            "{}: no recorded hash; run the test with NURT_BLESS=1 and review the result",
            tree.path.display()
        )),
    }
}

#[test]
fn reproduces_every_hash_of_the_specification() {
    let trees = spec_trees();
    assert!(trees.len() >= MIN_SPEC_TREES, "found {} trees", trees.len());
    let failures: Vec<String> = trees.iter().filter_map(|tree| check(tree, false)).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn reproduces_every_hash_of_the_language_golden_files() {
    let failures: Vec<String> = d1_trees()
        .iter()
        .filter_map(|tree| check(tree, true))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

fn hash_index() -> String {
    let mut index = String::from(HASH_INDEX_HEADER);
    index.push('\n');
    for tree in d1_trees() {
        let hash = tree.decode().hash();
        index.push_str(&format!(
            "{}\t{}\t{hash}\n",
            d1_relative(&tree.path),
            tree.format.name()
        ));
    }
    index
}

#[test]
fn hash_index_lists_every_language_golden_tree() {
    let path = d1_golden().join(HASH_INDEX);
    let index = hash_index();
    if bless() {
        fs::write(&path, &index).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        return;
    }
    assert_eq!(
        read_text(&path),
        index,
        "{} is out of date; run the test with NURT_BLESS=1 and review the result",
        path.display()
    );
}

#[test]
fn every_recorded_hash_has_the_canonical_form() {
    for tree in all_trees() {
        let Some(hash) = recorded(&tree) else {
            continue;
        };
        let digest = hash.strip_prefix("sha256:").unwrap_or_default();
        assert!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{}: {hash}",
            tree.path.display()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
