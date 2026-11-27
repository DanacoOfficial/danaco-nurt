// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::sys::ops
// Responsibility:   Single system calls that change names; each returns the Win32 code of a failure.
// Dependencies:     windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Single system calls that change names; each returns the Win32 code of a
//! failure.

use std::ffi::c_void;
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::io::AsRawHandle;
use std::ptr;

use windows_sys::Win32::Foundation::GENERIC_WRITE;
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateHardLinkW, CreateSymbolicLinkW, DELETE, FILE_BASIC_INFO,
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX,
    FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_WRITE_ATTRIBUTES, FileDispositionInfo,
    FileDispositionInfoEx, FileRenameInfoEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    MoveFileExW, OPEN_EXISTING, REPLACEFILE_WRITE_THROUGH, ReplaceFileW,
    SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE, SYMBOLIC_LINK_FLAG_DIRECTORY, SYNCHRONIZE,
    SetFileAttributesW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT};

use super::handle::{
    ENTRY_FLAGS, WidePath, aligned_words, basic_info, create_file, last_error, set_basic_info,
    set_info, words_to_bytes,
};
use crate::windows::error::codes::{
    ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER,
};
use crate::windows::error::is_unsupported_request;
use crate::windows::meta::{attributes, basic_info_attributes};
use crate::windows::reparse::MAXIMUM_REPARSE_DATA_BUFFER_SIZE;

/// `FILE_RENAME_FLAG_REPLACE_IF_EXISTS` of `FILE_RENAME_INFO`.
const FILE_RENAME_FLAG_REPLACE_IF_EXISTS: u32 = 0x0000_0001;
/// `FILE_RENAME_FLAG_POSIX_SEMANTICS` of `FILE_RENAME_INFO`.
const FILE_RENAME_FLAG_POSIX_SEMANTICS: u32 = 0x0000_0002;
/// Size of a reparse buffer word.
const WORD: usize = 4;

fn check(ok: bool) -> Result<(), u32> {
    if ok { Ok(()) } else { Err(last_error()) }
}

fn open_for_delete(path: &WidePath) -> Result<std::fs::File, u32> {
    match create_file(
        path,
        DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
        OPEN_EXISTING,
        ENTRY_FLAGS,
    ) {
        Err(ERROR_ACCESS_DENIED) => create_file(
            path,
            DELETE | FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            ENTRY_FLAGS,
        ),
        other => other,
    }
}

/// Removes the entry itself (file, empty directory, link or junction).
///
/// POSIX semantics remove the name when the handle closes, even while other
/// handles opened with `FILE_SHARE_DELETE` keep the file, and ignore the
/// read-only attribute. Systems without them fall back to the classic
/// disposition after clearing the read-only attribute, which is set again
/// when the removal fails.
pub(crate) fn delete_entry(path: &WidePath) -> Result<(), u32> {
    let file = open_for_delete(path)?;
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    // SAFETY: `FileDispositionInfoEx` reads a `FILE_DISPOSITION_INFO_EX`.
    let result = unsafe {
        set_info(
            &file,
            FileDispositionInfoEx,
            &raw const info,
            size_of::<FILE_DISPOSITION_INFO_EX>(),
        )
    };
    match result {
        Err(code) if is_unsupported_request(code) => {}
        other => return other,
    }
    let basic = basic_info(&file)?;
    let read_only = basic.FileAttributes & attributes::READONLY != 0;
    let with_attributes = |value: u32| FILE_BASIC_INFO {
        FileAttributes: basic_info_attributes(value),
        ..FILE_BASIC_INFO::default()
    };
    if read_only {
        set_basic_info(
            &file,
            &with_attributes(basic.FileAttributes & !attributes::READONLY),
        )?;
    }
    let legacy = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: `FileDispositionInfo` reads a `FILE_DISPOSITION_INFO`.
    let result = unsafe {
        set_info(
            &file,
            FileDispositionInfo,
            &raw const legacy,
            size_of::<FILE_DISPOSITION_INFO>(),
        )
    };
    if result.is_err() && read_only {
        // Best effort: the removal error is the one reported.
        let _ = set_basic_info(&file, &with_attributes(basic.FileAttributes));
    }
    result
}

/// Renames by handle with POSIX semantics, replacing an existing
/// destination even while it is open with `FILE_SHARE_DELETE`.
///
/// Fails with a code accepted by
/// [`is_unsupported_request`] on systems and file systems without
/// `FileRenameInfoEx`.
pub(crate) fn rename_posix(from: &WidePath, to: &WidePath) -> Result<(), u32> {
    let file = create_file(from, DELETE | SYNCHRONIZE, OPEN_EXISTING, ENTRY_FLAGS)?;
    let name = to.units();
    let name_offset = offset_of!(FILE_RENAME_INFO, FileName);
    let size = name_offset + (name.len() + 1) * size_of::<u16>();
    let capacity = size.max(size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![0_u64; capacity.div_ceil(size_of::<u64>())];
    let name_bytes = u32::try_from(size_of_val(name)).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the zeroed buffer is aligned to eight bytes and holds at least
    // `capacity` bytes, enough for the structure and the name with its
    // terminator; every write stays inside the buffer.
    unsafe {
        (*info).Anonymous.Flags =
            FILE_RENAME_FLAG_POSIX_SEMANTICS | FILE_RENAME_FLAG_REPLACE_IF_EXISTS;
        (*info).RootDirectory = ptr::null_mut();
        (*info).FileNameLength = name_bytes;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            (&raw mut (*info).FileName).cast::<u16>(),
            name.len(),
        );
    }
    // SAFETY: `FileRenameInfoEx` reads a `FILE_RENAME_INFO` followed by the
    // name; `size` bytes of it are initialised in `buffer`.
    unsafe { set_info(&file, FileRenameInfoEx, buffer.as_ptr(), size) }
}

/// `MoveFileExW` with write-through, replacing the destination when
/// `replace` is set; never copies across volumes.
pub(crate) fn move_file(from: &WidePath, to: &WidePath, replace: bool) -> Result<(), u32> {
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: both paths are zero-terminated and outlive the call.
    check(unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } != 0)
}

/// `ReplaceFileW` with write-through and a backup name.
pub(crate) fn replace_file(
    target: &WidePath,
    replacement: &WidePath,
    backup: &WidePath,
) -> Result<(), u32> {
    // SAFETY: the three paths are zero-terminated and outlive the call; the
    // exclusion and reserved arguments must be null.
    check(
        unsafe {
            ReplaceFileW(
                target.as_ptr(),
                replacement.as_ptr(),
                backup.as_ptr(),
                REPLACEFILE_WRITE_THROUGH,
                ptr::null(),
                ptr::null(),
            )
        } != 0,
    )
}

/// `CreateHardLinkW`.
pub(crate) fn hard_link(existing: &WidePath, new: &WidePath) -> Result<(), u32> {
    // SAFETY: both paths are zero-terminated; null security attributes are
    // required.
    check(unsafe { CreateHardLinkW(new.as_ptr(), existing.as_ptr(), ptr::null()) } != 0)
}

/// `CreateDirectoryW` with inherited security.
pub(crate) fn create_directory(path: &WidePath) -> Result<(), u32> {
    // SAFETY: the path is zero-terminated; null security attributes select the
    // inherited descriptor.
    check(unsafe { CreateDirectoryW(path.as_ptr(), ptr::null()) } != 0)
}

/// `SetFileAttributesW`.
pub(crate) fn set_attributes(path: &WidePath, value: u32) -> Result<(), u32> {
    // SAFETY: the path is zero-terminated.
    check(unsafe { SetFileAttributesW(path.as_ptr(), value) } != 0)
}

/// `CreateSymbolicLinkW`, also without the privilege when developer mode
/// allows it. Systems that do not know the unprivileged flag are retried
/// without it.
pub(crate) fn symlink(link: &WidePath, target: &WidePath, directory: bool) -> Result<(), u32> {
    let kind = if directory {
        SYMBOLIC_LINK_FLAG_DIRECTORY
    } else {
        0
    };
    let create = |flags: u32| {
        // SAFETY: both strings are zero-terminated and outlive the call.
        check(unsafe { CreateSymbolicLinkW(link.as_ptr(), target.as_ptr(), flags) })
    };
    match create(kind | SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE) {
        Err(ERROR_INVALID_PARAMETER) => create(kind),
        other => other,
    }
}

/// Reparse data of the entry itself.
pub(crate) fn read_reparse(path: &WidePath) -> Result<Vec<u8>, u32> {
    let file = create_file(path, FILE_READ_ATTRIBUTES, OPEN_EXISTING, ENTRY_FLAGS)?;
    let mut buffer = vec![0_u32; MAXIMUM_REPARSE_DATA_BUFFER_SIZE / WORD];
    let capacity = u32::try_from(buffer.len() * WORD).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    let mut returned = 0_u32;
    // SAFETY: the handle is open; there is no input buffer; the output buffer
    // is writable for `capacity` bytes; the call is synchronous.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            ptr::null(),
            0,
            buffer.as_mut_ptr().cast::<c_void>(),
            capacity,
            &raw mut returned,
            ptr::null_mut(),
        )
    };
    check(ok != 0)?;
    Ok(words_to_bytes(
        &buffer,
        usize::try_from(returned).unwrap_or(0),
    ))
}

/// Writes reparse data to the existing entry itself.
pub(crate) fn write_reparse(path: &WidePath, data: &[u8]) -> Result<(), u32> {
    let file = create_file(path, GENERIC_WRITE, OPEN_EXISTING, ENTRY_FLAGS)?;
    let words = aligned_words(data);
    let len = u32::try_from(data.len()).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    let mut returned = 0_u32;
    // SAFETY: the handle is open for writing; the input buffer holds `len`
    // readable bytes; there is no output buffer; the call is synchronous.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            words.as_ptr().cast::<c_void>(),
            len,
            ptr::null_mut(),
            0,
            &raw mut returned,
            ptr::null_mut(),
        )
    };
    check(ok != 0)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
