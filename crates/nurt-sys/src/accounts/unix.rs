// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! User and group databases on Linux and macOS.
//!
//! On the running system the databases are read through the system library
//! (`getpwent`, `getgrent`), so accounts of every configured source are
//! listed; on macOS the library forwards the query to the directory service
//! and returns the local accounts. Under a synthetic [`SysRoot`] the files
//! `/etc/passwd` and `/etc/group` are parsed instead.

use std::ffi::CStr;
use std::io;
use std::sync::{Mutex, PoisonError};

use libc::c_char;

use super::files::{self, GroupEntry, PasswdEntry};
use crate::host::SysRoot;

/// Serializes the enumeration functions of the system library, which keep
/// their position in process-wide state.
static DATABASE: Mutex<()> = Mutex::new(());

/// Entries of the user database.
///
/// # Errors
///
/// Returns the error of reading `/etc/passwd` under a synthetic root.
pub(crate) fn users(root: &SysRoot) -> io::Result<Vec<PasswdEntry>> {
    if root.is_live() {
        return Ok(system_users());
    }
    root.read_to_string("/etc/passwd")
        .map(|text| files::parse_passwd(&text))
}

/// Entries of the group database.
///
/// # Errors
///
/// Returns the error of reading `/etc/group` under a synthetic root.
pub(crate) fn groups(root: &SysRoot) -> io::Result<Vec<GroupEntry>> {
    if root.is_live() {
        return Ok(system_groups());
    }
    root.read_to_string("/etc/group")
        .map(|text| files::parse_group(&text))
}

/// Names of the groups `gids`; identifiers without a name stay numeric.
pub(crate) fn group_names(root: &SysRoot, gids: &[u32]) -> Vec<String> {
    let groups = groups(root).unwrap_or_default();
    gids.iter()
        .map(|gid| {
            groups
                .iter()
                .find(|group| group.gid == *gid)
                .map_or_else(|| gid.to_string(), |group| group.name.clone())
        })
        .collect()
}

/// User identifier of the process.
pub(crate) fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Primary and supplementary group identifiers of the process, without
/// duplicates, primary group first.
pub(crate) fn current_gids() -> Vec<u32> {
    let mut gids = vec![rustix::process::getgid().as_raw()];
    for gid in supplementary_gids() {
        if !gids.contains(&gid) {
            gids.push(gid);
        }
    }
    gids
}

fn supplementary_gids() -> Vec<u32> {
    // SAFETY: with a size of 0 `getgroups` only returns the number of groups
    // and does not write through the null pointer.
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    let Ok(len) = usize::try_from(count) else {
        return Vec::new();
    };
    let mut gids: Vec<libc::gid_t> = vec![0; len];
    // SAFETY: the buffer holds `count` elements, the size passed to the call.
    let written = unsafe { libc::getgroups(count, gids.as_mut_ptr()) };
    gids.truncate(usize::try_from(written).unwrap_or(0));
    gids
}

fn system_users() -> Vec<PasswdEntry> {
    let _guard = DATABASE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut users = Vec::new();
    // SAFETY: the enumeration state is used only while `DATABASE` is held.
    unsafe { libc::setpwent() };
    loop {
        // SAFETY: as above; the returned entry stays valid until the next
        // call, and its fields are copied before that.
        let entry = unsafe { libc::getpwent() };
        if entry.is_null() {
            break;
        }
        // SAFETY: `entry` is non-null and points to the static entry of the
        // library, whose strings are null-terminated or null.
        let entry = unsafe { &*entry };
        users.push(PasswdEntry {
            name: text(entry.pw_name),
            uid: entry.pw_uid,
            gid: entry.pw_gid,
            gecos: text(entry.pw_gecos),
            home: text(entry.pw_dir),
            shell: text(entry.pw_shell),
        });
    }
    // SAFETY: closes the enumeration opened above while `DATABASE` is held.
    unsafe { libc::endpwent() };
    users
}

fn system_groups() -> Vec<GroupEntry> {
    let _guard = DATABASE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut groups = Vec::new();
    // SAFETY: the enumeration state is used only while `DATABASE` is held.
    unsafe { libc::setgrent() };
    loop {
        // SAFETY: as above; the returned entry stays valid until the next
        // call, and its fields are copied before that.
        let entry = unsafe { libc::getgrent() };
        if entry.is_null() {
            break;
        }
        // SAFETY: `entry` is non-null and points to the static entry of the
        // library.
        let entry = unsafe { &*entry };
        groups.push(GroupEntry {
            name: text(entry.gr_name),
            gid: entry.gr_gid,
            members: members(entry.gr_mem),
        });
    }
    // SAFETY: closes the enumeration opened above while `DATABASE` is held.
    unsafe { libc::endgrent() };
    groups
}

fn members(mut list: *mut *mut c_char) -> Vec<String> {
    let mut names = Vec::new();
    if list.is_null() {
        return names;
    }
    // SAFETY: `gr_mem` is a null-terminated array of null-terminated strings
    // owned by the library entry, valid until the next enumeration call.
    unsafe {
        while !(*list).is_null() {
            names.push(text(*list));
            list = list.add(1);
        }
    }
    names
}

/// Login name of the user `uid` from the user database of the system.
#[cfg(target_os = "macos")]
pub(crate) fn user_name(uid: u32) -> Option<String> {
    /// Size of the buffer for the strings of one entry.
    const ENTRY_BUFFER: usize = 4096;
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::zeroed();
    let mut buffer: Vec<c_char> = vec![0; ENTRY_BUFFER];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `entry` and `buffer` are writable for their full sizes and
    // `found` receives either null or a pointer to `entry`.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            entry.as_mut_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    // SAFETY: `found` points to `entry`, filled by the call, whose strings
    // live in `buffer`, which is still alive.
    Some(text(unsafe { (*found).pw_name }))
}

fn text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: the pointer is non-null and points to a null-terminated string
    // of a library entry that is still valid.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}
