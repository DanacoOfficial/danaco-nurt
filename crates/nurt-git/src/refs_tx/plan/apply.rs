// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Declaration and execution of the planned file changes.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};

use super::super::read::read_loose;
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
    /// Content written to `lock` and renamed over `path`.
    Write {
        lock: AbsPath,
        path: AbsPath,
        content: Vec<u8>,
    },
    /// Loose reference deleted while `lock` is held.
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
    /// Directory left empty by the transaction.
    RemoveDir(AbsPath),
}

impl Step {
    /// Lock file taken by the step.
    pub(in crate::refs_tx) fn lock(&self) -> Option<&AbsPath> {
        match self {
            Self::Write { lock, .. } | Self::DeleteRef { lock, .. } => Some(lock),
            _ => None,
        }
    }

    /// Declared targets of the step.
    fn specs(&self) -> Vec<TargetSpec> {
        match self {
            Self::CreateDir(path) => vec![spec(path, TargetAction::Create, Expect::Absent)],
            Self::ClearDir(path) | Self::DeleteLog(path) | Self::RemoveDir(path) => {
                vec![spec(path, TargetAction::Delete, Expect::Unchecked)]
            }
            Self::Write { lock, path, .. } => lock_specs(lock, path),
            Self::DeleteRef { lock, path, old } => {
                let mut specs = lock_specs(lock, path);
                specs.push(spec(
                    path,
                    TargetAction::Delete,
                    Expect::Hash(content_hash(old)),
                ));
                specs
            }
            Self::AppendLog { path, .. } => {
                vec![spec(path, TargetAction::Append, Expect::Unchecked)]
            }
            Self::CreateLog { path, .. } => vec![spec(path, TargetAction::Create, Expect::Absent)],
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

/// Exclusive creation of `lock` and its rename over `path`.
fn lock_specs(lock: &AbsPath, path: &AbsPath) -> Vec<TargetSpec> {
    vec![
        spec(lock, TargetAction::Create, Expect::Absent),
        spec(
            lock,
            TargetAction::Rename {
                to: path.clone(),
                replace: true,
            },
            Expect::Unchecked,
        ),
    ]
}

fn content_hash(bytes: &[u8]) -> nurt_contracts::fs::ContentHash {
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

/// Replaces an existence or staleness error of a lock target by
/// `E_GIT_LOCKED` when the lock file is present.
fn lock_error(error: Error, locks: &[&AbsPath]) -> Error {
    if !matches!(error.code, ErrorCode::Exists | ErrorCode::Stale) {
        return error;
    }
    locks
        .iter()
        .find(|lock| lock.as_path().exists())
        .map_or(error, |lock| locked(lock))
}

impl Plan {
    pub(in crate::refs_tx) fn target_specs(&self) -> Vec<TargetSpec> {
        self.steps.iter().flat_map(Step::specs).collect()
    }

    pub(in crate::refs_tx) fn declare(&mut self, tx: &mut dyn MutationTx) -> Result<()> {
        if self.declared.is_some() {
            return Ok(());
        }
        let specs = self.target_specs();
        let preserved = if specs.is_empty() {
            Vec::new()
        } else {
            let locks: Vec<&AbsPath> = self.steps.iter().filter_map(Step::lock).collect();
            tx.preserve(&specs)
                .map_err(|error| lock_error(error, &locks))?
        };
        let mut indexes = preserved.into_iter().map(|item| item.index);
        let mut declared = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            let count = step.specs().len();
            let step_indexes: StepTargets = indexes.by_ref().take(count).collect();
            if step_indexes.len() != count {
                return Err(Error::new(
                    ErrorCode::Internal,
                    "the journal returned fewer preserved targets than declared",
                ));
            }
            declared.push(step_indexes);
        }
        self.declared = Some(declared);
        Ok(())
    }

    /// Applies the steps: directories, locks, verification under the locks,
    /// renames and deletions, reflogs, and removal of empty directories.
    pub(in crate::refs_tx) fn apply(&self, tx: &mut dyn MutationTx) -> Result<()> {
        let declared = self.declared.as_ref().ok_or_else(|| {
            Error::new(ErrorCode::Internal, "reference targets were not declared")
        })?;
        let steps: Vec<(&Step, &StepTargets)> = self.steps.iter().zip(declared).collect();
        for (step, targets) in &steps {
            apply_directories(tx, step, targets)?;
        }
        for (step, targets) in &steps {
            apply_lock(tx, step, targets)?;
        }
        verify_guards(&self.guards)?;
        for (step, targets) in &steps {
            apply_switch(tx, step, targets)?;
        }
        for (step, targets) in &steps {
            apply_logs(tx, step, targets)?;
        }
        for (step, targets) in &steps {
            if let Step::RemoveDir(_) = step {
                apply_at(tx, targets, 0, Mutation::Remove)?;
            }
        }
        Ok(())
    }
}

fn apply_at(
    tx: &mut dyn MutationTx,
    targets: &StepTargets,
    position: usize,
    mutation: Mutation,
) -> Result<()> {
    let index = targets.get(position).copied().ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            "missing target index of a reference step",
        )
    })?;
    tx.apply(index, mutation).map(drop)
}

fn apply_directories(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    match step {
        Step::CreateDir(_) => apply_at(tx, targets, 0, Mutation::CreateDir { mode: DIR_MODE }),
        Step::ClearDir(_) => apply_at(tx, targets, 0, Mutation::Remove),
        _ => Ok(()),
    }
}

fn apply_lock(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    let (lock, bytes) = match step {
        Step::Write { lock, content, .. } => (lock, content),
        Step::DeleteRef { lock, old, .. } => (lock, old),
        _ => return Ok(()),
    };
    apply_at(tx, targets, 0, Mutation::WriteBytes(bytes.clone()))
        .map_err(|error| lock_error(error, &[lock]))
}

fn apply_switch(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    match step {
        Step::Write { .. } => apply_at(tx, targets, 1, Mutation::Rename),
        Step::DeleteRef { .. } => {
            apply_at(tx, targets, 1, Mutation::Rename)?;
            apply_at(tx, targets, 2, Mutation::Remove)
        }
        _ => Ok(()),
    }
}

fn apply_logs(tx: &mut dyn MutationTx, step: &Step, targets: &StepTargets) -> Result<()> {
    match step {
        Step::DeleteLog(_) => apply_at(tx, targets, 0, Mutation::Remove),
        Step::AppendLog { bytes, .. } => {
            apply_at(tx, targets, 0, Mutation::AppendBytes(bytes.clone()))
        }
        Step::CreateLog { bytes, .. } => {
            apply_at(tx, targets, 0, Mutation::WriteBytes(bytes.clone()))
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
