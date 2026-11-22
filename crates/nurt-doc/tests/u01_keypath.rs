// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Key path vectors of `tests/d5/data/u01/keypath.toml` and the round trip
//! of the canonical printer.
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use std::path::PathBuf;

use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use toml_edit::{DocumentMut, InlineTable};

/// Smallest number of vectors in the file.
const MIN_VECTORS: usize = 200;
/// Cases of the printer round trip property.
const PROPERTY_CASES: u32 = 2_000;
/// Seed of the property, fixed so that runs are reproducible.
const PROPERTY_SEED: u64 = 0x6E75_7274;

fn vectors() -> DocumentMut {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/d5/data/u01/keypath.toml");
    std::fs::read_to_string(&path)
        .expect("vector file is readable")
        .parse()
        .expect("vector file is valid TOML")
}

fn entries<'d>(document: &'d DocumentMut, name: &str) -> Vec<&'d InlineTable> {
    document[name]
        .as_array()
        .expect("array of vectors")
        .iter()
        .map(|entry| entry.as_inline_table().expect("vector is an inline table"))
        .collect()
}

fn field(entry: &InlineTable, name: &str) -> Option<String> {
    entry
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

#[test]
fn vector_file_has_at_least_two_hundred_vectors() {
    let document = vectors();
    let total = entries(&document, "valid").len() + entries(&document, "invalid").len();
    assert!(total >= MIN_VECTORS, "{total} vectors");
}

#[test]
fn valid_vectors_print_canonically_and_parse_back() {
    let document = vectors();
    let mut failures = Vec::new();
    for entry in entries(&document, "valid") {
        let input = field(entry, "input").expect("input");
        let canonical = field(entry, "canonical").expect("canonical");
        let key = match KeyPath::parse(&input) {
            Ok(key) => key,
            Err(error) => {
                failures.push(format!("{input:?} fails: {error}"));
                continue;
            }
        };
        let printed = key.to_string();
        if printed != canonical {
            failures.push(format!(
                "{input:?} prints {printed:?}, expected {canonical:?}"
            ));
            continue;
        }
        match KeyPath::parse(&printed) {
            Ok(again) if again == key => {}
            Ok(again) => {
                failures.push(format!("{printed:?} parses to {again:?}, expected {key:?}"))
            }
            Err(error) => failures.push(format!("canonical {printed:?} fails: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn invalid_vectors_fail_with_key_syntax_kind_and_fix() {
    let document = vectors();
    let mut failures = Vec::new();
    for entry in entries(&document, "invalid") {
        let input = field(entry, "input").expect("input");
        let error = match KeyPath::parse(&input) {
            Ok(key) => {
                failures.push(format!("{input:?} parses to {key}"));
                continue;
            }
            Err(error) => error,
        };
        assert_eq!(error.code(), "E_KEY_SYNTAX");
        if let Some(kind) = field(entry, "kind") {
            let actual = serde_json::to_value(error.kind).expect("kind serializes");
            if actual != serde_json::Value::String(kind.clone()) {
                failures.push(format!("{input:?} fails as {actual}, expected {kind}"));
            }
        }
        if let Some(fix) = field(entry, "fix")
            && error.fix.as_deref() != Some(fix.as_str())
        {
            failures.push(format!(
                "{input:?} suggests {:?}, expected {fix:?}",
                error.fix
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn key_syntax_error_reports_column_of_problem() {
    let error = KeyPath::parse("a.[0]").expect_err("dot before bracket");
    assert!(error.column >= 2, "column {}", error.column);
    assert!(error.to_string().starts_with("E_KEY_SYNTAX at column"));
}

fn name() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-z][a-z0-9_-]{0,6}",
        2 => "[ -~]{1,6}",
        1 => "\\PC{1,4}",
    ]
}

fn segment() -> impl Strategy<Value = KeySegment> {
    prop_oneof![
        4 => name().prop_map(KeySegment::Key),
        2 => any::<i64>().prop_map(KeySegment::Index),
        1 => Just(KeySegment::Wildcard),
        1 => Just(KeySegment::Recursive),
    ]
}

fn document() -> impl Strategy<Value = Option<DocSelector>> {
    proptest::option::of(prop_oneof![
        (1_u32..64).prop_map(DocSelector::Index),
        Just(DocSelector::All),
    ])
}

fn key_path() -> impl Strategy<Value = KeyPath> {
    (
        document(),
        proptest::collection::vec(segment(), 0..6),
        any::<bool>(),
    )
        .prop_filter_map(
            "segments form a valid key path",
            |(document, mut segments, append)| {
                if append {
                    segments.push(KeySegment::Append);
                }
                KeyPath::new(document, segments).ok()
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: PROPERTY_CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        ..ProptestConfig::default()
    })]

    #[test]
    fn printed_key_path_parses_to_the_same_key_path(key in key_path()) {
        let printed = key.to_string();
        let parsed = KeyPath::parse(&printed);
        prop_assert_eq!(parsed.as_ref().ok(), Some(&key), "printed as {:?}: {:?}", printed, parsed);
    }
}
