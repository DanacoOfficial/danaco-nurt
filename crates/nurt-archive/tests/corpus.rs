// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Archives generated in the test: every supported format and compression,
//! Unicode names, links and members crafted to escape the destination.
//!
//! [`materialise`] writes an extraction plan into a temporary directory the
//! way the extraction command does (directories, file content, hard links,
//! symbolic links last), so the tests can check what reaches the disk.

// Helper functions outside `#[test]` functions build fixtures; an
// unexpected error there is a test failure, so they unwrap.
#![allow(
    clippy::unwrap_used,
    reason = "fixture helpers fail the test on unexpected errors"
)]

use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

use nurt_archive::codec::{self, Algorithm};
use nurt_archive::create::{CreateOptions, Input, InputKind, create};
use nurt_archive::extract::{ExtractPlan, Limits, plan, stream};
use nurt_archive::list::{
    ArchiveError, Format, LimitKind, Member, MemberKind, detect_format, list,
};
use nurt_archive::safety::{Rules, Violation};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Instant;

const FORMATS: [Format; 7] = [
    Format::Zip,
    Format::Tar(None),
    Format::Tar(Some(Algorithm::Gzip)),
    Format::Tar(Some(Algorithm::Zstd)),
    Format::Tar(Some(Algorithm::Xz)),
    Format::Tar(Some(Algorithm::Bzip2)),
    Format::Tar(Some(Algorithm::Lz4)),
];

/// 2023-11-14T22:13:20Z, inside the MS-DOS range of zip.
const MODIFIED: i64 = 1_700_000_000;

/// Name with Polish letters, used by several tests.
const POLISH: &str = "za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105}.txt";

fn at(offset: i64) -> Instant {
    Instant::from_unix_seconds(MODIFIED + offset).unwrap()
}

/// Inputs with their content: nested directories, names in several
/// scripts, an empty file, a file larger than one compression block and a
/// relative symbolic link.
fn sample() -> Vec<(Input, Vec<u8>)> {
    let large: Vec<u8> = (0..300_000_u32)
        .flat_map(|n| (n % 251).to_le_bytes())
        .collect();
    let file = |path: &str, mode: u32, offset: i64, content: Vec<u8>| {
        let input = Input::file(path, content.len() as u64)
            .with_mode(mode)
            .with_modified(at(offset));
        (input, content)
    };
    vec![
        (
            Input::directory("docs")
                .with_mode(0o755)
                .with_modified(at(0)),
            Vec::new(),
        ),
        file("docs/readme.txt", 0o644, 1, b"read me\n".to_vec()),
        file(
            &format!("docs/{POLISH}"),
            0o600,
            2,
            "ja\u{17a}\u{144}\n".as_bytes().to_vec(),
        ),
        file(
            "\u{395}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac}.txt",
            0o644,
            3,
            "\u{3b1}\u{3b2}".as_bytes().to_vec(),
        ),
        file(
            "\u{65e5}\u{672c}/\u{30d5}\u{30a1}\u{30a4}\u{30eb}.bin",
            0o755,
            4,
            large,
        ),
        file("empty", 0o644, 5, Vec::new()),
        (
            Input::symlink("docs/latest", "readme.txt").with_modified(at(6)),
            Vec::new(),
        ),
    ]
}

fn build(format: Format, inputs: &[(Input, Vec<u8>)]) -> Vec<u8> {
    let list: Vec<Input> = inputs.iter().map(|(input, _)| input.clone()).collect();
    create(Vec::new(), &CreateOptions::new(format), list, |input| {
        let content = inputs
            .iter()
            .find(|(candidate, _)| candidate.path == input.path)
            .map(|(_, content)| content.clone())
            .unwrap_or_default();
        Ok(Box::new(Cursor::new(content)))
    })
    .unwrap()
}

fn members(archive: &[u8]) -> Vec<Member> {
    let mut members = Vec::new();
    list(Cursor::new(archive), None, |member| {
        members.push(member);
        Ok(())
    })
    .unwrap();
    members
}

/// Writes the plan into `into` and returns it.
fn materialise(archive: &[u8], into: &Path, rules: Rules<'_>) -> ExtractPlan {
    let plan = plan(Cursor::new(archive), rules, &Limits::default()).unwrap();
    fs::create_dir_all(into).unwrap();
    for entry in &plan.entries {
        if entry.member.kind == MemberKind::Directory {
            fs::create_dir_all(into.join(&entry.dest)).unwrap();
        }
    }
    stream(Cursor::new(archive), &plan, |entry, content| {
        let path = into.join(&entry.dest);
        fs::create_dir_all(path.parent().unwrap())?;
        io::copy(content, &mut fs::File::create(path)?)?;
        Ok(())
    })
    .unwrap();
    for entry in &plan.entries {
        if let Some(target) = &entry.hard_link_to {
            fs::hard_link(into.join(target), into.join(&entry.dest)).unwrap();
        }
    }
    for entry in &plan.entries {
        if entry.member.kind == MemberKind::Symlink {
            symlink(
                entry.member.link_target.as_deref().unwrap(),
                &into.join(&entry.dest),
            );
        }
    }
    plan
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path) {
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(not(unix))]
fn symlink(_target: &str, _link: &Path) {}

/// Every path under `root`, depth first.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                pending.push(path.clone());
            }
            found.push(path);
        }
    }
    found
}

/// CRC-32 (ISO 3309) computed bit by bit, independent of the archive code.
fn crc32(data: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0xedb8_8320;
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ POLYNOMIAL
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn check_member(format: Format, member: &Member, input: &Input, content: &[u8]) {
    let context = format!("{format} {}", member.path);
    assert_eq!(member.path, input.path, "{context}");
    assert!(member.name_is_exact(), "{context}");
    assert!(!member.encrypted, "{context}");
    assert_eq!(member.modified, input.modified, "{context}");
    match &input.kind {
        InputKind::File { size } => {
            assert_eq!(member.kind, MemberKind::File, "{context}");
            assert_eq!(member.size, *size, "{context}");
            assert_eq!(member.mode, input.mode, "{context}");
        }
        InputKind::Directory => assert_eq!(member.kind, MemberKind::Directory, "{context}"),
        InputKind::Symlink(target) => {
            assert_eq!(member.kind, MemberKind::Symlink, "{context}");
            assert_eq!(
                member.link_target.as_deref(),
                Some(target.as_str()),
                "{context}"
            );
        }
    }
    check_format_fields(format, member, content, &context);
}

fn check_format_fields(format: Format, member: &Member, content: &[u8], context: &str) {
    match format {
        Format::Zip if member.kind == MemberKind::File => {
            assert_eq!(member.crc32, Some(crc32(content)), "{context}");
            assert!(member.packed.is_some(), "{context}");
        }
        Format::Tar(Some(algorithm)) => {
            assert_eq!(
                member.compression.as_deref(),
                Some(algorithm.name()),
                "{context}"
            );
        }
        _ => {}
    }
}

#[test]
fn created_archives_list_with_every_field() {
    let mut inputs = sample();
    inputs.sort_by(|left, right| left.0.path.cmp(&right.0.path));
    for format in FORMATS {
        let archive = build(format, &inputs);
        assert_eq!(detect_format(&mut Cursor::new(&archive)).unwrap(), format);
        let members = members(&archive);
        assert_eq!(members.len(), inputs.len(), "{format}");
        for (member, (input, content)) in members.iter().zip(&inputs) {
            check_member(format, member, input, content);
        }
        assert_eq!(members[1].name(), "latest");
    }
}

#[test]
fn round_trip_reproduces_content_and_links() {
    let inputs = sample();
    for format in FORMATS {
        let archive = build(format, &inputs);
        let temp = tempfile::tempdir().unwrap();
        let into = temp.path().join("into");
        let plan = materialise(&archive, &into, Rules::default());
        assert!(plan.rejected.is_empty(), "{format}: {:?}", plan.rejected);
        assert_eq!(plan.entries.len(), inputs.len());
        for (input, content) in &inputs {
            let path = into.join(&input.path);
            match &input.kind {
                InputKind::File { .. } => {
                    assert_eq!(
                        &fs::read(&path).unwrap(),
                        content,
                        "{format} {}",
                        input.path
                    );
                }
                InputKind::Directory => assert!(path.is_dir()),
                InputKind::Symlink(target) if cfg!(unix) => {
                    assert_eq!(fs::read_link(&path).unwrap(), PathBuf::from(target));
                    assert_eq!(fs::read(&path).unwrap(), b"read me\n");
                }
                InputKind::Symlink(_) => {}
            }
        }
    }
}

#[test]
fn glob_filter_selects_listed_members() {
    let archive = build(Format::Tar(Some(Algorithm::Gzip)), &sample());
    let only_text = |path: &str| path.ends_with(".txt");
    let mut paths = Vec::new();
    list(Cursor::new(&archive), Some(&only_text), |member| {
        paths.push(member.path);
        Ok(())
    })
    .unwrap();
    assert_eq!(paths.len(), 3);
    assert!(paths.iter().all(|path| path.ends_with(".txt")));
}

#[test]
fn strip_components_and_only_shape_the_plan() {
    let archive = build(Format::Zip, &sample());
    let only = |path: &str| path.starts_with("docs/");
    let rules = Rules {
        only: Some(&only),
        strip_components: 1,
        allow_outside: false,
    };
    let plan = plan(Cursor::new(&archive), rules, &Limits::default()).unwrap();
    let mut dests: Vec<&str> = plan
        .entries
        .iter()
        .map(|entry| entry.dest.as_str())
        .collect();
    dests.sort_unstable();
    assert_eq!(dests, ["latest", "readme.txt", POLISH]);
    assert_eq!(plan.skipped, 4);
}

/// Tar header with the name written verbatim, bypassing the checks of the
/// `tar` crate, which refuses `..` in new archives.
fn raw_header(kind: tar::EntryType, name: &str, link: Option<&str>, size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(0o644);
    header.set_size(size);
    header.set_mtime(0);
    let bytes = header.as_mut_bytes();
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    if let Some(link) = link {
        header.set_link_name_literal(link).unwrap();
    }
    header.set_cksum();
    header
}

/// Tar archive with every kind of escaping member and one safe file; the
/// absolute members point at `outside`, a sibling of the destination.
fn malicious_tar(outside: &str, compression: Option<Algorithm>) -> Vec<u8> {
    let payload = b"payload";
    let size = payload.len() as u64;
    let file = |name: &str| {
        (
            raw_header(tar::EntryType::Regular, name, None, size),
            &payload[..],
        )
    };
    let link = |kind, name: &str, target: &str| (raw_header(kind, name, Some(target), 0), &b""[..]);
    let members = [
        file("../escape.txt"),
        file("a/../../escape2.txt"),
        file("C:/drive.txt"),
        file("..\\backslash.txt"),
        link(tar::EntryType::Symlink, "up", "../outside"),
        file("up/through.txt"),
        link(tar::EntryType::Link, "hard-etc", "/etc/passwd"),
        link(tar::EntryType::Link, "hard-up", "../outside/secret"),
        link(tar::EntryType::Link, "hard-missing", "not-in-archive"),
        file("safe.txt"),
        link(tar::EntryType::Link, "safe-hard", "safe.txt"),
        link(tar::EntryType::Symlink, "safe-link", "safe.txt"),
    ];
    let mut builder = tar::Builder::new(Vec::new());
    builder.preserve_absolute(true);
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    builder
        .append_data(&mut header, format!("{outside}/absolute.txt"), &payload[..])
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    builder
        .append_link(&mut header, "abs-link", outside)
        .unwrap();
    for (header, data) in members {
        builder.append(&header, data).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    match compression {
        None => tar,
        Some(algorithm) => {
            let mut encoder = codec::encoder(algorithm, None, Vec::new()).unwrap();
            encoder.write_all(&tar).unwrap();
            encoder.finish().unwrap()
        }
    }
}

fn malicious_zip(outside: &str) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    let absolute = format!("{outside}/absolute.txt");
    for name in [
        "../escape.txt",
        absolute.as_str(),
        "C:/drive.txt",
        "..\\backslash.txt",
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(b"payload").unwrap();
    }
    writer.add_symlink("up", "../outside", options).unwrap();
    writer.start_file("up/through.txt", options).unwrap();
    writer.write_all(b"payload").unwrap();
    writer.add_symlink("abs-link", outside, options).unwrap();
    writer.start_file("safe.txt", options).unwrap();
    writer.write_all(b"payload").unwrap();
    writer
        .add_symlink("safe-link", "safe.txt", options)
        .unwrap();
    writer.finish().unwrap().into_inner()
}

/// Every path under `root` other than the prepared `outside` directory lies
/// inside `into`, and every symbolic link there resolves inside `into`.
fn assert_contained(root: &Path, into: &Path) {
    let outside = root.join("outside");
    let canonical_into = fs::canonicalize(into).unwrap();
    for path in walk(root) {
        if path == into || path.starts_with(&outside) {
            continue;
        }
        assert!(path.starts_with(into), "{} written outside", path.display());
        if fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
            && let Ok(resolved) = fs::canonicalize(&path)
        {
            assert!(
                resolved.starts_with(&canonical_into),
                "{} resolves outside",
                path.display()
            );
        }
    }
}

/// `path` as the absolute member name of an attack: on Windows without the
/// drive and with `/` separators, the name that a careless extraction writes
/// to the same place of the current drive.
fn rooted(path: &Path) -> String {
    let text = path.to_str().unwrap().replace('\\', "/");
    match text.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => text[2..].to_owned(),
        _ => text,
    }
}

#[test]
fn malicious_tar_writes_nothing_outside_destination() {
    for compression in [None, Some(Algorithm::Gzip), Some(Algorithm::Zstd)] {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret"), b"secret").unwrap();
        let archive = malicious_tar(&rooted(&outside), compression);
        let into = temp.path().join("into");
        let plan = materialise(&archive, &into, Rules::default());

        let violations: Vec<(&str, Violation)> = plan
            .rejected
            .iter()
            .map(|rejected| (rejected.path.as_str(), rejected.violation))
            .collect();
        let absolute = format!("{}/absolute.txt", rooted(&outside));
        assert_eq!(
            violations,
            [
                (absolute.as_str(), Violation::Absolute),
                ("abs-link", Violation::LinkOutside),
                ("../escape.txt", Violation::ParentComponent),
                ("a/../../escape2.txt", Violation::ParentComponent),
                ("C:/drive.txt", Violation::DriveLetter),
                ("..\\backslash.txt", Violation::Backslash),
                ("up", Violation::LinkOutside),
                ("hard-etc", Violation::HardLinkOutside),
                ("hard-up", Violation::HardLinkOutside),
                ("hard-missing", Violation::HardLinkOutside),
            ]
        );
        let mut dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|entry| entry.dest.as_str())
            .collect();
        dests.sort_unstable();
        assert_eq!(
            dests,
            ["safe-hard", "safe-link", "safe.txt", "up/through.txt"]
        );

        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        assert_eq!(fs::read(outside.join("secret")).unwrap(), b"secret");
        assert!(!temp.path().join("escape.txt").exists());
        assert_contained(temp.path(), &into);
    }
}

#[test]
fn malicious_zip_writes_nothing_outside_destination() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let archive = malicious_zip(&rooted(&outside));
    let into = temp.path().join("into");
    let plan = materialise(&archive, &into, Rules::default());
    let violations: Vec<Violation> = plan
        .rejected
        .iter()
        .map(|rejected| rejected.violation)
        .collect();
    assert_eq!(
        violations,
        [
            Violation::ParentComponent,
            Violation::Absolute,
            Violation::DriveLetter,
            Violation::Backslash,
            Violation::LinkOutside,
            Violation::LinkOutside,
        ]
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert_contained(temp.path(), &into);
}

#[test]
fn member_through_accepted_archive_link_is_rejected() {
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append(
            &raw_header(tar::EntryType::Symlink, "dir", Some("."), 0),
            &b""[..],
        )
        .unwrap();
    builder
        .append(
            &raw_header(tar::EntryType::Regular, "dir/file", None, 1),
            &b"x"[..],
        )
        .unwrap();
    let archive = builder.into_inner().unwrap();
    let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
    assert_eq!(plan.rejected[0].violation, Violation::ThroughLink);
    assert_eq!(plan.entries.len(), 1);
}

#[test]
fn allow_outside_plans_escaping_members_as_stored() {
    let archive = malicious_tar("/srv/outside", None);
    let rules = Rules {
        allow_outside: true,
        ..Rules::default()
    };
    let plan = plan(Cursor::new(&archive), rules, &Limits::default()).unwrap();
    let escape = plan
        .entries
        .iter()
        .find(|entry| entry.member.path == "../escape.txt")
        .unwrap();
    assert_eq!(escape.dest, "../escape.txt");
    assert_eq!(escape.waived, Some(Violation::ParentComponent));
    assert!(
        plan.entries
            .iter()
            .any(|entry| entry.dest == "/srv/outside/absolute.txt")
    );
    assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
}

#[test]
fn later_member_replaces_earlier_one_with_same_destination() {
    let mut builder = tar::Builder::new(Vec::new());
    for content in [&b"first"[..], &b"second!"[..]] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        builder
            .append_data(&mut header, "same.txt", content)
            .unwrap();
    }
    let archive = builder.into_inner().unwrap();
    let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].ordinal, 1);
    assert_eq!(plan.skipped, 1);
    assert_eq!(plan.total_size, 7);
    let mut content = Vec::new();
    stream(Cursor::new(&archive), &plan, |_, reader| {
        reader.read_to_end(&mut content)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(content, b"second!");
}

#[test]
fn special_files_are_listed_but_not_extracted() {
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append(&raw_header(tar::EntryType::Fifo, "pipe", None, 0), &b""[..])
        .unwrap();
    let archive = builder.into_inner().unwrap();
    assert_eq!(members(&archive)[0].kind, MemberKind::Other);
    let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
    assert!(plan.entries.is_empty());
    assert_eq!(plan.special, ["pipe"]);
}

fn zeros_archive(size: usize) -> Vec<u8> {
    let inputs = vec![(Input::file("zeros", size as u64), vec![0_u8; size])];
    build(Format::Tar(Some(Algorithm::Gzip)), &inputs)
}

#[test]
fn size_limit_stops_planning() {
    let archive = zeros_archive(4096);
    let limits = Limits {
        max_size: 1024,
        ..Limits::default()
    };
    let error = plan(Cursor::new(&archive), Rules::default(), &limits).unwrap_err();
    assert!(matches!(
        error,
        ArchiveError::Limit {
            limit: LimitKind::Size,
            allowed: 1024,
            actual: 4096
        }
    ));
    assert_eq!(error.code(), ErrorCode::Limit);
}

#[test]
fn ratio_limit_stops_decompression_bomb() {
    let archive = zeros_archive(1 << 20);
    let limits = Limits {
        max_ratio: 10,
        ratio_floor: 0,
        ..Limits::default()
    };
    let error = plan(Cursor::new(&archive), Rules::default(), &limits).unwrap_err();
    assert!(
        matches!(
            error,
            ArchiveError::Limit {
                limit: LimitKind::Ratio,
                ..
            }
        ),
        "{error}"
    );
    assert!(plan(Cursor::new(&archive), Rules::default(), &Limits::default()).is_ok());
}

#[test]
fn encrypted_zip_members_are_listed_and_refused() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .with_aes_encryption(zip::AesMode::Aes256, "password");
    writer.start_file("secret.txt", options).unwrap();
    writer.write_all(b"secret").unwrap();
    let archive = writer.finish().unwrap().into_inner();
    assert!(members(&archive)[0].encrypted);
    let error = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap_err();
    assert!(matches!(&error, ArchiveError::Encrypted { path } if path == "secret.txt"));
    assert_eq!(error.code(), ErrorCode::Encrypted);
}

#[test]
fn stream_detects_archive_changed_after_planning() {
    let first = build(Format::Tar(None), &[(Input::file("a", 1), b"1".to_vec())]);
    let second = build(Format::Tar(None), &[(Input::file("b", 1), b"2".to_vec())]);
    let plan = plan(Cursor::new(&first), Rules::default(), &Limits::default()).unwrap();
    let error = stream(Cursor::new(&second), &plan, |_, _| Ok(())).unwrap_err();
    assert!(matches!(&error, ArchiveError::Changed { path, .. } if path == "a"));
    let empty = build(Format::Tar(None), &[]);
    let error = stream(Cursor::new(&empty), &plan, |_, _| Ok(())).unwrap_err();
    assert!(matches!(error, ArchiveError::Changed { .. }));
}

#[test]
fn damaged_archives_report_format_errors() {
    let archive = build(Format::Tar(Some(Algorithm::Gzip)), &sample());
    let error = list(Cursor::new(&archive[..archive.len() / 2]), None, |_| Ok(())).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Format, "{error}");

    let zip = build(Format::Zip, &sample());
    let error = list(Cursor::new(&zip[..zip.len() - 10]), None, |_| Ok(())).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Format, "{error}");
}

#[test]
fn sink_errors_end_the_stream() {
    let archive = build(Format::Zip, &sample());
    let plan = plan(Cursor::new(&archive), Rules::default(), &Limits::default()).unwrap();
    let mut calls = 0;
    let error = stream(Cursor::new(&archive), &plan, |_, _| {
        calls += 1;
        Err(ArchiveError::Io(io::Error::other("disk full")))
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.code(), ErrorCode::Io);
}
