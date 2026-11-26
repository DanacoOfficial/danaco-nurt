// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of `git.stash`.

use gix::bstr::ByteSlice;
use nurt_contracts::stage::{ArgValue, BoundArgs, EffectStage, ReadStage, StageImpl, StageInput};
use nurt_contracts::value::Value;

use super::*;
use crate::reset::index;
use crate::reset::repo::{commit_parents, commit_tree};
use crate::reset::testing::{
    Fixture, Journal, RepoBuilder, blob_id, ctx, git_file, index_entries, index_id, read,
    reflog_messages, remove, rev, stage, state, write,
};
use nurt_git::repo::GitRepo;

/// One commit `first` with `a.txt`, `b.txt` and `dir/c.txt`.
fn base() -> Fixture {
    RepoBuilder::new()
        .file("a.txt", "a1\n")
        .file("b.txt", "b1\n")
        .file("dir/c.txt", "c1\n")
        .commit("first")
        .build()
}

fn run(fx: &Fixture, journal: &Journal, request: &StashRequest) -> Result<StashOutcome> {
    let planned = plan(fx.path(), request, &ctx())?;
    journal.run(fx.path(), |tx| planned.apply(tx))
}

fn push(fx: &Fixture, journal: &Journal) -> StashOutcome {
    run(fx, journal, &StashRequest::new(StashAction::Push)).unwrap()
}

fn on_entry(action: StashAction, entry: &str) -> StashRequest {
    StashRequest {
        entry: Some(entry.to_owned()),
        ..StashRequest::new(action)
    }
}

/// `a.txt` changed in the working tree only, `b.txt` changed and staged.
fn dirty(fx: &Fixture) {
    write(fx, "a.txt", "a-work\n");
    write(fx, "b.txt", "b-staged\n");
    stage(fx, "b.txt");
}

struct CommitInfo {
    message: String,
    parents: Vec<ObjectId>,
    tree: ObjectId,
}

fn commit(fx: &Fixture, id: ObjectId) -> CommitInfo {
    let repo = GitRepo::open(fx.path(), &ctx()).unwrap();
    let object = repo.gix().find_commit(id).unwrap();
    CommitInfo {
        message: object.message_raw().unwrap().to_str_lossy().into_owned(),
        parents: commit_parents(&repo, id).unwrap(),
        tree: commit_tree(&repo, id).unwrap(),
    }
}

fn tree_blob(fx: &Fixture, tree: ObjectId, path: &str) -> Option<ObjectId> {
    let repo = GitRepo::open(fx.path(), &ctx()).unwrap();
    let state = index::from_tree(&repo, tree).unwrap();
    index::merged_entry(&state, path.as_bytes().as_bstr()).map(|entry| entry.id)
}

fn short(fx: &Fixture, id: ObjectId) -> String {
    GitRepo::open(fx.path(), &ctx()).unwrap().short_id(&id)
}

#[test]
fn push_saves_index_and_working_tree_and_resets_both() {
    let fx = base();
    let journal = Journal::new();
    let head = rev(&fx, "HEAD");
    dirty(&fx);
    let outcome = push(&fx, &journal);
    let work = outcome.created.unwrap();
    let w = commit(&fx, work);
    assert_eq!(
        w.message,
        format!("WIP on main: {} first", short(&fx, head))
    );
    assert_eq!(w.parents.len(), 2);
    assert_eq!(w.parents[0], head);
    assert_eq!(tree_blob(&fx, w.tree, "a.txt"), Some(blob_id("a-work\n")));
    assert_eq!(tree_blob(&fx, w.tree, "b.txt"), Some(blob_id("b-staged\n")));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a1\n"));
    assert_eq!(read(&fx, "b.txt").as_deref(), Some("b1\n"));
    assert_eq!(index_id(&fx, "b.txt"), Some(blob_id("b1\n")));
}

#[test]
fn push_records_the_index_and_updates_the_logs() {
    let fx = base();
    let journal = Journal::new();
    let head = rev(&fx, "HEAD");
    dirty(&fx);
    let work = push(&fx, &journal).created.unwrap();
    let w = commit(&fx, work);
    let i = commit(&fx, w.parents[1]);
    assert_eq!(
        i.message,
        format!("index on main: {} first\n", short(&fx, head))
    );
    assert_eq!(i.parents, vec![head]);
    assert_eq!(tree_blob(&fx, i.tree, "a.txt"), Some(blob_id("a1\n")));
    assert_eq!(tree_blob(&fx, i.tree, "b.txt"), Some(blob_id("b-staged\n")));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a1\n"));
    assert_eq!(read(&fx, "b.txt").as_deref(), Some("b1\n"));
    assert_eq!(index_id(&fx, "b.txt"), Some(blob_id("b1\n")));
    assert_eq!(
        git_file(&fx, "refs/stash"),
        Some(format!("{}\n", work.to_hex()))
    );
    assert_eq!(reflog_messages(&fx, "refs/stash"), vec![w.message.clone()]);
    assert_eq!(
        reflog_messages(&fx, "HEAD").last().map(String::as_str),
        Some("reset: moving to HEAD")
    );
}

#[test]
fn push_with_a_message_names_the_branch() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    let request = StashRequest {
        message: Some("work in progress".to_owned()),
        ..StashRequest::new(StashAction::Push)
    };
    let outcome = run(&fx, &journal, &request).unwrap();
    let w = commit(&fx, outcome.created.unwrap());
    assert_eq!(w.message, "On main: work in progress");
}

#[test]
fn push_without_changes_does_nothing() {
    let fx = base();
    let journal = Journal::new();
    let before = state(&fx);
    let outcome = push(&fx, &journal);
    assert!(outcome.nothing_to_do);
    assert_eq!(outcome.created, None);
    assert_eq!(state(&fx), before);
}

#[test]
fn push_with_untracked_files_adds_a_third_parent() {
    let fx = base();
    let journal = Journal::new();
    let head = rev(&fx, "HEAD");
    write(&fx, "new/u.txt", "u\n");
    write(&fx, ".gitignore", "*.log\n");
    write(&fx, "skip.log", "ignored\n");
    let request = StashRequest {
        include_untracked: true,
        ..StashRequest::new(StashAction::Push)
    };
    let outcome = run(&fx, &journal, &request).unwrap();
    let w = commit(&fx, outcome.created.unwrap());
    assert_eq!(w.parents.len(), 3);
    let u = commit(&fx, w.parents[2]);
    assert!(u.parents.is_empty());
    assert_eq!(
        u.message,
        format!("untracked files on main: {} first\n", short(&fx, head))
    );
    assert_eq!(tree_blob(&fx, u.tree, "new/u.txt"), Some(blob_id("u\n")));
    assert_eq!(tree_blob(&fx, u.tree, "skip.log"), None);
    assert_eq!(read(&fx, "new/u.txt"), None);
    assert!(!fx.path().join("new").exists());
    assert_eq!(read(&fx, "skip.log").as_deref(), Some("ignored\n"));
    let entries = list(fx.path(), &ctx()).unwrap();
    assert!(entries[0].includes_untracked);
}

#[test]
fn push_of_paths_saves_and_resets_only_those_paths() {
    let fx = base();
    let journal = Journal::new();
    let head_log = reflog_messages(&fx, "HEAD");
    write(&fx, "a.txt", "a-work\n");
    write(&fx, "dir/c.txt", "c-work\n");
    let request = StashRequest {
        paths: vec![PathBuf::from("dir")],
        ..StashRequest::new(StashAction::Push)
    };
    let outcome = run(&fx, &journal, &request).unwrap();
    let w = commit(&fx, outcome.created.unwrap());
    assert_eq!(
        tree_blob(&fx, w.tree, "dir/c.txt"),
        Some(blob_id("c-work\n"))
    );
    assert_eq!(tree_blob(&fx, w.tree, "a.txt"), Some(blob_id("a1\n")));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a-work\n"));
    assert_eq!(read(&fx, "dir/c.txt").as_deref(), Some("c1\n"));
    assert_eq!(reflog_messages(&fx, "HEAD"), head_log);
}

#[cfg(unix)]
#[test]
fn push_saves_and_removes_an_untracked_symlink_that_points_nowhere() {
    let fx = base();
    let journal = Journal::new();
    std::os::unix::fs::symlink("missing-target", fx.path().join("dangling")).unwrap();
    let request = StashRequest {
        include_untracked: true,
        ..StashRequest::new(StashAction::Push)
    };
    let outcome = run(&fx, &journal, &request).unwrap();
    assert!(outcome.created.is_some());
    assert!(std::fs::symlink_metadata(fx.path().join("dangling")).is_err());
    run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap();
    assert_eq!(
        std::fs::read_link(fx.path().join("dangling")).unwrap(),
        Path::new("missing-target")
    );
}

#[test]
fn push_of_paths_without_changes_there_saves_nothing() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "b.txt", "b-staged\n");
    stage(&fx, "b.txt");
    let before = state(&fx);
    let request = StashRequest {
        paths: vec![PathBuf::from("a.txt")],
        ..StashRequest::new(StashAction::Push)
    };
    let outcome = run(&fx, &journal, &request).unwrap();
    assert!(outcome.nothing_to_do);
    assert_eq!(outcome.created, None);
    assert_eq!(state(&fx), before);
    assert!(list(fx.path(), &ctx()).unwrap().is_empty());
}

#[test]
fn push_of_an_unknown_path_is_rejected() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    let request = StashRequest {
        paths: vec![PathBuf::from("missing.txt")],
        ..StashRequest::new(StashAction::Push)
    };
    let err = run(&fx, &journal, &request).unwrap_err();
    assert_eq!(err.code, ErrorCode::NoMatch);
}

#[test]
fn push_of_a_path_outside_the_repository_is_an_invalid_value() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    let request = StashRequest {
        paths: vec![PathBuf::from("/nurt-outside-the-repository/a.txt")],
        ..StashRequest::new(StashAction::Push)
    };
    let err = run(&fx, &journal, &request).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidValue);
}

#[test]
fn push_counts_and_keeps_excluded_files() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "a-work\n");
    write(&fx, "dir/c.txt", "c-work\n");
    let request = StashRequest {
        exclude: vec!["c.txt".to_owned()],
        ..StashRequest::new(StashAction::Push)
    };
    let outcome = run(&fx, &journal, &request).unwrap();
    assert_eq!(outcome.excluded, 1);
    assert_eq!(read(&fx, "dir/c.txt").as_deref(), Some("c-work\n"));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a1\n"));
    assert!(
        outcome
            .notable()
            .iter()
            .any(|entry| entry.code == Notable::Excluded)
    );
}

#[test]
fn push_on_an_unborn_branch_is_rejected() {
    let fx = RepoBuilder::new().build();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    stage(&fx, "a.txt");
    let err = run(&fx, &journal, &StashRequest::new(StashAction::Push)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitRef);
}

#[test]
fn list_returns_entries_newest_first() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "one\n");
    push(&fx, &journal);
    write(&fx, "a.txt", "two\n");
    let request = StashRequest {
        message: Some("second".to_owned()),
        ..StashRequest::new(StashAction::Push)
    };
    run(&fx, &journal, &request).unwrap();
    let entries = list(fx.path(), &ctx()).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].index, 0);
    assert_eq!(entries[0].message, "On main: second");
    assert_eq!(entries[1].branch.as_deref(), Some("main"));
    assert!(entries[1].message.starts_with("WIP on main: "));
    assert_eq!(entries[0].time, crate::reset::testing::NOW);
}

#[test]
fn apply_restores_changes_unstaged_and_keeps_the_entry() {
    let fx = base();
    let journal = Journal::new();
    dirty(&fx);
    push(&fx, &journal);
    let outcome = run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap();
    assert!(outcome.conflicts.is_empty());
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a-work\n"));
    assert_eq!(read(&fx, "b.txt").as_deref(), Some("b-staged\n"));
    assert_eq!(index_id(&fx, "b.txt"), Some(blob_id("b1\n")));
    assert_eq!(list(fx.path(), &ctx()).unwrap().len(), 1);
}

#[test]
fn apply_keeps_new_files_added() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "added.txt", "new\n");
    stage(&fx, "added.txt");
    push(&fx, &journal);
    assert_eq!(read(&fx, "added.txt"), None);
    run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap();
    assert_eq!(index_id(&fx, "added.txt"), Some(blob_id("new\n")));
    assert_eq!(read(&fx, "added.txt").as_deref(), Some("new\n"));
}

#[test]
fn apply_on_another_commit_merges_cleanly() {
    let fx = RepoBuilder::new()
        .file("a.txt", "1\n2\n3\n4\n5\n")
        .file("b.txt", "b\n")
        .commit("first")
        .build();
    let journal = Journal::new();
    write(&fx, "a.txt", "1\n2\n3\n4\nfive\n");
    push(&fx, &journal);
    write(&fx, "a.txt", "one\n2\n3\n4\n5\n");
    stage(&fx, "a.txt");
    run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap();
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("one\n2\n3\n4\nfive\n"));
}

#[test]
fn a_number_that_names_no_commit_is_a_position() {
    let fx = base();
    let journal = Journal::new();
    dirty(&fx);
    let pushed = push(&fx, &journal);
    let repo = GitRepo::open(fx.path(), &ctx()).unwrap();
    for spec in ["0", "0000", "stash@{0}"] {
        let (position, id) = entries::resolve(&repo, Some(spec)).unwrap();
        assert_eq!(position, Some(0), "{spec}");
        assert_eq!(Some(id), pushed.created, "{spec}");
    }
    assert!(!entries::is_digit_only_hash(&repo, Some("0000")));
    assert!(!entries::is_digit_only_hash(&repo, None));
}

#[test]
fn pop_removes_the_entry_after_a_clean_merge() {
    let fx = base();
    let journal = Journal::new();
    dirty(&fx);
    let pushed = push(&fx, &journal);
    let outcome = run(&fx, &journal, &StashRequest::new(StashAction::Pop)).unwrap();
    assert_eq!(outcome.dropped, pushed.created);
    assert!(list(fx.path(), &ctx()).unwrap().is_empty());
    assert_eq!(git_file(&fx, "refs/stash"), None);
    assert_eq!(git_file(&fx, "logs/refs/stash"), None);
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a-work\n"));
    assert!(git_file(&fx, "AUTO_MERGE").is_some());
}

#[test]
fn pop_with_a_conflict_keeps_the_entry_and_records_the_stages() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "local\n");
    push(&fx, &journal);
    write(&fx, "a.txt", "other\n");
    stage(&fx, "a.txt");
    let outcome = run(&fx, &journal, &StashRequest::new(StashAction::Pop)).unwrap();
    assert!(outcome.kept);
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(outcome.conflicts[0].path, "a.txt");
    assert_eq!(list(fx.path(), &ctx()).unwrap().len(), 1);
    assert_eq!(
        read(&fx, "a.txt").as_deref(),
        Some("<<<<<<< Updated upstream\nother\n=======\nlocal\n>>>>>>> Stashed changes\n")
    );
    let stages: Vec<u32> = index_entries(&fx)
        .into_iter()
        .filter(|(path, _, _)| path == "a.txt")
        .map(|(_, stage, _)| stage)
        .collect();
    assert_eq!(stages, vec![1, 2, 3]);
    assert!(git_file(&fx, "AUTO_MERGE").is_some());
    let err = outcome.conflict_error();
    assert_eq!(err.code, ErrorCode::GitConflict);
    assert!(err.effect_applied);
    assert!(
        outcome
            .notable()
            .iter()
            .any(|entry| entry.code == Notable::StashKept)
    );
}

#[test]
fn local_changes_in_the_way_refuse_the_apply() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "stashed\n");
    push(&fx, &journal);
    write(&fx, "a.txt", "unsaved\n");
    let before = state(&fx);
    let err = run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitDirty);
    assert_eq!(state(&fx), before);
}

#[test]
fn apply_restores_untracked_files_and_refuses_existing_ones() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "u/new.txt", "u\n");
    let request = StashRequest {
        include_untracked: true,
        ..StashRequest::new(StashAction::Push)
    };
    run(&fx, &journal, &request).unwrap();
    run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap();
    assert_eq!(read(&fx, "u/new.txt").as_deref(), Some("u\n"));
    assert_eq!(index_id(&fx, "u/new.txt"), None);
    let before = state(&fx);
    let err = run(&fx, &journal, &StashRequest::new(StashAction::Apply)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitDirty);
    assert_eq!(err.problem.as_deref(), Some("untracked_overwritten"));
    assert_eq!(state(&fx), before);
}

#[test]
fn pop_of_a_commit_outside_the_list_is_rejected() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    let created = push(&fx, &journal).created.unwrap();
    let err = run(
        &fx,
        &journal,
        &on_entry(StashAction::Pop, &created.to_hex().to_string()),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::GitRef);
}

fn three_entries(fx: &Fixture, journal: &Journal) {
    for text in ["one", "two", "three"] {
        write(fx, "a.txt", &format!("{text}\n"));
        let request = StashRequest {
            message: Some(text.to_owned()),
            ..StashRequest::new(StashAction::Push)
        };
        run(fx, journal, &request).unwrap();
    }
}

#[test]
fn drop_of_a_middle_entry_rewrites_the_reflog() {
    let fx = base();
    let journal = Journal::new();
    three_entries(&fx, &journal);
    let newest = git_file(&fx, "refs/stash");
    let outcome = run(&fx, &journal, &on_entry(StashAction::Drop, "stash@{1}")).unwrap();
    assert!(outcome.dropped.is_some());
    let messages: Vec<String> = list(fx.path(), &ctx())
        .unwrap()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(messages, vec!["On main: three", "On main: one"]);
    assert_eq!(git_file(&fx, "refs/stash"), newest);
    let log = git_file(&fx, "logs/refs/stash").unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    let first_new = lines[0].split(' ').nth(1).unwrap_or_default();
    let second_old = lines[1].split(' ').next().unwrap_or_default();
    assert_eq!(first_new, second_old);
}

#[test]
fn drop_of_the_newest_entry_moves_the_reference() {
    let fx = base();
    let journal = Journal::new();
    three_entries(&fx, &journal);
    let second = list(fx.path(), &ctx()).unwrap()[1].id;
    run(&fx, &journal, &StashRequest::new(StashAction::Drop)).unwrap();
    assert_eq!(
        git_file(&fx, "refs/stash"),
        Some(format!("{}\n", second.to_hex()))
    );
}

#[test]
fn drop_of_a_missing_entry_is_rejected() {
    let fx = base();
    let journal = Journal::new();
    let err = run(&fx, &journal, &StashRequest::new(StashAction::Drop)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitRef);
    write(&fx, "a.txt", "x\n");
    push(&fx, &journal);
    let err = run(&fx, &journal, &on_entry(StashAction::Drop, "stash@{5}")).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitRevision);
}

#[test]
fn an_entry_number_beyond_the_range_of_integers_does_not_exist() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    push(&fx, &journal);
    let name = format!("stash@{{{}}}", usize::MAX);
    let err = run(&fx, &journal, &on_entry(StashAction::Drop, &name)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitRevision);
    assert_eq!(list(fx.path(), &ctx()).unwrap().len(), 1);
}

#[test]
fn clear_removes_the_reference_and_its_reflog() {
    let fx = base();
    let journal = Journal::new();
    three_entries(&fx, &journal);
    let outcome = run(&fx, &journal, &StashRequest::new(StashAction::Clear)).unwrap();
    assert_eq!(outcome.cleared, 3);
    assert_eq!(git_file(&fx, "refs/stash"), None);
    assert_eq!(git_file(&fx, "logs/refs/stash"), None);
    let again = run(&fx, &journal, &StashRequest::new(StashAction::Clear)).unwrap();
    assert!(again.nothing_to_do);
}

#[test]
fn undo_restores_every_changing_action_byte_for_byte() {
    let requests = [
        StashRequest::new(StashAction::Push),
        StashRequest::new(StashAction::Apply),
        StashRequest::new(StashAction::Pop),
        StashRequest::new(StashAction::Drop),
        StashRequest::new(StashAction::Clear),
    ];
    for request in requests {
        let fx = base();
        let journal = Journal::new();
        dirty(&fx);
        if request.action != StashAction::Push {
            push(&fx, &journal);
            write(&fx, "dir/c.txt", "c-later\n");
        }
        let before = state(&fx);
        run(&fx, &journal, &request).unwrap();
        assert_ne!(state(&fx), before, "{:?} changed nothing", request.action);
        journal.undo_last();
        let after = state(&fx);
        let differing: Vec<_> = before
            .keys()
            .chain(after.keys())
            .filter(|path| before.get(*path) != after.get(*path))
            .collect();
        assert!(
            differing.is_empty(),
            "undo of {:?}: {differing:?}",
            request.action
        );
    }
}

#[test]
fn a_stale_state_token_stops_the_operation() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    let token = state_token(fx.path()).unwrap();
    let request = StashRequest {
        expect_state: Some(token.clone()),
        ..StashRequest::new(StashAction::Push)
    };
    run(&fx, &journal, &request).unwrap();
    write(&fx, "a.txt", "y\n");
    let request = StashRequest {
        expect_state: Some(token),
        ..StashRequest::new(StashAction::Push)
    };
    let err = run(&fx, &journal, &request).unwrap_err();
    assert_eq!(err.code, ErrorCode::Stale);
}

#[test]
fn show_lists_the_files_of_an_entry() {
    let fx = base();
    let journal = Journal::new();
    write(&fx, "a.txt", "x\n");
    remove(&fx, "b.txt");
    write(&fx, "added.txt", "n\n");
    stage(&fx, "added.txt");
    push(&fx, &journal);
    let files = show(fx.path(), None, &ctx()).unwrap();
    let summary: Vec<(String, StashFileChange)> = files
        .into_iter()
        .map(|file| (file.path, file.change))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("a.txt".to_owned(), StashFileChange::Modified),
            ("added.txt".to_owned(), StashFileChange::Added),
            ("b.txt".to_owned(), StashFileChange::Deleted),
        ]
    );
}

#[test]
fn actions_parse_from_their_names() {
    for name in ACTIONS {
        assert_eq!(name.parse::<StashAction>().unwrap().as_str(), name);
    }
    assert_eq!(
        "stash".parse::<StashAction>().unwrap_err().code,
        ErrorCode::UnknownValue
    );
}

fn args(fx: &Fixture, action: &str) -> BoundArgs {
    BoundArgs::new("git.stash")
        .with(
            "repo",
            ArgValue::Value(Value::Path(fx.path().to_string_lossy().into_owned())),
            false,
        )
        .with(
            "action",
            ArgValue::Value(Value::Enum(action.to_owned())),
            false,
        )
}

#[test]
fn the_stage_pushes_and_lists_entries() {
    let fx = base();
    let journal = Journal::new();
    let ctx = ctx();
    let command = StashCommand;
    assert_eq!(command.descriptor().command, "git.stash");
    assert!(command.as_read().is_some() && command.as_effect().is_some());
    write(&fx, "a.txt", "x\n");
    let push_args = args(&fx, "push");
    let plan = command.plan(&push_args, StageInput::none(), &ctx).unwrap();
    assert_eq!(plan.changes.len(), 1);
    journal
        .run(fx.path(), |tx| command.apply(&push_args, plan, tx, &ctx))
        .unwrap();
    let list_args = args(&fx, "list");
    assert!(
        command
            .execute(&list_args, StageInput::none(), &ctx)
            .is_ok()
    );
    assert_eq!(
        command
            .plan(&list_args, StageInput::none(), &ctx)
            .unwrap_err()
            .code,
        ErrorCode::Internal
    );
    assert_eq!(
        command
            .execute(&push_args, StageInput::none(), &ctx)
            .err()
            .map(|err| err.code),
        Some(ErrorCode::Internal)
    );
    let records = read_records(fx.path(), &StashRequest::new(StashAction::List), &ctx).unwrap();
    assert_eq!(records.len(), 1);
    let shown = read_records(fx.path(), &StashRequest::new(StashAction::Show), &ctx).unwrap();
    assert_eq!(shown.len(), 1);
}

#[test]
fn abbreviated_hash_of_digits_only_is_a_hash_and_not_a_position() {
    for attempt in 0..400 {
        let fx = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit(&format!("c{attempt}"))
            .build();
        let repo = GitRepo::open(fx.path(), &ctx()).unwrap();
        let id = repo.head_id().unwrap().unwrap();
        let hex = id.to_hex_with_len(4).to_string();
        if hex.bytes().all(|byte| byte.is_ascii_digit()) {
            assert!(entries::is_digit_only_hash(&repo, Some(&hex)));
            assert!(!entries::is_digit_only_hash(&repo, Some("0")));
            assert!(!entries::is_digit_only_hash(&repo, None));
            return;
        }
    }
    panic!("no commit with an abbreviation of digits only was found");
}
