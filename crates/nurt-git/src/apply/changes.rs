// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Path-level decisions shared by the planners: what happens to each index
//! entry and to each working tree file, before the decisions are laid out
//! as journal targets.

use std::collections::{BTreeMap, BTreeSet};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix_index::entry::{Flags, Mode, Stat};
use nurt_contracts::error::Result;

use super::context::ApplyContext;
use super::error::{Rejections, repo_error, unsafe_path};
use super::plan::{Content, EntryKind, Notable};
use super::worktree::{Node, Worktree};

/// Mode and object id of a tree or index entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Blob {
    pub(crate) mode: Mode,
    pub(crate) id: ObjectId,
}

impl Blob {
    pub(crate) fn of(entry: &gix_index::Entry) -> Self {
        Self {
            mode: entry.mode,
            id: entry.id,
        }
    }
}

/// Entries of a tree flattened to paths, in path order.
pub(crate) type TreeMap = BTreeMap<BString, Blob>;

/// Flattens `tree`; `None` yields the empty tree. Paths are validated by the
/// protection rules of the repository (`.git`, `core.protectNTFS`,
/// `core.protectHFS`), and a tree that holds both `a` and `a/x` is refused.
pub(crate) fn flatten_tree(repo: &gix::Repository, tree: Option<&gix::oid>) -> Result<TreeMap> {
    let Some(tree) = tree else {
        return Ok(TreeMap::new());
    };
    let file = repo
        .index_from_tree(tree)
        .map_err(|err| unsafe_path(tree.to_string().as_bytes(), &err.to_string()))?;
    let state: &gix_index::State = &file;
    let map: TreeMap = state
        .entries()
        .iter()
        .map(|entry| (entry.path(state).to_owned(), Blob::of(entry)))
        .collect();
    reject_file_directory_clash(&map)?;
    Ok(map)
}

fn reject_file_directory_clash(map: &TreeMap) -> Result<()> {
    for path in map.keys() {
        for (position, byte) in path.iter().enumerate() {
            if *byte == b'/' && map.contains_key(path[..position].as_bstr()) {
                return Err(unsafe_path(path, "a parent directory is also a file"));
            }
        }
    }
    Ok(())
}

/// Same entry in the sense of `git`: both absent, or equal mode and id.
pub(crate) fn same(left: Option<Blob>, right: Option<Blob>) -> bool {
    left == right
}

/// New value of an index entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndexChange {
    /// The entry becomes `blob` at stage 0; conflict stages are dropped.
    Set {
        blob: Blob,
        stat: Stat,
        flags: Flags,
    },
    /// Every stage of the entry is removed.
    Remove,
}

/// Content written to one working tree path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Write {
    pub(crate) kind: EntryKind,
    pub(crate) content: Content,
    /// Only the executable bit differs from the current file.
    pub(crate) mode_only: bool,
}

/// Decisions of a planner.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    pub(crate) index: BTreeMap<BString, IndexChange>,
    pub(crate) writes: BTreeMap<BString, Write>,
    pub(crate) removals: BTreeSet<BString>,
    pub(crate) rejections: Rejections,
    pub(crate) notable: Vec<Notable>,
    pub(crate) carried: Vec<BString>,
}

impl Changes {
    /// Writes `blob` to the working tree path `path`.
    pub(crate) fn write_blob(&mut self, path: &BStr, blob: Blob) {
        if let Some(kind) = EntryKind::from_index_mode(blob.mode) {
            self.writes.insert(
                path.to_owned(),
                Write {
                    kind,
                    content: Content::Blob(blob.id),
                    mode_only: false,
                },
            );
        }
    }

    /// Records that the planned write of `path` changes only the executable
    /// bit of an up-to-date file.
    pub(crate) fn mark_mode_only(&mut self, path: &BStr) {
        if let Some(write) = self.writes.get_mut(path)
            && matches!(write.kind, EntryKind::File | EntryKind::Executable)
        {
            write.mode_only = true;
        }
    }

    /// Sets the index entry of `path` to `blob` with empty statistics.
    pub(crate) fn set_index(&mut self, path: &BStr, blob: Blob, flags: Flags) {
        self.index.insert(
            path.to_owned(),
            IndexChange::Set {
                blob,
                stat: Stat::default(),
                flags,
            },
        );
    }

    /// Removes the index entry and the working tree file of `path`.
    pub(crate) fn remove(&mut self, path: &BStr) {
        self.index.insert(path.to_owned(), IndexChange::Remove);
        self.removals.insert(path.to_owned());
    }

    /// Whether `path` is a tracked entry that the plan removes from the
    /// working tree.
    pub(crate) fn removes(&self, path: &BStr) -> bool {
        self.removals.contains(path)
    }
}

/// Kind of rejection for an untracked entry in the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Untracked {
    /// The update writes a file where the untracked entry is.
    Overwritten,
    /// The update removes the path that the untracked entry occupies.
    Removed,
}

/// Flags kept from a previous index entry when the planner replaces it.
pub(crate) fn kept_flags(previous: Option<&gix_index::Entry>) -> Flags {
    match previous {
        Some(entry) if entry.flags.contains(Flags::SKIP_WORKTREE) => skip_worktree_flags(),
        _ => Flags::empty(),
    }
}

/// Flags of an index entry without a working tree file; `skip-worktree` is
/// an extended flag, so the entry also carries the extended marker.
pub(crate) fn skip_worktree_flags() -> Flags {
    Flags::SKIP_WORKTREE | Flags::EXTENDED
}

/// Checks the working tree before a file is created at `path` that the
/// index does not track, as `git` does in `verify_absent`.
///
/// An untracked entry that is ignored may be overwritten; any other blocks
/// the update. A directory at `path` blocks the update when it holds
/// untracked files that are not ignored; tracked files in it are removed
/// when they are up to date.
pub(crate) fn verify_absent(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    changes: &mut Changes,
    path: &BStr,
    kind: Untracked,
) -> Result<()> {
    if !verify_leading(worktree, changes, path, kind)? {
        return Ok(());
    }
    let observation = worktree.observe(path)?;
    match observation.node {
        Node::Missing => Ok(()),
        Node::Dir => verify_clean_directory(ctx, worktree, changes, path),
        _ => {
            if !changes.removes(path) && !worktree.is_ignored(path, false)? {
                reject_untracked(changes, path, kind);
            }
            Ok(())
        }
    }
}

/// Checks the leading directories of `path`; returns whether the path
/// itself needs a check.
fn verify_leading(
    worktree: &mut Worktree<'_>,
    changes: &mut Changes,
    path: &BStr,
    kind: Untracked,
) -> Result<bool> {
    for (position, byte) in path.iter().enumerate() {
        if *byte != b'/' {
            continue;
        }
        let leading = path[..position].as_bstr();
        match worktree.observe(leading)?.node {
            Node::Dir => continue,
            Node::Missing => return Ok(false),
            _ => {
                if !changes.removes(leading) && !worktree.is_ignored(leading, false)? {
                    reject_untracked(changes, leading, kind);
                }
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn reject_untracked(changes: &mut Changes, path: &BStr, kind: Untracked) {
    let list = match kind {
        Untracked::Overwritten => &mut changes.rejections.untracked_overwritten,
        Untracked::Removed => &mut changes.rejections.untracked_removed,
    };
    list.insert(path.to_owned());
}

fn verify_clean_directory(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    changes: &mut Changes,
    path: &BStr,
) -> Result<()> {
    let mut prefix = path.to_owned();
    prefix.push(b'/');
    let tracked: Vec<(BString, usize)> = ctx
        .index
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.path(&ctx.index).starts_with(prefix.as_slice()))
        .map(|(position, entry)| (entry.path(&ctx.index).to_owned(), position))
        .collect();
    for (tracked_path, position) in &tracked {
        if changes.removes(tracked_path.as_bstr()) {
            continue;
        }
        let entry = &ctx.index.entries()[*position];
        if entry.stage_raw() == 0
            && worktree.is_uptodate(entry, tracked_path.as_bstr(), &ctx.index)?
        {
            changes.remove(tracked_path.as_bstr());
        } else {
            changes
                .rejections
                .local_changes
                .insert(tracked_path.clone());
        }
    }
    let index = &ctx.index;
    let untracked =
        worktree.untracked_below(path, &|candidate| index.entry_by_path(candidate).is_some())?;
    if !untracked.is_empty() {
        changes
            .rejections
            .dir_loses_untracked
            .insert(path.to_owned());
    }
    Ok(())
}

/// Paths whose local changes stay in place after an update: the index
/// differs from the target, or the working tree differs from the index.
pub(crate) fn local_changes(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    target: &TreeMap,
    changes: &Changes,
) -> Result<Vec<BString>> {
    let mut carried = Vec::new();
    for entry in ctx.index.entries() {
        let path = entry.path(&ctx.index);
        if changes.index.contains_key(path) || entry.stage_raw() != 0 {
            continue;
        }
        let staged = target.get(path).copied() != Some(Blob::of(entry));
        let skipped = entry.flags.contains(Flags::SKIP_WORKTREE);
        let changed_file = !skipped
            && (!worktree.is_uptodate(entry, path, &ctx.index)?
                || !worktree.observe(path)?.node.exists());
        if staged || changed_file {
            carried.push(path.to_owned());
        }
    }
    for path in target.keys() {
        if ctx.index.entry_by_path(path.as_bstr()).is_none() && !changes.index.contains_key(path) {
            carried.push(path.clone());
        }
    }
    carried.sort();
    carried.dedup();
    Ok(carried)
}

/// Builds the new index from the current one and the decisions.
pub(crate) fn build_index(base: &gix_index::State, changes: &Changes) -> Result<gix_index::State> {
    let mut state = base.clone();
    if changes.index.is_empty() {
        return Ok(state);
    }
    state.remove_entries(|_, path, _| changes.index.contains_key(path));
    for (path, change) in &changes.index {
        if let IndexChange::Set { blob, stat, flags } = change {
            state.dangerously_push_entry(*stat, blob.id, *flags, blob.mode, path.as_bstr());
        }
    }
    state.sort_entries();
    state.remove_tree();
    if state.entries().windows(2).any(|pair| {
        pair[0].path(&state) == pair[1].path(&state) && pair[0].stage_raw() == pair[1].stage_raw()
    }) {
        return Err(repo_error("duplicate entries", "build the new index"));
    }
    Ok(state)
}
