// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Round-trip harness runs over the JSON and JSONC corpus and the JSON Test
//! Suite, and a change of a document written and undone through the
//! in-memory journal.

#![cfg(feature = "testing")]
// Test code fails through `unwrap` and `panic`; the failure is the result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, OpKind, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;
use nurt_doc::formats::json::{Dialect, JsonFormat};
use nurt_doc::model::{DocTree, NodeId, SetOptions, Span};
use nurt_doc::registry::{DocFormat, DocOptions, open_with};
use nurt_doc::testing::{
    FileFilter, Guard, HarnessOperation, HarnessOptions, MemoryJournal, Summary, Violation,
    run_corpus,
};

/// Smallest number of authored JSON files expected in the corpus.
const MIN_OWN_JSON_FILES: usize = 140;

/// Smallest number of authored JSONC files expected in the corpus.
const MIN_OWN_JSONC_FILES: usize = 35;

/// Number of parsing cases of the JSON Test Suite.
const SUITE_CASES: usize = 318;

/// Bytes that may change next to the lines of a removed node.
const SURROUNDINGS: &[u8] = b" \t\r\n,";

fn data(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/d5")
        .join(path)
}

fn run(dir: &str, dialect: Dialect, filter: FileFilter) -> Summary {
    let options = HarnessOptions {
        filter,
        ..HarnessOptions::default()
    };
    run_corpus(&data(dir), &JsonFormat::new(dialect), &options).unwrap()
}

/// A member name on the path of `node` is shared by a sibling.
fn repeated_on_path(tree: &DocTree, node: NodeId) -> bool {
    let mut current = node;
    while let Some(parent) = tree[current].parent {
        let name = tree[current].name.as_deref();
        if name.is_some()
            && tree
                .members(parent)
                .filter(|member| tree[*member].name.as_deref() == name)
                .count()
                > 1
        {
            return true;
        }
        current = parent;
    }
    false
}

/// The harness selects every node, members shadowed by a later member of
/// the same name included; their key path reaches the last member, so a
/// change of a shadowed member changes another node. Violations on such
/// keys do not come from the format.
fn reaches_repeated_member(violation: &Violation, format: &dyn DocFormat) -> bool {
    let Ok(key) = KeyPath::parse(&violation.key) else {
        return false;
    };
    let bytes = std::fs::read(&violation.path).unwrap();
    let Ok(document) = open_with(format, &bytes, &DocOptions::default()) else {
        return false;
    };
    let tree = document.handle.tree();
    tree.resolve(&key, document.handle.semantics())
        .is_ok_and(|found| {
            found
                .iter()
                .any(|resolved| repeated_on_path(tree, resolved.node))
        })
}

/// Lines of `span` with the blanks, line breaks and commas around them.
fn reach(text: &[u8], span: Span) -> Span {
    let mut start = text[..span.start]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let mut end = text[span.end..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(text.len(), |at| span.end + at + 1);
    while start > 0 && SURROUNDINGS.contains(&text[start - 1]) {
        start -= 1;
    }
    while end < text.len() && SURROUNDINGS.contains(&text[end]) {
        end += 1;
    }
    Span::new(start, end)
}

/// The bytes before and after `region` are kept: the changed text starts
/// with the bytes before the region and ends with the bytes after it.
fn keeps_outside(original: &[u8], changed: &[u8], region: Span) -> bool {
    let (head, tail) = (&original[..region.start], &original[region.end..]);
    head.len() + tail.len() <= changed.len() && changed.starts_with(head) && changed.ends_with(tail)
}

/// Removing the only member or element of a container collapses its lines
/// (`{` and `}` on separate lines become `{}`), which the harness reports
/// as bytes outside the lines of the node. The removal is repeated and
/// its scope checked over the blanks, line breaks and commas around the node.
fn removal_in_scope(violation: &Violation, format: &dyn DocFormat) -> bool {
    if violation.guard != Guard::Scope || violation.operation != Some(HarnessOperation::Remove) {
        return false;
    }
    let Ok(key) = KeyPath::parse(&violation.key) else {
        return false;
    };
    let bytes = std::fs::read(&violation.path).unwrap();
    let Ok(mut document) = open_with(format, &bytes, &DocOptions::default()) else {
        return false;
    };
    let original = document.handle.text().to_vec();
    let Some(node) = document
        .handle
        .get(&key)
        .ok()
        .and_then(|found| found.first().map(|resolved| resolved.node))
    else {
        return false;
    };
    let region = reach(&original, document.handle.tree()[node].span);
    if document.handle.remove(&key).is_err() {
        return false;
    }
    let changed = document.handle.text();
    keeps_outside(&original, changed, region)
}

fn assert_clean(summary: &Summary, format: &dyn DocFormat) {
    let violations: Vec<&Violation> = summary
        .violations
        .iter()
        .filter(|violation| {
            violation.operation.is_none()
                || !(reaches_repeated_member(violation, format)
                    || removal_in_scope(violation, format))
        })
        .collect();
    assert!(
        violations.is_empty(),
        "{} violations: {:#?}",
        violations.len(),
        violations.iter().take(10).collect::<Vec<_>>()
    );
    assert!(summary.unsupported.is_empty(), "{:?}", summary.unsupported);
    assert!(summary.refused.is_empty(), "{:?}", summary.refused);
    assert!(summary.accepted > 0);
}

#[test]
fn harness_keeps_every_authored_json_file_and_change() {
    let summary = run("corpus/json", Dialect::Json, FileFilter::All);
    assert!(
        summary.files >= MIN_OWN_JSON_FILES,
        "{} files",
        summary.files
    );
    assert!(summary.not_opened.is_empty(), "{:?}", summary.not_opened);
    assert_clean(&summary, &JsonFormat::new(Dialect::Json));
}

#[test]
fn harness_keeps_every_authored_jsonc_file_and_change() {
    let summary = run("corpus/jsonc", Dialect::Jsonc, FileFilter::All);
    assert!(
        summary.files >= MIN_OWN_JSONC_FILES,
        "{} files",
        summary.files
    );
    assert!(summary.not_opened.is_empty(), "{:?}", summary.not_opened);
    assert_clean(&summary, &JsonFormat::new(Dialect::Jsonc));
}

#[test]
fn harness_opens_every_valid_suite_case_and_rejects_every_invalid_one() {
    let summary = run(
        "third_party/json/jsontestsuite",
        Dialect::Json,
        FileFilter::ByFormat,
    );
    assert_eq!(summary.files, SUITE_CASES);
    let name = |path: &Path| path.file_name().unwrap().to_string_lossy().into_owned();
    let invalid = std::fs::read_dir(data("third_party/json/jsontestsuite"))
        .unwrap()
        .filter(|entry| name(&entry.as_ref().unwrap().path()).starts_with("n_"))
        .count();
    let mut rejected = 0;
    for (path, message) in &summary.not_opened {
        let name = name(path);
        assert!(!name.starts_with("y_"), "{name}: {message}");
        rejected += usize::from(name.starts_with("n_"));
    }
    assert_eq!(rejected, invalid);
    assert_clean(&summary, &JsonFormat::new(Dialect::Json));
}

#[test]
fn a_change_written_through_the_journal_is_undone_byte_for_byte() {
    let path = AbsPath::new(std::env::temp_dir().join("nurt-u02").join("tsconfig.json")).unwrap();
    let original =
        b"{\n  // Output.\n  \"compilerOptions\": {\"target\": \"ES2020\"},\n}\n".to_vec();
    let journal = MemoryJournal::new().with_file(path.as_path(), original.clone());
    let format = JsonFormat::new(Dialect::Jsonc);
    let content = journal.file(path.as_path()).unwrap();
    let mut document = open_with(&format, &content, &DocOptions::default()).unwrap();
    let key = KeyPath::parse("compilerOptions.target").unwrap();
    document
        .handle
        .set(
            &key,
            &Value::Text("ES2022".to_owned()),
            &SetOptions { create: false },
        )
        .unwrap();
    let mut tx = journal.begin(OpKind::command("set"));
    tx.preserve(&[TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action: TargetAction::Modify,
        expect: Expect::Unchecked,
        size_hint: None,
    }])
    .unwrap();
    tx.apply(TargetIndex(0), Mutation::WriteBytes(document.bytes()))
        .unwrap();
    tx.commit().unwrap();
    let changed =
        b"{\n  // Output.\n  \"compilerOptions\": {\"target\": \"ES2022\"},\n}\n".to_vec();
    assert_eq!(journal.file(path.as_path()), Some(changed));
    journal.undo_last().unwrap();
    assert_eq!(journal.file(path.as_path()), Some(original));
}
