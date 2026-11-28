// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::win
// Responsibility:   Registry of the Windows host.
// Dependencies:     windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registry of the Windows host.
//!
//! [`SystemRegistry`] implements [`RegistryReader`] with the registry
//! functions of Win32. The module is compiled on Windows only and holds the
//! only `unsafe` code of the crate: each call passes buffers owned by the
//! calling function together with their lengths, and every opened key is
//! closed by [`Drop`].

#![allow(
    unsafe_code,
    reason = "calls of the Win32 registry functions; each block states its invariants"
)]

use std::collections::BTreeMap;
use std::ptr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS,
    FILETIME, WIN32_ERROR,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, RegCloseKey, RegEnumKeyExW,
    RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW,
};

use crate::source::{
    RegistryHive, RegistryReader, RegistryValue, RegistryView, SourceError, SourceErrorKind,
    SourceId,
};

/// Identifier reported in errors of the registry.
pub const REGISTRY_SOURCE: &str = "registry";

/// Seconds from 1601-01-01, the epoch of `FILETIME`, to 1970-01-01.
const FILETIME_UNIX_OFFSET_SECONDS: u64 = 11_644_473_600;
/// Intervals of 100 ns in one second.
const FILETIME_TICKS_PER_SECOND: u64 = 10_000_000;
/// Nanoseconds in one `FILETIME` interval.
const FILETIME_TICK_NANOS: u64 = 100;
/// Longest key name in UTF-16 units including the terminating NUL.
const MAX_KEY_NAME: usize = 256;
/// Attempts to read one value whose size grows between two calls.
const MAX_VALUE_ATTEMPTS: usize = 4;

/// Registry of the running Windows system, read-only.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRegistry;

impl SystemRegistry {
    /// Reader of the registry of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// Open registry key, closed when dropped.
struct OpenKey(HKEY);

impl Drop for OpenKey {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by a successful `RegOpenKeyExW` and
        // is closed exactly once, here.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

/// Size information of an open key.
struct KeyInfo {
    max_value_name: usize,
    max_value_data: usize,
    written: SystemTime,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn len_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn hive_name(hive: RegistryHive) -> &'static str {
    match hive {
        RegistryHive::LocalMachine => "HKLM",
        RegistryHive::CurrentUser => "HKCU",
    }
}

fn failure(hive: RegistryHive, key: &str, status: WIN32_ERROR) -> SourceError {
    let kind = if status == ERROR_ACCESS_DENIED {
        SourceErrorKind::Permission
    } else {
        SourceErrorKind::Unavailable
    };
    SourceError::new(
        &SourceId::new(REGISTRY_SOURCE),
        kind,
        format!("registry key cannot be read: error {status}"),
    )
    .at(format!("{}\\{key}", hive_name(hive)))
}

fn open(hive: RegistryHive, view: RegistryView, key: &str) -> Result<Option<OpenKey>, SourceError> {
    let root = match hive {
        RegistryHive::LocalMachine => HKEY_LOCAL_MACHINE,
        RegistryHive::CurrentUser => HKEY_CURRENT_USER,
    };
    let access = KEY_READ
        | match view {
            RegistryView::Native => KEY_WOW64_64KEY,
            RegistryView::Wow32 => KEY_WOW64_32KEY,
        };
    let name = wide(key);
    let mut handle: HKEY = ptr::null_mut();
    // SAFETY: `name` is a NUL-terminated UTF-16 string that lives until the
    // call returns, and `handle` is a valid location for the result.
    let status = unsafe { RegOpenKeyExW(root, name.as_ptr(), 0, access, &raw mut handle) };
    match status {
        ERROR_SUCCESS => Ok(Some(OpenKey(handle))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        other => Err(failure(hive, key, other)),
    }
}

fn info(open: &OpenKey, hive: RegistryHive, key: &str) -> Result<KeyInfo, SourceError> {
    let mut max_value_name = 0u32;
    let mut max_value_data = 0u32;
    let mut written = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: the key is open; every non-null pointer names a local variable
    // that outlives the call, and the class buffer is not requested.
    let status = unsafe {
        RegQueryInfoKeyW(
            open.0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut max_value_name,
            &raw mut max_value_data,
            ptr::null_mut(),
            &raw mut written,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(failure(hive, key, status));
    }
    Ok(KeyInfo {
        max_value_name: max_value_name as usize,
        max_value_data: max_value_data as usize,
        written: filetime(&written),
    })
}

/// Time of a `FILETIME`; times before 1970 become the Unix epoch.
fn filetime(time: &FILETIME) -> SystemTime {
    let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    let seconds = ticks / FILETIME_TICKS_PER_SECOND;
    let nanos = (ticks % FILETIME_TICKS_PER_SECOND) * FILETIME_TICK_NANOS;
    match seconds.checked_sub(FILETIME_UNIX_OFFSET_SECONDS) {
        Some(since) => UNIX_EPOCH + Duration::from_secs(since) + Duration::from_nanos(nanos),
        None => UNIX_EPOCH,
    }
}

fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Value of the registry type `kind` stored in `bytes`.
fn decode(kind: u32, bytes: &[u8]) -> RegistryValue {
    match kind {
        REG_SZ => RegistryValue::String(utf16_text(bytes)),
        REG_EXPAND_SZ => RegistryValue::ExpandString(utf16_text(bytes)),
        REG_MULTI_SZ => {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            RegistryValue::MultiString(
                units
                    .split(|unit| *unit == 0)
                    .filter(|item| !item.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect(),
            )
        }
        REG_DWORD => match <[u8; 4]>::try_from(bytes) {
            Ok(word) => RegistryValue::Dword(u32::from_le_bytes(word)),
            Err(_) => RegistryValue::Binary(bytes.to_vec()),
        },
        REG_QWORD => match <[u8; 8]>::try_from(bytes) {
            Ok(word) => RegistryValue::Qword(u64::from_le_bytes(word)),
            Err(_) => RegistryValue::Binary(bytes.to_vec()),
        },
        _ => RegistryValue::Binary(bytes.to_vec()),
    }
}

impl RegistryReader for SystemRegistry {
    fn subkeys(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<Vec<String>>, SourceError> {
        let Some(open) = open(hive, view, key)? else {
            return Ok(None);
        };
        let mut names = Vec::new();
        let mut buffer = vec![0u16; MAX_KEY_NAME];
        for index in 0..u32::MAX {
            let mut length = len_u32(buffer.len());
            // SAFETY: the key is open, `buffer` holds `length` UTF-16 units
            // and the optional class and time arguments are null.
            let status = unsafe {
                RegEnumKeyExW(
                    open.0,
                    index,
                    buffer.as_mut_ptr(),
                    &raw mut length,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => {
                    let end = (length as usize).min(buffer.len());
                    names.push(String::from_utf16_lossy(&buffer[..end]));
                }
                ERROR_NO_MORE_ITEMS => break,
                other => return Err(failure(hive, key, other)),
            }
        }
        Ok(Some(names))
    }

    fn values(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<BTreeMap<String, RegistryValue>>, SourceError> {
        let Some(open) = open(hive, view, key)? else {
            return Ok(None);
        };
        let sizes = info(&open, hive, key)?;
        let mut name = vec![0u16; sizes.max_value_name + 1];
        let mut data = vec![0u8; sizes.max_value_data.max(1)];
        let mut values = BTreeMap::new();
        let mut index = 0u32;
        let mut attempts = 0;
        loop {
            let mut name_length = len_u32(name.len());
            let mut data_length = len_u32(data.len());
            let mut kind = 0u32;
            // SAFETY: the key is open, `name` holds `name_length` UTF-16
            // units, `data` holds `data_length` bytes and `kind` is a local
            // variable; the reserved argument is null.
            let status = unsafe {
                RegEnumValueW(
                    open.0,
                    index,
                    name.as_mut_ptr(),
                    &raw mut name_length,
                    ptr::null(),
                    &raw mut kind,
                    data.as_mut_ptr(),
                    &raw mut data_length,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    let name_end = (name_length as usize).min(name.len());
                    let data_end = (data_length as usize).min(data.len());
                    values.insert(
                        String::from_utf16_lossy(&name[..name_end]),
                        decode(kind, &data[..data_end]),
                    );
                    index += 1;
                    attempts = 0;
                }
                ERROR_MORE_DATA if attempts < MAX_VALUE_ATTEMPTS => {
                    attempts += 1;
                    name.resize(name.len() * 2, 0);
                    data.resize((data_length as usize).max(data.len() * 2), 0);
                }
                ERROR_NO_MORE_ITEMS => break,
                other => return Err(failure(hive, key, other)),
            }
        }
        Ok(Some(values))
    }

    fn last_write(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<SystemTime>, SourceError> {
        let Some(open) = open(hive, view, key)? else {
            return Ok(None);
        };
        Ok(Some(info(&open, hive, key)?.written))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code fails loudly on a broken registry"
)]
mod tests {
    use super::*;

    #[test]
    fn decodes_registry_value_types() {
        let text: Vec<u8> = "abc\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            decode(REG_SZ, &text),
            RegistryValue::String("abc".to_owned())
        );
        let list: Vec<u8> = "a\0bc\0\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            decode(REG_MULTI_SZ, &list),
            RegistryValue::MultiString(vec!["a".to_owned(), "bc".to_owned()])
        );
        assert_eq!(decode(REG_DWORD, &[1, 0, 0, 0]), RegistryValue::Dword(1));
        assert_eq!(decode(REG_DWORD, &[1]), RegistryValue::Binary(vec![1]));
    }

    #[test]
    fn converts_filetime_to_unix_time() {
        let ticks = FILETIME_UNIX_OFFSET_SECONDS * FILETIME_TICKS_PER_SECOND;
        let time = FILETIME {
            dwLowDateTime: (ticks & 0xffff_ffff) as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        assert_eq!(filetime(&time), UNIX_EPOCH);
    }

    #[test]
    #[ignore = "requires the registry of a Windows system"]
    fn reads_uninstall_key_of_running_system() {
        let registry = SystemRegistry::new();
        let key = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion";
        let names = registry
            .subkeys(RegistryHive::LocalMachine, RegistryView::Native, key)
            .unwrap();
        assert!(names.is_some_and(|names| !names.is_empty()));
        assert!(
            registry
                .last_write(RegistryHive::LocalMachine, RegistryView::Native, key)
                .unwrap()
                .is_some()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
