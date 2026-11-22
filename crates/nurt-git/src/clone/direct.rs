// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Transaction adapter that writes directly below a directory created by
//! the same operation.
//!
//! The directory of a clone is declared to the journal as one `CreateTree`
//! target, so the files below it are not journaled one by one. The checkout
//! planner declares and changes every file through the transaction
//! interface; this adapter accepts those calls and performs them on the file
//! system, after checking that the path lies below the new directory.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle,
    OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

use crate::repo::error::io;

/// A declared target.
struct Target {
    path: PathBuf,
    action: TargetAction,
}

/// Writes the declared targets straight into the tree below `root`.
pub(super) struct DirectTx {
    root: PathBuf,
    targets: Vec<Target>,
}

impl DirectTx {
    /// Adapter for the tree at `root`.
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            targets: Vec::new(),
        }
    }

    fn inside(&self, path: &Path) -> Result<()> {
        if path.starts_with(&self.root) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::StageFailed,
                format!("{} lies outside the new directory", path.display()),
            ))
        }
    }

    fn target(&self, index: TargetIndex) -> Result<&Target> {
        usize::try_from(index.0)
            .ok()
            .and_then(|position| self.targets.get(position))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::StageFailed,
                    "a change was applied to a target that was not declared",
                )
            })
    }
}

impl MutationTx for DirectTx {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let mut preserved = Vec::with_capacity(targets.len());
        for spec in targets {
            let TargetPath::Fs(path) = &spec.path else {
                return Err(Error::new(
                    ErrorCode::StageFailed,
                    "a provider target cannot be declared inside a new directory",
                ));
            };
            let path = path.as_path().to_path_buf();
            self.inside(&path)?;
            let exists = std::fs::symlink_metadata(&path).is_ok();
            if matches!(spec.action, TargetAction::Create | TargetAction::Link) && exists {
                return Err(Error::new(
                    ErrorCode::Exists,
                    format!("{} exists", path.display()),
                ));
            }
            let index = TargetIndex(u32::try_from(self.targets.len()).map_err(|_| {
                Error::new(ErrorCode::StageFailed, "too many targets in one operation")
            })?);
            preserved.push(Preserved {
                index,
                volume: None,
                mode: PreserveMode::Created,
                bytes_held: 0,
                before: BeforeState {
                    exists,
                    ..BeforeState::default()
                },
                irreversible: None,
                notable: Vec::new(),
            });
            self.targets.push(Target {
                path,
                action: spec.action.clone(),
            });
        }
        Ok(preserved)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(Error::new(
            ErrorCode::StageFailed,
            "staged writes are not available inside a new directory",
        ))
    }

    fn apply(&mut self, index: TargetIndex, mutation: Mutation) -> Result<Applied> {
        let target = self.target(index)?;
        let path = target.path.as_path();
        let failed = |error: std::io::Error| io(path, &error);
        match mutation {
            Mutation::WriteBytes(bytes) => std::fs::write(path, bytes).map_err(failed)?,
            Mutation::AppendBytes(bytes) => {
                use std::io::Write as _;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .and_then(|mut file| file.write_all(&bytes))
                    .map_err(failed)?;
            }
            Mutation::Remove => remove(path).map_err(failed)?,
            Mutation::Rename => match &target.action {
                TargetAction::Rename { to, .. } => {
                    std::fs::rename(path, to.as_path()).map_err(failed)?;
                }
                _ => {
                    return Err(Error::new(
                        ErrorCode::StageFailed,
                        "a rename was applied to a target declared without a destination",
                    ));
                }
            },
            Mutation::CreateDir { mode } => create_dir(path, mode).map_err(failed)?,
            Mutation::CreateLink { target: link, .. } => {
                symlink(&link, path).map_err(failed)?;
            }
            Mutation::SetMeta(change) => {
                if let Some(mode) = change.mode {
                    set_mode(path, mode).map_err(failed)?;
                }
            }
            _ => {
                return Err(Error::new(
                    ErrorCode::StageFailed,
                    "the change is not available inside a new directory",
                ));
            }
        }
        Ok(Applied {
            index,
            after: AfterState::default(),
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Ok(Vec::new())
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Ok(OpHandle {
            op: None,
            kind: OpKind::command(super::COMMAND),
            bulk: true,
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Ok(RollbackReport {
            op: None,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(failure) => Err(failure),
    }
}

#[cfg(unix)]
fn create_dir(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(mode).create(path)
}

#[cfg(not(unix))]
fn create_dir(path: &Path, _mode: u32) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(unix)]
fn symlink(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(windows)]
fn symlink(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, path)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}
