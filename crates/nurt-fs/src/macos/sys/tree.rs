// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::macos::sys::tree
// Responsibility:   Bottom-up removal of directory trees.
// Dependencies:     nurt_contracts, rustix
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Bottom-up removal of directory trees.
//!
//! Every directory is opened relative to the descriptor of its parent with
//! `O_NOFOLLOW | O_DIRECTORY`, so a symbolic link that replaces a directory
//! during the removal is removed as a link and never followed. Directory
//! entries are listed completely before any of them is removed, because
//! removing entries while `readdir` advances can skip entries on HFS+.

use std::ffi::{CStr, CString, OsStr};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use nurt_contracts::fs::{AbsPath, FileKind, Follow, FsError, FsResult, RemoveStats};
use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags};
use rustix::io::Errno;

use super::metadata::stat_path;
use super::{retry, sys_error};
use crate::macos::{ErrnoContext, file_kind};

const OP: &str = "remove_tree";

/// Entry of a listed directory.
struct Entry {
    name: CString,
    directory: bool,
}

/// State shared by the workers of one removal.
struct Shared<'a> {
    parent: BorrowedFd<'a>,
    parent_path: &'a Path,
    entries: &'a [Entry],
    next: AtomicUsize,
    stopped: AtomicBool,
    stats: Mutex<RemoveStats>,
    failure: Mutex<Option<FsError>>,
}

/// Removes `path` and everything below it with up to `parallelism` workers
/// for the entries of the top directory.
pub(super) fn remove_tree(path: &AbsPath, parallelism: usize) -> FsResult<RemoveStats> {
    let stat = stat_path(path.as_path(), Follow::NoFollow, OP)?;
    if file_kind(u32::from(stat.st_mode)) != FileKind::Directory {
        retry(|| rustix::fs::unlinkat(CWD, path.as_path(), AtFlags::empty()))
            .map_err(|errno| error(errno, path.as_path()))?;
        return Ok(file_stats(stat.st_size, stat.st_mode));
    }
    let dir = open_dir(CWD, path.as_path(), path.as_path())?;
    let entries = list(&dir, path.as_path())?;
    let shared = Shared {
        parent: dir.as_fd(),
        parent_path: path.as_path(),
        entries: &entries,
        next: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
        stats: Mutex::new(RemoveStats::default()),
        failure: Mutex::new(None),
    };
    let workers = parallelism.clamp(1, entries.len().max(1));
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| work(&shared));
        }
    });
    if let Some(failure) = take(shared.failure).flatten() {
        return Err(failure);
    }
    let mut stats = take(shared.stats).unwrap_or_default();
    drop(dir);
    retry(|| rustix::fs::unlinkat(CWD, path.as_path(), AtFlags::REMOVEDIR))
        .map_err(|errno| error(errno, path.as_path()))?;
    stats.directories += 1;
    Ok(stats)
}

fn take<T>(value: Mutex<T>) -> Option<T> {
    value.into_inner().ok()
}

fn work(shared: &Shared<'_>) {
    while !shared.stopped.load(Ordering::Acquire) {
        let index = shared.next.fetch_add(1, Ordering::AcqRel);
        let Some(entry) = shared.entries.get(index) else {
            return;
        };
        match remove_entry(shared.parent, shared.parent_path, entry) {
            Ok(removed) => {
                if let Ok(mut stats) = shared.stats.lock() {
                    add(&mut stats, removed);
                }
            }
            Err(failure) => {
                shared.stopped.store(true, Ordering::Release);
                if let Ok(mut slot) = shared.failure.lock() {
                    slot.get_or_insert(failure);
                }
            }
        }
    }
}

fn remove_entry(
    parent: BorrowedFd<'_>,
    parent_path: &Path,
    entry: &Entry,
) -> FsResult<RemoveStats> {
    let path = child_path(parent_path, &entry.name);
    if entry.directory {
        match open_dir(parent, entry.name.as_c_str(), &path) {
            Ok(dir) => return remove_directory(parent, dir, &entry.name, &path),
            Err(failure) if !replaced_by_non_directory(&failure) => return Err(failure),
            Err(_) => {}
        }
    }
    let stat =
        retry(|| rustix::fs::statat(parent, entry.name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW))
            .map_err(|errno| error(errno, &path))?;
    retry(|| rustix::fs::unlinkat(parent, entry.name.as_c_str(), AtFlags::empty()))
        .map_err(|errno| error(errno, &path))?;
    Ok(file_stats(stat.st_size, stat.st_mode))
}

fn remove_directory(
    parent: BorrowedFd<'_>,
    dir: OwnedFd,
    name: &CStr,
    path: &Path,
) -> FsResult<RemoveStats> {
    let mut stats = RemoveStats::default();
    for entry in list(&dir, path)? {
        add(&mut stats, remove_entry(dir.as_fd(), path, &entry)?);
    }
    drop(dir);
    retry(|| rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR))
        .map_err(|errno| error(errno, path))?;
    stats.directories += 1;
    Ok(stats)
}

/// Whether opening an expected directory failed because the entry is now a
/// symbolic link (`ELOOP` under `O_NOFOLLOW`) or another non-directory.
fn replaced_by_non_directory(failure: &FsError) -> bool {
    matches!(
        failure.os_code,
        Some(code) if code == Errno::LOOP.raw_os_error() || code == Errno::NOTDIR.raw_os_error()
    )
}

fn open_dir<P: rustix::path::Arg + Copy>(
    parent: BorrowedFd<'_>,
    name: P,
    path: &Path,
) -> FsResult<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    retry(|| rustix::fs::openat(parent, name, flags, Mode::empty()))
        .map_err(|errno| error(errno, path))
}

fn list(dir: &OwnedFd, path: &Path) -> FsResult<Vec<Entry>> {
    let mut reader = Dir::read_from(dir).map_err(|errno| error(errno, path))?;
    let mut entries = Vec::new();
    while let Some(item) = reader.read() {
        let item = item.map_err(|errno| error(errno, path))?;
        let name = item.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        entries.push(Entry {
            name: name.to_owned(),
            directory: matches!(item.file_type(), FileType::Directory | FileType::Unknown),
        });
    }
    Ok(entries)
}

fn child_path(parent: &Path, name: &CStr) -> PathBuf {
    parent.join(OsStr::from_bytes(name.to_bytes()))
}

fn file_stats(size: i64, mode: u16) -> RemoveStats {
    let regular = file_kind(u32::from(mode)) == FileKind::File;
    RemoveStats {
        files: 1,
        directories: 0,
        bytes: if regular {
            u64::try_from(size).unwrap_or_default()
        } else {
            0
        },
    }
}

fn add(total: &mut RemoveStats, removed: RemoveStats) {
    total.files += removed.files;
    total.directories += removed.directories;
    total.bytes += removed.bytes;
}

fn error(errno: Errno, path: &Path) -> FsError {
    sys_error(errno, ErrnoContext::Plain, OP, path)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
