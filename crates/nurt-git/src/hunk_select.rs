// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::hunk_select
// Responsibility:   Selection of hunks and rows by identifier, and the index content that results from it.
// Dependencies:     bstr, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Selection of hunks and rows by identifier, and the index content that results from it.
//!
//! Identifiers `path#number[:fingerprint][@rows]` ([`HunkId`]) name hunks of
//! the canonical difference of a file ([`hunks_for`]). [`selected_content`]
//! computes the content of the index entry of one file after the selected
//! hunks are staged ([`Mode::Add`]: the hunks between the index and the
//! working tree applied to the index content) or unstaged ([`Mode::Unstage`]:
//! the hunks between `HEAD` and the index taken out of the index content).
//! The result is the one `git apply --cached` produces with a patch reduced
//! to the selection: an unselected removed line stays and an unselected added
//! line is dropped.

use std::collections::BTreeMap;
use std::str::FromStr;

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::Result;

use crate::diff::{
    Direction, Hunk, HunkId, HunkProblem, Side, apply_selected, hunk_error, hunks_for, text_hunks,
};

/// What a selection does to the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Stage the selected changes of the working tree (`git.stage add`).
    Add,
    /// Take the selected staged changes out of the index (`git.stage unstage`).
    Unstage,
}

impl Mode {
    fn side(self) -> Side {
        match self {
            Self::Add => Side::Worktree,
            Self::Unstage => Side::Staged,
        }
    }

    fn direction(self) -> Direction {
        match self {
            Self::Add => Direction::Forward,
            Self::Unstage => Direction::Reverse,
        }
    }

    fn changes(self) -> &'static str {
        match self {
            Self::Add => "unstaged",
            Self::Unstage => "staged",
        }
    }
}

/// Content the selection is applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base<'a> {
    /// A file the index contains: its current index content.
    Tracked(&'a [u8]),
    /// A file the index does not contain, or contains as intent-to-add: its
    /// content in the working tree, in the form stored by Git. Only
    /// [`Mode::Add`] applies; the hunks are those of a difference from an
    /// empty file.
    New {
        /// Content of the file in the working tree.
        worktree: &'a [u8],
    },
}

/// Parses identifiers and groups them by file, in path order.
///
/// # Errors
///
/// `E_ARG` with problem `invalid_id` or `invalid_rows` for an identifier
/// that does not follow the grammar.
///
/// # Examples
///
/// ```
/// use nurt_git::hunk_select::group_ids;
///
/// let groups = group_ids(&["b.txt#1", "a.txt#2@3-5", "a.txt#1"]).unwrap();
/// let paths: Vec<String> = groups.keys().map(|path| path.to_string()).collect();
/// assert_eq!(paths, ["a.txt", "b.txt"]);
/// assert_eq!(groups.values().next().map(Vec::len), Some(2));
/// ```
pub fn group_ids<S: AsRef<str>>(texts: &[S]) -> Result<BTreeMap<BString, Vec<HunkId>>> {
    let mut out: BTreeMap<BString, Vec<HunkId>> = BTreeMap::new();
    for text in texts {
        let id = HunkId::from_str(text.as_ref())?;
        out.entry(BString::from(id.path.as_str()))
            .or_default()
            .push(id);
    }
    Ok(out)
}

/// Canonical hunks of `path` that a selection in `mode` refers to.
///
/// # Errors
///
/// The errors of [`hunks_for`].
pub fn hunks(repo: &gix::Repository, path: &BStr, mode: Mode) -> Result<Vec<Hunk>> {
    hunks_for(repo, mode.side(), path)
}

/// Content of the index entry of `path` after applying `ids` in `mode`.
///
/// For [`Base::Tracked`] the hunks are read from the repository with
/// [`hunks`]; for [`Base::New`] they are computed against an empty file.
///
/// # Errors
///
/// * `E_NOT_FOUND` with problem `not_found` when the file has no changes of
///   the kind `mode` selects, or a hunk number does not exist;
/// * `E_STALE` with problem `changed` when a fingerprint no longer matches;
/// * `E_ARG` with problem `invalid_rows`, `no_change_rows` or
///   `rows_on_added_or_deleted` for a row selection the hunk does not allow;
/// * `E_STAGE_FAILED` with problem `apply_failed` when the base does not
///   match the hunks;
/// * the errors of [`hunks_for`].
pub fn selected_content(
    repo: &gix::Repository,
    path: &BStr,
    ids: &[HunkId],
    mode: Mode,
    base: Base<'_>,
) -> Result<Vec<u8>> {
    let (content, hunks) = match base {
        Base::Tracked(content) => (content, hunks(repo, path, mode)?),
        Base::New { worktree } if mode == Mode::Add => {
            (&b""[..], text_hunks(&path.to_str_lossy(), b"", worktree))
        }
        Base::New { .. } => return Err(no_changes(path, mode)),
    };
    if hunks.is_empty() {
        return Err(no_changes(path, mode));
    }
    apply_selected(content, &hunks, ids, mode.direction())
}

fn no_changes(path: &BStr, mode: Mode) -> nurt_contracts::error::Error {
    let quoted = serde_json::Value::String(path.to_str_lossy().into_owned());
    hunk_error(
        HunkProblem::NotFound,
        format!(
            "{path} has no {} changes; list the hunks again",
            mode.changes()
        ),
    )
    .with_found(path.to_str_lossy().into_owned())
    .with_next_step(format!("git.stage action: list paths: [{quoted}]"))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;
    use crate::testutil::RepoBuilder;

    fn ids(texts: &[&str]) -> Vec<HunkId> {
        texts
            .iter()
            .map(|text| HunkId::from_str(text).unwrap())
            .collect()
    }

    #[test]
    fn identifiers_are_grouped_by_path_and_validated() {
        let groups = group_ids(&["dir/a#b.txt#3", "z#1", "dir/a#b.txt#1@2"]).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[BStr::new("dir/a#b.txt")].len(), 2);
        let error = group_ids(&["a.txt#0"]).unwrap_err();
        assert_eq!(error.code, ErrorCode::Arg);
    }

    #[test]
    fn staging_rows_of_a_worktree_hunk_keeps_unselected_lines() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\nb\nc\n")
            .commit("first")
            .worktree_file("a.txt", "a\nB\nc\nd\n")
            .build();
        let repo = fixture.open();
        let path = BStr::new("a.txt");
        let index = b"a\nb\nc\n";
        let all = selected_content(
            &repo,
            path,
            &ids(&["a.txt#1"]),
            Mode::Add,
            Base::Tracked(index),
        )
        .unwrap();
        assert_eq!(all, b"a\nB\nc\nd\n");
        let rows = selected_content(
            &repo,
            path,
            &ids(&["a.txt#1@2-3"]),
            Mode::Add,
            Base::Tracked(index),
        )
        .unwrap();
        assert_eq!(rows, b"a\nB\nc\n");
    }

    #[test]
    fn unstaging_a_staged_hunk_restores_the_head_content() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "one\ntwo\n")
            .commit("first")
            .file("a.txt", "one\n2\n")
            .commit("second")
            .reference("refs/heads/main", "HEAD~1")
            .build();
        let repo = fixture.open();
        let restored = selected_content(
            &repo,
            BStr::new("a.txt"),
            &ids(&["a.txt#1"]),
            Mode::Unstage,
            Base::Tracked(b"one\n2\n"),
        )
        .unwrap();
        assert_eq!(restored, b"one\ntwo\n");
    }

    #[test]
    fn new_file_can_be_staged_partially() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .build();
        let repo = fixture.open();
        let content = selected_content(
            &repo,
            BStr::new("n.txt"),
            &ids(&["n.txt#1@1,3"]),
            Mode::Add,
            Base::New {
                worktree: b"one\ntwo\nthree\n",
            },
        )
        .unwrap();
        assert_eq!(content, b"one\nthree\n");
    }

    #[test]
    fn file_without_changes_of_the_mode_is_reported() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .build();
        let repo = fixture.open();
        let error = selected_content(
            &repo,
            BStr::new("a.txt"),
            &ids(&["a.txt#1"]),
            Mode::Add,
            Base::Tracked(b"a\n"),
        )
        .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("not_found"));
        assert!(error.next_steps[0].starts_with("git.stage action: list"));
        let error = selected_content(
            &repo,
            BStr::new("n.txt"),
            &ids(&["n.txt#1"]),
            Mode::Unstage,
            Base::New { worktree: b"x\n" },
        )
        .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("not_found"));
    }

    #[test]
    fn stale_fingerprint_is_rejected() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .worktree_file("a.txt", "b\n")
            .build();
        let repo = fixture.open();
        let error = selected_content(
            &repo,
            BStr::new("a.txt"),
            &ids(&["a.txt#1:000000"]),
            Mode::Add,
            Base::Tracked(b"a\n"),
        )
        .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("changed"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
