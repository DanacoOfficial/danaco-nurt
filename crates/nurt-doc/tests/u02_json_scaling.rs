// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cost of edits that touch every member of a large JSON or JSONC document.
//!
//! Each test runs one edit on a document and on one four times as large and
//! compares the times: an edit that takes time proportional to the size of
//! the document takes about four times as long on the larger one, an edit
//! that reads the whole document for each member about sixteen times as long.
//! The ratio is checked, not a duration, so the result does not depend on the
//! speed of the machine; the fastest of several runs of each size is taken, so
//! that a busy machine does not decide the result.

// Test code fails through `unwrap` and `panic`; the failure is the result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;
use nurt_doc::formats::json;
use nurt_doc::model::SetOptions;
use nurt_doc::registry::{DocOptions, Registry};

const NO_CREATE: SetOptions = SetOptions { create: false };

/// Runs of each size; the fastest counts.
const RUNS: usize = 3;

/// Largest ratio of the times for four times the size that counts as linear;
/// quadratic edits give about 16.
const LINEAR_RATIO: f64 = 9.0;

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn open(id: &DocFormatId, text: &str) -> nurt_doc::registry::OpenDocument {
    Registry::with(json::formats())
        .open(id, text.as_bytes(), &DocOptions::default())
        .unwrap()
}

/// The fastest of [`RUNS`] runs of `edit` on a document made by `build(size)`;
/// building and opening the document are not timed.
fn fastest(
    size: usize,
    build: &dyn Fn(usize) -> String,
    edit: &dyn Fn(&str) -> String,
) -> Duration {
    let text = build(size);
    (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            let edited = edit(&text);
            let elapsed = start.elapsed();
            assert_ne!(edited, text, "the edit changed nothing");
            elapsed
        })
        .min()
        .unwrap()
}

fn assert_linear(size: usize, build: &dyn Fn(usize) -> String, edit: &dyn Fn(&str) -> String) {
    let small = fastest(size, build, edit);
    let large = fastest(4 * size, build, edit);
    let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-4);
    assert!(
        ratio < LINEAR_RATIO,
        "{small:?} for {size} members, {large:?} for {} members: ratio {ratio:.1}",
        4 * size
    );
}

fn numbers(count: usize) -> String {
    let items: Vec<String> = (0..count).map(|index| index.to_string()).collect();
    format!("[{}]", items.join(", "))
}

fn commented_object(count: usize) -> String {
    let mut text = String::from("{\n");
    for index in 0..count {
        let comma = if index + 1 < count { "," } else { "" };
        text.push_str(&format!(
            "  // member {index}\n  \"k{index}\": {index}{comma}\n"
        ));
    }
    text.push_str("}\n");
    text
}

fn repeated_object(count: usize) -> String {
    let members: Vec<String> = (0..count).map(|index| format!("\"a\": {index}")).collect();
    format!("{{{}}}", members.join(", "))
}

#[test]
fn setting_every_element_of_a_large_array_is_linear() {
    assert_linear(10_000, &numbers, &|text| {
        let mut document = open(&DocFormatId::JSON, text);
        document
            .handle
            .set(&key("[*]"), &Value::Text("x".to_owned()), &NO_CREATE)
            .unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}

#[test]
fn setting_every_element_of_a_large_array_to_a_container_is_linear() {
    assert_linear(4_000, &numbers, &|text| {
        let mut document = open(&DocFormatId::JSON, text);
        let value = Value::List(vec![Value::Int(1)]);
        document
            .handle
            .set(&key("[*]"), &value, &NO_CREATE)
            .unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}

#[test]
fn removing_every_member_of_an_object_with_comments_is_linear() {
    assert_linear(1_000, &commented_object, &|text| {
        let mut document = open(&DocFormatId::JSONC, text);
        document.handle.remove(&key("*")).unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}

#[test]
fn removing_a_name_repeated_many_times_is_linear() {
    assert_linear(2_000, &repeated_object, &|text| {
        let mut document = open(&DocFormatId::JSON, text);
        document.handle.remove(&key("a")).unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}
