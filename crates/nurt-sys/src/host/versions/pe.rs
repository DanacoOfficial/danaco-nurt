// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Version resource of PE files (Windows executables and libraries).

use super::Bytes;

/// Offset of the file offset of the PE header in the DOS header.
const DOS_LFANEW: usize = 0x3C;
/// Signature of the PE header.
const PE_SIGNATURE: &[u8; 4] = b"PE\0\0";
/// Size of the signature and the COFF file header.
const COFF_END: usize = 24;
/// Magic number of a 32-bit optional header.
const PE32_MAGIC: u16 = 0x10B;
/// Magic number of a 64-bit optional header.
const PE32_PLUS_MAGIC: u16 = 0x20B;
/// Offset of the data directories in a 32-bit optional header.
const PE32_DIRECTORIES: usize = 96;
/// Offset of the data directories in a 64-bit optional header.
const PE32_PLUS_DIRECTORIES: usize = 112;
/// Index of the resource table among the data directories.
const RESOURCE_DIRECTORY: usize = 2;
/// Size of one section header.
const SECTION_HEADER: usize = 40;
/// Resource type of version information.
const RT_VERSION: u32 = 16;
/// Flag of a resource directory entry that points to a subdirectory.
const SUBDIRECTORY: u32 = 0x8000_0000;
/// Flag of a resource directory entry with a name instead of an identifier.
const NAMED_ENTRY: u32 = 0x8000_0000;
/// Signature of `VS_FIXEDFILEINFO`.
const FIXED_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
/// Key of the root block of the version resource.
const VERSION_INFO_KEY: &str = "VS_VERSION_INFO";
/// Key of the block of string tables.
const STRING_FILE_INFO_KEY: &str = "StringFileInfo";
/// Key of the product version string.
const PRODUCT_VERSION_KEY: &str = "ProductVersion";
/// `wType` of a block whose value is text.
const TEXT_VALUE: u16 = 1;

/// Product version of a PE file: the `ProductVersion` string of the version
/// resource, otherwise the product version of `VS_FIXEDFILEINFO`.
pub(crate) fn product_version(bytes: &[u8]) -> Option<String> {
    let image = Image::parse(bytes)?;
    let resource = image.version_resource()?;
    let root = Block::parse(resource, 0)?;
    if root.key != VERSION_INFO_KEY {
        return None;
    }
    string_version(resource, &root).or_else(|| fixed_version(root.value))
}

struct Image<'a> {
    bytes: Bytes<'a>,
    sections: Vec<Section>,
    resource_rva: usize,
}

struct Section {
    address: usize,
    size: usize,
    raw_offset: usize,
}

impl<'a> Image<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let bytes = Bytes::little(data);
        if bytes.slice(0, 2)? != b"MZ" {
            return None;
        }
        let header = bytes.index32(DOS_LFANEW)?;
        if bytes.slice(header, PE_SIGNATURE.len())? != PE_SIGNATURE {
            return None;
        }
        let section_count = usize::from(bytes.u16(header + 6)?);
        let optional_size = usize::from(bytes.u16(header + 20)?);
        let optional = header + COFF_END;
        let directories = match bytes.u16(optional)? {
            PE32_MAGIC => optional + PE32_DIRECTORIES,
            PE32_PLUS_MAGIC => optional + PE32_PLUS_DIRECTORIES,
            _ => return None,
        };
        let resource_rva = bytes.index32(directories + RESOURCE_DIRECTORY * 8)?;
        let table = optional + optional_size;
        let sections = (0..section_count)
            .map(|index| {
                let at = table + index * SECTION_HEADER;
                let virtual_size = bytes.index32(at + 8)?;
                let raw_size = bytes.index32(at + 16)?;
                Some(Section {
                    address: bytes.index32(at + 12)?,
                    size: virtual_size.max(raw_size),
                    raw_offset: bytes.index32(at + 20)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            bytes,
            sections,
            resource_rva,
        })
    }

    fn offset(&self, rva: usize) -> Option<usize> {
        self.sections
            .iter()
            .find(|section| rva >= section.address && rva < section.address + section.size)
            .map(|section| section.raw_offset + (rva - section.address))
    }

    /// Bytes of the first version resource.
    fn version_resource(&self) -> Option<&'a [u8]> {
        if self.resource_rva == 0 {
            return None;
        }
        let root = self.offset(self.resource_rva)?;
        let types = self.find_entry(root, root, Some(RT_VERSION))?;
        let names = self.find_entry(root, types, None)?;
        let data_entry = self.find_entry(root, names, None)?;
        let data_rva = self.bytes.index32(data_entry)?;
        let size = self.bytes.index32(data_entry + 4)?;
        self.bytes.slice(self.offset(data_rva)?, size)
    }

    /// Offset of the target of the entry with identifier `id` (or of the
    /// first entry) of the resource directory at `directory`.
    fn find_entry(&self, root: usize, directory: usize, id: Option<u32>) -> Option<usize> {
        let named = usize::from(self.bytes.u16(directory + 12)?);
        let ids = usize::from(self.bytes.u16(directory + 14)?);
        (0..named + ids).find_map(|index| {
            let entry = directory + 16 + index * 8;
            let name = self.bytes.u32(entry)?;
            let target = self.bytes.u32(entry + 4)?;
            let wanted = id.is_none_or(|id| name & NAMED_ENTRY == 0 && name == id);
            wanted.then(|| root + usize::try_from(target & !SUBDIRECTORY).ok().unwrap_or(0))
        })
    }
}

/// One block of the version resource.
struct Block<'a> {
    key: String,
    value: &'a [u8],
    kind: u16,
    value_words: usize,
    children: usize,
    end: usize,
}

impl<'a> Block<'a> {
    fn parse(data: &'a [u8], start: usize) -> Option<Self> {
        let bytes = Bytes::little(data);
        let length = usize::from(bytes.u16(start)?);
        let value_length = usize::from(bytes.u16(start + 2)?);
        let kind = bytes.u16(start + 4)?;
        let end = (start + length).min(data.len());
        let (key, after_key) = utf16_string(data, start + 6, end)?;
        let value_start = align4(after_key);
        let value_bytes = if kind == TEXT_VALUE {
            value_length * 2
        } else {
            value_length
        };
        let value = data.get(value_start..(value_start + value_bytes).min(end))?;
        Some(Self {
            key,
            value,
            kind,
            value_words: value_length,
            children: align4(value_start + value_bytes),
            end,
        })
    }

    fn children(&self, data: &'a [u8]) -> Vec<Block<'a>> {
        let mut blocks = Vec::new();
        let mut at = self.children;
        while at < self.end {
            let Some(block) = Block::parse(data, at) else {
                break;
            };
            let next = align4(block.end);
            if next <= at {
                break;
            }
            at = next;
            blocks.push(block);
        }
        blocks
    }

    fn text(&self) -> Option<String> {
        (self.kind == TEXT_VALUE && self.value_words > 0)
            .then(|| utf16_string(self.value, 0, self.value.len()))
            .flatten()
            .map(|(text, _)| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    }
}

fn string_version(data: &[u8], root: &Block<'_>) -> Option<String> {
    root.children(data)
        .into_iter()
        .filter(|block| block.key == STRING_FILE_INFO_KEY)
        .flat_map(|info| info.children(data))
        .flat_map(|table| table.children(data))
        .find(|entry| entry.key == PRODUCT_VERSION_KEY)
        .and_then(|entry| entry.text())
}

fn fixed_version(value: &[u8]) -> Option<String> {
    let bytes = Bytes::little(value);
    if bytes.u32(0)? != FIXED_INFO_SIGNATURE {
        return None;
    }
    let high = bytes.u32(16)?;
    let low = bytes.u32(20)?;
    Some(format!(
        "{}.{}.{}.{}",
        high >> 16,
        high & 0xFFFF,
        low >> 16,
        low & 0xFFFF
    ))
}

/// Null-terminated UTF-16 text at `start`, and the offset after the
/// terminator.
fn utf16_string(data: &[u8], start: usize, end: usize) -> Option<(String, usize)> {
    let bytes = Bytes::little(data);
    let mut units = Vec::new();
    let mut at = start;
    while at + 2 <= end {
        let unit = bytes.u16(at)?;
        at += 2;
        if unit == 0 {
            return Some((String::from_utf16_lossy(&units), at));
        }
        units.push(unit);
    }
    Some((String::from_utf16_lossy(&units), at))
}

const fn align4(offset: usize) -> usize {
    (offset + 3) & !3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16z(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn pad4(bytes: &mut Vec<u8>) {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
    }

    /// Version block with `key`, a value and children.
    fn block(key: &str, kind: u16, value: &[u8], value_length: u16, children: &[u8]) -> Vec<u8> {
        let mut body = utf16z(key);
        let mut bytes = vec![0; 6];
        bytes.append(&mut body);
        pad4(&mut bytes);
        bytes.extend_from_slice(value);
        pad4(&mut bytes);
        bytes.extend_from_slice(children);
        let length = u16::try_from(bytes.len()).unwrap();
        bytes[0..2].copy_from_slice(&length.to_le_bytes());
        bytes[2..4].copy_from_slice(&value_length.to_le_bytes());
        bytes[4..6].copy_from_slice(&kind.to_le_bytes());
        bytes
    }

    fn version_info(product: Option<&str>) -> Vec<u8> {
        let mut fixed = Vec::new();
        for word in [
            FIXED_INFO_SIGNATURE,
            0x0001_0000,
            0x0002_0003,
            0,
            0x0004_0005,
            0x0006_0007,
        ] {
            fixed.extend_from_slice(&word.to_le_bytes());
        }
        fixed.resize(52, 0);
        let mut children = Vec::new();
        if let Some(product) = product {
            let text = utf16z(product);
            let words = u16::try_from(text.len() / 2).unwrap();
            let entry = block(PRODUCT_VERSION_KEY, TEXT_VALUE, &text, words, &[]);
            let table = block("040904b0", TEXT_VALUE, &[], 0, &entry);
            children = block(STRING_FILE_INFO_KEY, TEXT_VALUE, &[], 0, &table);
        }
        block(VERSION_INFO_KEY, 0, &fixed, 52, &children)
    }

    /// Minimal 64-bit PE image with one `.rsrc` section holding `resource`.
    fn pe_with_version(resource: &[u8]) -> Vec<u8> {
        const HEADER: usize = 0x40;
        const OPTIONAL_SIZE: usize = 240;
        const SECTION_RVA: u32 = 0x1000;
        const RAW_OFFSET: usize = 0x200;
        let mut image = vec![0u8; RAW_OFFSET];
        image[0..2].copy_from_slice(b"MZ");
        image[DOS_LFANEW..DOS_LFANEW + 4].copy_from_slice(&(HEADER as u32).to_le_bytes());
        image[HEADER..HEADER + 4].copy_from_slice(PE_SIGNATURE);
        image[HEADER + 6..HEADER + 8].copy_from_slice(&1u16.to_le_bytes());
        image[HEADER + 20..HEADER + 22].copy_from_slice(&(OPTIONAL_SIZE as u16).to_le_bytes());
        let optional = HEADER + COFF_END;
        image[optional..optional + 2].copy_from_slice(&PE32_PLUS_MAGIC.to_le_bytes());
        let directory = optional + PE32_PLUS_DIRECTORIES + RESOURCE_DIRECTORY * 8;
        image[directory..directory + 4].copy_from_slice(&SECTION_RVA.to_le_bytes());

        let mut tree = Vec::new();
        let directory_with = |id: u32, target: u32| {
            let mut level = vec![0u8; 16];
            level[14..16].copy_from_slice(&1u16.to_le_bytes());
            level.extend_from_slice(&id.to_le_bytes());
            level.extend_from_slice(&target.to_le_bytes());
            level
        };
        tree.extend(directory_with(RT_VERSION, SUBDIRECTORY | 24));
        tree.extend(directory_with(1, SUBDIRECTORY | 48));
        tree.extend(directory_with(0x409, 72));
        let data_rva = SECTION_RVA + 88;
        tree.extend_from_slice(&data_rva.to_le_bytes());
        tree.extend_from_slice(&u32::try_from(resource.len()).unwrap().to_le_bytes());
        tree.extend_from_slice(&[0; 8]);
        tree.extend_from_slice(resource);

        let section = optional + OPTIONAL_SIZE;
        image[section..section + 5].copy_from_slice(b".rsrc");
        let size = u32::try_from(tree.len()).unwrap();
        image[section + 8..section + 12].copy_from_slice(&size.to_le_bytes());
        image[section + 12..section + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
        image[section + 16..section + 20].copy_from_slice(&size.to_le_bytes());
        image[section + 20..section + 24]
            .copy_from_slice(&u32::try_from(RAW_OFFSET).unwrap().to_le_bytes());
        image.extend(tree);
        image
    }

    #[test]
    fn host_versions_pe_resource_parser() {
        let with_string = pe_with_version(&version_info(Some("2.47.1")));
        assert_eq!(product_version(&with_string).as_deref(), Some("2.47.1"));

        let fixed_only = pe_with_version(&version_info(None));
        assert_eq!(product_version(&fixed_only).as_deref(), Some("4.5.6.7"));

        assert_eq!(product_version(b"MZ not a portable executable"), None);
        assert_eq!(product_version(&with_string[..0x100]), None);
    }
}
