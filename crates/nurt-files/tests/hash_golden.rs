// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/hash_golden.rs
// Responsibility:   Examples of `docs/commands/hash.md`, run on the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of `docs/commands/hash.md`, run on the virtual file system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::sync::Arc;

use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::Entry;
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

fn hash_stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "hash")
        .expect("hash is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn language(text: &str) -> String {
    abs(text).to_language_string()
}

fn with_files(files: &[(&str, &[u8])]) -> TestContext {
    let test = TestContext::new();
    for (path, content) in files {
        test.fs().write_file(&abs(path), content).unwrap();
    }
    test
}

fn args(paths: &[&str], algorithm: Option<&str>) -> BoundArgs {
    let mut args = BoundArgs::new("hash");
    if !paths.is_empty() {
        let list = paths
            .iter()
            .map(|path| Value::Path(language(path)))
            .collect();
        args.insert("path", ArgValue::Value(Value::List(list)), false);
    }
    if let Some(algorithm) = algorithm {
        args.insert(
            "algorithm",
            ArgValue::Value(Value::Enum(algorithm.to_owned())),
            false,
        );
    }
    args
}

fn run(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Vec<Record> {
    let stage = hash_stage();
    let read = stage.as_read().expect("hash is a read stage");
    let ctx = test.ctx();
    let output = read.execute(args, input, &ctx).unwrap();
    match output.data {
        StageData::Records(stream) => stream.map(Result::unwrap).collect(),
        other => panic!("expected records, got {other:?}"),
    }
}

/// Text of a path or text field of a record.
fn text_field(record: &Record, name: &str) -> String {
    match record.get(name) {
        Some(Value::Path(text) | Value::Text(text)) => text.clone(),
        other => panic!("`{name}` is not a text: {other:?}"),
    }
}

/// `(path, hash)` of every output record; records of the source form are
/// complete `entry` records.
fn digests(records: Vec<Record>) -> Vec<(String, String)> {
    records
        .iter()
        .map(|record| {
            if record.get("name").is_some() {
                assert!(Entry::try_from(record.clone()).is_ok());
            }
            (text_field(record, "path"), text_field(record, "hash"))
        })
        .collect()
}

/// Example `hash "/srv/app/release.txt"` hashes one file with SHA-256.
#[test]
fn hashes_one_file_with_sha256() {
    let test = with_files(&[("/srv/app/release.txt", b"abc")]);
    let output = run(
        &test,
        &args(&["/srv/app/release.txt"], None),
        StageInput::none(),
    );
    assert_eq!(
        digests(output),
        [(
            language("/srv/app/release.txt"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned()
        )]
    );
}

/// Example `hash "/srv/app/a.txt" "/srv/app/b.txt" algorithm: "md5"` hashes
/// two files with MD5.
#[test]
fn hashes_several_files_with_md5() {
    let test = with_files(&[
        ("/srv/app/a.txt", b"abc"),
        ("/srv/app/b.txt", b"message digest"),
    ]);
    let output = run(
        &test,
        &args(&["/srv/app/a.txt", "/srv/app/b.txt"], Some("md5")),
        StageInput::none(),
    );
    assert_eq!(
        digests(output),
        [
            (
                language("/srv/app/a.txt"),
                "md5:900150983cd24fb0d6963f7d28e17f72".to_owned()
            ),
            (
                language("/srv/app/b.txt"),
                "md5:f96b697d7cb7938d525a2f31aaf161d0".to_owned()
            ),
        ]
    );
}

/// Example `find "/srv/app" type: file |> hash algorithm: "blake3"` hashes the
/// files of a listing.
///
/// The records of `find` are given as input; files with equal content get
/// equal digests, which `group by: hash` collects.
#[test]
fn hashes_entry_records_of_a_listing_with_blake3() {
    let test = with_files(&[
        ("/srv/app/empty.log", b""),
        ("/srv/app/zero.bin", b"\x00"),
        ("/srv/app/old/empty.log", b""),
    ]);
    let records = [
        "/srv/app/empty.log",
        "/srv/app/old/empty.log",
        "/srv/app/zero.bin",
    ]
    .iter()
    .map(|path| Record::new(RecordKind::Entry).with("path", Value::Path(language(path))))
    .collect();
    let input = StageInput::records(RecordStream::from_records(records));
    let output = run(&test, &args(&[], Some("blake3")), input);
    let empty = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let zero = "blake3:2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213";
    assert_eq!(
        digests(output),
        [
            (language("/srv/app/empty.log"), empty.to_owned()),
            (language("/srv/app/old/empty.log"), empty.to_owned()),
            (language("/srv/app/zero.bin"), zero.to_owned()),
        ]
    );
}

/// Example `hash "/srv/app/check.txt" algorithm: "crc32"` computes a CRC-32.
///
/// The digest has the hexadecimal form of `cksum -a crc32b` and `7z h`.
#[test]
fn hashes_with_crc32() {
    let test = with_files(&[("/srv/app/check.txt", b"123456789")]);
    let output = run(
        &test,
        &args(&["/srv/app/check.txt"], Some("crc32")),
        StageInput::none(),
    );
    assert_eq!(
        digests(output),
        [(language("/srv/app/check.txt"), "crc32:cbf43926".to_owned())]
    );
}

/// The hexadecimal part equals the output of `sha256sum`, `sha1sum` and
/// `sha512sum` for the same content.
#[test]
fn hexadecimal_digest_matches_the_checksum_tools() {
    let test = with_files(&[("/srv/app/a.txt", b"abc")]);
    for (algorithm, tool_output) in [
        (
            "sha256",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        ("sha1", "a9993e364706816aba3e25717850c26c9cd0d89d"),
    ] {
        let output = run(
            &test,
            &args(&["/srv/app/a.txt"], Some(algorithm)),
            StageInput::none(),
        );
        let (_, digest) = digests(output).remove(0);
        assert_eq!(digest, format!("{algorithm}:{tool_output}"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
