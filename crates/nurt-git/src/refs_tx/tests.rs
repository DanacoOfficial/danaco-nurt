// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::refs_tx::tests
// Responsibility:   Unit tests of reference transactions on repositories built in temporary directories.
// Dependencies:     bstr, gix_hash, gix_ref, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Unit tests of reference transactions on repositories built in temporary
//! directories.

use std::path::{Path, PathBuf};

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_ref::{FullName, Target};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::{MutationTx, RollbackReason};

use super::test_journal::{TestJournal, abs, assert_only_declared_changes, tree_state};
use super::*;

const A: &str = "1111111111111111111111111111111111111111";
const B: &str = "2222222222222222222222222222222222222222";
const C: &str = "3333333333333333333333333333333333333333";
const NULL: &str = "0000000000000000000000000000000000000000";
const IDENT: &str = "Nurt Test <nurt@example.com> 1767225600 +0100";

fn id(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex.as_bytes()).unwrap()
}

fn name(text: &str) -> FullName {
    FullName::try_from(text).unwrap()
}

fn obj(hex: &str) -> Target {
    Target::Object(id(hex))
}

fn entry(old: &str, new: &str, message: &str) -> String {
    format!("{old} {new} {IDENT}\t{message}\n")
}

/// Repository directory with `HEAD -> refs/heads/main` at `A` and a reflog
/// for both.
struct Fixture {
    _dir: tempfile::TempDir,
    git: PathBuf,
    journal: TestJournal,
    log_all: LogAllRefUpdates,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        let fixture = Self {
            _dir: dir,
            git,
            journal: TestJournal::new(),
            log_all: LogAllRefUpdates::Standard,
        };
        fixture.write("HEAD", "ref: refs/heads/main\n");
        fixture.write("refs/heads/main", &format!("{A}\n"));
        fixture.write("logs/HEAD", &entry(NULL, A, "commit (initial): a"));
        fixture.write(
            "logs/refs/heads/main",
            &entry(NULL, A, "commit (initial): a"),
        );
        std::fs::create_dir_all(fixture.git.join("refs/tags")).unwrap();
        std::fs::create_dir_all(fixture.git.join("objects")).unwrap();
        fixture
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.git.join(relative)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read(self.path(relative))
            .ok()
            .map(|bytes| bytes.to_str_lossy().into_owned())
    }

    fn layout(&self) -> RefLayout {
        RefLayout::new(
            abs(&self.git),
            abs(&self.git),
            gix_hash::Kind::Sha1,
            self.log_all,
        )
    }

    fn refs(&self) -> RefTx {
        let committer = gix_actor::Signature {
            name: "Nurt Test".into(),
            email: "nurt@example.com".into(),
            time: gix_date::Time {
                seconds: 1_767_225_600,
                offset: 3600,
            },
        };
        RefTx::new(self.layout(), committer)
    }

    /// Applies the edits through a recording transaction, checks that only
    /// declared targets changed, commits, and checks that undo restores the
    /// repository byte for byte; the state after the operation is restored
    /// for the assertions of the caller.
    fn run(&self, edits: Vec<RefEdit>) -> Result<Vec<RefOutcome>> {
        let mut refs = self.refs();
        for edit in edits {
            refs.push(edit);
        }
        let before = tree_state(&self.git);
        let mut tx = self.journal.begin();
        let outcomes = match refs
            .prepare()
            .and_then(|prepared| prepared.apply(tx.as_mut()))
        {
            Ok(outcomes) => outcomes,
            Err(error) => {
                tx.rollback(RollbackReason::TargetFailed).unwrap();
                assert_eq!(
                    tree_state(&self.git),
                    before,
                    "failed transaction changed files"
                );
                return Err(error);
            }
        };
        let after = tree_state(&self.git);
        assert_only_declared_changes(&before, &after, tx.declared(), &[]);
        let committed = self.journal.committed();
        tx.commit().unwrap();
        if self.journal.committed() > committed {
            self.journal.undo_last();
            assert_eq!(
                tree_state(&self.git),
                before,
                "undo did not restore the repository"
            );
            restore_state(&after, &before);
        }
        Ok(outcomes)
    }
}

/// Writes the files of `state` over a tree in state `current`.
fn restore_state(
    state: &std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>,
    current: &std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>,
) {
    for path in current.keys().rev() {
        if !state.contains_key(path) {
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    for (path, content) in state {
        match content {
            None => {
                if path.is_file() {
                    std::fs::remove_file(path).unwrap();
                }
                std::fs::create_dir_all(path).unwrap();
            }
            Some(bytes) => {
                if path.is_dir() {
                    std::fs::remove_dir_all(path).unwrap();
                }
                std::fs::write(path, bytes).unwrap();
            }
        }
    }
}

fn set(target: &str, value: Target, expected: PreviousValue, message: &str) -> RefEdit {
    RefEdit::set(name(target), value, expected, message)
}

#[test]
fn creating_a_branch_writes_the_ref_and_a_new_reflog() {
    let fx = Fixture::new();
    let outcomes = fx
        .run(vec![set(
            "refs/heads/topic",
            obj(A),
            PreviousValue::MustNotExist,
            "branch: Created from HEAD",
        )])
        .unwrap();
    assert_eq!(outcomes[0].kind, RefChangeKind::Created);
    assert_eq!(fx.read("refs/heads/topic").unwrap(), format!("{A}\n"));
    assert_eq!(
        fx.read("logs/refs/heads/topic").unwrap(),
        entry(NULL, A, "branch: Created from HEAD")
    );
    assert_eq!(
        fx.read("logs/HEAD").unwrap(),
        entry(NULL, A, "commit (initial): a")
    );
}

#[test]
fn nested_branch_creates_and_undo_removes_directories() {
    let fx = Fixture::new();
    fx.run(vec![set(
        "refs/heads/feature/x/y",
        obj(B),
        PreviousValue::Any,
        "m",
    )])
    .unwrap();
    assert_eq!(fx.read("refs/heads/feature/x/y").unwrap(), format!("{B}\n"));
    assert!(fx.path("logs/refs/heads/feature/x/y").is_file());
}

#[test]
fn commit_through_head_logs_branch_and_head() {
    let fx = Fixture::new();
    let edit = set(
        "HEAD",
        obj(B),
        PreviousValue::MustExistAndMatch(obj(A)),
        "commit: b",
    )
    .deref();
    let outcomes = fx.run(vec![edit]).unwrap();
    assert_eq!(outcomes[0].name.as_bstr(), "refs/heads/main");
    assert_eq!(outcomes[0].kind, RefChangeKind::Updated);
    assert_eq!(fx.read("HEAD").unwrap(), "ref: refs/heads/main\n");
    assert_eq!(fx.read("refs/heads/main").unwrap(), format!("{B}\n"));
    let expected = format!(
        "{}{}",
        entry(NULL, A, "commit (initial): a"),
        entry(A, B, "commit: b")
    );
    assert_eq!(fx.read("logs/HEAD").unwrap(), expected);
    assert_eq!(fx.read("logs/refs/heads/main").unwrap(), expected);
}

#[test]
fn direct_update_of_the_current_branch_is_logged_for_head() {
    let fx = Fixture::new();
    fx.run(vec![set(
        "refs/heads/main",
        obj(B),
        PreviousValue::Any,
        "direct",
    )])
    .unwrap();
    assert!(
        fx.read("logs/HEAD")
            .unwrap()
            .ends_with(&entry(A, B, "direct"))
    );
}

#[test]
fn unchanged_value_is_not_rewritten_but_head_is_logged() {
    let fx = Fixture::new();
    let outcomes = fx
        .run(vec![
            set("HEAD", obj(A), PreviousValue::Any, "reset: moving to HEAD").deref(),
        ])
        .unwrap();
    assert_eq!(outcomes[0].kind, RefChangeKind::Unchanged);
    assert!(
        fx.read("logs/HEAD")
            .unwrap()
            .ends_with(&entry(A, A, "reset: moving to HEAD"))
    );
    assert_eq!(
        fx.read("logs/refs/heads/main").unwrap(),
        entry(NULL, A, "commit (initial): a")
    );
}

#[test]
fn expected_value_mismatch_is_rejected_without_changes() {
    let fx = Fixture::new();
    let error = fx
        .run(vec![set(
            "refs/heads/main",
            obj(C),
            PreviousValue::MustExistAndMatch(obj(B)),
            "x",
        )])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::GitRef);
    assert_eq!(error.problem.as_deref(), Some("changed"));
    let error = fx
        .run(vec![set(
            "refs/heads/main",
            obj(C),
            PreviousValue::MustNotExist,
            "x",
        )])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("exists"));
    let error = fx
        .run(vec![set(
            "refs/heads/none",
            obj(C),
            PreviousValue::MustExist,
            "x",
        )])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("missing"));
}

#[test]
fn existing_lock_file_fails_with_git_locked_and_a_fix() {
    let fx = Fixture::new();
    fx.write("refs/heads/main.lock", "");
    let error = fx
        .run(vec![set(
            "refs/heads/main",
            obj(B),
            PreviousValue::Any,
            "x",
        )])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::GitLocked);
    assert!(error.fix[0].starts_with("remove \""));
    assert!(error.fix[0].ends_with("refs/heads/main.lock\""));
}

#[test]
fn lock_appearing_after_prepare_fails_with_git_locked() {
    let fx = Fixture::new();
    let mut refs = fx.refs();
    refs.push(set("refs/heads/main", obj(B), PreviousValue::Any, "x"));
    let prepared = refs.prepare().unwrap();
    fx.write("refs/heads/main.lock", "");
    let mut tx = fx.journal.begin();
    let error = prepared.apply(tx.as_mut()).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitLocked);
}

#[test]
fn concurrent_change_after_prepare_fails_with_stale_and_rolls_back() {
    let fx = Fixture::new();
    let mut refs = fx.refs();
    refs.push(set("refs/heads/main", obj(B), PreviousValue::Any, "x"));
    let prepared = refs.prepare().unwrap();
    fx.write("refs/heads/main", &format!("{C}\n"));
    let before = tree_state(&fx.git);
    let mut tx = fx.journal.begin();
    let error = prepared.apply(tx.as_mut()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Stale);
    tx.rollback(RollbackReason::TargetFailed).unwrap();
    assert_eq!(tree_state(&fx.git), before);
}

#[test]
fn deleting_a_loose_branch_removes_its_reflog_and_empty_directories() {
    let fx = Fixture::new();
    fx.write("refs/heads/f/x", &format!("{B}\n"));
    fx.write(
        "logs/refs/heads/f/x",
        &entry(NULL, B, "branch: Created from HEAD"),
    );
    let outcomes = fx
        .run(vec![RefEdit::delete(
            name("refs/heads/f/x"),
            PreviousValue::MustExistAndMatch(obj(B)),
            "",
        )])
        .unwrap();
    assert_eq!(outcomes[0].kind, RefChangeKind::Deleted);
    assert!(!fx.path("refs/heads/f").exists());
    assert!(!fx.path("logs/refs/heads/f").exists());
    assert!(fx.path("refs/heads").is_dir());
    assert!(fx.path("logs/refs/heads").is_dir());
}

#[test]
fn deleting_a_packed_tag_rewrites_packed_refs() {
    let fx = Fixture::new();
    let header = "# pack-refs with: peeled fully-peeled sorted \n";
    fx.write(
        "packed-refs",
        &format!("{header}{A} refs/heads/old\n{B} refs/tags/v1\n^{A}\n{C} refs/tags/v2\n"),
    );
    fx.run(vec![RefEdit::delete(
        name("refs/tags/v1"),
        PreviousValue::MustExist,
        "",
    )])
    .unwrap();
    assert_eq!(
        fx.read("packed-refs").unwrap(),
        format!("{header}{A} refs/heads/old\n{C} refs/tags/v2\n")
    );
    assert!(!fx.path("packed-refs.lock").exists());
}

#[test]
fn deleting_a_ref_that_is_loose_and_packed_removes_both() {
    let fx = Fixture::new();
    fx.write("packed-refs", &format!("{A} refs/heads/side\n"));
    fx.write("refs/heads/side", &format!("{B}\n"));
    let outcomes = fx
        .run(vec![RefEdit::delete(
            name("refs/heads/side"),
            PreviousValue::Any,
            "",
        )])
        .unwrap();
    assert_eq!(outcomes[0].previous, Some(obj(B)));
    assert!(!fx.path("refs/heads/side").exists());
    assert!(!fx.read("packed-refs").unwrap().contains("side"));
}

#[test]
fn deleting_a_missing_ref_without_expectation_changes_nothing() {
    let fx = Fixture::new();
    let outcomes = fx
        .run(vec![RefEdit::delete(
            name("refs/heads/none"),
            PreviousValue::Any,
            "",
        )])
        .unwrap();
    assert_eq!(outcomes[0].kind, RefChangeKind::Unchanged);
}

#[test]
fn updating_a_packed_branch_writes_a_loose_file() {
    let fx = Fixture::new();
    fx.write("packed-refs", &format!("{A} refs/heads/side\n"));
    fx.run(vec![set(
        "refs/heads/side",
        obj(B),
        PreviousValue::MustExistAndMatch(obj(A)),
        "m",
    )])
    .unwrap();
    assert_eq!(fx.read("refs/heads/side").unwrap(), format!("{B}\n"));
    assert!(fx.read("packed-refs").unwrap().contains("refs/heads/side"));
}

#[test]
fn names_colliding_with_existing_directories_are_rejected() {
    let fx = Fixture::new();
    for (existing, new) in [
        ("refs/heads/a", "refs/heads/a/b"),
        ("refs/heads/c/d", "refs/heads/c"),
    ] {
        fx.write(existing, &format!("{A}\n"));
        let error = fx
            .run(vec![set(new, obj(A), PreviousValue::Any, "m")])
            .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("name_conflict"), "{new}");
    }
    fx.write("packed-refs", &format!("{A} refs/heads/p/q\n"));
    let error = fx
        .run(vec![set("refs/heads/p", obj(A), PreviousValue::Any, "m")])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("name_conflict"));
    let error = fx
        .run(vec![
            set("refs/heads/n", obj(A), PreviousValue::Any, "m"),
            set("refs/heads/n/m", obj(A), PreviousValue::Any, "m"),
        ])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("name_conflict"));
}

#[test]
fn empty_directory_in_place_of_a_new_ref_is_replaced() {
    let fx = Fixture::new();
    std::fs::create_dir_all(fx.path("refs/heads/e/empty")).unwrap();
    fx.run(vec![set("refs/heads/e", obj(A), PreviousValue::Any, "m")])
        .unwrap();
    assert_eq!(fx.read("refs/heads/e").unwrap(), format!("{A}\n"));
}

#[test]
fn renaming_the_current_branch_moves_head_and_reflog_like_git() {
    let fx = Fixture::new();
    let message = "Branch: renamed refs/heads/main to refs/heads/trunk";
    let outcomes = fx
        .run(vec![RefEdit::rename(
            name("refs/heads/main"),
            name("refs/heads/trunk"),
            message,
        )])
        .unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(fx.read("HEAD").unwrap(), "ref: refs/heads/trunk\n");
    assert!(!fx.path("refs/heads/main").exists());
    assert!(!fx.path("logs/refs/heads/main").exists());
    assert_eq!(
        fx.read("logs/refs/heads/trunk").unwrap(),
        format!(
            "{}{}",
            entry(NULL, A, "commit (initial): a"),
            entry(A, A, message)
        )
    );
    assert_eq!(
        fx.read("logs/HEAD").unwrap(),
        format!(
            "{}{}{}",
            entry(NULL, A, "commit (initial): a"),
            entry(A, NULL, message),
            entry(NULL, A, message)
        )
    );
}

#[test]
fn renaming_overwrites_a_leftover_reflog_of_the_new_name_like_git() {
    let fx = Fixture::new();
    fx.write("refs/heads/side", &format!("{B}\n"));
    fx.write("logs/refs/heads/side", &entry(NULL, B, "created"));
    fx.write("logs/refs/heads/other", "leftover\n");
    fx.run(vec![RefEdit::rename(
        name("refs/heads/side"),
        name("refs/heads/other"),
        "rename",
    )])
    .unwrap();
    assert_eq!(
        fx.read("logs/refs/heads/other").unwrap(),
        format!("{}{}", entry(NULL, B, "created"), entry(B, B, "rename"))
    );
}

#[test]
fn renaming_a_reference_without_reflog_appends_to_a_leftover_reflog_like_git() {
    let fx = Fixture::new();
    fx.write("refs/heads/side", &format!("{B}\n"));
    fx.write("logs/refs/heads/other", "leftover\n");
    fx.run(vec![RefEdit::rename(
        name("refs/heads/side"),
        name("refs/heads/other"),
        "rename",
    )])
    .unwrap();
    assert_eq!(
        fx.read("logs/refs/heads/other").unwrap(),
        format!("leftover\n{}", entry(B, B, "rename"))
    );
}

#[test]
fn renaming_updates_head_of_a_linked_worktree() {
    let fx = Fixture::new();
    fx.write("refs/heads/side", &format!("{B}\n"));
    fx.write("worktrees/wt/HEAD", "ref: refs/heads/side\n");
    fx.run(vec![RefEdit::rename(
        name("refs/heads/side"),
        name("refs/heads/other"),
        "rename",
    )])
    .unwrap();
    assert_eq!(
        fx.read("worktrees/wt/HEAD").unwrap(),
        "ref: refs/heads/other\n"
    );
    assert_eq!(fx.read("HEAD").unwrap(), "ref: refs/heads/main\n");
    assert_eq!(
        fx.read("worktrees/wt/logs/HEAD").unwrap(),
        format!("{}{}", entry(B, NULL, "rename"), entry(NULL, B, "rename"))
    );
}

#[test]
fn renaming_onto_an_existing_or_from_a_missing_branch_fails() {
    let fx = Fixture::new();
    fx.write("refs/heads/side", &format!("{B}\n"));
    let error = fx
        .run(vec![RefEdit::rename(
            name("refs/heads/side"),
            name("refs/heads/main"),
            "r",
        )])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("exists"));
    let error = fx
        .run(vec![RefEdit::rename(
            name("refs/heads/none"),
            name("refs/heads/x"),
            "r",
        )])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("missing"));
    let error = fx
        .run(vec![RefEdit::rename(
            name("HEAD"),
            name("refs/heads/x"),
            "r",
        )])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("symbolic"));
}

#[test]
fn switching_head_logs_resolved_values() {
    let fx = Fixture::new();
    fx.write("refs/heads/side", &format!("{B}\n"));
    let target = Target::Symbolic(name("refs/heads/side"));
    fx.run(vec![set(
        "HEAD",
        target,
        PreviousValue::Any,
        "checkout: moving from main to side",
    )])
    .unwrap();
    assert_eq!(fx.read("HEAD").unwrap(), "ref: refs/heads/side\n");
    assert!(fx.read("logs/HEAD").unwrap().ends_with(&entry(
        A,
        B,
        "checkout: moving from main to side"
    )));
}

#[test]
fn switching_head_to_an_unborn_branch_writes_no_reflog_entry() {
    let fx = Fixture::new();
    let target = Target::Symbolic(name("refs/heads/orphan"));
    fx.run(vec![set("HEAD", target, PreviousValue::Any, "checkout")])
        .unwrap();
    assert_eq!(
        fx.read("logs/HEAD").unwrap(),
        entry(NULL, A, "commit (initial): a")
    );
}

#[test]
fn switching_to_a_branch_created_in_the_same_transaction_logs_its_value() {
    let fx = Fixture::new();
    fx.run(vec![
        set(
            "refs/heads/new",
            obj(C),
            PreviousValue::MustNotExist,
            "branch: Created from HEAD",
        ),
        set(
            "HEAD",
            Target::Symbolic(name("refs/heads/new")),
            PreviousValue::Any,
            "checkout: moving from main to new",
        ),
    ])
    .unwrap();
    assert!(fx.read("logs/HEAD").unwrap().ends_with(&entry(
        A,
        C,
        "checkout: moving from main to new"
    )));
    assert_eq!(
        fx.read("logs/refs/heads/new").unwrap(),
        entry(NULL, C, "branch: Created from HEAD")
    );
}

#[test]
fn reflog_creation_follows_log_all_ref_updates() {
    let mut fx = Fixture::new();
    fx.run(vec![set("refs/tags/t", obj(A), PreviousValue::Any, "m")])
        .unwrap();
    assert!(!fx.path("logs/refs/tags/t").exists());
    fx.run(vec![
        set("refs/tags/f", obj(A), PreviousValue::Any, "m").create_reflog(),
    ])
    .unwrap();
    assert!(fx.path("logs/refs/tags/f").exists());
    fx.log_all = LogAllRefUpdates::Always;
    fx.run(vec![set("refs/tags/u", obj(A), PreviousValue::Any, "m")])
        .unwrap();
    assert!(fx.path("logs/refs/tags/u").exists());
    fx.log_all = LogAllRefUpdates::Never;
    fx.run(vec![set("refs/heads/n", obj(A), PreviousValue::Any, "m")])
        .unwrap();
    assert!(!fx.path("logs/refs/heads/n").exists());
    fx.run(vec![set(
        "refs/heads/main",
        obj(B),
        PreviousValue::Any,
        "kept",
    )])
    .unwrap();
    assert!(
        fx.read("logs/refs/heads/main")
            .unwrap()
            .ends_with(&entry(A, B, "kept"))
    );
}

#[test]
fn log_only_edit_appends_without_changing_the_ref() {
    let fx = Fixture::new();
    fx.run(vec![
        set("refs/heads/main", obj(B), PreviousValue::Any, "note").log_only(),
    ])
    .unwrap();
    assert_eq!(fx.read("refs/heads/main").unwrap(), format!("{A}\n"));
    assert!(
        fx.read("logs/refs/heads/main")
            .unwrap()
            .ends_with(&entry(A, B, "note"))
    );
}

#[test]
fn per_worktree_refs_live_in_the_worktree_directory() {
    let fx = Fixture::new();
    let common = fx.git.clone();
    let worktree = fx.path("worktrees/wt");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(worktree.join("HEAD"), format!("{B}\n")).unwrap();
    let layout = RefLayout::new(
        abs(&worktree),
        abs(&common),
        gix_hash::Kind::Sha1,
        LogAllRefUpdates::Standard,
    );
    let mut refs = RefTx::new(layout, fx.refs().committer);
    refs.push(set("refs/bisect/bad", obj(C), PreviousValue::Any, "m"));
    refs.push(set("refs/heads/shared", obj(C), PreviousValue::Any, "m"));
    refs.push(set("HEAD", obj(C), PreviousValue::Any, "detached"));
    let mut tx = fx.journal.begin();
    refs.prepare().unwrap().apply(tx.as_mut()).unwrap();
    assert!(worktree.join("refs/bisect/bad").is_file());
    assert!(common.join("refs/heads/shared").is_file());
    assert_eq!(
        std::fs::read_to_string(worktree.join("HEAD")).unwrap(),
        format!("{C}\n")
    );
    assert!(worktree.join("logs/HEAD").is_file());
}

#[test]
fn invalid_edits_are_rejected() {
    let fx = Fixture::new();
    let error = fx
        .run(vec![set(
            "main-worktree/HEAD",
            obj(A),
            PreviousValue::Any,
            "m",
        )])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::GitUnsupported);
    let error = fx
        .run(vec![
            set("refs/heads/x", obj(A), PreviousValue::Any, "m"),
            set("refs/heads/x", obj(B), PreviousValue::Any, "m"),
        ])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("duplicate"));
    fx.write("refs/heads/corrupt", "not a ref\n");
    let error = fx
        .run(vec![set(
            "refs/heads/corrupt",
            obj(A),
            PreviousValue::Any,
            "m",
        )])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("corrupt"));
}

#[test]
fn symbolic_loops_are_detected() {
    let fx = Fixture::new();
    fx.write("refs/heads/l1", "ref: refs/heads/l2\n");
    fx.write("refs/heads/l2", "ref: refs/heads/l1\n");
    let error = fx
        .run(vec![
            set("refs/heads/l1", obj(A), PreviousValue::Any, "m").deref(),
        ])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("symref_loop"));
}

#[test]
fn update_follows_a_long_symbolic_chain_like_git() {
    let fx = Fixture::new();
    for level in 0..5 {
        fx.write(
            &format!("refs/heads/s{level}"),
            &format!("ref: refs/heads/s{}\n", level + 1),
        );
    }
    fx.write("refs/heads/s5", &format!("{A}\n"));
    fx.run(vec![
        set("refs/heads/s0", obj(B), PreviousValue::Any, "m").deref(),
    ])
    .unwrap();
    assert_eq!(fx.read("refs/heads/s5").unwrap(), format!("{B}\n"));
    assert_eq!(fx.read("refs/heads/s0").unwrap(), "ref: refs/heads/s1\n");
}

#[test]
fn prepared_transaction_reports_targets_before_applying() {
    let fx = Fixture::new();
    let mut refs = fx.refs();
    refs.push(set("refs/heads/topic", obj(B), PreviousValue::Any, "m"));
    assert_eq!(refs.edits().len(), 1);
    assert_eq!(refs.layout().log_all(), LogAllRefUpdates::Standard);
    let mut prepared = refs.prepare().unwrap();
    assert!(!prepared.is_empty());
    assert_eq!(prepared.outcomes()[0].kind, RefChangeKind::Created);
    let targets = prepared.targets();
    let mut tx = fx.journal.begin();
    prepared.declare(tx.as_mut()).unwrap();
    assert_eq!(tx.declared(), targets.as_slice());
    prepared.apply(tx.as_mut()).unwrap();
    tx.commit().unwrap();
    assert!(fx.path("refs/heads/topic").is_file());
}

#[test]
fn layout_is_read_from_repository_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let repo = crate::testutil::init_isolated(dir.path()).unwrap();
    let layout = RefLayout::from_repo(&repo).unwrap();
    assert_eq!(layout.log_all(), LogAllRefUpdates::Standard);
    assert_eq!(layout.git_dir(), layout.common_dir());
    assert_eq!(layout.object_hash(), gix_hash::Kind::Sha1);
    let config = dir.path().join(".git/config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[core]\n\tlogAllRefUpdates = always\n");
    std::fs::write(&config, &text).unwrap();
    let repo = crate::testutil::open_isolated(dir.path()).unwrap();
    assert_eq!(
        RefLayout::from_repo(&repo).unwrap().log_all(),
        LogAllRefUpdates::Always
    );
    text.push_str("[core]\n\tlogAllRefUpdates = sometimes\n");
    std::fs::write(&config, &text).unwrap();
    let repo = crate::testutil::open_isolated(dir.path()).unwrap();
    let error = RefLayout::from_repo(&repo).unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("invalid_config"));
    let bare = tempfile::tempdir().unwrap();
    let repo = crate::testutil::init_bare_isolated(bare.path()).unwrap();
    assert_eq!(
        RefLayout::from_repo(&repo).unwrap().log_all(),
        LogAllRefUpdates::Never
    );
}

#[test]
fn every_file_is_one_target_checked_against_its_previous_content() {
    let fx = Fixture::new();
    fx.write("packed-refs", &format!("{B} refs/tags/v1\n"));
    let mut refs = fx.refs();
    refs.push(set("refs/heads/main", obj(B), PreviousValue::Any, "m"));
    refs.push(set("refs/heads/topic", obj(B), PreviousValue::Any, "m"));
    refs.push(RefEdit::delete(
        name("refs/tags/v1"),
        PreviousValue::Any,
        "",
    ));
    let targets = refs.prepare().unwrap().targets();
    let paths: Vec<PathBuf> = targets
        .iter()
        .map(|target| match &target.path {
            nurt_contracts::journal::TargetPath::Fs(path) => path.as_path().to_path_buf(),
            nurt_contracts::journal::TargetPath::Provider(_) => unreachable!(),
        })
        .collect();
    let unique: std::collections::BTreeSet<&PathBuf> = paths.iter().collect();
    assert_eq!(unique.len(), paths.len(), "a path is declared twice");
    assert!(
        paths
            .iter()
            .all(|path| path.extension().is_none_or(|ext| ext != "lock"))
    );
    let main = targets
        .iter()
        .find(|target| {
            target.path == nurt_contracts::journal::TargetPath::Fs(abs(&fx.path("refs/heads/main")))
        })
        .unwrap();
    assert_eq!(main.action, nurt_contracts::journal::TargetAction::Modify);
    assert!(matches!(
        main.expect,
        nurt_contracts::journal::Expect::Hash(_)
    ));
    let topic = targets
        .iter()
        .find(|target| {
            target.path
                == nurt_contracts::journal::TargetPath::Fs(abs(&fx.path("refs/heads/topic")))
        })
        .unwrap();
    assert_eq!(topic.expect, nurt_contracts::journal::Expect::Absent);
}

#[test]
fn transaction_without_changes_declares_nothing() {
    let fx = Fixture::new();
    let mut refs = fx.refs();
    refs.push(RefEdit::delete(
        name("refs/heads/none"),
        PreviousValue::Any,
        "",
    ));
    let prepared = refs.prepare().unwrap();
    assert!(prepared.is_empty());
    let mut tx = fx.journal.begin();
    prepared.apply(tx.as_mut()).unwrap();
    assert!(tx.declared().is_empty());
}

#[test]
fn paths_of_names_are_joined_per_component() {
    let base = abs(Path::new(if cfg!(windows) { "C:\\r" } else { "/r" }));
    let path = join_ref_path(&base, Some("logs"), "refs/heads/a/b".into()).unwrap();
    assert!(
        path.as_path().ends_with(
            Path::new("logs")
                .join("refs")
                .join("heads")
                .join("a")
                .join("b")
        )
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
