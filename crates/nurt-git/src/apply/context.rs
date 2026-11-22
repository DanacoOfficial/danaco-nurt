// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository state read before planning.

use std::path::{Path, PathBuf};

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::Expect;

use super::error::{locked, repo_error};
use super::sparse::Sparse;
use super::worktree::{Node, expect_of, observe_native};

/// The repository, its working tree and its current index, read once before
/// a plan is made.
///
/// Every plan function compares the trees it is given with this snapshot;
/// the executor checks that the index file is unchanged before it writes.
pub struct ApplyContext<'r> {
    pub(crate) repo: &'r gix::Repository,
    pub(crate) root: PathBuf,
    pub(crate) index: gix_index::State,
    pub(crate) index_path: PathBuf,
    pub(crate) index_expect: Expect,
    pub(crate) index_exists: bool,
    pub(crate) sparse: Sparse,
}

impl<'r> ApplyContext<'r> {
    /// Reads the index and the settings of the working tree of `repo`.
    ///
    /// # Errors
    ///
    /// `E_GIT_UNSUPPORTED` for a bare repository or a sparse index,
    /// `E_GIT_LOCKED` when `index.lock` exists, and `E_IO` when the index
    /// or the sparse checkout patterns cannot be read.
    pub fn open(repo: &'r gix::Repository) -> Result<Self> {
        let root = repo
            .workdir()
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::GitUnsupported,
                    "the repository is bare and has no working tree; use a repository with a working tree",
                )
                .with_problem("bare_repository")
            })?
            .to_path_buf();
        let index_path = repo.index_path();
        let lock = lock_path(&index_path);
        if observe_native(&lock)?.node.exists() {
            return Err(locked(&lock));
        }
        let observation = observe_native(&index_path)?;
        let index_expect = expect_of(&index_path, &observation)?;
        let index_exists = observation.node != Node::Missing;
        let index = if index_exists {
            repo.open_index()
                .map_err(|err| repo_error(err, "read the index"))?
                .into()
        } else {
            gix_index::State::new(repo.object_hash())
        };
        if index.is_sparse() {
            return Err(Error::new(
                ErrorCode::GitUnsupported,
                "the index is a sparse index; run `git sparse-checkout disable` or set index.sparse to false",
            )
            .with_problem("sparse_index"));
        }
        let caps = repo
            .filesystem_options()
            .map_err(|err| repo_error(err, "read the core settings"))?;
        let sparse = Sparse::load(repo, caps.ignore_case)?;
        Ok(Self {
            repo,
            root,
            index,
            index_path,
            index_expect,
            index_exists,
            sparse,
        })
    }

    /// The repository.
    pub fn repo(&self) -> &'r gix::Repository {
        self.repo
    }

    /// Root directory of the working tree.
    pub fn workdir(&self) -> &Path {
        &self.root
    }

    /// The index as read.
    pub fn index(&self) -> &gix_index::State {
        &self.index
    }

    /// Expectation of the index file as read, for an index written by a
    /// caller in the same transaction.
    pub fn index_expect(&self) -> &Expect {
        &self.index_expect
    }

    /// Paths of index entries in a conflict stage, sorted and without
    /// repetition.
    pub fn unmerged_paths(&self) -> Vec<BString> {
        let mut paths: Vec<BString> = self
            .index
            .entries()
            .iter()
            .filter(|entry| entry.stage_raw() != 0)
            .map(|entry| entry.path(&self.index).to_owned())
            .collect();
        paths.dedup();
        paths
    }
}

/// Path of the lock file that guards `path`.
pub(crate) fn lock_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".lock");
    path.with_file_name(name)
}
