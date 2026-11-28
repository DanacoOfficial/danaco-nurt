// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::macos::sys
// Responsibility:   System calls of the macOS back end.
// Dependencies:     backend, nurt_contracts, rustix
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! System calls of the macOS back end.

mod backend;
mod ffi;
mod handle;
mod metadata;
mod tree;
mod volume;

#[cfg(test)]
mod tests;

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult};
use rustix::io::Errno;

pub use backend::MacosFs;

use crate::macos::{ErrnoContext, errno_error};

/// Attempts of a system call interrupted by a signal before `EINTR` is
/// reported.
const EINTR_ATTEMPTS: usize = 8;

const _: () = {
    use std::mem::{offset_of, size_of};

    use crate::macos::{
        ATTR_BIT_MAP_COUNT, ATTR_CMN_CRTIME, ATTR_VOL_CAPABILITIES, ATTR_VOL_INFO, ATTR_VOL_UUID,
        AttrList, MNT_LOCAL, MNT_QUOTA, MNT_RDONLY, Q_GETQUOTA, SF_APPEND, SF_IMMUTABLE,
        SF_SETTABLE, TIMESPEC_SIZE, UF_APPEND, UF_COMPRESSED, UF_HIDDEN, UF_IMMUTABLE, UF_SETTABLE,
        VOL_CAP_FMT_CASE_SENSITIVE, VOL_CAP_INT_CLONE, VOL_CAP_INT_EXTENDED_ATTR,
        VOL_CAP_INT_EXTENDED_SECURITY, VOL_CAP_INT_RENAME_EXCL, VOL_CAP_INT_RENAME_SWAP,
    };

    assert!(MNT_RDONLY == libc::MNT_RDONLY.cast_unsigned());
    assert!(MNT_LOCAL == libc::MNT_LOCAL.cast_unsigned());
    assert!(MNT_QUOTA == libc::MNT_QUOTA.cast_unsigned());
    assert!(Q_GETQUOTA == libc::Q_GETQUOTA);
    assert!(ATTR_BIT_MAP_COUNT == libc::ATTR_BIT_MAP_COUNT);
    assert!(ATTR_CMN_CRTIME == libc::ATTR_CMN_CRTIME);
    assert!(ATTR_VOL_INFO == libc::ATTR_VOL_INFO);
    assert!(ATTR_VOL_CAPABILITIES == libc::ATTR_VOL_CAPABILITIES);
    assert!(ATTR_VOL_UUID == libc::ATTR_VOL_UUID);
    assert!(VOL_CAP_FMT_CASE_SENSITIVE == libc::VOL_CAP_FMT_CASE_SENSITIVE);
    assert!(VOL_CAP_INT_CLONE == libc::VOL_CAP_INT_CLONE);
    assert!(VOL_CAP_INT_RENAME_SWAP == libc::VOL_CAP_INT_RENAME_SWAP);
    assert!(VOL_CAP_INT_RENAME_EXCL == libc::VOL_CAP_INT_RENAME_EXCL);
    assert!(VOL_CAP_INT_EXTENDED_ATTR == libc::VOL_CAP_INT_EXTENDED_ATTR);
    assert!(VOL_CAP_INT_EXTENDED_SECURITY == libc::VOL_CAP_INT_EXTENDED_SECURITY);
    assert!(UF_IMMUTABLE == libc::UF_IMMUTABLE);
    assert!(UF_APPEND == libc::UF_APPEND);
    assert!(UF_COMPRESSED == libc::UF_COMPRESSED);
    assert!(UF_HIDDEN == libc::UF_HIDDEN);
    assert!(UF_SETTABLE == libc::UF_SETTABLE);
    assert!(SF_IMMUTABLE == libc::SF_IMMUTABLE);
    assert!(SF_APPEND == libc::SF_APPEND);
    assert!(SF_SETTABLE == libc::SF_SETTABLE);
    assert!(TIMESPEC_SIZE == size_of::<libc::timespec>());
    assert!(size_of::<AttrList>() == size_of::<libc::attrlist>());
    assert!(offset_of!(AttrList, bitmapcount) == offset_of!(libc::attrlist, bitmapcount));
    assert!(offset_of!(AttrList, commonattr) == offset_of!(libc::attrlist, commonattr));
    assert!(offset_of!(AttrList, volattr) == offset_of!(libc::attrlist, volattr));
    assert!(offset_of!(AttrList, forkattr) == offset_of!(libc::attrlist, forkattr));
};

/// Runs a system call again while it fails with `EINTR`, at most
/// [`EINTR_ATTEMPTS`] times.
fn retry<T>(mut call: impl FnMut() -> rustix::io::Result<T>) -> rustix::io::Result<T> {
    let mut attempts = 1;
    loop {
        match call() {
            Err(Errno::INTR) if attempts < EINTR_ATTEMPTS => attempts += 1,
            result => return result,
        }
    }
}

/// Runs a `std::io` call again while it fails with `EINTR`.
fn retry_io<T>(mut call: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempts = 1;
    loop {
        match call() {
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted && attempts < EINTR_ATTEMPTS =>
            {
                attempts += 1;
            }
            result => return result,
        }
    }
}

/// Basic metadata of the entry `name` of the directory `dir`; a symbolic link
/// in the last component is reported, not followed.
pub(crate) fn entry_meta(
    dir: std::os::fd::BorrowedFd<'_>,
    name: &std::ffi::OsStr,
    op: &str,
    report: &Path,
) -> FsResult<nurt_contracts::fs::FileMeta> {
    retry(|| rustix::fs::statat(dir, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW))
        .map(|stat| metadata::file_meta(&stat))
        .map_err(|errno| sys_error(errno, ErrnoContext::Plain, op, report))
}

/// Basic metadata of an open entry.
pub(crate) fn fd_meta(
    fd: std::os::fd::BorrowedFd<'_>,
    op: &str,
    report: &Path,
) -> FsResult<nurt_contracts::fs::FileMeta> {
    metadata::stat_fd(fd, op, report).map(|stat| metadata::file_meta(&stat))
}

/// Applies a metadata change to an open file or directory; the first refused
/// element fails the call.
pub(crate) fn set_meta_file(
    file: &std::fs::File,
    change: &nurt_contracts::fs::MetaChange,
    report: &Path,
) -> FsResult<()> {
    metadata::apply_change(file, change, report)
}

/// Applies complete metadata to an open file or directory and reports every
/// element that could not be applied.
pub(crate) fn restore_meta_file(
    file: &std::fs::File,
    meta: &nurt_contracts::fs::ExtendedMeta,
    report: &Path,
) -> FsResult<nurt_contracts::fs::MetaTransferReport> {
    metadata::apply_extended(file, meta, report)
}

fn sys_error(errno: Errno, context: ErrnoContext, op: &str, path: &Path) -> FsError {
    errno_error(errno.raw_os_error(), context, op, Some(path))
}

fn io_error(error: &io::Error, context: ErrnoContext, op: &str, path: &Path) -> FsError {
    match error.raw_os_error() {
        Some(code) => errno_error(code, context, op, Some(path)),
        None if error.kind() == io::ErrorKind::InvalidInput => {
            FsError::new(FsErrorKind::InvalidPath, op, Some(path))
        }
        None => FsError::new(FsErrorKind::Io, op, Some(path)),
    }
}

fn c_path(path: &Path, op: &str) -> FsResult<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| FsError::new(FsErrorKind::InvalidPath, op, Some(path)))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
