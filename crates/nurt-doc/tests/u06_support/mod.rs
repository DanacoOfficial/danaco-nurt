// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u06_support/mod.rs
// Responsibility:   Helpers of the YAML tests for sequences, anchors, merge keys, documents, comments and tags.
// Dependencies:     nurt_contracts, saphyr, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Helpers of the YAML tests for sequences, anchors, merge keys, documents,
//! comments and tags.

#![allow(
    dead_code,
    reason = "each test file uses a different subset of the helpers"
)]
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers fail the test on unexpected input"
)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_doc::edit::{ByteEdit, apply};
use nurt_doc::formats::yaml::anchor::Anchors;
use nurt_doc::formats::yaml::merge;
use nurt_doc::formats::yaml::model::{NodeId, NodeKind, YamlModel};
use saphyr::{LoadableYamlNode, Scalar, Yaml};
use serde_json::Value as Json;

/// Parses `text`, failing the test on error.
pub(crate) fn parse(text: &str) -> YamlModel {
    YamlModel::parse(text).unwrap_or_else(|error| panic!("parse failed: {error:?}\n{text}"))
}

/// Text after applying `edits` to the source of `model`.
pub(crate) fn apply_edits(model: &YamlModel, edits: &[ByteEdit]) -> String {
    let applied = apply(model.source().as_bytes(), edits).expect("edits apply");
    String::from_utf8(applied.bytes).expect("edits keep UTF-8")
}

/// Node selected by a concrete key path: aliases are followed between
/// segments and merged keys are found through their merge keys.
pub(crate) fn node_at(model: &YamlModel, path: &str) -> NodeId {
    try_node_at(model, &KeyPath::parse(path).expect("key path"))
        .unwrap_or_else(|| panic!("no node at `{path}`"))
}

/// Node selected by `key`, or `None`.
pub(crate) fn try_node_at(model: &YamlModel, key: &KeyPath) -> Option<NodeId> {
    let document = match key.document() {
        None => 0,
        Some(DocSelector::Index(index)) => usize::try_from(index).ok()?,
        Some(_) => return None,
    };
    let anchors = Anchors::new(model);
    let mut node = model.documents().get(document)?.root;
    for segment in key.segments() {
        node = anchors.resolve(node)?;
        node = match segment {
            KeySegment::Key(name) => merge::local_entry(model, node, name)
                .map(|entry| entry.value)
                .or_else(|| merge::inherited_value(model, node, name))?,
            KeySegment::Index(index) => {
                let items = &model.node(node).as_sequence()?.items;
                let position = if *index < 0 {
                    items
                        .len()
                        .checked_sub(usize::try_from(index.unsigned_abs()).ok()?)?
                } else {
                    usize::try_from(*index).ok()?
                };
                items.get(position)?.node
            }
            _ => return None,
        };
    }
    Some(node)
}

/// Returns `true` when `node` or one of its ancestors is the target of an
/// alias, so that a change of `node` is visible elsewhere.
pub(crate) fn reachable_by_alias(model: &YamlModel, anchors: &Anchors, node: NodeId) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if !anchors.aliases_of(id).is_empty() {
            return true;
        }
        current = model.parent(id);
    }
    false
}

/// Returns `true` when `node` or a node inside it defines an anchor that an
/// alias refers to, so that removing `node` leaves the alias undefined.
pub(crate) fn defines_used_anchor(model: &YamlModel, anchors: &Anchors, node: NodeId) -> bool {
    let span = model.node(node).full_span();
    model.node_ids().any(|id| {
        let inner = model.node(id).full_span();
        inner.start >= span.start && inner.end <= span.end && !anchors.aliases_of(id).is_empty()
    })
}

/// Returns `true` when `node` lies inside the key of a mapping entry.
pub(crate) fn inside_key(model: &YamlModel, node: NodeId) -> bool {
    let mut child = node;
    while let Some(parent) = model.parent(child) {
        if let NodeKind::Mapping(mapping) = &model.node(parent).kind
            && mapping.entries.iter().any(|entry| entry.key == child)
        {
            return true;
        }
        child = parent;
    }
    false
}

/// Documents of `text` read by the reference parser, as JSON values.
pub(crate) fn reference(text: &str) -> Result<Vec<Json>, String> {
    Yaml::load_from_str(text)
        .map(|documents| documents.iter().map(json_of).collect())
        .map_err(|error| error.to_string())
}

/// JSON form of a value of the reference parser; mapping keys become text.
pub(crate) fn json_of(yaml: &Yaml<'_>) -> Json {
    match yaml {
        Yaml::Value(scalar) => scalar_json(scalar),
        Yaml::Representation(text, _, _) => Json::String(text.to_string()),
        Yaml::Sequence(items) => Json::Array(items.iter().map(json_of).collect()),
        Yaml::Mapping(mapping) => Json::Object(
            mapping
                .iter()
                .map(|(key, value)| (key_text(key), json_of(value)))
                .collect(),
        ),
        Yaml::Tagged(_, inner) => json_of(inner),
        _ => Json::Null,
    }
}

fn scalar_json(scalar: &Scalar<'_>) -> Json {
    match scalar {
        Scalar::Null => Json::Null,
        Scalar::Boolean(flag) => Json::Bool(*flag),
        Scalar::Integer(number) => Json::from(*number),
        Scalar::FloatingPoint(number) => Json::from(number.into_inner()),
        Scalar::String(text) => Json::String(text.to_string()),
    }
}

fn key_text(key: &Yaml<'_>) -> String {
    match json_of(key) {
        Json::String(text) => text,
        Json::Null => "null".to_owned(),
        other => other.to_string(),
    }
}

/// Value at `key` inside the documents `documents`, for changes.
pub(crate) fn json_at<'a>(documents: &'a mut [Json], key: &KeyPath) -> Option<&'a mut Json> {
    let document = match key.document() {
        None => 0,
        Some(DocSelector::Index(index)) => usize::try_from(index).ok()?,
        Some(_) => return None,
    };
    let mut current = documents.get_mut(document)?;
    for segment in key.segments() {
        current = match segment {
            KeySegment::Key(name) => current.as_object_mut()?.get_mut(name)?,
            KeySegment::Index(index) => current
                .as_array_mut()?
                .get_mut(usize::try_from(*index).ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Repository directory of the advanced YAML corpus.
pub(crate) fn corpus_dir() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/corpus/yaml/advanced")
}

/// Every `.yaml` and `.yml` file below `dir`, sorted.
pub(crate) fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("corpus directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "yaml" || extension == "yml")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Identifiers of the valid YAML Test Suite cases that use the constructs
/// of this unit (sequences, anchors, aliases, documents, comments, tags,
/// explicit keys), from `tests/d5/data/u06/yaml-test-suite-cases.tsv`.
pub(crate) fn suite_case_ids() -> Vec<String> {
    let list = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/data/u06/yaml-test-suite-cases.tsv");
    let text = fs::read_to_string(list).expect("case list");
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.starts_with("case\t"))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?;
            (fields.next()? == "valid").then(|| id.to_owned())
        })
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
