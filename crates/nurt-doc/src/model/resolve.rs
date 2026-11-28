// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::model::resolve
// Responsibility:   Node selected by a key path.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::collections::HashSet;

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{DocSelector, FilterExpr, KeyPath, KeySegment};

use super::errors::{MAX_CANDIDATES, ambiguous_key, key_conflict};
use super::filter;
use super::semantics::{FormatSemantics, Repeated};
use super::{DocTree, NodeId, NodeKind};

/// Node selected by a key path.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The node; an alias node is returned as such, its value is the value
    /// of the target.
    pub node: NodeId,
    /// Concrete key path of the node in canonical form.
    pub key: KeyPath,
}

/// Members selected by one step: one node, or several members sharing a name
/// under [`Repeated::Conflict`].
type Group = Vec<NodeId>;

struct Walk<'a> {
    tree: &'a DocTree,
    semantics: &'a dyn FormatSemantics,
    key: &'a KeyPath,
    /// No wildcard, recursion or filter so far: a mismatch of kinds is an
    /// error rather than an empty selection.
    concrete: bool,
}

impl DocTree {
    /// Nodes selected by `key`, in document order of selection, without
    /// duplicates.
    ///
    /// A key that selects nothing returns an empty list. A name segment
    /// selects a member of a map or element, or an element of a list when the
    /// name is made of digits; an index selects an element of a list or, for
    /// repeated names, a member of the group. Paths through aliases continue
    /// at the alias target. The append marker `[+]` selects nothing.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_CONFLICT` when a concrete path passes through a node of another
    /// kind (a name segment on a scalar, an index on a map) or, under
    /// [`Repeated::Conflict`], when a name without index matches several
    /// members; the error lists the concrete paths of the members as
    /// candidates.
    pub fn resolve(&self, key: &KeyPath, semantics: &dyn FormatSemantics) -> Result<Vec<Resolved>> {
        let mut walk = Walk {
            tree: self,
            semantics,
            key,
            concrete: true,
        };
        let mut groups = walk.documents();
        let segments = key.segments();
        let mut index = 0;
        while let Some(segment) = segments.get(index) {
            let consumed = walk.width(&segments[index..]);
            let mut next = Vec::new();
            for group in groups {
                walk.step(&group, &segments[index..index + consumed], &mut next)?;
            }
            if !matches!(segment, KeySegment::Key(_) | KeySegment::Index(_)) {
                walk.concrete = false;
            }
            groups = next;
            index += consumed;
        }
        walk.finish(groups)
    }

    /// Existing keys close to `key`, best first, for `E_KEY_NOT_FOUND`.
    ///
    /// The longest prefix of `key` that selects a node is resolved; the
    /// members of that node are ranked by the edit distance between their
    /// names and the next segment of `key`.
    #[must_use]
    pub fn similar_keys(&self, key: &KeyPath, semantics: &dyn FormatSemantics) -> Vec<KeyPath> {
        let mut prefix = key.clone();
        let mut wanted = String::new();
        while let Some(parent) = prefix.parent() {
            if let Some(KeySegment::Key(name)) = prefix.segments().last() {
                wanted.clone_from(name);
            }
            prefix = parent;
            let Ok(found) = self.resolve(&prefix, semantics) else {
                continue;
            };
            let Some(first) = found.first() else {
                continue;
            };
            return self.rank_members(self.target(first.node), &wanted, semantics);
        }
        Vec::new()
    }

    fn rank_members(
        &self,
        node: NodeId,
        wanted: &str,
        semantics: &dyn FormatSemantics,
    ) -> Vec<KeyPath> {
        let mut ranked: Vec<(usize, NodeId)> = self
            .members(node)
            .filter_map(|member| {
                let name = self[member].name.as_deref()?;
                Some((edit_distance(name, wanted), member))
            })
            .collect();
        ranked.sort_by_key(|(distance, _)| *distance);
        ranked
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|(_, member)| self.path_of(member, semantics))
            .collect()
    }
}

impl Walk<'_> {
    fn documents(&self) -> Vec<Group> {
        let roots = self.tree.roots();
        match self.key.document() {
            None => roots
                .first()
                .map(|root| vec![vec![*root]])
                .unwrap_or_default(),
            Some(DocSelector::Index(index)) => usize::try_from(index)
                .ok()
                .and_then(|index| roots.get(index))
                .map(|root| vec![vec![*root]])
                .unwrap_or_default(),
            Some(DocSelector::All) => roots.iter().map(|root| vec![*root]).collect(),
        }
    }

    /// Number of segments consumed by the next step: consecutive names form
    /// one flat key in flat formats.
    fn width(&self, segments: &[KeySegment]) -> usize {
        if !self.semantics.flat_keys() {
            return 1;
        }
        segments
            .iter()
            .take_while(|segment| matches!(segment, KeySegment::Key(_)))
            .count()
            .max(1)
    }

    fn step(&self, group: &[NodeId], segments: &[KeySegment], next: &mut Vec<Group>) -> Result<()> {
        match segments.first() {
            Some(KeySegment::Key(_)) => {
                let name = joined_name(segments);
                for node in self.descend(group)? {
                    self.member(node, &name, next)?;
                }
            }
            Some(KeySegment::Index(index)) => self.index(group, *index, next)?,
            Some(KeySegment::Wildcard) => self.wildcard(group, next),
            Some(KeySegment::Recursive) => {
                let mut seen = HashSet::new();
                for node in self.descend(group)? {
                    self.descendants(node, &mut seen, next);
                }
            }
            Some(KeySegment::Filter(expr)) => self.filter(group, expr, next),
            _ => {}
        }
        Ok(())
    }

    /// Nodes of a group to continue from: the single node, the last member
    /// under [`Repeated::LastWins`], every member in a non-concrete path.
    fn descend(&self, group: &[NodeId]) -> Result<Vec<NodeId>> {
        if group.len() <= 1 || self.semantics.repeated() == Repeated::LastWins {
            return Ok(group.last().copied().into_iter().collect());
        }
        if self.concrete {
            return Err(self.ambiguous(group));
        }
        Ok(group.to_vec())
    }

    fn member(&self, node: NodeId, name: &str, next: &mut Vec<Group>) -> Result<()> {
        let target = self.tree.target(node);
        let kind = self.tree[target].kind;
        if kind == NodeKind::List
            && !name.is_empty()
            && name.bytes().all(|b| b.is_ascii_digit())
            && let Ok(position) = name.parse::<i64>()
        {
            return self.index(&[node], position, next);
        }
        if !kind.is_named_container() {
            if self.concrete {
                return Err(key_conflict(
                    self.key,
                    kind.as_str(),
                    format!("passes through a {}; a name needs a map", kind.as_str()),
                ));
            }
            return Ok(());
        }
        let matched: Group = self
            .tree
            .members(target)
            .filter(|member| self.semantics.name_matches(self.tree, *member, name))
            .collect();
        match (self.semantics.repeated(), matched.last()) {
            (_, None) => {}
            (Repeated::LastWins, Some(last)) => next.push(vec![*last]),
            (Repeated::Conflict, Some(_)) => next.push(matched),
        }
        Ok(())
    }

    fn index(&self, group: &[NodeId], position: i64, next: &mut Vec<Group>) -> Result<()> {
        if group.len() > 1 {
            if let Some(member) = pick(group, position) {
                next.push(vec![member]);
            }
            return Ok(());
        }
        let Some(node) = group.first().copied() else {
            return Ok(());
        };
        let target = self.tree.target(node);
        let kind = self.tree[target].kind;
        if kind == NodeKind::List {
            let members: Vec<NodeId> = self.tree.members(target).collect();
            if let Some(member) = pick(&members, position) {
                next.push(vec![member]);
            }
        } else if self.semantics.repeated() == Repeated::Conflict && matches!(position, 0 | -1) {
            next.push(vec![node]);
        } else if self.concrete {
            return Err(key_conflict(
                self.key,
                kind.as_str(),
                format!(
                    "applies an index to a {}; an index needs a list",
                    kind.as_str()
                ),
            ));
        }
        Ok(())
    }

    fn wildcard(&self, group: &[NodeId], next: &mut Vec<Group>) {
        if group.len() > 1 {
            next.extend(group.iter().map(|member| vec![*member]));
            return;
        }
        for node in group {
            let target = self.tree.target(*node);
            next.extend(self.tree.members(target).map(|member| vec![member]));
        }
    }

    fn descendants(&self, node: NodeId, seen: &mut HashSet<NodeId>, next: &mut Vec<Group>) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            next.push(vec![current]);
            let target = self.tree.target(current);
            let mut members: Vec<NodeId> = self.tree.members(target).collect();
            members.reverse();
            stack.extend(members);
        }
    }

    fn filter(&self, group: &[NodeId], expr: &FilterExpr, next: &mut Vec<Group>) {
        let candidates: Vec<NodeId> = if group.len() > 1 {
            group.to_vec()
        } else {
            group
                .iter()
                .flat_map(|node| self.tree.members(self.tree.target(*node)))
                .collect()
        };
        for candidate in candidates {
            let value = self.tree.value_of(candidate, self.semantics, None);
            if filter::matches(expr, &value) {
                next.push(vec![candidate]);
            }
        }
    }

    fn finish(&self, groups: Vec<Group>) -> Result<Vec<Resolved>> {
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();
        for group in groups {
            for node in self.descend(&group)? {
                if seen.insert(node) {
                    resolved.push(Resolved {
                        node,
                        key: self.tree.path_of(node, self.semantics),
                    });
                }
            }
        }
        Ok(resolved)
    }

    fn ambiguous(&self, group: &[NodeId]) -> nurt_contracts::error::Error {
        let candidates: Vec<KeyPath> = group
            .iter()
            .map(|member| self.tree.path_of(*member, self.semantics))
            .collect();
        ambiguous_key(self.key, &candidates)
    }
}

/// Name of a step: one name, or names joined with `.` for flat keys.
fn joined_name(segments: &[KeySegment]) -> String {
    let names: Vec<&str> = segments
        .iter()
        .filter_map(|segment| match segment {
            KeySegment::Key(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    names.join(".")
}

/// Element at `position`, negative positions counting from the end.
fn pick(items: &[NodeId], position: i64) -> Option<NodeId> {
    let index = if position < 0 {
        let back = usize::try_from(position.unsigned_abs()).ok()?;
        items.len().checked_sub(back)?
    } else {
        usize::try_from(position).ok()?
    };
    items.get(index).copied()
}

/// Levenshtein distance in characters.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1; right.len() + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        previous = current;
    }
    previous[right.len()]
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
