// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Exported functions of the C interface, in the order of `nurt.h`.
//!
//! Every function with a status runs its body through the panic boundary
//! and empties its output buffers before it validates the arguments, so a
//! failing call leaves them empty. Errors of the language never change the
//! status: they are carried by the JSON result.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use nurt_contracts::envelope::Request;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::schema;
use serde_json::{Value, json};

use crate::backend::Function;
use crate::boundary::{self, guard, guard_void};
use crate::buffer::NurtBuf;
use crate::engine::Engine;
use crate::error::{self, CallError, NurtStatus};
use crate::exec::{Callback, EventCallback, Exec};
use crate::info;
use crate::log::{self, LogCallback};

/// Major version of the interface (`NURT_ABI_MAJOR`).
pub const NURT_ABI_MAJOR: u32 = 1;

/// Minor version of the interface (`NURT_ABI_MINOR`).
pub const NURT_ABI_MINOR: u32 = 0;

const MAJOR_SHIFT: u32 = 16;
const MINOR_MASK: u32 = 0xFFFF;

/// Version of the interface in the form of `nurt_abi_version`
/// (`NURT_ABI_VERSION`).
pub const NURT_ABI_VERSION: u32 = (NURT_ABI_MAJOR << MAJOR_SHIFT) | NURT_ABI_MINOR;

/// Timeout of `nurt_exec_wait` that waits without limit
/// (`NURT_WAIT_FOREVER`).
pub const NURT_WAIT_FOREVER: u32 = u32::MAX;

/// Word notation (`NURT_VARIANT_A`).
pub const NURT_VARIANT_A: c_int = 1;

/// Call notation (`NURT_VARIANT_B`).
pub const NURT_VARIANT_B: c_int = 2;

/// Named notation (`NURT_VARIANT_C`).
pub const NURT_VARIANT_C: c_int = 3;

const VERSION: &CStr =
    match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
        Ok(version) => version,
        Err(_) => c"",
    };

fn variant(value: c_int) -> Result<SyntaxVariant, CallError> {
    match value {
        NURT_VARIANT_A => Ok(SyntaxVariant::A),
        NURT_VARIANT_B => Ok(SyntaxVariant::B),
        NURT_VARIANT_C => Ok(SyntaxVariant::C),
        _ => Err(CallError::arg(format!(
            "`v` is {value}; pass NURT_VARIANT_A (1), NURT_VARIANT_B (2) or NURT_VARIANT_C (3)"
        ))),
    }
}

fn request(text: &str) -> Result<Request, CallError> {
    serde_json::from_str(text).map_err(|error| {
        CallError::arg(format!(
            "`request_json` is not a request envelope nurt-request/1: {error}"
        ))
    })
}

fn request_value(request: &Request) -> Result<Value, CallError> {
    serde_json::to_value(request).map_err(|error| {
        CallError::internal(format!("cannot serialise the request envelope: {error}"))
    })
}

/// Moves the results of an operation into the output buffers.
fn store<const N: usize>(outputs: Vec<Vec<u8>>, targets: [&mut NurtBuf; N]) -> NurtStatus {
    for (target, bytes) in targets.into_iter().zip(outputs) {
        *target = NurtBuf::from_bytes(bytes);
    }
    NurtStatus::Ok
}

/// Runs an operation with one output buffer on a live engine.
///
/// # Safety
///
/// `e` is null or a live engine handle; `out` is null or a valid `nurt_buf`.
unsafe fn single(
    name: &'static str,
    e: *mut Engine,
    out: *mut NurtBuf,
    out_name: &'static str,
    function: Function,
    input: impl FnOnce() -> Result<Value, CallError>,
) -> NurtStatus {
    guard(name, || {
        // SAFETY: The pointer `out` is null or a valid buffer by the contract of `single`.
        let out = unsafe { boundary::output(out, out_name)? };
        // SAFETY: The pointer `e` is null or a live handle by the contract of `single`.
        let engine = unsafe { boundary::engine(e)? };
        let outputs = engine.call(function, &input()?)?;
        Ok(store(outputs, [out]))
    })
}

/// Runs an operation without output buffers on a live engine.
///
/// # Safety
///
/// `e` is null or a live engine handle; `arg` is null or a NUL-terminated
/// string valid for the call.
unsafe fn without_output(
    name: &'static str,
    e: *mut Engine,
    arg: *const c_char,
    arg_name: &'static str,
    function: Function,
) -> NurtStatus {
    guard(name, || {
        // SAFETY: The pointer `e` is null or a live handle by the contract of this function.
        let engine = unsafe { boundary::engine(e)? };
        // SAFETY: The pointer `arg` is null or a valid string by the contract of this function.
        let arg = unsafe { boundary::text(arg, arg_name)? };
        let outputs = engine.call(function, &Value::from(arg))?;
        Ok(store(outputs, []))
    })
}

/// Version of the interface implemented by the library:
/// `(NURT_ABI_MAJOR << 16) | NURT_ABI_MINOR`.
#[unsafe(no_mangle)]
pub extern "C" fn nurt_abi_version() -> u32 {
    NURT_ABI_VERSION
}

/// Checks that the library implements the interface of the caller's
/// header.
///
/// Returns [`NurtStatus::Ok`] when the major versions are equal and the
/// minor version of the library is not lower, otherwise
/// [`NurtStatus::Abi`].
#[unsafe(no_mangle)]
pub extern "C" fn nurt_abi_check(header_version: u32) -> NurtStatus {
    guard("nurt_abi_check", || {
        let major = header_version >> MAJOR_SHIFT;
        let minor = header_version & MINOR_MASK;
        if major == NURT_ABI_MAJOR && NURT_ABI_MINOR.checked_sub(minor).is_some() {
            Ok(NurtStatus::Ok)
        } else {
            Err(CallError::abi(format!(
                "the caller uses interface {major}.{minor} and the library implements \
                 {NURT_ABI_MAJOR}.{NURT_ABI_MINOR}; use a library with major version {major} \
                 and minor version {minor} or later"
            ))
            .with_detail("library", NURT_ABI_VERSION)
            .with_detail("caller", header_version))
        }
    })
}

/// Version of the library, for example `0.1.0`, as a static string that is
/// never released.
#[unsafe(no_mangle)]
pub extern "C" fn nurt_version() -> *const c_char {
    VERSION.as_ptr()
}

/// Information about the library as JSON: version, interface version,
/// format versions, published schemas, revision of the built-in command
/// catalog, features, target and build profile.
///
/// # Safety
///
/// `out_json` is null or points to a `nurt_buf` valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_info(out_json: *mut NurtBuf) -> NurtStatus {
    guard("nurt_info", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let out = unsafe { boundary::output(out_json, "out_json")? };
        *out = NurtBuf::from_bytes(info::to_json());
        Ok(NurtStatus::Ok)
    })
}

/// Opens an engine with the configuration `config_json` (null or an object
/// `nurt-config/1`).
///
/// [`NurtStatus::Arg`]: `out` is null or the configuration is not valid.
/// [`NurtStatus::Io`]: a file of the instance cannot be read.
///
/// # Safety
///
/// `config_json` is null or a NUL-terminated string valid for the call;
/// `out` is null or valid for writing a pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_engine_open(
    config_json: *const c_char,
    out: *mut *mut Engine,
) -> NurtStatus {
    guard("nurt_engine_open", || {
        if out.is_null() {
            return Err(CallError::arg("`out` must not be null"));
        }
        // SAFETY: The pointer `out` is not null and valid for writes by the contract.
        unsafe { out.write(ptr::null_mut()) };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let config = unsafe { boundary::optional_text(config_json, "config_json")? };
        let engine = Engine::open(boundary::optional_object(config, "config_json")?)?;
        // SAFETY: As above; the handle is released by `nurt_engine_close`.
        unsafe { out.write(Box::into_raw(Box::new(engine))) };
        Ok(NurtStatus::Ok)
    })
}

/// Closes an engine; running executions are cancelled and waited for. Null
/// is accepted.
///
/// # Safety
///
/// `e` is null or a handle returned by [`nurt_engine_open`] that is not
/// used after the call. The function is not called from an event callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_engine_close(e: *mut Engine) {
    guard_void("nurt_engine_close", || {
        if !e.is_null() {
            // SAFETY: A handle created by `Box::into_raw` in
            // `nurt_engine_open`, released exactly once by the contract.
            let engine = unsafe { Box::from_raw(e) };
            engine.close();
        }
    });
}

/// Effective configuration of the engine as JSON (`nurt-config/1`).
///
/// # Safety
///
/// `e` is null or a live engine handle; `out_json` is null or a valid
/// `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_engine_config_get(
    e: *mut Engine,
    out_json: *mut NurtBuf,
) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        single(
            "nurt_engine_config_get",
            e,
            out_json,
            "out_json",
            Function::EngineConfigGet,
            || Ok(Value::Null),
        )
    }
}

/// Parses a program in the notation `v` into a syntax tree `nurt-ast/1`;
/// diagnostics are written to `out_diag_json`.
///
/// # Safety
///
/// `e` is null or a live engine handle; `program` is null or a
/// NUL-terminated string valid for the call; the output pointers are null
/// or distinct valid `nurt_buf` structures.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_parse(
    e: *mut Engine,
    program: *const c_char,
    v: c_int,
    out_ast_json: *mut NurtBuf,
    out_diag_json: *mut NurtBuf,
) -> NurtStatus {
    guard("nurt_parse", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let (ast, diag) = unsafe {
            (
                boundary::output(out_ast_json, "out_ast_json"),
                boundary::output(out_diag_json, "out_diag_json"),
            )
        };
        let (ast, diag) = (ast?, diag?);
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let engine = unsafe { boundary::engine(e)? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let program = unsafe { boundary::text(program, "program")? };
        let input = json!({ "program": program, "variant": variant(v)? });
        let outputs = engine.call(Function::Parse, &input)?;
        Ok(store(outputs, [ast, diag]))
    })
}

/// Prints a syntax tree `nurt-ast/1` in the canonical text of the notation
/// `v`.
///
/// # Safety
///
/// `e` is null or a live engine handle; `ast_json` is null or a
/// NUL-terminated string valid for the call; `out_text` is null or a valid
/// `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_print(
    e: *mut Engine,
    ast_json: *const c_char,
    v: c_int,
    out_text: *mut NurtBuf,
) -> NurtStatus {
    let input = || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let ast = unsafe { boundary::text(ast_json, "ast_json")? };
        let ast = boundary::optional_object(Some(ast), "ast_json")?;
        Ok(json!({ "ast": ast, "variant": variant(v)? }))
    };
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        single(
            "nurt_print",
            e,
            out_text,
            "out_text",
            Function::Print,
            input,
        )
    }
}

/// Corrects the written form of a program without running it.
///
/// # Safety
///
/// `e` is null or a live engine handle; `program` is null or a
/// NUL-terminated string valid for the call; `out_result_json` is null or a
/// valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_correct(
    e: *mut Engine,
    program: *const c_char,
    v: c_int,
    out_result_json: *mut NurtBuf,
) -> NurtStatus {
    let input = || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let program = unsafe { boundary::text(program, "program")? };
        Ok(json!({ "program": program, "variant": variant(v)? }))
    };
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        single(
            "nurt_correct",
            e,
            out_result_json,
            "out_result_json",
            Function::Correct,
            input,
        )
    }
}

/// Binds a request envelope `nurt-request/1` and classifies its effect
/// without running it.
///
/// # Safety
///
/// `e` is null or a live engine handle; `request_json` is null or a
/// NUL-terminated string valid for the call; the output pointers are null
/// or distinct valid `nurt_buf` structures.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_check(
    e: *mut Engine,
    request_json: *const c_char,
    out_bound_json: *mut NurtBuf,
    out_diag_json: *mut NurtBuf,
) -> NurtStatus {
    guard("nurt_check", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let (bound, diag) = unsafe {
            (
                boundary::output(out_bound_json, "out_bound_json"),
                boundary::output(out_diag_json, "out_diag_json"),
            )
        };
        let (bound, diag) = (bound?, diag?);
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let engine = unsafe { boundary::engine(e)? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let request = request(unsafe { boundary::text(request_json, "request_json")? })?;
        let outputs = engine.call(Function::Check, &request_value(&request)?)?;
        Ok(store(outputs, [bound, diag]))
    })
}

/// Runs a request envelope `nurt-request/1` and waits for the response.
///
/// An envelope that matches the schema but breaks a rule of the envelope
/// (for example two inputs) gives [`NurtStatus::Ok`] and a response with the
/// language error `E_ARG`.
///
/// # Safety
///
/// `e` is null or a live engine handle; `request_json` is null or a
/// NUL-terminated string valid for the call; `out_response_json` is null or
/// a valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_run_json(
    e: *mut Engine,
    request_json: *const c_char,
    out_response_json: *mut NurtBuf,
) -> NurtStatus {
    guard("nurt_run_json", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let out = unsafe { boundary::output(out_response_json, "out_response_json")? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let engine = unsafe { boundary::engine(e)? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let request = request(unsafe { boundary::text(request_json, "request_json")? })?;
        *out = NurtBuf::from_bytes(engine.run(&request)?);
        Ok(NurtStatus::Ok)
    })
}

/// Starts a request envelope `nurt-request/1` asynchronously; `cb` may be
/// null. The handle is released with [`nurt_exec_free`].
///
/// # Safety
///
/// `e` is null or a live engine handle; `request_json` is null or a
/// NUL-terminated string valid for the call; `out` is null or valid for
/// writing a pointer; `cb` and `user` may be used from a library thread
/// until the event `exec.finished` has been delivered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_exec_start(
    e: *mut Engine,
    request_json: *const c_char,
    cb: EventCallback,
    user: *mut c_void,
    out: *mut *mut Exec,
) -> NurtStatus {
    guard("nurt_exec_start", || {
        if out.is_null() {
            return Err(CallError::arg("`out` must not be null"));
        }
        // SAFETY: The pointer `out` is not null and valid for writes by the contract.
        unsafe { out.write(ptr::null_mut()) };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let engine = unsafe { boundary::engine(e)? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let request = request(unsafe { boundary::text(request_json, "request_json")? })?;
        let exec = engine.start(&request, Callback::new(cb, user))?;
        // SAFETY: As above; the handle is released by `nurt_exec_free`.
        unsafe { out.write(Box::into_raw(Box::new(exec))) };
        Ok(NurtStatus::Ok)
    })
}

/// Waits for an execution for no longer than `timeout_ms` milliseconds; 0 checks
/// without waiting and [`NURT_WAIT_FOREVER`] waits without limit.
///
/// Returns [`NurtStatus::Ok`] with the response when the execution has
/// finished, [`NurtStatus::Timeout`] with an empty buffer while it runs and
/// [`NurtStatus::Cancelled`] with the response of the cancelled execution.
/// A completed execution returns the same result on every call.
///
/// # Safety
///
/// `x` is null or a live execution handle; `out_response_json` is null or a
/// valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_exec_wait(
    x: *mut Exec,
    timeout_ms: u32,
    out_response_json: *mut NurtBuf,
) -> NurtStatus {
    guard("nurt_exec_wait", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let out = unsafe { boundary::output(out_response_json, "out_response_json")? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let exec = unsafe { boundary::exec(x)? };
        let Some(outcome) = exec.wait(timeout_ms) else {
            return Ok(NurtStatus::Timeout);
        };
        match outcome.error {
            Some(error) => Err(error),
            None => {
                *out = NurtBuf::from_bytes(outcome.response);
                Ok(outcome.status)
            }
        }
    })
}

/// Requests cancellation of an execution. Idempotent; returns
/// [`NurtStatus::Ok`] also when the execution has already finished, whose
/// result then stays unchanged.
///
/// # Safety
///
/// `x` is null or a live execution handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_exec_cancel(x: *mut Exec) -> NurtStatus {
    guard("nurt_exec_cancel", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let exec = unsafe { boundary::exec(x)? };
        exec.cancel();
        Ok(NurtStatus::Ok)
    })
}

/// Releases an execution handle; a running execution is cancelled and
/// waited for. Null is accepted.
///
/// # Safety
///
/// `x` is null or a handle returned by [`nurt_exec_start`] that is not used
/// after the call. The function is not called from the event callback of
/// the same execution.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_exec_free(x: *mut Exec) {
    guard_void("nurt_exec_free", || {
        if !x.is_null() {
            // SAFETY: A handle created by `Box::into_raw` in
            // `nurt_exec_start`, released exactly once by the contract.
            drop(unsafe { Box::from_raw(x) });
        }
    });
}

/// Returns the next page of a paged result; `limit` 0 uses the limit of the
/// original request.
///
/// # Safety
///
/// `e` is null or a live engine handle; `cursor` is null or a
/// NUL-terminated string valid for the call; `out_response_json` is null or
/// a valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_page(
    e: *mut Engine,
    cursor: *const c_char,
    limit: u32,
    out_response_json: *mut NurtBuf,
) -> NurtStatus {
    let input = || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let cursor = unsafe { boundary::text(cursor, "cursor")? };
        Ok(json!({ "cursor": cursor, "limit": limit }))
    };
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        single(
            "nurt_page",
            e,
            out_response_json,
            "out_response_json",
            Function::Page,
            input,
        )
    }
}

/// Runs an operation whose input is a JSON object argument or null.
///
/// # Safety
///
/// As for [`single`]; `arg` is null or a NUL-terminated string valid for the
/// call.
unsafe fn with_object(
    name: &'static str,
    e: *mut Engine,
    arg: *const c_char,
    arg_name: &'static str,
    out: *mut NurtBuf,
    out_name: &'static str,
    function: Function,
) -> NurtStatus {
    let input = || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let arg = unsafe { boundary::optional_text(arg, arg_name)? };
        boundary::optional_object(arg, arg_name)
    };
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe { single(name, e, out, out_name, function, input) }
}

/// Runs an operation whose input is a text argument or null.
///
/// # Safety
///
/// As for [`with_object`].
unsafe fn with_optional_text(
    name: &'static str,
    e: *mut Engine,
    arg: *const c_char,
    arg_name: &'static str,
    out: *mut NurtBuf,
    out_name: &'static str,
    function: Function,
) -> NurtStatus {
    let input = || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let arg = unsafe { boundary::optional_text(arg, arg_name)? };
        Ok(arg.map_or(Value::Null, Value::from))
    };
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe { single(name, e, out, out_name, function, input) }
}

/// Command catalog as JSON; `what_json` is null for the whole catalog or an
/// object selecting a group or a command.
///
/// # Safety
///
/// `e` is null or a live engine handle; `what_json` is null or a
/// NUL-terminated string valid for the call; `out_json` is null or a valid
/// `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_catalog(
    e: *mut Engine,
    what_json: *const c_char,
    out_json: *mut NurtBuf,
) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        with_object(
            "nurt_catalog",
            e,
            what_json,
            "what_json",
            out_json,
            "out_json",
            Function::Catalog,
        )
    }
}

/// Description of the language for tool integrations, as text; `what_json`
/// selects the layer, the group and the language.
///
/// # Safety
///
/// As for [`nurt_catalog`], with the output `out_text`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_describe(
    e: *mut Engine,
    what_json: *const c_char,
    out_text: *mut NurtBuf,
) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        with_object(
            "nurt_describe",
            e,
            what_json,
            "what_json",
            out_text,
            "out_text",
            Function::Describe,
        )
    }
}

/// Built-in help on a topic, as text; null returns the index of topics.
///
/// # Safety
///
/// `e` is null or a live engine handle; `topic` is null or a NUL-terminated
/// string valid for the call; `out_text` is null or a valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_help(
    e: *mut Engine,
    topic: *const c_char,
    out_text: *mut NurtBuf,
) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        with_optional_text(
            "nurt_help",
            e,
            topic,
            "topic",
            out_text,
            "out_text",
            Function::Help,
        )
    }
}

/// Published JSON schema by name; an unknown name returns
/// [`NurtStatus::Arg`]. Needs no engine.
///
/// # Safety
///
/// `which` is null or a NUL-terminated string valid for the call; `out_json`
/// is null or a valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_schema(which: *const c_char, out_json: *mut NurtBuf) -> NurtStatus {
    guard("nurt_schema", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let out = unsafe { boundary::output(out_json, "out_json")? };
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let which = unsafe { boundary::text(which, "which")? };
        let Some(content) = schema::by_name(which) else {
            let known: Vec<&str> = schema::ALL.iter().map(|(name, _)| *name).collect();
            return Err(CallError::arg(format!(
                "unknown schema `{which}`; pass one of: {}",
                known.join(", ")
            ))
            .with_detail("expected", known));
        };
        *out = NurtBuf::from_bytes(content.as_bytes().to_vec());
        Ok(NurtStatus::Ok)
    })
}

/// Journal history as JSON; `query_json` is null or an object with the
/// parameters of the command `history`.
///
/// # Safety
///
/// As for [`nurt_catalog`], with the argument `query_json`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_history(
    e: *mut Engine,
    query_json: *const c_char,
    out_json: *mut NurtBuf,
) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        with_object(
            "nurt_history",
            e,
            query_json,
            "query_json",
            out_json,
            "out_json",
            Function::History,
        )
    }
}

/// Free space and journal usage of a volume as JSON; `volume` is a path on
/// the volume, or null for all volumes.
///
/// # Safety
///
/// As for [`nurt_help`], with the argument `volume` and the output
/// `out_json`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_space(
    e: *mut Engine,
    volume: *const c_char,
    out_json: *mut NurtBuf,
) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        with_optional_text(
            "nurt_space",
            e,
            volume,
            "volume",
            out_json,
            "out_json",
            Function::Space,
        )
    }
}

/// Ends a session; ending an unknown session succeeds.
///
/// # Safety
///
/// `e` is null or a live engine handle; `session` is null or a
/// NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_session_end(e: *mut Engine, session: *const c_char) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        without_output(
            "nurt_session_end",
            e,
            session,
            "session",
            Function::SessionEnd,
        )
    }
}

/// Loads an extension from its manifest `nurt-ext.toml`.
///
/// [`NurtStatus::Abi`]: the extension needs another major version of the
/// extension interface. [`NurtStatus::Io`]: the manifest or the module
/// cannot be read. [`NurtStatus::Arg`]: the manifest is not valid.
///
/// # Safety
///
/// `e` is null or a live engine handle; `manifest_path` is null or a
/// NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_ext_load(e: *mut Engine, manifest_path: *const c_char) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        without_output(
            "nurt_ext_load",
            e,
            manifest_path,
            "manifest_path",
            Function::ExtLoad,
        )
    }
}

/// Loaded extensions as JSON.
///
/// # Safety
///
/// `e` is null or a live engine handle; `out_json` is null or a valid
/// `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_ext_list(e: *mut Engine, out_json: *mut NurtBuf) -> NurtStatus {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    unsafe {
        single(
            "nurt_ext_list",
            e,
            out_json,
            "out_json",
            Function::ExtList,
            || Ok(Value::Null),
        )
    }
}

/// Releases a buffer returned by the library and sets it to zero.
/// Idempotent; null and zero buffers are accepted.
///
/// # Safety
///
/// `b` is null or points to a `nurt_buf` that is zero or was filled by this
/// library and not changed by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_buf_free(b: *mut NurtBuf) {
    guard_void("nurt_buf_free", || {
        // SAFETY: The pointer `b` is null or a valid buffer by the contract.
        if let Some(buffer) = unsafe { b.as_mut() } {
            buffer.release();
        }
    });
}

/// Last failure of the calling thread as JSON:
/// `{"status": 3, "code": "NURT_E_ARG", "message": "...", "detail": {...}}`.
///
/// The record is kept per thread and replaced by the next failing call of
/// the thread. `e` is ignored and may be null. Without a recorded failure
/// the buffer is empty.
///
/// # Safety
///
/// `out_json` is null or a valid `nurt_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_last_error(e: *mut Engine, out_json: *mut NurtBuf) -> NurtStatus {
    let _ = e;
    guard("nurt_last_error", || {
        // SAFETY: The caller upholds the contract stated under `# Safety`.
        let out = unsafe { boundary::output(out_json, "out_json")? };
        if let Some(record) = error::last() {
            *out = NurtBuf::from_bytes(record);
        }
        Ok(NurtStatus::Ok)
    })
}

/// Sets the receiver of diagnostic log messages for the whole process; null
/// stops logging.
///
/// # Safety
///
/// `cb` is null or a function that may be called from any thread with
/// `user` until it is replaced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nurt_set_log_callback(cb: LogCallback, user: *mut c_void) {
    guard_void("nurt_set_log_callback", || log::set(cb, user));
}
