// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Planning of changes on the node tree of a property list, shared by the
//! binary, XML and ASCII forms.
//!
//! Key paths are resolved on the [`DocTree`] of the document. A key that
//! selects nodes replaces them. A key that selects nothing adds a member: a
//! dictionary key needs `create`, an array index equal to the length or
//! `[+]` appends, and with `create` missing intermediate keys become
//! dictionaries (arrays for `[0]` and `[+]`).

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::{Fields, Value};

use super::PlistEdit;
use crate::model::{DocTree, MapSemantics, NodeId, NodeKind};

/// Rules of key paths in property lists: a repeated dictionary key selects
/// the last entry, as the `plist` crate reads it.
pub(crate) const SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// One change of the node tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Op {
    /// Replace the value of a node.
    Replace {
        /// The node.
        node: NodeId,
        /// New value.
        value: Value,
    },
    /// Add a key to a dictionary.
    Insert {
        /// The dictionary.
        map: NodeId,
        /// New key.
        name: String,
        /// Its value.
        value: Value,
    },
    /// Append to an array.
    Append {
        /// The array.
        list: NodeId,
        /// New element.
        value: Value,
    },
}

/// Changes that store `value` at `key`.
///
/// # Errors
///
/// Returns `E_KEY_NOT_FOUND` for a missing key without `create`,
/// `E_KEY_CONFLICT` for a path through a node of another kind and the
/// errors of [`DocTree::resolve`].
pub(crate) fn plan_set(tree: &DocTree, key: &KeyPath, value: &Value, create: bool) -> Result<Vec<Op>, Error> {
    if key.ends_with_append() {
        let parent = key.parent().unwrap_or_else(KeyPath::root);
        let lists = tree.resolve(&parent, &SEMANTICS)?;
        if lists.is_empty() {
            return missing(tree, key, value, create);
        }
        return lists
            .into_iter()
            .map(|found| match tree[found.node].kind {
                NodeKind::List => Ok(Op::Append { list: found.node, value: value.clone() }),
                kind => Err(conflict(key, &format!("[+] appends to an array, and {} is a {}", found.key, kind.as_str()))),
            })
            .collect();
    }
    let found = tree.resolve(key, &SEMANTICS)?;
    if found.is_empty() {
        return missing(tree, key, value, create);
    }
    Ok(found.into_iter().map(|found| Op::Replace { node: found.node, value: value.clone() }).collect())
}

/// Change for a concrete key whose last segments do not exist.
fn missing(tree: &DocTree, key: &KeyPath, value: &Value, create: bool) -> Result<Vec<Op>, Error> {
    let segments = key.segments();
    if !segments.iter().all(|segment| matches!(segment, KeySegment::Key(_) | KeySegment::Index(_) | KeySegment::Append)) {
        return Err(not_found(key, "the key selects no value"));
    }
    let (parent, depth) = deepest_existing(tree, key)?;
    let rest = &segments[depth..];
    let Some((first, tail)) = rest.split_first() else {
        return Err(not_found(key, "the key selects no value"));
    };
    if !tail.is_empty() && !create {
        return Err(not_found(key, "intermediate keys are missing; pass create: true to add them"));
    }
    let nested = nest(key, tail, value)?;
    let node = &tree[parent];
    match (node.kind, first) {
        (NodeKind::Map, KeySegment::Key(name)) if create => {
            Ok(vec![Op::Insert { map: parent, name: name.clone(), value: nested }])
        }
        (NodeKind::Map, KeySegment::Key(_)) => Err(not_found(key, "the key is missing; pass create: true to add it")),
        (NodeKind::List, KeySegment::Append) => Ok(vec![Op::Append { list: parent, value: nested }]),
        (NodeKind::List, KeySegment::Index(index)) if usize::try_from(*index).ok() == Some(node.children.len()) => {
            Ok(vec![Op::Append { list: parent, value: nested }])
        }
        (NodeKind::List, KeySegment::Index(_)) => Err(not_found(
            key,
            &format!("the array has {} elements; append with [+]", node.children.len()),
        )),
        (kind, _) => Err(conflict(key, &format!("the key passes through a {}", kind.as_str()))),
    }
}

/// Deepest existing node on the path of `key` and the number of segments
/// that lead to it.
fn deepest_existing(tree: &DocTree, key: &KeyPath) -> Result<(NodeId, usize), Error> {
    let segments = key.segments();
    for depth in (0..segments.len()).rev() {
        let prefix = KeyPath::new(key.document(), segments[..depth].to_vec())
            .map_err(|error| conflict(key, &error.to_string()))?;
        if let Some(found) = tree.resolve(&prefix, &SEMANTICS)?.first() {
            return Ok((found.node, depth));
        }
    }
    Err(not_found(key, "the document has no value"))
}

/// Value created for missing segments below the deepest existing node.
fn nest(key: &KeyPath, tail: &[KeySegment], value: &Value) -> Result<Value, Error> {
    tail.iter().rev().try_fold(value.clone(), |inner, segment| match segment {
        KeySegment::Key(name) => {
            let mut fields = Fields::new();
            fields.insert(name.clone(), inner);
            Ok(Value::Map(fields))
        }
        KeySegment::Index(0) | KeySegment::Append => Ok(Value::List(vec![inner])),
        _ => Err(not_found(key, "a missing array can be created only with [0] or [+]")),
    })
}

/// Nodes removed by `key`.
///
/// # Errors
///
/// Returns `E_KEY_NOT_FOUND` when the key selects nothing and
/// `E_KEY_CONFLICT` for the root value.
pub(crate) fn plan_remove(tree: &DocTree, key: &KeyPath) -> Result<Vec<NodeId>, Error> {
    let found = tree.resolve(key, &SEMANTICS)?;
    if found.is_empty() {
        return Err(not_found(key, "the key selects no value"));
    }
    found
        .into_iter()
        .map(|found| match tree[found.node].parent {
            Some(_) => Ok(found.node),
            None => Err(conflict(key, "the root value cannot be removed; set it to an empty dictionary instead")),
        })
        .collect()
}

/// Text form of a property list: the change of one node in the source text.
pub(crate) trait TextSyntax {
    /// Replacement of the value of `node`, or `None` when it already holds
    /// `value`.
    ///
    /// # Errors
    ///
    /// Returns `E_TYPE` for a value the form cannot hold.
    fn replace(&self, text: &str, tree: &DocTree, node: NodeId, value: &Value) -> Result<Option<PlistEdit>, Error>;

    /// Insertion of `name` with `value` into the dictionary `map`.
    ///
    /// # Errors
    ///
    /// As for [`TextSyntax::replace`].
    fn insert(&self, text: &str, tree: &DocTree, map: NodeId, name: &str, value: &Value) -> Result<PlistEdit, Error>;

    /// Insertion of `value` at the end of the array `list`.
    ///
    /// # Errors
    ///
    /// As for [`TextSyntax::replace`].
    fn append(&self, text: &str, tree: &DocTree, list: NodeId, value: &Value) -> Result<PlistEdit, Error>;

    /// Bytes removed with the member `node`.
    fn removal(&self, text: &str, tree: &DocTree, node: NodeId) -> Range<usize>;
}

/// Replacements of the source text for `ops`.
///
/// # Errors
///
/// Returns the errors of the syntax and `E_KEY_CONFLICT` for changes that
/// overlap.
pub(crate) fn text_set(syntax: &dyn TextSyntax, text: &str, tree: &DocTree, key: &KeyPath, ops: &[Op]) -> Result<Vec<PlistEdit>, Error> {
    let mut edits = Vec::new();
    for op in ops {
        let edit = match op {
            Op::Replace { node, value } => syntax.replace(text, tree, *node, value)?,
            Op::Insert { map, name, value } => Some(syntax.insert(text, tree, *map, name, value)?),
            Op::Append { list, value } => Some(syntax.append(text, tree, *list, value)?),
        };
        edits.extend(edit);
    }
    order(key, edits)
}

/// Removals of the source text for `nodes`.
///
/// # Errors
///
/// Returns `E_KEY_CONFLICT` for removals that overlap other changes.
pub(crate) fn text_remove(syntax: &dyn TextSyntax, text: &str, tree: &DocTree, key: &KeyPath, nodes: &[NodeId]) -> Result<Vec<PlistEdit>, Error> {
    let edits = nodes
        .iter()
        .map(|node| PlistEdit { range: syntax.removal(text, tree, *node), replacement: Vec::new() })
        .collect();
    order(key, edits)
}

/// Sorts replacements; overlapping removals are merged, other overlaps are a
/// conflict.
fn order(key: &KeyPath, mut edits: Vec<PlistEdit>) -> Result<Vec<PlistEdit>, Error> {
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    let mut ordered: Vec<PlistEdit> = Vec::with_capacity(edits.len());
    for edit in edits {
        if let Some(last) = ordered.last_mut() {
            let overlaps = edit.range.start < last.range.end
                || (edit.range.start == last.range.start && edit.range.is_empty() && last.range.is_empty());
            if overlaps {
                if !(edit.replacement.is_empty() && last.replacement.is_empty()) {
                    return Err(conflict(key, "the key selects values that contain each other; select them separately"));
                }
                last.range.end = last.range.end.max(edit.range.end);
                continue;
            }
        }
        ordered.push(edit);
    }
    Ok(ordered)
}

/// Next sibling of `node`, if any.
pub(crate) fn next_sibling(tree: &DocTree, node: NodeId) -> Option<NodeId> {
    let parent = tree[node].parent?;
    let siblings = &tree[parent].children;
    let index = siblings.iter().position(|sibling| *sibling == node)?;
    siblings.get(index + 1).copied()
}

/// Previous sibling of `node`, if any.
pub(crate) fn previous_sibling(tree: &DocTree, node: NodeId) -> Option<NodeId> {
    let parent = tree[node].parent?;
    let siblings = &tree[parent].children;
    let index = siblings.iter().position(|sibling| *sibling == node)?;
    index.checked_sub(1).map(|index| siblings[index])
}

fn not_found(key: &KeyPath, problem: &str) -> Error {
    Error::new(ErrorCode::KeyNotFound, format!("key {key} not found: {problem}")).with_found(key.to_string())
}

fn conflict(key: &KeyPath, problem: &str) -> Error {
    Error::new(ErrorCode::KeyConflict, format!("key {key} cannot be changed: {problem}")).with_found(key.to_string())
}
