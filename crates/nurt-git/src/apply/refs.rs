// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Loose reference files and reference logs written by a plan.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::plan::{RefPlan, RefTarget, ReflogLine};
use super::worktree::{expect_of, observe_native};

/// Reference names stored per working tree rather than in the common
/// repository directory.
const PER_WORKTREE_PREFIXES: [&str; 3] = ["refs/bisect/", "refs/worktree/", "refs/rewritten/"];

/// Prefixes of references whose log `git` creates by default
/// (`core.logAllRefUpdates=true`).
const LOGGED_PREFIXES: [&str; 3] = ["refs/heads/", "refs/remotes/", "refs/notes/"];

fn is_per_worktree(name: &str) -> bool {
    !name.starts_with("refs/")
        || PER_WORKTREE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn base_dir<'a>(repo: &'a gix::Repository, name: &str) -> &'a Path {
    if is_per_worktree(name) {
        repo.git_dir()
    } else {
        repo.common_dir()
    }
}

fn checked_name(name: &str) -> Result<&str> {
    if gix::refs::FullName::try_from(name).is_ok() {
        Ok(name)
    } else {
        Err(Error::new(
            ErrorCode::GitRef,
            format!("'{name}' is not a valid reference name; use a name such as refs/heads/main"),
        )
        .with_problem("invalid_name")
        .with_found(name.to_owned()))
    }
}

/// File of the loose reference `name`.
///
/// # Errors
///
/// `E_GIT_REF` for an invalid reference name.
pub fn ref_path(repo: &gix::Repository, name: &str) -> Result<PathBuf> {
    let name = checked_name(name)?;
    Ok(base_dir(repo, name).join(native_name(name)))
}

/// File of the log of the reference `name`.
///
/// # Errors
///
/// `E_GIT_REF` for an invalid reference name.
pub fn log_path(repo: &gix::Repository, name: &str) -> Result<PathBuf> {
    let name = checked_name(name)?;
    Ok(base_dir(repo, name).join("logs").join(native_name(name)))
}

fn native_name(name: &str) -> PathBuf {
    name.split('/').collect()
}

/// Whether `git` keeps a log for the reference `name` in `repo`: the log
/// exists already, or `core.logAllRefUpdates` asks for it.
///
/// # Errors
///
/// `E_GIT_REF` for an invalid reference name.
pub fn keeps_log(repo: &gix::Repository, name: &str) -> Result<bool> {
    if log_path(repo, name)?.exists() {
        return Ok(true);
    }
    let setting = repo
        .config_snapshot()
        .string("core.logAllRefUpdates")
        .map(|value| value.to_ascii_lowercase());
    Ok(match setting.as_deref() {
        Some(b"always") => true,
        Some(b"false" | b"no" | b"off" | b"0") => false,
        _ => {
            !repo.is_bare()
                && (name == "HEAD"
                    || LOGGED_PREFIXES
                        .iter()
                        .any(|prefix| name.starts_with(prefix)))
        }
    })
}

impl RefPlan {
    /// Update of `name` to `target`, expecting the loose reference file to
    /// be unchanged since this call; the log line is kept only when
    /// [`keeps_log`] says so.
    ///
    /// # Errors
    ///
    /// `E_GIT_REF` for an invalid reference name and `E_IO` when the
    /// reference file cannot be read.
    pub fn new(
        repo: &gix::Repository,
        name: &str,
        target: RefTarget,
        reflog: Option<ReflogLine>,
    ) -> Result<Self> {
        let path = ref_path(repo, name)?;
        let expect = expect_of(&path, &observe_native(&path)?)?;
        let reflog = match reflog {
            Some(line) if keeps_log(repo, name)? => Some(line),
            _ => None,
        };
        Ok(Self {
            name: name.to_owned(),
            target,
            expect,
            reflog,
        })
    }
}

/// Content of a loose reference file.
pub(crate) fn ref_content(target: &RefTarget) -> Vec<u8> {
    match target {
        RefTarget::Object(id) => format!("{id}\n").into_bytes(),
        RefTarget::Symbolic(name) => format!("ref: {name}\n").into_bytes(),
    }
}

/// One reference log line in the format of `git`; the message is folded to
/// one line with single spaces.
pub(crate) fn reflog_bytes(line: &ReflogLine) -> Result<Vec<u8>> {
    let mut out = format!("{} {} ", line.old, line.new).into_bytes();
    line.committer.write_to(&mut out).map_err(|err| {
        Error::new(
            ErrorCode::Io,
            format!("cannot format the reflog line: {err}"),
        )
    })?;
    let message = line
        .message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !message.is_empty() {
        out.push(b'\t');
        out.extend_from_slice(message.as_bytes());
    }
    out.push(b'\n');
    Ok(out)
}
