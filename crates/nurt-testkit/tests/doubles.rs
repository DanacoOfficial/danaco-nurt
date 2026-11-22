// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the restoring journal, of names and special entries of the
//! virtual file system and of the host styles.

// Test code: a failed unwrap is the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::env::{Host, Os, PathStyle};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{FileKind, Follow, Fs};
use nurt_contracts::journal::{
    FileOp, JournalStore, PreserveMode, RollbackReason, TargetPath, apply_ops,
};
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::{RecordingJournal, TxState};
use nurt_testkit::snapshot::{CaptureOptions, CompareOptions, FsSnapshot, SnapshotEntry};
use nurt_testkit::vfs::{VirtualFs, VolumeConfig, test_path};

fn tree(fs: &VirtualFs) -> FsSnapshot {
    FsSnapshot::capture(
        fs,
        &test_path("/w").unwrap(),
        CaptureOptions {
            mode: true,
            mtime: false,
        },
    )
    .unwrap()
}

fn prepared() -> Arc<VirtualFs> {
    let fs = Arc::new(VirtualFs::default());
    FsSnapshot::from_entries(vec![
        SnapshotEntry::file("keep.txt", "keep").with_mode(0o600),
        SnapshotEntry::file("dir/a.txt", "a"),
        SnapshotEntry::file("dir/sub/b.txt", "b"),
        SnapshotEntry::symlink("dir/link", "a.txt"),
        SnapshotEntry::file("old.txt", "old"),
    ])
    .apply(&fs, &test_path("/w").unwrap())
    .unwrap();
    fs
}

fn ops() -> Vec<FileOp> {
    vec![
        FileOp::write(test_path("/w/keep.txt").unwrap(), b"changed".to_vec()),
        FileOp::remove(test_path("/w/dir").unwrap()),
        FileOp::rename(
            test_path("/w/old.txt").unwrap(),
            test_path("/w/new.txt").unwrap(),
            false,
        ),
        FileOp::create(test_path("/w/created.txt").unwrap(), b"fresh".to_vec()),
    ]
}

#[test]
fn rollback_restores_the_tree_before_the_operation() {
    let fs = prepared();
    let before = tree(&fs);
    let journal = RecordingJournal::keeping(fs.clone());
    let mut tx = journal.begin(begin_request("write"));
    let done = apply_ops(tx.as_mut(), ops()).unwrap();
    let modes: Vec<PreserveMode> = done.iter().map(|outcome| outcome.preserved.mode).collect();
    assert_eq!(
        modes,
        [
            PreserveMode::Copy,
            PreserveMode::Copy,
            PreserveMode::Rename,
            PreserveMode::Created
        ]
    );
    assert!(
        !tree(&fs)
            .compare(&before, CompareOptions::default())
            .is_empty()
    );
    let report = tx.rollback(RollbackReason::VerifyFailed).unwrap();
    assert_eq!(report.restored.len(), 4);
    assert!(report.failed.is_empty());
    assert_eq!(
        before.compare(&tree(&fs), CompareOptions::default()),
        Vec::new()
    );
    let record = &journal.transactions()[0];
    assert_eq!(record.state, TxState::RolledBack);
    assert_eq!(record.reason, Some(RollbackReason::VerifyFailed));
    assert_eq!(record.preserved.len(), 4);
    assert_eq!(record.applied.len(), 4);
}

#[test]
fn committed_operation_is_rolled_back_on_request() {
    let fs = prepared();
    let before = tree(&fs);
    let journal = RecordingJournal::keeping(fs.clone());
    let mut tx = journal.begin(begin_request("write"));
    apply_ops(tx.as_mut(), ops()).unwrap();
    let handle = tx.commit().unwrap();
    let op = handle.op.unwrap();
    assert!(handle.irreversible.is_empty());
    assert_eq!(journal.transactions()[0].state, TxState::Committed);

    let outcome = journal
        .rollback_committed(op, RollbackReason::VerifyFailed)
        .unwrap();
    assert_eq!(outcome.sources, [op]);
    assert_eq!(outcome.restored.len(), 4);
    assert!(
        before
            .compare(&tree(&fs), CompareOptions::default())
            .is_empty()
    );
    assert_eq!(journal.transactions()[0].state, TxState::RolledBack);
    let again = journal
        .rollback_committed(op, RollbackReason::VerifyFailed)
        .unwrap_err();
    assert_eq!(again.code(), ErrorCode::NotFound);
}

#[test]
fn injected_rollback_failure_reports_the_target() {
    let fs = prepared();
    let journal = RecordingJournal::keeping(fs.clone());
    let keep = test_path("/w/keep.txt").unwrap();
    journal.fail_rollback_of(TargetPath::Fs(keep.clone()));

    let mut tx = journal.begin(begin_request("write"));
    apply_ops(tx.as_mut(), ops()).unwrap();
    let handle = tx.commit().unwrap();
    let error = journal
        .rollback_committed(handle.op.unwrap(), RollbackReason::VerifyFailed)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RollbackFailed);
    assert_eq!(fs.read_file(&keep).unwrap(), b"changed");
    assert!(fs.exists(&test_path("/w/dir/sub/b.txt").unwrap()));
    let record = &journal.transactions()[0];
    assert_eq!(record.not_restored, [TargetPath::Fs(keep.clone())]);
    assert_eq!(record.restored.len(), 3);
}

#[test]
fn failed_call_rolls_back_and_escalates_when_incomplete() {
    let fs = prepared();
    let journal = RecordingJournal::keeping(fs.clone());
    let keep = test_path("/w/keep.txt").unwrap();
    journal.fail_rollback_of(TargetPath::Fs(keep.clone()));
    let mut tx = journal.begin(begin_request("write"));
    let failing = vec![
        FileOp::write(keep.clone(), b"changed".to_vec()),
        FileOp::remove(test_path("/w/missing.txt").unwrap()),
    ];
    let error = apply_ops(tx.as_mut(), failing).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    drop(tx);

    let mut tx = journal.begin(begin_request("write"));
    let first = apply_ops(
        tx.as_mut(),
        vec![FileOp::write(keep.clone(), b"changed".to_vec())],
    );
    assert!(first.is_ok());
    let error = tx
        .apply(
            nurt_contracts::journal::TargetIndex(0),
            nurt_contracts::journal::Mutation::Remove,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RollbackFailed);
    assert_eq!(
        tx.apply(
            nurt_contracts::journal::TargetIndex(0),
            nurt_contracts::journal::Mutation::WriteBytes(Vec::new())
        )
        .unwrap_err()
        .code(),
        ErrorCode::RolledBack
    );
}

#[test]
fn pass_through_journal_keeps_nothing() {
    let fs = prepared();
    let journal = RecordingJournal::new(fs.clone());
    let mut tx = journal.begin(begin_request("write"));
    let done = apply_ops(tx.as_mut(), ops()).unwrap();
    assert!(
        done.iter()
            .all(|outcome| outcome.preserved.mode == PreserveMode::None)
    );
    let report = tx.rollback(RollbackReason::Cancelled).unwrap();
    assert_eq!(report.failed.len(), 4);
    assert_eq!(
        fs.read_file(&test_path("/w/keep.txt").unwrap()).unwrap(),
        b"changed"
    );
}

#[test]
fn special_files_are_not_kept() {
    let fs = Arc::new(VirtualFs::default());
    let fifo = test_path("/w/fifo").unwrap();
    fs.create_special(&fifo).unwrap();
    assert_eq!(
        fs.metadata(&fifo, Follow::NoFollow).unwrap().kind,
        FileKind::Special
    );
    let journal = RecordingJournal::keeping(fs.clone());
    let mut tx = journal.begin(begin_request("remove"));
    let done = apply_ops(tx.as_mut(), vec![FileOp::remove(fifo.clone())]).unwrap();
    assert_eq!(done[0].preserved.mode, PreserveMode::None);
    assert!(done[0].preserved.irreversible.is_some());
    assert!(!fs.exists(&fifo));
}

#[test]
fn hard_links_share_content_and_count_links() {
    let fs = Arc::new(VirtualFs::default());
    let original = test_path("/w/one.txt").unwrap();
    let second = test_path("/w/two.txt").unwrap();
    fs.write_file(&original, b"shared").unwrap();
    let journal = RecordingJournal::new(fs.clone());
    let mut tx = journal.begin(begin_request("link"));
    apply_ops(
        tx.as_mut(),
        vec![FileOp::create_hard_link(second.clone(), original.clone())],
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(fs.metadata(&original, Follow::NoFollow).unwrap().links, 2);
    fs.write_file(&second, b"changed").unwrap();
    assert_eq!(fs.read_file(&original).unwrap(), b"changed");
}

#[test]
fn case_insensitive_volume_matches_names_in_any_case() {
    let fs = VirtualFs::default();
    let mut config = VolumeConfig::default();
    config.features.case_sensitive = false;
    fs.add_volume(&test_path("/ci").unwrap(), config).unwrap();
    fs.write_file(&test_path("/ci/Readme.md").unwrap(), b"r")
        .unwrap();
    assert_eq!(
        fs.read_file(&test_path("/ci/README.MD").unwrap()).unwrap(),
        b"r"
    );
    let names: Vec<String> = fs
        .read_dir(&test_path("/ci").unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["Readme.md"]);
    assert!(fs.read_file(&test_path("/README.MD").unwrap()).is_err());
}

#[test]
fn decomposing_volume_stores_names_in_nfd() {
    let fs = VirtualFs::default();
    let mut config = VolumeConfig::default();
    config.features.decomposes_names = true;
    fs.add_volume(&test_path("/hfs").unwrap(), config).unwrap();
    fs.write_file(&test_path("/hfs/caf\u{e9}").unwrap(), b"c")
        .unwrap();
    assert_eq!(
        fs.read_file(&test_path("/hfs/cafe\u{301}").unwrap())
            .unwrap(),
        b"c"
    );
    let names: Vec<String> = fs
        .read_dir(&test_path("/hfs").unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["cafe\u{301}"]);
    fs.write_file(&test_path("/plain/caf\u{e9}").unwrap(), b"p")
        .unwrap();
    assert!(
        fs.read_file(&test_path("/plain/cafe\u{301}").unwrap())
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn names_outside_utf8_are_kept_byte_for_byte() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let fs = VirtualFs::default();
    let mut config = VolumeConfig::default();
    config.features.case_sensitive = false;
    fs.add_volume(&test_path("/ci").unwrap(), config).unwrap();
    for dir in ["/w", "/ci"] {
        let name = OsStr::from_bytes(b"bad\xFFname");
        let path = nurt_contracts::fs::AbsPath::new(std::path::Path::new(dir).join(name)).unwrap();
        fs.write_file(&path, b"x").unwrap();
        let entry = fs
            .read_dir(&test_path(dir).unwrap())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(entry.name.as_bytes(), b"bad\xFFname");
        assert_eq!(fs.read_file(&path).unwrap(), b"x");
    }
    let error =
        FsSnapshot::capture(&fs, &test_path("/w").unwrap(), CaptureOptions::default()).unwrap_err();
    assert!(error.to_string().contains("UTF-8"));
}

#[test]
fn hosts_report_their_path_styles() {
    let styles: Vec<(Os, PathStyle)> = [TestHost::linux(), TestHost::macos(), TestHost::windows()]
        .iter()
        .map(|host| (host.info().os, host.info().path_style))
        .collect();
    assert_eq!(
        styles,
        [
            (Os::Linux, PathStyle::Posix),
            (Os::Macos, PathStyle::Posix),
            (Os::Windows, PathStyle::Windows)
        ]
    );
}
