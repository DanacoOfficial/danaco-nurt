// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/archive_extract_safety.rs
// Responsibility:   Containment of `archive.extract` in its destination, the round trip of `archive.create` and `archive.extract`, and bulk extraction.
// Dependencies:     archive_support, nurt_archive, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Containment of `archive.extract` in its destination, the round trip of
//! `archive.create` and `archive.extract`, and bulk extraction, run on the
//! virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod archive_support;

use archive_support::{Raw, abs, args, entry, files, mutated_paths, path, raw_tar, run_effect};
use nurt_archive::create::Input;
use nurt_archive::list::Format;
use nurt_contracts::fs::{Follow, Fs};
use nurt_contracts::stage::{RecordStream, StageInput};
use nurt_contracts::value::Value;
use nurt_testkit::ctx::TestContext;

const PERMISSION_BITS: u32 = 0o7777;

fn extract_args(
    archive: &str,
    into: &str,
    given: &[(&str, Value)],
) -> nurt_contracts::stage::BoundArgs {
    let mut all = vec![("path", path(archive)), ("into", path(into))];
    all.extend(given.iter().cloned());
    args("archive.extract", &all)
}

/// Members that leave the destination by every route the checks cover,
/// followed by one member that is extracted.
fn malicious_archive() -> Vec<u8> {
    raw_tar(&[
        Raw::File("../evil.txt", b"x"),
        Raw::File("a/../../evil2.txt", b"x"),
        Raw::File("/abs/evil3.txt", b"x"),
        Raw::File("C:/evil4.txt", b"x"),
        Raw::File("dir\\..\\evil5.txt", b"x"),
        Raw::Symlink("escape", "../../outside"),
        Raw::Symlink("root", "/etc"),
        Raw::Symlink("inner", "."),
        Raw::File("inner/through.txt", b"x"),
        Raw::HardLink("passwd", "/etc/passwd"),
        Raw::File("ok.txt", b"fine"),
    ])
}

#[test]
fn escaping_members_are_rejected_and_nothing_is_written_outside() {
    let test = TestContext::new();
    test.fs()
        .write_file(&abs("/safe/in/evil.tar"), &malicious_archive())
        .expect("the archive is stored");

    let outcome = run_effect(
        &test,
        "archive.extract",
        &extract_args("/safe/in/evil.tar", "/safe/out", &[]),
        StageInput::none(),
    )
    .expect("the extraction runs");

    assert_eq!(
        test.fs()
            .read_file(&abs("/safe/out/ok.txt"))
            .expect("the safe member is extracted"),
        b"fine"
    );
    let escapes = outcome.notable("archive_path_escape");
    let rejected: Vec<&str> = escapes
        .iter()
        .filter_map(|entry| entry.path.as_deref())
        .filter_map(|path| path.split_once("!/").map(|(_, member)| member))
        .collect();
    for member in [
        "../evil.txt",
        "a/../../evil2.txt",
        "/abs/evil3.txt",
        "C:/evil4.txt",
        "dir\\..\\evil5.txt",
        "escape",
        "root",
        "inner/through.txt",
        "passwd",
    ] {
        assert!(rejected.contains(&member), "{member} is not reported");
    }
    assert_eq!(outcome.counter("rejected"), 9);
    let mutated = mutated_paths(&test);
    assert!(!mutated.is_empty());
    assert!(
        mutated
            .iter()
            .all(|changed| changed.starts_with(&abs("/safe/out"))),
        "{mutated:?}"
    );
    assert!(!test.fs().exists(&abs("/safe/evil.txt")));
    assert!(!test.fs().exists(&abs("/evil2.txt")));
}

#[test]
fn existing_links_in_the_destination_are_not_followed() {
    let test = TestContext::new();
    test.fs()
        .create_dir_all(&abs("/elsewhere"))
        .expect("the directory exists");
    test.fs()
        .create_dir_all(&abs("/safe/out"))
        .expect("the destination exists");
    test.fs()
        .symlink(abs("/elsewhere").as_path(), &abs("/safe/out/docs"))
        .expect("the link exists");
    test.fs()
        .write_file(
            &abs("/safe/in/docs.zip"),
            &files(Format::Zip, &[("docs/readme.txt", b"x"), ("top.txt", b"y")]),
        )
        .expect("the archive is stored");

    let outcome = run_effect(
        &test,
        "archive.extract",
        &extract_args("/safe/in/docs.zip", "/safe/out", &[]),
        StageInput::none(),
    )
    .expect("the extraction runs");

    assert!(!test.fs().exists(&abs("/elsewhere/readme.txt")));
    assert!(test.fs().exists(&abs("/safe/out/top.txt")));
    let escapes = outcome.notable("archive_path_escape");
    assert_eq!(escapes.len(), 1);
    assert_eq!(escapes[0].detail.as_deref(), Some("through_link"));
}

#[test]
fn allow_outside_extracts_escaping_members() {
    let test = TestContext::new();
    test.fs()
        .write_file(
            &abs("/safe/in/up.tar"),
            &raw_tar(&[Raw::File("../side/x.txt", b"side")]),
        )
        .expect("the archive is stored");

    let outcome = run_effect(
        &test,
        "archive.extract",
        &extract_args(
            "/safe/in/up.tar",
            "/safe/out",
            &[("allow_outside", Value::Bool(true))],
        ),
        StageInput::none(),
    )
    .expect("the extraction runs");

    assert_eq!(
        test.fs()
            .read_file(&abs("/safe/side/x.txt"))
            .expect("the member is extracted outside"),
        b"side"
    );
    let escapes = outcome.notable("archive_path_escape");
    assert_eq!(escapes.len(), 1);
    assert!(
        escapes[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("extracted"))
    );
    assert_eq!(outcome.counter("rejected"), 0);
}

#[test]
fn hard_links_to_extracted_files_are_created() {
    let test = TestContext::new();
    test.fs()
        .write_file(
            &abs("/safe/in/links.tar"),
            &raw_tar(&[
                Raw::File("data/a.txt", b"shared"),
                Raw::HardLink("data/b.txt", "data/a.txt"),
            ]),
        )
        .expect("the archive is stored");
    let outcome = run_effect(
        &test,
        "archive.extract",
        &extract_args("/safe/in/links.tar", "/safe/out", &[]),
        StageInput::none(),
    )
    .expect("the extraction runs");
    assert_eq!(
        test.fs()
            .read_file(&abs("/safe/out/data/b.txt"))
            .expect("the hard link is created"),
        b"shared"
    );
    assert_eq!(outcome.counter("links"), 1);
}

/// Tree of the round trip: names in Polish and Japanese, an executable
/// script, a private file and a link.
fn source_tree(test: &TestContext) {
    let fs = test.fs();
    for (file, content, mode) in [
        (
            "/src/proj/za\u{17c}\u{f3}\u{142}\u{107}.txt",
            "g\u{119}\u{15b}l".as_bytes(),
            0o644,
        ),
        (
            "/src/proj/\u{65e5}\u{672c}/\u{8a9e}.md",
            &b"# ja\n"[..],
            0o640,
        ),
        ("/src/proj/bin/run.sh", &b"#!/bin/sh\necho ok\n"[..], 0o755),
        ("/src/proj/secret.key", &b"k"[..], 0o600),
    ] {
        fs.write_file(&abs(file), content)
            .expect("the file is stored");
        fs.set_mode(&abs(file), mode).expect("the mode is set");
    }
    fs.symlink("bin/run.sh", &abs("/src/proj/run"))
        .expect("the link is stored");
}

#[test]
fn created_archives_extract_to_the_same_tree() {
    for (archive, format) in [
        ("/dst/proj.zip", None),
        ("/dst/proj.tar.zst", None),
        ("/dst/proj.data", Some("tar_bz2")),
        ("/dst/proj.7z", None),
    ] {
        let test = TestContext::new();
        source_tree(&test);
        test.fs()
            .create_dir_all(&abs("/dst"))
            .expect("the directory exists");
        let mut given = vec![("to", path(archive))];
        if let Some(format) = format {
            given.push(("format", Value::Enum(format.to_owned())));
        }
        let input = StageInput::records(RecordStream::from_records(vec![entry("/src/proj", None)]));
        run_effect(
            &test,
            "archive.create",
            &args("archive.create", &given),
            input,
        )
        .expect("the archive is created");
        run_effect(
            &test,
            "archive.extract",
            &extract_args(archive, "/restored", &[]),
            StageInput::none(),
        )
        .expect("the archive is extracted");

        for (file, mode) in [
            ("za\u{17c}\u{f3}\u{142}\u{107}.txt", 0o644),
            ("\u{65e5}\u{672c}/\u{8a9e}.md", 0o640),
            ("bin/run.sh", 0o755),
            ("secret.key", 0o600),
        ] {
            let original = abs(&format!("/src/proj/{file}"));
            let restored = abs(&format!("/restored/proj/{file}"));
            assert_eq!(
                test.fs()
                    .read_file(&restored)
                    .expect("the file is restored"),
                test.fs().read_file(&original).expect("the file exists"),
                "{archive}: {file}"
            );
            let meta = test
                .fs()
                .metadata(&restored, Follow::NoFollow)
                .expect("the file is restored");
            assert_eq!(meta.mode & PERMISSION_BITS, mode, "{archive}: {file}");
        }
        assert_eq!(
            test.fs()
                .read_link(&abs("/restored/proj/run"))
                .expect("the link is restored"),
            std::path::PathBuf::from("bin/run.sh"),
            "{archive}"
        );
    }
}

#[test]
fn compressed_tar_archives_round_trip_every_algorithm() {
    for algorithm in nurt_archive::codec::ALGORITHMS {
        let test = TestContext::new();
        let archive = archive_support::build(
            Format::Tar(Some(algorithm)),
            vec![Input::directory("d"), Input::file("d/f.txt", 3)],
            &[("d/f.txt", b"abc")],
        );
        test.fs()
            .write_file(&abs("/in/a.archive"), &archive)
            .expect("the archive is stored");
        run_effect(
            &test,
            "archive.extract",
            &extract_args("/in/a.archive", "/out", &[]),
            StageInput::none(),
        )
        .expect("the archive is extracted");
        assert_eq!(
            test.fs()
                .read_file(&abs("/out/d/f.txt"))
                .expect("the file is extracted"),
            b"abc",
            "{}",
            algorithm.name()
        );
    }
}

/// Number of members of the bulk extraction.
const BULK_MEMBERS: usize = 100_000;

#[test]
#[ignore = "extracts 100000 members; run with --ignored"]
fn bulk_archives_extract_every_member() {
    let test = TestContext::new();
    let names: Vec<String> = (0..BULK_MEMBERS)
        .map(|index| format!("d{:03}/f{index:06}.txt", index % 1000))
        .collect();
    let contents: Vec<(&str, &[u8])> = names
        .iter()
        .map(|name| (name.as_str(), &b"x"[..]))
        .collect();
    test.fs()
        .write_file(&abs("/in/bulk.tar"), &files(Format::Tar(None), &contents))
        .expect("the archive is stored");
    let outcome = run_effect(
        &test,
        "archive.extract",
        &extract_args("/in/bulk.tar", "/out", &[]),
        StageInput::none(),
    )
    .expect("the archive is extracted");
    assert_eq!(
        outcome.counter("files"),
        u64::try_from(BULK_MEMBERS).expect("the count fits")
    );
    assert!(test.fs().exists(&abs("/out/d999/f099999.txt")));
}

/// Member names that leave the destination in less common ways: a drive
/// without a separator, a UNC path, a name made only of `..`, a dot segment
/// before `..` and a name with a NUL character.
const MORE_ESCAPES: [&str; 8] = [
    "C:evil.txt",
    "\\\\server\\share\\evil.txt",
    "//server/share/evil.txt",
    "..",
    "./../evil.txt",
    "a/./../../evil.txt",
    "a/..\\..\\evil.txt",
    "nul\0../../evil.txt",
];

#[test]
fn less_common_escaping_names_never_write_outside_the_destination() {
    for member in MORE_ESCAPES {
        let test = TestContext::new();
        test.fs()
            .write_file(
                &abs("/safe/in/odd.tar"),
                &raw_tar(&[Raw::File(member, b"x"), Raw::File("ok.txt", b"fine")]),
            )
            .expect("the archive is stored");
        // A refusal of the whole archive is as safe as a rejected member.
        let _ = run_effect(
            &test,
            "archive.extract",
            &extract_args("/safe/in/odd.tar", "/safe/out", &[]),
            StageInput::none(),
        );
        let outside: Vec<_> = mutated_paths(&test)
            .into_iter()
            .filter(|changed| !changed.starts_with(&abs("/safe/out")))
            .collect();
        assert!(outside.is_empty(), "{member:?} wrote {outside:?}");
        assert!(!test.fs().exists(&abs("/safe/evil.txt")), "{member:?}");
        assert!(!test.fs().exists(&abs("/evil.txt")), "{member:?}");
        assert!(!test.fs().exists(&abs("/safe/in/evil.txt")), "{member:?}");
    }
}

#[test]
fn members_with_the_same_name_do_not_follow_a_link_created_by_an_earlier_member() {
    let test = TestContext::new();
    test.fs()
        .create_dir_all(&abs("/elsewhere"))
        .expect("the directory exists");
    test.fs()
        .write_file(
            &abs("/safe/in/chain.tar"),
            &raw_tar(&[
                Raw::Symlink("link", "/elsewhere"),
                Raw::File("link/payload.txt", b"x"),
                Raw::File("ok.txt", b"fine"),
            ]),
        )
        .expect("the archive is stored");
    let _ = run_effect(
        &test,
        "archive.extract",
        &extract_args("/safe/in/chain.tar", "/safe/out", &[]),
        StageInput::none(),
    );
    assert!(!test.fs().exists(&abs("/elsewhere/payload.txt")));
}

/// Modification time in nanoseconds, a whole number of seconds.
const MODIFIED_NS: i64 = 1_700_000_000_000_000_000;

#[test]
fn created_tar_archives_keep_the_modification_time_of_the_files() {
    for (archive, format) in [
        ("/dst/proj.tar", None),
        ("/dst/proj.tar.zst", None),
        ("/dst/proj.data", Some("tar_bz2")),
    ] {
        let test = TestContext::new();
        source_tree(&test);
        test.fs()
            .set_modified(&abs("/src/proj/secret.key"), MODIFIED_NS)
            .expect("the time is set");
        test.fs()
            .create_dir_all(&abs("/dst"))
            .expect("the directory exists");
        let mut given = vec![("to", path(archive))];
        if let Some(format) = format {
            given.push(("format", Value::Enum(format.to_owned())));
        }
        let input = StageInput::records(RecordStream::from_records(vec![entry("/src/proj", None)]));
        run_effect(
            &test,
            "archive.create",
            &args("archive.create", &given),
            input,
        )
        .expect("the archive is created");
        // The archive stores the time of the file; extraction dates the file by
        // its write, as the documentation of `archive.extract` says.
        let mut stored = Vec::new();
        std::io::Read::read_to_end(
            &mut test
                .fs()
                .open_read(&abs(archive))
                .expect("the archive exists"),
            &mut stored,
        )
        .expect("the archive is read");
        let mut times = Vec::new();
        nurt_archive::list::list(std::io::Cursor::new(stored), None, |member| {
            times.push((member.path, member.modified));
            Ok(())
        })
        .expect("the archive is listed");
        let member = times
            .iter()
            .find(|(path, _)| path.ends_with("secret.key"))
            .expect("the member is stored");
        assert_eq!(
            member.1,
            Some(nurt_contracts::value::Instant::from_unix_nanos(MODIFIED_NS)),
            "{archive}"
        );
        run_effect(
            &test,
            "archive.extract",
            &extract_args(archive, "/restored", &[]),
            StageInput::none(),
        )
        .expect("the archive is extracted");
        let meta = test
            .fs()
            .metadata(&abs("/restored/proj/secret.key"), Follow::NoFollow)
            .expect("the file is restored");
        assert_ne!(meta.modified_ns, MODIFIED_NS, "{archive}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
