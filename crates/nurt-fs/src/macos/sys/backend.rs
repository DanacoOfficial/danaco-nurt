// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! [`MacosFs`]: the file system interface on macOS.

use std::fs::File;
use std::io::Read;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::env::Os;
use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileKind, FileLock, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, SpecialKind, SpecialNode, TxScope,
    VolumeId, VolumeInfo, WriteHandle,
};
use rustix::fs::{Access, AtFlags, CWD, FileType, FlockOperation, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use sha2::{Digest, Sha256};

use super::handle::{MacosLock, MacosReadHandle, MacosWriteHandle, full_sync};
use super::metadata::{
    apply_change, apply_change_by_path, apply_extended, apply_extended_by_path, file_meta,
    needs_path_calls, open_for_meta, read_extended, read_link, stat_fd, stat_path,
};
use super::volume::{case_sensitive_at, describe, nearest_statfs, read_only, space};
use super::{c_path, ffi, io_error, retry, retry_io, sys_error, tree};
use crate::key::path_key;
use crate::macos::{ErrnoContext, file_kind, hidden_flags, temp_name};

/// Permission bits of temporary files.
const TEMP_MODE: u32 = 0o600;
/// Permission bits of files created at an exact name and of lock files,
/// before the process umask.
const CREATE_MODE: u32 = 0o666;
/// Names tried by [`FsWrite::create_temp_in`] before it reports
/// [`FsErrorKind::Exists`].
const TEMP_ATTEMPTS: u64 = 64;
/// Interval between attempts to take a lock held by another descriptor.
const LOCK_POLL: Duration = Duration::from_millis(10);
/// Buffer size of [`Fs::content_hash`].
const HASH_BUFFER: usize = 64 * 1024;

/// File system back end of macOS.
///
/// Atomic exchange uses `renamex_np(RENAME_SWAP)` and rename without
/// replacement `renamex_np(RENAME_EXCL)`; both report
/// [`FsErrorKind::Unsupported`] on volumes without them. Clones use
/// `clonefile`, which copies the metadata of the source (permission bits,
/// account and group, extended attributes, ACL, flags) together with the
/// shared blocks; callers that keep metadata separately restore it with
/// [`FsWrite::restore_meta`]. [`Durability::Full`] flushes use
/// `F_FULLFSYNC`.
///
/// [`Durability::Full`]: nurt_contracts::fs::Durability::Full
///
/// # Examples
///
/// ```
/// # #[cfg(target_os = "macos")]
/// # {
/// use nurt_contracts::fs::{AbsPath, Fs};
/// use nurt_fs::macos::MacosFs;
///
/// let fs = MacosFs::new();
/// let volume = fs.volume(&AbsPath::new("/").unwrap()).unwrap();
/// assert!(!volume.fs_type.is_empty());
/// # }
/// ```
#[derive(Debug, Default)]
pub struct MacosFs {
    temp_counter: AtomicU64,
}

impl MacosFs {
    /// Creates the back end.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn open(path: &Path, flags: OFlags, mode: u32, op: &str) -> FsResult<File> {
        let mode = Mode::from_bits_truncate(mode.try_into().unwrap_or_default());
        retry(|| rustix::fs::openat(CWD, path, flags, mode))
            .map(File::from)
            .map_err(|errno| sys_error(errno, ErrnoContext::Plain, op, path))
    }

    fn write_handle(file: File, path: AbsPath) -> Box<dyn WriteHandle> {
        Box::new(MacosWriteHandle::new(file, path))
    }
}

fn plain<'a>(op: &'static str, path: &'a AbsPath) -> impl Fn(Errno) -> FsError + 'a {
    move |errno| sys_error(errno, ErrnoContext::Plain, op, path.as_path())
}

fn std_kind(kind: std::fs::FileType) -> FileKind {
    if kind.is_dir() {
        FileKind::Directory
    } else if kind.is_file() {
        FileKind::File
    } else if kind.is_symlink() {
        FileKind::Symlink
    } else {
        FileKind::Special
    }
}

impl Fs for MacosFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        stat_path(path.as_path(), follow, "metadata").map(|stat| file_meta(&stat))
    }

    fn special_node(&self, path: &AbsPath) -> FsResult<Option<SpecialNode>> {
        let stat = stat_path(path.as_path(), Follow::NoFollow, "special_node")?;
        let numbers = (
            rustix::fs::major(stat.st_rdev),
            rustix::fs::minor(stat.st_rdev),
        );
        Ok(match FileType::from_raw_mode(stat.st_mode) {
            FileType::Fifo => Some(SpecialNode::fifo()),
            FileType::CharacterDevice => {
                Some(SpecialNode::device(SpecialKind::Char, numbers.0, numbers.1))
            }
            FileType::BlockDevice => Some(SpecialNode::device(
                SpecialKind::Block,
                numbers.0,
                numbers.1,
            )),
            _ => None,
        })
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        read_extended(path.as_path(), follow)
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        const OP: &str = "open_read";
        let file = Self::open(path.as_path(), OFlags::RDONLY | OFlags::CLOEXEC, 0, OP)?;
        let stat = stat_fd(file.as_fd(), OP, path.as_path())?;
        if file_kind(u32::from(stat.st_mode)) == FileKind::Directory {
            return Err(FsError::new(
                FsErrorKind::IsADirectory,
                OP,
                Some(path.as_path()),
            ));
        }
        Ok(Box::new(MacosReadHandle::new(file, path.clone())))
    }

    fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        Err(FsError::new(
            FsErrorKind::Unsupported,
            "open_stream_read",
            Some(path.as_path()),
        ))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        const OP: &str = "read_dir";
        let dir_path = path.as_path().to_path_buf();
        let reader = std::fs::read_dir(&dir_path)
            .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, &dir_path))?;
        Ok(Box::new(reader.map(move |entry| {
            let entry =
                entry.map_err(|error| io_error(&error, ErrnoContext::Plain, OP, &dir_path))?;
            Ok(DirEntry {
                name: entry.file_name(),
                kind: entry.file_type().ok().map(std_kind),
            })
        })))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        read_link(path.as_path())
    }

    /// Resolves links with `realpath`, which on macOS also returns every
    /// component in the spelling stored by the file system.
    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        const OP: &str = "real_path";
        let real = std::fs::canonicalize(path.as_path())
            .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, path.as_path()))?;
        AbsPath::new(real)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let mut reader = self.open_read(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; HASH_BUFFER];
        loop {
            let read = retry_io(|| reader.read(&mut buffer)).map_err(|error| {
                io_error(&error, ErrnoContext::Plain, "content_hash", path.as_path())
            })?;
            let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
                break;
            };
            hasher.update(chunk);
        }
        Ok(ContentHash(hasher.finalize().into()))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        describe(path)
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        space(volume)
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        let case_sensitive = case_sensitive_at(path, "path_key")?;
        Ok(path_key(Os::Macos, case_sensitive, path.as_path()))
    }

    /// Returns an empty list after checking that the entry exists. Advisory
    /// locks on macOS never prevent renaming or removing an entry, so the
    /// back end never reports [`FsErrorKind::InUse`] for open files.
    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        stat_path(path.as_path(), Follow::NoFollow, "holders").map(|_| Vec::new())
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        const OP: &str = "can_write_dir";
        let stat = stat_path(dir.as_path(), Follow::Follow, OP)?;
        if file_kind(u32::from(stat.st_mode)) != FileKind::Directory {
            return Err(FsError::new(
                FsErrorKind::NotADirectory,
                OP,
                Some(dir.as_path()),
            ));
        }
        let (_, volume) = nearest_statfs(dir, OP)?;
        if read_only(&volume) {
            return Ok(false);
        }
        let access = Access::WRITE_OK | Access::EXEC_OK;
        match retry(|| rustix::fs::accessat(CWD, dir.as_path(), access, AtFlags::EACCESS)) {
            Ok(()) => Ok(true),
            Err(Errno::ACCESS | Errno::PERM | Errno::ROFS) => Ok(false),
            Err(errno) => Err(plain(OP, dir)(errno)),
        }
    }
}

impl FsWrite for MacosFs {
    fn create_temp_in(
        &self,
        _scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        const OP: &str = "create_temp_in";
        let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC;
        for _ in 0..TEMP_ATTEMPTS {
            let counter = self.temp_counter.fetch_add(1, Ordering::Relaxed);
            let name = temp_name(name_hint, std::process::id(), counter)
                .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath, OP, Some(dir.as_path())))?;
            let path = dir.join_name(&name)?;
            match Self::open(path.as_path(), flags, TEMP_MODE, OP) {
                Ok(file) => return Ok(Self::write_handle(file, path)),
                Err(error) if error.kind == FsErrorKind::Exists => {}
                Err(error) => return Err(error),
            }
        }
        Err(FsError::new(FsErrorKind::Exists, OP, Some(dir.as_path())))
    }

    fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let flags =
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let file = Self::open(path.as_path(), flags, CREATE_MODE, "create_new")?;
        Ok(Self::write_handle(file, path.clone()))
    }

    fn open_in_place(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let flags = OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let file = Self::open(path.as_path(), flags, 0, "open_in_place")?;
        Ok(Self::write_handle(file, path.clone()))
    }

    fn open_stream_write(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        Err(FsError::new(
            FsErrorKind::Unsupported,
            "open_stream_write",
            Some(path.as_path()),
        ))
    }

    fn rename(
        &self,
        _scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        let (from, to) = (from.as_path(), to.as_path());
        let result = match mode {
            RenameMode::Replace => retry(|| rustix::fs::renameat(CWD, from, CWD, to)),
            RenameMode::NoReplace => {
                retry(|| rustix::fs::renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE))
            }
        };
        result.map_err(|errno| sys_error(errno, ErrnoContext::Plain, "rename", from))
    }

    fn exchange(&self, _scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        let (a, b) = (a.as_path(), b.as_path());
        retry(|| rustix::fs::renameat_with(CWD, a, CWD, b, RenameFlags::EXCHANGE))
            .map_err(|errno| sys_error(errno, ErrnoContext::Exchange, "exchange", a))
    }

    fn replace_with_backup(
        &self,
        _scope: &TxScope,
        target: &AbsPath,
        _replacement: &AbsPath,
        _backup: &AbsPath,
    ) -> FsResult<()> {
        Err(FsError::new(
            FsErrorKind::Unsupported,
            "replace_with_backup",
            Some(target.as_path()),
        ))
    }

    fn hard_link(&self, _scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        retry(|| {
            rustix::fs::linkat(
                CWD,
                existing.as_path(),
                CWD,
                new.as_path(),
                AtFlags::empty(),
            )
        })
        .map_err(plain("hard_link", new))
    }

    fn clone_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        const OP: &str = "clone_file";
        let (src_c, dst_c) = (c_path(src.as_path(), OP)?, c_path(dst.as_path(), OP)?);
        retry_io(|| ffi::clone_file(&src_c, &dst_c))
            .map_err(|error| io_error(&error, ErrnoContext::Clone, OP, dst.as_path()))
    }

    /// Copies content and extended attributes (including resource forks and
    /// `com.apple.*` attributes) with `fcopyfile`, which preserves sparse
    /// regions and the content of `decmpfs`-compressed files. The new file
    /// has the permission bits `0600`; on failure it is removed.
    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        const OP: &str = "copy_file";
        // `O_NONBLOCK` keeps the open of a FIFO from waiting for a writer; the
        // kind check refuses it before the destination is created.
        let source = Self::open(
            src.as_path(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            0,
            OP,
        )?;
        let source_stat = stat_fd(source.as_fd(), OP, src.as_path())?;
        match file_kind(u32::from(source_stat.st_mode)) {
            FileKind::File => {}
            FileKind::Directory => {
                return Err(FsError::new(
                    FsErrorKind::IsADirectory,
                    OP,
                    Some(src.as_path()),
                ));
            }
            _ => {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    OP,
                    Some(src.as_path()),
                ));
            }
        }
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let target = Self::open(dst.as_path(), flags, TEMP_MODE, OP)?;
        let copied = retry_io(|| ffi::copy_data_and_xattrs(source.as_fd(), target.as_fd()))
            .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, dst.as_path()))
            .and_then(|()| stat_fd(target.as_fd(), OP, dst.as_path()));
        match copied {
            Ok(stat) => Ok(u64::try_from(stat.st_size).unwrap_or_default()),
            Err(error) => {
                drop(target);
                // The partial copy is removed on a best-effort basis; the copy
                // error is the one reported.
                let _removed = self.remove_file(scope, dst);
                Err(error)
            }
        }
    }

    /// Removes a non-directory entry. `unlink` of a directory fails with
    /// `EPERM` on macOS; that case is reported as
    /// [`FsErrorKind::IsADirectory`].
    fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        const OP: &str = "remove_file";
        match retry(|| rustix::fs::unlinkat(CWD, path.as_path(), AtFlags::empty())) {
            Ok(()) => Ok(()),
            Err(errno) => {
                let directory = errno == Errno::PERM
                    && stat_path(path.as_path(), Follow::NoFollow, OP).is_ok_and(|stat| {
                        file_kind(u32::from(stat.st_mode)) == FileKind::Directory
                    });
                if directory {
                    let mut error =
                        FsError::new(FsErrorKind::IsADirectory, OP, Some(path.as_path()));
                    error.os_code = Some(errno.raw_os_error());
                    Err(error)
                } else {
                    Err(plain(OP, path)(errno))
                }
            }
        }
    }

    fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        retry(|| rustix::fs::unlinkat(CWD, path.as_path(), AtFlags::REMOVEDIR)).map_err(|errno| {
            match errno {
                Errno::EXIST => {
                    let mut error =
                        FsError::new(FsErrorKind::NotEmpty, "remove_dir", Some(path.as_path()));
                    error.os_code = Some(errno.raw_os_error());
                    error
                }
                other => plain("remove_dir", path)(other),
            }
        })
    }

    fn remove_tree(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        tree::remove_tree(path, parallelism)
    }

    fn create_node(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        node: SpecialNode,
        mode: Option<u32>,
    ) -> FsResult<()> {
        const OP: &str = "create_node";
        let (file_type, device) = match node.kind {
            SpecialKind::Fifo => (FileType::Fifo, 0),
            SpecialKind::Char => (
                FileType::CharacterDevice,
                rustix::fs::makedev(node.major, node.minor),
            ),
            SpecialKind::Block => (
                FileType::BlockDevice,
                rustix::fs::makedev(node.major, node.minor),
            ),
            _ => {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    OP,
                    Some(path.as_path()),
                ));
            }
        };
        let to_mode = |bits: u32| Mode::from_bits_truncate(bits.try_into().unwrap_or_default());
        let context = if node.kind == SpecialKind::Fifo {
            ErrnoContext::Plain
        } else {
            ErrnoContext::Privileged
        };
        let created = to_mode(mode.unwrap_or(0o666));
        // rustix has no `mknodat` on macOS, so the node is made with `mknod`.
        let name = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(
            path.as_path().as_os_str(),
        ))
        .map_err(|_| FsError::new(FsErrorKind::InvalidPath, OP, Some(path.as_path())))?;
        let kind_bits = match file_type {
            FileType::Fifo => libc::S_IFIFO,
            FileType::CharacterDevice => libc::S_IFCHR,
            _ => libc::S_IFBLK,
        };
        let dev = libc::dev_t::try_from(device).unwrap_or_default();
        let made = retry(|| {
            // SAFETY: `name` is a valid C string that lives through the call.
            let code = unsafe {
                libc::mknod(
                    name.as_ptr(),
                    kind_bits | libc::mode_t::from(created.bits()),
                    dev,
                )
            };
            if code == 0 {
                Ok(())
            } else {
                Err(rustix::io::Errno::from_raw_os_error(
                    std::io::Error::last_os_error()
                        .raw_os_error()
                        .unwrap_or(libc::EIO),
                ))
            }
        });
        made.map_err(|errno| sys_error(errno, context, OP, path.as_path()))?;
        if let Some(exact) = mode {
            retry(|| {
                rustix::fs::chmodat(
                    CWD,
                    path.as_path(),
                    to_mode(exact),
                    AtFlags::SYMLINK_NOFOLLOW,
                )
            })
            .map_err(plain(OP, path))?;
        }
        Ok(())
    }

    fn create_dir(&self, _scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        let mode = Mode::from_bits_truncate(mode.try_into().unwrap_or_default());
        retry(|| rustix::fs::mkdirat(CWD, path.as_path(), mode)).map_err(plain("create_dir", path))
    }

    fn create_link(
        &self,
        _scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        const OP: &str = "create_link";
        if kind == LinkKind::Junction {
            return Err(FsError::new(
                FsErrorKind::Unsupported,
                OP,
                Some(link.as_path()),
            ));
        }
        retry(|| rustix::fs::symlinkat(target, CWD, link.as_path())).map_err(plain(OP, link))
    }

    fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        match open_for_meta(path.as_path(), "set_meta") {
            Ok(file) => apply_change(&file, change, path.as_path()),
            Err(error) if needs_path_calls(&error) => apply_change_by_path(path.as_path(), change),
            Err(error) => Err(error),
        }
    }

    fn restore_meta(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        match open_for_meta(path.as_path(), "restore_meta") {
            Ok(file) => apply_extended(&file, meta, path.as_path()),
            Err(error) if needs_path_calls(&error) => apply_extended_by_path(path.as_path(), meta),
            Err(error) => Err(error),
        }
    }

    fn truncate(&self, _scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        const OP: &str = "truncate";
        let file = Self::open(
            path.as_path(),
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            0,
            OP,
        )?;
        retry(|| rustix::fs::ftruncate(&file, length)).map_err(plain(OP, path))
    }

    /// Flushes a directory with `F_FULLFSYNC`, falling back to `fsync` on
    /// file systems without it.
    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let file = Self::open(dir.as_path(), flags, 0, "sync_dir")?;
        full_sync(&file, nurt_contracts::fs::Durability::Full, dir)
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        const OP: &str = "lock_exclusive";
        let flags = OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let file = Self::open(path.as_path(), flags, CREATE_MODE, OP)?;
        let deadline = Instant::now().checked_add(timeout);
        loop {
            match retry(|| rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive)) {
                Ok(()) => return Ok(Box::new(MacosLock::new(file, path.clone()))),
                Err(Errno::WOULDBLOCK) => {
                    let now = Instant::now();
                    if deadline.is_some_and(|deadline| now >= deadline) {
                        return Err(sys_error(
                            Errno::WOULDBLOCK,
                            ErrnoContext::Lock,
                            OP,
                            path.as_path(),
                        ));
                    }
                    thread::sleep(
                        deadline.map_or(LOCK_POLL, |deadline| LOCK_POLL.min(deadline - now)),
                    );
                }
                Err(errno) => return Err(plain(OP, path)(errno)),
            }
        }
    }

    /// Sets `UF_HIDDEN` on the entry itself (`lchflags`). Spotlight indexing
    /// is controlled separately by the `.noindex` suffix of the directory
    /// name.
    fn mark_hidden(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        const OP: &str = "mark_hidden";
        let stat = stat_path(path.as_path(), Follow::NoFollow, OP)?;
        let Some(flags) = hidden_flags(stat.st_flags) else {
            return Ok(());
        };
        let path_c = c_path(path.as_path(), OP)?;
        retry_io(|| ffi::set_link_flags(&path_c, flags))
            .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, path.as_path()))
    }
}
