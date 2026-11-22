// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Open journal file with validated access to its objects.
//!
//! The read boundary is fixed when the file is opened: the smaller of the
//! file size, the area declared by the header and the end of the last
//! object recorded in the header. Objects that journald appends later are
//! not read, which keeps the result of one query consistent.

use super::Fault;
use super::header::{HEADER_READ_SIZE, Header};
use super::object::{
    DataObject, ENTRY_ARRAY_ITEMS_OFFSET, EntryObject, FieldObject, KIND_DATA,
    KIND_DATA_HASH_TABLE, KIND_ENTRY, KIND_ENTRY_ARRAY, KIND_FIELD, KIND_FIELD_HASH_TABLE,
    OBJECT_HEADER_SIZE, ObjectHeader, Payload, array_item_size, decompress,
};
use super::reader::{BlockReader, Source};

const HASH_ITEM_SIZE: u64 = 16;
const ALIGNMENT: u64 = 8;
const REALTIME_OFFSET: u64 = 24;
const ENTRY_ARRAY_NEXT_OFFSET: u64 = 16;
const DATA_MIN_REGULAR: u64 = 64;
const DATA_MIN_COMPACT: u64 = 72;

/// Journal file opened for reading.
#[derive(Debug)]
pub(super) struct JournalFile {
    reader: BlockReader,
    header: Header,
    max_steps: u64,
    truncated: bool,
}

impl JournalFile {
    /// Opens a file, reads its header and fixes the read boundary.
    pub(super) fn open(source: Source, cache_bytes: u64) -> Result<Self, Fault> {
        let mut reader = BlockReader::new(source, cache_bytes)?;
        let file_len = reader.limit();
        let bytes = reader.read(0, HEADER_READ_SIZE.min(file_len))?;
        let header = Header::parse(&bytes)?;
        let declared = header.declared_end().ok_or(Fault::Corrupt)?;
        let truncated = file_len < declared;
        let mut limit = file_len.min(declared);
        if header.tail_object_offset == 0 {
            limit = limit.min(header.header_size);
        } else if header.tail_object_offset < limit {
            let tail = header.tail_object_offset;
            if !tail.is_multiple_of(ALIGNMENT) || tail < header.header_size {
                return Err(Fault::Corrupt);
            }
            let end = match reader.read(tail, OBJECT_HEADER_SIZE) {
                Ok(bytes) => tail.checked_add(ObjectHeader::parse(&bytes)?.size),
                Err(Fault::Beyond) => None,
                Err(fault) => return Err(fault),
            };
            limit = match end {
                Some(end) if end <= limit => end,
                _ => tail,
            };
        } else if !truncated {
            return Err(Fault::Corrupt);
        }
        reader.set_limit(limit);
        let mut file = Self {
            reader,
            max_steps: limit / OBJECT_HEADER_SIZE + 1,
            header,
            truncated,
        };
        let data_table = (
            file.header.data_hash_table_offset,
            file.header.data_hash_table_size,
        );
        let field_table = (
            file.header.field_hash_table_offset,
            file.header.field_hash_table_size,
        );
        file.check_table(data_table, KIND_DATA_HASH_TABLE)?;
        file.check_table(field_table, KIND_FIELD_HASH_TABLE)?;
        Ok(file)
    }

    /// Header of the file.
    pub(super) const fn header(&self) -> &Header {
        &self.header
    }

    /// Whether the file is shorter than its header declares.
    pub(super) const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Read boundary.
    pub(super) const fn limit(&self) -> u64 {
        self.reader.limit()
    }

    /// Bound of the number of links followed in one chain.
    pub(super) const fn max_steps(&self) -> u64 {
        self.max_steps
    }

    fn check_table(&mut self, (offset, size): (u64, u64), kind: u8) -> Result<(), Fault> {
        if size == 0 || !size.is_multiple_of(HASH_ITEM_SIZE) || !offset.is_multiple_of(ALIGNMENT) {
            return Err(Fault::Corrupt);
        }
        let object = offset
            .checked_sub(OBJECT_HEADER_SIZE)
            .ok_or(Fault::Corrupt)?;
        let header = self.object_header(object, kind, OBJECT_HEADER_SIZE, u64::MAX)?;
        if header.size - OBJECT_HEADER_SIZE < size {
            return Err(Fault::Corrupt);
        }
        Ok(())
    }

    fn object_header(
        &mut self,
        offset: u64,
        kind: u8,
        min: u64,
        max: u64,
    ) -> Result<ObjectHeader, Fault> {
        if !offset.is_multiple_of(ALIGNMENT) || offset < self.header.header_size {
            return Err(Fault::Corrupt);
        }
        let header = ObjectHeader::parse(&self.reader.read(offset, OBJECT_HEADER_SIZE)?)?;
        if header.kind != kind || header.size < min || header.size > max {
            return Err(Fault::Corrupt);
        }
        let end = offset.checked_add(header.size).ok_or(Fault::Corrupt)?;
        if end > self.reader.limit() {
            return Err(Fault::Beyond);
        }
        Ok(header)
    }

    /// DATA object at `offset`.
    pub(super) fn data(&mut self, offset: u64) -> Result<DataObject, Fault> {
        let compact = self.header.compact();
        let min = if compact {
            DATA_MIN_COMPACT
        } else {
            DATA_MIN_REGULAR
        };
        let header = self.object_header(offset, KIND_DATA, min, u64::MAX)?;
        let bytes = self
            .reader
            .read(offset, DataObject::HEADER_READ.min(header.size))?;
        DataObject::parse(offset, header, &bytes, compact)
    }

    /// Payload of a DATA object, cut to `max` bytes.
    pub(super) fn payload(&mut self, data: &DataObject, max: u64) -> Result<Payload, Fault> {
        if !data.compressed() && data.payload_len > max {
            let bytes = self.reader.read(data.payload_offset, max)?;
            return Ok(Payload {
                bytes,
                truncated: true,
            });
        }
        if data.compressed() && data.payload_len > max {
            return Ok(Payload {
                bytes: Vec::new(),
                truncated: true,
            });
        }
        let stored = self.reader.read(data.payload_offset, data.payload_len)?;
        decompress(data.flags, stored, max)
    }

    /// ENTRY object at `offset`.
    pub(super) fn entry(&mut self, offset: u64) -> Result<EntryObject, Fault> {
        let compact = self.header.compact();
        let header = self.object_header(
            offset,
            KIND_ENTRY,
            EntryObject::MIN_SIZE,
            EntryObject::max_size(compact),
        )?;
        let bytes = self.reader.read(offset, header.size)?;
        EntryObject::parse(offset, &bytes, compact)
    }

    /// Reception time of the ENTRY object at `offset`.
    pub(super) fn entry_realtime(&mut self, offset: u64) -> Result<u64, Fault> {
        let compact = self.header.compact();
        self.object_header(
            offset,
            KIND_ENTRY,
            EntryObject::MIN_SIZE,
            EntryObject::max_size(compact),
        )?;
        self.reader.read_u64(offset + REALTIME_OFFSET)
    }

    /// Link to the next array and the capacity of the ENTRY_ARRAY at
    /// `offset`.
    pub(super) fn entry_array(&mut self, offset: u64) -> Result<(u64, u64), Fault> {
        let header =
            self.object_header(offset, KIND_ENTRY_ARRAY, ENTRY_ARRAY_ITEMS_OFFSET, u64::MAX)?;
        let next = self.reader.read_u64(offset + ENTRY_ARRAY_NEXT_OFFSET)?;
        let capacity =
            (header.size - ENTRY_ARRAY_ITEMS_OFFSET) / array_item_size(self.header.compact());
        Ok((next, capacity))
    }

    /// Item `index` of the ENTRY_ARRAY at `offset`; the caller has checked
    /// the index against the capacity.
    pub(super) fn entry_array_item(&mut self, offset: u64, index: u64) -> Result<u64, Fault> {
        let compact = self.header.compact();
        let position = index
            .checked_mul(array_item_size(compact))
            .and_then(|delta| delta.checked_add(offset + ENTRY_ARRAY_ITEMS_OFFSET))
            .ok_or(Fault::Corrupt)?;
        if compact {
            self.reader.read_u32(position).map(u64::from)
        } else {
            self.reader.read_u64(position)
        }
    }

    fn bucket_head(&mut self, (offset, size): (u64, u64), hash: u64) -> Result<u64, Fault> {
        let bucket = hash % (size / HASH_ITEM_SIZE);
        self.reader.read_u64(offset + bucket * HASH_ITEM_SIZE)
    }

    /// DATA object whose payload is `payload`, found through the hash table.
    pub(super) fn find_data(&mut self, payload: &[u8]) -> Result<Option<DataObject>, Fault> {
        let hash = self.header.hash(payload);
        let table = (
            self.header.data_hash_table_offset,
            self.header.data_hash_table_size,
        );
        let mut offset = self.bucket_head(table, hash)?;
        let mut steps = 0;
        while offset != 0 {
            steps += 1;
            if steps > self.max_steps {
                return Err(Fault::Corrupt);
            }
            let data = match self.data(offset) {
                Ok(data) => data,
                Err(Fault::Beyond) => return Ok(None),
                Err(fault) => return Err(fault),
            };
            if data.hash == hash {
                let stored = self.payload(&data, payload.len() as u64 + 1)?;
                if stored.bytes == payload {
                    return Ok(Some(data));
                }
            }
            offset = data.next_hash;
        }
        Ok(None)
    }

    /// DATA objects of the field `name`, with their payloads cut to `max`.
    pub(super) fn field_data(
        &mut self,
        name: &[u8],
        max: u64,
    ) -> Result<Vec<(DataObject, Payload)>, Fault> {
        let hash = self.header.hash(name);
        let table = (
            self.header.field_hash_table_offset,
            self.header.field_hash_table_size,
        );
        let mut offset = self.bucket_head(table, hash)?;
        let mut steps = 0;
        let mut head = 0;
        while offset != 0 {
            steps += 1;
            if steps > self.max_steps {
                return Err(Fault::Corrupt);
            }
            let header = match self.object_header(
                offset,
                KIND_FIELD,
                FieldObject::MIN_SIZE,
                FieldObject::MAX_SIZE,
            ) {
                Ok(header) => header,
                Err(Fault::Beyond) => break,
                Err(fault) => return Err(fault),
            };
            let field = FieldObject::parse(&self.reader.read(offset, header.size)?)?;
            if field.hash == hash && field.name == name {
                head = field.head_data;
                break;
            }
            offset = field.next_hash;
        }
        let mut values = Vec::new();
        let mut offset = head;
        let mut steps = 0;
        while offset != 0 {
            steps += 1;
            if steps > self.max_steps {
                return Err(Fault::Corrupt);
            }
            let data = match self.data(offset) {
                Ok(data) => data,
                Err(Fault::Beyond) => break,
                Err(fault) => return Err(fault),
            };
            let payload = self.payload(&data, max)?;
            offset = data.next_field;
            values.push((data, payload));
        }
        Ok(values)
    }
}
