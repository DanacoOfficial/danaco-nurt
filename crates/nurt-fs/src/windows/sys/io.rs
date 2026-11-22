// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Open file handles returned by the back end.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FileLock, FileMeta, FsResult, MetaTransferReport,
    ReadHandle, WriteHandle,
};

use super::handle::raw_info;
use super::security::restore_metadata;
use crate::windows::error::{from_io, os_error};

/// File or stream open for reading.
pub(crate) struct WinRead {
    file: File,
}

impl WinRead {
    pub(crate) fn new(file: File) -> Self {
        Self { file }
    }
}

impl Read for WinRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for WinRead {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl ReadHandle for WinRead {
    fn meta(&self) -> FsResult<FileMeta> {
        raw_info(&self.file)
            .map(|raw| raw.to_meta())
            .map_err(|code| os_error(code, "meta", None))
    }
}

/// File or stream open for writing.
pub(crate) struct WinWrite {
    file: File,
    path: AbsPath,
}

impl WinWrite {
    pub(crate) fn new(file: File, path: AbsPath) -> Self {
        Self { file, path }
    }
}

impl Write for WinWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for WinWrite {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl WriteHandle for WinWrite {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    /// Both durability levels use `FlushFileBuffers`, which writes data and
    /// metadata.
    fn sync(&mut self, _durability: Durability) -> FsResult<()> {
        self.file
            .sync_all()
            .map_err(|error| from_io(&error, "sync", Some(self.path.as_path())))
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        self.file
            .set_len(length)
            .map_err(|error| from_io(&error, "set_length", Some(self.path.as_path())))
    }

    /// Applies metadata through new handles on the same path; the handle of
    /// this file shares every access, so it stays open.
    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        restore_metadata(&self.path, meta)
    }
}

/// Exclusive byte-range lock over the whole lock file.
///
/// The lock belongs to the handle: closing the handle on drop releases it, and
/// the system releases it when the process ends, including termination by
/// `TerminateProcess`.
pub(crate) struct WinLock {
    _file: File,
    path: AbsPath,
}

impl WinLock {
    pub(crate) fn new(file: File, path: AbsPath) -> Self {
        Self { _file: file, path }
    }
}

impl FileLock for WinLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}
