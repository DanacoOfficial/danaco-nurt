// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Restoration of selected paths of the index or the working tree from the
//! index or a tree, as `git restore` does in its default no-overlay mode.

use std::collections::BTreeSet;

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix_index::entry::{Flags, Stage};
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::changes::{Blob, Changes, IndexChange, TreeMap, flatten_tree, kept_flags};
use super::context::ApplyContext;
use super::plan::{ApplyPlan, EntryKind};
use super::twoway::unmerged_error;
use super::worktree::{Node, Worktree};

/// Where restored content comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    /// The current index.
    Index,
    /// A tree of the object database.
    Tree(ObjectId),
}

/// What [`plan_paths`] restores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathsRequest {
    /// Source of the restored content.
    pub source: PathSource,
    /// Pathspecs relative to the working tree root, with `git` pathspec
    /// syntax (`dir`, `*.txt`, `:(glob)**/x`); empty selects every path.
    pub pathspec: Vec<BString>,
    /// Restore the index entries.
    pub to_index: bool,
    /// Restore the working tree files.
    pub to_worktree: bool,
}

/// Plans the restoration of the paths selected by a pathspec.
///
/// Selected paths present in the source get its content; selected paths
/// present in the index but absent from the source are removed. The
/// working tree is written only where it differs. The index is always
/// written, so the statistics of rewritten files are fresh.
///
/// # Errors
///
/// `E_NO_MATCH` when a pathspec matches no path of the source or the index;
/// `E_GIT_DIRTY` with the problem `unmerged` when the working tree is
/// restored from the index and a selected path has conflict stages;
/// `E_ARG` for a malformed pathspec; `E_IO` when the repository cannot be
/// read.
pub fn plan_paths(ctx: &ApplyContext<'_>, request: &PathsRequest) -> Result<ApplyPlan> {
    let source = match request.source {
        PathSource::Index => index_map(&ctx.index),
        PathSource::Tree(tree) => flatten_tree(ctx.repo, Some(&tree))?,
    };
    let selected = select(ctx, &source, &request.pathspec)?;
    let mut worktree = Worktree::new(ctx.repo, ctx.root.clone(), &ctx.index)?;
    let mut changes = Changes::default();
    let mut unmerged = Vec::new();
    for path in &selected {
        let path = path.as_bstr();
        let wanted = source.get(path).copied();
        let current = ctx.index.entry_by_path_and_stage(path, Stage::Unconflicted);
        let conflicted = current.is_none() && ctx.index.entry_by_path(path).is_some();
        if request.to_index {
            restore_index(&mut changes, path, wanted, current);
        }
        if !request.to_worktree {
            continue;
        }
        if conflicted && !request.to_index && request.source == PathSource::Index {
            unmerged.push(path.to_owned());
            continue;
        }
        let skipped = current.is_some_and(|entry| entry.flags.contains(Flags::SKIP_WORKTREE));
        if !skipped {
            restore_file(ctx, &mut worktree, &mut changes, path, wanted)?;
        }
    }
    if !unmerged.is_empty() {
        return Err(unmerged_error(&unmerged));
    }
    super::layout::finish(ctx, &mut worktree, changes, true)
}

fn index_map(index: &gix_index::State) -> TreeMap {
    index
        .entries()
        .iter()
        .filter(|entry| entry.stage_raw() == 0)
        .map(|entry| (entry.path(index).to_owned(), Blob::of(entry)))
        .collect()
}

fn select(
    ctx: &ApplyContext<'_>,
    source: &TreeMap,
    pathspec: &[BString],
) -> Result<BTreeSet<BString>> {
    let candidates: BTreeSet<BString> = source
        .keys()
        .cloned()
        .chain(
            ctx.index
                .entries()
                .iter()
                .map(|entry| entry.path(&ctx.index).to_owned()),
        )
        .collect();
    if pathspec.is_empty() {
        return Ok(candidates);
    }
    let patterns = pathspec
        .iter()
        .map(|spec| {
            gix_pathspec::parse(spec, gix_pathspec::Defaults::default()).map_err(|err| {
                Error::new(ErrorCode::Arg, format!("invalid pathspec '{spec}': {err}"))
                    .with_found(spec.to_string())
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut search =
        gix_pathspec::Search::from_specs(patterns, None, &ctx.root).map_err(|err| {
            Error::new(
                ErrorCode::Arg,
                format!("invalid pathspec: {err}; give paths inside the repository"),
            )
        })?;
    let mut matched = vec![false; pathspec.len()];
    let mut selected = BTreeSet::new();
    for path in candidates {
        let hit = search
            .pattern_matching_relative_path(path.as_bstr(), Some(false), &mut |_, _, _, _| false)
            .filter(|found| !found.is_excluded())
            .map(|found| found.sequence_number);
        if let Some(number) = hit {
            if let Some(flag) = matched.get_mut(number) {
                *flag = true;
            }
            selected.insert(path);
        }
    }
    if let Some(position) = matched.iter().position(|hit| !hit) {
        return Err(no_match(pathspec[position].as_bstr()));
    }
    Ok(selected)
}

fn no_match(spec: &BStr) -> Error {
    Error::new(
        ErrorCode::NoMatch,
        format!(
            "pathspec '{spec}' did not match any file known to git; check the path and the source"
        ),
    )
    .with_found(spec.to_string())
}

fn restore_index(
    changes: &mut Changes,
    path: &BStr,
    wanted: Option<Blob>,
    current: Option<&gix_index::Entry>,
) {
    let change = match (wanted, current) {
        (Some(blob), Some(entry))
            if Blob::of(entry) == blob && !entry.flags.contains(Flags::INTENT_TO_ADD) =>
        {
            return;
        }
        (Some(blob), entry) => IndexChange::Set {
            blob,
            stat: Default::default(),
            flags: kept_flags(entry),
        },
        (None, _) => IndexChange::Remove,
    };
    changes.index.insert(path.to_owned(), change);
}

fn restore_file(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    changes: &mut Changes,
    path: &BStr,
    wanted: Option<Blob>,
) -> Result<()> {
    let observation = worktree.observe(path)?;
    let Some(blob) = wanted else {
        if observation.node.exists() && !observation.node.is_dir() {
            changes.removals.insert(path.to_owned());
        }
        return Ok(());
    };
    if blob.mode == gix_index::entry::Mode::COMMIT {
        return Ok(());
    }
    if !matches_blob(ctx, worktree, path, observation.node, blob)? {
        changes.write_blob(path, blob);
    }
    Ok(())
}

/// Whether the working tree entry at `path` already holds `blob`.
fn matches_blob(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    path: &BStr,
    node: Node,
    blob: Blob,
) -> Result<bool> {
    let kind_agrees = match (EntryKind::from_index_mode(blob.mode), node) {
        (Some(EntryKind::Symlink), Node::Symlink) => true,
        (Some(EntryKind::Symlink), Node::File { .. }) => !worktree.caps.symlink,
        (Some(EntryKind::File), Node::File { executable }) => {
            !executable || !worktree.caps.executable_bit
        }
        (Some(EntryKind::Executable), Node::File { executable }) => {
            executable || !worktree.caps.executable_bit
        }
        _ => false,
    };
    if !kind_agrees {
        return Ok(false);
    }
    if let Some(entry) = ctx.index.entry_by_path_and_stage(path, Stage::Unconflicted)
        && Blob::of(entry) == blob
    {
        return worktree.is_uptodate(entry, path, &ctx.index);
    }
    Ok(worktree.hash_worktree(path, node, &ctx.index)? == blob.id)
}
