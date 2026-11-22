// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `git.stash action: push`: the stash commits and the reset of the saved
//! paths.
//!
//! The entry has the structure git uses: the index commit `I` (parent
//! `HEAD`), with untracked files the commit `U` (no parent), and the working
//! tree commit `W` (parents `HEAD`, `I` and `U`).

use std::collections::BTreeSet;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::State;
use nurt_contracts::error::{ErrorCode, Result};

use super::entries::STASH_REF;
use super::{PlannedStash, StashAction, StashOutcome, StashRequest};
use crate::reset::ident::Identity;
use crate::reset::index::{self, entry_kind, merged_entry};
use crate::reset::refs::{plan_head_update, plan_ref_write, plan_reflog_append, read_ref, reflog_line};
use crate::reset::repo::{HeadState, Repo, error, io_error, quote};
use crate::reset::txplan::TxPlan;
use crate::reset::worktree::{Checkout, Policy, Worktree, plan_checkout};

/// Paths a push is limited to.
pub(crate) struct Scope {
    paths: Vec<BString>,
    exclude: Vec<BString>,
}

impl Scope {
    /// Scope of `request`, with paths relative to the root of `repo`.
    pub(crate) fn new(repo: &Repo, request: &StashRequest) -> Result<Self> {
        let paths = request
            .paths
            .iter()
            .map(|path| {
                repo.relative_path(path).ok_or_else(|| {
                    error(
                        ErrorCode::NoMatch,
                        format!("path {} is outside the repository", path.display()),
                    )
                    .with_problem("outside_repository")
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            paths,
            exclude: request.exclude.iter().map(|glob| BString::from(glob.as_str())).collect(),
        })
    }

    /// Whether the scope covers every path.
    pub(crate) fn is_full(&self) -> bool {
        self.paths.iter().all(|path| path.is_empty()) && self.exclude.is_empty()
    }

    fn under(path: &BStr, prefix: &BStr) -> bool {
        prefix.is_empty()
            || path == prefix
            || (path.starts_with(prefix) && path.get(prefix.len()) == Some(&b'/'))
    }

    /// Whether `path` is excluded by a pattern.
    pub(crate) fn excluded(&self, path: &BStr) -> bool {
        let name = path.rsplit_str(b"/").next().unwrap_or(path).as_bstr();
        self.exclude.iter().any(|pattern| {
            let target = if pattern.contains(&b'/') { path } else { name };
            gix::glob::wildmatch(pattern.as_bstr(), target, gix::glob::wildmatch::Mode::empty())
        })
    }

    /// Whether `path` is saved by the push.
    pub(crate) fn contains(&self, path: &BStr) -> bool {
        let listed = self.paths.is_empty()
            || self.paths.iter().any(|prefix| Self::under(path, prefix.as_bstr()));
        listed && !self.excluded(path)
    }

    /// Fails with `E_NO_MATCH` for a path that names nothing in `known`.
    fn check_matches(&self, repo: &Repo, known: &BTreeSet<BString>) -> Result<()> {
        for prefix in &self.paths {
            if !known.iter().any(|path| Self::under(path.as_bstr(), prefix.as_bstr())) {
                let root = quote(&repo.workdir.to_string_lossy());
                return Err(error(
                    ErrorCode::NoMatch,
                    format!("path `{prefix}` did not match any file known to git"),
                )
                .with_problem("pathspec_no_match")
                .with_fix(format!("git.status {root}")));
            }
        }
        Ok(())
    }
}

fn paths_of(state: &State) -> impl Iterator<Item = BString> + '_ {
    state.entries().iter().map(|entry| entry.path(state).to_owned())
}

/// Writes a commit object.
pub(crate) fn write_commit(
    repo: &Repo,
    identity: &Identity,
    tree: ObjectId,
    parents: &[ObjectId],
    message: &str,
) -> Result<ObjectId> {
    let commit = gix::objs::Commit {
        tree,
        parents: parents.iter().copied().collect(),
        author: identity.author.clone(),
        committer: identity.committer.clone(),
        encoding: None,
        message: message.into(),
        extra_headers: Vec::new(),
    };
    repo.repo
        .write_object(&commit)
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write commit", cause))
}

fn write_blob(repo: &Repo, data: &[u8]) -> Result<ObjectId> {
    repo.repo
        .write_blob(data)
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write blob", cause))
}

/// Tree of the index with the working tree versions of the tracked paths
/// in scope.
fn worktree_tree(
    wt: &mut Worktree<'_>,
    current: &State,
    head: &State,
    index_tree: ObjectId,
    scope: &Scope,
) -> Result<ObjectId> {
    let repo = wt.repo();
    let mut editor = repo
        .repo
        .edit_tree(index_tree)
        .map_err(|cause| io_error("cannot edit tree", cause))?;
    let candidates: BTreeSet<BString> = paths_of(current).chain(paths_of(head)).collect();
    for path in candidates.iter().filter(|path| scope.contains(path.as_bstr())) {
        let path = path.as_bstr();
        let entry = merged_entry(current, path);
        if entry.is_some_and(|entry| entry.mode == gix::index::entry::Mode::COMMIT) {
            continue;
        }
        let tracked_and_clean = match entry {
            Some(entry) if !entry.flags.contains(gix::index::entry::Flags::INTENT_TO_ADD) => {
                wt.matches(path, entry)?
            }
            _ => false,
        };
        if tracked_and_clean {
            continue;
        }
        let probe = wt.probe(path)?;
        match wt.stored_form(path, &probe)? {
            Some((data, mode)) => {
                let id = write_blob(repo, &data)?;
                editor
                    .upsert(path, entry_kind(mode), id)
                    .map_err(|cause| io_error("cannot edit tree", cause))?;
            }
            None => {
                editor
                    .remove(path)
                    .map_err(|cause| io_error("cannot edit tree", cause))?;
            }
        }
    }
    editor
        .write()
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write tree", cause))
}

/// Tree of the untracked files.
fn untracked_tree(wt: &mut Worktree<'_>, files: &[BString]) -> Result<ObjectId> {
    let repo = wt.repo();
    let empty = ObjectId::empty_tree(repo.repo.object_hash());
    let mut editor = repo
        .repo
        .edit_tree(empty)
        .map_err(|cause| io_error("cannot edit tree", cause))?;
    for path in files {
        let probe = wt.probe(path.as_bstr())?;
        if let Some((data, mode)) = wt.stored_form(path.as_bstr(), &probe)? {
            let id = write_blob(repo, &data)?;
            editor
                .upsert(path.as_bstr(), entry_kind(mode), id)
                .map_err(|cause| io_error("cannot edit tree", cause))?;
        }
    }
    editor
        .write()
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write tree", cause))
}

fn require_head(repo: &Repo, head: &HeadState) -> Result<ObjectId> {
    head.id.ok_or_else(|| {
        let root = quote(&repo.workdir.to_string_lossy());
        error(
            ErrorCode::GitRef,
            "cannot stash changes before the first commit; commit them instead",
        )
        .with_problem("unborn_branch")
        .with_fix(format!("git.commit {root} message: \"initial commit\" all: true"))
    })
}

/// Messages of the three stash commits.
struct Messages {
    index: String,
    untracked: String,
    work: String,
}

fn messages(repo: &Repo, head: &HeadState, head_id: ObjectId, message: Option<&str>) -> Result<Messages> {
    let label = head.branch_label();
    let summary = format!("{} {}", repo.short_id(head_id), repo.commit_subject(head_id)?);
    Ok(Messages {
        index: format!("index on {label}: {summary}"),
        untracked: format!("untracked files on {label}: {summary}"),
        work: message.map_or_else(
            || format!("WIP on {label}: {summary}"),
            |message| format!("On {label}: {message}"),
        ),
    })
}

/// Plans `git stash push`.
pub(crate) fn plan_push(repo: &Repo, request: &StashRequest, identity: &Identity) -> Result<PlannedStash> {
    let head = repo.head()?;
    let head_id = require_head(repo, &head)?;
    let current = index::load(repo)?;
    if index::has_conflicts(&current) {
        return Err(error(
            ErrorCode::GitInProgress,
            "cannot stash while the index has unresolved conflicts; resolve them first",
        )
        .with_problem("unmerged_paths"));
    }
    let scope = Scope::new(repo, request)?;
    let head_tree = repo.commit_tree(head_id)?;
    let head_state = index::from_tree(repo, head_tree)?;
    let mut wt = Worktree::new(repo)?;
    let untracked: Vec<BString> = if request.include_untracked || request.all {
        wt.untracked(&current, request.all)?
    } else {
        Vec::new()
    };
    let known: BTreeSet<BString> = paths_of(&current)
        .chain(paths_of(&head_state))
        .chain(untracked.iter().cloned())
        .collect();
    scope.check_matches(repo, &known)?;
    let untracked: Vec<BString> = untracked
        .into_iter()
        .filter(|path| scope.contains(path.as_bstr()))
        .collect();
    let index_tree = index::write_tree(repo, &current)?;
    let work_tree = worktree_tree(&mut wt, &current, &head_state, index_tree, &scope)?;
    let mut outcome = StashOutcome {
        repo: repo.workdir.clone(),
        action: StashAction::Push,
        head: Some(head_id),
        branch: head.branch.as_ref().map(|name| name.shorten().to_string()),
        ..StashOutcome::default()
    };
    if index_tree == head_tree && work_tree == index_tree && untracked.is_empty() {
        outcome.nothing_to_do = true;
        return Ok(PlannedStash::new(TxPlan::new(), None, outcome));
    }
    let texts = messages(repo, &head, head_id, request.message.as_deref())?;
    let index_commit = write_commit(repo, identity, index_tree, &[head_id], &texts.index)?;
    let mut parents = vec![head_id, index_commit];
    if !untracked.is_empty() {
        let tree = untracked_tree(&mut wt, &untracked)?;
        parents.push(write_commit(repo, identity, tree, &[], &texts.untracked)?);
    }
    let work_commit = write_commit(repo, identity, work_tree, &parents, &texts.work)?;
    let mut tx = TxPlan::new();
    let null = ObjectId::null(repo.repo.object_hash());
    let previous = read_ref(repo, STASH_REF)?.unwrap_or(null);
    plan_ref_write(&mut tx, repo, STASH_REF, work_commit)?;
    plan_reflog_append(
        &mut tx,
        repo,
        STASH_REF,
        reflog_line(previous, work_commit, &identity.committer, &texts.work),
    )?;
    let target = if request.keep_index {
        index::from_tree(repo, index_tree)?
    } else {
        head_state
    };
    let full = scope.is_full();
    let in_scope = |path: &BStr| scope.contains(path);
    let checkout = plan_checkout(
        &mut wt,
        &Checkout {
            current: &current,
            head: None,
            target: &target,
            policy: Policy::Hard,
            scope: (!full).then_some(&in_scope as &dyn Fn(&BStr) -> bool),
            extra_removals: &untracked,
        },
    )?;
    tx.extend(checkout.tx);
    tx.locked_write(&repo.index_path(), None)?;
    if full {
        plan_head_update(&mut tx, repo, &head, head_id, &identity.committer, "reset: moving to HEAD")?;
        plan_ref_write(&mut tx, repo, "ORIG_HEAD", head_id)?;
    }
    outcome.created = Some(work_commit);
    outcome.written = super::strings(checkout.written);
    outcome.removed = super::strings(checkout.removed);
    Ok(PlannedStash::new(tx, Some(checkout.index), outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(paths: &[&str], exclude: &[&str]) -> Scope {
        Scope {
            paths: paths.iter().map(|path| BString::from(*path)).collect(),
            exclude: exclude.iter().map(|glob| BString::from(*glob)).collect(),
        }
    }

    #[test]
    fn scope_matches_files_and_directories() {
        let scope = scope(&["src", "a.txt"], &[]);
        assert!(scope.contains(b"src/main.rs".as_bstr()));
        assert!(scope.contains(b"a.txt".as_bstr()));
        assert!(!scope.contains(b"srcx/main.rs".as_bstr()));
        assert!(!scope.contains(b"b.txt".as_bstr()));
        assert!(!scope.is_full());
    }

    #[test]
    fn exclusion_matches_names_and_paths() {
        let scope = scope(&[], &["*.env", "secrets/**"]);
        assert!(scope.excluded(b"config/.prod.env".as_bstr()));
        assert!(scope.excluded(b"secrets/key".as_bstr()));
        assert!(!scope.excluded(b"src/env.rs".as_bstr()));
        assert!(scope.contains(b"src/env.rs".as_bstr()));
        assert!(!scope.is_full());
    }

    #[test]
    fn empty_scope_covers_everything() {
        assert!(scope(&[], &[]).is_full());
        assert!(scope(&[], &[]).contains(b"any/path".as_bstr()));
    }
}
