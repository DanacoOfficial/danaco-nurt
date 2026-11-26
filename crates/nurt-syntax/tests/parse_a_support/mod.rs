// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Helpers shared by the tests of the variant A parser.

#![allow(
    dead_code,
    reason = "each test crate uses a different subset of the helpers"
)]

use std::path::PathBuf;

use nurt_contracts::ast::{AST_HASH_PREFIX, AstDocument, strip_for_hash};
use nurt_syntax::error::SyntaxError;
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Keywords of the language; none of them is a valid `ident` node.
const KEYWORDS: [&str; 8] = ["let", "true", "false", "null", "and", "or", "not", "in"];

/// Directory of the golden surface trees of the specification.
pub(crate) fn golden_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/ast")
}

/// Golden programs of variant A whose file name starts with one of
/// `prefixes`, sorted by name: `(name, text)`. Programs of the variants B
/// and C carry the suffix `.b` or `.c` before `.nurt` and are skipped.
pub(crate) fn golden_programs(prefixes: &[&str]) -> Vec<(String, String)> {
    let mut programs: Vec<(String, String)> = std::fs::read_dir(golden_dir())
        .expect("golden directory is readable")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".nurt")?.to_owned();
            if stem.ends_with(".b") || stem.ends_with(".c") {
                return None;
            }
            prefixes
                .iter()
                .any(|prefix| stem.starts_with(prefix))
                .then(|| {
                    let text = std::fs::read_to_string(entry.path()).expect("golden text");
                    (stem, text)
                })
        })
        .collect();
    programs.sort();
    programs
}

/// Parses variant A with default options.
pub(crate) fn parse_a(text: &str) -> Result<AstDocument, SyntaxError> {
    parse(text, Variant::A, &ParseOptions::default())
}

/// Parses variant A and returns the JSON form of the tree.
pub(crate) fn json_a(text: &str) -> Value {
    let document = parse_a(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
    serde_json::to_value(&document).expect("tree serializes")
}

/// JSON form of the first statement without spans.
pub(crate) fn statement(text: &str) -> Value {
    let json = strip_for_hash(&json_a(text));
    json["program"]["statements"][0].clone()
}

/// JSON form, without spans, of the value of the first argument of the
/// head stage of the first statement.
pub(crate) fn first_arg(text: &str) -> Value {
    statement(text)["head"]["args"][0]["value"].clone()
}

/// `ast_hash` of a tree document.
pub(crate) fn ast_hash(document: &AstDocument) -> String {
    let value = serde_json::to_value(document).expect("tree serializes");
    let canonical =
        serde_json_canonicalizer::to_string(&strip_for_hash(&value)).expect("canonical JSON");
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{AST_HASH_PREFIX}{}", hex::encode(digest))
}

/// Checks the invariants of a surface tree that its types do not express:
/// spans nest in the span of their parent and do not overlap their
/// siblings, `field` has `of`, `spread` is a list item, integers are
/// non-negative, identifiers are not keywords, map keys are unique and a
/// reference omits an empty field path.
pub(crate) fn check_invariants(document: &Value) -> Result<(), String> {
    check_value(document, None, false)
}

fn span_of(value: &Value) -> Option<[u64; 4]> {
    let span = value.get("span")?.as_array()?;
    let numbers: Vec<u64> = span.iter().filter_map(Value::as_u64).collect();
    <[u64; 4]>::try_from(numbers).ok()
}

fn start(span: [u64; 4]) -> (u64, u64) {
    (span[0], span[1])
}

fn end(span: [u64; 4]) -> (u64, u64) {
    (span[2], span[3])
}

fn check_value(value: &Value, parent: Option<[u64; 4]>, in_list: bool) -> Result<(), String> {
    match value {
        Value::Object(members) => {
            check_node(value, in_list)?;
            let own = span_of(value);
            if let (Some(own), Some(parent)) = (own, parent)
                && (start(own) < start(parent) || end(own) > end(parent))
            {
                return Err(format!("span {own:?} outside parent {parent:?}"));
            }
            if let Some(own) = own
                && start(own) > end(own)
            {
                return Err(format!("span {own:?} ends before it starts"));
            }
            let scope = own.or(parent);
            let mut children = Vec::new();
            for (name, child) in members {
                if name == "span" {
                    continue;
                }
                check_value(child, scope, name == "items")?;
                collect_spans(child, &mut children);
            }
            check_siblings(children)
        }
        Value::Array(items) => {
            for item in items {
                check_value(item, parent, in_list)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn collect_spans(value: &Value, spans: &mut Vec<[u64; 4]>) {
    match value {
        Value::Object(members) => {
            if let Some(span) = span_of(value) {
                spans.push(span);
            } else {
                for (name, member) in members {
                    if name != "span" {
                        collect_spans(member, spans);
                    }
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_spans(item, spans)),
        _ => {}
    }
}

fn check_siblings(mut spans: Vec<[u64; 4]>) -> Result<(), String> {
    spans.sort_by_key(|&span| start(span));
    for pair in spans.windows(2) {
        if start(pair[1]) < end(pair[0]) {
            return Err(format!(
                "sibling spans {:?} and {:?} overlap",
                pair[0], pair[1]
            ));
        }
    }
    Ok(())
}

fn check_node(node: &Value, in_list: bool) -> Result<(), String> {
    match node.get("t").and_then(Value::as_str) {
        Some("field") if node.get("of").is_none() => Err("field without `of`".to_owned()),
        Some("spread") if !in_list => Err("spread outside a list".to_owned()),
        Some("int") if node["v"].as_i64().is_none_or(|v| v < 0) => {
            Err(format!("negative integer {}", node["v"]))
        }
        Some("ident") if KEYWORDS.contains(&node["v"].as_str().unwrap_or_default()) => {
            Err(format!("keyword {} as identifier", node["v"]))
        }
        Some("ref")
            if node
                .get("fields")
                .is_some_and(|f| f == &Value::Array(Vec::new())) =>
        {
            Err("reference with an empty field path".to_owned())
        }
        Some("map") => {
            let entries = node["entries"].as_array().cloned().unwrap_or_default();
            let mut keys: Vec<&str> = entries.iter().filter_map(|e| e["k"].as_str()).collect();
            let count = keys.len();
            keys.sort_unstable();
            keys.dedup();
            if keys.len() == count {
                Ok(())
            } else {
                Err("duplicate map key".to_owned())
            }
        }
        _ => Ok(()),
    }
}
