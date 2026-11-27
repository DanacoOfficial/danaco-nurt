// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::checkpoint::test_journal
// Responsibility:   Repositories and a checked journal for the tests of the checkpoint and.
// Dependencies:     gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Repositories and a checked journal for the tests of the checkpoint and
//! working tree commands.
//!
//! [`RecordingJournal`] wraps the journal of [`crate::apply::testjournal`] and compares
//! the directory of the fixture before and after every committed
//! transaction: a change outside the declared targets, including the
//! destinations of renames, panics. Additions to the object database are
//! exempt and ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::Result;
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

use crate::apply;
pub(crate) use crate::testutil::test_ctx;
use crate::testutil::{self, RepoBuilder, TreeEntry};

use super::support::{TreeEntry as Blob, flatten_tree, serialize_index};

/// Path fragment of the object database below the fixture directory.
const OBJECTS: &str = ".git/objects";

/// Path fragment of checkpoint references and their logs.
const CHECKPOINT_REFS: &str = "refs/nurt";

/// Repository with a working tree in a temporary directory.
pub(crate) struct Fixture {
    inner: testutil::Fixture,
    /// Root of the working tree.
    pub(crate) root: PathBuf,
}

impl Fixture {
    /// Repository with one commit holding `files` on branch `main`.
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        let mut builder = RepoBuilder::new();
        for (path, content) in files {
            builder = builder.file(path, content);
        }
        Self::build(builder.commit("initial"))
    }

    /// Repository without commits.
    pub(crate) fn empty() -> Self {
        Self::build(RepoBuilder::new())
    }

    /// Repository made by `builder`.
    pub(crate) fn build(builder: RepoBuilder) -> Self {
        let inner = builder.build();
        let root = inner.path().to_path_buf();
        Self { inner, root }
    }

    /// Directory that holds the repository and every working tree added
    /// next to it; transactions are checked below it.
    pub(crate) fn base(&self) -> &Path {
        self.inner.temp_dir()
    }

    /// Opens the repository.
    pub(crate) fn open(&self) -> gix::Repository {
        self.inner.open()
    }

    /// Writes text to a file of the working tree, creating directories.
    pub(crate) fn write(&self, path: &str, content: &str) {
        self.write_bytes(path, content.as_bytes());
    }

    /// Writes bytes to a file of the working tree, creating directories.
    pub(crate) fn write_bytes(&self, path: &str, content: &[u8]) {
        let file = self.root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    /// Removes a file or a directory of the working tree.
    pub(crate) fn remove(&self, path: &str) {
        let file = self.root.join(path);
        if file.is_dir() {
            std::fs::remove_dir_all(file).unwrap();
        } else {
            std::fs::remove_file(file).unwrap();
        }
    }

    /// Text of a file of the working tree.
    pub(crate) fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).unwrap()
    }

    /// Records the working tree content of `paths` in the index.
    pub(crate) fn stage(&self, paths: &[&str]) {
        let repo = self.open();
        let mut entries = index_entries(&repo);
        for path in paths {
            let content = std::fs::read(self.root.join(path)).unwrap();
            let id = repo.write_blob(&content).unwrap().detach();
            entries.insert(BString::from(*path), (EntryKind::Blob, id));
        }
        write_index(&repo, &entries);
    }

    /// Resets the index to the tree of `HEAD`.
    pub(crate) fn unstage_all(&self) {
        let repo = self.open();
        let tree = repo.head_commit().unwrap().tree_id().unwrap().detach();
        let entries = flatten_tree(&repo, tree).unwrap();
        write_index(&repo, &entries);
    }

    /// Paths recorded in the index, in index order.
    pub(crate) fn index_paths(&self) -> Vec<String> {
        let repo = self.open();
        let index = repo.index_or_empty().unwrap();
        index
            .entries()
            .iter()
            .map(|entry| entry.path(&index).to_str_lossy().into_owned())
            .collect()
    }

    /// Every entry below [`Fixture::base`] except the object database.
    pub(crate) fn state(&self) -> BTreeMap<PathBuf, TreeEntry> {
        checked_state(self.base())
    }

    /// [`Fixture::state`] without checkpoint references and their logs.
    pub(crate) fn state_without_refs(&self) -> BTreeMap<PathBuf, TreeEntry> {
        self.state()
            .into_iter()
            .filter(|(path, _)| {
                !path
                    .to_string_lossy()
                    .replace('\\', "/")
                    .contains(CHECKPOINT_REFS)
            })
            .collect()
    }

    /// Identifier of the commit `revision` names.
    pub(crate) fn rev(&self, revision: &str) -> ObjectId {
        self.open().rev_parse_single(revision).unwrap().detach()
    }
}

fn index_entries(repo: &gix::Repository) -> BTreeMap<BString, Blob> {
    let index = repo.index_or_empty().unwrap();
    index
        .entries()
        .iter()
        .map(|entry| {
            let kind = entry.mode.to_tree_entry_mode().unwrap().kind();
            (entry.path(&index).to_owned(), (kind, entry.id))
        })
        .collect()
}

fn write_index(repo: &gix::Repository, entries: &BTreeMap<BString, Blob>) {
    let mut state = gix::index::State::new(repo.object_hash());
    for (path, (kind, id)) in entries {
        let mode = if *kind == EntryKind::BlobExecutable {
            gix::index::entry::Mode::FILE_EXECUTABLE
        } else {
            gix::index::entry::Mode::FILE
        };
        state.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            *id,
            gix::index::entry::Flags::empty(),
            mode,
            path.as_bstr(),
        );
    }
    state.sort_entries();
    let bytes = serialize_index(state, &repo.index_path()).unwrap();
    std::fs::write(repo.index_path(), bytes).unwrap();
}

fn checked_state(base: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    testutil::snapshot(base)
        .into_iter()
        .filter(|(path, _)| !path.to_string_lossy().replace('\\', "/").contains(OBJECTS))
        .collect()
}

/// Paths declared by a transaction, shared with the journal that checks them.
type Declared = Arc<Mutex<Vec<PathBuf>>>;

struct Watch {
    base: PathBuf,
    before: BTreeMap<PathBuf, TreeEntry>,
    declared: Declared,
}

/// Journal that panics when a transaction changes an undeclared entry.
#[derive(Default)]
pub(crate) struct RecordingJournal {
    inner: apply::testjournal::RecordingJournal,
    watched: Mutex<Option<Watch>>,
}

impl RecordingJournal {
    /// Opens a transaction whose changes are checked below `base`.
    pub(crate) fn begin(&self, base: &Path) -> Box<dyn MutationTx + '_> {
        let declared = Declared::default();
        *self.watched.lock().unwrap() = Some(Watch {
            base: base.to_path_buf(),
            before: checked_state(base),
            declared: Arc::clone(&declared),
        });
        Box::new(CheckedTx {
            inner: Box::new(self.inner.begin()),
            declared,
        })
    }

    /// Commits `tx` and checks that only declared targets changed.
    pub(crate) fn commit(&self, tx: Box<dyn MutationTx + '_>) {
        tx.commit().unwrap();
        let watch = self.watched.lock().unwrap().take().unwrap();
        let after = checked_state(&watch.base);
        let declared = watch.declared.lock().unwrap().clone();
        let changed = watch
            .before
            .keys()
            .chain(after.keys())
            .filter(|path| watch.before.get(*path) != after.get(*path));
        for path in changed {
            let absolute = watch.base.join(path);
            assert!(
                declared
                    .iter()
                    .any(|target| absolute.starts_with(target) || target.starts_with(&absolute)),
                "{} changed outside the transaction",
                path.display()
            );
        }
    }

    /// Rolls `tx` back.
    pub(crate) fn rollback(&self, tx: Box<dyn MutationTx + '_>) {
        self.watched.lock().unwrap().take();
        tx.rollback(RollbackReason::TargetFailed).unwrap();
    }

    /// Undoes the last committed transaction.
    pub(crate) fn undo_last(&self) {
        self.inner.undo_last();
    }
}

/// Transaction that records every declared path before passing the call on.
struct CheckedTx<'j> {
    inner: Box<dyn MutationTx + 'j>,
    declared: Declared,
}

impl MutationTx for CheckedTx<'_> {
    fn op(&self) -> Option<OpId> {
        self.inner.op()
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let mut declared = self.declared.lock().unwrap();
        for spec in targets {
            if let TargetPath::Fs(path) = &spec.path {
                declared.push(path.as_path().to_path_buf());
            }
            if let TargetAction::Rename { to, .. } = &spec.action {
                declared.push(to.as_path().to_path_buf());
            }
        }
        drop(declared);
        self.inner.preserve(targets)
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        self.inner.stage(target)
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.inner.apply(target, mutation)
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.inner.observe_external(changes)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.inner.mark_irreversible(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.inner.record_exempt(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        self.inner.commit()
    }

    fn rollback(self: Box<Self>, reason: RollbackReason) -> Result<RollbackReport> {
        self.inner.rollback(reason)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
