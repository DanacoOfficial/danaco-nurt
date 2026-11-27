// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/fs_paths.rs
// Responsibility:   Paths, content hashes, entity tags and file system errors of the file.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Paths, content hashes, entity tags and file system errors of the file
//! system contract.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{
    AbsPath, ContentHash, Durability, Etag, FileId, FileKind, Follow, FsError, FsErrorKind, Holder,
    LinkKind, MetaTransferReport, ProviderValue, RenameMode, ScopePurpose, VolumeKind,
};
use serde_json::json;

/// Absolute path in language form on the host system: the path itself on
/// Unix, prefixed with a drive on Windows.
fn at(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.to_owned()
    }
}

fn abs(path: &str) -> AbsPath {
    AbsPath::new(at(path)).unwrap()
}

#[test]
fn normalisation_removes_separators_and_current_directory_components() {
    let cases = [
        ("/home/user/", "/home/user"),
        ("/home//user", "/home/user"),
        ("/home/user/./.", "/home/user"),
        ("/home/./user/./a.txt", "/home/user/a.txt"),
        ("/a/b/..", "/a"),
        ("/a/../b", "/b"),
        ("/a/b/../../c/d", "/c/d"),
        ("/a/..", "/"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            abs(input).to_language_string(),
            at(expected),
            "normal form of `{input}`"
        );
    }
}

#[test]
fn parent_directory_above_the_root_is_invalid() {
    for input in ["/..", "/a/../..", "/a/b/../../.."] {
        let error = AbsPath::new(at(input)).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{input}");
        assert_eq!(error.code_id(), "E_IO");
    }
}

#[test]
fn relative_and_empty_paths_are_invalid() {
    for input in ["", ".", "..", "a", "a/b", "./a", "../a"] {
        let error = AbsPath::new(input).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{input:?}");
        assert_eq!(error.op, "abs_path");
    }
}

#[test]
fn a_nul_character_is_invalid_in_a_path_and_in_a_name() {
    for input in ["/a\0b", "/\0", "/a/b\0"] {
        let error = AbsPath::new(at(input)).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{input:?}");
        assert_eq!(error.op, "abs_path");
    }
    let dir = abs("/home/user");
    for name in ["\0", "a\0", "\0a", "a\0b"] {
        let error = dir.join_name(name).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{name:?}");
        assert_eq!(error.op, "join_name");
    }
}

#[cfg(windows)]
#[test]
fn the_long_path_prefix_is_never_part_of_an_abs_path() {
    for input in [r"\\?\C:\a", r"\\?\UNC\server\share\a", r"\\?\Volume{1}\a"] {
        let error = AbsPath::new(input).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{input:?}");
    }
}

#[test]
fn root_has_no_parent_and_no_name() {
    let root = abs("/");
    assert_eq!(root.parent(), None);
    assert_eq!(root.file_name(), None);
    assert!(abs("/a").starts_with(&root));
    assert_eq!(abs("/a").parent(), Some(root));
}

#[test]
fn starts_with_compares_whole_components() {
    let dir = abs("/home/user");
    assert!(dir.starts_with(&dir));
    assert!(abs("/home/user/a.txt").starts_with(&dir));
    assert!(!abs("/home/user2").starts_with(&dir));
    assert!(!abs("/home").starts_with(&dir));
    assert!(!abs("/home/usr/a.txt").starts_with(&dir));
}

#[test]
fn join_name_keeps_the_parent_for_every_valid_name() {
    let dir = abs("/home/user");
    for name in [
        "a.txt",
        ".hidden",
        "..a",
        "a..",
        "a b",
        "trailing ",
        "-rf",
        "\u{65e5}\u{672c}\u{8a9e}.txt",
        "e\u{301}.txt",
    ] {
        let joined = dir.join_name(name).unwrap();
        assert_eq!(joined.parent().as_ref(), Some(&dir), "{name:?}");
        assert_eq!(joined.file_name().unwrap(), name);
    }
}

#[test]
fn join_name_rejects_names_that_are_not_one_component() {
    let dir = abs("/home/user");
    for name in ["", ".", "..", "a/b", "/", "/a", "a/", "../a", "a/.."] {
        let error = dir.join_name(name).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{name:?}");
        assert_eq!(error.op, "join_name");
        assert_eq!(error.path.as_deref(), Some(dir.as_path()));
    }
}

#[cfg(windows)]
#[test]
fn join_name_rejects_names_that_start_a_drive_prefix_or_a_stream() {
    let dir = abs("/home/user");
    for name in ["D:evil", "D:", "c:x", "a:b", "C:\\x", "a\\b"] {
        let error = dir.join_name(name).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{name:?}");
    }
}

#[cfg(not(windows))]
#[test]
fn join_name_accepts_a_colon_and_a_backslash_outside_windows() {
    let dir = abs("/home/user");
    for name in ["D:evil", "a:b", "a\\b", "c:"] {
        let joined = dir.join_name(name).unwrap();
        assert_eq!(joined.parent().as_ref(), Some(&dir), "{name:?}");
        assert_eq!(joined.file_name().unwrap(), name);
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_kept_natively_and_serialized_lossily() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    let raw = b"/home/user/caf\xe9".to_vec();
    let path = AbsPath::new(PathBuf::from(OsStr::from_bytes(&raw))).unwrap();
    assert_eq!(path.as_path().as_os_str().as_bytes(), raw.as_slice());
    assert_eq!(path.to_language_string(), "/home/user/caf\u{fffd}");
    assert_eq!(path.parent().unwrap(), abs("/home/user"));

    let other_raw = b"/home/user/caf\xe8".to_vec();
    let other = AbsPath::new(PathBuf::from(OsStr::from_bytes(&other_raw))).unwrap();
    assert_ne!(path, other);
    assert_eq!(path.to_language_string(), other.to_language_string());
}

#[test]
fn serialization_uses_the_language_form_and_rejects_invalid_text() {
    let path = abs("/home/user/./a.txt");
    assert_eq!(
        serde_json::to_value(&path).unwrap(),
        json!(at("/home/user/a.txt"))
    );
    assert_eq!(path.to_string(), at("/home/user/a.txt"));
    assert_eq!(
        format!("{path:?}"),
        format!("AbsPath({})", at("/home/user/a.txt"))
    );
    for invalid in [json!("relative/a"), json!(""), json!(at("/..")), json!(7)] {
        assert!(
            serde_json::from_value::<AbsPath>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn content_hash_rejects_malformed_text_without_panicking() {
    let zero = format!("sha256:{}", "0".repeat(64));
    assert_eq!(ContentHash::parse(&zero).unwrap(), ContentHash([0; 32]));
    assert_eq!(
        ContentHash([0xab; 32]).to_string(),
        format!("sha256:{}", "ab".repeat(32))
    );
    assert_eq!(
        String::from(ContentHash([1; 32])),
        format!("sha256:{}", "01".repeat(32))
    );
    assert_eq!(
        ContentHash::try_from(zero.clone()).unwrap(),
        ContentHash::from_str(&zero).unwrap()
    );
    assert_eq!(
        format!("{:?}", ContentHash([0; 32])),
        format!("ContentHash({zero})")
    );

    for text in [
        "sha256:".to_owned(),
        format!("sha256:{}", "0".repeat(63)),
        format!("sha256:{}", "0".repeat(65)),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "g".repeat(64)),
        format!("sha256:{}", "\u{e9}".repeat(32)),
        format!("sha256:{}", "\u{1f600}".repeat(16)),
        format!("SHA256:{}", "0".repeat(64)),
        format!(" sha256:{}", "0".repeat(64)),
        format!("sha256:{} ", "0".repeat(64)),
    ] {
        assert!(ContentHash::parse(&text).is_err(), "{text:?}");
    }
    let message = ContentHash::parse("x").unwrap_err().to_string();
    assert!(message.contains("sha256:"), "{message}");
}

fn tag() -> Etag {
    Etag {
        size: 12,
        modified_ns: -5,
        file_id: FileId {
            volume: 1,
            low: 42,
            high: 7,
        },
        hash: None,
    }
}

#[test]
fn etags_differ_in_every_identifying_member() {
    let base = tag();
    assert!(base.matches(&base));
    let variants = [
        Etag { size: 13, ..tag() },
        Etag {
            modified_ns: -4,
            ..tag()
        },
        Etag {
            file_id: FileId {
                volume: 2,
                ..tag().file_id
            },
            ..tag()
        },
        Etag {
            file_id: FileId {
                low: 43,
                ..tag().file_id
            },
            ..tag()
        },
        Etag {
            file_id: FileId {
                high: 8,
                ..tag().file_id
            },
            ..tag()
        },
    ];
    for other in variants {
        assert!(!base.matches(&other), "{other:?}");
        assert!(!other.matches(&base), "{other:?}");
    }
}

#[test]
fn etag_with_a_hash_serializes_the_hash_and_ignores_it_against_a_bare_tag() {
    let hashed = Etag {
        hash: Some(ContentHash([3; 32])),
        ..tag()
    };
    let value = serde_json::to_value(&hashed).unwrap();
    assert_eq!(value["hash"], json!(format!("sha256:{}", "03".repeat(32))));
    assert_eq!(serde_json::from_value::<Etag>(value).unwrap(), hashed);
    assert!(hashed.matches(&tag()));
    assert!(tag().matches(&hashed));
    assert!(hashed.matches(&hashed));
}

#[test]
fn every_error_kind_has_a_stable_name_and_language_code() {
    let table = [
        (FsErrorKind::NotFound, "not_found", "E_NOT_FOUND"),
        (
            FsErrorKind::PermissionDenied,
            "permission_denied",
            "E_PERMISSION",
        ),
        (FsErrorKind::ReadOnly, "read_only", "E_PERMISSION"),
        (FsErrorKind::Privilege, "privilege", "E_PRIVILEGE"),
        (FsErrorKind::Exists, "exists", "E_EXISTS"),
        (FsErrorKind::InUse, "in_use", "E_IN_USE"),
        (FsErrorKind::NoSpace, "no_space", "E_NO_SPACE"),
        (FsErrorKind::QuotaExceeded, "quota_exceeded", "E_NO_SPACE"),
        (FsErrorKind::Stale, "stale", "E_STALE"),
        (FsErrorKind::CrossDevice, "cross_device", "E_IO"),
        (FsErrorKind::Unsupported, "unsupported", "E_IO"),
        (FsErrorKind::InvalidPath, "invalid_path", "E_IO"),
        (FsErrorKind::NotADirectory, "not_a_directory", "E_IO"),
        (FsErrorKind::IsADirectory, "is_a_directory", "E_IO"),
        (FsErrorKind::NotEmpty, "not_empty", "E_IO"),
        (FsErrorKind::NameTooLong, "name_too_long", "E_IO"),
        (FsErrorKind::Loop, "loop", "E_IO"),
        (FsErrorKind::Interrupted, "interrupted", "E_IO"),
        (FsErrorKind::Io, "io", "E_IO"),
    ];
    let names: BTreeSet<&str> = table.iter().map(|(_, name, _)| *name).collect();
    assert_eq!(names.len(), table.len(), "duplicate kind names");
    for (kind, name, code) in table {
        assert_eq!(kind.as_str(), name);
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<FsErrorKind>(json!(name)).unwrap(),
            kind
        );
        assert_eq!(kind.code_id(), code, "{name}");
        let language = ErrorCode::from_str(code).unwrap();

        let error = Error::from(FsError::new(kind, "open", None));
        assert_eq!(error.code(), language, "{name}");
        assert_eq!(error.details["kind"], json!(name));
        assert_eq!(error.details["op"], json!("open"));
    }
}

#[test]
fn file_system_errors_convert_with_every_detail() {
    let mut cause = FsError::new(
        FsErrorKind::InUse,
        "remove",
        Some(Path::new("/home/user/a.txt")),
    );
    cause.os_code = Some(32);
    cause.holders = vec![Holder {
        pid: 4132,
        name: "editor".to_owned(),
    }];
    let error = Error::from(cause);
    assert_eq!(error.code(), ErrorCode::InUse);
    assert_eq!(error.message, "remove failed (in_use): /home/user/a.txt");
    assert_eq!(error.details["path"], json!("/home/user/a.txt"));
    assert_eq!(error.details["os_error"], json!(32));
    assert_eq!(
        error.details["holders"],
        json!([{"pid": 4132, "name": "editor"}])
    );
}

#[test]
fn file_system_errors_without_a_path_or_code_have_no_such_details() {
    let error = Error::from(FsError::new(FsErrorKind::Io, "sync", None));
    assert_eq!(error.message, "sync failed (io)");
    assert!(error.details.get("path").is_none());
    assert!(error.details.get("os_error").is_none());
    assert!(error.details.get("holders").is_none());

    let cause = FsError::new(FsErrorKind::Io, "sync", None);
    assert_eq!(cause.to_string(), "E_IO (io) in sync");
    let cause = FsError::new(FsErrorKind::NotFound, "open", Some(Path::new("/x")));
    assert_eq!(cause.to_string(), "E_NOT_FOUND (not_found) in open: /x");
}

#[test]
fn small_enumerations_serialize_in_snake_case() {
    assert_eq!(
        serde_json::to_value(RenameMode::NoReplace).unwrap(),
        json!("no_replace")
    );
    assert_eq!(
        serde_json::to_value(Durability::Full).unwrap(),
        json!("full")
    );
    assert_eq!(
        serde_json::to_value(LinkKind::SymlinkDirectory).unwrap(),
        json!("symlink_directory")
    );
    assert_eq!(
        serde_json::to_value(ScopePurpose::Maintenance).unwrap(),
        json!("maintenance")
    );
    assert_eq!(
        serde_json::to_value(VolumeKind::Overlay).unwrap(),
        json!("overlay")
    );
    assert_eq!(
        serde_json::to_value(FileKind::Junction).unwrap(),
        json!("junction")
    );
    assert_eq!(
        serde_json::to_value(Follow::NoFollow).unwrap(),
        json!("no_follow")
    );
}

#[test]
fn provider_values_are_tagged_with_type_and_data() {
    let cases = [
        (
            ProviderValue::Text("x".to_owned()),
            json!({"type": "text", "data": "x"}),
        ),
        (
            ProviderValue::ExpandableText("%PATH%".to_owned()),
            json!({"type": "expandable_text", "data": "%PATH%"}),
        ),
        (
            ProviderValue::TextList(vec!["a".to_owned(), "b".to_owned()]),
            json!({"type": "text_list", "data": ["a", "b"]}),
        ),
        (ProviderValue::U32(5), json!({"type": "u32", "data": 5})),
        (ProviderValue::U64(6), json!({"type": "u64", "data": 6})),
        (
            ProviderValue::Binary {
                raw_type: 3,
                bytes: vec![1, 2],
            },
            json!({"type": "binary", "data": {"raw_type": 3, "bytes": [1, 2]}}),
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(serde_json::to_value(&value).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<ProviderValue>(expected).unwrap(),
            value
        );
    }
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_valid_unicode_serializes_with_its_native_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let native = Path::new("/data").join(OsStr::from_bytes(b"bad\xffname.txt"));
    let path = AbsPath::new(native).unwrap();
    assert_eq!(
        path.raw_bytes().unwrap().as_slice(),
        b"/data/bad\xffname.txt"
    );
    assert_eq!(path.to_language_string(), "/data/bad\u{fffd}name.txt");
    let json = serde_json::to_value(&path).unwrap();
    assert_eq!(
        json,
        json!({"path": "/data/bad\u{fffd}name.txt", "raw_path": "L2RhdGEvYmFk/25hbWUudHh0"})
    );
    assert_eq!(serde_json::from_value::<AbsPath>(json).unwrap(), path);
    let target = nurt_contracts::journal::TargetPath::Fs(path.clone());
    let round = serde_json::to_string(&target).unwrap();
    assert_eq!(
        serde_json::from_str::<nurt_contracts::journal::TargetPath>(&round).unwrap(),
        target
    );
}

#[test]
fn a_path_that_is_valid_unicode_stays_a_plain_string() {
    let path = abs("/data/caf\u{e9}.txt");
    assert_eq!(path.raw_bytes(), None);
    assert_eq!(
        serde_json::to_value(&path).unwrap(),
        json!(at("/data/caf\u{e9}.txt"))
    );
    assert_eq!(
        serde_json::from_value::<AbsPath>(json!(at("/data/caf\u{e9}.txt"))).unwrap(),
        path
    );
}

#[test]
fn raw_bytes_of_a_relative_path_are_refused() {
    assert!(AbsPath::from_raw_bytes(b"data/a.txt").is_err());
}

#[test]
fn metadata_transfer_is_complete_only_without_refusals() {
    let mut report = MetaTransferReport::default();
    assert!(report.is_complete());
    report
        .not_transferred
        .push("xattr:security.selinux".to_owned());
    assert!(!report.is_complete());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
