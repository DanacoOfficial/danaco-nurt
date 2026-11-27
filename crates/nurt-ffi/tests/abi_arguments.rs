// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/tests/abi_arguments.rs
// Responsibility:   Invalid arguments of the exported functions: null pointers, text that is.
// Dependencies:     serde_json, tempfile
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Invalid arguments of the exported functions: null pointers, text that is
//! not UTF-8, JSON of the wrong shape, values outside an enumeration and the
//! boundaries of the version check. The failures happen before the engine is
//! asked, so the tests hold in the build with the engine and in the build with
//! the feature `stub`.

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

use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use nurt_ffi::{
    Engine, Exec, NURT_ABI_VERSION, NURT_VARIANT_A, NurtBuf, NurtStatus, nurt_abi_check,
    nurt_abi_version, nurt_buf_free, nurt_catalog, nurt_check, nurt_correct, nurt_describe,
    nurt_engine_close, nurt_engine_config_get, nurt_engine_open, nurt_exec_cancel, nurt_exec_free,
    nurt_exec_start, nurt_exec_wait, nurt_ext_list, nurt_ext_load, nurt_help, nurt_history,
    nurt_info, nurt_last_error, nurt_page, nurt_parse, nurt_print, nurt_run_json, nurt_schema,
    nurt_session_end, nurt_set_log_callback, nurt_space,
};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Text that ends at the first NUL byte and is not UTF-8.
static NOT_UTF8: [u8; 3] = [b'{', 0xff, 0];
const PROGRAM: &CStr = c"find \"/srv/app\"";
const AST: &CStr = cr#"{"nurt":"1.0","program":{"statements":[]}}"#;
const REQUEST: &CStr = cr#"{"nurt":"1.0","program":"find \"/srv/app\""}"#;
const NOT_OBJECTS: [&CStr; 4] = [c"[1]", c"{", c"\"text\"", c"null"];

fn not_utf8() -> *const c_char {
    NOT_UTF8.as_ptr().cast()
}

/// Output buffer released when it goes out of scope.
struct Buf(NurtBuf);

impl Buf {
    fn new() -> Self {
        Self(NurtBuf::EMPTY)
    }

    fn ptr(&mut self) -> *mut NurtBuf {
        &raw mut self.0
    }
}

impl Drop for Buf {
    fn drop(&mut self) {
        // SAFETY: The buffer is empty or was filled by the library.
        unsafe { nurt_buf_free(&raw mut self.0) };
    }
}

/// Open engine with its state directory in a temporary directory, closed
/// when it goes out of scope.
struct Open {
    engine: *mut Engine,
    _dir: TempDir,
}

impl Open {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = json!({"state": {"dir": dir.path().join("state").to_string_lossy()}});
        let config = std::ffi::CString::new(config.to_string()).unwrap();
        let mut engine = ptr::null_mut();
        // SAFETY: A NUL-terminated configuration and a valid output pointer.
        let status = unsafe { nurt_engine_open(config.as_ptr(), &raw mut engine) };
        assert_eq!(status, NurtStatus::Ok);
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
    let status = unsafe { nurt_last_error(ptr::null_mut(), record.ptr()) };
    assert_eq!(status, NurtStatus::Ok);
    serde_json::from_slice(record.0.as_bytes()).unwrap_or(Value::Null)
}

/// Runs a call with two fresh output buffers and expects `NURT_E_ARG` and
/// empty buffers.
macro_rules! rejected {
    ($name:expr, |$first:ident, $second:ident| $call:expr) => {{
        let (mut one, mut two) = (Buf::new(), Buf::new());
        let ($first, $second) = (one.ptr(), two.ptr());
        let status = $call;
        assert_eq!(status, NurtStatus::Arg, "{}", $name);
        assert!(one.0.is_empty(), "{}", $name);
        assert!(two.0.is_empty(), "{}", $name);
    }};
}

#[allow(clippy::cognitive_complexity)] // one assertion per function of the C interface, kept in one table
#[test]
fn functions_with_a_buffer_reject_a_null_output() {
    let open = Open::new();
    let e = open.engine;
    let mut filled = Buf::new();
    // SAFETY: Valid handles, static strings and buffers owned by this frame;
    // a null output pointer is the argument under test.
    unsafe {
        assert_eq!(nurt_info(ptr::null_mut()), NurtStatus::Arg);
        assert_eq!(nurt_engine_config_get(e, ptr::null_mut()), NurtStatus::Arg);
        assert_eq!(
            nurt_parse(
                e,
                PROGRAM.as_ptr(),
                NURT_VARIANT_A,
                ptr::null_mut(),
                ptr::null_mut()
            ),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_parse(
                e,
                PROGRAM.as_ptr(),
                NURT_VARIANT_A,
                filled.ptr(),
                ptr::null_mut()
            ),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_parse(
                e,
                PROGRAM.as_ptr(),
                NURT_VARIANT_A,
                ptr::null_mut(),
                filled.ptr()
            ),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_print(e, AST.as_ptr(), NURT_VARIANT_A, ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_correct(e, PROGRAM.as_ptr(), NURT_VARIANT_A, ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_check(e, REQUEST.as_ptr(), ptr::null_mut(), filled.ptr()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_check(e, REQUEST.as_ptr(), filled.ptr(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_run_json(e, REQUEST.as_ptr(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_page(e, c"c_91fe".as_ptr(), 0, ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_catalog(e, ptr::null(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_describe(e, ptr::null(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(nurt_help(e, ptr::null(), ptr::null_mut()), NurtStatus::Arg);
        assert_eq!(
            nurt_history(e, ptr::null(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(nurt_space(e, ptr::null(), ptr::null_mut()), NurtStatus::Arg);
        assert_eq!(nurt_ext_list(e, ptr::null_mut()), NurtStatus::Arg);
        assert_eq!(
            nurt_schema(c"ast".as_ptr(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_last_error(ptr::null_mut(), ptr::null_mut()),
            NurtStatus::Arg
        );
        assert_eq!(
            nurt_exec_wait(ptr::null_mut(), 0, ptr::null_mut()),
            NurtStatus::Arg
        );
    }
    assert_eq!(last_error()["detail"]["function"], "nurt_exec_wait");
}

#[test]
fn a_null_output_of_nurt_info_is_an_argument_error_with_the_argument_named() {
    // SAFETY: A null output pointer is the argument under test.
    let status = unsafe { nurt_info(ptr::null_mut()) };
    assert_eq!(status, NurtStatus::Arg);
    let record = last_error();
    assert_eq!(record["status"], 3);
    assert_eq!(record["code"], "NURT_E_ARG");
    assert_eq!(record["detail"]["function"], "nurt_info");
    assert_eq!(record["detail"]["argument"], "out_json");
}

#[allow(clippy::cognitive_complexity)] // one assertion per function of the C interface, kept in one table
#[test]
fn text_arguments_that_are_not_utf8_are_rejected_by_every_function() {
    let open = Open::new();
    let e = open.engine;
    let bad = not_utf8();
    // SAFETY: Valid handles, NUL-terminated strings and buffers owned by the
    // macro; `bad` is a NUL-terminated string that is not UTF-8.
    unsafe {
        rejected!("nurt_parse", |a, b| nurt_parse(
            e,
            bad,
            NURT_VARIANT_A,
            a,
            b
        ));
        rejected!("nurt_print", |a, _b| nurt_print(e, bad, NURT_VARIANT_A, a));
        rejected!("nurt_correct", |a, _b| nurt_correct(
            e,
            bad,
            NURT_VARIANT_A,
            a
        ));
        rejected!("nurt_check", |a, b| nurt_check(e, bad, a, b));
        rejected!("nurt_run_json", |a, _b| nurt_run_json(e, bad, a));
        rejected!("nurt_page", |a, _b| nurt_page(e, bad, 0, a));
        rejected!("nurt_catalog", |a, _b| nurt_catalog(e, bad, a));
        rejected!("nurt_describe", |a, _b| nurt_describe(e, bad, a));
        rejected!("nurt_help", |a, _b| nurt_help(e, bad, a));
        rejected!("nurt_history", |a, _b| nurt_history(e, bad, a));
        rejected!("nurt_space", |a, _b| nurt_space(e, bad, a));
        rejected!("nurt_schema", |a, _b| nurt_schema(bad, a));
        rejected!("nurt_session_end", |_a, _b| nurt_session_end(e, bad));
        rejected!("nurt_ext_load", |_a, _b| nurt_ext_load(e, bad));
    }
    let record = last_error();
    assert_eq!(record["code"], "NURT_E_ARG");
    assert!(
        record["message"].as_str().unwrap().contains("UTF-8"),
        "{record}"
    );
}

#[test]
fn a_configuration_that_is_not_utf8_or_not_an_object_opens_no_engine() {
    let mut candidates: Vec<*const c_char> = vec![not_utf8()];
    candidates.extend(NOT_OBJECTS.iter().map(|text| text.as_ptr()));
    for config in candidates {
        let mut engine = ptr::NonNull::<Engine>::dangling().as_ptr();
        // SAFETY: A NUL-terminated configuration and a valid output pointer.
        let status = unsafe { nurt_engine_open(config, &raw mut engine) };
        assert_eq!(status, NurtStatus::Arg);
        assert!(engine.is_null());
        assert_eq!(last_error()["detail"]["function"], "nurt_engine_open");
    }
}

#[test]
fn exec_start_clears_its_handle_when_the_request_is_rejected() {
    let open = Open::new();
    let e = open.engine;
    for request in [
        not_utf8(),
        c"[1]".as_ptr(),
        c"not json".as_ptr(),
        ptr::null(),
    ] {
        let mut exec = ptr::NonNull::<Exec>::dangling().as_ptr();
        // SAFETY: A live engine, a request that is invalid or null and a
        // valid output pointer.
        let status = unsafe { nurt_exec_start(e, request, None, ptr::null_mut(), &raw mut exec) };
        assert_eq!(status, NurtStatus::Arg);
        assert!(exec.is_null());
    }
    // SAFETY: A null output pointer is the argument under test.
    let status =
        unsafe { nurt_exec_start(e, REQUEST.as_ptr(), None, ptr::null_mut(), ptr::null_mut()) };
    assert_eq!(status, NurtStatus::Arg);
}

#[allow(clippy::cognitive_complexity)] // one assertion per function of the C interface, kept in one table
#[test]
fn json_arguments_that_are_not_objects_are_rejected() {
    let open = Open::new();
    let e = open.engine;
    for text in NOT_OBJECTS {
        let given = text.as_ptr();
        // SAFETY: Valid handles, NUL-terminated strings and buffers owned by
        // the macro.
        unsafe {
            rejected!("nurt_catalog", |a, _b| nurt_catalog(e, given, a));
            rejected!("nurt_describe", |a, _b| nurt_describe(e, given, a));
            rejected!("nurt_history", |a, _b| nurt_history(e, given, a));
            rejected!("nurt_print", |a, _b| nurt_print(
                e,
                given,
                NURT_VARIANT_A,
                a
            ));
        }
    }
    for text in [c"[1]", c"{", c"\"text\"", c"null", c"not json"] {
        let given = text.as_ptr();
        // SAFETY: As above.
        unsafe {
            rejected!("nurt_check", |a, b| nurt_check(e, given, a, b));
            rejected!("nurt_run_json", |a, _b| nurt_run_json(e, given, a));
        }
    }
    let record = last_error();
    assert_eq!(record["code"], "NURT_E_ARG");
    assert_eq!(record["detail"]["function"], "nurt_run_json");
}

#[test]
fn null_required_text_names_the_argument_in_the_last_error() {
    let open = Open::new();
    let e = open.engine;
    let (mut first, mut second) = (Buf::new(), Buf::new());
    // SAFETY: A live engine, a null program and valid buffers.
    let status = unsafe { nurt_parse(e, ptr::null(), NURT_VARIANT_A, first.ptr(), second.ptr()) };
    assert_eq!(status, NurtStatus::Arg);
    let record = last_error();
    assert_eq!(record["detail"]["function"], "nurt_parse");
    assert_eq!(record["detail"]["argument"], "program");
    // SAFETY: A live engine, a null cursor and a valid buffer.
    let status = unsafe { nurt_page(e, ptr::null(), 0, first.ptr()) };
    assert_eq!(status, NurtStatus::Arg);
    assert_eq!(last_error()["detail"]["argument"], "cursor");
    // SAFETY: A live engine and a null session identifier.
    let status = unsafe { nurt_session_end(e, ptr::null()) };
    assert_eq!(status, NurtStatus::Arg);
    assert_eq!(last_error()["detail"]["argument"], "session");
    // SAFETY: A live engine and a null manifest path.
    let status = unsafe { nurt_ext_load(e, ptr::null()) };
    assert_eq!(status, NurtStatus::Arg);
    assert_eq!(last_error()["detail"]["argument"], "manifest_path");
    // SAFETY: A null schema name and a valid buffer.
    let status = unsafe { nurt_schema(ptr::null(), first.ptr()) };
    assert_eq!(status, NurtStatus::Arg);
    assert_eq!(last_error()["detail"]["argument"], "which");
}

#[test]
fn variants_outside_the_enumeration_are_rejected() {
    let open = Open::new();
    let e = open.engine;
    for variant in [0, 4, -1, c_int::MAX, c_int::MIN] {
        // SAFETY: Valid handles, static strings and buffers owned by the macro.
        unsafe {
            rejected!("nurt_parse", |a, b| nurt_parse(
                e,
                PROGRAM.as_ptr(),
                variant,
                a,
                b
            ));
            rejected!("nurt_print", |a, _b| nurt_print(
                e,
                AST.as_ptr(),
                variant,
                a
            ));
            rejected!("nurt_correct", |a, _b| nurt_correct(
                e,
                PROGRAM.as_ptr(),
                variant,
                a
            ));
        }
        let record = last_error();
        assert_eq!(record["code"], "NURT_E_ARG");
        let message = record["message"].as_str().unwrap();
        assert!(message.contains(&format!("`v` is {variant}")), "{message}");
        assert!(message.contains("NURT_VARIANT_A"), "{message}");
    }
}

#[test]
fn the_version_check_accepts_only_the_same_major_and_a_minor_not_above_the_library() {
    assert_eq!(nurt_abi_version(), NURT_ABI_VERSION);
    assert_eq!(nurt_abi_check(NURT_ABI_VERSION), NurtStatus::Ok);
    let rejected = [
        0,
        NURT_ABI_VERSION - (1 << 16),
        NURT_ABI_VERSION + (1 << 16),
        NURT_ABI_VERSION + 1,
        NURT_ABI_VERSION | 0xFFFF,
        u32::MAX,
    ];
    for version in rejected {
        assert_eq!(nurt_abi_check(version), NurtStatus::Abi, "{version:#x}");
        let record = last_error();
        assert_eq!(record["code"], "NURT_E_ABI");
        assert_eq!(record["detail"]["library"], NURT_ABI_VERSION);
        assert_eq!(record["detail"]["caller"], version);
    }
}

#[test]
fn a_successful_call_keeps_the_last_error_of_the_thread() {
    // SAFETY: A null execution handle is rejected without being read.
    assert_eq!(
        unsafe { nurt_exec_cancel(ptr::null_mut()) },
        NurtStatus::Arg
    );
    let mut info = Buf::new();
    // SAFETY: A valid output buffer.
    assert_eq!(unsafe { nurt_info(info.ptr()) }, NurtStatus::Ok);
    assert_eq!(nurt_abi_check(NURT_ABI_VERSION), NurtStatus::Ok);
    assert_eq!(last_error()["detail"]["function"], "nurt_exec_cancel");
}

#[test]
fn null_handles_are_accepted_where_the_interface_allows_them() {
    // SAFETY: The interface accepts null for these functions.
    unsafe {
        nurt_engine_close(ptr::null_mut());
        nurt_exec_free(ptr::null_mut());
        nurt_buf_free(ptr::null_mut());
        nurt_set_log_callback(None, ptr::null_mut());
    }
    // SAFETY: A null execution handle is rejected without being read.
    assert_eq!(
        unsafe { nurt_exec_cancel(ptr::null_mut()) },
        NurtStatus::Arg
    );
    let mut empty = Buf::new();
    // SAFETY: A zero buffer is released twice, which is allowed.
    unsafe {
        nurt_buf_free(empty.ptr());
        nurt_buf_free(empty.ptr());
    }
    assert!(empty.0.is_empty());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
