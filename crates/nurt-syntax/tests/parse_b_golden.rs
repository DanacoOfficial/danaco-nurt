// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden programs of the call notation (variant B).
//!
//! Every program in `tests/d1/golden/ast/b` has a word notation counterpart
//! of the same name in `spec/golden/ast`. Both notations describe the same
//! program, so the variant B text parses into the surface tree of the word
//! notation, compared without spans and metadata, and has its recorded
//! `ast_hash`.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{AST_HASH_PREFIX, AstDocument, strip_for_hash};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Number of golden task programs.
const TASK_PROGRAMS: usize = 14;
/// Smallest number of golden example programs written in variant B.
const MIN_EXAMPLE_PROGRAMS: usize = 10;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn variant_b_dir() -> PathBuf {
    repository_root().join("tests/d1/golden/ast/b")
}

fn word_notation_dir() -> PathBuf {
    repository_root().join("spec/golden/ast")
}

/// Returns the variant B programs whose file name starts with `prefix`,
/// sorted by name.
fn programs(prefix: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(variant_b_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "nurt")
                && stem(path).starts_with(prefix)
        })
        .collect();
    paths.sort();
    paths
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

fn ast_hash(document: &Value) -> String {
    let canonical = serde_json_canonicalizer::to_vec(&strip_for_hash(document)).unwrap();
    format!(
        "{AST_HASH_PREFIX}{}",
        hex::encode(Sha256::digest(&canonical))
    )
}

fn parse_b(path: &Path) -> (String, Value) {
    let text = fs::read_to_string(path).unwrap();
    let document = parse(&text, Variant::B, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
    (text, serde_json::to_value(&document).unwrap())
}

/// Checks that a variant B program has the tree and the hash of its word
/// notation counterpart.
fn assert_same_as_word_notation(path: &Path) {
    let name = stem(path);
    let (_, actual) = parse_b(path);
    let expected_path = word_notation_dir().join(format!("{name}.json"));
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(&expected_path).unwrap()).unwrap();
    assert_eq!(
        strip_for_hash(&actual),
        strip_for_hash(&expected),
        "{name}: tree differs from the word notation"
    );
    let recorded = fs::read_to_string(word_notation_dir().join(format!("{name}.hash"))).unwrap();
    assert_eq!(ast_hash(&actual), recorded.trim(), "{name}: ast_hash");
    assert_eq!(ast_hash(&actual), ast_hash(&expected), "{name}: ast_hash");
}

/// A span as `[line, column, end_line, end_column]`.
type Position = (u64, u64);

fn span_of(node: &Value) -> Option<(Position, Position)> {
    let span = node.get("span")?.as_array()?;
    let number = |index: usize| span.get(index).and_then(Value::as_u64).unwrap();
    Some(((number(0), number(1)), (number(2), number(3))))
}

/// Collects the nearest descendants of `node` that carry a span, in
/// document order.
fn spanned_children<'a>(node: &'a Value, children: &mut Vec<&'a Value>) {
    match node {
        Value::Object(members) => {
            for (name, value) in members {
                if name == "span" {
                    continue;
                }
                if span_of(value).is_some() {
                    children.push(value);
                } else {
                    spanned_children(value, children);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if span_of(item).is_some() {
                    children.push(item);
                } else {
                    spanned_children(item, children);
                }
            }
        }
        _ => {}
    }
}

/// Checks that every span lies within the span of its nearest spanned
/// ancestor and that sibling spans follow each other without overlap.
fn assert_spans_nested(node: &Value, outer: Option<(Position, Position)>, context: &str) {
    let own = span_of(node);
    if let (Some((start, end)), Some((outer_start, outer_end))) = (own, outer) {
        assert!(
            outer_start <= start && end <= outer_end,
            "{context}: span {own:?} outside of {outer:?}"
        );
    }
    if let Some((start, end)) = own {
        assert!(
            start <= end,
            "{context}: span {own:?} ends before it starts"
        );
    }
    let mut children = Vec::new();
    spanned_children(node, &mut children);
    let mut previous_end: Option<Position> = None;
    for child in &children {
        let child_span = span_of(child);
        if let (Some(end), Some((start, _))) = (previous_end, child_span) {
            assert!(
                end <= start,
                "{context}: sibling spans overlap at {child_span:?}"
            );
        }
        previous_end = child_span.map(|(_, end)| end);
        assert_spans_nested(child, own.or(outer), context);
    }
}

/// Returns the character at a 1-based line and column of `text`.
fn char_at(text: &str, (line, column): Position) -> Option<char> {
    let line_text = text
        .split('\n')
        .nth(usize::try_from(line).ok()?.checked_sub(1)?)?;
    line_text
        .chars()
        .nth(usize::try_from(column).ok()?.checked_sub(1)?)
}

/// Checks that every stage span of a variant B program starts with the
/// command name and ends with the closing parenthesis.
fn assert_stage_spans_cover_parentheses(text: &str, node: &Value, context: &str) {
    match node {
        Value::Object(members) => {
            if members.get("t").and_then(Value::as_str) == Some("stage") {
                let ((line, column), (end_line, end_column)) = span_of(node).unwrap();
                let name = members["name"].as_str().unwrap();
                assert_eq!(
                    char_at(text, (line, column)),
                    name.chars().next(),
                    "{context}: stage `{name}` span start"
                );
                assert_eq!(
                    char_at(text, (end_line, end_column - 1)),
                    Some(')'),
                    "{context}: stage `{name}` span end"
                );
            }
            for value in members.values() {
                assert_stage_spans_cover_parentheses(text, value, context);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_stage_spans_cover_parentheses(text, item, context);
            }
        }
        _ => {}
    }
}

#[test]
fn task_programs_match_word_notation_trees_and_hashes() {
    let paths = programs("task-");
    assert_eq!(paths.len(), TASK_PROGRAMS);
    for path in &paths {
        assert_same_as_word_notation(path);
    }
}

#[test]
fn example_programs_match_word_notation_trees_and_hashes() {
    let paths = programs("example-");
    assert!(
        paths.len() >= MIN_EXAMPLE_PROGRAMS,
        "{} examples",
        paths.len()
    );
    for path in &paths {
        assert_same_as_word_notation(path);
    }
}

#[test]
fn extra_programs_match_word_notation_trees_and_hashes() {
    for path in &programs("extra-") {
        assert_same_as_word_notation(path);
    }
}

#[test]
fn golden_trees_have_nested_and_ordered_spans() {
    for path in &programs("") {
        let (text, tree) = parse_b(path);
        let name = stem(path);
        assert_spans_nested(&tree, None, &name);
        assert_stage_spans_cover_parentheses(&text, &tree, &name);
    }
}

#[test]
fn golden_trees_deserialize_into_contract_types() {
    for path in &programs("") {
        let (_, tree) = parse_b(path);
        let document: AstDocument = serde_json::from_value(tree.clone()).unwrap();
        assert_eq!(serde_json::to_value(&document).unwrap(), tree);
    }
}
