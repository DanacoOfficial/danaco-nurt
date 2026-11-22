// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Handles, verbatim paths, file information and directory enumeration.

use std::ffi::{OsString, c_void};
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr;

use nurt_contracts::fs::{AbsPath, AlternateStream, FsError, FsErrorKind, FsResult};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_TAG_INFO, FILE_BASIC_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_INFO_BY_HANDLE_CLASS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_STANDARD_INFO, FIND_FIRST_EX_LARGE_FETCH, FileAttributeTagInfo, FileBasicInfo, FileIdInfo,
    FileStandardInfo, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindFirstStreamW, FindNextFileW, FindNextStreamW, FindStreamInfoStandard, GetFileAttributesW,
    GetFileInformationByHandle, GetFileInformationByHandleEx, INVALID_FILE_ATTRIBUTES,
    SetFileInformationByHandle, WIN32_FIND_DATAW, WIN32_FIND_STREAM_DATA,
};

use crate::windows::error::codes::{
    ERROR_FILE_NOT_FOUND, ERROR_HANDLE_EOF, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_NAME,
    ERROR_NO_MORE_FILES,
};
use crate::windows::error::is_unsupported_request;
use crate::windows::meta::{RawFileInfo, attributes};
use crate::windows::path::{decode_stream_name, until_nul, verbatim_wide};

/// Share mode of every handle opened by the back end: other programs keep
/// full access, including rename and delete.
pub(crate) const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

/// Flags that open the entry itself: directories and reparse points are
/// opened without following links.
pub(crate) const ENTRY_FLAGS: u32 = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;

const BACKSLASH: u16 = b'\\' as u16;
const ASTERISK: u16 = b'*' as u16;
const DOT: u16 = b'.' as u16;

/// Code of the last failed system call of the calling thread.
pub(crate) fn last_error() -> u32 {
    io::Error::last_os_error()
        .raw_os_error()
        .map_or(0, i32::cast_unsigned)
}

/// Zero-terminated UTF-16 path passed to system calls.
pub(crate) struct WidePath(Vec<u16>);

impl WidePath {
    /// Verbatim form of an absolute native path.
    pub(crate) fn verbatim(path: &Path) -> FsResult<Self> {
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        let mut out = verbatim_wide(&units)
            .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath, "verbatim_path", Some(path)))?;
        out.push(0);
        Ok(Self(out))
    }

    /// Verbatim form of an [`AbsPath`].
    pub(crate) fn of(path: &AbsPath) -> FsResult<Self> {
        Self::verbatim(path.as_path())
    }

    /// Path from code units, used as given up to the first zero.
    pub(crate) fn from_units(units: &[u16]) -> Self {
        let mut out = until_nul(units).to_vec();
        out.push(0);
        Self(out)
    }

    /// Pointer to the zero-terminated units.
    pub(crate) fn as_ptr(&self) -> *const u16 {
        self.0.as_ptr()
    }

    /// Code units without the terminator.
    pub(crate) fn units(&self) -> &[u16] {
        self.0.split_last().map_or(&[], |(_, units)| units)
    }

    /// Native path with the same units.
    pub(crate) fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(OsString::from_wide(self.units()))
    }
}

/// Opens or creates a file with [`SHARE_ALL`].
pub(crate) fn create_file(
    path: &WidePath,
    access: u32,
    disposition: u32,
    flags: u32,
) -> Result<File, u32> {
    // SAFETY: `path` is zero-terminated and outlives the call; a null security
    // attributes pointer and a null template handle are valid arguments.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            SHARE_ALL,
            ptr::null(),
            disposition,
            flags,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: `handle` is a valid handle that no other object owns; the
    // returned file closes it exactly once.
    Ok(File::from(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

/// Reads one information class of an open file.
///
/// # Safety
///
/// `T` must be the structure that `class` writes.
unsafe fn query<T: Default>(file: &File, class: FILE_INFO_BY_HANDLE_CLASS) -> Result<T, u32> {
    let mut value = T::default();
    let size = u32::try_from(size_of::<T>()).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    // SAFETY: the handle is open; `value` is writable for `size` bytes and
    // aligned for `T`, which the caller guarantees `class` writes.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            class,
            (&raw mut value).cast::<c_void>(),
            size,
        )
    };
    if ok == 0 {
        Err(last_error())
    } else {
        Ok(value)
    }
}

/// Writes one information class of an open file.
///
/// # Safety
///
/// `T` must be the structure that `class` reads, and `size` must not exceed
/// the bytes readable at `value`.
pub(crate) unsafe fn set_info<T>(
    file: &File,
    class: FILE_INFO_BY_HANDLE_CLASS,
    value: *const T,
    size: usize,
) -> Result<(), u32> {
    let size = u32::try_from(size).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    // SAFETY: the handle is open and the caller guarantees that `value` holds
    // `size` readable bytes of the structure expected by `class`.
    let ok = unsafe {
        SetFileInformationByHandle(file.as_raw_handle(), class, value.cast::<c_void>(), size)
    };
    if ok == 0 { Err(last_error()) } else { Ok(()) }
}

/// Basic information (times and attributes) of an open file.
pub(crate) fn basic_info(file: &File) -> Result<FILE_BASIC_INFO, u32> {
    // SAFETY: `FileBasicInfo` writes a `FILE_BASIC_INFO`.
    unsafe { query(file, FileBasicInfo) }
}

/// Sets times and attributes of an open file; zero fields stay unchanged.
pub(crate) fn set_basic_info(file: &File, info: &FILE_BASIC_INFO) -> Result<(), u32> {
    // SAFETY: `FileBasicInfo` reads a `FILE_BASIC_INFO`, which `info` holds.
    unsafe {
        set_info(
            file,
            FileBasicInfo,
            &raw const *info,
            size_of::<FILE_BASIC_INFO>(),
        )
    }
}

fn legacy_identity(file: &File) -> Result<(u64, [u8; 16]), u32> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open and `info` is a writable
    // `BY_HANDLE_FILE_INFORMATION`.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) };
    if ok == 0 {
        return Err(last_error());
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    let mut identifier = [0_u8; 16];
    identifier[..8].copy_from_slice(&index.to_le_bytes());
    Ok((u64::from(info.dwVolumeSerialNumber), identifier))
}

/// Information of an open file. File systems without 128-bit file ids (FAT,
/// exFAT) report the 64-bit file index in the low half.
pub(crate) fn raw_info(file: &File) -> Result<RawFileInfo, u32> {
    let basic = basic_info(file)?;
    // SAFETY: `FileStandardInfo` writes a `FILE_STANDARD_INFO`.
    let standard: FILE_STANDARD_INFO = unsafe { query(file, FileStandardInfo) }?;
    let reparse_tag = if basic.FileAttributes & attributes::REPARSE_POINT == 0 {
        0
    } else {
        // SAFETY: `FileAttributeTagInfo` writes a `FILE_ATTRIBUTE_TAG_INFO`.
        match unsafe { query::<FILE_ATTRIBUTE_TAG_INFO>(file, FileAttributeTagInfo) } {
            Ok(info) => info.ReparseTag,
            Err(code) if is_unsupported_request(code) => 0,
            Err(code) => return Err(code),
        }
    };
    // SAFETY: `FileIdInfo` writes a `FILE_ID_INFO`.
    let (volume_serial, file_id) = match unsafe { query::<FILE_ID_INFO>(file, FileIdInfo) } {
        Ok(info) => (info.VolumeSerialNumber, info.FileId.Identifier),
        Err(code) if is_unsupported_request(code) => legacy_identity(file)?,
        Err(code) => return Err(code),
    };
    Ok(RawFileInfo {
        attributes: basic.FileAttributes,
        reparse_tag,
        size: u64::try_from(standard.EndOfFile).unwrap_or(0),
        links: standard.NumberOfLinks,
        created: basic.CreationTime,
        accessed: basic.LastAccessTime,
        modified: basic.LastWriteTime,
        changed: basic.ChangeTime,
        volume_serial,
        file_id,
    })
}

/// Volume serial number of an open file, as used in file identities.
pub(crate) fn volume_serial(file: &File) -> Result<u64, u32> {
    // SAFETY: `FileIdInfo` writes a `FILE_ID_INFO`.
    match unsafe { query::<FILE_ID_INFO>(file, FileIdInfo) } {
        Ok(info) => Ok(info.VolumeSerialNumber),
        Err(code) if is_unsupported_request(code) => {
            legacy_identity(file).map(|(serial, _)| serial)
        }
        Err(code) => Err(code),
    }
}

/// Attributes of an entry by path, without following a final link; `None`
/// when the entry cannot be queried.
pub(crate) fn attributes_of(path: &WidePath) -> Option<u32> {
    // SAFETY: `path` is zero-terminated and outlives the call.
    let value = unsafe { GetFileAttributesW(path.as_ptr()) };
    (value != INVALID_FILE_ATTRIBUTES).then_some(value)
}

/// Whether the nearest existing ancestor of `path` is not a directory, which
/// turns a missing path into a non-directory component.
pub(crate) fn has_file_ancestor(path: &Path) -> bool {
    for ancestor in path.ancestors().skip(1) {
        let Ok(wide) = WidePath::verbatim(ancestor) else {
            return false;
        };
        if let Some(value) = attributes_of(&wide) {
            return value & attributes::DIRECTORY == 0;
        }
    }
    false
}

/// Copies bytes into 4-byte aligned storage, as security descriptors and
/// reparse buffers require.
pub(crate) fn aligned_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks(4)
        .map(|chunk| {
            let mut word = [0_u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            u32::from_ne_bytes(word)
        })
        .collect()
}

/// First `len` bytes of aligned storage in memory order.
pub(crate) fn words_to_bytes(words: &[u32], len: usize) -> Vec<u8> {
    words
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .take(len)
        .collect()
}

/// Search handle of `FindFirstFileExW` or `FindFirstStreamW`.
pub(crate) struct FindHandle(HANDLE);

// SAFETY: a search handle is not bound to the thread that opened it, and the
// owning value uses it from one thread at a time.
unsafe impl Send for FindHandle {}

impl Drop for FindHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by a successful find call and is
        // closed exactly once here; a failure leaves nothing to release.
        unsafe { FindClose(self.0) };
    }
}

/// Directory entry from an enumeration.
pub(crate) struct RawEntry {
    /// Name without the directory.
    pub(crate) name: Vec<u16>,
    /// `FILE_ATTRIBUTE_*` bits.
    pub(crate) attributes: u32,
    /// Reparse tag; zero unless the reparse-point attribute is set.
    pub(crate) reparse_tag: u32,
    /// Size of the default stream.
    pub(crate) size: u64,
}

/// Entries of a directory, excluding `.` and `..`.
pub(crate) struct DirEntries {
    handle: Option<FindHandle>,
    pending: Option<Box<WIN32_FIND_DATAW>>,
}

/// Starts enumerating `dir` with `FindFirstFileExW(FindExInfoBasic,
/// FIND_FIRST_EX_LARGE_FETCH)`. A search that finds nothing yields an empty
/// enumeration; callers check that the directory exists.
pub(crate) fn list_dir(dir: &WidePath) -> Result<DirEntries, u32> {
    let mut pattern = dir.units().to_vec();
    if pattern.last() != Some(&BACKSLASH) {
        pattern.push(BACKSLASH);
    }
    pattern.extend_from_slice(&[ASTERISK, 0]);
    let mut data = Box::new(WIN32_FIND_DATAW::default());
    // SAFETY: `pattern` is zero-terminated; `data` is a writable
    // `WIN32_FIND_DATAW`, the structure of `FindExInfoBasic`; the search filter
    // must be null for `FindExSearchNameMatch`.
    let handle = unsafe {
        FindFirstFileExW(
            pattern.as_ptr(),
            FindExInfoBasic,
            (&raw mut *data).cast::<c_void>(),
            FindExSearchNameMatch,
            ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let code = last_error();
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(DirEntries {
                handle: None,
                pending: None,
            });
        }
        return Err(code);
    }
    Ok(DirEntries {
        handle: Some(FindHandle(handle)),
        pending: Some(data),
    })
}

impl Iterator for DirEntries {
    type Item = Result<RawEntry, u32>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let data = if let Some(data) = self.pending.take() {
                data
            } else {
                let handle = self.handle.as_ref()?;
                let mut data = Box::new(WIN32_FIND_DATAW::default());
                // SAFETY: the search handle is open and `data` is a
                // writable `WIN32_FIND_DATAW`.
                let ok = unsafe { FindNextFileW(handle.0, &raw mut *data) };
                if ok == 0 {
                    let code = last_error();
                    self.handle = None;
                    return (code != ERROR_NO_MORE_FILES).then_some(Err(code));
                }
                data
            };
            let name = until_nul(&data.cFileName);
            if name == [DOT] || name == [DOT, DOT] {
                continue;
            }
            let reparse_tag = if data.dwFileAttributes & attributes::REPARSE_POINT == 0 {
                0
            } else {
                data.dwReserved0
            };
            return Some(Ok(RawEntry {
                name: name.to_vec(),
                attributes: data.dwFileAttributes,
                reparse_tag,
                size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
            }));
        }
    }
}

fn push_stream(out: &mut Vec<AlternateStream>, data: &WIN32_FIND_STREAM_DATA) -> Result<(), u32> {
    match decode_stream_name(&data.cStreamName) {
        Ok(Some(name)) => {
            out.push(AlternateStream {
                name,
                size: u64::try_from(data.StreamSize).unwrap_or(0),
            });
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(_) => Err(ERROR_INVALID_NAME),
    }
}

/// Named data streams of a file or directory, in the order of the file
/// system. Volumes without streams yield an empty list; a stream name that is
/// not valid UTF-16 fails with `ERROR_INVALID_NAME`.
pub(crate) fn list_streams(path: &WidePath) -> Result<Vec<AlternateStream>, u32> {
    let mut data = WIN32_FIND_STREAM_DATA::default();
    // SAFETY: `path` is zero-terminated; `data` is a writable
    // `WIN32_FIND_STREAM_DATA`, the structure of `FindStreamInfoStandard`; the
    // flags must be zero.
    let handle = unsafe {
        FindFirstStreamW(
            path.as_ptr(),
            FindStreamInfoStandard,
            (&raw mut data).cast::<c_void>(),
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let code = last_error();
        if code == ERROR_HANDLE_EOF || is_unsupported_request(code) {
            return Ok(Vec::new());
        }
        return Err(code);
    }
    let handle = FindHandle(handle);
    let mut out = Vec::new();
    loop {
        push_stream(&mut out, &data)?;
        // SAFETY: the search handle is open and `data` is a writable
        // `WIN32_FIND_STREAM_DATA`.
        let ok = unsafe { FindNextStreamW(handle.0, (&raw mut data).cast::<c_void>()) };
        if ok == 0 {
            let code = last_error();
            return if code == ERROR_HANDLE_EOF {
                Ok(out)
            } else {
                Err(code)
            };
        }
    }
}
