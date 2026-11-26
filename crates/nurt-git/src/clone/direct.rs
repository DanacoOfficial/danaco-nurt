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
use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FsError, FsErrorKind, FsResult, MetaTransferReport,
    WriteHandle,
};
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

    /// Refuses a write through a link, whether `path` or a directory between
    /// the new directory and `path` is one: the write would land outside the
    /// new directory.
    fn ensure_no_link(&self, path: &Path) -> Result<()> {
        let mut current = Some(path);
        while let Some(candidate) = current.filter(|candidate| *candidate != self.root) {
            if std::fs::symlink_metadata(candidate).is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Err(Error::new(
                    ErrorCode::StageFailed,
                    format!(
                        "{} is a link; a write through it would leave the new directory",
                        candidate.display()
                    ),
                ));
            }
            current = candidate.parent();
        }
        Ok(())
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

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        let path = self.target(target)?.path.clone();
        self.ensure_no_link(&path)?;
        let file = std::fs::File::create(&path).map_err(|failure| io(&path, &failure))?;
        let handle = DirectWriter::new(path, file)?;
        Ok(Box::new(handle))
    }

    fn apply(&mut self, index: TargetIndex, mutation: Mutation) -> Result<Applied> {
        let target = self.target(index)?;
        let path = target.path.as_path();
        if matches!(mutation, Mutation::WriteBytes(_) | Mutation::AppendBytes(_)) {
            self.ensure_no_link(path)?;
        }
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
            Mutation::CommitStaged => {}
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

/// Open file below the new directory, written in place.
///
/// The file is not journaled, so a staged write needs no switch-in step and
/// [`Mutation::CommitStaged`] leaves it as it is.
struct DirectWriter {
    path: AbsPath,
    file: std::fs::File,
}

impl DirectWriter {
    fn new(path: PathBuf, file: std::fs::File) -> Result<Self> {
        let path = AbsPath::new(path).map_err(|failure| {
            Error::new(ErrorCode::StageFailed, format!("invalid path: {failure}"))
        })?;
        Ok(Self { path, file })
    }

    fn failure(&self, operation: &str, error: &std::io::Error) -> FsError {
        let kind = match error.kind() {
            std::io::ErrorKind::NotFound => FsErrorKind::NotFound,
            std::io::ErrorKind::StorageFull => FsErrorKind::NoSpace,
            _ => FsErrorKind::Io,
        };
        FsError::new(kind, operation, Some(self.path.as_path()))
    }
}

impl std::io::Write for DirectWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.file.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl std::io::Seek for DirectWriter {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

impl WriteHandle for DirectWriter {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        let synced = match durability {
            Durability::Data => self.file.sync_data(),
            Durability::Full => self.file.sync_all(),
        };
        synced.map_err(|error| self.failure("sync", &error))
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        self.file
            .set_len(length)
            .map_err(|error| self.failure("set_length", &error))
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        set_mode(self.path.as_path(), meta.basic.mode)
            .map_err(|error| self.failure("apply_meta", &error))?;
        Ok(MetaTransferReport::default())
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::testutil::target;

    #[test]
    fn writes_through_a_link_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("new");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("victim"), "keep").unwrap();
        std::os::unix::fs::symlink(outside.join("victim"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dir")).unwrap();
        let mut tx = DirectTx::new(&root);
        let specs = [
            target(&root.join("link"), TargetAction::Modify),
            target(&root.join("dir/victim"), TargetAction::Modify),
        ];
        let preserved = tx.preserve(&specs).unwrap();
        assert!(tx.stage(preserved[0].index).is_err());
        assert!(
            tx.apply(preserved[0].index, Mutation::WriteBytes(b"x".to_vec()))
                .is_err()
        );
        assert!(
            tx.apply(preserved[1].index, Mutation::AppendBytes(b"x".to_vec()))
                .is_err()
        );
        assert_eq!(std::fs::read(outside.join("victim")).unwrap(), b"keep");
    }
}
