// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u07_xml_handle.rs
// Responsibility:   The format `xml` through the document model.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The format `xml` through the document model: registration, detection,
//! changes in other encodings and the round-trip harness of the shared
//! guards on `tests/d5/corpus/xml/core`.
#![cfg(feature = "testing")]
// Failing tests report the panic itself, so `unwrap` is the intended way to
// stop on a broken fixture.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::Value;
use nurt_doc::edit::guard::Expectation;
use nurt_doc::formats::xml::{XmlFormat, formats};
use nurt_doc::model::{DocHandle, SetOptions};
use nurt_doc::registry::{DocFormat, DocOptions, Registry};
use nurt_doc::testing::{Guard, HarnessOperation, HarnessOptions, Summary, Violation, run_corpus};

/// Name of the member that the harness inserts.
const NEW_KEY: &str = "nurt_new_key";

/// Construct names of the adapter's own guards, which must not refuse a
/// change of the harness.
const GUARD_CONSTRUCTS: [&str; 2] = ["doc.out_of_scope_change", "doc.semantic_mismatch"];

/// Largest number of violations shown when a test fails.
const SHOWN: usize = 10;

fn corpus() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/corpus/xml/core")
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn run(operations: &[HarnessOperation]) -> Summary {
    let options = HarnessOptions {
        operations: operations.to_vec(),
        ..HarnessOptions::default()
    };
    run_corpus(&corpus(), &XmlFormat, &options).unwrap()
}

fn corpus_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            corpus_files(&path, files);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| XmlFormat.extensions().contains(&ext))
        {
            files.push(path);
        }
    }
}

/// Renames members `prefix:nurt_new_key`, which a child of a prefixed
/// element receives, to `nurt_new_key`.
fn unprefixed(value: Value) -> Value {
    match value {
        Value::Map(fields) => Value::Map(
            fields
                .into_iter()
                .map(|(name, member)| {
                    let name = match name.split_once(':') {
                        Some((_, local)) if local == NEW_KEY => NEW_KEY.to_owned(),
                        _ => name,
                    };
                    (name, unprefixed(member))
                })
                .collect(),
        ),
        Value::List(items) => Value::List(items.into_iter().map(unprefixed).collect()),
        other => other,
    }
}

fn documents(handle: &dyn DocHandle) -> Vec<Value> {
    let tree = handle.tree();
    tree.roots()
        .iter()
        .map(|root| tree.value_of(*root, handle.semantics(), None))
        .collect()
}

/// Returns `true` when a meaning violation of the harness disappears once
/// both values are compared in the form XML reads them back.
fn explained(violation: &Violation) -> bool {
    if violation.guard != Guard::Meaning {
        return false;
    }
    let options = DocOptions::default();
    let bytes = std::fs::read(&violation.path).unwrap();
    let original = XmlFormat.open(bytes.clone(), &options).unwrap();
    let node_key = key(&violation.key);
    let extended = |segment: KeySegment| {
        let mut extended = node_key.clone();
        extended.push(segment).unwrap();
        extended
    };
    let (change_key, value) = match violation.operation {
        Some(HarnessOperation::Remove) => (node_key.clone(), None),
        Some(HarnessOperation::InsertKey) => (
            extended(KeySegment::Key(NEW_KEY.to_owned())),
            Some(Value::Int(1)),
        ),
        Some(HarnessOperation::Append) => (extended(KeySegment::Append), Some(Value::Int(1))),
        _ => return false,
    };
    let expectation = Expectation::new(documents(original.as_ref()), original.semantics());
    let expectation = match &value {
        None => expectation.remove(change_key.clone()),
        Some(value) => expectation.set(change_key.clone(), value.clone()),
    };
    let Ok(expected) = expectation.expected() else {
        return false;
    };
    let mut changed = XmlFormat.open(bytes, &options).unwrap();
    let result = match &value {
        None => changed.remove(&change_key),
        Some(value) => changed.set(&change_key, value, &SetOptions { create: true }),
    };
    if result.is_err() {
        return false;
    }
    let reopened = XmlFormat.open(changed.text().to_vec(), &options).unwrap();
    let semantics = original.semantics();
    let expected: Vec<Value> = expected
        .into_iter()
        .map(|document| semantics.canonical(document))
        .collect();
    let actual: Vec<Value> = documents(reopened.as_ref())
        .into_iter()
        .map(|document| unprefixed(semantics.canonical(document)))
        .collect();
    expected == actual
}

#[test]
fn registry_detects_and_opens_xml_files() {
    let registry = Registry::from_functions(&[formats]);
    let id = registry
        .detect(Some(Path::new("app/pom.xml")), b"<project/>")
        .unwrap();
    assert_eq!(id.as_str(), "xml");
    let sniffed = registry
        .detect(None, b"<?xml version=\"1.0\"?><a/>")
        .unwrap();
    assert_eq!(sniffed.as_str(), "xml");
    let builtin = Registry::builtin();
    let svg = builtin
        .detect(Some(Path::new("icon.svg")), b"<svg/>")
        .unwrap();
    assert_eq!(svg.as_str(), "xml");
    let bytes = b"\xEF\xBB\xBF<a>\r\n  <b>1</b>\r\n</a>\r\n";
    let document = registry.open(&id, bytes, &DocOptions::default()).unwrap();
    assert_eq!(document.bytes(), bytes);
}

#[test]
fn edits_keep_the_byte_order_mark_and_line_endings() {
    let bytes = b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\r\n<a>\r\n  <b>1</b>\r\n</a>\r\n".to_vec();
    let mut handle = XmlFormat.open(bytes, &DocOptions::default()).unwrap();
    handle
        .set(
            &key("a.c"),
            &Value::Text("x".into()),
            &SetOptions { create: true },
        )
        .unwrap();
    assert_eq!(
        handle.text(),
        b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\r\n<a>\r\n  <b>1</b>\r\n  <c>x</c>\r\n</a>\r\n"
    );
    let node = handle.get(&key("a.b")).unwrap()[0].node;
    let span = handle.tree()[node].span;
    assert_eq!(&handle.text()[span.start..span.end], b"<b>1</b>");
}

#[test]
fn check_syntax_reports_the_line_of_an_error() {
    let issues = XmlFormat.check_syntax(b"<a>\n  <b>\n</a>", &DocOptions::default());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].span.start.line, 3);
    assert!(
        XmlFormat
            .check_syntax(b"<a/>", &DocOptions::default())
            .is_empty()
    );
}

#[test]
fn model_tree_reads_as_the_document_value_for_the_corpus() {
    let mut files = Vec::new();
    corpus_files(&corpus(), &mut files);
    assert!(files.len() >= 300, "{} files", files.len());
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let handle = XmlFormat.open(bytes, &DocOptions::default()).unwrap();
        let model = documents(handle.as_ref());
        let semantics = handle.semantics();
        assert_eq!(
            model
                .into_iter()
                .map(|value| semantics.canonical(value))
                .collect::<Vec<_>>(),
            vec![semantics.canonical(handle.to_value().unwrap())],
            "{}",
            path.display()
        );
    }
}

#[test]
fn harness_set_operations_keep_identity_scope_and_values() {
    let summary = run(&[HarnessOperation::SetSame, HarnessOperation::ReplaceScalar]);
    assert!(summary.files >= 300, "{} files", summary.files);
    assert!(summary.not_opened.is_empty(), "{:?}", summary.not_opened);
    let shown = &summary.violations[..summary.violations.len().min(SHOWN)];
    assert!(summary.violations.is_empty(), "{shown:#?}");
    assert!(summary.accepted > 0, "{summary:?}");
    for construct in GUARD_CONSTRUCTS {
        assert!(
            !summary.unsupported.contains_key(construct),
            "{construct}: {:?}",
            summary.unsupported
        );
    }
}

#[test]
fn harness_structural_changes_differ_only_by_xml_value_rules() {
    let summary = run(&[
        HarnessOperation::Remove,
        HarnessOperation::InsertKey,
        HarnessOperation::Append,
    ]);
    assert!(summary.not_opened.is_empty(), "{:?}", summary.not_opened);
    let unexplained: Vec<&Violation> = summary
        .violations
        .iter()
        .filter(|violation| !explained(violation))
        .collect();
    let shown = &unexplained[..unexplained.len().min(SHOWN)];
    assert!(unexplained.is_empty(), "{shown:#?}");
    assert!(summary.accepted > 0, "{summary:?}");
    assert!(
        !summary.unsupported.contains_key(GUARD_CONSTRUCTS[0]),
        "{:?}",
        summary.unsupported
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
