// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! YAML Test Suite: every valid stream is read with the documents and
//! values the suite expects, every invalid stream is rejected.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "the suite loader and helpers outside test functions fail the test on unexpected input"
)]

#[path = "u05_support/suite.rs"]
mod suite;

use nurt_contracts::value::Value;
use nurt_doc::formats::yaml::YamlModel;
use nurt_doc::value::equivalent;

/// Valid cases that are read differently from the suite, with the reason.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    (
        "ZYU8-02",
        "reserved directive with parameters; the scanner rejects it",
    ),
    (
        "565N",
        "`!!binary` is read as bytes; the suite JSON holds the base64 text",
    ),
];

/// Expected values of the documents of a case, from its JSON field.
fn expected_documents(json: &str) -> Vec<Value> {
    serde_json::Deserializer::from_str(json)
        .into_iter::<serde_json::Value>()
        .map(|document| Value::from_plain_json(document.unwrap()))
        .collect()
}

fn known(id: &str) -> bool {
    KNOWN_DIFFERENCES.iter().any(|(known, _)| *known == id)
}

/// Difference between the model of a valid case and the suite, if any.
fn difference(case: &suite::SuiteCase) -> Option<String> {
    let model = match YamlModel::parse(&case.yaml) {
        Ok(model) => model,
        Err(error) => return Some(format!("rejected: {}", error.message)),
    };
    if model.source() != case.yaml {
        return Some("source not kept".to_owned());
    }
    let json = case.json.as_deref().filter(|_| !case.yaml.contains("<<"))?;
    let expected = expected_documents(json);
    let documents: Vec<Value> = (0..model.documents().len())
        .map(|index| model.document_value(index).unwrap_or_default())
        .filter(|value| !(expected.is_empty() && *value == Value::Null))
        .collect();
    let same = documents.len() == expected.len()
        && documents
            .iter()
            .zip(&expected)
            .all(|(actual, wanted)| equivalent(actual, wanted));
    (!same).then(|| format!("values {documents:?}, expected {expected:?}"))
}

#[test]
fn reads_valid_streams_with_the_expected_values() {
    let cases: Vec<suite::SuiteCase> = suite::load_cases()
        .into_iter()
        .filter(|case| !case.fail)
        .collect();
    let with_tree = cases.iter().filter(|case| case.tree.is_some()).count();
    let failures: Vec<String> = cases
        .iter()
        .filter(|case| !known(&case.id))
        .filter_map(|case| difference(case).map(|reason| format!("{}: {reason}", case.id)))
        .collect();
    println!(
        "valid cases {} (with event tree {with_tree}), differences {}",
        cases.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn rejects_invalid_streams() {
    let accepted: Vec<String> = suite::load_cases()
        .into_iter()
        .filter(|case| case.fail && YamlModel::parse(&case.yaml).is_ok())
        .map(|case| case.id)
        .collect();
    assert!(accepted.is_empty(), "{accepted:?}");
}

#[test]
fn converts_the_visual_notation_of_the_suite() {
    assert_eq!(suite::visual_to_raw("a:␣b↵\n»c←\n∎\n"), "a: b\n\tc\r\n");
    assert!(suite::suite_dir().join("LICENSE").is_file());
}
