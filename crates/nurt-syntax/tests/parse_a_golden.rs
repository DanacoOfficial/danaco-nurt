// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_a_golden.rs
// Responsibility:   Golden surface trees of the specification: the reference tasks, the examples and the extra programs, compared with their spans and their `ast_hash`.
// Dependencies:     parse_a_support, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Golden surface trees of the specification: the reference tasks, the
//! examples and the extra programs, compared with their spans and their
//! `ast_hash`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod parse_a_support;

use nurt_syntax::parse::{ParseOptions, Variant, parse};
use parse_a_support::{ast_hash, check_invariants, golden_dir, golden_programs};
use serde_json::Value;

/// Number of reference tasks.
const TASKS: usize = 14;

/// Number of examples.
const EXAMPLES: usize = 48;

/// Parse options that reproduce the metadata of a golden tree: the source
/// text and the comments are kept when the golden tree records them.
fn options_for(expected: &Value) -> ParseOptions {
    let meta = expected.get("meta");
    let records = |member: &str| meta.is_some_and(|meta| meta.get(member).is_some());
    ParseOptions {
        keep_comments: records("comments"),
        keep_source: records("source"),
        ..ParseOptions::default()
    }
}

/// The program text recorded in `meta.source` of a golden tree, which is the
/// program file without its final line end; `file` when nothing is recorded.
fn recorded_source<'a>(expected: &'a Value, file: &'a str) -> &'a str {
    match expected.pointer("/meta/source").and_then(Value::as_str) {
        Some(source) => {
            assert_eq!(
                source,
                file.trim_end_matches(['\n', '\r']),
                "meta.source is the program text"
            );
            source
        }
        None => file,
    }
}

fn assert_golden(prefix: &str, expected_count: usize) {
    let programs = golden_programs(&[prefix]);
    assert_eq!(
        programs.len(),
        expected_count,
        "golden programs `{prefix}*`"
    );
    let mut mismatches = Vec::new();
    for (name, text) in &programs {
        let tree = std::fs::read_to_string(golden_dir().join(format!("{name}.json")))
            .expect("golden tree");
        let expected: Value = serde_json::from_str(&tree).expect("golden tree is JSON");
        let text = recorded_source(&expected, text);
        let document = match parse(text, Variant::A, &options_for(&expected)) {
            Ok(document) => document,
            Err(error) => {
                mismatches.push(format!("{name}: {error}"));
                continue;
            }
        };
        let actual = serde_json::to_value(&document).expect("tree serializes");
        if actual != expected {
            mismatches.push(format!(
                "{name}: tree differs\nexpected {expected}\nactual   {actual}"
            ));
        }
        let hash = std::fs::read_to_string(golden_dir().join(format!("{name}.hash")))
            .expect("golden hash");
        if ast_hash(&document) != hash.trim() {
            mismatches.push(format!("{name}: ast_hash differs"));
        }
        if let Err(problem) = check_invariants(&actual) {
            mismatches.push(format!("{name}: {problem}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn reference_tasks_match_their_golden_trees() {
    assert_golden("task-", TASKS);
}

#[test]
fn examples_match_their_golden_trees() {
    assert_golden("example-", EXAMPLES);
}

#[test]
fn extra_programs_match_their_golden_trees() {
    assert_golden("extra-", 2);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
