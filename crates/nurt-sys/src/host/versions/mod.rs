// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Versions of programs read from the metadata of their files.
//!
//! No program is started. The version is taken from the first source that
//! has one: the `Info.plist` of the application bundle that holds the file,
//! the version resource of a PE file (`ProductVersion`, then the fixed
//! product version), the `__TEXT,__info_plist` section of a Mach-O file, or
//! the `.note.package` note of an ELF file. A file without metadata has no
//! version.

mod elf;
mod macho;
mod pe;

use std::io::Read;
use std::path::Path;

/// Largest file whose metadata is read; larger files have no version.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Version of the program file `path`, from its metadata.
pub(crate) fn read_version(path: &Path) -> Option<String> {
    if let Some(version) = macho::bundle_version(path) {
        return Some(version);
    }
    let bytes = read_limited(path)?;
    pe::product_version(&bytes)
        .or_else(|| macho::embedded_version(&bytes))
        .or_else(|| elf::package_version(&bytes))
}

fn read_limited(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Bounds-checked reader of integers in a byte slice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bytes<'a> {
    data: &'a [u8],
    big_endian: bool,
}

impl<'a> Bytes<'a> {
    /// Reader of little-endian integers.
    pub(crate) const fn little(data: &'a [u8]) -> Self {
        Self {
            data,
            big_endian: false,
        }
    }

    /// Reader of big-endian integers.
    pub(crate) const fn big(data: &'a [u8]) -> Self {
        Self {
            data,
            big_endian: true,
        }
    }

    /// The bytes `offset..offset + len`.
    pub(crate) fn slice(&self, offset: usize, len: usize) -> Option<&'a [u8]> {
        self.data.get(offset..offset.checked_add(len)?)
    }

    /// The bytes `offset..offset + len` with 64-bit bounds.
    pub(crate) fn slice64(&self, offset: u64, len: u64) -> Option<&'a [u8]> {
        self.slice(usize::try_from(offset).ok()?, usize::try_from(len).ok()?)
    }

    fn array<const N: usize>(&self, offset: usize) -> Option<[u8; N]> {
        self.slice(offset, N)?.try_into().ok()
    }

    /// 16-bit integer at `offset`.
    pub(crate) fn u16(&self, offset: usize) -> Option<u16> {
        let raw = self.array(offset)?;
        Some(if self.big_endian {
            u16::from_be_bytes(raw)
        } else {
            u16::from_le_bytes(raw)
        })
    }

    /// 32-bit integer at `offset`.
    pub(crate) fn u32(&self, offset: usize) -> Option<u32> {
        let raw = self.array(offset)?;
        Some(if self.big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    }

    /// 64-bit integer at `offset`.
    pub(crate) fn u64(&self, offset: usize) -> Option<u64> {
        let raw = self.array(offset)?;
        Some(if self.big_endian {
            u64::from_be_bytes(raw)
        } else {
            u64::from_le_bytes(raw)
        })
    }

    /// 32-bit integer at `offset` as an index.
    pub(crate) fn index32(&self, offset: usize) -> Option<usize> {
        usize::try_from(self.u32(offset)?).ok()
    }
}

/// Version from the keys of an `Info.plist` document.
pub(crate) fn plist_version(document: &plist::Value) -> Option<String> {
    let dictionary = document.as_dictionary()?;
    ["CFBundleShortVersionString", "CFBundleVersion"]
        .iter()
        .find_map(|key| dictionary.get(key)?.as_string())
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
}
