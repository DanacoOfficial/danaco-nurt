// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Planned file changes of one rebase call and their execution through the
//! journal transaction.
//!
//! Every target is declared and preserved before the first change. Files
//! that Git protects with a lock file (`index`, references) are written by
//! creating `<file>.lock` and renaming it over the file, so a concurrent Git
//! process sees the same protocol; an existing lock file fails the
//! transaction with `E_GIT_LOCKED`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, LinkKind, MetaChange};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec};
use sha2::{Digest, Sha256};

use super::error;

/// Permission bits of directories created in the work tree and the Git
/// directory.
const DIR_MODE: u32 = 0o755;
/// Suffix of Git lock files.
const LOCK_SUFFIX: &str = ".lock";

/// Change applied to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Write {
    /// Write the complete content.
    Bytes(Vec<u8>),
    /// Write the complete content, then set the permission bits.
    BytesWithMode(Vec<u8>, u32),
    /// Append to the end of the file.
    Append(Vec<u8>),
    /// Set the permission bits only.
    Mode(u32),
    /// Delete the entry.
    Remove,
    /// Create a directory.
    Dir,
    /// Create a symbolic link with the given target.
    Link(PathBuf),
    /// Rename the declared lock file over its destination.
    Rename,
    /// Write the index; the content is computed after the work tree changes.
    Index,
}

/// One declared target and its change.
#[derive(Debug, Clone)]
pub(super) struct Step {
    pub(super) spec: TargetSpec,
    pub(super) write: Write,
}

/// Ordered changes of one call.
#[derive(Debug, Default)]
pub(super) struct TxPlan {
    steps: Vec<Step>,
    created_dirs: BTreeSet<PathBuf>,
}

/// SHA-256 of content, as expected by the journal.
pub(super) fn content_hash(data: &[u8]) -> ContentHash {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(data));
    ContentHash(out)
}

/// Absolute path for the journal.
pub(super) fn abs(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(Error::from)
}

/// Path of the lock file of `path`.
pub(super) fn lock_path(path: &Path) -> PathBuf {
    let mut text = path.as_os_str().to_owned();
    text.push(LOCK_SUFFIX);
    PathBuf::from(text)
}

impl TxPlan {
    /// Whether nothing is planned.
    pub(super) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Planned steps.
    pub(super) fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Adds a step.
    pub(super) fn push(&mut self, path: &Path, action: TargetAction, expect: Expect, write: Write) -> Result<()> {
        self.steps.push(Step {
            spec: TargetSpec {
                path: TargetPath::Fs(abs(path)?),
                action,
                expect,
                size_hint: None,
            },
            write,
        });
        Ok(())
    }

    /// Creates the missing directories between `root` and the parent of
    /// `path`, outermost first.
    pub(super) fn ensure_parents(&mut self, root: &Path, path: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = path.parent();
        while let Some(dir) = current {
            if dir == root || !dir.starts_with(root) || dir.is_dir() || self.created_dirs.contains(dir) {
                break;
            }
            missing.push(dir.to_path_buf());
            current = dir.parent();
        }
        for dir in missing.into_iter().rev() {
            self.created_dirs.insert(dir.clone());
            self.push(&dir, TargetAction::Create, Expect::Absent, Write::Dir)?;
        }
        Ok(())
    }

    /// Marks a directory as created by an earlier step.
    pub(super) fn note_created_dir(&mut self, dir: &Path) {
        self.created_dirs.insert(dir.to_path_buf());
    }

    /// Writes a file of the Git directory with plain replacement: creates it
    /// when absent, otherwise replaces content equal to `current`.
    pub(super) fn write_file(&mut self, root: &Path, path: &Path, bytes: Vec<u8>, current: Option<&[u8]>) -> Result<()> {
        match current {
            Some(old) if old == bytes.as_slice() => Ok(()),
            Some(old) => self.push(
                path,
                TargetAction::Modify,
                Expect::Hash(content_hash(old)),
                Write::Bytes(bytes),
            ),
            None => {
                self.ensure_parents(root, path)?;
                self.push(path, TargetAction::Create, Expect::Absent, Write::Bytes(bytes))
            }
        }
    }

    /// Writes a file protected by a Git lock file: `<path>.lock` is created
    /// with the content and renamed over `path`.
    pub(super) fn write_locked(&mut self, root: &Path, path: &Path, write: Write) -> Result<()> {
        self.ensure_parents(root, path)?;
        let lock = lock_path(path);
        self.push(&lock, TargetAction::Create, Expect::Absent, write)?;
        let to = abs(path)?;
        self.push(&lock, TargetAction::Rename { to, replace: true }, Expect::Unchecked, Write::Rename)
    }

    /// Appends lines to a log file, creating it and its directories when
    /// absent.
    pub(super) fn append(&mut self, root: &Path, path: &Path, bytes: Vec<u8>) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if path.is_file() {
            return self.push(path, TargetAction::Append, Expect::Unchecked, Write::Append(bytes));
        }
        self.ensure_parents(root, path)?;
        self.push(path, TargetAction::Create, Expect::Absent, Write::Bytes(bytes))
    }

    /// Deletes a file or a directory tree when it exists.
    pub(super) fn delete(&mut self, path: &Path) -> Result<()> {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return Ok(());
        };
        let expect = if meta.is_file() {
            let data = std::fs::read(path).map_err(|cause| error::io(&format!("cannot read {}", path.display()), cause))?;
            Expect::Hash(content_hash(&data))
        } else {
            Expect::Unchecked
        };
        self.push(path, TargetAction::Delete, expect, Write::Remove)
    }

    /// Declares and preserves every target, then applies the changes in
    /// order; `index` produces the index content when its step is reached.
    pub(super) fn execute(
        self,
        tx: &mut dyn MutationTx,
        index: &mut dyn FnMut() -> Result<Vec<u8>>,
    ) -> Result<()> {
        if self.steps.is_empty() {
            return Ok(());
        }
        let specs: Vec<TargetSpec> = self.steps.iter().map(|step| step.spec.clone()).collect();
        let preserved = tx.preserve(&specs).map_err(|cause| map_lock_error(cause, &specs))?;
        for (step, preserved) in self.steps.into_iter().zip(preserved) {
            apply(tx, preserved.index, step.write, index)?;
        }
        Ok(())
    }
}

fn apply(
    tx: &mut dyn MutationTx,
    target: TargetIndex,
    write: Write,
    index: &mut dyn FnMut() -> Result<Vec<u8>>,
) -> Result<()> {
    let mutation = match write {
        Write::Bytes(bytes) => Mutation::WriteBytes(bytes),
        Write::BytesWithMode(bytes, mode) => {
            tx.apply(target, Mutation::WriteBytes(bytes))?;
            Mutation::SetMeta(MetaChange {
                mode: Some(mode),
                ..MetaChange::default()
            })
        }
        Write::Append(bytes) => Mutation::AppendBytes(bytes),
        Write::Mode(mode) => Mutation::SetMeta(MetaChange {
            mode: Some(mode),
            ..MetaChange::default()
        }),
        Write::Remove => Mutation::Remove,
        Write::Dir => Mutation::CreateDir { mode: DIR_MODE },
        Write::Link(link) => Mutation::CreateLink {
            target: link,
            kind: LinkKind::SymlinkFile,
        },
        Write::Rename => Mutation::Rename,
        Write::Index => Mutation::WriteBytes(index()?),
    };
    tx.apply(target, mutation)?;
    Ok(())
}

/// An existing lock file reported as `E_EXISTS` becomes `E_GIT_LOCKED`.
fn map_lock_error(cause: Error, specs: &[TargetSpec]) -> Error {
    if cause.code != ErrorCode::Exists {
        return cause;
    }
    let lock = specs.iter().find_map(|spec| match (&spec.path, &spec.action) {
        (TargetPath::Fs(path), TargetAction::Create)
            if path.as_path().to_string_lossy().ends_with(LOCK_SUFFIX) && path.as_path().exists() =>
        {
            Some(path.as_path().to_path_buf())
        }
        _ => None,
    });
    match lock {
        Some(lock) => {
            let repo = lock.parent().map(Path::to_path_buf).unwrap_or_default();
            error::locked(&repo, &lock).with_cause(cause)
        }
        None => cause,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_appends_the_suffix() {
        assert_eq!(lock_path(Path::new("/r/.git/index")), PathBuf::from("/r/.git/index.lock"));
    }

    #[test]
    fn unchanged_content_plans_nothing() {
        let mut plan = TxPlan::default();
        assert!(plan.write_file(Path::new("/r"), Path::new("/r/.git/x"), b"a".to_vec(), Some(b"a")).is_ok());
        assert!(plan.is_empty());
    }

    #[test]
    fn locked_write_declares_lock_and_rename() {
        let mut plan = TxPlan::default();
        let root = std::env::temp_dir();
        let path = root.join("HEAD");
        assert!(plan.write_locked(&root, &path, Write::Bytes(b"x".to_vec())).is_ok());
        let actions: Vec<_> = plan.steps().iter().map(|step| step.spec.action.clone()).collect();
        assert!(matches!(actions.as_slice(), [TargetAction::Create, TargetAction::Rename { replace: true, .. }]));
    }

    #[test]
    fn content_hash_is_sha256() {
        let hash = content_hash(b"");
        assert_eq!(hash.0[0], 0xe3);
    }
}
