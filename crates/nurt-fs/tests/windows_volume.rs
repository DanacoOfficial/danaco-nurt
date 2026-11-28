// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-fs/tests/windows_volume.rs
// Responsibility:   Volume description, space figures, journal directory attributes, flushes and block cloning.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Volume description, space figures, journal directory attributes, flushes
//! and block cloning.
//!
//! The temporary directory of the account is expected on a fixed NTFS volume.
//! Block cloning (`FSCTL_DUPLICATE_EXTENTS_TO_FILE`) needs a directory on
//! `ReFS` named by `NURT_TEST_REFS_DIR`; that test is ignored by default.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod common;

use std::io::{Seek, SeekFrom, Write};

use nurt_contracts::fs::{Durability, Follow, Fs, FsErrorKind, FsWrite, VolumeKind};
use nurt_fs::windows::meta::JOURNAL_DIRECTORY_ATTRIBUTES;

use common::{Fixture, fixture, fixture_in, refs_dir};

/// Sizes of cloned files: below one cluster and not a multiple of the
/// cluster size.
const CLONE_SIZES: [usize; 2] = [5_000, 3 * 65_536 + 17];
/// Multiplier and modulus of the deterministic content pattern; the modulus
/// is prime so that the pattern does not repeat at cluster boundaries.
const PATTERN_STEP: usize = 31;
const PATTERN_MODULUS: usize = 251;

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| u8::try_from(index * PATTERN_STEP % PATTERN_MODULUS).unwrap())
        .collect()
}

#[test]
fn describes_the_temporary_volume_as_fixed_ntfs() {
    let fx = fixture();
    let volume = fx.fs.volume(&fx.root).unwrap();
    assert_eq!(volume.fs_type, "ntfs", "{volume:?}");
    assert_eq!(volume.kind, VolumeKind::Fixed, "{volume:?}");
    assert!(fx.root.starts_with(&volume.mount), "{volume:?}");
    assert!(volume.id.uuid.is_some(), "{volume:?}");
    assert!(!volume.read_only);
    let features = volume.features;
    let declared = [
        features.replace_with_backup,
        features.hardlink,
        features.acl,
        features.alternate_streams,
        features.durable_metadata,
        features.rename_noreplace,
        features.exchange,
        features.xattr,
        features.case_sensitive,
        features.reflink,
    ];
    let expected = [
        true, true, true, true, true, true, false, false, false, false,
    ];
    assert_eq!(declared, expected, "{features:?}");
    let missing = fx.path("missing").join_name("child").unwrap();
    assert_eq!(fx.fs.volume(&missing).unwrap().id, volume.id);
}

#[test]
fn reports_consistent_space_figures() {
    let fx = fixture();
    let volume = fx.fs.volume(&fx.root).unwrap();
    let space = fx.fs.space(&volume.id).unwrap();
    assert!(space.total > 0, "{space:?}");
    assert!(space.available <= space.total, "{space:?}");
    assert!(
        space
            .quota_remaining
            .is_none_or(|quota| quota <= space.total),
        "{space:?}"
    );
    assert_eq!(space.purgeable_system, None);
}

#[test]
fn marks_the_journal_directory_hidden_system_and_not_indexed() {
    let fx = fixture();
    let dir = fx.dir("journal");
    fx.fs.mark_hidden(&fx.scope, &dir).unwrap();
    let meta = fx.fs.metadata(&dir, Follow::NoFollow).unwrap();
    assert_eq!(
        meta.windows_attributes & JOURNAL_DIRECTORY_ATTRIBUTES,
        JOURNAL_DIRECTORY_ATTRIBUTES,
        "attributes {:#x}",
        meta.windows_attributes
    );
    let error = fx
        .fs
        .mark_hidden(&fx.scope, &fx.path("missing"))
        .unwrap_err();
    assert_eq!(error.kind, FsErrorKind::NotFound, "{error}");
}

#[test]
fn flushes_files_and_directories() {
    let fx = fixture();
    let path = fx.path("flushed.txt");
    let mut handle = fx.fs.create_new(&fx.scope, &path).unwrap();
    handle.write_all(b"data").unwrap();
    handle.sync(Durability::Data).unwrap();
    handle.set_length(2).unwrap();
    handle.sync(Durability::Full).unwrap();
    drop(handle);
    fx.fs.sync_dir(&fx.scope, &fx.root).unwrap();
    assert_eq!(fx.read(&path), b"da");
}

#[test]
fn clone_on_ntfs_is_unsupported_and_creates_nothing() {
    let fx = fixture();
    let source = fx.file("source.bin", &pattern(CLONE_SIZES[0]));
    let clone = fx.path("clone.bin");
    let error = fx.fs.clone_file(&fx.scope, &source, &clone).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Unsupported, "{error}");
    assert!(!fx.exists(&clone));
}

fn assert_independent_clone(fx: &Fixture, size: usize) {
    let content = pattern(size);
    let source = fx.file(&format!("source-{size}.bin"), &content);
    let clone = fx.path(&format!("clone-{size}.bin"));
    fx.fs.clone_file(&fx.scope, &source, &clone).unwrap();
    assert_eq!(fx.read(&clone), content, "{size}");
    let mut handle = fx.fs.open_in_place(&fx.scope, &clone).unwrap();
    handle.seek(SeekFrom::Start(0)).unwrap();
    handle.write_all(b"changed").unwrap();
    handle.sync(Durability::Full).unwrap();
    drop(handle);
    assert_eq!(
        fx.read(&source),
        content,
        "the clone shares writes with {size}"
    );
    let error = fx.fs.clone_file(&fx.scope, &source, &clone).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Exists, "{error}");
}

#[test]
#[ignore = "requires NURT_TEST_REFS_DIR naming a directory on a ReFS volume"]
fn clones_blocks_on_refs() {
    let fx = fixture_in(&refs_dir());
    let volume = fx.fs.volume(&fx.root).unwrap();
    assert_eq!(volume.fs_type, "refs", "{volume:?}");
    assert!(volume.features.reflink, "{:?}", volume.features);
    for size in CLONE_SIZES {
        assert_independent_clone(&fx, size);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
