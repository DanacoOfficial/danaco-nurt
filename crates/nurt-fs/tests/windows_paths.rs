// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-fs/tests/windows_paths.rs
// Responsibility:   Verbatim paths, reserved names, long paths and tree removal across reparse points.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Verbatim paths, reserved names, long paths and tree removal across
//! reparse points.
//!
//! Every system call receives a `\\?\` path, so names that Win32 path
//! parsing reserves or alters (device names, trailing dots and spaces) and
//! paths longer than `MAX_PATH` are created and removed unchanged.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod common;

use std::ffi::OsString;
use std::path::PathBuf;

use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, Fs, FsErrorKind, FsWrite, LinkKind, RenameMode,
};

use common::{DIR_MODE, Fixture, fixture};

const CONTENT: &[u8] = b"content";
/// Names reserved for devices or altered by Win32 path parsing.
const RESERVED_NAMES: [&str; 6] = ["NUL.txt", "CON", "aux.log", "COM1", "trailing.", "space "];
/// Path length limit of Win32 calls without the `\\?\` prefix.
const MAX_PATH: usize = 260;
const SEGMENT_LEN: usize = 60;
const DEPTH: usize = 5;

/// The same path in verbatim form, which the standard library passes to the
/// system unchanged.
fn verbatim(path: &AbsPath) -> PathBuf {
    let mut text = OsString::from(r"\\?\");
    text.push(path.as_path().as_os_str());
    PathBuf::from(text)
}

fn names_in(fx: &Fixture, dir: &AbsPath) -> Vec<OsString> {
    let mut names: Vec<OsString> = fx
        .fs
        .read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().name)
        .collect();
    names.sort();
    names
}

#[test]
fn creates_reads_and_removes_entries_with_reserved_names() {
    let fx = fixture();
    let dir = fx.dir("reserved");
    for name in RESERVED_NAMES {
        let path = dir.join_name(name).unwrap();
        fx.write_new(&path, CONTENT);
        let meta = fx.fs.metadata(&path, Follow::NoFollow).unwrap();
        assert_eq!(meta.kind, FileKind::File, "{name}");
        assert_eq!(fx.read(&path), CONTENT, "{name}");
        let stored = std::fs::read(verbatim(&path)).unwrap();
        assert_eq!(stored, CONTENT, "{name} is stored under its exact name");
    }
    let mut expected: Vec<OsString> = RESERVED_NAMES.iter().map(OsString::from).collect();
    expected.sort();
    assert_eq!(names_in(&fx, &dir), expected);
    for name in RESERVED_NAMES {
        let path = dir.join_name(name).unwrap();
        fx.fs.remove_file(&fx.scope, &path).unwrap();
        assert!(!fx.exists(&path), "{name}");
    }
    fx.fs.remove_dir(&fx.scope, &dir).unwrap();
}

#[test]
fn handles_paths_longer_than_max_path() {
    let fx = fixture();
    let top = fx.path("long");
    let mut dir = top.clone();
    fx.fs.create_dir(&fx.scope, &dir, DIR_MODE).unwrap();
    for level in 1..DEPTH {
        let name = format!("{level}{}", "d".repeat(SEGMENT_LEN));
        dir = dir.join_name(&name).unwrap();
        fx.fs.create_dir(&fx.scope, &dir, DIR_MODE).unwrap();
    }
    let file = dir
        .join_name(&format!("file-{}.txt", "f".repeat(SEGMENT_LEN)))
        .unwrap();
    assert!(file.as_path().as_os_str().len() > MAX_PATH, "{file}");
    fx.write_new(&file, CONTENT);
    assert_eq!(fx.read(&file), CONTENT);
    let meta = fx.extended(&file);
    assert_eq!(meta.basic.size, u64::try_from(CONTENT.len()).unwrap());
    let renamed = dir.join_name("renamed.txt").unwrap();
    fx.fs
        .rename(&fx.scope, &file, &renamed, RenameMode::NoReplace)
        .unwrap();
    let copy = dir.join_name("copy.txt").unwrap();
    fx.fs.copy_file(&fx.scope, &renamed, &copy).unwrap();
    assert_eq!(fx.read(&copy), CONTENT);
    assert_eq!(fx.fs.real_path(&copy).unwrap(), copy);
    let stats = fx.fs.remove_tree(&fx.scope, &top, 2).unwrap();
    assert_eq!(stats.files, 2);
    assert_eq!(stats.directories, u64::try_from(DEPTH).unwrap());
    assert!(!fx.exists(&top));
}

#[test]
fn tree_removal_does_not_descend_into_a_junction() {
    let fx = fixture();
    let outside = fx.dir("outside");
    let kept = outside.join_name("kept.txt").unwrap();
    fx.write_new(&kept, CONTENT);
    let tree = fx.dir("tree");
    let sub = tree.join_name("sub").unwrap();
    fx.fs.create_dir(&fx.scope, &sub, DIR_MODE).unwrap();
    fx.write_new(&tree.join_name("one.txt").unwrap(), b"12");
    fx.write_new(&sub.join_name("two.txt").unwrap(), b"345");
    let junction = tree.join_name("junction").unwrap();
    fx.fs
        .create_link(&fx.scope, outside.as_path(), &junction, LinkKind::Junction)
        .unwrap();
    let meta = fx.fs.metadata(&junction, Follow::NoFollow).unwrap();
    assert_eq!(meta.kind, FileKind::Junction);
    assert_eq!(fx.fs.read_link(&junction).unwrap(), outside.as_path());
    let symlink = tree.join_name("symlink").unwrap();
    let linked = fx.fs.create_link(
        &fx.scope,
        outside.as_path(),
        &symlink,
        LinkKind::SymlinkDirectory,
    );
    if let Err(error) = linked {
        assert_eq!(error.kind, FsErrorKind::Privilege, "{error}");
    }
    let stats = fx.fs.remove_tree(&fx.scope, &tree, 2).unwrap();
    assert!(!fx.exists(&tree));
    assert_eq!(fx.read(&kept), CONTENT, "entry behind the junction");
    assert_eq!(stats.directories, 2, "{stats:?}");
    assert!(stats.bytes >= 5, "{stats:?}");
}

#[test]
fn tree_removal_removes_read_only_entries() {
    let fx = fixture();
    let tree = fx.dir("tree");
    let file = tree.join_name("read-only.txt").unwrap();
    fx.write_new(&file, CONTENT);
    let mut permissions = std::fs::metadata(file.as_path()).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(file.as_path(), permissions).unwrap();
    let stats = fx.fs.remove_tree(&fx.scope, &tree, 1).unwrap();
    assert_eq!(stats.files, 1);
    assert!(!fx.exists(&tree));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
