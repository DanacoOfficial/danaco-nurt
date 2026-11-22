// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reference semantics of the virtual file system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]
// The tests drive the mutating interface directly and stand in for the
// journal, which the scope constructor is reserved for.
#![allow(clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::Clock;
use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, Fs, FsErrorKind, FsWrite, Holder, LinkKind, MetaChange, RenameMode,
    ScopePurpose, TxScope,
};
use nurt_contracts::journal::OpId;
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::hash::sha256;
use nurt_testkit::vfs::{Fault, VirtualFs, VolumeConfig, test_path};

fn setup() -> (Arc<VirtualClock>, VirtualFs, TxScope) {
    let clock = Arc::new(VirtualClock::new());
    let fs = VirtualFs::new(clock.clone());
    let scope = TxScope::new_for_journal(OpId(7), ScopePurpose::Operation);
    (clock, fs, scope)
}

fn path(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn names(fs: &VirtualFs, dir: &str) -> Vec<String> {
    fs.read_dir(&path(dir))
        .unwrap()
        .map(|entry| entry.unwrap().name.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn creates_and_reads_a_file() {
    let (clock, fs, scope) = setup();
    fs.create_dir_all(&path("/data")).unwrap();
    let file = path("/data/new.txt");
    let mut handle = fs.create_new(&scope, &file).unwrap();
    handle.write_all(b"hello").unwrap();
    drop(handle);

    let meta = fs.metadata(&file, Follow::NoFollow).unwrap();
    assert_eq!(meta.kind, FileKind::File);
    assert_eq!(meta.size, 5);
    let now = i64::try_from(clock.now().unix_nanos()).unwrap();
    assert_eq!(meta.modified_ns, now);

    let mut content = String::new();
    fs.open_read(&file)
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert_eq!(content, "hello");
    assert_eq!(fs.content_hash(&file).unwrap(), sha256(b"hello"));
}

#[test]
fn create_new_fails_for_an_existing_file() {
    let (_clock, fs, scope) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"a").unwrap();
    let error = fs.create_new(&scope, &file).err().unwrap();
    assert_eq!(error.kind, FsErrorKind::Exists);
}

#[test]
fn missing_entries_and_parents_are_not_found() {
    let (_clock, fs, scope) = setup();
    let missing = fs.metadata(&path("/nothing"), Follow::Follow).unwrap_err();
    assert_eq!(missing.kind, FsErrorKind::NotFound);
    let no_parent = fs
        .create_new(&scope, &path("/no/parent.txt"))
        .err()
        .unwrap();
    assert_eq!(no_parent.kind, FsErrorKind::NotFound);
}

#[test]
fn a_file_used_as_a_directory_is_not_a_directory() {
    let (_clock, fs, _scope) = setup();
    fs.write_file(&path("/data/file"), b"x").unwrap();
    let error = fs
        .metadata(&path("/data/file/child"), Follow::Follow)
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::NotADirectory);
}

#[test]
fn read_dir_returns_byte_order() {
    let (_clock, fs, _scope) = setup();
    for name in ["b", "a", "C", "_"] {
        fs.write_file(&path(&format!("/dir/{name}")), b"").unwrap();
    }
    assert_eq!(names(&fs, "/dir"), ["C", "_", "a", "b"]);
}

#[test]
fn rename_keeps_identity_and_moves_subtrees() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/src/inner/file.txt"), b"data")
        .unwrap();
    let before = fs
        .metadata(&path("/src/inner/file.txt"), Follow::NoFollow)
        .unwrap();

    fs.rename(&scope, &path("/src"), &path("/dst"), RenameMode::NoReplace)
        .unwrap();

    assert!(!fs.exists(&path("/src")));
    let after = fs
        .metadata(&path("/dst/inner/file.txt"), Follow::NoFollow)
        .unwrap();
    assert_eq!(after.file_id, before.file_id);
    assert_eq!(fs.read_file(&path("/dst/inner/file.txt")).unwrap(), b"data");
}

#[test]
fn rename_without_replacement_fails_for_an_existing_destination() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/a"), b"a").unwrap();
    fs.write_file(&path("/b"), b"b").unwrap();
    let error = fs
        .rename(&scope, &path("/a"), &path("/b"), RenameMode::NoReplace)
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Exists);

    fs.rename(&scope, &path("/a"), &path("/b"), RenameMode::Replace)
        .unwrap();
    assert_eq!(fs.read_file(&path("/b")).unwrap(), b"a");
}

#[test]
fn rename_between_volumes_is_cross_device() {
    let (_clock, fs, scope) = setup();
    fs.add_volume(&path("/mnt/usb"), VolumeConfig::default())
        .unwrap();
    fs.write_file(&path("/a.txt"), b"a").unwrap();
    let error = fs
        .rename(
            &scope,
            &path("/a.txt"),
            &path("/mnt/usb/a.txt"),
            RenameMode::Replace,
        )
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::CrossDevice);
    assert!(
        !fs.same_volume(&path("/a.txt"), &path("/mnt/usb/x"))
            .unwrap()
    );
}

#[test]
fn removes_files_directories_and_trees() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/tree/a/one.txt"), b"1").unwrap();
    fs.write_file(&path("/tree/two.txt"), b"22").unwrap();

    let not_empty = fs.remove_dir(&scope, &path("/tree")).unwrap_err();
    assert_eq!(not_empty.kind, FsErrorKind::NotEmpty);
    let is_dir = fs.remove_file(&scope, &path("/tree/a")).unwrap_err();
    assert_eq!(is_dir.kind, FsErrorKind::IsADirectory);

    fs.remove_file(&scope, &path("/tree/two.txt")).unwrap();
    let stats = fs.remove_tree(&scope, &path("/tree"), 4).unwrap();
    assert_eq!((stats.files, stats.directories, stats.bytes), (1, 2, 1));
    assert!(!fs.exists(&path("/tree")));
}

#[test]
fn entity_tag_follows_content_changes() {
    let (clock, fs, scope) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"one").unwrap();
    let first = fs.etag(&file, Follow::NoFollow).unwrap();
    assert!(first.matches(&fs.etag(&file, Follow::NoFollow).unwrap()));

    clock.advance(Duration::from_secs(1));
    let mut handle = fs.open_in_place(&scope, &file).unwrap();
    handle.write_all(b"two").unwrap();
    drop(handle);

    let second = fs.etag(&file, Follow::NoFollow).unwrap();
    assert_eq!(second.size, first.size);
    assert_eq!(second.file_id, first.file_id);
    assert!(!first.matches(&second));
    assert_eq!(
        second.modified_ns - first.modified_ns,
        i64::try_from(Duration::from_secs(1).as_nanos()).unwrap()
    );
}

#[test]
fn file_identities_count_up_per_volume() {
    let (_clock, fs, _scope) = setup();
    fs.write_file(&path("/a"), b"").unwrap();
    fs.write_file(&path("/b"), b"").unwrap();
    let a = fs.metadata(&path("/a"), Follow::NoFollow).unwrap().file_id;
    let b = fs.metadata(&path("/b"), Follow::NoFollow).unwrap().file_id;
    assert_eq!((a.low, b.low), (2, 3));
}

#[test]
fn symbolic_links_resolve_and_loops_are_detected() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/real/file.txt"), b"x").unwrap();
    fs.symlink("/real", &path("/alias")).unwrap();
    assert_eq!(fs.read_file(&path("/alias/file.txt")).unwrap(), b"x");
    assert_eq!(
        fs.real_path(&path("/alias/file.txt")).unwrap(),
        path("/real/file.txt")
    );
    assert_eq!(
        fs.metadata(&path("/alias"), Follow::NoFollow).unwrap().kind,
        FileKind::Symlink
    );

    fs.create_link(
        &scope,
        std::path::Path::new("/loop-b"),
        &path("/loop-a"),
        LinkKind::SymlinkFile,
    )
    .unwrap();
    fs.create_link(
        &scope,
        std::path::Path::new("/loop-a"),
        &path("/loop-b"),
        LinkKind::SymlinkFile,
    )
    .unwrap();
    let error = fs.metadata(&path("/loop-a"), Follow::Follow).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Loop);
}

#[test]
fn faults_apply_once_unless_persistent() {
    let (_clock, fs, scope) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"a").unwrap();
    let holders = vec![Holder {
        pid: 42,
        name: "editor".to_owned(),
    }];
    fs.inject(Fault::in_use(&file, "remove_file", holders.clone()));
    let error = fs.remove_file(&scope, &file).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::InUse);
    assert_eq!(error.holders, holders);

    fs.inject(Fault::any_op(&file, FsErrorKind::Io).persistent());
    assert!(fs.content_hash(&file).is_err());
    assert!(fs.content_hash(&file).is_err());
    fs.clear_faults();
    fs.remove_file(&scope, &file).unwrap();
}

#[test]
fn short_reads_return_partial_chunks() {
    let (_clock, fs, _scope) = setup();
    let file = path("/data/a.txt");
    fs.write_file(&file, b"abcdef").unwrap();
    fs.inject(Fault::short_reads(&file, 2));
    let mut handle = fs.open_read(&file).unwrap();
    assert!(handle.as_bytes().is_none());
    let mut buffer = [0_u8; 6];
    assert_eq!(handle.read(&mut buffer).unwrap(), 2);
    let mut rest = Vec::new();
    handle.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, b"cdef");
}

#[test]
fn features_absent_from_the_volume_are_unsupported() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/a"), b"a").unwrap();
    let error = fs.clone_file(&scope, &path("/a"), &path("/b")).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    let error = fs
        .replace_with_backup(&scope, &path("/a"), &path("/a"), &path("/c"))
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Unsupported);
}

#[test]
fn exchange_swaps_names() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/a"), b"first").unwrap();
    fs.write_file(&path("/b"), b"second").unwrap();
    fs.exchange(&scope, &path("/a"), &path("/b")).unwrap();
    assert_eq!(fs.read_file(&path("/a")).unwrap(), b"second");
    assert_eq!(fs.read_file(&path("/b")).unwrap(), b"first");
}

#[test]
fn case_insensitive_volumes_keep_the_stored_spelling() {
    let clock = Arc::new(VirtualClock::new());
    let mut config = VolumeConfig::default();
    config.features.case_sensitive = false;
    let fs = VirtualFs::with_root(clock, config);
    fs.write_file(&path("/Docs/Readme.md"), b"x").unwrap();
    assert_eq!(fs.read_file(&path("/docs/README.md")).unwrap(), b"x");
    assert_eq!(
        fs.real_path(&path("/DOCS/readme.MD")).unwrap(),
        path("/Docs/Readme.md")
    );
    assert_eq!(
        fs.path_key(&path("/docs/readme.md")).unwrap(),
        fs.path_key(&path("/DOCS/README.MD")).unwrap()
    );
}

#[test]
fn write_permission_follows_mode_bits() {
    let (_clock, fs, scope) = setup();
    fs.create_dir_all(&path("/locked")).unwrap();
    fs.set_mode(&path("/locked"), 0o555).unwrap();
    assert!(!fs.can_write_dir(&path("/locked")).unwrap());
    let error = fs.create_new(&scope, &path("/locked/a")).err().unwrap();
    assert_eq!(error.kind, FsErrorKind::PermissionDenied);
    fs.set_account(0, 0);
    assert!(fs.can_write_dir(&path("/locked")).unwrap());
}

#[test]
fn capacity_limits_writes() {
    let clock = Arc::new(VirtualClock::new());
    let fs = VirtualFs::with_root(
        clock,
        VolumeConfig {
            capacity: Some(4),
            ..VolumeConfig::default()
        },
    );
    let scope = TxScope::new_for_journal(OpId(1), ScopePurpose::Operation);
    fs.create_dir_all(&path("/d")).unwrap();
    let mut handle = fs.create_new(&scope, &path("/d/a")).unwrap();
    assert!(handle.write_all(b"12345").is_err());
    let volume = fs.volume(&path("/d")).unwrap();
    assert_eq!(fs.space(&volume.id).unwrap().total, 4);
}

#[test]
fn mutations_are_recorded_with_their_scope() {
    let (_clock, fs, scope) = setup();
    fs.write_file(&path("/a"), b"a").unwrap();
    fs.set_meta(
        &scope,
        &path("/a"),
        &MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        },
    )
    .unwrap();
    fs.remove_file(&scope, &path("/a")).unwrap();
    let records = fs.mutations();
    let ops: Vec<&str> = records.iter().map(|record| record.op.as_str()).collect();
    assert_eq!(ops, ["set_meta", "remove_file"]);
    assert!(records.iter().all(|record| record.scope_op == OpId(7)));
}
