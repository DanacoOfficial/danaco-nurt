// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::ffi::{CString, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, Durability, Follow, Fs, FsErrorKind, FsWrite, LinkKind, RenameMode, ScopePurpose,
    TxScope, VolumeKind, Xattr,
};
use nurt_contracts::journal::OpId;
use tempfile::TempDir;

use super::MacosFs;
use crate::macos::UF_HIDDEN;

/// ACL text with one deny entry for the `everyone` group (gid 12), whose
/// UUID is fixed on every macOS installation.
const EVERYONE_DENY_DELETE: &[u8] =
    b"!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF0000000C:everyone:12:deny:delete\n";
/// Creation time used by the metadata round trip: 2001-09-09T01:46:40Z.
const CREATED_NS: i64 = 1_000_000_000_000_000_000;
/// Modification time used by the metadata round trip, later than
/// [`CREATED_NS`].
const MODIFIED_NS: i64 = 1_100_000_000_000_000_000;

#[expect(
    clippy::disallowed_methods,
    reason = "back end tests act as the journal that owns the scope"
)]
fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
}

fn root() -> (TempDir, AbsPath) {
    let dir = tempfile::tempdir().unwrap();
    let path = AbsPath::new(dir.path().canonicalize().unwrap()).unwrap();
    (dir, path)
}

fn write(fs: &MacosFs, path: &AbsPath, content: &[u8]) {
    let mut handle = fs.create_new(&scope(), path).unwrap();
    handle.write_all(content).unwrap();
    handle.sync(Durability::Full).unwrap();
}

fn read(path: &AbsPath) -> Vec<u8> {
    std::fs::read(path.as_path()).unwrap()
}

#[test]
fn passes_conformance_suite_on_temporary_volume() {
    let (_dir, root) = root();
    let report = crate::conformance::run(&MacosFs::new(), &root, &scope());
    assert!(report.passed(), "{report}");
}

#[test]
fn swaps_files_and_directories_atomically() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let (a, b) = (root.join_name("a").unwrap(), root.join_name("b").unwrap());
    write(&fs, &a, b"old");
    write(&fs, &b, b"new");
    fs.exchange(&scope(), &a, &b).unwrap();
    assert_eq!((read(&a), read(&b)), (b"new".to_vec(), b"old".to_vec()));

    let (d1, d2) = (root.join_name("d1").unwrap(), root.join_name("d2").unwrap());
    fs.create_dir(&scope(), &d1, 0o755).unwrap();
    fs.create_dir(&scope(), &d2, 0o700).unwrap();
    fs.exchange(&scope(), &d1, &d2).unwrap();
    assert_eq!(fs.metadata(&d1, Follow::NoFollow).unwrap().mode, 0o700);
}

#[test]
fn refuses_rename_without_replacement_onto_existing_name() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let (a, b) = (root.join_name("a").unwrap(), root.join_name("b").unwrap());
    write(&fs, &a, b"a");
    write(&fs, &b, b"b");
    let error = fs
        .rename(&scope(), &a, &b, RenameMode::NoReplace)
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Exists);
    fs.rename(&scope(), &a, &b, RenameMode::Replace).unwrap();
    assert_eq!(read(&b), b"a");
}

#[test]
fn clones_on_apfs_and_reports_unsupported_elsewhere() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let (src, dst) = (
        root.join_name("src").unwrap(),
        root.join_name("dst").unwrap(),
    );
    write(&fs, &src, b"content");
    let result = fs.clone_file(&scope(), &src, &dst);
    if fs.volume(&root).unwrap().features.reflink {
        result.unwrap();
        assert_eq!(read(&dst), b"content");
    } else {
        assert_eq!(result.unwrap_err().kind, FsErrorKind::Unsupported);
    }
}

#[test]
fn restores_complete_metadata_onto_new_file() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let (old, new) = (
        root.join_name("old").unwrap(),
        root.join_name("new").unwrap(),
    );
    write(&fs, &old, b"old");
    write(&fs, &new, b"new");
    let mut meta = fs.extended_metadata(&old, Follow::NoFollow).unwrap();
    meta.xattrs = vec![
        Xattr {
            name: OsString::from("com.apple.ResourceFork"),
            value: b"fork".to_vec(),
        },
        Xattr {
            name: OsString::from("com.apple.quarantine"),
            value: b"0081;00000000;Nurt;".to_vec(),
        },
    ];
    meta.acl = Some(EVERYONE_DENY_DELETE.to_vec());
    meta.flags = Some(u64::from(UF_HIDDEN | libc::UF_NODUMP));
    meta.basic.mode = 0o640;
    meta.basic.created_ns = Some(CREATED_NS);
    meta.basic.modified_ns = MODIFIED_NS;
    meta.basic.accessed_ns = Some(MODIFIED_NS);
    let report = fs.restore_meta(&scope(), &old, &meta).unwrap();
    assert!(report.is_complete(), "{report:?}");
    let expected = fs.extended_metadata(&old, Follow::NoFollow).unwrap();

    let report = fs.restore_meta(&scope(), &new, &expected).unwrap();
    assert!(report.is_complete(), "{report:?}");
    let actual = fs.extended_metadata(&new, Follow::NoFollow).unwrap();
    assert_eq!(actual.xattrs, expected.xattrs);
    assert_eq!(actual.acl, expected.acl);
    assert_eq!(actual.flags, expected.flags);
    assert_eq!(actual.basic.mode, 0o640);
    assert_eq!(actual.basic.created_ns, Some(CREATED_NS));
    assert_eq!(actual.basic.modified_ns, MODIFIED_NS);
    assert_eq!(actual.basic.uid, expected.basic.uid);
}

#[test]
fn flushes_files_and_directories_with_full_fsync() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let mut handle = fs.create_temp_in(&scope(), &root, "flush").unwrap();
    handle.write_all(b"data").unwrap();
    handle.sync(Durability::Full).unwrap();
    handle.sync(Durability::Data).unwrap();
    fs.sync_dir(&scope(), &root).unwrap();
}

#[test]
fn describes_temporary_volume_and_its_space() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let volume = fs.volume(&root.join_name("missing").unwrap()).unwrap();
    assert_eq!(volume.kind, VolumeKind::Fixed);
    if volume.fs_type == "apfs" {
        assert!(volume.features.exchange && volume.features.reflink);
        assert!(volume.id.uuid.is_some());
    }
    let space = fs.space(&volume.id).unwrap();
    assert!(space.total >= space.available);
    assert!(space.available > 0);
}

#[test]
fn removes_tree_without_following_symbolic_links() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let (outside, tree) = (
        root.join_name("outside").unwrap(),
        root.join_name("tree").unwrap(),
    );
    fs.create_dir(&scope(), &outside, 0o755).unwrap();
    let kept = outside.join_name("kept").unwrap();
    write(&fs, &kept, b"kept");
    fs.create_dir(&scope(), &tree, 0o755).unwrap();
    let nested = tree.join_name("nested").unwrap();
    fs.create_dir(&scope(), &nested, 0o755).unwrap();
    write(&fs, &nested.join_name("file").unwrap(), b"12345");
    let link = tree.join_name("link").unwrap();
    fs.create_link(
        &scope(),
        outside.as_path(),
        &link,
        LinkKind::SymlinkDirectory,
    )
    .unwrap();
    let stats = fs.remove_tree(&scope(), &tree, 4).unwrap();
    assert_eq!((stats.files, stats.directories, stats.bytes), (2, 2, 5));
    assert_eq!(read(&kept), b"kept");
    assert_eq!(
        fs.metadata(&tree, Follow::NoFollow).unwrap_err().kind,
        FsErrorKind::NotFound
    );
}

#[test]
fn reports_directory_given_to_remove_file() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let dir = root.join_name("dir").unwrap();
    fs.create_dir(&scope(), &dir, 0o755).unwrap();
    let error = fs.remove_file(&scope(), &dir).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::IsADirectory);
}

#[test]
fn times_out_on_lock_held_by_another_descriptor() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let path = root.join_name("lock").unwrap();
    let held = fs.lock_exclusive(&path, Duration::ZERO).unwrap();
    let error = fs
        .lock_exclusive(&path, Duration::from_millis(30))
        .err()
        .unwrap();
    assert_eq!(error.kind, FsErrorKind::InUse);
    drop(held);
    fs.lock_exclusive(&path, Duration::ZERO).unwrap();
}

#[test]
fn copy_refuses_directories_and_fifos_without_creating_the_destination() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let dir = root.join_name("dir").unwrap();
    fs.create_dir(&scope(), &dir, 0o755).unwrap();
    let target = root.join_name("target").unwrap();
    let error = fs.copy_file(&scope(), &dir, &target).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::IsADirectory);
    assert!(fs.metadata(&target, Follow::NoFollow).is_err());

    let fifo = root.join_name("fifo").unwrap();
    let name = CString::new(fifo.as_path().as_os_str().as_bytes()).unwrap();
    // SAFETY: `name` is a NUL-terminated string that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let error = fs.copy_file(&scope(), &fifo, &target).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Unsupported);
    assert!(fs.metadata(&target, Follow::NoFollow).is_err());
}

#[test]
fn lock_with_the_longest_timeout_is_taken_when_the_file_is_free() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let path = root.join_name("lock").unwrap();
    let lock = fs.lock_exclusive(&path, Duration::MAX).unwrap();
    assert_eq!(lock.path(), &path);
}

#[test]
fn lock_refuses_a_symbolic_link_at_the_lock_path() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let (target, link) = (
        root.join_name("target").unwrap(),
        root.join_name("lock").unwrap(),
    );
    std::os::unix::fs::symlink(target.as_path(), link.as_path()).unwrap();
    let error = fs.lock_exclusive(&link, Duration::ZERO).err().unwrap();
    assert_eq!(error.kind, FsErrorKind::Loop);
    assert!(!target.as_path().exists());
}

#[test]
fn hides_noindex_directory_and_keys_it_case_insensitively() {
    let fs = MacosFs::new();
    let (_dir, root) = root();
    let journal = root.join_name(".nurt-journal.noindex").unwrap();
    fs.create_dir(&scope(), &journal, 0o700).unwrap();
    fs.mark_hidden(&scope(), &journal).unwrap();
    let meta = fs.extended_metadata(&journal, Follow::NoFollow).unwrap();
    assert_ne!(meta.flags.unwrap() & u64::from(UF_HIDDEN), 0);
    if !fs.volume(&root).unwrap().features.case_sensitive {
        let spelled = root.join_name(".NURT-JOURNAL.NOINDEX").unwrap();
        assert_eq!(
            fs.path_key(&journal).unwrap(),
            fs.path_key(&spelled).unwrap()
        );
    }
}

#[test]
fn metadata_of_a_file_without_read_permission_is_changed_by_path() {
    use nurt_contracts::fs::MetaChange;

    let fs = MacosFs::new();
    let (_dir, root) = root();
    let file = root.join_name("sealed").unwrap();
    write(&fs, &file, b"x");
    let seal = MetaChange {
        mode: Some(0o000),
        ..MetaChange::default()
    };
    fs.set_meta(&scope(), &file, &seal).unwrap();
    let wanted = MetaChange {
        mode: Some(0o640),
        modified_ns: Some(MODIFIED_NS),
        set_xattrs: vec![Xattr {
            name: OsString::from("user.kept"),
            value: b"1".to_vec(),
        }],
        ..MetaChange::default()
    };
    fs.set_meta(&scope(), &file, &wanted).unwrap();
    let meta = fs.extended_metadata(&file, Follow::NoFollow).unwrap();
    assert_eq!(meta.basic.mode, 0o640);
    assert_eq!(meta.basic.modified_ns, MODIFIED_NS);
    fs.set_meta(&scope(), &file, &seal).unwrap();
    let report = fs.restore_meta(&scope(), &file, &meta).unwrap();
    assert!(report.not_transferred.is_empty(), "{report:?}");
    assert_eq!(
        fs.extended_metadata(&file, Follow::NoFollow)
            .unwrap()
            .basic
            .mode,
        0o640
    );
}
