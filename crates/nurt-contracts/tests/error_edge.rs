// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stages, positions, candidates and the builder methods of the error
//! structure, and the status of the C interface.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::str::FromStr;

use nurt_contracts::error::{
    AbiStatus, At, Candidate, Error, ErrorCode, ErrorData, Location, Stage, UnknownName,
};
use serde_json::json;

#[test]
fn stages_are_ordered_by_execution() {
    assert!(Stage::ALL.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(Stage::ALL.len(), 9);
    assert_eq!(Stage::ALL.first(), Some(&Stage::Parse));
    assert_eq!(Stage::ALL.last(), Some(&Stage::Report));
    let names: Vec<&str> = Stage::ALL.iter().map(|stage| stage.name()).collect();
    assert_eq!(
        names,
        [
            "parse", "bind", "read", "check", "plan", "apply", "after", "verify", "report"
        ]
    );
}

#[test]
fn only_the_stages_before_the_effect_leave_the_world_unchanged() {
    let before: Vec<Stage> = Stage::ALL
        .iter()
        .copied()
        .filter(|stage| stage.is_before_effect())
        .collect();
    assert_eq!(
        before,
        [
            Stage::Parse,
            Stage::Bind,
            Stage::Read,
            Stage::Check,
            Stage::Plan
        ]
    );
    let changing: Vec<Stage> = Stage::ALL
        .iter()
        .copied()
        .filter(|stage| stage.changes_world())
        .collect();
    assert_eq!(changing, [Stage::Apply, Stage::Verify]);
}

#[test]
fn stage_text_is_a_number_or_a_name() {
    assert_eq!(Stage::from_str("0"), Ok(Stage::Parse));
    assert_eq!(Stage::from_str("5a"), Ok(Stage::After));
    assert_eq!(Stage::from_str("after"), Ok(Stage::After));
    assert_eq!(Stage::from_str("report"), Ok(Stage::Report));
    for text in ["", " 5", "5A", "5b", "8", "-1", "Parse", "parse "] {
        let error = Stage::from_str(text).unwrap_err();
        assert_eq!(error.what, "stage");
        assert_eq!(error.text, text);
        assert_eq!(error.to_string(), format!("unknown stage `{text}`"));
    }
}

#[test]
fn unknown_names_name_what_was_parsed() {
    let error = ErrorCode::from_str("E_NOPE").unwrap_err();
    assert_eq!(
        error,
        UnknownName {
            what: "error code".to_owned(),
            text: "E_NOPE".to_owned()
        }
    );
    assert_eq!(error.to_string(), "unknown error code `E_NOPE`");
    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value, json!({"what": "error code", "text": "E_NOPE"}));
    assert_eq!(serde_json::from_value::<UnknownName>(value).unwrap(), error);
}

#[test]
fn a_position_without_members_is_an_empty_object() {
    assert_eq!(serde_json::to_value(At::default()).unwrap(), json!({}));
    let at: At = serde_json::from_value(json!({"line": 2})).unwrap();
    assert_eq!(at.line, Some(2));
    assert_eq!(at.column, None);
    assert_eq!(at.stage, None);
    assert!(serde_json::from_value::<At>(json!({"line": 2, "row": 3})).is_err());
    assert!(serde_json::from_value::<At>(json!({"stage": "8"})).is_err());
    assert_eq!(
        serde_json::to_value(At::new(1, 2, 3).with_stage(Stage::After)).unwrap(),
        json!({"line": 1, "column": 2, "length": 3, "stage": "5a"})
    );
}

#[test]
fn a_location_requires_a_path_and_rejects_unknown_members() {
    let location: Location = serde_json::from_value(json!({
        "path": "/home/user/project/a.json",
        "line": 3,
        "column": 14,
        "end_line": 3,
        "end_column": 15,
        "byte": 41
    }))
    .unwrap();
    assert_eq!(location.byte, Some(41));
    assert_eq!(
        serde_json::from_value::<Location>(serde_json::to_value(&location).unwrap()).unwrap(),
        location
    );
    assert!(serde_json::from_value::<Location>(json!({"line": 3})).is_err());
    assert!(serde_json::from_value::<Location>(json!({"path": "/a", "row": 3})).is_err());
}

#[test]
fn a_candidate_names_its_reference_ref() {
    let candidate: Candidate = serde_json::from_value(json!({
        "rank": 2,
        "program": "remove \"/home/user/old.log\""
    }))
    .unwrap();
    assert_eq!(candidate.reference, None);
    assert_eq!(candidate.why, None);
    let with_reference = Candidate {
        reference: Some("corr-1".to_owned()),
        ..candidate.clone()
    };
    let value = serde_json::to_value(&with_reference).unwrap();
    assert_eq!(value["ref"], "corr-1");
    assert!(value.get("reference").is_none());
    assert!(
        serde_json::to_value(&candidate)
            .unwrap()
            .get("ref")
            .is_none()
    );
    assert!(serde_json::from_value::<Candidate>(json!({"rank": 1})).is_err());
    assert!(
        serde_json::from_value::<Candidate>(json!({"rank": 1, "program": "x", "extra": true}))
            .is_err()
    );
}

#[test]
fn error_data_reads_a_code_and_a_message_only() {
    let data: ErrorData =
        serde_json::from_value(json!({"code": "E_NO_MATCH", "message": "found nothing"})).unwrap();
    assert_eq!(data.code, ErrorCode::NoMatch);
    assert!(data.at.is_none() && data.location.is_none());
    assert!(data.fix.is_empty() && data.sample.is_empty());
    assert!(data.candidates.is_empty() && data.next_steps.is_empty());
    assert!(!data.effect_applied);
    assert!(data.details.is_empty());
    assert!(data.cause.is_none());
    assert!(serde_json::from_value::<ErrorData>(json!({"code": "E_NO_MATCH"})).is_err());
    assert!(serde_json::from_value::<ErrorData>(json!({"message": "x"})).is_err());
}

#[test]
fn the_effect_marker_is_written_only_when_true() {
    let error = Error::new(ErrorCode::Assert, "expected 12 paths, found 14");
    assert!(
        serde_json::to_value(&error)
            .unwrap()
            .get("effect_applied")
            .is_none()
    );
    let error = error.with_effect_applied(true);
    assert_eq!(
        serde_json::to_value(&error).unwrap()["effect_applied"],
        true
    );
    let error = error.with_effect_applied(false);
    assert!(
        serde_json::to_value(&error)
            .unwrap()
            .get("effect_applied")
            .is_none()
    );
}

#[test]
fn a_place_with_a_stage_replaces_the_stage_of_the_error() {
    let error = Error::new(ErrorCode::Syntax, "unexpected token").with_stage(Stage::Parse);
    assert_eq!(error.stage(), Some(Stage::Parse));

    let kept = error.clone().with_at(At::new(1, 5, 2));
    assert_eq!(kept.stage(), Some(Stage::Parse));
    assert_eq!(kept.at.unwrap().column, Some(5));

    let replaced = error.with_at(At::new(1, 5, 2).with_stage(Stage::Bind));
    assert_eq!(replaced.stage(), Some(Stage::Bind));

    let only_stage = Error::new(ErrorCode::Stale, "changed").with_at(At::stage(Stage::Apply));
    assert_eq!(only_stage.stage(), Some(Stage::Apply));
    assert_eq!(only_stage.at.unwrap().line, None);
}

#[test]
fn builder_methods_accumulate_lists_and_replace_details() {
    let error = Error::new(ErrorCode::Ambiguous, "two candidates")
        .with_fix("remove \"/home/user/a.log\"")
        .with_fix("remove \"/home/user/b.log\"")
        .with_sample("/home/user/a.log")
        .with_sample("/home/user/b.log")
        .with_next_step("list \"/home/user\"")
        .with_candidate(Candidate {
            rank: 1,
            reference: None,
            program: "remove \"/home/user/a.log\"".to_owned(),
            why: None,
        })
        .with_detail("count", json!(1))
        .with_detail("count", json!(2))
        .with_expected(json!(["a", "b"]))
        .with_actual(json!("c"))
        .with_found("c")
        .with_problem("typo")
        .with_help("correction");
    assert_eq!(error.fix.len(), 2);
    assert_eq!(error.sample, ["/home/user/a.log", "/home/user/b.log"]);
    assert_eq!(error.next_steps, ["list \"/home/user\""]);
    assert_eq!(error.candidates.len(), 1);
    assert_eq!(error.details["count"], json!(2));
    assert_eq!(error.details.len(), 1);
    assert_eq!(error.expected, Some(json!(["a", "b"])));
    assert_eq!(error.actual, Some(json!("c")));
    assert_eq!(error.found.as_deref(), Some("c"));
    assert_eq!(error.problem.as_deref(), Some("typo"));
    assert_eq!(error.help.as_deref(), Some("correction"));
}

#[test]
fn changes_through_the_mutable_reference_reach_the_data() {
    let mut error = Error::new(ErrorCode::Io, "disk error");
    error.message = "disk error on /dev/sda".to_owned();
    error.problem = Some("read".to_owned());
    assert_eq!(error.data().message, "disk error on /dev/sda");
    assert_eq!(error.clone().into_data().problem.as_deref(), Some("read"));
    assert_eq!(error.to_string(), "E_IO: disk error on /dev/sda");
}

#[test]
fn error_data_converts_into_an_error() {
    let data = Error::new(ErrorCode::Limit, "too many records")
        .with_stage(Stage::Read)
        .into_data();
    let error = Error::from(data.clone());
    assert_eq!(error.data(), &data);
    assert_eq!(error.code(), ErrorCode::Limit);
}

#[test]
fn nested_causes_form_a_chain() {
    let root = Error::new(ErrorCode::Io, "disk error");
    let middle = Error::new(ErrorCode::InUse, "file is in use").with_cause(root);
    let top = Error::new(ErrorCode::Stale, "the file changed")
        .with_stage(Stage::Apply)
        .with_cause(middle);
    let mut codes = Vec::new();
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(&top);
    while let Some(error) = current {
        codes.push(error.to_string());
        current = error.source();
    }
    assert_eq!(
        codes,
        [
            "E_STALE: the file changed",
            "E_IN_USE: file is in use",
            "E_IO: disk error"
        ]
    );
    let json = serde_json::to_value(&top).unwrap();
    assert_eq!(json["cause"]["cause"]["code"], "E_IO");
    assert_eq!(serde_json::from_value::<Error>(json).unwrap(), top);
}

#[test]
fn internal_errors_may_occur_in_every_stage() {
    assert_eq!(ErrorCode::Internal.stages(), Stage::ALL);
    assert_eq!(ErrorCode::InternalCorrection.abi(), AbiStatus::Internal);
    assert_eq!(ErrorCode::Internal.abi_status(), 6);
}

#[test]
fn interface_statuses_have_numbers_names_and_json_forms() {
    let table = [
        (AbiStatus::Ok, 0, "NURT_OK", "ok"),
        (AbiStatus::Timeout, 1, "NURT_E_TIMEOUT", "timeout"),
        (AbiStatus::Cancelled, 2, "NURT_E_CANCELLED", "cancelled"),
        (AbiStatus::Arg, 3, "NURT_E_ARG", "arg"),
        (AbiStatus::Abi, 4, "NURT_E_ABI", "abi"),
        (AbiStatus::Io, 5, "NURT_E_IO", "io"),
        (AbiStatus::Internal, 6, "NURT_E_INTERNAL", "internal"),
    ];
    assert_eq!(AbiStatus::ALL.len(), table.len());
    for (status, number, name, json_name) in table {
        assert_eq!(status.code(), number);
        assert_eq!(status.name(), name);
        assert_eq!(serde_json::to_value(status).unwrap(), json!(json_name));
    }
}

#[test]
fn the_error_schema_describes_the_data_structure() {
    let schema = serde_json::to_value(schemars::schema_for!(Error)).unwrap();
    assert!(schema.is_object());
    let text = schema.to_string();
    assert!(text.contains("\"code\""), "{text}");
    assert!(text.contains("\"message\""), "{text}");
}
