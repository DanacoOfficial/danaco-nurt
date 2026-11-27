// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-archive/tests/sevenz_roundtrip.rs
// Responsibility:   7z archives through `create`, `list`, `plan` and `stream`: content.
// Dependencies:     nurt_contracts, sevenz_rust2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! 7z archives through `create`, `list`, `plan` and `stream`: content,
//! metadata and order survive the round trip, damaged archives fail with
//! `E_FORMAT` and encrypted ones with `E_ENCRYPTED`.

// Helper functions outside `#[test]` functions build fixtures; an
// unexpected error there is a test failure, so they unwrap.
#![allow(
    clippy::unwrap_used,
    reason = "fixture helpers fail the test on unexpected errors"
)]

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read};

use nurt_archive::create::{CreateOptions, Input, create};
use nurt_archive::extract::{ExtractPlan, Limits, plan, stream};
use nurt_archive::list::{ArchiveError, Format, Member, MemberKind, list};
use nurt_archive::safety::Rules;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Instant;
use sevenz_rust2::encoder_options::AesEncoderOptions;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};

/// Modification time of every input: 2023-11-14T22:13:20Z.
const MODIFIED_SECONDS: i64 = 1_700_000_000;
/// Permission bits of every regular file.
const FILE_MODE: u32 = 0o640;

/// Regular files of the fixture: an ASCII name, a Polish name (`\u{17c}\u{f3}\u{142}w`),
/// a Japanese directory, an empty file and 200 000 bytes of counting
/// integers.
fn files() -> BTreeMap<String, Vec<u8>> {
    let numbers: Vec<u8> = (0..50_000_u32).flat_map(u32::to_le_bytes).collect();
    [
        ("docs/readme.txt", b"read me\n".to_vec()),
        ("docs/\u{17c}\u{f3}\u{142}w.txt", b"turtle\n".to_vec()),
        ("\u{65e5}\u{672c}/\u{8a9e}.txt", b"language\n".to_vec()),
        ("data/empty.bin", Vec::new()),
        ("data/numbers.bin", numbers),
    ]
    .into_iter()
    .map(|(path, content)| (path.to_owned(), content))
    .collect()
}

fn inputs(files: &BTreeMap<String, Vec<u8>>) -> Vec<Input> {
    let modified = Instant::from_unix_seconds(MODIFIED_SECONDS).unwrap();
    let mut inputs: Vec<Input> = files
        .iter()
        .map(|(path, content)| {
            Input::file(path.clone(), content.len() as u64)
                .with_mode(FILE_MODE)
                .with_modified(modified)
        })
        .collect();
    for directory in ["data", "docs", "\u{65e5}\u{672c}"] {
        inputs.push(Input::directory(directory).with_modified(modified));
    }
    inputs.push(Input::symlink("docs/latest", "readme.txt").with_modified(modified));
    inputs
}

fn build(files: &BTreeMap<String, Vec<u8>>, level: Option<u32>) -> Result<Vec<u8>, ArchiveError> {
    let options = CreateOptions {
        format: Format::SevenZ,
        level,
    };
    create(Vec::new(), &options, inputs(files), |input| {
        let content = files.get(&input.path).cloned().unwrap_or_default();
        Ok(Box::new(Cursor::new(content)) as Box<dyn Read>)
    })
}

fn members(archive: &[u8]) -> Result<Vec<Member>, ArchiveError> {
    let mut found = Vec::new();
    let format = list(Cursor::new(archive), None, |member| {
        found.push(member);
        Ok(())
    })?;
    assert_eq!(format, Format::SevenZ);
    Ok(found)
}

fn extract(archive: &[u8], plan: &ExtractPlan) -> BTreeMap<String, Vec<u8>> {
    let mut written = BTreeMap::new();
    stream(Cursor::new(archive), plan, |entry, content| {
        let mut bytes = Vec::new();
        content.read_to_end(&mut bytes)?;
        written.insert(entry.dest.clone(), bytes);
        Ok(())
    })
    .unwrap();
    written
}

fn encrypted_archive(encrypt_header: bool) -> Vec<u8> {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![
        AesEncoderOptions::new("secret".into()).into(),
        EncoderConfiguration::new(EncoderMethod::LZMA2),
    ]);
    writer.set_encrypt_header(encrypt_header);
    writer
        .push_archive_entry(ArchiveEntry::new_directory("vault"), None::<io::Empty>)
        .unwrap();
    writer
        .push_archive_entry(
            ArchiveEntry::new_file("vault/hidden.txt"),
            Some(&b"classified"[..]),
        )
        .unwrap();
    writer.finish().unwrap().into_inner()
}

/// Checks the metadata of a listed member against the inputs of [`build`].
fn check_member(member: &Member, files: &BTreeMap<String, Vec<u8>>) {
    let path = member.path.as_str();
    assert_eq!(
        member.modified,
        Instant::from_unix_seconds(MODIFIED_SECONDS),
        "{path}"
    );
    assert!(member.name_is_exact(), "{path}");
    assert!(!member.encrypted, "{path}");
    let Some(content) = files.get(path) else {
        let expected = if path == "docs/latest" {
            MemberKind::Symlink
        } else {
            MemberKind::Directory
        };
        assert_eq!(member.kind, expected, "{path}");
        if expected == MemberKind::Symlink {
            assert_eq!(member.link_target.as_deref(), Some("readme.txt"));
        }
        return;
    };
    assert_eq!(member.kind, MemberKind::File, "{path}");
    assert_eq!(member.size, content.len() as u64, "{path}");
    assert_eq!(member.mode, Some(FILE_MODE), "{path}");
    if !content.is_empty() {
        assert_eq!(member.compression.as_deref(), Some("lzma2"), "{path}");
        assert!(member.crc32.is_some(), "{path}");
    }
}

#[test]
fn round_trip_preserves_content_metadata_and_order() {
    let files = files();
    let archive = build(&files, None).unwrap();
    let found = members(&archive).unwrap();

    let mut expected: Vec<String> = inputs(&files).into_iter().map(|input| input.path).collect();
    expected.sort();
    let paths: Vec<&str> = found.iter().map(|member| member.path.as_str()).collect();
    assert_eq!(paths, expected);

    for member in &found {
        check_member(member, &files);
    }

    let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
    assert_eq!(plan.format, Format::SevenZ);
    assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
    assert_eq!(plan.entries.len(), expected.len());
    let total: usize = files.values().map(Vec::len).sum();
    assert_eq!(plan.total_size, total as u64);
    assert_eq!(extract(&archive, &plan), files);
}

#[test]
fn every_tested_level_round_trips() {
    let files = files();
    let mut archives = Vec::new();
    for level in [0, 1, 6] {
        let archive = build(&files, Some(level)).unwrap();
        let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
        assert_eq!(extract(&archive, &plan), files, "level {level}");
        archives.push(archive);
    }
    assert_ne!(archives[0], archives[2]);
}

#[test]
fn level_outside_the_lzma2_presets_is_an_invalid_value() {
    let error = build(&files(), Some(10)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(error.to_string().contains("0..9"), "{error}");
}

#[test]
fn stream_stops_after_the_selected_members() {
    let files = files();
    let archive = build(&files, None).unwrap();
    let only = |path: &str| path.starts_with("docs/");
    let rules = Rules {
        only: Some(&only),
        ..Rules::default()
    };
    let plan = plan(Cursor::new(&archive), rules, &Limits::default()).unwrap();
    let written = extract(&archive, &plan);
    let expected: BTreeMap<String, Vec<u8>> = files
        .into_iter()
        .filter(|(path, _)| only(path.as_str()))
        .collect();
    assert_eq!(written, expected);
}

#[test]
fn damaged_archives_fail_with_format_error() {
    let files = files();
    let archive = build(&files, None).unwrap();
    let mut damaged: Vec<(String, Vec<u8>)> = [archive.len() - 1, archive.len() / 2, 12]
        .into_iter()
        .map(|keep| {
            (
                format!("truncated to {keep} bytes"),
                archive[..keep].to_vec(),
            )
        })
        .collect();
    let mut header = archive.clone();
    let last = header.len() - 1;
    header[last - 8] ^= 0x55;
    damaged.push(("damaged header".to_owned(), header));
    let mut content = archive.clone();
    content[40] ^= 0xff;
    damaged.push(("damaged content".to_owned(), content));

    for (case, bytes) in damaged {
        let error = members(&bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format, "{case}: {error}");
        let error = plan(Cursor::new(&bytes), Rules::default(), &Limits::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format, "{case}: {error}");
    }
}

#[test]
fn encrypted_content_is_listed_and_refused_by_the_plan() {
    let archive = encrypted_archive(false);
    let found = members(&archive).unwrap();
    assert_eq!(found.len(), 2);
    assert!(!found[0].encrypted);
    assert!(found[1].encrypted);
    assert_eq!(found[1].path, "vault/hidden.txt");

    let error = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encrypted);
    assert!(
        matches!(&error, ArchiveError::Encrypted { path } if path == "vault/hidden.txt"),
        "{error}"
    );
}

#[test]
fn encrypted_header_fails_with_encrypted_error() {
    let archive = encrypted_archive(true);
    let error = members(&archive).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encrypted);
    let error = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encrypted);
    assert_eq!(
        error.to_string(),
        "the archive is encrypted; decrypt it with its own tool first"
    );
}

#[test]
fn inputs_of_every_kind_are_accepted() {
    let inputs = vec![
        Input::directory("d"),
        Input::file("d/f", 3),
        Input::symlink("d/l", "f"),
    ];
    let archive = create(
        Vec::new(),
        &CreateOptions::new(Format::SevenZ),
        inputs,
        |_| Ok(Box::new(&b"abc"[..]) as Box<dyn Read>),
    )
    .unwrap();
    let kinds: Vec<MemberKind> = members(&archive)
        .unwrap()
        .into_iter()
        .map(|member| member.kind)
        .collect();
    assert_eq!(
        kinds,
        [MemberKind::Directory, MemberKind::File, MemberKind::Symlink]
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
