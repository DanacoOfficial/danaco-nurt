// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Error conversion, retries and time conversion shared by the system calls
//! of the Linux back end.

use std::io;
use std::path::Path;

use nurt_contracts::fs::{FsError, FsErrorKind};
use rustix::fs::Timespec;
use rustix::io::Errno;

use super::errno::{ErrnoContext, errno_error};

/// Calls of a system call interrupted by a signal before the interruption
/// is reported as [`FsErrorKind::Interrupted`].
const EINTR_ATTEMPTS: u32 = 8;
/// Nanoseconds per second.
const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// Repeats a system call interrupted by a signal (`EINTR`), at most
/// [`EINTR_ATTEMPTS`] times in total.
pub(super) fn retry<T>(mut call: impl FnMut() -> rustix::io::Result<T>) -> rustix::io::Result<T> {
    let mut attempts = 1;
    loop {
        match call() {
            Err(Errno::INTR) if attempts < EINTR_ATTEMPTS => attempts += 1,
            result => return result,
        }
    }
}

/// Error of an ordinary call.
pub(super) fn sys_error(errno: Errno, op: &str, path: &Path) -> FsError {
    errno_error(errno.raw_os_error(), ErrnoContext::Plain, op, Some(path))
}

/// Error of a call that needs a privilege beyond file permissions.
pub(super) fn privileged_error(errno: Errno, op: &str, path: &Path) -> FsError {
    errno_error(errno.raw_os_error(), ErrnoContext::Privileged, op, Some(path))
}

/// Error of a call made through the standard library.
pub(super) fn io_error(error: &io::Error, op: &str, path: &Path) -> FsError {
    match error.raw_os_error() {
        Some(code) => errno_error(code, ErrnoContext::Plain, op, Some(path)),
        None => FsError::new(FsErrorKind::Io, op, Some(path)),
    }
}

/// Error with a fixed kind and the `errno` value that caused it.
pub(super) fn kind_error(kind: FsErrorKind, errno: Errno, op: &str, path: &Path) -> FsError {
    let mut error = FsError::new(kind, op, Some(path));
    error.os_code = Some(errno.raw_os_error());
    error
}

/// Nanoseconds since the Unix epoch, saturated to the range of `i64`.
pub(super) fn nanos(seconds: i64, nanoseconds: u32) -> i64 {
    seconds
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(i64::from(nanoseconds))
}

/// Time stamp for `utimensat` from nanoseconds since the Unix epoch.
pub(super) fn timespec(nanoseconds: i64) -> Timespec {
    Timespec {
        tv_sec: nanoseconds.div_euclid(NANOS_PER_SECOND),
        tv_nsec: nanoseconds.rem_euclid(NANOS_PER_SECOND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_interrupted_call_a_bounded_number_of_times() {
        let mut calls = 0;
        let result: rustix::io::Result<()> = retry(|| {
            calls += 1;
            Err(Errno::INTR)
        });
        assert_eq!(result, Err(Errno::INTR));
        assert_eq!(calls, EINTR_ATTEMPTS);

        let mut calls = 0;
        let result = retry(|| {
            calls += 1;
            if calls < 3 { Err(Errno::INTR) } else { Ok(calls) }
        });
        assert_eq!(result, Ok(3));
    }

    #[test]
    fn reports_interruption_after_retries() {
        let error = sys_error(Errno::INTR, "fsync", Path::new("/a"));
        assert_eq!(error.kind, FsErrorKind::Interrupted);
    }

    #[test]
    fn converts_standard_library_errors() {
        let error = io_error(&io::Error::from_raw_os_error(2), "open", Path::new("/a"));
        assert_eq!((error.kind, error.os_code), (FsErrorKind::NotFound, Some(2)));
        let error = io_error(&io::Error::from(io::ErrorKind::WriteZero), "write", Path::new("/a"));
        assert_eq!((error.kind, error.os_code), (FsErrorKind::Io, None));
    }

    #[test]
    fn classifies_privileged_eperm() {
        let error = privileged_error(Errno::PERM, "fchown", Path::new("/a"));
        assert_eq!(error.kind, FsErrorKind::Privilege);
        let error = kind_error(FsErrorKind::Unsupported, Errno::INVAL, "exchange", Path::new("/a"));
        assert_eq!(error.os_code, Some(Errno::INVAL.raw_os_error()));
    }

    #[test]
    fn converts_time_stamps_in_both_directions() {
        assert_eq!(nanos(1, 5), 1_000_000_005);
        assert_eq!(nanos(-1, 999_999_999), -1);
        assert_eq!(nanos(i64::MAX, 0), i64::MAX);
        let stamp = timespec(-1);
        assert_eq!((stamp.tv_sec, stamp.tv_nsec), (-1, 999_999_999));
        let stamp = timespec(1_500_000_000);
        assert_eq!((stamp.tv_sec, stamp.tv_nsec), (1, 500_000_000));
    }
}
