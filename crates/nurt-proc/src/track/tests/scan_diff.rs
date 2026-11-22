// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection by scanning before and after.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Command;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::Tracking;
use nurt_contracts::report::{ChangeAction, Irreversible, Notable};

use super::support::{Harness, RecordingTx};
use crate::iface::{TrackMode, TrackSpec};

#[test]
fn reports_every_kind_of_change() {
    let h = Harness::new();
    h.write("modified.txt", "one");
    h.write("deleted.txt", "two");
    h.write("renamed.txt", "three");
    h.write("script.sh", "echo");
    h.write("target.txt", "target");
    h.write("file_to_dir", "file");
    h.write("dir_to_file/inner.txt", "inner");
    h.write("unchanged/kept.txt", "same");
    symlink("target.txt", h.path("link")).unwrap();
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write("modified.txt", "one and more");
        fs::remove_file(h.path("deleted.txt")).unwrap();
        fs::rename(h.path("renamed.txt"), h.path("moved.txt")).unwrap();
        h.chmod("script.sh", 0o755);
        fs::remove_file(h.path("link")).unwrap();
        symlink("modified.txt", h.path("link")).unwrap();
        fs::remove_file(h.path("file_to_dir")).unwrap();
        fs::create_dir(h.path("file_to_dir")).unwrap();
        fs::remove_dir_all(h.path("dir_to_file")).unwrap();
        h.write("dir_to_file", "now a file");
        h.write("created.txt", "new");
    });
    assert_eq!(
        h.actions(&outcome),
        vec![
            ("created.txt".to_owned(), ChangeAction::Create),
            ("deleted.txt".to_owned(), ChangeAction::Delete),
            ("dir_to_file".to_owned(), ChangeAction::Modify),
            ("dir_to_file/inner.txt".to_owned(), ChangeAction::Delete),
            ("file_to_dir".to_owned(), ChangeAction::Modify),
            ("link".to_owned(), ChangeAction::Link),
            ("modified.txt".to_owned(), ChangeAction::Modify),
            ("moved.txt".to_owned(), ChangeAction::Rename),
            ("script.sh".to_owned(), ChangeAction::Attributes),
        ]
    );
    let renamed = h.change(&outcome, "moved.txt");
    assert_eq!(
        renamed.from.as_ref().map(|from| h.relative(from.as_str())),
        Some("renamed.txt".to_owned())
    );
    let modified = h.change(&outcome, "modified.txt");
    assert_eq!(modified.size_before.map(|size| size.bytes()), Some(3));
    assert_eq!(modified.size_after.map(|size| size.bytes()), Some(12));
    assert_eq!(modified.store.as_deref(), Some("copy"));
    assert_eq!(
        h.change(&outcome, "created.txt").store.as_deref(),
        Some("created")
    );
    assert!(h.change(&outcome, "link").link_target.is_some());
    assert_eq!(outcome.tracking, Tracking::Scan);
    assert_eq!(outcome.watch, vec![h.root.clone()]);
    assert_eq!(outcome.notes[0].code, Notable::DetectionScope);
}

#[test]
fn undo_restores_previous_content_byte_for_byte() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    h.write("b.txt", "beta");
    h.write("sub/c.txt", "gamma");
    let mut tx = RecordingTx::new();
    h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write("a.txt", "alpha rewritten");
        fs::remove_file(h.path("b.txt")).unwrap();
        fs::rename(h.path("sub/c.txt"), h.path("c.txt")).unwrap();
        h.write("new.txt", "created");
    });
    tx.undo();
    assert_eq!(h.read("a.txt"), "alpha");
    assert_eq!(h.read("b.txt"), "beta");
    assert_eq!(h.read("sub/c.txt"), "gamma");
    assert!(!h.path("c.txt").exists());
    assert!(!h.path("new.txt").exists());
}

#[test]
fn rename_is_matched_by_file_identity() {
    let h = Harness::new();
    h.write("dir/one.txt", "1");
    h.write("dir/two.txt", "22");
    h.write("plain.txt", "plain");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        fs::rename(h.path("dir"), h.path("moved")).unwrap();
        fs::rename(h.path("plain.txt"), h.path("moved/plain.txt")).unwrap();
    });
    assert_eq!(
        h.actions(&outcome),
        vec![
            ("moved".to_owned(), ChangeAction::Rename),
            ("moved/plain.txt".to_owned(), ChangeAction::Rename),
        ]
    );
    tx.undo();
    assert_eq!(h.read("dir/one.txt"), "1");
    assert_eq!(h.read("dir/two.txt"), "22");
    assert_eq!(h.read("plain.txt"), "plain");
    assert!(!h.path("moved").exists());
}

#[test]
fn same_size_rewrite_is_found_by_content_hash() {
    let h = Harness::new();
    h.write("data.bin", "aaaa");
    let spec = TrackSpec {
        scan_hash: true,
        ..h.spec(TrackMode::Scan)
    };
    let mut tx = RecordingTx::new();
    let outcome = h.run(&spec, &mut tx, |h| h.write("data.bin", "bbbb"));
    let change = h.change(&outcome, "data.bin");
    assert_eq!(change.action, ChangeAction::Modify);
    let before = change.hash_before.clone().unwrap();
    let after = change.hash_after.clone().unwrap();
    assert!(before.starts_with("sha256:"));
    assert_ne!(before, after);
}

#[test]
fn no_changes_record_no_operation() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |_| {});
    assert!(outcome.changes.is_empty());
    assert!(tx.observed.is_empty());
    let handle = Box::new(tx).commit().unwrap();
    assert_eq!(handle.op, None);
}

#[test]
fn ignored_names_are_not_scanned() {
    let h = Harness::new();
    h.write(".git/HEAD", "ref: main");
    h.write("src/.git/config", "x");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write(".git/HEAD", "ref: other branch");
        h.write(".git/objects/new", "object");
        h.write("src/.git/config", "changed");
    });
    assert!(outcome.changes.is_empty());
    assert!(tx.declared.is_empty());
}

#[test]
fn extra_ignored_names_come_from_the_request() {
    let h = Harness::new();
    h.write("cache/entry", "1");
    let spec = TrackSpec {
        ignore: vec!["cache".to_owned()],
        ..h.spec(TrackMode::Scan)
    };
    let mut tx = RecordingTx::new();
    let outcome = h.run(&spec, &mut tx, |h| h.write("cache/entry", "22"));
    assert!(outcome.changes.is_empty());
}

#[test]
fn without_detection_nothing_is_scanned_or_kept() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::None), &mut tx, |h| {
        h.write("a.txt", "changed")
    });
    assert_eq!(outcome.tracking, Tracking::None);
    assert!(outcome.changes.is_empty());
    assert!(outcome.notes.is_empty());
    assert_eq!(tx.preserve_calls, 0);
}

#[test]
fn exact_files_are_kept_and_compared_without_detection() {
    let h = Harness::new();
    h.write("formatted.rs", "fn main(){}");
    h.write("other.rs", "x");
    let spec = TrackSpec {
        exact: Some(vec![h.path("formatted.rs")]),
        ..h.spec(TrackMode::None)
    };
    let mut tx = RecordingTx::new();
    let outcome = h.run(&spec, &mut tx, |h| {
        h.write("formatted.rs", "fn main() {}\n");
        h.write("other.rs", "changed");
    });
    assert_eq!(
        h.actions(&outcome),
        vec![("formatted.rs".to_owned(), ChangeAction::Modify)]
    );
    assert_eq!(tx.declared.len(), 1);
    tx.undo();
    assert_eq!(h.read("formatted.rs"), "fn main(){}");
}

#[test]
fn rejects_relative_watch_path() {
    let h = Harness::new();
    let spec = TrackSpec {
        watch: vec![PathBuf::from("relative/dir")],
        ..h.spec(TrackMode::Scan)
    };
    let mut tx = RecordingTx::new();
    let error = crate::track::tracker()
        .prepare(&h.ctx, &spec, &mut tx)
        .err()
        .unwrap();
    assert_eq!(error.code(), ErrorCode::Arg);
}

#[test]
fn unlistable_directory_is_reported() {
    let h = Harness::new();
    h.write("closed/secret.txt", "s");
    h.chmod("closed", 0o000);
    let readable = fs::read_dir(h.path("closed")).is_ok();
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |_| {});
    h.chmod("closed", 0o755);
    if !readable {
        assert!(
            outcome
                .notes
                .iter()
                .any(|note| note.code == Notable::PermissionDeniedSkipped)
        );
    }
}

/// Path of the test program built next to the test binaries.
fn fixture() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent()
        .and_then(|deps| deps.parent())
        .unwrap()
        .join("examples")
        .join("proc_fixture")
}

#[test]
fn changes_made_by_a_child_process_are_detected() {
    let h = Harness::new();
    h.write("keep.txt", "keep");
    h.write("edit.txt", "edit");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        let status = Command::new(fixture())
            .arg("track-mutate")
            .args([
                "write",
                h.path("edit.txt").to_str().unwrap(),
                "edited by child",
            ])
            .args(["write", h.path("out/new.txt").to_str().unwrap(), "new"])
            .args(["remove", h.path("keep.txt").to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success());
    });
    assert_eq!(
        h.actions(&outcome),
        vec![
            ("edit.txt".to_owned(), ChangeAction::Modify),
            ("keep.txt".to_owned(), ChangeAction::Delete),
            ("out".to_owned(), ChangeAction::Create),
            ("out/new.txt".to_owned(), ChangeAction::Create),
        ]
    );
    assert!(h.change(&outcome, "keep.txt").irreversible.is_none());
    tx.undo();
    assert_eq!(h.read("edit.txt"), "edit");
    assert_eq!(h.read("keep.txt"), "keep");
    assert!(!h.path("out").exists());
    assert_ne!(
        h.change(&outcome, "edit.txt").irreversible,
        Some(Irreversible::PreviousContentNotKept)
    );
}
