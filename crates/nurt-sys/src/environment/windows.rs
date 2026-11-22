// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Persistent environment variables in the Windows registry.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::EnvironmentScope;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS,
    ERROR_SUCCESS,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
    RegEnumValueW, RegOpenKeyExW,
};

use super::persistent_location;
use crate::accounts::windows::wide;

/// Longest value name of the registry, in UTF-16 units, with the terminator.
const MAX_VALUE_NAME: usize = 16_384;
/// First size of the buffer of value data, in bytes.
const INITIAL_DATA: usize = 4096;

/// Open registry key, closed on drop.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `RegOpenKeyExW` and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

/// Text variables of `scope` in the order of the registry; values of other
/// types are skipped. A missing key yields no variables.
///
/// # Errors
///
/// Fails with `E_PERMISSION` when the key may not be read and `E_IO` for
/// other errors of the registry.
pub(crate) fn read_persistent(scope: EnvironmentScope) -> Result<Vec<(String, String)>> {
    let Some(location) = persistent_location(scope) else {
        return Ok(Vec::new());
    };
    let Some((hive, path)) = location.segments.split_first() else {
        return Ok(Vec::new());
    };
    let hive = if hive == "HKLM" {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    let subkey = wide(&path.join("\\"));
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY: `subkey` is a null-terminated UTF-16 path and `handle` is
    // writable.
    let status = unsafe { RegOpenKeyExW(hive, subkey.as_ptr(), 0, KEY_READ, &mut handle) };
    match status {
        ERROR_SUCCESS => enumerate(&Key(handle), scope),
        ERROR_FILE_NOT_FOUND => Ok(Vec::new()),
        other => Err(registry_error(other, scope)),
    }
}

fn enumerate(key: &Key, scope: EnvironmentScope) -> Result<Vec<(String, String)>> {
    let mut variables = Vec::new();
    let mut name = vec![0u16; MAX_VALUE_NAME];
    let mut data = vec![0u8; INITIAL_DATA];
    let mut index = 0u32;
    loop {
        let mut name_len = u32::try_from(name.len()).unwrap_or(u32::MAX);
        let mut data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let mut kind = 0u32;
        // SAFETY: the buffers have the lengths passed in `name_len` and
        // `data_len`, and the key handle is open.
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut name_len,
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut data_len,
            )
        };
        match status {
            ERROR_SUCCESS => {}
            ERROR_NO_MORE_ITEMS => return Ok(variables),
            ERROR_MORE_DATA => {
                data.resize(usize::try_from(data_len).unwrap_or(0).max(data.len() * 2), 0);
                continue;
            }
            other => return Err(registry_error(other, scope)),
        }
        index += 1;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            continue;
        }
        let name_len = usize::try_from(name_len).unwrap_or(0).min(name.len());
        let data_len = usize::try_from(data_len).unwrap_or(0).min(data.len());
        variables.push((
            String::from_utf16_lossy(&name[..name_len]),
            utf16_value(&data[..data_len]),
        ));
    }
}

fn utf16_value(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn registry_error(status: u32, scope: EnvironmentScope) -> Error {
    let code = if status == ERROR_ACCESS_DENIED {
        ErrorCode::Permission
    } else {
        ErrorCode::Io
    };
    Error::new(
        code,
        format!("the persistent `{scope}` variables could not be read (system error {status})"),
    )
    .with_detail("os_error", serde_json::json!(status))
}
