// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::macos::sys::handle
// Responsibility:   Open files and locks.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Open files and locks.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd;

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FileLock, FileMeta, FsResult, MetaTransferReport,
    ReadHandle, WriteHandle,
};

use super::metadata::{apply_extended, file_meta, stat_fd};
use super::{retry, sys_error};
use crate::macos::{ErrnoContext, full_sync_unsupported};

/// File opened for reading.
#[derive(Debug)]
pub(super) struct MacosReadHandle {
    file: File,
    path: AbsPath,
}

impl MacosReadHandle {
    pub(super) fn new(file: File, path: AbsPath) -> Self {
        Self { file, path }
    }
}

impl Read for MacosReadHandle {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for MacosReadHandle {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl ReadHandle for MacosReadHandle {
    fn meta(&self) -> FsResult<FileMeta> {
        stat_fd(self.file.as_fd(), "meta", self.path.as_path()).map(|stat| file_meta(&stat))
    }
}

/// File opened for writing.
#[derive(Debug)]
pub(super) struct MacosWriteHandle {
    file: File,
    path: AbsPath,
}

impl MacosWriteHandle {
    pub(super) fn new(file: File, path: AbsPath) -> Self {
        Self { file, path }
    }
}

impl Write for MacosWriteHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for MacosWriteHandle {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl WriteHandle for MacosWriteHandle {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    /// Flushes with `fsync` for [`Durability::Data`] and with
    /// `fcntl(F_FULLFSYNC)` for [`Durability::Full`]; `fsync` on macOS does
    /// not flush the drive cache. File systems that do not implement
    /// `F_FULLFSYNC` (network and FAT volumes) fall back to `fsync`.
    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        full_sync(&self.file, durability, &self.path)
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        retry(|| rustix::fs::ftruncate(&self.file, length)).map_err(|errno| {
            sys_error(
                errno,
                ErrnoContext::Plain,
                "set_length",
                self.path.as_path(),
            )
        })
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        apply_extended(&self.file, meta, self.path.as_path())
    }
}

/// Flushes an open file or directory with the requested durability.
pub(super) fn full_sync(file: &File, durability: Durability, path: &AbsPath) -> FsResult<()> {
    const OP: &str = "sync";
    let fsync = || retry(|| rustix::fs::fsync(file));
    let result = match durability {
        Durability::Data => fsync(),
        Durability::Full => match retry(|| rustix::fs::fcntl_fullfsync(file)) {
            Err(errno) if full_sync_unsupported(errno.raw_os_error()) => fsync(),
            other => other,
        },
    };
    result.map_err(|errno| sys_error(errno, ErrnoContext::Plain, OP, path.as_path()))
}

/// Exclusive `flock` lock, released when the descriptor is closed on drop or
/// at process exit.
#[derive(Debug)]
pub(super) struct MacosLock {
    _file: File,
    path: AbsPath,
}

impl MacosLock {
    pub(super) fn new(file: File, path: AbsPath) -> Self {
        Self { _file: file, path }
    }
}

impl FileLock for MacosLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
