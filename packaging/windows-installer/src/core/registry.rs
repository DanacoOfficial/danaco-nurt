// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/registry.rs
// Responsibility:   Registry access of the installation: the entry of the list of installed applications, App Paths, PATH and shell folders.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registry access of the installation.
//!
//! The functions that change the registry return what they replaced, so that the
//! rollback of a failed run can put it back and the journal can record it.

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_BINARY, REG_DWORD,
    REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, RegType,
};
use winreg::{RegKey, RegValue};

use crate::model::{Layout, Scope};
use crate::paths::expand_env;
use crate::sys;
use crate::textedit::{path_add, path_remove};

/// Key of the entry in "Installed apps".
pub(crate) const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\DanacoNurt";

/// Key of the registration of `nurt.exe` for the "Run" dialog.
pub(crate) const APP_PATHS_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\App Paths\nurt.exe";

const USER_ENVIRONMENT: &str = "Environment";
const MACHINE_ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// A value of a key, kept as bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredValue {
    /// Name of the value.
    pub(crate) name: String,
    /// Type of the value, the number of the registry.
    pub(crate) kind: u32,
    /// The bytes as hexadecimal text.
    pub(crate) data: String,
}

/// The state of a key before the setup changed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Snapshot {
    /// The key existed.
    pub(crate) existed: bool,
    /// Its values.
    pub(crate) values: Vec<StoredValue>,
}

/// A key that the setup wrote with its state before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyRecord {
    /// Path of the key below the hive.
    pub(crate) key: String,
    /// The state before.
    pub(crate) previous: Snapshot,
}

/// Values of the entry in "Installed apps".
#[derive(Debug, Clone)]
pub(crate) struct UninstallEntry {
    /// Installed version.
    pub(crate) version: String,
    /// Places of the installation.
    pub(crate) layout: Layout,
    /// Size of the installation in kibibytes.
    pub(crate) size_kib: u32,
    /// Installation date, `YYYYMMDD`.
    pub(crate) date: String,
}

fn root(scope: Scope) -> RegKey {
    RegKey::predef(match scope {
        Scope::User => HKEY_CURRENT_USER,
        Scope::Machine => HKEY_LOCAL_MACHINE,
    })
}

fn kind_of(number: u32) -> RegType {
    match number {
        0 => RegType::REG_NONE,
        1 => REG_SZ,
        2 => REG_EXPAND_SZ,
        4 => REG_DWORD,
        7 => REG_MULTI_SZ,
        11 => REG_QWORD,
        _ => REG_BINARY,
    }
}

fn number_of(kind: &RegType) -> u32 {
    match kind {
        RegType::REG_NONE => 0,
        RegType::REG_SZ => 1,
        RegType::REG_EXPAND_SZ => 2,
        RegType::REG_DWORD => 4,
        RegType::REG_MULTI_SZ => 7,
        RegType::REG_QWORD => 11,
        _ => 3,
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .filter_map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok())
        .collect()
}

/// Takes the state of a key.
pub(crate) fn capture(scope: Scope, path: &str) -> io::Result<Snapshot> {
    let Ok(key) = root(scope).open_subkey_with_flags(path, KEY_READ) else {
        return Ok(Snapshot::default());
    };
    let mut values = Vec::new();
    for item in key.enum_values() {
        let (name, value) = item?;
        values.push(StoredValue {
            name,
            kind: number_of(&value.vtype),
            data: to_hex(&value.bytes),
        });
    }
    Ok(Snapshot {
        existed: true,
        values,
    })
}

/// Puts a key back to a state taken earlier.
pub(crate) fn restore(scope: Scope, path: &str, snapshot: &Snapshot) -> io::Result<()> {
    delete_tree(scope, path)?;
    if !snapshot.existed {
        return Ok(());
    }
    let (key, _) = root(scope).create_subkey(path)?;
    for value in &snapshot.values {
        let bytes = from_hex(&value.data);
        key.set_raw_value(
            &value.name,
            &RegValue {
                bytes: Cow::Borrowed(&bytes),
                vtype: kind_of(value.kind),
            },
        )?;
    }
    Ok(())
}

/// Deletes a key with its subkeys; a key that does not exist is not an error.
pub(crate) fn delete_tree(scope: Scope, path: &str) -> io::Result<()> {
    match root(scope).delete_subkey_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// An installation found in the registry, the machine one first.
pub(crate) fn find_installation() -> Option<(Scope, PathBuf, String)> {
    [Scope::Machine, Scope::User].into_iter().find_map(|scope| {
        let key = root(scope)
            .open_subkey_with_flags(UNINSTALL_KEY, KEY_READ)
            .ok()?;
        let dir: String = key.get_value("InstallLocation").ok()?;
        let version: String = key.get_value("DisplayVersion").unwrap_or_default();
        Some((scope, PathBuf::from(dir), version))
    })
}

/// Writes the entry in "Installed apps".
pub(crate) fn write_uninstall(scope: Scope, entry: &UninstallEntry) -> io::Result<()> {
    let (key, _) = root(scope).create_subkey(UNINSTALL_KEY)?;
    let uninstaller = entry.layout.uninstaller().display().to_string();
    let parts: Vec<u32> = entry
        .version
        .split(['.', '-'])
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    key.set_value("DisplayName", &"Danaco Nurt")?;
    key.set_value("DisplayVersion", &entry.version.as_str())?;
    key.set_value("Publisher", &"Danaco Holding Group Sp. z o.o.")?;
    key.set_value("InstallLocation", &entry.layout.root.display().to_string())?;
    key.set_value("InstallDate", &entry.date.as_str())?;
    key.set_value("DisplayIcon", &entry.layout.icon().display().to_string())?;
    key.set_value("UninstallString", &format!("\"{uninstaller}\" --undo"))?;
    key.set_value(
        "QuietUninstallString",
        &format!("\"{uninstaller}\" --undo /S"),
    )?;
    key.set_value(
        "URLInfoAbout",
        &"https://github.com/DanacoOfficial/danaco-nurt",
    )?;
    key.set_value("HelpLink", &"https://github.com/DanacoOfficial/danaco-nurt")?;
    key.set_value(
        "Comments",
        &"Danaco Nurt: a command shell and language where every change can be undone",
    )?;
    key.set_value("EstimatedSize", &entry.size_kib)?;
    key.set_value("VersionMajor", &parts.first().copied().unwrap_or(0))?;
    key.set_value("VersionMinor", &parts.get(1).copied().unwrap_or(0))?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    Ok(())
}

/// Registers `nurt.exe` for the "Run" dialog.
pub(crate) fn write_app_paths(scope: Scope, layout: &Layout) -> io::Result<()> {
    let (key, _) = root(scope).create_subkey(APP_PATHS_KEY)?;
    key.set_value("", &layout.exe().display().to_string())?;
    key.set_value("Path", &layout.bin.display().to_string())?;
    Ok(())
}

fn environment_key(scope: Scope) -> io::Result<RegKey> {
    let path = if scope == Scope::Machine {
        MACHINE_ENVIRONMENT
    } else {
        USER_ENVIRONMENT
    };
    root(scope).open_subkey_with_flags(path, KEY_READ | KEY_WRITE)
}

fn decode(value: &RegValue<'_>) -> String {
    let units: Vec<u16> = value
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_string()
}

fn encode(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Reads the unexpanded `Path` of a scope; `None` when the value does not exist.
pub(crate) fn read_path(scope: Scope) -> io::Result<Option<String>> {
    let key = environment_key(scope)?;
    match key.get_raw_value("Path") {
        Ok(value) => Ok(Some(decode(&value))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes the unexpanded `Path` of a scope as `REG_EXPAND_SZ` and announces the change.
pub(crate) fn write_path(scope: Scope, text: Option<&str>) -> io::Result<()> {
    let key = environment_key(scope)?;
    match text {
        Some(text) => key.set_raw_value(
            "Path",
            &RegValue {
                bytes: Cow::Owned(encode(text)),
                vtype: REG_EXPAND_SZ,
            },
        )?,
        None => match key.delete_value("Path") {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        },
    }
    sys::broadcast_environment_change();
    Ok(())
}

/// Adds a directory to `Path`; returns the previous value when `Path` changed.
pub(crate) fn path_insert(scope: Scope, entry: &str) -> io::Result<Option<Option<String>>> {
    let current = read_path(scope)?;
    match path_add(current.as_deref().unwrap_or(""), entry) {
        Some(updated) => {
            write_path(scope, Some(&updated))?;
            Ok(Some(current))
        }
        None => Ok(None),
    }
}

/// Removes a directory from `Path`; returns whether `Path` changed.
pub(crate) fn path_delete(scope: Scope, entry: &str) -> io::Result<bool> {
    let Some(current) = read_path(scope)? else {
        return Ok(false);
    };
    match path_remove(&current, entry) {
        Some(updated) => {
            write_path(scope, Some(&updated))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Reads a folder of the user from the shell folders of the registry and expands the variables.
pub(crate) fn shell_folder(name: &str) -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
            KEY_READ,
        )
        .ok()?;
    let value: String = key.get_value(name).ok()?;
    Some(PathBuf::from(expand_env(&value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = vec![0u8, 1, 0xab, 0xff, 0x10];
        assert_eq!(from_hex(&to_hex(&bytes)), bytes);
    }

    #[test]
    fn types_round_trip_through_numbers() {
        for number in [0u32, 1, 2, 4, 7, 11] {
            assert_eq!(number_of(&kind_of(number)), number);
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
