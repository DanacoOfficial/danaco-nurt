// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::reset::repo
// Responsibility:   Reading `HEAD`, commits and working tree paths of a [`GitRepo`].
// Dependencies:     gix, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reading `HEAD`, commits and working tree paths of a [`GitRepo`].

use std::path::{Path, PathBuf};

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::refs::FullName;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::ExecCtx;
use nurt_git::repo::GitRepo;
use nurt_git::repo::error::quote_path;

/// State of `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeadState {
    /// Branch `HEAD` points to; `None` when detached.
    pub(crate) branch: Option<FullName>,
    /// Commit of `HEAD`; `None` on an unborn branch.
    pub(crate) id: Option<ObjectId>,
}

impl HeadState {
    /// Short name of the branch, `None` when `HEAD` is detached.
    pub(crate) fn short_branch(&self) -> Option<String> {
        self.branch.as_ref().map(|name| name.shorten().to_string())
    }

    /// Short branch name as used in stash messages, `(no branch)` when
    /// detached.
    pub(crate) fn branch_label(&self) -> String {
        self.short_branch()
            .unwrap_or_else(|| "(no branch)".to_owned())
    }
}

/// Error with a code and the stage it belongs to.
pub(crate) fn error(code: ErrorCode, message: impl Into<String>) -> Error {
    let stage = match code {
        ErrorCode::GitNotRepository | ErrorCode::GitRevision => Stage::Read,
        ErrorCode::GitConflict | ErrorCode::Io | ErrorCode::Stale => Stage::Apply,
        _ => Stage::Plan,
    };
    Error::new(code, message).with_stage(stage)
}

/// Input or output error of the repository.
pub(crate) fn io_error(context: &str, cause: impl std::fmt::Display) -> Error {
    error(ErrorCode::Io, format!("{context}: {cause}"))
}

/// Reference name checked for validity.
pub(crate) fn full_name(name: &str) -> Result<FullName> {
    FullName::try_from(name).map_err(|cause| {
        error(
            ErrorCode::GitRef,
            format!("`{name}` is not a valid reference name: {cause}"),
        )
    })
}

/// Root of the working tree as a text literal for ready calls.
pub(crate) fn root_literal(repo: &GitRepo) -> String {
    quote_path(repo.root())
}

/// Opens the repository containing `path`; it must have a working tree.
///
/// The repository is opened by [`GitRepo::discover`], so no program is
/// started and the configuration of the machine is taken from `ctx`.
pub(crate) fn open(path: &Path, ctx: &ExecCtx) -> Result<GitRepo> {
    let repo = GitRepo::discover(path, ctx)?;
    if repo.is_bare() {
        return Err(error(
            ErrorCode::GitUnsupported,
            format!(
                "the repository at {} has no working tree; use a repository with a working tree",
                path.display()
            ),
        )
        .with_problem("bare_repository"));
    }
    Ok(repo)
}

/// Reads `HEAD`.
pub(crate) fn head(repo: &GitRepo) -> Result<HeadState> {
    Ok(HeadState {
        branch: repo.head_ref()?,
        id: repo.head_id()?,
    })
}

/// Tree of a commit.
pub(crate) fn commit_tree(repo: &GitRepo, commit: ObjectId) -> Result<ObjectId> {
    repo.gix()
        .find_commit(commit)
        .and_then(|commit| commit.tree_id().map(gix::Id::detach))
        .map_err(|cause| io_error("cannot read commit", cause))
}

/// Parents of a commit.
pub(crate) fn commit_parents(repo: &GitRepo, commit: ObjectId) -> Result<Vec<ObjectId>> {
    let commit = repo
        .gix()
        .find_commit(commit)
        .map_err(|cause| io_error("cannot read commit", cause))?;
    Ok(commit.parent_ids().map(gix::Id::detach).collect())
}

/// Subject of a commit as `git log --format=%s` prints it.
pub(crate) fn commit_subject(repo: &GitRepo, commit: ObjectId) -> Result<String> {
    let commit = repo
        .gix()
        .find_commit(commit)
        .map_err(|cause| io_error("cannot read commit", cause))?;
    let message = commit
        .message_raw()
        .map_err(|cause| io_error("cannot read commit message", cause))?;
    Ok(subject_of(message))
}

/// Whether `ancestor` is reachable from `descendant`.
pub(crate) fn is_ancestor(repo: &GitRepo, ancestor: ObjectId, descendant: ObjectId) -> bool {
    ancestor == descendant
        || repo
            .gix()
            .merge_base(ancestor, descendant)
            .is_ok_and(|base| base.is_some_and(|base| base.detach() == ancestor))
}

/// Absolute path of a path relative to the working tree root.
pub(crate) fn worktree_path(repo: &GitRepo, relative: &BStr) -> PathBuf {
    join_slash(repo.root(), relative)
}

/// Path relative to the working tree root, with `/` separators, for a path
/// given absolute or relative to the root; `None` outside the root.
pub(crate) fn relative_path(repo: &GitRepo, path: &Path) -> Option<BString> {
    let relative = if path.is_absolute() {
        path.strip_prefix(repo.root()).ok()?
    } else {
        path
    };
    let mut out = BString::default();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => {
                if !out.is_empty() {
                    out.push(b'/');
                }
                out.extend_from_slice(gix::path::os_str_into_bstr(part).ok()?);
            }
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Joins a `/`-separated relative name to a directory; the bytes of every
/// component are kept, so names that are not UTF-8 reach the file they name.
pub(crate) fn join_slash(base: &Path, name: impl AsRef<[u8]>) -> PathBuf {
    name.as_ref()
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .fold(base.to_path_buf(), |path, part| path.join(os_name(part)))
}

#[cfg(unix)]
fn os_name(part: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(part).to_owned()
}

#[cfg(not(unix))]
fn os_name(part: &[u8]) -> std::ffi::OsString {
    String::from_utf8_lossy(part).into_owned().into()
}

/// First paragraph of a commit message with its lines joined by spaces.
pub(crate) fn subject_of(message: &BStr) -> String {
    let text = message.to_str_lossy();
    let mut parts = Vec::new();
    for line in text.lines().skip_while(|line| line.trim().is_empty()) {
        if line.trim().is_empty() {
            break;
        }
        parts.push(line.trim());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_joins_the_lines_of_the_first_paragraph() {
        assert_eq!(
            subject_of(b"first\nsecond\n\nbody\n".as_bstr()),
            "first second"
        );
        assert_eq!(subject_of(b"\n\nonly\n".as_bstr()), "only");
        assert_eq!(subject_of(b"".as_bstr()), "");
    }

    #[test]
    fn join_slash_ignores_empty_components() {
        let joined = join_slash(Path::new("/r"), "refs//heads/main");
        assert_eq!(
            joined,
            Path::new("/r").join("refs").join("heads").join("main")
        );
    }

    #[cfg(unix)]
    #[test]
    fn join_slash_keeps_the_bytes_of_names_that_are_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let joined = join_slash(Path::new("/r"), b"d//\xff.txt".as_bstr());
        assert_eq!(joined.parent(), Some(Path::new("/r/d")));
        assert_eq!(joined.file_name().unwrap().as_bytes(), b"\xff.txt");
    }

    #[test]
    fn detached_head_has_the_label_of_git() {
        let head = HeadState {
            branch: None,
            id: None,
        };
        assert_eq!(head.branch_label(), "(no branch)");
        assert_eq!(head.short_branch(), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
