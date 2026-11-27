// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::apply::guard
// Responsibility:   Identity of the directories that a plan writes through.
// Dependencies:     bstr, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Identity of the directories that a plan writes through.
//!
//! The planner records every existing parent directory of a changed path.
//! The executor checks the parents of each entry against the record before
//! the entry is written, so that a directory replaced by a symbolic link (or
//! by another directory) after the plan was made fails with `E_STALE`
//! instead of redirecting the write, as `has_symlink_leading_path` does in
//! `git`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use bstr::BStr;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::layout::ancestors;
use super::worktree::{Node, Observation, native_path, observe_native};

/// Device and inode of a directory; zero on systems without them.
type DirKey = (u64, u64);

#[cfg(unix)]
fn key_of(meta: &std::fs::Metadata) -> DirKey {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn key_of(_meta: &std::fs::Metadata) -> DirKey {
    (0, 0)
}

/// Directories observed below the working tree root, by native path.
#[derive(Default)]
pub(crate) struct DirGuards {
    known: Mutex<BTreeMap<PathBuf, DirKey>>,
}

impl DirGuards {
    /// Records the directory `path` as observed when the plan was made.
    pub(crate) fn record(&self, path: PathBuf, observation: &Observation) {
        if let (Node::Dir, Some(meta)) = (observation.node, &observation.meta) {
            self.known
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(path, key_of(meta));
        }
    }

    /// Checks the existing parent directories of `rela` below `root`.
    ///
    /// A parent without a record is recorded when it is a directory; a parent
    /// that is a symbolic link, a file, or differs from its record fails.
    ///
    /// # Errors
    ///
    /// `E_STALE` when a parent changed after the plan was made or is not a
    /// directory.
    pub(crate) fn verify(&self, root: &Path, rela: &BStr) -> Result<()> {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        for dir in ancestors(rela) {
            let native = native_path(root, dir)?;
            let observation = observe_native(&native)?;
            let recorded = known.get(&native).copied();
            match (observation.node, recorded) {
                (Node::Dir, Some(key)) => {
                    if observation.meta.as_ref().map(key_of) != Some(key) {
                        return Err(stale(&native, "was replaced"));
                    }
                }
                (Node::Dir, None) => {
                    if let Some(meta) = &observation.meta {
                        known.insert(native, key_of(meta));
                    }
                }
                (Node::Missing, None) => return Ok(()),
                (Node::Missing, Some(_)) => return Err(stale(&native, "was removed")),
                _ => return Err(stale(&native, "is no longer a directory")),
            }
        }
        Ok(())
    }

    /// Drops the record of `path` and of everything below it, after the plan
    /// created, replaced or removed it.
    pub(crate) fn forget(&self, path: &Path) {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|known, _| !known.starts_with(path));
    }
}

fn stale(path: &Path, what: &str) -> Error {
    Error::new(
        ErrorCode::Stale,
        format!(
            "'{}' {what} after the plan was made; repeat the command",
            path.display()
        ),
    )
    .with_found(path.display().to_string())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
