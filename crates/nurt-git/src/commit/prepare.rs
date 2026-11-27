// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::commit::prepare
// Responsibility:   Planning of a commit: the content selected for the tree, the parents, the identities, the message settings and the checks that run before any change.
// Dependencies:     bstr, gix, gix_ref, nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Planning of a commit: the content selected for the tree, the parents, the
//! identities, the message settings and the checks that run before any
//! change.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix_ref::FullName;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::{Notable, NotableEntry};
use sha2::{Digest, Sha256};

use super::access::{self, Worktree};
use super::identity::{self, Identities, IdentityInput};
use super::{CommitRequest, Selection, render_call};
use crate::hooks::{Hook, HookDir};
use crate::message::Cleanup;
use crate::repo::GitRepo;
use crate::repo::error::quote_path;
use crate::repo::index::{IndexFile, index_to_tree};
use crate::repo::pathspec::PathFilter;

/// Files of the merge state, removed when a merge is concluded.
pub(crate) const MERGE_FILES: [&str; 4] = ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "AUTO_MERGE"];

/// Entries of the repository directory that mark an operation a commit does
/// not conclude.
const OPERATION_MARKERS: [&str; 5] = [
    "rebase-merge",
    "rebase-apply",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "BISECT_LOG",
];

/// Hooks that run around a commit.
pub(crate) const COMMIT_HOOKS: [Hook; 4] = [
    Hook::PreCommit,
    Hook::PrepareCommitMsg,
    Hook::CommitMsg,
    Hook::PostCommit,
];

/// Comment character of commit messages when `core.commentChar` is unset.
const DEFAULT_COMMENT: char = '#';

/// Bytes of the SHA-256 digest kept in a state token.
const STATE_TOKEN_BYTES: usize = 16;

/// Kind of commit, which selects the reflog message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReflogKind {
    /// First commit of a branch.
    Initial,
    /// Commit on top of `HEAD`.
    Normal,
    /// Replacement of the commit at `HEAD`.
    Amend,
    /// Commit that concludes a merge.
    Merge,
}

impl ReflogKind {
    /// Reflog message of the commit, as `git commit` writes it.
    pub(crate) fn message(self, subject: &str) -> String {
        match self {
            Self::Initial => format!("commit (initial): {subject}"),
            Self::Normal => format!("commit: {subject}"),
            Self::Amend => format!("commit (amend): {subject}"),
            Self::Merge => format!("commit (merge): {subject}"),
        }
    }
}

/// File of the merge state with the content read while planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeFile {
    /// Location.
    pub(crate) path: PathBuf,
    /// Content.
    pub(crate) content: Vec<u8>,
}

/// Everything a commit needs after planning.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    /// The request.
    pub(crate) request: CommitRequest,
    /// Root of the repository shown to the user.
    pub(crate) root: PathBuf,
    /// Branch `HEAD` points to; `None` when detached.
    pub(crate) branch: Option<FullName>,
    /// Commit of `HEAD`; `None` on an unborn branch.
    pub(crate) head: Option<ObjectId>,
    /// Parents of the new commit.
    pub(crate) parents: Vec<ObjectId>,
    /// Tree of the first parent; `None` without parents.
    pub(crate) base_tree: Option<ObjectId>,
    /// Index state the tree is written from; `None` when only the message
    /// changes.
    pub(crate) state: Option<gix::index::State>,
    /// Tree of the new commit.
    pub(crate) tree: ObjectId,
    /// New content of the index of the working tree, when it changes.
    pub(crate) index: Option<IndexFile>,
    /// Author and committer.
    pub(crate) identities: Identities,
    /// Merge state concluded by the commit; empty without a merge.
    pub(crate) merge: Vec<MergeFile>,
    /// Kind of commit.
    pub(crate) kind: ReflogKind,
    /// Cleanup of the message (`commit.cleanup`).
    pub(crate) cleanup: Cleanup,
    /// Comment character (`core.commentChar`).
    pub(crate) comment: char,
    /// Sign the commit (`sign`, otherwise `commit.gpgSign`).
    pub(crate) sign: bool,
    /// Value of the `encoding` header (`i18n.commitEncoding`).
    pub(crate) encoding: Option<BString>,
    /// Hooks to run; `None` when none runs.
    pub(crate) hooks: Option<HookDir>,
    /// Unusual effects known before execution.
    pub(crate) notes: Vec<NotableEntry>,
    /// Planned changes of files.
    pub(crate) changes: Vec<Change>,
}

impl Prepared {
    /// Tree the new tree is compared with by the empty-commit check.
    pub(crate) fn parent_tree(&self) -> ObjectId {
        self.base_tree
            .unwrap_or_else(|| ObjectId::empty_tree(self.tree.kind()))
    }

    /// Fails with `E_GIT_NOTHING_TO_COMMIT` when the commit would record no
    /// change and neither an empty commit nor a merge allows that.
    pub(crate) fn check_not_empty(&self) -> Result<()> {
        if self.request.allow_empty || !self.merge.is_empty() || self.tree != self.parent_tree() {
            return Ok(());
        }
        Err(nothing_to_commit(&self.root, &self.request))
    }
}

/// Plans a commit without changing the repository; trees and blobs of the
/// selected content are added to the object store.
pub(crate) fn prepare(repo: &GitRepo, request: &CommitRequest) -> Result<Prepared> {
    if let Some(expected) = &request.expect_state {
        check_state(repo, expected)?;
    }
    let root = repo.root().to_path_buf();
    let branch = repo.head_ref()?;
    let head = repo.head_id()?;
    let (merge_heads, merge) = merge_state(repo)?;
    check_combination(repo, request, !merge.is_empty())?;
    let amended = if request.amend {
        Some(amended_commit(repo, head)?)
    } else {
        None
    };
    let (parents, base_tree, head_tree) = match &amended {
        Some(amended) => (
            amended.parents.clone(),
            amended.parent_tree,
            Some(amended.tree),
        ),
        None => {
            let head_tree = head.map(|id| commit_tree(repo, id)).transpose()?;
            let mut parents: Vec<ObjectId> = head.into_iter().collect();
            parents.extend(merge_heads);
            (parents, head_tree, head_tree)
        }
    };
    let selected = select(repo, request, head_tree)?;
    let identities = identity::resolve(
        repo,
        repo.ctx(),
        IdentityInput {
            author: request.author.as_deref(),
            amended: amended.as_ref().map(|amended| &amended.author),
        },
    )?;
    let kind = match (&amended, merge.is_empty(), head) {
        (Some(_), _, _) => ReflogKind::Amend,
        (None, false, _) => ReflogKind::Merge,
        (None, true, None) => ReflogKind::Initial,
        (None, true, Some(_)) => ReflogKind::Normal,
    };
    let mut notes = Vec::new();
    let hooks = hook_dir(repo, request, &mut notes);
    notes.extend(state_notes(
        repo,
        branch.is_none(),
        amended.as_ref().map(|amended| amended.id),
        selected.excluded,
    ));
    let mut prepared = Prepared {
        request: request.clone(),
        root,
        branch,
        head,
        parents,
        base_tree,
        state: selected.state,
        tree: selected.tree,
        index: selected.index,
        identities,
        merge,
        kind,
        cleanup: cleanup_mode(repo)?,
        comment: comment_char(repo),
        sign: request
            .sign
            .unwrap_or_else(|| repo.config_bool("commit.gpgSign").unwrap_or(false)),
        encoding: commit_encoding(repo),
        hooks,
        notes,
        changes: Vec::new(),
    };
    prepared.check_not_empty()?;
    prepared.changes = planned_changes(repo, &prepared);
    Ok(prepared)
}

/// The error of a commit that would record no change.
pub(crate) fn nothing_to_commit(root: &std::path::Path, request: &CommitRequest) -> Error {
    let (problem, message) = if request.amend {
        (
            "amend_empty",
            "amending would leave the commit without changes; allow an empty commit or select changes",
        )
    } else {
        (
            "nothing_to_commit",
            "the commit would record no change; stage changes, select them with `all` or `paths`, or allow an empty commit",
        )
    };
    let mut error = Error::new(ErrorCode::GitNothingToCommit, message)
        .with_problem(problem)
        .with_fix(render_call(
            root,
            request,
            &request.selection,
            &["allow_empty: true"],
        ));
    if request.selection == Selection::Staged {
        error = error.with_fix(render_call(root, request, &Selection::All, &[]));
    }
    error.with_next_step(format!("git.status {}", quote_path(root)))
}

/// Hash of the state a commit depends on: `HEAD`, the merge state and the
/// index.
///
/// The token is compared with the `expect_state` argument.
pub(crate) fn state_token(repo: &GitRepo) -> Result<String> {
    let mut hasher = Sha256::new();
    let head_ref = repo.head_ref()?;
    hasher.update(
        head_ref
            .as_ref()
            .map(|name| name.as_bstr().to_vec())
            .unwrap_or_default(),
    );
    hasher.update([0]);
    hasher.update(repo.head_id()?.map(|id| id.to_string()).unwrap_or_default());
    hasher.update([0]);
    for file in [repo.git_dir().join("MERGE_HEAD"), repo.index_path()] {
        match std::fs::read(&file) {
            Ok(bytes) => hasher.update(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(crate::repo::error::io(&file, &error)),
        }
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    Ok(hex::encode(&digest[..STATE_TOKEN_BYTES]))
}

fn check_state(repo: &GitRepo, expected: &str) -> Result<()> {
    let actual = state_token(repo)?;
    if actual == expected.trim() {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::Stale,
        "the repository changed since the state token was read; read the state again and repeat the call",
    )
    .with_problem("state_changed")
    .with_found(expected.to_owned())
    .with_next_step(format!("git.status {}", quote_path(repo.root()))))
}

fn check_combination(repo: &GitRepo, request: &CommitRequest, merging: bool) -> Result<()> {
    if matches!(request.selection, Selection::All | Selection::Paths(_)) {
        access::require_workdir(repo)?;
    }
    if !merging {
        return Ok(());
    }
    let root = repo.root();
    if request.amend {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            "a merge is in progress and cannot be amended; conclude it with `staged: true` first",
        )
        .with_problem("amend_during_merge")
        .with_fix(render_call(
            root,
            &CommitRequest {
                amend: false,
                ..request.clone()
            },
            &Selection::Staged,
            &[],
        )));
    }
    if matches!(request.selection, Selection::Paths(_)) {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            "a merge cannot be concluded with selected paths; commit with `staged: true` or `all: true`",
        )
        .with_problem("partial_merge_commit")
        .with_fix(render_call(root, request, &Selection::Staged, &[])));
    }
    Ok(())
}

/// The commit replaced by `amend`.
struct Amended {
    id: ObjectId,
    tree: ObjectId,
    parents: Vec<ObjectId>,
    parent_tree: Option<ObjectId>,
    author: gix::actor::Signature,
}

fn read_error(what: &str, error: impl std::fmt::Display) -> Error {
    access::lib_error(what, error)
}

fn commit_tree(repo: &GitRepo, id: ObjectId) -> Result<ObjectId> {
    repo.gix()
        .find_commit(id)
        .map_err(|error| read_error(&format!("read commit {id}"), error))?
        .tree_id()
        .map(gix::Id::detach)
        .map_err(|error| read_error(&format!("read the tree of commit {id}"), error))
}

fn amended_commit(repo: &GitRepo, head: Option<ObjectId>) -> Result<Amended> {
    let id = head.ok_or_else(|| {
        Error::new(
            ErrorCode::GitRevision,
            "the branch has no commit to amend; commit without `amend`",
        )
        .with_problem("nothing_to_amend")
    })?;
    let commit = repo
        .gix()
        .find_commit(id)
        .map_err(|error| read_error(&format!("read commit {id}"), error))?;
    let tree = commit
        .tree_id()
        .map_err(|error| read_error(&format!("read the tree of commit {id}"), error))?
        .detach();
    let parents: Vec<ObjectId> = commit.parent_ids().map(gix::Id::detach).collect();
    let author = commit
        .author()
        .map_err(|error| read_error(&format!("read the author of commit {id}"), error))?
        .to_owned()
        .map_err(|error| read_error(&format!("read the author of commit {id}"), error))?;
    let parent_tree = parents
        .first()
        .map(|parent| commit_tree(repo, *parent))
        .transpose()?;
    Ok(Amended {
        id,
        tree,
        parents,
        parent_tree,
        author,
    })
}

fn merge_state(repo: &GitRepo) -> Result<(Vec<ObjectId>, Vec<MergeFile>)> {
    let head_file = repo.git_dir().join(MERGE_FILES[0]);
    let content = match std::fs::read(&head_file) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => return Err(crate::repo::error::io(&head_file, &error)),
    };
    let mut heads = Vec::new();
    for line in content
        .lines()
        .map(ByteSlice::trim)
        .filter(|line| !line.is_empty())
    {
        let id = ObjectId::from_hex(line).map_err(|_| {
            Error::new(
                ErrorCode::GitInProgress,
                format!(
                    "{} does not hold commit identifiers; abort the merge and merge again",
                    head_file.display()
                ),
            )
            .with_problem("invalid_merge_head")
        })?;
        heads.push(id);
    }
    let mut files = Vec::new();
    for name in MERGE_FILES {
        let path = repo.git_dir().join(name);
        match std::fs::read(&path) {
            Ok(content) => files.push(MergeFile { path, content }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(crate::repo::error::io(&path, &error)),
        }
    }
    Ok((heads, files))
}

/// Content selected for the tree.
struct Selected {
    state: Option<gix::index::State>,
    index: Option<IndexFile>,
    tree: ObjectId,
    excluded: u64,
}

fn select(
    repo: &GitRepo,
    request: &CommitRequest,
    head_tree: Option<ObjectId>,
) -> Result<Selected> {
    let exclude = PathFilter::new(repo.root(), &[], &request.exclude)?;
    let excluding = !request.exclude.is_empty();
    match &request.selection {
        Selection::MessageOnly => {
            let tree = head_tree.unwrap_or_else(|| ObjectId::empty_tree(repo.gix().object_hash()));
            Ok(Selected {
                state: None,
                index: None,
                tree,
                excluded: 0,
            })
        }
        Selection::Staged => {
            let mut state = repo.read_index()?.state;
            let excluded = if excluding {
                revert_excluded(repo, &mut state, head_tree, &exclude)?
            } else {
                0
            };
            let tree = index_to_tree(repo, &state)?;
            Ok(Selected {
                state: Some(state),
                index: None,
                tree,
                excluded,
            })
        }
        Selection::All => {
            let mut index = repo.read_index()?;
            let excluded = stage_all(repo, &mut index.state, &exclude)?;
            let tree = index_to_tree(repo, &index.state)?;
            Ok(Selected {
                state: Some(index.state.clone()),
                index: Some(index),
                tree,
                excluded,
            })
        }
        Selection::Paths(paths) => {
            let mut index = repo.read_index()?;
            let mut state = access::state_of_tree(repo, head_tree)?;
            let excluded = stage_paths(repo, &mut index.state, &mut state, paths, &exclude)?;
            let tree = index_to_tree(repo, &state)?;
            Ok(Selected {
                state: Some(state),
                index: Some(index),
                tree,
                excluded,
            })
        }
    }
}

/// Puts the `HEAD` version of every excluded path back into `state`;
/// returns the number of paths whose staged change was left out.
fn revert_excluded(
    repo: &GitRepo,
    state: &mut gix::index::State,
    head_tree: Option<ObjectId>,
    exclude: &PathFilter,
) -> Result<u64> {
    let base = access::tree_files(repo, head_tree)?;
    let paths: BTreeSet<BString> = access::index_paths(state)
        .into_iter()
        .chain(base.keys().cloned())
        .filter(|path| exclude.excluded(path.as_ref()))
        .collect();
    let mut excluded = 0;
    for path in paths {
        let current = access::staged_entry(state, path.as_ref());
        let original = base.get(&path).copied();
        if current == original && !has_conflict(state, path.as_ref()) {
            continue;
        }
        excluded += 1;
        match original {
            Some((id, kind)) => {
                access::upsert_entry(state, path.as_ref(), id, kind, Default::default())
            }
            None => access::remove_entry(state, path.as_ref()),
        }
    }
    Ok(excluded)
}

fn has_conflict(state: &gix::index::State, path: &BStr) -> bool {
    state.entry_range(path).is_some_and(|range| {
        state.entries()[range]
            .iter()
            .any(|entry| entry.stage_raw() != 0)
    })
}

/// A path of the working tree compared with its index entry.
struct Observed {
    /// The file, when it exists.
    file: Option<access::WorkFile>,
    /// The file equals the stage-0 entry, or both are absent.
    same: bool,
    /// The entry is a gitlink and its directory exists, so the entry stays as
    /// it is.
    gitlink: bool,
}

fn observe(
    repo: &GitRepo,
    worktree: &mut Worktree<'_>,
    state: &gix::index::State,
    path: &BStr,
) -> Result<Observed> {
    let entry = access::staged_entry(state, path);
    let conflicted = has_conflict(state, path);
    let file = worktree.read(path, entry.map(|(_, kind)| kind))?;
    let gitlink = matches!(entry, Some((_, gix::objs::tree::EntryKind::Commit)))
        && file.is_none()
        && std::fs::symlink_metadata(worktree.absolute(path)).is_ok_and(|meta| meta.is_dir());
    let same = match &file {
        None => gitlink || (entry.is_none() && !conflicted),
        Some(file) => {
            !conflicted
                && entry
                    == Some((
                        access::blob_id(repo.gix().object_hash(), &file.data)?,
                        file.kind,
                    ))
        }
    };
    Ok(Observed {
        file,
        same,
        gitlink,
    })
}

fn write_blob(repo: &GitRepo, data: &[u8]) -> Result<ObjectId> {
    repo.gix()
        .write_blob(data)
        .map(gix::Id::detach)
        .map_err(|error| access::lib_error("write a file to the object store", error))
}

/// Stages every change of the working tree: modified and deleted tracked
/// files and untracked files that are not ignored. Returns the number of
/// changed paths left out by `exclude`.
fn stage_all(repo: &GitRepo, state: &mut gix::index::State, exclude: &PathFilter) -> Result<u64> {
    let mut worktree = Worktree::new(repo)?;
    let mut paths = access::index_paths(state);
    paths.extend(worktree.untracked(state, &PathFilter::all())?);
    let mut excluded = 0;
    for path in paths {
        repo.ctx().checkpoint()?;
        let observed = observe(repo, &mut worktree, state, path.as_ref())?;
        if observed.same {
            continue;
        }
        if exclude.excluded(path.as_ref()) {
            excluded += 1;
            continue;
        }
        match observed.file {
            None => access::remove_entry(state, path.as_ref()),
            Some(file) => {
                let id = write_blob(repo, &file.data)?;
                access::upsert_entry(state, path.as_ref(), id, file.kind, file.stat);
            }
        }
    }
    Ok(excluded)
}

/// Records the working tree content of the selected paths in `index` and in
/// `commit`, the state built from `HEAD`, as `git commit --only` does.
/// Returns the number of changed paths left out by `exclude`.
fn stage_paths(
    repo: &GitRepo,
    index: &mut gix::index::State,
    commit: &mut gix::index::State,
    paths: &[String],
    exclude: &PathFilter,
) -> Result<u64> {
    let root = access::require_workdir(repo)?;
    let filter = PathFilter::new(root, paths, &[])?;
    let mut worktree = Worktree::new(repo)?;
    let mut candidates: BTreeSet<BString> = access::index_paths(index)
        .into_iter()
        .chain(access::index_paths(commit))
        .filter(|path| filter.matches(path.as_ref()))
        .collect();
    candidates.extend(worktree.untracked(index, &filter)?);
    for literal in filter.literal_paths() {
        if !candidates
            .iter()
            .any(|path| is_below(path.as_ref(), literal))
        {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!(
                    "path `{literal}` matches no file of the working tree or the index; check the spelling"
                ),
            )
            .with_problem("path_not_found")
            .with_found(literal.to_str_lossy().into_owned())
            .with_next_step(format!("git.status {}", quote_path(repo.root()))));
        }
    }
    let mut excluded = 0;
    for path in candidates {
        repo.ctx().checkpoint()?;
        let observed = observe(repo, &mut worktree, index, path.as_ref())?;
        if exclude.excluded(path.as_ref()) {
            excluded += u64::from(!observed.same);
            continue;
        }
        match observed.file {
            None if observed.gitlink => {}
            None => {
                access::remove_entry(index, path.as_ref());
                access::remove_entry(commit, path.as_ref());
            }
            Some(file) => {
                let id = write_blob(repo, &file.data)?;
                if !observed.same {
                    access::upsert_entry(index, path.as_ref(), id, file.kind, file.stat);
                }
                access::upsert_entry(commit, path.as_ref(), id, file.kind, file.stat);
            }
        }
    }
    Ok(excluded)
}

/// Whether `path` is `prefix` or lies below it.
fn is_below(path: &BStr, prefix: &BStr) -> bool {
    prefix.is_empty()
        || path == prefix
        || (path.starts_with(prefix.as_bytes()) && path.get(prefix.len()) == Some(&b'/'))
}

fn hook_dir(
    repo: &GitRepo,
    request: &CommitRequest,
    notes: &mut Vec<NotableEntry>,
) -> Option<HookDir> {
    let dir = HookDir::new(
        repo.common_dir(),
        repo.workdir(),
        repo.config_string("core.hooksPath").as_deref(),
        repo.env().home(),
    );
    if !dir.has_any(&COMMIT_HOOKS) {
        return None;
    }
    let skipped = |detail: &str| {
        let mut note = NotableEntry::new(Notable::HooksSkipped);
        note.path = Some(dir.dir().display().to_string());
        note.detail = Some(detail.to_owned());
        note
    };
    if !request.hooks {
        notes.push(skipped(
            "the hooks of the repository were not run because `hooks` is false",
        ));
        return None;
    }
    if repo.ctx().process().is_none() {
        notes.push(skipped("the hooks of the repository were not run because programs cannot be started in this context"));
        return None;
    }
    Some(dir)
}

fn state_notes(
    repo: &GitRepo,
    detached: bool,
    amended: Option<ObjectId>,
    excluded: u64,
) -> Vec<NotableEntry> {
    let mut notes = Vec::new();
    if let Some(id) = amended {
        let mut note = NotableEntry::new(Notable::HistoryRewritten);
        note.detail = Some(format!("commit {id} is replaced"));
        notes.push(note);
    }
    if detached {
        let mut note = NotableEntry::new(Notable::CommitDetachedHead);
        note.detail =
            Some("no branch points to the new commit; create one with git.branch".to_owned());
        notes.push(note);
    }
    let operations: Vec<&str> = OPERATION_MARKERS
        .iter()
        .copied()
        .filter(|marker| repo.git_dir().join(marker).exists())
        .collect();
    if !operations.is_empty() {
        let mut note = NotableEntry::new(Notable::OperationInProgress);
        note.detail = Some(format!("in progress: {}", operations.join(", ")));
        notes.push(note);
    }
    if excluded > 0 {
        let mut note = NotableEntry::new(Notable::Excluded);
        note.count = Some(excluded);
        notes.push(note);
    }
    notes
}

fn cleanup_mode(repo: &GitRepo) -> Result<Cleanup> {
    Cleanup::from_config(repo.config_string("commit.cleanup").as_deref()).map_err(|error| {
        Error::new(ErrorCode::Config, error.to_string()).with_problem("invalid_value")
    })
}

fn comment_char(repo: &GitRepo) -> char {
    repo.config_string("core.commentChar")
        .or_else(|| repo.config_string("core.commentString"))
        .filter(|value| value != "auto")
        .and_then(|value| value.chars().next())
        .unwrap_or(DEFAULT_COMMENT)
}

fn commit_encoding(repo: &GitRepo) -> Option<BString> {
    repo.config_string("i18n.commitEncoding")
        .map(|value| value.trim().to_owned())
        .filter(|value| {
            !value.is_empty()
                && !value.eq_ignore_ascii_case("utf-8")
                && !value.eq_ignore_ascii_case("utf8")
        })
        .map(BString::from)
}

fn display(path: &std::path::Path) -> String {
    path.display().to_string()
}

fn planned_changes(repo: &GitRepo, prepared: &Prepared) -> Vec<Change> {
    let mut changes = Vec::new();
    if prepared.index.is_some() {
        changes.push(Change::new(
            display(&repo.index_path()),
            ChangeAction::Modify,
        ));
    }
    let message_file = repo.git_dir().join("COMMIT_EDITMSG");
    let message_action = if message_file.exists() {
        ChangeAction::Modify
    } else {
        ChangeAction::Create
    };
    changes.push(Change::new(display(&message_file), message_action));
    for file in &prepared.merge {
        changes.push(Change::new(display(&file.path), ChangeAction::Delete));
    }
    let ref_file = match &prepared.branch {
        Some(name) => name
            .as_bstr()
            .split_str("/")
            .fold(repo.common_dir().to_path_buf(), |path, part| {
                path.join(part.to_str_lossy().as_ref())
            }),
        None => repo.git_dir().join("HEAD"),
    };
    let ref_action = if prepared.head.is_some() {
        ChangeAction::Modify
    } else {
        ChangeAction::Create
    };
    changes.push(Change::new(display(&ref_file), ref_action));
    changes
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
