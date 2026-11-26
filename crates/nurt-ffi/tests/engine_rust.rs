// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Exported functions called through their Rust declarations on the engine
//! of `nurt-core`, in a build without the feature `stub`. The engine has no
//! language front end in this crate, so programs end with language errors,
//! which the tests expect in the JSON results.

#![cfg(not(feature = "stub"))]
#![allow(
    unsafe_code,
    reason = "the tests call the exported functions with raw pointers, as a C caller does"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

mod common;

use std::ffi::{CStr, CString, c_char, c_void};
use std::fs;
use std::ptr;
use std::sync::Mutex;

use nurt_contracts::envelope::Response;
use nurt_ffi::{
    Engine, Exec, NURT_ABI_VERSION, NURT_VARIANT_A, NURT_WAIT_FOREVER, NurtBuf, NurtStatus,
    nurt_abi_check, nurt_buf_free, nurt_catalog, nurt_check, nurt_correct, nurt_describe,
    nurt_engine_close, nurt_engine_config_get, nurt_engine_open, nurt_exec_cancel, nurt_exec_free,
    nurt_exec_start, nurt_exec_wait, nurt_ext_list, nurt_ext_load, nurt_help, nurt_history,
    nurt_info, nurt_last_error, nurt_page, nurt_parse, nurt_print, nurt_run_json, nurt_schema,
    nurt_session_end, nurt_space,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const READ: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\""}"#;
const TWO_INPUTS: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\"","cursor":"c_91fe"}"#;
const RELATIVE_SAVE: &CStr =
    cr#"{"nurt":"1.0","program":"find \"/srv/app\"","options":{"save_to":"out.json"}}"#;
const PROGRAM: &CStr = cr#"find "/srv/app""#;
const AST: &CStr = cr#"{"nurt":"1.0","program":{"statements":[]}}"#;
const EXTENSION: &CStr = c"/opt/nurt/ext/sample/nurt-ext.toml";

/// Output buffer released when it goes out of scope.
struct Buf(NurtBuf);

impl Buf {
    fn new() -> Self {
        Self(NurtBuf::EMPTY)
    }

    fn ptr(&mut self) -> *mut NurtBuf {
        &raw mut self.0
    }

    fn text(&self) -> &str {
        self.0.as_str().expect("UTF-8 buffer")
    }

    fn json(&self) -> Value {
        serde_json::from_slice(self.0.as_bytes()).expect("JSON buffer")
    }
}

impl Drop for Buf {
    fn drop(&mut self) {
        // SAFETY: The buffer is empty or was filled by the library.
        unsafe { nurt_buf_free(&raw mut self.0) };
    }
}

/// Configuration placing the state directory of the engine in `dir`.
fn config_in(dir: &std::path::Path) -> CString {
    let config = json!({"state": {"dir": dir.join("state").to_string_lossy()}});
    CString::new(config.to_string()).unwrap()
}

fn open_with(config: &CStr) -> (NurtStatus, *mut Engine) {
    let mut engine = ptr::NonNull::<Engine>::dangling().as_ptr();
    // SAFETY: A NUL-terminated configuration and a valid output pointer.
    let status = unsafe { nurt_engine_open(config.as_ptr(), &raw mut engine) };
    (status, engine)
}

/// Engine with its state directory in a temporary directory, closed when
/// it goes out of scope.
struct Open {
    engine: *mut Engine,
    _dir: TempDir,
}

impl Open {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (status, engine) = open_with(&config_in(dir.path()));
        assert_eq!(status, NurtStatus::Ok, "{}", last_error());
        assert!(!engine.is_null());
        Self { engine, _dir: dir }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: The handle was returned by `nurt_engine_open`.
        unsafe { nurt_engine_close(self.engine) };
    }
}

fn last_error() -> Value {
    let mut record = Buf::new();
    // SAFETY: A valid output buffer; the engine may be null.
    assert_eq!(
        unsafe { nurt_last_error(ptr::null_mut(), record.ptr()) },
        NurtStatus::Ok
    );
    if record.0.is_empty() {
        Value::Null
    } else {
        record.json()
    }
}

fn start(e: *mut Engine, request: &CStr, events: Option<&Mutex<Vec<Value>>>) -> *mut Exec {
    let mut exec = ptr::null_mut();
    let (callback, user) = match events {
        Some(events) => (
            Some(collect_event as unsafe extern "C" fn(*mut c_void, *const c_char)),
            ptr::from_ref(events).cast_mut().cast(),
        ),
        None => (None, ptr::null_mut()),
    };
    // SAFETY: A live engine, a static request, a callback with its state and
    // a valid output pointer.
    assert_eq!(
        unsafe { nurt_exec_start(e, request.as_ptr(), callback, user, &raw mut exec) },
        NurtStatus::Ok
    );
    exec
}

unsafe extern "C" fn collect_event(user: *mut c_void, event: *const c_char) {
    // SAFETY: `user` is the `Mutex<Vec<Value>>` passed by the test, alive until
    // the execution completes; the event is NUL-terminated.
    let (events, text) = unsafe { (&*user.cast::<Mutex<Vec<Value>>>(), CStr::from_ptr(event)) };
    events
        .lock()
        .unwrap()
        .push(serde_json::from_slice(text.to_bytes()).unwrap());
}

/// Calls an exported function with valid arguments; `e` may be null.
fn engine_call(function: &str, e: *mut Engine) -> NurtStatus {
    let (mut first, mut second) = (Buf::new(), Buf::new());
    let (a, b) = (first.ptr(), second.ptr());
    // SAFETY: Every pointer is null, a live handle, a static string or a
    // buffer owned by this frame.
    unsafe {
        match function {
            "nurt_engine_config_get" => nurt_engine_config_get(e, a),
            "nurt_parse" => nurt_parse(e, PROGRAM.as_ptr(), NURT_VARIANT_A, a, b),
            "nurt_print" => nurt_print(e, AST.as_ptr(), NURT_VARIANT_A, a),
            "nurt_correct" => nurt_correct(e, PROGRAM.as_ptr(), NURT_VARIANT_A, a),
            "nurt_check" => nurt_check(e, READ.as_ptr(), a, b),
            "nurt_run_json" => nurt_run_json(e, READ.as_ptr(), a),
            "nurt_exec_start" => {
                let mut exec = ptr::null_mut();
                let status =
                    nurt_exec_start(e, READ.as_ptr(), None, ptr::null_mut(), &raw mut exec);
                nurt_exec_free(exec);
                status
            }
            "nurt_page" => nurt_page(e, c"c_91fe".as_ptr(), 0, a),
            "nurt_catalog" => nurt_catalog(e, ptr::null(), a),
            "nurt_describe" => nurt_describe(e, ptr::null(), a),
            "nurt_help" => nurt_help(e, ptr::null(), a),
            "nurt_history" => nurt_history(e, ptr::null(), a),
            "nurt_space" => nurt_space(e, ptr::null(), a),
            "nurt_session_end" => nurt_session_end(e, c"s-1".as_ptr()),
            "nurt_ext_load" => nurt_ext_load(e, EXTENSION.as_ptr()),
            "nurt_ext_list" => nurt_ext_list(e, a),
            other => panic!("{other} does not take an engine"),
        }
    }
}

fn valid_call(function: &str, engine: &Open) -> NurtStatus {
    let mut out = Buf::new();
    // SAFETY: Valid buffers, static strings and live handles.
    unsafe {
        match function {
            "nurt_abi_check" => nurt_abi_check(NURT_ABI_VERSION),
            "nurt_info" => nurt_info(out.ptr()),
            "nurt_engine_open" => {
                let dir = tempfile::tempdir().unwrap();
                let (status, other) = open_with(&config_in(dir.path()));
                nurt_engine_close(other);
                status
            }
            "nurt_schema" => nurt_schema(c"ast".as_ptr(), out.ptr()),
            "nurt_last_error" => nurt_last_error(ptr::null_mut(), out.ptr()),
            "nurt_exec_wait" => {
                let exec = start(engine.engine, READ, None);
                let status = nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr());
                nurt_exec_free(exec);
                status
            }
            "nurt_exec_cancel" => {
                let exec = start(engine.engine, READ, None);
                let status = nurt_exec_cancel(exec);
                nurt_exec_free(exec);
                status
            }
            other => engine_call(other, engine.engine),
        }
    }
}

fn null_call(function: &str) -> NurtStatus {
    let mut out = Buf::new();
    // SAFETY: Null pointers are passed where the interface rejects them;
    // every other pointer is valid.
    unsafe {
        match function {
            "nurt_info" => nurt_info(ptr::null_mut()),
            "nurt_engine_open" => nurt_engine_open(ptr::null(), ptr::null_mut()),
            "nurt_exec_wait" => nurt_exec_wait(ptr::null_mut(), 0, out.ptr()),
            "nurt_exec_cancel" => nurt_exec_cancel(ptr::null_mut()),
            "nurt_schema" => nurt_schema(c"unknown".as_ptr(), out.ptr()),
            "nurt_last_error" => nurt_last_error(ptr::null_mut(), ptr::null_mut()),
            other => engine_call(other, ptr::null_mut()),
        }
    }
}

fn status_name(status: NurtStatus) -> &'static str {
    status
        .name()
        .trim_start_matches("NURT_E_")
        .trim_start_matches("NURT_")
}

#[test]
fn every_status_returned_by_the_engine_is_allowed_by_the_header() {
    let engine = Open::new();
    let matrix = common::header_matrix();
    assert_eq!(matrix.len(), 23, "rows of the table in nurt.h");
    for (function, statuses) in &matrix {
        let status = valid_call(function, &engine);
        assert!(
            statuses.contains(status_name(status)),
            "{function} returned {}",
            status.name()
        );
        if statuses.contains("ARG") {
            assert_eq!(null_call(function), NurtStatus::Arg, "{function} with null");
        }
    }
}

#[test]
fn run_json_reports_language_errors_in_the_response() {
    let engine = Open::new();
    for (request, code) in [
        (READ, "E_UNSUPPORTED"),
        (TWO_INPUTS, "E_ARG"),
        (RELATIVE_SAVE, "E_PATH_RELATIVE"),
    ] {
        let mut out = Buf::new();
        // SAFETY: A live engine, a static request and a valid buffer.
        assert_eq!(
            unsafe { nurt_run_json(engine.engine, request.as_ptr(), out.ptr()) },
            NurtStatus::Ok
        );
        let response: Response = serde_json::from_slice(out.0.as_bytes()).unwrap();
        response.validate().unwrap();
        assert_eq!(response.report.error.unwrap().code.as_str(), code);
    }
    let mut out = Buf::new();
    // SAFETY: A live engine, a static request and a valid buffer.
    assert_eq!(
        unsafe { nurt_run_json(engine.engine, c"not json".as_ptr(), out.ptr()) },
        NurtStatus::Arg
    );
    assert!(out.0.is_empty());
    assert_eq!(last_error()["detail"]["function"], "nurt_run_json");
}

#[test]
fn executions_deliver_the_framed_events_of_the_engine() {
    let engine = Open::new();
    let events = Mutex::new(Vec::new());
    let exec = start(engine.engine, READ, Some(&events));
    let (mut first, mut second) = (Buf::new(), Buf::new());
    // SAFETY: A live execution handle and valid buffers, used until the
    // handle is released at the end of the block.
    unsafe {
        assert_eq!(
            nurt_exec_wait(exec, NURT_WAIT_FOREVER, first.ptr()),
            NurtStatus::Ok
        );
        assert_eq!(nurt_exec_cancel(exec), NurtStatus::Ok);
        assert_eq!(nurt_exec_wait(exec, 0, second.ptr()), NurtStatus::Ok);
        nurt_exec_free(exec);
    }
    assert_eq!(first.text(), second.text());
    assert_eq!(first.json()["report"]["error"]["code"], "E_UNSUPPORTED");
    let events = events.into_inner().unwrap();
    let frames: Vec<(&str, u64)> = events
        .iter()
        .map(|event| (event["t"].as_str().unwrap(), event["seq"].as_u64().unwrap()))
        .collect();
    assert_eq!(frames, [("exec.started", 1), ("exec.finished", 2)]);
    assert_eq!(events[1]["status"], "failed");
    let request_id = events[0]["request_id"].as_str().unwrap();
    assert!(!request_id.is_empty());
    assert_eq!(events[1]["request_id"], request_id);
    assert!(!events[0]["time"].is_null());
}

#[test]
fn cancelled_or_closed_executions_end_with_a_consistent_result() {
    let engine = Open::new();
    let exec = start(engine.engine, READ, None);
    let mut out = Buf::new();
    // SAFETY: A live execution handle and a valid buffer.
    let status = unsafe {
        nurt_exec_cancel(exec);
        nurt_exec_cancel(exec);
        nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr())
    };
    let code = out.json()["report"]["error"]["code"].clone();
    match status {
        NurtStatus::Cancelled => assert_eq!(code, "E_CANCELLED"),
        NurtStatus::Ok => assert_ne!(code, "E_CANCELLED"),
        other => panic!("cancelled execution returned {}", other.name()),
    }
    // SAFETY: The handle is released once.
    unsafe { nurt_exec_free(exec) };
    let running = start(engine.engine, READ, None);
    drop(engine);
    let mut after = Buf::new();
    // SAFETY: The execution handle outlives the engine until it is released.
    unsafe {
        let status = nurt_exec_wait(running, 0, after.ptr());
        assert!(matches!(status, NurtStatus::Ok | NurtStatus::Cancelled));
        assert!(!after.0.is_empty());
        nurt_exec_free(running);
    }
}

#[test]
fn documents_of_the_language_front_end_carry_its_errors() {
    let engine = Open::new();
    let e = engine.engine;
    let (mut first, mut second) = (Buf::new(), Buf::new());
    // SAFETY: A live engine, static strings and valid buffers.
    unsafe {
        assert_eq!(
            nurt_parse(
                e,
                PROGRAM.as_ptr(),
                NURT_VARIANT_A,
                first.ptr(),
                second.ptr()
            ),
            NurtStatus::Ok
        );
        assert_eq!(first.text(), "null");
        assert_eq!(second.json()["error"]["code"], "E_UNSUPPORTED");
        assert_eq!(
            nurt_check(e, READ.as_ptr(), first.ptr(), second.ptr()),
            NurtStatus::Ok
        );
        assert_eq!(first.text(), "null");
        assert_eq!(second.json()["diagnostics"], json!([]));
        assert_eq!(second.json()["error"]["code"], "E_UNSUPPORTED");
        assert_eq!(
            nurt_correct(e, PROGRAM.as_ptr(), NURT_VARIANT_A, first.ptr()),
            NurtStatus::Ok
        );
        assert_eq!(nurt_catalog(e, ptr::null(), second.ptr()), NurtStatus::Ok);
    }
    assert_eq!(first.json()["error"]["code"], "E_UNSUPPORTED");
    assert!(second.json().get("error").is_none());
}

#[test]
fn documents_of_the_journal_and_the_session_carry_their_errors() {
    let engine = Open::new();
    let e = engine.engine;
    let mut out = Buf::new();
    // SAFETY: A live engine, static strings and a valid buffer.
    let (history, invalid_history, space) = unsafe {
        let history = nurt_history(e, c"{\"limit\":5}".as_ptr(), out.ptr());
        let code = out.json()["error"]["code"].clone();
        let invalid = nurt_history(e, c"{\"colour\":1}".as_ptr(), out.ptr());
        let space = nurt_space(e, c"srv".as_ptr(), out.ptr());
        assert_eq!(code, "E_UNSUPPORTED");
        (history, invalid, space)
    };
    assert_eq!(
        (history, invalid_history, space),
        (NurtStatus::Ok, NurtStatus::Arg, NurtStatus::Ok)
    );
    assert_eq!(out.json()["error"]["code"], "E_PATH_RELATIVE");
    // SAFETY: As above.
    let statuses = unsafe {
        let page = nurt_page(e, c"c_91fe".as_ptr(), 3, out.ptr());
        assert_eq!(out.json()["report"]["status"], "failed");
        let extensions = nurt_ext_list(e, out.ptr());
        [
            page,
            extensions,
            nurt_session_end(e, c"s-1".as_ptr()),
            nurt_session_end(e, c"".as_ptr()),
        ]
    };
    assert_eq!(out.json(), json!({"extensions": []}));
    assert_eq!(
        statuses,
        [
            NurtStatus::Ok,
            NurtStatus::Ok,
            NurtStatus::Ok,
            NurtStatus::Arg
        ]
    );
}

#[test]
fn text_results_fail_with_the_status_of_the_language_error() {
    let engine = Open::new();
    let e = engine.engine;
    let mut out = Buf::new();
    // SAFETY: A live engine, static strings and a valid buffer.
    let statuses = unsafe {
        [
            nurt_print(e, AST.as_ptr(), NURT_VARIANT_A, out.ptr()),
            nurt_describe(
                e,
                cr#"{"layer":"core","language":"en"}"#.as_ptr(),
                out.ptr(),
            ),
            nurt_help(e, c"find".as_ptr(), out.ptr()),
            nurt_ext_load(e, EXTENSION.as_ptr()),
        ]
    };
    assert!(statuses.iter().all(|status| *status == NurtStatus::Arg));
    assert!(out.0.is_empty());
    assert_eq!(last_error()["detail"]["error"]["code"], "E_UNSUPPORTED");
    // SAFETY: A live engine, a JSON array instead of a tree and a valid buffer.
    let status = unsafe { nurt_print(e, c"[1]".as_ptr(), NURT_VARIANT_A, out.ptr()) };
    assert_eq!(status, NurtStatus::Arg);
}

#[test]
fn engine_open_applies_the_configuration_and_reports_failures() {
    let dir = tempfile::tempdir().unwrap();
    let (status, engine) = open_with(&config_in(dir.path()));
    assert_eq!(status, NurtStatus::Ok);
    let mut out = Buf::new();
    // SAFETY: A live engine and a valid buffer; the engine is closed once.
    unsafe {
        assert_eq!(nurt_engine_config_get(engine, out.ptr()), NurtStatus::Ok);
        nurt_engine_close(engine);
    }
    let state = dir.path().join("state");
    assert_eq!(out.json()["state"]["dir"], state.to_string_lossy().as_ref());
    assert!(state.is_dir());
    let (status, engine) = open_with(cr#"{"state":{"dir":"state"}}"#);
    assert_eq!(status, NurtStatus::Arg);
    assert!(engine.is_null());
    assert_eq!(last_error()["detail"]["function"], "nurt_engine_open");
    let file = dir.path().join("file");
    fs::write(&file, b"").unwrap();
    let blocked = json!({"state": {"dir": file.join("state").to_string_lossy()}});
    let (status, engine) = open_with(&CString::new(blocked.to_string()).unwrap());
    assert_eq!(status, NurtStatus::Io);
    assert!(engine.is_null());
    let (status, _) = open_with(c"[1]");
    assert_eq!(status, NurtStatus::Arg);
}

#[test]
fn info_reports_the_revision_of_the_built_in_catalog() {
    let mut out = Buf::new();
    // SAFETY: A valid buffer.
    assert_eq!(unsafe { nurt_info(out.ptr()) }, NurtStatus::Ok);
    let info = out.json();
    assert!(info["catalog_rev"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(info["features"], json!([]));
}

#[test]
fn session_identifiers_are_checked_by_characters() {
    let engine = Open::new();
    let too_long = CString::new("s".repeat(100_000)).unwrap();
    let control = CString::new("s\u{7}1").unwrap();
    let multi_byte = CString::new("sesja-żółw-日本").unwrap();
    let cases = [
        (c"", NurtStatus::Arg),
        (too_long.as_c_str(), NurtStatus::Arg),
        (control.as_c_str(), NurtStatus::Arg),
        (multi_byte.as_c_str(), NurtStatus::Ok),
    ];
    for (session, expected) in cases {
        // SAFETY: A live engine and a NUL-terminated session identifier.
        let status = unsafe { nurt_session_end(engine.engine, session.as_ptr()) };
        assert_eq!(status, expected, "{session:?}");
    }
    assert_eq!(last_error()["detail"]["function"], "nurt_session_end");
}
