// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           jaq-json::limit
// Responsibility:   Limit on the length of a string made by multiplying a string by a number.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Limit on the length of a string made by multiplying a string by a number.
//!
//! Not part of upstream jaq-json. The multiplication is implemented by
//! `impl Mul for Val`, which has no access to the caller, so the limit is held
//! per thread: [`with_repeat_limit`] sets it for the duration of a closure and
//! reports whether the multiplication hit it.

use alloc::string::String;
use core::cell::Cell;

use crate::Error;

std::thread_local! {
    static MAX_BYTES: Cell<usize> = const { Cell::new(usize::MAX) };
    static EXCEEDED: Cell<bool> = const { Cell::new(false) };
}

/// Restores the limit of the enclosing call when the closure ends, also by a
/// panic.
struct Restore {
    max_bytes: usize,
    exceeded: bool,
}

impl Drop for Restore {
    fn drop(&mut self) {
        MAX_BYTES.with(|cell| cell.set(self.max_bytes));
        EXCEEDED.with(|cell| cell.set(self.exceeded));
    }
}

/// Runs `f` with a limit of `max_bytes` bytes on the string that a
/// multiplication of a string by a number may make.
///
/// A multiplication above the limit fails with an error that a filter can
/// catch with `try`, so the second element of the result tells whether any
/// multiplication in `f` hit the limit.
pub fn with_repeat_limit<R>(max_bytes: usize, f: impl FnOnce() -> R) -> (R, bool) {
    let _restore = Restore {
        max_bytes: MAX_BYTES.with(|cell| cell.replace(max_bytes)),
        exceeded: EXCEEDED.with(|cell| cell.replace(false)),
    };
    let result = f();
    (result, EXCEEDED.with(Cell::get))
}

/// Checks that a string of `bytes` bytes repeated `times` times stays within
/// the limit.
pub(crate) fn check_repeat(bytes: usize, times: usize) -> Result<(), Error> {
    let max_bytes = MAX_BYTES.with(Cell::get);
    if bytes
        .checked_mul(times)
        .is_some_and(|total| total <= max_bytes)
    {
        return Ok(());
    }
    EXCEEDED.with(|cell| cell.set(true));
    Err(Error::str(String::from(
        "the repeated string is longer than the limit of the query",
    )))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
