// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::restore::hunks
// Responsibility:   Restoration of selected hunks, as `git restore --patch` does.
// Dependencies:     bstr, gix, gix_index, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Restoration of selected hunks, as `git restore --patch` does.
//!
//! Hunks are named by the identifiers of `git.stage` (`path#number`, with an
//! optional fingerprint and row selection). In the working tree the selected
//! hunks between the index and the file are taken back; in the index the
//! selected hunks between `HEAD` and the index are taken back. The restored
//! content of every file is written to a tree of its own, which is then
//! restored like a source revision, so the working tree and index rules of
//! [`plan_paths`] apply unchanged.

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::object::tree::EntryKind;
use gix_index::entry::{Flags, Stage};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_git::apply::{ApplyContext, ApplyPlan, PathSource, PathsRequest, plan_paths};
use nurt_git::diff::{
    Direction, HunkId, HunkProblem, Reader, Side, apply_selected, hunk_error, hunks_for,
};
use nurt_git::hunk_select::{self, Base, Mode};

use super::RestoreRequest;

/// Mode of a regular file in the index.
const MODE_FILE: u32 = 0o100_644;
/// Mode of an executable file in the index.
const MODE_EXECUTABLE: u32 = 0o100_755;
/// Mode of a symbolic link in the index.
const MODE_LINK: u32 = 0o120_000;
/// Pathspec prefix that matches a path byte for byte.
const LITERAL: &str = ":(literal)";

/// Side whose hunks are taken back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Changes of the working tree against the index.
    Worktree,
    /// Changes of the index against `HEAD`.
    Index,
}

/// Plans the restoration of the hunks of `request`.
pub(super) fn plan_hunks(ctx: &ApplyContext<'_>, request: &RestoreRequest) -> Result<ApplyPlan> {
    let target = target_of(request)?;
    let groups = hunk_select::group_ids(&request.hunks)?;
    let repo = ctx.repo();
    let mut reader = Reader::new(repo)?;
    let mut editor = repo
        .edit_tree(ObjectId::empty_tree(repo.object_hash()))
        .map_err(|err| object_error("start a tree", err))?;
    let mut pathspec = Vec::with_capacity(groups.len());
    for (path, ids) in &groups {
        let path = path.as_bstr();
        ensure_selected(&request.paths, path)?;
        let restored = match target {
            Target::Worktree => worktree_content(ctx, &mut reader, path, ids)?,
            Target::Index => index_content(ctx, &mut reader, path, ids)?,
        };
        if let Some((content, kind)) = restored {
            let blob = repo
                .write_blob(&content)
                .map_err(|err| object_error("write the restored content", err))?
                .detach();
            editor
                .upsert(path, kind, blob)
                .map_err(|err| object_error("build the restored tree", err))?;
        }
        let mut spec = BString::from(LITERAL);
        spec.extend_from_slice(path);
        pathspec.push(spec);
    }
    let tree = editor
        .write()
        .map_err(|err| object_error("write the restored tree", err))?
        .detach();
    plan_paths(
        ctx,
        &PathsRequest {
            source: PathSource::Tree(tree),
            pathspec,
            to_index: target == Target::Index,
            to_worktree: target == Target::Worktree,
        },
    )
}

fn target_of(request: &RestoreRequest) -> Result<Target> {
    if request.source.is_some() {
        return Err(unsupported(
            "hunks_source",
            "hunks are taken back against the index or HEAD; remove source, or restore whole files from the revision",
        ));
    }
    match (request.staged, request.worktree) {
        (true, false) => Ok(Target::Index),
        (false, true) => Ok(Target::Worktree),
        _ => Err(unsupported(
            "hunks_both",
            "hunks restore the working tree or the index, not both; give staged: true worktree: false for staged hunks",
        )),
    }
}

/// Content of the working tree file after the hunks `ids` are taken back,
/// with the kind of the file.
fn worktree_content(
    ctx: &ApplyContext<'_>,
    reader: &mut Reader<'_>,
    path: &BStr,
    ids: &[HunkId],
) -> Result<Option<(Vec<u8>, EntryKind)>> {
    let mode = ctx
        .index()
        .entry_by_path_and_stage(path, Stage::Unconflicted)
        .filter(|entry| !entry.flags.contains(Flags::INTENT_TO_ADD))
        .map(|entry| entry.mode.bits())
        .ok_or_else(|| no_changes(path))?;
    let hunks = hunks_for(ctx.repo(), Side::Worktree, path)?;
    if hunks.is_empty() {
        return Err(no_changes(path));
    }
    let (current, current_mode) = reader
        .worktree(path, Some(mode))?
        .unwrap_or((Vec::new(), mode));
    let restored = apply_selected(&current, &hunks, ids, Direction::Reverse)?;
    let kind = kind_of(current_mode)
        .or_else(|| kind_of(mode))
        .ok_or_else(|| unsupported_mode(path))?;
    Ok(Some((restored, kind)))
}

/// Content of the index entry after the hunks `ids` are taken back, with
/// its kind; `None` when the entry leaves the index.
fn index_content(
    ctx: &ApplyContext<'_>,
    reader: &mut Reader<'_>,
    path: &BStr,
    ids: &[HunkId],
) -> Result<Option<(Vec<u8>, EntryKind)>> {
    let current = ctx
        .index()
        .entry_by_path_and_stage(path, Stage::Unconflicted)
        .map(|entry| (entry.id, entry.mode.bits()));
    let base = match current {
        Some((id, _)) => reader.blob(id)?,
        None => Vec::new(),
    };
    let restored =
        hunk_select::selected_content(ctx.repo(), path, ids, Mode::Unstage, Base::Tracked(&base))?;
    let head = head_kind(ctx.repo(), path)?;
    if head.is_none() && restored.is_empty() {
        return Ok(None);
    }
    let kind = match current {
        Some((_, mode)) => kind_of(mode).ok_or_else(|| unsupported_mode(path))?,
        None => head.unwrap_or(EntryKind::Blob),
    };
    Ok(Some((restored, kind)))
}

/// Kind of the entry of `path` in the tree of `HEAD`; `None` on an unborn
/// branch or when `HEAD` lacks the path.
fn head_kind(repo: &gix::Repository, path: &BStr) -> Result<Option<EntryKind>> {
    let Ok(head) = repo.head_id() else {
        return Ok(None);
    };
    let tree = repo
        .find_commit(head.detach())
        .map_err(|err| object_error("read HEAD", err))?
        .tree()
        .map_err(|err| object_error("read the tree of HEAD", err))?;
    let relative =
        gix::path::from_bstr(path).map_err(|err| object_error("convert the path", err))?;
    Ok(tree
        .lookup_entry_by_path(relative)
        .map_err(|err| object_error("read the tree of HEAD", err))?
        .map(|entry| entry.mode().kind())
        .filter(|kind| *kind != EntryKind::Tree))
}

fn kind_of(mode: u32) -> Option<EntryKind> {
    match mode {
        MODE_FILE => Some(EntryKind::Blob),
        MODE_EXECUTABLE => Some(EntryKind::BlobExecutable),
        MODE_LINK => Some(EntryKind::Link),
        _ => None,
    }
}

/// Checks that `path` lies at or below one of `paths`, compared literally.
fn ensure_selected(paths: &[BString], path: &BStr) -> Result<()> {
    let selected = paths.is_empty()
        || paths.iter().any(|spec| {
            let spec = spec.trim_end_with(|c| c == '/');
            spec.is_empty()
                || spec == b"."
                || spec == b":/"
                || spec == path.as_bytes()
                || (path.starts_with(spec) && path.get(spec.len()) == Some(&b'/'))
        });
    if selected {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::Arg,
        format!("the hunks of {path} are outside paths; add {path} to paths"),
    )
    .with_problem("hunk_outside_paths")
    .with_found(path.to_string()))
}

fn no_changes(path: &BStr) -> Error {
    let quoted = serde_json::Value::String(path.to_str_lossy().into_owned());
    hunk_error(
        HunkProblem::NotFound,
        format!("{path} has no unstaged changes; list the hunks again"),
    )
    .with_found(path.to_str_lossy().into_owned())
    .with_next_step(format!("git.stage action: list paths: [{quoted}]"))
}

fn unsupported(problem: &str, message: &str) -> Error {
    Error::new(ErrorCode::GitUnsupported, message.to_owned()).with_problem(problem)
}

fn unsupported_mode(path: &BStr) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!("{path} is a submodule; restore it as a whole path without hunks"),
    )
    .with_problem("hunks_submodule")
    .with_found(path.to_string())
}

fn object_error(what: &str, err: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {err}"))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::journal::MutationTx;
    use nurt_git::apply::apply_plan;
    use nurt_git::apply::testjournal::{RecordingJournal, tree_state};
    use nurt_git::testutil::{Fixture, RepoBuilder};

    use super::super::plan_restore;
    use super::*;

    /// Ten numbered lines; a change of the first and of the last line gives
    /// two hunks of the canonical difference.
    const LINES: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    const CHANGED: &str = "one\n2\n3\n4\n5\n6\n7\n8\n9\nten\n";

    fn request(paths: &[&str], hunks: &[&str]) -> RestoreRequest {
        RestoreRequest {
            paths: paths.iter().map(|path| BString::from(*path)).collect(),
            source: None,
            staged: false,
            worktree: true,
            hunks: hunks.iter().map(|hunk| (*hunk).to_owned()).collect(),
        }
    }

    fn staged(paths: &[&str], hunks: &[&str]) -> RestoreRequest {
        let mut request = request(paths, hunks);
        request.staged = true;
        request.worktree = false;
        request
    }

    fn restore(
        fixture: &Fixture,
        journal: &RecordingJournal,
        request: &RestoreRequest,
    ) -> Result<()> {
        let repo = fixture.open();
        let ctx = ApplyContext::open(&repo)?;
        let plan = plan_restore(&ctx, request)?;
        let mut tx = journal.begin();
        apply_plan(&mut tx, &ctx, plan)?;
        Box::new(tx).commit()?;
        Ok(())
    }

    fn plan_error(fixture: &Fixture, request: &RestoreRequest) -> Error {
        let repo = fixture.open();
        let ctx = ApplyContext::open(&repo).expect("context");
        plan_restore(&ctx, request).expect_err("refused")
    }

    fn read(fixture: &Fixture, path: &str) -> Option<String> {
        std::fs::read_to_string(fixture.path().join(path)).ok()
    }

    fn index_text(fixture: &Fixture, path: &str) -> Option<String> {
        let repo = fixture.open();
        let index = repo.open_index().expect("index");
        let entry = index.entry_by_path(path.as_bytes().as_bstr())?;
        let blob = repo.find_blob(entry.id).expect("blob");
        Some(String::from_utf8_lossy(&blob.data).into_owned())
    }

    fn worktree_fixture() -> Fixture {
        RepoBuilder::new()
            .file("a.txt", LINES)
            .file("b.txt", "b\n")
            .commit("first")
            .worktree_file("a.txt", CHANGED)
            .build()
    }

    /// `HEAD` has [`CHANGED`]; the index and the working tree have
    /// [`LINES`], so both hunks are staged changes of the index.
    fn staged_fixture(journal: &RecordingJournal) -> Fixture {
        let fixture = RepoBuilder::new()
            .file("a.txt", LINES)
            .commit("first")
            .file("a.txt", CHANGED)
            .commit("second")
            .worktree_file("a.txt", LINES)
            .build();
        let mut older = request(&["a.txt"], &[]);
        older.source = Some("HEAD~1".to_owned());
        older.staged = true;
        restore(&fixture, journal, &older).expect("stage the older content");
        fixture
    }

    #[test]
    fn worktree_hunk_is_taken_back_and_other_hunk_stays() {
        let fixture = worktree_fixture();
        let journal = RecordingJournal::new();
        restore(&fixture, &journal, &request(&["a.txt"], &["a.txt#2"])).expect("restore");
        assert_eq!(
            read(&fixture, "a.txt").as_deref(),
            Some("one\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")
        );
        assert_eq!(index_text(&fixture, "a.txt").as_deref(), Some(LINES));
    }

    #[test]
    fn all_worktree_hunks_restore_the_index_content() {
        let fixture = worktree_fixture();
        let journal = RecordingJournal::new();
        restore(
            &fixture,
            &journal,
            &request(&["."], &["a.txt#1", "a.txt#2"]),
        )
        .expect("restore");
        assert_eq!(read(&fixture, "a.txt").as_deref(), Some(LINES));
    }

    #[test]
    fn deleted_worktree_file_comes_back_from_its_hunk() {
        let fixture = worktree_fixture();
        std::fs::remove_file(fixture.path().join("b.txt")).expect("remove");
        let journal = RecordingJournal::new();
        restore(&fixture, &journal, &request(&["b.txt"], &["b.txt#1"])).expect("restore");
        assert_eq!(read(&fixture, "b.txt").as_deref(), Some("b\n"));
    }

    #[test]
    fn staged_hunk_is_taken_out_of_the_index_and_worktree_stays() {
        let journal = RecordingJournal::new();
        let fixture = staged_fixture(&journal);
        assert_eq!(index_text(&fixture, "a.txt").as_deref(), Some(LINES));
        restore(&fixture, &journal, &staged(&["a.txt"], &["a.txt#1"])).expect("restore");
        assert_eq!(
            index_text(&fixture, "a.txt").as_deref(),
            Some("one\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")
        );
        assert_eq!(read(&fixture, "a.txt").as_deref(), Some(LINES));
    }

    #[test]
    fn staged_new_file_leaves_the_index_when_its_hunk_is_taken_back() {
        let journal = RecordingJournal::new();
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .file("n.txt", "new\n")
            .commit("second")
            .reference("refs/heads/main", "HEAD~1")
            .build();
        restore(&fixture, &journal, &staged(&["n.txt"], &["n.txt#1"])).expect("restore");
        assert_eq!(index_text(&fixture, "n.txt"), None);
        assert_eq!(read(&fixture, "n.txt").as_deref(), Some("new\n"));
    }

    #[test]
    fn hunks_refuse_a_source_and_both_sides() {
        let fixture = worktree_fixture();
        let mut with_source = request(&["a.txt"], &["a.txt#1"]);
        with_source.source = Some("HEAD".to_owned());
        let error = plan_error(&fixture, &with_source);
        assert_eq!(error.code(), ErrorCode::GitUnsupported);
        assert_eq!(error.data().problem.as_deref(), Some("hunks_source"));
        let mut both = request(&["a.txt"], &["a.txt#1"]);
        both.staged = true;
        let error = plan_error(&fixture, &both);
        assert_eq!(error.data().problem.as_deref(), Some("hunks_both"));
    }

    #[test]
    fn hunk_outside_paths_is_refused() {
        let fixture = worktree_fixture();
        let error = plan_error(&fixture, &request(&["src", "a"], &["a.txt#1"]));
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(error.data().problem.as_deref(), Some("hunk_outside_paths"));
        assert!(ensure_selected(&[BString::from("dir/")], BStr::new("dir/a.txt")).is_ok());
        assert!(ensure_selected(&[BString::from("dir")], BStr::new("dirx/a.txt")).is_err());
    }

    #[test]
    fn missing_hunks_and_files_without_changes_are_reported() {
        let fixture = worktree_fixture();
        let error = plan_error(&fixture, &request(&["a.txt"], &["a.txt#3"]));
        assert_eq!(error.data().problem.as_deref(), Some("not_found"));
        let error = plan_error(&fixture, &request(&["b.txt"], &["b.txt#1"]));
        assert_eq!(error.data().problem.as_deref(), Some("not_found"));
        assert!(!error.data().next_steps.is_empty());
        let error = plan_error(&fixture, &request(&["new.txt"], &["new.txt#1"]));
        assert_eq!(error.data().problem.as_deref(), Some("not_found"));
        let error = plan_error(&fixture, &request(&["a.txt"], &["a.txt"]));
        assert_eq!(error.code(), ErrorCode::Arg);
    }

    #[test]
    fn stale_fingerprint_stops_the_restoration() {
        let fixture = worktree_fixture();
        let error = plan_error(&fixture, &request(&["a.txt"], &["a.txt#1:000000"]));
        assert_eq!(error.data().problem.as_deref(), Some("changed"));
    }

    #[test]
    fn undo_of_hunk_restoration_restores_the_local_change() {
        let fixture = worktree_fixture();
        let before = tree_state(fixture.path());
        let journal = RecordingJournal::new();
        restore(&fixture, &journal, &request(&["a.txt"], &["a.txt#1"])).expect("restore");
        assert_ne!(tree_state(fixture.path()), before);
        journal.undo_last();
        assert_eq!(tree_state(fixture.path()), before);
    }

    #[test]
    fn index_modes_map_to_tree_kinds() {
        assert_eq!(kind_of(MODE_FILE), Some(EntryKind::Blob));
        assert_eq!(kind_of(MODE_EXECUTABLE), Some(EntryKind::BlobExecutable));
        assert_eq!(kind_of(MODE_LINK), Some(EntryKind::Link));
        assert_eq!(kind_of(0o160_000), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
