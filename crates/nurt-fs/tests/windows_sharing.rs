// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-fs/tests/windows_sharing.rs
// Responsibility:   Renames, removals and sharing violations on NTFS.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Renames, removals and sharing violations on NTFS.
//!
//! Renames that replace use `FileRenameInfoEx` with POSIX semantics and
//! removals use `FileDispositionInfoEx` with POSIX semantics, so a destination
//! or a file open with `FILE_SHARE_DELETE` does not block them. A handle
//! without `FILE_SHARE_DELETE` blocks them; the back end retries for the
//! configured period and then reports the entry in use with the processes the
//! Restart Manager lists.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod common;

use std::io::Read;
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::fs::{Follow, Fs, FsErrorKind, FsWrite, MetaChange, RenameMode};
use nurt_fs::windows::WindowsFs;
use nurt_fs::windows::meta::attributes::READONLY;

use common::{
    assert_held_by_this_process, fixture, fixture_with, hold_with_delete_sharing,
    hold_without_delete_sharing,
};

const OLD: &[u8] = b"old content";
const NEW: &[u8] = b"new content";
/// Retry period of the tests that hold a name until the period ends.
const SHORT_RETRY: Duration = Duration::from_millis(400);
/// Retry period of the tests that release a name before the period ends.
const LONG_RETRY: Duration = Duration::from_secs(5);
const RELEASE_AFTER: Duration = Duration::from_millis(150);

#[test]
fn posix_rename_replaces_a_destination_open_with_delete_sharing() {
    let fx = fixture();
    let destination = fx.file("destination.txt", OLD);
    let source = fx.file("source.txt", NEW);
    let source_id = fx.fs.metadata(&source, Follow::NoFollow).unwrap().file_id;
    let mut reader = hold_with_delete_sharing(&destination);
    fx.fs
        .rename(&fx.scope, &source, &destination, RenameMode::Replace)
        .unwrap();
    assert_eq!(fx.read(&destination), NEW);
    assert!(!fx.exists(&source));
    let renamed = fx.fs.metadata(&destination, Follow::NoFollow).unwrap();
    assert_eq!(renamed.file_id, source_id);
    let mut previous = Vec::new();
    reader.read_to_end(&mut previous).unwrap();
    assert_eq!(previous, OLD, "the open handle keeps the replaced file");
}

#[test]
fn rename_without_replacement_keeps_both_entries() {
    let fx = fixture();
    let destination = fx.file("destination.txt", OLD);
    let source = fx.file("source.txt", NEW);
    let error = fx
        .fs
        .rename(&fx.scope, &source, &destination, RenameMode::NoReplace)
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Exists, "{error}");
    assert_eq!(fx.read(&destination), OLD);
    assert_eq!(fx.read(&source), NEW);
    let free = fx.path("free.txt");
    fx.fs
        .rename(&fx.scope, &source, &free, RenameMode::NoReplace)
        .unwrap();
    assert_eq!(fx.read(&free), NEW);
    assert!(!fx.exists(&source));
}

#[test]
fn rename_onto_a_destination_held_without_delete_sharing_reports_in_use() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(SHORT_RETRY));
    let destination = fx.file("destination.txt", OLD);
    let source = fx.file("source.txt", NEW);
    let held = hold_without_delete_sharing(&destination);
    let error = fx
        .fs
        .rename(&fx.scope, &source, &destination, RenameMode::Replace)
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    assert_held_by_this_process(&error.holders);
    assert_eq!(fx.read(&destination), OLD);
    assert_eq!(fx.read(&source), NEW);
    drop(held);
}

#[test]
fn rename_of_a_source_held_without_delete_sharing_reports_in_use() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(SHORT_RETRY));
    let source = fx.file("source.txt", NEW);
    let destination = fx.path("destination.txt");
    let held = hold_without_delete_sharing(&source);
    let error = fx
        .fs
        .rename(&fx.scope, &source, &destination, RenameMode::NoReplace)
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    assert!(fx.exists(&source));
    assert!(!fx.exists(&destination));
    drop(held);
}

#[test]
fn removal_frees_the_name_of_a_file_open_with_delete_sharing() {
    let fx = fixture();
    let path = fx.file("open.txt", OLD);
    let mut reader = hold_with_delete_sharing(&path);
    fx.fs.remove_file(&fx.scope, &path).unwrap();
    assert!(!fx.exists(&path), "the name stays pending deletion");
    fx.write_new(&path, NEW);
    assert_eq!(fx.read(&path), NEW);
    let mut previous = Vec::new();
    reader.read_to_end(&mut previous).unwrap();
    assert_eq!(previous, OLD);
}

#[test]
fn removes_a_read_only_file() {
    let fx = fixture();
    let path = fx.file("read-only.txt", OLD);
    let change = MetaChange {
        set_windows_attributes: READONLY,
        ..MetaChange::default()
    };
    fx.fs.set_meta(&fx.scope, &path, &change).unwrap();
    let meta = fx.fs.metadata(&path, Follow::NoFollow).unwrap();
    assert_ne!(meta.windows_attributes & READONLY, 0);
    fx.fs.remove_file(&fx.scope, &path).unwrap();
    assert!(!fx.exists(&path));
}

#[test]
fn removal_of_a_held_file_reports_in_use_after_the_retry_period() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(SHORT_RETRY));
    let path = fx.file("held.txt", OLD);
    let held = hold_without_delete_sharing(&path);
    let started = Instant::now();
    let error = fx.fs.remove_file(&fx.scope, &path).unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    assert!(error.os_code.is_some(), "{error:?}");
    assert!(
        elapsed >= SHORT_RETRY / 2,
        "gave up after {elapsed:?} of a {SHORT_RETRY:?} retry period"
    );
    assert_held_by_this_process(&error.holders);
    assert_eq!(fx.read(&path), OLD);
    drop(held);
}

#[test]
fn removal_succeeds_when_the_holder_closes_within_the_retry_period() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(LONG_RETRY));
    let path = fx.file("held.txt", OLD);
    let held = hold_without_delete_sharing(&path);
    let releaser = thread::spawn(move || {
        thread::sleep(RELEASE_AFTER);
        drop(held);
    });
    fx.fs.remove_file(&fx.scope, &path).unwrap();
    releaser.join().unwrap();
    assert!(!fx.exists(&path));
}

#[test]
fn zero_retry_period_reports_in_use_at_once() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(Duration::ZERO));
    let path = fx.file("held.txt", OLD);
    let held = hold_without_delete_sharing(&path);
    let started = Instant::now();
    let error = fx.fs.remove_file(&fx.scope, &path).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    assert!(started.elapsed() < LONG_RETRY);
    drop(held);
}

#[test]
fn holders_lists_this_process_while_a_handle_is_open() {
    let fx = fixture();
    let path = fx.file("open.txt", OLD);
    let held = hold_with_delete_sharing(&path);
    assert_held_by_this_process(&fx.fs.holders(&path).unwrap());
    drop(held);
    let pid = std::process::id();
    let holders = fx.fs.holders(&path).unwrap();
    assert!(
        holders.iter().all(|holder| holder.pid != pid),
        "{holders:?}"
    );
}

#[test]
fn lock_with_the_longest_timeout_is_taken_when_the_file_is_free() {
    let fx = fixture();
    let path = fx.path("lock");
    let lock = fx.fs.lock_exclusive(&path, Duration::MAX).unwrap();
    assert_eq!(lock.path(), &path);
}

#[test]
fn lock_with_zero_timeout_reports_in_use_while_it_is_held() {
    let fx = fixture();
    let path = fx.path("lock");
    let first = fx.fs.lock_exclusive(&path, Duration::ZERO).unwrap();
    let error = fx.fs.lock_exclusive(&path, Duration::ZERO).err().unwrap();
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    drop(first);
    fx.fs.lock_exclusive(&path, Duration::ZERO).unwrap();
}

#[test]
fn holders_of_a_missing_file_is_not_found() {
    let fx = fixture();
    let error = fx.fs.holders(&fx.path("missing.txt")).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::NotFound, "{error}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
