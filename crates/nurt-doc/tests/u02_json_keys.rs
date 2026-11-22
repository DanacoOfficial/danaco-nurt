// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Key paths that JSON edits refuse or resolve in a less common way: the
//! document root, document selectors, member names on lists, positions on
//! objects, missing parents that cannot be created and names repeated more
//! than twice.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;
use nurt_doc::formats::json;
use nurt_doc::model::SetOptions;
use nurt_doc::registry::{DocOptions, OpenDocument, Registry};

const CREATE: SetOptions = SetOptions { create: true };

fn open(bytes: &[u8]) -> OpenDocument {
    Registry::with(json::formats())
        .open(&DocFormatId::JSON, bytes, &DocOptions::default())
        .unwrap()
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn name(text: &str) -> KeySegment {
    KeySegment::Key(text.to_owned())
}

#[test]
fn removing_the_document_root_is_a_key_conflict() {
    let mut document = open(b"{\"a\": 1}");
    let error = document.handle.remove(&KeyPath::root()).unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyConflict);
    assert_eq!(document.bytes(), b"{\"a\": 1}");
}

#[test]
fn a_selector_of_a_later_document_is_not_found() {
    let mut document = open(b"{\"a\": 1}");
    let later = KeyPath::new(Some(DocSelector::Index(1)), vec![name("a")]).unwrap();
    let error = document
        .handle
        .set(&later, &Value::Int(2), &CREATE)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert!(error.message.contains("one document"), "{}", error.message);
    let first = KeyPath::new(Some(DocSelector::Index(0)), vec![name("a")]).unwrap();
    document
        .handle
        .set(&first, &Value::Int(2), &CREATE)
        .unwrap();
    assert_eq!(document.bytes(), b"{\"a\": 2}");
}

#[test]
fn a_digit_name_addresses_a_list_element() {
    let mut document = open(b"{\"l\": [1, 2]}");
    let element = KeyPath::new(None, vec![name("l"), name("1")]).unwrap();
    document
        .handle
        .set(&element, &Value::Int(5), &CREATE)
        .unwrap();
    assert_eq!(
        document.handle.to_value().unwrap().get("l"),
        Some(&Value::List(vec![Value::Int(1), Value::Int(5)]))
    );
    let signed = KeyPath::new(None, vec![name("l"), name("+0")]).unwrap();
    let error = document
        .handle
        .set(&signed, &Value::Int(6), &CREATE)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyConflict);
}

#[test]
fn positions_on_an_object_are_key_conflicts() {
    let mut document = open(b"{\"a\": {\"b\": 1}}");
    for path in ["a[0]", "a[+]"] {
        let error = document
            .handle
            .set(&key(path), &Value::Int(2), &CREATE)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict, "{path}");
        assert!(error.message.contains("member name"), "{}", error.message);
    }
    assert_eq!(document.bytes(), b"{\"a\": {\"b\": 1}}");
}

#[test]
fn missing_parents_are_created_only_with_names_and_first_positions() {
    let mut document = open(b"{}");
    let error = document
        .handle
        .set(&key("x[1].y"), &Value::Int(1), &CREATE)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyConflict);
    assert!(error.message.contains("[0] and [+]"), "{}", error.message);
    assert_eq!(document.bytes(), b"{}");
    document
        .handle
        .set(&key("x[0].y"), &Value::Int(1), &CREATE)
        .unwrap();
    assert_eq!(
        document.handle.to_value().unwrap().get("x"),
        Some(&Value::List(vec![Value::Map(
            [("y".to_owned(), Value::Int(1))].into_iter().collect()
        )]))
    );
}

#[test]
fn removing_a_name_repeated_three_times_removes_every_occurrence() {
    let mut document = open(b"{\"a\": 1, \"a\": 2, \"b\": 3, \"a\": 4}");
    let report = document.handle.remove(&key("a")).unwrap();
    assert!(
        report
            .notable
            .iter()
            .any(|entry| entry.code == Notable::DuplicateKey)
    );
    let text = String::from_utf8(document.bytes()).unwrap();
    assert!(!text.contains("\"a\""), "{text}");
    assert_eq!(
        document.handle.to_value().unwrap(),
        Value::Map([("b".to_owned(), Value::Int(3))].into_iter().collect())
    );
}
