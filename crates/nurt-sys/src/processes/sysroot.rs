// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Root directory of the system files read by the Linux readers.
//!
//! The readers of `/proc`, `/sys`, `/dev` and `/etc` resolve every system path
//! against a [`SysRoot`]. In production the root is `/`; tests point it at a
//! temporary directory that holds a synthetic tree.

use std::path::{Path, PathBuf};

/// Directory against which absolute system paths are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SysRoot {
    root: PathBuf,
}

impl SysRoot {
    /// The root of the running system.
    pub(crate) fn live() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    /// A root at `root`, holding a synthetic system tree.
    #[cfg(test)]
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Location of the absolute system path `path` under this root.
    pub(crate) fn path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        self.root
            .join(path.strip_prefix("/").unwrap_or(path))
    }

    /// Returns `true` for the root of the running system, where lookups that
    /// do not go through files (account databases of the system library) are
    /// allowed.
    pub(crate) fn is_live(&self) -> bool {
        self.root == Path::new("/")
    }
}
