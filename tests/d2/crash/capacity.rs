// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Capacity limits of `CrashFs`.

use std::io::Write;

use nurt_contracts::fs::{AbsPath, Durability, Fs, FsErrorKind, FsResult, FsWrite, RenameMode};
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};

use crate::support::{at, content, durable_file, exists, scope, volume_root};

/// Capacity of the volumes in these tests: 1 KiB and 8 entries.
const BYTES: u64 = 1024;
const INODES: u64 = 8;

fn small_fs(quota: Option<u64>) -> CrashFs {
    let mut spec = VolumeSpec::new(volume_root()).with_capacity(BYTES, INODES);
    if let Some(quota) = quota {
        spec = spec.with_quota(quota);
    }
    CrashFs::new(CrashProfile::PosixWorstCase, 1, vec![spec]).unwrap()
}

fn used(fs: &CrashFs) -> (u64, u64) {
    let info = fs.space(&fs.volume(&volume_root()).unwrap().id).unwrap();
    (info.total, info.available)
}

#[test]
fn crashfs_capacity_rejects_write_beyond_capacity() {
    let fs = small_fs(None);
    let mut file = fs.create_new(&scope(), &at("f")).unwrap();
    file.write_all(&[0; 1000]).unwrap();
    let err = file.write_all(&[0; 100]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(used(&fs), (BYTES, 24));
}

#[test]
fn crashfs_capacity_reports_quota_before_capacity() {
    let fs = small_fs(Some(512));
    assert_eq!(used(&fs), (512, 512));
    durable_file(&fs, &at("a"), &[1; 500]);
    let err = fs.copy_file(&scope(), &at("a"), &at("b")).unwrap_err();
    assert_eq!(err.kind, FsErrorKind::QuotaExceeded);
    assert!(!exists(&fs, &at("b")));
    let info = fs.space(&fs.volume(&volume_root()).unwrap().id).unwrap();
    assert_eq!(info.quota_remaining, Some(12));
}

#[test]
fn crashfs_capacity_limits_entries() {
    let fs = small_fs(None);
    for index in 1..INODES {
        fs.create_dir(&scope(), &at(&format!("d{index}")), 0o755)
            .unwrap();
    }
    let err = fs.create_dir(&scope(), &at("last"), 0o755).unwrap_err();
    assert_eq!(err.kind, FsErrorKind::NoSpace);
    fs.remove_dir(&scope(), &at("d1")).unwrap();
    fs.create_dir(&scope(), &at("last"), 0o755).unwrap();
}

#[test]
fn crashfs_capacity_space_info_follows_usage() {
    let fs = small_fs(None);
    durable_file(&fs, &at("a"), &[1; 300]);
    assert_eq!(used(&fs), (BYTES, BYTES - 300));
    fs.truncate(&scope(), &at("a"), 100).unwrap();
    assert_eq!(used(&fs), (BYTES, BYTES - 100));
    fs.hard_link(&scope(), &at("a"), &at("b")).unwrap();
    assert_eq!(
        used(&fs),
        (BYTES, BYTES - 100),
        "a second name takes no space"
    );
    fs.remove_file(&scope(), &at("a")).unwrap();
    fs.remove_file(&scope(), &at("b")).unwrap();
    assert_eq!(used(&fs), (BYTES, BYTES));
    fs.crash();
    assert!(used(&fs).1 <= BYTES);
}

/// Steps of a small workload; each needs space or an entry.
fn workload(fs: &CrashFs) -> Vec<Box<dyn Fn() -> FsResult<()> + '_>> {
    let file = |name: &str| at(name);
    vec![
        Box::new(move || fs.create_dir(&scope(), &file("d"), 0o755)),
        Box::new(move || {
            let mut handle = fs.create_new(&scope(), &file("f"))?;
            handle.write_all(&[2; 200]).map_err(|_| {
                nurt_contracts::fs::FsError::new(FsErrorKind::NoSpace, "write", None)
            })?;
            handle.sync(Durability::Data)
        }),
        Box::new(move || fs.copy_file(&scope(), &file("f"), &file("g")).map(|_| ())),
        Box::new(move || fs.truncate(&scope(), &file("g"), 400)),
        Box::new(move || fs.clone_file(&scope(), &file("g"), &file("h"))),
        Box::new(move || fs.rename(&scope(), &file("h"), &file("i"), RenameMode::NoReplace)),
    ]
}

#[test]
fn crashfs_capacity_fails_any_step_with_no_space() {
    let total: u64 = 200 + 200 + 200 + 400;
    for capacity in (0..=total).step_by(50) {
        let fs = CrashFs::new(
            CrashProfile::PosixWorstCase,
            capacity,
            vec![VolumeSpec::new(volume_root()).with_capacity(capacity, 64)],
        )
        .unwrap();
        for step in workload(&fs) {
            let before = fs.space(&fs.volume(&volume_root()).unwrap().id).unwrap();
            match step() {
                Ok(()) => {}
                Err(err) => {
                    assert!(
                        matches!(err.kind, FsErrorKind::NoSpace | FsErrorKind::NotFound),
                        "capacity {capacity}: {err}"
                    );
                    let after = fs.space(&fs.volume(&volume_root()).unwrap().id).unwrap();
                    if err.kind == FsErrorKind::NoSpace {
                        assert!(after.available <= before.available);
                    }
                }
            }
            let info = fs.space(&fs.volume(&volume_root()).unwrap().id).unwrap();
            assert!(info.available <= info.total);
        }
    }
}

#[test]
fn crashfs_capacity_keeps_second_volume_separate() {
    let other: AbsPath = crate::support::second_root();
    let fs = CrashFs::new(
        CrashProfile::PosixWorstCase,
        2,
        vec![
            VolumeSpec::new(volume_root()).with_capacity(BYTES, INODES),
            VolumeSpec::new(other.clone()),
        ],
    )
    .unwrap();
    durable_file(&fs, &at("a"), &[3; 900]);
    let target = other.join_name("a").unwrap();
    assert_eq!(fs.copy_file(&scope(), &at("a"), &target).unwrap(), 900);
    let err = fs
        .rename(
            &scope(),
            &at("a"),
            &other.join_name("b").unwrap(),
            RenameMode::Replace,
        )
        .unwrap_err();
    assert_eq!(err.kind, FsErrorKind::CrossDevice);
    assert_eq!(content(&fs, &target).unwrap(), vec![3; 900]);
    assert_ne!(
        fs.volume(&target).unwrap().id,
        fs.volume(&at("a")).unwrap().id
    );
}
