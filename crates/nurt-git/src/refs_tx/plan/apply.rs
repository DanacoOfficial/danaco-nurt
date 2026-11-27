// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::refs_tx::plan::apply
// Responsibility:   Declaration and execution of the planned file changes.
// Dependencies:     nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Declaration and execution of the planned file changes.
//!
//! Every file is one target of the mutation transaction. A written file is
//! declared as `modify` with the expected previous content (`Expect::Hash`,
//! or `Expect::Absent` for a new file), so the journal checks the previous
//! value atomically with the switch. A lock file of Git (`<file>.lock`) held
//! by another process fails the transaction with `E_GIT_LOCKED`; the check is
//! made when planning, when declaring and immediately before the first switch.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};

use super::super::io_error;
use super::super::read::{is_missing, read_loose};
use super::{Guard, Plan};

/// Permission bits of created reference directories, before the umask.
const DIR_MODE: u32 = 0o777;

/// Target indexes of one step, in the order of [`Step::specs`].
pub(in crate::refs_tx) type StepTargets = Vec<TargetIndex>;

/// One planned file change.
#[derive(Debug, Clone)]
pub(in crate::refs_tx) enum Step {
    /// Missing parent directory of a new file.
    CreateDir(AbsPath),
    /// Empty directory tree at the path of a new file.
    ClearDir(AbsPath),
    /// Complete new content of `path`; `lock` is the lock file of Git that
    /// must not exist.
    Write {
        lock: AbsPath,
        path: AbsPath,
        content: Vec<u8>,
        expect: Expect,
    },
    /// Deleted loose reference with its previous content.
    DeleteRef {
        lock: AbsPath,
        path: AbsPath,
        old: Vec<u8>,
    },
    /// Deleted reflog.
    DeleteLog(AbsPath),
    /// Entries appended to an existing reflog.
    AppendLog { path: AbsPath, bytes: Vec<u8> },
    /// New reflog.
    CreateLog { path: AbsPath, bytes: Vec<u8> },
    /// Reflog that overwrites the existing one with content `old`.
    ReplaceLog {
        path: AbsPath,
        old: Vec<u8>,
        bytes: Vec<u8>,
    },
    /// Directory left empty by the transaction.
    RemoveDir(AbsPath),
}

impl Step {
    /// Lock file of Git guarding the file of the step.
    pub(in crate::refs_tx) fn lock(&self) -> Option<&AbsPath> {
        match self {
            Self::Write { lock, .. } | Self::DeleteRef { lock, .. } => Some(lock),
            _ => None,
        }
    }

    /// Declared target of the step.
    fn spec(&self) -> TargetSpec {
        match self {
            Self::CreateDir(path) => spec(path, TargetAction::Create, Expect::Absent),
            Self::ClearDir(path) | Self::DeleteLog(path) | Self::RemoveDir(path) => {
                spec(path, TargetAction::Delete, Expect::Unchecked)
            }
            Self::Write { path, expect, .. } => spec(path, TargetAction::Modify, expect.clone()),
            Self::DeleteRef { path, old, .. } => {
                spec(path, TargetAction::Delete, Expect::Hash(content_hash(old)))
            }
            Self::AppendLog { path, .. } => spec(path, TargetAction::Append, Expect::Unchecked),
            Self::CreateLog { path, .. } => spec(path, TargetAction::Create, Expect::Absent),
            Self::ReplaceLog { path, old, .. } => {
                spec(path, TargetAction::Modify, Expect::Hash(content_hash(old)))
            }
        }
    }
}

fn spec(path: &AbsPath, action: TargetAction, expect: Expect) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect,
        size_hint: None,
    }
}

/// SHA-256 of `bytes`, the content hash of the journal.
pub(in crate::refs_tx) fn content_hash(bytes: &[u8]) -> nurt_contracts::fs::ContentHash {
    use sha2::Digest;
    nurt_contracts::fs::ContentHash(sha2::Sha256::digest(bytes).into())
}

/// `E_GIT_LOCKED` for an existing lock file.
pub(in crate::refs_tx) fn locked(lock: &AbsPath) -> Error {
    let path = lock.to_language_string();
    let quoted = serde_json::Value::String(path.clone()).to_string();
    Error::new(
        ErrorCode::GitLocked,
        format!(
            "lock file {path} exists; finish the other git process or remove the stale lock file"
        ),
    )
    .with_problem("locked")
    .with_found(path.clone())
    .with_fix(format!("remove {quoted}"))
    .with_detail("path", serde_json::Value::String(path))
}

/// Fails with `E_GIT_LOCKED` when a lock file of a written file exists.
pub(in crate::refs_tx) fn check_locks(steps: &[Step]) -> Result<()> {
    for lock in steps.iter().filter_map(Step::lock) {
        match std::fs::symlink_metadata(lock.as_path()) {
            Ok(_) => return Err(locked(lock)),
            Err(error) if is_missing(&error) => {}
            Err(error) => return Err(io_error(lock.as_path(), &error)),
        }
    }
    Ok(())
}

/// Replaces a staleness error by `E_GIT_LOCKED` when a lock file of the
/// transaction is present: Git holds the lock while it changes the file.
fn lock_error(error: Error, steps: &[Step]) -> Error {
    if error.code != ErrorCode::Stale {
        return error;
    }
    steps
        .iter()
        .filter_map(Step::lock)
        .find(|lock| lock.as_path().exists())
        .map_or(error, locked)
}

impl Plan {
    pub(in crate::refs_tx) fn target_specs(&self) -> Vec<TargetSpec> {
        self.steps.iter().map(Step::spec).collect()
    }

    pub(in crate::refs_tx) fn declare(&mut self, tx: &mut dyn MutationTx) -> Result<()> {
        if self.declared.is_some() {
            return Ok(());
        }
        check_locks(&self.steps)?;
        let specs = self.target_specs();
        let preserved = if specs.is_empty() {
            Vec::new()
        } else {
            tx.preserve(&specs)
                .map_err(|error| lock_error(error, &self.steps))?
        };
        if preserved.len() != specs.len() {
            return Err(Error::new(
                ErrorCode::Internal,
                "the journal returned fewer preserved targets than declared",
            ));
        }
        self.declared = Some(preserved.into_iter().map(|item| vec![item.index]).collect());
        Ok(())
    }

    /// Applies the steps: directories, verification of the locks and of the
    /// guarded files, switches and deletions, reflogs, and removal of the
    /// directories left empty.
    pub(in crate::refs_tx) fn apply(&self, tx: &mut dyn MutationTx) -> Result<()> {
        let declared = self.declared.as_ref().ok_or_else(|| {
            Error::new(ErrorCode::Internal, "reference targets were not declared")
        })?;
        let steps: Vec<(&Step, &StepTargets)> = self.steps.iter().zip(declared).collect();
        check_locks(&self.steps)?;
        verify_guards(&self.guards)?;
        for (step, targets) in &steps {
            apply_directories(tx, step, targets)?;
        }
        for (step, targets) in &steps {
            apply_switch(tx, step, targets).map_err(|error| lock_error(error, &self.steps))?;
        }
        for (step, targets) in &steps {
            apply_logs(tx, step, targets)?;
        }
        for (step, targets) in &steps {
            if let Step::RemoveDir(_) = step {
                apply_at(tx, targets, Mutation::Remove)?;
            }
        }
        Ok(())
    }
}

fn apply_at(tx: &mut dyn MutationTx, targets: &StepTargets, mutation: Mutation) -> Result<()> {
    let index = targets.first().copied().ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            "missing target index of a reference step",
        )
    })?;
    tx.apply(index, mutation).map(drop)
}

fn apply_directories(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    match step {
        Step::CreateDir(_) => apply_at(tx, targets, Mutation::CreateDir { mode: DIR_MODE }),
        Step::ClearDir(_) => apply_at(tx, targets, Mutation::Remove),
        _ => Ok(()),
    }
}

fn apply_switch(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    match step {
        Step::Write { content, .. } => apply_at(tx, targets, Mutation::WriteBytes(content.clone())),
        Step::DeleteRef { .. } => apply_at(tx, targets, Mutation::Remove),
        _ => Ok(()),
    }
}

fn apply_logs(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    match step {
        Step::DeleteLog(_) => apply_at(tx, targets, Mutation::Remove),
        Step::AppendLog { bytes, .. } => {
            apply_at(tx, targets, Mutation::AppendBytes(bytes.clone()))
        }
        Step::CreateLog { bytes, .. } | Step::ReplaceLog { bytes, .. } => {
            apply_at(tx, targets, Mutation::WriteBytes(bytes.clone()))
        }
        _ => Ok(()),
    }
}

/// Fails with `E_STALE` when a guarded file changed since it was read.
fn verify_guards(guards: &[Guard]) -> Result<()> {
    for guard in guards {
        let current = read_loose(guard.path.as_path())?;
        if current != guard.expected {
            let path = guard.path.to_language_string();
            return Err(Error::new(
                ErrorCode::Stale,
                format!("{path} changed after it was read; run the command again"),
            )
            .with_detail("path", serde_json::Value::String(path)));
        }
    }
    Ok(())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
