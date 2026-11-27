// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u01_harness.rs
// Responsibility:   Round-trip harness, the `key = value` test format and the in-memory.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Round-trip harness, the `key = value` test format and the in-memory
//! journal.
#![cfg(feature = "testing")]
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, OpKind, RollbackReason, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;
use nurt_doc::edit::EditReport;
use nurt_doc::edit::guard::ReferenceParser;
use nurt_doc::model::{DocHandle, DocTree, FormatSemantics, Repeated, SetOptions, Span};
use nurt_doc::registry::{Confidence, DocFormat, DocOptions, Registry, open_with};
use nurt_doc::testing::{
    FileFilter, Guard, HarnessOperation, HarnessOptions, KeyValueFormat, KeyValueReference,
    MemoryJournal, Summary, first_change_outside, run_corpus, run_document,
};
use serde_json::json;

/// Three entries, the first with a comment line above it.
const SAMPLE: &[u8] = b"# port\nport = 8080\nname = web\n";

fn format() -> KeyValueFormat {
    KeyValueFormat::new(DocFormatId::new("kv").expect("valid identifier"))
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).expect("valid key path")
}

fn open(text: &[u8]) -> Box<dyn DocHandle> {
    format()
        .open(text.to_vec(), &DocOptions::default())
        .expect("document opens")
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

#[test]
fn key_value_set_replaces_value_in_place() {
    let mut document = open(b"a = 1\nb = 2\n");
    let report = document
        .set(&key("a"), &Value::Int(5), &SetOptions::default())
        .expect("sets");
    assert_eq!(document.text(), b"a = 5\nb = 2\n");
    assert_eq!(report.replacements, 1);
    assert_eq!(document.get(&key("a")).expect("resolves").len(), 1);
    assert_eq!(
        document.to_value().expect("value"),
        Value::from_plain_json(json!({"a": 5, "b": 2}))
    );
}

#[test]
fn key_value_create_follows_spacing_and_line_endings() {
    let mut tight = open(b"a=1");
    tight
        .set(&key("b"), &text("x"), &SetOptions { create: true })
        .expect("creates");
    assert_eq!(tight.text(), b"a=1\nb=x\n");
    let mut crlf = open(b"a = 1\r\n");
    crlf.set(&key("b"), &Value::Bool(true), &SetOptions { create: true })
        .expect("creates");
    assert_eq!(crlf.text(), b"a = 1\r\nb = true\r\n");
    let mut empty = open(b"");
    empty
        .set(
            &key("server.port"),
            &Value::Int(80),
            &SetOptions { create: true },
        )
        .expect("creates");
    assert_eq!(empty.text(), b"server.port = 80\n");
}

#[test]
fn key_value_remove_deletes_the_line() {
    let mut document = open(b"a = 1\nb = 2\nc = 3\n");
    document.remove(&key("b")).expect("removes");
    assert_eq!(document.text(), b"a = 1\nc = 3\n");
    let missing = document.remove(&key("b")).expect_err("b is gone");
    assert_eq!(missing.code, ErrorCode::KeyNotFound);
    let root = document
        .remove(&KeyPath::root())
        .expect_err("root cannot be removed");
    assert_eq!(root.code, ErrorCode::KeyConflict);
}

#[test]
fn key_value_set_of_current_value_changes_nothing() {
    let mut document = open(SAMPLE);
    let report = document
        .set(&key("port"), &Value::Int(8080), &SetOptions::default())
        .expect("sets");
    assert!(report.is_unchanged());
    assert_eq!(document.text(), SAMPLE);
}

#[test]
fn key_value_set_rejects_values_the_format_cannot_hold() {
    let mut document = open(SAMPLE);
    let options = SetOptions { create: true };
    let map = document
        .set(
            &key("port"),
            &Value::from_plain_json(json!({"a": 1})),
            &options,
        )
        .expect_err("maps are not scalars");
    assert_eq!(map.code, ErrorCode::KeyConflict);
    let multiline = document
        .set(&key("name"), &text("a\nb"), &options)
        .expect_err("line break");
    assert_eq!(multiline.code, ErrorCode::InvalidValue);
    let padded = document
        .set(&key("name"), &text(" a"), &options)
        .expect_err("leading blank");
    assert_eq!(padded.code, ErrorCode::InvalidValue);
    let root = document
        .set(&KeyPath::root(), &text("x"), &options)
        .expect_err("root is a map");
    assert_eq!(root.code, ErrorCode::KeyConflict);
    let index = document
        .set(&key("[0]"), &text("x"), &options)
        .expect_err("index on a map");
    assert_eq!(index.code, ErrorCode::KeyConflict);
    assert_eq!(document.text(), SAMPLE);
}

#[test]
fn key_value_missing_key_suggests_similar_names() {
    let mut document = open(SAMPLE);
    let error = document
        .set(&key("prot"), &Value::Int(1), &SetOptions::default())
        .expect_err("no create");
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert_eq!(
        error
            .candidates
            .first()
            .map(|candidate| candidate.program.as_str()),
        Some("port")
    );
}

#[test]
fn key_value_wildcard_sets_every_entry() {
    let mut document = open(b"a = 1\nb = 2\n");
    let report = document
        .set(&key("[*]"), &Value::Int(0), &SetOptions::default())
        .expect("sets");
    assert_eq!(document.text(), b"a = 0\nb = 0\n");
    assert_eq!(report.replacements, 2);
    assert_eq!(report.changed.len(), 2);
}

#[test]
fn key_value_format_sniffs_and_parses() {
    let format = format();
    assert_eq!(format.sniff(b"a=1\n# c\n"), Confidence::Low);
    assert_eq!(format.sniff(b"hello"), Confidence::None);
    assert_eq!(format.sniff(b""), Confidence::None);
    let error = format
        .open(b"a = 1\n= 2\n".to_vec(), &DocOptions::default())
        .err()
        .map(|error| error.code);
    assert_eq!(error, Some(ErrorCode::DocParse));
    let reference = KeyValueReference
        .parse(b"a = x\n# c\nb = 2\n")
        .expect("parses");
    assert_eq!(
        reference,
        vec![Value::from_plain_json(json!({"a": "x", "b": 2}))]
    );
    assert!(KeyValueReference.parse(b"oops\n").is_err());
    assert!(KeyValueReference.parse(b"\xFF").is_err());
}

#[test]
fn harness_accepts_guarded_changes_and_counts_refusals() {
    let summary = run_document("sample.kv", SAMPLE, &format(), &HarnessOptions::default());
    assert!(summary.violations.is_empty(), "{:?}", summary.violations);
    assert_eq!(summary.files, 1);
    assert_eq!(summary.operations, 11);
    assert_eq!(summary.accepted, 7);
    assert_eq!(summary.refused.get("E_KEY_CONFLICT"), Some(&4));
    assert!(summary.unsupported.is_empty());
    assert!(summary.not_opened.is_empty());
}

#[test]
fn harness_is_deterministic_and_limits_nodes() {
    let options = HarnessOptions {
        nodes_per_file: 1,
        operations: vec![HarnessOperation::SetSame],
        ..HarnessOptions::default()
    };
    let first = run_document("a.kv", SAMPLE, &format(), &options);
    let second = run_document("a.kv", SAMPLE, &format(), &options);
    assert_eq!(first, second);
    assert_eq!(first.operations, 1);
    assert_eq!(HarnessOperation::ALL.len(), 6);
}

#[test]
fn harness_skips_members_shadowed_by_a_later_member() {
    let options = HarnessOptions {
        operations: vec![HarnessOperation::SetSame, HarnessOperation::ReplaceScalar],
        ..HarnessOptions::default()
    };
    let summary = run_document("dup.kv", b"a = 1\na = 2\n", &format(), &options);
    assert!(summary.violations.is_empty(), "{:?}", summary.violations);
    assert_eq!(summary.operations, 3);
    assert_eq!(summary.accepted, 2);
}

#[test]
fn change_check_accepts_separator_removed_past_a_comment() {
    let original = b"[1, // one\n 2\n]";
    let region = Span::new(11, 14);
    assert_eq!(
        first_change_outside(original, b"[1 // one\n]", region),
        None
    );
    assert_eq!(
        first_change_outside(original, b"[1 // one\n]\n", region),
        Some(14)
    );
    assert_eq!(
        first_change_outside(original, b"[0, // one\n]", region),
        Some(1)
    );
    assert_eq!(
        first_change_outside(original, b"[1; // one\n]", region),
        Some(2)
    );
}

#[test]
fn change_check_accepts_separator_added_before_the_region() {
    let original = b"[1 // one\n]";
    let at_end = Span::at(10);
    assert_eq!(
        first_change_outside(original, b"[1, // one\n 2\n]", at_end),
        None
    );
    assert_eq!(
        first_change_outside(original, b"[1 /* one\n 2\n]", at_end),
        Some(4)
    );
}

#[test]
fn change_check_locates_removal_among_equal_lines() {
    let original = b"k = 1\nk = 1\nend = 0\n";
    assert_eq!(
        first_change_outside(original, b"k = 1\nend = 0\n", Span::new(0, 6)),
        None
    );
    assert_eq!(
        first_change_outside(original, b"k = 1\nk = 1\n", Span::new(0, 6)),
        Some(18)
    );
    assert_eq!(
        first_change_outside(original, original, Span::new(40, 50)),
        None
    );
}

#[test]
fn harness_reports_documents_that_do_not_open() {
    let summary = run_document(
        "bad.kv",
        b"no separator\n",
        &format(),
        &HarnessOptions::default(),
    );
    assert_eq!(summary.not_opened.len(), 1);
    assert!(summary.violations.is_empty());
}

#[test]
fn harness_runs_corpus_by_format_or_every_file() {
    let dir = tempfile::tempdir().expect("temporary directory");
    std::fs::write(dir.path().join("a.kv"), SAMPLE).expect("writes");
    std::fs::write(dir.path().join("b.kv"), b"x = 1\n").expect("writes");
    std::fs::write(dir.path().join("notes.txt"), b"not a document\n").expect("writes");
    std::fs::create_dir(dir.path().join("nested")).expect("creates directory");
    std::fs::write(dir.path().join("nested").join("c.kv"), b"y = true\n").expect("writes");
    let by_format = run_corpus(dir.path(), &format(), &HarnessOptions::default()).expect("runs");
    assert_eq!(by_format.files, 3);
    assert!(
        by_format.violations.is_empty(),
        "{:?}",
        by_format.violations
    );
    let all = HarnessOptions {
        filter: FileFilter::All,
        ..HarnessOptions::default()
    };
    let every = run_corpus(dir.path(), &format(), &all).expect("runs");
    assert_eq!(every.files, 4);
    assert_eq!(every.not_opened.len(), 1);
    assert!(run_corpus(&dir.path().join("missing"), &format(), &all).is_err());
}

#[test]
fn summaries_merge_counts() {
    let mut total = Summary::default();
    total.merge(run_document(
        "a.kv",
        SAMPLE,
        &format(),
        &HarnessOptions::default(),
    ));
    total.merge(run_document(
        "b.kv",
        SAMPLE,
        &format(),
        &HarnessOptions::default(),
    ));
    assert_eq!(total.files, 2);
    assert_eq!(total.operations, 22);
    assert_eq!(total.refused.get("E_KEY_CONFLICT"), Some(&8));
}

/// Defect injected by [`Faulty`] to check that the harness detects it.
#[derive(Clone, Copy)]
enum Defect {
    /// The opened document drops the final line break.
    Lossy,
    /// Every change also inserts a line at the start of the document.
    Prefix,
    /// Every change writes another value than the requested one.
    Wrong,
    /// As [`Defect::Wrong`], under semantics whose canonical form of every
    /// scalar is `null`.
    Blind,
}

/// Semantics of the test format under which every scalar compares as
/// `null`, so that writing another scalar keeps the canonical value.
struct ScalarBlind;

impl FormatSemantics for ScalarBlind {
    fn repeated(&self) -> Repeated {
        Repeated::LastWins
    }

    fn flat_keys(&self) -> bool {
        true
    }

    fn canonical(&self, value: Value) -> Value {
        match value {
            Value::Map(fields) => Value::Map(
                fields
                    .into_iter()
                    .map(|(name, value)| (name, self.canonical(value)))
                    .collect(),
            ),
            Value::List(items) => {
                Value::List(items.into_iter().map(|item| self.canonical(item)).collect())
            }
            _ => Value::Null,
        }
    }
}

struct Faulty {
    defect: Defect,
}

impl DocFormat for Faulty {
    fn id(&self) -> DocFormatId {
        format().id()
    }

    fn extensions(&self) -> &[&'static str] {
        &["kv"]
    }

    fn sniff(&self, content: &[u8]) -> Confidence {
        format().sniff(content)
    }

    fn open(&self, content: Vec<u8>, options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let inner = format().open(content, options)?;
        let text = match self.defect {
            Defect::Lossy => Some(
                inner
                    .text()
                    .strip_suffix(b"\n")
                    .unwrap_or_default()
                    .to_vec(),
            ),
            Defect::Prefix | Defect::Wrong | Defect::Blind => None,
        };
        Ok(Box::new(FaultyDocument {
            defect: self.defect,
            inner,
            text,
        }))
    }
}

struct FaultyDocument {
    defect: Defect,
    inner: Box<dyn DocHandle>,
    text: Option<Vec<u8>>,
}

impl DocHandle for FaultyDocument {
    fn format(&self) -> DocFormatId {
        self.inner.format()
    }

    fn text(&self) -> &[u8] {
        self.text.as_deref().unwrap_or_else(|| self.inner.text())
    }

    fn tree(&self) -> &DocTree {
        self.inner.tree()
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        match self.defect {
            Defect::Blind => &ScalarBlind,
            Defect::Lossy | Defect::Prefix | Defect::Wrong => self.inner.semantics(),
        }
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        match self.defect {
            Defect::Wrong | Defect::Blind => self.inner.set(key, &text("wrong"), options),
            Defect::Lossy | Defect::Prefix => {
                let report = self.inner.set(key, value, options)?;
                self.text = Some([b"# x\n".as_slice(), self.inner.text()].concat());
                Ok(report)
            }
        }
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        self.inner.remove(key)
    }
}

fn guards(summary: &Summary) -> Vec<Guard> {
    let mut guards: Vec<Guard> = summary
        .violations
        .iter()
        .map(|violation| violation.guard)
        .collect();
    guards.sort();
    guards.dedup();
    guards
}

#[test]
fn harness_detects_document_that_loses_bytes_on_opening() {
    let summary = run_document(
        "a.kv",
        SAMPLE,
        &Faulty {
            defect: Defect::Lossy,
        },
        &HarnessOptions::default(),
    );
    assert_eq!(guards(&summary), [Guard::Identity]);
    assert_eq!(summary.operations, 0);
    assert!(summary.violations[0].operation.is_none());
}

#[test]
fn harness_detects_changes_outside_the_node_and_changed_current_value() {
    let summary = run_document(
        "a.kv",
        SAMPLE,
        &Faulty {
            defect: Defect::Prefix,
        },
        &HarnessOptions::default(),
    );
    let found = guards(&summary);
    assert!(found.contains(&Guard::Scope), "{:?}", summary.violations);
    assert!(
        found.contains(&Guard::Idempotence),
        "{:?}",
        summary.violations
    );
    let scope = summary
        .violations
        .iter()
        .find(|violation| violation.guard == Guard::Scope)
        .expect("scope violation");
    assert_eq!(scope.path.to_string_lossy(), "a.kv");
    assert!(!scope.key.is_empty());
}

#[test]
fn harness_detects_changes_with_another_meaning() {
    let options = HarnessOptions {
        operations: vec![HarnessOperation::ReplaceScalar],
        ..HarnessOptions::default()
    };
    let summary = run_document(
        "a.kv",
        SAMPLE,
        &Faulty {
            defect: Defect::Wrong,
        },
        &options,
    );
    assert_eq!(guards(&summary), [Guard::Meaning]);
    assert_eq!(summary.violations.len(), 2);
}

#[test]
fn harness_compares_canonical_forms_of_the_format() {
    let options = HarnessOptions {
        operations: vec![HarnessOperation::ReplaceScalar],
        ..HarnessOptions::default()
    };
    let summary = run_document(
        "a.kv",
        SAMPLE,
        &Faulty {
            defect: Defect::Blind,
        },
        &options,
    );
    assert!(summary.violations.is_empty(), "{:?}", summary.violations);
    assert_eq!(summary.accepted, 2);
}

/// Format that opens documents only when the file path is known, as a
/// format whose dialect follows from the file name does.
struct NeedsPath;

impl DocFormat for NeedsPath {
    fn id(&self) -> DocFormatId {
        format().id()
    }

    fn extensions(&self) -> &[&'static str] {
        &["kv"]
    }

    fn sniff(&self, content: &[u8]) -> Confidence {
        format().sniff(content)
    }

    fn open(&self, _content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        Err(Error::new(ErrorCode::DocParse, "the file path is unknown"))
    }

    fn open_at(
        &self,
        path: Option<&Path>,
        content: Vec<u8>,
        options: &DocOptions,
    ) -> Result<Box<dyn DocHandle>> {
        match path {
            Some(_) => format().open(content, options),
            None => self.open(content, options),
        }
    }
}

#[test]
fn harness_and_registry_pass_the_file_path_to_the_format() {
    let summary = run_document("a.kv", SAMPLE, &NeedsPath, &HarnessOptions::default());
    assert!(summary.not_opened.is_empty(), "{:?}", summary.not_opened);
    assert!(summary.violations.is_empty(), "{:?}", summary.violations);
    assert_eq!(summary.accepted, 7);
    let options = DocOptions::default();
    assert!(open_with(&NeedsPath, SAMPLE, &options).is_err());
    let registry = Registry::with([Box::new(NeedsPath) as Box<dyn DocFormat>]);
    let id = format().id();
    assert!(registry.open(&id, SAMPLE, &options).is_err());
    let opened = registry
        .open_at(&id, Some(Path::new("a.kv")), SAMPLE, &options)
        .expect("opens with the path");
    assert_eq!(opened.bytes(), SAMPLE);
    assert!(
        format()
            .open_at(Some(Path::new("a.kv")), SAMPLE.to_vec(), &options)
            .is_ok()
    );
}

fn target(path: &AbsPath, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

fn settings() -> AbsPath {
    AbsPath::for_test("/srv/app/settings.kv")
}

#[test]
fn document_change_goes_through_transaction_and_undo_restores_bytes() {
    let path = settings();
    let journal = MemoryJournal::new().with_file(path.as_path(), SAMPLE.to_vec());
    let mut document = open_with(&format(), SAMPLE, &DocOptions::default()).expect("opens");
    document
        .handle
        .set(&key("port"), &Value::Int(9090), &SetOptions::default())
        .expect("sets");
    let mut tx = journal.begin(OpKind::command("set"));
    tx.preserve(&[target(&path, TargetAction::Modify)])
        .expect("preserves");
    tx.apply(TargetIndex(0), Mutation::WriteBytes(document.bytes()))
        .expect("writes");
    let handle = tx.commit().expect("commits");
    assert!(handle.op.is_some());
    assert_eq!(handle.targets.len(), 1);
    assert_eq!(journal.operations(), 1);
    assert_eq!(journal.last_kind(), Some(OpKind::command("set")));
    assert_eq!(
        journal.file(path.as_path()).as_deref(),
        Some(&b"# port\nport = 9090\nname = web\n"[..])
    );
    journal.undo_last().expect("undoes");
    assert_eq!(journal.file(path.as_path()).as_deref(), Some(SAMPLE));
    assert_eq!(
        journal.undo_last().expect_err("nothing left").code,
        ErrorCode::JournalNoop
    );
}

#[test]
fn transaction_without_changes_records_no_operation() {
    let path = settings();
    let journal = MemoryJournal::new().with_file(path.as_path(), SAMPLE.to_vec());
    let mut tx = journal.begin(OpKind::command("set"));
    tx.preserve(&[target(&path, TargetAction::Modify)])
        .expect("preserves");
    assert!(tx.op().is_some());
    let handle = tx.commit().expect("commits");
    assert!(handle.op.is_none());
    assert_eq!(journal.operations(), 0);
}

#[test]
fn rollback_restores_changed_targets() {
    let path = settings();
    let created = AbsPath::for_test("/srv/app/new.kv");
    let journal = MemoryJournal::new().with_file(path.as_path(), SAMPLE.to_vec());
    let mut tx = journal.begin(OpKind::command("set"));
    tx.preserve(&[
        target(&path, TargetAction::Modify),
        target(&created, TargetAction::Create),
    ])
    .expect("preserves");
    tx.apply(TargetIndex(0), Mutation::AppendBytes(b"x = 1\n".to_vec()))
        .expect("appends");
    tx.apply(TargetIndex(1), Mutation::WriteBytes(b"y = 2\n".to_vec()))
        .expect("creates");
    let report = tx.rollback(RollbackReason::Cancelled).expect("rolls back");
    assert_eq!(report.restored.len(), 2);
    assert_eq!(journal.file(path.as_path()).as_deref(), Some(SAMPLE));
    assert_eq!(journal.file(created.as_path()), None);
}

#[test]
fn removal_and_creation_are_undone() {
    let path = settings();
    let created = AbsPath::for_test("/srv/app/new.kv");
    let journal = MemoryJournal::new().with_file(path.as_path(), SAMPLE.to_vec());
    let mut tx = journal.begin(OpKind::command("remove"));
    tx.preserve(&[
        target(&path, TargetAction::Delete),
        target(&created, TargetAction::Create),
    ])
    .expect("preserves");
    tx.apply(TargetIndex(0), Mutation::Remove).expect("removes");
    tx.apply(TargetIndex(1), Mutation::WriteBytes(b"y = 2\n".to_vec()))
        .expect("creates");
    tx.commit().expect("commits");
    assert_eq!(journal.file(path.as_path()), None);
    journal.undo_last().expect("undoes");
    assert_eq!(journal.file(path.as_path()).as_deref(), Some(SAMPLE));
    assert_eq!(journal.file(created.as_path()), None);
}

#[test]
fn transaction_rejects_misuse() {
    let path = settings();
    let missing = AbsPath::for_test("/srv/app/missing.kv");
    let journal = MemoryJournal::new().with_file(path.as_path(), SAMPLE.to_vec());
    let mut tx = journal.begin(OpKind::command("set"));
    let exists = tx
        .preserve(&[target(&path, TargetAction::Create)])
        .expect_err("file exists");
    assert_eq!(exists.code, ErrorCode::Exists);
    let absent = tx
        .preserve(&[target(&missing, TargetAction::Delete)])
        .expect_err("file is missing");
    assert_eq!(absent.code, ErrorCode::NotFound);
    tx.preserve(&[target(&path, TargetAction::Modify)])
        .expect("preserves");
    assert_eq!(
        tx.stage(TargetIndex(0)).err().map(|error| error.code),
        Some(ErrorCode::Unsupported)
    );
    let undeclared = tx
        .apply(TargetIndex(5), Mutation::Remove)
        .expect_err("undeclared target");
    assert_eq!(undeclared.code, ErrorCode::StageFailed);
    tx.apply(TargetIndex(0), Mutation::WriteBytes(Vec::new()))
        .expect("writes");
    let late = tx
        .preserve(&[target(&missing, TargetAction::Create)])
        .expect_err("after a change");
    assert_eq!(late.code, ErrorCode::StageFailed);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
