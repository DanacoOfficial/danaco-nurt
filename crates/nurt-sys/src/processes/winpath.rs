// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::processes::winpath
// Responsibility:   Canonical form of Windows paths and wide strings returned by Win32.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Canonical form of Windows paths and wide strings returned by Win32.

/// Prefix of extended-length paths.
const VERBATIM_PREFIX: &str = r"\\?\";

/// Prefix of extended-length network paths.
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

/// Canonical form of a Windows path: `/` as separator, upper-case drive
/// letter, without the extended-length prefix.
pub(crate) fn canonical(path: &str) -> String {
    let path = if let Some(share) = path.strip_prefix(VERBATIM_UNC_PREFIX) {
        format!(r"\\{share}")
    } else {
        path.strip_prefix(VERBATIM_PREFIX)
            .unwrap_or(path)
            .to_owned()
    };
    let mut canonical = path.replace('\\', "/");
    if canonical.as_bytes().get(1) == Some(&b':')
        && let Some(drive) = canonical.get_mut(..1)
    {
        drive.make_ascii_uppercase();
    }
    canonical
}

/// Text of a wide string, up to its first zero.
pub(crate) fn from_wide(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(units.get(..end).unwrap_or(units))
}

/// Wide string with a terminating zero.
#[cfg(windows)]
pub(crate) fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
