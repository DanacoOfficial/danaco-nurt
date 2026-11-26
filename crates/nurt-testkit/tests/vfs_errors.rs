// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Error paths and optional features of the virtual file system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]
#![allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, Durability, FileKind, Follow, Fs, FsErrorKind, FsFeatures, FsResult, FsWrite, Holder,
    LinkKind, MetaChange, RenameMode, ScopePurpose, TxScope, Xattr,
};
use nurt_contracts::journal::OpId;
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::vfs::{Fault, FaultEffect, VirtualFs, VolumeConfig, test_path};

fn path(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
}

fn fs_with(features: FsFeatures) -> VirtualFs {
    VirtualFs::with_root(
        Arc::new(VirtualClock::new()),
        VolumeConfig {
            features,
            ..VolumeConfig::default()
        },
    )
}

fn features(change: impl FnOnce(&mut FsFeatures)) -> FsFeatures {
    let mut features = VolumeConfig::default_features();
    change(&mut features);
    features
}

fn kind<T>(result: FsResult<T>) -> FsErrorKind {
    result.err().unwrap().kind
}

#[test]
fn test_path_rejects_relative_text() {
    assert_eq!(
        test_path("relative/path").unwrap_err().kind,
        FsErrorKind::InvalidPath
    );
}

#[test]
fn helpers_reject_wrong_entry_kinds() {
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/file"), b"f").unwrap();
    assert_eq!(
        kind(fs.create_dir_all(&path("/w/file/sub"))),
        FsErrorKind::NotADirectory
    );
    assert_eq!(
        kind(fs.write_file(&path("/w"), b"x")),
        FsErrorKind::IsADirectory
    );
    assert_eq!(kind(fs.symlink("x", &path("/w/file"))), FsErrorKind::Exists);
    assert_eq!(kind(fs.read_file(&path("/w"))), FsErrorKind::IsADirectory);
    assert_eq!(kind(fs.read_file(&path("/w/none"))), FsErrorKind::NotFound);
    assert_eq!(
        kind(fs.create_special(&path("/w/file"))),
        FsErrorKind::Exists
    );
    assert_eq!(
        kind(fs.set_ids(&path("/w/none"), 1, 1)),
        FsErrorKind::NotFound
    );
    fs.set_ids(&path("/w/file"), 5, 6).unwrap();
    assert_eq!(
        fs.metadata(&path("/w/file"), Follow::NoFollow).unwrap().uid,
        Some(5)
    );
    assert!(fs.root().is_ok());
}

#[test]
fn volumes_have_unique_mounts_and_report_holders() {
    let fs = VirtualFs::default();
    let mount = path("/mnt");
    fs.create_dir_all(&mount).unwrap();
    fs.add_volume(&mount, VolumeConfig::default()).unwrap();
    assert_eq!(
        kind(fs.add_volume(&mount, VolumeConfig::default())),
        FsErrorKind::Exists
    );
    assert_eq!(kind(fs.remove_entry(&mount)), FsErrorKind::InUse);
    let file = path("/mnt/a");
    fs.write_file(&file, b"a").unwrap();
    let holder = Holder {
        pid: 1,
        name: "p".to_owned(),
    };
    fs.set_holders(&file, vec![holder.clone()]);
    fs.set_holders(&file, vec![holder.clone(), holder.clone()]);
    assert_eq!(fs.holders(&file).unwrap().len(), 2);
    assert_eq!(kind(fs.holders(&path("/mnt/none"))), FsErrorKind::NotFound);
    assert!(fs.mutations().is_empty());
}

#[test]
fn capacity_limits_every_growing_operation() {
    let fs = VirtualFs::with_root(
        Arc::new(VirtualClock::new()),
        VolumeConfig {
            capacity: Some(16),
            ..VolumeConfig::default()
        },
    );
    let scope = scope();
    let file = path("/w/a");
    fs.write_file(&file, b"1234567890").unwrap();
    assert_eq!(
        kind(fs.copy_file(&scope, &file, &path("/w/b"))),
        FsErrorKind::NoSpace
    );
    assert_eq!(
        kind(fs.truncate(&scope, &file, 1_000)),
        FsErrorKind::NoSpace
    );
    let mut handle = fs.open_in_place(&scope, &file).unwrap();
    handle.seek(SeekFrom::End(0)).unwrap();
    let error = handle.write_all(&[0; 64]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(
        handle.set_length(1_000).unwrap_err().kind,
        FsErrorKind::NoSpace
    );
    handle.set_length(4).unwrap();
    handle.sync(Durability::Data).unwrap();
}

#[test]
fn streams_need_the_volume_feature() {
    let plain = VirtualFs::default();
    let scope = scope();
    plain.write_file(&path("/w/a"), b"a").unwrap();
    assert_eq!(
        kind(plain.open_stream_read(&path("/w/a"), "s")),
        FsErrorKind::Unsupported
    );
    assert_eq!(
        kind(plain.open_stream_write(&scope, &path("/w/a"), "s")),
        FsErrorKind::Unsupported
    );

    let streams = fs_with(features(|f| f.alternate_streams = true));
    streams.write_file(&path("/w/a"), b"a").unwrap();
    assert_eq!(
        kind(streams.open_stream_read(&path("/w/a"), "s")),
        FsErrorKind::NotFound
    );
    let mut writer = streams
        .open_stream_write(&scope, &path("/w/a"), "s")
        .unwrap();
    writer.write_all(b"data").unwrap();
    drop(writer);
    let mut reader = streams.open_stream_read(&path("/w/a"), "s").unwrap();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "data");
    streams.create_dir_all(&path("/w/dir")).unwrap();
    assert_eq!(
        kind(streams.open_stream_write(&scope, &path("/w/dir"), "s")),
        FsErrorKind::IsADirectory
    );
    assert_eq!(
        kind(streams.open_stream_read(&path("/w/dir"), "s")),
        FsErrorKind::IsADirectory
    );
    streams
        .copy_file(&scope, &path("/w/a"), &path("/w/b"))
        .unwrap();
}

#[test]
fn permissions_and_read_only_volumes_refuse_changes() {
    let scope = scope();
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.set_mode(&path("/w/a"), 0o444).unwrap();
    fs.set_mode(&path("/w"), 0o555).unwrap();
    fs.set_account(1000, 1000);
    fs.set_ids(&path("/w/a"), 0, 0).ok();
    assert_eq!(
        kind(fs.open_in_place(&scope, &path("/w/a"))),
        FsErrorKind::PermissionDenied
    );
    assert_eq!(
        kind(fs.create_dir(&scope, &path("/w/d"), 0o755)),
        FsErrorKind::PermissionDenied
    );
    assert!(!fs.can_write_dir(&path("/w")).unwrap());

    let read_only = VirtualFs::with_root(
        Arc::new(VirtualClock::new()),
        VolumeConfig {
            read_only: true,
            ..VolumeConfig::default()
        },
    );
    assert_eq!(
        kind(read_only.create_dir(&scope, &path("/d"), 0o755)),
        FsErrorKind::ReadOnly
    );
    assert_eq!(
        kind(read_only.set_meta(&scope, &path("/"), &MetaChange::default())),
        FsErrorKind::ReadOnly
    );
}

#[test]
fn the_root_cannot_be_created_or_replaced() {
    let fs = VirtualFs::default();
    let scope = scope();
    assert_eq!(
        kind(fs.create_dir(&scope, &path("/"), 0o755)),
        FsErrorKind::InUse
    );
    assert_eq!(kind(fs.remove_dir(&scope, &path("/"))), FsErrorKind::InUse);
}

#[test]
fn rename_validates_both_ends() {
    let fs = fs_with(features(|f| f.rename_noreplace = true));
    let scope = scope();
    fs.write_file(&path("/w/file"), b"f").unwrap();
    fs.write_file(&path("/w/d1/x"), b"x").unwrap();
    fs.write_file(&path("/w/d2/y"), b"y").unwrap();
    fs.create_dir_all(&path("/w/empty")).unwrap();
    let replace = RenameMode::Replace;
    let rename = |from: &str, to: &str, mode| fs.rename(&scope, &path(from), &path(to), mode);
    assert_eq!(
        kind(rename("/w/none", "/w/z", replace)),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(rename("/w/d1", "/w/d1/inner", replace)),
        FsErrorKind::InvalidPath
    );
    assert_eq!(
        kind(rename("/w/file", "/w/d1", replace)),
        FsErrorKind::IsADirectory
    );
    assert_eq!(
        kind(rename("/w/d1", "/w/file", replace)),
        FsErrorKind::NotADirectory
    );
    assert_eq!(
        kind(rename("/w/d1", "/w/d2", replace)),
        FsErrorKind::NotEmpty
    );
    assert_eq!(
        kind(rename("/w/file", "/w/d1/x", RenameMode::NoReplace)),
        FsErrorKind::Exists
    );
    rename("/w/d1", "/w/empty", replace).unwrap();
    rename("/w/file", "/w/empty/x", replace).unwrap();
    rename("/w/d2", "/w/renamed", replace).unwrap();
    rename("/w/renamed", "/w/renamed", replace).unwrap();
}

#[test]
fn rename_without_the_noreplace_feature_refuses_that_mode() {
    let fs = fs_with(features(|f| f.rename_noreplace = false));
    let scope = scope();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    assert_eq!(
        kind(fs.rename(&scope, &path("/w/a"), &path("/w/b"), RenameMode::NoReplace)),
        FsErrorKind::Unsupported
    );
    fs.rename(&scope, &path("/w/a"), &path("/w/b"), RenameMode::Replace)
        .unwrap();
}

#[test]
fn renaming_a_file_onto_another_name_of_the_same_inode_changes_nothing() {
    let fs = VirtualFs::default();
    let scope = scope();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.hard_link(&scope, &path("/w/a"), &path("/w/b")).unwrap();
    fs.rename(&scope, &path("/w/a"), &path("/w/b"), RenameMode::Replace)
        .unwrap();
    assert_eq!(fs.read_file(&path("/w/a")).unwrap(), b"a");
    assert_eq!(fs.read_file(&path("/w/b")).unwrap(), b"a");
}

#[test]
fn hashing_a_special_file_is_refused() {
    let fs = VirtualFs::default();
    fs.create_dir_all(&path("/w")).unwrap();
    fs.create_special(&path("/w/pipe")).unwrap();
    assert_eq!(
        kind(fs.content_hash(&path("/w/pipe"))),
        FsErrorKind::Unsupported
    );
}

#[test]
fn rename_across_volumes_is_cross_device() {
    let fs = VirtualFs::default();
    let scope = scope();
    fs.create_dir_all(&path("/mnt")).unwrap();
    fs.add_volume(&path("/mnt"), VolumeConfig::default())
        .unwrap();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.write_file(&path("/mnt/b"), b"b").unwrap();
    assert_eq!(
        kind(fs.rename(&scope, &path("/w/a"), &path("/mnt/c"), RenameMode::Replace)),
        FsErrorKind::CrossDevice
    );
    assert_eq!(
        kind(fs.hard_link(&scope, &path("/w/a"), &path("/mnt/c"))),
        FsErrorKind::CrossDevice
    );
    assert_eq!(
        kind(fs.exchange(&scope, &path("/w/a"), &path("/mnt/b"))),
        FsErrorKind::CrossDevice
    );
    let clone = fs_with(features(|f| f.reflink = true));
    clone.create_dir_all(&path("/mnt")).unwrap();
    clone
        .add_volume(
            &path("/mnt"),
            VolumeConfig {
                features: features(|f| f.reflink = true),
                ..VolumeConfig::default()
            },
        )
        .unwrap();
    clone.write_file(&path("/w/a"), b"a").unwrap();
    assert_eq!(
        kind(clone.clone_file(&scope, &path("/w/a"), &path("/mnt/c"))),
        FsErrorKind::CrossDevice
    );
}

#[test]
fn exchange_swaps_entries_and_validates_them() {
    let fs = VirtualFs::default();
    let scope = scope();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.write_file(&path("/w/b"), b"b").unwrap();
    fs.write_file(&path("/w/d/x"), b"x").unwrap();
    assert_eq!(
        kind(fs.exchange(&scope, &path("/w/none"), &path("/w/b"))),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(fs.exchange(&scope, &path("/w/a"), &path("/w/none"))),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(fs.exchange(&scope, &path("/w/d"), &path("/w/d/x"))),
        FsErrorKind::InvalidPath
    );
    fs.exchange(&scope, &path("/w/a"), &path("/w/b")).unwrap();
    assert_eq!(fs.read_file(&path("/w/a")).unwrap(), b"b");
    let without = fs_with(features(|f| f.exchange = false));
    without.write_file(&path("/w/a"), b"a").unwrap();
    without.write_file(&path("/w/b"), b"b").unwrap();
    assert_eq!(
        kind(without.exchange(&scope, &path("/w/a"), &path("/w/b"))),
        FsErrorKind::Unsupported
    );
}

#[test]
fn replace_with_backup_needs_the_feature_and_files() {
    let scope = scope();
    let plain = VirtualFs::default();
    plain.write_file(&path("/w/t"), b"t").unwrap();
    assert_eq!(
        kind(plain.replace_with_backup(&scope, &path("/w/t"), &path("/w/r"), &path("/w/b"))),
        FsErrorKind::Unsupported
    );
    let fs = fs_with(features(|f| f.replace_with_backup = true));
    fs.write_file(&path("/w/t"), b"t").unwrap();
    fs.write_file(&path("/w/r"), b"r").unwrap();
    fs.create_dir_all(&path("/w/dir")).unwrap();
    let call = |target: &str, replacement: &str, backup: &str| {
        fs.replace_with_backup(&scope, &path(target), &path(replacement), &path(backup))
    };
    assert_eq!(kind(call("/w/none", "/w/r", "/w/b")), FsErrorKind::NotFound);
    assert_eq!(kind(call("/w/t", "/w/none", "/w/b")), FsErrorKind::NotFound);
    assert_eq!(
        kind(call("/w/dir", "/w/r", "/w/b")),
        FsErrorKind::IsADirectory
    );
    call("/w/t", "/w/r", "/w/b").unwrap();
    assert_eq!(fs.read_file(&path("/w/t")).unwrap(), b"r");
    assert_eq!(fs.read_file(&path("/w/b")).unwrap(), b"t");

    fs.create_dir_all(&path("/mnt")).unwrap();
    fs.add_volume(
        &path("/mnt"),
        VolumeConfig {
            features: features(|f| f.replace_with_backup = true),
            ..VolumeConfig::default()
        },
    )
    .unwrap();
    fs.write_file(&path("/mnt/r"), b"r").unwrap();
    assert_eq!(
        kind(call("/w/t", "/mnt/r", "/w/b2")),
        FsErrorKind::CrossDevice
    );
}

#[test]
fn links_and_copies_validate_their_arguments() {
    let scope = scope();
    let fs = fs_with(features(|f| f.reflink = true));
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.create_dir_all(&path("/w/dir")).unwrap();
    assert_eq!(
        kind(fs.hard_link(&scope, &path("/w/dir"), &path("/w/h"))),
        FsErrorKind::PermissionDenied
    );
    assert_eq!(
        kind(fs.hard_link(&scope, &path("/w/none"), &path("/w/h"))),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(fs.hard_link(&scope, &path("/w/a"), &path("/w/dir"))),
        FsErrorKind::Exists
    );
    assert_eq!(
        kind(fs.copy_file(&scope, &path("/w/dir"), &path("/w/c"))),
        FsErrorKind::IsADirectory
    );
    assert_eq!(
        kind(fs.copy_file(&scope, &path("/w/none"), &path("/w/c"))),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(fs.clone_file(&scope, &path("/w/a"), &path("/w/dir"))),
        FsErrorKind::Exists
    );
    fs.clone_file(&scope, &path("/w/a"), &path("/w/clone"))
        .unwrap();
    assert_eq!(fs.read_file(&path("/w/clone")).unwrap(), b"a");
    assert_eq!(
        kind(fs.create_link(&scope, "x".as_ref(), &path("/w/a"), LinkKind::SymlinkFile)),
        FsErrorKind::Exists
    );
    fs.create_link(&scope, "dir".as_ref(), &path("/w/j"), LinkKind::Junction)
        .unwrap();
    assert_eq!(
        fs.metadata(&path("/w/j"), Follow::NoFollow).unwrap().kind,
        FileKind::Junction
    );

    let none = fs_with(features(|f| {
        f.hardlink = false;
        f.reflink = false;
    }));
    none.write_file(&path("/w/a"), b"a").unwrap();
    assert_eq!(
        kind(none.hard_link(&scope, &path("/w/a"), &path("/w/h"))),
        FsErrorKind::Unsupported
    );
    assert_eq!(
        kind(none.clone_file(&scope, &path("/w/a"), &path("/w/c"))),
        FsErrorKind::Unsupported
    );
}

#[test]
fn a_clone_keeps_the_mode_but_not_the_extended_attributes() {
    let scope = scope();
    let fs = fs_with(features(|f| f.reflink = true));
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.set_mode(&path("/w/a"), 0o640).unwrap();
    let with_xattr = MetaChange {
        set_xattrs: vec![Xattr {
            name: "user.k".into(),
            value: b"v".to_vec(),
        }],
        ..MetaChange::default()
    };
    fs.set_meta(&scope, &path("/w/a"), &with_xattr).unwrap();
    fs.clone_file(&scope, &path("/w/a"), &path("/w/c")).unwrap();
    let meta = fs
        .extended_metadata(&path("/w/c"), Follow::NoFollow)
        .unwrap();
    assert_eq!(meta.basic.mode & 0o777, 0o640);
    assert!(meta.xattrs.is_empty());
    assert_eq!(
        fs.extended_metadata(&path("/w/a"), Follow::NoFollow)
            .unwrap()
            .xattrs
            .len(),
        1
    );
}

#[test]
fn removal_and_metadata_changes_validate_their_targets() {
    let scope = scope();
    let fs = fs_with(features(|f| {
        f.xattr = false;
        f.acl = false;
    }));
    fs.write_file(&path("/w/a"), b"a").unwrap();
    fs.write_file(&path("/w/d/x"), b"x").unwrap();
    assert_eq!(
        kind(fs.remove_file(&scope, &path("/w/d"))),
        FsErrorKind::IsADirectory
    );
    assert_eq!(
        kind(fs.remove_dir(&scope, &path("/w/none"))),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(fs.remove_dir(&scope, &path("/w/a"))),
        FsErrorKind::NotADirectory
    );
    assert_eq!(
        kind(fs.remove_tree(&scope, &path("/w/none"), 1)),
        FsErrorKind::NotFound
    );
    assert_eq!(
        kind(fs.truncate(&scope, &path("/w/d"), 0)),
        FsErrorKind::IsADirectory
    );
    assert_eq!(
        kind(fs.truncate(&scope, &path("/w/none"), 0)),
        FsErrorKind::NotFound
    );
    let with_xattr = MetaChange {
        set_xattrs: vec![Xattr {
            name: "user.k".into(),
            value: b"v".to_vec(),
        }],
        ..MetaChange::default()
    };
    assert_eq!(
        kind(fs.set_meta(&scope, &path("/w/a"), &with_xattr)),
        FsErrorKind::Unsupported
    );
    let with_acl = MetaChange {
        acl: Some(vec![1]),
        ..MetaChange::default()
    };
    assert_eq!(
        kind(fs.set_meta(&scope, &path("/w/a"), &with_acl)),
        FsErrorKind::Unsupported
    );
    let full = fs_with(VolumeConfig::default_features());
    full.write_file(&path("/w/a"), b"a").unwrap();
    let change = MetaChange {
        mode: Some(0o600),
        uid: Some(3),
        gid: Some(4),
        modified_ns: Some(5),
        accessed_ns: Some(6),
        created_ns: Some(7),
        set_xattrs: vec![
            Xattr {
                name: "user.k".into(),
                value: b"v".to_vec(),
            },
            Xattr {
                name: "user.k".into(),
                value: b"w".to_vec(),
            },
        ],
        remove_xattrs: vec!["user.gone".into()],
        set_windows_attributes: 1,
        clear_windows_attributes: 2,
        ..MetaChange::default()
    };
    full.set_meta(&scope, &path("/w/a"), &change).unwrap();
    let extended = full
        .extended_metadata(&path("/w/a"), Follow::NoFollow)
        .unwrap();
    assert_eq!(extended.xattrs.len(), 1);
    assert_eq!(extended.basic.uid, Some(3));
    let other = fs_with(features(|f| {
        f.xattr = false;
        f.acl = false;
    }));
    other.write_file(&path("/w/b"), b"b").unwrap();
    let report = other
        .restore_meta(&scope, &path("/w/b"), &extended)
        .unwrap();
    assert!(!report.is_complete());
    let mut rich = extended.clone();
    rich.acl = Some(vec![9]);
    rich.account_sid = Some("S-1".to_owned());
    let report = other.restore_meta(&scope, &path("/w/b"), &rich).unwrap();
    assert!(!report.is_complete());
    other.mark_hidden(&scope, &path("/w/b")).unwrap();
    assert_ne!(
        other
            .metadata(&path("/w/b"), Follow::NoFollow)
            .unwrap()
            .windows_attributes,
        0
    );
}

#[test]
fn directories_and_locks_validate_their_targets() {
    let scope = scope();
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    assert_eq!(
        kind(fs.sync_dir(&scope, &path("/w/a"))),
        FsErrorKind::NotADirectory
    );
    fs.sync_dir(&scope, &path("/w")).unwrap();
    assert_eq!(
        kind(fs.lock_exclusive(&path("/w"), Duration::ZERO)),
        FsErrorKind::IsADirectory
    );
    let held = fs.lock_exclusive(&path("/w/new"), Duration::ZERO).unwrap();
    assert!(fs.exists(&path("/w/new")));
    assert_eq!(
        kind(fs.lock_exclusive(&path("/w/new"), Duration::ZERO)),
        FsErrorKind::InUse
    );
    drop(held);
    assert_eq!(
        kind(fs.can_write_dir(&path("/w/a"))),
        FsErrorKind::NotADirectory
    );
    assert_eq!(
        kind(fs.can_write_dir(&path("/w/none"))),
        FsErrorKind::NotFound
    );
}

#[test]
fn handles_seek_and_apply_metadata() {
    let scope = scope();
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/a"), b"abcdef").unwrap();
    let mut reader = fs.open_read(&path("/w/a")).unwrap();
    assert!(reader.as_bytes().is_some());
    assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), 4);
    assert_eq!(reader.seek(SeekFrom::Current(1)).unwrap(), 5);
    assert!(reader.seek(SeekFrom::Current(-10)).is_err());
    assert!(reader.seek(SeekFrom::End(-10)).is_err());
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, b"f");

    fs.inject(Fault::short_reads(&path("/w/a"), 2));
    let mut short = fs.open_read(&path("/w/a")).unwrap();
    assert!(short.as_bytes().is_none());
    assert_eq!(short.read(&mut [0; 8]).unwrap(), 2);
    assert_eq!(short.meta().unwrap().size, 6);

    let mut writer = fs.open_in_place(&scope, &path("/w/a")).unwrap();
    assert_eq!(writer.seek(SeekFrom::Current(2)).unwrap(), 2);
    assert!(writer.seek(SeekFrom::Current(-5)).is_err());
    let meta = fs
        .extended_metadata(&path("/w/a"), Follow::NoFollow)
        .unwrap();
    let report = writer.apply_meta(&meta).unwrap();
    assert!(report.is_complete());
    writer.flush().unwrap();
    assert_eq!(writer.path(), &path("/w/a"));
}

#[test]
fn faults_apply_once_or_persistently() {
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/a"), b"a").unwrap();
    let file = path("/w/a");
    fs.inject(Fault::any_op(&file, FsErrorKind::Io));
    assert_eq!(kind(fs.metadata(&file, Follow::NoFollow)), FsErrorKind::Io);
    assert!(fs.metadata(&file, Follow::NoFollow).is_ok());
    fs.inject(
        Fault::in_use(
            &file,
            "open_read",
            vec![Holder {
                pid: 2,
                name: "x".to_owned(),
            }],
        )
        .persistent(),
    );
    for _ in 0..2 {
        let error = fs.open_read(&file).err().unwrap();
        assert_eq!(error.kind, FsErrorKind::InUse);
        assert_eq!(error.holders.len(), 1);
    }
    fs.clear_faults();
    assert!(fs.open_read(&file).is_ok());
    fs.inject(Fault {
        path: file.clone(),
        op: None,
        effect: FaultEffect::ShortReads { max: 1 },
        persistent: false,
    });
    assert!(fs.metadata(&file, Follow::NoFollow).is_ok());
}

#[test]
fn path_resolution_handles_parent_components_and_links() {
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/a/file"), b"f").unwrap();
    fs.symlink("a", &path("/w/link")).unwrap();
    fs.symlink("../a/file", &path("/w/a/up")).unwrap();
    assert_eq!(fs.read_file(&path("/w/link/file")).unwrap(), b"f");
    assert_eq!(fs.read_file(&path("/w/a/up")).unwrap(), b"f");
    let sneaky = AbsPath::new("/w/a/../a/file").unwrap();
    assert_eq!(fs.metadata(&sneaky, Follow::Follow).unwrap().size, 1);
    assert_eq!(fs.real_path(&path("/w/link")).unwrap(), path("/w/a"));
    assert!(fs.path_key(&path("/w/a")).is_ok());
}
