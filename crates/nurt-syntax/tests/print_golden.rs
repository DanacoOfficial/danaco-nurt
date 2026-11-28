// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/print_golden.rs
// Responsibility:   Canonical text of the golden trees.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Canonical text of the golden trees.
//!
//! The trees of the specification (`spec/golden/ast`) print to the recorded
//! canonical texts of `tests/d1/golden/ast/a`, and every golden tree prints
//! to text that reads back as the same tree.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{AstDocument, strip_for_hash};
use nurt_syntax::json;
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print::{self, PrintError};

/// Number of reference tasks with a recorded canonical text.
const TASKS: usize = 14;

/// Smallest number of program trees: 14 tasks, 48 examples, 2 extra trees.
const MIN_PROGRAM_TREES: usize = 64;

/// Suffix of the variant C trees of the reference tasks, which name every
/// argument and have no recorded canonical text.
const C_TREE_SUFFIX: &str = ".c.json";

fn workspace() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn spec_trees(prefixes: &[&str]) -> Vec<PathBuf> {
    let directory = workspace().join("spec/golden/ast");
    let mut trees: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.ends_with(".json")
                && !name.ends_with(C_TREE_SUFFIX)
                && prefixes.iter().any(|prefix| name.starts_with(prefix))
        })
        .collect();
    trees.sort();
    trees
}

fn decode(path: &Path) -> AstDocument {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    json::decode_surface(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn hash_form(document: &AstDocument) -> serde_json::Value {
    strip_for_hash(&serde_json::to_value(document).expect("tree serializes"))
}

fn stem(path: &Path) -> &str {
    path.file_stem()
        .and_then(|s| s.to_str())
        .expect("file name")
}

#[test]
fn prints_the_recorded_canonical_text_of_every_task() {
    let tasks = spec_trees(&["task-"]);
    assert_eq!(tasks.len(), TASKS);
    let canonical = workspace().join("tests/d1/golden/ast/a");
    let mut mismatches = Vec::new();
    for task in &tasks {
        let expected_path = canonical.join(format!("{}.canonical.nurt", stem(task)));
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
        let printed = print::try_print(&decode(task), Variant::A)
            .unwrap_or_else(|error| panic!("{}: {error}", task.display()));
        if printed != expected {
            mismatches.push(format!(
                "{}:\n{printed}--- expected:\n{expected}",
                stem(task)
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn every_golden_program_reads_back_as_the_same_tree() {
    let trees = spec_trees(&["task-", "example-", "extra-"]);
    assert!(
        trees.len() >= MIN_PROGRAM_TREES,
        "found {} trees",
        trees.len()
    );
    let options = ParseOptions::default();
    let mut failures = Vec::new();
    for path in &trees {
        let document = decode(path);
        let printed = print::try_print(&document, Variant::A)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        match parse::parse(&printed, Variant::A, &options) {
            Ok(parsed) if hash_form(&parsed) != hash_form(&document) => {
                failures.push(format!("{}: tree differs after\n{printed}", stem(path)));
            }
            Ok(parsed) if print::print(&parsed, Variant::A) != printed => {
                failures.push(format!(
                    "{}: printing is not idempotent\n{printed}",
                    stem(path)
                ));
            }
            Ok(_) => {}
            Err(error) => failures.push(format!("{}: {error}\n{printed}", stem(path))),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn structured_key_prints_as_key_path_text_and_is_rejected_by_try_print() {
    let document = decode(&workspace().join("spec/golden/ast/structured-key.json"));
    assert_eq!(
        print::print(&document, Variant::A),
        "set \"/srv/app/package.json\" key: r#\"workspaces[0][*].**[+][?name == \"web\"][-1]\"# value: \"vitest\"\n"
    );
    assert!(matches!(
        print::try_print(&document, Variant::A),
        Err(PrintError::Unprintable { node: "key", .. })
    ));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
