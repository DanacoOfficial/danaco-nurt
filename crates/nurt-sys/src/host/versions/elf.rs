// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Package metadata note of ELF files.
//!
//! The note follows the package metadata specification of the freedesktop.org
//! community: a section `.note.package` with a note of owner `FDO` and type
//! `0xcafe1a7e` whose descriptor is a JSON object with the member `version`.

use serde::Deserialize;

use super::Bytes;

/// Magic number of ELF files.
const MAGIC: &[u8; 4] = b"\x7fELF";
/// Value of `EI_CLASS` for 64-bit files.
const CLASS_64: u8 = 2;
/// Value of `EI_CLASS` for 32-bit files.
const CLASS_32: u8 = 1;
/// Value of `EI_DATA` for big-endian files.
const DATA_BIG_ENDIAN: u8 = 2;
/// Name of the section with the package note.
const SECTION_NAME: &[u8] = b".note.package";
/// Owner of the package note.
const NOTE_OWNER: &[u8] = b"FDO\0";
/// Type of the package note.
const NOTE_TYPE: u32 = 0xCAFE_1A7E;

#[derive(Deserialize)]
struct PackageMetadata {
    version: Option<String>,
}

/// Layout of the header fields that locate the section headers.
struct Layout {
    shoff: usize,
    shentsize: usize,
    shnum: usize,
    shstrndx: usize,
}

/// Version from the package note of an ELF file.
pub(crate) fn package_version(data: &[u8]) -> Option<String> {
    if data.get(..4)? != MAGIC {
        return None;
    }
    let class = *data.get(4)?;
    let big = *data.get(5)? == DATA_BIG_ENDIAN;
    let bytes = if big {
        Bytes::big(data)
    } else {
        Bytes::little(data)
    };
    let wide = match class {
        CLASS_64 => true,
        CLASS_32 => false,
        _ => return None,
    };
    let layout = layout(bytes, wide)?;
    let names = section(bytes, &layout, layout.shstrndx, wide)?;
    let note = (0..layout.shnum).find_map(|index| {
        let header = layout.shoff + index * layout.shentsize;
        let name = usize::try_from(bytes.u32(header)?).ok()?;
        let name = names.get(name..)?;
        name.starts_with(SECTION_NAME)
            .then(|| name.get(SECTION_NAME.len()) == Some(&0))
            .filter(|ends| *ends)
            .and_then(|_| section(bytes, &layout, index, wide))
    })?;
    note_version(note, big)
}

fn layout(bytes: Bytes<'_>, wide: bool) -> Option<Layout> {
    let (shoff, sizes) = if wide {
        (usize::try_from(bytes.u64(0x28)?).ok()?, 0x3A)
    } else {
        (bytes.index32(0x20)?, 0x2E)
    };
    Some(Layout {
        shoff,
        shentsize: usize::from(bytes.u16(sizes)?),
        shnum: usize::from(bytes.u16(sizes + 2)?),
        shstrndx: usize::from(bytes.u16(sizes + 4)?),
    })
}

fn section<'a>(bytes: Bytes<'a>, layout: &Layout, index: usize, wide: bool) -> Option<&'a [u8]> {
    let header = layout.shoff + index * layout.shentsize;
    if wide {
        bytes.slice64(bytes.u64(header + 0x18)?, bytes.u64(header + 0x20)?)
    } else {
        bytes.slice(bytes.index32(header + 0x10)?, bytes.index32(header + 0x14)?)
    }
}

fn note_version(section: &[u8], big: bool) -> Option<String> {
    let notes = if big {
        Bytes::big(section)
    } else {
        Bytes::little(section)
    };
    let mut at = 0;
    while at + 12 <= section.len() {
        let name_size = notes.index32(at)?;
        let desc_size = notes.index32(at + 4)?;
        let kind = notes.u32(at + 8)?;
        let name_at = at + 12;
        let desc_at = name_at + align4(name_size);
        if kind == NOTE_TYPE && notes.slice(name_at, name_size)? == NOTE_OWNER {
            let desc = notes.slice(desc_at, desc_size)?;
            let text = std::str::from_utf8(desc).ok()?.trim_end_matches('\0');
            let metadata: PackageMetadata = serde_json::from_str(text).ok()?;
            return metadata.version.filter(|version| !version.is_empty());
        }
        at = desc_at + align4(desc_size);
    }
    None
}

const fn align4(size: usize) -> usize {
    (size + 3) & !3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(owner: &[u8], kind: u32, desc: &[u8]) -> Vec<u8> {
        let mut note = Vec::new();
        note.extend_from_slice(&u32::try_from(owner.len()).unwrap().to_le_bytes());
        note.extend_from_slice(&u32::try_from(desc.len()).unwrap().to_le_bytes());
        note.extend_from_slice(&kind.to_le_bytes());
        note.extend_from_slice(owner);
        note.resize(12 + align4(owner.len()), 0);
        note.extend_from_slice(desc);
        note.resize(note.len() + align4(desc.len()) - desc.len(), 0);
        note
    }

    /// Minimal little-endian ELF64 file with the sections `.note.package`
    /// (holding `notes`) and `.shstrtab`.
    fn elf_with_notes(notes: &[u8]) -> Vec<u8> {
        const HEADER: usize = 64;
        const SECTION_HEADER: usize = 64;
        let names = b"\0.note.package\0.shstrtab\0";
        let mut file = vec![0u8; HEADER];
        file[..4].copy_from_slice(MAGIC);
        file[4] = CLASS_64;
        file[5] = 1;
        let notes_at = file.len();
        file.extend_from_slice(notes);
        let names_at = file.len();
        file.extend_from_slice(names);
        while file.len() % 8 != 0 {
            file.push(0);
        }
        let headers_at = file.len();
        let mut header = |name: u32, offset: usize, size: usize| {
            let mut entry = vec![0u8; SECTION_HEADER];
            entry[..4].copy_from_slice(&name.to_le_bytes());
            entry[0x18..0x20].copy_from_slice(&(offset as u64).to_le_bytes());
            entry[0x20..0x28].copy_from_slice(&(size as u64).to_le_bytes());
            file.extend(entry);
        };
        header(0, 0, 0);
        header(1, notes_at, notes.len());
        header(15, names_at, names.len());
        file[0x28..0x30].copy_from_slice(&(headers_at as u64).to_le_bytes());
        file[0x3A..0x3C].copy_from_slice(&(SECTION_HEADER as u16).to_le_bytes());
        file[0x3C..0x3E].copy_from_slice(&3u16.to_le_bytes());
        file[0x3E..0x40].copy_from_slice(&2u16.to_le_bytes());
        file
    }

    #[test]
    fn host_versions_elf_note_package() {
        let metadata = br#"{"type":"deb","name":"curl","version":"8.14.1-2"}"#;
        let mut notes = note(b"GNU\0", 3, &[0; 20]);
        notes.extend(note(NOTE_OWNER, NOTE_TYPE, metadata));
        let file = elf_with_notes(&notes);
        assert_eq!(package_version(&file).as_deref(), Some("8.14.1-2"));

        let without = elf_with_notes(&note(b"GNU\0", 3, &[0; 20]));
        assert_eq!(package_version(&without), None);
        assert_eq!(package_version(b"\x7fELF"), None);
    }
}
