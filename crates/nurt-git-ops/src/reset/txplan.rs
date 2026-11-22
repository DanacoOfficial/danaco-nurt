// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Ordered list of journal targets and their mutations.
//!
//! Every change is first collected here. Execution declares and preserves
//! all targets in one call and only then applies the mutations in order, so
//! the journal holds the previous state of every target before the first
//! change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, LinkKind, MetaChange};
use nurt_contracts::journal::{
    Applied, Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec,
};
use sha2::{Digest, Sha256};

use super::repo::error;

/// Permission bits of directories created in the working tree.
const DIR_MODE: u32 = 0o755;

/// Suffix of git lock files.
const LOCK_SUFFIX: &str = ".lock";

/// What exists at a path before a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Before {
    /// Nothing.
    Absent,
    /// A regular file with this content hash.
    File(ContentHash),
    /// An entry of another kind (directory, link) that is replaced.
    Other,
}

#[derive(Debug)]
struct Step {
    spec: TargetSpec,
    mutation: Option<Mutation>,
}

/// Planned targets in execution order.
#[derive(Debug, Default)]
pub(crate) struct TxPlan {
    steps: Vec<Step>,
    dirs: BTreeSet<PathBuf>,
}

/// SHA-256 of content, as used by [`Expect::Hash`].
pub(crate) fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

fn abs(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|cause| {
        error(
            ErrorCode::Io,
            format!("invalid target path {}: {cause}", path.display()),
        )
    })
}

impl TxPlan {
    /// An empty plan.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether the plan has no targets.
    pub(crate) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Declared targets in order.
    pub(crate) fn targets(&self) -> impl Iterator<Item = &TargetSpec> {
        self.steps.iter().map(|step| &step.spec)
    }

    fn push(
        &mut self,
        path: &Path,
        action: TargetAction,
        expect: Expect,
        mutation: Option<Mutation>,
    ) -> Result<()> {
        self.steps.push(Step {
            spec: TargetSpec {
                path: TargetPath::Fs(abs(path)?),
                action,
                expect,
                size_hint: None,
            },
            mutation,
        });
        Ok(())
    }

    /// Writes a file with `bytes`.
    pub(crate) fn write(&mut self, path: &Path, bytes: Vec<u8>, before: &Before) -> Result<()> {
        let (action, expect) = match before {
            Before::Absent => (TargetAction::Create, Expect::Absent),
            Before::File(hash) => (TargetAction::Modify, Expect::Hash(*hash)),
            Before::Other => (TargetAction::Replace, Expect::Unchecked),
        };
        self.push(path, action, expect, Some(Mutation::WriteBytes(bytes)))
    }

    /// Creates a symbolic link to `target`.
    pub(crate) fn link(&mut self, path: &Path, target: PathBuf, before: &Before) -> Result<()> {
        let (action, expect) = match before {
            Before::Absent => (TargetAction::Link, Expect::Absent),
            Before::File(hash) => (TargetAction::Replace, Expect::Hash(*hash)),
            Before::Other => (TargetAction::Replace, Expect::Unchecked),
        };
        let mutation = Mutation::CreateLink {
            target,
            kind: LinkKind::SymlinkFile,
        };
        self.push(path, action, expect, Some(mutation))
    }

    /// Sets the permission bits of a file.
    pub(crate) fn set_mode(&mut self, path: &Path, mode: u32) -> Result<()> {
        let change = MetaChange {
            mode: Some(mode),
            ..MetaChange::default()
        };
        self.push(
            path,
            TargetAction::Attributes,
            Expect::Unchecked,
            Some(Mutation::SetMeta(change)),
        )
    }

    /// Deletes a file, a link or a directory tree.
    pub(crate) fn remove(&mut self, path: &Path, before: &Before) -> Result<()> {
        let expect = match before {
            Before::File(hash) => Expect::Hash(*hash),
            Before::Absent | Before::Other => Expect::Unchecked,
        };
        self.push(path, TargetAction::Delete, expect, Some(Mutation::Remove))
    }

    /// Replaces a file or a link by a new directory.
    pub(crate) fn replace_with_dir(&mut self, path: &Path) -> Result<()> {
        self.dirs.insert(path.to_path_buf());
        self.push(
            path,
            TargetAction::Replace,
            Expect::Unchecked,
            Some(Mutation::CreateDir { mode: DIR_MODE }),
        )
    }

    /// Creates the missing directories above `path`, up to `root`; a file
    /// or a link in the way is replaced by a directory.
    pub(crate) fn ensure_parent_dirs(&mut self, path: &Path, root: &Path) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = path.parent();
        while let Some(dir) = current {
            if dir == root || !dir.starts_with(root) || self.dirs.contains(dir) {
                break;
            }
            match std::fs::symlink_metadata(dir) {
                Ok(meta) if meta.is_dir() => break,
                Ok(_) => missing.push((dir.to_path_buf(), true)),
                Err(_) => missing.push((dir.to_path_buf(), false)),
            }
            current = dir.parent();
        }
        for (dir, occupied) in missing.into_iter().rev() {
            self.dirs.insert(dir.clone());
            let (action, expect) = if occupied {
                (TargetAction::Replace, Expect::Unchecked)
            } else {
                (TargetAction::Create, Expect::Absent)
            };
            self.push(
                &dir,
                action,
                expect,
                Some(Mutation::CreateDir { mode: DIR_MODE }),
            )?;
        }
        Ok(())
    }

    /// Appends `bytes` to an existing file.
    pub(crate) fn append(&mut self, path: &Path, bytes: Vec<u8>) -> Result<()> {
        self.push(
            path,
            TargetAction::Append,
            Expect::Unchecked,
            Some(Mutation::AppendBytes(bytes)),
        )
    }

    /// Writes `path` by the git lock protocol: the lock file `<path>.lock`
    /// is created (an existing lock fails), filled and renamed over `path`.
    /// `bytes` of `None` are supplied at execution time.
    pub(crate) fn locked_write(&mut self, path: &Path, bytes: Option<Vec<u8>>) -> Result<()> {
        let mut lock = path.as_os_str().to_owned();
        lock.push(LOCK_SUFFIX);
        let lock = PathBuf::from(lock);
        self.push(
            &lock,
            TargetAction::Create,
            Expect::Absent,
            bytes.map(Mutation::WriteBytes),
        )?;
        self.push(
            &lock,
            TargetAction::Rename {
                to: abs(path)?,
                replace: true,
            },
            Expect::Unchecked,
            Some(Mutation::Rename),
        )
    }

    /// Appends the steps of `other`.
    pub(crate) fn extend(&mut self, other: Self) {
        self.dirs.extend(other.dirs);
        self.steps.extend(other.steps);
    }

    /// Declares and preserves every target, then applies the mutations in
    /// order. A step without mutation takes its bytes from `deferred`, called
    /// once when the step is reached.
    pub(crate) fn execute(
        self,
        tx: &mut dyn MutationTx,
        deferred: &mut dyn FnMut() -> Result<Vec<u8>>,
    ) -> Result<Vec<Applied>> {
        if self.steps.is_empty() {
            return Ok(Vec::new());
        }
        let specs: Vec<TargetSpec> = self.steps.iter().map(|step| step.spec.clone()).collect();
        let preserved = tx.preserve(&specs)?;
        let mut applied = Vec::with_capacity(preserved.len());
        for (step, kept) in self.steps.into_iter().zip(preserved) {
            let mutation = match step.mutation {
                Some(mutation) => mutation,
                None => Mutation::WriteBytes(deferred()?),
            };
            applied.push(tx.apply(kept.index, mutation)?);
        }
        Ok(applied)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
mod tests {
    use super::*;

    #[test]
    fn locked_write_declares_lock_creation_then_rename() {
        let mut plan = TxPlan::new();
        let target = std::env::temp_dir().join("index");
        plan.locked_write(&target, Some(b"x".to_vec())).unwrap();
        let actions: Vec<_> = plan.targets().map(|t| t.action.clone()).collect();
        assert_eq!(actions[0], TargetAction::Create);
        assert!(matches!(actions[1], TargetAction::Rename { replace: true, .. }));
    }

    #[test]
    fn write_expects_absence_or_previous_hash() {
        let mut plan = TxPlan::new();
        let dir = std::env::temp_dir();
        let hash = content_hash(b"old");
        plan.write(&dir.join("a"), b"new".to_vec(), &Before::Absent).unwrap();
        plan.write(&dir.join("b"), b"new".to_vec(), &Before::File(hash)).unwrap();
        let expects: Vec<_> = plan.targets().map(|t| t.expect.clone()).collect();
        assert_eq!(expects, vec![Expect::Absent, Expect::Hash(hash)]);
    }
}
