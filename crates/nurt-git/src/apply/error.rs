// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::apply::error
// Responsibility:   Errors of planning and applying working tree changes.
// Dependencies:     bstr, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Errors of planning and applying working tree changes.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;

use bstr::{BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode};

/// Largest number of paths quoted in an error message; the details carry
/// the complete lists.
const MESSAGE_PATHS: usize = 5;

/// Error of a file system read during planning.
pub(crate) fn io_error(err: &std::io::Error, path: &Path) -> Error {
    let code = match err.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::Permission,
        _ => ErrorCode::Io,
    };
    Error::new(code, format!("cannot read '{}': {err}", path.display()))
        .with_found(path.display().to_string())
}

/// Error reported by the repository library.
pub(crate) fn repo_error(err: impl Display, what: &str) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {err}"))
}

/// Error for a missing or malformed object.
pub(crate) fn object_error(err: impl Display, id: &gix::oid) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read object {id}: {err}; run git.status to check the repository"),
    )
}

/// Error for a path of a tree that cannot be checked out safely.
pub(crate) fn unsafe_path(path: &[u8], reason: &str) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!(
            "the tree contains the path '{}' that cannot be checked out: {reason}",
            path.as_bstr()
        ),
    )
    .with_problem("unsafe_path")
    .with_found(path.as_bstr().to_string())
}

/// Error for a lock file left in the repository directory.
pub(crate) fn locked(lock: &Path) -> Error {
    Error::new(
        ErrorCode::GitLocked,
        format!(
            "the lock file '{}' exists; another git process may be running, otherwise remove the file",
            lock.display()
        ),
    )
    .with_found(lock.display().to_string())
}

/// Paths that block a working tree update, grouped as `git` groups them.
///
/// All rejections of a plan are collected before the plan fails, so the
/// error lists every blocking path at once and nothing is changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rejections {
    /// Tracked files with local changes that the update would overwrite or
    /// remove.
    pub local_changes: BTreeSet<BString>,
    /// Untracked files that the update would overwrite.
    pub untracked_overwritten: BTreeSet<BString>,
    /// Untracked files that the update would remove.
    pub untracked_removed: BTreeSet<BString>,
    /// Directories holding untracked files that the update would replace by
    /// a file.
    pub dir_loses_untracked: BTreeSet<BString>,
}

impl Rejections {
    /// Whether no path blocks the update.
    pub fn is_empty(&self) -> bool {
        self.local_changes.is_empty()
            && self.untracked_overwritten.is_empty()
            && self.untracked_removed.is_empty()
            && self.dir_loses_untracked.is_empty()
    }

    /// `E_GIT_DIRTY` for the collected paths; `operation` names the command
    /// in the message (`switch`, `restore`), `fixes` are complete Nurt calls
    /// that resolve the situation.
    pub fn into_error(self, operation: &str, fixes: &[String]) -> Error {
        let groups = [
            ("local_changes", &self.local_changes, "local changes"),
            (
                "untracked_overwritten",
                &self.untracked_overwritten,
                "untracked files that would be overwritten",
            ),
            (
                "untracked_removed",
                &self.untracked_removed,
                "untracked files that would be removed",
            ),
            (
                "dir_loses_untracked",
                &self.dir_loses_untracked,
                "directories with untracked files",
            ),
        ];
        let problem = groups
            .iter()
            .find(|(_, paths, _)| !paths.is_empty())
            .map_or("local_changes", |(name, _, _)| *name);
        let summary: Vec<String> = groups
            .iter()
            .filter(|(_, paths, _)| !paths.is_empty())
            .map(|(_, paths, label)| format!("{label}: {}", quote_paths(paths)))
            .collect();
        let mut error = Error::new(
            ErrorCode::GitDirty,
            format!(
                "{operation} would lose {}; commit, stash or remove them first",
                summary.join("; ")
            ),
        )
        .with_problem(problem);
        for (name, paths, _) in groups {
            if !paths.is_empty() {
                let list: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
                error = error.with_detail(name, serde_json::json!(list));
            }
        }
        for path in self.all_paths() {
            error = error.with_sample(path.to_string());
        }
        for fix in fixes {
            error = error.with_fix(fix.clone());
        }
        error
    }

    fn all_paths(&self) -> impl Iterator<Item = &BString> {
        self.local_changes
            .iter()
            .chain(&self.untracked_overwritten)
            .chain(&self.untracked_removed)
            .chain(&self.dir_loses_untracked)
    }
}

fn quote_paths(paths: &BTreeSet<BString>) -> String {
    let mut quoted: Vec<String> = paths
        .iter()
        .take(MESSAGE_PATHS)
        .map(|path| format!("'{path}'"))
        .collect();
    if paths.len() > MESSAGE_PATHS {
        quoted.push(format!("and {} more", paths.len() - MESSAGE_PATHS));
    }
    quoted.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_maps_kinds_to_codes() {
        let path = Path::new("/r/f");
        let cases = [
            (std::io::ErrorKind::NotFound, ErrorCode::NotFound),
            (std::io::ErrorKind::PermissionDenied, ErrorCode::Permission),
            (std::io::ErrorKind::InvalidData, ErrorCode::Io),
        ];
        for (kind, code) in cases {
            assert_eq!(io_error(&std::io::Error::from(kind), path).code(), code);
        }
    }

    #[test]
    fn repository_and_object_errors_carry_operation_and_id() {
        let repo = repo_error("broken", "read the index");
        assert_eq!(repo.code(), ErrorCode::Io);
        assert!(repo.to_string().contains("cannot read the index"));
        let id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        assert!(
            object_error("missing", &id)
                .to_string()
                .contains("git.status")
        );
    }

    #[test]
    fn unsafe_path_and_lock_errors_use_contract_codes() {
        let unsafe_error = unsafe_path(b".git/config", "reserved name");
        assert_eq!(unsafe_error.code(), ErrorCode::GitUnsupported);
        assert!(unsafe_error.to_string().contains(".git/config"));
        let lock = locked(Path::new("/r/.git/index.lock"));
        assert_eq!(lock.code(), ErrorCode::GitLocked);
        assert!(lock.to_string().contains("index.lock"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
