// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Volumes of Windows: the drive letters of `GetLogicalDriveStringsW`.

use std::ptr::null_mut;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::VolumeKind;
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};

use super::{Volume, VolumeList, VolumeSource};
use crate::processes::winpath::{canonical, from_wide, to_wide};

/// Values of `GetDriveTypeW` (`winbase.h`).
mod drive {
    pub(super) const REMOVABLE: u32 = 2;
    pub(super) const FIXED: u32 = 3;
    pub(super) const REMOTE: u32 = 4;
    pub(super) const CDROM: u32 = 5;
    pub(super) const RAMDISK: u32 = 6;
}

/// File system flags of `GetVolumeInformationW` (`winnt.h`).
const FILE_CASE_SENSITIVE_SEARCH: u32 = 0x0000_0001;
const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;

/// Length of the buffers for names, in UTF-16 units (`MAX_PATH + 1`).
const NAME_UNITS: usize = 261;

/// Volume source of the running system.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Drives;

fn drive_kind(drive_type: u32) -> Option<VolumeKind> {
    match drive_type {
        drive::FIXED => Some(VolumeKind::Fixed),
        drive::REMOVABLE | drive::CDROM => Some(VolumeKind::Removable),
        drive::REMOTE => Some(VolumeKind::Network),
        drive::RAMDISK => Some(VolumeKind::Memory),
        _ => None,
    }
}

fn drive_roots() -> Result<Vec<String>> {
    // SAFETY: a null buffer of length zero asks for the required length only.
    let needed = unsafe { GetLogicalDriveStringsW(0, null_mut()) };
    let mut buffer = vec![0_u16; usize::try_from(needed).unwrap_or(0) + 1];
    let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: `buffer` holds `length` writable units.
    let written = unsafe { GetLogicalDriveStringsW(length, buffer.as_mut_ptr()) };
    if written == 0 {
        return Err(Error::new(
            ErrorCode::Io,
            format!("cannot list drives: {}", std::io::Error::last_os_error()),
        ));
    }
    Ok(buffer
        .split(|&unit| unit == 0)
        .filter(|root| !root.is_empty())
        .map(from_wide)
        .collect())
}

struct VolumeInformation {
    label: String,
    filesystem: String,
    flags: u32,
}

fn volume_information(root: &[u16]) -> Option<VolumeInformation> {
    let mut label = vec![0_u16; NAME_UNITS];
    let mut filesystem = vec![0_u16; NAME_UNITS];
    let mut flags = 0_u32;
    let units = u32::try_from(NAME_UNITS).ok()?;
    // SAFETY: `root` is zero-terminated; both buffers hold `units` writable units.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            units,
            null_mut(),
            null_mut(),
            &mut flags,
            filesystem.as_mut_ptr(),
            units,
        )
    };
    (ok != 0).then(|| VolumeInformation {
        label: from_wide(&label),
        filesystem: from_wide(&filesystem),
        flags,
    })
}

/// Bytes available to the caller, total bytes and free bytes.
fn space(root: &[u16]) -> Option<(u64, u64, u64)> {
    let (mut available, mut total, mut free) = (0_u64, 0_u64, 0_u64);
    // SAFETY: `root` is zero-terminated; the three pointers refer to values of this frame.
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, &mut free) };
    (ok != 0).then_some((available, total, free))
}

impl VolumeSource for Drives {
    fn volumes(&self) -> Result<VolumeList> {
        let mut list = VolumeList::default();
        for root in drive_roots()? {
            let wide = to_wide(&root);
            // SAFETY: `wide` is a zero-terminated wide string.
            let Some(kind) = drive_kind(unsafe { GetDriveTypeW(wide.as_ptr()) }) else {
                continue;
            };
            let (Some(info), Some((available, total, free))) =
                (volume_information(&wide), space(&wide))
            else {
                list.unreadable += u64::from(kind != VolumeKind::Removable);
                continue;
            };
            let drive = root.trim_end_matches('\\').to_owned();
            list.volumes.push(Volume {
                mount: canonical(&root),
                device: drive,
                filesystem: info.filesystem,
                kind: Some(kind),
                total,
                free: available,
                used: total.saturating_sub(free),
                case_sensitive: info.flags & FILE_CASE_SENSITIVE_SEARCH != 0,
                readonly: info.flags & FILE_READ_ONLY_VOLUME != 0,
                label: (!info.label.is_empty()).then_some(info.label),
                inodes_total: None,
                inodes_free: None,
                options: None,
            });
        }
        Ok(list)
    }
}
