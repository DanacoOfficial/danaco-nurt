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
use nurt_git::apply::{ApplyContext, PathSource, PathsRequest, plan_paths};
use nurt_git::repo::GitRepo;
use nurt_git::repo::index::index_to_tree;

use super::entries::plan_create;
use super::{StashAction, StashOutcome, StashRequest};
use crate::reset::execute::{Change, move_head, ref_tx, remove_files, touched};
use crate::reset::ident::Identity;
use crate::reset::index::{self, entry_kind, merged_entry};
use crate::reset::repo::{
    HeadState, commit_subject, commit_tree, error, head, io_error, relative_path, root_literal,
};
use crate::reset::worktree::Worktree;

/// Paths a push is limited to.
pub(crate) struct Scope {
    paths: Vec<BString>,
    exclude: Vec<BString>,
}

impl Scope {
    /// Scope of `request`, with paths relative to the root of `repo`.
    pub(crate) fn new(repo: &GitRepo, request: &StashRequest) -> Result<Self> {
        let paths = request
            .paths
            .iter()
            .map(|path| {
                relative_path(repo, path).ok_or_else(|| {
                    error(
                        ErrorCode::InvalidValue,
                        format!("path {} is outside the repository", path.display()),
                    )
                    .with_problem("outside_repository")
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            paths,
            exclude: request
                .exclude
                .iter()
                .map(|glob| BString::from(glob.as_str()))
                .collect(),
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
            gix::glob::wildmatch(
                pattern.as_bstr(),
                target,
                gix::glob::wildmatch::Mode::empty(),
            )
        })
    }

    /// Whether `path` is saved by the push.
    pub(crate) fn contains(&self, path: &BStr) -> bool {
        let listed = self.paths.is_empty()
            || self
                .paths
                .iter()
                .any(|prefix| Self::under(path, prefix.as_bstr()));
        listed && !self.excluded(path)
    }

    /// Fails with `E_NO_MATCH` for a path that names nothing in `known`.
    fn check_matches(&self, repo: &GitRepo, known: &BTreeSet<BString>) -> Result<()> {
        for prefix in &self.paths {
            if !known
                .iter()
                .any(|path| Self::under(path.as_bstr(), prefix.as_bstr()))
            {
                return Err(error(
                    ErrorCode::NoMatch,
                    format!("path `{prefix}` did not match any file known to git"),
                )
                .with_problem("pathspec_no_match")
                .with_fix(format!("git.status {}", root_literal(repo))));
            }
        }
        Ok(())
    }
}

fn paths_of(state: &State) -> impl Iterator<Item = BString> + '_ {
    state
        .entries()
        .iter()
        .map(|entry| entry.path(state).to_owned())
}

/// Writes a commit object.
pub(crate) fn write_commit(
    repo: &GitRepo,
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
    repo.gix()
        .write_object(&commit)
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write commit", cause))
}

fn write_blob(repo: &GitRepo, data: &[u8]) -> Result<ObjectId> {
    repo.gix()
        .write_blob(data)
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write blob", cause))
}

/// Tree of the index with the working tree versions of the tracked paths
/// in scope.
fn worktree_tree(
    repo: &GitRepo,
    wt: &mut Worktree<'_>,
    current: &State,
    head: &State,
    index_tree: ObjectId,
    scope: &Scope,
) -> Result<ObjectId> {
    let mut editor = repo
        .gix()
        .edit_tree(index_tree)
        .map_err(|cause| io_error("cannot edit tree", cause))?;
    let candidates: BTreeSet<BString> = paths_of(current).chain(paths_of(head)).collect();
    for path in candidates
        .iter()
        .filter(|path| scope.contains(path.as_bstr()))
    {
        let path = path.as_bstr();
        let entry = merged_entry(current, path);
        if entry.is_some_and(|entry| entry.mode == gix::index::entry::Mode::COMMIT) {
            continue;
        }
        let tracked_and_clean = match entry {
            Some(entry)
                if !entry
                    .flags
                    .contains(gix::index::entry::Flags::INTENT_TO_ADD) =>
            {
                wt.matches(path, entry)?
            }
            _ => false,
        };
        if tracked_and_clean {
            continue;
        }
        match wt.stored_form(path)? {
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
fn untracked_tree(repo: &GitRepo, wt: &mut Worktree<'_>, files: &[BString]) -> Result<ObjectId> {
    let empty = ObjectId::empty_tree(repo.gix().object_hash());
    let mut editor = repo
        .gix()
        .edit_tree(empty)
        .map_err(|cause| io_error("cannot edit tree", cause))?;
    for path in files {
        if let Some((data, mode)) = wt.stored_form(path.as_bstr())? {
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

/// Number of changed files left out by the exclusion patterns: staged or
/// unstaged changes of tracked files and untracked files.
fn count_excluded(
    wt: &mut Worktree<'_>,
    scope: &Scope,
    current: &State,
    head: &State,
    untracked: &[BString],
) -> Result<usize> {
    let mut count = untracked
        .iter()
        .filter(|path| scope.excluded(path.as_bstr()))
        .count();
    let tracked: BTreeSet<BString> = paths_of(current)
        .chain(paths_of(head))
        .filter(|path| scope.excluded(path.as_bstr()))
        .collect();
    for path in &tracked {
        let index = merged_entry(current, path.as_bstr());
        let staged = match (index, merged_entry(head, path.as_bstr())) {
            (Some(index), Some(head)) => index.id != head.id || index.mode != head.mode,
            (None, None) => false,
            _ => true,
        };
        let changed = staged
            || match index {
                Some(entry) => !wt.matches(path.as_bstr(), entry)?,
                None => false,
            };
        count += usize::from(changed);
    }
    Ok(count)
}

fn require_head(repo: &GitRepo, head: &HeadState) -> Result<ObjectId> {
    head.id.ok_or_else(|| {
        error(
            ErrorCode::GitRef,
            "cannot stash changes before the first commit; commit them instead",
        )
        .with_problem("unborn_branch")
        .with_fix(format!(
            "git.commit {} message: \"initial commit\" all: true",
            root_literal(repo)
        ))
    })
}

/// Messages of the three stash commits.
struct Messages {
    index: String,
    untracked: String,
    work: String,
}

fn messages(
    repo: &GitRepo,
    head: &HeadState,
    head_id: ObjectId,
    message: Option<&str>,
) -> Result<Messages> {
    let label = head.branch_label();
    let summary = format!(
        "{} {}",
        repo.short_id(&head_id),
        commit_subject(repo, head_id)?
    );
    Ok(Messages {
        index: format!("index on {label}: {summary}\n"),
        untracked: format!("untracked files on {label}: {summary}\n"),
        work: message.map_or_else(
            || format!("WIP on {label}: {summary}"),
            |message| format!("On {label}: {message}"),
        ),
    })
}

/// Whether a path of `scope` differs between `HEAD`, the index and the
/// working tree version of the tracked files.
fn scope_changed(scope: &Scope, head: &State, current: &State, work: &State) -> bool {
    let paths: BTreeSet<BString> = paths_of(head)
        .chain(paths_of(current))
        .chain(paths_of(work))
        .filter(|path| scope.contains(path.as_bstr()))
        .collect();
    paths.iter().any(|path| {
        let key = |state: &State| merged_entry(state, path.as_bstr()).map(|e| (e.id, e.mode));
        key(head) != key(current) || key(current) != key(work)
    })
}

/// Pathspecs naming exactly the tracked paths of `scope`.
fn literal_specs(current: &State, head: &State, scope: &Scope) -> Vec<BString> {
    paths_of(current)
        .chain(paths_of(head))
        .filter(|path| scope.contains(path.as_bstr()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|path| {
            let mut spec = BString::from(":(literal)");
            spec.extend_from_slice(&path);
            spec
        })
        .collect()
}

/// Plans `git stash push`.
pub(crate) fn plan_push(
    repo: &GitRepo,
    request: &StashRequest,
    identity: &Identity,
) -> Result<(Change, StashOutcome)> {
    let head = head(repo)?;
    let head_id = require_head(repo, &head)?;
    let context = ApplyContext::open(repo.gix())?;
    let current = context.index();
    if index::has_conflicts(current) {
        return Err(error(
            ErrorCode::GitInProgress,
            "cannot stash while the index has unresolved conflicts; resolve them first",
        )
        .with_problem("unmerged_paths"));
    }
    let scope = Scope::new(repo, request)?;
    let head_tree = commit_tree(repo, head_id)?;
    let head_state = index::from_tree(repo, head_tree)?;
    let mut wt = Worktree::new(repo)?;
    let untracked: Vec<BString> = if request.include_untracked {
        wt.untracked(current)?
    } else {
        Vec::new()
    };
    let known: BTreeSet<BString> = paths_of(current)
        .chain(paths_of(&head_state))
        .chain(untracked.iter().cloned())
        .collect();
    scope.check_matches(repo, &known)?;
    let excluded = count_excluded(&mut wt, &scope, current, &head_state, &untracked)?;
    let untracked: Vec<BString> = untracked
        .into_iter()
        .filter(|path| scope.contains(path.as_bstr()))
        .collect();
    let index_tree = index_to_tree(repo, current)?;
    let work_tree = worktree_tree(repo, &mut wt, current, &head_state, index_tree, &scope)?;
    let mut outcome = StashOutcome {
        repo: repo.root().to_path_buf(),
        action: StashAction::Push,
        head: Some(head_id),
        branch: head.short_branch(),
        excluded,
        ..StashOutcome::default()
    };
    let mut change = Change::new();
    change.objects = true;
    let work_state = index::from_tree(repo, work_tree)?;
    if untracked.is_empty() && !scope_changed(&scope, &head_state, current, &work_state) {
        outcome.nothing_to_do = true;
        return Ok((change, outcome));
    }
    let texts = messages(repo, &head, head_id, request.message.as_deref())?;
    let index_commit = write_commit(repo, identity, index_tree, &[head_id], &texts.index)?;
    let mut parents = vec![head_id, index_commit];
    if !untracked.is_empty() {
        let tree = untracked_tree(repo, &mut wt, &untracked)?;
        parents.push(write_commit(repo, identity, tree, &[], &texts.untracked)?);
    }
    let work_commit = write_commit(repo, identity, work_tree, &parents, &texts.work)?;
    let full = scope.is_full();
    let specs = if full {
        Vec::new()
    } else {
        literal_specs(current, &head_state, &scope)
    };
    if full || !specs.is_empty() {
        change.plan = plan_paths(
            &context,
            &PathsRequest {
                source: PathSource::Tree(head_tree),
                pathspec: specs,
                to_index: true,
                to_worktree: true,
            },
        )?;
    }
    remove_files(&mut change.plan, repo.root(), &untracked)?;
    let (written, removed) = touched(&change.plan);
    let mut refs = ref_tx(repo, &identity.committer)?;
    plan_create(&mut refs, work_commit, &texts.work)?;
    if full {
        move_head(&mut refs, &head, head_id, "reset: moving to HEAD")?;
    }
    change.set_refs(refs)?;
    outcome.created = Some(work_commit);
    outcome.written = written;
    outcome.removed = removed;
    Ok((change, outcome))
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
