// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the Linux back end on the file systems of the test host.
//!
//! The temporary directory decides the file system under test. Tests that
//! need a privilege the account lacks, or a second volume, return early. Tests that
//! need a volume with block clones or with user quotas for the current
//! account are ignored by default; they run with `--ignored` on the volumes
//! named by the environment variables `NURT_TEST_REFLINK_DIR` and
//! `NURT_TEST_QUOTA_DIR`.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FileKind, Follow, Fs, FsErrorKind, FsWrite, LinkKind,
    MetaChange, RenameMode, ScopePurpose, TxScope, VolumeKind, Xattr,
};
use nurt_contracts::journal::OpId;
use rustix::fs::IFlags;
use rustix::io::Errno;
use tempfile::TempDir;

use super::fs::emulate_noreplace;
use super::{LinuxFs, MountTable, features_for, volume_kind};
use crate::conformance;

/// Mount point of the shared memory file system.
const DEV_SHM: &str = "/dev/shm";
/// Size of the file written by the space test.
const SPACE_PROBE: usize = 64 << 20;
/// Least decrease of free space accepted after writing [`SPACE_PROBE`]; the
/// margin covers concurrent writers on a shared volume.
const SPACE_TOLERANCE: u64 = 48 << 20;
/// Length of the sparse copy test file: above 2 GiB, the largest transfer
/// of one `copy_file_range` call.
const SPARSE_LENGTH: u64 = (2 << 30) + 4097;
/// `struct posix_acl_xattr_header` version.
const ACL_VERSION: u32 = 2;
const ACL_USER_OBJ: u16 = 0x01;
const ACL_USER: u16 = 0x02;
const ACL_GROUP_OBJ: u16 = 0x04;
const ACL_MASK: u16 = 0x10;
const ACL_OTHER: u16 = 0x20;
const ACL_UNDEFINED_ID: u32 = u32::MAX;
/// Named user of the test ACL; the account need not exist.
const ACL_NAMED_USER: u32 = 12_345;

#[expect(
    clippy::disallowed_methods,
    reason = "the tests act as the journal that owns the transaction"
)]
fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
}

struct Fixture {
    _dir: TempDir,
    root: AbsPath,
    fs: LinuxFs,
    scope: TxScope,
}

impl Fixture {
    fn new() -> Self {
        Self::within(None)
    }

    fn within(parent: Option<&Path>) -> Self {
        let dir = match parent {
            Some(parent) => tempfile::tempdir_in(parent),
            None => tempfile::tempdir(),
        }
        .expect("temporary directory");
        let root = AbsPath::new(dir.path()).expect("absolute temporary directory");
        Self {
            _dir: dir,
            root,
            fs: LinuxFs::new(),
            scope: scope(),
        }
    }

    fn path(&self, name: &str) -> AbsPath {
        self.root.join_name(name).expect("valid name")
    }

    fn file(&self, name: &str, content: &[u8]) -> AbsPath {
        let path = self.path(name);
        fs::write(&path, content).expect("write test file");
        path
    }

    fn dir(&self, name: &str) -> AbsPath {
        let path = self.path(name);
        fs::create_dir(&path).expect("create test directory");
        path
    }
}

fn read(path: &AbsPath) -> Vec<u8> {
    fs::read(path).expect("read test file")
}

fn inode_of(path: &AbsPath) -> u64 {
    fs::symlink_metadata(path).expect("metadata").ino()
}

fn privileged() -> bool {
    rustix::process::geteuid().is_root()
}

fn acl_value(entries: &[(u16, u16, u32)]) -> Vec<u8> {
    let mut value = ACL_VERSION.to_le_bytes().to_vec();
    for (tag, permissions, id) in entries {
        value.extend(tag.to_le_bytes());
        value.extend(permissions.to_le_bytes());
        value.extend(id.to_le_bytes());
    }
    value
}

/// Access ACL equivalent to mode `rwxr-x---` with read access for a named
/// user.
fn test_acl() -> Vec<u8> {
    acl_value(&[
        (ACL_USER_OBJ, 7, ACL_UNDEFINED_ID),
        (ACL_USER, 4, ACL_NAMED_USER),
        (ACL_GROUP_OBJ, 5, ACL_UNDEFINED_ID),
        (ACL_MASK, 5, ACL_UNDEFINED_ID),
        (ACL_OTHER, 0, ACL_UNDEFINED_ID),
    ])
}

/// Metadata that a transfer must reproduce: everything except identity,
/// size, link count, status change time and birth time.
fn transferable(meta: &ExtendedMeta) -> impl PartialEq + std::fmt::Debug {
    (
        meta.basic.kind,
        meta.basic.mode,
        meta.basic.uid,
        meta.basic.gid,
        meta.basic.modified_ns,
        meta.basic.accessed_ns,
        meta.xattrs.clone(),
        meta.flags,
        meta.link_target.clone(),
    )
}

/// Gives `path` a user attribute, setuid and setgid, an access ACL when
/// the volume has ACL, the `nodump` flag when the volume has flags, and
/// fixed times.
fn decorate(fixture: &Fixture, path: &AbsPath) -> bool {
    let features = fixture.fs.volume(path).expect("volume").features;
    rustix::fs::setxattr(
        path.as_path(),
        "user.origin",
        b"test",
        rustix::fs::XattrFlags::empty(),
    )
    .expect("user attribute");
    fs::set_permissions(path, fs::Permissions::from_mode(0o6750)).expect("mode");
    if features.acl {
        rustix::fs::setxattr(
            path.as_path(),
            "system.posix_acl_access",
            &test_acl(),
            rustix::fs::XattrFlags::empty(),
        )
        .expect("access ACL");
    }
    let file = File::open(path).expect("open for flags");
    let flagged = rustix::fs::ioctl_getflags(file.as_fd())
        .and_then(|flags| rustix::fs::ioctl_setflags(file.as_fd(), flags | IFlags::NODUMP))
        .is_ok();
    let change = MetaChange {
        modified_ns: Some(1_600_000_000_123_456_789),
        accessed_ns: Some(1_500_000_000_000_000_001),
        ..MetaChange::default()
    };
    fixture
        .fs
        .set_meta(&fixture.scope, path, &change)
        .expect("times");
    features.acl && flagged
}

#[test]
fn passes_conformance_suite_on_temporary_directory() {
    let fixture = Fixture::new();
    let other = TempDir::new_in(DEV_SHM).ok();
    let options = conformance::Options {
        other_volume: other
            .as_ref()
            .map(|dir| AbsPath::new(dir.path()).expect("absolute directory")),
    };
    let report = conformance::run_with(&fixture.fs, &fixture.root, &fixture.scope, &options);
    assert!(report.passed(), "{report}");
}

#[test]
fn passes_conformance_suite_on_other_host_file_systems() {
    for parent in ["/tmp", DEV_SHM] {
        let Ok(dir) = TempDir::new_in(parent) else {
            continue;
        };
        let fs = LinuxFs::new();
        let root = AbsPath::new(dir.path()).expect("absolute directory");
        let volume = fs.volume(&root).expect("volume");
        let report = conformance::run(&fs, &root, &scope());
        assert!(report.passed(), "{parent} ({}): {report}", volume.fs_type);
    }
}

#[test]
fn exchange_swaps_files_and_keeps_inodes() {
    let fixture = Fixture::new();
    let a = fixture.file("a", b"first");
    let b = fixture.file("b", b"second");
    let (inode_a, inode_b) = (inode_of(&a), inode_of(&b));
    fixture
        .fs
        .exchange(&fixture.scope, &a, &b)
        .expect("exchange");
    assert_eq!(
        (read(&a), read(&b)),
        (b"second".to_vec(), b"first".to_vec())
    );
    assert_eq!((inode_of(&a), inode_of(&b)), (inode_b, inode_a));
}

#[test]
fn exchange_swaps_directory_and_file() {
    let fixture = Fixture::new();
    let dir = fixture.dir("dir");
    fs::write(dir.join_name("inner").expect("name"), b"x").expect("inner file");
    let file = fixture.file("file", b"y");
    fixture
        .fs
        .exchange(&fixture.scope, &dir, &file)
        .expect("exchange");
    assert!(fs::symlink_metadata(&file).expect("metadata").is_dir());
    assert_eq!(read(&dir), b"y");
    assert_eq!(
        fs::read(file.join_name("inner").expect("name")).expect("inner"),
        b"x"
    );
}

#[test]
fn exchange_with_missing_entry_fails_with_not_found() {
    let fixture = Fixture::new();
    let a = fixture.file("a", b"x");
    let error = fixture
        .fs
        .exchange(&fixture.scope, &a, &fixture.path("missing"))
        .expect_err("missing entry");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    assert_eq!(error.os_code, Some(Errno::NOENT.raw_os_error()));
}

#[test]
fn rename_noreplace_refuses_existing_destination() {
    let fixture = Fixture::new();
    let a = fixture.file("a", b"a");
    let b = fixture.file("b", b"b");
    let error = fixture
        .fs
        .rename(&fixture.scope, &a, &b, RenameMode::NoReplace)
        .expect_err("destination exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
    assert_eq!((read(&a), read(&b)), (b"a".to_vec(), b"b".to_vec()));
    let c = fixture.path("c");
    fixture
        .fs
        .rename(&fixture.scope, &a, &c, RenameMode::NoReplace)
        .expect("rename to a new name");
    assert_eq!(read(&c), b"a");
    fixture
        .fs
        .rename(&fixture.scope, &c, &b, RenameMode::Replace)
        .expect("rename over existing name");
    assert_eq!(read(&b), b"a");
    assert!(!c.as_path().exists());
}

#[test]
fn emulated_noreplace_moves_files_and_refuses_directories() {
    let fixture = Fixture::new();
    let from = fixture.file("from", b"data");
    let inode = inode_of(&from);
    let to = fixture.path("to");
    emulate_noreplace(from.as_path(), to.as_path(), Errno::INVAL).expect("emulated rename");
    assert!(!from.as_path().exists());
    assert_eq!((read(&to), inode_of(&to)), (b"data".to_vec(), inode));
    let other = fixture.file("other", b"other");
    let error = emulate_noreplace(other.as_path(), to.as_path(), Errno::INVAL)
        .expect_err("destination exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
    assert_eq!(read(&other), b"other");
    let dir = fixture.dir("dir");
    let error = emulate_noreplace(dir.as_path(), fixture.path("moved").as_path(), Errno::INVAL)
        .expect_err("directory");
    assert_eq!(
        (error.kind, error.os_code),
        (FsErrorKind::Unsupported, Some(Errno::INVAL.raw_os_error()))
    );
}

#[test]
fn rename_reports_missing_source_and_cross_device_moves() {
    let fixture = Fixture::new();
    let error = fixture
        .fs
        .rename(
            &fixture.scope,
            &fixture.path("missing"),
            &fixture.path("to"),
            RenameMode::Replace,
        )
        .expect_err("missing source");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    let Ok(other) = TempDir::new_in(DEV_SHM) else {
        return;
    };
    let source = fixture.file("a", b"a");
    let target = AbsPath::new(other.path().join("a")).expect("absolute path");
    if fixture.fs.same_volume(&source, &target).expect("volumes") {
        return;
    }
    for mode in [RenameMode::Replace, RenameMode::NoReplace] {
        let error = fixture
            .fs
            .rename(&fixture.scope, &source, &target, mode)
            .expect_err("cross-device rename");
        assert_eq!(error.kind, FsErrorKind::CrossDevice);
    }
}

#[test]
fn clone_shares_blocks_or_reports_unsupported_without_leftover() {
    let fixture = Fixture::new();
    let source = fixture.file("source", b"content");
    let target = fixture.path("clone");
    let reflink = fixture.fs.volume(&source).expect("volume").features.reflink;
    match fixture.fs.clone_file(&fixture.scope, &source, &target) {
        Ok(()) => {
            assert!(
                reflink,
                "clone succeeded on a volume declared without clones"
            );
            assert_eq!(read(&target), b"content");
        }
        Err(error) => {
            assert_eq!(error.kind, FsErrorKind::Unsupported, "{error}");
            assert!(error.os_code.is_some());
            assert!(
                !target.as_path().exists(),
                "failed clone left its destination"
            );
        }
    }
    let error = fixture
        .fs
        .clone_file(&fixture.scope, &source, &source)
        .expect_err("destination exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
}

#[test]
#[ignore = "needs a volume with block clones named by NURT_TEST_REFLINK_DIR"]
fn clones_on_reflink_volume() {
    let Some(dir) = std::env::var_os("NURT_TEST_REFLINK_DIR") else {
        return;
    };
    let fixture = Fixture::within(Some(Path::new(&dir)));
    assert!(
        fixture
            .fs
            .volume(&fixture.root)
            .expect("volume")
            .features
            .reflink
    );
    let source = fixture.file("source", &vec![7; 65_536]);
    let target = fixture.path("clone");
    fixture
        .fs
        .clone_file(&fixture.scope, &source, &target)
        .expect("clone");
    assert_eq!(read(&target), read(&source));
}

#[test]
fn clone_and_copy_refuse_directories_and_special_files() {
    let fixture = Fixture::new();
    let dir = fixture.dir("dir");
    let target = fixture.path("target");
    let error = fixture
        .fs
        .clone_file(&fixture.scope, &dir, &target)
        .expect_err("directory source");
    assert_eq!(error.kind, FsErrorKind::IsADirectory);
    let error = fixture
        .fs
        .copy_file(
            &fixture.scope,
            &AbsPath::new("/dev/null").expect("path"),
            &target,
        )
        .expect_err("device source");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    assert!(!target.as_path().exists());
}

#[test]
fn copies_empty_and_single_byte_files() {
    let fixture = Fixture::new();
    for content in [&b""[..], b"x"] {
        let source = fixture.file("source", content);
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).expect("mode");
        let target = fixture.path("target");
        let copied = fixture
            .fs
            .copy_file(&fixture.scope, &source, &target)
            .expect("copy");
        assert_eq!(copied, content.len() as u64);
        assert_eq!(read(&target), content);
        assert_eq!(
            fs::metadata(&target).expect("metadata").mode() & 0o777,
            0o640
        );
        let error = fixture
            .fs
            .copy_file(&fixture.scope, &source, &target)
            .expect_err("destination exists");
        assert_eq!(error.kind, FsErrorKind::Exists);
        fs::remove_file(&target).expect("remove copy");
    }
}

#[test]
fn copies_sparse_file_larger_than_two_gibibytes() {
    let fixture = Fixture::new();
    let source = fixture.path("sparse");
    let mut file = File::create(&source).expect("create sparse file");
    file.set_len(SPARSE_LENGTH).expect("length");
    for (offset, data) in [
        (0, &[1_u8; 4096][..]),
        (1 << 30, &[2; 4096]),
        (SPARSE_LENGTH - 1, &[3]),
    ] {
        file.seek(SeekFrom::Start(offset)).expect("seek");
        file.write_all(data).expect("data");
    }
    drop(file);
    let target = fixture.path("copy");
    let copied = fixture
        .fs
        .copy_file(&fixture.scope, &source, &target)
        .expect("copy");
    assert_eq!(copied, SPARSE_LENGTH);
    let meta = fs::metadata(&target).expect("metadata");
    assert_eq!(meta.len(), SPARSE_LENGTH);
    assert!(
        meta.blocks() * 512 < 64 << 20,
        "holes were filled: {} blocks",
        meta.blocks()
    );
    assert_eq!(
        fixture.fs.content_hash(&target).expect("copy hash"),
        fixture.fs.content_hash(&source).expect("source hash")
    );
}

#[test]
fn writer_with_open_descriptor_changes_exchanged_old_content() {
    let fixture = Fixture::new();
    let current = fixture.file("current", b"old");
    let mut writer = OpenOptions::new()
        .append(true)
        .open(&current)
        .expect("writer");
    let replacement = fixture.file("replacement", b"new");
    fixture
        .fs
        .exchange(&fixture.scope, &current, &replacement)
        .expect("exchange");
    writer.write_all(b"+late").expect("late write");
    drop(writer);
    assert_eq!(read(&current), b"new");
    assert_eq!(read(&replacement), b"old+late");
}

#[test]
fn writes_in_place_through_every_hard_link() {
    let fixture = Fixture::new();
    let first = fixture.file("first", b"previous content");
    let second = fixture.path("second");
    fixture
        .fs
        .hard_link(&fixture.scope, &first, &second)
        .expect("hard link");
    let inode = inode_of(&first);
    assert_eq!(
        fixture
            .fs
            .metadata(&first, Follow::NoFollow)
            .expect("metadata")
            .links,
        2
    );
    let mut handle = fixture
        .fs
        .open_in_place(&fixture.scope, &second)
        .expect("open in place");
    handle.write_all(b"next").expect("write");
    handle.set_length(4).expect("length");
    handle.sync(Durability::Data).expect("sync");
    drop(handle);
    assert_eq!(
        (read(&first), read(&second)),
        (b"next".to_vec(), b"next".to_vec())
    );
    assert_eq!((inode_of(&first), inode_of(&second)), (inode, inode));
    let error = fixture
        .fs
        .hard_link(&fixture.scope, &first, &second)
        .expect_err("link exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
}

#[test]
fn open_in_place_and_truncate_refuse_symbolic_links() {
    let fixture = Fixture::new();
    let target = fixture.file("target", b"content");
    let link = fixture.path("link");
    symlink(&target, &link).expect("link");
    let error = fixture
        .fs
        .open_in_place(&fixture.scope, &link)
        .err()
        .expect("symbolic link");
    assert_eq!(error.kind, FsErrorKind::Loop);
    let error = fixture
        .fs
        .truncate(&fixture.scope, &link, 0)
        .expect_err("symbolic link");
    assert_eq!(error.kind, FsErrorKind::Loop);
    fixture
        .fs
        .truncate(&fixture.scope, &target, 3)
        .expect("truncate");
    assert_eq!(read(&target), b"con");
}

fn build_tree(root: &Path, directories: usize, files: usize) {
    for directory in 0..directories {
        let dir = root.join(format!("d{directory}"));
        fs::create_dir_all(&dir).expect("tree directory");
        for file in 0..files {
            fs::write(dir.join(format!("f{file}")), b"ab").expect("tree file");
        }
    }
}

#[test]
fn remove_tree_never_follows_symbolic_links() {
    let fixture = Fixture::new();
    let outside = fixture.dir("outside");
    let kept = outside.join_name("kept").expect("name");
    fs::write(&kept, b"keep").expect("outside file");
    let tree = fixture.dir("tree");
    build_tree(tree.as_path(), 2, 3);
    symlink(&outside, tree.join_name("to-dir").expect("name")).expect("directory link");
    symlink(&kept, tree.as_path().join("d0").join("to-file")).expect("file link");
    let stats = fixture
        .fs
        .remove_tree(&fixture.scope, &tree, 4)
        .expect("remove tree");
    assert!(!tree.as_path().exists());
    assert_eq!(read(&kept), b"keep");
    assert_eq!((stats.files, stats.directories), (8, 3));
    assert!(stats.bytes >= 12);
}

#[test]
fn removes_ten_thousand_files_with_one_and_eight_workers() {
    for parallelism in [1, 8] {
        let fixture = Fixture::new();
        let tree = fixture.dir("tree");
        build_tree(tree.as_path(), 100, 100);
        let stats = fixture
            .fs
            .remove_tree(&fixture.scope, &tree, parallelism)
            .expect("remove tree");
        assert_eq!(
            (stats.files, stats.directories, stats.bytes),
            (10_000, 101, 20_000),
            "parallelism {parallelism}"
        );
        assert!(!tree.as_path().exists());
    }
}

#[test]
fn remove_tree_removes_single_entries_and_reports_errors() {
    let fixture = Fixture::new();
    let file = fixture.file("file", b"abc");
    let stats = fixture
        .fs
        .remove_tree(&fixture.scope, &file, 1)
        .expect("remove file");
    assert_eq!((stats.files, stats.directories, stats.bytes), (1, 0, 3));
    let error = fixture
        .fs
        .remove_tree(&fixture.scope, &fixture.path("missing"), 1)
        .expect_err("missing entry");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    let error = fixture
        .fs
        .remove_tree(&fixture.scope, &AbsPath::new("/").expect("root"), 1)
        .expect_err("root");
    assert_eq!(error.kind, FsErrorKind::InvalidPath);
    let error = fixture
        .fs
        .remove_tree(
            &fixture.scope,
            &AbsPath::new("/proc/self/fd").expect("path"),
            1,
        )
        .expect_err("pseudo file system");
    assert_ne!(error.kind, FsErrorKind::NotFound);
}

#[test]
fn round_trips_complete_metadata_through_restore_meta() {
    let fixture = Fixture::new();
    let source = fixture.file("source", b"content");
    // Without ACL or inode flags on the volume only attributes, mode and times are checked.
    decorate(&fixture, &source);
    let before = fixture
        .fs
        .extended_metadata(&source, Follow::NoFollow)
        .expect("metadata before");
    assert!(
        before
            .xattrs
            .iter()
            .any(|xattr| xattr.name == "user.origin")
    );
    assert_eq!(before.basic.mode, 0o6750);
    assert_eq!(
        (before.acl.as_ref(), before.account_sid.as_ref()),
        (None, None)
    );
    assert!(before.streams.is_empty());

    let target = fixture.file("target", b"other content");
    let report = fixture
        .fs
        .restore_meta(&fixture.scope, &target, &before)
        .expect("restore onto another file");
    assert!(report.is_complete(), "{report:?}");
    let restored = fixture
        .fs
        .extended_metadata(&target, Follow::NoFollow)
        .expect("restored metadata");
    assert_eq!(transferable(&restored), transferable(&before));

    let change = MetaChange {
        mode: Some(0o600),
        modified_ns: Some(1),
        remove_xattrs: vec!["user.origin".into()],
        set_xattrs: vec![Xattr {
            name: "user.extra".into(),
            value: b"1".to_vec(),
        }],
        ..MetaChange::default()
    };
    fixture
        .fs
        .set_meta(&fixture.scope, &source, &change)
        .expect("change metadata");
    let report = fixture
        .fs
        .restore_meta(&fixture.scope, &source, &before)
        .expect("restore in place");
    assert!(report.is_complete(), "{report:?}");
    let after = fixture
        .fs
        .extended_metadata(&source, Follow::NoFollow)
        .expect("metadata after");
    assert_eq!(transferable(&after), transferable(&before));
    assert_eq!(after.basic.size, before.basic.size);
}

#[test]
fn applies_complete_metadata_through_write_handle() {
    let fixture = Fixture::new();
    let source = fixture.file("source", b"content");
    decorate(&fixture, &source);
    let wanted = fixture
        .fs
        .extended_metadata(&source, Follow::NoFollow)
        .expect("metadata");
    let mut handle = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.root, "staged")
        .expect("temporary file");
    handle.write_all(b"replacement").expect("write");
    let report = handle.apply_meta(&wanted).expect("apply");
    assert!(report.is_complete(), "{report:?}");
    let staged = handle.path().clone();
    drop(handle);
    let applied = fixture
        .fs
        .extended_metadata(&staged, Follow::NoFollow)
        .expect("applied metadata");
    assert_eq!(transferable(&applied), transferable(&wanted));
}

#[test]
fn round_trips_default_acl_of_directory_and_times_of_symbolic_link() {
    let fixture = Fixture::new();
    let source = fixture.dir("source");
    if fixture.fs.volume(&source).expect("volume").features.acl {
        rustix::fs::setxattr(
            source.as_path(),
            "system.posix_acl_default",
            &test_acl(),
            rustix::fs::XattrFlags::empty(),
        )
        .expect("default ACL");
    }
    let wanted = fixture
        .fs
        .extended_metadata(&source, Follow::NoFollow)
        .expect("directory metadata");
    let target = fixture.dir("target");
    let report = fixture
        .fs
        .restore_meta(&fixture.scope, &target, &wanted)
        .expect("restore directory");
    assert!(report.is_complete(), "{report:?}");
    let restored = fixture
        .fs
        .extended_metadata(&target, Follow::NoFollow)
        .expect("restored directory");
    assert_eq!(transferable(&restored), transferable(&wanted));

    let link = fixture.path("link");
    symlink("target", &link).expect("link");
    let mut wanted = fixture
        .fs
        .extended_metadata(&link, Follow::NoFollow)
        .expect("link metadata");
    assert_eq!(wanted.link_target.as_deref(), Some(Path::new("target")));
    assert_eq!(wanted.flags, None);
    wanted.basic.modified_ns = 1_000_000_000;
    let report = fixture
        .fs
        .restore_meta(&fixture.scope, &link, &wanted)
        .expect("restore link");
    assert!(report.is_complete(), "{report:?}");
    let link_meta = fixture.fs.metadata(&link, Follow::NoFollow).expect("link");
    assert_eq!(link_meta.modified_ns, 1_000_000_000);
    let target_meta = fixture.fs.metadata(&link, Follow::Follow).expect("target");
    assert_ne!(target_meta.modified_ns, 1_000_000_000);
}

#[test]
fn reports_metadata_that_requires_missing_privileges() {
    if privileged() {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture.file("file", b"x");
    let mut wanted = fixture
        .fs
        .extended_metadata(&path, Follow::NoFollow)
        .expect("metadata");
    wanted.basic.uid = Some(0);
    wanted.basic.mode = 0o640;
    let capability = [1, 0, 0, 2, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    wanted.xattrs.push(Xattr {
        name: "security.capability".into(),
        value: capability.to_vec(),
    });
    wanted.flags = wanted
        .flags
        .map(|flags| flags | u64::from(IFlags::IMMUTABLE.bits()));
    let report = fixture
        .fs
        .restore_meta(&fixture.scope, &path, &wanted)
        .expect("restore");
    assert!(
        report.not_transferred.contains(&"owner".to_owned()),
        "{report:?}"
    );
    assert!(
        report
            .not_transferred
            .contains(&"xattr:security.capability".to_owned()),
        "{report:?}"
    );
    if wanted.flags.is_some() {
        assert!(
            report.not_transferred.contains(&"flags".to_owned()),
            "{report:?}"
        );
    }
    assert!(
        !report.not_transferred.contains(&"mode".to_owned()),
        "{report:?}"
    );
    assert_eq!(
        fs::metadata(&path).expect("metadata").mode() & 0o7777,
        0o640
    );

    let error = fixture
        .fs
        .set_meta(
            &fixture.scope,
            &path,
            &MetaChange {
                uid: Some(0),
                ..MetaChange::default()
            },
        )
        .expect_err("change of account");
    assert_eq!(error.kind, FsErrorKind::Privilege);
    let error = fixture
        .fs
        .set_meta(
            &fixture.scope,
            &path,
            &MetaChange {
                set_xattrs: vec![Xattr {
                    name: "security.capability".into(),
                    value: capability.to_vec(),
                }],
                ..MetaChange::default()
            },
        )
        .expect_err("capability attribute");
    assert_eq!(error.kind, FsErrorKind::Privilege);
}

#[test]
fn set_meta_refuses_elements_linux_cannot_set() {
    let fixture = Fixture::new();
    let path = fixture.file("file", b"x");
    let changes = [
        MetaChange {
            created_ns: Some(1),
            ..MetaChange::default()
        },
        MetaChange {
            acl: Some(vec![1]),
            ..MetaChange::default()
        },
        MetaChange {
            set_windows_attributes: 1,
            ..MetaChange::default()
        },
    ];
    for change in changes {
        let error = fixture
            .fs
            .set_meta(&fixture.scope, &path, &change)
            .expect_err("unsupported element");
        assert_eq!(error.kind, FsErrorKind::Unsupported);
    }
    let link = fixture.path("link");
    symlink(&path, &link).expect("link");
    let error = fixture
        .fs
        .set_meta(
            &fixture.scope,
            &link,
            &MetaChange {
                mode: Some(0o600),
                ..MetaChange::default()
            },
        )
        .expect_err("mode of a link");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    let error = fixture
        .fs
        .set_meta(
            &fixture.scope,
            &path,
            &MetaChange {
                remove_xattrs: vec!["user.absent".into()],
                ..MetaChange::default()
            },
        )
        .expect_err("absent attribute");
    assert_eq!(error.kind, FsErrorKind::Io);
    assert_eq!(error.os_code, Some(Errno::NODATA.raw_os_error()));
}

#[test]
fn takes_exclusive_lock_and_times_out_while_it_is_held() {
    let fixture = Fixture::new();
    let path = fixture.path("lock");
    let lock = fixture
        .fs
        .lock_exclusive(&path, Duration::from_secs(1))
        .expect("first lock");
    assert_eq!(lock.path(), &path);
    let started = Instant::now();
    let error = fixture
        .fs
        .lock_exclusive(&path, Duration::from_millis(100))
        .err()
        .expect("second lock");
    assert_eq!(error.kind, FsErrorKind::InUse);
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(
        error
            .holders
            .iter()
            .any(|holder| holder.pid == std::process::id())
    );
    drop(lock);
    fixture
        .fs
        .lock_exclusive(&path, Duration::ZERO)
        .expect("lock after release");
}

#[test]
fn flushes_files_and_directories() {
    let fixture = Fixture::new();
    fixture
        .fs
        .sync_dir(&fixture.scope, &fixture.root)
        .expect("directory flush");
    let file = fixture.file("file", b"x");
    let error = fixture
        .fs
        .sync_dir(&fixture.scope, &file)
        .expect_err("file is not a directory");
    assert_eq!(error.kind, FsErrorKind::NotADirectory);
    let mut handle = fixture
        .fs
        .create_new(&fixture.scope, &fixture.path("new"))
        .expect("new file");
    handle.write_all(b"data").expect("write");
    handle.flush().expect("flush");
    handle.sync(Durability::Data).expect("data flush");
    handle.sync(Durability::Full).expect("full flush");
    assert_eq!(handle.seek(SeekFrom::Start(1)).expect("seek"), 1);
}

#[test]
fn flushes_many_paths_at_once_and_few_one_by_one() {
    let fixture = Fixture::new();
    let few = vec![fixture.file("a", b"a")];
    fixture
        .fs
        .sync_paths(&fixture.scope, &few, std::slice::from_ref(&fixture.root))
        .expect("few entries are flushed one by one");
    let many: Vec<AbsPath> = (0..SYNC_PATHS_MANY)
        .map(|index| fixture.file(&format!("many-{index}"), b"x"))
        .collect();
    fixture
        .fs
        .sync_paths(&fixture.scope, &many, std::slice::from_ref(&fixture.root))
        .expect("many entries are flushed with the file system");
    let read_only = fixture.file("read-only", b"x");
    std::fs::set_permissions(
        read_only.as_path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o444),
    )
    .expect("permissions");
    fixture
        .fs
        .sync_paths(&fixture.scope, &[read_only], &[])
        .expect("a file that cannot be opened for writing is flushed through its directory");
    let missing = fixture.path("missing");
    let error = fixture
        .fs
        .sync_paths(&fixture.scope, &[missing], &[])
        .expect_err("a missing file cannot be flushed");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    let error = fixture
        .fs
        .sync_paths(&fixture.scope, &[], &[fixture.path("missing-dir")])
        .expect_err("a missing directory cannot be flushed");
    assert_eq!(error.kind, FsErrorKind::NotFound);
}

/// Number of entries that makes `sync_paths` flush the whole file system.
const SYNC_PATHS_MANY: usize = 80;

#[test]
fn creates_unique_private_temporary_files() {
    let fixture = Fixture::new();
    let first = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.root, "blob")
        .expect("first");
    let second = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.root, "blob")
        .expect("second");
    assert_ne!(first.path(), second.path());
    let name = first.path().file_name().expect("name").to_string_lossy();
    assert!(name.starts_with("blob."), "{name}");
    assert_eq!(
        fs::metadata(first.path()).expect("metadata").mode() & 0o777,
        0o600
    );
    for hint in ["", "a/b", ".."] {
        let error = fixture
            .fs
            .create_temp_in(&fixture.scope, &fixture.root, hint)
            .err()
            .expect("invalid hint");
        assert_eq!(error.kind, FsErrorKind::InvalidPath);
    }
    let error = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.path("missing"), "blob")
        .err()
        .expect("missing directory");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    let error = fixture
        .fs
        .create_new(&fixture.scope, first.path())
        .err()
        .expect("existing file");
    assert_eq!(error.kind, FsErrorKind::Exists);
}

#[test]
fn shortens_a_long_hint_of_a_temporary_file_to_the_name_limit() {
    let fixture = Fixture::new();
    let hint = "\u{e9}".repeat(127);
    let handle = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.root, &hint)
        .expect("temporary file");
    let name = handle.path().file_name().expect("name");
    assert!(name.len() <= 255, "{}", name.len());
    assert!(name.to_string_lossy().starts_with('\u{e9}'));
}

#[test]
fn reads_files_directories_and_links() {
    let fixture = Fixture::new();
    let file = fixture.file("file", b"abc");
    let dir = fixture.dir("dir");
    let link = fixture.path("link");
    symlink("file", &link).expect("link");
    let mut handle = fixture.fs.open_read(&link).expect("open through link");
    let mut content = String::new();
    handle.read_to_string(&mut content).expect("read");
    assert_eq!(content, "abc");
    assert_eq!(handle.seek(SeekFrom::Start(1)).expect("seek"), 1);
    assert_eq!(handle.meta().expect("handle metadata").size, 3);
    assert!(handle.as_bytes().is_none());
    let error = fixture.fs.open_read(&dir).err().expect("directory");
    assert_eq!(error.kind, FsErrorKind::IsADirectory);

    let mut entries: Vec<_> = fixture
        .fs
        .read_dir(&fixture.root)
        .expect("listing")
        .map(|entry| entry.expect("entry"))
        .map(|entry| (entry.name.to_string_lossy().into_owned(), entry.kind))
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        entries,
        [
            ("dir".to_owned(), Some(FileKind::Directory)),
            ("file".to_owned(), Some(FileKind::File)),
            ("link".to_owned(), Some(FileKind::Symlink)),
        ]
    );
    let error = fixture.fs.read_dir(&file).err().expect("file");
    assert_eq!(error.kind, FsErrorKind::NotADirectory);

    assert_eq!(
        fixture.fs.read_link(&link).expect("link"),
        Path::new("file")
    );
    let error = fixture.fs.read_link(&file).expect_err("not a link");
    assert_eq!(error.kind, FsErrorKind::InvalidPath);
    assert_eq!(
        fixture.fs.real_path(&link).expect("real path"),
        fixture.fs.real_path(&file).expect("file")
    );
    let error = fixture
        .fs
        .real_path(&fixture.path("missing"))
        .expect_err("missing");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    let extended = fixture
        .fs
        .extended_metadata(&link, Follow::Follow)
        .expect("followed link");
    assert_eq!(
        (extended.basic.kind, extended.link_target),
        (FileKind::File, None)
    );
}

#[test]
fn reports_metadata_kinds_and_error_classes() {
    let fixture = Fixture::new();
    let file = fixture.file("file", b"abc");
    let meta = fixture
        .fs
        .metadata(&file, Follow::NoFollow)
        .expect("metadata");
    assert_eq!((meta.kind, meta.size, meta.links), (FileKind::File, 3, 1));
    assert_eq!(meta.uid, Some(rustix::process::getuid().as_raw()));
    assert!(meta.changed_ns.is_some() && meta.accessed_ns.is_some());
    assert_eq!(meta.file_id.low, inode_of(&file));
    assert_eq!(meta.file_id.volume, fs::metadata(&file).expect("std").dev());
    let null = AbsPath::new("/dev/null").expect("path");
    assert_eq!(
        fixture
            .fs
            .metadata(&null, Follow::Follow)
            .expect("device")
            .kind,
        FileKind::Special
    );
    let under_file = file.join_name("child").expect("name");
    let error = fixture
        .fs
        .metadata(&under_file, Follow::NoFollow)
        .expect_err("file as directory");
    assert_eq!(error.kind, FsErrorKind::NotADirectory);
    let loop_link = fixture.path("loop");
    symlink("loop", &loop_link).expect("loop link");
    let error = fixture
        .fs
        .metadata(&loop_link, Follow::Follow)
        .expect_err("loop");
    assert_eq!(error.kind, FsErrorKind::Loop);
    let long = fixture.path(&"n".repeat(300));
    let error = fixture
        .fs
        .metadata(&long, Follow::Follow)
        .expect_err("long name");
    assert_eq!(error.kind, FsErrorKind::NameTooLong);
}

#[test]
fn creates_and_removes_entries() {
    let fixture = Fixture::new();
    let dir = fixture.path("dir");
    fixture
        .fs
        .create_dir(&fixture.scope, &dir, 0o700)
        .expect("create directory");
    assert_eq!(fs::metadata(&dir).expect("metadata").mode() & 0o777, 0o700);
    let error = fixture
        .fs
        .create_dir(&fixture.scope, &dir, 0o700)
        .expect_err("exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
    let inner = dir.join_name("inner").expect("name");
    fs::write(&inner, b"x").expect("inner file");
    let error = fixture
        .fs
        .remove_dir(&fixture.scope, &dir)
        .expect_err("not empty");
    assert_eq!(error.kind, FsErrorKind::NotEmpty);
    let error = fixture
        .fs
        .remove_dir(&fixture.scope, &inner)
        .expect_err("not a directory");
    assert_eq!(error.kind, FsErrorKind::NotADirectory);
    let error = fixture
        .fs
        .remove_file(&fixture.scope, &dir)
        .expect_err("directory");
    assert_eq!(error.kind, FsErrorKind::IsADirectory);
    fixture
        .fs
        .remove_file(&fixture.scope, &inner)
        .expect("remove file");
    fixture
        .fs
        .remove_dir(&fixture.scope, &dir)
        .expect("remove directory");

    let link = fixture.path("link");
    fixture
        .fs
        .create_link(
            &fixture.scope,
            Path::new("dangling"),
            &link,
            LinkKind::SymlinkFile,
        )
        .expect("symbolic link");
    assert_eq!(fs::read_link(&link).expect("target"), Path::new("dangling"));
    let error = fixture
        .fs
        .create_link(
            &fixture.scope,
            Path::new("dir"),
            &fixture.path("junction"),
            LinkKind::Junction,
        )
        .expect_err("junction");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    fixture
        .fs
        .remove_file(&fixture.scope, &link)
        .expect("remove link");
}

#[test]
fn refuses_windows_only_operations() {
    let fixture = Fixture::new();
    let file = fixture.file("file", b"x");
    let error = fixture
        .fs
        .open_stream_read(&file, "s")
        .err()
        .expect("stream read");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    let error = fixture
        .fs
        .open_stream_write(&fixture.scope, &file, "s")
        .err()
        .expect("stream write");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    let error = fixture
        .fs
        .replace_with_backup(&fixture.scope, &file, &file, &file)
        .expect_err("replace with backup");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    fixture
        .fs
        .mark_hidden(&fixture.scope, &file)
        .expect("hidden by name");
    let error = fixture
        .fs
        .mark_hidden(&fixture.scope, &fixture.path("missing"))
        .expect_err("missing");
    assert_eq!(error.kind, FsErrorKind::NotFound);
}

#[test]
fn hashes_content() {
    let fixture = Fixture::new();
    let file = fixture.file("abc", b"abc");
    assert_eq!(
        fixture.fs.content_hash(&file).expect("hash").to_string(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn reports_writable_directories() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .fs
            .can_write_dir(&fixture.root)
            .expect("own directory")
    );
    let file = fixture.file("file", b"x");
    let error = fixture.fs.can_write_dir(&file).expect_err("file");
    assert_eq!(error.kind, FsErrorKind::NotADirectory);
    if !privileged() {
        let locked = fixture.dir("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).expect("mode");
        assert!(
            !fixture
                .fs
                .can_write_dir(&locked)
                .expect("read-only directory")
        );
        let proc = AbsPath::new("/proc").expect("path");
        assert!(!fixture.fs.can_write_dir(&proc).expect("procfs"));
    }
}

#[test]
fn lists_own_process_among_holders() {
    let fixture = Fixture::new();
    let file = fixture.file("held", b"x");
    let other = fixture.file("free", b"x");
    let held = File::open(&file).expect("open");
    let holders = fixture.fs.holders(&file).expect("holders");
    let own = holders
        .iter()
        .find(|holder| holder.pid == std::process::id())
        .expect("own process");
    assert!(!own.name.is_empty());
    let free = fixture
        .fs
        .holders(&other)
        .expect("holders of a closed file");
    assert!(free.iter().all(|holder| holder.pid != std::process::id()));
    drop(held);
    let error = fixture
        .fs
        .holders(&fixture.path("missing"))
        .expect_err("missing");
    assert_eq!(error.kind, FsErrorKind::NotFound);
}

fn describe(fs: &LinuxFs, path: &Path) -> Option<nurt_contracts::fs::VolumeInfo> {
    let path = AbsPath::new(path).ok()?;
    let volume = fs.volume(&path).ok()?;
    Some(volume)
}

#[test]
fn classifies_volumes_of_the_host() {
    let fixture = Fixture::new();
    let volume = describe(&fixture.fs, fixture.root.as_path()).expect("temporary volume");
    let entry = fixture.fs.mount_entry(&fixture.root).expect("mount entry");
    assert_eq!(volume.fs_type, entry.fs_type);
    assert!(fixture.root.starts_with(&volume.mount));
    let removable = volume.kind == VolumeKind::Removable;
    assert_eq!(
        volume.kind,
        volume_kind(&entry.fs_type, &entry.source, removable)
    );
    let mut expected = features_for(&entry.fs_type, volume.kind, &entry.super_options);
    expected.reflink = volume.features.reflink;
    assert_eq!(volume.features, expected);
    let missing = fixture.path("missing").join_name("deeper").expect("name");
    assert_eq!(
        fixture.fs.volume(&missing).expect("missing path").id,
        volume.id
    );

    if let Some(shm) = describe(&fixture.fs, Path::new(DEV_SHM)) {
        assert_eq!(
            (shm.fs_type.as_str(), shm.kind),
            ("tmpfs", VolumeKind::Memory)
        );
        assert!(!shm.rotational);
    }
    let proc = describe(&fixture.fs, Path::new("/proc")).expect("procfs");
    assert_eq!(proc.kind, VolumeKind::Other);
    if let Some(root) = describe(&fixture.fs, Path::new("/"))
        && root.fs_type == "overlay"
    {
        assert_eq!(root.kind, VolumeKind::Overlay);
        let entry = fixture
            .fs
            .mount_entry(&AbsPath::new("/").expect("root"))
            .expect("root mount");
        let layers = entry.overlay_layers().expect("overlay layers");
        assert!(!layers.lower.is_empty());
    }
    let error = fixture
        .fs
        .volume(&AbsPath::new("/proc/self/fd/999999/x").expect("path"))
        .map(|volume| volume.kind);
    assert_eq!(error.ok(), Some(VolumeKind::Other));
}

#[test]
fn reads_own_mount_table() {
    let text = fs::read("/proc/self/mountinfo").expect("mount table");
    let table = MountTable::parse(&text).expect("parse");
    assert!(
        table
            .entries()
            .iter()
            .any(|entry| entry.mount_point == b"/")
    );
    assert!(
        table
            .entries()
            .iter()
            .all(|entry| !entry.fs_type.is_empty())
    );
}

#[test]
fn free_space_shrinks_after_written_and_flushed_data() {
    let fixture = Fixture::new();
    let volume = fixture.fs.volume(&fixture.root).expect("volume");
    let mut decreases = Vec::new();
    for attempt in 0..5 {
        let before = fixture.fs.space(&volume.id).expect("space before");
        assert!(before.available <= before.total);
        assert!(before.total > 0);
        let path = fixture.path(&format!("probe{attempt}"));
        let mut file = File::create(&path).expect("probe file");
        file.write_all(&vec![0x5a; SPACE_PROBE])
            .expect("probe data");
        file.sync_all().expect("probe flush");
        let after = fixture.fs.space(&volume.id).expect("space after");
        let decrease = before.available.saturating_sub(after.available);
        decreases.push(decrease);
        if decrease >= SPACE_TOLERANCE {
            return;
        }
    }
    panic!("free space did not shrink by {SPACE_TOLERANCE} bytes: {decreases:?}");
}

#[test]
fn space_of_unknown_volume_fails_with_not_found() {
    let fs = LinuxFs::new();
    let id = nurt_contracts::fs::VolumeId {
        dev: u64::MAX,
        uuid: None,
        subvolume: 0,
    };
    assert_eq!(
        fs.space(&id).expect_err("unknown").kind,
        FsErrorKind::NotFound
    );
    let root = AbsPath::new("/").expect("root");
    let volume = LinuxFs::new().volume(&root).expect("volume");
    let space = fs
        .space(&volume.id)
        .expect("volume found through the mount table");
    assert!(space.available <= space.total);
}

#[test]
fn reports_no_quota_on_volumes_without_quotas() {
    let fixture = Fixture::new();
    let volume = fixture.fs.volume(&fixture.root).expect("volume");
    let space = fixture.fs.space(&volume.id).expect("space");
    match space.quota_remaining {
        None => assert_eq!(space.purgeable_system, None),
        Some(remaining) => assert!(
            space.available <= remaining,
            "available {} exceeds the remaining quota {remaining}",
            space.available
        ),
    }
}

#[test]
#[ignore = "needs a volume with user quotas named by NURT_TEST_QUOTA_DIR"]
fn reports_enforced_user_quota() {
    let Some(dir) = std::env::var_os("NURT_TEST_QUOTA_DIR") else {
        return;
    };
    let fs = LinuxFs::new();
    let path = AbsPath::new(PathBuf::from(dir)).expect("absolute directory");
    let volume = fs.volume(&path).expect("volume");
    let space = fs.space(&volume.id).expect("space");
    let remaining = space.quota_remaining.expect("quota of the account");
    assert!(space.available <= remaining && space.total > 0);
}

#[test]
fn lists_sockets_as_special_entries() {
    let fixture = Fixture::new();
    let socket = fixture.path("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).expect("socket");
    let entries: Vec<_> = fixture
        .fs
        .read_dir(&fixture.root)
        .expect("listing")
        .map(|entry| entry.expect("entry"))
        .map(|entry| (entry.name.to_string_lossy().into_owned(), entry.kind))
        .collect();
    assert_eq!(entries, [("socket".to_owned(), Some(FileKind::Special))]);
}

#[test]
fn restore_meta_and_lock_report_missing_entries() {
    let fixture = Fixture::new();
    let meta = fixture
        .fs
        .extended_metadata(&fixture.root, Follow::NoFollow)
        .expect("metadata");
    let error = fixture
        .fs
        .restore_meta(&fixture.scope, &fixture.path("missing"), &meta)
        .expect_err("missing entry");
    assert_eq!(error.kind, FsErrorKind::NotFound);
    let lock = fixture.path("missing").join_name("lock").expect("name");
    let error = fixture
        .fs
        .lock_exclusive(&lock, Duration::ZERO)
        .err()
        .expect("missing directory");
    assert_eq!(error.kind, FsErrorKind::NotFound);
}

#[test]
fn remove_tree_stops_at_unreadable_subdirectory() {
    if privileged() {
        return;
    }
    let fixture = Fixture::new();
    let tree = fixture.dir("tree");
    let locked = tree.join_name("locked").expect("name");
    fs::create_dir(&locked).expect("locked directory");
    fs::write(locked.join_name("kept").expect("name"), b"x").expect("file");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("mode");
    let result = fixture.fs.remove_tree(&fixture.scope, &tree, 1);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).expect("mode");
    let error = result.expect_err("unreadable directory");
    assert_eq!(error.kind, FsErrorKind::PermissionDenied);
    assert_eq!(error.path.as_deref(), Some(locked.as_path()));
    assert!(locked.as_path().join("kept").exists());
}

fn make_fifo(path: &AbsPath) {
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        path.as_path(),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .expect("fifo");
}

#[test]
fn restore_meta_without_flags_keeps_the_immutable_flag_of_the_target() {
    if !privileged() {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture.file("protected", b"x");
    let file = File::open(&path).expect("open");
    let Ok(flags) = rustix::fs::ioctl_getflags(file.as_fd()) else {
        return;
    };
    if rustix::fs::ioctl_setflags(file.as_fd(), flags | IFlags::IMMUTABLE).is_err() {
        return;
    }
    let mut wanted = fixture
        .fs
        .extended_metadata(&path, Follow::NoFollow)
        .expect("metadata");
    wanted.flags = None;
    let result = fixture.fs.restore_meta(&fixture.scope, &path, &wanted);
    let after = rustix::fs::ioctl_getflags(file.as_fd());
    rustix::fs::ioctl_setflags(file.as_fd(), flags).expect("lift the flag");
    result.expect("restore");
    assert!(after.expect("flags").contains(IFlags::IMMUTABLE));
}

#[test]
fn lock_with_the_longest_timeout_is_taken_at_once_when_the_file_is_free() {
    let fixture = Fixture::new();
    let path = fixture.path("lock");
    let lock = fixture
        .fs
        .lock_exclusive(&path, Duration::MAX)
        .expect("lock");
    assert_eq!(lock.path(), &path);
}

#[test]
fn lock_with_zero_timeout_fails_at_once_while_it_is_held() {
    let fixture = Fixture::new();
    let path = fixture.path("lock");
    let first = fixture
        .fs
        .lock_exclusive(&path, Duration::ZERO)
        .expect("first lock");
    let error = fixture
        .fs
        .lock_exclusive(&path, Duration::ZERO)
        .err()
        .expect("second lock");
    assert_eq!(error.kind, FsErrorKind::InUse);
    drop(first);
    fixture
        .fs
        .lock_exclusive(&path, Duration::ZERO)
        .expect("lock after release");
}

#[test]
fn lock_refuses_a_symbolic_link_at_the_lock_path() {
    let fixture = Fixture::new();
    let target = fixture.path("target");
    let link = fixture.path("lock");
    symlink(&target, &link).expect("link");
    let error = fixture
        .fs
        .lock_exclusive(&link, Duration::ZERO)
        .err()
        .expect("link refused");
    assert_eq!(error.kind, FsErrorKind::Loop);
    assert!(!target.as_path().exists());
}

#[test]
fn copy_and_clone_refuse_a_fifo_source_without_waiting_for_a_writer() {
    let fixture = Fixture::new();
    let fifo = fixture.path("fifo");
    make_fifo(&fifo);
    let target = fixture.path("target");
    let error = fixture
        .fs
        .copy_file(&fixture.scope, &fifo, &target)
        .expect_err("fifo source");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    assert!(!target.as_path().exists());
    let error = fixture
        .fs
        .clone_file(&fixture.scope, &fifo, &target)
        .expect_err("fifo source");
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    assert!(!target.as_path().exists());
}

#[test]
fn lists_fifos_as_special_entries() {
    let fixture = Fixture::new();
    make_fifo(&fixture.path("fifo"));
    let entries: Vec<_> = fixture
        .fs
        .read_dir(&fixture.root)
        .expect("listing")
        .map(|entry| entry.expect("entry"))
        .map(|entry| (entry.name.to_string_lossy().into_owned(), entry.kind))
        .collect();
    assert_eq!(entries, [("fifo".to_owned(), Some(FileKind::Special))]);
}

#[test]
fn remove_tree_of_a_symbolic_link_removes_only_the_link() {
    let fixture = Fixture::new();
    let outside = fixture.dir("outside");
    let kept = outside.join_name("kept").expect("name");
    fs::write(&kept, b"keep").expect("outside file");
    let link = fixture.path("link");
    symlink(&outside, &link).expect("link");
    let stats = fixture
        .fs
        .remove_tree(&fixture.scope, &link, 2)
        .expect("remove link");
    assert_eq!((stats.files, stats.directories), (1, 0));
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(read(&kept), b"keep");
}

#[test]
fn remove_tree_does_not_follow_links_that_point_to_their_own_ancestors() {
    let fixture = Fixture::new();
    let tree = fixture.dir("tree");
    let sub = tree.join_name("sub").expect("name");
    fs::create_dir(&sub).expect("subdirectory");
    symlink(&tree, sub.join_name("up").expect("name")).expect("link to the ancestor");
    symlink("self", tree.join_name("self").expect("name")).expect("link to itself");
    let stats = fixture
        .fs
        .remove_tree(&fixture.scope, &tree, 2)
        .expect("remove tree");
    assert_eq!((stats.files, stats.directories), (2, 2));
    assert!(!tree.as_path().exists());
}

#[test]
fn remove_file_and_remove_dir_treat_a_link_to_a_directory_as_a_link() {
    let fixture = Fixture::new();
    let outside = fixture.dir("outside");
    let link = fixture.path("link");
    symlink(&outside, &link).expect("link");
    let error = fixture
        .fs
        .remove_dir(&fixture.scope, &link)
        .expect_err("a link is not a directory");
    assert_eq!(error.kind, FsErrorKind::NotADirectory);
    fixture
        .fs
        .remove_file(&fixture.scope, &link)
        .expect("remove link");
    assert!(fs::symlink_metadata(&link).is_err());
    assert!(outside.as_path().is_dir());
}

#[test]
fn rename_moves_a_symbolic_link_and_leaves_its_target() {
    let fixture = Fixture::new();
    let target = fixture.file("target", b"data");
    let link = fixture.path("link");
    symlink("target", &link).expect("link");
    let moved = fixture.path("moved");
    fixture
        .fs
        .rename(&fixture.scope, &link, &moved, RenameMode::NoReplace)
        .expect("rename");
    assert_eq!(
        fixture.fs.read_link(&moved).expect("target"),
        Path::new("target")
    );
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(read(&target), b"data");
}

#[test]
fn copy_of_a_symbolic_link_source_copies_the_content_of_its_target() {
    let fixture = Fixture::new();
    fixture.file("source", b"payload");
    let link = fixture.path("link");
    symlink("source", &link).expect("link");
    let copy = fixture.path("copy");
    let copied = fixture
        .fs
        .copy_file(&fixture.scope, &link, &copy)
        .expect("copy");
    assert_eq!(copied, 7);
    assert!(
        fs::symlink_metadata(&copy)
            .expect("metadata")
            .file_type()
            .is_file()
    );
    assert_eq!(read(&copy), b"payload");
}

#[test]
fn creation_over_a_dangling_link_fails_without_creating_its_target() {
    let fixture = Fixture::new();
    let source = fixture.file("source", b"x");
    let outside = fixture.path("outside");
    let dangling = fixture.path("dangling");
    symlink("outside", &dangling).expect("link");
    let error = fixture
        .fs
        .copy_file(&fixture.scope, &source, &dangling)
        .expect_err("destination exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
    let error = fixture
        .fs
        .create_new(&fixture.scope, &dangling)
        .err()
        .expect("name exists");
    assert_eq!(error.kind, FsErrorKind::Exists);
    assert!(!outside.as_path().exists());
}

#[test]
fn copy_drops_set_user_id_and_sticky_bits_of_the_source() {
    let fixture = Fixture::new();
    let source = fixture.file("source", b"x");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o4755)).expect("mode");
    let target = fixture.path("target");
    fixture
        .fs
        .copy_file(&fixture.scope, &source, &target)
        .expect("copy");
    assert_eq!(fs::metadata(&target).expect("metadata").mode() & 0o7000, 0);
}

#[test]
fn created_files_receive_no_execute_bits() {
    let fixture = Fixture::new();
    let path = fixture.path("new");
    drop(
        fixture
            .fs
            .create_new(&fixture.scope, &path)
            .expect("create"),
    );
    assert_eq!(fs::metadata(&path).expect("metadata").mode() & 0o7111, 0);
}

#[test]
fn set_meta_keeps_the_sticky_bit_of_a_shared_directory() {
    let fixture = Fixture::new();
    let dir = fixture.dir("shared");
    let change = MetaChange {
        mode: Some(0o1777),
        ..MetaChange::default()
    };
    fixture
        .fs
        .set_meta(&fixture.scope, &dir, &change)
        .expect("chmod");
    let meta = fixture.fs.metadata(&dir, Follow::NoFollow).expect("meta");
    assert_eq!(meta.mode, 0o1777);
}

#[test]
fn set_meta_changes_the_times_of_a_link_and_not_of_its_target() {
    let fixture = Fixture::new();
    let target = fixture.file("target", b"x");
    let link = fixture.path("link");
    symlink("target", &link).expect("link");
    let before = fixture
        .fs
        .metadata(&target, Follow::NoFollow)
        .expect("target")
        .modified_ns;
    let change = MetaChange {
        modified_ns: Some(1_000_000_000),
        ..MetaChange::default()
    };
    fixture
        .fs
        .set_meta(&fixture.scope, &link, &change)
        .expect("times of the link");
    let link_meta = fixture.fs.metadata(&link, Follow::NoFollow).expect("link");
    assert_eq!(link_meta.modified_ns, 1_000_000_000);
    let after = fixture
        .fs
        .metadata(&target, Follow::NoFollow)
        .expect("target")
        .modified_ns;
    assert_eq!(after, before);
}

#[test]
fn operations_keep_names_that_are_not_valid_utf8() {
    let fixture = Fixture::new();
    let named = |name: &[u8]| {
        AbsPath::new(fixture.root.as_path().join(OsStr::from_bytes(name))).expect("path")
    };
    let source = named(b"s\xff\xfe");
    fs::write(&source, b"raw").expect("file");
    let copy = named(b"c\xff");
    assert_eq!(
        fixture
            .fs
            .copy_file(&fixture.scope, &source, &copy)
            .expect("copy"),
        3
    );
    let moved = named(b"m\xfe");
    fixture
        .fs
        .rename(&fixture.scope, &copy, &moved, RenameMode::NoReplace)
        .expect("rename");
    let mut names: Vec<Vec<u8>> = fixture
        .fs
        .read_dir(&fixture.root)
        .expect("listing")
        .map(|entry| entry.expect("entry").name.as_bytes().to_vec())
        .collect();
    names.sort();
    assert_eq!(names, [b"m\xfe".to_vec(), b"s\xff\xfe".to_vec()]);
    assert_eq!(
        fixture.fs.content_hash(&moved).expect("hash"),
        fixture.fs.content_hash(&source).expect("hash")
    );
    let stats = fixture
        .fs
        .remove_tree(&fixture.scope, &moved, 1)
        .expect("remove");
    assert_eq!((stats.files, stats.bytes), (1, 3));
    assert!(!moved.as_path().exists());
}

/// Modification time given to the replaced file: 2021-05-06T00:00:00Z.
const OLD_MODIFIED_NS: i64 = 1_620_259_200_000_000_000;
/// One hour in nanoseconds.
const HOUR_NS: i64 = 3_600_000_000_000;

/// Replaces `target` by a staged file with new content, as the journal does:
/// the content is written, the metadata of `previous` is transferred through
/// the handle (completely when `full`, prepared for a content change
/// otherwise) and the staged file is renamed over the target.
fn switch_in(fixture: &Fixture, target: &AbsPath, previous: &ExtendedMeta, full: bool) {
    let mut handle = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.root, "target")
        .expect("staged file");
    handle.write_all(b"new line\n").expect("write");
    handle.sync(Durability::Data).expect("data flush");
    let written = fixture
        .fs
        .metadata(handle.path(), Follow::NoFollow)
        .expect("metadata of the written file");
    let wanted = if full {
        previous.clone()
    } else {
        crate::transfer::for_content_change(previous, &written)
    };
    handle.apply_meta(&wanted).expect("apply metadata");
    handle.sync(Durability::Full).expect("flush");
    let staged = handle.path().clone();
    drop(handle);
    fixture
        .fs
        .rename(&fixture.scope, &staged, target, RenameMode::Replace)
        .expect("switch in");
}

fn aged_target(fixture: &Fixture) -> (AbsPath, ExtendedMeta, bool) {
    let target = fixture.file("target", b"old");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).expect("mode");
    let attribute = rustix::fs::setxattr(
        target.as_path(),
        "user.origin",
        b"test",
        rustix::fs::XattrFlags::empty(),
    )
    .is_ok();
    let change = MetaChange {
        modified_ns: Some(OLD_MODIFIED_NS),
        ..MetaChange::default()
    };
    fixture
        .fs
        .set_meta(&fixture.scope, &target, &change)
        .expect("age the file");
    let previous = fixture
        .fs
        .extended_metadata(&target, Follow::NoFollow)
        .expect("metadata");
    assert_eq!(previous.basic.modified_ns, OLD_MODIFIED_NS);
    (target, previous, attribute)
}

#[test]
fn content_switch_keeps_mode_owner_and_attributes_and_takes_the_write_time() {
    let fixture = Fixture::new();
    let (target, previous, attribute) = aged_target(&fixture);
    switch_in(&fixture, &target, &previous, false);
    assert_eq!(read(&target), b"new line\n");
    let after = fixture
        .fs
        .extended_metadata(&target, Follow::NoFollow)
        .expect("metadata after");
    assert_eq!(after.basic.mode, 0o640);
    assert_eq!(
        (after.basic.uid, after.basic.gid),
        (previous.basic.uid, previous.basic.gid)
    );
    if attribute {
        assert!(
            after
                .xattrs
                .iter()
                .any(|xattr| xattr.name == "user.origin" && xattr.value == b"test")
        );
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock");
    let now_ns = i64::try_from(now.as_nanos()).expect("time");
    assert!(after.basic.modified_ns > OLD_MODIFIED_NS);
    assert!(
        (now_ns - after.basic.modified_ns).abs() < HOUR_NS,
        "modification time {} is not the time of the write",
        after.basic.modified_ns
    );
}

#[test]
fn full_metadata_transfer_restores_the_old_modification_time() {
    let fixture = Fixture::new();
    let (target, previous, _) = aged_target(&fixture);
    switch_in(&fixture, &target, &previous, true);
    assert_eq!(read(&target), b"new line\n");
    let after = fixture
        .fs
        .metadata(&target, Follow::NoFollow)
        .expect("metadata after");
    assert_eq!(after.modified_ns, OLD_MODIFIED_NS);
    assert_eq!(after.mode, 0o640);
}

#[test]
fn staged_file_replaces_the_target_with_one_rename() {
    let fixture = Fixture::new();
    let target = fixture.file("target", b"old");
    let mut handle = fixture
        .fs
        .create_temp_in(&fixture.scope, &fixture.root, "target")
        .expect("staged file");
    handle.write_all(b"new content").expect("write");
    handle.sync(Durability::Full).expect("flush");
    let staged = handle.path().clone();
    let staged_inode = inode_of(&staged);
    drop(handle);
    fixture
        .fs
        .rename(&fixture.scope, &staged, &target, RenameMode::Replace)
        .expect("rename over the target");
    fixture
        .fs
        .sync_dir(&fixture.scope, &fixture.root)
        .expect("directory flush");
    assert_eq!(read(&target), b"new content");
    assert_eq!(inode_of(&target), staged_inode);
    assert!(!staged.as_path().exists());
}

#[test]
fn path_keys_are_raw_bytes_on_case_sensitive_volumes() {
    let fixture = Fixture::new();
    let upper = fixture.path("Name");
    let lower = fixture.path("name");
    let key = fixture.fs.path_key(&upper).expect("key");
    assert_eq!(key.0, upper.as_path().as_os_str().as_encoded_bytes());
    assert_ne!(key, fixture.fs.path_key(&lower).expect("key"));
    let non_utf8 = OsStr::from_bytes(b"n\xff");
    let raw = AbsPath::new(fixture.root.as_path().join(non_utf8)).expect("path");
    assert_eq!(
        fixture.fs.path_key(&raw).expect("key").0,
        raw.as_path().as_os_str().as_encoded_bytes()
    );
}
