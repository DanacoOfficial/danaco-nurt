// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/tests/abi_rust.rs
// Responsibility:   Exported functions called through their Rust declarations, on the engine.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Exported functions called through their Rust declarations, on the engine
//! of the feature `stub` and the cases of `tests/fixtures/stub`.

#![cfg(feature = "stub")]
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

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nurt_contracts::envelope::Response;
use nurt_ffi::stub;
use nurt_ffi::{
    Engine, Exec, NURT_ABI_VERSION, NURT_VARIANT_A, NURT_WAIT_FOREVER, NurtBuf, NurtStatus,
    nurt_abi_check, nurt_abi_version, nurt_buf_free, nurt_catalog, nurt_check, nurt_correct,
    nurt_describe, nurt_engine_close, nurt_engine_config_get, nurt_engine_open, nurt_exec_cancel,
    nurt_exec_free, nurt_exec_start, nurt_exec_wait, nurt_ext_list, nurt_ext_load, nurt_help,
    nurt_history, nurt_info, nurt_last_error, nurt_page, nurt_parse, nurt_print, nurt_run_json,
    nurt_schema, nurt_session_end, nurt_set_log_callback, nurt_space, nurt_version,
};
use serde_json::Value;

const READ: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\" glob: \"*.go\""}"#;
const SYNTAX_ERROR: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\" glob: [\"*.go\""}"#;
const EVENTS: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\" glob: \"*.rs\""}"#;
const SLOW: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/archive\""}"#;
const PANICS: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/panic\""}"#;
const UNKNOWN: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/none\""}"#;
const TWO_INPUTS: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\"","cursor":"c_91fe"}"#;
const PROGRAM: &CStr = cr#"find "/srv/app""#;
const MISSPELT: &CStr = cr#"fnd "/srv/app""#;
const AST: &CStr =
    cr#"{"nurt":"1.0","program":{"statements":[{"t":"pipeline","head":{"t":"stage","name":"find","args":[{"value":{"t":"text","v":"/srv/app"}}]},"stages":[]}]}}"#;
const EXTENSION: &CStr = c"/opt/nurt/ext/sample/nurt-ext.toml";
const EXTENSION_FUTURE: &CStr = c"/opt/nurt/ext/future/nurt-ext.toml";
const EXTENSION_MISSING: &CStr = c"/opt/nurt/ext/missing/nurt-ext.toml";

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

/// Open engine closed when it goes out of scope.
struct Open(*mut Engine);

impl Open {
    fn new() -> Self {
        let mut engine = ptr::null_mut();
        // SAFETY: A null configuration and a valid output pointer.
        assert_eq!(
            unsafe { nurt_engine_open(ptr::null(), &raw mut engine) },
            NurtStatus::Ok
        );
        assert!(!engine.is_null());
        Self(engine)
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: The handle was returned by `nurt_engine_open`.
        unsafe { nurt_engine_close(self.0) };
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

fn start(
    e: *mut Engine,
    request: &CStr,
    callback: nurt_ffi::EventCallback,
    user: *mut c_void,
) -> *mut Exec {
    let mut exec = ptr::null_mut();
    // SAFETY: A live engine or null, a static request and a valid output pointer.
    assert_eq!(
        unsafe { nurt_exec_start(e, request.as_ptr(), callback, user, &raw mut exec) },
        NurtStatus::Ok
    );
    exec
}

fn with_exec(
    e: *mut Engine,
    request: &CStr,
    body: impl FnOnce(*mut Exec) -> NurtStatus,
) -> NurtStatus {
    let exec = start(e, request, None, ptr::null_mut());
    let status = body(exec);
    // SAFETY: The handle was returned by `nurt_exec_start` and is released once.
    unsafe { nurt_exec_free(exec) };
    status
}

/// Calls an exported function that takes an engine, with valid other
/// arguments; `e` may be null.
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
            "nurt_correct" => nurt_correct(e, MISSPELT.as_ptr(), NURT_VARIANT_A, a),
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

fn ok_scenario(function: &str) -> NurtStatus {
    let mut out = Buf::new();
    // SAFETY: Valid buffers, static strings and live handles.
    unsafe {
        match function {
            "nurt_abi_check" => nurt_abi_check(NURT_ABI_VERSION),
            "nurt_info" => nurt_info(out.ptr()),
            "nurt_engine_open" => {
                let mut other = ptr::null_mut();
                let status = nurt_engine_open(ptr::null(), &raw mut other);
                nurt_engine_close(other);
                status
            }
            "nurt_schema" => nurt_schema(c"ast".as_ptr(), out.ptr()),
            "nurt_last_error" => nurt_last_error(ptr::null_mut(), out.ptr()),
            "nurt_exec_wait" => {
                let engine = Open::new();
                with_exec(engine.0, READ, |exec| {
                    nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr())
                })
            }
            "nurt_exec_cancel" => {
                let engine = Open::new();
                with_exec(engine.0, READ, |exec| nurt_exec_cancel(exec))
            }
            other => engine_call(other, Open::new().0),
        }
    }
}

fn arg_scenario(function: &str) -> NurtStatus {
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

fn scenario(function: &str, status: &str) -> NurtStatus {
    let engine = Open::new();
    let mut out = Buf::new();
    match (function, status) {
        (_, "OK") => ok_scenario(function),
        (_, "ARG") => arg_scenario(function),
        (_, "INTERNAL") => {
            stub::set_thread_panic(Some(function));
            let status = ok_scenario(function);
            stub::set_thread_panic(None);
            status
        }
        ("nurt_abi_check", "ABI") => nurt_abi_check(2 << 16),
        // SAFETY: A live engine and a static path.
        ("nurt_ext_load", "ABI") => unsafe { nurt_ext_load(engine.0, EXTENSION_FUTURE.as_ptr()) },
        // SAFETY: A live engine and a static path.
        ("nurt_ext_load", "IO") => unsafe { nurt_ext_load(engine.0, EXTENSION_MISSING.as_ptr()) },
        // SAFETY: A live engine and a static session name.
        ("nurt_session_end", "IO") => unsafe { nurt_session_end(engine.0, c"s-io".as_ptr()) },
        ("nurt_engine_open", "IO") => {
            let dir = tempfile::tempdir().unwrap();
            stub::set_thread_cases_dir(Some(dir.path().join("missing")));
            let mut other = ptr::null_mut();
            // SAFETY: A null configuration and a valid output pointer.
            let status = unsafe { nurt_engine_open(ptr::null(), &raw mut other) };
            stub::set_thread_cases_dir(None);
            assert!(other.is_null());
            status
        }
        // SAFETY: A live execution handle and a valid buffer.
        ("nurt_exec_wait", "TIMEOUT") => with_exec(engine.0, SLOW, |exec| unsafe {
            nurt_exec_wait(exec, 0, out.ptr())
        }),
        ("nurt_exec_wait", "CANCELLED") => with_exec(engine.0, SLOW, |exec| {
            // SAFETY: A live execution handle and a valid buffer.
            unsafe {
                nurt_exec_cancel(exec);
                nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr())
            }
        }),
        _ => panic!("no scenario gives {status} for {function}"),
    }
}

fn status_named(name: &str) -> NurtStatus {
    *NurtStatus::ALL
        .iter()
        .find(|status| {
            status
                .name()
                .trim_start_matches("NURT_E_")
                .trim_start_matches("NURT_")
                == name
        })
        .expect("status name")
}

#[test]
fn status_matrix_of_the_header_holds() {
    let matrix = common::header_matrix();
    assert_eq!(matrix.len(), 23, "rows of the table in nurt.h");
    for (function, statuses) in &matrix {
        assert!(statuses.contains("OK"), "{function} has no OK column");
        for status in statuses {
            assert_eq!(
                scenario(function, status),
                status_named(status),
                "{function} -> {status}"
            );
        }
    }
}

#[test]
fn run_json_returns_the_response_of_a_read_request() {
    let engine = Open::new();
    let mut out = Buf::new();
    // SAFETY: A live engine, a static request and a valid buffer.
    assert_eq!(
        unsafe { nurt_run_json(engine.0, READ.as_ptr(), out.ptr()) },
        NurtStatus::Ok
    );
    let response: Response = serde_json::from_slice(out.0.as_bytes()).unwrap();
    response.validate().unwrap();
    assert_eq!(response.report.records.unwrap().matched, Some(148));
}

#[test]
fn language_errors_return_ok_with_the_error_in_the_response() {
    let engine = Open::new();
    for (request, code) in [(SYNTAX_ERROR, "E_SYNTAX"), (TWO_INPUTS, "E_ARG")] {
        let mut out = Buf::new();
        // SAFETY: A live engine, a static request and a valid buffer.
        assert_eq!(
            unsafe { nurt_run_json(engine.0, request.as_ptr(), out.ptr()) },
            NurtStatus::Ok
        );
        let response: Response = serde_json::from_slice(out.0.as_bytes()).unwrap();
        response.validate().unwrap();
        assert_eq!(response.report.error.unwrap().code.as_str(), code);
    }
}

#[test]
fn invalid_envelopes_are_argument_errors_with_empty_output() {
    let engine = Open::new();
    let invalid_utf8: [u8; 3] = [b'{', 0xff, 0];
    let requests: [*const c_char; 4] = [
        c"not json".as_ptr(),
        c"[1]".as_ptr(),
        cr#"{"nurt":"1.0","program":"find","colour":1}"#.as_ptr(),
        invalid_utf8.as_ptr().cast(),
    ];
    for request in requests {
        let mut out = Buf::new();
        // SAFETY: A live engine, a NUL-terminated request and a valid buffer.
        assert_eq!(
            unsafe { nurt_run_json(engine.0, request, out.ptr()) },
            NurtStatus::Arg
        );
        assert!(out.0.is_empty());
        let record = last_error();
        assert_eq!(record["code"], "NURT_E_ARG");
        assert_eq!(record["detail"]["function"], "nurt_run_json");
    }
    let mut out = Buf::new();
    // SAFETY: A live engine, a static program and a valid buffer.
    let status = unsafe { nurt_correct(engine.0, PROGRAM.as_ptr(), 7, out.ptr()) };
    assert_eq!(status, NurtStatus::Arg);
}

#[test]
fn calls_without_a_case_and_panics_in_the_engine_are_internal() {
    let engine = Open::new();
    let mut out = Buf::new();
    // SAFETY: A live engine, a static request and a valid buffer.
    assert_eq!(
        unsafe { nurt_run_json(engine.0, UNKNOWN.as_ptr(), out.ptr()) },
        NurtStatus::Internal
    );
    assert_eq!(last_error()["detail"]["key"].as_str().unwrap().len(), 64);
    // SAFETY: As above.
    assert_eq!(
        unsafe { nurt_run_json(engine.0, PANICS.as_ptr(), out.ptr()) },
        NurtStatus::Internal
    );
    assert!(
        last_error()["message"]
            .as_str()
            .unwrap()
            .contains("the stub case raised a panic")
    );
    let status = with_exec(engine.0, PANICS, |exec| {
        // SAFETY: A live execution handle and a valid buffer.
        unsafe { nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr()) }
    });
    assert_eq!(status, NurtStatus::Internal);
    assert!(out.0.is_empty());
    // SAFETY: The engine stays usable after a caught panic.
    assert_eq!(
        unsafe { nurt_run_json(engine.0, READ.as_ptr(), out.ptr()) },
        NurtStatus::Ok
    );
}

static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "C" fn collect_log(_user: *mut c_void, level: c_int, msg: *const c_char) {
    // SAFETY: The library passes a NUL-terminated message valid during the call.
    let text = unsafe { CStr::from_ptr(msg) }
        .to_string_lossy()
        .into_owned();
    LOGGED.lock().unwrap().push(format!("{level} {text}"));
}

#[test]
fn injected_panic_is_caught_recorded_and_logged() {
    // SAFETY: The receiver is a function without state that may run on any thread.
    unsafe { nurt_set_log_callback(Some(collect_log), ptr::null_mut()) };
    stub::set_thread_panic(Some("nurt_info"));
    let mut out = Buf::new();
    // SAFETY: A valid buffer.
    let status = unsafe { nurt_info(out.ptr()) };
    stub::set_thread_panic(None);
    // SAFETY: Logging is stopped.
    unsafe { nurt_set_log_callback(None, ptr::null_mut()) };
    assert_eq!(status, NurtStatus::Internal);
    assert!(out.0.is_empty());
    let record = last_error();
    assert_eq!(record["status"], 6);
    assert!(
        record["message"]
            .as_str()
            .unwrap()
            .contains("injected panic in nurt_info")
    );
    let logged = LOGGED.lock().unwrap();
    assert!(
        logged
            .iter()
            .any(|line| line.starts_with("1 ") && line.contains("nurt_info"))
    );
}

#[test]
fn buffers_are_terminated_and_release_is_idempotent() {
    let mut info = NurtBuf::EMPTY;
    // SAFETY: A valid buffer.
    assert_eq!(unsafe { nurt_info(&raw mut info) }, NurtStatus::Ok);
    assert!(!info.is_empty());
    // SAFETY: The library writes `len + 1` bytes; the last one is NUL.
    let terminated = unsafe { CStr::from_ptr(info.as_bytes().as_ptr().cast()) };
    assert_eq!(terminated.to_bytes(), info.as_bytes());
    // SAFETY: A buffer filled by the library, then the zero buffer it leaves,
    // then a null pointer.
    unsafe {
        nurt_buf_free(&raw mut info);
        assert_eq!(info, NurtBuf::EMPTY);
        nurt_buf_free(&raw mut info);
        nurt_buf_free(ptr::null_mut());
    }
    assert_eq!(info, NurtBuf::EMPTY);
}

#[test]
fn last_error_is_kept_per_thread() {
    let arg = std::thread::spawn(|| {
        // SAFETY: A null buffer is rejected.
        assert_eq!(unsafe { nurt_info(ptr::null_mut()) }, NurtStatus::Arg);
        last_error()
    });
    let abi = std::thread::spawn(|| {
        assert_eq!(nurt_abi_check(3 << 16), NurtStatus::Abi);
        last_error()
    });
    let clean = std::thread::spawn(last_error);
    assert_eq!(arg.join().unwrap()["code"], "NURT_E_ARG");
    assert_eq!(abi.join().unwrap()["code"], "NURT_E_ABI");
    assert_eq!(clean.join().unwrap(), Value::Null);
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

fn types(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["t"].as_str().unwrap())
        .collect()
}

#[test]
fn events_arrive_in_seq_order_and_end_with_exec_finished() {
    let engine = Open::new();
    let events = Mutex::new(Vec::new());
    let exec = start(
        engine.0,
        EVENTS,
        Some(collect_event),
        (&raw const events).cast_mut().cast(),
    );
    let mut out = Buf::new();
    // SAFETY: A live execution handle and a valid buffer.
    assert_eq!(
        unsafe { nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr()) },
        NurtStatus::Ok
    );
    // SAFETY: The handle is released once.
    unsafe { nurt_exec_free(exec) };
    let events = events.into_inner().unwrap();
    assert_eq!(
        types(&events),
        [
            "exec.started",
            "log",
            "progress",
            "progress",
            "exec.finished"
        ]
    );
    let seq: Vec<u64> = events
        .iter()
        .map(|event| event["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seq, [1, 2, 3, 4, 5]);
    assert_eq!(events[4]["status"], "ok");
    assert!(
        events
            .iter()
            .all(|event| event["request"] == events[0]["request"] && event["request"].is_string())
    );
    assert_eq!(out.json()["report"]["status"], "ok");
}

#[test]
fn zero_timeout_polls_and_cancellation_gives_the_cancelled_response() {
    let engine = Open::new();
    let events = Mutex::new(Vec::new());
    let exec = start(
        engine.0,
        SLOW,
        Some(collect_event),
        (&raw const events).cast_mut().cast(),
    );
    let mut out = Buf::new();
    // SAFETY: A live execution handle and a valid buffer, used until the
    // handle is released at the end of the block.
    unsafe {
        assert_eq!(nurt_exec_wait(exec, 0, out.ptr()), NurtStatus::Timeout);
        assert!(out.0.is_empty());
        assert_eq!(nurt_exec_cancel(exec), NurtStatus::Ok);
        assert_eq!(nurt_exec_cancel(exec), NurtStatus::Ok);
        assert_eq!(
            nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr()),
            NurtStatus::Cancelled
        );
        let first = out.text().to_owned();
        assert_eq!(nurt_exec_wait(exec, 0, out.ptr()), NurtStatus::Cancelled);
        assert_eq!(out.text(), first);
        nurt_exec_free(exec);
    }
    let response: Response = serde_json::from_slice(out.0.as_bytes()).unwrap();
    response.validate().unwrap();
    assert_eq!(response.report.error.unwrap().code.as_str(), "E_CANCELLED");
    let events = events.into_inner().unwrap();
    assert_eq!(types(&events).last(), Some(&"exec.finished"));
    assert_eq!(events.last().unwrap()["status"], "failed");
}

#[test]
fn cancellation_after_completion_keeps_the_result() {
    let engine = Open::new();
    let exec = start(engine.0, READ, None, ptr::null_mut());
    let (mut first, mut second) = (Buf::new(), Buf::new());
    // SAFETY: A live execution handle and valid buffers.
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
}

#[test]
fn closing_the_engine_cancels_running_executions() {
    let engine = Open::new();
    let exec = start(engine.0, SLOW, None, ptr::null_mut());
    let begin = Instant::now();
    drop(engine);
    let mut out = Buf::new();
    // SAFETY: The execution handle outlives the engine until it is released.
    unsafe {
        assert_eq!(nurt_exec_wait(exec, 0, out.ptr()), NurtStatus::Cancelled);
        nurt_exec_free(exec);
    }
    assert!(begin.elapsed() < Duration::from_secs(10));
}

#[test]
fn releasing_a_running_execution_cancels_it() {
    let engine = Open::new();
    let exec = start(engine.0, SLOW, None, ptr::null_mut());
    let begin = Instant::now();
    // SAFETY: The handle is released once; null is accepted.
    unsafe {
        nurt_exec_free(exec);
        nurt_exec_free(ptr::null_mut());
    }
    assert!(begin.elapsed() < Duration::from_secs(10));
}

#[test]
fn asynchronous_envelope_rule_violation_completes_with_e_arg() {
    let engine = Open::new();
    let mut out = Buf::new();
    let status = with_exec(engine.0, TWO_INPUTS, |exec| {
        // SAFETY: A live execution handle and a valid buffer.
        unsafe { nurt_exec_wait(exec, NURT_WAIT_FOREVER, out.ptr()) }
    });
    assert_eq!(status, NurtStatus::Ok);
    assert_eq!(out.json()["report"]["error"]["code"], "E_ARG");
}

#[test]
fn every_published_schema_is_returned_by_name() {
    for (name, content) in nurt_contracts::schema::ALL {
        let mut out = Buf::new();
        let name = std::ffi::CString::new(*name).unwrap();
        // SAFETY: A NUL-terminated name and a valid buffer.
        assert_eq!(
            unsafe { nurt_schema(name.as_ptr(), out.ptr()) },
            NurtStatus::Ok
        );
        assert_eq!(out.text(), *content);
    }
    let mut out = Buf::new();
    // SAFETY: A null name is rejected.
    assert_eq!(
        unsafe { nurt_schema(ptr::null(), out.ptr()) },
        NurtStatus::Arg
    );
    // SAFETY: A static name and a valid buffer.
    assert_eq!(
        unsafe { nurt_schema(c"unknown".as_ptr(), out.ptr()) },
        NurtStatus::Arg
    );
    assert!(
        last_error()["detail"]["expected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "ast")
    );
}

#[test]
fn versions_and_information_describe_the_library() {
    assert_eq!(nurt_abi_version(), NURT_ABI_VERSION);
    assert_eq!(nurt_abi_check(NURT_ABI_VERSION + 1), NurtStatus::Abi);
    // SAFETY: The version is a static NUL-terminated string.
    let version = unsafe { CStr::from_ptr(nurt_version()) };
    assert_eq!(version.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    let mut out = Buf::new();
    // SAFETY: A valid buffer.
    assert_eq!(unsafe { nurt_info(out.ptr()) }, NurtStatus::Ok);
    let info = out.json();
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(info["features"][0], "stub");
}

#[test]
fn operations_return_the_results_of_their_cases() {
    let engine = Open::new();
    let (mut first, mut second) = (Buf::new(), Buf::new());
    // SAFETY: A live engine, static strings and valid buffers.
    unsafe {
        assert_eq!(
            nurt_parse(
                engine.0,
                PROGRAM.as_ptr(),
                NURT_VARIANT_A,
                first.ptr(),
                second.ptr()
            ),
            NurtStatus::Ok
        );
        assert_eq!(
            first.json()["program"]["statements"][0]["head"]["name"],
            "find"
        );
        assert_eq!(second.json()["diagnostics"], serde_json::json!([]));
        assert_eq!(
            nurt_print(engine.0, AST.as_ptr(), NURT_VARIANT_A, first.ptr()),
            NurtStatus::Ok
        );
        assert_eq!(first.text(), r#"find "/srv/app""#);
        assert_eq!(
            nurt_check(engine.0, READ.as_ptr(), first.ptr(), second.ptr()),
            NurtStatus::Ok
        );
        assert_eq!(first.json()["effect"], "read");
        assert_eq!(
            nurt_help(engine.0, ptr::null(), first.ptr()),
            NurtStatus::Ok
        );
        assert!(first.text().starts_with("Topics:"));
        assert_eq!(
            nurt_print(engine.0, c"[1]".as_ptr(), NURT_VARIANT_A, first.ptr()),
            NurtStatus::Arg
        );
        assert!(first.0.is_empty());
    }
}

#[test]
fn engine_open_rejects_invalid_configuration() {
    for config in [c"[1]", c"{", c"\"text\""] {
        let mut engine = ptr::NonNull::<Engine>::dangling().as_ptr();
        // SAFETY: A static configuration and a valid output pointer.
        assert_eq!(
            unsafe { nurt_engine_open(config.as_ptr(), &raw mut engine) },
            NurtStatus::Arg
        );
        assert!(engine.is_null());
    }
    let mut engine = ptr::null_mut();
    // SAFETY: An empty configuration object and a valid output pointer.
    assert_eq!(
        unsafe { nurt_engine_open(c"{}".as_ptr(), &raw mut engine) },
        NurtStatus::Ok
    );
    // SAFETY: The handle was returned by `nurt_engine_open`; null is accepted.
    unsafe {
        nurt_engine_close(engine);
        nurt_engine_close(ptr::null_mut());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
