// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks of the journal and file system contracts: object safety and thread
//! safety of the interfaces, stable serialized names, path normalisation,
//! content hashes and entity tags.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::fs::{
    AbsPath, ContentHash, Etag, FileId, FileKind, FileLock, FileMeta, Fs, FsError, FsErrorKind,
    FsWrite, Provider, ProviderWrite, ReadHandle, ScopePurpose, TxScope, WriteHandle,
};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, EntryState, Expect, JournalStore, MutationTx, OpId, OpKind,
    OpState, PreserveMode, TargetAction, TargetPath, TargetSpec,
};
use nurt_contracts::report::Irreversible;
use serde_json::json;

fn assert_send_sync<T: ?Sized + Send + Sync>() {}

#[test]
fn interfaces_are_object_safe_send_and_sync() {
    assert_send_sync::<dyn JournalStore>();
    assert_send_sync::<dyn MutationTx>();
    assert_send_sync::<dyn Admission>();
    assert_send_sync::<dyn Fs>();
    assert_send_sync::<dyn FsWrite>();
    assert_send_sync::<dyn Provider>();
    assert_send_sync::<dyn ProviderWrite>();
    assert_send_sync::<dyn ReadHandle>();
    assert_send_sync::<dyn WriteHandle>();
    assert_send_sync::<dyn FileLock>();
}

#[test]
fn op_state_follows_entry_state() {
    let cases = [
        (EntryState::Prepared, None),
        (EntryState::Committed, Some(OpState::Available)),
        (EntryState::RollingBack, None),
        (EntryState::Partial, Some(OpState::Partial)),
        (EntryState::Undone, Some(OpState::Undone)),
        (EntryState::Expiring, Some(OpState::Expired)),
        (EntryState::Expired, Some(OpState::Expired)),
        (EntryState::Corrupt, Some(OpState::Expired)),
    ];
    for (entry, expected) in cases {
        assert_eq!(OpState::from_entry(entry), expected, "{entry:?}");
    }
}

#[test]
fn op_state_names_are_stable() {
    for state in [
        OpState::Available,
        OpState::Partial,
        OpState::Expired,
        OpState::Undone,
    ] {
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(text, format!("\"{}\"", state.as_str()));
        assert_eq!(serde_json::from_str::<OpState>(&text).unwrap(), state);
    }
}

#[test]
fn preserve_mode_names_are_stable() {
    let all = [
        (PreserveMode::Moved, "moved"),
        (PreserveMode::Exchange, "exchange"),
        (PreserveMode::ReplaceBackup, "replace_backup"),
        (PreserveMode::Hardlink, "hardlink"),
        (PreserveMode::Clone, "clone"),
        (PreserveMode::Copy, "copy"),
        (PreserveMode::None, "none"),
        (PreserveMode::Created, "created"),
        (PreserveMode::Meta, "meta"),
        (PreserveMode::Ref, "ref"),
        (PreserveMode::Value, "value"),
        (PreserveMode::Append, "append"),
        (PreserveMode::Rename, "rename"),
        (PreserveMode::External, "external"),
    ];
    for (mode, name) in all {
        assert_eq!(mode.as_str(), name);
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<PreserveMode>(json!(name)).unwrap(),
            mode
        );
        let keeps = !matches!(mode, PreserveMode::None | PreserveMode::External);
        assert_eq!(mode.is_preserved(), keeps, "{name}");
    }
}

#[test]
fn admission_skip_carries_a_report_reason() {
    let decision = AdmissionDecision::Skip {
        reason: Irreversible::NoSpaceForCopy,
    };
    let value = serde_json::to_value(&decision).unwrap();
    assert_eq!(value, json!({"skip": {"reason": "no_space_for_copy"}}));
    assert_eq!(
        serde_json::from_value::<AdmissionDecision>(value).unwrap(),
        decision
    );
    assert_eq!(
        serde_json::to_value(AdmissionDecision::Keep).unwrap(),
        json!("keep")
    );
}

#[test]
fn op_kind_constructors_set_command_and_action() {
    assert_eq!(OpKind::command("write").action, None);
    let stash = OpKind::with_action("git.stash", "push");
    assert_eq!(stash.command, "git.stash");
    assert_eq!(stash.action.as_deref(), Some("push"));
    assert_eq!(OpKind::undo().command, "undo");
    assert_eq!(OpKind::redo().command, "redo");
    assert_eq!(
        serde_json::to_value(OpKind::command("remove")).unwrap(),
        json!({"command": "remove"})
    );
}

#[test]
fn content_hash_round_trips_through_its_text_form() {
    let text = format!("sha256:{}", "0123456789abcdef".repeat(4));
    let hash = ContentHash::parse(&text).unwrap();
    assert_eq!(hash.0[0], 0x01);
    assert_eq!(hash.0[31], 0xef);
    assert_eq!(hash.to_string(), text);
    assert_eq!(text.parse::<ContentHash>().unwrap(), hash);
    assert_eq!(serde_json::to_value(hash).unwrap(), json!(text));
    assert_eq!(
        serde_json::from_value::<ContentHash>(json!(text)).unwrap(),
        hash
    );

    let invalid = [
        String::new(),
        "0123".to_owned(),
        format!("sha256:{}", "AB".repeat(32)),
        format!("sha256:{}", "ab".repeat(31)),
        format!("sha512:{}", "ab".repeat(32)),
        format!("sha256:{}g", "ab".repeat(31)),
    ];
    for text in invalid {
        assert!(ContentHash::parse(&text).is_err(), "{text}");
        assert!(
            serde_json::from_value::<ContentHash>(json!(text)).is_err(),
            "{text}"
        );
    }
}

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
fn abs_path_is_normalised_lexically() {
    let path = abs("/home/user/./src/../README.md");
    assert_eq!(path.to_language_string(), at("/home/user/README.md"));
    assert_eq!(
        path.parent().unwrap().to_language_string(),
        at("/home/user")
    );
    assert_eq!(path.file_name().unwrap(), "README.md");
    assert!(path.starts_with(&abs("/home")));
    assert_eq!(abs("/").parent(), None);

    for invalid in [
        "relative/path".to_owned(),
        String::new(),
        at("/.."),
        at("/a/../.."),
    ] {
        let error = AbsPath::new(&invalid).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InvalidPath, "{invalid}");
    }

    let dir = abs("/home/user");
    assert_eq!(
        dir.join_name("a.txt").unwrap().to_language_string(),
        at("/home/user/a.txt")
    );
    for invalid in ["", ".", "..", "a/b"] {
        assert!(dir.join_name(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn abs_path_serializes_in_language_form() {
    let path = abs("/home/user/a.txt");
    assert_eq!(
        serde_json::to_value(&path).unwrap(),
        json!(at("/home/user/a.txt"))
    );
    let back: AbsPath = serde_json::from_value(json!(at("/home/user/./b/../a.txt"))).unwrap();
    assert_eq!(back, path);
    assert!(serde_json::from_value::<AbsPath>(json!("home/user")).is_err());
}

#[test]
fn target_spec_serializes_with_tagged_path_and_action() {
    let spec = TargetSpec {
        path: TargetPath::Fs(abs("/home/user/a.txt")),
        action: TargetAction::Rename {
            to: abs("/home/user/b.txt"),
            replace: true,
        },
        expect: Expect::Unchecked,
        size_hint: None,
    };
    let value = serde_json::to_value(&spec).unwrap();
    assert_eq!(
        value,
        json!({
            "path": {"kind": "fs", "at": at("/home/user/a.txt")},
            "action": {"action": "rename", "to": at("/home/user/b.txt"), "replace": true},
            "expect": "unchecked"
        })
    );
    assert_eq!(serde_json::from_value::<TargetSpec>(value).unwrap(), spec);

    let minimal: TargetSpec = serde_json::from_value(json!({
        "path": {"kind": "fs", "at": at("/home/user/a.txt")},
        "action": {"action": "delete"}
    }))
    .unwrap();
    assert_eq!(minimal.expect, Expect::Unchecked);
    assert_eq!(minimal.action, TargetAction::Delete);
}

fn sample_meta() -> FileMeta {
    FileMeta {
        kind: FileKind::File,
        size: 12,
        mode: 0o644,
        uid: Some(1000),
        gid: Some(1000),
        links: 1,
        modified_ns: 1_700_000_000_000_000_000,
        accessed_ns: None,
        created_ns: None,
        changed_ns: None,
        file_id: FileId {
            volume: 1,
            low: 42,
            high: 0,
        },
        windows_attributes: 0,
    }
}

#[test]
fn etags_match_on_size_time_identity_and_common_hash() {
    let meta = sample_meta();
    let tag = meta.etag();
    assert_eq!(tag.size, 12);
    assert_eq!(tag.hash, None);
    assert!(tag.matches(&tag));

    let first = ContentHash([1; 32]);
    let second = ContentHash([2; 32]);
    let with_first = Etag {
        hash: Some(first),
        ..tag.clone()
    };
    let with_second = Etag {
        hash: Some(second),
        ..tag.clone()
    };
    assert!(tag.matches(&with_first));
    assert!(with_first.matches(&tag));
    assert!(!with_first.matches(&with_second));

    let resized = Etag {
        size: 13,
        ..tag.clone()
    };
    assert!(!tag.matches(&resized));
    let moved = Etag {
        file_id: FileId {
            low: 43,
            ..tag.file_id
        },
        ..tag.clone()
    };
    assert!(!tag.matches(&moved));

    let value = serde_json::to_value(&tag).unwrap();
    assert!(value.get("hash").is_none());
    assert_eq!(serde_json::from_value::<Etag>(value).unwrap(), tag);
}

#[test]
fn fs_error_kinds_map_to_language_codes() {
    let cases = [
        (FsErrorKind::NotFound, "E_NOT_FOUND"),
        (FsErrorKind::PermissionDenied, "E_PERMISSION"),
        (FsErrorKind::ReadOnly, "E_PERMISSION"),
        (FsErrorKind::Privilege, "E_PRIVILEGE"),
        (FsErrorKind::Exists, "E_EXISTS"),
        (FsErrorKind::InUse, "E_IN_USE"),
        (FsErrorKind::NoSpace, "E_NO_SPACE"),
        (FsErrorKind::QuotaExceeded, "E_NO_SPACE"),
        (FsErrorKind::Stale, "E_STALE"),
        (FsErrorKind::CrossDevice, "E_IO"),
        (FsErrorKind::Unsupported, "E_IO"),
        (FsErrorKind::Loop, "E_IO"),
        (FsErrorKind::Io, "E_IO"),
    ];
    for (kind, code) in cases {
        assert_eq!(kind.code_id(), code, "{kind:?}");
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.as_str()));
    }
    let error = FsError::new(FsErrorKind::InUse, "rename", None);
    assert_eq!(error.code_id(), "E_IN_USE");
    assert!(error.to_string().starts_with("E_IN_USE (in_use) in rename"));
}

#[allow(
    clippy::disallowed_methods,
    reason = "the constructor is reserved for journal implementations; this test checks the constructor itself"
)]
#[test]
fn tx_scope_carries_operation_and_purpose() {
    let scope = TxScope::new_for_journal(OpId(7), ScopePurpose::Recovery);
    assert_eq!(scope.op(), OpId(7));
    assert_eq!(scope.purpose(), ScopePurpose::Recovery);
    assert_eq!(serde_json::to_value(OpId(7)).unwrap(), json!(7));
}

#[test]
fn schemas_describe_hash_and_path_strings() {
    let spec = serde_json::to_value(schemars::schema_for!(TargetSpec)).unwrap();
    assert!(spec.is_object());
    let hash = serde_json::to_value(schemars::schema_for!(ContentHash)).unwrap();
    assert_eq!(hash["pattern"], json!("^sha256:[0-9a-f]{64}$"));
    let path = serde_json::to_value(schemars::schema_for!(AbsPath)).unwrap();
    assert_eq!(path["type"], json!("string"));
}
