// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of `git.reset`.

use nurt_contracts::stage::{ArgValue, BoundArgs, EffectStage, StageImpl, StageInput};
use nurt_contracts::value::Value;

use super::testing::{
    Fixture, Journal, RepoBuilder, blob_id, ctx, git_file, index_id, read, reflog_messages, rev,
    state, two_commits, write,
};
use super::*;

fn reset(fx: &Fixture, journal: &Journal, revision: &str, mode: ResetMode) -> Result<ResetOutcome> {
    let request = ResetRequest {
        revision: revision.to_owned(),
        mode,
    };
    let planned = plan_reset(fx.path(), &request, &ctx())?;
    journal.run(fx.path(), |tx| planned.apply(tx))
}

fn hex_line(id: ObjectId) -> String {
    format!("{}\n", id.to_hex())
}

#[test]
fn soft_reset_moves_only_the_branch() {
    let fx = two_commits();
    let journal = Journal::new();
    let first = rev(&fx, "HEAD~1");
    let second = rev(&fx, "HEAD");
    let outcome = reset(&fx, &journal, "HEAD~1", ResetMode::Soft).unwrap();
    assert_eq!(git_file(&fx, "refs/heads/main"), Some(hex_line(first)));
    assert_eq!(git_file(&fx, "ORIG_HEAD"), Some(hex_line(second)));
    assert_eq!(index_id(&fx, "new.txt"), Some(blob_id("n2\n")));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a2\n"));
    assert_eq!(outcome.head_before, Some(second));
    assert_eq!(outcome.head_after, Some(first));
    assert_eq!(outcome.branch.as_deref(), Some("main"));
    assert!(outcome.history_rewritten);
    assert!(outcome.written.is_empty() && outcome.removed.is_empty());
}

#[test]
fn reflogs_of_branch_and_head_name_the_revision() {
    let fx = two_commits();
    let journal = Journal::new();
    reset(&fx, &journal, "HEAD~1", ResetMode::Mixed).unwrap();
    let expected = "reset: moving to HEAD~1".to_owned();
    assert_eq!(reflog_messages(&fx, "HEAD").last(), Some(&expected));
    assert_eq!(
        reflog_messages(&fx, "refs/heads/main").last(),
        Some(&expected)
    );
    let log = git_file(&fx, "logs/HEAD").unwrap_or_default();
    let last = log.lines().last().unwrap_or_default();
    assert!(last.contains("Grace Hopper <grace@example.com>"), "{last}");
}

#[test]
fn reset_to_the_current_commit_logs_head_but_not_the_branch() {
    let fx = two_commits();
    let journal = Journal::new();
    let branch_log = reflog_messages(&fx, "refs/heads/main");
    reset(&fx, &journal, "HEAD", ResetMode::Hard).unwrap();
    assert_eq!(reflog_messages(&fx, "refs/heads/main"), branch_log);
    assert_eq!(
        reflog_messages(&fx, "HEAD").last().map(String::as_str),
        Some("reset: moving to HEAD")
    );
}

#[test]
fn mixed_reset_replaces_the_index_and_keeps_the_working_tree() {
    let fx = two_commits();
    let journal = Journal::new();
    reset(&fx, &journal, "HEAD~1", ResetMode::Mixed).unwrap();
    assert_eq!(index_id(&fx, "new.txt"), None);
    assert_eq!(index_id(&fx, "a.txt"), Some(blob_id("a1\n")));
    assert_eq!(index_id(&fx, "b.txt"), Some(blob_id("b1\n")));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a2\n"));
    assert_eq!(read(&fx, "new.txt").as_deref(), Some("n2\n"));
    assert_eq!(read(&fx, "b.txt"), None);
}

#[test]
fn hard_reset_brings_the_working_tree_to_the_revision() {
    let fx = two_commits();
    let journal = Journal::new();
    let outcome = reset(&fx, &journal, "HEAD~1", ResetMode::Hard).unwrap();
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a1\n"));
    assert_eq!(read(&fx, "b.txt").as_deref(), Some("b1\n"));
    assert_eq!(read(&fx, "new.txt"), None);
    assert_eq!(index_id(&fx, "b.txt"), Some(blob_id("b1\n")));
    assert_eq!(outcome.written, vec!["a.txt", "b.txt"]);
    assert_eq!(outcome.removed, vec!["new.txt"]);
}

#[test]
fn hard_reset_keeps_untracked_files_and_drops_local_changes() {
    let fx = two_commits();
    let journal = Journal::new();
    write(&fx, "untracked.txt", "u\n");
    write(&fx, "a.txt", "local\n");
    reset(&fx, &journal, "HEAD", ResetMode::Hard).unwrap();
    assert_eq!(read(&fx, "untracked.txt").as_deref(), Some("u\n"));
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a2\n"));
}

#[test]
fn hard_reset_overwrites_a_colliding_untracked_file() {
    let fx = two_commits();
    let journal = Journal::new();
    write(&fx, "b.txt", "local\n");
    reset(&fx, &journal, "HEAD~1", ResetMode::Hard).unwrap();
    assert_eq!(read(&fx, "b.txt").as_deref(), Some("b1\n"));
}

#[test]
fn hard_reset_removes_directories_left_empty() {
    let fx = RepoBuilder::new()
        .file("a.txt", "a\n")
        .commit("first")
        .file("deep/er/x.txt", "x\n")
        .commit("second")
        .build();
    let journal = Journal::new();
    reset(&fx, &journal, "HEAD~1", ResetMode::Hard).unwrap();
    assert!(!fx.path().join("deep").exists());
}

#[test]
fn moving_forward_is_not_a_history_rewrite() {
    let fx = two_commits();
    let journal = Journal::new();
    let second = rev(&fx, "HEAD").to_hex().to_string();
    reset(&fx, &journal, "HEAD~1", ResetMode::Hard).unwrap();
    let outcome = reset(&fx, &journal, &second, ResetMode::Hard).unwrap();
    assert!(!outcome.history_rewritten);
    assert_eq!(read(&fx, "new.txt").as_deref(), Some("n2\n"));
}

#[test]
fn reset_removes_the_state_files_of_a_merge() {
    let fx = two_commits();
    let journal = Journal::new();
    write(
        &fx,
        ".git/MERGE_HEAD",
        "1111111111111111111111111111111111111111\n",
    );
    write(&fx, ".git/MERGE_MSG", "Merge branch 'x'\n");
    let outcome = reset(&fx, &journal, "HEAD", ResetMode::Mixed).unwrap();
    assert_eq!(outcome.state_removed, vec!["MERGE_HEAD", "MERGE_MSG"]);
    assert_eq!(git_file(&fx, "MERGE_HEAD"), None);
    assert_eq!(git_file(&fx, "MERGE_MSG"), None);
}

#[test]
fn soft_reset_is_refused_during_a_merge() {
    let fx = two_commits();
    let journal = Journal::new();
    write(
        &fx,
        ".git/MERGE_HEAD",
        "1111111111111111111111111111111111111111\n",
    );
    let err = reset(&fx, &journal, "HEAD~1", ResetMode::Soft).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitInProgress);
    assert!(err.fix.iter().any(|fix| fix.contains("mode: mixed")));
}

#[test]
fn undo_restores_every_mode_byte_for_byte() {
    for mode in [ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard] {
        let fx = two_commits();
        let journal = Journal::new();
        write(&fx, "untracked.txt", "u\n");
        let before = state(&fx);
        reset(&fx, &journal, "HEAD~1", mode).unwrap();
        assert_ne!(state(&fx), before, "{mode:?} changed nothing");
        journal.undo_last();
        assert_eq!(state(&fx), before, "undo of {mode:?}");
    }
}

#[test]
fn an_existing_index_lock_is_refused() {
    let fx = two_commits();
    let journal = Journal::new();
    write(&fx, ".git/index.lock", "");
    let err = reset(&fx, &journal, "HEAD~1", ResetMode::Mixed).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitLocked);
}

#[test]
fn a_lock_created_after_the_plan_stops_the_reset() {
    let fx = two_commits();
    let journal = Journal::new();
    let request = ResetRequest {
        revision: "HEAD~1".to_owned(),
        mode: ResetMode::Hard,
    };
    let planned = plan_reset(fx.path(), &request, &ctx()).unwrap();
    write(&fx, ".git/refs/heads/main.lock", "");
    let before = state(&fx);
    let err = journal.run(fx.path(), |tx| planned.apply(tx)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitLocked);
    assert_eq!(state(&fx), before);
}

#[test]
fn an_unknown_revision_is_rejected() {
    let fx = two_commits();
    let journal = Journal::new();
    let err = reset(&fx, &journal, "no-such-branch", ResetMode::Hard).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitRevision);
}

#[test]
fn a_change_after_the_plan_rolls_the_reset_back() {
    let fx = two_commits();
    let journal = Journal::new();
    let request = ResetRequest {
        revision: "HEAD~1".to_owned(),
        mode: ResetMode::Hard,
    };
    let planned = plan_reset(fx.path(), &request, &ctx()).unwrap();
    write(&fx, "a.txt", "changed meanwhile\n");
    let before = state(&fx);
    let err = journal.run(fx.path(), |tx| planned.apply(tx)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Stale);
    assert_eq!(state(&fx), before);
}

#[test]
fn detached_head_is_moved_in_place() {
    let fx = two_commits();
    let journal = Journal::new();
    let first = rev(&fx, "HEAD~1");
    let second = rev(&fx, "HEAD");
    write(&fx, ".git/HEAD", &hex_line(second));
    let outcome = reset(&fx, &journal, "HEAD~1", ResetMode::Hard).unwrap();
    assert_eq!(git_file(&fx, "HEAD"), Some(hex_line(first)));
    assert_eq!(git_file(&fx, "refs/heads/main"), Some(hex_line(second)));
    assert_eq!(outcome.branch, None);
    assert!(!outcome.history_rewritten);
    assert!(
        outcome
            .notable()
            .iter()
            .any(|entry| entry.code == Notable::DetachedHead)
    );
}

#[cfg(unix)]
#[test]
fn hard_reset_restores_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let fx = RepoBuilder::new()
        .executable("run.sh", "echo\n")
        .commit("first")
        .file("run.sh", "echo\n")
        .commit("second")
        .build();
    let journal = Journal::new();
    std::fs::set_permissions(
        fx.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    reset(&fx, &journal, "HEAD~1", ResetMode::Hard).unwrap();
    let mode = std::fs::metadata(fx.path().join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0);
}

#[test]
fn modes_parse_from_their_names() {
    for mode in [ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard] {
        assert_eq!(mode.as_str().parse::<ResetMode>().unwrap(), mode);
    }
    let err = "bogus".parse::<ResetMode>().unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownValue);
}

#[test]
fn reset_outside_a_repository_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let request = ResetRequest {
        revision: "HEAD".to_owned(),
        mode: ResetMode::Hard,
    };
    let err = plan_reset(dir.path(), &request, &ctx()).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitNotRepository);
}

fn args(fx: &Fixture, revision: &str, mode: &str) -> BoundArgs {
    BoundArgs::new("git.reset")
        .with(
            "repo",
            ArgValue::Value(Value::Path(fx.path().to_string_lossy().into_owned())),
            false,
        )
        .with(
            "revision",
            ArgValue::Value(Value::Text(revision.to_owned())),
            false,
        )
        .with("mode", ArgValue::Value(Value::Enum(mode.to_owned())), false)
}

#[test]
fn the_stage_plans_and_applies_a_reset_with_an_effect_record() {
    let fx = two_commits();
    let journal = Journal::new();
    let ctx = ctx();
    let command = ResetCommand;
    assert_eq!(command.descriptor().command, "git.reset");
    assert!(command.as_effect().is_some());
    let args = args(&fx, "HEAD~1", "hard");
    let plan = command.plan(&args, StageInput::none(), &ctx).unwrap();
    assert!(plan.changes.len() >= 3);
    assert!(plan.bulk);
    let outcome = journal
        .run(fx.path(), |tx| command.apply(&args, plan, tx, &ctx))
        .unwrap();
    assert!(
        outcome
            .notable
            .iter()
            .any(|entry| entry.code == Notable::HistoryRewritten)
    );
    assert_eq!(read(&fx, "a.txt").as_deref(), Some("a1\n"));
    assert!(
        journal
            .last_targets()
            .iter()
            .any(|path| path.ends_with("ORIG_HEAD"))
    );
}

#[test]
fn the_stage_rejects_an_unknown_mode() {
    let fx = two_commits();
    let ctx = ctx();
    let err = ResetCommand
        .plan(&args(&fx, "HEAD", "medium"), StageInput::none(), &ctx)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownValue);
}
