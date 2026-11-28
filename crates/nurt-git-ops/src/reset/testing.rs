// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::reset::testing
// Responsibility:   Test support for `git.reset` and `git.stash`.
// Dependencies:     gix, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test support for `git.reset` and `git.stash`.
//!
//! [`Journal`] runs a transaction against the real file system and fails
//! when anything below the test directory changed without being declared;
//! the object store is exempt. Committed operations are undone byte for
//! byte with [`Journal::undo_last`]. The remaining helpers build
//! repositories with staged and unstaged changes and read their state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gix::bstr::{BStr, ByteSlice};
use gix::hash::ObjectId;
use gix::index::entry::{Flags, Mode, Stat};
use nurt_contracts::error::Result;
use nurt_contracts::journal::{MutationTx, RollbackReason};
use nurt_contracts::stage::{ExecCtx, StageFrame};
use nurt_git::apply::testjournal::{RecordingJournal, Snapshot, assert_declared, tree_state};
use nurt_git::testutil::{FixedClock, TestHost};
pub(crate) use nurt_git::testutil::{Fixture, RepoBuilder};

/// Time of operations in tests: one day after the first fixture commit.
pub(crate) const NOW: i64 = RepoBuilder::EPOCH + 86_400;

/// Committer of operations in tests.
pub(crate) const COMMITTER: &str = "Grace Hopper";

/// E-mail of [`COMMITTER`].
pub(crate) const COMMITTER_EMAIL: &str = "grace@example.com";

/// Execution context with a fixed time and identity.
pub(crate) fn ctx() -> ExecCtx {
    let host = TestHost::new("/nonexistent-home")
        .with_var("GIT_AUTHOR_NAME", COMMITTER)
        .with_var("GIT_AUTHOR_EMAIL", COMMITTER_EMAIL)
        .with_var("GIT_COMMITTER_NAME", COMMITTER)
        .with_var("GIT_COMMITTER_EMAIL", COMMITTER_EMAIL);
    ExecCtx::builder(Arc::new(FixedClock::at_unix_seconds(NOW)), Arc::new(host))
        .frame(StageFrame {
            command: "git.reset".to_owned(),
            ..StageFrame::default()
        })
        .build()
}

/// Journal for tests: runs a transaction on the real file system and
/// checks that every change below the test directory was declared.
#[derive(Debug, Default)]
pub(crate) struct Journal {
    inner: RecordingJournal,
}

impl Journal {
    /// Empty journal.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Runs `body` in a transaction on the files below `root`.
    ///
    /// On success the transaction is committed and every changed path must
    /// be a declared target or lie below one. On failure the applied
    /// changes are rolled back and the state below `root` must equal the
    /// state before.
    pub(crate) fn run<T>(
        &self,
        root: &Path,
        body: impl FnOnce(&mut dyn MutationTx) -> Result<T>,
    ) -> Result<T> {
        let before = tree_state(root);
        let mut tx = self.inner.begin();
        match body(&mut tx) {
            Ok(value) => {
                Box::new(tx).commit()?;
                assert_declared(root, &before, &tree_state(root), &self.inner);
                Ok(value)
            }
            Err(error) => {
                Box::new(tx).rollback(RollbackReason::TargetFailed)?;
                assert_eq!(
                    tree_state(root),
                    before,
                    "a failed operation left changes behind"
                );
                Err(error)
            }
        }
    }

    /// Restores the previous state of every target of the last committed
    /// operation.
    pub(crate) fn undo_last(&self) {
        self.inner.undo_last();
    }

    /// Paths of the targets of the last committed operation.
    pub(crate) fn last_targets(&self) -> Vec<PathBuf> {
        self.inner.last_declared()
    }
}

/// State of the test directory without the object store, for byte-level
/// comparison before and after an operation and its undo.
pub(crate) fn state(fx: &Fixture) -> BTreeMap<PathBuf, Snapshot> {
    tree_state(fx.path())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

/// Writes a working tree file, creating its directories.
pub(crate) fn write(fx: &Fixture, rel: &str, content: &str) {
    let path = fx.path().join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent directory");
    }
    std::fs::write(path, content).expect("file written");
}

/// Content of a working tree file, `None` when it does not exist.
pub(crate) fn read(fx: &Fixture, rel: &str) -> Option<String> {
    std::fs::read(fx.path().join(rel))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Removes a working tree file.
pub(crate) fn remove(fx: &Fixture, rel: &str) {
    std::fs::remove_file(fx.path().join(rel)).expect("file removed");
}

fn load_index(fx: &Fixture) -> gix::index::File {
    let path = fx.git_dir().join("index");
    if !path.exists() {
        let empty = gix::index::State::new(gix::hash::Kind::Sha1);
        return gix::index::File::from_state(empty, path);
    }
    gix::index::File::at(&path, gix::hash::Kind::Sha1, false, Default::default())
        .expect("index read")
}

fn store_index(fx: &Fixture, mut file: gix::index::File) {
    file.remove_tree();
    let mut out = Vec::new();
    file.write_to(&mut out, gix::index::write::Options::default())
        .expect("index serialized");
    std::fs::write(fx.git_dir().join("index"), out).expect("index written");
}

fn file_stat(path: &Path) -> Stat {
    gix::index::fs::Metadata::from_path_no_follow(path)
        .ok()
        .and_then(|meta| Stat::from_fs(&meta).ok())
        .unwrap_or_default()
}

/// Records the working tree content of `rel` in the index, or removes the
/// entry when the file does not exist, like `git add`.
pub(crate) fn stage(fx: &Fixture, rel: &str) {
    let repo = fx.open();
    let mut file = load_index(fx);
    let path: &BStr = rel.as_bytes().as_bstr();
    match std::fs::read(fx.path().join(rel)) {
        Ok(data) => {
            let id = repo.write_blob(&data).expect("blob written").detach();
            let mode = if is_executable(&fx.path().join(rel)) {
                Mode::FILE_EXECUTABLE
            } else {
                Mode::FILE
            };
            match file.entry_index_by_path(path) {
                Ok(position) => {
                    let entry = &mut file.entries_mut()[position];
                    entry.id = id;
                    entry.mode = mode;
                    entry.stat = file_stat(&fx.path().join(rel));
                }
                Err(_) => {
                    file.dangerously_push_entry(
                        file_stat(&fx.path().join(rel)),
                        id,
                        Flags::empty(),
                        mode,
                        path,
                    );
                    file.sort_entries();
                }
            }
        }
        Err(_) => file.remove_entries(|_, entry_path, _| entry_path == path),
    }
    store_index(fx, file);
}

/// Index entries as `(path, stage, object)`, in index order.
pub(crate) fn index_entries(fx: &Fixture) -> Vec<(String, u32, String)> {
    let file = load_index(fx);
    file.entries()
        .iter()
        .map(|entry| {
            (
                entry.path(&file).to_str_lossy().into_owned(),
                entry.stage() as u32,
                entry.id.to_hex().to_string(),
            )
        })
        .collect()
}

/// Object of the merged index entry of `rel`.
pub(crate) fn index_id(fx: &Fixture, rel: &str) -> Option<ObjectId> {
    let file = load_index(fx);
    file.entry_by_path_and_stage(
        rel.as_bytes().as_bstr(),
        gix::index::entry::Stage::Unconflicted,
    )
    .map(|entry| entry.id)
}

/// Object of a blob with `content`.
pub(crate) fn blob_id(content: &str) -> ObjectId {
    gix::objs::compute_hash(
        gix::hash::Kind::Sha1,
        gix::objs::Kind::Blob,
        content.as_bytes(),
    )
    .expect("hash computed")
}

/// Commit the revision `spec` names.
pub(crate) fn rev(fx: &Fixture, spec: &str) -> ObjectId {
    fx.open()
        .rev_parse_single(spec)
        .expect("revision")
        .object()
        .expect("object")
        .peel_to_commit()
        .expect("commit")
        .id
}

/// Content of a file in the repository directory, `None` when absent.
pub(crate) fn git_file(fx: &Fixture, name: &str) -> Option<String> {
    std::fs::read(fx.git_dir().join(name))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Messages of the reflog of `name`, oldest first.
pub(crate) fn reflog_messages(fx: &Fixture, name: &str) -> Vec<String> {
    git_file(fx, &format!("logs/{name}"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, message)| message.to_owned()))
        .collect()
}

/// Two commits on `main`: `first` with `a.txt`, `b.txt`, `dir/c.txt`;
/// `second` changes `a.txt`, removes `b.txt` and adds `new.txt`.
pub(crate) fn two_commits() -> Fixture {
    RepoBuilder::new()
        .file("a.txt", "a1\n")
        .file("b.txt", "b1\n")
        .file("dir/c.txt", "c1\n")
        .commit("first")
        .file("a.txt", "a2\n")
        .remove("b.txt")
        .file("new.txt", "n2\n")
        .commit("second")
        .build()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
