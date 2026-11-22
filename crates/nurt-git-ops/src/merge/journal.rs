// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Writes of a merge through the journal transaction: working tree entries,
//! the index and references under Git lock files, reflogs and state files.
//!
//! Every target is declared and preserved before the first change. Files
//! guarded by a Git lock (`index`, references) are written as Git does:
//! the lock file is created, filled and renamed over the file, so a lock
//! held by another process fails the transaction with `E_GIT_LOCKED`.

use std::fs;
use std::path::{Path, PathBuf};

use bstr::{BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{LinkKind, MetaChange};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec};
use nurt_contracts::report::{Change, ChangeAction};

use super::trees::io_error;
use super::worktree::{Existing, Op};
use crate::state::{StateChange, abs_path};

/// Content of a step known only after the working tree is written.
pub(crate) enum Content {
    Ready(Mutation),
    Index,
}

struct Step {
    target: usize,
    content: Content,
    mode: Option<u32>,
}

/// Ordered list of journal targets and their changes.
#[derive(Default)]
pub(crate) struct Batch {
    targets: Vec<TargetSpec>,
    steps: Vec<Step>,
    changes: Vec<Change>,
    locks: Vec<PathBuf>,
}

impl Batch {
    fn declare(&mut self, path: &Path, action: TargetAction, expect: Expect) -> Result<usize> {
        self.targets.push(TargetSpec {
            path: TargetPath::Fs(abs_path(path)?),
            action,
            expect,
            size_hint: None,
        });
        Ok(self.targets.len() - 1)
    }

    fn step(&mut self, target: usize, content: Content) {
        self.steps.push(Step {
            target,
            content,
            mode: None,
        });
    }

    fn change(&mut self, path: &Path, action: ChangeAction) {
        self.changes.push(Change::new(path.display().to_string(), action));
    }

    /// Changes in target order, for the plan and the report.
    pub(crate) fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Adds the working tree operations of an update.
    pub(crate) fn worktree(&mut self, workdir: &Path, ops: &[Op], file_mode: bool) -> Result<()> {
        for op in ops {
            match op {
                Op::RemoveFile(path) => {
                    let full = join(workdir, path)?;
                    let target = self.declare(&full, TargetAction::Delete, Expect::Unchecked)?;
                    self.step(target, Content::Ready(Mutation::Remove));
                    self.change(&full, ChangeAction::Delete);
                }
                Op::RemoveDir(path) => {
                    let full = join(workdir, path)?;
                    let target = self.declare(&full, TargetAction::Delete, Expect::Unchecked)?;
                    self.step(target, Content::Ready(Mutation::Remove));
                    self.change(&full, ChangeAction::Delete);
                }
                Op::CreateDir(path) => {
                    let full = join(workdir, path)?;
                    let target = self.declare(&full, TargetAction::Create, Expect::Absent)?;
                    self.step(target, Content::Ready(Mutation::CreateDir { mode: 0o755 }));
                    self.change(&full, ChangeAction::Create);
                }
                Op::Write {
                    path,
                    data,
                    mode,
                    existed,
                } => self.write_file(workdir, path, data, *mode, *existed, file_mode)?,
                Op::Link { path, target, existed } => {
                    let full = join(workdir, path)?;
                    let (action, change) = match existed {
                        Existing::Nothing => (TargetAction::Create, ChangeAction::Create),
                        _ => (TargetAction::Replace, ChangeAction::Modify),
                    };
                    let expect = expect_for(*existed);
                    let index = self.declare(&full, action, expect)?;
                    self.step(
                        index,
                        Content::Ready(Mutation::CreateLink {
                            target: target.clone(),
                            kind: LinkKind::Symlink,
                        }),
                    );
                    self.change(&full, change);
                }
            }
        }
        Ok(())
    }

    fn write_file(
        &mut self,
        workdir: &Path,
        path: &BString,
        data: &[u8],
        mode: u32,
        existed: Existing,
        file_mode: bool,
    ) -> Result<()> {
        let full = join(workdir, path)?;
        let (action, change) = match existed {
            Existing::Nothing => (TargetAction::Create, ChangeAction::Create),
            Existing::File { .. } => (TargetAction::Modify, ChangeAction::Modify),
            Existing::Link => (TargetAction::Replace, ChangeAction::Modify),
        };
        let index = self.declare(&full, action, expect_for(existed))?;
        let previous_mode = match existed {
            Existing::File { mode } => Some(mode),
            _ => None,
        };
        let set_mode = file_mode && cfg!(unix) && previous_mode.map_or(mode != DEFAULT_NEW_MODE, |old| old & 0o111 != mode & 0o111);
        self.steps.push(Step {
            target: index,
            content: Content::Ready(Mutation::WriteBytes(data.to_vec())),
            mode: set_mode.then_some(mode),
        });
        self.change(&full, change);
        Ok(())
    }

    /// Adds a file written under a Git lock: `<file>.lock` is created with
    /// the content and renamed over `<file>`.
    pub(crate) fn locked(&mut self, file: &Path, content: Content) -> Result<()> {
        let lock = lock_path(file);
        let create = self.declare(&lock, TargetAction::Create, Expect::Absent)?;
        let rename = self.declare(
            &lock,
            TargetAction::Rename {
                to: abs_path(file)?,
                replace: true,
            },
            Expect::Unchecked,
        )?;
        self.step(create, content);
        self.step(rename, Content::Ready(Mutation::Rename));
        self.change(file, ChangeAction::Modify);
        self.locks.push(lock);
        Ok(())
    }

    /// Adds a line to a reflog, creating the log and its directories when
    /// absent.
    pub(crate) fn reflog(&mut self, log: &Path, line: Vec<u8>) -> Result<()> {
        if log.exists() {
            let target = self.declare(log, TargetAction::Append, Expect::Unchecked)?;
            self.step(target, Content::Ready(Mutation::AppendBytes(line)));
            self.change(log, ChangeAction::Modify);
            return Ok(());
        }
        self.parents(log)?;
        let target = self.declare(log, TargetAction::Create, Expect::Absent)?;
        self.step(target, Content::Ready(Mutation::WriteBytes(line)));
        self.change(log, ChangeAction::Create);
        Ok(())
    }

    fn parents(&mut self, file: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = file.parent();
        while let Some(dir) = current {
            if dir.exists() {
                break;
            }
            missing.push(dir.to_path_buf());
            current = dir.parent();
        }
        for dir in missing.into_iter().rev() {
            let target = self.declare(&dir, TargetAction::Create, Expect::Absent)?;
            self.step(target, Content::Ready(Mutation::CreateDir { mode: 0o755 }));
            self.change(&dir, ChangeAction::Create);
        }
        Ok(())
    }

    /// Adds a change of a state file.
    pub(crate) fn state(&mut self, git_dir: &Path, change: &StateChange) -> Result<()> {
        let spec = change.target(git_dir)?;
        let path = change.file().path(git_dir);
        let action = match change {
            StateChange::Write { .. } if path.exists() => ChangeAction::Modify,
            StateChange::Write { .. } => ChangeAction::Create,
            StateChange::Remove(_) => ChangeAction::Delete,
        };
        self.targets.push(spec);
        let target = self.targets.len() - 1;
        self.step(target, Content::Ready(change.mutation()));
        self.change(&path, action);
        Ok(())
    }

    /// Adds a plain file written in full, such as `COMMIT_EDITMSG`.
    pub(crate) fn file(&mut self, path: &Path, content: Vec<u8>) -> Result<()> {
        let action = if path.exists() {
            ChangeAction::Modify
        } else {
            ChangeAction::Create
        };
        let target = self.declare(path, TargetAction::Modify, Expect::Unchecked)?;
        self.step(target, Content::Ready(Mutation::WriteBytes(content)));
        self.change(path, action);
        Ok(())
    }

    /// Fails with `E_GIT_LOCKED` when a lock file of the batch exists.
    pub(crate) fn check_locks(&self, repo: &str) -> Result<()> {
        for lock in &self.locks {
            if lock.exists() {
                return Err(locked_error(lock, repo));
            }
        }
        Ok(())
    }

    /// Declares and preserves every target, then applies the steps in
    /// order; `index` produces the index content once the working tree is
    /// written.
    pub(crate) fn run(self, tx: &mut dyn MutationTx, repo: &str, index: &mut dyn FnMut() -> Result<Vec<u8>>) -> Result<()> {
        self.check_locks(repo)?;
        if self.targets.is_empty() {
            return Ok(());
        }
        let preserved = tx.preserve(&self.targets).map_err(|error| lock_conflict(error, &self.locks, repo))?;
        let indexes: Vec<TargetIndex> = preserved.iter().map(|p| p.index).collect();
        for step in self.steps {
            let target = indexes.get(step.target).copied().ok_or_else(|| {
                Error::new(ErrorCode::StageFailed, "the journal returned fewer targets than declared")
            })?;
            let mutation = match step.content {
                Content::Ready(mutation) => mutation,
                Content::Index => Mutation::WriteBytes(index()?),
            };
            tx.apply(target, mutation)?;
            if let Some(mode) = step.mode {
                tx.apply(
                    target,
                    Mutation::SetMeta(MetaChange {
                        mode: Some(mode),
                        ..MetaChange::default()
                    }),
                )?;
            }
        }
        Ok(())
    }
}

/// Permission bits a new file receives when no mode is set.
const DEFAULT_NEW_MODE: u32 = 0o644;

fn expect_for(existed: Existing) -> Expect {
    match existed {
        Existing::Nothing => Expect::Absent,
        _ => Expect::Unchecked,
    }
}

fn join(workdir: &Path, path: &BString) -> Result<PathBuf> {
    let rela = path.to_path().map_err(io_error("convert a path"))?;
    Ok(workdir.join(rela))
}

pub(crate) fn lock_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    name.push(".lock");
    file.with_file_name(name)
}

fn locked_error(lock: &Path, repo: &str) -> Error {
    Error::new(
        ErrorCode::GitLocked,
        format!(
            "{} exists; another Git process may be running, otherwise remove the file",
            lock.display()
        ),
    )
    .with_found(lock.display().to_string())
    .with_next_step(format!("git.status {repo}"))
}

fn lock_conflict(error: Error, locks: &[PathBuf], repo: &str) -> Error {
    if error.code() != ErrorCode::Exists {
        return error;
    }
    match locks.iter().find(|lock| lock.exists()) {
        Some(lock) => locked_error(lock, repo),
        None => error,
    }
}

/// Removes nothing and reads nothing: lock files are only inspected.
pub(crate) fn lock_exists(file: &Path) -> bool {
    fs::symlink_metadata(lock_path(file)).is_ok()
}
