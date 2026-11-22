// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reparse data of symbolic links and junctions.
//!
//! The layout follows `REPARSE_DATA_BUFFER` of the Windows Driver Kit: a
//! header with the tag and data length, then offsets and lengths (in bytes) of
//! the substitute name and the print name inside the path buffer. Symbolic
//! links carry an additional flags field before the path buffer.

use nurt_contracts::fs::FileKind;

use crate::windows::meta::reparse_tags;
use crate::windows::path::{VERBATIM_PREFIX, until_nul};

/// Largest reparse data buffer accepted by the file systems.
pub const MAXIMUM_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;

/// Symbolic link flag: the target is relative to the link directory.
pub const SYMLINK_FLAG_RELATIVE: u32 = 1;

const HEADER_LEN: usize = 8;
const NAMES_LEN: usize = 8;
const SYMLINK_FLAGS_LEN: usize = 4;
const BACKSLASH: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
/// Prefix of an object manager path in substitute names.
const NT_PREFIX: &str = r"\??\";
/// Prefix of a UNC path in substitute names.
const NT_UNC_PREFIX: &str = r"\??\UNC\";

/// Target of a symbolic link or junction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// [`FileKind::Symlink`] or [`FileKind::Junction`].
    pub kind: FileKind,
    /// Target in Win32 form, as stored in the link.
    pub target: Vec<u16>,
    /// Whether the target is relative to the directory of the link.
    pub relative: bool,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn units_to_bytes(units: &[u16]) -> impl Iterator<Item = u8> + '_ {
    units.iter().flat_map(|unit| unit.to_le_bytes())
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn name_at(bytes: &[u8], buffer: usize, offset_field: usize) -> Option<Vec<u16>> {
    let offset = usize::from(read_u16(bytes, offset_field)?);
    let len = usize::from(read_u16(bytes, offset_field + 2)?);
    if len % 2 != 0 {
        return None;
    }
    let start = buffer + offset;
    let slice = bytes.get(start..start + len)?;
    Some(
        slice
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect(),
    )
}

/// Win32 form of a substitute name (`\??\C:\dir` becomes `C:\dir`,
/// `\??\UNC\server\share` becomes `\\server\share`, `\??\Volume{…}\`
/// becomes `\\?\Volume{…}\`).
fn win32_from_substitute(name: &[u16]) -> Vec<u16> {
    let unc = wide(NT_UNC_PREFIX);
    if let Some(rest) = name.strip_prefix(unc.as_slice()) {
        let mut out = vec![BACKSLASH, BACKSLASH];
        out.extend_from_slice(rest);
        return out;
    }
    let nt = wide(NT_PREFIX);
    match name.strip_prefix(nt.as_slice()) {
        Some(rest @ [_, COLON, ..]) => rest.to_vec(),
        Some(rest) => {
            let mut out = wide(VERBATIM_PREFIX);
            out.extend_from_slice(rest);
            out
        }
        None => name.to_vec(),
    }
}

/// Reparse data of a junction pointing to `target`.
///
/// `target` is an absolute drive path (`C:\dir`) or volume path
/// (`\\?\Volume{…}\dir`), with or without the verbatim prefix; the zero
/// terminator is optional. Junctions cannot point to network shares.
///
/// Returns `None` for other targets and when the data exceeds
/// [`MAXIMUM_REPARSE_DATA_BUFFER_SIZE`].
#[must_use]
pub fn junction_data(target: &[u16]) -> Option<Vec<u8>> {
    let target = until_nul(target);
    let verbatim = wide(VERBATIM_PREFIX);
    let bare = target.strip_prefix(verbatim.as_slice()).unwrap_or(target);
    let is_drive = matches!(bare, [letter, COLON, BACKSLASH, ..]
        if u8::try_from(*letter).is_ok_and(|byte| byte.is_ascii_alphabetic()));
    let is_volume = bare.starts_with(&wide("Volume{"));
    if !is_drive && !is_volume {
        return None;
    }
    let mut substitute = wide(NT_PREFIX);
    substitute.extend_from_slice(bare);
    let print: &[u16] = if is_drive { bare } else { &[] };
    let substitute_bytes = substitute.len() * 2;
    let print_bytes = print.len() * 2;
    let path_bytes = substitute_bytes + 2 + print_bytes + 2;
    let data_len = NAMES_LEN + path_bytes;
    if HEADER_LEN + data_len > MAXIMUM_REPARSE_DATA_BUFFER_SIZE {
        return None;
    }
    let field = |value: usize| u16::try_from(value).ok().map(u16::to_le_bytes);
    let mut out = Vec::with_capacity(HEADER_LEN + data_len);
    out.extend_from_slice(&reparse_tags::MOUNT_POINT.to_le_bytes());
    out.extend_from_slice(&field(data_len)?);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&field(0)?);
    out.extend_from_slice(&field(substitute_bytes)?);
    out.extend_from_slice(&field(substitute_bytes + 2)?);
    out.extend_from_slice(&field(print_bytes)?);
    out.extend(units_to_bytes(&substitute));
    out.extend_from_slice(&[0, 0]);
    out.extend(units_to_bytes(print));
    out.extend_from_slice(&[0, 0]);
    Some(out)
}

/// Target stored in the reparse data of a symbolic link or junction.
///
/// The print name is returned when present; otherwise the substitute name is
/// converted to Win32 form. Returns `None` for other reparse tags and for
/// malformed data.
#[must_use]
pub fn parse_link(data: &[u8]) -> Option<LinkTarget> {
    let tag = read_u32(data, 0)?;
    let (kind, buffer, relative) = match tag {
        reparse_tags::SYMLINK => {
            let flags = read_u32(data, HEADER_LEN + NAMES_LEN)?;
            (
                FileKind::Symlink,
                HEADER_LEN + NAMES_LEN + SYMLINK_FLAGS_LEN,
                flags & SYMLINK_FLAG_RELATIVE != 0,
            )
        }
        reparse_tags::MOUNT_POINT => (FileKind::Junction, HEADER_LEN + NAMES_LEN, false),
        _ => return None,
    };
    let substitute = name_at(data, buffer, HEADER_LEN)?;
    let print = name_at(data, buffer, HEADER_LEN + 4)?;
    let target = if print.is_empty() {
        if relative {
            substitute
        } else {
            win32_from_substitute(&substitute)
        }
    } else {
        print
    };
    Some(LinkTarget {
        kind,
        target,
        relative,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(units: &[u16]) -> String {
        String::from_utf16_lossy(units)
    }

    /// Symbolic link data with the given names and flags.
    fn symlink_data(substitute: &str, print: &str, flags: u32) -> Vec<u8> {
        let substitute = wide(substitute);
        let print = wide(print);
        let sub_bytes = u16::try_from(substitute.len() * 2).unwrap();
        let print_bytes = u16::try_from(print.len() * 2).unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(&reparse_tags::SYMLINK.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&print_bytes.to_le_bytes());
        out.extend_from_slice(&sub_bytes.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&print_bytes.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend(units_to_bytes(&print));
        out.extend(units_to_bytes(&substitute));
        out
    }

    #[test]
    fn builds_and_reads_junction_data() {
        let data = junction_data(&wide(r"C:\target\dir")).unwrap();
        assert_eq!(read_u32(&data, 0), Some(reparse_tags::MOUNT_POINT));
        assert_eq!(
            usize::from(read_u16(&data, 4).unwrap()),
            data.len() - HEADER_LEN
        );
        let substitute = name_at(&data, HEADER_LEN + NAMES_LEN, HEADER_LEN).unwrap();
        assert_eq!(text(&substitute), r"\??\C:\target\dir");
        let link = parse_link(&data).unwrap();
        assert_eq!(link.kind, FileKind::Junction);
        assert_eq!(text(&link.target), r"C:\target\dir");
        assert!(!link.relative);
    }

    #[test]
    fn accepts_verbatim_and_volume_targets_for_junctions() {
        let data = junction_data(&wide("\\\\?\\C:\\t\0")).unwrap();
        assert_eq!(text(&parse_link(&data).unwrap().target), r"C:\t");
        let volume = r"\\?\Volume{3f2504e0-4f89-11d3-9a0c-0305e82c3301}\";
        let data = junction_data(&wide(volume)).unwrap();
        assert_eq!(text(&parse_link(&data).unwrap().target), volume);
    }

    #[test]
    fn rejects_junction_targets_that_are_not_local_absolute_paths() {
        for target in [r"relative\dir", r"\\server\share\dir", r"\dir", ""] {
            assert_eq!(junction_data(&wide(target)), None, "{target}");
        }
        let long = format!(r"C:\{}", "a".repeat(MAXIMUM_REPARSE_DATA_BUFFER_SIZE / 2));
        assert_eq!(junction_data(&wide(&long)), None);
    }

    #[test]
    fn reads_symbolic_link_print_name() {
        let data = symlink_data(r"\??\C:\target", r"C:\target", 0);
        let link = parse_link(&data).unwrap();
        assert_eq!(link.kind, FileKind::Symlink);
        assert_eq!(text(&link.target), r"C:\target");
        assert!(!link.relative);
    }

    #[test]
    fn reads_relative_symbolic_links() {
        let data = symlink_data(r"..\sibling", "", SYMLINK_FLAG_RELATIVE);
        let link = parse_link(&data).unwrap();
        assert!(link.relative);
        assert_eq!(text(&link.target), r"..\sibling");
    }

    #[test]
    fn converts_substitute_names_without_print_name() {
        let cases = [
            (r"\??\C:\target", r"C:\target"),
            (r"\??\UNC\server\share\dir", r"\\server\share\dir"),
            (r"\??\Volume{1}\dir", r"\\?\Volume{1}\dir"),
            (r"C:\plain", r"C:\plain"),
        ];
        for (substitute, expected) in cases {
            let data = symlink_data(substitute, "", 0);
            assert_eq!(text(&parse_link(&data).unwrap().target), expected);
        }
    }

    #[test]
    fn rejects_other_tags_and_malformed_data() {
        let mut other = symlink_data(r"\??\C:\t", "", 0);
        other[..4].copy_from_slice(&0x8000_0023_u32.to_le_bytes());
        assert_eq!(parse_link(&other), None);
        let full = symlink_data(r"\??\C:\t", "", 0);
        assert_eq!(parse_link(&full[..full.len() - 2]), None);
        let mut odd = symlink_data(r"\??\C:\t", "", 0);
        odd[10] = 3;
        assert_eq!(parse_link(&odd), None);
        assert_eq!(parse_link(&[]), None);
    }
}
