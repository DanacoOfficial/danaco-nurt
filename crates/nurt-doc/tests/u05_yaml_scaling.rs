// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u05_yaml_scaling.rs
// Responsibility:   Cost of edits that touch every entry of a large YAML mapping or sequence.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Cost of edits that touch every entry of a large YAML mapping or sequence.
//!
//! Each test runs one edit on a document and on one four times as large and
//! compares the times: an edit that takes time proportional to the size of the
//! document takes about four times as long on the larger one, an edit that
//! searches the whole mapping for each entry about sixteen times as long. The
//! ratio is checked, not a duration, so the result does not depend on the speed
//! of the machine; the fastest of several runs of each size is taken.

// Test code fails through `unwrap` and `panic`; the failure is the result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;
use nurt_doc::formats::yaml;
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

fn mapping(count: usize) -> String {
    let mut text = String::from("m:\n");
    for index in 0..count {
        text.push_str(&format!("  k{index}: {index}\n"));
    }
    text
}

fn sequence(count: usize) -> String {
    let mut text = String::from("s:\n");
    for index in 0..count {
        text.push_str(&format!("  - {index}\n"));
    }
    text
}

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
        "{small:?} for {size} entries, {large:?} for {} entries: ratio {ratio:.1}",
        4 * size
    );
}

fn open(text: &str) -> nurt_doc::registry::OpenDocument {
    Registry::with(yaml::formats())
        .open(&DocFormatId::YAML, text.as_bytes(), &DocOptions::default())
        .unwrap()
}

#[test]
fn setting_every_entry_of_a_large_mapping_is_linear() {
    assert_linear(2_500, &mapping, &|text| {
        let mut document = open(text);
        document
            .handle
            .set(&key("m.*"), &Value::Text("x".to_owned()), &NO_CREATE)
            .unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}

#[test]
fn setting_every_item_of_a_large_sequence_is_linear() {
    assert_linear(2_500, &sequence, &|text| {
        let mut document = open(text);
        document
            .handle
            .set(&key("s[*]"), &Value::Text("x".to_owned()), &NO_CREATE)
            .unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}

#[test]
fn removing_every_entry_of_a_large_mapping_is_linear() {
    assert_linear(1_000, &mapping, &|text| {
        let mut document = open(text);
        document.handle.remove(&key("m.*")).unwrap();
        String::from_utf8(document.bytes()).unwrap()
    });
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
