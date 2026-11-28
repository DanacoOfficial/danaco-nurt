// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/requirements.rs
// Responsibility:   Check of the requirements shown on the first screen: Windows version, architecture, free space and the embedded archive.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Check of the requirements.
//!
//! The rows carry data only (values found and required); the interface supplies the words.

use std::path::Path;

use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

use crate::model::Requirement;
use crate::payload;
use crate::sys::{self, Machine};

/// Oldest supported build of Windows 10 (version 1809).
pub(crate) const MINIMUM_BUILD: u32 = 17763;

/// Margin of free space beyond the unpacked files, in bytes.
const SPACE_MARGIN: u64 = 16 * 1024 * 1024;

/// Version of Windows read from the registry.
#[derive(Debug, Clone, Default)]
pub(crate) struct WindowsVersion {
    /// Build number.
    pub(crate) build: u32,
    /// Name with the edition, for example `Windows 11 Pro`.
    pub(crate) name: String,
}

/// Reads the version of Windows.
pub(crate) fn windows_version() -> WindowsVersion {
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", KEY_READ)
    else {
        return WindowsVersion::default();
    };
    let build: u32 = key
        .get_value::<String, _>("CurrentBuildNumber")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    let product: String = key.get_value("ProductName").unwrap_or_default();
    let edition = product
        .trim_start_matches("Windows 10 ")
        .trim_start_matches("Windows 11 ")
        .trim_start_matches("Windows ");
    let family = if build >= 22000 {
        "Windows 11"
    } else if product.starts_with("Windows Server") {
        ""
    } else {
        "Windows 10"
    };
    let name = if family.is_empty() {
        product.clone()
    } else {
        format!("{family} {edition}").trim().to_string()
    };
    WindowsVersion { build, name }
}

/// Space that an installation needs, in bytes, with the documentation or
/// without it.
pub(crate) fn required_space_for(docs: bool) -> u64 {
    let uninstaller = std::env::current_exe()
        .and_then(std::fs::metadata)
        .map_or(0, |meta| meta.len());
    payload::installed_size(docs) + uninstaller + payload::ICON.len() as u64 + SPACE_MARGIN
}

/// Architecture of this setup as the design names it: `x86_64` or `aarch64`.
pub(crate) fn setup_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

/// Checks every requirement; `directory` is the default folder of `nurt.exe`
/// and `docs` tells whether the documentation is installed.
pub(crate) fn check(directory: &Path, docs: bool) -> Vec<Requirement> {
    let version = windows_version();
    let mut rows = Vec::new();
    rows.push(Requirement {
        id: "windows",
        ok: version.build >= MINIMUM_BUILD,
        found: if version.build == 0 {
            String::new()
        } else {
            format!("{} (10.0.{})", version.name, version.build)
        },
        required: format!("10.0.{MINIMUM_BUILD}"),
    });
    let setup = setup_arch();
    let native = sys::native_machine();
    let machine = match native {
        Machine::X64 => "x86_64",
        Machine::Arm64 => "aarch64",
        Machine::Other => "other",
    };
    // An x86_64 setup runs on aarch64 Windows through the emulation of the system.
    let ok = native == Machine::Arm64 && setup == "aarch64"
        || native == Machine::X64 && setup == "x86_64"
        || native == Machine::Arm64 && setup == "x86_64";
    rows.push(Requirement {
        id: "architecture",
        ok,
        found: machine.to_string(),
        required: setup.to_string(),
    });
    let required = required_space_for(docs);
    let free = crate::paths::nearest_existing(directory)
        .and_then(|path| sys::free_space(&path))
        .unwrap_or(0);
    rows.push(Requirement {
        id: "space",
        ok: free >= required,
        found: free.to_string(),
        required: required.to_string(),
    });
    rows.push(Requirement {
        id: "package",
        ok: payload::present() && !payload::expected_sum().is_empty(),
        found: payload::expected_sum().chars().take(12).collect(),
        required: "SHA-256".into(),
    });
    rows
}

/// A requirement that blocks the installation failed.
pub(crate) fn blocked(rows: &[Requirement]) -> bool {
    rows.iter().any(|row| !row.ok)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
