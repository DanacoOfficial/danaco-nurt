// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! One-tree updates: the index and the working tree become a given tree,
//! as with `git reset --hard`, `git switch --discard-changes` and the
//! checkout of a new working tree.

use std::collections::BTreeSet;

use bstr::{BString, ByteSlice};
use gix_index::entry::Flags;
use nurt_contracts::error::Result;

use super::changes::{
    Blob, Changes, IndexChange, Untracked, flatten_tree, kept_flags, verify_absent,
};
use super::context::ApplyContext;
use super::plan::ApplyPlan;
use super::worktree::Worktree;

/// Options of [`plan_reset`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResetOptions {
    /// Untracked files in the way of the target are overwritten, as with
    /// `git reset --hard`; otherwise they block the update, as with
    /// `git switch --discard-changes`.
    pub overwrite_untracked: bool,
}

/// Plans the update of the index and the working tree to `target`,
/// discarding local changes of tracked files.
///
/// Every entry of the target is written where the working tree differs;
/// tracked entries absent from the target are removed from the index and
/// the working tree; conflict stages are replaced. Untracked files that are
/// not in the way stay; directories left empty are removed.
///
/// # Errors
///
/// `E_GIT_DIRTY` with the problem `untracked_overwritten` or
/// `dir_loses_untracked` when untracked files are in the way and
/// `overwrite_untracked` is false; `E_GIT_UNSUPPORTED` for a target tree
/// with unsafe paths; `E_IO` when the repository cannot be read.
pub fn plan_reset(
    ctx: &ApplyContext<'_>,
    target: &gix::oid,
    options: ResetOptions,
) -> Result<ApplyPlan> {
    let new = flatten_tree(ctx.repo, Some(target))?;
    let mut worktree = Worktree::new(ctx.repo, ctx.root.clone(), &ctx.index)?;
    let mut changes = Changes::default();
    let mut absent = Vec::new();
    let tracked: BTreeSet<BString> = ctx
        .index
        .entries()
        .iter()
        .map(|entry| entry.path(&ctx.index).to_owned())
        .collect();
    for path in tracked.iter().filter(|path| !new.contains_key(*path)) {
        changes.remove(path.as_bstr());
        let skipped = ctx
            .index
            .entry_by_path(path.as_bstr())
            .is_some_and(|entry| entry.flags.contains(Flags::SKIP_WORKTREE));
        if skipped {
            changes.removals.remove(path);
        }
    }
    for (path, blob) in &new {
        let path = path.as_bstr();
        match ctx
            .index
            .entry_by_path_and_stage(path, gix_index::entry::Stage::Unconflicted)
        {
            Some(entry) if Blob::of(entry) == *blob => {
                let skipped = entry.flags.contains(Flags::SKIP_WORKTREE);
                if !skipped
                    && (!worktree.observe(path)?.node.exists()
                        || !worktree.is_uptodate(entry, path, &ctx.index)?)
                {
                    changes.write_blob(path, *blob);
                }
            }
            Some(entry) => {
                changes.set_index(path, *blob, kept_flags(Some(entry)));
                if !entry.flags.contains(Flags::SKIP_WORKTREE) {
                    changes.write_blob(path, *blob);
                }
            }
            None => {
                changes.set_index(path, *blob, Flags::empty());
                changes.write_blob(path, *blob);
                if !tracked.contains(path) {
                    absent.push(path.to_owned());
                }
            }
        }
    }
    if !options.overwrite_untracked {
        for path in absent {
            verify_absent(
                ctx,
                &mut worktree,
                &mut changes,
                path.as_bstr(),
                Untracked::Overwritten,
            )?;
        }
        if !changes.rejections.is_empty() {
            return Err(std::mem::take(&mut changes.rejections).into_error("the reset", &[]));
        }
    }
    super::layout::finish(ctx, &mut worktree, changes, true)
}

/// Plans the checkout of `tree` into an empty working tree with an empty
/// index, as for a new clone or a new linked working tree.
///
/// # Errors
///
/// `E_GIT_DIRTY` with the problem `untracked_overwritten` when the working
/// tree already holds entries in the way; `E_GIT_UNSUPPORTED` for a tree
/// with unsafe paths; `E_IO` when the repository cannot be read.
pub fn plan_new_tree(ctx: &ApplyContext<'_>, tree: &gix::oid) -> Result<ApplyPlan> {
    let new = flatten_tree(ctx.repo, Some(tree))?;
    let mut worktree = Worktree::new(ctx.repo, ctx.root.clone(), &ctx.index)?;
    let mut changes = Changes::default();
    for entry in ctx.index.entries() {
        changes
            .index
            .insert(entry.path(&ctx.index).to_owned(), IndexChange::Remove);
    }
    for (path, blob) in &new {
        changes.set_index(path.as_bstr(), *blob, Flags::empty());
        changes.write_blob(path.as_bstr(), *blob);
    }
    for path in new.keys() {
        if ctx.index.entry_by_path(path.as_bstr()).is_none() {
            verify_absent(
                ctx,
                &mut worktree,
                &mut changes,
                path.as_bstr(),
                Untracked::Overwritten,
            )?;
        }
    }
    if !changes.rejections.is_empty() {
        return Err(std::mem::take(&mut changes.rejections).into_error("the checkout", &[]));
    }
    super::layout::finish(ctx, &mut worktree, changes, true)
}
