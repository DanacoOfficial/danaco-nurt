// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conformance and edit runs over the JSON and JSONC corpus in
//! `tests/d5/corpus` and the JSON Test Suite in
//! `tests/d5/third_party/json/jsontestsuite`.
//!
//! Every accepted document is edited at up to [`MAX_NODES`] values chosen
//! with a fixed stride. Each edit must stay inside the span of its target,
//! produce text that both parsers read as the expected value, keep the line
//! ending style and leave a repeated set of the same value without change.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::Value;

use super::editor::apply;
use super::syntax::{NodeId, Span};
use super::{Dialect, JsonDocument, Outcome, Target, reference};

/// Path that adds a value (a new member name or `[+]`) and path that reads
/// it back (the member name or the new index).
type AddedPaths = (Vec<KeySegment>, Vec<KeySegment>);

/// Largest number of values edited per document.
const MAX_NODES: usize = 50;

/// Largest number of failures listed in a test message.
const MAX_REPORTED: usize = 25;

/// Number of parsing cases of the JSON Test Suite.
const SUITE_CASES: usize = 318;

/// Test data of the formats: `corpus/` and `third_party/`.
fn data_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d5")
}

fn json_test_suite() -> PathBuf {
    data_root().join("third_party/json/jsontestsuite")
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json" | "jsonc")
            ) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Text of a document file: UTF-8 with an optional byte order mark.
fn decode(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(
        bytes
            .strip_prefix(b"\xEF\xBB\xBF".as_slice())
            .unwrap_or(bytes),
    )
    .ok()
}

fn report(failures: &[String], total: usize) -> String {
    let mut message = format!("{} of {total} checks failed:\n", failures.len());
    for failure in failures.iter().take(MAX_REPORTED) {
        let _ = writeln!(message, "  {failure}");
    }
    message
}

#[test]
fn reads_every_valid_and_rejects_every_invalid_json_test_suite_case() {
    let suite = files(&json_test_suite());
    assert_eq!(suite.len(), SUITE_CASES);
    let mut failures = Vec::new();
    for path in &suite {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = std::fs::read(path).unwrap();
        let parsed = decode(&bytes).map(|text| (text, JsonDocument::parse(text, Dialect::Json)));
        match (&name[..2], parsed) {
            ("y_", Some((text, Ok(document)))) => {
                let expected = reference::read(text, Dialect::Json).unwrap();
                if !reference::same_data(&document.to_value(), &expected) {
                    failures.push(format!("{name}: value differs from serde_json"));
                }
            }
            ("y_", _) => failures.push(format!("{name}: rejected")),
            ("n_", Some((_, Ok(_)))) => failures.push(format!("{name}: accepted")),
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", report(&failures, suite.len()));
}

#[test]
fn edits_every_corpus_document_inside_the_target_span() {
    let root = data_root();
    let mut documents: Vec<(PathBuf, Dialect)> = Vec::new();
    for path in files(&root.join("corpus/json"))
        .into_iter()
        .chain(files(&json_test_suite()))
    {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("n_") {
            documents.push((path, Dialect::Json));
        }
    }
    documents.extend(
        files(&root.join("corpus/jsonc"))
            .into_iter()
            .map(|path| (path, Dialect::Jsonc)),
    );
    let mut failures = Vec::new();
    let mut operations = 0;
    let mut edited = 0;
    for (path, dialect) in &documents {
        let bytes = std::fs::read(path).unwrap();
        let Some(text) = decode(&bytes) else { continue };
        let Ok(document) = JsonDocument::parse(text, *dialect) else {
            assert!(
                path.to_string_lossy().contains("/i_"),
                "{} does not parse",
                path.display()
            );
            continue;
        };
        edited += 1;
        let name = path.strip_prefix(&root).unwrap().display().to_string();
        let mut run = Run {
            document: &document,
            name: &name,
            failures: &mut failures,
            operations: &mut operations,
        };
        run.all();
    }
    assert!(edited >= 180, "only {edited} documents edited");
    assert!(failures.is_empty(), "{}", report(&failures, operations));
}

/// Edits of one document.
struct Run<'a> {
    document: &'a JsonDocument,
    name: &'a str,
    failures: &'a mut Vec<String>,
    operations: &'a mut usize,
}

/// Edit applied to one value.
enum Operation {
    Replace(Value),
    Remove,
    Add(Value),
}

impl Run<'_> {
    fn all(&mut self) {
        let model = self.document.to_value();
        match reference::read(self.document.text(), self.document.dialect()) {
            Ok(expected) if reference::same_data(&model, &expected) => {}
            _ => self
                .failures
                .push(format!("{}: value differs from serde_json", self.name)),
        }
        let mut paths = Vec::new();
        collect_paths(&model, &mut Vec::new(), &mut paths);
        let stride = paths.len().div_ceil(MAX_NODES).max(1);
        for (position, (segments, current)) in paths.iter().step_by(stride).enumerate() {
            let key = KeyPath::new(None, segments.clone()).unwrap();
            self.check_unchanged(&key, current);
            let operation = match position % 4 {
                0 => Operation::Replace(Value::Text("nurt \"edit\"\n".to_owned())),
                1 => Operation::Replace(replacement_of_other_kind(current)),
                2 if !segments.is_empty() => Operation::Remove,
                _ => Operation::Add(Value::Map(IndexMap::from([(
                    "nurt".to_owned(),
                    Value::List(vec![Value::Int(1), Value::Decimal("1.50".to_owned())]),
                )]))),
            };
            self.check(&model, segments, &key, &operation);
        }
    }

    fn check_unchanged(&mut self, key: &KeyPath, current: &Value) {
        *self.operations += 1;
        match self.document.plan_set(key, current, false) {
            Ok(outcome) if outcome.changes.is_empty() => {}
            other => self.failures.push(format!(
                "{} {key}: same value changed text: {other:?}",
                self.name
            )),
        }
    }

    fn check(
        &mut self,
        model: &Value,
        segments: &[KeySegment],
        key: &KeyPath,
        operation: &Operation,
    ) {
        *self.operations += 1;
        let document = self.document;
        let (planned, expected, bounds, added_key) = match operation {
            Operation::Replace(value) => (
                document.plan_set(key, value, false),
                set_at(model, segments, value.clone()),
                self.target_span(key, false),
                None,
            ),
            Operation::Remove => (
                document.plan_remove(key),
                remove_at(model, segments),
                self.target_span(key, true),
                None,
            ),
            Operation::Add(value) => {
                let (paths, expected) = add_below(model, segments, value);
                let Some((added, reread)) = paths else { return };
                let target = KeyPath::new(None, added).unwrap();
                (
                    document.plan_set(&target, value, true),
                    expected,
                    self.target_span(key, false),
                    Some(KeyPath::new(None, reread).unwrap()),
                )
            }
        };
        let label = format!("{} {key}", self.name);
        let outcome = match planned {
            Ok(outcome) => outcome,
            Err(error) => {
                self.failures.push(format!("{label}: {}", error.message));
                return;
            }
        };
        if let Err(problem) = self.verify(&outcome, &expected, bounds) {
            self.failures.push(format!("{label}: {problem}"));
            return;
        }
        let reread_key = added_key.as_ref().unwrap_or(key);
        if let Operation::Replace(value) | Operation::Add(value) = operation {
            let text = apply(document.text(), &outcome.changes).unwrap();
            let again = JsonDocument::parse(text, document.dialect()).unwrap();
            match again.plan_set(reread_key, value, false) {
                Ok(repeat) if repeat.changes.is_empty() => {}
                other => self
                    .failures
                    .push(format!("{label}: repeated set changed text: {other:?}")),
            }
        }
    }

    /// Span an edit of `key` may change: the value itself for a
    /// replacement, the enclosing container for removals and additions.
    fn target_span(&self, key: &KeyPath, container: bool) -> Span {
        let tree = &self.document.tree;
        let Ok(Target::Node { id, .. }) = self.document.locate(key) else {
            return Span::new(0, self.document.text().len());
        };
        let chosen: NodeId = if container {
            tree.nodes[id].parent.unwrap_or(id)
        } else {
            id
        };
        tree.nodes[chosen].span
    }

    fn verify(&self, outcome: &Outcome, expected: &Value, bounds: Span) -> Result<(), String> {
        let document = self.document;
        for change in &outcome.changes {
            if change.span.start < bounds.start || change.span.end > bounds.end {
                return Err(format!("change {:?} outside {bounds:?}", change.span));
            }
        }
        let text =
            apply(document.text(), &outcome.changes).map_err(|error| error.message.clone())?;
        let reparsed = JsonDocument::parse(text.clone(), document.dialect())
            .map_err(|error| format!("{}\n{text}", error.message))?;
        if !reference::same_data(&reparsed.to_value(), expected) {
            return Err(format!("unexpected value\n{text}"));
        }
        let reference = reference::read(&text, document.dialect())?;
        if !reference::same_data(&reference, expected) {
            return Err(format!("serde_json reads another value\n{text}"));
        }
        let original = document.text();
        let lf_only = !original.contains('\r');
        let crlf_only = original.contains('\n')
            && original.matches('\n').count() == original.matches("\r\n").count();
        if (lf_only && text.contains('\r'))
            || (crlf_only && text.matches('\n').count() != text.matches("\r\n").count())
        {
            return Err(format!("line ending style changed\n{text:?}"));
        }
        Ok(())
    }
}

/// Paths of all values in pre-order with their current values.
fn collect_paths(
    value: &Value,
    prefix: &mut Vec<KeySegment>,
    out: &mut Vec<(Vec<KeySegment>, Value)>,
) {
    out.push((prefix.clone(), value.clone()));
    match value {
        Value::Map(fields) => {
            for (name, item) in fields {
                prefix.push(KeySegment::Key(name.clone()));
                collect_paths(item, prefix, out);
                prefix.pop();
            }
        }
        Value::List(items) => {
            for (index, item) in items.iter().enumerate() {
                prefix.push(KeySegment::Index(i64::try_from(index).unwrap()));
                collect_paths(item, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn replacement_of_other_kind(current: &Value) -> Value {
    match current {
        Value::Map(_) | Value::List(_) => Value::Int(-12),
        _ => Value::Map(IndexMap::from([
            ("kind".to_owned(), Value::Text("changed".to_owned())),
            (
                "values".to_owned(),
                Value::List(vec![Value::Bool(true), Value::Null]),
            ),
        ])),
    }
}

fn child_mut<'v>(value: &'v mut Value, segment: &KeySegment) -> &'v mut Value {
    match (value, segment) {
        (Value::Map(fields), KeySegment::Key(name)) => fields.get_mut(name).unwrap(),
        (Value::List(items), KeySegment::Index(index)) => {
            &mut items[usize::try_from(*index).unwrap()]
        }
        _ => unreachable!("collected paths follow the value"),
    }
}

fn set_at(model: &Value, segments: &[KeySegment], value: Value) -> Value {
    let mut result = model.clone();
    let mut slot = &mut result;
    for segment in segments {
        slot = child_mut(slot, segment);
    }
    *slot = value;
    result
}

fn remove_at(model: &Value, segments: &[KeySegment]) -> Value {
    let mut result = model.clone();
    let (last, parents) = segments.split_last().unwrap();
    let mut slot = &mut result;
    for segment in parents {
        slot = child_mut(slot, segment);
    }
    match (slot, last) {
        (Value::Map(fields), KeySegment::Key(name)) => {
            fields.shift_remove(name);
        }
        (Value::List(items), KeySegment::Index(index)) => {
            items.remove(usize::try_from(*index).unwrap());
        }
        _ => unreachable!("collected paths follow the value"),
    }
    result
}

/// Path of a new member or element inside the container at `segments`,
/// and the model after adding `value` there; `None` for scalars.
fn add_below(model: &Value, segments: &[KeySegment], value: &Value) -> (Option<AddedPaths>, Value) {
    let mut result = model.clone();
    let mut slot = &mut result;
    for segment in segments {
        slot = child_mut(slot, segment);
    }
    let mut added = segments.to_vec();
    let mut reread = segments.to_vec();
    match slot {
        Value::Map(fields) => {
            let name = (0..)
                .map(|n| format!("nurt_added_{n}"))
                .find(|name| !fields.contains_key(name))
                .unwrap();
            fields.insert(name.clone(), value.clone());
            added.push(KeySegment::Key(name.clone()));
            reread.push(KeySegment::Key(name));
        }
        Value::List(items) => {
            reread.push(KeySegment::Index(i64::try_from(items.len()).unwrap()));
            items.push(value.clone());
            added.push(KeySegment::Append);
        }
        _ => return (None, result),
    }
    (Some((added, reread)), result)
}
