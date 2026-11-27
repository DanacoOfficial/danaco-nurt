// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::disks::macos
// Responsibility:   Volumes of macOS, read with `getmntinfo` and the volume resource keys of Core Foundation.
// Dependencies:     core_foundation, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Volumes of macOS, read with `getmntinfo` and the volume resource keys of
//! Core Foundation.

use std::ffi::{CString, c_void};
use std::os::unix::ffi::OsStrExt;
use std::ptr::null_mut;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::number::kCFBooleanTrue;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::{
    CFURL, CFURLRef, kCFURLVolumeIsInternalKey, kCFURLVolumeIsRemovableKey, kCFURLVolumeNameKey,
};
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::classify::{classify, is_pseudo};
use super::{Volume, VolumeList, VolumeSource};
use crate::processes::macos::c_text;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCopyResourcePropertyForKey(
        url: CFURLRef,
        key: CFStringRef,
        value: *mut CFTypeRef,
        error: *mut c_void,
    ) -> u8;
}

/// Volume source of the running system.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MountList;

/// Resource value of a volume; the caller releases it.
struct Property(CFTypeRef);

impl Drop for Property {
    fn drop(&mut self) {
        // SAFETY: the value was returned by a Copy function and is released once.
        unsafe { CFRelease(self.0) };
    }
}

fn property(url: &CFURL, key: CFStringRef) -> Option<Property> {
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `url` and `key` are valid objects; `value` receives a retained object or null.
    let ok = unsafe {
        CFURLCopyResourcePropertyForKey(url.as_concrete_TypeRef(), key, &mut value, null_mut())
    };
    (ok != 0 && !value.is_null()).then_some(Property(value))
}

fn flag(url: &CFURL, key: CFStringRef) -> Option<bool> {
    // SAFETY: `kCFBooleanTrue` is a constant of Core Foundation.
    property(url, key).map(|value| value.0 == unsafe { kCFBooleanTrue }.cast())
}

fn name(url: &CFURL) -> Option<String> {
    // SAFETY: the key is a constant of Core Foundation.
    let value = property(url, unsafe { kCFURLVolumeNameKey })?;
    // SAFETY: the volume name key yields a string; the wrapper takes its own reference.
    let text = unsafe { CFString::wrap_under_get_rule(value.0.cast()) };
    Some(text.to_string())
}

/// Removable when the medium is removable or the device is external.
fn removable(url: &CFURL) -> bool {
    // SAFETY: the keys are constants of Core Foundation.
    let (removable, internal) = unsafe {
        (
            flag(url, kCFURLVolumeIsRemovableKey),
            flag(url, kCFURLVolumeIsInternalKey),
        )
    };
    removable == Some(true) || internal == Some(false)
}

fn case_sensitive(mount: &str) -> bool {
    let Ok(path) = CString::new(std::ffi::OsStr::new(mount).as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a zero-terminated string.
    unsafe { libc::pathconf(path.as_ptr(), libc::_PC_CASE_SENSITIVE) == 1 }
}

fn volume(stat: &libc::statfs) -> Option<Volume> {
    let fs_type = c_text(&stat.f_fstypename);
    if is_pseudo(&fs_type) {
        return None;
    }
    let mount = c_text(&stat.f_mntonname);
    let local = stat.f_flags & u32::try_from(libc::MNT_LOCAL).unwrap_or(0) != 0;
    let url = CFURL::from_path(&mount, true);
    let kind = if local {
        classify(&fs_type, url.as_ref().is_some_and(removable))
    } else {
        nurt_contracts::fs::VolumeKind::Network
    };
    let block = u64::from(stat.f_bsize);
    let total = stat.f_blocks.saturating_mul(block);
    Some(Volume {
        device: c_text(&stat.f_mntfromname),
        filesystem: fs_type,
        kind: Some(kind),
        total,
        free: stat.f_bavail.saturating_mul(block),
        used: total.saturating_sub(stat.f_bfree.saturating_mul(block)),
        case_sensitive: case_sensitive(&mount),
        readonly: stat.f_flags & u32::try_from(libc::MNT_RDONLY).unwrap_or(0) != 0,
        label: url.as_ref().and_then(name),
        inodes_total: (stat.f_files > 0).then_some(stat.f_files),
        inodes_free: (stat.f_files > 0).then_some(stat.f_ffree),
        options: None,
        mount,
    })
}

impl VolumeSource for MountList {
    fn volumes(&self) -> Result<VolumeList> {
        let mut mounts: *mut libc::statfs = null_mut();
        // SAFETY: the call stores a pointer to a static array of `count` structures.
        let count = unsafe { libc::getmntinfo(&mut mounts, libc::MNT_NOWAIT) };
        let count = usize::try_from(count).unwrap_or(0);
        if count == 0 || mounts.is_null() {
            return Err(Error::new(
                ErrorCode::Io,
                format!(
                    "cannot read the mount table: {}",
                    std::io::Error::last_os_error()
                ),
            ));
        }
        // SAFETY: `getmntinfo` returned `count` initialized structures at `mounts`, valid
        // until the next call in this thread.
        let stats = unsafe { std::slice::from_raw_parts(mounts, count) };
        Ok(VolumeList {
            volumes: stats.iter().filter_map(volume).collect(),
            unreadable: 0,
        })
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
