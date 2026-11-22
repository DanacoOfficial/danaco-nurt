// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Panic boundary and conversion of the arguments of exported functions.

use std::any::Any;
use std::ffi::{CStr, c_char};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::Value;

use crate::buffer::NurtBuf;
use crate::engine::Engine;
use crate::error::{self, CallError, NurtStatus};
use crate::exec::Exec;
use crate::log;

/// Runs the body of the exported function `function`.
///
/// A panic of the body is caught and becomes [`NurtStatus::Internal`]. A
/// failure is recorded for `nurt_last_error` of the calling thread; the
/// statuses returned by a successful body (`NURT_OK`, `NURT_E_TIMEOUT`,
/// `NURT_E_CANCELLED`) are not failures and leave the record unchanged.
pub(crate) fn guard<F>(function: &'static str, body: F) -> NurtStatus
where
    F: FnOnce() -> Result<NurtStatus, CallError>,
{
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        #[cfg(feature = "stub")]
        crate::stub::check_injected_panic(function);
        body()
    }));
    let error = match outcome {
        Ok(Ok(status)) => return status,
        Ok(Err(error)) => error,
        Err(payload) => caught_panic(function, payload.as_ref()),
    };
    error::record(function, &error);
    error.status()
}

/// Runs the body of an exported function without a status.
///
/// A panic is caught, logged and recorded for `nurt_last_error`.
pub(crate) fn guard_void<F>(function: &'static str, body: F)
where
    F: FnOnce(),
{
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        #[cfg(feature = "stub")]
        crate::stub::check_injected_panic(function);
        body();
    }));
    if let Err(payload) = outcome {
        let error = caught_panic(function, payload.as_ref());
        error::record(function, &error);
    }
}

/// Logs a caught panic and converts it to an internal failure.
pub(crate) fn caught_panic(context: &str, payload: &(dyn Any + Send)) -> CallError {
    let message = format!("internal failure in {context}: {}", panic_message(payload));
    log::error(&message);
    CallError::internal(message)
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "panic without a message"
    }
}

/// Locks a mutex; a lock poisoned by a caught panic is still usable because
/// every critical section leaves the data consistent.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Text argument that must not be null.
///
/// # Safety
///
/// `ptr` is null or points to a NUL-terminated string that stays valid and
/// unchanged for `'a`.
pub(crate) unsafe fn text<'a>(
    ptr: *const c_char,
    name: &'static str,
) -> Result<&'a str, CallError> {
    // SAFETY: The caller upholds the contract stated under `# Safety`.
    match unsafe { optional_text(ptr, name)? } {
        Some(text) => Ok(text),
        None => {
            Err(CallError::arg(format!("`{name}` must not be null")).with_detail("argument", name))
        }
    }
}

/// Text argument that may be null.
///
/// # Safety
///
/// As for [`text`].
pub(crate) unsafe fn optional_text<'a>(
    ptr: *const c_char,
    name: &'static str,
) -> Result<Option<&'a str>, CallError> {
    if ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: The pointer `ptr` is not null and, by the contract of this function, points
    // to a NUL-terminated string valid for `'a`.
    let text = unsafe { CStr::from_ptr(ptr) };
    text.to_str().map(Some).map_err(|_| {
        CallError::arg(format!("`{name}` is not valid UTF-8; pass UTF-8 text"))
            .with_detail("argument", name)
    })
}

/// JSON argument that is null or an object; null gives [`Value::Null`].
pub(crate) fn optional_object(text: Option<&str>, name: &'static str) -> Result<Value, CallError> {
    let Some(text) = text else {
        return Ok(Value::Null);
    };
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => Ok(value),
        Ok(_) => {
            Err(CallError::arg(format!("`{name}` must be a JSON object"))
                .with_detail("argument", name))
        }
        Err(error) => Err(
            CallError::arg(format!("`{name}` is not valid JSON: {error}"))
                .with_detail("argument", name),
        ),
    }
}

/// Output buffer: emptied when the pointer is not null, so that a failing
/// call leaves it empty.
///
/// # Safety
///
/// `ptr` is null or points to a `nurt_buf` valid for reads and writes for
/// `'a` and not aliased by another argument of the call.
pub(crate) unsafe fn output<'a>(
    ptr: *mut NurtBuf,
    name: &'static str,
) -> Result<&'a mut NurtBuf, CallError> {
    if ptr.is_null() {
        return Err(
            CallError::arg(format!("`{name}` must not be null")).with_detail("argument", name)
        );
    }
    // SAFETY: Not null and valid for writes by the contract of this function;
    // the previous content is overwritten without being read, as the
    // interface specifies.
    unsafe { ptr.write(NurtBuf::EMPTY) };
    // SAFETY: As above; the buffer was initialised by the write.
    Ok(unsafe { &mut *ptr })
}

/// Engine handle that must not be null and belongs to this process.
///
/// # Safety
///
/// `ptr` is null or a handle returned by `nurt_engine_open` and not yet
/// passed to `nurt_engine_close`.
pub(crate) unsafe fn engine<'a>(ptr: *mut Engine) -> Result<&'a Engine, CallError> {
    if ptr.is_null() {
        return Err(CallError::arg(
            "`e` must not be null; open an engine with nurt_engine_open",
        ));
    }
    // SAFETY: A live handle by the contract of this function; the engine is
    // only read through shared references.
    let engine = unsafe { &*ptr };
    engine.check_process()?;
    Ok(engine)
}

/// Execution handle that must not be null.
///
/// # Safety
///
/// `ptr` is null or a handle returned by `nurt_exec_start` and not yet
/// passed to `nurt_exec_free`.
pub(crate) unsafe fn exec<'a>(ptr: *mut Exec) -> Result<&'a Exec, CallError> {
    if ptr.is_null() {
        return Err(CallError::arg(
            "`x` must not be null; start an execution with nurt_exec_start",
        ));
    }
    // SAFETY: A live handle by the contract of this function.
    Ok(unsafe { &*ptr })
}

#[cfg(test)]
mod tests {
    use std::ffi::c_char;
    use std::ptr;

    use super::{guard, optional_object, optional_text, text};
    use crate::error::{CallError, NurtStatus, last};

    #[test]
    fn panic_becomes_internal_status_with_record() {
        let status = guard("nurt_test", || -> Result<NurtStatus, CallError> {
            std::panic::panic_any(String::from("broken invariant"))
        });
        assert_eq!(status, NurtStatus::Internal);
        let json: serde_json::Value = serde_json::from_slice(&last().unwrap()).unwrap();
        assert_eq!(json["code"], "NURT_E_INTERNAL");
        assert!(
            json["message"]
                .as_str()
                .unwrap()
                .contains("broken invariant")
        );
    }

    #[test]
    fn non_failure_statuses_are_not_recorded() {
        let status = guard("nurt_test", || Ok(NurtStatus::Timeout));
        assert_eq!(status, NurtStatus::Timeout);
        assert!(last().is_none());
    }

    #[test]
    fn text_arguments_reject_null_and_invalid_utf8() {
        // SAFETY: Null is accepted by the contract.
        let missing = unsafe { text(ptr::null(), "program") };
        assert_eq!(missing.unwrap_err().status(), NurtStatus::Arg);
        let invalid = [0xffu8, 0];
        // SAFETY: The array `invalid` is NUL-terminated and outlives the call.
        let result = unsafe { optional_text(invalid.as_ptr().cast::<c_char>(), "program") };
        assert_eq!(result.unwrap_err().status(), NurtStatus::Arg);
    }

    #[test]
    fn object_arguments_reject_other_json() {
        assert_eq!(
            optional_object(None, "what_json").unwrap(),
            serde_json::Value::Null
        );
        assert!(optional_object(Some("{\"layer\":\"core\"}"), "what_json").is_ok());
        assert_eq!(
            optional_object(Some("[1]"), "what_json")
                .unwrap_err()
                .status(),
            NurtStatus::Arg
        );
        assert_eq!(
            optional_object(Some("{"), "what_json")
                .unwrap_err()
                .status(),
            NurtStatus::Arg
        );
    }
}
