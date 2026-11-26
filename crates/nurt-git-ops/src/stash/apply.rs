// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `git.stash action: apply|pop`: merging a stash entry into the working
//! tree.
//!
//! The working tree commit `W` of the entry is merged into the current
//! index with the parent of `W` as the base, as `git stash apply` does. A
//! clean merge leaves the index as it was, except that files new in the
//! result stay added; with `index` the index commit is merged into the
//! index first. Conflicts leave the stages of the merge in the index and
//! the conflict markers in the working tree. Untracked files of the entry
//! are restored; one of them colliding with an existing file refuses the
//! whole operation before anything changes.

use std::collections::HashSet;

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::State;
use gix::index::entry::{Flags, Stat};
use nurt_contracts::error::{ErrorCode, Result};
use nurt_git::apply::{ApplyContext, IndexPlan, plan_two_way};
use nurt_git::repo::GitRepo;
use nurt_git::repo::index::index_to_tree;

use super::entries::{self, StashParts, plan_drop};
use super::{StashAction, StashOutcome, StashRequest};
use crate::merge::{MergeLabels, MergeOptions, MergeOutcome, merge_trees};
use crate::reset::execute::{Change, ref_tx, state_write, touched};
use crate::reset::ident::Identity;
use crate::reset::index::{self, StatSources, entry_kind, merged_entry};
use crate::reset::repo::{commit_tree, error, head, io_error, root_literal, worktree_path};

/// Label of the current side when the index differs from the base.
const OURS_LABEL: &str = "Updated upstream";
/// Label of the current side when the index equals the base of the entry.
const OURS_AT_BASE_LABEL: &str = "Version stash was based on";
/// Label of the stashed side.
const THEIRS_LABEL: &str = "Stashed changes";
/// Label of the base.
const BASE_LABEL: &str = "Stash base";
/// State file with the tree of a merge that stopped with conflicts.
const AUTO_MERGE: &str = "AUTO_MERGE";

fn merge(
    repo: &GitRepo,
    base: ObjectId,
    ours: ObjectId,
    theirs: ObjectId,
    ours_label: &str,
) -> Result<MergeOutcome> {
    let mut options = MergeOptions::from_repo(repo.gix())?;
    options.labels = MergeLabels {
        base: Some(BASE_LABEL.to_owned()),
        ours: ours_label.to_owned(),
        theirs: THEIRS_LABEL.to_owned(),
    };
    merge_trees(repo.gix(), &base, &ours, &theirs, &options)
}

fn spec_text(position: Option<usize>, id: ObjectId) -> String {
    position.map_or_else(
        || id.to_hex().to_string(),
        |position| format!("stash@{{{position}}}"),
    )
}

/// Index of a clean merge: the current index with the paths that are new in
/// the result added.
fn index_keeping_new_paths(current: &State, result: &State) -> State {
    let mut state = current.clone();
    for entry in result.entries() {
        let path = entry.path(result);
        if current.entry_index_by_path(path).is_err() {
            state.dangerously_push_entry(
                Stat::default(),
                entry.id,
                Flags::empty(),
                entry.mode,
                path,
            );
        }
    }
    state.sort_entries();
    state
}

/// Untracked files of the entry as `(path, mode, object)`, refused when one
/// of them exists in the working tree or in the merge result.
fn untracked_files(
    repo: &GitRepo,
    parts: &StashParts,
    result: &State,
) -> Result<Vec<(BString, gix::index::entry::Mode, ObjectId)>> {
    let Some(commit) = parts.untracked else {
        return Ok(Vec::new());
    };
    let state = index::from_tree(repo, commit_tree(repo, commit)?)?;
    let mut files = Vec::new();
    let mut existing = Vec::new();
    for entry in state.entries() {
        let path = entry.path(&state).to_owned();
        if merged_entry(result, path.as_bstr()).is_some()
            || std::fs::symlink_metadata(worktree_path(repo, path.as_bstr())).is_ok()
        {
            existing.push(path.to_str_lossy().into_owned());
        } else {
            files.push((path, entry.mode, entry.id));
        }
    }
    if existing.is_empty() {
        return Ok(files);
    }
    let mut err = error(
        ErrorCode::GitDirty,
        format!(
            "{} untracked file(s) of the entry already exist; move them away first",
            existing.len()
        ),
    )
    .with_problem("untracked_overwritten")
    .with_fix(format!("git.status {}", root_literal(repo)))
    .with_detail("paths", serde_json::Value::from(existing.clone()));
    for path in existing {
        err = err.with_sample(path);
    }
    Err(err)
}

fn refuse_unmerged(repo: &GitRepo) -> nurt_contracts::error::Error {
    error(
        ErrorCode::GitInProgress,
        "cannot apply a stash while the index has unresolved conflicts; resolve them first",
    )
    .with_problem("unmerged_paths")
    .with_fix(format!("git.status {}", root_literal(repo)))
}

fn not_a_stash_entry(repo: &GitRepo, spec: &str) -> nurt_contracts::error::Error {
    error(
        ErrorCode::GitRef,
        format!("`{spec}` is not a stash entry; pop and drop take entries of the list"),
    )
    .with_problem("not_a_stash_reference")
    .with_fix(format!("git.stash {} action: list", root_literal(repo)))
}

/// Target tree of the update: the merge result with the untracked files of
/// the entry added.
fn target_tree(
    repo: &GitRepo,
    result: ObjectId,
    untracked: &[(BString, gix::index::entry::Mode, ObjectId)],
) -> Result<ObjectId> {
    if untracked.is_empty() {
        return Ok(result);
    }
    let mut editor = repo
        .gix()
        .edit_tree(result)
        .map_err(|cause| io_error("cannot edit tree", cause))?;
    for (path, mode, id) in untracked {
        editor
            .upsert(path.as_bstr(), entry_kind(*mode), *id)
            .map_err(|cause| io_error("cannot edit tree", cause))?;
    }
    editor
        .write()
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write tree", cause))
}

/// Plans `git stash apply`, and with `pop` also the removal of the entry
/// when the merge is clean.
pub(crate) fn plan_apply(
    repo: &GitRepo,
    request: &StashRequest,
    pop: bool,
    identity: &Identity,
) -> Result<(Change, StashOutcome)> {
    let (position, id) = entries::resolve(repo, request.entry.as_deref())?;
    let spec = spec_text(position, id);
    if pop && position.is_none() {
        return Err(not_a_stash_entry(repo, &spec));
    }
    let parts = entries::parts(repo, id)?;
    let context = ApplyContext::open(repo.gix())?;
    let current = context.index();
    if index::has_conflicts(current) {
        return Err(refuse_unmerged(repo));
    }
    let base = commit_tree(repo, parts.base)?;
    let work = commit_tree(repo, id)?;
    let current_tree = index_to_tree(repo, current)?;
    let ours_label = if base == current_tree {
        OURS_AT_BASE_LABEL
    } else {
        OURS_LABEL
    };
    let result = merge(repo, base, current_tree, work, ours_label)?;
    let merged = index::from_tree(repo, result.tree)?;
    let untracked = untracked_files(repo, &parts, &merged)?;
    let target = target_tree(repo, result.tree, &untracked)?;
    let mut change = Change::new();
    change.objects = true;
    change.plan = plan_two_way(&context, Some(&current_tree), &target)?;
    let (written, removed) = touched(&change.plan);
    let clean = result.is_clean();
    let final_state = if clean {
        index_keeping_new_paths(current, &merged)
    } else {
        result.index.clone()
    };
    let written_set: HashSet<BString> = written
        .iter()
        .map(|path| BString::from(path.as_str()))
        .collect();
    change.plan.index = Some(IndexPlan {
        state: index::with_stats(
            repo.root(),
            final_state,
            current,
            &StatSources {
                written: Some(&written_set),
                read_now: None,
            },
        ),
        expect: context.index_expect().clone(),
        record_resolve_undo: false,
    });
    // As in Git, the tree with conflict markers is kept for conflicts only.
    if !clean {
        change.plan.state_files.push(state_write(
            repo,
            AUTO_MERGE,
            format!("{}\n", result.tree.to_hex()).into_bytes(),
        )?);
    }
    let head = head(repo)?;
    let mut outcome = StashOutcome {
        repo: repo.root().to_path_buf(),
        action: if pop {
            StashAction::Pop
        } else {
            StashAction::Apply
        },
        head: head.id,
        branch: head.short_branch(),
        applied: Some(id),
        conflicts: result
            .conflicts
            .iter()
            .map(crate::merge::MergeConflict::to_record)
            .collect(),
        notable: result.notes.clone(),
        written,
        removed,
        ..StashOutcome::default()
    };
    match position {
        Some(position) if pop && clean => {
            let mut refs = ref_tx(repo, &identity.committer)?;
            plan_drop(repo, &mut refs, &mut change.writes, position)?;
            change.set_refs(refs)?;
            outcome.dropped = Some(id);
        }
        _ => outcome.kept = pop,
    }
    Ok((change, outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_of_the_list_are_named_by_position() {
        let id = ObjectId::null(gix::hash::Kind::Sha1);
        assert_eq!(spec_text(Some(2), id), "stash@{2}");
        assert_eq!(spec_text(None, id), id.to_hex().to_string());
    }

    #[test]
    fn new_paths_of_the_result_stay_in_the_index() {
        let hash = gix::hash::Kind::Sha1;
        let id = ObjectId::empty_blob(hash);
        let mut current = State::new(hash);
        current.dangerously_push_entry(
            Stat::default(),
            id,
            Flags::empty(),
            gix::index::entry::Mode::FILE,
            b"a".as_bstr(),
        );
        let mut result = current.clone();
        result.dangerously_push_entry(
            Stat::default(),
            id,
            Flags::empty(),
            gix::index::entry::Mode::FILE,
            b"b".as_bstr(),
        );
        result.sort_entries();
        let state = index_keeping_new_paths(&current, &result);
        let paths: Vec<_> = state
            .entries()
            .iter()
            .map(|entry| entry.path(&state).to_owned())
            .collect();
        assert_eq!(paths, vec![BString::from("a"), BString::from("b")]);
    }
}
