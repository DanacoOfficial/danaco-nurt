// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Writes of repository files that respect the lock files of `git`.
//!
//! A file such as `.git/index` or `.git/ORIG_HEAD` is replaced by one
//! target of the transaction: `modify`, expecting the content read when the
//! target is declared, or the absence of the file. The journal keeps the
//! previous content, and a change of the file by another program between
//! the declaration and the write fails the operation with `E_STALE`.
//!
//! The lock file `<file>.lock` of `git` is honoured but not created: when it
//! exists, a running `git` process is changing the file or a previous run
//! ended without removing it, and the operation fails with `E_GIT_LOCKED`.
//! The lock is checked when the targets are declared and again before each
//! file is written.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{AbsPath, ContentHash};
use nurt_contracts::journal::{
    Applied, ExemptWrite, Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath,
    TargetSpec,
};
use sha2::{Digest, Sha256};

use super::{GitRepo, error};

/// Suffix of lock files.
pub const LOCK_SUFFIX: &str = ".lock";

/// Number of transaction targets declared for one [`LockedWrite`].
pub const TARGETS_PER_WRITE: u32 = 1;

/// Path of the lock file of `file`.
#[must_use]
pub fn lock_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

/// Hash of `bytes` in the form the journal compares.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

/// Checks that no `git` process holds the lock of `file`.
///
/// # Errors
///
/// Fails with `E_GIT_LOCKED` when `<file>.lock` exists.
pub fn ensure_unlocked(file: &Path) -> Result<(), Error> {
    let lock = lock_path(file);
    if std::fs::symlink_metadata(&lock).is_ok() {
        return Err(error::locked(&lock));
    }
    Ok(())
}

/// Replacement of one repository file.
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

    /// The lock file of `git` for the file.
    #[must_use]
    pub fn lock(&self) -> PathBuf {
        lock_path(&self.file)
    }

    /// The new content.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The target of the write: `modify` of the file, expecting its current
    /// content, or its absence when it does not exist.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the path is not absolute or the file exists
    /// but cannot be read.
    pub fn target(&self) -> Result<TargetSpec, Error> {
        let expect = match std::fs::read(&self.file) {
            Ok(previous) => Expect::Hash(content_hash(&previous)),
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => Expect::Absent,
            Err(failure) => return Err(error::io(&self.file, &failure)),
        };
        Ok(TargetSpec {
            path: TargetPath::Fs(abs(&self.file)?),
            action: TargetAction::Modify,
            expect,
            size_hint: u64::try_from(self.bytes.len()).ok(),
        })
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

    /// The writes of the group.
    #[must_use]
    pub fn writes(&self) -> &[LockedWrite] {
        &self.writes
    }

    /// All targets of the group, one per write, for one call of
    /// [`MutationTx::preserve`] together with other targets.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_LOCKED` when a lock file exists and with the
    /// errors of [`LockedWrite::target`].
    pub fn targets(&self) -> Result<Vec<TargetSpec>, Error> {
        self.ensure_unlocked()?;
        self.writes.iter().map(LockedWrite::target).collect()
    }

    /// Declares the targets of the group in `tx` and returns the index of
    /// the first one.
    ///
    /// # Errors
    ///
    /// As [`LockedWrites::targets`], and with the errors of
    /// [`MutationTx::preserve`].
    pub fn declare(&self, tx: &mut dyn MutationTx) -> Result<TargetIndex, Error> {
        let targets = self.targets()?;
        let preserved = tx.preserve(&targets)?;
        Ok(preserved
            .first()
            .map_or(TargetIndex(0), |first| first.index))
    }

    /// Writes the content of every file; `first` is the index returned by
    /// [`LockedWrites::declare`] or the position of the first target of
    /// [`LockedWrites::targets`] in a combined declaration.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_LOCKED` when a lock file appeared after the
    /// declaration, `E_STALE` when a file changed after it, and with the
    /// other errors of [`MutationTx::apply`].
    pub fn apply(
        &self,
        tx: &mut dyn MutationTx,
        first: TargetIndex,
    ) -> Result<Vec<Applied>, Error> {
        let mut applied = Vec::with_capacity(self.writes.len());
        let mut index = first.0;
        for write in &self.writes {
            ensure_unlocked(&write.file)?;
            applied.push(tx.apply(
                TargetIndex(index),
                Mutation::WriteBytes(write.bytes.clone()),
            )?);
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

    fn ensure_unlocked(&self) -> Result<(), Error> {
        self.writes
            .iter()
            .try_for_each(|write| ensure_unlocked(&write.file))
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

    fn single(file: &Path, bytes: &[u8]) -> LockedWrites {
        LockedWrites::new(vec![LockedWrite::new(file, bytes.to_vec())])
    }

    #[test]
    fn locked_write_replaces_the_file_and_undo_restores_it() {
        let fixture = RepoBuilder::new().build();
        let before = fixture.snapshot();
        let file = fixture.git_dir().join("description");
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        single(&file, b"new\n").run(tx.as_mut()).unwrap();
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
        let writes = single(&file, b"x\n");
        assert_eq!(writes.targets().unwrap()[0].expect, Expect::Absent);
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        writes.run(tx.as_mut()).unwrap();
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
        let failure = single(&file, b"new\n").run(tx.as_mut()).unwrap_err();
        assert_eq!(failure.code, ErrorCode::GitLocked);
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn lock_taken_after_the_declaration_fails_the_write() {
        let fixture = RepoBuilder::new().build();
        let file = fixture.git_dir().join("description");
        let writes = single(&file, b"new\n");
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let first = writes.declare(tx.as_mut()).unwrap();
        std::fs::write(lock_path(&file), b"").unwrap();
        let failure = writes.apply(tx.as_mut(), first).unwrap_err();
        assert_eq!(failure.code, ErrorCode::GitLocked);
    }

    #[test]
    fn change_after_the_declaration_is_stale() {
        let fixture = RepoBuilder::new().build();
        let file = fixture.git_dir().join("description");
        let writes = single(&file, b"new\n");
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let first = writes.declare(tx.as_mut()).unwrap();
        std::fs::write(&file, b"changed by another program\n").unwrap();
        let failure = writes.apply(tx.as_mut(), first).unwrap_err();
        assert_eq!(failure.code, ErrorCode::Stale);
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
        assert_eq!(writes.targets().unwrap().len(), 2);
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        writes.run(tx.as_mut()).unwrap();
        tx.commit().unwrap();
        assert_eq!(std::fs::read(&second).unwrap(), b"2");
        assert_eq!(journal.last_targets(), vec![first, second]);
    }

    #[test]
    fn lock_path_appends_the_suffix() {
        assert_eq!(
            lock_path(Path::new("/r/.git/index")),
            PathBuf::from("/r/.git/index.lock")
        );
    }

    #[test]
    fn content_hash_is_sha256() {
        assert_eq!(
            content_hash(b"").to_string(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
