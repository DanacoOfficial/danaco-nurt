// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Alternate data streams and security descriptors on NTFS.
//!
//! Without `SeSecurityPrivilege` enabled the SACL is not captured, and its
//! restoration is reported as not transferred. The SACL round trip enables
//! the privilege, needs an elevated account and is ignored by default.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod common;

use std::io::Write;

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, Follow, Fs, FsErrorKind, FsWrite, MetaTransferReport,
};
use nurt_fs::windows::security::control::SE_DACL_PROTECTED;
use nurt_fs::windows::security::{SecurityDescriptor, security_differences};

use common::descriptor::{
    Builder, EVERYONE, FILE_ALL_ACCESS, FILE_GENERIC_READ, FILE_GENERIC_WRITE, LOCAL_SYSTEM, allow,
    audit,
};
use common::{Fixture, fixture, privilege};

const MAIN: &[u8] = b"main stream";
/// Stream written by browsers for downloaded files.
const ZONE_STREAM: &str = "Zone.Identifier";
const ZONE: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";
const OWN_STREAM: &str = "nurt.data";
/// Binary content with zero and high bytes.
const OWN: &[u8] = &[0, 1, 2, 3, 0x7f, 0x80, 0xfe, 0xff];

fn stream_size(meta: &ExtendedMeta, name: &str) -> Option<u64> {
    meta.streams
        .iter()
        .find(|stream| stream.name == name)
        .map(|stream| stream.size)
}

fn file_with_streams(fx: &Fixture, name: &str) -> AbsPath {
    let path = fx.file(name, MAIN);
    fx.write_stream(&path, ZONE_STREAM, ZONE);
    fx.write_stream(&path, OWN_STREAM, OWN);
    path
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap()
}

/// Elements the descriptors differ in.
fn differences(before: &ExtendedMeta, after: &ExtendedMeta) -> Vec<&'static str> {
    security_differences(
        before.acl.as_deref().expect("descriptor before"),
        after.acl.as_deref().expect("descriptor after"),
    )
    .unwrap()
}

fn dacl_protected(meta: &ExtendedMeta) -> bool {
    let descriptor = SecurityDescriptor::parse(meta.acl.as_deref().unwrap()).unwrap();
    descriptor.control() & SE_DACL_PROTECTED != 0
}

fn sacl_captured(meta: &ExtendedMeta) -> bool {
    SecurityDescriptor::parse(meta.acl.as_deref().unwrap())
        .unwrap()
        .sacl_captured()
}

/// Every element was restored, except a SACL that was not captured.
fn assert_complete_but_sacl(report: &MetaTransferReport) {
    assert!(
        report
            .not_transferred
            .iter()
            .all(|element| element == "sacl"),
        "{report:?}"
    );
}

/// Protected DACL distinct from any inherited one; `account` keeps full
/// access.
fn protected_dacl(account: &[u8], everyone: u32) -> Vec<u8> {
    Builder::default()
        .dacl(
            vec![
                allow(account, FILE_ALL_ACCESS),
                allow(LOCAL_SYSTEM, FILE_ALL_ACCESS),
                allow(EVERYONE, everyone),
            ],
            true,
        )
        .build()
}

#[test]
fn lists_and_reads_alternate_streams() {
    let fx = fixture();
    let path = file_with_streams(&fx, "streams.txt");
    let meta = fx.extended(&path);
    assert_eq!(meta.streams.len(), 2, "{:?}", meta.streams);
    assert_eq!(stream_size(&meta, ZONE_STREAM), Some(len(ZONE)));
    assert_eq!(stream_size(&meta, OWN_STREAM), Some(len(OWN)));
    assert_eq!(meta.basic.size, len(MAIN));
    assert!(meta.xattrs.is_empty());
    assert_eq!(fx.read_stream(&path, ZONE_STREAM), ZONE);
    assert_eq!(fx.read_stream(&path, OWN_STREAM), OWN);
    assert_eq!(fx.read(&path), MAIN);
}

#[test]
fn rewriting_a_stream_truncates_it_and_keeps_the_main_content() {
    let fx = fixture();
    let path = file_with_streams(&fx, "rewrite.txt");
    fx.write_stream(&path, OWN_STREAM, b"x");
    assert_eq!(fx.read_stream(&path, OWN_STREAM), b"x");
    assert_eq!(fx.read(&path), MAIN);
    assert_eq!(
        fx.fs.metadata(&path, Follow::NoFollow).unwrap().size,
        len(MAIN)
    );
}

#[test]
fn copy_includes_every_stream_and_counts_their_bytes() {
    let fx = fixture();
    let source = file_with_streams(&fx, "source.txt");
    let copy = fx.path("copy.txt");
    let copied = fx.fs.copy_file(&fx.scope, &source, &copy).unwrap();
    assert_eq!(copied, len(MAIN) + len(ZONE) + len(OWN));
    assert_eq!(fx.read(&copy), MAIN);
    assert_eq!(fx.read_stream(&copy, ZONE_STREAM), ZONE);
    assert_eq!(fx.read_stream(&copy, OWN_STREAM), OWN);
    let mut source_streams = fx.extended(&source).streams;
    let mut copy_streams = fx.extended(&copy).streams;
    source_streams.sort_by(|a, b| a.name.cmp(&b.name));
    copy_streams.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(copy_streams, source_streams);
    let error = fx.fs.copy_file(&fx.scope, &source, &copy).unwrap_err();
    assert_eq!(error.kind, FsErrorKind::Exists, "{error}");
}

#[test]
fn missing_stream_is_not_found() {
    let fx = fixture();
    let path = fx.file("plain.txt", MAIN);
    let error = fx.fs.open_stream_read(&path, OWN_STREAM).err().unwrap();
    assert_eq!(error.kind, FsErrorKind::NotFound, "{error}");
    let missing = fx.path("missing.txt");
    let error = fx
        .fs
        .open_stream_write(&fx.scope, &missing, OWN_STREAM)
        .err()
        .unwrap();
    assert_eq!(error.kind, FsErrorKind::NotFound, "{error}");
    assert!(!fx.exists(&missing));
}

#[test]
fn restores_an_inherited_dacl_after_protection() {
    let fx = fixture();
    let path = fx.file("inherited.txt", MAIN);
    let before = fx.extended(&path);
    assert!(!dacl_protected(&before), "a new file inherits its DACL");
    let account = fx.account(&path);
    fx.set_descriptor(&path, protected_dacl(&account, FILE_GENERIC_READ));
    let changed = fx.extended(&path);
    assert!(dacl_protected(&changed));
    assert!(differences(&before, &changed).contains(&"dacl"));
    let report = fx.fs.restore_meta(&fx.scope, &path, &before).unwrap();
    assert_complete_but_sacl(&report);
    let after = fx.extended(&path);
    assert!(differences(&before, &after).is_empty(), "{after:?}");
    assert!(!dacl_protected(&after));
}

#[test]
fn restores_a_protected_dacl() {
    let fx = fixture();
    let path = fx.file("protected.txt", MAIN);
    let account = fx.account(&path);
    fx.set_descriptor(&path, protected_dacl(&account, FILE_GENERIC_READ));
    let before = fx.extended(&path);
    fx.set_descriptor(
        &path,
        protected_dacl(&account, FILE_GENERIC_READ | FILE_GENERIC_WRITE),
    );
    assert!(differences(&before, &fx.extended(&path)).contains(&"dacl"));
    let report = fx.fs.restore_meta(&fx.scope, &path, &before).unwrap();
    assert_complete_but_sacl(&report);
    let after = fx.extended(&path);
    assert!(differences(&before, &after).is_empty(), "{after:?}");
    assert!(dacl_protected(&after));
}

#[test]
fn reports_an_uncaptured_sacl_as_not_transferred() {
    let fx = fixture();
    let path = fx.file("sacl.txt", MAIN);
    let before = fx.extended(&path);
    let captured = sacl_captured(&before);
    let report = fx.fs.restore_meta(&fx.scope, &path, &before).unwrap();
    let reported = report
        .not_transferred
        .iter()
        .any(|element| element == "sacl");
    assert_eq!(reported, !captured, "{report:?}");
}

#[test]
fn write_handle_takes_the_descriptor_of_the_preserved_file() {
    let fx = fixture();
    let source = fx.file("source.txt", MAIN);
    let account = fx.account(&source);
    fx.set_descriptor(&source, protected_dacl(&account, FILE_GENERIC_READ));
    let preserved = fx.extended(&source);
    let mut handle = fx.fs.create_temp_in(&fx.scope, &fx.root, "staged").unwrap();
    handle.write_all(MAIN).unwrap();
    let report = handle.apply_meta(&preserved).unwrap();
    handle.sync(Durability::Full).unwrap();
    let staged = handle.path().clone();
    drop(handle);
    assert_complete_but_sacl(&report);
    let after = fx.extended(&staged);
    assert!(differences(&preserved, &after).is_empty(), "{after:?}");
    assert!(dacl_protected(&after));
}

#[test]
#[ignore = "requires an elevated account holding SeSecurityPrivilege"]
fn restores_a_sacl_with_the_security_privilege() {
    let _enabled = privilege::enable(privilege::SECURITY)
        .unwrap_or_else(|code| panic!("SeSecurityPrivilege not available: error {code}"));
    let fx = fixture();
    let path = fx.file("audited.txt", MAIN);
    let before = fx.extended(&path);
    assert!(
        sacl_captured(&before),
        "SACL not captured with the privilege"
    );
    let audited = Builder::default()
        .sacl(vec![audit(EVERYONE, FILE_GENERIC_WRITE)])
        .build();
    fx.set_descriptor(&path, audited);
    let changed = fx.extended(&path);
    assert!(sacl_captured(&changed));
    assert!(differences(&before, &changed).contains(&"sacl"));
    let report = fx.fs.restore_meta(&fx.scope, &path, &before).unwrap();
    assert!(report.is_complete(), "{report:?}");
    let after = fx.extended(&path);
    assert!(differences(&before, &after).is_empty(), "{after:?}");
    let report = fx.fs.restore_meta(&fx.scope, &path, &changed).unwrap();
    assert!(report.is_complete(), "{report:?}");
    assert!(differences(&changed, &fx.extended(&path)).is_empty());
}
