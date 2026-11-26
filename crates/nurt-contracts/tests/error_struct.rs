// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Tests assert through `unwrap`, `expect` and `panic!`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour and JSON form of the error structure.

use std::error::Error as _;

use nurt_contracts::error::{At, Candidate, Error, ErrorCode, ErrorData, Location, Result, Stage};
use serde_json::json;

fn sample() -> Error {
    Error::new(
        ErrorCode::Ambiguous,
        "the correction touches a target of a change",
    )
    .with_stage(Stage::Parse)
    .with_at(At::new(1, 8, 22))
    .with_found("\"/home/user/projct\"")
    .with_problem("target_correction")
    .with_fix("remove \"/home/user/project/old.log\"")
    .with_help("correction")
    .with_candidate(Candidate {
        rank: 1,
        reference: Some("corr-1".to_owned()),
        program: "remove \"/home/user/project/old.log\"".to_owned(),
        why: Some("nearest existing path".to_owned()),
    })
    .with_next_step("list \"/home/user/project\"")
    .with_detail("path", json!("/home/user/projct"))
}

#[test]
fn error_is_one_pointer_wide() {
    assert_eq!(std::mem::size_of::<Error>(), std::mem::size_of::<usize>());
    assert!(std::mem::size_of::<Result<()>>() <= 2 * std::mem::size_of::<usize>());
}

#[test]
fn error_serializes_without_empty_members() {
    let json = serde_json::to_value(sample()).unwrap();
    assert_eq!(
        json,
        json!({
            "code": "E_AMBIGUOUS",
            "message": "the correction touches a target of a change",
            "at": {"line": 1, "column": 8, "length": 22, "stage": "0"},
            "found": "\"/home/user/projct\"",
            "problem": "target_correction",
            "fix": ["remove \"/home/user/project/old.log\""],
            "help": "correction",
            "candidates": [{
                "rank": 1,
                "ref": "corr-1",
                "program": "remove \"/home/user/project/old.log\"",
                "why": "nearest existing path"
            }],
            "next_steps": ["list \"/home/user/project\""],
            "details": {"path": "/home/user/projct"}
        })
    );
    let back: Error = serde_json::from_value(json).unwrap();
    assert_eq!(back, sample());
}

#[test]
fn minimal_error_has_code_and_message() {
    let error = Error::new(ErrorCode::Stale, "the file changed after it was read");
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({"code": "E_STALE", "message": "the file changed after it was read"})
    );
}

#[test]
fn unknown_members_are_ignored_and_unknown_codes_are_rejected() {
    let json = json!({"code": "E_STALE", "message": "changed", "extra": 1});
    let error = serde_json::from_value::<Error>(json).unwrap();
    assert_eq!(error.message, "changed");
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({"code": "E_STALE", "message": "changed"})
    );
    let json = json!({"code": "E_NOT_A_CODE", "message": "changed"});
    assert!(serde_json::from_value::<Error>(json).is_err());
}

#[test]
fn stage_and_place_merge_in_at() {
    let stage_first = Error::new(ErrorCode::Type, "expected a number")
        .with_stage(Stage::Bind)
        .with_at(At::new(2, 4, 1));
    let place_first = Error::new(ErrorCode::Type, "expected a number")
        .with_at(At::new(2, 4, 1))
        .with_stage(Stage::Bind);
    assert_eq!(stage_first, place_first);
    assert_eq!(
        serde_json::to_value(&stage_first).unwrap()["at"],
        json!({"line": 2, "column": 4, "length": 1, "stage": "1"})
    );
    assert_eq!(Error::new(ErrorCode::Stale, "changed").stage(), None);
}

#[test]
fn display_shows_code_and_source_is_cause() {
    let cause = Error::new(ErrorCode::Io, "input/output error");
    let error = Error::new(ErrorCode::RollbackFailed, "two files were not restored")
        .with_stage(Stage::Verify)
        .with_cause(cause.clone());
    assert_eq!(
        error.to_string(),
        "E_ROLLBACK_FAILED: two files were not restored"
    );
    let source = error.source().unwrap();
    assert_eq!(source.to_string(), cause.to_string());
    assert!(source.source().is_none());
}

#[test]
fn fields_are_reachable_through_deref() {
    let mut error = Error::new(ErrorCode::Assert, "expected 12 paths, found 14")
        .with_stage(Stage::After)
        .with_effect_applied(true)
        .with_expected(json!(12))
        .with_actual(json!(14))
        .with_sample("/home/user/src/a.go");
    assert_eq!(error.code, ErrorCode::Assert);
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(error.stage(), Some(Stage::After));
    assert!(error.effect_applied);
    error.sample.push("/home/user/src/b.go".to_owned());
    assert_eq!(error.data().sample.len(), 2);
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(json["at"], json!({"stage": "5a"}));
    assert_eq!(json["effect_applied"], true);
    let data: ErrorData = error.into_data();
    assert_eq!(Error::from(data.clone()).into_data(), data);
}

#[test]
fn location_points_into_data_file() {
    let error = Error::new(ErrorCode::DocParse, "expected a value").with_location(Location {
        path: "/home/user/project/package.json".to_owned(),
        line: Some(3),
        column: Some(14),
        end_line: None,
        end_column: None,
        byte: Some(41),
    });
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(
        json["location"],
        json!({"path": "/home/user/project/package.json", "line": 3, "column": 14, "byte": 41})
    );
}

#[test]
fn error_survives_cbor_round_trip() {
    let error = sample().with_cause(Error::new(ErrorCode::NotFound, "no such file"));
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&error, &mut bytes).unwrap();
    let back: Error = ciborium::de::from_reader(bytes.as_slice()).unwrap();
    assert_eq!(back, error);
}

#[test]
fn record_errors_convert_to_type_errors() {
    use nurt_contracts::record::{Entry, RecordError};
    use nurt_contracts::value::{Record, RecordKind};

    let error = Entry::try_from(Record::new(RecordKind::Line)).unwrap_err();
    assert_eq!(
        error,
        RecordError::KindMismatch {
            expected: RecordKind::Entry,
            found: RecordKind::Line
        }
    );
    let error = Error::from(error);
    assert_eq!(error.code, ErrorCode::Type);
    assert_eq!(error.details["record"]["problem"], "kind_mismatch");
}

#[test]
fn file_system_errors_convert_with_details() {
    use nurt_contracts::fs::{FsError, FsErrorKind};

    let mut cause = FsError::new(
        FsErrorKind::NotFound,
        "rename",
        Some(std::path::Path::new("/home/user/a.txt")),
    );
    cause.os_code = Some(2);
    let error = Error::from(cause);
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.details["op"], "rename");
    assert_eq!(error.details["path"], "/home/user/a.txt");
    assert_eq!(error.details["os_error"], 2);
    assert!(error.details.get("holders").is_none());
    assert!(error.message.starts_with("rename failed"));
}
