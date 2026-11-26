// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! System layer of the Windows back end.

mod handle;
mod io;
mod ops;
mod restart;
mod security;
mod tree;
mod volume;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, TryLockError};
use std::io::Read;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{process, thread};

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileKind, FileLock, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, TxScope, VolumeId, VolumeInfo,
    WriteHandle,
};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, MAX_PATH};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CREATE_NEW, FILE_ADD_FILE, FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
    FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES, GetFinalPathNameByHandleW, OPEN_ALWAYS,
    OPEN_EXISTING, VOLUME_NAME_DOS,
};

use self::handle::{
    WidePath, attributes_of, basic_info, create_file, has_file_ancestor, last_error, list_dir,
    list_streams, raw_info, set_basic_info,
};
use self::io::{WinLock, WinRead, WinWrite};
use crate::windows::error::codes::{
    ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_PATH_NOT_FOUND, ERROR_WRITE_PROTECT,
};
use crate::windows::error::{from_io, is_sharing_conflict, is_unsupported_request, os_error};
use crate::windows::meta::{
    JOURNAL_DIRECTORY_ATTRIBUTES, attributes_for_mode, basic_info_attributes, classify_kind,
    unix_ns_to_filetime, unsettable_attributes,
};
use crate::windows::path::{stream_path, strip_verbatim, temp_file_name};
use crate::windows::reparse::{junction_data, parse_link};
use crate::windows::replace::{is_retryable, replace_error};
use crate::windows::retry::{self, DEFAULT_SHARING_RETRY, RetryPolicy};
use crate::windows::security::SecurityDescriptor;
use crate::windows::volume::{features, fs_type_name, is_read_only, volume_guid_path, volume_kind};
use nurt_contracts::env::Os;

/// Attempts to find a free temporary name before reporting a conflict.
const TEMP_NAME_ATTEMPTS: u32 = 16;
/// Read buffer of content hashing.
const HASH_BUFFER_LEN: usize = 64 * 1024;
const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;

fn follow_flags(follow: Follow) -> u32 {
    match follow {
        Follow::Follow => FILE_FLAG_BACKUP_SEMANTICS,
        Follow::NoFollow => FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    }
}

fn failure(kind: FsErrorKind, op: &str, path: &Path) -> FsError {
    FsError::new(kind, op, Some(path))
}

/// Link target in Windows form: `/` separators become `\`, which symbolic
/// links require.
fn link_target_units(target: &Path) -> Option<Vec<u16>> {
    let units: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .map(|unit| if unit == SLASH { BACKSLASH } else { unit })
        .collect();
    (!units.contains(&0)).then_some(units)
}

/// File system back end of Windows.
///
/// Every system call receives a verbatim path (`\\?\C:\…`,
/// `\\?\UNC\server\share\…`), so reserved names such as `NUL.txt` and paths
/// longer than `MAX_PATH` are handled. Handles are opened with
/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
///
/// Sharing and lock violations are retried with growing intervals for the
/// period given at construction (`fs.sharing_retry_ms`); afterwards the
/// operation fails with [`FsErrorKind::InUse`] and the holders reported by the
/// Restart Manager.
///
/// Renames that replace use POSIX semantics, with `MoveFileExW` and
/// write-through as fallback, and never copy across volumes. Name exchange is
/// unsupported; the states after a failed `ReplaceFileW` are described in
/// [`crate::windows::replace`]. `create_dir` ignores permission bits, `set_meta`
/// follows links and `restore_meta` changes the entry itself. `sync_dir`
/// succeeds where directories cannot be flushed, because NTFS journals its
/// metadata.
#[derive(Debug)]
pub struct WindowsFs {
    retry: RetryPolicy,
    roots: Mutex<HashMap<VolumeId, Vec<u16>>>,
    sequence: AtomicU64,
}

impl WindowsFs {
    /// Back end that retries sharing violations for 2000 ms.
    #[must_use]
    pub fn new() -> Self {
        Self::with_sharing_retry(DEFAULT_SHARING_RETRY)
    }

    /// Back end that retries sharing violations for `period`
    /// (`fs.sharing_retry_ms`); a zero period disables retries.
    #[must_use]
    pub fn with_sharing_retry(period: Duration) -> Self {
        Self {
            retry: RetryPolicy::new(period),
            roots: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(0),
        }
    }

    /// Retry period after sharing violations.
    pub fn sharing_retry(&self) -> Duration {
        self.retry.budget()
    }

    fn retrying<T>(&self, attempt: impl FnMut() -> Result<T, u32>) -> Result<T, u32> {
        retry::run(&self.retry, thread::sleep, attempt, |code| {
            is_sharing_conflict(*code)
        })
    }

    fn holders_of(path: &AbsPath, also: &[&AbsPath]) -> Vec<Holder> {
        let wides: Vec<WidePath> = std::iter::once(path)
            .chain(also.iter().copied())
            .filter_map(|path| WidePath::of(path).ok())
            .collect();
        let refs: Vec<&WidePath> = wides.iter().collect();
        restart::holders(&refs)
    }

    /// Classified error of a failed call on `path`; a missing path below a
    /// file becomes [`FsErrorKind::NotADirectory`], and an entry in use
    /// carries the processes holding `path` and `also`.
    fn error(code: u32, op: &str, path: &AbsPath, also: &[&AbsPath]) -> FsError {
        let mut error = os_error(code, op, Some(path.as_path()));
        if code == ERROR_PATH_NOT_FOUND && has_file_ancestor(path.as_path()) {
            error.kind = FsErrorKind::NotADirectory;
        }
        if error.kind == FsErrorKind::InUse {
            error.holders = Self::holders_of(path, also);
        }
        error
    }

    fn open_wide(
        &self,
        wide: &WidePath,
        path: &AbsPath,
        op: &str,
        (access, disposition, flags): (u32, u32, u32),
    ) -> FsResult<File> {
        self.retrying(|| create_file(wide, access, disposition, flags))
            .map_err(|code| Self::error(code, op, path, &[]))
    }

    fn open(
        &self,
        path: &AbsPath,
        op: &str,
        access: u32,
        disposition: u32,
        flags: u32,
    ) -> FsResult<File> {
        self.open_wide(&WidePath::of(path)?, path, op, (access, disposition, flags))
    }

    fn file_meta(file: &File, op: &str, path: &AbsPath) -> FsResult<FileMeta> {
        raw_info(file)
            .map(|raw| raw.to_meta())
            .map_err(|code| Self::error(code, op, path, &[]))
    }

    /// Refuses a handle opened on a symbolic link or a junction, as the
    /// back ends of Linux and macOS refuse them with `O_NOFOLLOW`.
    fn refuse_link(file: &File, op: &str, path: &AbsPath) -> FsResult<()> {
        match Self::file_meta(file, op, path)?.kind {
            FileKind::Symlink | FileKind::Junction => {
                Err(failure(FsErrorKind::Loop, op, path.as_path()))
            }
            _ => Ok(()),
        }
    }

    fn create_writable(
        &self,
        path: &AbsPath,
        op: &str,
        disposition: u32,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let file = self.open(
            path,
            op,
            GENERIC_READ | GENERIC_WRITE,
            disposition,
            FILE_ATTRIBUTE_NORMAL,
        )?;
        Ok(Box::new(WinWrite::new(file, path.clone())))
    }

    fn delete(&self, path: &AbsPath, op: &str) -> FsResult<()> {
        let wide = WidePath::of(path)?;
        self.retrying(|| ops::delete_entry(&wide))
            .map_err(|code| Self::error(code, op, path, &[]))
    }

    /// Error for a stream request: [`FsErrorKind::Unsupported`] on volumes
    /// without named streams.
    fn stream_error(&self, error: FsError, path: &AbsPath) -> FsError {
        let streams = self
            .volume(path)
            .map_or(true, |volume| volume.features.alternate_streams);
        if streams {
            error
        } else {
            failure(FsErrorKind::Unsupported, &error.op, path.as_path())
        }
    }

    fn stream_wide(path: &AbsPath, stream: &str, op: &str) -> FsResult<WidePath> {
        let file = WidePath::of(path)?;
        stream_path(file.units(), stream)
            .map(|units| WidePath::from_units(&units))
            .ok_or_else(|| failure(FsErrorKind::InvalidPath, op, path.as_path()))
    }

    fn copy_content(&self, src: &AbsPath, dst: &AbsPath, target: &mut File) -> FsResult<u64> {
        let op = "copy_file";
        let mut source = self.open(
            src,
            op,
            GENERIC_READ,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        )?;
        if Self::file_meta(&source, op, src)?.kind == FileKind::Directory {
            return Err(failure(FsErrorKind::IsADirectory, op, src.as_path()));
        }
        let io_error = |error: std::io::Error| from_io(&error, op, Some(dst.as_path()));
        let mut bytes = std::io::copy(&mut source, target).map_err(io_error)?;
        target.sync_all().map_err(io_error)?;
        let streams =
            list_streams(&WidePath::of(src)?).map_err(|code| Self::error(code, op, src, &[]))?;
        for stream in streams {
            let from = Self::stream_wide(src, &stream.name, op)?;
            let to = Self::stream_wide(dst, &stream.name, op)?;
            let mut reader = self.open_wide(&from, src, op, (GENERIC_READ, OPEN_EXISTING, 0))?;
            let mut writer = self.open_wide(&to, dst, op, (GENERIC_WRITE, CREATE_NEW, 0))?;
            bytes += std::io::copy(&mut reader, &mut writer).map_err(io_error)?;
            writer.sync_all().map_err(io_error)?;
        }
        Ok(bytes)
    }

    fn set_basic(&self, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        let op = "set_meta";
        let file = self.open(
            path,
            op,
            FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        )?;
        let current = basic_info(&file).map_err(|code| Self::error(code, op, path, &[]))?;
        let mut attributes = current.FileAttributes;
        if let Some(mode) = change.mode {
            attributes = attributes_for_mode(attributes, mode);
        }
        attributes =
            (attributes | change.set_windows_attributes) & !change.clear_windows_attributes;
        let info = FILE_BASIC_INFO {
            CreationTime: change.created_ns.map_or(0, unix_ns_to_filetime),
            LastAccessTime: change.accessed_ns.map_or(0, unix_ns_to_filetime),
            LastWriteTime: change.modified_ns.map_or(0, unix_ns_to_filetime),
            ChangeTime: 0,
            FileAttributes: basic_info_attributes(attributes),
        };
        set_basic_info(&file, &info).map_err(|code| Self::error(code, op, path, &[]))
    }

    fn final_path(file: &File, path: &AbsPath) -> FsResult<AbsPath> {
        let mut buffer = vec![0_u16; MAX_PATH as usize + 1];
        loop {
            let capacity = u32::try_from(buffer.len()).map_err(|_| {
                os_error(ERROR_INSUFFICIENT_BUFFER, "real_path", Some(path.as_path()))
            })?;
            // SAFETY: the handle is open and `buffer` is writable for
            // `capacity` characters.
            let len = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    capacity,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            };
            if len == 0 {
                return Err(Self::error(last_error(), "real_path", path, &[]));
            }
            let len = usize::try_from(len).unwrap_or(usize::MAX);
            if len < buffer.len() {
                buffer.truncate(len);
                break;
            }
            buffer.resize(len.saturating_add(1), 0);
        }
        AbsPath::new(PathBuf::from(OsString::from_wide(&strip_verbatim(&buffer))))
    }

    fn nonce() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| u64::from(elapsed.subsec_nanos()))
    }
}

impl Default for WindowsFs {
    fn default() -> Self {
        Self::new()
    }
}

impl Fs for WindowsFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let file = self.open(
            path,
            "metadata",
            FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            follow_flags(follow),
        )?;
        Self::file_meta(&file, "metadata", path)
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        let op = "extended_metadata";
        let basic = self.metadata(path, follow)?;
        let wide = WidePath::of(path)?;
        let acl = security::read_descriptor(&wide, follow_flags(follow))
            .map_err(|code| Self::error(code, op, path, &[]))?;
        let account_sid = acl
            .as_deref()
            .and_then(|bytes| SecurityDescriptor::parse(bytes).ok())
            .and_then(|descriptor| descriptor.account_sid());
        let is_link = matches!(basic.kind, FileKind::Symlink | FileKind::Junction);
        let streams = if is_link {
            Vec::new()
        } else {
            list_streams(&wide).map_err(|code| Self::error(code, op, path, &[]))?
        };
        let link_target = if is_link {
            Some(self.read_link(path)?)
        } else {
            None
        };
        Ok(ExtendedMeta {
            basic,
            xattrs: Vec::new(),
            acl,
            account_sid,
            flags: None,
            streams,
            link_target,
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let file = self.open(
            path,
            "open_read",
            GENERIC_READ,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        )?;
        if Self::file_meta(&file, "open_read", path)?.kind == FileKind::Directory {
            return Err(failure(
                FsErrorKind::IsADirectory,
                "open_read",
                path.as_path(),
            ));
        }
        Ok(Box::new(WinRead::new(file)))
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        let op = "open_stream_read";
        let wide = Self::stream_wide(path, stream, op)?;
        self.open_wide(&wide, path, op, (GENERIC_READ, OPEN_EXISTING, 0))
            .map(|file| Box::new(WinRead::new(file)) as Box<dyn ReadHandle>)
            .map_err(|error| self.stream_error(error, path))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        if self.metadata(path, Follow::Follow)?.kind != FileKind::Directory {
            return Err(failure(
                FsErrorKind::NotADirectory,
                "read_dir",
                path.as_path(),
            ));
        }
        let entries = list_dir(&WidePath::of(path)?)
            .map_err(|code| Self::error(code, "read_dir", path, &[]))?;
        let dir = path.as_path().to_path_buf();
        Ok(Box::new(entries.map(move |entry| {
            entry
                .map(|entry| DirEntry {
                    kind: Some(classify_kind(entry.attributes, entry.reparse_tag)),
                    name: OsString::from_wide(&entry.name),
                })
                .map_err(|code| os_error(code, "read_dir", Some(&dir)))
        })))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        let data = ops::read_reparse(&WidePath::of(path)?)
            .map_err(|code| Self::error(code, "read_link", path, &[]))?;
        let link = parse_link(&data)
            .ok_or_else(|| failure(FsErrorKind::InvalidPath, "read_link", path.as_path()))?;
        Ok(PathBuf::from(OsString::from_wide(&link.target)))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let file = self.open(
            path,
            "real_path",
            FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        )?;
        Self::final_path(&file, path)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let mut reader = self.open_read(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; HASH_BUFFER_LEN];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| from_io(&error, "content_hash", Some(path.as_path())))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(ContentHash(hasher.finalize().into()))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        let op = "volume";
        let existing = path
            .as_path()
            .ancestors()
            .find(|ancestor| {
                WidePath::verbatim(ancestor)
                    .ok()
                    .and_then(|wide| attributes_of(&wide))
                    .is_some()
            })
            .ok_or_else(|| failure(FsErrorKind::NotFound, op, path.as_path()))?;
        let raw = volume::describe(&WidePath::verbatim(existing)?)
            .map_err(|code| os_error(code, op, Some(path.as_path())))?;
        let fs_type = fs_type_name(&raw.fs_name);
        let kind = volume_kind(raw.drive_type);
        let id = VolumeId {
            dev: raw.serial,
            uuid: raw.guid.clone(),
            subvolume: 0,
        };
        let mount = AbsPath::new(PathBuf::from(OsString::from_wide(&raw.root)))?;
        self.roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), raw.verbatim_root.units().to_vec());
        Ok(VolumeInfo {
            features: features(&fs_type, raw.flags, kind),
            rotational: raw.guid.as_deref().is_some_and(volume::incurs_seek_penalty),
            read_only: is_read_only(raw.flags),
            id,
            mount,
            fs_type,
            kind,
        })
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        let known = self
            .roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(volume)
            .cloned();
        let root = match (known, &volume.uuid) {
            (Some(units), _) => WidePath::from_units(&units),
            (None, Some(guid)) => {
                WidePath::from_units(&volume_guid_path(guid).encode_utf16().collect::<Vec<_>>())
            }
            (None, None) => return Err(FsError::new(FsErrorKind::NotFound, "space", None)),
        };
        volume::disk_space(&root)
            .map_err(|code| os_error(code, "space", Some(root.to_path_buf().as_path())))
    }

    /// Volumes are treated as case-insensitive (see
    /// [`crate::windows::volume::features`]).
    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        Ok(crate::key::path_key(Os::Windows, false, path.as_path()))
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        self.metadata(path, Follow::NoFollow)?;
        Ok(Self::holders_of(path, &[]))
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        if self.metadata(dir, Follow::Follow)?.kind != FileKind::Directory {
            return Err(failure(
                FsErrorKind::NotADirectory,
                "can_write_dir",
                dir.as_path(),
            ));
        }
        match create_file(
            &WidePath::of(dir)?,
            FILE_ADD_FILE,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        ) {
            Ok(_) => Ok(true),
            Err(ERROR_ACCESS_DENIED | ERROR_WRITE_PROTECT) => Ok(false),
            Err(code) => Err(Self::error(code, "can_write_dir", dir, &[])),
        }
    }
}

impl FsWrite for WindowsFs {
    fn create_temp_in(
        &self,
        _scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        for _ in 0..TEMP_NAME_ATTEMPTS {
            let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
            let name = temp_file_name(name_hint, process::id(), sequence, Self::nonce());
            match self.create_writable(&dir.join_name(&name)?, "create_temp_in", CREATE_NEW) {
                Err(error) if error.kind == FsErrorKind::Exists => {}
                other => return other,
            }
        }
        Err(failure(
            FsErrorKind::Exists,
            "create_temp_in",
            dir.as_path(),
        ))
    }

    fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.create_writable(path, "create_new", CREATE_NEW)
    }

    fn open_in_place(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let file = self.open(
            path,
            "open_in_place",
            GENERIC_READ | GENERIC_WRITE,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        Self::refuse_link(&file, "open_in_place", path)?;
        Ok(Box::new(WinWrite::new(file, path.clone())))
    }

    /// Creates the stream or truncates it; the file itself must exist.
    fn open_stream_write(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let op = "open_stream_write";
        self.metadata(path, Follow::Follow)?;
        let wide = Self::stream_wide(path, stream, op)?;
        let file = self
            .open_wide(
                &wide,
                path,
                op,
                (GENERIC_READ | GENERIC_WRITE, CREATE_ALWAYS, 0),
            )
            .map_err(|error| self.stream_error(error, path))?;
        Ok(Box::new(WinWrite::new(file, path.clone())))
    }

    fn rename(
        &self,
        _scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        let (source, destination) = (WidePath::of(from)?, WidePath::of(to)?);
        let result = match mode {
            RenameMode::Replace => {
                self.retrying(|| match ops::rename_posix(&source, &destination) {
                    Err(code) if is_unsupported_request(code) => {
                        ops::move_file(&source, &destination, true)
                    }
                    other => other,
                })
            }
            RenameMode::NoReplace => self.retrying(|| ops::move_file(&source, &destination, false)),
        };
        result.map_err(|code| Self::error(code, "rename", from, &[to]))
    }

    fn exchange(&self, _scope: &TxScope, a: &AbsPath, _b: &AbsPath) -> FsResult<()> {
        Err(failure(FsErrorKind::Unsupported, "exchange", a.as_path()))
    }

    fn replace_with_backup(
        &self,
        _scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        let (t, r, b) = (
            WidePath::of(target)?,
            WidePath::of(replacement)?,
            WidePath::of(backup)?,
        );
        retry::run(
            &self.retry,
            thread::sleep,
            || ops::replace_file(&t, &r, &b),
            |code| is_retryable(*code),
        )
        .map_err(|code| {
            let mut error = replace_error(code, Some(target.as_path()));
            if error.kind == FsErrorKind::InUse {
                error.holders = Self::holders_of(target, &[replacement]);
            }
            error
        })
    }

    fn hard_link(&self, _scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        let (source, link) = (WidePath::of(existing)?, WidePath::of(new)?);
        self.retrying(|| ops::hard_link(&source, &link))
            .map_err(|code| Self::error(code, "hard_link", new, &[existing]))
    }

    /// Block cloning with `FSCTL_DUPLICATE_EXTENTS_TO_FILE` on `ReFS`; other
    /// volumes fail with [`FsErrorKind::Unsupported`] and nothing is copied.
    fn clone_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        if !self.volume(src)?.features.reflink {
            return Err(failure(
                FsErrorKind::Unsupported,
                "clone_file",
                src.as_path(),
            ));
        }
        if self.metadata(dst, Follow::NoFollow).is_ok() {
            return Err(failure(FsErrorKind::Exists, "clone_file", dst.as_path()));
        }
        reflink_copy::reflink(
            WidePath::of(src)?.to_path_buf(),
            WidePath::of(dst)?.to_path_buf(),
        )
        .map_err(|error| from_io(&error, "clone_file", Some(dst.as_path())))
    }

    /// Copies the default stream and every named stream and flushes the
    /// copy; returns the sum of the stream sizes. A failed copy is removed.
    fn copy_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        let mut target = self.open(
            dst,
            "copy_file",
            GENERIC_READ | GENERIC_WRITE,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
        )?;
        let result = self.copy_content(src, dst, &mut target);
        if result.is_err() {
            drop(target);
            // The copy was created by this call; the copy error is reported.
            let _ = self.delete(dst, "copy_file");
        }
        result
    }

    fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        if self.metadata(path, Follow::NoFollow)?.kind == FileKind::Directory {
            return Err(failure(
                FsErrorKind::IsADirectory,
                "remove_file",
                path.as_path(),
            ));
        }
        self.delete(path, "remove_file")
    }

    fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        if self.metadata(path, Follow::NoFollow)?.kind != FileKind::Directory {
            return Err(failure(
                FsErrorKind::NotADirectory,
                "remove_dir",
                path.as_path(),
            ));
        }
        self.delete(path, "remove_dir")
    }

    fn remove_tree(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        self.remove_tree_at(path, parallelism)
    }

    fn create_dir(&self, _scope: &TxScope, path: &AbsPath, _mode: u32) -> FsResult<()> {
        let wide = WidePath::of(path)?;
        self.retrying(|| ops::create_directory(&wide))
            .map_err(|code| Self::error(code, "create_dir", path, &[]))
    }

    fn create_link(
        &self,
        _scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        let op = "create_link";
        let link_wide = WidePath::of(link)?;
        let units = link_target_units(target)
            .ok_or_else(|| failure(FsErrorKind::InvalidPath, op, target))?;
        let directory = match kind {
            LinkKind::SymlinkFile => false,
            LinkKind::SymlinkDirectory => true,
            LinkKind::Junction => {
                let data = junction_data(&units)
                    .ok_or_else(|| failure(FsErrorKind::InvalidPath, op, target))?;
                ops::create_directory(&link_wide)
                    .map_err(|code| Self::error(code, op, link, &[]))?;
                return ops::write_reparse(&link_wide, &data).map_err(|code| {
                    // The directory was created by this call; the reparse error is reported.
                    let _ = ops::delete_entry(&link_wide);
                    Self::error(code, op, link, &[])
                });
            }
        };
        ops::symlink(&link_wide, &WidePath::from_units(&units), directory)
            .map_err(|code| Self::error(code, op, link, &[]))
    }

    fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        let op = "set_meta";
        let unsupported = change.uid.is_some()
            || change.gid.is_some()
            || !change.set_xattrs.is_empty()
            || !change.remove_xattrs.is_empty()
            || unsettable_attributes(
                change.set_windows_attributes | change.clear_windows_attributes,
            ) != 0;
        if unsupported {
            return Err(failure(FsErrorKind::Unsupported, op, path.as_path()));
        }
        if let Some(acl) = &change.acl {
            let (refused, _) =
                security::apply_descriptor(&WidePath::of(path)?, FILE_FLAG_BACKUP_SEMANTICS, acl)
                    .map_err(|code| Self::error(code, op, path, &[]))?;
            if let Some((_, code)) = refused.first() {
                return Err(Self::error(*code, op, path, &[]));
            }
        }
        let basic_change = change.mode.is_some()
            || change.modified_ns.is_some()
            || change.accessed_ns.is_some()
            || change.created_ns.is_some()
            || change.set_windows_attributes != 0
            || change.clear_windows_attributes != 0;
        if basic_change {
            self.set_basic(path, change)?;
        }
        Ok(())
    }

    fn restore_meta(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        security::restore_metadata(path, meta)
    }

    fn truncate(&self, _scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        let file = self.open(
            path,
            "truncate",
            GENERIC_WRITE | FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        Self::refuse_link(&file, "truncate", path)?;
        file.set_len(length)
            .map_err(|error| from_io(&error, "truncate", Some(path.as_path())))
    }

    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        let tolerated = |code: u32| code == ERROR_ACCESS_DENIED || is_unsupported_request(code);
        match create_file(
            &WidePath::of(dir)?,
            GENERIC_READ | GENERIC_WRITE,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        ) {
            Ok(file) => match file.sync_all() {
                Err(error)
                    if !error
                        .raw_os_error()
                        .is_some_and(|code| tolerated(code.cast_unsigned())) =>
                {
                    Err(from_io(&error, "sync_dir", Some(dir.as_path())))
                }
                _ => Ok(()),
            },
            Err(code) if tolerated(code) => self.metadata(dir, Follow::Follow).map(|_| ()),
            Err(code) => Err(Self::error(code, "sync_dir", dir, &[])),
        }
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        let op = "lock_exclusive";
        let file = self.open(
            path,
            op,
            GENERIC_READ | GENERIC_WRITE,
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
        )?;
        let locked = retry::run(
            &RetryPolicy::new(timeout),
            thread::sleep,
            || file.try_lock(),
            |error| matches!(error, TryLockError::WouldBlock),
        );
        match locked {
            Ok(()) => Ok(Box::new(WinLock::new(file, path.clone()))),
            Err(TryLockError::WouldBlock) => {
                let mut error = failure(FsErrorKind::InUse, op, path.as_path());
                error.holders = Self::holders_of(path, &[]);
                Err(error)
            }
            Err(TryLockError::Error(error)) => Err(from_io(&error, op, Some(path.as_path()))),
        }
    }

    fn mark_hidden(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let wide = WidePath::of(path)?;
        let current = attributes_of(&wide)
            .ok_or_else(|| Self::error(last_error(), "mark_hidden", path, &[]))?;
        ops::set_attributes(
            &wide,
            basic_info_attributes(current | JOURNAL_DIRECTORY_ATTRIBUTES),
        )
        .map_err(|code| Self::error(code, "mark_hidden", path, &[]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_link_targets_to_backslashes() {
        let units = link_target_units(Path::new("../dir/file")).unwrap();
        assert_eq!(String::from_utf16_lossy(&units), r"..\dir\file");
        assert_eq!(link_target_units(Path::new("a\0b")), None);
    }

    #[test]
    fn reports_the_configured_retry_period() {
        assert_eq!(WindowsFs::new().sharing_retry(), DEFAULT_SHARING_RETRY);
        let fs = WindowsFs::with_sharing_retry(Duration::from_millis(5));
        assert_eq!(fs.sharing_retry(), Duration::from_millis(5));
    }
}
