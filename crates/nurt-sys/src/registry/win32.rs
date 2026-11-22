// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registry back end over the Win32 registry functions.
//!
//! Keys are opened with `REG_OPTION_OPEN_LINK`, so a symbolic link key is
//! read as itself. The Win32 interface does not report whether a key is
//! volatile; [`KeyInfo::volatile`] is therefore always `false` here, and
//! volatile keys are preserved and restored as ordinary keys.

use std::iter;
use std::path::Path;
use std::ptr;

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult};
use nurt_contracts::record::RegistryView;
use nurt_contracts::value::Instant;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_CHILD_MUST_BE_VOLATILE, ERROR_FILE_NOT_FOUND,
    ERROR_INSUFFICIENT_BUFFER, ERROR_KEY_DELETED, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS,
    ERROR_PATH_NOT_FOUND, ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, FILETIME, WIN32_ERROR,
};
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::Storage::FileSystem::{DELETE, READ_CONTROL, WRITE_DAC};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    HKEY_USERS, KEY_CREATE_SUB_KEY, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE,
    KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_CREATED_NEW_KEY, REG_OPTION_NON_VOLATILE,
    REG_OPTION_OPEN_LINK, REG_OPTION_VOLATILE, REG_SAM_FLAGS, RegCloseKey, RegCreateKeyExW,
    RegDeleteKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW,
    RegGetKeySecurity, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, RegSetKeySecurity,
    RegSetValueExW,
};

use super::api::{KeyInfo, RegistryApi};
use super::mem::LINK_VALUE;
use super::path::{Hive, KeyPath};
use super::value::{REG_LINK, RawValue};

/// Longest key or value name, in UTF-16 units, including the terminator.
const MAX_NAME_UNITS: usize = 16_384;

/// Longest class name, in UTF-16 units, including the terminator.
const MAX_CLASS_UNITS: usize = 32_768;

/// Initial size of a data buffer in bytes; it grows on `ERROR_MORE_DATA`.
const INITIAL_DATA_BYTES: usize = 1_024;

/// Largest value the provider reads, in bytes.
const MAX_DATA_BYTES: usize = 1 << 30;

/// 100-nanosecond intervals between 1601-01-01 and 1970-01-01.
const FILETIME_UNIX_OFFSET: i64 = 116_444_736_000_000_000;

/// Nanoseconds per `FILETIME` interval.
const FILETIME_TICK_NANOS: i64 = 100;

/// Registry of the running system.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Win32Registry;

/// Open key handle, closed on drop.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by `RegOpenKeyExW` or
        // `RegCreateKeyExW`, is owned by this value and is closed once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

impl RegistryApi for Win32Registry {
    fn key_info(&self, view: RegistryView, key: &KeyPath) -> FsResult<Option<KeyInfo>> {
        let Some(handle) = open(view, key, KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS)? else {
            return Ok(None);
        };
        let counts = query_info(&handle, key)?;
        let link =
            query_value(&handle, key, LINK_VALUE)?.is_some_and(|value| value.raw_type == REG_LINK);
        Ok(Some(KeyInfo {
            class: counts.class,
            subkeys: counts.subkeys,
            values: counts.values,
            last_write: counts.last_write,
            volatile: false,
            link,
        }))
    }

    fn subkeys(&self, view: RegistryView, key: &KeyPath) -> FsResult<Vec<String>> {
        let handle = open_existing(view, key, KEY_ENUMERATE_SUB_KEYS | KEY_QUERY_VALUE)?;
        let mut names = Vec::new();
        let mut buffer = vec![0u16; MAX_NAME_UNITS];
        let mut index = 0u32;
        loop {
            let mut length = units(&buffer);
            // SAFETY: `buffer` holds `length` writable UTF-16 units; the class
            // and time outputs are not requested.
            let status = unsafe {
                RegEnumKeyExW(
                    handle.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => names.push(text(&buffer, length)),
                ERROR_NO_MORE_ITEMS => return Ok(names),
                code => return Err(error(code, "enumerate_keys", key)),
            }
            index += 1;
        }
    }

    fn values(&self, view: RegistryView, key: &KeyPath) -> FsResult<Vec<(String, RawValue)>> {
        let handle = open_existing(view, key, KEY_QUERY_VALUE)?;
        let mut values = Vec::new();
        let mut name = vec![0u16; MAX_NAME_UNITS];
        let mut data = vec![0u8; INITIAL_DATA_BYTES];
        let mut index = 0u32;
        loop {
            let mut name_length = units(&name);
            let mut data_length = bytes(&data);
            let mut raw_type = 0u32;
            // SAFETY: `name` holds `name_length` writable UTF-16 units and
            // `data` holds `data_length` writable bytes.
            let status = unsafe {
                RegEnumValueW(
                    handle.0,
                    index,
                    name.as_mut_ptr(),
                    &mut name_length,
                    ptr::null(),
                    &mut raw_type,
                    data.as_mut_ptr(),
                    &mut data_length,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    let length = usize::try_from(data_length).unwrap_or(data.len());
                    let bytes = data.get(..length).unwrap_or_default().to_vec();
                    values.push((text(&name, name_length), RawValue::new(raw_type, bytes)));
                    index += 1;
                }
                ERROR_MORE_DATA => grow(&mut data, data_length, key)?,
                ERROR_NO_MORE_ITEMS => return Ok(values),
                code => return Err(error(code, "enumerate_values", key)),
            }
        }
    }

    fn value(&self, view: RegistryView, key: &KeyPath, name: &str) -> FsResult<Option<RawValue>> {
        let handle = open_existing(view, key, KEY_QUERY_VALUE)?;
        query_value(&handle, key, name)
    }

    fn create_key(
        &self,
        view: RegistryView,
        key: &KeyPath,
        class: &str,
        volatile: bool,
    ) -> FsResult<bool> {
        let parent_path = key.parent().ok_or_else(|| denied(key))?;
        let parent = open_existing(view, &parent_path, KEY_CREATE_SUB_KEY)?;
        let name = wide(key.name());
        let class = wide(class);
        let options = if volatile {
            REG_OPTION_VOLATILE
        } else {
            REG_OPTION_NON_VOLATILE
        };
        let mut handle: HKEY = ptr::null_mut();
        let mut disposition = 0u32;
        // SAFETY: `name` and `class` are NUL-terminated UTF-16 strings that
        // outlive the call; the handle and disposition outputs are valid.
        let status = unsafe {
            RegCreateKeyExW(
                parent.0,
                name.as_ptr(),
                0,
                class.as_ptr(),
                options,
                KEY_QUERY_VALUE | view_flag(view),
                ptr::null(),
                &mut handle,
                &mut disposition,
            )
        };
        check(status, "create_key", key)?;
        drop(Key(handle));
        Ok(disposition == REG_CREATED_NEW_KEY)
    }

    fn set_value(
        &self,
        view: RegistryView,
        key: &KeyPath,
        name: &str,
        value: &RawValue,
    ) -> FsResult<()> {
        let handle = open_existing(view, key, KEY_SET_VALUE)?;
        let name = wide(name);
        let length = u32::try_from(value.bytes.len()).map_err(|_| {
            FsError::new(FsErrorKind::Io, "set_value", Some(language(key).as_ref()))
        })?;
        // SAFETY: `name` is a NUL-terminated UTF-16 string and the data
        // pointer is valid for `length` bytes; both outlive the call.
        let status = unsafe {
            RegSetValueExW(
                handle.0,
                name.as_ptr(),
                0,
                value.raw_type,
                value.bytes.as_ptr(),
                length,
            )
        };
        check(status, "set_value", key)
    }

    fn delete_value(&self, view: RegistryView, key: &KeyPath, name: &str) -> FsResult<()> {
        let handle = open_existing(view, key, KEY_SET_VALUE)?;
        let name = wide(name);
        // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the
        // call.
        let status = unsafe { RegDeleteValueW(handle.0, name.as_ptr()) };
        check(status, "delete_value", key)
    }

    fn delete_tree(&self, view: RegistryView, key: &KeyPath) -> FsResult<()> {
        let parent_path = key.parent().ok_or_else(|| denied(key))?;
        {
            let handle = open_existing(
                view,
                key,
                DELETE | KEY_ENUMERATE_SUB_KEYS | KEY_QUERY_VALUE | KEY_SET_VALUE,
            )?;
            // SAFETY: a null subkey deletes the content of the open key,
            // which stays valid for the call.
            let status = unsafe { RegDeleteTreeW(handle.0, ptr::null()) };
            check(status, "delete_tree", key)?;
        }
        let parent = open_existing(view, &parent_path, KEY_ENUMERATE_SUB_KEYS)?;
        let name = wide(key.name());
        // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the
        // call; the view flag selects the same view the key was opened in.
        let status = unsafe { RegDeleteKeyExW(parent.0, name.as_ptr(), view_flag(view), 0) };
        check(status, "delete_key", key)
    }

    fn security(&self, view: RegistryView, key: &KeyPath) -> FsResult<Option<Vec<u8>>> {
        let handle = match open(view, key, READ_CONTROL) {
            Ok(Some(handle)) => handle,
            Ok(None) => return Err(missing(key)),
            Err(error) if error.kind == FsErrorKind::PermissionDenied => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut length = 0u32;
        let mut buffer: Vec<u32> = Vec::new();
        loop {
            let mut capacity = u32::try_from(buffer.len() * size_of::<u32>()).unwrap_or(u32::MAX);
            // SAFETY: `buffer` is aligned for a security descriptor and holds
            // `capacity` writable bytes; an empty buffer only asks for the
            // required size.
            let status = unsafe {
                RegGetKeySecurity(
                    handle.0,
                    DACL_SECURITY_INFORMATION,
                    buffer.as_mut_ptr().cast(),
                    &mut capacity,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    length = capacity;
                    break;
                }
                ERROR_INSUFFICIENT_BUFFER => {
                    let words = usize::try_from(capacity)
                        .unwrap_or(usize::MAX)
                        .div_ceil(size_of::<u32>());
                    buffer.resize(words, 0);
                }
                ERROR_ACCESS_DENIED => return Ok(None),
                code => return Err(error(code, "get_security", key)),
            }
        }
        let mut descriptor: Vec<u8> = buffer.iter().flat_map(|word| word.to_ne_bytes()).collect();
        descriptor.truncate(usize::try_from(length).unwrap_or(descriptor.len()));
        Ok(Some(descriptor))
    }

    fn set_security(&self, view: RegistryView, key: &KeyPath, descriptor: &[u8]) -> FsResult<()> {
        let handle = open_existing(view, key, WRITE_DAC)?;
        let mut aligned: Vec<u32> = descriptor
            .chunks(size_of::<u32>())
            .map(|chunk| {
                let mut word = [0u8; size_of::<u32>()];
                word.iter_mut()
                    .zip(chunk)
                    .for_each(|(slot, byte)| *slot = *byte);
                u32::from_ne_bytes(word)
            })
            .collect();
        // SAFETY: `aligned` holds the bytes of a self-relative security
        // descriptor read by `RegGetKeySecurity`, with the alignment the
        // descriptor requires, and outlives the call.
        let status = unsafe {
            RegSetKeySecurity(
                handle.0,
                DACL_SECURITY_INFORMATION,
                aligned.as_mut_ptr().cast(),
            )
        };
        check(status, "set_security", key)
    }
}

struct Counts {
    class: String,
    subkeys: u32,
    values: u32,
    last_write: Instant,
}

fn query_info(handle: &Key, key: &KeyPath) -> FsResult<Counts> {
    let mut class = vec![0u16; MAX_CLASS_UNITS];
    let mut class_length = units(&class);
    let mut subkeys = 0u32;
    let mut values = 0u32;
    let mut written = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: `class` holds `class_length` writable UTF-16 units, the other
    // outputs are valid locations, and unused outputs are null.
    let status = unsafe {
        RegQueryInfoKeyW(
            handle.0,
            class.as_mut_ptr(),
            &mut class_length,
            ptr::null(),
            &mut subkeys,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut values,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut written,
        )
    };
    check(status, "query_key", key)?;
    Ok(Counts {
        class: text(&class, class_length),
        subkeys,
        values,
        last_write: filetime_to_instant(&written),
    })
}

fn query_value(handle: &Key, key: &KeyPath, name: &str) -> FsResult<Option<RawValue>> {
    let name = wide(name);
    let mut data = vec![0u8; INITIAL_DATA_BYTES];
    loop {
        let mut length = bytes(&data);
        let mut raw_type = 0u32;
        // SAFETY: `name` is a NUL-terminated UTF-16 string and `data` holds
        // `length` writable bytes; both outlive the call.
        let status = unsafe {
            RegQueryValueExW(
                handle.0,
                name.as_ptr(),
                ptr::null(),
                &mut raw_type,
                data.as_mut_ptr(),
                &mut length,
            )
        };
        match status {
            ERROR_SUCCESS => {
                data.truncate(usize::try_from(length).unwrap_or(data.len()));
                return Ok(Some(RawValue::new(raw_type, data)));
            }
            ERROR_MORE_DATA => grow(&mut data, length, key)?,
            ERROR_FILE_NOT_FOUND => return Ok(None),
            code => return Err(error(code, "query_value", key)),
        }
    }
}

fn open(view: RegistryView, key: &KeyPath, access: REG_SAM_FLAGS) -> FsResult<Option<Key>> {
    let subkey = wide(&key.native_subkey());
    let mut handle: HKEY = ptr::null_mut();
    // SAFETY: `subkey` is a NUL-terminated UTF-16 string that outlives the
    // call and `handle` is a valid output location; the predefined root
    // handles need not be opened.
    let status = unsafe {
        RegOpenKeyExW(
            hive_handle(key.hive()),
            subkey.as_ptr(),
            REG_OPTION_OPEN_LINK,
            access | view_flag(view),
            &mut handle,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(Key(handle))),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        code => Err(error(code, "open_key", key)),
    }
}

fn open_existing(view: RegistryView, key: &KeyPath, access: REG_SAM_FLAGS) -> FsResult<Key> {
    open(view, key, access)?.ok_or_else(|| missing(key))
}

fn hive_handle(hive: Hive) -> HKEY {
    match hive {
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::ClassesRoot => HKEY_CLASSES_ROOT,
        Hive::Users => HKEY_USERS,
        Hive::CurrentConfig => HKEY_CURRENT_CONFIG,
    }
}

fn view_flag(view: RegistryView) -> REG_SAM_FLAGS {
    if view == RegistryView::Bits32 {
        KEY_WOW64_32KEY
    } else {
        KEY_WOW64_64KEY
    }
}

fn grow(data: &mut Vec<u8>, required: u32, key: &KeyPath) -> FsResult<()> {
    let required = usize::try_from(required).unwrap_or(usize::MAX);
    let size = required.max(data.len().saturating_mul(2));
    if size > MAX_DATA_BYTES {
        return Err(FsError::new(
            FsErrorKind::Io,
            "query_value",
            Some(language(key).as_ref()),
        ));
    }
    data.resize(size, 0);
    Ok(())
}

fn filetime_to_instant(time: &FILETIME) -> Instant {
    let ticks = (i64::from(time.dwHighDateTime) << u32::BITS) | i64::from(time.dwLowDateTime);
    let nanos = ticks
        .saturating_sub(FILETIME_UNIX_OFFSET)
        .saturating_mul(FILETIME_TICK_NANOS);
    Instant::from_unix_nanos(nanos)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(iter::once(0)).collect()
}

fn text(buffer: &[u16], length: u32) -> String {
    let length = usize::try_from(length).unwrap_or(buffer.len());
    String::from_utf16_lossy(buffer.get(..length).unwrap_or(buffer))
}

fn units(buffer: &[u16]) -> u32 {
    u32::try_from(buffer.len()).unwrap_or(u32::MAX)
}

fn bytes(buffer: &[u8]) -> u32 {
    u32::try_from(buffer.len()).unwrap_or(u32::MAX)
}

fn check(status: WIN32_ERROR, op: &str, key: &KeyPath) -> FsResult<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(error(status, op, key))
    }
}

fn error(code: WIN32_ERROR, op: &str, key: &KeyPath) -> FsError {
    let kind = match code {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_KEY_DELETED => FsErrorKind::NotFound,
        ERROR_ACCESS_DENIED => FsErrorKind::PermissionDenied,
        ERROR_PRIVILEGE_NOT_HELD => FsErrorKind::Privilege,
        ERROR_CHILD_MUST_BE_VOLATILE => FsErrorKind::Unsupported,
        _ => FsErrorKind::Io,
    };
    let mut error = FsError::new(kind, op, Some(language(key).as_ref()));
    error.os_code = i32::try_from(code).ok();
    error
}

fn missing(key: &KeyPath) -> FsError {
    FsError::new(
        FsErrorKind::NotFound,
        "open_key",
        Some(language(key).as_ref()),
    )
}

fn denied(key: &KeyPath) -> FsError {
    FsError::new(
        FsErrorKind::PermissionDenied,
        "registry",
        Some(language(key).as_ref()),
    )
}

fn language(key: &KeyPath) -> Box<Path> {
    Path::new(&key.to_language_string()).into()
}
