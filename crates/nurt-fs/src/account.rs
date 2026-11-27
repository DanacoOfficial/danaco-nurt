// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::account
// Responsibility:   File system of a process that runs with elevated rights and keeps the.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
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
//!   only read, rename or remove check the chain of directories the same way
//!   and then run by path; the account could swap a directory of its own
//!   journal for a link between the check and the call, which can make the
//!   administrator read, rename or remove inside a path that the account
//!   chose; it cannot make the administrator create a file there.
//!
//! In both modes a copy from a target into the journal reads the target with
//! the rights of the process and writes the copy as the account, block by
//! block. Operations that would leave a file of the target in the journal under
//! its old owner (hard link, clone, rename into the journal) are not offered;
//! the journal then copies.

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, ExtendedMeta, FileLock, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, SpecialNode, TxScope, VolumeId,
    VolumeInfo, WriteHandle,
};

use crate::account_secure::{self, Owner};

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

    /// Checks the chain of directories above `path`.
    fn guard(&self, op: &str, path: &AbsPath) -> FsResult<()> {
        match path.as_path().parent() {
            Some(parent) => self.secure_dir(op, parent).map(drop),
            None => Ok(()),
        }
    }

    /// The directory of `path`, opened, and the name in it.
    fn parent_of(&self, op: &str, path: &AbsPath) -> FsResult<(OwnedFd, std::ffi::OsString)> {
        let (parent, name) = account_secure::split(path.as_path())
            .map_err(|error| io_error(&error, op, path.as_path()))?;
        Ok((self.secure_dir(op, parent)?, name.to_owned()))
    }

    /// Runs `f` as the account when `path` is the path of the account.
    fn at<T>(&self, op: &str, path: &AbsPath, f: impl FnOnce() -> FsResult<T>) -> FsResult<T> {
        if !self.mine(path) {
            return f();
        }
        match self.mode {
            #[cfg(target_os = "linux")]
            Mode::FsIdentity => as_account(&self.identity, f),
            Mode::Descriptors => {
                self.guard(op, path)?;
                f()
            }
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

    /// Copies block by block: the source is read with the rights of the
    /// process, the destination is created and written as the account.
    fn stream_copy(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        let mut from = self.at("copy_file", src, || self.inner.open_read(src))?;
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
        self.at("metadata", path, || self.inner.metadata(path, follow))
    }

    fn special_node(&self, path: &AbsPath) -> FsResult<Option<SpecialNode>> {
        self.at("special_node", path, || self.inner.special_node(path))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        self.at("extended_metadata", path, || {
            self.inner.extended_metadata(path, follow)
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
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
        self.at("read_dir", path, || self.inner.read_dir(path))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        self.at("read_link", path, || self.inner.read_link(path))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        self.at("real_path", path, || self.inner.real_path(path))
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
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
        match (self.mine(from), self.mine(to)) {
            (true, true) => self.at("rename", from, || {
                self.at("rename", to, || self.inner.rename(scope, from, to, mode))
            }),
            (false, false) => self.inner.rename(scope, from, to, mode),
            _ => Err(Self::unsupported("rename", to)),
        }
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
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
        self.at("remove_file", path, || self.inner.remove_file(scope, path))
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.at("remove_dir", path, || self.inner.remove_dir(scope, path))
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
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
            return Err(Self::unsupported("create_node", path));
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
        self.at("restore_meta", path, || {
            self.inner.restore_meta(scope, path, meta)
        })
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        self.at("truncate", path, || {
            self.inner.truncate(scope, path, length)
        })
    }

    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        self.at("sync_dir", dir, || self.inner.sync_dir(scope, dir))
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        if self.secure(path) {
            match self.create_secure("lock_exclusive", path) {
                Ok(_) => {}
                Err(error) if error.kind == FsErrorKind::Exists => {}
                Err(error) => return Err(error),
            }
        }
        self.at("lock_exclusive", path, || {
            self.inner.lock_exclusive(path, timeout)
        })
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
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
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
