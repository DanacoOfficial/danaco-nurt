// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Recursive removal of a directory tree from the leaves.
//!
//! Directories are opened relative to the descriptor of their parent with
//! `O_DIRECTORY | O_NOFOLLOW`, so a directory replaced by a symbolic link
//! during the removal fails to open instead of being followed, and symbolic
//! links found in the tree are removed as entries. Entries are read with
//! `getdents64` in fixed-size batches; no list of the tree is built in
//! memory. Subdirectories are handed to additional threads while fewer than
//! `parallelism` threads work on the tree.

use std::ffi::{OsStr, OsString};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult, RemoveStats};
use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, StatxFlags};
use rustix::io::Errno;

use super::sys::{retry, sys_error};

const OP: &str = "remove_tree";

/// Removes `path`: a directory with its whole content, or a single entry of
/// any other kind (a symbolic link is removed, not followed).
pub(super) fn remove_tree(path: &Path, parallelism: usize) -> FsResult<RemoveStats> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(FsError::new(FsErrorKind::InvalidPath, OP, Some(path)));
    };
    let parent_fd =
        open_directory(CWD, parent, false).map_err(|errno| sys_error(errno, OP, parent))?;
    let state = State {
        files: AtomicU64::new(0),
        directories: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        spare_workers: AtomicUsize::new(parallelism.saturating_sub(1)),
        failed: AtomicBool::new(false),
        error: Mutex::new(None),
    };
    let kind =
        entry_type(parent_fd.as_fd(), name, None).map_err(|errno| sys_error(errno, OP, path))?;
    if kind == FileType::Directory {
        state.remove_directory(parent_fd.as_fd(), name, path);
    } else {
        state.remove_entry(parent_fd.as_fd(), name, path);
    }
    state.finish()
}

fn open_directory(
    dir: BorrowedFd<'_>,
    name: &Path,
    no_follow: bool,
) -> rustix::io::Result<OwnedFd> {
    let mut flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    if no_follow {
        flags |= OFlags::NOFOLLOW;
    }
    retry(|| rustix::fs::openat(dir, name, flags, Mode::empty()))
}

/// Type of an entry without following a symbolic link; `known` is the type
/// reported by `getdents64`, which some file systems leave unknown.
fn entry_type(
    dir: BorrowedFd<'_>,
    name: &OsStr,
    known: Option<FileType>,
) -> rustix::io::Result<FileType> {
    match known {
        Some(kind) if kind != FileType::Unknown => Ok(kind),
        _ => retry(|| rustix::fs::statx(dir, name, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::TYPE))
            .map(|stat| FileType::from_raw_mode(u32::from(stat.stx_mode))),
    }
}

struct State {
    files: AtomicU64,
    directories: AtomicU64,
    bytes: AtomicU64,
    spare_workers: AtomicUsize,
    failed: AtomicBool,
    error: Mutex<Option<FsError>>,
}

impl State {
    fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    fn fail(&self, error: FsError) {
        if let Ok(mut first) = self.error.lock()
            && first.is_none()
        {
            *first = Some(error);
        }
        self.failed.store(true, Ordering::Release);
    }

    fn take_worker(&self) -> bool {
        self.spare_workers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |spare| {
                spare.checked_sub(1)
            })
            .is_ok()
    }

    fn return_worker(&self) {
        self.spare_workers.fetch_add(1, Ordering::AcqRel);
    }

    fn finish(self) -> FsResult<RemoveStats> {
        let error = self
            .error
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match error {
            Some(error) => Err(error),
            None => Ok(RemoveStats {
                files: self.files.into_inner(),
                directories: self.directories.into_inner(),
                bytes: self.bytes.into_inner(),
            }),
        }
    }

    /// Removes a non-directory entry; an entry removed concurrently by
    /// another process is not an error.
    fn remove_entry(&self, dir: BorrowedFd<'_>, name: &OsStr, path: &Path) {
        let size =
            retry(|| rustix::fs::statx(dir, name, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::SIZE))
                .map_or(0, |stat| stat.stx_size);
        match retry(|| rustix::fs::unlinkat(dir, name, AtFlags::empty())) {
            Ok(()) => {
                self.files.fetch_add(1, Ordering::Relaxed);
                self.bytes.fetch_add(size, Ordering::Relaxed);
            }
            Err(Errno::NOENT) => {}
            Err(errno) => self.fail(sys_error(errno, OP, path)),
        }
    }

    fn remove_directory(&self, parent: BorrowedFd<'_>, name: &OsStr, path: &Path) {
        let fd = match open_directory(parent, Path::new(name), true) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return,
            Err(errno) => return self.fail(sys_error(errno, OP, path)),
        };
        // A mount point inside the tree holds another file system, whose
        // content is not part of the tree and is never removed.
        let same_volume = match (rustix::fs::fstat(parent), rustix::fs::fstat(&fd)) {
            (Ok(outer), Ok(inner)) => outer.st_dev == inner.st_dev,
            (Err(errno), _) | (_, Err(errno)) => return self.fail(sys_error(errno, OP, path)),
        };
        if !same_volume {
            return self.fail(FsError::new(FsErrorKind::InUse, OP, Some(path)));
        }
        self.remove_contents(fd.as_fd(), path);
        drop(fd);
        if self.failed() {
            return;
        }
        match retry(|| rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR)) {
            Ok(()) => {
                self.directories.fetch_add(1, Ordering::Relaxed);
            }
            Err(Errno::NOENT) => {}
            Err(errno) => self.fail(sys_error(errno, OP, path)),
        }
    }

    fn remove_contents(&self, dir: BorrowedFd<'_>, path: &Path) {
        let mut entries = match Dir::read_from(dir) {
            Ok(entries) => entries,
            Err(errno) => return self.fail(sys_error(errno, OP, path)),
        };
        std::thread::scope(|scope| {
            while let Some(entry) = entries.read() {
                if self.failed() {
                    break;
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(errno) => return self.fail(sys_error(errno, OP, path)),
                };
                let name = OsStr::from_bytes(entry.file_name().to_bytes());
                if name == "." || name == ".." {
                    continue;
                }
                let child: PathBuf = path.join(name);
                let kind = match entry_type(dir, name, Some(entry.file_type())) {
                    Ok(kind) => kind,
                    Err(Errno::NOENT) => continue,
                    Err(errno) => return self.fail(sys_error(errno, OP, &child)),
                };
                if kind != FileType::Directory {
                    self.remove_entry(dir, name, &child);
                } else if self.take_worker() {
                    let name: OsString = name.to_os_string();
                    scope.spawn(move || {
                        self.remove_directory(dir, &name, &child);
                        self.return_worker();
                    });
                } else {
                    self.remove_directory(dir, name, &child);
                }
            }
        });
    }
}
