// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Open files returned by the Linux back end.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FileLock, FileMeta, FsResult, MetaTransferReport,
    ReadHandle, WriteHandle,
};

use super::meta::{Target, Transfer, stat_fd};
use super::sys::{retry, sys_error};

/// File opened for reading.
pub(super) struct LinuxReadHandle {
    file: File,
    path: AbsPath,
}

impl LinuxReadHandle {
    pub(super) fn new(fd: OwnedFd, path: AbsPath) -> Self {
        Self {
            file: File::from(fd),
            path,
        }
    }
}

impl Read for LinuxReadHandle {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for LinuxReadHandle {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl ReadHandle for LinuxReadHandle {
    fn meta(&self) -> FsResult<FileMeta> {
        stat_fd(self.file.as_fd(), "meta", self.path.as_path())
    }
}

/// File opened for writing.
pub(super) struct LinuxWriteHandle {
    file: File,
    path: AbsPath,
}

impl LinuxWriteHandle {
    pub(super) fn new(fd: OwnedFd, path: AbsPath) -> Self {
        Self {
            file: File::from(fd),
            path,
        }
    }
}

impl Write for LinuxWriteHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for LinuxWriteHandle {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl WriteHandle for LinuxWriteHandle {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        let fd = self.file.as_fd();
        let result = match durability {
            Durability::Data => retry(|| rustix::fs::fdatasync(fd)),
            Durability::Full => retry(|| rustix::fs::fsync(fd)),
        };
        result.map_err(|errno| sys_error(errno, "sync", self.path.as_path()))
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        retry(|| rustix::fs::ftruncate(self.file.as_fd(), length))
            .map_err(|errno| sys_error(errno, "set_length", self.path.as_path()))
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        let fd = self.file.as_fd();
        Transfer {
            target: Target::Fd(fd),
            flags_fd: Some(fd),
            symlink: false,
            report: self.path.as_path(),
        }
        .apply(meta)
    }
}

/// Exclusive `flock` lock; the kernel releases it when the descriptor is
/// closed, including when the process is killed.
pub(super) struct LinuxFileLock {
    _fd: OwnedFd,
    path: AbsPath,
}

impl LinuxFileLock {
    pub(super) fn new(fd: OwnedFd, path: AbsPath) -> Self {
        Self { _fd: fd, path }
    }
}

impl FileLock for LinuxFileLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}
