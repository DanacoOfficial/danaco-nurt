// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pass-through journal: transaction protocol and store queries.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::io::Write;
use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{AbsPath, Follow, Fs};
use nurt_contracts::journal::{
    Expect, ExternalChange, ExternalChangeKind, FileOp, HistoryQuery, JournalStore, Mutation, OpId,
    PreserveMode, RollbackReason, TargetAction, TargetIndex, TargetPath, UndoRequest, UndoSelector,
    apply_ops,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_testkit::fixtures::{begin_request, empty_program, fs_target};
use nurt_testkit::journal::{JournalCall, NoopJournal, RecordingJournal};
use nurt_testkit::vfs::{VirtualFs, test_path};

fn path(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn setup() -> (Arc<VirtualFs>, RecordingJournal) {
    let fs = Arc::new(VirtualFs::default());
    let journal = RecordingJournal::new(fs.clone());
    (fs, journal)
}

#[test]
fn preserve_reports_journal_inactive() {
    let (fs, journal) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"old").unwrap();
    let mut tx = journal.begin(begin_request("write"));
    let preserved = tx
        .preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    assert_eq!(tx.op(), Some(OpId(1)));
    assert_eq!(preserved.len(), 1);
    assert_eq!(preserved[0].mode, PreserveMode::None);
    assert_eq!(
        preserved[0].irreversible,
        Some(Irreversible::JournalInactive)
    );
    assert_eq!(preserved[0].before.size, Some(3));
}

#[test]
fn preserve_apply_commit_changes_the_file() {
    let (fs, journal) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"old").unwrap();
    fs.set_mode(&file, 0o640).unwrap();
    let tag = fs.etag(&file, Follow::NoFollow).unwrap();

    let mut tx = journal.begin(begin_request("write"));
    let mut spec = fs_target(&file, TargetAction::Modify);
    spec.expect = Expect::Etag(tag);
    tx.preserve(&[spec]).unwrap();
    let applied = tx
        .apply(
            TargetIndex(0),
            Mutation::WriteBytes(b"new content".to_vec()),
        )
        .unwrap();
    assert_eq!(applied.after.size, Some(11));
    let handle = tx.commit().unwrap();

    assert_eq!(fs.read_file(&file).unwrap(), b"new content");
    assert_eq!(fs.metadata(&file, Follow::NoFollow).unwrap().mode, 0o640);
    assert_eq!(handle.op, Some(OpId(1)));
    assert_eq!(handle.targets.len(), 1);
    assert_eq!(handle.targets[0].mode, PreserveMode::None);
    assert_eq!(handle.irreversible.len(), 1);
    assert_eq!(handle.irreversible[0].reason, Irreversible::JournalInactive);
    assert_eq!(journal.committed(), [OpId(1)]);
    assert!(
        fs.mutations()
            .iter()
            .all(|record| record.scope_op == OpId(1))
    );
}

#[test]
fn rollback_restores_nothing_and_lists_changed_targets() {
    let (fs, journal) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"old").unwrap();
    let mut tx = journal.begin(begin_request("remove"));
    tx.preserve(&[fs_target(&file, TargetAction::Delete)])
        .unwrap();
    tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
    let report = tx.rollback(RollbackReason::VerifyFailed).unwrap();

    assert!(!fs.exists(&file));
    assert_eq!(report.op, Some(OpId(1)));
    assert!(report.restored.is_empty());
    assert_eq!(report.failed, [TargetPath::Fs(file)]);
    assert!(journal.calls().contains(&JournalCall::Rollback {
        op: Some(OpId(1)),
        reason: Some(RollbackReason::VerifyFailed),
    }));
}

#[test]
fn stale_entity_tag_fails_and_rolls_back() {
    let (fs, journal) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"old").unwrap();
    let tag = fs.etag(&file, Follow::NoFollow).unwrap();

    let mut tx = journal.begin(begin_request("write"));
    let mut spec = fs_target(&file, TargetAction::Modify);
    spec.expect = Expect::Etag(tag);
    tx.preserve(&[spec]).unwrap();
    fs.write_file(&file, b"changed elsewhere").unwrap();

    let error = tx
        .apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    let again = tx
        .apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
        .unwrap_err();
    assert_eq!(again.code(), ErrorCode::RolledBack);
    assert_eq!(tx.commit().unwrap_err().code(), ErrorCode::RolledBack);
    assert_eq!(fs.read_file(&file).unwrap(), b"changed elsewhere");
}

#[test]
fn preserve_validates_existence() {
    let (fs, journal) = setup();
    fs.write_file(&path("/data/a.txt"), b"a").unwrap();

    let mut tx = journal.begin(begin_request("write"));
    let error = tx
        .preserve(&[fs_target(&path("/data/a.txt"), TargetAction::Create)])
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Exists);

    let mut tx = journal.begin(begin_request("remove"));
    let error = tx
        .preserve(&[fs_target(&path("/data/missing.txt"), TargetAction::Delete)])
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn protocol_misuse_fails_with_stage_failed() {
    let (fs, journal) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"a").unwrap();

    let mut tx = journal.begin(begin_request("write"));
    tx.preserve(&[fs_target(&file, TargetAction::Delete)])
        .unwrap();
    let error = tx
        .apply(TargetIndex(0), Mutation::WriteBytes(Vec::new()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);

    let mut tx = journal.begin(begin_request("write"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    let error = tx.apply(TargetIndex(5), Mutation::Remove).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);

    let mut tx = journal.begin(begin_request("write"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    tx.apply(TargetIndex(0), Mutation::WriteBytes(b"b".to_vec()))
        .unwrap();
    let error = tx
        .preserve(&[fs_target(&path("/data/b.txt"), TargetAction::Create)])
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
}

#[test]
fn staged_content_is_switched_in() {
    let (fs, journal) = setup();
    let file = path("/data/new.txt");
    fs.create_dir_all(&path("/data")).unwrap();

    let mut tx = journal.begin(begin_request("write"));
    let mut spec = fs_target(&file, TargetAction::Create);
    spec.expect = Expect::Absent;
    tx.preserve(&[spec]).unwrap();
    let mut handle = tx.stage(TargetIndex(0)).unwrap();
    handle.write_all(b"streamed").unwrap();
    drop(handle);
    tx.apply(TargetIndex(0), Mutation::CommitStaged).unwrap();
    tx.commit().unwrap();

    assert_eq!(fs.read_file(&file).unwrap(), b"streamed");
    let listing: Vec<String> = fs
        .read_dir(&path("/data"))
        .unwrap()
        .map(|entry| entry.unwrap().name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(listing, ["new.txt"]);
}

#[test]
fn rollback_removes_staged_files() {
    let (fs, journal) = setup();
    fs.create_dir_all(&path("/data")).unwrap();
    let mut tx = journal.begin(begin_request("write"));
    tx.preserve(&[fs_target(&path("/data/new.txt"), TargetAction::Create)])
        .unwrap();
    drop(tx.stage(TargetIndex(0)).unwrap());
    tx.rollback(RollbackReason::Cancelled).unwrap();
    assert_eq!(fs.read_dir(&path("/data")).unwrap().count(), 0);
}

#[test]
fn rename_append_and_directories() {
    let (fs, journal) = setup();
    fs.write_file(&path("/data/log.txt"), b"one\n").unwrap();
    fs.write_file(&path("/data/old.txt"), b"x").unwrap();

    let mut tx = journal.begin(begin_request("move"));
    tx.preserve(&[
        fs_target(&path("/data/log.txt"), TargetAction::Append),
        fs_target(
            &path("/data/old.txt"),
            TargetAction::Rename {
                to: path("/data/renamed.txt"),
                replace: false,
            },
        ),
        fs_target(&path("/data/sub"), TargetAction::Create),
    ])
    .unwrap();
    tx.apply(TargetIndex(0), Mutation::AppendBytes(b"two\n".to_vec()))
        .unwrap();
    tx.apply(TargetIndex(1), Mutation::Rename).unwrap();
    tx.apply(TargetIndex(2), Mutation::CreateDir { mode: 0o700 })
        .unwrap();
    let handle = tx.commit().unwrap();

    assert_eq!(fs.read_file(&path("/data/log.txt")).unwrap(), b"one\ntwo\n");
    assert_eq!(fs.read_file(&path("/data/renamed.txt")).unwrap(), b"x");
    assert_eq!(
        fs.metadata(&path("/data/sub"), Follow::NoFollow)
            .unwrap()
            .mode,
        0o700
    );
    assert_eq!(handle.targets.len(), 3);
}

#[test]
fn transactions_without_changes_have_no_number() {
    let (_fs, journal) = setup();
    let handle = journal.begin(begin_request("write")).commit().unwrap();
    assert_eq!(handle.op, None);

    let mut tx = journal.begin(begin_request("net.request"));
    tx.mark_irreversible(IrreversibleEntry {
        reason: Irreversible::Network,
        path: None,
        count: None,
        detail: Some("POST https://example.com/".to_owned()),
    });
    let handle = tx.commit().unwrap();
    assert_eq!(handle.op, Some(OpId(1)));
    assert_eq!(handle.irreversible.len(), 1);
}

#[test]
fn undeclared_external_changes_are_irreversible() {
    let (fs, journal) = setup();
    let file = path("/work/out.bin");
    let mut tx = journal.begin(begin_request("run"));
    fs.write_file(&file, b"produced").unwrap();
    let applied = tx
        .observe_external(&[ExternalChange {
            path: file.clone(),
            kind: ExternalChangeKind::Created,
            declared: None,
        }])
        .unwrap();
    assert_eq!(applied[0].after.size, Some(8));
    let handle = tx.commit().unwrap();
    assert_eq!(
        handle.targets[0].irreversible,
        Some(Irreversible::PreviousContentNotKept)
    );
}

#[test]
fn operation_numbers_count_up_per_instance() {
    let fs = Arc::new(VirtualFs::default());
    fs.write_file(&path("/a"), b"").unwrap();
    let journal = NoopJournal::new(fs.clone());
    for expected in 1..=3 {
        let mut tx = journal.begin(begin_request("touch"));
        tx.preserve(&[fs_target(&path("/a"), TargetAction::Attributes)])
            .unwrap();
        assert_eq!(tx.op(), Some(OpId(expected)));
        tx.commit().unwrap();
    }
}

#[test]
fn store_queries_return_empty_results() {
    let (_fs, journal) = setup();
    assert!(
        journal
            .history(&HistoryQuery::default())
            .unwrap()
            .is_empty()
    );
    assert!(journal.status(None).unwrap().is_empty());
    assert!(journal.space(None).unwrap().is_empty());
    assert_eq!(journal.recover().unwrap(), Default::default());
    assert_eq!(journal.sweep().unwrap(), Default::default());
    let error = journal
        .undo(&UndoRequest {
            selector: UndoSelector::Last,
            on_conflict: Default::default(),
            session: None,
            program: empty_program(),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::JournalNoop);
    assert_eq!(
        journal.details(OpId(1)).unwrap_err().code(),
        ErrorCode::JournalNoop
    );
}

#[test]
fn apply_ops_preserves_every_target_before_the_first_change() {
    let (fs, journal) = setup();
    let old = path("/data/old.txt");
    let new = path("/data/new.txt");
    let moved = path("/data/moved.txt");
    fs.write_file(&old, b"old").unwrap();
    let mut tx = journal.begin(begin_request("write"));
    let outcomes = apply_ops(
        tx.as_mut(),
        vec![
            FileOp::write(old.clone(), b"changed".to_vec()),
            FileOp::create(new.clone(), b"created".to_vec()),
            FileOp::rename(old.clone(), moved.clone(), false),
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[1].applied.after.size, Some(7));
    assert!(outcomes[1].applied.after.meta.is_some());
    assert_eq!(fs.read_file(&new).unwrap(), b"created");
    assert_eq!(fs.read_file(&moved).unwrap(), b"changed");
    let calls = journal.calls();
    let first_apply = calls
        .iter()
        .position(|call| matches!(call, JournalCall::Apply { .. }))
        .unwrap();
    let preserves = calls
        .iter()
        .filter(|call| matches!(call, JournalCall::Preserve { .. }))
        .count();
    assert_eq!(preserves, 1);
    assert!(
        calls[..first_apply]
            .iter()
            .any(|call| matches!(call, JournalCall::Preserve { .. }))
    );
}

#[test]
fn apply_ops_rejects_a_stale_target() {
    let (fs, journal) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"old").unwrap();
    let mut tx = journal.begin(begin_request("write"));
    let error = apply_ops(
        tx.as_mut(),
        vec![FileOp::create(file.clone(), b"new".to_vec())],
    )
    .unwrap_err();
    assert!(matches!(error.code(), ErrorCode::Exists | ErrorCode::Stale));
    assert_eq!(fs.read_file(&file).unwrap(), b"old");
}
