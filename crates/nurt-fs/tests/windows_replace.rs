// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-fs/tests/windows_replace.rs
// Responsibility:   Replacement with a backup name (`ReplaceFileW`) on NTFS.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Replacement with a backup name (`ReplaceFileW`) on NTFS.
//!
//! Covers what the replacement takes from the replaced file (creation time,
//! attributes, named streams and DACL) and the names left by a
//! failed replacement. The tests that give the replaced file another account
//! need an elevated account holding `SeRestorePrivilege` and are ignored by
//! default.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod common;

use std::thread;
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, AlternateStream, Follow, Fs, FsError, FsErrorKind, FsWrite, MetaChange,
};
use nurt_fs::windows::WindowsFs;
use nurt_fs::windows::meta::attributes::HIDDEN;
use nurt_fs::windows::replace::ReplaceState;
use nurt_fs::windows::security::{SecurityDescriptor, security_differences};

use common::descriptor::{
    Builder, EVERYONE, FILE_ALL_ACCESS, FILE_GENERIC_READ, LOCAL_SYSTEM, LOCAL_SYSTEM_TEXT, allow,
};
use common::{
    Fixture, assert_held_by_this_process, fixture, fixture_with, hold_without_delete_sharing,
    privilege,
};

const OLD: &[u8] = b"old content";
const NEW: &[u8] = b"replacement content";
/// Creation time given to the replaced file (2020-09-13T12:26:40Z), a
/// multiple of the 100 ns `FILETIME` tick.
const CREATED_NS: i64 = 1_600_000_000_000_000_000;
/// Stream present only in the replaced file.
const MARKER_STREAM: &str = "nurt.marker";
const MARKER: &[u8] = b"stream of the replaced file";
/// Stream present in both files.
const SHARED_STREAM: &str = "nurt.shared";
const SHARED_OLD: &[u8] = b"shared stream of the replaced file";
const SHARED_NEW: &[u8] = b"shared stream of the replacement";
/// Retry period of the tests that hold a name until the period ends.
const SHORT_RETRY: Duration = Duration::from_millis(200);
/// Retry period of the tests that release a name before the period ends.
const LONG_RETRY: Duration = Duration::from_secs(5);
const RELEASE_AFTER: Duration = Duration::from_millis(150);

struct Names {
    target: AbsPath,
    replacement: AbsPath,
    backup: AbsPath,
}

fn names(fx: &Fixture) -> Names {
    Names {
        target: fx.file("target.txt", OLD),
        replacement: fx.file("replacement.txt", NEW),
        backup: fx.path("target.txt.bak"),
    }
}

fn replace(fx: &Fixture, names: &Names) -> Result<(), FsError> {
    fx.fs
        .replace_with_backup(&fx.scope, &names.target, &names.replacement, &names.backup)
}

fn sorted(mut streams: Vec<AlternateStream>) -> Vec<AlternateStream> {
    streams.sort_by(|a, b| a.name.cmp(&b.name));
    streams
}

/// Protected DACL that differs from the inherited one of a new file and keeps
/// full access for the account `owner`.
fn distinct_dacl(owner: &[u8]) -> Vec<u8> {
    Builder::default()
        .dacl(
            vec![
                allow(owner, FILE_ALL_ACCESS),
                allow(EVERYONE, FILE_GENERIC_READ),
            ],
            true,
        )
        .build()
}

#[test]
fn keeps_the_old_file_under_the_backup_name() {
    let fx = fixture();
    let names = names(&fx);
    let old_id = fx
        .fs
        .metadata(&names.target, Follow::NoFollow)
        .unwrap()
        .file_id;
    let new_id = fx
        .fs
        .metadata(&names.replacement, Follow::NoFollow)
        .unwrap()
        .file_id;
    replace(&fx, &names).unwrap();
    assert_eq!(fx.read(&names.target), NEW);
    assert_eq!(fx.read(&names.backup), OLD);
    assert!(!fx.exists(&names.replacement));
    let backup = fx.fs.metadata(&names.backup, Follow::NoFollow).unwrap();
    let target = fx.fs.metadata(&names.target, Follow::NoFollow).unwrap();
    assert_eq!(backup.file_id, old_id, "the replaced file was copied");
    assert_eq!(target.file_id, new_id, "the replacement was copied");
}

#[test]
fn replacement_takes_the_creation_time_of_the_replaced_file() {
    let fx = fixture();
    let names = names(&fx);
    let change = MetaChange {
        created_ns: Some(CREATED_NS),
        ..MetaChange::default()
    };
    fx.fs.set_meta(&fx.scope, &names.target, &change).unwrap();
    replace(&fx, &names).unwrap();
    let target = fx.fs.metadata(&names.target, Follow::NoFollow).unwrap();
    assert_eq!(target.created_ns, Some(CREATED_NS));
}

#[test]
fn replacement_takes_the_attributes_of_the_replaced_file() {
    let fx = fixture();
    let names = names(&fx);
    let change = MetaChange {
        set_windows_attributes: HIDDEN,
        ..MetaChange::default()
    };
    fx.fs.set_meta(&fx.scope, &names.target, &change).unwrap();
    let replacement = fx
        .fs
        .metadata(&names.replacement, Follow::NoFollow)
        .unwrap();
    assert_eq!(replacement.windows_attributes & HIDDEN, 0);
    replace(&fx, &names).unwrap();
    let target = fx.fs.metadata(&names.target, Follow::NoFollow).unwrap();
    assert_ne!(
        target.windows_attributes & HIDDEN,
        0,
        "attributes {:#x}",
        target.windows_attributes
    );
}

#[test]
fn replacement_receives_the_streams_it_lacks_and_keeps_its_own() {
    let fx = fixture();
    let names = names(&fx);
    fx.write_stream(&names.target, MARKER_STREAM, MARKER);
    fx.write_stream(&names.target, SHARED_STREAM, SHARED_OLD);
    fx.write_stream(&names.replacement, SHARED_STREAM, SHARED_NEW);
    replace(&fx, &names).unwrap();
    assert_eq!(fx.read_stream(&names.target, MARKER_STREAM), MARKER);
    assert_eq!(fx.read_stream(&names.target, SHARED_STREAM), SHARED_NEW);
    assert_eq!(fx.read_stream(&names.backup, MARKER_STREAM), MARKER);
}

/// `descriptor` without the control bit `SE_DACL_AUTO_INHERITED`.
///
/// `ReplaceFileW` keeps the entries and the protection of the DACL of the
/// replaced file and may set this bit on it; the journal then compares the
/// whole control and takes another strategy, which this test does not cover.
fn without_dacl_auto_inherited(descriptor: &[u8]) -> Vec<u8> {
    let mut descriptor = descriptor.to_vec();
    if let Some(high) = descriptor.get_mut(3) {
        *high &= !0x04;
    }
    descriptor
}

/// The journal compares account, group, DACL and streams after every
/// replacement and falls back to another strategy on any difference.
#[test]
fn keeps_account_group_dacl_and_streams_of_the_replaced_file() {
    let fx = fixture();
    let names = names(&fx);
    let owner = fx.account(&names.replacement);
    fx.set_descriptor(&names.target, distinct_dacl(&owner));
    fx.write_stream(&names.target, MARKER_STREAM, MARKER);
    let before = fx.extended(&names.target);
    let replacement = fx.extended(&names.replacement);
    let differences = security_differences(
        before.acl.as_deref().unwrap(),
        replacement.acl.as_deref().unwrap(),
    )
    .unwrap();
    assert!(
        differences.contains(&"dacl"),
        "the replacement must start with another DACL: {differences:?}"
    );
    replace(&fx, &names).unwrap();
    let after = fx.extended(&names.target);
    assert_eq!(after.account_sid, before.account_sid);
    let differences = security_differences(
        &without_dacl_auto_inherited(before.acl.as_deref().unwrap()),
        &without_dacl_auto_inherited(after.acl.as_deref().unwrap()),
    )
    .unwrap();
    let control = |acl: &[u8]| SecurityDescriptor::parse(acl).map(|sd| sd.control()).ok();
    assert!(
        differences.is_empty(),
        "ReplaceFileW changed {differences:?}: control {:?} before, {:?} after; descriptor {:02x?} before, {:02x?} after",
        control(before.acl.as_deref().unwrap()),
        control(after.acl.as_deref().unwrap()),
        before.acl.as_deref().unwrap(),
        after.acl.as_deref().unwrap()
    );
    assert_eq!(sorted(after.streams), sorted(before.streams));
}

/// Gives the replaced file the local system account, then replaces it with a
/// file of the running account. `privileged_replace` keeps
/// `SeRestorePrivilege` enabled during the replacement. `ReplaceFileW` carries
/// the DACL of the replaced file over but not its account: the replacement
/// keeps its own account, with or without the privilege, and the old account
/// stays with the file under the backup name. The journal detects the
/// difference with its comparison of the account identifiers and then selects
/// another way to keep the previous file.
fn assert_account_of_the_replacement_taken(privileged_replace: bool) {
    let fx = fixture();
    let names = names(&fx);
    let owner = fx.account(&names.replacement);
    let enabled = privilege::enable(privilege::RESTORE)
        .unwrap_or_else(|code| panic!("SeRestorePrivilege not available: error {code}"));
    let descriptor = Builder::default()
        .owner(LOCAL_SYSTEM)
        .dacl(
            vec![
                allow(&owner, FILE_ALL_ACCESS),
                allow(EVERYONE, FILE_GENERIC_READ),
            ],
            true,
        )
        .build();
    fx.set_descriptor(&names.target, descriptor);
    let before = fx.extended(&names.target);
    assert_eq!(before.account_sid.as_deref(), Some(LOCAL_SYSTEM_TEXT));
    let replacement_account = fx.extended(&names.replacement).account_sid;
    assert_ne!(replacement_account.as_deref(), Some(LOCAL_SYSTEM_TEXT));
    let held = if privileged_replace {
        Some(enabled)
    } else {
        drop(enabled);
        None
    };
    let result = replace(&fx, &names);
    drop(held);
    result.unwrap_or_else(|error| panic!("replacement of a file of another account: {error}"));
    let after = fx.extended(&names.target);
    assert_eq!(
        after.account_sid, replacement_account,
        "ReplaceFileW changed the account of the replacement"
    );
    let differences = security_differences(
        &without_dacl_auto_inherited(before.acl.as_deref().unwrap()),
        &without_dacl_auto_inherited(after.acl.as_deref().unwrap()),
    )
    .unwrap();
    let control = |acl: &[u8]| SecurityDescriptor::parse(acl).map(|sd| sd.control()).ok();
    assert!(
        !differences.contains(&"dacl"),
        "ReplaceFileW changed {differences:?}: control {:?} before, {:?} after",
        control(before.acl.as_deref().unwrap()),
        control(after.acl.as_deref().unwrap())
    );
    let backup = fx.extended(&names.backup);
    assert_eq!(backup.account_sid.as_deref(), Some(LOCAL_SYSTEM_TEXT));
}

#[test]
#[ignore = "requires an elevated account holding SeRestorePrivilege"]
fn takes_the_account_of_the_replacement_and_keeps_the_dacl_without_privilege() {
    assert_account_of_the_replacement_taken(false);
}

#[test]
#[ignore = "requires an elevated account holding SeRestorePrivilege"]
fn takes_the_account_of_the_replacement_and_keeps_the_dacl_with_restore_privilege() {
    assert_account_of_the_replacement_taken(true);
}

/// Verifies that a failed replacement left the old content under the name
/// given by its state and the replacement under its own name.
fn assert_old_content_kept(fx: &Fixture, names: &Names, result: Result<(), FsError>) -> FsError {
    let error = result.expect_err("replacement succeeded although a name was unavailable");
    let state = ReplaceState::of(&error)
        .unwrap_or_else(|| panic!("error without a replacement state: {error}"));
    assert_eq!(fx.read(&names.replacement), NEW, "{error}");
    match state {
        ReplaceState::Unchanged => assert_eq!(fx.read(&names.target), OLD, "{error}"),
        ReplaceState::OldAtBackup => {
            assert_eq!(fx.read(&names.backup), OLD, "{error}");
            assert!(!fx.exists(&names.target), "{error}");
        }
    }
    error
}

#[test]
fn reports_a_held_target_as_in_use_and_keeps_both_names() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(SHORT_RETRY));
    let names = names(&fx);
    let held = hold_without_delete_sharing(&names.target);
    let error = assert_old_content_kept(&fx, &names, replace(&fx, &names));
    assert_eq!(ReplaceState::of(&error), Some(ReplaceState::Unchanged));
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    assert_held_by_this_process(&error.holders);
    drop(held);
}

#[test]
fn reports_a_held_replacement_as_in_use_and_keeps_both_names() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(SHORT_RETRY));
    let names = names(&fx);
    let held = hold_without_delete_sharing(&names.replacement);
    let error = assert_old_content_kept(&fx, &names, replace(&fx, &names));
    assert_eq!(ReplaceState::of(&error), Some(ReplaceState::Unchanged));
    assert_eq!(error.kind, FsErrorKind::InUse, "{error}");
    assert_held_by_this_process(&error.holders);
    drop(held);
}

#[test]
fn keeps_the_old_content_when_the_backup_name_is_unavailable() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(SHORT_RETRY));
    let mut names = names(&fx);
    names.backup = fx
        .path("missing")
        .join_name("target.txt.bak")
        .expect("valid file name");
    assert_old_content_kept(&fx, &names, replace(&fx, &names));
}

#[test]
fn replaces_after_the_holder_closes_the_target_within_the_retry_period() {
    let fx = fixture_with(WindowsFs::with_sharing_retry(LONG_RETRY));
    let names = names(&fx);
    let held = hold_without_delete_sharing(&names.target);
    let releaser = thread::spawn(move || {
        thread::sleep(RELEASE_AFTER);
        drop(held);
    });
    replace(&fx, &names).unwrap();
    releaser.join().unwrap();
    assert_eq!(fx.read(&names.target), NEW);
    assert_eq!(fx.read(&names.backup), OLD);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
