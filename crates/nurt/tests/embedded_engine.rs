// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The embedded engine, opened over a state directory in a temporary
//! directory. No language front end is connected, so programs fail with
//! `E_UNSUPPORTED`; the tests cover the facade: conversion of envelopes,
//! events, errors, cancellation and sharing between threads.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::Duration;

use nurt::{Embedded, ErrorKind, Request, Status, Variant};
use serde_json::{Value, json};

const PROGRAM: &str = r#"find "/srv/app" glob: "*.go""#;
const WAIT: Duration = Duration::from_secs(30);

fn engine(dir: &tempfile::TempDir) -> Embedded {
    Embedded::builder()
        .isolated_configuration()
        .state_dir(dir.path().join("state"))
        .open()
        .unwrap()
}

#[test]
fn program_without_a_front_end_fails_with_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let response = engine(&dir).run(&Request::program(PROGRAM)).unwrap();
    assert!(!response.is_ok());
    assert_eq!(response.status(), Some("failed"));
    assert_eq!(response.error().unwrap().code(), "E_UNSUPPORTED");
}

#[test]
fn relative_save_path_is_reported_in_the_response() {
    let dir = tempfile::tempdir().unwrap();
    let request = Request::program(PROGRAM).save_to("result.json");
    let response = engine(&dir).run(&request).unwrap();
    assert_eq!(response.error().unwrap().code(), "E_PATH_RELATIVE");
}

#[test]
fn unknown_cursor_is_a_language_error_in_the_response() {
    let dir = tempfile::tempdir().unwrap();
    let response = engine(&dir).page("c_unknown", Some(10)).unwrap();
    assert!(!response.is_ok());
    assert!(response.error().unwrap().code().starts_with("E_"));
}

#[test]
fn info_names_the_state_directory_and_the_formats() {
    let dir = tempfile::tempdir().unwrap();
    let info = engine(&dir).info().unwrap();
    assert_eq!(info["formats"]["envelope"], "1.0");
    let state = info["state_dir"].as_str().unwrap();
    assert!(std::path::Path::new(state).starts_with(dir.path()));
    assert!(std::path::Path::new(state).is_dir());
}

#[test]
fn engine_without_a_state_directory_reports_none() {
    let engine = Embedded::builder()
        .isolated_configuration()
        .without_state_dir()
        .open()
        .unwrap();
    assert_eq!(engine.info().unwrap()["state_dir"], Value::Null);
}

#[test]
fn configuration_overrides_reach_the_engine() {
    let engine = Embedded::builder()
        .isolated_configuration()
        .without_state_dir()
        .config(json!({"journal": {"max_ops": 50}}))
        .open()
        .unwrap();
    assert!(engine.info().is_ok());
}

#[test]
fn invalid_configuration_value_is_an_argument_error() {
    let error = Embedded::builder()
        .isolated_configuration()
        .without_state_dir()
        .config(json!({"journal": {"max_ops": 0}}))
        .open()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidArgument);
    assert_eq!(error.status(), Status::InvalidArgument);
}

#[test]
fn empty_session_is_an_argument_error() {
    let error = Embedded::builder()
        .isolated_configuration()
        .without_state_dir()
        .session("")
        .open()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn state_directory_that_cannot_be_created_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("file");
    std::fs::write(&blocker, b"x").unwrap();
    let error = Embedded::builder()
        .isolated_configuration()
        .state_dir(blocker.join("state"))
        .open()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
}

#[test]
fn known_schema_is_returned_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let schema = engine(&dir).schema("report").unwrap();
    let document: Value = serde_json::from_str(schema.as_str().unwrap()).unwrap();
    assert!(document.is_object());
}

#[test]
fn unknown_schema_is_a_language_error_with_a_code() {
    let dir = tempfile::tempdir().unwrap();
    let error = engine(&dir).schema("nothing").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Language);
    assert_eq!(error.code(), Some("E_UNKNOWN_VALUE"));
    assert!(error.message().contains("report"));
}

#[test]
fn calls_that_need_a_missing_component_report_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    let failures = [
        engine.parse(PROGRAM, Variant::A).unwrap_err(),
        engine.correct(PROGRAM, Variant::B).unwrap_err(),
        engine.check(&Request::program(PROGRAM)).unwrap_err(),
        engine.catalog().unwrap_err(),
        engine.history(None).unwrap_err(),
        engine.space(None).unwrap_err(),
        engine.help("core", None).unwrap_err(),
        engine.describe("core", None).unwrap_err(),
    ];
    for error in failures {
        assert_eq!(error.kind(), ErrorKind::Language);
        assert_eq!(error.code(), Some("E_UNSUPPORTED"), "{error}");
    }
}

#[test]
fn invalid_call_arguments_are_argument_errors() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    let failures = [
        engine
            .print(&json!({"not": "a tree"}), Variant::A)
            .unwrap_err(),
        engine.history(Some(&json!({"limit": "many"}))).unwrap_err(),
        engine.space(Some("relative/volume")).unwrap_err(),
    ];
    for error in failures {
        assert_eq!(error.kind(), ErrorKind::InvalidArgument, "{error}");
    }
}

#[test]
fn started_request_delivers_events_in_order_and_one_response() {
    let dir = tempfile::tempdir().unwrap();
    let execution = engine(&dir).start(&Request::program(PROGRAM)).unwrap();
    let events: Vec<_> = execution.events().collect();
    assert_eq!(events.first().unwrap().kind(), "exec.started");
    assert_eq!(events.last().unwrap().kind(), "exec.finished");
    let sequence: Vec<_> = events.iter().filter_map(nurt::Event::seq).collect();
    assert_eq!(sequence, (1..=events.len() as u64).collect::<Vec<_>>());
    for event in &events {
        assert_eq!(event.raw()["request_id"], execution.request_id());
    }
    let response = execution.wait().unwrap();
    assert_eq!(response.status(), Some("failed"));
    assert_eq!(execution.wait().unwrap(), response);
    assert!(execution.is_finished());
}

#[test]
fn wait_timeout_returns_the_response_of_a_finished_request() {
    let dir = tempfile::tempdir().unwrap();
    let execution = engine(&dir).start(&Request::program(PROGRAM)).unwrap();
    let response = execution.wait_timeout(WAIT).unwrap();
    assert_eq!(response.unwrap().status(), Some("failed"));
}

#[test]
fn cancelling_a_finished_request_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let execution = engine(&dir).start(&Request::program(PROGRAM)).unwrap();
    let response = execution.wait().unwrap();
    execution.cancel();
    assert_eq!(execution.wait().unwrap(), response);
}

#[test]
fn dropped_execution_does_not_block_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    drop(engine.start(&Request::program(PROGRAM)).unwrap());
    let response = engine.run(&Request::program(PROGRAM)).unwrap();
    assert!(!response.is_ok());
}

#[test]
fn request_ids_differ_between_executions() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    let first = engine.start(&Request::program(PROGRAM)).unwrap();
    let second = engine.start(&Request::program(PROGRAM)).unwrap();
    assert_ne!(first.request_id(), second.request_id());
}

#[test]
fn clones_run_requests_from_many_threads() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let engine = engine.clone();
            thread::spawn(move || {
                (0..5)
                    .map(|_| engine.run(&Request::program(PROGRAM)).unwrap())
                    .all(|response| response.error().is_some())
            })
        })
        .collect();
    for worker in workers {
        assert!(worker.join().unwrap());
    }
}
