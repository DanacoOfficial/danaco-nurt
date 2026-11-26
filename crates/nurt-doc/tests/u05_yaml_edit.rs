// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes of YAML scalars and mappings through the document handle: style
//! of replaced scalars, placement of new keys, removal of keys, changes of
//! the kind of a value, and the preservation of byte order marks, line
//! breaks, tabs and directives. Profile files of common tools are read and
//! changed key by key.

#![allow(
    clippy::unwrap_used,
    reason = "helpers outside test functions fail the test on unexpected errors"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::Value;
use nurt_doc::formats::yaml::{YamlDoc, YamlFormat, YamlVersion};
use nurt_doc::model::{DocHandle, SetOptions};
use nurt_doc::registry::{DocFormat, DocOptions};
use nurt_doc::value::equivalent;

const CREATE: SetOptions = SetOptions { create: true };

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn set(source: &str, path: &str, value: &Value) -> String {
    let mut doc = YamlDoc::parse(source, YamlVersion::V1_2).unwrap();
    doc.set(&key(path), value, &CREATE).unwrap();
    String::from_utf8(doc.text().to_vec()).unwrap()
}

fn remove(source: &str, path: &str) -> String {
    let mut doc = YamlDoc::parse(source, YamlVersion::V1_2).unwrap();
    doc.remove(&key(path)).unwrap();
    String::from_utf8(doc.text().to_vec()).unwrap()
}

/// Source of `k` in each scalar style, holding the text `old`.
const STYLES: &[(&str, &str)] = &[
    ("plain", "k: old # c\n"),
    ("single", "k: 'old' # c\n"),
    ("double", "k: \"old\" # c\n"),
    ("literal", "k: |\n  old\nz: 0\n"),
    ("folded", "k: >\n  old\nz: 0\n"),
];

fn values() -> Vec<Value> {
    vec![
        text("new"),
        text("yes"),
        text("it's"),
        text("a\nb\n"),
        text("a\nb"),
        text(""),
        text("x: y"),
        Value::Int(5),
        Value::Number(1.5),
        Value::Bool(false),
        Value::Null,
        Value::Decimal("1.10".to_owned()),
    ]
}

#[test]
fn every_style_reads_back_every_new_value() {
    for (style, source) in STYLES {
        for value in values() {
            let out = set(source, "k", &value);
            let doc = YamlDoc::parse(out.as_str(), YamlVersion::V1_1).unwrap();
            let read = doc.to_value().unwrap();
            let Value::Map(fields) = read else {
                panic!("{style}: not a map");
            };
            assert!(equivalent(&fields["k"], &value), "{style} {value:?}: {out}");
            let other = YamlDoc::parse(out.as_str(), YamlVersion::V1_2)
                .unwrap()
                .to_value()
                .unwrap();
            let Value::Map(other) = other else {
                panic!("{style}: not a map");
            };
            assert!(
                equivalent(&other["k"], &value),
                "{style} {value:?} under 1.2: {out}"
            );
        }
    }
}

#[test]
fn keeps_the_style_of_replaced_scalars() {
    let cases = [
        ("k: old # c\n", text("new"), "k: new # c\n"),
        ("k: old # c\n", text("yes"), "k: \"yes\" # c\n"),
        ("k: old # c\n", text("a\nb\n"), "k: | # c\n  a\n  b\n"),
        ("k: 'old' # c\n", text("it's"), "k: 'it''s' # c\n"),
        ("k: 'old' # c\n", Value::Int(5), "k: 5 # c\n"),
        ("k: \"old\" # c\n", text("new"), "k: \"new\" # c\n"),
        ("k: |\n  old\nz: 0\n", text("new"), "k: |-\n  new\nz: 0\n"),
        ("k: |\n  old\nz: 0\n", text("new\n"), "k: |\n  new\nz: 0\n"),
        ("k: >\n  old\nz: 0\n", text("new\n"), "k: |\n  new\nz: 0\n"),
        ("k: >\n  old\nz: 0\n", text("new"), "k: >-\n  new\nz: 0\n"),
        (
            "k: |\n    old\nz: 0\n",
            Value::Bool(true),
            "k: true\nz: 0\n",
        ),
        ("k: old\n", Value::Number(1e300), "k: 1.0e+300\n"),
    ];
    for (source, value, expected) in cases {
        let out = set(source, "k", &value);
        assert_eq!(out, expected, "{source:?} {value:?}");
    }
}

#[test]
fn quotes_an_element_of_a_flow_list_that_contains_the_separator() {
    assert_eq!(
        set("k: [a, old]\n", "k[1]", &text("x, y")),
        "k: [a, \"x, y\"]\n"
    );
}

#[test]
fn inserts_keys_with_the_indentation_of_their_siblings() {
    assert_eq!(
        set("a:\n    b: 1\n", "a.c", &Value::Int(2)),
        "a:\n    b: 1\n    c: 2\n"
    );
    assert_eq!(set("a: {}\n", "a.b", &Value::Int(1)), "a: {b: 1}\n");
    assert_eq!(
        set("a: {x: 1}\n", "a.b", &Value::Int(1)),
        "a: {x: 1, b: 1}\n"
    );
    assert_eq!(set("a: 1\n", "[\"b.c\"]", &Value::Int(2)), "a: 1\nb.c: 2\n");
    assert_eq!(
        set("a: 1\n", "[\"x: y\"]", &Value::Int(2)),
        "a: 1\n\"x: y\": 2\n"
    );
    assert_eq!(set("a: 1\n", "on", &Value::Int(2)), "a: 1\n\"on\": 2\n");
    assert_eq!(
        set("l:\n- a\n", "m.names", &Value::List(vec![text("x")])),
        "l:\n- a\nm:\n  names:\n  - x\n"
    );
}

#[test]
fn removes_keys_with_multi_line_values_and_comments() {
    let source = "a: 1\nb: |\n  one\n  two\nc:\n  - x # item\nd: 4 # four\n";
    assert_eq!(remove(source, "b"), "a: 1\nc:\n  - x # item\nd: 4 # four\n");
    assert_eq!(
        remove(source, "c"),
        "a: 1\nb: |\n  one\n  two\nd: 4 # four\n"
    );
    assert_eq!(
        remove(source, "d"),
        "a: 1\nb: |\n  one\n  two\nc:\n  - x # item\n"
    );
    assert_eq!(remove("m: {a: 1, b: 2}\n", "m.a"), "m: {b: 2}\n");
    assert_eq!(remove("m:\n  a: 1\n", "m.a"), "m: {}\n");
    assert_eq!(remove("\"q.k\": 1\nz: 2\n", "[\"q.k\"]"), "z: 2\n");
}

#[test]
fn changes_values_between_scalars_maps_and_lists() {
    let map = Value::Map([("x".to_owned(), Value::Int(1))].into_iter().collect());
    assert_eq!(set("a: 1\nb: 2\n", "a", &map), "a:\n  x: 1\nb: 2\n");
    assert_eq!(set("a:\n  x: 1\nb: 2\n", "a", &text("s")), "a: s\nb: 2\n");
    assert_eq!(set("a: [1]\n", "a", &map), "a: {x: 1}\n");
    assert_eq!(
        set(
            "a: 1\n",
            "a",
            &Value::List(vec![Value::Int(1), Value::Int(2)])
        ),
        "a:\n  - 1\n  - 2\n"
    );
    assert_eq!(set("a: !!str 1\n", "a", &Value::Int(2)), "a: 2\n");
}

#[test]
fn keeps_byte_order_marks_line_breaks_tabs_and_directives() {
    assert_eq!(
        set("\u{FEFF}a: 1\r\nb: 2\r\n", "c", &Value::Int(3)),
        "\u{FEFF}a: 1\r\nb: 2\r\nc: 3\r\n"
    );
    assert_eq!(
        set("a: \"x\ty\"\nb: 1\n", "b", &Value::Int(2)),
        "a: \"x\ty\"\nb: 2\n"
    );
    let source = "%YAML 1.1\n---\na: yes\nb: 1\n";
    let doc = YamlDoc::parse(source, YamlVersion::V1_2).unwrap();
    let Value::Map(fields) = doc.to_value().unwrap() else {
        panic!("not a map");
    };
    assert_eq!(fields["a"], Value::Bool(true));
    assert_eq!(
        set(source, "b", &text("no")),
        "%YAML 1.1\n---\na: yes\nb: \"no\"\n"
    );
}

#[test]
fn version_setting_switches_the_typing_of_plain_scalars() {
    let options = DocOptions {
        yaml_version: YamlVersion::V1_1,
        ..DocOptions::default()
    };
    let handle = YamlFormat
        .open(b"mode: 0755\nflag: off\n".to_vec(), &options)
        .unwrap();
    let Value::Map(fields) = handle.to_value().unwrap() else {
        panic!("not a map");
    };
    assert_eq!(fields["mode"], Value::Int(493));
    assert_eq!(fields["flag"], Value::Bool(false));
}

#[test]
fn rejects_templates_with_a_fix_to_edit_the_text() {
    let opened = YamlFormat.open(
        b"replicas: {{ .Values.replicas }}\n".to_vec(),
        &DocOptions::default(),
    );
    let Err(error) = opened else {
        panic!("a template opened as YAML");
    };
    assert_eq!(error.code, ErrorCode::DocParse);
    assert_eq!(error.fix.first().map(String::as_str), Some("replace"));
}

#[test]
fn setting_the_current_value_changes_no_byte() {
    let mut doc = YamlDoc::parse("a: 1.10\nb: 'x'\n", YamlVersion::V1_2).unwrap();
    let report = doc
        .set(&key("a"), &Value::Decimal("1.10".to_owned()), &CREATE)
        .unwrap();
    assert!(report.is_unchanged());
    let report = doc.set(&key("b"), &text("x"), &CREATE).unwrap();
    assert!(report.is_unchanged());
}

fn profiles() -> Vec<PathBuf> {
    let dir = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/corpus/yaml/profiles");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

/// Keys of the scalar values of the first document, through mappings only.
fn scalar_keys(doc: &YamlDoc) -> Vec<KeyPath> {
    let mut keys = Vec::new();
    let mut stack = vec![(Vec::new(), doc.to_value().unwrap())];
    while let Some((path, value)) = stack.pop() {
        match value {
            Value::Map(fields) => {
                for (name, item) in fields {
                    let mut child: Vec<KeySegment> = path.clone();
                    child.push(KeySegment::Key(name));
                    stack.push((child, item));
                }
            }
            Value::List(_) => {}
            _ => keys.push(KeyPath::new(None, path).unwrap()),
        }
    }
    keys
}

#[test]
fn changes_every_scalar_of_the_profiles_without_touching_other_bytes() {
    for path in profiles() {
        let source = std::fs::read_to_string(&path).unwrap();
        let doc = YamlDoc::parse(source.as_str(), YamlVersion::V1_2).unwrap();
        assert_eq!(doc.text(), source.as_bytes());
        for scalar in scalar_keys(&doc) {
            let mut changed = doc.clone();
            changed
                .set(&scalar, &text("changed"), &SetOptions::default())
                .unwrap();
            let new = String::from_utf8(changed.text().to_vec()).unwrap();
            let prefix = source
                .bytes()
                .zip(new.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = source
                .bytes()
                .rev()
                .zip(new.bytes().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let removed = &source[prefix..source.len().saturating_sub(suffix).max(prefix)];
            assert!(
                !removed.contains('\n'),
                "{}: {scalar} changed other lines",
                path.display()
            );
        }
    }
}

#[test]
fn matches_yq_json_output_when_yq_is_installed() {
    let available = Command::new("yq")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        return;
    }
    for path in profiles() {
        let output = Command::new("yq")
            .args(["-o", "json", "."])
            .arg(&path)
            .output()
            .unwrap();
        let documents: Vec<Value> = serde_json::Deserializer::from_slice(&output.stdout)
            .into_iter::<serde_json::Value>()
            .map(|document| Value::from_plain_json(document.unwrap()))
            .collect();
        let source = std::fs::read_to_string(&path).unwrap();
        let doc = YamlDoc::parse(source, YamlVersion::V1_2).unwrap();
        let values = doc.values().unwrap();
        assert_eq!(values.len(), documents.len(), "{}", path.display());
        for (actual, expected) in values.iter().zip(&documents) {
            assert!(equivalent(actual, expected), "{}", path.display());
        }
    }
}
