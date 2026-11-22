// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Identity keys of paths on case-insensitive and normalising volumes.
//!
//! Two spellings of a path denote the same entry exactly when their keys are
//! equal. The key depends on the operating system and on whether the volume
//! distinguishes letter case:
//!
//! | System | Case-sensitive volume | Case-insensitive volume |
//! |---|---|---|
//! | Linux | raw bytes | NFC, then lower-case folding |
//! | macOS | NFC | NFC, then lower-case folding |
//! | Windows | NFC | NFC, then simple upper-case mapping |
//!
//! Windows compares names by the volume upper-case table (`$UpCase`), which
//! maps one UTF-16 unit to one unit; the key applies the Unicode simple
//! upper-case mapping and keeps characters whose upper-case form has more than
//! one character (for example `ß`). macOS and case-folding Linux volumes fold
//! with the full lower-case mapping. Names that are not valid Unicode keep
//! their raw bytes.

use std::ffi::OsStr;
use std::path::{Component, Path};

use nurt_contracts::env::Os;
use nurt_contracts::fs::PathKey;
use unicode_normalization::UnicodeNormalization;

const SEPARATOR: u8 = b'/';

/// Key of one path component.
///
/// # Examples
///
/// ```
/// use std::ffi::OsStr;
/// use nurt_contracts::env::Os;
/// use nurt_fs::key::name_key;
///
/// let upper = name_key(Os::Windows, false, OsStr::new("README.md"));
/// let lower = name_key(Os::Windows, false, OsStr::new("readme.MD"));
/// assert_eq!(upper, lower);
/// assert_ne!(
///     name_key(Os::Linux, true, OsStr::new("README.md")),
///     name_key(Os::Linux, true, OsStr::new("readme.md")),
/// );
/// ```
#[must_use]
pub fn name_key(os: Os, case_sensitive: bool, name: &OsStr) -> Vec<u8> {
    let raw = name.as_encoded_bytes();
    if case_sensitive && os == Os::Linux {
        return raw.to_vec();
    }
    let Some(text) = name.to_str() else {
        return raw.to_vec();
    };
    let composed: String = text.nfc().collect();
    if case_sensitive {
        return composed.into_bytes();
    }
    let folded: String = if os == Os::Windows {
        composed.chars().map(simple_upper).collect()
    } else {
        composed.chars().flat_map(char::to_lowercase).collect()
    };
    folded.nfc().collect::<String>().into_bytes()
}

/// Key of an absolute path on a volume with the given case sensitivity.
///
/// Components are keyed by [`name_key`] and joined with `/`; a Windows
/// prefix (drive or UNC share) is keyed like a name.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use nurt_contracts::env::Os;
/// use nurt_fs::key::path_key;
///
/// let composed = path_key(Os::Macos, false, Path::new("/Users/user/Caf\u{e9}"));
/// let decomposed = path_key(Os::Macos, false, Path::new("/users/USER/cafe\u{301}"));
/// assert_eq!(composed, decomposed);
/// ```
#[must_use]
pub fn path_key(os: Os, case_sensitive: bool, path: &Path) -> PathKey {
    let mut out = Vec::with_capacity(path.as_os_str().len());
    for component in path.components() {
        match component {
            Component::RootDir => out.push(SEPARATOR),
            Component::Prefix(prefix) => {
                out.extend(name_key(os, case_sensitive, prefix.as_os_str()));
            }
            Component::CurDir | Component::ParentDir | Component::Normal(_) => {
                if out.last().is_some_and(|last| *last != SEPARATOR) {
                    out.push(SEPARATOR);
                }
                out.extend(name_key(os, case_sensitive, component.as_os_str()));
            }
        }
    }
    PathKey(out)
}

fn simple_upper(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEMS: [Os; 3] = [Os::Linux, Os::Macos, Os::Windows];

    #[test]
    fn case_insensitive_volume_equates_case_and_normalisation_variants() {
        for os in SYSTEMS {
            let a = path_key(os, false, Path::new("/Data/R\u{e9}sum\u{e9}.TXT"));
            let b = path_key(os, false, Path::new("/data/re\u{301}sume\u{301}.txt"));
            assert_eq!(a, b, "{os:?}");
        }
    }

    #[test]
    fn case_sensitive_volume_distinguishes_case() {
        for os in SYSTEMS {
            let a = path_key(os, true, Path::new("/Data/file"));
            let b = path_key(os, true, Path::new("/data/file"));
            assert_ne!(a, b, "{os:?}");
        }
    }

    #[test]
    fn linux_case_sensitive_key_keeps_raw_bytes() {
        let composed = path_key(Os::Linux, true, Path::new("/caf\u{e9}"));
        let decomposed = path_key(Os::Linux, true, Path::new("/cafe\u{301}"));
        assert_ne!(composed, decomposed);
        assert_eq!(composed.0, "/caf\u{e9}".as_bytes());
    }

    #[test]
    fn macos_and_windows_case_sensitive_keys_equate_normalisation_forms() {
        for os in [Os::Macos, Os::Windows] {
            let composed = path_key(os, true, Path::new("/caf\u{e9}"));
            let decomposed = path_key(os, true, Path::new("/cafe\u{301}"));
            assert_eq!(composed, decomposed, "{os:?}");
        }
    }

    #[test]
    fn windows_simple_mapping_keeps_sharp_s() {
        let sharp = name_key(Os::Windows, false, OsStr::new("stra\u{df}e"));
        let double = name_key(Os::Windows, false, OsStr::new("STRASSE"));
        assert_ne!(sharp, double);
        assert_eq!(sharp, "STRA\u{df}E".as_bytes());
    }

    #[test]
    fn lower_case_folding_handles_dotted_capital_i() {
        let dotted = name_key(Os::Macos, false, OsStr::new("\u{130}"));
        assert_eq!(dotted, "i\u{307}".as_bytes());
    }

    #[test]
    fn root_and_nested_components_are_separated_once() {
        assert_eq!(path_key(Os::Linux, true, Path::new("/")).0, b"/");
        assert_eq!(path_key(Os::Linux, true, Path::new("/a/b")).0, b"/a/b");
    }

    #[cfg(unix)]
    #[test]
    fn invalid_unicode_names_keep_raw_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let name = OsStr::from_bytes(b"bad\xff");
        assert_eq!(name_key(Os::Macos, false, name), b"bad\xff");
    }
}
