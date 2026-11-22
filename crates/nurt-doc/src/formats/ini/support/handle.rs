// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Document handle shared by the TOML, INI, property and environment
//! formats.
//!
//! The document of a format makes a change on a copy of itself. The handle
//! then passes the difference between the old and the new bytes through
//! [`guard::commit`]: the edits must lie in the lines of the changed nodes
//! (or, for a created key, of its nearest existing ancestor), and the
//! reference reader of the format must read the new bytes as the value of
//! the old document with the change applied. Only then does the copy
//! replace the document.

use std::cmp::Reverse;

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Fields, Value};

use super::flat::scalar_text;
use super::tree::Node;
use crate::constructs::{self, DOC_SEMANTIC_MISMATCH};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser};
use crate::edit::{EditReport, diff};
use crate::model::{
    DocHandle, DocTree, FormatSemantics, NodeId, NodeKind, NodeSpec, SetOptions, Span, TreeBuilder,
    doc_parse, key_not_found,
};

/// Flag of a change that created keys.
const KEYS_CREATED_FLAG: &str = "keys_created";

/// Parsed document of a format served by [`KeyValueHandle`].
pub(crate) trait Backend: Clone + Send + Sync + 'static {
    /// Format of the document.
    fn format(&self) -> DocFormatId;

    /// Bytes of the document in the encoding of its file.
    fn bytes(&self) -> Vec<u8>;

    /// Node tree over `bytes`, the result of [`Backend::bytes`].
    fn tree(&self, bytes: &[u8]) -> DocTree;

    /// Key path rules of the document.
    fn semantics(&self) -> &'static dyn FormatSemantics;

    /// Sets the value at `key`; returns the number of values set.
    fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize>;

    /// Removes the values selected by `key`; returns their number.
    fn remove(&mut self, key: &KeyPath) -> Result<usize>;

    /// Reads `bytes` with the settings of this document, independently of
    /// its parsed state, into one value per document of the file.
    fn reference(&self, bytes: &[u8]) -> Result<Vec<Value>>;

    /// Returns `true` when every scalar is stored as text, so that a value
    /// set is read back as its text.
    fn text_values(&self) -> bool {
        false
    }

    /// Spans, besides the selected nodes, that a removal of `key` may
    /// change: earlier occurrences of a repeated flat key.
    fn removal_spans(&self, _key: &KeyPath) -> Vec<Span> {
        Vec::new()
    }
}

/// Reference reader of a [`Backend`] for the meaning guard.
struct Reference<'a, B>(&'a B);

impl<B: Backend> ReferenceParser for Reference<'_, B> {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        self.0.reference(text)
    }
}

/// Open document of a key-value format.
#[derive(Debug, Clone)]
pub(crate) struct KeyValueHandle<B> {
    document: B,
    bytes: Vec<u8>,
    tree: DocTree,
}

impl<B: Backend> KeyValueHandle<B> {
    /// Handle of a parsed document.
    pub(crate) fn new(document: B) -> Self {
        let bytes = document.bytes();
        let tree = document.tree(&bytes);
        Self {
            document,
            bytes,
            tree,
        }
    }

    fn expectation(&self) -> Expectation<'static> {
        let semantics = self.document.semantics();
        Expectation::new(tree_values(&self.tree, semantics), semantics)
    }

    /// Concrete keys selected by `key` with their nodes; a key of names,
    /// indexes and a final `[+]` that selects nothing is kept without a node
    /// when `keep_missing` is set.
    fn concrete(
        &self,
        key: &KeyPath,
        keep_missing: bool,
    ) -> Result<Vec<(KeyPath, Option<NodeId>)>> {
        match self.tree.resolve(key, self.document.semantics()) {
            Ok(found) if !found.is_empty() => {
                let mut keys: Vec<(KeyPath, Option<NodeId>)> = Vec::new();
                for resolved in found {
                    if !keys.iter().any(|(known, _)| *known == resolved.key) {
                        keys.push((resolved.key, Some(resolved.node)));
                    }
                }
                Ok(keys)
            }
            _ if keep_missing && is_direct(key) => Ok(vec![(key.clone(), None)]),
            Ok(_) => Err(key_not_found(key, &[])),
            Err(error) => Err(error),
        }
    }

    /// Lines of the node, or of the nearest existing ancestor of a missing
    /// key.
    fn region(&self, key: &KeyPath, node: Option<NodeId>) -> Span {
        let span = node
            .or_else(|| self.nearest(key))
            .map_or(Span::new(0, self.bytes.len()), |id| self.tree[id].span);
        lines_of(&self.bytes, span)
    }

    fn nearest(&self, key: &KeyPath) -> Option<NodeId> {
        let segments = key.segments();
        (0..segments.len()).rev().find_map(|length| {
            let prefix = KeyPath::new(key.document(), segments[..length].to_vec()).ok()?;
            let found = self.tree.resolve(&prefix, self.document.semantics()).ok()?;
            found.first().map(|resolved| resolved.node)
        })
    }

    /// Returns `true` when an ancestor of `node` is in `selected`.
    fn below_any(&self, node: NodeId, selected: &[NodeId]) -> bool {
        let mut current = self.tree[node].parent;
        while let Some(parent) = current {
            if selected.contains(&parent) {
                return true;
            }
            current = self.tree[parent].parent;
        }
        false
    }

    /// Checks the change made on `changed` with the guards and takes it.
    fn commit(
        &mut self,
        changed: B,
        targets: Vec<Span>,
        count: usize,
        expectation: &Expectation<'_>,
        mut report: EditReport,
    ) -> Result<EditReport> {
        let after = changed.bytes();
        let mut plan = EditPlan::new();
        for target in targets {
            plan.target(target);
        }
        for edit in diff(&self.bytes, &after) {
            plan.edit(edit);
        }
        report.replacements = u64::try_from(count).unwrap_or(u64::MAX);
        plan.report = report;
        let committed = guard::commit(&self.bytes, plan, expectation, &Reference(&changed))?;
        if !committed.report.is_unchanged() {
            *self = Self::new(changed);
        }
        Ok(committed.report)
    }
}

impl<B: Backend + std::fmt::Debug> DocHandle for KeyValueHandle<B> {
    fn format(&self) -> DocFormatId {
        self.document.format()
    }

    fn text(&self) -> &[u8] {
        &self.bytes
    }

    fn tree(&self) -> &DocTree {
        &self.tree
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        self.document.semantics()
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        let keys = self.concrete(key, options.create)?;
        let semantics = self.document.semantics();
        let current = |node: Option<NodeId>| node.map(|id| self.tree.value_of(id, semantics, None));
        if keys
            .iter()
            .all(|(_, node)| current(*node).as_ref() == Some(value))
        {
            return Ok(EditReport::default());
        }
        let targets = keys
            .iter()
            .map(|(concrete, node)| self.region(concrete, *node))
            .collect();
        let mut report = EditReport::default();
        if let Some((created, _)) = keys.iter().find(|(_, node)| node.is_none()) {
            report.note(Notable::KeysCreated, Some(created.to_string()));
            report.flag(KEYS_CREATED_FLAG);
        }
        let mut changed = self.document.clone();
        let count = changed.set(key, value, options.create)?;
        let mut expectation = self.expectation();
        if self.document.text_values() {
            let after = changed.bytes();
            let tree = changed.tree(&after);
            for (concrete, _) in keys {
                let read = read_back(&tree, changed.semantics(), &concrete, value)?;
                expectation = expectation.set(concrete, read);
            }
        } else {
            for (concrete, _) in keys {
                expectation = expectation.set(concrete, value.clone());
            }
        }
        self.commit(changed, targets, count, &expectation, report)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let mut selected: Vec<(KeyPath, NodeId)> = self
            .concrete(key, false)?
            .into_iter()
            .filter_map(|(concrete, node)| node.map(|node| (concrete, node)))
            .collect();
        let nodes: Vec<NodeId> = selected.iter().map(|(_, node)| *node).collect();
        selected.retain(|(_, node)| !self.below_any(*node, &nodes));
        selected.sort_by_key(|(_, node)| Reverse(*node));
        let mut targets: Vec<Span> = selected
            .iter()
            .map(|(_, node)| lines_of(&self.bytes, self.tree[*node].span))
            .collect();
        for (concrete, _) in &selected {
            targets.extend(self.document.removal_spans(concrete));
        }
        let mut changed = self.document.clone();
        let count = changed.remove(key)?;
        let mut expectation = self.expectation();
        for (concrete, _) in selected {
            expectation = expectation.remove(concrete);
        }
        self.commit(changed, targets, count, &expectation, EditReport::default())
    }
}

/// Returns `true` for a key of names, indexes and a final `[+]` in one
/// document.
fn is_direct(key: &KeyPath) -> bool {
    matches!(key.document(), None | Some(DocSelector::Index(_)))
        && key.segments().iter().all(|segment| {
            matches!(
                segment,
                KeySegment::Key(_) | KeySegment::Index(_) | KeySegment::Append
            )
        })
}

/// Value read back at `key` after a change of a text-valued document; it
/// must have the text of `value`.
fn read_back(
    tree: &DocTree,
    semantics: &dyn FormatSemantics,
    key: &KeyPath,
    value: &Value,
) -> Result<Value> {
    let Some(node) = tree
        .resolve(key, semantics)
        .ok()
        .and_then(|found| found.first().map(|resolved| resolved.node))
    else {
        return Ok(as_text(value));
    };
    let read = tree.value_of(node, semantics, None);
    if same_text(&read, value) {
        Ok(read)
    } else {
        Err(constructs::unsupported(
            DOC_SEMANTIC_MISMATCH,
            format!("the change of `{key}` would read back with another text; nothing was changed"),
        ))
    }
}

/// `value` with every scalar replaced by its text.
fn as_text(value: &Value) -> Value {
    match value {
        Value::List(items) => Value::List(items.iter().map(as_text).collect()),
        Value::Map(fields) => Value::Map(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), as_text(item)))
                .collect::<Fields>(),
        ),
        scalar => scalar_text(scalar).map_or_else(|| scalar.clone(), Value::Text),
    }
}

/// Returns `true` when two values have the same structure and the same
/// text in every scalar.
fn same_text(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::List(left), Value::List(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(l, r)| same_text(l, r))
        }
        (Value::Map(left), Value::Map(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(name, item)| right.get(name).is_some_and(|other| same_text(item, other)))
        }
        _ => scalar_text(left).is_some_and(|text| scalar_text(right) == Some(text)),
    }
}

/// Value of every document of the tree.
pub(crate) fn tree_values(tree: &DocTree, semantics: &dyn FormatSemantics) -> Vec<Value> {
    tree.roots()
        .iter()
        .map(|root| tree.value_of(*root, semantics, None))
        .collect()
}

/// Values of a document parsed again by its own parser, for formats
/// without an independent reader.
///
/// # Errors
///
/// The error of `parsed`.
pub(crate) fn reparsed<B: Backend>(parsed: Result<B>) -> Result<Vec<Value>> {
    let document = parsed?;
    let bytes = document.bytes();
    let tree = document.tree(&bytes);
    Ok(tree_values(&tree, document.semantics()))
}

/// Text of a document that must be UTF-8.
///
/// # Errors
///
/// Fails with `E_DOC_PARSE` at the first byte that is not UTF-8.
pub(crate) fn utf8_text(content: Vec<u8>, format: &DocFormatId) -> Result<String> {
    String::from_utf8(content).map_err(|error| {
        let offset = error.utf8_error().valid_up_to();
        doc_parse(
            format,
            error.as_bytes(),
            Some(offset),
            "the text is not valid UTF-8; save the file as UTF-8",
        )
    })
}

/// Builds the node tree of a key tree.
///
/// `describe` gives the spans, value and comment of a node from its key
/// path in the key tree; the kind and the name are set here. Members of a
/// repeated key become siblings sharing the name.
pub(crate) fn build_tree(
    bytes: &[u8],
    root: &Node,
    describe: &dyn Fn(&Node, &[KeySegment]) -> NodeSpec,
) -> DocTree {
    let mut builder = TreeBuilder::new(bytes);
    let mut path = Vec::new();
    add_node(&mut builder, None, None, root, &mut path, describe);
    builder.finish()
}

fn add_node(
    builder: &mut TreeBuilder<'_>,
    parent: Option<NodeId>,
    name: Option<&str>,
    node: &Node,
    path: &mut Vec<KeySegment>,
    describe: &dyn Fn(&Node, &[KeySegment]) -> NodeSpec,
) {
    let mut spec = describe(node, path);
    spec.kind = Some(match node {
        Node::Leaf(_) => NodeKind::Scalar,
        Node::Map(_) => NodeKind::Map,
        Node::List(_) | Node::Repeated(_) => NodeKind::List,
    });
    if let Some(name) = name {
        spec = spec.with_name(name);
    }
    let id = builder.node(parent, spec);
    match node {
        Node::Leaf(_) => {}
        Node::Map(map) => {
            for (member, child) in &map.members {
                path.push(KeySegment::Key(member.clone()));
                if let Node::Repeated(elements) = child {
                    for (index, element) in elements.iter().enumerate() {
                        path.push(KeySegment::Index(to_index(index)));
                        add_node(builder, Some(id), Some(member), element, path, describe);
                        path.pop();
                    }
                } else {
                    add_node(builder, Some(id), Some(member), child, path, describe);
                }
                path.pop();
            }
        }
        Node::List(elements) | Node::Repeated(elements) => {
            for (index, element) in elements.iter().enumerate() {
                path.push(KeySegment::Index(to_index(index)));
                add_node(builder, Some(id), None, element, path, describe);
                path.pop();
            }
        }
    }
}

fn to_index(position: usize) -> i64 {
    i64::try_from(position).unwrap_or(i64::MAX)
}

/// `span` widened to whole lines, with the line break of the last line.
pub(crate) fn lines_of(text: &[u8], span: Span) -> Span {
    let start = text[..span.start.min(text.len())]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let end = text[span.end.min(text.len())..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(text.len(), |at| span.end + at + 1);
    Span::new(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widens_spans_to_whole_lines() {
        let text = b"a = 1\nb = 2\nc = 3";
        assert_eq!(lines_of(text, Span::new(8, 9)), Span::new(6, 12));
        assert_eq!(lines_of(text, Span::new(14, 15)), Span::new(12, 17));
    }

    #[test]
    fn compares_values_by_their_text() {
        assert!(same_text(&Value::Text("1".into()), &Value::Int(1)));
        assert!(!same_text(&Value::Text("1".into()), &Value::Int(2)));
        let list = Value::List(vec![Value::Bool(true)]);
        assert!(same_text(
            &list,
            &Value::List(vec![Value::Text("true".into())])
        ));
        assert_eq!(
            as_text(&list),
            Value::List(vec![Value::Text("true".into())])
        );
        assert!(!same_text(&list, &Value::Text("true".into())));
    }

    #[test]
    fn direct_keys_have_only_names_indexes_and_append() {
        assert!(is_direct(&KeyPath::parse("a.b[0][+]").unwrap()));
        assert!(!is_direct(&KeyPath::parse("a.*").unwrap()));
    }
}
