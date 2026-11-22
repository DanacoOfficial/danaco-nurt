// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process-wide receiver of diagnostic log messages.

use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::{PoisonError, RwLock};

/// Receiver of diagnostic log messages (`nurt_log_cb`): the user pointer,
/// the level (`NURT_LOG_ERROR` to `NURT_LOG_TRACE`) and the message, valid
/// only during the call.
pub type LogCallback =
    Option<unsafe extern "C" fn(user: *mut c_void, level: c_int, msg: *const c_char)>;

/// Level `NURT_LOG_ERROR` of `nurt_log_level`.
pub const NURT_LOG_ERROR: c_int = 1;

#[derive(Clone, Copy)]
struct Receiver {
    function: unsafe extern "C" fn(*mut c_void, c_int, *const c_char),
    user: *mut c_void,
}

// SAFETY: The function `nurt_set_log_callback` documents that the receiver is called from
// any thread; by registering `user` with it the caller allows its use from
// every thread of the library.
unsafe impl Send for Receiver {}
// SAFETY: The receiver is only copied out of the lock and called; see `Send`.
unsafe impl Sync for Receiver {}

static RECEIVER: RwLock<Option<Receiver>> = RwLock::new(None);

/// Sets or, with `None`, removes the receiver.
pub(crate) fn set(callback: LogCallback, user: *mut c_void) {
    let receiver = callback.map(|function| Receiver { function, user });
    *RECEIVER.write().unwrap_or_else(PoisonError::into_inner) = receiver;
}

/// Sends an error message to the receiver, when one is set.
pub(crate) fn error(message: &str) {
    let receiver = *RECEIVER.read().unwrap_or_else(PoisonError::into_inner);
    let Some(receiver) = receiver else {
        return;
    };
    let Ok(text) = CString::new(message.replace('\0', " ")) else {
        return;
    };
    // SAFETY: The callback `function` and its `user` pointer were registered together through
    // `nurt_set_log_callback`; `text` is NUL-terminated and outlives the call.
    // The lock is not held, so the receiver may replace itself.
    unsafe { (receiver.function)(receiver.user, NURT_LOG_ERROR, text.as_ptr()) };
}
