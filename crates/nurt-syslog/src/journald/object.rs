// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Objects of a journal file and decompression of field payloads.
//!
//! Every object starts with a 16-byte header: type (1 byte), flags
//! (1 byte), 6 reserved bytes and the size of the object including the
//! header but without padding. Objects start at multiples of 8.

use std::io::Read;

use crate::cursor::Id128;

use super::Fault;

/// Size of the common object header.
pub(super) const OBJECT_HEADER_SIZE: u64 = 16;
/// Type of a DATA object.
pub(super) const KIND_DATA: u8 = 1;
/// Type of a FIELD object.
pub(super) const KIND_FIELD: u8 = 2;
/// Type of an ENTRY object.
pub(super) const KIND_ENTRY: u8 = 3;
/// Type of the DATA hash table.
pub(super) const KIND_DATA_HASH_TABLE: u8 = 4;
/// Type of the FIELD hash table.
pub(super) const KIND_FIELD_HASH_TABLE: u8 = 5;
/// Type of an ENTRY_ARRAY object.
pub(super) const KIND_ENTRY_ARRAY: u8 = 6;

const FLAG_COMPRESSED_XZ: u8 = 1 << 0;
const FLAG_COMPRESSED_LZ4: u8 = 1 << 1;
const FLAG_COMPRESSED_ZSTD: u8 = 1 << 2;

const DATA_PAYLOAD_REGULAR: u64 = 64;
const DATA_PAYLOAD_COMPACT: u64 = 72;
const DATA_HEADER_READ: u64 = 72;
const FIELD_PAYLOAD: u64 = 40;
/// Longest field name accepted; journald limits names to 64 bytes.
const MAX_FIELD_NAME: u64 = 4096;
const ENTRY_ITEMS: u64 = 64;
const ENTRY_ITEM_REGULAR: u64 = 16;
const ENTRY_ITEM_COMPACT: u64 = 4;
/// Largest number of fields of one entry accepted; journald writes at most
/// 1024 fields per entry.
const MAX_ENTRY_ITEMS: u64 = 1 << 16;
const ENTRY_ARRAY_ITEMS: u64 = 24;
const LZ4_SIZE_PREFIX: usize = 8;

/// Common header of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ObjectHeader {
    /// Object type.
    pub(super) kind: u8,
    /// Compression flags of a DATA object.
    pub(super) flags: u8,
    /// Size including the header, without padding.
    pub(super) size: u64,
}

impl ObjectHeader {
    /// Parses the 16 header bytes.
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, Fault> {
        Ok(Self {
            kind: *bytes.first().ok_or(Fault::Truncated)?,
            flags: *bytes.get(1).ok_or(Fault::Truncated)?,
            size: le_u64(bytes, 8)?,
        })
    }
}

/// DATA object without its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DataObject {
    /// Offset of the object.
    pub(super) offset: u64,
    /// Compression flags.
    pub(super) flags: u8,
    /// Hash of the uncompressed payload.
    pub(super) hash: u64,
    /// Next DATA object in the same hash bucket.
    pub(super) next_hash: u64,
    /// Next DATA object of the same field.
    pub(super) next_field: u64,
    /// First entry that references the object.
    pub(super) entry_offset: u64,
    /// First array of further entries that reference the object.
    pub(super) entry_array_offset: u64,
    /// Number of entries that reference the object.
    pub(super) n_entries: u64,
    /// Offset of the payload.
    pub(super) payload_offset: u64,
    /// Stored size of the payload.
    pub(super) payload_len: u64,
}

impl DataObject {
    /// Number of bytes to read for [`DataObject::parse`].
    pub(super) const HEADER_READ: u64 = DATA_HEADER_READ;

    /// Parses the fixed part of a DATA object read at `offset`.
    pub(super) fn parse(offset: u64, header: ObjectHeader, bytes: &[u8], compact: bool) -> Result<Self, Fault> {
        let start = if compact { DATA_PAYLOAD_COMPACT } else { DATA_PAYLOAD_REGULAR };
        if header.kind != KIND_DATA || header.size < start {
            return Err(Fault::Corrupt);
        }
        Ok(Self {
            offset,
            flags: header.flags,
            hash: le_u64(bytes, 16)?,
            next_hash: le_u64(bytes, 24)?,
            next_field: le_u64(bytes, 32)?,
            entry_offset: le_u64(bytes, 40)?,
            entry_array_offset: le_u64(bytes, 48)?,
            n_entries: le_u64(bytes, 56)?,
            payload_offset: offset.checked_add(start).ok_or(Fault::Corrupt)?,
            payload_len: header.size - start,
        })
    }

    /// Whether the payload is stored compressed.
    pub(super) const fn compressed(&self) -> bool {
        self.flags & (FLAG_COMPRESSED_XZ | FLAG_COMPRESSED_LZ4 | FLAG_COMPRESSED_ZSTD) != 0
    }
}

/// FIELD object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FieldObject {
    /// Hash of the name.
    pub(super) hash: u64,
    /// Next FIELD object in the same hash bucket.
    pub(super) next_hash: u64,
    /// First DATA object of the field.
    pub(super) head_data: u64,
    /// Field name.
    pub(super) name: Vec<u8>,
}

impl FieldObject {
    /// Minimum size of a FIELD object.
    pub(super) const MIN_SIZE: u64 = FIELD_PAYLOAD;

    /// Parses a whole FIELD object.
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, Fault> {
        let header = ObjectHeader::parse(bytes)?;
        if header.kind != KIND_FIELD || header.size < FIELD_PAYLOAD || header.size - FIELD_PAYLOAD > MAX_FIELD_NAME {
            return Err(Fault::Corrupt);
        }
        let name = bytes
            .get(usize::try_from(FIELD_PAYLOAD).map_err(|_| Fault::Corrupt)?..)
            .ok_or(Fault::Truncated)?;
        Ok(Self {
            hash: le_u64(bytes, 16)?,
            next_hash: le_u64(bytes, 24)?,
            head_data: le_u64(bytes, 32)?,
            name: name.to_vec(),
        })
    }

    /// Largest FIELD object accepted.
    pub(super) const MAX_SIZE: u64 = FIELD_PAYLOAD + MAX_FIELD_NAME;
}

/// ENTRY object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EntryObject {
    /// Offset of the object.
    pub(super) offset: u64,
    /// Sequence number.
    pub(super) seqnum: u64,
    /// Reception time in microseconds since the UNIX epoch.
    pub(super) realtime: u64,
    /// Monotonic time of the boot in microseconds.
    pub(super) monotonic: u64,
    /// Boot identifier.
    pub(super) boot_id: Id128,
    /// XOR of the lookup3 hashes of the payloads.
    pub(super) xor_hash: u64,
    /// Offsets of the DATA objects.
    pub(super) items: Vec<u64>,
}

impl EntryObject {
    /// Size of the fixed part of an ENTRY object.
    pub(super) const MIN_SIZE: u64 = ENTRY_ITEMS;

    /// Largest ENTRY object accepted.
    pub(super) const fn max_size(compact: bool) -> u64 {
        ENTRY_ITEMS + MAX_ENTRY_ITEMS * entry_item_size(compact)
    }

    /// Parses a whole ENTRY object read at `offset`.
    pub(super) fn parse(offset: u64, bytes: &[u8], compact: bool) -> Result<Self, Fault> {
        let header = ObjectHeader::parse(bytes)?;
        if header.kind != KIND_ENTRY || header.size < ENTRY_ITEMS {
            return Err(Fault::Corrupt);
        }
        let item_size = entry_item_size(compact);
        let count = (header.size - ENTRY_ITEMS) / item_size;
        let mut items = Vec::with_capacity(usize::try_from(count).map_err(|_| Fault::Corrupt)?);
        for index in 0..count {
            let at = usize::try_from(ENTRY_ITEMS + index * item_size).map_err(|_| Fault::Corrupt)?;
            let item = if compact { u64::from(le_u32(bytes, at)?) } else { le_u64(bytes, at)? };
            items.push(item);
        }
        let boot = bytes.get(40..56).ok_or(Fault::Truncated)?;
        Ok(Self {
            offset,
            seqnum: le_u64(bytes, 16)?,
            realtime: le_u64(bytes, 24)?,
            monotonic: le_u64(bytes, 32)?,
            boot_id: Id128::new(boot.try_into().map_err(|_| Fault::Truncated)?),
            xor_hash: le_u64(bytes, 56)?,
            items,
        })
    }
}

/// Size of one item of an ENTRY object.
pub(super) const fn entry_item_size(compact: bool) -> u64 {
    if compact { ENTRY_ITEM_COMPACT } else { ENTRY_ITEM_REGULAR }
}

/// Size of one item of an ENTRY_ARRAY object.
pub(super) const fn array_item_size(compact: bool) -> u64 {
    if compact { ENTRY_ITEM_COMPACT } else { 8 }
}

/// Offset of the first item of an ENTRY_ARRAY object.
pub(super) const ENTRY_ARRAY_ITEMS_OFFSET: u64 = ENTRY_ARRAY_ITEMS;

/// Payload of a DATA object after decompression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Payload {
    /// Bytes `NAME=value`, possibly cut.
    pub(super) bytes: Vec<u8>,
    /// Whether the payload exceeded the size limit and was cut.
    pub(super) truncated: bool,
}

/// Decompresses a stored payload according to the object flags, producing
/// at most `max` bytes.
pub(super) fn decompress(flags: u8, stored: Vec<u8>, max: u64) -> Result<Payload, Fault> {
    let limit = usize::try_from(max).unwrap_or(usize::MAX);
    match flags & (FLAG_COMPRESSED_XZ | FLAG_COMPRESSED_LZ4 | FLAG_COMPRESSED_ZSTD) {
        0 => Ok(cut(stored, limit)),
        FLAG_COMPRESSED_ZSTD => {
            let decoder = zstd::stream::read::Decoder::new(stored.as_slice()).map_err(|_| Fault::Corrupt)?;
            read_limited(decoder, limit)
        }
        FLAG_COMPRESSED_XZ => read_limited(liblzma::read::XzDecoder::new(stored.as_slice()), limit),
        FLAG_COMPRESSED_LZ4 => decompress_lz4(&stored, limit),
        _ => Err(Fault::Corrupt),
    }
}

fn cut(mut bytes: Vec<u8>, limit: usize) -> Payload {
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    Payload { bytes, truncated }
}

fn read_limited(reader: impl Read, limit: usize) -> Result<Payload, Fault> {
    let mut bytes = Vec::new();
    let take = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    reader.take(take).read_to_end(&mut bytes).map_err(|_| Fault::Corrupt)?;
    Ok(cut(bytes, limit))
}

/// LZ4 payloads are a 64-bit little-endian uncompressed size followed by
/// one LZ4 block.
fn decompress_lz4(stored: &[u8], limit: usize) -> Result<Payload, Fault> {
    let prefix: [u8; LZ4_SIZE_PREFIX] = stored
        .get(..LZ4_SIZE_PREFIX)
        .and_then(|prefix| prefix.try_into().ok())
        .ok_or(Fault::Corrupt)?;
    let size = usize::try_from(u64::from_le_bytes(prefix)).map_err(|_| Fault::Corrupt)?;
    if size > limit {
        return Ok(Payload {
            bytes: Vec::new(),
            truncated: true,
        });
    }
    let mut bytes = vec![0_u8; size];
    let block = stored.get(LZ4_SIZE_PREFIX..).ok_or(Fault::Corrupt)?;
    let written = lz4_flex::block::decompress_into(block, &mut bytes).map_err(|_| Fault::Corrupt)?;
    if written != size {
        return Err(Fault::Corrupt);
    }
    Ok(Payload {
        bytes,
        truncated: false,
    })
}

fn le_u64(bytes: &[u8], offset: usize) -> Result<u64, Fault> {
    let slice = bytes.get(offset..offset + 8).ok_or(Fault::Truncated)?;
    Ok(u64::from_le_bytes(slice.try_into().map_err(|_| Fault::Truncated)?))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, Fault> {
    let slice = bytes.get(offset..offset + 4).ok_or(Fault::Truncated)?;
    Ok(u32::from_le_bytes(slice.try_into().map_err(|_| Fault::Truncated)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const TEXT: &[u8] = b"MESSAGE=the quick brown fox jumps over the lazy dog, again and again";

    #[test]
    fn plain_payload_is_cut_at_limit() {
        let payload = decompress(0, TEXT.to_vec(), 7).unwrap();
        assert_eq!(payload.bytes, b"MESSAGE");
        assert!(payload.truncated);
        assert!(!decompress(0, TEXT.to_vec(), 1000).unwrap().truncated);
    }

    #[test]
    fn decompresses_zstd_frame() {
        let stored = zstd::encode_all(TEXT, 3).unwrap();
        assert_eq!(decompress(FLAG_COMPRESSED_ZSTD, stored.clone(), 1000).unwrap().bytes, TEXT);
        let cut = decompress(FLAG_COMPRESSED_ZSTD, stored, 10).unwrap();
        assert_eq!(cut.bytes.len(), 10);
        assert!(cut.truncated);
    }

    #[test]
    fn decompresses_xz_stream() {
        let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 6);
        encoder.write_all(TEXT).unwrap();
        let stored = encoder.finish().unwrap();
        assert_eq!(decompress(FLAG_COMPRESSED_XZ, stored, 1000).unwrap().bytes, TEXT);
    }

    #[test]
    fn decompresses_lz4_block_with_size_prefix() {
        let mut stored = (TEXT.len() as u64).to_le_bytes().to_vec();
        stored.extend(lz4_flex::block::compress(TEXT));
        assert_eq!(decompress(FLAG_COMPRESSED_LZ4, stored.clone(), 1000).unwrap().bytes, TEXT);
        let over = decompress(FLAG_COMPRESSED_LZ4, stored.clone(), 5).unwrap();
        assert!(over.truncated && over.bytes.is_empty());
        let mut wrong = stored;
        wrong[0] = wrong[0].wrapping_add(1);
        assert!(matches!(decompress(FLAG_COMPRESSED_LZ4, wrong, 1000), Err(Fault::Corrupt)));
    }

    #[test]
    fn rejects_damaged_or_ambiguous_payloads() {
        assert!(decompress(FLAG_COMPRESSED_ZSTD, b"not zstd".to_vec(), 100).is_err());
        assert!(decompress(FLAG_COMPRESSED_XZ, b"not xz".to_vec(), 100).is_err());
        assert!(decompress(FLAG_COMPRESSED_LZ4, b"short".to_vec(), 100).is_err());
        assert!(decompress(FLAG_COMPRESSED_XZ | FLAG_COMPRESSED_LZ4, TEXT.to_vec(), 100).is_err());
    }

    #[test]
    fn parses_entry_items_in_both_modes() {
        let mut regular = vec![0_u8; 64 + 32];
        regular[0] = KIND_ENTRY;
        regular[8..16].copy_from_slice(&96_u64.to_le_bytes());
        regular[16..24].copy_from_slice(&5_u64.to_le_bytes());
        regular[64..72].copy_from_slice(&800_u64.to_le_bytes());
        regular[80..88].copy_from_slice(&808_u64.to_le_bytes());
        let entry = EntryObject::parse(400, &regular, false).unwrap();
        assert_eq!(entry.items, [800, 808]);
        assert_eq!(entry.seqnum, 5);

        let mut compact = vec![0_u8; 64 + 8];
        compact[0] = KIND_ENTRY;
        compact[8..16].copy_from_slice(&72_u64.to_le_bytes());
        compact[64..68].copy_from_slice(&800_u32.to_le_bytes());
        compact[68..72].copy_from_slice(&808_u32.to_le_bytes());
        assert_eq!(EntryObject::parse(400, &compact, true).unwrap().items, [800, 808]);
        assert_eq!(EntryObject::max_size(true), 64 + 4 * 65_536);
    }

    #[test]
    fn rejects_objects_of_wrong_type_or_size() {
        let mut bytes = vec![0_u8; 72];
        bytes[0] = KIND_FIELD;
        bytes[8..16].copy_from_slice(&72_u64.to_le_bytes());
        let header = ObjectHeader::parse(&bytes).unwrap();
        assert!(DataObject::parse(0, header, &bytes, false).is_err());
        assert!(EntryObject::parse(0, &bytes, false).is_err());
        bytes[8..16].copy_from_slice(&8_u64.to_le_bytes());
        assert!(FieldObject::parse(&bytes).is_err());
        assert!(ObjectHeader::parse(&bytes[..4]).is_err());
    }

    #[test]
    fn parses_field_name() {
        let mut bytes = vec![0_u8; 47];
        bytes[0] = KIND_FIELD;
        bytes[8..16].copy_from_slice(&47_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&1000_u64.to_le_bytes());
        bytes[40..].copy_from_slice(b"MESSAGE");
        let field = FieldObject::parse(&bytes).unwrap();
        assert_eq!(field.name, b"MESSAGE");
        assert_eq!(field.head_data, 1000);
    }

    #[test]
    fn data_object_reports_compression() {
        let mut bytes = vec![0_u8; 72];
        bytes[0] = KIND_DATA;
        bytes[1] = FLAG_COMPRESSED_ZSTD;
        bytes[8..16].copy_from_slice(&80_u64.to_le_bytes());
        let header = ObjectHeader::parse(&bytes).unwrap();
        let data = DataObject::parse(8, header, &bytes, true).unwrap();
        assert!(data.compressed());
        assert_eq!(data.payload_offset, 80);
        assert_eq!(data.payload_len, 8);
    }
}
