// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::commit::tests
// Responsibility:   Files and links of the fixture, without the object store, which keeps written objects after undo.
// Dependencies:     bstr, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bstr::{BStr, ByteSlice};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::RollbackReason;
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec, Services,
};
use nurt_contracts::stage::{ArgValue, CancelToken, StageFrame};
use nurt_contracts::value::Value;

use super::*;
use crate::hooks::Hook;
use crate::testutil::{FixedClock, Fixture, RecordingJournal, RepoBuilder, TestHost, TreeEntry};

fn with_identity(builder: RepoBuilder) -> RepoBuilder {
    builder
        .config("user.name", RepoBuilder::AUTHOR_NAME)
        .config("user.email", RepoBuilder::AUTHOR_EMAIL)
}

fn context(host: Option<Arc<dyn ProcessHost>>) -> ExecCtx {
    let mut builder = ExecCtx::builder(
        Arc::new(FixedClock::at_unix_seconds(RepoBuilder::EPOCH)),
        Arc::new(TestHost::new("/nonexistent-home")),
    )
    .frame(StageFrame {
        command: COMMAND.to_owned(),
        ..StageFrame::default()
    });
    if let Some(host) = host {
        builder = builder.services(Arc::new(Services::new().with_process_host(host)));
    }
    builder.build()
}

fn commit_with(
    fixture: &Fixture,
    request: &CommitRequest,
    ctx: &ExecCtx,
    journal: &RecordingJournal,
) -> nurt_contracts::error::Result<CommitOutcome> {
    let repo = GitRepo::open(fixture.path(), ctx)?;
    let plan = plan(&repo, request)?;
    let mut tx = journal.begin();
    match apply(&repo, plan, tx.as_mut()) {
        Ok(outcome) => {
            tx.commit()?;
            Ok(outcome)
        }
        Err(error) => {
            tx.rollback(RollbackReason::TargetFailed)?;
            Err(error)
        }
    }
}

fn commit(
    fixture: &Fixture,
    request: &CommitRequest,
) -> nurt_contracts::error::Result<CommitOutcome> {
    commit_with(fixture, request, &context(None), &RecordingJournal::new())
}

/// Files and links of the fixture, without the object store, which keeps
/// written objects after undo.
fn files(fixture: &Fixture) -> BTreeMap<PathBuf, TreeEntry> {
    fixture
        .snapshot()
        .into_iter()
        .filter(|(path, entry)| {
            !matches!(entry, TreeEntry::Directory)
                && !path.components().any(|part| part.as_os_str() == "objects")
        })
        .collect()
}

fn head_commit(fixture: &Fixture) -> gix::ObjectId {
    fixture.open().head_id().unwrap().detach()
}

fn tree_paths(fixture: &Fixture, id: gix::ObjectId) -> Vec<(String, String)> {
    let repo = fixture.open();
    let tree = repo.find_commit(id).unwrap().tree_id().unwrap().detach();
    let state: gix::index::State = repo.index_from_tree(&tree).unwrap().into();
    state
        .entries()
        .iter()
        .map(|entry| {
            let data = repo.find_object(entry.id).unwrap().data.clone();
            (
                entry.path(&state).to_string(),
                String::from_utf8(data).unwrap(),
            )
        })
        .collect()
}

fn last_reflog(fixture: &Fixture, name: &str) -> String {
    let log = std::fs::read_to_string(fixture.git_dir().join("logs").join(name)).unwrap();
    log.lines().last().unwrap().to_owned()
}

fn stage(fixture: &Fixture, path: &str, content: &str) {
    std::fs::write(fixture.path().join(path), content).unwrap();
    let ctx = context(None);
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    let mut index = repo.read_index().unwrap();
    let id = repo.gix().write_blob(content.as_bytes()).unwrap().detach();
    access::upsert_entry(
        &mut index.state,
        BStr::new(path),
        id,
        EntryKind::Blob,
        Default::default(),
    );
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    crate::repo::index::write_index(tx.as_mut(), &mut index).unwrap();
    tx.commit().unwrap();
}

fn has_note(outcome: &CommitOutcome, code: Notable) -> bool {
    outcome.notable.iter().any(|note| note.code == code)
}

#[test]
fn first_commit_creates_the_branch_and_undo_removes_it() {
    let fixture = with_identity(RepoBuilder::new())
        .worktree_file("a.txt", "1\n")
        .build();
    let before = files(&fixture);
    let journal = RecordingJournal::new();
    let request = CommitRequest::new("first", Selection::All);
    let outcome = commit_with(&fixture, &request, &context(None), &journal).unwrap();
    assert!(outcome.commit.parents.is_empty());
    assert_eq!(outcome.commit.refs, Some(vec!["main".to_owned()]));
    assert_eq!(outcome.commit.branch.as_deref(), Some("main"));
    assert_eq!(head_commit(&fixture), outcome.id);
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![("a.txt".to_owned(), "1\n".to_owned())]
    );
    assert!(last_reflog(&fixture, "HEAD").ends_with("\tcommit (initial): first"));
    let git = outcome.effect.git.clone().unwrap();
    assert_eq!(git.head_before, None);
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert_eq!(outcome.commit.stats.as_ref().unwrap().added, 1);
    journal.undo_last();
    assert_eq!(files(&fixture), before);
}

#[test]
fn staged_commit_records_exactly_the_index() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.txt", "2\n")
        .commit("one")
        .build();
    stage(&fixture, "a.txt", "staged\n");
    std::fs::write(fixture.path().join("a.txt"), "worktree\n").unwrap();
    std::fs::write(fixture.path().join("b.txt"), "changed\n").unwrap();
    let parent = head_commit(&fixture);
    let before = files(&fixture);
    let journal = RecordingJournal::new();
    let outcome = commit_with(
        &fixture,
        &CommitRequest::new("two", Selection::Staged),
        &context(None),
        &journal,
    )
    .unwrap();
    assert_eq!(outcome.commit.parents, vec![parent.to_string()]);
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("a.txt".to_owned(), "staged\n".to_owned()),
            ("b.txt".to_owned(), "2\n".to_owned())
        ]
    );
    assert!(last_reflog(&fixture, "refs/heads/main").ends_with("\tcommit: two"));
    let file = &outcome.commit.files.as_ref().unwrap()[0];
    assert_eq!(
        (file.path.as_str(), file.status),
        ("a.txt", GitFileState::Modified)
    );
    journal.undo_last();
    assert_eq!(files(&fixture), before);
}

#[test]
fn all_stages_modifications_deletions_and_untracked_files() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.txt", "2\n")
        .file(".gitignore", "*.log\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "1+\n").unwrap();
    std::fs::remove_file(fixture.path().join("b.txt")).unwrap();
    std::fs::write(fixture.path().join("c.txt"), "3\n").unwrap();
    std::fs::write(fixture.path().join("debug.log"), "x\n").unwrap();
    let before = files(&fixture);
    let journal = RecordingJournal::new();
    let outcome = commit_with(
        &fixture,
        &CommitRequest::new("two", Selection::All),
        &context(None),
        &journal,
    )
    .unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            (".gitignore".to_owned(), "*.log\n".to_owned()),
            ("a.txt".to_owned(), "1+\n".to_owned()),
            ("c.txt".to_owned(), "3\n".to_owned())
        ]
    );
    let index = fixture.open().open_index().unwrap();
    let paths: Vec<String> = index
        .entries()
        .iter()
        .map(|entry| entry.path(&index).to_string())
        .collect();
    assert_eq!(paths, vec![".gitignore", "a.txt", "c.txt"]);
    let statuses: Vec<GitFileState> = outcome
        .commit
        .files
        .unwrap()
        .iter()
        .map(|file| file.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            GitFileState::Modified,
            GitFileState::Deleted,
            GitFileState::Added
        ]
    );
    journal.undo_last();
    assert_eq!(files(&fixture), before);
}

#[test]
fn paths_commit_head_with_the_selected_files_only() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.txt", "2\n")
        .commit("one")
        .build();
    stage(&fixture, "b.txt", "staged b\n");
    std::fs::write(fixture.path().join("a.txt"), "worktree a\n").unwrap();
    std::fs::write(fixture.path().join("n.txt"), "new\n").unwrap();
    let before = files(&fixture);
    let journal = RecordingJournal::new();
    let request = CommitRequest::new(
        "only",
        Selection::Paths(vec!["a.txt".to_owned(), "n.txt".to_owned()]),
    );
    let outcome = commit_with(&fixture, &request, &context(None), &journal).unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("a.txt".to_owned(), "worktree a\n".to_owned()),
            ("b.txt".to_owned(), "2\n".to_owned()),
            ("n.txt".to_owned(), "new\n".to_owned())
        ]
    );
    let repo = fixture.open();
    let index = repo.open_index().unwrap();
    let staged_b = index.entry_by_path(BStr::new("b.txt")).unwrap().id;
    assert_eq!(
        repo.find_object(staged_b).unwrap().data.as_slice(),
        b"staged b\n"
    );
    assert!(index.entry_by_path(BStr::new("n.txt")).is_some());
    journal.undo_last();
    assert_eq!(files(&fixture), before);
}

#[test]
fn paths_that_match_nothing_are_rejected() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let request = CommitRequest::new("x", Selection::Paths(vec!["missing.txt".to_owned()]));
    let error = commit(&fixture, &request).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(error.data().problem.as_deref(), Some("path_not_found"));
}

/// Size that the index holds for the stage 0 entry of `path`.
fn index_size(fixture: &Fixture, path: &str) -> u32 {
    let ctx = context(None);
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    let index = repo.read_index().unwrap();
    index
        .state
        .entry_by_path(BStr::new(path))
        .unwrap()
        .stat
        .size
}

#[test]
fn a_file_changed_in_the_second_of_the_index_write_is_marked_when_other_paths_are_committed() {
    let fixture = with_identity(RepoBuilder::new().config("core.trustctime", "false"))
        .worktree_file("kept.txt", "1\n")
        .worktree_file("same.txt", "s\n")
        .worktree_file("other.txt", "a\n")
        .build();
    commit(&fixture, &CommitRequest::new("first", Selection::All)).unwrap();
    assert_eq!(index_size(&fixture, "kept.txt"), 2);

    // The change keeps the size, and the file and the index keep the time of
    // the entry: by its statistics the file is unchanged, and only the age of
    // the index tells a reader to compare the content.
    let kept = fixture.path().join("kept.txt");
    let written = std::fs::metadata(&kept).unwrap().modified().unwrap();
    std::fs::write(&kept, "2\n").unwrap();
    for path in [kept, fixture.git_dir().join("index")] {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(written).unwrap();
    }
    std::fs::write(fixture.path().join("other.txt"), "b\n").unwrap();

    let request = CommitRequest::new("second", Selection::Paths(vec!["other.txt".to_owned()]));
    let outcome = commit(&fixture, &request).unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("kept.txt".to_owned(), "1\n".to_owned()),
            ("other.txt".to_owned(), "b\n".to_owned()),
            ("same.txt".to_owned(), "s\n".to_owned()),
        ]
    );
    assert_eq!(
        index_size(&fixture, "kept.txt"),
        0,
        "the changed file is compared by content from now on"
    );
    assert_eq!(index_size(&fixture, "same.txt"), 2);
    assert_eq!(index_size(&fixture, "other.txt"), 2);
}

#[test]
fn amend_alone_replaces_only_the_message() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .file("a.txt", "2\n")
        .commit("two")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "3\n").unwrap();
    let repo = fixture.open();
    let old = repo.find_commit(head_commit(&fixture)).unwrap();
    let old_tree = old.tree_id().unwrap().detach();
    let old_parents: Vec<String> = old.parent_ids().map(|id| id.to_string()).collect();
    let old_time = old.author().unwrap().seconds();
    let before = files(&fixture);
    let journal = RecordingJournal::new();
    let request = CommitRequest::new("two, reworded", Selection::MessageOnly);
    let outcome = commit_with(&fixture, &request, &context(None), &journal).unwrap();
    let new = fixture
        .open()
        .find_commit(outcome.id)
        .unwrap()
        .tree_id()
        .unwrap()
        .detach();
    assert_eq!(new, old_tree);
    assert_eq!(outcome.commit.parents, old_parents);
    assert_eq!(
        outcome.commit.time,
        crate::repo::records::instant(gix::date::Time {
            seconds: old_time,
            offset: 0
        })
    );
    assert!(has_note(&outcome, Notable::HistoryRewritten));
    assert!(last_reflog(&fixture, "HEAD").ends_with("\tcommit (amend): two, reworded"));
    journal.undo_last();
    assert_eq!(files(&fixture), before);
}

#[test]
fn amend_with_staged_changes_replaces_the_tree() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .file("a.txt", "2\n")
        .commit("two")
        .build();
    stage(&fixture, "a.txt", "3\n");
    let mut request = CommitRequest::new("two again", Selection::Staged);
    request.amend = true;
    let outcome = commit(&fixture, &request).unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![("a.txt".to_owned(), "3\n".to_owned())]
    );
    assert_eq!(outcome.commit.parents.len(), 1);
}

#[test]
fn nothing_to_commit_fails_unless_empty_commits_are_allowed() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "changed\n").unwrap();
    let request = CommitRequest::new("empty", Selection::Staged);
    let error = commit(&fixture, &request).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitNothingToCommit);
    assert!(
        error
            .data()
            .fix
            .iter()
            .any(|fix| fix.ends_with("allow_empty: true"))
    );
    assert!(
        error
            .data()
            .fix
            .iter()
            .any(|fix| fix.ends_with("all: true"))
    );
    let mut allowed = request;
    allowed.allow_empty = true;
    let outcome = commit(&fixture, &allowed).unwrap();
    assert_eq!(outcome.commit.stats.unwrap().files, 0);
}

#[test]
fn sign_off_and_trailers_produce_the_commit_object_of_git() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let parent = head_commit(&fixture);
    let mut request = CommitRequest::new("feat: x", Selection::All);
    request.sign_off = true;
    request.trailers = vec![Trailer::parse("Refs: 1").unwrap()];
    let outcome = commit(&fixture, &request).unwrap();
    let repo = fixture.open();
    let object = repo.find_object(outcome.id).unwrap();
    let tree = repo.find_commit(outcome.id).unwrap().tree_id().unwrap();
    let ident = format!(
        "{} <{}> {} +0000",
        RepoBuilder::AUTHOR_NAME,
        RepoBuilder::AUTHOR_EMAIL,
        RepoBuilder::EPOCH
    );
    let expected = format!(
        "tree {tree}\nparent {parent}\nauthor {ident}\ncommitter {ident}\n\nfeat: x\n\nSigned-off-by: {} <{}>\nRefs: 1\n",
        RepoBuilder::AUTHOR_NAME,
        RepoBuilder::AUTHOR_EMAIL
    );
    assert_eq!(object.data.to_str().unwrap(), expected);
    assert_eq!(
        outcome
            .commit
            .trailers
            .unwrap()
            .get("Refs")
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn commit_encoding_is_recorded_in_the_header() {
    let fixture = with_identity(RepoBuilder::new())
        .config("i18n.commitEncoding", "ISO-8859-2")
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let mut request = CommitRequest::new("x", Selection::Staged);
    request.allow_empty = true;
    let outcome = commit(&fixture, &request).unwrap();
    let data = fixture.open().find_object(outcome.id).unwrap().data.clone();
    assert!(data.find(b"\nencoding ISO-8859-2\n").is_some());
}

#[test]
fn author_argument_and_cleanup_shape_the_commit() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let mut request = CommitRequest::new("  subject  \n\n\n\nbody\n\n", Selection::Staged);
    request.allow_empty = true;
    request.author = Some("Bo Builder <bo@example.com>".to_owned());
    let outcome = commit(&fixture, &request).unwrap();
    assert_eq!(outcome.commit.author, "Bo Builder");
    assert_eq!(
        outcome.commit.committer.as_deref(),
        Some(RepoBuilder::AUTHOR_NAME)
    );
    assert_eq!(outcome.commit.message, "  subject\n\nbody\n");
}

#[test]
fn merge_in_progress_is_concluded_by_a_staged_commit() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .reference("refs/heads/side", "HEAD")
        .file("b.txt", "2\n")
        .commit("two")
        .build();
    let side = fixture.open().rev_parse_single("side").unwrap().detach();
    let head = head_commit(&fixture);
    std::fs::write(fixture.git_dir().join("MERGE_HEAD"), format!("{side}\n")).unwrap();
    std::fs::write(fixture.git_dir().join("MERGE_MSG"), "Merge branch 'side'\n").unwrap();
    std::fs::write(fixture.git_dir().join("MERGE_MODE"), "").unwrap();
    let mut amend = CommitRequest::new("x", Selection::Staged);
    amend.amend = true;
    assert_eq!(
        commit(&fixture, &amend).unwrap_err().code(),
        ErrorCode::ParamConflict
    );
    let partial = CommitRequest::new("x", Selection::Paths(vec!["a.txt".to_owned()]));
    assert_eq!(
        commit(&fixture, &partial).unwrap_err().code(),
        ErrorCode::ParamConflict
    );
    let before = files(&fixture);
    let journal = RecordingJournal::new();
    let request = CommitRequest::new("Merge branch 'side'", Selection::Staged);
    let outcome = commit_with(&fixture, &request, &context(None), &journal).unwrap();
    assert_eq!(
        outcome.commit.parents,
        vec![head.to_string(), side.to_string()]
    );
    assert!(!fixture.git_dir().join("MERGE_HEAD").exists());
    assert!(!fixture.git_dir().join("MERGE_MSG").exists());
    assert!(last_reflog(&fixture, "HEAD").ends_with("\tcommit (merge): Merge branch 'side'"));
    journal.undo_last();
    assert_eq!(files(&fixture), before);
}

#[test]
fn detached_head_moves_head_itself() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let id = head_commit(&fixture);
    std::fs::write(fixture.git_dir().join("HEAD"), format!("{id}\n")).unwrap();
    let mut request = CommitRequest::new("detached", Selection::Staged);
    request.allow_empty = true;
    let outcome = commit(&fixture, &request).unwrap();
    let head = std::fs::read_to_string(fixture.git_dir().join("HEAD")).unwrap();
    assert_eq!(head.trim(), outcome.id.to_string());
    assert!(has_note(&outcome, Notable::CommitDetachedHead));
    assert_eq!(outcome.effect.git.unwrap().detached, Some(true));
}

#[test]
fn excluded_paths_stay_out_of_the_commit_and_are_counted() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("secret.env", "k=1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    std::fs::write(fixture.path().join("secret.env"), "k=2\n").unwrap();
    let mut request = CommitRequest::new("two", Selection::All);
    request.exclude = vec!["*.env".to_owned()];
    let outcome = commit(&fixture, &request).unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("a.txt".to_owned(), "2\n".to_owned()),
            ("secret.env".to_owned(), "k=1\n".to_owned())
        ]
    );
    let note = outcome
        .notable
        .iter()
        .find(|note| note.code == Notable::Excluded)
        .unwrap();
    assert_eq!(note.count, Some(1));
}

#[test]
fn staged_exclusion_restores_the_head_version() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.cfg", "x\n")
        .commit("one")
        .build();
    stage(&fixture, "a.txt", "2\n");
    stage(&fixture, "b.cfg", "y\n");
    let mut request = CommitRequest::new("two", Selection::Staged);
    request.exclude = vec!["*.cfg".to_owned()];
    let outcome = commit(&fixture, &request).unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("a.txt".to_owned(), "2\n".to_owned()),
            ("b.cfg".to_owned(), "x\n".to_owned())
        ]
    );
}

#[test]
fn state_token_guards_against_changes() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let ctx = context(None);
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    let token = prepare::state_token(&repo).unwrap();
    stage(&fixture, "a.txt", "2\n");
    let mut request = CommitRequest::new("two", Selection::Staged);
    request.expect_state = Some(token);
    assert_eq!(
        commit(&fixture, &request).unwrap_err().code(),
        ErrorCode::Stale
    );
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    request.expect_state = Some(prepare::state_token(&repo).unwrap());
    let outcome = commit(&fixture, &request).unwrap();
    let after = GitRepo::open(fixture.path(), &ctx).unwrap();
    assert_eq!(
        outcome.effect.git.unwrap().state_token,
        Some(prepare::state_token(&after).unwrap())
    );
}

#[test]
fn missing_identity_names_the_author_argument() {
    let fixture = RepoBuilder::new()
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let mut request = CommitRequest::new("x", Selection::Staged);
    request.allow_empty = true;
    let error = commit(&fixture, &request).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Config);
    assert_eq!(error.data().problem.as_deref(), Some("missing_identity"));
}

#[test]
fn unresolved_conflicts_stop_the_commit() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let ctx = context(None);
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    let mut index = repo.read_index().unwrap();
    let entry = index.state.entries()[0].clone();
    let ours = gix::index::entry::Flags::from_stage(gix::index::entry::Stage::Ours);
    index
        .state
        .dangerously_push_entry(entry.stat, entry.id, ours, entry.mode, BStr::new("a.txt"));
    index.state.sort_entries();
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    crate::repo::index::write_index(tx.as_mut(), &mut index).unwrap();
    tx.commit().unwrap();
    let error = commit(&fixture, &CommitRequest::new("x", Selection::Staged)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitInProgress);
}

#[test]
fn selection_flags_exclude_each_other() {
    assert_eq!(
        Selection::from_flags(Vec::new(), false, false, true).unwrap(),
        Selection::MessageOnly
    );
    assert_eq!(
        Selection::from_flags(Vec::new(), true, false, false).unwrap(),
        Selection::All
    );
    let missing = Selection::from_flags(Vec::new(), false, false, false).unwrap_err();
    assert_eq!(missing.data().problem.as_deref(), Some("missing_selection"));
    let both = Selection::from_flags(vec!["a".to_owned()], true, false, false).unwrap_err();
    assert_eq!(both.code(), ErrorCode::ParamConflict);
    let request = CommitRequest::new("say \"hi\"", Selection::Paths(vec!["a b.txt".to_owned()]));
    assert_eq!(
        render_call(
            Path::new("/r"),
            &request,
            &request.selection,
            &["hooks: false"]
        ),
        "git.commit \"/r\" message: \"say \\\"hi\\\"\" paths: [\"a b.txt\"] hooks: false"
    );
}

/// Program service that plays the part of the hooks of a repository.
#[derive(Default)]
struct HookHost {
    fail: Option<Hook>,
    append_to_message: Option<String>,
    stage_on_pre_commit: Option<(PathBuf, String, String)>,
    ran: Mutex<Vec<(String, Vec<String>)>>,
}

impl HookHost {
    fn ran(&self) -> Vec<String> {
        self.ran
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn stage_into(index_file: &Path, repo_path: &Path, path: &str, content: &str) {
        let repo = gix::open_opts(repo_path, crate::repo::open_options()).unwrap();
        let id = repo.write_blob(content.as_bytes()).unwrap().detach();
        let file =
            gix::index::File::at(index_file, gix::hash::Kind::Sha1, false, Default::default())
                .unwrap();
        let mut state: gix::index::State = file.into();
        access::upsert_entry(
            &mut state,
            BStr::new(path),
            id,
            EntryKind::Blob,
            Default::default(),
        );
        let bytes = crate::repo::index::serialize(&mut state, &[]).unwrap();
        std::fs::write(index_file, bytes).unwrap();
    }
}

impl ProcessHost for HookHost {
    fn resolve_program(
        &self,
        program: &str,
        _cwd: &Path,
    ) -> nurt_contracts::error::Result<PathBuf> {
        Ok(PathBuf::from(program))
    }

    fn run_program(
        &self,
        spec: &ProgramSpec,
        _cancel: &CancelToken,
    ) -> nurt_contracts::error::Result<ProgramOutcome> {
        let name = Path::new(&spec.program)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let index = spec
            .env
            .iter()
            .find(|var| var.name == "GIT_INDEX_FILE")
            .and_then(|var| var.value.clone())
            .unwrap();
        self.ran
            .lock()
            .unwrap()
            .push((name.clone(), spec.args.clone()));
        if name == "commit-msg"
            && let Some(suffix) = &self.append_to_message
        {
            let mut text = std::fs::read_to_string(&spec.args[0]).unwrap();
            text.push_str(suffix);
            std::fs::write(&spec.args[0], text).unwrap();
        }
        if name == "pre-commit"
            && let Some((repo, path, content)) = &self.stage_on_pre_commit
        {
            Self::stage_into(Path::new(&index), repo, path, content);
        }
        let failed = self.fail.is_some_and(|hook| hook.file_name() == name);
        Ok(ProgramOutcome {
            program_path: PathBuf::from(&spec.program),
            exit_code: Some(i32::from(failed)),
            stderr: if failed {
                b"rejected by hook\n".to_vec()
            } else {
                Vec::new()
            },
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(
        &self,
        _spec: &ServiceSpawnSpec,
    ) -> nurt_contracts::error::Result<Box<dyn ServiceChild>> {
        Err(nurt_contracts::error::Error::new(
            ErrorCode::Unsupported,
            "services are not used by hooks",
        ))
    }
}

fn install_hooks(fixture: &Fixture, hooks: &[Hook]) {
    let dir = fixture.git_dir().join("hooks");
    std::fs::create_dir_all(&dir).unwrap();
    for hook in hooks {
        let path = dir.join(hook.file_name());
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[test]
fn hooks_run_in_order_and_commit_msg_changes_the_message() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    install_hooks(
        &fixture,
        &[
            Hook::PreCommit,
            Hook::PrepareCommitMsg,
            Hook::CommitMsg,
            Hook::PostCommit,
        ],
    );
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let host = Arc::new(HookHost {
        append_to_message: Some("\nChecked-by: hook\n".to_owned()),
        ..HookHost::default()
    });
    let ctx = context(Some(host.clone()));
    let outcome = commit_with(
        &fixture,
        &CommitRequest::new("two", Selection::All),
        &ctx,
        &RecordingJournal::new(),
    )
    .unwrap();
    assert_eq!(
        host.ran(),
        vec![
            "pre-commit",
            "prepare-commit-msg",
            "commit-msg",
            "post-commit"
        ]
    );
    assert_eq!(outcome.commit.message, "two\n\nChecked-by: hook\n");
    assert_eq!(outcome.irreversible.len(), 4);
    let editmsg = std::fs::read_to_string(fixture.git_dir().join("COMMIT_EDITMSG")).unwrap();
    assert_eq!(editmsg, "two\n\nChecked-by: hook\n");
    let ran = host.ran.lock().unwrap();
    assert_eq!(ran[1].1[1], "message");
}

#[test]
fn failing_hook_stops_the_commit_without_changes() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    install_hooks(&fixture, &[Hook::PreCommit]);
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let before = files(&fixture);
    let host = Arc::new(HookHost {
        fail: Some(Hook::PreCommit),
        ..HookHost::default()
    });
    let ctx = context(Some(host));
    let error = commit_with(
        &fixture,
        &CommitRequest::new("two", Selection::All),
        &ctx,
        &RecordingJournal::new(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitHook);
    assert_eq!(error.data().sample, vec!["rejected by hook".to_owned()]);
    assert!(error.data().fix[0].ends_with("all: true hooks: false"));
    assert_eq!(files(&fixture), before);
}

#[test]
fn pre_commit_hook_can_stage_more_content() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    install_hooks(&fixture, &[Hook::PreCommit]);
    stage(&fixture, "a.txt", "2\n");
    let host = Arc::new(HookHost {
        stage_on_pre_commit: Some((
            fixture.path().to_path_buf(),
            "gen.txt".to_owned(),
            "generated\n".to_owned(),
        )),
        ..HookHost::default()
    });
    let ctx = context(Some(host));
    let outcome = commit_with(
        &fixture,
        &CommitRequest::new("two", Selection::Staged),
        &ctx,
        &RecordingJournal::new(),
    )
    .unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("a.txt".to_owned(), "2\n".to_owned()),
            ("gen.txt".to_owned(), "generated\n".to_owned())
        ]
    );
    let index = fixture.open().open_index().unwrap();
    assert!(index.entry_by_path(BStr::new("gen.txt")).is_some());
}

#[test]
fn disabled_hooks_are_reported_and_not_run() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    install_hooks(&fixture, &[Hook::CommitMsg]);
    let host = Arc::new(HookHost::default());
    let ctx = context(Some(host.clone()));
    let mut request = CommitRequest::new("x", Selection::Staged);
    request.allow_empty = true;
    request.hooks = false;
    let outcome = commit_with(&fixture, &request, &ctx, &RecordingJournal::new()).unwrap();
    assert!(host.ran().is_empty());
    assert!(has_note(&outcome, Notable::HooksSkipped));
}

#[test]
fn plan_lists_the_changed_files_without_writing() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let before = files(&fixture);
    let ctx = context(None);
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    let plan = plan(&repo, &CommitRequest::new("two", Selection::All)).unwrap();
    let paths: Vec<String> = plan
        .changes()
        .iter()
        .map(|change| change.path.to_string())
        .collect();
    assert_eq!(paths.len(), 3);
    assert!(paths[0].ends_with("index"));
    assert!(paths[1].ends_with("COMMIT_EDITMSG"));
    assert!(paths[2].ends_with("main"));
    assert_eq!(plan.parents().len(), 1);
    assert_eq!(files(&fixture), before);
}

#[test]
fn sign_false_overrides_commit_gpg_sign() {
    let fixture = with_identity(RepoBuilder::new())
        .config("commit.gpgSign", "true")
        .config("gpg.format", "unknown-format")
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let mut request = CommitRequest::new("feat: x", Selection::All);
    let error = commit(&fixture, &request).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitSign);
    request.sign = Some(false);
    let outcome = commit(&fixture, &request).unwrap();
    assert!(!has_note(&outcome, Notable::Signed));
    let repo = fixture.open();
    let object = repo.find_object(outcome.id).unwrap();
    assert!(!object.data.contains_str("gpgsig"));
}

#[test]
fn sign_true_signs_without_commit_gpg_sign() {
    let fixture = with_identity(RepoBuilder::new())
        .config("gpg.format", "unknown-format")
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let mut request = CommitRequest::new("feat: x", Selection::All);
    request.sign = Some(true);
    let error = commit(&fixture, &request).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitSign);
    request.sign = None;
    assert!(commit(&fixture, &request).is_ok());
}

fn bound(pairs: &[(&str, Value)]) -> BoundArgs {
    pairs
        .iter()
        .fold(BoundArgs::new(COMMAND), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

fn path_value(fixture: &Fixture) -> Value {
    Value::Path(fixture.path().to_string_lossy().into_owned())
}

fn text_value(text: &str) -> Value {
    Value::Text(text.to_owned())
}

#[test]
fn stage_plans_and_applies_a_commit_from_bound_arguments() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.txt", "2\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "1+\n").unwrap();
    std::fs::write(fixture.path().join("b.txt"), "2+\n").unwrap();
    let ctx = context(None);
    let command = GitCommit;
    assert_eq!(command.descriptor().command, COMMAND);
    let args = bound(&[
        ("repo", path_value(&fixture)),
        ("message", text_value("two")),
        ("all", Value::Bool(true)),
    ]);
    let effect = command.as_effect().unwrap();
    let plan = effect.plan(&args, StageInput::none(), &ctx).unwrap();
    assert!(plan.changes.len() >= 2);
    assert!(plan.bulk);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let outcome = effect.apply(&args, plan, tx.as_mut(), &ctx).unwrap();
    tx.commit().unwrap();
    assert!(outcome.changes.len() >= 2);
    match outcome.output.data {
        nurt_contracts::stage::StageData::Records(stream) => {
            let records = stream.collect::<Result<Vec<_>>>().unwrap();
            assert_eq!(records.len(), 2);
        }
        _ => panic!("expected records"),
    }
}

#[test]
fn stage_apply_rejects_a_plan_of_another_stage() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let args = bound(&[("repo", path_value(&fixture))]);
    let error = GitCommit
        .apply(
            &args,
            EffectPlan::new(),
            RecordingJournal::new().begin().as_mut(),
            &context(None),
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
}

#[test]
fn bound_arguments_are_validated() {
    let fixture = RepoBuilder::new().file("a.txt", "1\n").build();
    let repo = path_value(&fixture);
    let missing_message = bound(&[("repo", repo.clone()), ("staged", Value::Bool(true))]);
    assert_eq!(
        request_from_args(&missing_message).unwrap_err().code(),
        ErrorCode::MissingParam
    );
    let wrong_type = bound(&[("message", Value::Int(1)), ("staged", Value::Bool(true))]);
    assert_eq!(
        request_from_args(&wrong_type).unwrap_err().code(),
        ErrorCode::Type
    );
    let wrong_list = bound(&[
        ("message", text_value("m")),
        ("paths", Value::List(vec![Value::Int(1)])),
    ]);
    assert_eq!(
        request_from_args(&wrong_list).unwrap_err().code(),
        ErrorCode::Type
    );
    let bad_trailer = bound(&[
        ("message", text_value("m")),
        ("staged", Value::Bool(true)),
        ("trailers", Value::List(vec![text_value("no separator")])),
    ]);
    let error = request_from_args(&bad_trailer).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.data().problem.as_deref(), Some("invalid_trailer"));
    let no_repo = bound(&[("message", text_value("m")), ("staged", Value::Bool(true))]);
    assert_eq!(
        GitCommit
            .as_effect()
            .unwrap()
            .plan(&no_repo, StageInput::none(), &context(None))
            .map(|_| ())
            .unwrap_err()
            .code(),
        ErrorCode::MissingParam
    );
    let full = bound(&[
        ("message", text_value("m")),
        ("paths", path_value(&fixture)),
        ("author", text_value("A <a@x.invalid>")),
        ("allow_empty", Value::Bool(true)),
        ("hooks", Value::Bool(false)),
        ("sign", Value::Bool(false)),
        ("sign_off", Value::Bool(true)),
        (
            "exclude",
            Value::List(vec![Value::Glob("*.log".to_owned())]),
        ),
        ("expect_state", text_value("token")),
        (
            "trailers",
            Value::List(vec![text_value("Reviewed-by: R <r@x.invalid>")]),
        ),
    ]);
    let request = request_from_args(&full).unwrap();
    assert!(matches!(request.selection, Selection::Paths(ref paths) if paths.len() == 1));
    assert!(request.allow_empty && request.sign_off && !request.hooks);
    assert_eq!(request.sign, Some(false));
    assert_eq!(request.exclude, ["*.log"]);
    assert_eq!(request.expect_state.as_deref(), Some("token"));
    assert_eq!(request.trailers.len(), 1);
}

#[cfg(unix)]
#[test]
fn file_list_reports_type_changes_and_binary_files() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.txt", "2\n")
        .commit("one")
        .build();
    std::fs::remove_file(fixture.path().join("a.txt")).unwrap();
    std::os::unix::fs::symlink("b.txt", fixture.path().join("a.txt")).unwrap();
    std::fs::write(fixture.path().join("blob.bin"), [0_u8, 1, 2, 0]).unwrap();
    let outcome = commit(&fixture, &CommitRequest::new("two", Selection::All)).unwrap();
    let files = outcome.commit.files.unwrap();
    let find = |name: &str| files.iter().find(|file| file.path == name).unwrap();
    assert_eq!(find("a.txt").status, GitFileState::TypeChanged);
    assert_eq!(find("blob.bin").status, GitFileState::Added);
    assert_eq!(find("blob.bin").binary, Some(true));
    assert_eq!(find("a.txt").binary, Some(false));
}

#[test]
fn all_keeps_a_gitlink_whose_directory_exists() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let submodule = head_commit(&fixture);
    let ctx = context(None);
    let repo = GitRepo::open(fixture.path(), &ctx).unwrap();
    let mut index = repo.read_index().unwrap();
    access::upsert_entry(
        &mut index.state,
        BStr::new("sub"),
        submodule,
        EntryKind::Commit,
        Default::default(),
    );
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    crate::repo::index::write_index(tx.as_mut(), &mut index).unwrap();
    tx.commit().unwrap();
    std::fs::create_dir(fixture.path().join("sub")).unwrap();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let outcome = commit(&fixture, &CommitRequest::new("two", Selection::All)).unwrap();
    let repo = fixture.open();
    let tree = repo
        .find_commit(outcome.id)
        .unwrap()
        .tree_id()
        .unwrap()
        .detach();
    let state: gix::index::State = repo.index_from_tree(&tree).unwrap().into();
    let entry = state.entry_by_path(BStr::new("sub")).expect("gitlink kept");
    assert_eq!(entry.mode, gix::index::entry::Mode::COMMIT);
    assert_eq!(entry.id, submodule);
}

#[cfg(unix)]
#[test]
fn all_records_the_text_of_a_link_byte_for_byte() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::os::unix::fs::symlink("dir\\file", fixture.path().join("link")).unwrap();
    let outcome = commit(&fixture, &CommitRequest::new("two", Selection::All)).unwrap();
    assert!(
        tree_paths(&fixture, outcome.id).contains(&("link".to_owned(), "dir\\file".to_owned()))
    );
}

#[cfg(unix)]
#[test]
fn a_real_link_is_recorded_as_a_link_when_symlinks_are_disabled() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .config("core.symlinks", "false")
        .commit("one")
        .build();
    std::os::unix::fs::symlink("a.txt", fixture.path().join("link")).unwrap();
    let outcome = commit(&fixture, &CommitRequest::new("two", Selection::All)).unwrap();
    assert!(tree_paths(&fixture, outcome.id).contains(&("link".to_owned(), "a.txt".to_owned())));
}

#[test]
fn a_message_with_a_zero_byte_is_refused_before_anything_is_written() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let head = head_commit(&fixture);
    let before = files(&fixture);
    let error = commit(&fixture, &CommitRequest::new("a\0b", Selection::All)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.data().problem.as_deref(), Some("nul_in_message"));
    assert_eq!(head_commit(&fixture), head);
    assert_eq!(files(&fixture), before);
}

#[cfg(unix)]
#[test]
fn all_leaves_out_a_socket_and_refuses_a_tracked_path_taken_by_one() {
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .file("b.txt", "1\n")
        .commit("one")
        .build();
    let Ok(_listener) = std::os::unix::net::UnixListener::bind(fixture.path().join("s.sock"))
    else {
        // The path of the temporary directory may exceed the length of a socket address.
        return;
    };
    std::fs::write(fixture.path().join("a.txt"), "2\n").unwrap();
    let outcome = commit(&fixture, &CommitRequest::new("two", Selection::All)).unwrap();
    assert_eq!(
        tree_paths(&fixture, outcome.id),
        vec![
            ("a.txt".to_owned(), "2\n".to_owned()),
            ("b.txt".to_owned(), "1\n".to_owned())
        ]
    );
    std::fs::remove_file(fixture.path().join("b.txt")).unwrap();
    let Ok(_other) = std::os::unix::net::UnixListener::bind(fixture.path().join("b.txt")) else {
        return;
    };
    let error = commit(&fixture, &CommitRequest::new("three", Selection::All)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
    assert_eq!(error.data().problem.as_deref(), Some("special_file"));
}

// APFS rejects a name that is not valid UTF-8, so the setup cannot be made there.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn all_follows_files_whose_names_are_not_unicode() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = with_identity(RepoBuilder::new())
        .file("a.txt", "1\n")
        .commit("one")
        .build();
    let name = b"caf\xe9.txt";
    let file = fixture.path().join(std::ffi::OsStr::from_bytes(name));
    std::fs::write(&file, "1\n").unwrap();
    let added = commit(&fixture, &CommitRequest::new("add", Selection::All)).unwrap();
    std::fs::write(&file, "2\n").unwrap();
    let changed = commit(&fixture, &CommitRequest::new("change", Selection::All)).unwrap();
    for (id, content) in [(added.id, b"1\n"), (changed.id, b"2\n")] {
        let repo = fixture.open();
        let tree = repo.find_commit(id).unwrap().tree_id().unwrap().detach();
        let state: gix::index::State = repo.index_from_tree(&tree).unwrap().into();
        let entry = state.entry_by_path(BStr::new(name)).expect("file recorded");
        assert_eq!(repo.find_object(entry.id).unwrap().data.as_slice(), content);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
