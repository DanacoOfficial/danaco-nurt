// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::account
// Responsibility:   File system of a process that runs with elevated rights and keeps the journal of an ordinary account (Unix systems).
// Dependencies:     account_secure, nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! File system of a process that runs with elevated rights and keeps the
//! journal of an ordinary account (Unix systems).
//!
//! The elevated process changes targets with the rights of the administrator
//! and writes the journal of the account that asked for the change. The
//! journal belongs to the account: every file and directory below it must be
//! created with the user and the group of the account, and the elevated
//! process must not follow a link that the account placed there with more
//! rights than the account has. Windows needs none of this: the backend `self`
//! runs as the same account.
//!
//! [`AccountFs`] works in one of two modes.
//!
//! - **File system identity** (Linux, the default there). A call that concerns
//!   a path of the account (the journal directories `.nurt-journal` and the
//!   state directory of the account) runs with the file system user and group
//!   (`fsuid`, `fsgid`) of the calling thread set to the account, so the
//!   kernel checks every component of the path, a link included, with the
//!   permissions of the account and creates files that the account owns.
//! - **Descriptors** (every other Unix system such as macOS and `doas` or
//!   `sudo` there, and Linux through [`AccountFs::with_descriptors`]). There
//!   is no `fsuid`, so a file is created with
//!   `openat(O_CREAT | O_EXCL | O_NOFOLLOW)` and a directory with `mkdirat`,
//!   relative to the descriptor of its directory, which was opened without
//!   following links and checked to belong to the account; the new entry is
//!   given to the account with `fchown` and `fchmod` (`fchownat` with
//!   `AT_SYMLINK_NOFOLLOW` for a link) before anything is written. Calls that
//!   read, rename, link or remove use the same descriptor: the entry is opened
//!   (`openat` with `O_NOFOLLOW`), examined (`fstatat` with
//!   `AT_SYMLINK_NOFOLLOW`), listed, renamed (`renameat`), linked (`linkat`) or
//!   removed (`unlinkat`) relative to the descriptor of its checked directory,
//!   so a directory of the journal that the account swaps for a link after the
//!   check cannot redirect the call. A call that has no form of this kind
//!   (`replace_with_backup`, `clone_file` between two paths of the account,
//!   the alternate streams, a change of the metadata of a link or a special
//!   file) is refused with [`FsErrorKind::Unsupported`] on a path of the
//!   account; the call never falls back to a path. The journal copes with
//!   every refusal: it copies the bytes when a clone is refused, and no
//!   volume of Unix reports `replace_with_backup`, so the journal never asks
//!   for it. `set_meta` and `restore_meta` of a file or a directory run on the
//!   descriptor of the entry (`fchown`, `fchmod`, `futimens`, `fsetxattr`),
//!   and `create_node` makes the node with `mknodat` relative to the
//!   descriptor of its directory. A link in the last component is never followed, so
//!   `metadata` reports the link itself whatever the [`Follow`] argument says.
//!   The extended metadata of an entry carries its extended attributes and the
//!   target of a link; flags and ACL are not read.
//!
//! In both modes a copy from a target into the journal reads the target with
//! the rights of the process and writes the copy as the account, block by
//! block. Operations that would leave a file of the target in the journal under
//! its old owner (hard link, clone) are not offered. A rename between a path
//! of the account and any other path ends with the error of a rename across
//! devices, as it would between two volumes, so that the journal copies the
//! content instead of moving it.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rustix::fs::FileType;
use sha2::{Digest, Sha256};

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, ExtendedMeta, FileKind, FileLock, FileMeta, Follow,
    Fs, FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, SpecialKind, SpecialNode, TxScope,
    VolumeId, VolumeInfo, WriteHandle, Xattr,
};

use crate::account_secure::{self, NodeKind, Owner, RenameHow};

/// Name that marks a journal directory.
const JOURNAL_DIR: &str = ".nurt-journal";

/// User and group of an account, and the state directory of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    /// Numeric user.
    pub uid: u32,
    /// Numeric group.
    pub gid: u32,
    /// State directory of the account.
    pub state_dir: AbsPath,
}

/// Switches the file system user and group of the calling thread and
/// restores them when it is dropped.
#[cfg(target_os = "linux")]
struct FsIdentity {
    previous: Option<(u32, u32)>,
}

#[cfg(target_os = "linux")]
impl FsIdentity {
    fn enter(uid: u32, gid: u32) -> Self {
        // SAFETY: `setfsgid` and `setfsuid` take plain numbers, have no
        // memory effects and return the previous value.
        let old_gid = unsafe { libc::setfsgid(gid) };
        // SAFETY: as above.
        let old_uid = unsafe { libc::setfsuid(uid) };
        Self {
            previous: Some((
                u32::try_from(old_uid).unwrap_or(0),
                u32::try_from(old_gid).unwrap_or(0),
            )),
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for FsIdentity {
    fn drop(&mut self) {
        if let Some((uid, gid)) = self.previous.take() {
            // SAFETY: plain numbers, no memory effects.
            unsafe {
                libc::setfsuid(uid);
                libc::setfsgid(gid);
            }
        }
    }
}

/// Runs `f` with the file system identity of `identity` (Linux).
#[cfg(target_os = "linux")]
pub fn as_account<T>(identity: &AccountIdentity, f: impl FnOnce() -> T) -> T {
    let _guard = FsIdentity::enter(identity.uid, identity.gid);
    f()
}

/// Whether `path` lies in the state directory of the account or in a journal
/// directory.
fn is_account_path(identity: &AccountIdentity, path: &AbsPath) -> bool {
    path.starts_with(&identity.state_dir)
        || path
            .as_path()
            .components()
            .any(|part| part.as_os_str().to_string_lossy().starts_with(JOURNAL_DIR))
}

/// How the process becomes the account for the paths of the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `fsuid` and `fsgid` of the thread (Linux).
    #[cfg(target_os = "linux")]
    FsIdentity,
    /// Descriptors of checked directories and `fchown` of every new entry.
    Descriptors,
}

/// The file system of the elevated process with the journal of an account.
pub struct AccountFs {
    inner: Arc<dyn FsWrite>,
    identity: AccountIdentity,
    mode: Mode,
}

impl std::fmt::Debug for AccountFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountFs")
            .field("identity", &self.identity)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// Unique part of the name of a temporary file.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Longest hint of a temporary name, in bytes.
const TEMP_HINT_MAX: usize = 200;

fn temp_name(hint: &str) -> String {
    let mut keep = hint.len().min(TEMP_HINT_MAX);
    while !hint.is_char_boundary(keep) {
        keep = keep.saturating_sub(1);
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    format!(
        "{}.{}-{}-{nanos}",
        hint.get(..keep).unwrap_or_default(),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

impl AccountFs {
    /// Wraps `inner` for the account `identity`; on Linux the file system
    /// identity of the thread is used, elsewhere descriptors.
    #[must_use]
    pub fn new(inner: Arc<dyn FsWrite>, identity: AccountIdentity) -> Self {
        #[cfg(target_os = "linux")]
        let mode = Mode::FsIdentity;
        #[cfg(not(target_os = "linux"))]
        let mode = Mode::Descriptors;
        Self {
            inner,
            identity,
            mode,
        }
    }

    /// Uses descriptors of checked directories instead of the file system
    /// identity of the thread, as on systems that have no `fsuid`.
    #[must_use]
    pub fn with_descriptors(mut self) -> Self {
        self.mode = Mode::Descriptors;
        self
    }

    fn mine(&self, path: &AbsPath) -> bool {
        is_account_path(&self.identity, path)
    }

    /// Whether `path` is handled through descriptors.
    fn secure(&self, path: &AbsPath) -> bool {
        self.mode == Mode::Descriptors && self.mine(path)
    }

    fn owner(&self) -> Owner<'_> {
        Owner {
            uid: self.identity.uid,
            gid: self.identity.gid,
            state_dir: self.identity.state_dir.as_path(),
        }
    }

    /// Opens the directory `dir` of the account without following links.
    fn secure_dir(&self, op: &str, dir: &Path) -> FsResult<OwnedFd> {
        account_secure::open_dir(&self.owner(), dir).map_err(|error| io_error(&error, op, dir))
    }

    /// The directory of `path`, opened, and the name in it.
    fn parent_of(&self, op: &str, path: &AbsPath) -> FsResult<(OwnedFd, std::ffi::OsString)> {
        let (parent, name) = account_secure::split(path.as_path())
            .map_err(|error| io_error(&error, op, path.as_path()))?;
        Ok((self.secure_dir(op, parent)?, name.to_owned()))
    }

    /// Runs `f` as the account when `path` is the path of the account. In the
    /// descriptor mode a path of the account is never handed to a call by
    /// path: a call that reaches this function for such a path has no
    /// descriptor form and is refused.
    fn at<T>(&self, op: &str, path: &AbsPath, f: impl FnOnce() -> FsResult<T>) -> FsResult<T> {
        if !self.mine(path) {
            return f();
        }
        match self.mode {
            #[cfg(target_os = "linux")]
            Mode::FsIdentity => as_account(&self.identity, f),
            Mode::Descriptors => Err(Self::unsupported(op, path)),
        }
    }

    /// Creates a file of the account through its directory and returns the
    /// handle.
    fn create_secure(&self, op: &str, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let (dir, name) = self.parent_of(op, path)?;
        let file = account_secure::create_file_in(&self.owner(), &dir, &name, 0o600)
            .map_err(|error| io_error(&error, op, path.as_path()))?;
        Ok(Box::new(FdWriter {
            file,
            path: path.clone(),
        }))
    }

    fn unsupported(op: &str, path: &AbsPath) -> FsError {
        FsError::new(FsErrorKind::Unsupported, op, Some(path.as_path()))
    }

    /// Metadata of an entry of the account, the entry named in its checked
    /// directory.
    fn secure_metadata(&self, op: &str, path: &AbsPath) -> FsResult<FileMeta> {
        let (dir, name) = self.parent_of(op, path)?;
        entry_meta(&dir, &name, op, path.as_path())
    }

    /// Opens a file of the account for reading through its checked directory.
    fn secure_open_read(&self, op: &str, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let (dir, name) = self.parent_of(op, path)?;
        let file = account_secure::open_file_for_read(&dir, &name)
            .map_err(|error| io_error(&error, op, path.as_path()))?;
        match fd_meta(file.as_fd(), op, path.as_path())?.kind {
            FileKind::Directory => Err(FsError::new(
                FsErrorKind::IsADirectory,
                op,
                Some(path.as_path()),
            )),
            FileKind::Special => Err(Self::unsupported(op, path)),
            _ => Ok(Box::new(FdReader {
                file,
                path: path.clone(),
            })),
        }
    }

    /// Opens a file or directory of the account to change its metadata
    /// through the open descriptor. A link or a special file is refused: the
    /// entry is examined before it is opened, so a device or a pipe is never
    /// opened, and examined again through the descriptor, so an entry that
    /// changed in between is not changed.
    fn secure_meta_file(&self, op: &str, path: &AbsPath) -> FsResult<File> {
        let (dir, name) = self.parent_of(op, path)?;
        match entry_meta(&dir, &name, op, path.as_path())?.kind {
            FileKind::File | FileKind::Directory => {}
            _ => return Err(Self::unsupported(op, path)),
        }
        let file = account_secure::open_entry(&dir, &name, libc::O_RDONLY | libc::O_NONBLOCK)
            .map(File::from)
            .map_err(|error| io_error(&error, op, path.as_path()))?;
        match fd_meta(file.as_fd(), op, path.as_path())?.kind {
            FileKind::File | FileKind::Directory => Ok(file),
            _ => Err(Self::unsupported(op, path)),
        }
    }

    fn secure_extended_metadata(&self, op: &str, path: &AbsPath) -> FsResult<ExtendedMeta> {
        let (dir, name) = self.parent_of(op, path)?;
        let basic = entry_meta(&dir, &name, op, path.as_path())?;
        let mut xattrs = Vec::new();
        let mut link_target = None;
        match basic.kind {
            FileKind::Symlink => {
                link_target = Some(
                    account_secure::read_link_in(&dir, &name)
                        .map_err(|error| io_error(&error, op, path.as_path()))?,
                );
            }
            FileKind::File | FileKind::Directory => {
                let mut flags = libc::O_RDONLY | libc::O_NONBLOCK;
                if basic.kind == FileKind::Directory {
                    flags |= libc::O_DIRECTORY;
                }
                let file = account_secure::open_entry(&dir, &name, flags)
                    .map(File::from)
                    .map_err(|error| io_error(&error, op, path.as_path()))?;
                xattrs = account_secure::read_xattrs_of(&file)
                    .map_err(|error| io_error(&error, op, path.as_path()))?
                    .into_iter()
                    .map(|(name, value)| Xattr { name, value })
                    .collect();
            }
            _ => {}
        }
        Ok(ExtendedMeta {
            basic,
            xattrs,
            acl: None,
            account_sid: None,
            flags: None,
            streams: Vec::new(),
            link_target,
        })
    }

    /// Moves or exchanges two entries of the account, each named in its
    /// checked directory.
    fn secure_rename(
        &self,
        op: &str,
        from: &AbsPath,
        to: &AbsPath,
        how: RenameHow,
    ) -> FsResult<()> {
        let (from_dir, from_name) = self.parent_of(op, from)?;
        let (to_dir, to_name) = self.parent_of(op, to)?;
        account_secure::rename_in(&from_dir, &from_name, &to_dir, &to_name, how)
            .map_err(|error| rename_error(&error, op, from, to))
    }

    fn secure_lock(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        const OP: &str = "lock_exclusive";
        let (dir, name) = self.parent_of(OP, path)?;
        let file = account_secure::open_lock_file(&self.owner(), &dir, &name)
            .map_err(|error| io_error(&error, OP, path.as_path()))?;
        let deadline = Instant::now().checked_add(timeout);
        let mut pause = LOCK_FIRST_PAUSE;
        loop {
            match account_secure::try_lock_exclusive(&file) {
                Ok(true) => {
                    return Ok(Box::new(FdLock {
                        _held: file,
                        path: path.clone(),
                    }));
                }
                Ok(false) => {
                    let now = Instant::now();
                    if deadline.is_some_and(|deadline| now >= deadline) {
                        return Err(FsError::new(FsErrorKind::InUse, OP, Some(path.as_path())));
                    }
                    std::thread::sleep(
                        deadline.map_or(pause, |deadline| pause.min(deadline - now)),
                    );
                    pause = (pause * 2).min(LOCK_MAX_PAUSE);
                }
                Err(error) => return Err(io_error(&error, OP, path.as_path())),
            }
        }
    }

    /// Copies block by block: the source is read with the rights of the
    /// process, the destination is created and written as the account.
    fn stream_copy(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        let mut from = self.open_read(src)?;
        let mut to = self.create_new(scope, dst)?;
        let mut block = vec![0_u8; COPY_BLOCK];
        let mut total = 0_u64;
        loop {
            let read = from
                .read(&mut block)
                .map_err(|error| io_error(&error, "copy_file", src.as_path()))?;
            if read == 0 {
                break;
            }
            self.write_as_account(dst, &mut *to, &block[..read])?;
            total += read as u64;
        }
        self.sync_as_account(dst, &mut *to)?;
        Ok(total)
    }

    fn write_as_account(
        &self,
        path: &AbsPath,
        to: &mut dyn WriteHandle,
        bytes: &[u8],
    ) -> FsResult<()> {
        let write = |to: &mut dyn WriteHandle| {
            to.write_all(bytes)
                .map_err(|error| io_error(&error, "copy_file", path.as_path()))
        };
        match self.mode {
            #[cfg(target_os = "linux")]
            Mode::FsIdentity if self.mine(path) => as_account(&self.identity, || write(to)),
            _ => write(to),
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn sync_as_account(&self, path: &AbsPath, to: &mut dyn WriteHandle) -> FsResult<()> {
        match self.mode {
            #[cfg(target_os = "linux")]
            Mode::FsIdentity if self.mine(path) => {
                as_account(&self.identity, || to.sync(Durability::Full))
            }
            _ => to.sync(Durability::Full),
        }
    }
}

/// Pause before the first retry of a lock that another holder has.
const LOCK_FIRST_PAUSE: Duration = Duration::from_millis(2);

/// Longest pause between two attempts to take a lock.
const LOCK_MAX_PAUSE: Duration = Duration::from_millis(100);

/// Metadata of the entry `name` of `dir`, a link in the last component not
/// followed.
#[cfg(target_os = "linux")]
fn entry_meta(dir: &OwnedFd, name: &OsStr, op: &str, report: &Path) -> FsResult<FileMeta> {
    crate::linux::stat_at(dir.as_fd(), Path::new(name), Follow::NoFollow, op, report)
}

/// Metadata of the entry `name` of `dir`, a link in the last component not
/// followed.
#[cfg(target_os = "macos")]
fn entry_meta(dir: &OwnedFd, name: &OsStr, op: &str, report: &Path) -> FsResult<FileMeta> {
    crate::macos::entry_meta(dir.as_fd(), name, op, report)
}

/// Metadata of an open entry.
#[cfg(target_os = "linux")]
fn fd_meta(fd: std::os::fd::BorrowedFd<'_>, op: &str, report: &Path) -> FsResult<FileMeta> {
    crate::linux::stat_fd(fd, op, report)
}

/// Metadata of an open entry.
#[cfg(target_os = "macos")]
fn fd_meta(fd: std::os::fd::BorrowedFd<'_>, op: &str, report: &Path) -> FsResult<FileMeta> {
    crate::macos::fd_meta(fd, op, report)
}

/// Applies a metadata change to an open entry.
#[cfg(target_os = "linux")]
fn apply_change_to(file: &File, change: &MetaChange, report: &Path) -> FsResult<()> {
    crate::linux::set_meta_fd(file.as_fd(), change, report)
}

/// Applies a metadata change to an open entry.
#[cfg(target_os = "macos")]
fn apply_change_to(file: &File, change: &MetaChange, report: &Path) -> FsResult<()> {
    crate::macos::set_meta_file(file, change, report)
}

/// Applies complete metadata to an open entry.
#[cfg(target_os = "linux")]
fn apply_meta_to(file: &File, meta: &ExtendedMeta, report: &Path) -> FsResult<MetaTransferReport> {
    crate::linux::restore_meta_fd(file.as_fd(), meta, report)
}

/// Applies complete metadata to an open entry.
#[cfg(target_os = "macos")]
fn apply_meta_to(file: &File, meta: &ExtendedMeta, report: &Path) -> FsResult<MetaTransferReport> {
    crate::macos::restore_meta_file(file, meta, report)
}

/// Kind of a listed entry.
fn listed_kind(kind: FileType) -> Option<FileKind> {
    match kind {
        FileType::RegularFile => Some(FileKind::File),
        FileType::Directory => Some(FileKind::Directory),
        FileType::Symlink => Some(FileKind::Symlink),
        FileType::Unknown => None,
        _ => Some(FileKind::Special),
    }
}

/// Error of a rename: the destination is named when it is the cause.
fn rename_error(error: &std::io::Error, op: &str, from: &AbsPath, to: &AbsPath) -> FsError {
    let code = error.raw_os_error();
    let inside = to != from && to.as_path().starts_with(from.as_path());
    match code {
        Some(libc::EINVAL | libc::EOPNOTSUPP) if inside => {
            FsError::new(FsErrorKind::InvalidPath, op, Some(to.as_path()))
        }
        Some(libc::EOPNOTSUPP) => FsError::new(FsErrorKind::Unsupported, op, Some(to.as_path())),
        Some(libc::EEXIST) => io_error(error, op, to.as_path()),
        _ => io_error(error, op, from.as_path()),
    }
}

/// Read handle of a file that was opened through a descriptor.
struct FdReader {
    file: File,
    path: AbsPath,
}

impl Read for FdReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for FdReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl ReadHandle for FdReader {
    fn meta(&self) -> FsResult<FileMeta> {
        fd_meta(self.file.as_fd(), "meta", self.path.as_path())
    }
}

/// Exclusive lock held through the descriptor of a lock file.
struct FdLock {
    _held: File,
    path: AbsPath,
}

impl FileLock for FdLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

/// Write handle of a file that was created through a descriptor.
struct FdWriter {
    file: std::fs::File,
    path: AbsPath,
}

impl Write for FdWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Seek for FdWriter {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl WriteHandle for FdWriter {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        let result = match durability {
            Durability::Data => self.file.sync_data(),
            Durability::Full => self.file.sync_all(),
        };
        result.map_err(|error| io_error(&error, "sync", self.path.as_path()))
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        self.file
            .set_len(length)
            .map_err(|error| io_error(&error, "set_length", self.path.as_path()))
    }

    /// The file already belongs to the account; the handle applies nothing
    /// and lists what it left out.
    fn apply_meta(&mut self, _meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        Ok(MetaTransferReport {
            not_transferred: vec!["account".to_owned()],
        })
    }
}

impl Fs for AccountFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        if self.secure(path) {
            return self.secure_metadata("metadata", path);
        }
        self.at("metadata", path, || self.inner.metadata(path, follow))
    }

    fn special_node(&self, path: &AbsPath) -> FsResult<Option<SpecialNode>> {
        if self.secure(path) {
            return match self.secure_metadata("special_node", path)?.kind {
                FileKind::Special => Err(Self::unsupported("special_node", path)),
                _ => Ok(None),
            };
        }
        self.at("special_node", path, || self.inner.special_node(path))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        if self.secure(path) {
            return self.secure_extended_metadata("extended_metadata", path);
        }
        self.at("extended_metadata", path, || {
            self.inner.extended_metadata(path, follow)
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        if self.secure(path) {
            return self.secure_open_read("open_read", path);
        }
        self.at("open_read", path, || self.inner.open_read(path))
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        self.at("open_stream_read", path, || {
            self.inner.open_stream_read(path, stream)
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        if self.secure(path) {
            let dir = self.secure_dir("read_dir", path.as_path())?;
            let entries = account_secure::list_entries(&dir)
                .map_err(|error| io_error(&error, "read_dir", path.as_path()))?;
            let listed: Vec<FsResult<DirEntry>> = entries
                .into_iter()
                .map(|(name, kind)| {
                    Ok(DirEntry {
                        name,
                        kind: listed_kind(kind),
                    })
                })
                .collect();
            return Ok(Box::new(listed.into_iter()));
        }
        self.at("read_dir", path, || self.inner.read_dir(path))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("read_link", path)?;
            return account_secure::read_link_in(&dir, &name).map_err(|error| {
                if error.raw_os_error() == Some(libc::EINVAL) {
                    FsError::new(FsErrorKind::InvalidPath, "read_link", Some(path.as_path()))
                } else {
                    io_error(&error, "read_link", path.as_path())
                }
            });
        }
        self.at("read_link", path, || self.inner.read_link(path))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        if self.secure(path) {
            return match self.secure_metadata("real_path", path)?.kind {
                FileKind::Symlink => Err(FsError::new(
                    FsErrorKind::Loop,
                    "real_path",
                    Some(path.as_path()),
                )),
                _ => Ok(path.clone()),
            };
        }
        self.at("real_path", path, || self.inner.real_path(path))
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        if self.secure(path) {
            let mut handle = self.secure_open_read("content_hash", path)?;
            let mut hasher = Sha256::new();
            let mut buffer = vec![0_u8; COPY_BLOCK];
            loop {
                match handle.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => hasher.update(&buffer[..read]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(io_error(&error, "content_hash", path.as_path())),
                }
            }
            return Ok(ContentHash(hasher.finalize().into()));
        }
        self.at("content_hash", path, || self.inner.content_hash(path))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        self.inner.volume(path)
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        self.inner.space(volume)
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        self.inner.path_key(path)
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        self.inner.holders(path)
    }

    /// The journal is created by the account, so the directory must be
    /// writable for the account, not for the administrator.
    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        match self.mode {
            #[cfg(target_os = "linux")]
            Mode::FsIdentity => as_account(&self.identity, || self.inner.can_write_dir(dir)),
            Mode::Descriptors => {
                if !self.mine(dir) {
                    return self.inner.can_write_dir(dir);
                }
                let fd = self.secure_dir("can_write_dir", dir.as_path())?;
                let meta = std::fs::File::from(fd)
                    .metadata()
                    .map_err(|error| io_error(&error, "can_write_dir", dir.as_path()))?;
                let mode = std::os::unix::fs::PermissionsExt::mode(&meta.permissions());
                Ok(mode & 0o300 == 0o300)
            }
        }
    }
}

/// Error of a call made through the standard library.
fn io_error(error: &std::io::Error, op: &str, path: &Path) -> FsError {
    if let Some(code) = error.raw_os_error() {
        return os_error(code, op, path);
    }
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => FsErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        std::io::ErrorKind::AlreadyExists => FsErrorKind::Exists,
        _ => FsErrorKind::Io,
    };
    let mut failure = FsError::new(kind, op, Some(path));
    failure.os_code = error.raw_os_error();
    failure
}

/// Error of a failed system call, classified with the `errno` values of the
/// system.
#[cfg(target_os = "macos")]
fn os_error(code: i32, op: &str, path: &Path) -> FsError {
    crate::macos::errno_error(code, crate::macos::ErrnoContext::Plain, op, Some(path))
}

/// Error of a failed system call, classified with the `errno` values of the
/// system.
#[cfg(not(target_os = "macos"))]
fn os_error(code: i32, op: &str, path: &Path) -> FsError {
    crate::linux::errno_error(code, crate::linux::ErrnoContext::Plain, op, Some(path))
}

/// Block size of a copy from a target into the journal.
const COPY_BLOCK: usize = 256 * 1024;

impl FsWrite for AccountFs {
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        if self.secure(dir) {
            const ATTEMPTS: usize = 16;
            let mut last = None;
            for _ in 0..ATTEMPTS {
                let path = dir.join_name(&temp_name(name_hint))?;
                match self.create_secure("create_temp_in", &path) {
                    Err(error) if error.kind == FsErrorKind::Exists => last = Some(error),
                    other => return other,
                }
            }
            return Err(last.unwrap_or_else(|| Self::unsupported("create_temp_in", dir)));
        }
        self.at("create_temp_in", dir, || {
            self.inner.create_temp_in(scope, dir, name_hint)
        })
    }

    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        if self.secure(path) {
            return self.create_secure("create_new", path);
        }
        self.at("create_new", path, || self.inner.create_new(scope, path))
    }

    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("open_in_place", path)?;
            let file = account_secure::open_file_for_write(&dir, &name)
                .map_err(|error| io_error(&error, "open_in_place", path.as_path()))?;
            return Ok(Box::new(FdWriter {
                file,
                path: path.clone(),
            }));
        }
        self.at("open_in_place", path, || {
            self.inner.open_in_place(scope, path)
        })
    }

    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.at("open_stream_write", path, || {
            self.inner.open_stream_write(scope, path, stream)
        })
    }

    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        if self.secure(from) && self.secure(to) {
            let how = match mode {
                RenameMode::Replace => RenameHow::Replace,
                RenameMode::NoReplace => RenameHow::NoReplace,
            };
            return self.secure_rename("rename", from, to, how);
        }
        match (self.mine(from), self.mine(to)) {
            (true, true) => self.at("rename", from, || {
                self.at("rename", to, || self.inner.rename(scope, from, to, mode))
            }),
            (false, false) => self.inner.rename(scope, from, to, mode),
            _ => Err(FsError::new(
                FsErrorKind::CrossDevice,
                "rename",
                Some(to.as_path()),
            )),
        }
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        if self.secure(a) && self.secure(b) {
            return self.secure_rename("exchange", a, b, RenameHow::Exchange);
        }
        match (self.mine(a), self.mine(b)) {
            (true, true) => self.at("exchange", a, || {
                self.at("exchange", b, || self.inner.exchange(scope, a, b))
            }),
            (false, false) => self.inner.exchange(scope, a, b),
            _ => Err(Self::unsupported("exchange", b)),
        }
    }

    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        let mine = [self.mine(target), self.mine(replacement), self.mine(backup)];
        if mine == [false; 3] {
            return self
                .inner
                .replace_with_backup(scope, target, replacement, backup);
        }
        if mine != [true; 3] {
            return Err(Self::unsupported("replace_with_backup", target));
        }
        self.at("replace_with_backup", target, || {
            self.at("replace_with_backup", replacement, || {
                self.at("replace_with_backup", backup, || {
                    self.inner
                        .replace_with_backup(scope, target, replacement, backup)
                })
            })
        })
    }

    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        if self.secure(existing) && self.secure(new) {
            let (from_dir, from_name) = self.parent_of("hard_link", existing)?;
            let (to_dir, to_name) = self.parent_of("hard_link", new)?;
            return account_secure::link_in(&from_dir, &from_name, &to_dir, &to_name)
                .map_err(|error| io_error(&error, "hard_link", new.as_path()));
        }
        match (self.mine(existing), self.mine(new)) {
            (true, true) => self.at("hard_link", existing, || {
                self.at("hard_link", new, || {
                    self.inner.hard_link(scope, existing, new)
                })
            }),
            (false, false) => self.inner.hard_link(scope, existing, new),
            _ => Err(Self::unsupported("hard_link", new)),
        }
    }

    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        match (self.mine(src), self.mine(dst)) {
            (true, true) if self.mode == Mode::Descriptors => {
                Err(Self::unsupported("clone_file", dst))
            }
            (true, true) => self.at("clone_file", src, || {
                self.at("clone_file", dst, || self.inner.clone_file(scope, src, dst))
            }),
            (false, false) => self.inner.clone_file(scope, src, dst),
            _ => Err(Self::unsupported("clone_file", dst)),
        }
    }

    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        match (self.mine(src), self.mine(dst)) {
            (true, true) if self.mode != Mode::Descriptors => self.at("copy_file", src, || {
                self.at("copy_file", dst, || self.inner.copy_file(scope, src, dst))
            }),
            (false, false) => self.inner.copy_file(scope, src, dst),
            _ => self.stream_copy(scope, src, dst),
        }
    }

    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("remove_file", path)?;
            return account_secure::unlink_entry(&dir, &name, false)
                .map_err(|error| io_error(&error, "remove_file", path.as_path()));
        }
        self.at("remove_file", path, || self.inner.remove_file(scope, path))
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("remove_dir", path)?;
            return account_secure::unlink_entry(&dir, &name, true)
                .map_err(|error| io_error(&error, "remove_dir", path.as_path()));
        }
        self.at("remove_dir", path, || self.inner.remove_dir(scope, path))
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("remove_tree", path)?;
            let removed = account_secure::remove_tree_in(&dir, &name)
                .map_err(|error| io_error(&error, "remove_tree", path.as_path()))?;
            return Ok(RemoveStats {
                files: removed.files,
                directories: removed.directories,
                bytes: removed.bytes,
            });
        }
        self.at("remove_tree", path, || {
            self.inner.remove_tree(scope, path, parallelism)
        })
    }

    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("create_dir", path)?;
            return account_secure::create_dir_in(&self.owner(), &dir, &name, mode)
                .map_err(|error| io_error(&error, "create_dir", path.as_path()));
        }
        self.at("create_dir", path, || {
            self.inner.create_dir(scope, path, mode)
        })
    }

    fn create_node(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        node: SpecialNode,
        mode: Option<u32>,
    ) -> FsResult<()> {
        if self.secure(path) {
            let kind = match node.kind {
                SpecialKind::Fifo => NodeKind::Fifo,
                SpecialKind::Char => NodeKind::Char(node.major, node.minor),
                SpecialKind::Block => NodeKind::Block(node.major, node.minor),
                _ => return Err(Self::unsupported("create_node", path)),
            };
            let (dir, name) = self.parent_of("create_node", path)?;
            return account_secure::create_node_in(&dir, &name, kind, mode.unwrap_or(0o666))
                .map_err(|error| match error.raw_os_error() {
                    Some(libc::EOPNOTSUPP) => Self::unsupported("create_node", path),
                    _ => io_error(&error, "create_node", path.as_path()),
                });
        }
        self.at("create_node", path, || {
            self.inner.create_node(scope, path, node, mode)
        })
    }

    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        if self.secure(link) {
            if kind == LinkKind::Junction {
                return Err(Self::unsupported("create_link", link));
            }
            let (dir, name) = self.parent_of("create_link", link)?;
            return account_secure::symlink_in(&self.owner(), &dir, &name, target)
                .map_err(|error| io_error(&error, "create_link", link.as_path()));
        }
        self.at("create_link", link, || {
            self.inner.create_link(scope, target, link, kind)
        })
    }

    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        if self.secure(path) {
            let file = self.secure_meta_file("set_meta", path)?;
            return apply_change_to(&file, change, path.as_path());
        }
        self.at("set_meta", path, || {
            self.inner.set_meta(scope, path, change)
        })
    }

    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        if self.secure(path) {
            let file = self.secure_meta_file("restore_meta", path)?;
            return apply_meta_to(&file, meta, path.as_path());
        }
        self.at("restore_meta", path, || {
            self.inner.restore_meta(scope, path, meta)
        })
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        if self.secure(path) {
            let (dir, name) = self.parent_of("truncate", path)?;
            return account_secure::open_file_for_write(&dir, &name)
                .and_then(|file| file.set_len(length))
                .map_err(|error| io_error(&error, "truncate", path.as_path()));
        }
        self.at("truncate", path, || {
            self.inner.truncate(scope, path, length)
        })
    }

    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        if self.secure(dir) {
            let handle = File::from(self.secure_dir("sync_dir", dir.as_path())?);
            return handle
                .sync_all()
                .map_err(|error| io_error(&error, "sync_dir", dir.as_path()));
        }
        self.at("sync_dir", dir, || self.inner.sync_dir(scope, dir))
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        if self.secure(path) {
            return self.secure_lock(path, timeout);
        }
        self.at("lock_exclusive", path, || {
            self.inner.lock_exclusive(path, timeout)
        })
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        if self.secure(path) {
            return self.secure_metadata("mark_hidden", path).map(drop);
        }
        self.at("mark_hidden", path, || self.inner.mark_hidden(scope, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> AccountIdentity {
        AccountIdentity {
            uid: 1000,
            gid: 1000,
            state_dir: AbsPath::new("/home/user/.local/state/nurt").unwrap(),
        }
    }

    #[test]
    fn journal_directories_and_the_state_directory_belong_to_the_account() {
        let who = identity();
        let path = |text: &str| AbsPath::new(text).unwrap();
        assert!(is_account_path(&who, &path("/.nurt-journal/ops/1/op.json")));
        assert!(is_account_path(&who, &path("/home/user/.nurt-journal")));
        assert!(is_account_path(
            &who,
            &path("/home/user/.local/state/nurt/user-config.toml")
        ));
        assert!(!is_account_path(&who, &path("/etc/hosts")));
        assert!(!is_account_path(&who, &path("/etc/.nurt-tmp-1-1")));
    }

    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]
    fn scope() -> TxScope {
        TxScope::new_for_journal(
            nurt_contracts::journal::OpId(1),
            nurt_contracts::fs::ScopePurpose::Maintenance,
        )
    }

    fn chown(path: &Path, uid: u32, gid: u32) {
        let text = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: `text` is a valid C string that lives through the call.
        assert_eq!(unsafe { libc::chown(text.as_ptr(), uid, gid) }, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs root"]
    fn the_copy_of_a_root_only_file_is_owned_by_the_account_and_links_are_not_followed() {
        const ACCOUNT: u32 = 54_321;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        chown(&state, ACCOUNT, ACCOUNT);
        let secret = dir.path().join("secret");
        std::fs::write(&secret, b"root only").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let who = AccountIdentity {
            uid: ACCOUNT,
            gid: ACCOUNT,
            state_dir: AbsPath::new(&state).unwrap(),
        };
        let fs = AccountFs::new(crate::system_fs(), who);
        let journal = AbsPath::new(state.join(".nurt-journal")).unwrap();
        fs.create_dir(&scope(), &journal, 0o700).unwrap();
        let copy = journal.join_name("blob").unwrap();
        let source = AbsPath::new(&secret).unwrap();
        assert_eq!(fs.copy_file(&scope(), &source, &copy).unwrap(), 9);
        let meta = std::fs::metadata(copy.as_path()).unwrap();
        assert_eq!((meta.uid(), meta.gid()), (ACCOUNT, ACCOUNT));
        assert_eq!(std::fs::metadata(journal.as_path()).unwrap().uid(), ACCOUNT);

        // A link that the account puts in its journal to a directory that only
        // the administrator may write is not followed with more rights.
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, state.join(".nurt-journal/link")).unwrap();
        let through = AbsPath::new(state.join(".nurt-journal/link/planted")).unwrap();
        assert!(fs.create_new(&scope(), &through).is_err());
        assert!(!outside.join("planted").exists());
    }

    /// The account of the test is the user that runs it, so `fchown` to it
    /// needs no administrator.
    fn own_identity(state: &Path) -> AccountIdentity {
        // SAFETY: `geteuid` and `getegid` take no arguments and cannot fail.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        AccountIdentity {
            uid,
            gid,
            state_dir: AbsPath::new(state).unwrap(),
        }
    }

    fn descriptor_fs(state: &Path) -> AccountFs {
        AccountFs::new(crate::system_fs(), own_identity(state)).with_descriptors()
    }

    #[test]
    fn descriptors_create_the_journal_tree_owned_by_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let fs = descriptor_fs(&state);
        let journal = AbsPath::new(state.join(".nurt-journal")).unwrap();
        fs.create_dir(&scope(), &journal, 0o700).unwrap();
        let file = journal.join_name("entry.json").unwrap();
        let mut handle = fs.create_new(&scope(), &file).unwrap();
        handle.write_all(b"{}").unwrap();
        handle.sync(Durability::Full).unwrap();
        let temp = fs.create_temp_in(&scope(), &journal, "part").unwrap();
        assert!(temp.path().as_path().starts_with(journal.as_path()));
        let meta = std::fs::metadata(file.as_path()).unwrap();
        let uid = own_identity(&state).uid;
        assert_eq!(meta.uid(), uid);
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(file.as_path()).unwrap(), b"{}");
        assert!(fs.can_write_dir(&journal).unwrap());
        assert!(fs.create_new(&scope(), &file).is_err());
    }

    #[test]
    fn descriptors_copy_a_target_into_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let source = dir.path().join("target");
        std::fs::write(&source, b"before").unwrap();
        let fs = descriptor_fs(&state);
        let copy = AbsPath::new(state.join("copy")).unwrap();
        let from = AbsPath::new(&source).unwrap();
        assert_eq!(fs.copy_file(&scope(), &from, &copy).unwrap(), 6);
        assert_eq!(std::fs::read(copy.as_path()).unwrap(), b"before");
        let other = AbsPath::new(dir.path().join("moved")).unwrap();
        assert!(
            fs.rename(&scope(), &copy, &other, RenameMode::Replace)
                .is_err()
        );
        assert!(fs.hard_link(&scope(), &from, &copy).is_err());
    }

    #[test]
    fn descriptors_refuse_a_link_in_the_journal_chain() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&state).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, state.join(".nurt-journal")).unwrap();
        let fs = descriptor_fs(&state);
        let planted = AbsPath::new(state.join(".nurt-journal/planted")).unwrap();
        assert!(fs.create_new(&scope(), &planted).is_err());
        assert!(
            fs.create_dir(&scope(), &planted, 0o700).is_err(),
            "a directory must not be created through the link"
        );
        assert!(fs.metadata(&planted, Follow::NoFollow).is_err());
        assert!(!outside.join("planted").exists());
    }

    #[test]
    fn descriptors_leave_paths_of_targets_to_the_inner_file_system() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let fs = descriptor_fs(&state);
        let target = AbsPath::new(dir.path().join("target.txt")).unwrap();
        fs.create_new(&scope(), &target).unwrap();
        assert!(target.as_path().exists());
        let made = AbsPath::new(dir.path().join("made")).unwrap();
        fs.create_dir(&scope(), &made, 0o755).unwrap();
        assert!(made.as_path().is_dir());
    }

    #[test]
    #[ignore = "needs root"]
    fn descriptors_give_a_root_made_copy_to_another_account_and_refuse_its_links() {
        const ACCOUNT: u32 = 54_322;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        chown(&state, ACCOUNT, ACCOUNT);
        let secret = dir.path().join("secret");
        std::fs::write(&secret, b"root only").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let who = AccountIdentity {
            uid: ACCOUNT,
            gid: ACCOUNT,
            state_dir: AbsPath::new(&state).unwrap(),
        };
        let fs = AccountFs::new(crate::system_fs(), who).with_descriptors();
        let journal = AbsPath::new(state.join(".nurt-journal")).unwrap();
        fs.create_dir(&scope(), &journal, 0o700).unwrap();
        let copy = journal.join_name("blob").unwrap();
        let source = AbsPath::new(&secret).unwrap();
        assert_eq!(fs.copy_file(&scope(), &source, &copy).unwrap(), 9);
        let meta = std::fs::metadata(copy.as_path()).unwrap();
        assert_eq!((meta.uid(), meta.gid()), (ACCOUNT, ACCOUNT));
        assert_eq!(std::fs::metadata(journal.as_path()).unwrap().uid(), ACCOUNT);
        let temp = fs.create_temp_in(&scope(), &journal, "part").unwrap();
        assert_eq!(
            std::fs::metadata(temp.path().as_path()).unwrap().uid(),
            ACCOUNT
        );

        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, state.join(".nurt-journal/link")).unwrap();
        let through = AbsPath::new(state.join(".nurt-journal/link/planted")).unwrap();
        assert!(fs.create_new(&scope(), &through).is_err());
        assert!(fs.create_dir(&scope(), &through, 0o700).is_err());
        assert!(!outside.join("planted").exists());
    }

    #[test]
    fn a_rename_between_a_target_and_the_journal_is_reported_as_across_devices() {
        for descriptors in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let state = dir.path().join("state");
            std::fs::create_dir(&state).unwrap();
            let mut fs = AccountFs::new(crate::system_fs(), own_identity(&state));
            if descriptors {
                fs = fs.with_descriptors();
            }
            let journal = AbsPath::new(state.join(".nurt-journal")).unwrap();
            fs.create_dir(&scope(), &journal, 0o700).unwrap();
            let blob = journal.join_name("blob").unwrap();
            let target = AbsPath::new(dir.path().join("target")).unwrap();
            std::fs::write(target.as_path(), b"previous").unwrap();

            let into = fs
                .rename(&scope(), &target, &blob, RenameMode::NoReplace)
                .unwrap_err();
            assert_eq!(into.kind, FsErrorKind::CrossDevice);
            assert!(target.as_path().exists() && !blob.as_path().exists());

            assert_eq!(fs.copy_file(&scope(), &target, &blob).unwrap(), 8);
            let back = AbsPath::new(dir.path().join("back")).unwrap();
            let out = fs
                .rename(&scope(), &blob, &back, RenameMode::NoReplace)
                .unwrap_err();
            assert_eq!(out.kind, FsErrorKind::CrossDevice);
            assert!(blob.as_path().exists() && !back.as_path().exists());
            assert_eq!(std::fs::read(blob.as_path()).unwrap(), b"previous");

            let plain = AbsPath::new(dir.path().join("plain")).unwrap();
            fs.rename(&scope(), &target, &plain, RenameMode::NoReplace)
                .unwrap();
            assert!(plain.as_path().exists());
        }
    }

    /// State directory whose `.nurt-journal/ops` is a link to a directory
    /// outside it, as the account can leave it between the check of a chain
    /// and a call: the file system, the planted path of a file, the directory
    /// outside and the path of the journal directory.
    fn planted_link(root: &Path) -> (AccountFs, AbsPath, PathBuf, PathBuf) {
        let state = root.join("state");
        let outside = root.join("outside");
        std::fs::create_dir_all(state.join(".nurt-journal")).unwrap();
        std::fs::create_dir_all(outside.join("tree")).unwrap();
        for name in ["file", "other"] {
            std::fs::write(outside.join(name), b"outside").unwrap();
        }
        std::fs::write(outside.join("tree/leaf"), b"outside").unwrap();
        let ops = state.join(".nurt-journal/ops");
        std::os::unix::fs::symlink(&outside, &ops).unwrap();
        let file = AbsPath::new(ops.join("file")).unwrap();
        (descriptor_fs(&state), file, outside, ops)
    }

    fn untouched(outside: &Path) {
        for name in ["file", "other", "tree/leaf"] {
            assert_eq!(std::fs::read(outside.join(name)).unwrap(), b"outside");
        }
        assert_eq!(std::fs::read_dir(outside).unwrap().count(), 3);
    }

    #[test]
    fn descriptors_do_not_read_through_a_link_in_the_journal_chain() {
        let root = tempfile::tempdir().unwrap();
        let (fs, file, outside, ops) = planted_link(root.path());
        let ops = AbsPath::new(ops).unwrap();
        assert!(fs.open_read(&file).is_err());
        assert!(fs.content_hash(&file).is_err());
        assert!(fs.metadata(&file, Follow::NoFollow).is_err());
        assert!(fs.metadata(&file, Follow::Follow).is_err());
        assert!(fs.extended_metadata(&file, Follow::NoFollow).is_err());
        assert!(fs.special_node(&file).is_err());
        assert!(fs.read_link(&file).is_err());
        assert!(fs.real_path(&file).is_err());
        assert!(fs.read_dir(&ops).is_err());
        untouched(&outside);
    }

    #[test]
    fn descriptors_do_not_change_in_place_through_a_link_in_the_journal_chain() {
        let root = tempfile::tempdir().unwrap();
        let (fs, file, outside, ops) = planted_link(root.path());
        let ops = AbsPath::new(ops).unwrap();
        assert!(fs.sync_dir(&scope(), &ops).is_err());
        assert!(fs.mark_hidden(&scope(), &file).is_err());
        assert!(
            fs.lock_exclusive(&file, Duration::from_millis(1)).is_err(),
            "no lock file may be made or locked through the link"
        );
        assert!(fs.open_in_place(&scope(), &file).is_err());
        assert!(fs.truncate(&scope(), &file, 0).is_err());
        assert!(
            fs.set_meta(&scope(), &file, &MetaChange::default())
                .is_err()
        );
        let meta = crate::system_fs()
            .extended_metadata(
                &AbsPath::new(outside.join("other")).unwrap(),
                Follow::NoFollow,
            )
            .unwrap();
        assert!(fs.restore_meta(&scope(), &file, &meta).is_err());
        assert!(
            fs.create_node(&scope(), &file, SpecialNode::fifo(), Some(0o600))
                .is_err()
        );
        untouched(&outside);
    }

    #[test]
    fn descriptors_do_not_move_or_remove_through_a_link_in_the_journal_chain() {
        let root = tempfile::tempdir().unwrap();
        let (fs, file, outside, ops) = planted_link(root.path());
        let state = root.path().join("state");
        let other = AbsPath::new(ops.join("other")).unwrap();
        let tree = AbsPath::new(ops.join("tree")).unwrap();
        let inside = AbsPath::new(state.join("inside")).unwrap();
        std::fs::write(inside.as_path(), b"inside").unwrap();
        for mode in [RenameMode::Replace, RenameMode::NoReplace] {
            assert!(fs.rename(&scope(), &file, &other, mode).is_err());
            assert!(fs.rename(&scope(), &inside, &other, mode).is_err());
            assert!(fs.rename(&scope(), &file, &inside, mode).is_err());
        }
        assert!(fs.exchange(&scope(), &file, &other).is_err());
        assert!(fs.exchange(&scope(), &inside, &other).is_err());
        assert!(fs.hard_link(&scope(), &file, &other).is_err());
        assert!(fs.hard_link(&scope(), &inside, &other).is_err());
        assert!(fs.remove_file(&scope(), &file).is_err());
        assert!(fs.remove_dir(&scope(), &tree).is_err());
        assert!(fs.remove_tree(&scope(), &tree, 2).is_err());
        assert!(fs.copy_file(&scope(), &file, &inside).is_err());
        assert!(
            fs.replace_with_backup(&scope(), &file, &other, &inside)
                .is_err()
        );
        assert_eq!(std::fs::read(inside.as_path()).unwrap(), b"inside");
        untouched(&outside);
    }

    #[test]
    fn descriptors_read_move_and_remove_inside_the_journal() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let ops = state.join(".nurt-journal/ops");
        std::fs::create_dir_all(&ops).unwrap();
        std::fs::write(ops.join("file"), b"inside").unwrap();
        let fs = descriptor_fs(&state);
        let path = |name: &str| AbsPath::new(ops.join(name)).unwrap();
        assert_eq!(
            fs.metadata(&path("file"), Follow::NoFollow).unwrap().size,
            6
        );
        let mut text = String::new();
        fs.open_read(&path("file"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "inside");
        assert_eq!(
            fs.content_hash(&path("file")).unwrap(),
            fs.content_hash(&path("file")).unwrap()
        );
        assert!(
            fs.extended_metadata(&path("file"), Follow::NoFollow)
                .unwrap()
                .link_target
                .is_none()
        );
        fs.rename(
            &scope(),
            &path("file"),
            &path("moved"),
            RenameMode::NoReplace,
        )
        .unwrap();
        fs.truncate(&scope(), &path("moved"), 2).unwrap();
        assert_eq!(
            fs.metadata(&path("moved"), Follow::NoFollow).unwrap().size,
            2
        );
        fs.hard_link(&scope(), &path("moved"), &path("linked"))
            .unwrap();
        fs.exchange(&scope(), &path("moved"), &path("linked"))
            .unwrap();
        let names: Vec<_> = fs
            .read_dir(&AbsPath::new(&ops).unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().name)
            .collect();
        assert_eq!(names.len(), 2);
        fs.remove_file(&scope(), &path("linked")).unwrap();
        fs.remove_file(&scope(), &path("moved")).unwrap();
        assert!(fs.metadata(&path("moved"), Follow::NoFollow).is_err());
        fs.create_dir(&scope(), &path("tree"), 0o700).unwrap();
        fs.create_new(&scope(), &path("tree/leaf")).unwrap();
        let stats = fs.remove_tree(&scope(), &path("tree"), 2).unwrap();
        assert_eq!((stats.files, stats.directories), (1, 1));
        let lock = fs
            .lock_exclusive(&path("lock"), Duration::from_secs(5))
            .unwrap();
        assert!(
            fs.lock_exclusive(&path("lock"), Duration::from_millis(5))
                .is_err()
        );
        drop(lock);
        // A process started by another test between the open and the close of
        // the descriptor can hold the lock until it executes its program.
        fs.lock_exclusive(&path("lock"), Duration::from_secs(5))
            .unwrap();
    }

    const ATTRIBUTE: &str = "user.nurt-account-test";

    fn attribute_of(path: &Path) -> Option<Vec<u8>> {
        xattr::get(path, ATTRIBUTE).ok().flatten()
    }

    #[test]
    fn descriptors_change_metadata_and_restore_it_on_the_open_entry() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let ops = state.join(".nurt-journal/ops");
        std::fs::create_dir_all(&ops).unwrap();
        let file = ops.join("file");
        std::fs::write(&file, b"inside").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        if xattr::set(&file, ATTRIBUTE, b"before").is_err() {
            return;
        }
        let fs = descriptor_fs(&state);
        let target = AbsPath::new(&file).unwrap();
        let kept = fs.extended_metadata(&target, Follow::NoFollow).unwrap();
        assert_eq!(kept.xattrs.len(), 1);

        let change = MetaChange {
            mode: Some(0o600),
            set_xattrs: vec![Xattr {
                name: ATTRIBUTE.into(),
                value: b"after".to_vec(),
            }],
            modified_ns: Some(1_000_000_000),
            ..MetaChange::default()
        };
        fs.set_meta(&scope(), &target, &change).unwrap();
        let now = std::fs::metadata(&file).unwrap();
        assert_eq!(now.permissions().mode() & 0o777, 0o600);
        assert_eq!(now.mtime(), 1);
        assert_eq!(attribute_of(&file).as_deref(), Some(&b"after"[..]));

        let report = fs.restore_meta(&scope(), &target, &kept).unwrap();
        assert!(report.is_complete(), "{report:?}");
        let back = std::fs::metadata(&file).unwrap();
        assert_eq!(back.permissions().mode() & 0o777, 0o640);
        assert_eq!(attribute_of(&file).as_deref(), Some(&b"before"[..]));
        assert_eq!(std::fs::read(&file).unwrap(), b"inside");

        let link = ops.join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let through = AbsPath::new(&link).unwrap();
        assert!(fs.set_meta(&scope(), &through, &change).is_err());
        assert!(fs.restore_meta(&scope(), &through, &kept).is_err());
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn descriptors_make_a_named_pipe_with_the_requested_mode() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let ops = state.join(".nurt-journal/ops");
        std::fs::create_dir_all(&ops).unwrap();
        let fs = descriptor_fs(&state);
        let pipe = AbsPath::new(ops.join("pipe")).unwrap();
        fs.create_node(&scope(), &pipe, SpecialNode::fifo(), Some(0o660))
            .unwrap();
        let made = std::fs::symlink_metadata(pipe.as_path()).unwrap();
        assert!(std::os::unix::fs::FileTypeExt::is_fifo(&made.file_type()));
        assert_eq!(made.permissions().mode() & 0o777, 0o660);
        assert!(
            fs.create_node(&scope(), &pipe, SpecialNode::fifo(), None)
                .is_err(),
            "an existing name is not replaced"
        );
        assert!(fs.metadata(&pipe, Follow::NoFollow).is_ok());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
