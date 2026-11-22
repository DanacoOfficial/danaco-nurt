// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Path normalisation, reserved names and long-path handling.
//!
//! Every function takes the path style explicitly, so the rules of both
//! systems apply and are testable on any host; [`host_style`] selects the
//! style of the running system.
//!
//! | Form | Windows example | POSIX example |
//! |---|---|---|
//! | language | `C:/Users/user/a.txt`, `//server/share/a.txt` | `/home/user/a.txt` |
//! | native | `C:\Users\user\a.txt`, `\\server\share\a.txt` | `/home/user/a.txt` |
//! | system call | `\\?\C:\Users\user\a.txt`, `\\?\UNC\server\share\a.txt` | `/home/user/a.txt` |

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt;

use nurt_contracts::env::PathStyle;
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

const LONG_PREFIX: &str = r"\\?\";
const LONG_UNC_PREFIX: &str = r"\\?\UNC\";
const DEVICE_PREFIX: &str = r"\\.\";
const WINDOWS_FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*', '\\', '/'];
const DEVICE_NAMES: &[&str] = &["CON", "PRN", "AUX", "NUL"];
const NUMBERED_DEVICES: &[&str] = &["COM", "LPT"];
const DEVICE_SUFFIXES: &[char] = &[
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '\u{b9}', '\u{b2}', '\u{b3}',
];

/// Path style of the running system.
#[must_use]
pub fn host_style() -> PathStyle {
    if cfg!(windows) {
        PathStyle::Windows
    } else {
        PathStyle::Posix
    }
}

/// Notation of a path text, independent of the system that wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notation {
    /// Starts at `/` (`/home/user`).
    Posix,
    /// Starts with a drive letter and a separator (`C:/dir`, `c:\dir`).
    Drive,
    /// Network share (`//server/share`, `\\server\share`).
    Unc,
    /// Windows long-path or device namespace (`\\?\C:\dir`, `\\.\pipe`).
    Namespace,
    /// Anything else, including drive-relative forms such as `C:dir`.
    Relative,
}

/// Notation of `text` as written in the given style.
///
/// In POSIX style a leading `//` is an ordinary absolute path; only
/// backslash-separated shares count as UNC.
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::PathStyle;
/// use nurt_fs::path::{Notation, notation};
///
/// assert_eq!(notation("C:/Users", PathStyle::Windows), Notation::Drive);
/// assert_eq!(notation("//server/share", PathStyle::Windows), Notation::Unc);
/// assert_eq!(notation("//server/share", PathStyle::Posix), Notation::Posix);
/// ```
#[must_use]
pub fn notation(text: &str, style: PathStyle) -> Notation {
    if text.starts_with(LONG_PREFIX) || text.starts_with(DEVICE_PREFIX) {
        return Notation::Namespace;
    }
    let windows = style == PathStyle::Windows;
    if text.starts_with(r"\\") || (windows && text.starts_with("//")) {
        return Notation::Unc;
    }
    let bytes = text.as_bytes();
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && matches!(bytes.get(2), Some(b'/' | b'\\'));
    if drive {
        Notation::Drive
    } else if text.starts_with('/') {
        Notation::Posix
    } else {
        Notation::Relative
    }
}

/// Whether `text` is an absolute path in the notation of the other system:
/// a drive, share or namespace path in POSIX style, or a `/`-rooted path
/// without a drive in Windows style.
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::PathStyle;
/// use nurt_fs::path::is_foreign;
///
/// assert!(is_foreign("C:/Users/user", PathStyle::Posix));
/// assert!(is_foreign("/home/user", PathStyle::Windows));
/// assert!(!is_foreign("/home/user", PathStyle::Posix));
/// ```
#[must_use]
pub fn is_foreign(text: &str, style: PathStyle) -> bool {
    match notation(text, style) {
        Notation::Drive | Notation::Unc | Notation::Namespace => style != PathStyle::Windows,
        Notation::Posix => style == PathStyle::Windows,
        Notation::Relative => false,
    }
}

fn upper_drive(text: &mut String) {
    let bytes = text.as_bytes();
    if bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && let Some(drive) = text.get_mut(..1)
    {
        drive.make_ascii_uppercase();
    }
}

/// Language form of a native path: `/` separators, an upper-case drive
/// letter and shares as `//server/share`; the long-path prefixes are
/// removed. POSIX paths are returned unchanged.
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::PathStyle;
/// use nurt_fs::path::to_language;
///
/// assert_eq!(to_language(r"c:\Users\user", PathStyle::Windows), "C:/Users/user");
/// assert_eq!(to_language(r"\\?\UNC\srv\share\a", PathStyle::Windows), "//srv/share/a");
/// ```
#[must_use]
pub fn to_language(native: &str, style: PathStyle) -> String {
    if style != PathStyle::Windows {
        return native.to_owned();
    }
    let mut text = if let Some(rest) = native.strip_prefix(LONG_UNC_PREFIX) {
        format!(r"\\{rest}")
    } else if let Some(rest) = native.strip_prefix(LONG_PREFIX) {
        rest.to_owned()
    } else {
        native.to_owned()
    };
    text = text.replace('\\', "/");
    upper_drive(&mut text);
    text
}

/// Native form of a language path: `\` separators and an upper-case drive
/// letter in Windows style; POSIX paths are returned unchanged.
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::PathStyle;
/// use nurt_fs::path::to_native;
///
/// assert_eq!(to_native("c:/dir/a.txt", PathStyle::Windows), r"C:\dir\a.txt");
/// assert_eq!(to_native("//srv/share", PathStyle::Windows), r"\\srv\share");
/// ```
#[must_use]
pub fn to_native(language: &str, style: PathStyle) -> String {
    if style != PathStyle::Windows {
        return language.to_owned();
    }
    let mut text = language.replace('/', "\\");
    upper_drive(&mut text);
    text
}

/// Path for Windows system calls: absolute drive paths receive `\\?\` and
/// shares `\\?\UNC\`, which lifts the length limit and disables the
/// interpretation of reserved names and trailing dots. Paths already in a
/// namespace and relative paths are returned in native form only.
///
/// # Examples
///
/// ```
/// use nurt_fs::path::to_system_path;
///
/// assert_eq!(to_system_path("C:/dir/nul.txt"), r"\\?\C:\dir\nul.txt");
/// assert_eq!(to_system_path("//srv/share/a"), r"\\?\UNC\srv\share\a");
/// assert_eq!(to_system_path(r"\\?\C:\x"), r"\\?\C:\x");
/// ```
#[must_use]
pub fn to_system_path(path: &str) -> String {
    if path.starts_with(LONG_PREFIX) || path.starts_with(DEVICE_PREFIX) {
        return path.to_owned();
    }
    let native = to_native(path, PathStyle::Windows);
    match notation(&native, PathStyle::Windows) {
        Notation::Drive => format!("{LONG_PREFIX}{native}"),
        Notation::Unc => format!("{LONG_UNC_PREFIX}{}", native.trim_start_matches('\\')),
        Notation::Posix | Notation::Namespace | Notation::Relative => native,
    }
}

/// Reason why a text is not a valid single path component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameError {
    /// The name is empty.
    Empty,
    /// The name is `.` or `..`.
    Dot,
    /// The name contains a path separator.
    Separator,
    /// The name contains the NUL character.
    Nul,
    /// The name contains a character that Windows forbids in names.
    Forbidden(char),
    /// The name contains a control character (U+0001 to U+001F), which
    /// Windows forbids in names.
    Control(char),
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("name is empty; give a file name"),
            Self::Dot => f.write_str("`.` and `..` are not names; give a file name"),
            Self::Separator => f.write_str("name contains a path separator; give one component"),
            Self::Nul => f.write_str("name contains the NUL character; remove it"),
            Self::Forbidden(c) => {
                write!(f, "name contains `{c}`, which Windows forbids; remove it")
            }
            Self::Control(c) => write!(
                f,
                "name contains control character U+{:04X}, which Windows forbids; remove it",
                u32::from(*c)
            ),
        }
    }
}

impl std::error::Error for NameError {}

/// Checks that `name` is one valid path component in the given style.
///
/// Both styles refuse empty names, `.`, `..`, `/` and NUL. Windows style
/// also refuses `\`, `< > : " | ? *` and control characters.
///
/// # Errors
///
/// Returns the first [`NameError`] found.
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::PathStyle;
/// use nurt_fs::path::{NameError, check_name};
///
/// assert_eq!(check_name("report.txt", PathStyle::Windows), Ok(()));
/// assert_eq!(check_name("a:b", PathStyle::Posix), Ok(()));
/// assert_eq!(check_name("a:b", PathStyle::Windows), Err(NameError::Forbidden(':')));
/// ```
pub fn check_name(name: &str, style: PathStyle) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Dot);
    }
    let windows = style == PathStyle::Windows;
    for c in name.chars() {
        match c {
            '/' => return Err(NameError::Separator),
            '\0' => return Err(NameError::Nul),
            '\\' if windows => return Err(NameError::Separator),
            '\u{1}'..='\u{1f}' if windows => return Err(NameError::Control(c)),
            _ if windows && WINDOWS_FORBIDDEN.contains(&c) => return Err(NameError::Forbidden(c)),
            _ => {}
        }
    }
    Ok(())
}

/// Whether Windows treats `name` specially: a device name (`CON`, `PRN`,
/// `AUX`, `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9` and the superscript variants
/// `COM¹`–`COM³`, `LPT¹`–`LPT³`) in any letter case, also followed by an
/// extension or by spaces, or a name ending in a dot or a space.
///
/// Such names are valid only through the long-path prefix; the result is a
/// portability marker, not a refusal.
///
/// # Examples
///
/// ```
/// use nurt_fs::path::windows_reserved_name;
///
/// assert!(windows_reserved_name("nul.txt"));
/// assert!(windows_reserved_name("Com\u{b9}"));
/// assert!(windows_reserved_name("notes."));
/// assert!(!windows_reserved_name("console"));
/// ```
#[must_use]
pub fn windows_reserved_name(name: &str) -> bool {
    if name.ends_with('.') || name.ends_with(' ') {
        return true;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_uppercase();
    if DEVICE_NAMES.contains(&upper.as_str()) {
        return true;
    }
    NUMBERED_DEVICES.iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            matches!((chars.next(), chars.next()), (Some(c), None) if DEVICE_SUFFIXES.contains(&c))
        })
    })
}

/// Name of a directory entry in the form shown to the language.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryName {
    /// Name in NFC; invalid sequences are replaced by U+FFFD.
    pub name: String,
    /// Bytes as stored by the file system, present only when they differ
    /// from `name` (names stored decomposed or not valid Unicode).
    pub raw_name: Option<Vec<u8>>,
}

/// Composes a stored entry name to NFC, keeping the stored bytes when they
/// differ.
///
/// # Examples
///
/// ```
/// use std::ffi::OsStr;
/// use nurt_fs::path::entry_name;
///
/// let decomposed = entry_name(OsStr::new("cafe\u{301}"));
/// assert_eq!(decomposed.name, "caf\u{e9}");
/// assert_eq!(decomposed.raw_name.as_deref(), Some("cafe\u{301}".as_bytes()));
/// assert_eq!(entry_name(OsStr::new("plain")).raw_name, None);
/// ```
#[must_use]
pub fn entry_name(stored: &OsStr) -> EntryName {
    let raw = stored.as_encoded_bytes();
    match stored.to_str() {
        Some(text) => {
            let name = nfc(text).into_owned();
            let raw_name = (name.as_bytes() != raw).then(|| raw.to_vec());
            EntryName { name, raw_name }
        }
        None => EntryName {
            name: nfc(&stored.to_string_lossy()).into_owned(),
            raw_name: Some(raw.to_vec()),
        },
    }
}

/// `text` in Unicode normalisation form C, borrowed when already composed.
#[must_use]
pub fn nfc(text: &str) -> Cow<'_, str> {
    if is_nfc_quick(text.chars()) == IsNormalized::Yes {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.nfc().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_and_native_forms_round_trip_in_windows_style() {
        let cases = [
            ("C:/Users/user/a.txt", r"C:\Users\user\a.txt"),
            ("C:/", r"C:\"),
            ("//server/share/dir", r"\\server\share\dir"),
        ];
        for (language, native) in cases {
            assert_eq!(to_native(language, PathStyle::Windows), native);
            assert_eq!(to_language(native, PathStyle::Windows), language);
        }
        assert_eq!(to_language(r"d:\x", PathStyle::Windows), "D:/x");
        assert_eq!(
            to_language(r"\\?\C:\long\x", PathStyle::Windows),
            "C:/long/x"
        );
        assert_eq!(
            to_language(r"\\?\UNC\server\share\x", PathStyle::Windows),
            "//server/share/x"
        );
    }

    #[test]
    fn system_paths_receive_the_long_path_prefix() {
        assert_eq!(to_system_path("c:/a/b"), r"\\?\C:\a\b");
        assert_eq!(
            to_system_path(r"\\server\share\x"),
            r"\\?\UNC\server\share\x"
        );
        assert_eq!(to_system_path(r"\\.\pipe\name"), r"\\.\pipe\name");
        assert_eq!(to_system_path("relative/x"), r"relative\x");
        assert_eq!(
            to_language(&to_system_path("//server/share/x"), PathStyle::Windows),
            "//server/share/x"
        );
    }

    #[test]
    fn posix_style_keeps_paths_unchanged() {
        assert_eq!(
            to_native("/home/user/a\\b", PathStyle::Posix),
            "/home/user/a\\b"
        );
        assert_eq!(to_language("/home/user", PathStyle::Posix), "/home/user");
        assert_eq!(
            host_style(),
            if cfg!(windows) {
                PathStyle::Windows
            } else {
                PathStyle::Posix
            }
        );
    }

    #[test]
    fn notation_and_foreign_paths_are_classified() {
        assert_eq!(notation(r"\\?\C:\x", PathStyle::Posix), Notation::Namespace);
        assert_eq!(notation(r"\\srv\share", PathStyle::Posix), Notation::Unc);
        assert_eq!(
            notation("C:relative", PathStyle::Windows),
            Notation::Relative
        );
        assert_eq!(notation("dir/file", PathStyle::Posix), Notation::Relative);
        assert!(is_foreign(r"\\srv\share", PathStyle::Posix));
        assert!(is_foreign(r"\\?\C:\x", PathStyle::Posix));
        assert!(!is_foreign("C:/x", PathStyle::Windows));
        assert!(!is_foreign("relative", PathStyle::Windows));
    }

    #[test]
    fn rooted_paths_are_posix_and_foreign_only_in_windows_style() {
        assert_eq!(notation("/home/user", PathStyle::Posix), Notation::Posix);
        assert_eq!(notation("/home/user", PathStyle::Windows), Notation::Posix);
        assert_eq!(
            notation("//server/share", PathStyle::Posix),
            Notation::Posix
        );
        assert!(is_foreign("/home/user", PathStyle::Windows));
        assert!(!is_foreign("/home/user", PathStyle::Posix));
    }

    #[test]
    fn every_reserved_device_name_is_flagged() {
        for base in ["CON", "PRN", "AUX", "NUL"] {
            for variant in [base.to_owned(), base.to_lowercase(), format!("{base}.txt")] {
                assert!(windows_reserved_name(&variant), "{variant}");
            }
        }
        for prefix in ["COM", "LPT", "com", "Lpt"] {
            for suffix in DEVICE_SUFFIXES {
                assert!(windows_reserved_name(&format!("{prefix}{suffix}")));
                assert!(windows_reserved_name(&format!("{prefix}{suffix}.tar.gz")));
            }
        }
        assert!(windows_reserved_name("nul .txt"));
        assert!(windows_reserved_name("name."));
        assert!(windows_reserved_name("name "));
        for allowed in ["COM10", "LPT", "CONSOLE", "nul_file", "con-x.txt", "a.b"] {
            assert!(!windows_reserved_name(allowed), "{allowed}");
        }
    }

    #[test]
    fn decomposed_names_are_composed_and_raw_bytes_kept() {
        let stored = entry_name(OsStr::new("Re\u{301}sume\u{301}.txt"));
        assert_eq!(stored.name, "R\u{e9}sum\u{e9}.txt");
        assert_eq!(
            stored.raw_name,
            Some("Re\u{301}sume\u{301}.txt".as_bytes().to_vec())
        );
        assert!(matches!(nfc("plain"), Cow::Borrowed(_)));
        assert_eq!(nfc("e\u{301}"), "\u{e9}");
    }

    #[cfg(unix)]
    #[test]
    fn invalid_unicode_names_keep_raw_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let stored = entry_name(OsStr::from_bytes(b"bad\xff"));
        assert_eq!(stored.name, "bad\u{fffd}");
        assert_eq!(stored.raw_name.as_deref(), Some(&b"bad\xff"[..]));
    }

    #[test]
    fn invalid_components_are_rejected() {
        for style in [PathStyle::Posix, PathStyle::Windows] {
            assert_eq!(check_name("", style), Err(NameError::Empty));
            assert_eq!(check_name(".", style), Err(NameError::Dot));
            assert_eq!(check_name("..", style), Err(NameError::Dot));
            assert_eq!(check_name("a/b", style), Err(NameError::Separator));
            assert_eq!(check_name("a\0b", style), Err(NameError::Nul));
            assert_eq!(check_name("...", style), Ok(()));
        }
        assert_eq!(check_name("a\\b", PathStyle::Posix), Ok(()));
        assert_eq!(
            check_name("a\\b", PathStyle::Windows),
            Err(NameError::Separator)
        );
        for c in ['<', '>', ':', '"', '|', '?', '*'] {
            let name = format!("a{c}b");
            assert_eq!(
                check_name(&name, PathStyle::Windows),
                Err(NameError::Forbidden(c))
            );
            assert_eq!(check_name(&name, PathStyle::Posix), Ok(()));
        }
        assert_eq!(
            check_name("a\u{7}b", PathStyle::Windows),
            Err(NameError::Control('\u{7}'))
        );
    }

    #[test]
    fn name_error_messages_are_lower_case_without_period() {
        for error in [
            NameError::Empty,
            NameError::Dot,
            NameError::Separator,
            NameError::Nul,
            NameError::Forbidden('*'),
            NameError::Control('\u{1f}'),
        ] {
            let message = error.to_string();
            assert!(
                message.starts_with("name") || message.starts_with('`'),
                "{message}"
            );
            assert!(!message.ends_with('.'), "{message}");
        }
    }
}
