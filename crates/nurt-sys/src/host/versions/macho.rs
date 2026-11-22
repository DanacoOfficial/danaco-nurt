// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Versions of macOS programs: application bundles and the embedded
//! `Info.plist` of Mach-O files.

use std::io::Cursor;
use std::path::Path;

use super::{Bytes, plist_version};

/// Magic number of 32-bit Mach-O files, little-endian.
const MAGIC_32: u32 = 0xFEED_FACE;
/// Magic number of 64-bit Mach-O files, little-endian.
const MAGIC_64: u32 = 0xFEED_FACF;
/// Magic number of universal files, big-endian.
const FAT_MAGIC: u32 = 0xCAFE_BABE;
/// Largest number of architectures accepted in a universal file; the same
/// magic number starts Java class files, whose next field is much larger.
const MAX_FAT_ARCHES: u32 = 32;
/// Offset of the first architecture entry of a universal file, whose
/// `offset` and `size` fields follow the processor type and subtype.
const FIRST_FAT_ARCH: usize = 8;
/// Load command of a 32-bit segment.
const LC_SEGMENT: u32 = 0x1;
/// Load command of a 64-bit segment.
const LC_SEGMENT_64: u32 = 0x19;
/// Segment holding the embedded property list.
const TEXT_SEGMENT: &[u8] = b"__TEXT";
/// Section holding the embedded property list.
const INFO_PLIST_SECTION: &[u8] = b"__info_plist";
/// Suffix of application bundle directories.
const APP_SUFFIX: &str = ".app";

/// Version from the `Info.plist` of the application bundle that holds
/// `path`, as in `Name.app/Contents/MacOS/name`.
pub(crate) fn bundle_version(path: &Path) -> Option<String> {
    let bundle = path.ancestors().find(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(APP_SUFFIX))
    })?;
    let document = plist::Value::from_file(bundle.join("Contents/Info.plist")).ok()?;
    plist_version(&document)
}

/// Version from the `__TEXT,__info_plist` section of a Mach-O file; for a
/// universal file the first architecture is read.
pub(crate) fn embedded_version(data: &[u8]) -> Option<String> {
    let image = thin_image(data)?;
    let section = info_plist(image)?;
    let document = plist::Value::from_reader(Cursor::new(section)).ok()?;
    plist_version(&document)
}

fn thin_image(data: &[u8]) -> Option<&[u8]> {
    let big = Bytes::big(data);
    if big.u32(0)? != FAT_MAGIC {
        return Some(data);
    }
    let count = big.u32(4)?;
    if count == 0 || count > MAX_FAT_ARCHES {
        return None;
    }
    let offset = big.index32(FIRST_FAT_ARCH + 8)?;
    let size = big.index32(FIRST_FAT_ARCH + 12)?;
    big.slice(offset, size)
}

fn info_plist(image: &[u8]) -> Option<&[u8]> {
    let bytes = Bytes::little(image);
    let (header, wide) = match bytes.u32(0)? {
        MAGIC_64 => (32, true),
        MAGIC_32 => (28, false),
        _ => return None,
    };
    let count = bytes.index32(16)?;
    let mut at = header;
    for _ in 0..count {
        let command = bytes.u32(at)?;
        let size = bytes.index32(at + 4)?;
        let found = match command {
            LC_SEGMENT_64 if wide => segment_section(bytes, at, true),
            LC_SEGMENT if !wide => segment_section(bytes, at, false),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
        if size == 0 {
            return None;
        }
        at = at.checked_add(size)?;
    }
    None
}

fn segment_section(bytes: Bytes<'_>, command: usize, wide: bool) -> Option<&[u8]> {
    if !name_is(bytes.slice(command + 8, 16)?, TEXT_SEGMENT) {
        return None;
    }
    let (count_at, first, size) = if wide { (64, 72, 80) } else { (48, 56, 68) };
    let count = bytes.index32(command + count_at)?;
    (0..count).find_map(|index| {
        let section = command + first + index * size;
        if !name_is(bytes.slice(section, 16)?, INFO_PLIST_SECTION) {
            return None;
        }
        if wide {
            let length = bytes.u64(section + 40)?;
            let offset = u64::from(bytes.u32(section + 48)?);
            bytes.slice64(offset, length)
        } else {
            bytes.slice(bytes.index32(section + 40)?, bytes.index32(section + 36)?)
        }
    })
}

/// Compares a fixed-size, null-padded name field with `name`.
fn name_is(field: &[u8], name: &[u8]) -> bool {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    &field[..end] == name
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.example.tool</string>
<key>CFBundleShortVersionString</key><string>3.2.1</string>
</dict></plist>"#;

    fn name16(name: &[u8]) -> [u8; 16] {
        let mut field = [0u8; 16];
        field[..name.len()].copy_from_slice(name);
        field
    }

    /// 64-bit Mach-O file with one `__TEXT` segment holding `__info_plist`.
    fn macho_with_plist(plist: &[u8]) -> Vec<u8> {
        let mut file = vec![0u8; 32];
        file[..4].copy_from_slice(&MAGIC_64.to_le_bytes());
        file[16..20].copy_from_slice(&1u32.to_le_bytes());
        let command_size: u32 = 72 + 80;
        file[20..24].copy_from_slice(&command_size.to_le_bytes());
        let mut command = vec![0u8; 72];
        command[..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
        command[4..8].copy_from_slice(&command_size.to_le_bytes());
        command[8..24].copy_from_slice(&name16(TEXT_SEGMENT));
        command[64..68].copy_from_slice(&1u32.to_le_bytes());
        let mut section = vec![0u8; 80];
        section[..16].copy_from_slice(&name16(INFO_PLIST_SECTION));
        section[16..32].copy_from_slice(&name16(TEXT_SEGMENT));
        let offset = u32::try_from(32 + 72 + 80).unwrap();
        section[40..48].copy_from_slice(&(plist.len() as u64).to_le_bytes());
        section[48..52].copy_from_slice(&offset.to_le_bytes());
        file.extend(command);
        file.extend(section);
        file.extend_from_slice(plist);
        file
    }

    #[test]
    fn host_versions_macho_embedded_info_plist() {
        let file = macho_with_plist(PLIST.as_bytes());
        assert_eq!(embedded_version(&file).as_deref(), Some("3.2.1"));
        assert_eq!(embedded_version(&file[..40]), None);
    }

    #[test]
    fn host_versions_app_bundle_info_plist() {
        let dir = tempfile::tempdir().unwrap();
        let contents = dir.path().join("Tool.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(contents.join("Info.plist"), PLIST).unwrap();
        let program = contents.join("MacOS/tool");
        std::fs::write(&program, b"").unwrap();
        assert_eq!(bundle_version(&program).as_deref(), Some("3.2.1"));
        assert_eq!(bundle_version(dir.path()), None);
    }
}
