// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Writes of repository files under the lock protocol of `git`.
//!
//! A file such as `.git/index` or `.git/refs/heads/main` is replaced the way
//! `git` replaces it: the lock file `<file>.lock` is created exclusively,
//! the new content is written to it, and it is renamed over the file. Both
//! steps are targets of the transaction, so the previous content is kept by
//! the journal and an existing lock, held by a running `git` process or left
//! by a crash, fails the operation with `E_GIT_LOCKED` before anything
//! changes.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Applied, ExemptWrite, Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath,
    TargetSpec,
};

use super::{GitRepo, error};

/// Suffix of lock files.
pub const LOCK_SUFFIX: &str = ".lock";

/// Number of transaction targets declared for one [`LockedWrite`].
pub const TARGETS_PER_WRITE: u32 = 2;

/// Path of the lock file of `file`.
#[must_use]
pub fn lock_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

/// Replacement of one repository file through its lock file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedWrite {
    file: PathBuf,
    bytes: Vec<u8>,
}

impl LockedWrite {
    /// Replacement of `file` by `bytes`.
    #[must_use]
    pub fn new(file: impl Into<PathBuf>, bytes: Vec<u8>) -> Self {
        Self {
            file: file.into(),
            bytes,
        }
    }

    /// The file replaced.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The lock file.
    #[must_use]
    pub fn lock(&self) -> PathBuf {
        lock_path(&self.file)
    }

    /// The new content.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The two targets of the write, in the order they are applied: the
    /// lock file created exclusively, then its rename over the file.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the path is not absolute.
    pub fn targets(&self) -> Result<[TargetSpec; 2], Error> {
        let lock = abs(&self.lock())?;
        let file = abs(&self.file)?;
        let size = u64::try_from(self.bytes.len()).ok();
        Ok([
            TargetSpec {
                path: TargetPath::Fs(lock.clone()),
                action: TargetAction::Create,
                expect: Expect::Absent,
                size_hint: Some(0),
            },
            TargetSpec {
                path: TargetPath::Fs(lock),
                action: TargetAction::Rename {
                    to: file,
                    replace: true,
                },
                expect: Expect::Unchecked,
                size_hint: size,
            },
        ])
    }
}

fn abs(path: &Path) -> Result<AbsPath, Error> {
    AbsPath::new(path).map_err(|_| {
        Error::new(
            ErrorCode::Io,
            format!("repository path is not absolute: {}", path.display()),
        )
    })
}

/// Declaration of a group of locked writes within a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedWrites {
    writes: Vec<LockedWrite>,
}

impl LockedWrites {
    /// Group of the given writes, applied in order.
    #[must_use]
    pub fn new(writes: Vec<LockedWrite>) -> Self {
        Self { writes }
    }

    /// All targets of the group, two per write, for one call of
    /// [`MutationTx::preserve`] together with other targets.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a path is not absolute.
    pub fn targets(&self) -> Result<Vec<TargetSpec>, Error> {
        let mut targets = Vec::with_capacity(self.writes.len() * 2);
        for write in &self.writes {
            targets.extend(write.targets()?);
        }
        Ok(targets)
    }

    /// Declares the targets of the group in `tx`, the first one at the next
    /// index of the transaction, and maps an existing lock file to
    /// `E_GIT_LOCKED`.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_LOCKED` when a lock file exists, and with the
    /// errors of [`MutationTx::preserve`].
    pub fn declare(&self, tx: &mut dyn MutationTx) -> Result<TargetIndex, Error> {
        let targets = self.targets()?;
        let preserved = tx.preserve(&targets).map_err(|failure| self.lock_error(failure))?;
        Ok(preserved
            .first()
            .map_or(TargetIndex(0), |first| first.index))
    }

    /// Writes the content of every lock file and renames it over its file;
    /// `first` is the index returned by [`LockedWrites::declare`] or the
    /// position of the first target of [`LockedWrites::targets`] in a
    /// combined declaration.
    ///
    /// # Errors
    ///
    /// Fails with the errors of [`MutationTx::apply`]; the transaction is
    /// then rolled back.
    pub fn apply(&self, tx: &mut dyn MutationTx, first: TargetIndex) -> Result<Vec<Applied>, Error> {
        let mut applied = Vec::with_capacity(self.writes.len());
        let mut index = first.0;
        for write in &self.writes {
            tx.apply(TargetIndex(index), Mutation::WriteBytes(write.bytes.clone()))?;
            applied.push(tx.apply(TargetIndex(index + 1), Mutation::Rename)?);
            index += TARGETS_PER_WRITE;
        }
        Ok(applied)
    }

    /// Declares and applies the group in one transaction that has no other
    /// targets.
    ///
    /// # Errors
    ///
    /// As [`LockedWrites::declare`] and [`LockedWrites::apply`].
    pub fn run(&self, tx: &mut dyn MutationTx) -> Result<Vec<Applied>, Error> {
        let first = self.declare(tx)?;
        self.apply(tx, first)
    }

    fn lock_error(&self, failure: Error) -> Error {
        if failure.code != ErrorCode::Exists {
            return failure;
        }
        self.writes
            .iter()
            .map(LockedWrite::lock)
            .find(|lock| lock.exists())
            .map_or(failure, |lock| error::locked(&lock))
    }
}

/// Records in `tx` that objects were added to the object store of `repo`.
pub fn record_object_writes(tx: &mut dyn MutationTx, repo: &GitRepo) {
    if let Ok(common) = AbsPath::new(repo.common_dir()) {
        tx.record_exempt(ExemptWrite::ObjectStore { repo: common });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{RecordingJournal, RepoBuilder};

    #[test]
    fn locked_write_replaces_the_file_and_leaves_no_lock() {
        let fixture = RepoBuilder::new().build();
        let before = fixture.snapshot();
        let file = fixture.git_dir().join("description");
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        LockedWrites::new(vec![LockedWrite::new(&file, b"new\n".to_vec())])
            .run(tx.as_mut())
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new\n");
        assert!(!lock_path(&file).exists());
        journal.undo_last();
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn locked_write_creates_a_missing_file_and_undo_removes_it() {
        let fixture = RepoBuilder::new().build();
        let before = fixture.snapshot();
        let file = fixture.git_dir().join("ORIG_HEAD");
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        LockedWrites::new(vec![LockedWrite::new(&file, b"x\n".to_vec())])
            .run(tx.as_mut())
            .unwrap();
        tx.commit().unwrap();
        assert!(file.exists());
        journal.undo_last();
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn existing_lock_fails_with_git_locked_before_any_change() {
        let fixture = RepoBuilder::new().build();
        let file = fixture.git_dir().join("description");
        std::fs::write(lock_path(&file), b"").unwrap();
        let before = fixture.snapshot();
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let error = LockedWrites::new(vec![LockedWrite::new(&file, b"new\n".to_vec())])
            .run(tx.as_mut())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::GitLocked);
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn several_writes_use_consecutive_targets() {
        let fixture = RepoBuilder::new().build();
        let first = fixture.git_dir().join("A");
        let second = fixture.git_dir().join("B");
        let writes = LockedWrites::new(vec![
            LockedWrite::new(&first, b"1".to_vec()),
            LockedWrite::new(&second, b"2".to_vec()),
        ]);
        assert_eq!(writes.targets().unwrap().len(), 4);
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        writes.run(tx.as_mut()).unwrap();
        tx.commit().unwrap();
        assert_eq!(std::fs::read(&second).unwrap(), b"2");
        assert_eq!(journal.last_targets().len(), 4);
    }

    #[test]
    fn lock_path_appends_the_suffix() {
        assert_eq!(lock_path(Path::new("/r/.git/index")), PathBuf::from("/r/.git/index.lock"));
    }
}
