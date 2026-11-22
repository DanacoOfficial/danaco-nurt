// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of macOS `errno` values.
//!
//! The classification is a pure function of the error number and of the call
//! context, so it is tested on every host system. On macOS the numbers come
//! from `libc`; on other hosts the values of the Darwin ABI
//! (`sys/errno.h`) are used, because they differ from the Linux values.

use std::path::Path;

use nurt_contracts::fs::{FsError, FsErrorKind};

#[cfg(target_os = "macos")]
mod code {
    pub(super) use libc::{
        EACCES, EAGAIN, EBUSY, EDQUOT, EEXIST, EILSEQ, EINTR, EINVAL, EISDIR, ELOOP, ENAMETOOLONG,
        ENOATTR, ENOENT, ENOSPC, ENOSYS, ENOTDIR, ENOTEMPTY, ENOTSUP, ENOTTY, EOPNOTSUPP, EPERM,
        EROFS, ESTALE, ETXTBSY, EXDEV,
    };
}

/// Values of the Darwin ABI (`sys/errno.h`), used when the crate is built for
/// another system.
#[cfg(not(target_os = "macos"))]
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
    pub(super) const EINVAL: i32 = 22;
    pub(super) const ENOTTY: i32 = 25;
    pub(super) const ETXTBSY: i32 = 26;
    pub(super) const ENOSPC: i32 = 28;
    pub(super) const EROFS: i32 = 30;
    pub(super) const EAGAIN: i32 = 35;
    pub(super) const ENOTSUP: i32 = 45;
    pub(super) const ELOOP: i32 = 62;
    pub(super) const ENAMETOOLONG: i32 = 63;
    pub(super) const ENOTEMPTY: i32 = 66;
    pub(super) const EDQUOT: i32 = 69;
    pub(super) const ESTALE: i32 = 70;
    pub(super) const ENOSYS: i32 = 78;
    pub(super) const EILSEQ: i32 = 92;
    pub(super) const ENOATTR: i32 = 93;
    pub(super) const EOPNOTSUPP: i32 = 102;
}

/// Context of a failing call; decides how ambiguous values are classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrnoContext {
    /// Ordinary call: `EPERM` means that permissions or file flags refuse the
    /// operation.
    Plain,
    /// Call that needs a privilege beyond file permissions, such as giving a
    /// file to another account or setting a system (`SF_*`) file flag:
    /// `EPERM` means that the privilege is missing.
    Privileged,
    /// `renamex_np` with `RENAME_SWAP`: `EINVAL` means that the volume cannot
    /// exchange names.
    Exchange,
    /// `clonefile`: `EXDEV` means that the source and the destination are on
    /// different volumes, which no clone can span.
    Clone,
    /// Non-blocking `flock`: `EAGAIN` means that another descriptor holds the
    /// lock.
    Lock,
}

/// Classifies a macOS `errno` value.
///
/// `EINTR` is classified as [`FsErrorKind::Interrupted`]; system calls of the
/// back end retry it a bounded number of times before reporting it. `ESTALE`
/// (a stale network file handle: the entry was removed or replaced on the
/// server after it was looked up) is classified as [`FsErrorKind::Stale`].
/// `EILSEQ` is returned by APFS for names that are not valid UTF-8 and is
/// classified as [`FsErrorKind::InvalidPath`]. Values without a dedicated
/// kind are [`FsErrorKind::Io`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::FsErrorKind;
/// use nurt_fs::macos::{ErrnoContext, error_kind};
///
/// // `EDQUOT` is 69 on macOS and 122 on Linux.
/// assert_eq!(error_kind(69, ErrnoContext::Plain), FsErrorKind::QuotaExceeded);
/// assert_eq!(error_kind(22, ErrnoContext::Exchange), FsErrorKind::Unsupported);
/// ```
#[must_use]
pub fn error_kind(errno: i32, context: ErrnoContext) -> FsErrorKind {
    match (errno, context) {
        (code::EPERM, ErrnoContext::Privileged) => FsErrorKind::Privilege,
        (code::EINVAL, ErrnoContext::Exchange) | (code::EXDEV, ErrnoContext::Clone) => {
            FsErrorKind::Unsupported
        }
        (code::EAGAIN, ErrnoContext::Lock) => FsErrorKind::InUse,
        _ => plain_kind(errno),
    }
}

fn plain_kind(errno: i32) -> FsErrorKind {
    match errno {
        code::ENOENT | code::ENOATTR => FsErrorKind::NotFound,
        code::EACCES | code::EPERM => FsErrorKind::PermissionDenied,
        code::EEXIST => FsErrorKind::Exists,
        code::EBUSY | code::ETXTBSY => FsErrorKind::InUse,
        code::ENOSPC => FsErrorKind::NoSpace,
        code::EDQUOT => FsErrorKind::QuotaExceeded,
        code::EXDEV => FsErrorKind::CrossDevice,
        code::ENOTSUP | code::EOPNOTSUPP | code::ENOSYS | code::ENOTTY => FsErrorKind::Unsupported,
        code::EROFS => FsErrorKind::ReadOnly,
        code::EILSEQ => FsErrorKind::InvalidPath,
        code::ENAMETOOLONG => FsErrorKind::NameTooLong,
        code::ENOTDIR => FsErrorKind::NotADirectory,
        code::EISDIR => FsErrorKind::IsADirectory,
        code::ENOTEMPTY => FsErrorKind::NotEmpty,
        code::ELOOP => FsErrorKind::Loop,
        code::EINTR | code::EAGAIN => FsErrorKind::Interrupted,
        code::ESTALE => FsErrorKind::Stale,
        _ => FsErrorKind::Io,
    }
}

/// Builds an [`FsError`] from a macOS `errno` value, keeping the value in
/// `os_code`.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use nurt_contracts::fs::FsErrorKind;
/// use nurt_fs::macos::{ErrnoContext, errno_error};
///
/// let error = errno_error(66, ErrnoContext::Plain, "remove_dir", Some(Path::new("/Users/user/a")));
/// assert_eq!(error.kind, FsErrorKind::NotEmpty);
/// assert_eq!(error.os_code, Some(66));
/// ```
#[must_use]
pub fn errno_error(errno: i32, context: ErrnoContext, op: &str, path: Option<&Path>) -> FsError {
    let mut error = FsError::new(error_kind(errno, context), op, path);
    error.os_code = Some(errno);
    error
}

/// Whether a failed `F_FULLFSYNC` request means that the file system does not
/// implement it, so that `fsync` is the strongest flush available.
///
/// Network and FAT file systems answer `F_FULLFSYNC` with `ENOTSUP`,
/// `ENOTTY` or `EINVAL` instead of flushing the drive cache.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::full_sync_unsupported;
///
/// assert!(full_sync_unsupported(45));
/// assert!(!full_sync_unsupported(5));
/// ```
#[must_use]
pub fn full_sync_unsupported(errno: i32) -> bool {
    matches!(
        errno,
        code::ENOTSUP | code::EOPNOTSUPP | code::ENOTTY | code::EINVAL
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Darwin `errno` values from `sys/errno.h` with the kind expected for an
    /// ordinary call.
    const DARWIN_TABLE: &[(i32, &str, FsErrorKind)] = &[
        (1, "EPERM", FsErrorKind::PermissionDenied),
        (2, "ENOENT", FsErrorKind::NotFound),
        (4, "EINTR", FsErrorKind::Interrupted),
        (5, "EIO", FsErrorKind::Io),
        (13, "EACCES", FsErrorKind::PermissionDenied),
        (16, "EBUSY", FsErrorKind::InUse),
        (17, "EEXIST", FsErrorKind::Exists),
        (18, "EXDEV", FsErrorKind::CrossDevice),
        (20, "ENOTDIR", FsErrorKind::NotADirectory),
        (21, "EISDIR", FsErrorKind::IsADirectory),
        (22, "EINVAL", FsErrorKind::Io),
        (25, "ENOTTY", FsErrorKind::Unsupported),
        (26, "ETXTBSY", FsErrorKind::InUse),
        (28, "ENOSPC", FsErrorKind::NoSpace),
        (30, "EROFS", FsErrorKind::ReadOnly),
        (35, "EAGAIN", FsErrorKind::Interrupted),
        (45, "ENOTSUP", FsErrorKind::Unsupported),
        (62, "ELOOP", FsErrorKind::Loop),
        (63, "ENAMETOOLONG", FsErrorKind::NameTooLong),
        (66, "ENOTEMPTY", FsErrorKind::NotEmpty),
        (69, "EDQUOT", FsErrorKind::QuotaExceeded),
        (70, "ESTALE", FsErrorKind::Stale),
        (78, "ENOSYS", FsErrorKind::Unsupported),
        (92, "EILSEQ", FsErrorKind::InvalidPath),
        (93, "ENOATTR", FsErrorKind::NotFound),
        (102, "EOPNOTSUPP", FsErrorKind::Unsupported),
    ];

    #[test]
    fn maps_darwin_errno_table_to_kinds() {
        for &(errno, name, expected) in DARWIN_TABLE {
            assert_eq!(
                error_kind(errno, ErrnoContext::Plain),
                expected,
                "{name} ({errno})"
            );
        }
    }

    #[test]
    fn classifies_eperm_of_privileged_call_as_privilege() {
        assert_eq!(
            error_kind(1, ErrnoContext::Privileged),
            FsErrorKind::Privilege
        );
        assert_eq!(
            error_kind(13, ErrnoContext::Privileged),
            FsErrorKind::PermissionDenied
        );
    }

    #[test]
    fn classifies_einval_of_exchange_as_unsupported() {
        assert_eq!(
            error_kind(22, ErrnoContext::Exchange),
            FsErrorKind::Unsupported
        );
        assert_eq!(
            error_kind(45, ErrnoContext::Exchange),
            FsErrorKind::Unsupported
        );
        assert_eq!(error_kind(2, ErrnoContext::Exchange), FsErrorKind::NotFound);
    }

    #[test]
    fn classifies_exdev_of_clone_as_unsupported() {
        assert_eq!(
            error_kind(18, ErrnoContext::Clone),
            FsErrorKind::Unsupported
        );
        assert_eq!(error_kind(17, ErrnoContext::Clone), FsErrorKind::Exists);
    }

    #[test]
    fn classifies_eagain_of_lock_as_in_use() {
        assert_eq!(error_kind(35, ErrnoContext::Lock), FsErrorKind::InUse);
    }

    #[test]
    fn classifies_unknown_errno_as_io() {
        assert_eq!(error_kind(9999, ErrnoContext::Plain), FsErrorKind::Io);
        assert_eq!(error_kind(-1, ErrnoContext::Privileged), FsErrorKind::Io);
    }

    #[test]
    fn keeps_errno_operation_and_path_in_error() {
        let error = errno_error(
            69,
            ErrnoContext::Plain,
            "write",
            Some(Path::new("/Users/user/f")),
        );
        assert_eq!(error.kind, FsErrorKind::QuotaExceeded);
        assert_eq!(error.os_code, Some(69));
        assert_eq!(error.op, "write");
        assert_eq!(error.path.as_deref(), Some(Path::new("/Users/user/f")));
    }

    #[test]
    fn falls_back_to_fsync_only_for_unsupported_full_sync() {
        for errno in [45, 102, 25, 22] {
            assert!(full_sync_unsupported(errno), "{errno}");
        }
        for errno in [5, 28, 9] {
            assert!(!full_sync_unsupported(errno), "{errno}");
        }
    }
}
