// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The Linux back end type and its implementation of the file system
//! interface.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::Os;
use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileKind, FileLock, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, TxScope, VolumeId, VolumeInfo,
    WriteHandle,
};
use rustix::fs::{Access, AtFlags, CWD, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use sha2::{Digest, Sha256};

use crate::key;

use super::handle::{LinuxFileLock, LinuxReadHandle, LinuxWriteHandle};
use super::meta::{self, Target, Transfer};
use super::mountinfo::MountEntry;
use super::sys::{io_error, kind_error, retry, sys_error};
use super::volume::{self, VolumeCache};
use super::{copy, holders, remove};

/// Permission bits of files created by [`FsWrite::create_temp_in`].
const TEMP_MODE: u32 = 0o600;
/// Permission bits, before the umask, of files created by
/// [`FsWrite::create_new`] and of lock files.
const NEW_FILE_MODE: u32 = 0o666;
/// Attempts to find an unused temporary name.
const TEMP_NAME_ATTEMPTS: u32 = 64;
/// Longest file name in bytes (`NAME_MAX`).
const NAME_MAX_BYTES: usize = 255;
/// Buffer used to hash content.
const HASH_BUFFER: usize = 1 << 20;
/// First pause between attempts to take a lock.
const LOCK_FIRST_PAUSE: Duration = Duration::from_millis(1);
/// Longest pause between attempts to take a lock.
const LOCK_MAX_PAUSE: Duration = Duration::from_millis(50);

/// Counter that makes temporary names unique within the process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// File system back end for Linux.
///
/// Every operation is a direct system call on the given absolute path;
/// paths are passed to the kernel as bytes, without assuming UTF-8. Name
/// changes use `renameat2`, so [`FsWrite::exchange`] and
/// [`RenameMode::NoReplace`] are atomic where the file system supports
/// them; [`RenameMode::NoReplace`] on other file systems is emulated for
/// files by `linkat` followed by `unlinkat` (a crash between the calls
/// leaves both names on the same file).
///
/// Linux has no hidden-file attribute: names starting with a dot are
/// hidden by convention, so [`FsWrite::mark_hidden`] changes nothing.
///
/// # Examples
///
/// ```
/// # #[cfg(target_os = "linux")]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use nurt_contracts::fs::{AbsPath, Fs, Follow};
/// use nurt_fs::linux::LinuxFs;
///
/// let fs = LinuxFs::new();
/// let root = AbsPath::new("/")?;
/// assert!(fs.metadata(&root, Follow::Follow)?.kind == nurt_contracts::fs::FileKind::Directory);
/// let volume = fs.volume(&root)?;
/// assert!(fs.space(&volume.id)?.available <= fs.space(&volume.id)?.total);
/// # Ok(())
/// # }
/// # #[cfg(not(target_os = "linux"))]
/// # fn main() {}
/// ```
#[derive(Debug, Default)]
pub struct LinuxFs {
    volumes: VolumeCache,
}

impl LinuxFs {
    /// Creates the back end.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Entry of the mount table for the mount that contains `path` (the
    /// nearest existing ancestor decides); for an overlay mount its layers
    /// are available through [`MountEntry::overlay_layers`].
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] when no ancestor exists or no
    /// mount contains the path, with [`FsErrorKind::Unsupported`] when the
    /// mount table cannot be read, and with the classified system error
    /// otherwise.
    pub fn mount_entry(&self, path: &AbsPath) -> FsResult<MountEntry> {
        volume::mount_entry(path.as_path())
    }
}

fn open(path: &Path, flags: OFlags, mode: u32, op: &str) -> FsResult<OwnedFd> {
    let flags = flags | OFlags::CLOEXEC | OFlags::NOCTTY;
    retry(|| rustix::fs::openat(CWD, path, flags, Mode::from_raw_mode(mode)))
        .map_err(|errno| sys_error(errno, op, path))
}

fn write_handle(
    path: &AbsPath,
    flags: OFlags,
    mode: u32,
    op: &str,
) -> FsResult<Box<dyn WriteHandle>> {
    let fd = open(path.as_path(), flags | OFlags::WRONLY, mode, op)?;
    Ok(Box::new(LinuxWriteHandle::new(fd, path.clone())))
}

/// `RENAME_NOREPLACE` for files on file systems that refuse the flag: a
/// hard link under the new name, then removal of the old name. A failed
/// removal of the old name removes the new link again.
pub(super) fn emulate_noreplace(from: &Path, to: &Path, refused: Errno) -> FsResult<()> {
    const OP: &str = "rename";
    let meta = meta::stat_at(CWD, from, Follow::NoFollow, OP, from)?;
    if meta.kind == FileKind::Directory {
        return Err(kind_error(FsErrorKind::Unsupported, refused, OP, from));
    }
    retry(|| rustix::fs::linkat(CWD, from, CWD, to, AtFlags::empty())).map_err(
        |errno| match errno {
            Errno::PERM | Errno::OPNOTSUPP => kind_error(FsErrorKind::Unsupported, errno, OP, to),
            errno => sys_error(errno, OP, to),
        },
    )?;
    if let Err(errno) = retry(|| rustix::fs::unlinkat(CWD, from, AtFlags::empty())) {
        let _ = retry(|| rustix::fs::unlinkat(CWD, to, AtFlags::empty()));
        return Err(sys_error(errno, OP, from));
    }
    Ok(())
}

fn unique_suffix() -> String {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    format!("{:x}-{counter:x}-{clock:08x}", process::id())
}

/// Name of a temporary file: the hint, shortened at a character boundary so
/// that the whole name fits in [`NAME_MAX_BYTES`], and a unique suffix.
fn temp_name(hint: &str) -> String {
    let suffix = format!(".{}", unique_suffix());
    let mut keep = hint.len().min(NAME_MAX_BYTES.saturating_sub(suffix.len()));
    while !hint.is_char_boundary(keep) {
        keep = keep.saturating_sub(1);
    }
    format!("{}{suffix}", hint.get(..keep).unwrap_or_default())
}

fn os_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

impl Fs for LinuxFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        meta::stat_at(CWD, path.as_path(), follow, "metadata", path.as_path())
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        meta::extended_metadata(path.as_path(), follow)
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        const OP: &str = "open_read";
        // `O_NONBLOCK` keeps the open of a FIFO from waiting for a writer; the
        // kind check below refuses every special file, so no read can block.
        let fd = open(path.as_path(), OFlags::RDONLY | OFlags::NONBLOCK, 0, OP)?;
        let meta = meta::stat_fd(fd.as_fd(), OP, path.as_path())?;
        match meta.kind {
            FileKind::Directory => Err(kind_error(
                FsErrorKind::IsADirectory,
                Errno::ISDIR,
                OP,
                path.as_path(),
            )),
            FileKind::Special => Err(kind_error(
                FsErrorKind::Unsupported,
                Errno::NXIO,
                OP,
                path.as_path(),
            )),
            _ => Ok(Box::new(LinuxReadHandle::new(fd, path.clone()))),
        }
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
        let fd = open(path.as_path(), OFlags::RDONLY | OFlags::DIRECTORY, 0, OP)?;
        let dir = Dir::new(fd).map_err(|errno| sys_error(errno, OP, path.as_path()))?;
        let report = path.as_path().to_path_buf();
        Ok(Box::new(dir.filter_map(move |entry| match entry {
            Ok(entry) => {
                let name = entry.file_name().to_bytes();
                if name == b"." || name == b".." {
                    return None;
                }
                let kind = match entry.file_type() {
                    FileType::RegularFile => Some(FileKind::File),
                    FileType::Directory => Some(FileKind::Directory),
                    FileType::Symlink => Some(FileKind::Symlink),
                    FileType::Unknown => None,
                    _ => Some(FileKind::Special),
                };
                Some(Ok(DirEntry {
                    name: OsStr::from_bytes(name).to_os_string(),
                    kind,
                }))
            }
            Err(errno) => Some(Err(sys_error(errno, OP, &report))),
        })))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        const OP: &str = "read_link";
        match retry(|| rustix::fs::readlinkat(CWD, path.as_path(), Vec::new())) {
            Ok(target) => Ok(os_path(target.into_bytes())),
            Err(Errno::INVAL) => Err(kind_error(
                FsErrorKind::InvalidPath,
                Errno::INVAL,
                OP,
                path.as_path(),
            )),
            Err(errno) => Err(sys_error(errno, OP, path.as_path())),
        }
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let real = std::fs::canonicalize(path.as_path())
            .map_err(|error| io_error(&error, "real_path", path.as_path()))?;
        AbsPath::new(real)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let mut handle = self.open_read(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; HASH_BUFFER];
        loop {
            match handle.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => hasher.update(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io_error(&error, "content_hash", path.as_path())),
            }
        }
        Ok(ContentHash(hasher.finalize().into()))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        volume::volume(&self.volumes, path.as_path())
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        volume::space(&self.volumes, volume)
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        let case_sensitive = volume::case_sensitive(&self.volumes, path.as_path())?;
        Ok(key::path_key(Os::Linux, case_sensitive, path.as_path()))
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        holders::holders(path.as_path())
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        const OP: &str = "can_write_dir";
        let meta = meta::stat_at(CWD, dir.as_path(), Follow::Follow, OP, dir.as_path())?;
        if meta.kind != FileKind::Directory {
            return Err(kind_error(
                FsErrorKind::NotADirectory,
                Errno::NOTDIR,
                OP,
                dir.as_path(),
            ));
        }
        let access = Access::WRITE_OK | Access::EXEC_OK;
        match retry(|| rustix::fs::accessat(CWD, dir.as_path(), access, AtFlags::EACCESS)) {
            Ok(()) => Ok(true),
            Err(Errno::ACCESS | Errno::ROFS | Errno::PERM) => Ok(false),
            Err(errno) => Err(sys_error(errno, OP, dir.as_path())),
        }
    }
}

impl FsWrite for LinuxFs {
    fn create_temp_in(
        &self,
        _scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        const OP: &str = "create_temp_in";
        dir.join_name(name_hint)?;
        let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL;
        let mut last = Errno::EXIST;
        for _ in 0..TEMP_NAME_ATTEMPTS {
            let path = dir.join_name(&temp_name(name_hint))?;
            match open(path.as_path(), flags, TEMP_MODE, OP) {
                Ok(fd) => return Ok(Box::new(LinuxWriteHandle::new(fd, path))),
                Err(error) if error.kind == FsErrorKind::Exists => last = Errno::EXIST,
                Err(error) => return Err(error),
            }
        }
        Err(sys_error(last, OP, dir.as_path()))
    }

    fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        write_handle(
            path,
            OFlags::CREATE | OFlags::EXCL,
            NEW_FILE_MODE,
            "create_new",
        )
    }

    fn open_in_place(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        write_handle(path, OFlags::NOFOLLOW, 0, "open_in_place")
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
        const OP: &str = "rename";
        let (from, to) = (from.as_path(), to.as_path());
        match mode {
            RenameMode::Replace => {
                retry(|| rustix::fs::renameat(CWD, from, CWD, to)).map_err(|errno| match errno {
                    Errno::INVAL if to != from && to.starts_with(from) => {
                        kind_error(FsErrorKind::InvalidPath, errno, OP, to)
                    }
                    errno => sys_error(errno, OP, from),
                })
            }
            RenameMode::NoReplace => {
                match retry(|| {
                    rustix::fs::renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE)
                }) {
                    Ok(()) => Ok(()),
                    // A directory moved into its own subtree is refused with
                    // `EINVAL` whatever the flags: a path error, not a missing
                    // feature to emulate.
                    Err(errno @ Errno::INVAL) if to != from && to.starts_with(from) => {
                        Err(kind_error(FsErrorKind::InvalidPath, errno, OP, to))
                    }
                    Err(errno @ (Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP)) => {
                        emulate_noreplace(from, to, errno)
                    }
                    Err(Errno::EXIST) => Err(sys_error(Errno::EXIST, OP, to)),
                    Err(errno) => Err(sys_error(errno, OP, from)),
                }
            }
        }
    }

    fn exchange(&self, _scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        const OP: &str = "exchange";
        retry(|| {
            rustix::fs::renameat_with(CWD, a.as_path(), CWD, b.as_path(), RenameFlags::EXCHANGE)
        })
        .map_err(|errno| match errno {
            Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP => {
                kind_error(FsErrorKind::Unsupported, errno, OP, a.as_path())
            }
            errno => sys_error(errno, OP, a.as_path()),
        })
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
        .map_err(|errno| sys_error(errno, "hard_link", new.as_path()))
    }

    fn clone_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        copy::clone_file(src.as_path(), dst.as_path())
    }

    fn copy_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        copy::copy_file(src.as_path(), dst.as_path())
    }

    fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        retry(|| rustix::fs::unlinkat(CWD, path.as_path(), AtFlags::empty()))
            .map_err(|errno| sys_error(errno, "remove_file", path.as_path()))
    }

    fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        const OP: &str = "remove_dir";
        retry(|| rustix::fs::unlinkat(CWD, path.as_path(), AtFlags::REMOVEDIR)).map_err(|errno| {
            match errno {
                Errno::EXIST => kind_error(FsErrorKind::NotEmpty, errno, OP, path.as_path()),
                errno => sys_error(errno, OP, path.as_path()),
            }
        })
    }

    fn remove_tree(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        remove::remove_tree(path.as_path(), parallelism)
    }

    fn create_dir(&self, _scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        retry(|| rustix::fs::mkdirat(CWD, path.as_path(), Mode::from_raw_mode(mode)))
            .map_err(|errno| sys_error(errno, "create_dir", path.as_path()))
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
        retry(|| rustix::fs::symlinkat(target, CWD, link.as_path()))
            .map_err(|errno| sys_error(errno, OP, link.as_path()))
    }

    fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        const OP: &str = "set_meta";
        let current = meta::stat_at(CWD, path.as_path(), Follow::NoFollow, OP, path.as_path())?;
        if current.kind == FileKind::Symlink && change.mode.is_some() {
            return Err(FsError::new(
                FsErrorKind::Unsupported,
                OP,
                Some(path.as_path()),
            ));
        }
        let target = Target::Path {
            path: path.as_path(),
            follow: false,
        };
        meta::set_meta(target, change, path.as_path())
    }

    fn restore_meta(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        let current = meta::stat_at(
            CWD,
            path.as_path(),
            Follow::NoFollow,
            "restore_meta",
            path.as_path(),
        )?;
        let opened = meta::open_for_flags(path.as_path(), current.kind, false);
        let target = match &opened {
            Some(fd) => Target::Fd(fd.as_fd()),
            None => Target::Path {
                path: path.as_path(),
                follow: false,
            },
        };
        Transfer {
            target,
            flags_fd: opened.as_ref().map(AsFd::as_fd),
            symlink: current.kind == FileKind::Symlink,
            report: path.as_path(),
        }
        .apply(meta)
    }

    fn truncate(&self, _scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        const OP: &str = "truncate";
        let fd = open(path.as_path(), OFlags::WRONLY | OFlags::NOFOLLOW, 0, OP)?;
        retry(|| rustix::fs::ftruncate(&fd, length))
            .map_err(|errno| sys_error(errno, OP, path.as_path()))
    }

    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        const OP: &str = "sync_dir";
        let fd = open(dir.as_path(), OFlags::RDONLY | OFlags::DIRECTORY, 0, OP)?;
        retry(|| rustix::fs::fsync(&fd)).map_err(|errno| sys_error(errno, OP, dir.as_path()))
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        const OP: &str = "lock_exclusive";
        let fd = open(
            path.as_path(),
            OFlags::RDONLY | OFlags::CREATE | OFlags::NOFOLLOW,
            NEW_FILE_MODE,
            OP,
        )?;
        let deadline = Instant::now().checked_add(timeout);
        let mut pause = LOCK_FIRST_PAUSE;
        loop {
            match retry(|| rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive)) {
                Ok(()) => return Ok(Box::new(LinuxFileLock::new(fd, path.clone()))),
                Err(Errno::WOULDBLOCK) => {
                    let now = Instant::now();
                    if deadline.is_some_and(|deadline| now >= deadline) {
                        let mut error =
                            kind_error(FsErrorKind::InUse, Errno::WOULDBLOCK, OP, path.as_path());
                        error.holders = self.holders(path).unwrap_or_default();
                        return Err(error);
                    }
                    thread::sleep(deadline.map_or(pause, |deadline| pause.min(deadline - now)));
                    pause = (pause * 2).min(LOCK_MAX_PAUSE);
                }
                Err(errno) => return Err(sys_error(errno, OP, path.as_path())),
            }
        }
    }

    fn mark_hidden(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        meta::stat_at(
            CWD,
            path.as_path(),
            Follow::NoFollow,
            "mark_hidden",
            path.as_path(),
        )
        .map(|_| ())
    }
}
