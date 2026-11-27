// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::stage
// Responsibility:   The `git.stage` command: staging and unstaging files and hunks.
// Dependencies:     bstr, command, gix, gix_hash, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.stage` command: staging and unstaging files and hunks.
//!
//! [`plan`] reads the index, the tree of `HEAD` and the working tree and
//! computes the new index without writing. [`StagePlan::apply`] adds the new
//! blobs to the object store, an exempt write, and replaces the index through
//! the mutation transaction. Hunk identifiers select changes down to single
//! rows ([`crate::hunk_select`]). [`GitStageCommand`] is the implementation
//! used by the executor; its action `list` reads the hunks of `git.diff`.
//!
//! Entries written by this module carry no file status (all fields zero),
//! as the entries written by `git apply --cached` do; Git compares their
//! content the next time it reads the working tree. Staging a conflicted path
//! resolves the conflict and records its stages in the resolve-undo data, as
//! `git add` does.

mod command;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;

use bstr::{BStr, BString, ByteSlice};
use gix::index::entry::{Flags, Mode as EntryMode, Stage, Stat};
use gix_hash::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;

pub use command::GitStageCommand;

use crate::branch::quoted_repo;
use crate::diff::command::glob_match;
use crate::diff::{HunkId, Reader};
use crate::hunk_select::{self, Base, Mode};
use crate::repo::GitRepo;
use crate::repo::index::{IndexFile, ResolveUndo, write_index};
use crate::repo::lock::record_object_writes;

/// Changing action of `git.stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Copy the working tree state of the selection into the index.
    Add,
    /// Copy the `HEAD` state of the selection into the index.
    Unstage,
}

/// Selection of a changing `git.stage` call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageRequest {
    /// Files or directories relative to the root of the working tree,
    /// `/`-separated; `.` names the whole working tree.
    pub paths: Vec<BString>,
    /// Hunk identifiers `path#number[:fingerprint][@rows]`.
    pub hunks: Vec<String>,
    /// Record new files of `paths` as intent-to-add entries with empty
    /// content instead of staging their content (`git add -N`).
    pub intent_to_add: bool,
    /// Glob patterns of files left out of [`Action::Add`]; a pattern without
    /// `/` matches the file name in any directory.
    pub exclude: Vec<String>,
}

/// How the index entry of a path changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// The entry was created.
    Added,
    /// The content or mode of the entry changed.
    Modified,
    /// The entry was removed.
    Deleted,
    /// An intent-to-add entry was created.
    IntentToAdd,
}

/// Changed index entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPath {
    /// Path relative to the root of the working tree.
    pub path: BString,
    /// Kind of the change.
    pub kind: ChangeKind,
    /// Object of the entry after the change; `None` for a removed entry.
    pub id: Option<ObjectId>,
}

/// Planned index of a `git.stage` call.
#[derive(Debug)]
pub struct StagePlan {
    index: IndexFile,
    blobs: BTreeMap<ObjectId, Vec<u8>>,
    changes: Vec<StagedPath>,
    excluded: u64,
    ignored: Vec<BString>,
}

impl StagePlan {
    /// Changed index entries, in the order of the request.
    #[must_use]
    pub fn changes(&self) -> &[StagedPath] {
        &self.changes
    }

    /// Whether the index stays as it is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Number of files of directories left out by the `exclude` patterns.
    #[must_use]
    pub fn excluded(&self) -> u64 {
        self.excluded
    }

    /// Ignored files that were named explicitly and staged anyway.
    ///
    /// `git add` refuses such files unless it is given `-f`; the plan stages
    /// them, so the caller reports them.
    #[must_use]
    pub fn ignored(&self) -> &[BString] {
        &self.ignored
    }

    /// Number of stage 0 entries of the planned index.
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.index
            .state
            .entries()
            .iter()
            .filter(|entry| entry.stage() == Stage::Unconflicted)
            .count() as u64
    }

    /// Writes the new blobs and replaces the index.
    ///
    /// # Errors
    ///
    /// `E_IO` when a blob cannot be written, and the errors of
    /// [`write_index`] (`E_GIT_LOCKED` when `index.lock` exists).
    pub fn apply(mut self, repo: &GitRepo, tx: &mut dyn MutationTx) -> Result<Vec<StagedPath>> {
        if self.changes.is_empty() {
            return Ok(Vec::new());
        }
        for data in self.blobs.values() {
            repo.gix().write_blob(data).map_err(|error| {
                Error::new(
                    ErrorCode::Io,
                    format!("cannot write a blob to the object store: {error}"),
                )
            })?;
        }
        if !self.blobs.is_empty() {
            record_object_writes(tx, repo);
        }
        write_index(tx, &mut self.index)?;
        Ok(self.changes)
    }
}

/// Plans `action` for the files and hunks of `request`.
///
/// A directory stands for the files below it: for [`Action::Add`] the
/// entries of the index and the untracked files that are not ignored, for
/// [`Action::Unstage`] the entries of the index and of `HEAD`. Whole files are
/// processed first, in the order given, then hunks grouped by file in path
/// order.
///
/// # Errors
///
/// * `E_MISSING_PARAM` when neither files nor hunks are given;
/// * `E_ARG` with problem `invalid_path` for an absolute path, a path with
///   `.` or `..` components or a path inside `.git`;
/// * `E_NOT_FOUND` with problem `pathspec_no_match` for a file that is
///   neither in the working tree, the index nor `HEAD`;
/// * `E_GIT_UNSUPPORTED` with problem `bare` for a repository without a
///   working tree;
/// * the errors of [`hunk_select::group_ids`] and
///   [`hunk_select::selected_content`];
/// * `E_IO` when the index, an object or a file cannot be read.
pub fn plan(repo: &GitRepo, action: Action, request: &StageRequest) -> Result<StagePlan> {
    if request.paths.is_empty() && request.hunks.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "nothing to stage; pass paths or hunks",
        )
        .with_problem("nothing_selected")
        .with_next_step(format!(
            "git.stage {} action: list",
            quoted_repo(repo.gix())
        )));
    }
    let Some(workdir) = repo.workdir() else {
        return Err(Error::new(
            ErrorCode::GitUnsupported,
            "a bare repository has no working tree to stage from; run the command in a working tree",
        )
        .with_problem("bare"));
    };
    let groups = hunk_select::group_ids(&request.hunks)?;
    let mut builder = Builder {
        repo,
        workdir: workdir.to_path_buf(),
        index: repo.read_index()?,
        reader: Reader::new(repo.gix())?,
        head_tree: head_tree(repo)?,
        untracked: None,
        blobs: BTreeMap::new(),
        changes: Vec::new(),
    };
    let mut excluded = 0u64;
    let mut ignored = Vec::new();
    for path in &request.paths {
        let path = normalize(repo, path.as_bstr())?;
        let files = builder.files_of(path.as_bstr(), action)?;
        let named = files.len() == 1 && files[0] == path;
        for file in files {
            if action == Action::Add
                && request
                    .exclude
                    .iter()
                    .any(|pattern| glob_match(BStr::new(pattern.as_str()), file.as_bstr()))
            {
                excluded += 1;
                continue;
            }
            match action {
                Action::Add => {
                    let new = builder.is_new(file.as_bstr());
                    builder.add_file(file.as_bstr(), request.intent_to_add)?;
                    if named && new && builder.is_ignored(file.as_bstr())? {
                        ignored.push(file);
                    }
                }
                Action::Unstage => builder.unstage_file(file.as_bstr())?,
            }
        }
    }
    for (path, ids) in &groups {
        let path = normalize(repo, path.as_bstr())?;
        builder.select(path.as_bstr(), ids, action)?;
    }
    Ok(StagePlan {
        index: builder.index,
        blobs: builder.blobs,
        changes: builder.changes,
        excluded,
        ignored,
    })
}

/// Index entry at stage 0: object, mode and whether it is intent-to-add.
type Current = Option<(ObjectId, u32, bool)>;

struct Builder<'a> {
    repo: &'a GitRepo,
    workdir: std::path::PathBuf,
    index: IndexFile,
    reader: Reader<'a>,
    head_tree: Option<ObjectId>,
    untracked: Option<Vec<BString>>,
    blobs: BTreeMap<ObjectId, Vec<u8>>,
    changes: Vec<StagedPath>,
}

impl Builder<'_> {
    fn current(&self, path: &BStr) -> Current {
        self.index
            .state
            .entry_by_path_and_stage(path, Stage::Unconflicted)
            .map(|entry| {
                (
                    entry.id,
                    entry.mode.bits(),
                    entry.flags.contains(Flags::INTENT_TO_ADD),
                )
            })
    }

    fn conflicted(&self, path: &BStr) -> bool {
        self.index.state.entries().iter().any(|entry| {
            entry.stage() != Stage::Unconflicted && entry.path(&self.index.state) == path
        })
    }

    /// Whether `path` has no entry in the index at any stage.
    fn is_new(&self, path: &BStr) -> bool {
        self.current(path).is_none() && !self.conflicted(path)
    }

    /// Whether the file `path`, absent from the index, is excluded by the
    /// ignore rules: the walk for untracked files leaves it out.
    fn is_ignored(&mut self, path: &BStr) -> Result<bool> {
        let relative = gix_path::from_bstr(path).map_err(read_error)?;
        let on_disk = self
            .workdir
            .join(&*relative)
            .symlink_metadata()
            .is_ok_and(|meta| !meta.is_dir());
        Ok(on_disk && !self.untracked()?.iter().any(|file| file == path))
    }

    fn is_worktree_dir(&self, path: &BStr) -> Result<bool> {
        if path.is_empty() {
            return Ok(true);
        }
        let relative = gix_path::from_bstr(path).map_err(read_error)?;
        Ok(self.workdir.join(&*relative).is_dir())
    }

    /// Files `path` stands for: the path itself, or the files below a
    /// directory, sorted.
    fn files_of(&mut self, path: &BStr, action: Action) -> Result<Vec<BString>> {
        let is_dir = self.is_worktree_dir(path)?;
        let tracked = self.current(path).is_some() || self.conflicted(path);
        if tracked && !is_dir {
            return Ok(vec![path.to_owned()]);
        }
        let mut found = BTreeSet::new();
        for entry in self.index.state.entries() {
            let entry_path = entry.path(&self.index.state);
            if below(path, entry_path) {
                found.insert(entry_path.to_owned());
            }
        }
        let others = match action {
            Action::Add => self.untracked()?.to_vec(),
            Action::Unstage => self.head_files()?,
        };
        found.extend(
            others
                .into_iter()
                .filter(|other| below(path, other.as_bstr())),
        );
        if found.is_empty() && !is_dir {
            return Ok(vec![path.to_owned()]);
        }
        Ok(found.into_iter().collect())
    }

    /// Untracked files that are not ignored, read once per plan.
    fn untracked(&mut self) -> Result<&[BString]> {
        if self.untracked.is_none() {
            self.untracked = Some(untracked_files(self.repo.gix(), &self.index.state)?);
        }
        Ok(self.untracked.as_deref().unwrap_or_default())
    }

    /// Paths of the blobs and links of the tree of `HEAD`.
    fn head_files(&self) -> Result<Vec<BString>> {
        let Some(tree) = self.head_tree else {
            return Ok(Vec::new());
        };
        let tree = self
            .repo
            .gix()
            .find_object(tree)
            .map_err(read_error)?
            .try_into_tree()
            .map_err(read_error)?;
        let mut recorder = gix::traverse::tree::Recorder::default();
        tree.traverse()
            .breadthfirst(&mut recorder)
            .map_err(read_error)?;
        Ok(recorder
            .records
            .into_iter()
            .filter(|entry| !entry.mode.is_tree())
            .map(|entry| entry.filepath)
            .collect())
    }

    fn add_file(&mut self, path: &BStr, intent_to_add: bool) -> Result<()> {
        let current = self.current(path);
        let conflicted = self.conflicted(path);
        let index_mode = current.map(|(_, mode, _)| mode);
        match self.reader.worktree(path, index_mode)? {
            Some((_, mode)) if intent_to_add && current.is_none() && !conflicted => {
                let id = self.blob(Vec::new())?;
                self.set_entry(path, id, mode, Flags::INTENT_TO_ADD | Flags::EXTENDED)?;
                self.record(path, ChangeKind::IntentToAdd, Some(id));
            }
            Some((data, mode)) => {
                let id = self.blob(data)?;
                let unchanged = current == Some((id, mode, false)) && !conflicted;
                if !unchanged {
                    let kind = match current {
                        Some((_, _, false)) => ChangeKind::Modified,
                        _ => ChangeKind::Added,
                    };
                    self.set_entry(path, id, mode, Flags::empty())?;
                    self.record(path, kind, Some(id));
                }
            }
            None if current.is_some() || conflicted => {
                self.remove(path);
                self.record(path, ChangeKind::Deleted, None);
            }
            None => return Err(self.no_match(path)),
        }
        Ok(())
    }

    fn unstage_file(&mut self, path: &BStr) -> Result<()> {
        let current = self.current(path);
        let conflicted = self.conflicted(path);
        match self.head_entry(path)? {
            Some((id, mode)) => {
                if current != Some((id, mode, false)) || conflicted {
                    let kind = if current.is_some() {
                        ChangeKind::Modified
                    } else {
                        ChangeKind::Added
                    };
                    self.set_entry(path, id, mode, Flags::empty())?;
                    self.record(path, kind, Some(id));
                }
            }
            None if current.is_some() || conflicted => {
                self.remove(path);
                self.record(path, ChangeKind::Deleted, None);
            }
            None => return Err(self.no_match(path)),
        }
        Ok(())
    }

    fn select(&mut self, path: &BStr, ids: &[HunkId], action: Action) -> Result<()> {
        let mode = match action {
            Action::Add => Mode::Add,
            Action::Unstage => Mode::Unstage,
        };
        let current = self.current(path);
        let (content, entry_mode) = match current {
            Some((id, entry_mode, false)) => {
                let data = self.blob_content(id)?;
                (
                    hunk_select::selected_content(
                        self.repo.gix(),
                        path,
                        ids,
                        mode,
                        Base::Tracked(&data),
                    )?,
                    entry_mode,
                )
            }
            _ => {
                let worktree = match action {
                    Action::Add => self.reader.worktree(path, None)?,
                    Action::Unstage => None,
                };
                let (data, entry_mode) = worktree.unwrap_or_default();
                let base = Base::New { worktree: &data };
                (
                    hunk_select::selected_content(self.repo.gix(), path, ids, mode, base)?,
                    entry_mode,
                )
            }
        };
        let id = self.blob(content)?;
        let kind = match current {
            Some((_, _, false)) => ChangeKind::Modified,
            _ => ChangeKind::Added,
        };
        self.set_entry(path, id, entry_mode, Flags::empty())?;
        self.record(path, kind, Some(id));
        Ok(())
    }

    /// Object and mode of `path` in the tree of `HEAD`.
    fn head_entry(&self, path: &BStr) -> Result<Option<(ObjectId, u32)>> {
        let Some(tree) = self.head_tree else {
            return Ok(None);
        };
        let tree = self
            .repo
            .gix()
            .find_object(tree)
            .map_err(read_error)?
            .try_into_tree()
            .map_err(read_error)?;
        let relative = gix_path::from_bstr(path).map_err(read_error)?;
        let Some(entry) = tree.lookup_entry_by_path(&*relative).map_err(read_error)? else {
            return Ok(None);
        };
        if entry.mode().is_tree() {
            return Ok(None);
        }
        Ok(Some((entry.object_id(), u32::from(entry.mode().value()))))
    }

    fn blob_content(&self, id: ObjectId) -> Result<Vec<u8>> {
        if let Some(data) = self.blobs.get(&id) {
            return Ok(data.clone());
        }
        Ok(self
            .repo
            .gix()
            .find_object(id)
            .map_err(read_error)?
            .data
            .clone())
    }

    /// Object id of `data`; the blob is written on apply unless it exists.
    fn blob(&mut self, data: Vec<u8>) -> Result<ObjectId> {
        let id = blob_id(self.repo, &data)?;
        if !self.repo.gix().has_object(id) {
            self.blobs.insert(id, data);
        }
        Ok(id)
    }

    /// Sets the stage 0 entry of `path`, resolving a conflict of the path.
    fn set_entry(&mut self, path: &BStr, id: ObjectId, mode: u32, flags: Flags) -> Result<()> {
        let mode = EntryMode::from_bits(mode).ok_or_else(|| {
            Error::new(
                ErrorCode::GitUnsupported,
                format!("{path} has the file mode {mode:o}, which the index cannot store"),
            )
            .with_problem("file_mode")
        })?;
        self.resolve_conflict(path);
        let state = &mut self.index.state;
        if let Some(entry) = state.entry_mut_by_path_and_stage(path, Stage::Unconflicted) {
            entry.id = id;
            entry.mode = mode;
            entry.stat = Stat::default();
            entry.flags.remove(Flags::INTENT_TO_ADD);
            entry.flags.insert(flags);
            if !entry
                .flags
                .intersects(Flags::INTENT_TO_ADD | Flags::SKIP_WORKTREE)
            {
                entry.flags.remove(Flags::EXTENDED);
            }
        } else {
            state.dangerously_push_entry(Stat::default(), id, flags, mode, path);
            state.sort_entries();
        }
        Ok(())
    }

    fn remove(&mut self, path: &BStr) {
        self.resolve_conflict(path);
        self.index
            .state
            .remove_entries(|_, entry_path, _| entry_path == path);
    }

    /// Moves the conflict stages of `path` into the resolve-undo data.
    fn resolve_conflict(&mut self, path: &BStr) {
        let mut stages: [Option<(u32, ObjectId)>; 3] = [None, None, None];
        let mut found = false;
        for entry in self.index.state.entries() {
            if entry.path(&self.index.state) != path {
                continue;
            }
            let slot = match entry.stage() {
                Stage::Unconflicted => continue,
                Stage::Base => 0,
                Stage::Ours => 1,
                Stage::Theirs => 2,
            };
            stages[slot] = Some((entry.mode.bits(), entry.id));
            found = true;
        }
        if !found {
            return;
        }
        self.index.state.remove_entries(|_, entry_path, entry| {
            entry_path == path && entry.stage() != Stage::Unconflicted
        });
        self.index
            .resolve_undo
            .retain(|record| record.path.as_bstr() != path);
        self.index.resolve_undo.push(ResolveUndo {
            path: path.to_owned(),
            stages,
        });
    }

    fn record(&mut self, path: &BStr, kind: ChangeKind, id: Option<ObjectId>) {
        self.changes.push(StagedPath {
            path: path.to_owned(),
            kind,
            id,
        });
    }

    fn no_match(&self, path: &BStr) -> Error {
        Error::new(
            ErrorCode::NotFound,
            format!("{path} is not in the working tree, the index or HEAD; check the path"),
        )
        .with_problem("pathspec_no_match")
        .with_found(path.to_str_lossy().into_owned())
        .with_next_step(format!("git.status {}", quoted_repo(self.repo.gix())))
    }
}

/// Whether `path` lies below the directory `dir`; every path lies below the
/// root, named by an empty `dir`.
fn below(dir: &BStr, path: &BStr) -> bool {
    dir.is_empty() || (path.starts_with(dir) && path.get(dir.len()) == Some(&b'/'))
}

/// Untracked files and links that are not ignored, sorted by path;
/// directories holding another repository are not entered.
fn untracked_files(repo: &gix::Repository, index: &gix::index::State) -> Result<Vec<BString>> {
    let options = repo
        .dirwalk_options()
        .map_err(read_error)?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_ignored(None)
        .emit_tracked(false)
        .recurse_repositories(false)
        .emit_pruned(false)
        .emit_empty_directories(false);
    let mut collect = gix::dir::walk::delegate::Collect::default();
    let interrupt = AtomicBool::new(false);
    repo.dirwalk(index, None::<&str>, &interrupt, options, &mut collect)
        .map_err(read_error)?;
    Ok(collect
        .into_entries_by_path()
        .into_iter()
        .filter(|(entry, _)| {
            entry.status == gix::dir::entry::Status::Untracked
                && matches!(
                    entry.disk_kind,
                    Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink)
                )
        })
        .map(|(entry, _)| entry.rela_path)
        .collect())
}

/// Tree of `HEAD`; `None` on an unborn branch.
fn head_tree(repo: &GitRepo) -> Result<Option<ObjectId>> {
    let Some(head) = repo.head_id()? else {
        return Ok(None);
    };
    let tree = repo
        .gix()
        .find_object(head)
        .map_err(read_error)?
        .peel_to_tree()
        .map_err(read_error)?;
    Ok(Some(tree.id))
}

fn blob_id(repo: &GitRepo, data: &[u8]) -> Result<ObjectId> {
    gix_object::compute_hash(repo.gix().object_hash(), gix_object::Kind::Blob, data)
        .map_err(|error| Error::new(ErrorCode::Io, format!("cannot hash a blob: {error}")))
}

/// Path relative to the root of the working tree, without a trailing `/`;
/// `.` is the root, returned as an empty path.
fn normalize(repo: &GitRepo, path: &BStr) -> Result<BString> {
    let trimmed: &[u8] = path.strip_suffix(b"/").unwrap_or(path);
    if trimmed == b"." {
        return Ok(BString::default());
    }
    let invalid = trimmed.is_empty()
        || trimmed.starts_with(b"/")
        || trimmed.contains(&b'\\')
        || trimmed.split_str("/").any(|component| {
            component.is_empty()
                || component == b"."
                || component == b".."
                || component.eq_ignore_ascii_case(b".git")
        });
    if invalid {
        return Err(Error::new(
            ErrorCode::Arg,
            format!(
                "{path} is not a path inside the working tree; use a path relative to its root without . or .. components"
            ),
        )
        .with_problem("invalid_path")
        .with_found(path.to_str_lossy().into_owned())
        .with_next_step(format!("git.status {}", quoted_repo(repo.gix()))));
    }
    Ok(trimmed.as_bstr().to_owned())
}

fn read_error(error: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read the repository: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{RecordingJournal, RepoBuilder, test_ctx};

    fn open(fixture: &crate::testutil::Fixture) -> GitRepo {
        GitRepo::open(fixture.path(), &test_ctx()).unwrap()
    }

    fn request(paths: &[&str], hunks: &[&str]) -> StageRequest {
        StageRequest {
            paths: paths.iter().map(|path| BString::from(*path)).collect(),
            hunks: hunks.iter().map(|hunk| (*hunk).to_owned()).collect(),
            ..StageRequest::default()
        }
    }

    fn entry(repo: &GitRepo, path: &str) -> Option<(ObjectId, bool)> {
        let index = repo.read_index().unwrap();
        index
            .state
            .entry_by_path_and_stage(BStr::new(path), Stage::Unconflicted)
            .map(|entry| (entry.id, entry.flags.contains(Flags::INTENT_TO_ADD)))
    }

    /// Plans and applies `action`, checks that undo restores the index byte
    /// for byte, and applies the change again for the assertions of the
    /// caller.
    fn run(
        fixture: &crate::testutil::Fixture,
        action: Action,
        request: &StageRequest,
    ) -> Vec<StagedPath> {
        let repo = open(fixture);
        let index_path = repo.index_path();
        let before = std::fs::read(&index_path).ok();
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let changes = plan(&repo, action, request)
            .unwrap()
            .apply(&repo, tx.as_mut())
            .unwrap();
        tx.commit().unwrap();
        if changes.is_empty() {
            assert_eq!(std::fs::read(&index_path).ok(), before);
            return changes;
        }
        let after = std::fs::read(&index_path).unwrap();
        journal.undo_last();
        assert_eq!(
            std::fs::read(&index_path).ok(),
            before,
            "undo restores the index"
        );
        std::fs::write(&index_path, after).unwrap();
        changes
    }

    fn hash(repo: &GitRepo, data: &[u8]) -> ObjectId {
        blob_id(repo, data).unwrap()
    }

    fn paths_of(changes: &[StagedPath]) -> Vec<String> {
        changes
            .iter()
            .map(|change| change.path.to_string())
            .collect()
    }

    #[test]
    fn adding_a_modified_file_stages_its_content() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "one\n")
            .commit("first")
            .worktree_file("a.txt", "two\n")
            .build();
        let changes = run(&fixture, Action::Add, &request(&["a.txt"], &[]));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        let repo = open(&fixture);
        assert_eq!(entry(&repo, "a.txt"), Some((hash(&repo, b"two\n"), false)));
        assert!(repo.gix().has_object(hash(&repo, b"two\n")));
    }

    #[test]
    fn adding_new_and_deleted_files_creates_and_removes_entries() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .file("b.txt", "b\n")
            .commit("first")
            .remove("b.txt")
            .worktree_file("n.txt", "new\n")
            .build();
        let changes = run(&fixture, Action::Add, &request(&["n.txt", "b.txt"], &[]));
        let kinds: Vec<ChangeKind> = changes.iter().map(|change| change.kind).collect();
        assert_eq!(kinds, [ChangeKind::Added, ChangeKind::Deleted]);
        let repo = open(&fixture);
        assert!(entry(&repo, "n.txt").is_some());
        assert!(entry(&repo, "b.txt").is_none());
    }

    #[test]
    fn an_ignored_file_named_explicitly_is_staged_and_reported() {
        let fixture = RepoBuilder::new()
            .file(".gitignore", "*.log\n")
            .file("a.txt", "a\n")
            .commit("first")
            .worktree_file("debug.log", "log\n")
            .worktree_file("new.txt", "n\n")
            .build();
        let repo = open(&fixture);
        let planned = plan(&repo, Action::Add, &request(&["debug.log", "new.txt"], &[])).unwrap();
        assert_eq!(paths_of(planned.changes()), ["debug.log", "new.txt"]);
        assert_eq!(planned.ignored(), [BString::from("debug.log")]);
        let planned = plan(&repo, Action::Add, &request(&["."], &[])).unwrap();
        assert_eq!(paths_of(planned.changes()), ["new.txt"]);
        assert!(planned.ignored().is_empty());
    }

    #[test]
    fn adding_a_directory_stages_changed_new_and_deleted_files_below_it() {
        let fixture = RepoBuilder::new()
            .file("dir/a.txt", "a\n")
            .file("dir/gone.txt", "g\n")
            .file("top.txt", "t\n")
            .commit("first")
            .worktree_file("dir/a.txt", "A\n")
            .worktree_file("dir/sub/new.txt", "n\n")
            .worktree_file("top.txt", "T\n")
            .remove("dir/gone.txt")
            .build();
        let changes = run(&fixture, Action::Add, &request(&["dir/"], &[]));
        assert_eq!(
            paths_of(&changes),
            ["dir/a.txt", "dir/gone.txt", "dir/sub/new.txt"]
        );
        let repo = open(&fixture);
        assert!(entry(&repo, "dir/gone.txt").is_none());
        assert_eq!(entry(&repo, "top.txt"), Some((hash(&repo, b"t\n"), false)));
    }

    #[test]
    fn the_root_directory_and_exclusions_select_the_whole_tree() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .worktree_file("a.txt", "b\n")
            .worktree_file("build.log", "log\n")
            .worktree_file("d/x.log", "log\n")
            .build();
        let mut req = request(&["."], &[]);
        req.exclude = vec!["*.log".to_owned()];
        let repo = open(&fixture);
        let planned = plan(&repo, Action::Add, &req).unwrap();
        assert_eq!(planned.excluded(), 2);
        assert_eq!(paths_of(planned.changes()), ["a.txt"]);
        assert_eq!(planned.entry_count(), 1);
    }

    #[test]
    fn unstaging_a_directory_restores_head_entries_below_it() {
        let fixture = RepoBuilder::new()
            .file("dir/a.txt", "one\n")
            .commit("first")
            .file("dir/a.txt", "two\n")
            .file("dir/n.txt", "new\n")
            .commit("second")
            .reference("refs/heads/main", "HEAD~1")
            .build();
        let changes = run(&fixture, Action::Unstage, &request(&["dir"], &[]));
        assert_eq!(paths_of(&changes), ["dir/a.txt", "dir/n.txt"]);
        let repo = open(&fixture);
        assert_eq!(
            entry(&repo, "dir/a.txt"),
            Some((hash(&repo, b"one\n"), false))
        );
        assert!(entry(&repo, "dir/n.txt").is_none());
    }

    #[test]
    fn intent_to_add_records_an_empty_entry() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .worktree_file("n.txt", "new\n")
            .build();
        let mut req = request(&["n.txt"], &[]);
        req.intent_to_add = true;
        let changes = run(&fixture, Action::Add, &req);
        assert_eq!(changes[0].kind, ChangeKind::IntentToAdd);
        let repo = open(&fixture);
        assert_eq!(entry(&repo, "n.txt"), Some((hash(&repo, b""), true)));
    }

    #[test]
    fn unchanged_file_leaves_the_index_untouched() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .build();
        let changes = run(&fixture, Action::Add, &request(&["a.txt"], &[]));
        assert!(changes.is_empty());
    }

    #[test]
    fn unstaging_restores_the_entry_of_head() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "one\n")
            .commit("first")
            .file("a.txt", "two\n")
            .file("n.txt", "new\n")
            .commit("second")
            .reference("refs/heads/main", "HEAD~1")
            .build();
        let changes = run(
            &fixture,
            Action::Unstage,
            &request(&["a.txt", "n.txt"], &[]),
        );
        let kinds: Vec<ChangeKind> = changes.iter().map(|change| change.kind).collect();
        assert_eq!(kinds, [ChangeKind::Modified, ChangeKind::Deleted]);
        let repo = open(&fixture);
        assert_eq!(entry(&repo, "a.txt"), Some((hash(&repo, b"one\n"), false)));
        assert!(entry(&repo, "n.txt").is_none());
    }

    #[test]
    fn hunks_stage_selected_rows_only() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\nb\nc\n")
            .commit("first")
            .worktree_file("a.txt", "a\nB\nc\nd\n")
            .build();
        let changes = run(&fixture, Action::Add, &request(&[], &["a.txt#1@2-3"]));
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        let repo = open(&fixture);
        assert_eq!(
            entry(&repo, "a.txt"),
            Some((hash(&repo, b"a\nB\nc\n"), false))
        );
    }

    #[test]
    fn hunks_of_a_new_file_are_staged_partially() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .worktree_file("n.txt", "one\ntwo\nthree\n")
            .build();
        run(&fixture, Action::Add, &request(&[], &["n.txt#1@1,3"]));
        let repo = open(&fixture);
        assert_eq!(
            entry(&repo, "n.txt"),
            Some((hash(&repo, b"one\nthree\n"), false))
        );
    }

    #[test]
    fn invalid_requests_are_rejected_with_a_problem() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .file("dir/b.txt", "b\n")
            .commit("first")
            .build();
        let repo = open(&fixture);
        let problem = |action, req: StageRequest| {
            plan(&repo, action, &req)
                .unwrap_err()
                .problem
                .clone()
                .unwrap_or_default()
        };
        assert_eq!(problem(Action::Add, request(&[], &[])), "nothing_selected");
        for bad in ["../x", "/abs", ".git/config", "a/./b", ""] {
            assert_eq!(
                problem(Action::Add, request(&[bad], &[])),
                "invalid_path",
                "{bad}"
            );
        }
        assert_eq!(
            problem(Action::Add, request(&["none.txt"], &[])),
            "pathspec_no_match"
        );
        assert_eq!(
            problem(Action::Unstage, request(&["none.txt"], &[])),
            "pathspec_no_match"
        );
        assert_eq!(
            problem(Action::Add, request(&[], &["a.txt#x"])),
            "invalid_id"
        );
    }

    #[test]
    fn below_matches_whole_components_only() {
        assert!(below(BStr::new(""), BStr::new("a/b")));
        assert!(below(BStr::new("a"), BStr::new("a/b")));
        assert!(!below(BStr::new("a"), BStr::new("ab/c")));
        assert!(!below(BStr::new("a"), BStr::new("a")));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
