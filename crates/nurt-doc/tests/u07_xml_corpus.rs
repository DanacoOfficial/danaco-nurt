// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Round trip over the XML corpus `tests/d5/corpus/xml/core`.
//!
//! For every file: the tree keeps the bytes (identity), setting the current
//! value of every node changes nothing (idempotence), and up to
//! `MAX_TARGETS` deterministically chosen nodes are changed, removed or
//! extended. After each change the document is parsed again; the changed
//! node must hold the new value and every other element must keep its
//! name, attributes and text. Changes that the tree refuses must fail with
//! `E_UNSUPPORTED_CONSTRUCT`; they are counted by construct name.
//!
//! The value model is compared with the output of `yq -p xml -o json`
//! (yq 4.53.6) stored in `tests/d5/data/u07/yq`; documented differences
//! are listed in `tests/d5/data/u07/yq-known-differences.txt`.

// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::{Fields, Value};
use nurt_doc::formats::xml::{NodeId, Target, XmlChange, XmlTree};

/// Nodes changed per file at most.
const MAX_TARGETS: usize = 50;

/// Number of distinct operations applied in turn to the chosen nodes.
const OPERATIONS: usize = 5;

/// Minimum number of corpus files.
const MIN_FILES: usize = 300;

fn data_dir(relative: &str) -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5")
    .join(relative)
}

fn corpus() -> Vec<(String, String)> {
    let mut files: Vec<PathBuf> = fs::read_dir(data_dir("corpus/xml/core"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext != "md"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect()
}

/// Element of the document with what an unrelated edit must not change.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    id: NodeId,
    depth: usize,
    name: String,
    attributes: Vec<(String, String)>,
    chunks: Vec<String>,
}

impl Entry {
    fn same_content(&self, other: &Self) -> bool {
        self.depth == other.depth
            && self.name == other.name
            && self.attributes == other.attributes
            && self.chunks == other.chunks
    }
}

fn depth(tree: &XmlTree, id: NodeId) -> usize {
    let mut depth = 0;
    let mut current = tree.parent(id);
    while let Some(parent) = current {
        depth += 1;
        current = tree.parent(parent);
    }
    depth
}

fn signature(tree: &XmlTree) -> Vec<Entry> {
    tree.node_ids()
        .filter_map(|id| {
            let element = tree.element(id)?;
            Some(Entry {
                id,
                depth: depth(tree, id),
                name: tree.name(id).to_owned(),
                attributes: element
                    .attributes
                    .iter()
                    .map(|a| {
                        (
                            tree.attribute_name(a).to_owned(),
                            tree.attribute_value(a).unwrap(),
                        )
                    })
                    .collect(),
                chunks: tree
                    .text_chunks(id)
                    .into_iter()
                    .map(|chunk| tree.chunk_value(chunk).unwrap().unwrap_or_default())
                    .collect(),
            })
        })
        .collect()
}

/// Signature without the subtree of the element `id`.
fn without_subtree(entries: &[Entry], id: NodeId) -> Vec<Entry> {
    let Some(start) = entries.iter().position(|entry| entry.id == id) else {
        return entries.to_vec();
    };
    let depth = entries[start].depth;
    let end = entries[start + 1..]
        .iter()
        .position(|entry| entry.depth <= depth)
        .map_or(entries.len(), |offset| start + 1 + offset);
    let mut kept = entries[..start].to_vec();
    kept.extend_from_slice(&entries[end..]);
    kept
}

fn differing(old: &[Entry], new: &[Entry]) -> Option<usize> {
    (old.len() == new.len()).then(|| {
        old.iter()
            .zip(new)
            .filter(|(a, b)| !a.same_content(b))
            .count()
    })
}

fn same(old: &[Entry], new: &[Entry]) -> bool {
    differing(old, new) == Some(0)
}

#[derive(Default)]
struct Summary {
    files: usize,
    operations: usize,
    violations: Vec<String>,
    unsupported: BTreeMap<String, usize>,
}

impl Summary {
    fn refused(&mut self, file: &str, key: &str, error: &Error) {
        if error.code() == ErrorCode::UnsupportedConstruct {
            let name = error.data().found.clone().unwrap_or_default();
            *self.unsupported.entry(name).or_default() += 1;
        } else {
            self.violations.push(format!("{file}: {key}: {error}"));
        }
    }
}

/// Candidate nodes: leaf elements, attributes other than namespace
/// declarations and text chunks of elements with children.
fn candidates(tree: &XmlTree) -> Vec<Target> {
    let mut targets = Vec::new();
    for id in tree.node_ids() {
        let Some(element) = tree.element(id) else {
            continue;
        };
        let leaf = tree.child_elements(id).next().is_none();
        if leaf && id != tree.root() {
            targets.push(Target::Element(id));
        }
        for (index, attribute) in element.attributes.iter().enumerate() {
            if !tree.attribute_name(attribute).starts_with("xmlns") {
                targets.push(Target::Attribute { element: id, index });
            }
        }
        if !leaf {
            for node in tree.text_chunks(id) {
                targets.push(Target::Text { element: id, node });
            }
        }
    }
    targets
}

/// Text of a value: the `+content` member of an element that keeps its
/// attributes, the value itself otherwise.
fn content_of(value: &Value) -> Value {
    match value {
        Value::Map(map) => map.get("+content").cloned().unwrap_or(Value::Null),
        other => other.clone(),
    }
}

fn chosen(targets: Vec<Target>) -> Vec<Target> {
    let step = targets.len().div_ceil(MAX_TARGETS).max(1);
    targets.into_iter().step_by(step).collect()
}

struct Case<'a> {
    file: &'a str,
    text: &'a str,
    tree: &'a XmlTree,
    before: &'a [Entry],
}

impl Case<'_> {
    fn reparse(&self, change: &XmlChange) -> Result<(XmlTree, String), String> {
        let output = change.apply(self.text).map_err(|e| e.to_string())?;
        let tree = XmlTree::parse(&output).map_err(|e| format!("{e}\n{output}"))?;
        Ok((tree, output))
    }

    fn element_of(&self, target: Target) -> NodeId {
        match target {
            Target::Element(id)
            | Target::Attribute { element: id, .. }
            | Target::Text { element: id, .. } => id,
            Target::Document => self.tree.root(),
        }
    }

    fn run(&self, summary: &mut Summary, index: usize, target: Target) {
        let key_text = self.tree.canonical_key(target);
        let key = KeyPath::parse(&key_text).unwrap();
        summary.operations += 1;
        let outcome = match (index % OPERATIONS, target) {
            (1, _) => self.add_attribute(&key_text, self.element_of(target)),
            (2, Target::Element(_) | Target::Attribute { .. }) => self.remove(&key, target),
            (3, Target::Element(id)) => self.append_child(&key_text, id),
            (4, Target::Element(id)) => self.change_type(&key, id),
            _ => self.replace_scalar(&key, index),
        };
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(message)) => summary
                .violations
                .push(format!("{}: {key_text}: {message}", self.file)),
            Err(error) => summary.refused(self.file, &key_text, &error),
        }
    }

    fn replace_scalar(&self, key: &KeyPath, index: usize) -> Result<Result<(), String>, Error> {
        let value = Value::Text(format!("n{index} <&> \"q'\" ]]> end"));
        let change = self.tree.set(key, &value, false)?;
        Ok(self.reparse(&change).and_then(|(tree, output)| {
            let read = tree.read(key).map_err(|e| e.to_string())?;
            if content_of(&read[0].value) != value {
                return Err(format!("read {:?} after set\n{output}", read[0].value));
            }
            match differing(self.before, &signature(&tree)) {
                Some(0 | 1) => Ok(()),
                other => Err(format!("{other:?} elements changed\n{output}")),
            }
        }))
    }

    fn add_attribute(&self, key_text: &str, element: NodeId) -> Result<Result<(), String>, Error> {
        let element_key = self.tree.canonical_key(Target::Element(element));
        let key = KeyPath::parse(&format!("{element_key}.@nurt-added")).unwrap();
        let value = Value::Text("a&b".to_owned());
        let change = self.tree.set(&key, &value, true)?;
        Ok(self.reparse(&change).and_then(|(tree, output)| {
            let after = signature(&tree);
            let read = tree.read(&key).map_err(|e| e.to_string())?;
            if read[0].value != value || differing(self.before, &after) != Some(1) {
                return Err(format!(
                    "attribute for {key_text} not added alone\n{output}"
                ));
            }
            Ok(())
        }))
    }

    fn remove(&self, key: &KeyPath, target: Target) -> Result<Result<(), String>, Error> {
        let change = self.tree.remove(key)?;
        Ok(self.reparse(&change).and_then(|(tree, output)| {
            let after = signature(&tree);
            let expected_ok = match target {
                Target::Element(id) => {
                    let old = without_subtree(self.before, id);
                    old.len() == after.len()
                        && old.iter().zip(&after).all(|(a, b)| a.same_content(b))
                }
                _ => differing(self.before, &after) == Some(1),
            };
            if expected_ok {
                Ok(())
            } else {
                Err(format!("removal changed other nodes\n{output}"))
            }
        }))
    }

    fn append_child(&self, key_text: &str, element: NodeId) -> Result<Result<(), String>, Error> {
        let key = KeyPath::parse(&format!("{key_text}.added")).unwrap();
        let value = Value::Text("c".to_owned());
        let change = self.tree.set(&key, &value, true)?;
        let created = change.keys_created.first().cloned().unwrap_or_default();
        Ok(self.reparse(&change).and_then(|(tree, output)| {
            let created_key = KeyPath::parse(&created).map_err(|e| e.to_string())?;
            let targets = tree.resolve(&created_key).map_err(|e| e.to_string())?;
            let Target::Element(new_id) = targets[0] else {
                return Err("created key is not an element".to_owned());
            };
            if tree.element_value(new_id).map_err(|e| e.to_string())? != value {
                return Err(format!(
                    "created element of {element:?} holds another value\n{output}"
                ));
            }
            if same(self.before, &without_subtree(&signature(&tree), new_id)) {
                Ok(())
            } else {
                Err(format!("insertion changed other nodes\n{output}"))
            }
        }))
    }

    fn change_type(&self, key: &KeyPath, element: NodeId) -> Result<Result<(), String>, Error> {
        let mut fields = Fields::new();
        fields.insert("@k".to_owned(), Value::Text("v".to_owned()));
        fields.insert("child".to_owned(), Value::Text("c".to_owned()));
        let change = self.tree.set(key, &Value::Map(fields), false)?;
        Ok(self.reparse(&change).and_then(|(tree, output)| {
            let read = tree.read(key).map_err(|e| e.to_string())?;
            // An unprefixed child of a prefixed element receives the prefix.
            let child = match self.tree.name(element).split_once(':') {
                Some((prefix, _)) => format!("{prefix}:child"),
                None => "child".to_owned(),
            };
            let mut members = serde_json::Map::new();
            members.insert("+@k".to_owned(), "v".into());
            members.insert(child, "c".into());
            let expected = serde_json::Value::Object(members);
            if read[0].value.to_plain_json() != expected {
                return Err(format!(
                    "read {:?} after type change\n{output}",
                    read[0].value
                ));
            }
            let Target::Element(new_id) = read[0].target else {
                return Err("type change lost the element".to_owned());
            };
            let old = without_subtree(self.before, element);
            if same(&old, &without_subtree(&signature(&tree), new_id)) {
                Ok(())
            } else {
                Err(format!("type change changed other nodes\n{output}"))
            }
        }))
    }
}

fn idempotence_violations(file: &str, tree: &XmlTree) -> Vec<String> {
    let mut violations = Vec::new();
    for target in candidates(tree)
        .into_iter()
        .chain(std::iter::once(Target::Element(tree.root())))
    {
        let key_text = tree.canonical_key(target);
        let key = KeyPath::parse(&key_text).unwrap();
        let value = tree.target_value(target).unwrap();
        match tree.set(&key, &value, false) {
            Ok(change) if change.is_empty() => {}
            Ok(change) => violations.push(format!(
                "{file}: {key_text}: same value changed {:?}",
                change.edits
            )),
            Err(error) => violations.push(format!("{file}: {key_text}: {error}")),
        }
    }
    violations
}

#[test]
fn corpus_has_at_least_three_hundred_files() {
    assert!(corpus().len() >= MIN_FILES, "{} files", corpus().len());
}

#[test]
fn corpus_round_trip_preserves_identity_and_isolates_changes() {
    let mut summary = Summary::default();
    for (file, text) in corpus() {
        let tree = XmlTree::parse(&text).unwrap_or_else(|error| panic!("{file}: {error}"));
        assert_eq!(tree.text(), text, "{file}: identity");
        summary.files += 1;
        summary
            .violations
            .extend(idempotence_violations(&file, &tree));
        let before = signature(&tree);
        let case = Case {
            file: &file,
            text: &text,
            tree: &tree,
            before: &before,
        };
        for (index, target) in chosen(candidates(&tree)).into_iter().enumerate() {
            case.run(&mut summary, index, target);
        }
    }
    assert!(
        summary.violations.is_empty(),
        "{} violations in {} files and {} operations:\n{}",
        summary.violations.len(),
        summary.files,
        summary.operations,
        summary.violations.join("\n")
    );
    assert!(
        summary.operations > summary.files,
        "{} operations",
        summary.operations
    );
    assert!(
        summary.unsupported.is_empty(),
        "refused: {:?}",
        summary.unsupported
    );
}

fn known_differences() -> BTreeMap<String, String> {
    fs::read_to_string(data_dir("data/u07/yq-known-differences.txt"))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once(':'))
        .map(|(file, reason)| (file.trim().to_owned(), reason.trim().to_owned()))
        .collect()
}

#[test]
fn value_model_matches_yq_output() {
    let known = known_differences();
    let mut unexpected = Vec::new();
    let mut matching_known = Vec::new();
    for (file, text) in corpus() {
        let expected_text =
            fs::read_to_string(data_dir(&format!("data/u07/yq/{file}.json"))).unwrap();
        let expected: serde_json::Value = serde_json::from_str(&expected_text).unwrap();
        let actual = XmlTree::parse(&text)
            .unwrap()
            .to_value()
            .unwrap()
            .to_plain_json();
        match (actual == expected, known.contains_key(&file)) {
            (false, false) => {
                unexpected.push(format!("{file}:\n  nurt {actual}\n  yq   {expected}"))
            }
            (true, true) => matching_known.push(file),
            _ => {}
        }
    }
    assert!(
        unexpected.is_empty(),
        "differences from yq:\n{}",
        unexpected.join("\n")
    );
    assert!(
        matching_known.is_empty(),
        "listed but equal: {matching_known:?}"
    );
}
