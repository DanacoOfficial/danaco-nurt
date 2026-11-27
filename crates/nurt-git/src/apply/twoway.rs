// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::apply::twoway
// Responsibility:   Two-tree update of the working tree and the index, the rules of.
// Dependencies:     bstr, gix_index, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Two-tree update of the working tree and the index, the rules of
//! `twoway_merge` used by `git checkout` and `git switch`.
//!
//! For every path whose entry differs between the current tree `H` and the
//! target tree `M`, the index entry `I` and the working tree file decide:
//!
//! | `I` | Condition | Result |
//! |---|---|---|
//! | absent | `M` present, `H` absent | write `M` after the untracked check |
//! | absent | `M` present, `H` present | rejected: the deletion is staged |
//! | absent | `M` absent | nothing, after the untracked check |
//! | present | `I` equals `M` | keep `I` and the file |
//! | present | `I` equals `H`, `M` absent | remove when the file is up to date |
//! | present | `I` equals `H`, `M` present | write `M` when the file is up to date |
//! | present | otherwise | rejected |
//!
//! Paths where `H` equals `M` keep their index entry and file, so local
//! changes are carried over. A missing working tree file counts as up to
//! date; an entry with `skip-worktree` is never compared with the working
//! tree and never written to it, and a new entry outside the sparse checkout
//! selection gets `skip-worktree` instead of a file.

use bstr::{BStr, BString, ByteSlice};
use gix_index::entry::Flags;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::changes::{
    Blob, Changes, TreeMap, Untracked, flatten_tree, kept_flags, local_changes, same,
    skip_worktree_flags, verify_absent,
};
use super::context::ApplyContext;
use super::plan::ApplyPlan;
use super::worktree::Worktree;

/// Plans the update of the working tree and the index from the tree `head`
/// (`None` for an unborn branch) to the tree `target`, keeping local
/// changes of paths that do not differ between the two trees.
///
/// The plan is complete before anything changes: every path that blocks
/// the update is reported at once.
///
/// # Errors
///
/// `E_GIT_DIRTY` with the problem `unmerged` when the index has conflict
/// stages, or with the blocking paths grouped in the details
/// (`local_changes`, `untracked_overwritten`, `untracked_removed`,
/// `dir_loses_untracked`); `E_GIT_UNSUPPORTED` for a target tree with
/// unsafe paths; `E_IO` when the repository cannot be read.
pub fn plan_two_way(
    ctx: &ApplyContext<'_>,
    head: Option<&gix::oid>,
    target: &gix::oid,
) -> Result<ApplyPlan> {
    let unmerged = ctx.unmerged_paths();
    if !unmerged.is_empty() {
        return Err(unmerged_error(&unmerged));
    }
    let old = flatten_tree(ctx.repo, head)?;
    let new = flatten_tree(ctx.repo, Some(target))?;
    let mut worktree = Worktree::new(ctx.repo, ctx.root.clone(), &ctx.index)?;
    let mut changes = Changes::default();
    let mut pending_absent: Vec<(BString, Untracked)> = Vec::new();
    for path in changed_paths(&old, &new) {
        let path = path.as_bstr();
        let decision = decide(
            ctx,
            &mut worktree,
            path,
            old.get(path).copied(),
            new.get(path).copied(),
        )?;
        match decision {
            Decision::Keep => {}
            Decision::Write(blob, flags) => {
                changes.set_index(path, blob, flags);
                if !flags.contains(Flags::SKIP_WORKTREE) {
                    changes.write_blob(path, blob);
                    let previous = old.get(path).map(|blob| blob.id);
                    if previous == Some(blob.id) && worktree.observe(path)?.node.exists() {
                        changes.mark_mode_only(path);
                    }
                }
            }
            Decision::WriteNew(blob) if ctx.sparse.includes(path) => {
                changes.set_index(path, blob, Flags::empty());
                changes.write_blob(path, blob);
                pending_absent.push((path.to_owned(), Untracked::Overwritten));
            }
            Decision::WriteNew(blob) => changes.set_index(path, blob, skip_worktree_flags()),
            Decision::Remove { skip_worktree } => {
                changes.remove(path);
                if skip_worktree {
                    changes.removals.remove(path);
                }
            }
            Decision::CheckRemoved => pending_absent.push((path.to_owned(), Untracked::Removed)),
            Decision::Reject => {
                changes.rejections.local_changes.insert(path.to_owned());
            }
        }
    }
    for (path, kind) in pending_absent {
        verify_absent(ctx, &mut worktree, &mut changes, path.as_bstr(), kind)?;
    }
    if !changes.rejections.is_empty() {
        return Err(std::mem::take(&mut changes.rejections).into_error("the update", &[]));
    }
    changes.carried = local_changes(ctx, &mut worktree, &new, &changes)?;
    super::layout::finish(ctx, &mut worktree, changes, true, false)
}

/// Paths whose entries differ between two flattened trees, in path order.
pub(crate) fn changed_paths(old: &TreeMap, new: &TreeMap) -> Vec<BString> {
    let mut paths: Vec<BString> = old
        .iter()
        .filter(|(path, blob)| new.get(*path) != Some(*blob))
        .map(|(path, _)| path.clone())
        .chain(new.keys().filter(|path| !old.contains_key(*path)).cloned())
        .collect();
    paths.sort();
    paths
}

enum Decision {
    Keep,
    Write(Blob, Flags),
    WriteNew(Blob),
    Remove { skip_worktree: bool },
    CheckRemoved,
    Reject,
}

fn decide(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    path: &BStr,
    old: Option<Blob>,
    new: Option<Blob>,
) -> Result<Decision> {
    let Some(entry) = ctx.index.entry_by_path(path) else {
        return Ok(match (old, new) {
            (Some(_), Some(_)) if ctx.index_exists => Decision::Reject,
            (_, Some(blob)) => Decision::WriteNew(blob),
            (_, None) => Decision::CheckRemoved,
        });
    };
    let current = Some(Blob::of(entry));
    if same(current, new) {
        return Ok(Decision::Keep);
    }
    if !same(current, old) {
        return Ok(Decision::Reject);
    }
    let skip_worktree = entry.flags.contains(Flags::SKIP_WORKTREE);
    if !skip_worktree && !worktree.is_uptodate(entry, path, &ctx.index)? {
        return Ok(Decision::Reject);
    }
    Ok(match new {
        Some(blob) => Decision::Write(blob, kept_flags(Some(entry))),
        None => Decision::Remove { skip_worktree },
    })
}

/// `E_GIT_DIRTY` for an index with conflict stages.
pub(crate) fn unmerged_error(paths: &[BString]) -> Error {
    let mut error = Error::new(
        ErrorCode::GitDirty,
        format!(
            "the index has unresolved conflicts in {} path(s); resolve them and stage the result, or restore the paths with staged: true",
            paths.len()
        ),
    )
    .with_problem("unmerged");
    for path in paths {
        error = error.with_sample(path.to_string());
    }
    error
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
