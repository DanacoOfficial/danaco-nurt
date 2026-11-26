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

fn enable_sparse_cone(dir: &Path, patterns: &str) -> gix::Repository {
    let git = dir.join(".git");
    std::fs::create_dir_all(git.join("info")).expect("info");
    std::fs::write(git.join("info/sparse-checkout"), patterns).expect("patterns");
    let mut config = std::fs::read_to_string(git.join("config")).expect("config");
    config.push_str("[core]\n\tsparseCheckout = true\n\tsparseCheckoutCone = true\n");
    std::fs::write(git.join("config"), config).expect("config");
    gix::open_opts(dir, gix::open::Options::isolated()).expect("reopen")
}

fn has_skip_worktree(repo: &gix::Repository, path: &str) -> bool {
    let index = repo.open_index().expect("index");
    index
        .entry_by_path(path.as_bytes().as_bstr())
        .expect("entry")
        .flags
        .contains(gix_index::entry::Flags::SKIP_WORKTREE)
}

#[test]
fn two_way_marks_new_path_outside_sparse_cone_skip_worktree() {
    let (dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(
        &repo,
        &[("keep/k.txt", F, "k\n"), ("other/o.txt", F, "o\n")],
    );
    let new = tree(
        &repo,
        &[
            ("keep/k.txt", F, "k\n"),
            ("other/o.txt", F, "o\n"),
            ("other/new.txt", F, "n\n"),
            ("root.txt", F, "r\n"),
        ],
    );
    checkout(&repo, &journal, old);
    let repo = enable_sparse_cone(dir.path(), "/*\n!/*/\n/keep/\n");
    switch(&repo, &journal, old, new);
    assert!(!exists(&repo, "other/new.txt"));
    assert_eq!(read(&repo, "root.txt"), "r\n");
    assert!(has_skip_worktree(&repo, "other/new.txt"));
    assert!(!has_skip_worktree(&repo, "root.txt"));
}

#[test]
fn two_way_keeps_skip_worktree_of_changed_entry_without_writing_it() {
    let (dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(
        &repo,
        &[("keep/k.txt", F, "k\n"), ("other/o.txt", F, "o\n")],
    );
    let new = tree(
        &repo,
        &[("keep/k.txt", F, "k\n"), ("other/o.txt", F, "o2\n")],
    );
    checkout(&repo, &journal, old);
    let repo = enable_sparse_cone(dir.path(), "/*\n!/*/\n/keep/\n");
    let mut index = repo.open_index().expect("index");
    let position = index
        .entry_index_by_path(b"other/o.txt".as_bstr())
        .expect("entry");
    index.entries_mut()[position].flags |=
        gix_index::entry::Flags::SKIP_WORKTREE | gix_index::entry::Flags::EXTENDED;
    #[allow(
        clippy::disallowed_methods,
        reason = "test fixtures prepare the index directly, outside a transaction"
    )]
    index
        .write(gix_index::write::Options::default(), Default::default())
        .expect("write index");
    std::fs::remove_dir_all(dir.path().join("other")).expect("remove");
    switch(&repo, &journal, old, new);
    assert!(!exists(&repo, "other"));
    assert!(has_skip_worktree(&repo, "other/o.txt"));
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

fn reopen_with_config(dir: &Path, config: &str) -> gix::Repository {
    let path = dir.join(".git/config");
    let mut text = std::fs::read_to_string(&path).expect("config");
    text.push_str(config);
    std::fs::write(&path, text).expect("config");
    gix::open_opts(dir, gix::open::Options::isolated()).expect("reopen")
}

fn make_unmerged(repo: &gix::Repository, path: &str) {
    use gix_index::entry::{Flags, Mode, Stage, Stat};
    let mut index = repo.open_index().expect("index");
    let id = repo.write_blob(b"stage\n").expect("blob").detach();
    let name = path.as_bytes().as_bstr();
    index.remove_entries(|_, entry_path, _| entry_path == name);
    for stage in [Stage::Base, Stage::Ours, Stage::Theirs] {
        index.dangerously_push_entry(
            Stat::default(),
            id,
            Flags::from_stage(stage),
            Mode::FILE,
            name,
        );
    }
    index.sort_entries();
    #[allow(
        clippy::disallowed_methods,
        reason = "test fixtures prepare the index directly, outside a transaction"
    )]
    index
        .write(gix_index::write::Options::default(), Default::default())
        .expect("write index");
}

#[test]
fn two_way_refuses_index_with_conflict_stages() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("f.txt", F, "1\n"), ("g.txt", F, "g\n")]);
    let new = tree(&repo, &[("f.txt", F, "2\n"), ("g.txt", F, "g\n")]);
    checkout(&repo, &journal, old);
    make_unmerged(&repo, "f.txt");
    let ctx = ApplyContext::open(&repo).expect("context");
    assert_eq!(ctx.unmerged_paths(), [BString::from("f.txt")]);
    let error = plan_two_way(&ctx, Some(&old), &new).expect_err("unmerged");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(error.data().problem.as_deref(), Some("unmerged"));
}

#[test]
fn paths_refuse_unmerged_worktree_restore_and_staged_restore_resolves_it() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("f.txt", F, "1\n")]);
    checkout(&repo, &journal, base);
    make_unmerged(&repo, "f.txt");
    let ctx = ApplyContext::open(&repo).expect("context");
    let request = |source, to_index, to_worktree| PathsRequest {
        source,
        pathspec: vec!["f.txt".into()],
        to_index,
        to_worktree,
    };
    let error = plan_paths(&ctx, &request(PathSource::Index, false, true)).expect_err("unmerged");
    assert_eq!(error.data().problem.as_deref(), Some("unmerged"));
    let plan = plan_paths(&ctx, &request(PathSource::Tree(base), true, false)).expect("plan");
    run(&repo, &journal, plan);
    let index = repo.open_index().expect("index");
    assert_eq!(index.entries().len(), 1);
    assert_eq!(index.entries()[0].stage_raw(), 0);
}

#[test]
fn open_refuses_bare_repository() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let repo = gix::ThreadSafeRepository::init_opts(
        dir.path(),
        gix::create::Kind::Bare,
        gix::create::Options::default(),
        gix::open::Options::isolated(),
    )
    .expect("init")
    .to_thread_local();
    let error = ApplyContext::open(&repo).err().expect("bare");
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
    assert_eq!(error.data().problem.as_deref(), Some("bare_repository"));
}

#[test]
fn gitlink_is_checked_out_as_empty_directory_and_removed_with_it() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let stand_in = repo
        .write_blob(b"submodule commit\n")
        .expect("blob")
        .detach();
    let without = tree(&repo, &[("a.txt", F, "a\n")]);
    let mut editor = repo.edit_tree(without).expect("editor");
    editor
        .upsert("sub", TreeKind::Commit, stand_in)
        .expect("upsert");
    let with = editor.write().expect("tree").detach();
    checkout(&repo, &journal, without);
    switch(&repo, &journal, without, with);
    let sub = repo.workdir().expect("workdir").join("sub");
    assert!(sub.is_dir());
    assert_eq!(std::fs::read_dir(&sub).expect("read").count(), 0);
    assert!(index_paths(&repo).contains(&"sub".to_owned()));
    switch(&repo, &journal, with, without);
    assert!(!exists(&repo, "sub"));
}

#[test]
fn symlinks_disabled_write_link_text_as_file() {
    let (dir, repo) = init();
    let repo_off = reopen_with_config(dir.path(), "[core]\n\tsymlinks = false\n");
    let journal = RecordingJournal::new();
    let target = tree(
        &repo,
        &[
            ("target.txt", F, "t\n"),
            ("link", TreeKind::Link, "target.txt"),
        ],
    );
    checkout(&repo_off, &journal, target);
    let meta = std::fs::symlink_metadata(dir.path().join("link")).expect("meta");
    assert!(meta.is_file());
    assert_eq!(read(&repo_off, "link"), "target.txt");
    let ctx = ApplyContext::open(&repo_off).expect("context");
    let again = plan_two_way(&ctx, Some(&target), &target).expect("plan");
    assert!(again.carried.is_empty(), "{:?}", again.carried);
}

#[cfg(unix)]
#[test]
fn two_way_replaces_symbolic_link_by_file_and_back() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let link = tree(
        &repo,
        &[("t.txt", F, "t\n"), ("p", TreeKind::Link, "t.txt")],
    );
    let file = tree(&repo, &[("t.txt", F, "t\n"), ("p", F, "regular\n")]);
    checkout(&repo, &journal, link);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_two_way(&ctx, Some(&link), &file).expect("plan");
    assert_eq!(plan.entries[0].action, EntryAction::TypeChange);
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "p"), "regular\n");
    switch(&repo, &journal, file, link);
    let root = repo.workdir().expect("workdir");
    assert_eq!(
        std::fs::read_link(root.join("p")).expect("link"),
        Path::new("t.txt")
    );
}

#[test]
fn unborn_head_refuses_untracked_file_in_the_way() {
    let (_dir, repo) = init();
    let target = tree(&repo, &[("a.txt", F, "a\n")]);
    write(&repo, "a.txt", "local\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let error = plan_two_way(&ctx, None, &target).expect_err("refused");
    assert_eq!(detail(&error, "untracked_overwritten"), ["a.txt"]);
}

#[test]
fn configured_filter_driver_is_reported_as_not_applied() {
    let (dir, _repo) = init();
    let repo = reopen_with_config(dir.path(), "[filter \"lfs\"]\n\tsmudge = lfs-smudge\n");
    let journal = RecordingJournal::new();
    let target = tree(
        &repo,
        &[
            (".gitattributes", F, "*.bin filter=lfs\n"),
            ("a.bin", F, "payload\n"),
        ],
    );
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_new_tree(&ctx, &target).expect("plan");
    let report = run(&repo, &journal, plan);
    assert!(
        report
            .notable
            .contains(&Notable::at("filter_not_applied", "a.bin")),
        "{:?}",
        report.notable
    );
    assert_eq!(read(&repo, "a.bin"), "payload\n");
}

#[test]
fn rejection_message_quotes_five_paths_and_details_list_all() {
    let mut rejections = Rejections::default();
    for number in 0..7 {
        rejections
            .local_changes
            .insert(BString::from(format!("f{number}.txt")));
    }
    rejections.untracked_removed.insert(BString::from("u.txt"));
    assert!(!rejections.is_empty());
    let error = rejections.into_error("switch", &["git.stash \".\" action: push".to_owned()]);
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert!(
        error.data().message.contains("and 2 more"),
        "{}",
        error.data().message
    );
    assert_eq!(detail(&error, "local_changes").len(), 7);
    assert_eq!(detail(&error, "untracked_removed"), ["u.txt"]);
    assert_eq!(error.data().problem.as_deref(), Some("local_changes"));
    assert_eq!(error.data().fix.len(), 1);
}

#[test]
fn reference_names_are_validated_and_logs_follow_the_configuration() {
    let (dir, repo) = init();
    let error = ref_path(&repo, "refs/heads/bad..name").expect_err("invalid");
    assert_eq!(error.code(), ErrorCode::GitRef);
    assert!(ref_path(&repo, "HEAD").expect("path").ends_with("HEAD"));
    assert!(
        log_path(&repo, "refs/heads/main")
            .expect("path")
            .ends_with(Path::new("logs").join("refs").join("heads").join("main"))
    );
    assert!(keeps_log(&repo, "refs/heads/topic").expect("default"));
    assert!(keeps_log(&repo, "HEAD").expect("default"));
    assert!(!keeps_log(&repo, "refs/tags/v1").expect("default"));
    let always = reopen_with_config(dir.path(), "[core]\n\tlogAllRefUpdates = always\n");
    assert!(keeps_log(&always, "refs/tags/v1").expect("always"));
    let never = reopen_with_config(dir.path(), "[core]\n\tlogAllRefUpdates = false\n");
    assert!(!keeps_log(&never, "refs/heads/topic").expect("false"));
}

#[test]
fn state_file_plan_removes_existing_file_and_undo_restores_it() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let target = tree(&repo, &[("a.txt", F, "a\n")]);
    checkout(&repo, &journal, target);
    std::fs::write(repo.git_dir().join("ORIG_HEAD"), b"previous\n").expect("state file");
    let root = repo.workdir().expect("workdir").to_path_buf();
    let before = tree_state(&root);
    let ctx = ApplyContext::open(&repo).expect("context");
    let mut plan = plan_two_way(&ctx, Some(&target), &target).expect("plan");
    plan.state_files.push(StateFilePlan {
        path: "ORIG_HEAD".into(),
        content: None,
        expect: nurt_contracts::journal::Expect::Unchecked,
    });
    run(&repo, &journal, plan);
    assert!(!repo.git_dir().join("ORIG_HEAD").exists());
    journal.undo_last();
    assert_eq!(tree_state(&root), before);
}

#[test]
fn plan_without_changes_is_empty_and_actions_have_stable_names() {
    let plan = ApplyPlan {
        entries: Vec::new(),
        index: None,
        refs: Vec::new(),
        state_files: Vec::new(),
        carried: Vec::new(),
        notable: Vec::new(),
    };
    assert!(plan.is_empty());
    let names: Vec<&str> = [
        EntryAction::Create,
        EntryAction::Modify,
        EntryAction::Delete,
        EntryAction::Mkdir,
        EntryAction::Rmdir,
        EntryAction::Symlink,
        EntryAction::Chmod,
        EntryAction::TypeChange,
    ]
    .into_iter()
    .map(EntryAction::as_str)
    .collect();
    assert_eq!(
        names,
        [
            "create",
            "modify",
            "delete",
            "mkdir",
            "rmdir",
            "symlink",
            "chmod",
            "type_change"
        ]
    );
}

#[test]
fn reset_removes_paths_absent_from_target_and_restores_changed_files() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(
        &repo,
        &[
            ("a.txt", F, "1\n"),
            ("b.txt", F, "1\n"),
            ("d/c.txt", F, "c\n"),
        ],
    );
    let new = tree(&repo, &[("a.txt", F, "1\n"), ("b.txt", F, "2\n")]);
    checkout(&repo, &journal, old);
    std::fs::remove_file(repo.workdir().expect("workdir").join("a.txt")).expect("remove");
    write(&repo, "b.txt", "local\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_reset(&ctx, &new, ResetOptions::default()).expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "a.txt"), "1\n");
    assert_eq!(read(&repo, "b.txt"), "2\n");
    assert!(!exists(&repo, "d"));
    assert_eq!(index_paths(&repo), ["a.txt", "b.txt"]);
}

#[test]
fn reset_overwrites_untracked_file_when_allowed() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let old = tree(&repo, &[("a.txt", F, "1\n")]);
    let new = tree(&repo, &[("a.txt", F, "1\n"), ("n.txt", F, "n\n")]);
    checkout(&repo, &journal, old);
    write(&repo, "n.txt", "local\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_reset(
        &ctx,
        &new,
        ResetOptions {
            overwrite_untracked: true,
        },
    )
    .expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "n.txt"), "n\n");
}

#[test]
fn new_tree_refuses_untracked_file_in_the_way() {
    let (_dir, repo) = init();
    let target = tree(&repo, &[("n.txt", F, "n\n")]);
    write(&repo, "n.txt", "local\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let error = plan_new_tree(&ctx, &target).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(detail(&error, "untracked_overwritten"), ["n.txt"]);
}

#[test]
fn paths_with_malformed_pathspec_fail_with_arg_error() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    checkout(&repo, &journal, tree(&repo, &[("a.txt", F, "1\n")]));
    let ctx = ApplyContext::open(&repo).expect("context");
    let error = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Index,
            pathspec: vec![BString::from(":(bogus)a.txt")],
            to_index: false,
            to_worktree: true,
        },
    )
    .expect_err("malformed");
    assert_eq!(error.code(), ErrorCode::Arg);
}

#[test]
fn paths_with_glob_restore_only_matching_files() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    checkout(
        &repo,
        &journal,
        tree(&repo, &[("a.txt", F, "1\n"), ("b.md", F, "1\n")]),
    );
    write(&repo, "a.txt", "local\n");
    write(&repo, "b.md", "local\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_paths(
        &ctx,
        &PathsRequest {
            source: PathSource::Index,
            pathspec: vec![BString::from("*.txt")],
            to_index: false,
            to_worktree: true,
        },
    )
    .expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "a.txt"), "1\n");
    assert_eq!(read(&repo, "b.md"), "local\n");
}

#[test]
fn new_tree_refuses_names_that_collide_on_a_case_insensitive_file_system() {
    let (dir, _repo) = init();
    let repo = reopen_with_config(dir.path(), "[core]\n\tignorecase = true\n");
    let ctx = ApplyContext::open(&repo).expect("context");
    let clashes: [&[Spec<'_>]; 2] = [
        &[("Readme", F, "1\n"), ("README", F, "2\n")],
        &[("A", TreeKind::Link, "elsewhere"), ("a/x", F, "x\n")],
    ];
    for clash in clashes {
        let target = tree(&repo, clash);
        let error = plan_new_tree(&ctx, &target).expect_err("collision");
        assert_eq!(error.code(), ErrorCode::GitUnsupported);
        assert_eq!(error.data().problem.as_deref(), Some("unsafe_path"));
    }
    let distinct = tree(&repo, &[("one", F, "1\n"), ("two/x", F, "2\n")]);
    plan_new_tree(&ctx, &distinct).expect("distinct names are accepted");
}

#[test]
fn paths_restore_index_and_working_tree_together_from_a_tree() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("a.txt", F, "new\n"), ("b.txt", F, "b\n")]);
    checkout(&repo, &journal, base);
    let source = tree(&repo, &[("a.txt", F, "old\n"), ("b.txt", F, "b\n")]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let request = |source, to_index, to_worktree| PathsRequest {
        source,
        pathspec: vec!["a.txt".into()],
        to_index,
        to_worktree,
    };
    let plan = plan_paths(&ctx, &request(PathSource::Tree(source), true, true)).expect("plan");
    run(&repo, &journal, plan);
    assert_eq!(read(&repo, "a.txt"), "old\n");
    assert_eq!(read(&repo, "b.txt"), "b\n");
    let old_blob = repo.write_blob(b"old\n").expect("blob").detach();
    let index = repo.open_index().expect("index");
    let entry = index
        .entry_by_path("a.txt".as_bytes().as_bstr())
        .expect("entry");
    assert_eq!(entry.id, old_blob);
    let ctx = ApplyContext::open(&repo).expect("context");
    let again = plan_paths(&ctx, &request(PathSource::Index, false, true)).expect("plan");
    assert!(again.entries.is_empty(), "{:?}", again.entries);
}

#[test]
fn reset_to_the_empty_tree_removes_tracked_files_and_keeps_untracked_ones() {
    let (_dir, repo) = init();
    let journal = RecordingJournal::new();
    let base = tree(&repo, &[("a.txt", F, "a\n"), ("d/b.txt", F, "b\n")]);
    checkout(&repo, &journal, base);
    write(&repo, "free.txt", "u\n");
    let empty = tree(&repo, &[]);
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_reset(&ctx, &empty, ResetOptions::default()).expect("plan");
    run(&repo, &journal, plan);
    assert!(!exists(&repo, "a.txt"));
    assert!(!exists(&repo, "d"));
    assert_eq!(read(&repo, "free.txt"), "u\n");
    assert!(index_paths(&repo).is_empty());
}
