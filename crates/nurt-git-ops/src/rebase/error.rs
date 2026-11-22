// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Construction of the errors reported by `git.rebase`.

use std::fmt::Display;

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Stage};

use crate::state::quote;

/// Failure to read or write repository data.
pub(super) fn io(what: &str, cause: impl Display) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {cause}"))
}

/// A repository file or object that does not have the expected form.
pub(super) fn corrupt(what: &str, cause: impl Display) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!("{what}: {cause}; abort the rebase with git.rebase action: abort"),
    )
    .with_problem("unreadable_state")
    .with_stage(Stage::Read)
}

/// A revision that cannot be resolved to a commit.
pub(super) fn revision(repo: &str, spec: &str, cause: impl Display) -> Error {
    Error::new(
        ErrorCode::GitRevision,
        format!(
            "cannot resolve revision {spec} to a commit: {cause}; give a branch, tag or commit"
        ),
    )
    .with_stage(Stage::Read)
    .with_found(spec.to_owned())
    .with_fix(format!("git.log {} count: 20", quote(repo)))
}

/// `start` without the revision to rebase onto.
pub(super) fn missing_onto(repo: &str) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        "a rebase needs the revision to rebase onto; give onto: \"<revision>\"",
    )
    .with_problem("onto")
    .with_fix(format!("git.rebase {} onto: \"main\"", quote(repo)))
}

/// `continue`, `skip` or `abort` while no rebase is in progress.
pub(super) fn no_rebase(repo: &str, action: &str) -> Error {
    Error::new(
        ErrorCode::GitInProgress,
        format!("there is no rebase to {action}; start one with onto: \"<revision>\""),
    )
    .with_problem("no_rebase_in_progress")
    .with_stage(Stage::Plan)
    .with_fix(format!("git.rebase {} onto: \"main\"", quote(repo)))
    .with_next_step(format!("git.status {}", quote(repo)))
}

/// Feature of the repository or of the request that is not supported.
pub(super) fn unsupported(problem: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::GitUnsupported, message)
        .with_stage(Stage::Plan)
        .with_problem(problem)
}

/// Commit signing requested by `commit.gpgSign`.
pub(super) fn signing(repo: &str) -> Error {
    Error::new(
        ErrorCode::GitSign,
        "commit.gpgSign is set, but replayed commits cannot be signed; set commit.gpgSign to false for this repository",
    )
    .with_problem("signing_unavailable")
    .with_stage(Stage::Plan)
    .with_next_step(format!("git.status {}", quote(repo)))
}

/// `HEAD` has no commit.
pub(super) fn unborn(repo: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        "HEAD has no commit; create a commit before rebasing",
    )
    .with_problem("unborn_branch")
    .with_stage(Stage::Plan)
    .with_next_step(format!("git.status {}", quote(repo)))
}

/// The index has unresolved paths.
pub(super) fn unmerged(repo: &str, paths: &[BString]) -> Error {
    let list = path_list(paths);
    let mut error = Error::new(
        ErrorCode::GitConflict,
        "the index has unresolved conflicts; resolve them and stage the result",
    )
    .with_problem("unmerged_paths")
    .with_stage(Stage::Plan)
    .with_effect_applied(false);
    for path in paths {
        error = error.with_sample(path.to_string());
    }
    error
        .with_fix(format!(
            "git.stage {} action: add paths: [{list}]",
            quote(repo)
        ))
        .with_next_step(format!("git.rebase {} action: skip", quote(repo)))
        .with_next_step(format!("git.rebase {} action: abort", quote(repo)))
}

/// Uncommitted changes that a rebase would lose.
pub(super) fn dirty(repo: &str, problem: &str, paths: &[BString]) -> Error {
    let what = if problem == "staged_changes" {
        "the index contains uncommitted changes"
    } else {
        "the working tree has unstaged changes"
    };
    let mut error = Error::new(
        ErrorCode::GitDirty,
        format!("cannot rebase: {what}; commit or stash them first"),
    )
    .with_problem(problem)
    .with_stage(Stage::Plan);
    for path in paths {
        error = error.with_sample(path.to_string());
    }
    error
        .with_fix(format!("git.stash {} action: push", quote(repo)))
        .with_next_step(format!("git.status {}", quote(repo)))
}

/// Untracked files that the checkout of the new base would overwrite.
pub(super) fn untracked(repo: &str, paths: &[BString]) -> Error {
    let mut error = Error::new(
        ErrorCode::GitDirty,
        "untracked working tree files would be overwritten by the rebase; move or remove them first",
    )
    .with_problem("untracked_overwritten")
    .with_stage(Stage::Plan);
    for path in paths {
        error = error.with_sample(path.to_string());
    }
    error.with_next_step(format!("git.status {}", quote(repo)))
}

/// Unstaged changes left while continuing a stopped rebase.
pub(super) fn unstaged_on_continue(repo: &str, paths: &[BString]) -> Error {
    let list = path_list(paths);
    let mut error = Error::new(
        ErrorCode::GitDirty,
        "the working tree has unstaged changes; stage them or restore the files before continuing",
    )
    .with_problem("unstaged_changes")
    .with_stage(Stage::Plan);
    for path in paths {
        error = error.with_sample(path.to_string());
    }
    error
        .with_fix(format!(
            "git.stage {} action: add paths: [{list}]",
            quote(repo)
        ))
        .with_next_step(format!("git.rebase {} action: skip", quote(repo)))
}

/// Comma separated list of quoted paths for a call.
fn path_list(paths: &[BString]) -> String {
    paths
        .iter()
        .map(|path| quote(&path.to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_errors_name_the_problem_and_offer_a_stash() {
        let error = dirty("/r", "staged_changes", &[BString::from("a.txt")]);
        assert_eq!(error.code(), ErrorCode::GitDirty);
        assert!(error.data().message.contains("index"));
        assert_eq!(error.data().sample, vec!["a.txt".to_owned()]);
        assert_eq!(
            error.data().fix,
            vec!["git.stash \"/r\" action: push".to_owned()]
        );
        let error = dirty("/r", "unstaged_changes", &[]);
        assert!(error.data().message.contains("working tree"));
    }

    #[test]
    fn unmerged_error_stages_the_listed_paths() {
        let error = unmerged("/r", &[BString::from("a"), BString::from("b c")]);
        assert_eq!(
            error.data().fix,
            vec!["git.stage \"/r\" action: add paths: [\"a\", \"b c\"]".to_owned()]
        );
        assert!(!error.data().effect_applied);
    }

    #[test]
    fn plan_errors_carry_their_codes() {
        assert_eq!(missing_onto("/r").code(), ErrorCode::MissingParam);
        assert_eq!(no_rebase("/r", "skip").code(), ErrorCode::GitInProgress);
        assert_eq!(signing("/r").code(), ErrorCode::GitSign);
        assert_eq!(unborn("/r").code(), ErrorCode::GitRef);
        assert_eq!(
            revision("/r", "x", "missing").code(),
            ErrorCode::GitRevision
        );
        assert_eq!(io("read", "denied").code(), ErrorCode::Io);
        assert_eq!(corrupt("bad", "x").code(), ErrorCode::GitUnsupported);
        assert_eq!(unsupported("p", "m").data().problem.as_deref(), Some("p"));
        assert_eq!(unstaged_on_continue("/r", &[]).code(), ErrorCode::GitDirty);
        assert_eq!(
            untracked("/r", &[BString::from("x")])
                .data()
                .problem
                .as_deref(),
            Some("untracked_overwritten")
        );
    }
}
