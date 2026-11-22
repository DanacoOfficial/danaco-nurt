// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Unit tests of the working tree update engine on repositories built in
//! temporary directories with `gix`.

use std::path::Path;

use bstr::{BString, ByteSlice};
use gix::object::tree::EntryKind as TreeKind;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::MutationTx;

use super::testjournal::{RecordingJournal, Snapshot, assert_declared, tree_state};
use super::*;

/// One file of a test tree: path, kind and content.
type Spec<'a> = (&'a str, TreeKind, &'a str);

fn init() -> (tempfile::TempDir, gix::Repository) {
    let dir = tempfile::tempdir().expect("temporary directory");
    let repo = gix::ThreadSafeRepository::init_opts(
        dir.path(),
        gix::create::Kind::WithWorktree,
        gix::create::Options::default(),
        gix::open::Options::isolated(),
    )
    .expect("init")
    .to_thread_local();
    (dir, repo)
}

fn tree(repo: &gix::Repository, files: &[Spec<'_>]) -> gix::ObjectId {
    let mut editor = repo.edit_tree(repo.empty_tree().id).expect("editor");
    for (path, kind, content) in files {
        let blob = repo.write_blob(content.as_bytes()).expect("blob").detach();
        editor.upsert(*path, *kind, blob).expect("upsert");
    }
    editor.write().expect("write tree").detach()
}

fn run(repo: &gix::Repository, journal: &RecordingJournal, plan: ApplyPlan) -> ApplyReport {
    let root = repo.workdir().expect("workdir").to_path_buf();
    let before = tree_state(&root);
    let ctx = ApplyContext::open(repo).expect("context");
    let mut tx = journal.begin();
    let report = apply_plan(&mut tx, &ctx, plan).expect("apply");
    let handle = Box::new(tx).commit().expect("commit");
    if handle.op.is_some() {
        assert_declared(&root, &before, &tree_state(&root), journal);
    }
    report
}

fn checkout(repo: &gix::Repository, journal: &RecordingJournal, tree: gix::ObjectId) {
    let ctx = ApplyContext::open(repo).expect("context");
    let plan = plan_new_tree(&ctx, &tree).expect("plan");
    run(repo, journal, plan);
}

fn switch(
    repo: &gix::Repository,
    journal: &RecordingJournal,
    from: gix::ObjectId,
    to: gix::ObjectId,
) -> ApplyReport {
    let ctx = ApplyContext::open(repo).expect("context");
    let plan = plan_two_way(&ctx, Some(&from), &to).expect("plan");
    run(repo, journal, plan)
}

fn two_way_error(
    repo: &gix::Repository,
    from: gix::ObjectId,
    to: gix::ObjectId,
) -> nurt_contracts::error::Error {
    let ctx = ApplyContext::open(repo).expect("context");
    plan_two_way(&ctx, Some(&from), &to).expect_err("refused")
}

fn read(repo: &gix::Repository, path: &str) -> String {
    std::fs::read_to_string(repo.workdir().expect("workdir").join(path)).expect("read")
}

fn write(repo: &gix::Repository, path: &str, content: &str) {
    let full = repo.workdir().expect("workdir").join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(full, content).expect("write");
}

fn exists(repo: &gix::Repository, path: &str) -> bool {
    std::fs::symlink_metadata(repo.workdir().expect("workdir").join(path)).is_ok()
}

fn index_paths(repo: &gix::Repository) -> Vec<String> {
    let index = repo.open_index().expect("index");
    index
        .entries()
        .iter()
        .map(|entry| entry.path(&index).to_string())
        .collect()
}

fn detail(error: &nurt_contracts::error::Error, name: &str) -> Vec<String> {
    error
        .data()
        .details
        .get(name)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

const F: TreeKind = TreeKind::Blob;

#[test]
fn new_tree_writes_files_and_index() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let target = tree(
        &repo,
        &[
            ("a.txt", F, "a\n"),
            ("dir/b.txt", F, "b\n"),
            ("run.sh", TreeKind::BlobExecutable, "#!/bin/sh\n"),
        ],
    );
    checkout(&repo, &journal, target);
    assert_eq!(read(&repo, "a.txt"), "a\n");
    assert_eq!(read(&repo, "dir/b.txt"), "b\n");
    assert_eq!(index_paths(&repo), ["a.txt", "dir/b.txt", "run.sh"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(repo.workdir().expect("workdir").join("run.sh"))
            .expect("meta")
            .permissions()
            .mode();
        assert_ne!(mode & 0o100, 0, "executable bit");
    }
    let ctx = ApplyContext::open(&repo).expect("context");
    let again = plan_two_way(&ctx, Some(&target), &target).expect("plan");
    assert!(again.entries.is_empty(), "a fresh checkout is clean");
    assert!(
        again.carried.is_empty(),
        "statistics are fresh: {:?}",
        again.carried
    );
}

#[test]
fn two_way_updates_changed_paths_and_removes_deleted_ones() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(
        &repo,
        &[
            ("keep.txt", F, "k\n"),
            ("change.txt", F, "1\n"),
            ("gone/x.txt", F, "x\n"),
        ],
    );
    let new = tree(
        &repo,
        &[
            ("keep.txt", F, "k\n"),
            ("change.txt", F, "2\n"),
            ("added.txt", F, "n\n"),
        ],
    );
    checkout(&repo, &journal, old);
    let report = switch(&repo, &journal, old, new);
    assert_eq!(read(&repo, "change.txt"), "2\n");
    assert_eq!(read(&repo, "added.txt"), "n\n");
    assert!(!exists(&repo, "gone"), "emptied directory removed");
    assert!(report.index_written);
    assert_eq!(index_paths(&repo), ["added.txt", "change.txt", "keep.txt"]);
}

#[test]
fn two_way_keeps_local_change_of_path_equal_in_both_trees() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("same.txt", F, "s\n"), ("other.txt", F, "1\n")]);
    let new = tree(&repo, &[("same.txt", F, "s\n"), ("other.txt", F, "2\n")]);
    checkout(&repo, &journal, old);
    write(&repo, "same.txt", "local\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_two_way(&ctx, Some(&old), &new).expect("plan");
    assert_eq!(plan.carried, [BString::from("same.txt")]);
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "same.txt"), "local\n");
    assert_eq!(read(&repo, "other.txt"), "2\n");
}

#[test]
fn two_way_refuses_local_change_of_path_that_differs() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("f.txt", F, "1\n")]);
    let new = tree(&repo, &[("f.txt", F, "2\n")]);
    checkout(&repo, &journal, old);
    write(&repo, "f.txt", "2\n");
    let error = two_way_error(&repo, old, new);
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(detail(&error, "local_changes"), ["f.txt"]);
    assert_eq!(journal.len(), 1, "a refused update records nothing");
}

#[test]
fn two_way_treats_missing_tracked_file_as_up_to_date() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("f.txt", F, "1\n")]);
    let new = tree(&repo, &[("f.txt", F, "2\n")]);
    checkout(&repo, &journal, old);
    std::fs::remove_file(repo.workdir().expect("workdir").join("f.txt")).expect("remove");
    switch(&repo, &journal, old, new);
    assert_eq!(read(&repo, "f.txt"), "2\n");
}

#[test]
fn two_way_refuses_untracked_file_in_the_way_and_overwrites_ignored_one() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[(".gitignore", F, "*.log\n")]);
    let new = tree(
        &repo,
        &[
            (".gitignore", F, "*.log\n"),
            ("new.txt", F, "n\n"),
            ("build.log", F, "tracked\n"),
        ],
    );
    checkout(&repo, &journal, old);
    write(&repo, "new.txt", "n\n");
    write(&repo, "build.log", "local log\n");
    let error = two_way_error(&repo, old, new);
    assert_eq!(detail(&error, "untracked_overwritten"), ["new.txt"]);
    std::fs::remove_file(repo.workdir().expect("workdir").join("new.txt")).expect("remove");
    switch(&repo, &journal, old, new);
    assert_eq!(read(&repo, "build.log"), "tracked\n");
}

#[test]
fn two_way_refuses_directory_with_untracked_files() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("d/x.txt", F, "x\n")]);
    let new = tree(&repo, &[("d", F, "file\n")]);
    checkout(&repo, &journal, old);
    write(&repo, "d/untracked.txt", "u\n");
    let error = two_way_error(&repo, old, new);
    assert_eq!(detail(&error, "dir_loses_untracked"), ["d"]);
}

#[test]
fn two_way_replaces_file_by_directory_and_back() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let file = tree(&repo, &[("p", F, "file\n")]);
    let dir = tree(
        &repo,
        &[
            ("p/inner.txt", F, "inner\n"),
            ("p/sub/deep.txt", F, "deep\n"),
        ],
    );
    checkout(&repo, &journal, file);
    switch(&repo, &journal, file, dir);
    assert_eq!(read(&repo, "p/sub/deep.txt"), "deep\n");
    switch(&repo, &journal, dir, file);
    assert_eq!(read(&repo, "p"), "file\n");
    assert_eq!(index_paths(&repo), ["p"]);
}

#[test]
fn two_way_reports_staged_deletion_as_local_change() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("f.txt", F, "1\n"), ("g.txt", F, "g\n")]);
    let new = tree(&repo, &[("f.txt", F, "2\n"), ("g.txt", F, "g\n")]);
    checkout(&repo, &journal, old);
    let only_g = tree(&repo, &[("g.txt", F, "g\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Tree(only_g),
            pathspec: Vec::new(),
            to_index: true,
            to_worktree: false,
        },
    )
    .expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(index_paths(&repo), ["g.txt"]);
    let error = two_way_error(&repo, old, new);
    assert_eq!(detail(&error, "local_changes"), ["f.txt"]);
}

#[cfg(unix)]
#[test]
fn two_way_changes_only_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let plain = tree(&repo, &[("tool", F, "echo\n")]);
    let executable = tree(&repo, &[("tool", TreeKind::BlobExecutable, "echo\n")]);
    checkout(&repo, &journal, plain);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_two_way(&ctx, Some(&plain), &executable).expect("plan");
    assert_eq!(plan.entries[0].action, EntryAction::Chmod);
    run(&repo, &journal, plan);
    let mode = std::fs::metadata(repo.workdir().expect("workdir").join("tool"))
        .expect("meta")
        .permissions()
        .mode();
    assert_ne!(mode & 0o100, 0);
}

#[cfg(unix)]
#[test]
fn two_way_writes_symbolic_links() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("target.txt", F, "t\n")]);
    let new = tree(
        &repo,
        &[
            ("target.txt", F, "t\n"),
            ("link", TreeKind::Link, "target.txt"),
        ],
    );
    checkout(&repo, &journal, old);
    switch(&repo, &journal, old, new);
    let link = std::fs::read_link(repo.workdir().expect("workdir").join("link")).expect("link");
    assert_eq!(link, Path::new("target.txt"));
}

#[test]
fn undo_restores_working_tree_and_index_bytewise() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(
        &repo,
        &[
            ("a.txt", F, "1\n"),
            ("d/b.txt", F, "b\n"),
            ("p", F, "file\n"),
        ],
    );
    let new = tree(
        &repo,
        &[("a.txt", F, "2\n"), ("c.txt", F, "c\n"), ("p/x", F, "x\n")],
    );
    checkout(&repo, &journal, old);
    write(&repo, "untracked.txt", "u\n");
    let root = repo.workdir().expect("workdir").to_path_buf();
    let before = tree_state(&root);
    switch(&repo, &journal, old, new);
    assert_ne!(tree_state(&root), before);
    journal.undo_last();
    assert_eq!(tree_state(&root), before);
}

#[test]
fn reset_discards_local_changes_and_keeps_unrelated_untracked_files() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("a.txt", F, "a\n"), ("b.txt", F, "b\n")]);
    checkout(&repo, &journal, base);
    write(&repo, "a.txt", "changed\n");
    write(&repo, "free.txt", "untracked\n");
    let target = tree(&repo, &[("a.txt", F, "a\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_reset(
        &ctx,
        &target,
        ResetOptions {
            overwrite_untracked: true,
        },
    )
    .expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "a.txt"), "a\n");
    assert!(!exists(&repo, "b.txt"));
    assert_eq!(read(&repo, "free.txt"), "untracked\n");
    assert_eq!(index_paths(&repo), ["a.txt"]);
}

#[test]
fn reset_protecting_untracked_files_refuses_file_in_the_way() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("a.txt", F, "a\n")]);
    checkout(&repo, &journal, base);
    write(&repo, "n.txt", "local\n");
    let target = tree(&repo, &[("a.txt", F, "a\n"), ("n.txt", F, "new\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let error = plan_reset(&ctx, &target, ResetOptions::default()).expect_err("refused");
    assert_eq!(detail(&error, "untracked_overwritten"), ["n.txt"]);
    let plan = plan_reset(
        &ctx,
        &target,
        ResetOptions {
            overwrite_untracked: true,
        },
    )
    .expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "n.txt"), "new\n");
}

#[test]
fn paths_restore_working_tree_from_index() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("a.txt", F, "a\n"), ("b.txt", F, "b\n")]);
    checkout(&repo, &journal, base);
    write(&repo, "a.txt", "changed\n");
    std::fs::remove_file(repo.workdir().expect("workdir").join("b.txt")).expect("remove");
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Index,
            pathspec: vec!["*.txt".into()],
            to_index: false,
            to_worktree: true,
        },
    )
    .expect("plan");
    assert_eq!(plan.entries.len(), 2);
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "a.txt"), "a\n");
    assert_eq!(read(&repo, "b.txt"), "b\n");
}

#[test]
fn paths_restore_from_tree_removes_paths_absent_from_source() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("d/a.txt", F, "new\n"), ("d/c.txt", F, "c\n")]);
    checkout(&repo, &journal, base);
    let source = tree(&repo, &[("d/a.txt", F, "old\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Tree(source),
            pathspec: vec!["d".into()],
            to_index: false,
            to_worktree: true,
        },
    )
    .expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "d/a.txt"), "old\n");
    assert!(!exists(&repo, "d/c.txt"));
    assert_eq!(
        index_paths(&repo),
        ["d/a.txt", "d/c.txt"],
        "index unchanged"
    );
}

#[test]
fn paths_restore_index_only_keeps_working_tree() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("a.txt", F, "new\n")]);
    checkout(&repo, &journal, base);
    let source = tree(&repo, &[("a.txt", F, "old\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Tree(source),
            pathspec: vec!["a.txt".into()],
            to_index: true,
            to_worktree: false,
        },
    )
    .expect("plan");
    assert!(plan.entries.is_empty());
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "a.txt"), "new\n");
    let index = repo.open_index().expect("index");
    let old_blob = repo.write_blob(b"old\n").expect("blob").detach();
    assert_eq!(index.entries()[0].id, old_blob);
}

#[test]
fn paths_with_unknown_pathspec_fail_with_no_match() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    checkout(&repo, &journal, tree(&repo, &[("a.txt", F, "a\n")]));
    let ctx = ApplyContext::open(&repo).expect("context");
    let error = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Index,
            pathspec: vec!["missing.txt".into()],
            to_index: false,
            to_worktree: true,
        },
    )
    .expect_err("no match");
    assert_eq!(error.code(), ErrorCode::NoMatch);
}

#[test]
fn open_refuses_existing_index_lock() {
    let (_dir, repo) = init();
    std::fs::write(repo.git_dir().join("index.lock"), b"").expect("lock");
    let error = ApplyContext::open(&repo).err().expect("locked");
    assert_eq!(error.code(), ErrorCode::GitLocked);
}

#[test]
fn lock_taken_after_planning_fails_the_execution() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let target = tree(&repo, &[("a.txt", F, "a\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_new_tree(&ctx, &target).expect("plan");
    std::fs::write(repo.git_dir().join("index.lock"), b"").expect("lock");
    let mut tx = journal.begin();
    let error = apply_plan(&mut tx, &ctx, plan).expect_err("locked");
    assert_eq!(error.code(), ErrorCode::GitLocked);
    assert!(!exists(&repo, "a.txt"));
}

#[test]
fn reference_update_uses_lock_protocol_and_appends_reflog() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let target = tree(&repo, &[("a.txt", F, "a\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let mut plan = plan_new_tree(&ctx, &target).expect("plan");
    let id = repo
        .write_blob(b"stand-in object\n")
        .expect("blob")
        .detach();
    let committer = gix::actor::Signature {
        name: "Test".into(),
        email: "test@example.com".into(),
        time: gix::date::Time::new(1_700_000_000, 3600),
    };
    plan.refs.push(
        RefPlan::new(
            &repo,
            "refs/heads/topic/one",
            RefTarget::Object(id),
            Some(ReflogLine {
                old: gix::ObjectId::null(gix::hash::Kind::Sha1),
                new: id,
                committer,
                message: "branch: Created from\nHEAD".into(),
            }),
        )
        .expect("ref plan"),
    );
    plan.state_files.push(StateFilePlan {
        path: "ORIG_HEAD".into(),
        content: Some(format!("{id}\n").into_bytes()),
        expect: nurt_contracts::journal::Expect::Absent,
    });
    let root = repo.workdir().expect("workdir").to_path_buf();
    let before = tree_state(&root);
    run(&repo, &journal, plan);
    let git = repo.git_dir();
    assert_eq!(
        std::fs::read(git.join("refs/heads/topic/one")).expect("ref"),
        format!("{id}\n").into_bytes()
    );
    let log = std::fs::read_to_string(git.join("logs/refs/heads/topic/one")).expect("log");
    assert!(
        log.ends_with("Test <test@example.com> 1700000000 +0100\tbranch: Created from HEAD\n"),
        "{log}"
    );
    assert!(!git.join("refs/heads/topic/one.lock").exists());
    journal.undo_last();
    assert_eq!(tree_state(&root), before);
}

#[test]
fn eol_attribute_converts_on_write_and_file_stays_clean() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let target = tree(
        &repo,
        &[
            (".gitattributes", F, "*.txt text eol=crlf\n"),
            ("a.txt", F, "one\ntwo\n"),
        ],
    );
    checkout(&repo, &journal, target);
    assert_eq!(read(&repo, "a.txt"), "one\r\ntwo\r\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let entry = ctx
        .index()
        .entry_by_path("a.txt".as_bytes().as_bstr())
        .expect("entry")
        .clone();
    let mut worktree =
        super::worktree::Worktree::new(&repo, ctx.workdir().to_path_buf(), ctx.index())
            .expect("worktree");
    assert_eq!(
        worktree
            .hash_worktree(
                "a.txt".as_bytes().as_bstr(),
                super::worktree::Node::File { executable: false },
                ctx.index()
            )
            .expect("hash"),
        entry.id
    );
}

#[test]
fn snapshot_restores_directories_files_and_links() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let root = dir.path().join("tree");
    std::fs::create_dir_all(root.join("sub")).expect("dirs");
    std::fs::write(root.join("sub/file"), b"content").expect("file");
    let snapshot = Snapshot::take(&root);
    std::fs::remove_dir_all(&root).expect("remove");
    snapshot.restore(&root);
    assert_eq!(Snapshot::take(&root), snapshot);
}

#[test]
fn transaction_rollback_restores_changed_targets() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let file = dir.path().join("f");
    std::fs::write(&file, b"before").expect("file");
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let preserved = tx
        .preserve(&[nurt_contracts::journal::TargetSpec {
            path: nurt_contracts::journal::TargetPath::Fs(
                nurt_contracts::fs::AbsPath::new(&file).expect("abs"),
            ),
            action: nurt_contracts::journal::TargetAction::Modify,
            expect: nurt_contracts::journal::Expect::Unchecked,
            size_hint: None,
        }])
        .expect("preserve");
    tx.apply(
        preserved[0].index,
        nurt_contracts::journal::Mutation::WriteBytes(b"after".to_vec()),
    )
    .expect("apply");
    Box::new(tx)
        .rollback(nurt_contracts::journal::RollbackReason::Cancelled)
        .expect("rollback");
    assert_eq!(std::fs::read(&file).expect("read"), b"before");
}
