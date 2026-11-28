// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-verify/tests/analyze_support/disk.rs
// Responsibility:   Read-only file system over the real directory tree, for the change detection of the process layer.
// Dependencies:     nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Read-only file system over the real directory tree, for the change
//! detection of the process layer.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileId, FileKind, FileMeta, Follow, Fs, FsError,
    FsErrorKind, FsResult, Holder, PathKey, ReadHandle, SpaceInfo, VolumeId, VolumeInfo,
};
use sha2::{Digest, Sha256};

/// Permission bits reported for every entry.
const MODE: u32 = 0o644;

fn fs_error(error: &io::Error, op: &str, path: &Path) -> FsError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => FsErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        _ => FsErrorKind::Io,
    };
    FsError::new(kind, op, Some(path))
}

fn unsupported(op: &str) -> FsError {
    FsError::new(FsErrorKind::Unsupported, op, None)
}

#[cfg(unix)]
fn file_id(meta: &fs::Metadata) -> FileId {
    use std::os::unix::fs::MetadataExt;
    FileId {
        volume: meta.dev(),
        low: meta.ino(),
        high: 0,
    }
}

#[cfg(not(unix))]
fn file_id(_meta: &fs::Metadata) -> FileId {
    FileId {
        volume: 0,
        low: 0,
        high: 0,
    }
}

fn file_meta(meta: &fs::Metadata) -> FileMeta {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Directory
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Special
    };
    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
        .unwrap_or(0);
    FileMeta {
        kind,
        size: meta.len(),
        mode: MODE,
        uid: None,
        gid: None,
        links: 1,
        modified_ns,
        accessed_ns: None,
        created_ns: None,
        changed_ns: None,
        file_id: file_id(meta),
        windows_attributes: 0,
    }
}

struct DiskRead(fs::File);

impl Read for DiskRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for DiskRead {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

impl ReadHandle for DiskRead {
    fn meta(&self) -> FsResult<FileMeta> {
        self.0
            .metadata()
            .map(|meta| file_meta(&meta))
            .map_err(|error| fs_error(&error, "meta", Path::new("")))
    }
}

/// File system over the real directory tree.
pub(crate) struct DiskFs;

impl Fs for DiskFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let meta = match follow {
            Follow::NoFollow => fs::symlink_metadata(path.as_path()),
            Follow::Follow => fs::metadata(path.as_path()),
        };
        meta.map(|meta| file_meta(&meta))
            .map_err(|error| fs_error(&error, "metadata", path.as_path()))
    }

    fn extended_metadata(&self, _path: &AbsPath, _follow: Follow) -> FsResult<ExtendedMeta> {
        Err(unsupported("extended_metadata"))
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let file = fs::File::open(path.as_path())
            .map_err(|error| fs_error(&error, "open", path.as_path()))?;
        Ok(Box::new(DiskRead(file)))
    }

    fn open_stream_read(&self, _path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        Err(unsupported("open_stream_read"))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let dir = path.as_path().to_path_buf();
        let entries =
            fs::read_dir(&dir).map_err(|error| fs_error(&error, "read_dir", path.as_path()))?;
        Ok(Box::new(entries.map(move |entry| {
            entry
                .map(|entry| DirEntry {
                    name: entry.file_name(),
                    kind: None,
                })
                .map_err(|error| fs_error(&error, "read_dir", &dir))
        })))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        fs::read_link(path.as_path()).map_err(|error| fs_error(&error, "read_link", path.as_path()))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let real = fs::canonicalize(path.as_path())
            .map_err(|error| fs_error(&error, "real_path", path.as_path()))?;
        AbsPath::new(real)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let bytes =
            fs::read(path.as_path()).map_err(|error| fs_error(&error, "read", path.as_path()))?;
        let mut hash = [0; 32];
        hash.copy_from_slice(Sha256::digest(&bytes).as_slice());
        Ok(ContentHash(hash))
    }

    fn volume(&self, _path: &AbsPath) -> FsResult<VolumeInfo> {
        Err(unsupported("volume"))
    }

    fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
        Err(unsupported("space"))
    }

    fn path_key(&self, _path: &AbsPath) -> FsResult<PathKey> {
        Err(unsupported("path_key"))
    }

    fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
        Ok(Vec::new())
    }

    fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
        Ok(true)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
