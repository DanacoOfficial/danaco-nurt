// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! YAML anchors and aliases.
//!
//! An alias `*name` stands for the node defined by the closest preceding
//! anchor `&name` of the same document. Reading through an alias yields the
//! value of that node; `item.anchor` names the anchor of a node and
//! `item.alias_of` the anchor an alias refers to.
//!
//! A change of a node that is reachable through aliases, whether addressed
//! directly or by a key path that passes through an alias, changes the
//! value seen through every alias. The change is made and reported with
//! the notable effect `alias_shared` and the key paths of the aliases
//! ([`shared_by`]). Replacing the alias node itself changes only that node.
//!
//! Documents are checked when they are opened ([`check_expansion`]): an
//! alias inside the node its anchor defines forms a cycle, and nested
//! aliases whose expansion exceeds [`MAX_ALIAS_EXPANSION`] nodes are refused
//! instead of being expanded.

use std::collections::{HashMap, HashSet};

use nurt_contracts::error::{Error, Result};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::report::Notable;

use super::merge;
use super::model::{NodeId, NodeKind, YamlModel};
use super::multidoc;
use crate::constructs;
use crate::edit::EditReport;

/// Construct name of an alias that refers to a node containing it.
pub const YAML_ALIAS_CYCLE: &str = "yaml.alias_cycle";

/// Construct name of a document whose aliases expand to more nodes than
/// [`MAX_ALIAS_EXPANSION`].
pub const YAML_ALIAS_EXPANSION_LIMIT: &str = "yaml.alias_expansion_limit";

/// Largest number of nodes that aliases may add to a document when it is
/// expanded; guards against exponential expansion ("billion laughs").
pub const MAX_ALIAS_EXPANSION: u64 = 1_000_000;

/// Largest number of alias key paths listed for one change.
pub const MAX_SHARED_PATHS: usize = 64;

/// Flag of a change visible through aliases.
pub const ALIAS_SHARED_FLAG: &str = "alias_shared";

/// Targets of the aliases of a stream.
#[derive(Debug, Clone, Default)]
pub struct Anchors {
    targets: HashMap<NodeId, NodeId>,
    aliases: HashMap<NodeId, Vec<NodeId>>,
}

impl Anchors {
    /// Resolves every alias of `model` to the node of its anchor.
    pub fn new(model: &YamlModel) -> Self {
        let mut defined: HashMap<(usize, &str), Vec<(usize, NodeId)>> = HashMap::new();
        for id in model.node_ids() {
            let node = model.node(id);
            if let Some(anchor) = &node.anchor {
                defined
                    .entry((node.document, anchor.name.as_str()))
                    .or_default()
                    .push((anchor.span.start, id));
            }
        }
        for list in defined.values_mut() {
            list.sort_unstable();
        }
        let mut anchors = Self::default();
        for id in model.node_ids() {
            let node = model.node(id);
            let NodeKind::Alias(name) = &node.kind else {
                continue;
            };
            let Some(list) = defined.get(&(node.document, name.name.as_str())) else {
                continue;
            };
            let before = list.partition_point(|(start, _)| *start < node.span.start);
            if let Some((_, target)) = before.checked_sub(1).and_then(|index| list.get(index)) {
                anchors.targets.insert(id, *target);
                anchors.aliases.entry(*target).or_default().push(id);
            }
        }
        anchors
    }

    /// Node an alias refers to; `None` for an alias without a preceding
    /// anchor and for a node that is not an alias.
    pub fn target(&self, alias: NodeId) -> Option<NodeId> {
        self.targets.get(&alias).copied()
    }

    /// The node itself, or the node an alias refers to; `None` for an alias
    /// without an anchor.
    pub fn resolve(&self, node: NodeId) -> Option<NodeId> {
        match self.targets.get(&node) {
            Some(target) => Some(*target),
            None => Some(node),
        }
    }

    /// Aliases that refer to the anchored node `target`, in source order.
    pub fn aliases_of(&self, target: NodeId) -> &[NodeId] {
        self.aliases.get(&target).map_or(&[], Vec::as_slice)
    }
}

/// Checks every document of `model` for alias cycles and for an expansion
/// larger than [`MAX_ALIAS_EXPANSION`] nodes.
///
/// # Errors
///
/// Returns `E_UNSUPPORTED_CONSTRUCT` `yaml.alias_cycle` for an alias inside
/// the node its anchor defines and `yaml.alias_expansion_limit` when the
/// aliases of a document expand to too many nodes.
pub fn check_expansion(model: &YamlModel) -> Result<()> {
    let anchors = Anchors::new(model);
    let mut sizes = vec![None; model.node_ids().count()];
    let mut open = vec![false; sizes.len()];
    for (index, document) in model.documents().iter().enumerate() {
        let size = expanded_size(model, &anchors, document.root, &mut sizes, &mut open)?;
        let own = model
            .node_ids()
            .filter(|&id| model.node(id).document == index)
            .count();
        let own = u64::try_from(own).unwrap_or(u64::MAX);
        if size.saturating_sub(own) > MAX_ALIAS_EXPANSION {
            return Err(expansion_error(index));
        }
    }
    Ok(())
}

fn expansion_error(document: usize) -> Error {
    constructs::unsupported(
        YAML_ALIAS_EXPANSION_LIMIT,
        format!(
            "the aliases of document {document} expand to more than {MAX_ALIAS_EXPANSION} nodes; reduce the nesting of aliases"
        ),
    )
}

/// Number of nodes of `root` with every alias replaced by its target,
/// computed without recursion; `sizes` memoizes subtrees shared by aliases.
fn expanded_size(
    model: &YamlModel,
    anchors: &Anchors,
    root: NodeId,
    sizes: &mut [Option<u64>],
    open: &mut [bool],
) -> Result<u64> {
    let mut stack = vec![(root, false)];
    while let Some((id, finished)) = stack.pop() {
        let index = id.index();
        if sizes[index].is_some() {
            continue;
        }
        let dependencies = dependencies(model, anchors, id);
        if finished {
            let size = dependencies
                .iter()
                .map(|dependency| sizes[dependency.index()].unwrap_or(0))
                .fold(own_weight(model, id), u64::saturating_add);
            if size > MAX_ALIAS_EXPANSION.saturating_mul(2) {
                return Err(expansion_error(model.node(id).document));
            }
            sizes[index] = Some(size);
            open[index] = false;
            continue;
        }
        if open[index] {
            return Err(cycle_error(model, id));
        }
        open[index] = true;
        stack.push((id, true));
        for dependency in dependencies {
            if open[dependency.index()] {
                return Err(cycle_error(model, dependency));
            }
            stack.push((dependency, false));
        }
    }
    Ok(sizes[root.index()].unwrap_or(0))
}

fn own_weight(model: &YamlModel, id: NodeId) -> u64 {
    u64::from(!matches!(model.node(id).kind, NodeKind::Alias(_)))
}

fn dependencies(model: &YamlModel, anchors: &Anchors, id: NodeId) -> Vec<NodeId> {
    match &model.node(id).kind {
        NodeKind::Alias(_) => anchors.target(id).into_iter().collect(),
        NodeKind::Mapping(mapping) => mapping
            .entries
            .iter()
            .flat_map(|entry| [entry.key, entry.value])
            .collect(),
        NodeKind::Sequence(sequence) => sequence.items.iter().map(|item| item.node).collect(),
        NodeKind::Scalar(_) => Vec::new(),
    }
}

fn cycle_error(model: &YamlModel, node: NodeId) -> Error {
    let name = model
        .node(node)
        .anchor
        .as_ref()
        .map_or_else(String::new, |anchor| anchor.name.clone());
    let (line, column) = model.line_col(model.node(node).full_span().start);
    constructs::unsupported(
        YAML_ALIAS_CYCLE,
        format!(
            "the node with anchor `&{name}` at line {line}, column {column} contains an alias of itself; replace the alias with a copy of the value"
        ),
    )
}

/// Anchor name of a node; `None` without an anchor.
pub fn anchor_of(model: &YamlModel, node: NodeId) -> Option<&str> {
    model
        .node(node)
        .anchor
        .as_ref()
        .map(|anchor| anchor.name.as_str())
}

/// Anchor an alias node refers to (`item.alias_of`); `None` for a node that
/// is not an alias.
pub fn alias_of(model: &YamlModel, node: NodeId) -> Option<&str> {
    match &model.node(node).kind {
        NodeKind::Alias(name) => Some(name.name.as_str()),
        _ => None,
    }
}

/// Key path of a node: keys of the mappings and indices of the sequences
/// from the root of its document, with the document selector.
///
/// A key of a mapping that is not a value has the path of its mapping.
pub fn path_of(model: &YamlModel, node: NodeId) -> KeyPath {
    let anchors = Anchors::new(model);
    path_with(model, &anchors, node)
}

fn path_with(model: &YamlModel, anchors: &Anchors, node: NodeId) -> KeyPath {
    let mut segments = Vec::new();
    let mut current = node;
    while let Some(parent) = model.parent(current) {
        if let Some(segment) = segment_in(model, anchors, parent, current) {
            segments.push(segment);
        } else {
            segments.clear();
        }
        current = parent;
    }
    segments.reverse();
    let document = multidoc::selector_of(model.node(node).document);
    KeyPath::new(document, segments).unwrap_or_default()
}

/// Segment that selects `child` in `parent`; `None` when `child` is a key.
fn segment_in(
    model: &YamlModel,
    anchors: &Anchors,
    parent: NodeId,
    child: NodeId,
) -> Option<KeySegment> {
    match &model.node(parent).kind {
        NodeKind::Mapping(mapping) => mapping
            .entries
            .iter()
            .find(|entry| entry.value == child)
            .and_then(|entry| merge::key_name_with(model, anchors, entry.key))
            .map(KeySegment::Key),
        NodeKind::Sequence(sequence) => sequence
            .items
            .iter()
            .position(|item| item.node == child)
            .and_then(|index| i64::try_from(index).ok())
            .map(KeySegment::Index),
        NodeKind::Scalar(_) | NodeKind::Alias(_) => None,
    }
}

/// Key paths through which a change of `node` is also visible: for `node`
/// and each of its ancestors with an anchor, the path of every alias of that
/// anchor followed by the path from the ancestor down to `node`.
///
/// An alias used as the value of a merge key `<<` makes the change visible
/// as a key of the merging mapping, unless that mapping writes the key
/// itself. Aliases inside aliased nodes are followed; no more than
/// [`MAX_SHARED_PATHS`] paths are returned, in source order of the aliases.
pub fn shared_by(model: &YamlModel, node: NodeId) -> Vec<KeyPath> {
    let anchors = Anchors::new(model);
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![(node, Vec::new())];
    while let Some((start, suffix)) = pending.pop() {
        let mut relative: Vec<KeySegment> = suffix;
        let mut current = start;
        loop {
            if model.node(current).anchor.is_some() {
                for &alias in anchors.aliases_of(current) {
                    if paths.len() >= MAX_SHARED_PATHS || !seen.insert((alias, format!("{relative:?}"))) {
                        continue;
                    }
                    if let Some(path) = visible_path(model, &anchors, alias, &relative) {
                        paths.push(path);
                        pending.push((alias, relative.clone()));
                    }
                }
            }
            let Some(parent) = model.parent(current) else {
                break;
            };
            match segment_in(model, &anchors, parent, current) {
                Some(segment) => relative.insert(0, segment),
                None => break,
            }
            current = parent;
        }
    }
    paths.sort_by_key(ToString::to_string);
    paths.dedup();
    paths
}

/// Path at which a node reached through `alias` by `relative` is seen.
fn visible_path(
    model: &YamlModel,
    anchors: &Anchors,
    alias: NodeId,
    relative: &[KeySegment],
) -> Option<KeyPath> {
    let mut merging = model.parent(alias)?;
    if model.node(merging).as_sequence().is_some() {
        let holder = model.parent(merging)?;
        if is_merge_value(model, holder, merging) {
            merging = holder;
        } else {
            return Some(join(path_with(model, anchors, alias), relative));
        }
    } else if !is_merge_value(model, merging, alias) {
        return Some(join(path_with(model, anchors, alias), relative));
    }
    let Some(KeySegment::Key(name)) = relative.first() else {
        return None;
    };
    let local = model.node(merging).as_mapping()?.entries.iter().any(|entry| {
        !entry.merge && merge::key_name_with(model, anchors, entry.key).as_deref() == Some(name)
    });
    (!local).then(|| join(path_with(model, anchors, merging), relative))
}

fn is_merge_value(model: &YamlModel, map: NodeId, value: NodeId) -> bool {
    model.node(map).as_mapping().is_some_and(|mapping| {
        mapping
            .entries
            .iter()
            .any(|entry| entry.merge && entry.value == value)
    })
}

fn join(base: KeyPath, relative: &[KeySegment]) -> KeyPath {
    let mut path = base;
    for segment in relative {
        if path.push(segment.clone()).is_err() {
            break;
        }
    }
    path
}

/// Adds the notable effect `alias_shared` with the number and the key paths
/// of the aliases to `report`; nothing for an empty list.
pub fn note_shared(report: &mut EditReport, paths: &[KeyPath]) {
    if paths.is_empty() {
        return;
    }
    let listed: Vec<String> = paths.iter().map(ToString::to_string).collect();
    report.note(
        Notable::AliasShared,
        Some(format!("also visible at {}", listed.join(", "))),
    );
    if let Some(entry) = report
        .notable
        .iter_mut()
        .find(|entry| entry.code == Notable::AliasShared)
    {
        entry.count = u64::try_from(paths.len()).ok();
    }
    report.flag(ALIAS_SHARED_FLAG);
}
