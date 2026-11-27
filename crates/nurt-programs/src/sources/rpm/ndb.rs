// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::sources::rpm::ndb
// Responsibility:   Reader of the NDB package file of rpm (`Packages.db`).
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reader of the NDB package file of rpm (`Packages.db`).
//!
//! The file starts with slot pages. The first 32 bytes are the file header
//! (magic `RpmP`, version, generation, number of slot pages); every further
//! 16-byte slot holds the magic `Slot`, a package index, the block offset
//! and the block count of one blob. A blob starts at `offset * 16` with the
//! magic `BlbS`, the package index, the generation and the blob length,
//! followed by the stored header. All numbers are little-endian. The reader
//! never writes and skips slots and blobs that fail a check.

/// Magic of the file header.
pub const PKGDB_MAGIC: u32 = u32::from_le_bytes(*b"RpmP");
/// Magic of a slot.
pub const SLOT_MAGIC: u32 = u32::from_le_bytes(*b"Slot");
/// Magic of a blob header.
pub const BLOB_MAGIC: u32 = u32::from_le_bytes(*b"BlbS");
/// Supported version of the file format.
pub const PKGDB_VERSION: u32 = 0;
/// Size of the file header.
pub const HEADER_SIZE: usize = 32;
/// Size of a slot.
pub const SLOT_SIZE: usize = 16;
/// Size of a block, the unit of blob offsets.
pub const BLOCK_SIZE: usize = 16;
/// Size of a slot page.
pub const PAGE_SIZE: usize = 4096;
/// Size of a blob header.
pub const BLOB_HEADER_SIZE: usize = 16;
const OFFSET_SLOT_PAGES: usize = 12;

/// Stored headers of an NDB file and the number of skipped slots.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NdbContent {
    /// Stored headers in slot order.
    pub blobs: Vec<Vec<u8>>,
    /// Slots whose blob failed a check.
    pub skipped: usize,
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Reads the stored headers of an NDB file.
///
/// # Errors
///
/// Fails when the file header has a wrong magic or version, or the slot
/// pages exceed the file.
pub fn read(bytes: &[u8]) -> Result<NdbContent, String> {
    if le32(bytes, 0) != Some(PKGDB_MAGIC) {
        return Err("not an ndb package file".to_owned());
    }
    if le32(bytes, 4) != Some(PKGDB_VERSION) {
        return Err("unsupported ndb version".to_owned());
    }
    let pages = le32(bytes, OFFSET_SLOT_PAGES).ok_or("truncated ndb header")? as usize;
    let slots_end = pages
        .checked_mul(PAGE_SIZE)
        .filter(|end| *end <= bytes.len() && *end >= HEADER_SIZE)
        .ok_or("ndb slot pages exceed the file")?;
    let mut content = NdbContent::default();
    // Blobs of a valid file do not overlap, so their total size cannot exceed
    // the file; slots that point to the same bytes would multiply the copies.
    let mut kept = 0usize;
    for at in (HEADER_SIZE..slots_end).step_by(SLOT_SIZE) {
        if le32(bytes, at) != Some(SLOT_MAGIC) {
            continue;
        }
        let index = le32(bytes, at + 4).unwrap_or_default();
        if index == 0 {
            continue;
        }
        let offset = le32(bytes, at + 8).unwrap_or_default() as usize;
        match blob(bytes, index, offset) {
            Some(data) if kept.saturating_add(data.len()) <= bytes.len() => {
                kept += data.len();
                content.blobs.push(data.to_vec());
            }
            _ => content.skipped += 1,
        }
    }
    Ok(content)
}

fn blob(bytes: &[u8], index: u32, offset: usize) -> Option<&[u8]> {
    let start = offset.checked_mul(BLOCK_SIZE)?;
    if le32(bytes, start)? != BLOB_MAGIC || le32(bytes, start + 4)? != index {
        return None;
    }
    let length = le32(bytes, start + 12)? as usize;
    let data_start = start + BLOB_HEADER_SIZE;
    bytes.get(data_start..data_start.checked_add(length)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(blobs: &[&[u8]]) -> Vec<u8> {
        let mut out = vec![0u8; PAGE_SIZE];
        out[..4].copy_from_slice(&PKGDB_MAGIC.to_le_bytes());
        out[OFFSET_SLOT_PAGES..OFFSET_SLOT_PAGES + 4].copy_from_slice(&1u32.to_le_bytes());
        for (number, data) in blobs.iter().enumerate() {
            let index = number as u32 + 1;
            let offset = out.len() / BLOCK_SIZE;
            let slot = HEADER_SIZE + number * SLOT_SIZE;
            out[slot..slot + 4].copy_from_slice(&SLOT_MAGIC.to_le_bytes());
            out[slot + 4..slot + 8].copy_from_slice(&index.to_le_bytes());
            out[slot + 8..slot + 12].copy_from_slice(&(offset as u32).to_le_bytes());
            out.extend(BLOB_MAGIC.to_le_bytes());
            out.extend(index.to_le_bytes());
            out.extend(0u32.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(*data);
            while !out.len().is_multiple_of(BLOCK_SIZE) {
                out.push(0);
            }
        }
        out
    }

    #[test]
    fn reads_blobs_in_slot_order() {
        let content = read(&file(&[b"first", b"second"])).unwrap();
        assert_eq!(content.blobs, vec![b"first".to_vec(), b"second".to_vec()]);
        assert_eq!(content.skipped, 0);
    }

    #[test]
    fn slots_that_share_one_blob_do_not_multiply_the_copies() {
        let blob_data = vec![7u8; 3000];
        let mut bytes = file(&[blob_data.as_slice()]);
        let first_slot: Vec<u8> = bytes[HEADER_SIZE..HEADER_SIZE + SLOT_SIZE].to_vec();
        for number in 1..6 {
            let slot = HEADER_SIZE + number * SLOT_SIZE;
            bytes[slot..slot + SLOT_SIZE].copy_from_slice(&first_slot);
        }
        let content = read(&bytes).unwrap();
        let total: usize = content.blobs.iter().map(Vec::len).sum();
        assert!(total <= bytes.len());
        assert!(!content.blobs.is_empty());
        assert!(content.skipped >= 1);
        assert_eq!(content.blobs.len() + content.skipped, 6);
    }

    #[test]
    fn rejects_wrong_magic_and_skips_damaged_blob() {
        assert!(read(b"nope").is_err());
        let mut bytes = file(&[b"first"]);
        let blob_start = PAGE_SIZE;
        bytes[blob_start] = 0;
        let content = read(&bytes).unwrap();
        assert!(content.blobs.is_empty());
        assert_eq!(content.skipped, 1);
        let mut bad_pages = file(&[]);
        bad_pages[OFFSET_SLOT_PAGES] = 9;
        assert!(read(&bad_pages).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
