// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden surface trees of the specification: the reference tasks, the
//! examples and the extra programs, compared with their spans and their
//! `ast_hash`.

mod parse_a_support;

use parse_a_support::{ast_hash, check_invariants, golden_dir, golden_programs, parse_a};
use serde_json::Value;

/// Number of reference tasks.
const TASKS: usize = 14;

/// Number of examples.
const EXAMPLES: usize = 48;

fn assert_golden(prefix: &str, expected_count: usize) {
    let programs = golden_programs(&[prefix]);
    assert_eq!(
        programs.len(),
        expected_count,
        "golden programs `{prefix}*`"
    );
    let mut mismatches = Vec::new();
    for (name, text) in &programs {
        let document = match parse_a(text) {
            Ok(document) => document,
            Err(error) => {
                mismatches.push(format!("{name}: {error}"));
                continue;
            }
        };
        let actual = serde_json::to_value(&document).expect("tree serializes");
        let tree = std::fs::read_to_string(golden_dir().join(format!("{name}.json")))
            .expect("golden tree");
        let expected: Value = serde_json::from_str(&tree).expect("golden tree is JSON");
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
