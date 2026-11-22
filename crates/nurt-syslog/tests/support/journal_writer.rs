// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Writer of systemd journal files for tests.
//!
//! The writer follows the published journal file format: a 272-byte header,
//! the FIELD and DATA hash tables as the first objects, DATA and FIELD
//! objects linked into their hash chains, ENTRY objects linked into the
//! global entry array chain and into the entry lists of their DATA objects.
//! Entry arrays double in size. As in journald, a payload is stored
//! compressed only when compression makes it shorter, and the header flag
//! follows the configured compression. Every identifier and time comes from
//! the options and the entries, so equal input gives equal bytes.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "test crates and fuzz targets include the writer as a private module and each uses a different part of it"
)]

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use nurt_syslog::journald::hash::{lookup3, siphash24};

/// Size of the header written.
pub const HEADER_SIZE: u64 = 272;
/// Incompatible flag of XZ compression.
pub const FLAG_XZ: u32 = 1;
/// Incompatible flag of LZ4 compression.
pub const FLAG_LZ4: u32 = 2;
/// Incompatible flag of the keyed hash.
pub const FLAG_KEYED_HASH: u32 = 4;
/// Incompatible flag of zstd compression.
pub const FLAG_ZSTD: u32 = 8;
/// Incompatible flag of the compact mode.
pub const FLAG_COMPACT: u32 = 16;
const COMPATIBLE_TAIL_ENTRY_BOOT_ID: u32 = 2;

const KIND_DATA: u8 = 1;
const KIND_FIELD: u8 = 2;
const KIND_ENTRY: u8 = 3;
const KIND_DATA_HASH_TABLE: u8 = 4;
const KIND_FIELD_HASH_TABLE: u8 = 5;
const KIND_ENTRY_ARRAY: u8 = 6;
const OBJECT_COMPRESSED_XZ: u8 = 1;
const OBJECT_COMPRESSED_LZ4: u8 = 2;
const OBJECT_COMPRESSED_ZSTD: u8 = 4;
/// Preset 0 has a 256 KiB dictionary, close to the 1 MiB journald uses.
const XZ_PRESET: u32 = 0;
const ZSTD_LEVEL: i32 = 3;

/// Machine identifier used by default.
pub const MACHINE_ID: [u8; 16] = [0x5a; 16];

/// Compression of DATA payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Payloads are stored as they are.
    None,
    /// XZ stream.
    Xz,
    /// LZ4 block after a 64-bit size.
    Lz4,
    /// Frame of the zstd format.
    Zstd,
}

impl Compression {
    /// Incompatible header flag of the compression.
    pub const fn flag(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Xz => FLAG_XZ,
            Self::Lz4 => FLAG_LZ4,
            Self::Zstd => FLAG_ZSTD,
        }
    }
}

/// State byte of the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Closed after writing.
    Offline = 0,
    /// Open for writing.
    Online = 1,
    /// Rotated.
    Archived = 2,
}

/// Options of one file.
#[derive(Debug, Clone)]
pub struct Options {
    /// 32-bit offsets in entries and entry arrays.
    pub compact: bool,
    /// SipHash-2-4 keyed with the file id instead of lookup3.
    pub keyed_hash: bool,
    /// Compression of payloads of at least `compress_threshold` bytes.
    pub compression: Compression,
    /// Smallest payload that is compressed.
    pub compress_threshold: usize,
    /// State written to the header.
    pub state: State,
    /// Buckets of the DATA hash table.
    pub data_buckets: u64,
    /// Buckets of the FIELD hash table.
    pub field_buckets: u64,
    /// File identifier.
    pub file_id: [u8; 16],
    /// Machine identifier.
    pub machine_id: [u8; 16],
    /// Sequence number space.
    pub seqnum_id: [u8; 16],
    /// Sequence number of the first entry.
    pub first_seqnum: u64,
    /// Capacity of the first entry array of every chain.
    pub first_array_capacity: u64,
    /// Further incompatible flags written to the header.
    pub extra_incompatible: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            compact: false,
            keyed_hash: false,
            compression: Compression::None,
            compress_threshold: 512,
            state: State::Offline,
            data_buckets: 64,
            field_buckets: 16,
            file_id: [0x11; 16],
            machine_id: MACHINE_ID,
            seqnum_id: [0x22; 16],
            first_seqnum: 1,
            first_array_capacity: 4,
            extra_incompatible: 0,
        }
    }
}

/// Entry to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Reception time in microseconds.
    pub realtime: u64,
    /// Monotonic time in microseconds.
    pub monotonic: u64,
    /// Boot identifier.
    pub boot_id: [u8; 16],
    /// Fields in order; a name may repeat.
    pub fields: Vec<(String, Vec<u8>)>,
}

impl Entry {
    /// Entry with text fields, monotonic time equal to `realtime` and a
    /// fixed boot.
    pub fn new(realtime: u64, fields: &[(&str, &str)]) -> Self {
        Self {
            realtime,
            monotonic: realtime,
            boot_id: [0x33; 16],
            fields: fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.as_bytes().to_vec()))
                .collect(),
        }
    }

    /// Adds a binary field.
    pub fn with_bytes(mut self, name: &str, value: &[u8]) -> Self {
        self.fields.push((name.to_owned(), value.to_vec()));
        self
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Chain {
    first: u64,
    last: u64,
    capacity: u64,
    used: u64,
}

#[derive(Debug)]
struct DataInfo {
    offset: u64,
    hash: u64,
    chain: Chain,
    n_entries: u64,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    n_objects: u64,
    n_entries: u64,
    n_data: u64,
    n_fields: u64,
    n_entry_arrays: u64,
    tail_object: u64,
    head_seqnum: u64,
    tail_seqnum: u64,
    head_realtime: u64,
    tail_realtime: u64,
    tail_monotonic: u64,
    tail_boot: [u8; 16],
    tail_entry: u64,
    global: Chain,
}

/// Journal file under construction.
#[derive(Debug)]
pub struct Writer {
    buf: Vec<u8>,
    options: Options,
    field_table: u64,
    data_table: u64,
    data: HashMap<Vec<u8>, DataInfo>,
    fields: HashMap<Vec<u8>, u64>,
    next_seqnum: u64,
    current: Snapshot,
    frozen: Option<Snapshot>,
}

impl Writer {
    /// Empty file with its hash tables.
    pub fn new(options: Options) -> Self {
        let mut writer = Self {
            buf: vec![0; HEADER_SIZE as usize],
            next_seqnum: options.first_seqnum,
            options,
            field_table: 0,
            data_table: 0,
            data: HashMap::new(),
            fields: HashMap::new(),
            current: Snapshot::default(),
            frozen: None,
        };
        let field_size = 16 + writer.options.field_buckets * 16;
        writer.field_table =
            writer.append(KIND_FIELD_HASH_TABLE, 0, &vec![0; field_size as usize - 16]) + 16;
        let data_size = 16 + writer.options.data_buckets * 16;
        writer.data_table =
            writer.append(KIND_DATA_HASH_TABLE, 0, &vec![0; data_size as usize - 16]) + 16;
        writer
    }

    /// File with `entries` written in order.
    pub fn build(options: Options, entries: &[Entry]) -> Vec<u8> {
        let mut writer = Self::new(options);
        for entry in entries {
            writer.add(entry);
        }
        writer.finish()
    }

    /// Keeps the header at the current state: entries added later are
    /// written and linked, as by a writer that has not updated the header
    /// yet.
    pub fn freeze(&mut self) {
        self.frozen = Some(self.current.clone());
    }

    /// Offset of the DATA object with `payload`, once written.
    pub fn data_offset(&self, payload: &[u8]) -> Option<u64> {
        self.data.get(payload).map(|info| info.offset)
    }

    /// Offset of the last ENTRY object written.
    pub fn last_entry_offset(&self) -> u64 {
        self.current.tail_entry
    }

    /// Offset of the first array of the global entry index.
    pub fn first_entry_array(&self) -> u64 {
        self.current.global.first
    }

    fn hash(&self, payload: &[u8]) -> u64 {
        if self.options.keyed_hash {
            siphash24(&self.options.file_id, payload)
        } else {
            lookup3(payload)
        }
    }

    fn item_size(&self) -> u64 {
        if self.options.compact { 4 } else { 8 }
    }

    fn append(&mut self, kind: u8, flags: u8, body: &[u8]) -> u64 {
        while !self.buf.len().is_multiple_of(8) {
            self.buf.push(0);
        }
        let offset = self.buf.len() as u64;
        let size = 16 + body.len() as u64;
        self.buf.push(kind);
        self.buf.push(flags);
        self.buf.extend_from_slice(&[0; 6]);
        self.buf.extend_from_slice(&size.to_le_bytes());
        self.buf.extend_from_slice(body);
        self.current.n_objects += 1;
        self.current.tail_object = offset;
        offset
    }

    fn put_u64(&mut self, offset: u64, value: u64) {
        let at = offset as usize;
        self.buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(&mut self, offset: u64, value: u32) {
        let at = offset as usize;
        self.buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn get_u64(&self, offset: u64) -> u64 {
        let at = offset as usize;
        u64::from_le_bytes(self.buf[at..at + 8].try_into().unwrap())
    }

    fn link_bucket(&mut self, table: u64, buckets: u64, hash: u64, offset: u64, next_hash_at: u64) {
        let bucket = table + (hash % buckets) * 16;
        let tail = self.get_u64(bucket + 8);
        if tail == 0 {
            self.put_u64(bucket, offset);
        } else {
            self.put_u64(tail + next_hash_at, offset);
        }
        self.put_u64(bucket + 8, offset);
    }

    fn field_object(&mut self, name: &[u8]) -> u64 {
        if let Some(offset) = self.fields.get(name) {
            return *offset;
        }
        let hash = self.hash(name);
        let mut body = Vec::new();
        body.extend_from_slice(&hash.to_le_bytes());
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(name);
        let offset = self.append(KIND_FIELD, 0, &body);
        self.link_bucket(
            self.field_table,
            self.options.field_buckets,
            hash,
            offset,
            24,
        );
        self.fields.insert(name.to_vec(), offset);
        self.current.n_fields += 1;
        offset
    }

    fn compress(&self, payload: &[u8]) -> (u8, Vec<u8>) {
        if payload.len() < self.options.compress_threshold {
            return (0, payload.to_vec());
        }
        let (flag, stored) = match self.options.compression {
            Compression::None => return (0, payload.to_vec()),
            Compression::Zstd => (
                OBJECT_COMPRESSED_ZSTD,
                zstd::encode_all(payload, ZSTD_LEVEL).unwrap(),
            ),
            Compression::Lz4 => {
                let mut stored = (payload.len() as u64).to_le_bytes().to_vec();
                stored.extend(lz4_flex::block::compress(payload));
                (OBJECT_COMPRESSED_LZ4, stored)
            }
            Compression::Xz => {
                let options = lzma_rust2::XzOptions::with_preset(XZ_PRESET);
                let mut encoder = lzma_rust2::XzWriter::new(Vec::new(), options).unwrap();
                encoder.write_all(payload).unwrap();
                (OBJECT_COMPRESSED_XZ, encoder.finish().unwrap())
            }
        };
        if stored.len() < payload.len() {
            (flag, stored)
        } else {
            (0, payload.to_vec())
        }
    }

    fn data_object(&mut self, payload: &[u8]) -> (u64, u64) {
        if let Some(info) = self.data.get(payload) {
            return (info.offset, info.hash);
        }
        let name_end = payload
            .iter()
            .position(|byte| *byte == b'=')
            .unwrap_or(payload.len());
        let field = self.field_object(&payload[..name_end]);
        let hash = self.hash(payload);
        let (flags, stored) = self.compress(payload);
        let mut body = Vec::new();
        body.extend_from_slice(&hash.to_le_bytes());
        body.extend_from_slice(&[0; 40]);
        if self.options.compact {
            body.extend_from_slice(&[0; 8]);
        }
        body.extend_from_slice(&stored);
        let offset = self.append(KIND_DATA, flags, &body);
        self.link_bucket(self.data_table, self.options.data_buckets, hash, offset, 24);
        let head = self.get_u64(field + 32);
        self.put_u64(offset + 32, head);
        self.put_u64(field + 32, offset);
        self.data.insert(
            payload.to_vec(),
            DataInfo {
                offset,
                hash,
                chain: Chain::default(),
                n_entries: 0,
            },
        );
        self.current.n_data += 1;
        (offset, hash)
    }

    fn chain_append(&mut self, mut chain: Chain, entry: u64) -> Chain {
        if chain.first == 0 || chain.used == chain.capacity {
            let capacity = if chain.first == 0 {
                self.options.first_array_capacity
            } else {
                chain.capacity * 2
            };
            let body = vec![0; (8 + capacity * self.item_size()) as usize];
            let array = self.append(KIND_ENTRY_ARRAY, 0, &body);
            self.current.n_entry_arrays += 1;
            if chain.first == 0 {
                chain.first = array;
            } else {
                self.put_u64(chain.last + 16, array);
            }
            chain.last = array;
            chain.capacity = capacity;
            chain.used = 0;
        }
        let at = chain.last + 24 + chain.used * self.item_size();
        if self.options.compact {
            self.put_u32(at, u32::try_from(entry).unwrap());
        } else {
            self.put_u64(at, entry);
        }
        chain.used += 1;
        chain
    }

    /// Appends an entry and links it everywhere.
    pub fn add(&mut self, entry: &Entry) {
        let seqnum = self.next_seqnum;
        self.next_seqnum += 1;
        let mut items = Vec::new();
        let mut xor_hash = 0;
        for (name, value) in &entry.fields {
            let mut payload = name.as_bytes().to_vec();
            payload.push(b'=');
            payload.extend_from_slice(value);
            let (offset, hash) = self.data_object(&payload);
            if !items.iter().any(|(item, _, _)| *item == offset) {
                xor_hash ^= lookup3(&payload);
                items.push((offset, hash, payload));
            }
        }
        let mut body = Vec::new();
        body.extend_from_slice(&seqnum.to_le_bytes());
        body.extend_from_slice(&entry.realtime.to_le_bytes());
        body.extend_from_slice(&entry.monotonic.to_le_bytes());
        body.extend_from_slice(&entry.boot_id);
        body.extend_from_slice(&xor_hash.to_le_bytes());
        for (offset, hash, _) in &items {
            if self.options.compact {
                body.extend_from_slice(&u32::try_from(*offset).unwrap().to_le_bytes());
            } else {
                body.extend_from_slice(&offset.to_le_bytes());
                body.extend_from_slice(&hash.to_le_bytes());
            }
        }
        let offset = self.append(KIND_ENTRY, 0, &body);
        for (data_offset, _, payload) in &items {
            let data_offset = *data_offset;
            let payload = payload.clone();
            let (n_entries, chain) = {
                let info = &self.data[&payload];
                (info.n_entries, info.chain)
            };
            if n_entries == 0 {
                self.put_u64(data_offset + 40, offset);
            } else {
                let chain = self.chain_append(chain, offset);
                self.put_u64(data_offset + 48, chain.first);
                if self.options.compact {
                    self.put_u32(data_offset + 64, u32::try_from(chain.last).unwrap());
                    self.put_u32(data_offset + 68, u32::try_from(n_entries).unwrap());
                }
                self.data.get_mut(&payload).unwrap().chain = chain;
            }
            self.put_u64(data_offset + 56, n_entries + 1);
            self.data.get_mut(&payload).unwrap().n_entries = n_entries + 1;
        }
        self.current.global = self.chain_append(self.current.global, offset);
        if self.current.n_entries == 0 {
            self.current.head_seqnum = seqnum;
            self.current.head_realtime = entry.realtime;
        }
        self.current.n_entries += 1;
        self.current.tail_seqnum = seqnum;
        self.current.tail_realtime = entry.realtime;
        self.current.tail_monotonic = entry.monotonic;
        self.current.tail_boot = entry.boot_id;
        self.current.tail_entry = offset;
    }

    /// Writes the header and returns the bytes of the file.
    pub fn finish(mut self) -> Vec<u8> {
        while !self.buf.len().is_multiple_of(8) {
            self.buf.push(0);
        }
        let snapshot = self.frozen.clone().unwrap_or_else(|| self.current.clone());
        let mut incompatible = self.options.compression.flag() | self.options.extra_incompatible;
        if self.options.keyed_hash {
            incompatible |= FLAG_KEYED_HASH;
        }
        if self.options.compact {
            incompatible |= FLAG_COMPACT;
        }
        let arena = self.buf.len() as u64 - HEADER_SIZE;
        let header = &mut self.buf[..HEADER_SIZE as usize];
        header[..8].copy_from_slice(b"LPKSHHRH");
        header[8..12].copy_from_slice(&COMPATIBLE_TAIL_ENTRY_BOOT_ID.to_le_bytes());
        header[12..16].copy_from_slice(&incompatible.to_le_bytes());
        header[16] = self.options.state as u8;
        header[24..40].copy_from_slice(&self.options.file_id);
        header[40..56].copy_from_slice(&self.options.machine_id);
        header[56..72].copy_from_slice(&snapshot.tail_boot);
        header[72..88].copy_from_slice(&self.options.seqnum_id);
        let values: [(usize, u64); 22] = [
            (88, HEADER_SIZE),
            (96, arena),
            (104, self.data_table),
            (112, self.options.data_buckets * 16),
            (120, self.field_table),
            (128, self.options.field_buckets * 16),
            (136, snapshot.tail_object),
            (144, snapshot.n_objects),
            (152, snapshot.n_entries),
            (160, snapshot.tail_seqnum),
            (168, snapshot.head_seqnum),
            (176, snapshot.global.first),
            (184, snapshot.head_realtime),
            (192, snapshot.tail_realtime),
            (200, snapshot.tail_monotonic),
            (208, snapshot.n_data),
            (216, snapshot.n_fields),
            (224, 0),
            (232, snapshot.n_entry_arrays),
            (240, 0),
            (248, 0),
            (264, snapshot.tail_entry),
        ];
        for (at, value) in values {
            header[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        header[256..260].copy_from_slice(
            &u32::try_from(snapshot.global.last)
                .unwrap_or(0)
                .to_le_bytes(),
        );
        header[260..264].copy_from_slice(
            &u32::try_from(snapshot.global.used)
                .unwrap_or(0)
                .to_le_bytes(),
        );
        self.buf
    }
}

/// Entries `count` entries of two services with priorities 0–7 and
/// increasing times from `start`.
pub fn sample_entries(count: u64, start: u64) -> Vec<Entry> {
    (0..count)
        .map(|index| {
            let (unit, identifier) = if index.is_multiple_of(2) {
                ("ssh.service", "sshd")
            } else {
                ("cron.service", "cron")
            };
            let priority = (index % 8).to_string();
            let message = format!("event {index}");
            Entry::new(
                start + index * 1_000,
                &[
                    ("MESSAGE", message.as_str()),
                    ("PRIORITY", priority.as_str()),
                    ("SYSLOG_IDENTIFIER", identifier),
                    ("_SYSTEMD_UNIT", unit),
                    ("_PID", "100"),
                    ("_HOSTNAME", "host"),
                ],
            )
        })
        .collect()
}

/// Creates a system root with `/etc/machine-id` and the journal files in
/// `/var/log/journal/<machine-id>/`.
pub fn system_root(root: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let machine = MACHINE_ID
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("etc/machine-id"), format!("{machine}\n")).unwrap();
    let directory = root.join("var/log/journal").join(&machine);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, bytes) in files {
        std::fs::write(directory.join(name), bytes).unwrap();
    }
    directory
}
