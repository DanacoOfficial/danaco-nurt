// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Header of a journal file.
//!
//! All numbers are little-endian. Fields after `tail_entry_monotonic`
//! (offset 208) were added in later versions and exist only when
//! `header_size` covers them.

use crate::cursor::Id128;

use super::Fault;
use super::hash::{lookup3, siphash24};

/// Signature at the start of every journal file.
pub(super) const SIGNATURE: &[u8; 8] = b"LPKSHHRH";
/// Smallest supported header: every field up to `tail_entry_monotonic`.
pub(super) const MIN_HEADER_SIZE: u64 = 208;
/// Bytes read for the header; covers every field known to the reader.
pub(super) const HEADER_READ_SIZE: u64 = 272;

/// Incompatible flag: DATA objects compressed with XZ.
pub(super) const INCOMPATIBLE_COMPRESSED_XZ: u32 = 1 << 0;
/// Incompatible flag: DATA objects compressed with LZ4.
pub(super) const INCOMPATIBLE_COMPRESSED_LZ4: u32 = 1 << 1;
/// Incompatible flag: hash tables use SipHash-2-4 keyed with the file id.
pub(super) const INCOMPATIBLE_KEYED_HASH: u32 = 1 << 2;
/// Incompatible flag: DATA objects compressed with zstd.
pub(super) const INCOMPATIBLE_COMPRESSED_ZSTD: u32 = 1 << 3;
/// Incompatible flag: 32-bit offsets in entries and entry arrays.
pub(super) const INCOMPATIBLE_COMPACT: u32 = 1 << 4;
const INCOMPATIBLE_KNOWN: u32 = INCOMPATIBLE_COMPRESSED_XZ
    | INCOMPATIBLE_COMPRESSED_LZ4
    | INCOMPATIBLE_KEYED_HASH
    | INCOMPATIBLE_COMPRESSED_ZSTD
    | INCOMPATIBLE_COMPACT;

const OFFSET_COMPATIBLE: usize = 8;
const OFFSET_INCOMPATIBLE: usize = 12;
const OFFSET_FILE_ID: usize = 24;
const OFFSET_MACHINE_ID: usize = 40;
const OFFSET_SEQNUM_ID: usize = 72;
const OFFSET_HEADER_SIZE: usize = 88;
const OFFSET_ARENA_SIZE: usize = 96;
const OFFSET_DATA_HASH_TABLE: usize = 104;
const OFFSET_FIELD_HASH_TABLE: usize = 120;
const OFFSET_TAIL_OBJECT: usize = 136;
const OFFSET_N_OBJECTS: usize = 144;
const OFFSET_N_ENTRIES: usize = 152;
const OFFSET_HEAD_ENTRY_SEQNUM: usize = 168;
const OFFSET_ENTRY_ARRAY: usize = 176;
const OFFSET_HEAD_ENTRY_REALTIME: usize = 184;
const OFFSET_TAIL_ENTRY_REALTIME: usize = 192;
const ID_LEN: usize = 16;
const HEADER_ALIGNMENT: u64 = 8;

/// Parsed header of a journal file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Header {
    /// Incompatible feature flags.
    pub(super) incompatible: u32,
    /// Random identifier of the file; the key of the keyed hash.
    pub(super) file_id: Id128,
    /// Machine that wrote the file.
    pub(super) machine_id: Id128,
    /// Sequence number space of the entries.
    pub(super) seqnum_id: Id128,
    /// Size of the header in bytes.
    pub(super) header_size: u64,
    /// Size of the object area after the header.
    pub(super) arena_size: u64,
    /// Offset of the items of the DATA hash table.
    pub(super) data_hash_table_offset: u64,
    /// Size of the DATA hash table items in bytes.
    pub(super) data_hash_table_size: u64,
    /// Offset of the items of the FIELD hash table.
    pub(super) field_hash_table_offset: u64,
    /// Size of the FIELD hash table items in bytes.
    pub(super) field_hash_table_size: u64,
    /// Offset of the last object, 0 without objects.
    pub(super) tail_object_offset: u64,
    /// Number of objects.
    pub(super) n_objects: u64,
    /// Number of entries.
    pub(super) n_entries: u64,
    /// Sequence number of the first entry.
    pub(super) head_entry_seqnum: u64,
    /// Offset of the first array of the global entry index.
    pub(super) entry_array_offset: u64,
    /// Reception time of the first entry in microseconds.
    pub(super) head_entry_realtime: u64,
    /// Reception time of the last entry in microseconds.
    pub(super) tail_entry_realtime: u64,
}

impl Header {
    /// Parses the first bytes of a file.
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, Fault> {
        if bytes.len() < SIGNATURE.len() {
            return Err(Fault::Truncated);
        }
        if bytes.get(..SIGNATURE.len()) != Some(SIGNATURE.as_slice()) {
            return Err(Fault::Unsupported);
        }
        let incompatible = le_u32(bytes, OFFSET_INCOMPATIBLE)?;
        le_u32(bytes, OFFSET_COMPATIBLE)?;
        if incompatible & !INCOMPATIBLE_KNOWN != 0 {
            return Err(Fault::Unsupported);
        }
        let header_size = le_u64(bytes, OFFSET_HEADER_SIZE)?;
        if header_size < MIN_HEADER_SIZE || !header_size.is_multiple_of(HEADER_ALIGNMENT) {
            return Err(Fault::Unsupported);
        }
        if (bytes.len() as u64) < MIN_HEADER_SIZE {
            return Err(Fault::Truncated);
        }
        Ok(Self {
            incompatible,
            file_id: id(bytes, OFFSET_FILE_ID)?,
            machine_id: id(bytes, OFFSET_MACHINE_ID)?,
            seqnum_id: id(bytes, OFFSET_SEQNUM_ID)?,
            header_size,
            arena_size: le_u64(bytes, OFFSET_ARENA_SIZE)?,
            data_hash_table_offset: le_u64(bytes, OFFSET_DATA_HASH_TABLE)?,
            data_hash_table_size: le_u64(bytes, OFFSET_DATA_HASH_TABLE + 8)?,
            field_hash_table_offset: le_u64(bytes, OFFSET_FIELD_HASH_TABLE)?,
            field_hash_table_size: le_u64(bytes, OFFSET_FIELD_HASH_TABLE + 8)?,
            tail_object_offset: le_u64(bytes, OFFSET_TAIL_OBJECT)?,
            n_objects: le_u64(bytes, OFFSET_N_OBJECTS)?,
            n_entries: le_u64(bytes, OFFSET_N_ENTRIES)?,
            head_entry_seqnum: le_u64(bytes, OFFSET_HEAD_ENTRY_SEQNUM)?,
            entry_array_offset: le_u64(bytes, OFFSET_ENTRY_ARRAY)?,
            head_entry_realtime: le_u64(bytes, OFFSET_HEAD_ENTRY_REALTIME)?,
            tail_entry_realtime: le_u64(bytes, OFFSET_TAIL_ENTRY_REALTIME)?,
        })
    }

    /// Whether entries and entry arrays hold 32-bit offsets.
    pub(super) const fn compact(&self) -> bool {
        self.incompatible & INCOMPATIBLE_COMPACT != 0
    }

    /// Hash of a DATA payload or a FIELD name as stored in this file.
    pub(super) fn hash(&self, bytes: &[u8]) -> u64 {
        if self.incompatible & INCOMPATIBLE_KEYED_HASH == 0 {
            lookup3(bytes)
        } else {
            siphash24(self.file_id.as_bytes(), bytes)
        }
    }

    /// End of the area declared by the header.
    pub(super) fn declared_end(&self) -> Option<u64> {
        self.header_size.checked_add(self.arena_size)
    }
}

fn le_u64(bytes: &[u8], offset: usize) -> Result<u64, Fault> {
    let slice = bytes.get(offset..offset + 8).ok_or(Fault::Truncated)?;
    let array: [u8; 8] = slice.try_into().map_err(|_| Fault::Truncated)?;
    Ok(u64::from_le_bytes(array))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, Fault> {
    let slice = bytes.get(offset..offset + 4).ok_or(Fault::Truncated)?;
    let array: [u8; 4] = slice.try_into().map_err(|_| Fault::Truncated)?;
    Ok(u32::from_le_bytes(array))
}

fn id(bytes: &[u8], offset: usize) -> Result<Id128, Fault> {
    let slice = bytes.get(offset..offset + ID_LEN).ok_or(Fault::Truncated)?;
    let array: [u8; ID_LEN] = slice.try_into().map_err(|_| Fault::Truncated)?;
    Ok(Id128::new(array))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes(header_size: u64, incompatible: u32) -> Vec<u8> {
        let mut bytes = vec![0_u8; HEADER_READ_SIZE as usize];
        bytes[..8].copy_from_slice(SIGNATURE);
        bytes[OFFSET_INCOMPATIBLE..OFFSET_INCOMPATIBLE + 4]
            .copy_from_slice(&incompatible.to_le_bytes());
        bytes[OFFSET_HEADER_SIZE..OFFSET_HEADER_SIZE + 8]
            .copy_from_slice(&header_size.to_le_bytes());
        bytes[OFFSET_FILE_ID] = 7;
        bytes[OFFSET_TAIL_ENTRY_REALTIME] = 9;
        bytes
    }

    #[test]
    fn parses_known_flags_and_fields() {
        let header = Header::parse(&header_bytes(
            272,
            INCOMPATIBLE_COMPACT | INCOMPATIBLE_KEYED_HASH,
        ))
        .unwrap();
        assert!(header.compact());
        assert_eq!(header.header_size, 272);
        assert_eq!(header.file_id.as_bytes()[0], 7);
        assert_eq!(header.tail_entry_realtime, 9);
        assert_eq!(header.declared_end(), Some(272));
        assert_eq!(
            header.hash(b"MESSAGE=hello"),
            siphash24(header.file_id.as_bytes(), b"MESSAGE=hello")
        );
    }

    #[test]
    fn unkeyed_header_uses_lookup3() {
        let header = Header::parse(&header_bytes(208, 0)).unwrap();
        assert!(!header.compact());
        assert_eq!(header.hash(b"MESSAGE=hello"), lookup3(b"MESSAGE=hello"));
    }

    #[test]
    fn rejects_unknown_incompatible_flag() {
        assert!(matches!(
            Header::parse(&header_bytes(272, 1 << 5)),
            Err(Fault::Unsupported)
        ));
    }

    #[test]
    fn rejects_bad_signature_and_small_header() {
        let mut bytes = header_bytes(272, 0);
        bytes[0] = b'X';
        assert!(matches!(Header::parse(&bytes), Err(Fault::Unsupported)));
        assert!(matches!(
            Header::parse(&header_bytes(200, 0)),
            Err(Fault::Unsupported)
        ));
        assert!(matches!(
            Header::parse(&header_bytes(212, 0)),
            Err(Fault::Unsupported)
        ));
    }

    #[test]
    fn short_input_is_truncated() {
        let bytes = header_bytes(272, 0);
        for len in [0, 4, 8, 12, 100, 207] {
            assert!(Header::parse(&bytes[..len]).is_err(), "{len}");
        }
        assert!(matches!(
            Header::parse(&bytes[..100]),
            Err(Fault::Truncated)
        ));
    }
}
