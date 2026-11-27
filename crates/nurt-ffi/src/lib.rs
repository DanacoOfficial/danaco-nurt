// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! C ABI of the Danaco Nurt library.
//!
//! The crate exports the functions declared in `include/nurt.h`; the list of
//! exported symbols is kept in `nurt.symbols`. Complex values cross the
//! boundary as UTF-8 JSON, memory returned by the library is held in a
//! [`NurtBuf`] and released with [`nurt_buf_free`], and handles
//! ([`Engine`], [`Exec`]) are opaque to the caller.
//!
//! Every exported function runs inside a panic boundary: a panic is caught,
//! logged through the callback set with [`nurt_set_log_callback`] and
//! reported as [`NurtStatus::Internal`]; it never unwinds into the caller.
//! The boundary relies on unwinding, so the library is built with
//! `panic = "unwind"`. Details of the last failure of the calling thread are
//! returned by [`nurt_last_error`].
//!
//! An engine handle serves the engine of `nurt-core`, opened with the clock
//! and the host of the process. With the cargo feature `stub` the engine is
//! replaced by a test double that answers from a directory of recorded cases
//! (module `stub`).
//!
//! # Examples
//!
//! ```
//! use nurt_ffi::{NURT_ABI_VERSION, NurtBuf, NurtStatus, nurt_abi_check, nurt_buf_free, nurt_info};
//!
//! assert_eq!(nurt_abi_check(NURT_ABI_VERSION), NurtStatus::Ok);
//!
//! let mut info = NurtBuf::EMPTY;
//! // SAFETY: The buffer `info` is a valid, empty buffer owned by this frame.
//! let status = unsafe { nurt_info(&mut info) };
//! assert_eq!(status, NurtStatus::Ok);
//! let json: serde_json::Value = serde_json::from_slice(info.as_bytes()).unwrap();
//! assert_eq!(json["abi"]["major"], 1);
//! // SAFETY: The buffer `info` was filled by the library and is released once.
//! unsafe { nurt_buf_free(&mut info) };
//! ```

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(all(test, windows), allow(dead_code, unused_imports))]
#![deny(missing_docs)]
#![allow(
    unsafe_code,
    reason = "the C ABI boundary dereferences pointers passed by the caller; this overrides the workspace `deny`"
)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod abi;
mod backend;
mod boundary;
mod buffer;
#[cfg(feature = "e2e")]
pub mod e2e;
mod engine;
#[cfg_attr(
    feature = "stub",
    allow(
        dead_code,
        reason = "the test double replaces the engine; the unit tests still use it"
    )
)]
mod engine_backend;
mod error;
mod exec;
mod info;
mod log;
#[cfg(feature = "stub")]
mod response;
#[cfg(feature = "stub")]
pub mod stub;
#[cfg_attr(
    feature = "stub",
    allow(
        dead_code,
        reason = "the test double uses only the wall time of the system"
    )
)]
mod system;

pub use abi::*;
pub use backend::{Function, OutputKind};
pub use buffer::NurtBuf;
pub use engine::Engine;
pub use error::NurtStatus;
pub use exec::{EventCallback, Exec};
pub use log::{LogCallback, NURT_LOG_ERROR};
