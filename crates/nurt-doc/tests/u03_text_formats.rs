// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes to property and environment documents through the document
//! handle: replaced, created and removed values, other lines kept in order,
//! comments kept and refused changes.
// Helpers outside `#[test]` functions fail the test on unexpected errors.
#![allow(clippy::unwrap_used)]

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;
use nurt_doc::formats::env::EnvFormat;
use nurt_doc::formats::properties::PropertiesFormat;
use nurt_doc::model::{DocHandle, SetOptions};
use nurt_doc::registry::{DocFormat, DocOptions};

const CREATE: SetOptions = SetOptions { create: true };
const NO_CREATE: SetOptions = SetOptions { create: false };

const PROPERTIES: &str = "# server\nhost=a\nport = 80\n! legacy comment\nname:web\n";

const ENVIRONMENT: &str = "# service\nA=1\nB=2\n\n# end\n";

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn open(format: &dyn DocFormat, source: &str) -> Box<dyn DocHandle> {
    format
        .open(source.as_bytes().to_vec(), &DocOptions::default())
        .unwrap()
}

fn written(handle: &dyn DocHandle) -> String {
    String::from_utf8(handle.text().to_vec()).unwrap()
}

fn member(handle: &dyn DocHandle, name: &str) -> Option<Value> {
    handle.to_value().unwrap().get(name).cloned()
}

/// Whether every line of `before` occurs in `after`, in the same order.
fn keeps_lines(before: &str, after: &str) -> bool {
    let mut later = after.lines();
    before.lines().all(|line| later.any(|other| other == line))
}

#[test]
fn a_property_is_replaced_and_the_other_lines_stay() {
    let mut handle = open(&PropertiesFormat, PROPERTIES);
    handle.set(&key("port"), &text("8080"), &NO_CREATE).unwrap();
    assert_eq!(member(&*handle, "port"), Some(text("8080")));
    assert_eq!(member(&*handle, "host"), Some(text("a")));
    assert_eq!(member(&*handle, "name"), Some(text("web")));
    let after = written(&*handle);
    assert!(after.starts_with("# server\nhost=a\n"), "{after}");
    assert!(after.contains("! legacy comment\n"), "{after}");
    assert!(after.contains("name:web"), "{after}");
}

#[test]
fn setting_the_current_property_changes_no_byte() {
    let mut handle = open(&PropertiesFormat, PROPERTIES);
    let report = handle.set(&key("host"), &text("a"), &NO_CREATE).unwrap();
    assert!(report.is_unchanged());
    assert_eq!(written(&*handle), PROPERTIES);
}

#[test]
fn a_new_property_is_created_only_when_asked() {
    let mut handle = open(&PropertiesFormat, PROPERTIES);
    let error = handle
        .set(&key("timeout"), &text("5"), &NO_CREATE)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert_eq!(written(&*handle), PROPERTIES);
    handle.set(&key("timeout"), &text("5"), &CREATE).unwrap();
    assert_eq!(member(&*handle, "timeout"), Some(text("5")));
    assert!(keeps_lines(PROPERTIES, &written(&*handle)));
}

#[test]
fn a_property_is_removed_with_its_line_only() {
    let mut handle = open(&PropertiesFormat, PROPERTIES);
    handle.remove(&key("host")).unwrap();
    assert_eq!(member(&*handle, "host"), None);
    assert_eq!(member(&*handle, "port"), Some(text("80")));
    let after = written(&*handle);
    assert!(!after.contains("host=a"), "{after}");
    assert!(keeps_lines(
        "port = 80\n! legacy comment\nname:web\n",
        &after
    ));
    let error = handle.remove(&key("host")).unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
}

#[test]
fn a_variable_is_replaced_and_comments_and_blank_lines_stay() {
    let mut handle = open(&EnvFormat, ENVIRONMENT);
    handle
        .set(&key("B"), &text("two words"), &NO_CREATE)
        .unwrap();
    assert_eq!(member(&*handle, "B"), Some(text("two words")));
    assert_eq!(member(&*handle, "A"), Some(text("1")));
    let after = written(&*handle);
    assert!(after.starts_with("# service\nA=1\n"), "{after}");
    assert!(after.ends_with("\n\n# end\n"), "{after}");
}

#[test]
fn a_new_variable_is_created_only_when_asked() {
    let mut handle = open(&EnvFormat, ENVIRONMENT);
    let error = handle.set(&key("C"), &text("3"), &NO_CREATE).unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    handle.set(&key("C"), &text("3"), &CREATE).unwrap();
    assert_eq!(member(&*handle, "C"), Some(text("3")));
    assert!(keeps_lines(ENVIRONMENT, &written(&*handle)));
}

#[test]
fn a_variable_is_removed_and_the_others_keep_their_values() {
    let mut handle = open(&EnvFormat, ENVIRONMENT);
    handle.remove(&key("A")).unwrap();
    assert_eq!(member(&*handle, "A"), None);
    assert_eq!(member(&*handle, "B"), Some(text("2")));
    assert!(keeps_lines("B=2\n\n# end\n", &written(&*handle)));
    let error = handle.remove(&key("A")).unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
}
