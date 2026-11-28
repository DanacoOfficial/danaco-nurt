// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::linux::errno
// Responsibility:   Classification of Linux `errno` values.
// Dependencies:     libc, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Classification of Linux `errno` values.
//!
//! The classification is a pure function of the error number and of the
//! call context, so it is tested on every host system. On Linux the numbers
//! come from `libc` and therefore follow the ABI of the target architecture;
//! on other hosts the values of the generic Linux ABI are used.

use std::path::Path;

use nurt_contracts::fs::{FsError, FsErrorKind};

#[cfg(target_os = "linux")]
mod code {
    pub(super) use libc::{
        EACCES, EBUSY, EDQUOT, EEXIST, EINTR, EISDIR, ELOOP, ENAMETOOLONG, ENOENT, ENOSPC, ENOSYS,
        ENOTDIR, ENOTEMPTY, ENOTTY, EOPNOTSUPP, EPERM, EROFS, ESTALE, ETXTBSY, EXDEV,
    };
}

/// Values of the generic Linux ABI (`asm-generic/errno-base.h` and
/// `asm-generic/errno.h`), used when the crate is built for another system.
/// `ENOTSUP` equals `EOPNOTSUPP` on Linux and needs no entry of its own.
#[cfg(not(target_os = "linux"))]
mod code {
    pub(super) const EPERM: i32 = 1;
    pub(super) const ENOENT: i32 = 2;
    pub(super) const EINTR: i32 = 4;
    pub(super) const EACCES: i32 = 13;
    pub(super) const EBUSY: i32 = 16;
    pub(super) const EEXIST: i32 = 17;
    pub(super) const EXDEV: i32 = 18;
    pub(super) const ENOTDIR: i32 = 20;
    pub(super) const EISDIR: i32 = 21;
    pub(super) const ENOTTY: i32 = 25;
    pub(super) const ETXTBSY: i32 = 26;
    pub(super) const ENOSPC: i32 = 28;
    pub(super) const EROFS: i32 = 30;
    pub(super) const ENAMETOOLONG: i32 = 36;
    pub(super) const ENOSYS: i32 = 38;
    pub(super) const ENOTEMPTY: i32 = 39;
    pub(super) const ELOOP: i32 = 40;
    pub(super) const EOPNOTSUPP: i32 = 95;
    pub(super) const ESTALE: i32 = 116;
    pub(super) const EDQUOT: i32 = 122;
}

/// Context of a failing call; decides how `EPERM` is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrnoContext {
    /// Ordinary call: `EPERM` means that permissions refuse the operation.
    Plain,
    /// Call that needs a privilege beyond file permissions, such as giving a
    /// file to another account, setting `security.capability` or `trusted.*`
    /// attributes, or setting the immutable or append-only flag: `EPERM`
    /// means that the privilege is missing.
    Privileged,
}

/// Classifies a Linux `errno` value.
///
/// `EINTR` is classified as [`FsErrorKind::Interrupted`]; system calls of the
/// back end retry it a bounded number of times before reporting it.
/// `ESTALE` (a stale network file handle: the entry was removed or replaced
/// on the server after it was looked up) is classified as
/// [`FsErrorKind::Stale`]. Values without a dedicated kind are
/// [`FsErrorKind::Io`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::FsErrorKind;
/// use nurt_fs::linux::{ErrnoContext, error_kind};
///
/// // `ENOENT` has the same value on every Linux architecture.
/// assert_eq!(error_kind(2, ErrnoContext::Plain), FsErrorKind::NotFound);
/// ```
#[must_use]
pub fn error_kind(errno: i32, context: ErrnoContext) -> FsErrorKind {
    match errno {
        code::ENOENT => FsErrorKind::NotFound,
        code::EACCES => FsErrorKind::PermissionDenied,
        code::EPERM => match context {
            ErrnoContext::Plain => FsErrorKind::PermissionDenied,
            ErrnoContext::Privileged => FsErrorKind::Privilege,
        },
        code::EEXIST => FsErrorKind::Exists,
        code::EBUSY | code::ETXTBSY => FsErrorKind::InUse,
        code::ENOSPC => FsErrorKind::NoSpace,
        code::EDQUOT => FsErrorKind::QuotaExceeded,
        code::EXDEV => FsErrorKind::CrossDevice,
        code::EOPNOTSUPP | code::ENOSYS | code::ENOTTY => FsErrorKind::Unsupported,
        code::EROFS => FsErrorKind::ReadOnly,
        code::ENAMETOOLONG => FsErrorKind::NameTooLong,
        code::ENOTDIR => FsErrorKind::NotADirectory,
        code::EISDIR => FsErrorKind::IsADirectory,
        code::ENOTEMPTY => FsErrorKind::NotEmpty,
        code::ELOOP => FsErrorKind::Loop,
        code::EINTR => FsErrorKind::Interrupted,
        code::ESTALE => FsErrorKind::Stale,
        _ => FsErrorKind::Io,
    }
}

/// Builds an [`FsError`] from a Linux `errno` value, keeping the value in
/// `os_code`.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use nurt_contracts::fs::FsErrorKind;
/// use nurt_fs::linux::{ErrnoContext, errno_error};
///
/// let error = errno_error(2, ErrnoContext::Plain, "rename", Some(Path::new("/home/user/a")));
/// assert_eq!(error.kind, FsErrorKind::NotFound);
/// assert_eq!(error.os_code, Some(2));
/// ```
#[must_use]
pub fn errno_error(errno: i32, context: ErrnoContext, op: &str, path: Option<&Path>) -> FsError {
    let mut error = FsError::new(error_kind(errno, context), op, path);
    error.os_code = Some(errno);
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_errno_to_its_kind() {
        let table = [
            (code::ENOENT, FsErrorKind::NotFound),
            (code::EACCES, FsErrorKind::PermissionDenied),
            (code::EPERM, FsErrorKind::PermissionDenied),
            (code::EEXIST, FsErrorKind::Exists),
            (code::EBUSY, FsErrorKind::InUse),
            (code::ETXTBSY, FsErrorKind::InUse),
            (code::ENOSPC, FsErrorKind::NoSpace),
            (code::EDQUOT, FsErrorKind::QuotaExceeded),
            (code::EXDEV, FsErrorKind::CrossDevice),
            (code::EOPNOTSUPP, FsErrorKind::Unsupported),
            (code::ENOSYS, FsErrorKind::Unsupported),
            (code::ENOTTY, FsErrorKind::Unsupported),
            (code::EROFS, FsErrorKind::ReadOnly),
            (code::ENAMETOOLONG, FsErrorKind::NameTooLong),
            (code::ENOTDIR, FsErrorKind::NotADirectory),
            (code::EISDIR, FsErrorKind::IsADirectory),
            (code::ENOTEMPTY, FsErrorKind::NotEmpty),
            (code::ELOOP, FsErrorKind::Loop),
            (code::EINTR, FsErrorKind::Interrupted),
            (code::ESTALE, FsErrorKind::Stale),
        ];
        for (errno, kind) in table {
            assert_eq!(
                error_kind(errno, ErrnoContext::Plain),
                kind,
                "errno {errno}"
            );
        }
    }

    #[test]
    fn classifies_eperm_of_privileged_call_as_privilege() {
        assert_eq!(
            error_kind(code::EPERM, ErrnoContext::Privileged),
            FsErrorKind::Privilege
        );
        assert_eq!(
            error_kind(code::EACCES, ErrnoContext::Privileged),
            FsErrorKind::PermissionDenied
        );
    }

    #[test]
    fn classifies_unknown_errno_as_io() {
        let eio = 5;
        assert_eq!(error_kind(eio, ErrnoContext::Plain), FsErrorKind::Io);
        assert_eq!(error_kind(0, ErrnoContext::Plain), FsErrorKind::Io);
        assert_eq!(error_kind(-1, ErrnoContext::Plain), FsErrorKind::Io);
        assert_eq!(error_kind(4095, ErrnoContext::Plain), FsErrorKind::Io);
    }

    #[test]
    fn keeps_errno_operation_and_path_in_error() {
        let error = errno_error(
            code::EXDEV,
            ErrnoContext::Plain,
            "exchange",
            Some(Path::new("/a")),
        );
        assert_eq!(error.kind, FsErrorKind::CrossDevice);
        assert_eq!(error.op, "exchange");
        assert_eq!(error.os_code, Some(code::EXDEV));
        assert_eq!(error.path.as_deref(), Some(Path::new("/a")));
        assert!(error.holders.is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
