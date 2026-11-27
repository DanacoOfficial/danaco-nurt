// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::sys::volume
// Responsibility:   Volume roots, volume information and free space.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Volume roots, volume information and free space.

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::ptr;

use nurt_contracts::fs::SpaceInfo;
use windows_sys::Win32::Foundation::MAX_PATH;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, GetDiskFreeSpaceExW, GetDriveTypeW,
    GetVolumeInformationByHandleW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
};

use super::handle::{WidePath, create_file, last_error, volume_serial};
use crate::windows::error::codes::ERROR_INSUFFICIENT_BUFFER;
use crate::windows::path::{nul_terminated, strip_verbatim, until_nul, verbatim_wide};
use crate::windows::volume::{parse_volume_guid_path, space_info};

/// Characters of a volume GUID path including the terminator, with margin.
const VOLUME_NAME_CAPACITY: usize = 64;
/// Characters of a file system name buffer.
const FS_NAME_CAPACITY: usize = MAX_PATH as usize + 1;

/// Raw description of a volume.
pub(crate) struct RawVolume {
    /// Root without the verbatim prefix (`C:\`, `\\server\share\`).
    pub(crate) root: Vec<u16>,
    /// Verbatim root for system calls.
    pub(crate) verbatim_root: WidePath,
    /// Volume serial number, as used in file identities.
    pub(crate) serial: u64,
    /// File system name as reported by the system.
    pub(crate) fs_name: String,
    /// File system flags.
    pub(crate) flags: u32,
    /// Drive type.
    pub(crate) drive_type: u32,
    /// Lower-case volume GUID, when the volume has one.
    pub(crate) guid: Option<String>,
}

/// Verbatim root of the volume containing the existing entry `path`.
fn volume_root(path: &WidePath) -> Result<WidePath, u32> {
    let capacity = path.units().len().max(MAX_PATH as usize) + 1;
    let mut buffer = vec![0_u16; capacity];
    let len = u32::try_from(capacity).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    // SAFETY: `path` is zero-terminated and `buffer` is writable for `len`
    // characters.
    let ok = unsafe { GetVolumePathNameW(path.as_ptr(), buffer.as_mut_ptr(), len) };
    if ok == 0 {
        return Err(last_error());
    }
    let root = until_nul(&buffer);
    Ok(WidePath::from_units(
        &verbatim_wide(root).unwrap_or_else(|| root.to_vec()),
    ))
}

fn volume_guid(root: &[u16]) -> Option<String> {
    let root = WidePath::from_units(root);
    let mut buffer = [0_u16; VOLUME_NAME_CAPACITY];
    let len = u32::try_from(buffer.len()).ok()?;
    // SAFETY: `root` is zero-terminated with a trailing backslash and `buffer`
    // is writable for `len` characters.
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), buffer.as_mut_ptr(), len) };
    if ok == 0 {
        return None;
    }
    parse_volume_guid_path(&String::from_utf16_lossy(until_nul(&buffer)))
}

/// Describes the volume containing the existing entry `path`.
pub(crate) fn describe(path: &WidePath) -> Result<RawVolume, u32> {
    let verbatim_root = volume_root(path)?;
    let root = strip_verbatim(verbatim_root.units());
    let handle = create_file(
        &verbatim_root,
        FILE_READ_ATTRIBUTES,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
    )?;
    let mut serial32 = 0_u32;
    let mut max_component = 0_u32;
    let mut flags = 0_u32;
    let mut fs_name = [0_u16; FS_NAME_CAPACITY];
    let fs_capacity = u32::try_from(fs_name.len()).map_err(|_| ERROR_INSUFFICIENT_BUFFER)?;
    // SAFETY: the handle is open; the volume name buffer is null with size
    // zero; the counters and the file system name buffer of `fs_capacity`
    // characters are writable.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            handle.as_raw_handle(),
            ptr::null_mut(),
            0,
            &raw mut serial32,
            &raw mut max_component,
            &raw mut flags,
            fs_name.as_mut_ptr(),
            fs_capacity,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    let serial = volume_serial(&handle).unwrap_or_else(|_| u64::from(serial32));
    let plain_root = WidePath::from_units(&root);
    // SAFETY: `plain_root` is zero-terminated.
    let drive_type = unsafe { GetDriveTypeW(plain_root.as_ptr()) };
    Ok(RawVolume {
        guid: volume_guid(&root),
        root,
        verbatim_root,
        serial,
        fs_name: String::from_utf16_lossy(until_nul(&fs_name)),
        flags,
        drive_type,
    })
}

/// Whether the device behind the volume incurs a seek penalty (rotational
/// medium). Unknown answers, for example for volumes spanning several disks
/// or without the right to query the device, are reported as `false`.
pub(crate) fn incurs_seek_penalty(guid: &str) -> bool {
    let device = WidePath::from_units(&nul_terminated(&format!(r"\\?\Volume{{{guid}}}")));
    let Ok(file) = create_file(&device, 0, OPEN_EXISTING, 0) else {
        return false;
    };
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
    let mut returned = 0_u32;
    let (Ok(query_len), Ok(descriptor_len)) = (
        u32::try_from(size_of::<STORAGE_PROPERTY_QUERY>()),
        u32::try_from(size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()),
    ) else {
        return false;
    };
    // SAFETY: the handle is open; the input and output buffers are the
    // structures of `IOCTL_STORAGE_QUERY_PROPERTY` with their sizes; the call
    // is synchronous (no `OVERLAPPED`).
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&raw const query).cast::<c_void>(),
            query_len,
            (&raw mut descriptor).cast::<c_void>(),
            descriptor_len,
            &raw mut returned,
            ptr::null_mut(),
        )
    };
    ok != 0 && descriptor.IncursSeekPenalty
}

/// Space figures of the volume with the given root (verbatim root or volume
/// GUID path).
pub(crate) fn disk_space(root: &WidePath) -> Result<SpaceInfo, u32> {
    let (mut free_to_caller, mut total_to_caller, mut total_free) = (0_u64, 0_u64, 0_u64);
    // SAFETY: `root` is zero-terminated and the three counters are writable.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            root.as_ptr(),
            &raw mut free_to_caller,
            &raw mut total_to_caller,
            &raw mut total_free,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(space_info(free_to_caller, total_to_caller, total_free))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
