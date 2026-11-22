// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Construction of the errors reported by `git.rebase`.

use std::fmt::Display;
use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Stage};

/// Program text of the repository argument in fixes and next steps.
pub(super) fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    format!("\"{}\"", text.replace('"', "\\\""))
}

/// Failure to read or write repository data.
pub(super) fn io(what: &str, cause: impl Display) -> Error {
    Error::new(ErrorCode::Io, format!("{what}: {cause}"))
}

/// A repository file or object that does not have the expected form.
pub(super) fn corrupt(what: &str, cause: impl Display) -> Error {
    Error::new(ErrorCode::GitUnsupported, format!("{what}: {cause}"))
        .with_problem("unreadable_state")
        .with_stage(Stage::Read)
}

/// A revision that cannot be resolved to a commit.
pub(super) fn revision(repo: &Path, spec: &str, cause: impl Display) -> Error {
    Error::new(
        ErrorCode::GitRevision,
        format!("cannot resolve revision `{spec}` to a commit: {cause}"),
    )
    .with_stage(Stage::Read)
    .with_found(spec)
    .with_fix(format!("git.log {} count: 20", quoted(repo)))
}

/// A rebase is required but none is in progress, or one is in progress
/// when a new one is requested.
pub(super) fn in_progress(repo: &Path, running: bool) -> Error {
    let repo_text = quoted(repo);
    if running {
        Error::new(
            ErrorCode::GitInProgress,
            "a rebase is already in progress; continue, skip or abort it first",
        )
        .with_stage(Stage::Plan)
        .with_problem("rebase")
        .with_fix(format!("git.rebase {repo_text} action: continue"))
        .with_fix(format!("git.rebase {repo_text} action: abort"))
    } else {
        Error::new(
            ErrorCode::GitInProgress,
            "no rebase is in progress; start one with a revision in `onto`",
        )
        .with_stage(Stage::Plan)
        .with_problem("none")
        .with_fix(format!("git.rebase {repo_text} onto: \"main\""))
    }
}

/// Another operation (merge, cherry-pick, revert, apply) is in progress.
pub(super) fn other_operation(repo: &Path, operation: &str) -> Error {
    Error::new(
        ErrorCode::GitInProgress,
        format!("a {operation} is in progress; finish or abort it before a rebase"),
    )
    .with_stage(Stage::Plan)
    .with_problem(operation)
    .with_fix(format!("git.status {}", quoted(repo)))
}

/// Feature of the repository or of the request that is not supported.
pub(super) fn unsupported(problem: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::GitUnsupported, message)
        .with_stage(Stage::Plan)
        .with_problem(problem)
}

/// A lock file of the repository exists.
pub(super) fn locked(repo: &Path, lock: &Path) -> Error {
    Error::new(
        ErrorCode::GitLocked,
        format!(
            "lock file {} exists; another process is changing the repository",
            lock.display()
        ),
    )
    .with_stage(Stage::Plan)
    .with_detail(
        "path",
        serde_json::Value::from(lock.to_string_lossy().into_owned()),
    )
    .with_next_step(format!("git.status {}", quoted(repo)))
}
