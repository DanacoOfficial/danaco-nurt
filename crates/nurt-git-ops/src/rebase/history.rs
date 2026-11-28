// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::rebase::history
// Responsibility:   Selection of the commits to replay.
// Dependencies:     gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Selection of the commits to replay.
//!
//! The list equals `git rev-list --reverse --topo-order --no-merges
//! --cherry-pick --right-only <upstream>...HEAD`: commits reachable from
//! `HEAD` and not from the upstream, oldest first in graph order, without
//! merge commits and without commits whose patch identifier equals the one
//! of a commit on the upstream side.

use std::collections::{HashMap, HashSet};

use gix::ObjectId;
use nurt_contracts::error::Result;

use super::error;
use super::objects;
use super::patch;

/// A commit to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TodoItem {
    /// Commit to replay.
    pub(super) id: ObjectId,
    /// One-line subject shown after the commit name.
    pub(super) subject: String,
}

/// Commits to pick and commits skipped because their change is already in
/// the upstream.
#[derive(Debug, Default)]
pub(super) struct Selection {
    /// Commits to pick, oldest first.
    pub(super) items: Vec<TodoItem>,
    /// Commits skipped as already applied.
    pub(super) skipped: Vec<ObjectId>,
}

/// One commit of a walk with its parents.
struct Node {
    id: ObjectId,
    parents: Vec<ObjectId>,
}

/// Commits reachable from `tip` and not from `hidden`, newest first.
fn walk(repo: &gix::Repository, tip: ObjectId, hidden: ObjectId) -> Result<Vec<Node>> {
    let walk = repo
        .rev_walk([tip])
        .with_hidden([hidden])
        .all()
        .map_err(|cause| error::io("walk the history", cause))?;
    let mut nodes = Vec::new();
    for info in walk {
        let info = info.map_err(|cause| error::io("walk the history", cause))?;
        nodes.push(Node {
            id: info.id,
            parents: info.parent_ids().map(|id| id.detach()).collect(),
        });
    }
    Ok(nodes)
}

/// Orders commits like Git's graph order: a commit is emitted only after
/// all its children in the set, using a stack of ready commits so that one
/// line of history is followed to its end before another begins.
fn graph_order(nodes: &[Node], tip: ObjectId) -> Vec<ObjectId> {
    let in_set: HashSet<ObjectId> = nodes.iter().map(|node| node.id).collect();
    let parents: HashMap<ObjectId, &Vec<ObjectId>> =
        nodes.iter().map(|node| (node.id, &node.parents)).collect();
    let mut children: HashMap<ObjectId, usize> = HashMap::new();
    for node in nodes {
        for parent in node
            .parents
            .iter()
            .filter(|parent| in_set.contains(*parent))
        {
            *children.entry(*parent).or_default() += 1;
        }
    }
    let mut stack: Vec<ObjectId> = Vec::new();
    if in_set.contains(&tip) {
        stack.push(tip);
    }
    let mut out = Vec::with_capacity(nodes.len());
    while let Some(id) = stack.pop() {
        out.push(id);
        for parent in parents.get(&id).copied().into_iter().flatten() {
            let Some(count) = children.get_mut(parent) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                stack.push(*parent);
            }
        }
    }
    out
}

/// Selects the commits of `upstream..head` to replay onto the upstream;
/// commits whose patch identifier equals that of a non-merge commit of
/// `head..upstream` are skipped.
pub(super) fn select(
    repo: &gix::Repository,
    head: ObjectId,
    upstream: ObjectId,
) -> Result<Selection> {
    let ours = walk(repo, head, upstream)?;
    let merges: HashSet<ObjectId> = ours
        .iter()
        .filter(|node| node.parents.len() > 1)
        .map(|node| node.id)
        .collect();
    let mut order = graph_order(&ours, head);
    order.reverse();
    order.retain(|id| !merges.contains(id));
    let applied = if order.is_empty() {
        HashSet::new()
    } else {
        upstream_patch_ids(repo, upstream, head)?
    };
    let mut selection = Selection::default();
    for id in order {
        let commit = objects::read_commit(repo, id)?;
        if !applied.is_empty() && applied.contains(&commit_patch_id(repo, &commit)?) {
            selection.skipped.push(id);
            continue;
        }
        selection.items.push(TodoItem {
            id,
            subject: commit.oneline(),
        });
    }
    Ok(selection)
}

/// Whether every commit of `hidden..tip` has exactly one parent.
pub(super) fn is_linear(repo: &gix::Repository, tip: ObjectId, hidden: ObjectId) -> Result<bool> {
    Ok(walk(repo, tip, hidden)?
        .iter()
        .all(|node| node.parents.len() == 1))
}

/// Patch identifiers of the non-merge commits of `head..upstream`.
fn upstream_patch_ids(
    repo: &gix::Repository,
    upstream: ObjectId,
    head: ObjectId,
) -> Result<HashSet<[u8; 20]>> {
    let mut ids = HashSet::new();
    for node in walk(repo, upstream, head)? {
        if node.parents.len() > 1 {
            continue;
        }
        let commit = objects::read_commit(repo, node.id)?;
        ids.insert(commit_patch_id(repo, &commit)?);
    }
    Ok(ids)
}

/// Patch identifier of a commit against its first parent, or against the
/// empty tree for a root commit.
pub(super) fn commit_patch_id(
    repo: &gix::Repository,
    commit: &objects::CommitInfo,
) -> Result<[u8; 20]> {
    let base = match commit.parents.first() {
        Some(parent) => objects::tree_of(repo, *parent)?,
        None => objects::empty_tree(repo),
    };
    patch::patch_id(repo, base, commit.tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[byte; 20])
    }

    #[test]
    fn graph_order_follows_one_line_before_the_other() {
        let nodes = vec![
            Node {
                id: id(4),
                parents: vec![id(2), id(3)],
            },
            Node {
                id: id(3),
                parents: vec![id(1)],
            },
            Node {
                id: id(2),
                parents: vec![id(1)],
            },
            Node {
                id: id(1),
                parents: vec![],
            },
        ];
        assert_eq!(graph_order(&nodes, id(4)), vec![id(4), id(3), id(2), id(1)]);
    }

    #[test]
    fn graph_order_ignores_parents_outside_the_set() {
        let nodes = vec![
            Node {
                id: id(2),
                parents: vec![id(1)],
            },
            Node {
                id: id(1),
                parents: vec![id(9)],
            },
        ];
        assert_eq!(graph_order(&nodes, id(2)), vec![id(2), id(1)]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
