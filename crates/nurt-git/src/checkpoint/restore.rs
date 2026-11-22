// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Restoring the working tree, and optionally the index, from a checkpoint.
//!
//! Files of the checkpoint that differ from the working tree are written,
//! tracked and untracked files that are not ignored and are absent from the
//! checkpoint are removed, and ignored files stay. Directories left empty by
//! the removals are removed as a whole. `HEAD` and references do not change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::journal::{Expect, MutationTx};
use nurt_contracts::report::{Change, ChangeAction, Notable, NotableEntry};

use super::snapshot::{Blobs, snapshot_worktree};
use super::support::{
    Filters, TreeEntry, TxPlan, display_path, expect_current, flatten_tree, in_scope, open_repo,
    problem, quote, repo_error, repo_path, repo_relative, serialize_index, workdir, worktree_path,
};
use super::{CreatePlan, read_commit, resolve};
use crate::repo::pathspec::PathFilter;

/// Parameters of a restore.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreRequest {
    /// Checkpoint to restore, as `cp-<N>`.
    pub checkpoint: String,
    /// Files or directories to restore, relative to the root of the working
    /// tree or absolute inside it; all files when empty.
    pub paths: Vec<String>,
    /// Also restore the index recorded by the checkpoint.
    pub index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Removal {
    File { expect: Expect },
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Write {
    kind: EntryKind,
    id: ObjectId,
    expect: Expect,
    existed: bool,
    replaces_directory: bool,
}

#[derive(Debug, Clone)]
struct IndexPlan {
    tree: ObjectId,
    scopes: Vec<BString>,
    excluded: PathFilter,
    expected: Option<Vec<u8>>,
}

/// Changes computed by [`plan_restore`]: file contents are referenced by
/// object identifiers and read when applied.
#[derive(Debug, Clone)]
pub struct RestorePlan {
    repo: PathBuf,
    root: PathBuf,
    index_path: PathBuf,
    removals: BTreeMap<PathBuf, Removal>,
    writes: BTreeMap<PathBuf, Write>,
    index: Option<IndexPlan>,
    untracked_removed: u64,
    saved: Option<CreatePlan>,
}

impl RestorePlan {
    /// Records the current state as a checkpoint that [`apply_restore`] makes
    /// reachable in the same transaction as the restore.
    pub fn save_current(&mut self, plan: CreatePlan) {
        self.saved = Some(plan);
    }

    /// Checkpoint of the state before the restore, when one is planned.
    #[must_use]
    pub fn saved(&self) -> Option<&nurt_contracts::record::GitCheckpoint> {
        self.saved.as_ref().map(CreatePlan::checkpoint)
    }

    /// Whether the working tree and index already match the checkpoint.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.removals.is_empty() && self.writes.is_empty() && self.index.is_none()
    }

    /// Planned changes of files, in the form of the effect report.
    #[must_use]
    pub fn changes(&self) -> Vec<Change> {
        let mut out: Vec<Change> = self
            .removals
            .keys()
            .map(|path| Change::new(display_path(path), ChangeAction::Delete))
            .collect();
        out.extend(self.writes.iter().map(|(path, write)| {
            let action = if write.existed {
                ChangeAction::Modify
            } else {
                ChangeAction::Create
            };
            Change::new(display_path(path), action)
        }));
        if self.index.is_some() {
            out.push(Change::new(
                display_path(&self.index_path),
                ChangeAction::Modify,
            ));
        }
        if let Some(saved) = &self.saved {
            out.push(Change::new(
                display_path(&saved.reference_file()),
                ChangeAction::Create,
            ));
        }
        out
    }

    /// Unusual effects known before the restore.
    #[must_use]
    pub fn notable(&self) -> Vec<NotableEntry> {
        let mut out = Vec::new();
        if self.untracked_removed > 0 {
            let mut entry = NotableEntry::new(Notable::UntrackedRemoved);
            entry.count = Some(self.untracked_removed);
            out.push(entry);
        }
        if self.is_empty() {
            out.push(NotableEntry::new(Notable::NothingMatched));
        }
        out
    }
}

/// Compares the working tree with the requested checkpoint and plans the
/// changes that make them equal.
///
/// # Errors
///
/// Fails with `E_GIT_REF` and problem `not_found` for an unknown checkpoint.
/// Fails with `E_GIT_CHECKPOINT` and problem `no_index` when the index is requested
/// from a checkpoint that did not record it. Fails with
/// `E_GIT_NOT_REPOSITORY` and problem `path_outside_repo` for a path outside
/// the working tree. Fails with `E_IO` when the repository cannot be read.
pub fn plan_restore(repo_path: &Path, request: &RestoreRequest) -> Result<RestorePlan> {
    let repo = open_repo(repo_path)?;
    let root = workdir(&repo)?;
    let shown = display_path(repo_path);
    let found = resolve(&repo, &request.checkpoint, &shown)?;
    let checkpoint = read_commit(&repo, &found)?;
    let excluded = PathFilter::new(&root, &[], &checkpoint.excludes)?;
    let scopes = request
        .paths
        .iter()
        .map(|path| repo_relative(&root, path))
        .collect::<Result<Vec<_>>>()?;
    let index_tree = match (request.index, checkpoint.index_tree) {
        (true, None) => {
            return Err(problem(
                ErrorCode::GitCheckpoint,
                "no_index",
                format!(
                    "checkpoint `{}` has no recorded index; restore without `index`",
                    request.checkpoint
                ),
            )
            .with_fix(format!(
                "git.checkpoint {} action: restore checkpoint_id: {}",
                quote(&shown),
                quote(&request.checkpoint)
            )));
        }
        (true, Some(tree)) => Some(tree),
        (false, _) => None,
    };
    let index = repo
        .index_or_empty()
        .map_err(|err| repo_error("read the index", err))?;
    let mut filters = Filters::new(&repo, &index)?;
    let current = snapshot_worktree(&repo, &index, &mut filters, Blobs::HashOnly)?;
    let target: BTreeMap<BString, TreeEntry> = flatten_tree(&repo, checkpoint.tree)?
        .into_iter()
        .filter(|(path, _)| in_scope(path.as_bstr(), &scopes) && !excluded.excluded(path.as_bstr()))
        .collect();
    let mut removals = BTreeMap::new();
    let mut deleted = BTreeSet::new();
    let mut untracked_removed = 0;
    for path in current.files.keys() {
        if !in_scope(path.as_bstr(), &scopes)
            || excluded.excluded(path.as_bstr())
            || target.contains_key(path)
        {
            continue;
        }
        let file = worktree_path(&root, path.as_bstr())?;
        untracked_removed += u64::from(current.untracked.contains(path));
        deleted.insert(path.clone());
        removals.insert(
            file.clone(),
            Removal::File {
                expect: expect_current(&file)?,
            },
        );
    }
    let mut writes = BTreeMap::new();
    for (path, entry) in &target {
        if current.files.get(path) == Some(entry) {
            continue;
        }
        let file = worktree_path(&root, path.as_bstr())?;
        let metadata = std::fs::symlink_metadata(&file).ok();
        let replaces_directory = metadata.as_ref().is_some_and(std::fs::Metadata::is_dir);
        let expect = if replaces_directory {
            Expect::Unchecked
        } else {
            expect_current(&file)?
        };
        if replaces_directory {
            removals.retain(|removed, _| !removed.starts_with(&file));
        }
        writes.insert(
            file,
            Write {
                kind: entry.0,
                id: entry.1,
                expect,
                existed: metadata.is_some(),
                replaces_directory,
            },
        );
    }
    collapse_directories(&root, &deleted, &target, &writes, &mut removals)?;
    let index = index_tree.map(|tree| IndexPlan {
        tree,
        scopes,
        excluded,
        expected: std::fs::read(repo.index_path()).ok(),
    });
    Ok(RestorePlan {
        repo: repo_path.to_path_buf(),
        root,
        index_path: repo.index_path(),
        removals,
        writes,
        index,
        untracked_removed,
        saved: None,
    })
}

/// Replaces the removal of every file of a directory by the removal of the
/// directory, when the directory holds nothing else and receives no file.
fn collapse_directories(
    root: &Path,
    deleted: &BTreeSet<BString>,
    target: &BTreeMap<BString, TreeEntry>,
    writes: &BTreeMap<PathBuf, Write>,
    removals: &mut BTreeMap<PathBuf, Removal>,
) -> Result<()> {
    let mut candidates = BTreeSet::new();
    for path in deleted {
        let mut current = path.as_bstr();
        while let Some(slash) = current.rfind_byte(b'/') {
            current = current[..slash].as_bstr();
            candidates.insert(current.to_owned());
        }
    }
    let mut chosen: Vec<BString> = Vec::new();
    for dir in candidates {
        if chosen.iter().any(|parent| is_inside(&dir, parent)) {
            continue;
        }
        if target.keys().any(|path| is_inside(path, &dir)) {
            continue;
        }
        let absolute = worktree_path(root, dir.as_bstr())?;
        if writes.contains_key(&absolute) {
            continue;
        }
        if only_deleted_files(&absolute, &dir, deleted)? {
            chosen.push(dir);
        }
    }
    for dir in chosen {
        let absolute = worktree_path(root, dir.as_bstr())?;
        removals.retain(|path, _| !path.starts_with(&absolute));
        removals.insert(absolute, Removal::Directory);
    }
    Ok(())
}

fn is_inside(path: &BString, dir: &BString) -> bool {
    path.starts_with(dir.as_slice()) && path.get(dir.len()) == Some(&b'/')
}

fn only_deleted_files(dir: &Path, rela: &BString, deleted: &BTreeSet<BString>) -> Result<bool> {
    let read_error = |err: std::io::Error| repo_error(&format!("read `{}`", dir.display()), err);
    for entry in std::fs::read_dir(dir).map_err(read_error)? {
        let entry = entry.map_err(read_error)?;
        let mut child = rela.clone();
        child.push(b'/');
        child.extend_from_slice(&repo_path(Path::new(&entry.file_name()))?);
        let kind = entry.file_type().map_err(read_error)?;
        let removed = if kind.is_dir() {
            only_deleted_files(&entry.path(), &child, deleted)?
        } else {
            deleted.contains(&child)
        };
        if !removed {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Applies a planned restore through `tx`.
///
/// # Errors
///
/// Fails with `E_STALE` when a file or the index changed after planning.
/// Fails with `E_GIT_LOCKED` when the index is locked. Fails with
/// `E_GIT_UNSUPPORTED` and problem `symlink_restore` when a symbolic link
/// must be written. Errors of the journal are passed on.
pub fn apply_restore(plan: &RestorePlan, tx: &mut dyn MutationTx) -> Result<()> {
    if plan.is_empty() {
        return Ok(());
    }
    let repo = open_repo(&plan.repo)?;
    let index = repo
        .index_or_empty()
        .map_err(|err| repo_error("read the index", err))?;
    let mut filters = Filters::new(&repo, &index)?;
    let mut changes = TxPlan::default();
    if let Some(saved) = &plan.saved {
        saved.declare(&mut changes, tx)?;
    }
    for (path, removal) in &plan.removals {
        let expect = match removal {
            Removal::File { expect } => expect.clone(),
            Removal::Directory => Expect::Unchecked,
        };
        changes.delete(path, expect)?;
    }
    for (path, write) in &plan.writes {
        if write.kind == EntryKind::Link {
            return Err(problem(
                ErrorCode::GitUnsupported,
                "symlink_restore",
                format!(
                    "`{}` is a symbolic link in the checkpoint; restore it with `git.restore`",
                    path.display()
                ),
            ));
        }
        let blob = repo
            .find_blob(write.id)
            .map_err(|err| repo_error(&format!("read the content of `{}`", path.display()), err))?;
        let rela = match path.strip_prefix(&plan.root) {
            Ok(rela) => repo_path(rela)?,
            Err(_) => BString::default(),
        };
        let content = filters.convert_to_worktree(rela.as_bstr(), &blob.data)?;
        if write.replaces_directory {
            changes.replace_with_file(path, content)?;
        } else {
            changes.write_file(path, content, write.expect.clone())?;
        }
    }
    if let Some(index_plan) = &plan.index {
        let bytes = restored_index(&repo, &index, index_plan)?;
        changes.write_locked(&plan.index_path, bytes, index_plan.expected.clone())?;
    }
    changes.preserve(tx)?;
    changes.apply_rest(tx)
}

fn restored_index(
    repo: &gix::Repository,
    current: &gix::index::State,
    plan: &IndexPlan,
) -> Result<Vec<u8>> {
    let mut state = gix::index::State::new(repo.object_hash());
    for entry in current.entries() {
        let path = entry.path(current);
        if !in_scope(path, &plan.scopes) || plan.excluded.excluded(path) {
            state.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, path);
        }
    }
    for (path, (kind, id)) in flatten_tree(repo, plan.tree)? {
        if in_scope(path.as_bstr(), &plan.scopes) && !plan.excluded.excluded(path.as_bstr()) {
            state.dangerously_push_entry(
                gix::index::entry::Stat::default(),
                id,
                gix::index::entry::Flags::empty(),
                index_mode(kind),
                path.as_bstr(),
            );
        }
    }
    state.sort_entries();
    serialize_index(state, &repo.index_path())
}

fn index_mode(kind: EntryKind) -> gix::index::entry::Mode {
    use gix::index::entry::Mode;
    match kind {
        EntryKind::BlobExecutable => Mode::FILE_EXECUTABLE,
        EntryKind::Link => Mode::SYMLINK,
        EntryKind::Commit => Mode::COMMIT,
        EntryKind::Blob | EntryKind::Tree => Mode::FILE,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_journal::{Fixture, RecordingJournal, test_ctx};
    use super::super::{CreateRequest, apply_create, plan_create};
    use super::*;

    fn checkpoint(fixture: &Fixture, journal: &RecordingJournal, index: bool) {
        let plan = plan_create(
            &fixture.root,
            &CreateRequest {
                label: None,
                index,
                exclude: Vec::new(),
            },
            &test_ctx(),
        )
        .unwrap();
        let mut tx = journal.begin(fixture.base());
        apply_create(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
    }

    fn restore(fixture: &Fixture, journal: &RecordingJournal, request: &RestoreRequest) {
        let plan = plan_restore(&fixture.root, request).unwrap();
        let mut tx = journal.begin(fixture.base());
        apply_restore(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
    }

    fn request(paths: &[&str], index: bool) -> RestoreRequest {
        RestoreRequest {
            checkpoint: "cp-1".into(),
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            index,
        }
    }

    fn last_name(path: &str) -> String {
        path.rsplit('/').next().unwrap_or_default().to_owned()
    }

    #[test]
    fn restore_rewrites_recreates_and_removes_files_and_undo_returns_the_later_state() {
        let fixture = Fixture::new(&[
            ("a.txt", "a = 2\n"),
            ("keep.txt", "keep\n"),
            (".gitignore", ".env\n"),
        ]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal, false);
        fixture.write("a.txt", "a = 99\n");
        fixture.write("extra.txt", "extra\n");
        fixture.write("tmp/deep/x.txt", "x\n");
        fixture.write(".env", "SECRET=1\n");
        fixture.remove("keep.txt");
        let changed = fixture.state();
        let plan = plan_restore(&fixture.root, &request(&[], false)).unwrap();
        let mut actions: Vec<(String, ChangeAction)> = plan
            .changes()
            .into_iter()
            .map(|change| (last_name(change.path.as_str()), change.action))
            .collect();
        actions.sort_by(|a, b| a.0.cmp(&b.0));
        let expected = [
            ("a.txt", ChangeAction::Modify),
            ("extra.txt", ChangeAction::Delete),
            ("keep.txt", ChangeAction::Create),
            ("tmp", ChangeAction::Delete),
        ]
        .map(|(name, action)| (name.to_owned(), action));
        assert_eq!(actions, expected);
        assert_eq!(plan.notable()[0].code, Notable::UntrackedRemoved);
        assert_eq!(plan.notable()[0].count, Some(2));
        let mut tx = journal.begin(fixture.base());
        apply_restore(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        assert_eq!(fixture.read("a.txt"), "a = 2\n");
        assert_eq!(fixture.read("keep.txt"), "keep\n");
        assert_eq!(fixture.read(".env"), "SECRET=1\n");
        assert!(!fixture.root.join("extra.txt").exists());
        assert!(!fixture.root.join("tmp").exists());
        let again = plan_restore(&fixture.root, &request(&[], false)).unwrap();
        assert!(again.is_empty());
        assert_eq!(again.notable()[0].code, Notable::NothingMatched);
        journal.undo_last();
        assert_eq!(fixture.state(), changed);
    }

    #[test]
    fn restore_limited_to_paths_leaves_other_files() {
        let fixture = Fixture::new(&[("src/a.txt", "one\n"), ("b.txt", "two\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal, false);
        fixture.write("src/a.txt", "changed\n");
        fixture.write("b.txt", "changed\n");
        fixture.write("src/new.txt", "new\n");
        restore(&fixture, &journal, &request(&["src"], false));
        assert_eq!(fixture.read("src/a.txt"), "one\n");
        assert_eq!(fixture.read("b.txt"), "changed\n");
        assert!(!fixture.root.join("src/new.txt").exists());
    }

    #[test]
    fn restore_with_index_restores_staged_entries() {
        let fixture = Fixture::new(&[("README.md", "readme\n")]);
        fixture.write("t.txt", "staged\n");
        fixture.stage(&["t.txt"]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal, true);
        fixture.unstage_all();
        assert!(!fixture.index_paths().contains(&"t.txt".to_owned()));
        let before = fixture.state();
        restore(&fixture, &journal, &request(&[], true));
        assert!(fixture.index_paths().contains(&"t.txt".to_owned()));
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }

    #[test]
    fn restore_errors_name_the_problem() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal, false);
        let no_index = plan_restore(&fixture.root, &request(&[], true)).unwrap_err();
        assert_eq!(no_index.data().problem.as_deref(), Some("no_index"));
        assert!(!no_index.data().fix.is_empty());
        let outside = plan_restore(&fixture.root, &request(&["../x"], false)).unwrap_err();
        assert_eq!(outside.code(), ErrorCode::GitNotRepository);
        let mut unknown = request(&[], false);
        unknown.checkpoint = "cp-7".into();
        assert_eq!(
            plan_restore(&fixture.root, &unknown).unwrap_err().code(),
            ErrorCode::GitRef
        );
    }

    #[test]
    fn restore_detects_change_after_planning() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal, false);
        fixture.write("a.txt", "two\n");
        let plan = plan_restore(&fixture.root, &request(&[], false)).unwrap();
        fixture.write("a.txt", "three\n");
        let mut tx = journal.begin(fixture.base());
        let err = apply_restore(&plan, tx.as_mut()).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Stale);
        journal.rollback(tx);
        assert_eq!(fixture.read("a.txt"), "three\n");
    }

    #[test]
    fn restore_replaces_directory_by_file_and_undo_brings_the_directory_back() {
        let fixture = Fixture::new(&[("item", "file\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal, false);
        fixture.remove("item");
        fixture.write("item/inner.txt", "inner\n");
        let before = fixture.state();
        restore(&fixture, &journal, &request(&[], false));
        assert_eq!(fixture.read("item"), "file\n");
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }
}
