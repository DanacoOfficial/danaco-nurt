// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Local accounts, groups and security identifiers on Windows.
//!
//! Users come from `NetUserEnum` (level 3), groups from `NetLocalGroupEnum`
//! (level 1) with members from `NetLocalGroupGetMembers` (level 3), and the
//! groups of a user from `NetUserGetLocalGroups`. Every buffer of the network
//! management functions is released with `NetApiBufferFree`.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Account, AccountKind};
use nurt_contracts::value::{Instant, Value};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE, LocalFree,
};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    FILTER_NORMAL_ACCOUNT, LG_INCLUDE_INDIRECT, LOCALGROUP_INFO_1, LOCALGROUP_MEMBERS_INFO_3,
    LOCALGROUP_USERS_INFO_0, MAX_PREFERRED_LENGTH, NetApiBufferFree, NetLocalGroupEnum,
    NetLocalGroupGetMembers, NetUserEnum, NetUserGetLocalGroups, UF_ACCOUNTDISABLE, USER_INFO_3,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetTokenInformation, LookupAccountNameW, LookupAccountSidW, PSID, SID_NAME_USE, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::Selection;
use crate::host::canonical_path;

/// Information level of `NetUserEnum` with flags, full name and last logon.
const USER_LEVEL: u32 = 3;
/// Information level of `NetLocalGroupEnum` with name and comment.
const GROUP_LEVEL: u32 = 1;
/// Information level of `NetLocalGroupGetMembers` with `DOMAIN\name`.
const MEMBER_LEVEL: u32 = 3;
/// Information level of `NetUserGetLocalGroups` with the group name.
const USER_GROUPS_LEVEL: u32 = 0;
/// Relative identifiers of accounts created by people start here.
const FIRST_USER_RID: u32 = 1000;
/// Prefix of the security identifiers of the built-in groups.
const BUILTIN_PREFIX: &str = "S-1-5-32-";
/// Largest security identifier in bytes (`SECURITY_MAX_SID_SIZE`).
const MAX_SID_BYTES: usize = 68;
/// Length in UTF-16 units of the name buffers of account lookups.
const NAME_CAPACITY: u32 = 256;

/// Null-terminated UTF-16 form of `text`.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Text of a null-terminated UTF-16 string; empty for a null pointer.
pub(crate) fn from_wide_ptr(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0;
    // SAFETY: the pointer is non-null and points to a null-terminated string
    // owned by the caller for the duration of the call; reading stops at the
    // terminator.
    while unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the `length` units before the terminator were read above.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, length) })
}

/// Security identifier of the account running the process.
pub(crate) fn current_sid() -> Option<String> {
    // SAFETY: returns a pseudo handle of the current process, which needs no
    // closing.
    let process = unsafe { GetCurrentProcess() };
    token_account(process).map(|(sid, _)| sid)
}

/// Security identifier and account name of the process `pid`.
pub(crate) fn process_account(pid: u32) -> Option<(String, String)> {
    // SAFETY: plain call with a process identifier; a null handle signals
    // failure and is not used.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let account = token_account(process);
    // SAFETY: `process` was opened above and is closed once.
    unsafe { CloseHandle(process) };
    account
}

/// Names of the local groups of the account `user`, including groups it
/// belongs to through other groups.
pub(crate) fn user_local_groups(user: &str) -> Vec<String> {
    let name = wide(user);
    let mut buffer = null_mut();
    let (mut read, mut total) = (0_u32, 0_u32);
    // SAFETY: `name` is null-terminated and every output pointer is writable.
    let status = unsafe {
        NetUserGetLocalGroups(
            null(),
            name.as_ptr(),
            USER_GROUPS_LEVEL,
            LG_INCLUDE_INDIRECT,
            &mut buffer,
            MAX_PREFERRED_LENGTH,
            &mut read,
            &mut total,
        )
    };
    let buffer = NetBuffer(buffer);
    if status != ERROR_SUCCESS {
        return Vec::new();
    }
    buffer
        .entries::<LOCALGROUP_USERS_INFO_0>(read)
        .iter()
        .map(|entry| from_wide_ptr(entry.lgrui0_name))
        .collect()
}

/// Local accounts of `selection`.
///
/// # Errors
///
/// Fails with `E_PRIVILEGE` when the account database refuses access and
/// with `E_IO` for other failures.
pub(crate) fn accounts(selection: Selection) -> Result<Vec<Account>> {
    let mut accounts = Vec::new();
    if selection.users() {
        accounts.extend(users()?);
    }
    if selection.groups() {
        accounts.extend(groups()?);
    }
    Ok(accounts)
}

fn users() -> Result<Vec<Account>> {
    let mut users = Vec::new();
    let mut resume = 0_u32;
    loop {
        let mut buffer = null_mut();
        let (mut read, mut total) = (0_u32, 0_u32);
        // SAFETY: every output pointer is writable; the buffer is released by
        // `NetBuffer`.
        let status = unsafe {
            NetUserEnum(
                null(),
                USER_LEVEL,
                FILTER_NORMAL_ACCOUNT,
                &mut buffer,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
                &mut resume,
            )
        };
        let buffer = NetBuffer(buffer);
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return Err(net_error(status, "user accounts"));
        }
        users.extend(buffer.entries::<USER_INFO_3>(read).iter().map(user_account));
        if status == ERROR_SUCCESS {
            return Ok(users);
        }
    }
}

fn user_account(info: &USER_INFO_3) -> Account {
    let name = from_wide_ptr(info.usri3_name);
    let home = from_wide_ptr(info.usri3_home_dir);
    let full_name = from_wide_ptr(info.usri3_full_name);
    let mut raw = IndexMap::new();
    raw.insert("rid".to_owned(), Value::Int(i64::from(info.usri3_user_id)));
    Account {
        id: account_sid(&name).unwrap_or_else(|| info.usri3_user_id.to_string()),
        kind: AccountKind::User,
        home: (!home.is_empty()).then(|| canonical_path(std::path::Path::new(&home))),
        shell: None,
        groups: Some(user_local_groups(&name)),
        disabled: Some(info.usri3_flags & UF_ACCOUNTDISABLE != 0),
        full_name: (!full_name.is_empty()).then_some(full_name),
        system: Some(info.usri3_user_id < FIRST_USER_RID),
        last_login: (info.usri3_last_logon > 0)
            .then(|| Instant::from_unix_seconds(i64::from(info.usri3_last_logon)))
            .flatten(),
        os_raw: Some(raw),
        name,
    }
}

fn groups() -> Result<Vec<Account>> {
    let mut groups = Vec::new();
    let mut resume = 0_usize;
    loop {
        let mut buffer = null_mut();
        let (mut read, mut total) = (0_u32, 0_u32);
        // SAFETY: every output pointer is writable; the buffer is released by
        // `NetBuffer`.
        let status = unsafe {
            NetLocalGroupEnum(
                null(),
                GROUP_LEVEL,
                &mut buffer,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
                &mut resume,
            )
        };
        let buffer = NetBuffer(buffer);
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return Err(net_error(status, "local groups"));
        }
        groups.extend(
            buffer
                .entries::<LOCALGROUP_INFO_1>(read)
                .iter()
                .map(group_account),
        );
        if status == ERROR_SUCCESS {
            return Ok(groups);
        }
    }
}

fn group_account(info: &LOCALGROUP_INFO_1) -> Account {
    let name = from_wide_ptr(info.lgrpi1_name);
    let comment = from_wide_ptr(info.lgrpi1_comment);
    let id = account_sid(&name).unwrap_or_default();
    let mut raw = IndexMap::new();
    if !comment.is_empty() {
        raw.insert("comment".to_owned(), Value::Text(comment));
    }
    Account {
        kind: AccountKind::Group,
        groups: Some(group_members(&name)),
        system: Some(id.starts_with(BUILTIN_PREFIX)),
        os_raw: (!raw.is_empty()).then_some(raw),
        id,
        name,
        ..Account::default()
    }
}

fn group_members(group: &str) -> Vec<String> {
    let name = wide(group);
    let mut members = Vec::new();
    let mut resume = 0_usize;
    loop {
        let mut buffer = null_mut();
        let (mut read, mut total) = (0_u32, 0_u32);
        // SAFETY: `name` is null-terminated and every output pointer is
        // writable; the buffer is released by `NetBuffer`.
        let status = unsafe {
            NetLocalGroupGetMembers(
                null(),
                name.as_ptr(),
                MEMBER_LEVEL,
                &mut buffer,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
                &mut resume,
            )
        };
        let buffer = NetBuffer(buffer);
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return members;
        }
        members.extend(
            buffer
                .entries::<LOCALGROUP_MEMBERS_INFO_3>(read)
                .iter()
                .map(|entry| from_wide_ptr(entry.lgrmi3_domainandname)),
        );
        if status == ERROR_SUCCESS {
            return members;
        }
    }
}

/// Security identifier of the account or group `name` in text form.
fn account_sid(name: &str) -> Option<String> {
    let name = wide(name);
    let mut sid = [0_u64; MAX_SID_BYTES.div_ceil(size_of::<u64>())];
    let mut sid_size = u32::try_from(size_of_val(&sid)).ok()?;
    let mut domain = [0_u16; NAME_CAPACITY as usize];
    let mut domain_size = NAME_CAPACITY;
    let mut usage: SID_NAME_USE = 0;
    // SAFETY: the buffers have the sizes passed with them and `name` is
    // null-terminated.
    let found = unsafe {
        LookupAccountNameW(
            null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut usage,
        )
    };
    if found == 0 {
        return None;
    }
    sid_text(sid.as_mut_ptr().cast())
}

fn sid_text(sid: PSID) -> Option<String> {
    let mut text = null_mut();
    // SAFETY: `sid` points to a valid security identifier and `text` is
    // writable; the string is released with `LocalFree` below.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return None;
    }
    let value = from_wide_ptr(text);
    // SAFETY: `text` was allocated by `ConvertSidToStringSidW` and is released
    // once.
    unsafe { LocalFree(text.cast()) };
    Some(value)
}

fn sid_name(sid: PSID) -> Option<String> {
    let mut name = [0_u16; NAME_CAPACITY as usize];
    let mut name_size = NAME_CAPACITY;
    let mut domain = [0_u16; NAME_CAPACITY as usize];
    let mut domain_size = NAME_CAPACITY;
    let mut usage: SID_NAME_USE = 0;
    // SAFETY: the buffers have the sizes passed with them and `sid` is valid.
    let found = unsafe {
        LookupAccountSidW(
            null(),
            sid,
            name.as_mut_ptr(),
            &mut name_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut usage,
        )
    };
    (found != 0).then(|| from_wide_ptr(name.as_ptr()))
}

/// Security identifier and account name of the token of `process`.
fn token_account(process: HANDLE) -> Option<(String, String)> {
    let mut token: HANDLE = null_mut();
    // SAFETY: `process` is a valid process handle and `token` is writable.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let account = token_user(token);
    // SAFETY: `token` was opened above and is closed once.
    unsafe { CloseHandle(token) };
    account
}

fn token_user(token: HANDLE) -> Option<(String, String)> {
    let mut length = 0_u32;
    // SAFETY: a null buffer of length zero asks for the required size only.
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut length) };
    if length == 0 {
        return None;
    }
    let words = usize::try_from(length).ok()?.div_ceil(size_of::<u64>());
    let mut buffer = vec![0_u64; words];
    // SAFETY: `buffer` holds at least `length` writable bytes aligned for
    // `TOKEN_USER`.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            length,
            &mut length,
        )
    };
    if read == 0 {
        return None;
    }
    // SAFETY: the call filled the buffer with a `TOKEN_USER` structure whose
    // identifier points into the same buffer, which outlives its use here.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let sid = sid_text(user.User.Sid)?;
    let name = sid_name(user.User.Sid).unwrap_or_else(|| sid.clone());
    Some((sid, name))
}

fn net_error(status: u32, what: &str) -> Error {
    if status == ERROR_ACCESS_DENIED {
        Error::new(
            ErrorCode::Privilege,
            format!("cannot list {what}: access denied; run as an administrator"),
        )
        .with_detail("required", serde_json::json!("administrator"))
    } else {
        Error::new(
            ErrorCode::Io,
            format!("cannot list {what}: system error {status}"),
        )
    }
}

/// Buffer returned by a network management function.
struct NetBuffer(*mut u8);

impl NetBuffer {
    /// The first `count` entries of the buffer.
    fn entries<T>(&self, count: u32) -> &[T] {
        let Ok(count) = usize::try_from(count) else {
            return &[];
        };
        if self.0.is_null() || count == 0 {
            return &[];
        }
        // SAFETY: the function that filled the buffer reported `count`
        // entries of type `T`; the buffer lives as long as `self`.
        unsafe { std::slice::from_raw_parts(self.0.cast::<T>(), count) }
    }
}

impl Drop for NetBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the buffer was allocated by a network management
            // function and is released once.
            unsafe { NetApiBufferFree(self.0.cast()) };
        }
    }
}
