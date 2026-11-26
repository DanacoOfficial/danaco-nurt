// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Path forms passed to Windows system calls.
//!
//! Every call of the back end receives a verbatim path (`\\?\C:\dir`,
//! `\\?\UNC\server\share\dir`). A verbatim path bypasses the Win32 name
//! normalisation, so reserved device names (`NUL.txt`), names ending with a dot
//! or a space and paths longer than `MAX_PATH` reach the file system
//! unchanged. Functions operate on UTF-16 code units, the native form of
//! Windows paths, which keeps unpaired surrogates intact.

use std::string::FromUtf16Error;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const ZERO: u16 = 0;

/// Prefix of a verbatim path to a drive or volume.
pub const VERBATIM_PREFIX: &str = r"\\?\";
/// Prefix of a verbatim UNC path.
pub const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
/// Prefix of a path in the Win32 device namespace.
pub const DEVICE_PREFIX: &str = r"\\.\";
/// Type suffix of a named data stream.
pub const DATA_STREAM_SUFFIX: &str = ":$DATA";
/// Longest part of a name hint kept in a temporary file name, in characters.
pub const TEMP_HINT_MAX_CHARS: usize = 64;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Encodes text as UTF-16 followed by a terminating zero.
#[must_use]
pub fn nul_terminated(text: &str) -> Vec<u16> {
    let mut out = wide(text);
    out.push(ZERO);
    out
}

/// Returns the code units before the first zero, or the whole slice.
#[must_use]
pub fn until_nul(units: &[u16]) -> &[u16] {
    let end = units
        .iter()
        .position(|&unit| unit == ZERO)
        .unwrap_or(units.len());
    &units[..end]
}

fn is_drive_letter(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| byte.is_ascii_alphabetic())
}

fn is_separator(unit: u16) -> bool {
    unit == BACKSLASH || unit == SLASH
}

/// Appends the components of `rest` separated by single backslashes.
///
/// Fails on `.` and `..` components, which a verbatim path would pass to the
/// file system literally.
fn push_components(out: &mut Vec<u16>, rest: &[u16]) -> Option<()> {
    let mut first = true;
    for component in rest.split(|&unit| is_separator(unit)) {
        if component.is_empty() {
            continue;
        }
        if component == [DOT] || component == [DOT, DOT] {
            return None;
        }
        if !first {
            out.push(BACKSLASH);
        }
        out.extend_from_slice(component);
        first = false;
    }
    Some(())
}

/// Converts an absolute path to the verbatim form used for system calls.
///
/// Accepted inputs are drive paths in native or language form (`C:\dir`,
/// `C:/dir`), UNC paths (`\\server\share\dir`) and paths that already carry
/// the verbatim or device prefix, which are returned unchanged. Separators
/// become backslashes and repeated separators collapse. A trailing separator
/// is kept only for a drive or share root.
///
/// Returns `None` for relative and drive-relative paths (`dir`, `C:dir`,
/// `\dir`), for a UNC path without a share, for `.` or `..` components and for
/// a path with an embedded zero.
///
/// # Examples
///
/// ```
/// use nurt_fs::windows::path::verbatim_wide;
///
/// let input: Vec<u16> = r"\\server\share\dir".encode_utf16().collect();
/// let verbatim = verbatim_wide(&input).unwrap();
/// assert_eq!(String::from_utf16_lossy(&verbatim), r"\\?\UNC\server\share\dir");
/// ```
#[must_use]
pub fn verbatim_wide(path: &[u16]) -> Option<Vec<u16>> {
    if path.contains(&ZERO) {
        return None;
    }
    let normalised: Vec<u16> = path
        .iter()
        .map(|&unit| if unit == SLASH { BACKSLASH } else { unit })
        .collect();
    let verbatim = wide(VERBATIM_PREFIX);
    let device = wide(DEVICE_PREFIX);
    if normalised.starts_with(&verbatim) || normalised.starts_with(&device) {
        return Some(normalised);
    }
    match normalised.as_slice() {
        [BACKSLASH, BACKSLASH, rest @ ..] => {
            let mut parts = rest.split(|&unit| unit == BACKSLASH);
            let server = parts.next().unwrap_or_default();
            let share = parts.next().unwrap_or_default();
            if server.is_empty() || share.is_empty() {
                return None;
            }
            let mut out = wide(VERBATIM_UNC_PREFIX);
            push_components(&mut out, rest)?;
            if rest.ends_with(&[BACKSLASH]) && rest.len() == server.len() + share.len() + 2 {
                out.push(BACKSLASH);
            }
            Some(out)
        }
        [letter, COLON, BACKSLASH, rest @ ..] if is_drive_letter(*letter) => {
            let mut out = verbatim;
            out.extend_from_slice(&[*letter, COLON, BACKSLASH]);
            push_components(&mut out, rest)?;
            Some(out)
        }
        _ => None,
    }
}

/// Removes the verbatim prefix from a path returned by the system.
///
/// A drive path `\\?\C:\dir` becomes `C:\dir` and a UNC path
/// `\\?\UNC\server\share` becomes `\\server\share`. Paths without the prefix
/// and volume paths (`\\?\Volume{…}\`), which have no other form, are
/// returned unchanged.
#[must_use]
pub fn strip_verbatim(path: &[u16]) -> Vec<u16> {
    let unc = wide(VERBATIM_UNC_PREFIX);
    if let Some(rest) = path.strip_prefix(unc.as_slice()) {
        let mut out = vec![BACKSLASH, BACKSLASH];
        out.extend_from_slice(rest);
        return out;
    }
    let verbatim = wide(VERBATIM_PREFIX);
    if let Some(rest) = path.strip_prefix(verbatim.as_slice())
        && let [letter, COLON, ..] = rest
        && is_drive_letter(*letter)
    {
        return rest.to_vec();
    }
    path.to_vec()
}

/// Tells whether `name` is a valid name of an alternate data stream.
///
/// A valid name is not empty and contains no `:`, `\`, `/` or zero.
#[must_use]
pub fn is_valid_stream_name(name: &str) -> bool {
    !name.is_empty() && !name.contains([':', '\\', '/', '\0'])
}

/// Builds the path of a named data stream of the file at a verbatim path.
///
/// The result has the form `<file>:<name>:$DATA`. Returns `None` when the
/// stream name is not valid according to [`is_valid_stream_name`].
///
/// # Examples
///
/// ```
/// use nurt_fs::windows::path::stream_path;
///
/// let file: Vec<u16> = r"\\?\C:\data\report.txt".encode_utf16().collect();
/// let stream = stream_path(&file, "Summary").unwrap();
/// assert_eq!(
///     String::from_utf16_lossy(&stream),
///     r"\\?\C:\data\report.txt:Summary:$DATA"
/// );
/// ```
#[must_use]
pub fn stream_path(file: &[u16], name: &str) -> Option<Vec<u16>> {
    if !is_valid_stream_name(name) {
        return None;
    }
    let mut out = file.to_vec();
    out.push(COLON);
    out.extend(name.encode_utf16());
    out.extend(DATA_STREAM_SUFFIX.encode_utf16());
    Some(out)
}

/// Decodes a stream name reported by the stream enumeration of Windows.
///
/// `FindFirstStreamW` and `FindNextStreamW` report names of the form
/// `:<name>:<type>`. The result is `Ok(None)` for the unnamed default stream
/// (`::$DATA`) and for streams of a type other than `$DATA`, and the bare name
/// otherwise.
///
/// # Errors
///
/// Fails when the name is not valid UTF-16; such a stream cannot be
/// represented in [`nurt_contracts::fs::AlternateStream`].
pub fn decode_stream_name(raw: &[u16]) -> Result<Option<String>, FromUtf16Error> {
    let text = String::from_utf16(until_nul(raw))?;
    let Some(rest) = text.strip_prefix(':') else {
        return Ok(None);
    };
    let Some(name) = rest.strip_suffix(DATA_STREAM_SUFFIX) else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(name.to_owned()))
}

/// Builds the name of a temporary file from a hint and unique numbers.
///
/// The name is unique for the process id, the per-process sequence number and
/// the nonce. Characters that Windows forbids in names (`<>:"/\|?*` and
/// control characters) become `_`, and the hint is shortened to
/// [`TEMP_HINT_MAX_CHARS`] characters. The name starts with a dot and ends
/// with `.tmp`, so it never ends with a dot or a space.
///
/// # Examples
///
/// ```
/// use nurt_fs::windows::path::temp_file_name;
///
/// assert_eq!(temp_file_name("a:b", 7, 2, 0xff), ".a_b.7.2.ff.tmp");
/// ```
#[must_use]
pub fn temp_file_name(hint: &str, pid: u32, sequence: u64, nonce: u64) -> String {
    let hint: String = hint
        .chars()
        .take(TEMP_HINT_MAX_CHARS)
        .map(|character| {
            if character.is_control() || r#"<>:"/\|?*"#.contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    format!(".{hint}.{pid}.{sequence}.{nonce:x}.tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verbatim(text: &str) -> Option<String> {
        verbatim_wide(&wide(text)).map(|units| String::from_utf16_lossy(&units))
    }

    #[test]
    fn derives_safe_temporary_names() {
        assert_eq!(temp_file_name("data.json", 1, 0, 0), ".data.json.1.0.0.tmp");
        assert_eq!(temp_file_name("a<b>c\u{1}", 1, 0, 0), ".a_b_c_.1.0.0.tmp");
        let long = temp_file_name(&"x".repeat(200), 1, 0, 0);
        assert_eq!(long.matches('x').count(), TEMP_HINT_MAX_CHARS);
        assert_ne!(temp_file_name("a", 1, 0, 0), temp_file_name("a", 1, 1, 0));
    }

    #[test]
    fn prefixes_drive_paths() {
        assert_eq!(
            verbatim(r"C:\dir\file.txt").as_deref(),
            Some(r"\\?\C:\dir\file.txt")
        );
        assert_eq!(
            verbatim("d:/dir/file.txt").as_deref(),
            Some(r"\\?\d:\dir\file.txt")
        );
        assert_eq!(verbatim(r"C:\").as_deref(), Some(r"\\?\C:\"));
        assert_eq!(
            verbatim(r"C:\dir\\sub\").as_deref(),
            Some(r"\\?\C:\dir\sub")
        );
    }

    #[test]
    fn prefixes_unc_paths() {
        assert_eq!(
            verbatim(r"\\server\share\dir").as_deref(),
            Some(r"\\?\UNC\server\share\dir")
        );
        assert_eq!(
            verbatim("//server/share/dir").as_deref(),
            Some(r"\\?\UNC\server\share\dir")
        );
        assert_eq!(
            verbatim(r"\\server\share\").as_deref(),
            Some(r"\\?\UNC\server\share\")
        );
        assert_eq!(
            verbatim(r"\\server\share").as_deref(),
            Some(r"\\?\UNC\server\share")
        );
    }

    #[test]
    fn keeps_reserved_names_and_trailing_dots() {
        assert_eq!(
            verbatim(r"C:\dir\NUL.txt").as_deref(),
            Some(r"\\?\C:\dir\NUL.txt")
        );
        assert_eq!(verbatim(r"C:\dir\con").as_deref(), Some(r"\\?\C:\dir\con"));
        assert_eq!(
            verbatim(r"C:\dir\name. ").as_deref(),
            Some(r"\\?\C:\dir\name. ")
        );
    }

    #[test]
    fn keeps_paths_longer_than_max_path() {
        let long = format!(r"C:\{}", ["segment"; 60].join(r"\"));
        assert!(long.len() > 260);
        let converted = verbatim(&long).unwrap();
        assert_eq!(converted, format!(r"\\?\{long}"));
    }

    #[test]
    fn returns_prefixed_paths_unchanged() {
        assert_eq!(verbatim(r"\\?\C:\dir").as_deref(), Some(r"\\?\C:\dir"));
        assert_eq!(
            verbatim(r"\\?\Volume{0}\dir").as_deref(),
            Some(r"\\?\Volume{0}\dir")
        );
        assert_eq!(
            verbatim(r"\\.\PhysicalDrive0").as_deref(),
            Some(r"\\.\PhysicalDrive0")
        );
    }

    #[test]
    fn rejects_relative_and_malformed_paths() {
        for input in [
            "dir",
            r"C:dir",
            r"\dir",
            r"\\server",
            r"\\server\",
            r"\\\share",
            r"C:\dir\..\x",
            r"C:\.\x",
            "C:\\dir\0x",
            "1:\\dir",
        ] {
            assert_eq!(verbatim(input), None, "{input:?}");
        }
    }

    #[test]
    fn rejects_non_ascii_drive_letter() {
        let mut input = wide(r"C:\dir");
        input[0] = 0x0100;
        assert_eq!(verbatim_wide(&input), None);
    }

    #[test]
    fn keeps_unpaired_surrogates() {
        let mut input = wide(r"C:\x");
        input.push(0xD800);
        let converted = verbatim_wide(&input).unwrap();
        assert_eq!(converted.last(), Some(&0xD800));
    }

    #[test]
    fn strips_verbatim_prefix() {
        let strip = |text: &str| String::from_utf16_lossy(&strip_verbatim(&wide(text)));
        assert_eq!(strip(r"\\?\C:\dir"), r"C:\dir");
        assert_eq!(strip(r"\\?\UNC\server\share\"), r"\\server\share\");
        assert_eq!(strip(r"\\?\Volume{0}\"), r"\\?\Volume{0}\");
        assert_eq!(strip(r"C:\dir"), r"C:\dir");
    }

    #[test]
    fn builds_stream_paths() {
        let file = wide(r"\\?\C:\f");
        let path = stream_path(&file, "s1").map(|units| String::from_utf16_lossy(&units));
        assert_eq!(path.as_deref(), Some(r"\\?\C:\f:s1:$DATA"));
        for name in ["", "a:b", r"a\b", "a/b", "a\0b"] {
            assert_eq!(stream_path(&file, name), None, "{name:?}");
        }
    }

    #[test]
    fn decodes_stream_names() {
        let decode = |text: &str| decode_stream_name(&nul_terminated(text)).unwrap();
        assert_eq!(decode("::$DATA"), None);
        assert_eq!(decode(":Summary:$DATA").as_deref(), Some("Summary"));
        assert_eq!(decode(":name:$INDEX_ALLOCATION"), None);
        assert_eq!(decode("plain"), None);
    }

    #[test]
    fn rejects_stream_names_that_are_not_utf16() {
        let mut raw = wide(":a");
        raw.push(0xDC00);
        raw.extend(wide(":$DATA"));
        assert!(decode_stream_name(&raw).is_err());
    }

    #[test]
    fn keeps_multi_byte_names_and_surrogate_pairs() {
        assert_eq!(
            verbatim("C:/za\u{17c}\u{f3}\u{142}\u{107}/\u{1f600}.txt").as_deref(),
            Some("\\\\?\\C:\\za\u{17c}\u{f3}\u{142}\u{107}\\\u{1f600}.txt")
        );
        let hint = "\u{17c}".repeat(2 * TEMP_HINT_MAX_CHARS);
        let name = temp_file_name(&hint, 1, 0, 0);
        assert_eq!(name.matches('\u{17c}').count(), TEMP_HINT_MAX_CHARS);
        let decoded = decode_stream_name(&nul_terminated(":\u{17c}\u{f3}\u{142}\u{107}:$DATA"));
        assert_eq!(
            decoded.unwrap().as_deref(),
            Some("\u{17c}\u{f3}\u{142}\u{107}")
        );
    }

    #[test]
    fn terminates_and_trims_wide_strings() {
        let units = nul_terminated("ab");
        assert_eq!(units, [97, 98, 0]);
        assert_eq!(until_nul(&units), [97, 98]);
        assert_eq!(until_nul(&[97, 98]), [97, 98]);
    }
}
