// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journaled changes of repository files.
//!
//! A [`FilePlan`] lists the journal targets of a set of file changes and the
//! mutations that perform them. A file guarded by a `git` lock (`<file>.lock`)
//! is one `modify` target expecting the content seen at planning time, or its
//! absence. The lock file is honoured but not created: when it exists the
//! plan fails with `E_GIT_LOCKED`, at planning time and again before the
//! first file is replaced. Callers combine several plans into one
//! [`MutationTx::preserve`] call, because a transaction declares every target
//! before its first change.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};

use super::refs::content_hash;

/// Permission bits of directories created for references and logs, before
/// the process umask.
const DIRECTORY_MODE: u32 = 0o777;

/// Suffix of `git` lock files.
pub const LOCK_SUFFIX: &str = ".lock";

/// Check run after the directories of a plan are created and before any file
/// is replaced.
pub type LockedCheck = Box<dyn Fn() -> Result<()> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Lock,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    CreateDir(u32),
    Write(u32, Vec<u8>),
    Remove(u32),
    Append(u32, Vec<u8>),
}

/// Planned file changes with their journal targets.
#[derive(Default)]
pub struct FilePlan {
    targets: Vec<TargetSpec>,
    steps: Vec<(Phase, Step)>,
    planned_dirs: HashSet<PathBuf>,
    locked: HashSet<PathBuf>,
    checks: Vec<LockedCheck>,
}

impl std::fmt::Debug for FilePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilePlan")
            .field("targets", &self.targets)
            .field("steps", &self.steps.len())
            .field("checks", &self.checks.len())
            .finish()
    }
}

impl FilePlan {
    /// An empty plan.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the plan changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Journal targets in declaration order.
    #[must_use]
    pub fn targets(&self) -> &[TargetSpec] {
        &self.targets
    }

    /// Paths of the files the plan changes, without directories.
    #[must_use]
    pub fn changed_files(&self) -> Vec<PathBuf> {
        self.targets
            .iter()
            .filter_map(|target| match (&target.path, &target.action) {
                (
                    TargetPath::Fs(path),
                    TargetAction::Delete | TargetAction::Append | TargetAction::Modify,
                ) => Some(path.as_path().to_path_buf()),
                (TargetPath::Fs(path), TargetAction::Create)
                    if !self.planned_dirs.contains(path.as_path()) =>
                {
                    Some(path.as_path().to_path_buf())
                }
                _ => None,
            })
            .collect()
    }

    /// Replaces `file` with `bytes`, expecting the content the file has now
    /// (or its absence); missing directories up to `root` are created.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_LOCKED` when the lock file `<file>.lock` exists and
    /// with `E_IO` when a path is not absolute or the file cannot be read.
    pub fn replace_locked(&mut self, root: &Path, file: &Path, bytes: Vec<u8>) -> Result<()> {
        let lock = lock_path(file);
        if lock.exists() || self.locked.contains(&lock) {
            return Err(locked_error(&lock));
        }
        if let Some(parent) = file.parent() {
            self.ensure_dirs(root, parent)?;
        }
        let expect = match std::fs::read(file) {
            Ok(current) => Expect::Hash(content_hash(&current)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Expect::Absent,
            Err(error) => {
                return Err(Error::new(
                    ErrorCode::Io,
                    format!("cannot read `{}`: {error}", file.display()),
                ));
            }
        };
        let target = self.push_target(file, TargetAction::Modify, expect)?;
        self.locked.insert(lock);
        self.steps.push((Phase::Commit, Step::Write(target, bytes)));
        Ok(())
    }

    /// Deletes `file`, which must have the state `expect` at the change.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_LOCKED` when `<file>.lock` exists.
    pub fn delete(&mut self, file: &Path, expect: Expect) -> Result<()> {
        let lock = lock_path(file);
        if lock.exists() || self.locked.contains(&lock) {
            return Err(locked_error(&lock));
        }
        let target = self.push_target(file, TargetAction::Delete, expect)?;
        self.steps.push((Phase::Commit, Step::Remove(target)));
        Ok(())
    }

    /// Appends `bytes` to `file`, creating it and missing directories up to
    /// `root` when it does not exist.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a path is not absolute.
    pub fn append(&mut self, root: &Path, file: &Path, bytes: Vec<u8>) -> Result<()> {
        if file.is_file() {
            let target = self.push_target(file, TargetAction::Append, Expect::Unchecked)?;
            self.steps
                .push((Phase::Commit, Step::Append(target, bytes)));
            return Ok(());
        }
        if let Some(parent) = file.parent() {
            self.ensure_dirs(root, parent)?;
        }
        let target = self.push_target(file, TargetAction::Create, Expect::Absent)?;
        self.steps.push((Phase::Commit, Step::Write(target, bytes)));
        Ok(())
    }

    /// Adds a check that runs before the files of the plan are replaced.
    pub fn check_when_locked(&mut self, check: LockedCheck) {
        self.checks.push(check);
    }

    /// Applies the plan whose first target has index `first`.
    ///
    /// Directories are created first, then the checks run and the lock files
    /// are looked for, then the files are replaced, deleted and appended in
    /// plan order.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_LOCKED` when a lock file appeared since planning;
    /// otherwise returns the error of a failed mutation or check. The caller
    /// rolls the transaction back.
    pub fn apply(&self, tx: &mut dyn MutationTx, first: TargetIndex) -> Result<()> {
        for phase in [Phase::Lock, Phase::Commit] {
            for (step_phase, step) in &self.steps {
                if *step_phase == phase {
                    apply_step(tx, first, step)?;
                }
            }
            if phase == Phase::Lock {
                for check in &self.checks {
                    check()?;
                }
                if let Some(lock) = self.locked.iter().find(|lock| lock.exists()) {
                    return Err(locked_error(lock));
                }
            }
        }
        Ok(())
    }

    fn ensure_dirs(&mut self, root: &Path, dir: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = Some(dir);
        while let Some(path) = current {
            if path == root || !path.starts_with(root) || path.is_dir() {
                break;
            }
            if path.exists() {
                return Err(Error::new(
                    ErrorCode::GitRef,
                    format!(
                        "`{}` is a file where a directory of references is needed; delete or rename the conflicting reference",
                        path.display()
                    ),
                )
                .with_problem("name_conflict"));
            }
            if !self.planned_dirs.contains(path) {
                missing.push(path.to_path_buf());
            }
            current = path.parent();
        }
        for path in missing.into_iter().rev() {
            let target = self.push_target(&path, TargetAction::Create, Expect::Absent)?;
            self.planned_dirs.insert(path);
            self.steps.push((Phase::Lock, Step::CreateDir(target)));
        }
        Ok(())
    }

    fn push_target(&mut self, path: &Path, action: TargetAction, expect: Expect) -> Result<u32> {
        let index = u32::try_from(self.targets.len())
            .map_err(|_| Error::new(ErrorCode::Internal, "the plan has too many targets"))?;
        self.targets.push(TargetSpec {
            path: TargetPath::Fs(abs(path)?),
            action,
            expect,
            size_hint: None,
        });
        Ok(index)
    }
}

/// Declares the targets of all `plans` in one call and applies the plans in
/// order.
///
/// # Errors
///
/// Fails with the error of the journal or of a plan, `E_GIT_LOCKED` when a
/// lock file exists.
pub fn preserve_and_apply(tx: &mut dyn MutationTx, plans: &[&FilePlan]) -> Result<()> {
    let targets: Vec<TargetSpec> = plans
        .iter()
        .flat_map(|plan| plan.targets.iter().cloned())
        .collect();
    if targets.is_empty() {
        return Ok(());
    }
    let preserved = tx.preserve(&targets).map_err(map_lock_exists)?;
    let mut base = preserved.first().map_or(0, |entry| entry.index.0);
    for plan in plans {
        plan.apply(tx, TargetIndex(base))?;
        base += u32::try_from(plan.targets.len()).unwrap_or(u32::MAX);
    }
    Ok(())
}

fn apply_step(tx: &mut dyn MutationTx, first: TargetIndex, step: &Step) -> Result<()> {
    let at = |offset: u32| TargetIndex(first.0 + offset);
    let result = match step {
        Step::CreateDir(target) => tx.apply(
            at(*target),
            Mutation::CreateDir {
                mode: DIRECTORY_MODE,
            },
        ),
        Step::Write(target, bytes) => tx.apply(at(*target), Mutation::WriteBytes(bytes.clone())),
        Step::Remove(target) => tx.apply(at(*target), Mutation::Remove),
        Step::Append(target, bytes) => tx.apply(at(*target), Mutation::AppendBytes(bytes.clone())),
    };
    result.map(|_| ()).map_err(map_lock_exists)
}

fn map_lock_exists(error: Error) -> Error {
    if error.code() == ErrorCode::Exists {
        return Error::new(
            ErrorCode::GitLocked,
            format!(
                "a lock file of the repository exists: {}; wait for the other git process or remove the stale lock",
                error.message
            ),
        )
        .with_cause(error);
    }
    error
}

/// Lock file of `file`.
#[must_use]
pub fn lock_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

fn locked_error(lock: &Path) -> Error {
    Error::new(
        ErrorCode::GitLocked,
        format!(
            "the lock file `{}` exists; wait for the other git process or remove the stale lock",
            lock.display()
        ),
    )
    .with_detail(
        "path",
        serde_json::Value::String(lock.display().to_string()),
    )
}

fn abs(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("`{}` is not an absolute path: {error}", path.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send_pack::testing::RecordingTx;

    #[test]
    fn replaces_a_file_in_one_target_and_undoes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("refs").join("heads").join("main");
        let mut plan = FilePlan::new();
        plan.replace_locked(&root, &file, b"new\n".to_vec())
            .unwrap();
        plan.append(&root, &root.join("logs").join("x"), b"line\n".to_vec())
            .unwrap();
        assert_eq!(
            plan.changed_files(),
            vec![file.clone(), root.join("logs").join("x")]
        );
        let mut tx = RecordingTx::new();
        preserve_and_apply(&mut tx, &[&plan]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new\n");
        assert!(!lock_path(&file).exists());
        assert!(tx.targets().iter().all(|target| {
            !matches!(target.action, TargetAction::Rename { .. })
                && !matches!(&target.path, TargetPath::Fs(path) if path.as_path().ends_with("main.lock"))
        }));
        tx.undo();
        assert!(!root.join("refs").exists());
        assert!(!root.join("logs").exists());
    }

    #[test]
    fn rejects_existing_locks_and_name_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file = root.join("main");
        std::fs::write(lock_path(&file), b"").unwrap();
        let mut plan = FilePlan::new();
        let error = plan.replace_locked(&root, &file, Vec::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitLocked);
        assert_eq!(
            plan.delete(&file, Expect::Unchecked).unwrap_err().code(),
            ErrorCode::GitLocked
        );
        std::fs::write(root.join("a"), b"x").unwrap();
        let error = plan
            .replace_locked(&root, &root.join("a").join("b"), Vec::new())
            .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("name_conflict"));
        let error = plan
            .replace_locked(&root, Path::new("relative"), Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
        assert!(plan.is_empty());
        let mut tx = RecordingTx::new();
        preserve_and_apply(&mut tx, &[&plan]).unwrap();
    }

    #[test]
    fn runs_checks_before_replacing_and_detects_lock_races() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file = root.join("main");
        std::fs::write(&file, b"old\n").unwrap();
        let mut plan = FilePlan::new();
        plan.replace_locked(&root, &file, b"new\n".to_vec())
            .unwrap();
        let watched = file.clone();
        plan.check_when_locked(Box::new(move || {
            if std::fs::read(&watched).is_ok_and(|bytes| bytes == b"old\n") {
                Err(Error::new(ErrorCode::Stale, "changed"))
            } else {
                Ok(())
            }
        }));
        let mut tx = RecordingTx::new();
        let error = preserve_and_apply(&mut tx, &[&plan]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(std::fs::read(&file).unwrap(), b"old\n");

        let mut plan = FilePlan::new();
        plan.replace_locked(&root, &file, b"new\n".to_vec())
            .unwrap();
        std::fs::write(lock_path(&file), b"").unwrap();
        let mut tx = RecordingTx::new();
        let error = preserve_and_apply(&mut tx, &[&plan]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitLocked);
        assert_eq!(std::fs::read(&file).unwrap(), b"old\n");
    }

    #[test]
    fn deletes_and_appends_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file = root.join("ref");
        let log = root.join("log");
        std::fs::write(&file, b"x\n").unwrap();
        std::fs::write(&log, b"a\n").unwrap();
        let mut plan = FilePlan::new();
        plan.delete(&file, Expect::Unchecked).unwrap();
        plan.append(&root, &log, b"b\n".to_vec()).unwrap();
        let mut tx = RecordingTx::new();
        preserve_and_apply(&mut tx, &[&plan, &FilePlan::new()]).unwrap();
        assert!(!file.exists());
        assert_eq!(std::fs::read(&log).unwrap(), b"a\nb\n");
        tx.undo();
        assert_eq!(std::fs::read(&file).unwrap(), b"x\n");
        assert_eq!(std::fs::read(&log).unwrap(), b"a\n");
        assert!(format!("{plan:?}").contains("FilePlan"));
    }
}
