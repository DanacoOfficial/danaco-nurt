// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Positioned reads of a journal file through a block cache.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;

use super::Fault;

/// Size of a cached block.
pub(super) const BLOCK_SIZE: u64 = 64 * 1024;
const MIN_CACHED_BLOCKS: u64 = 2;

/// Bytes of a journal file: an open file or a copy in memory.
#[derive(Debug)]
pub(super) enum Source {
    /// File read with positioned reads.
    File(File),
    /// File content held in memory.
    Memory(Vec<u8>),
}

impl Source {
    /// Current length in bytes.
    pub(super) fn len(&self) -> io::Result<u64> {
        match self {
            Self::File(file) => Ok(file.metadata()?.len()),
            Self::Memory(bytes) => Ok(bytes.len() as u64),
        }
    }

    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        match self {
            Self::File(file) => file_read_at(file, buf, offset),
            Self::Memory(bytes) => {
                let start = usize::try_from(offset).unwrap_or(usize::MAX);
                let available = bytes.get(start..).unwrap_or_default();
                let count = available.len().min(buf.len());
                if let (Some(target), Some(source)) = (buf.get_mut(..count), available.get(..count))
                {
                    target.copy_from_slice(source);
                }
                Ok(count)
            }
        }
    }
}

#[cfg(unix)]
fn file_read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn file_read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(not(any(unix, windows)))]
fn file_read_at(_file: &File, _buf: &mut [u8], _offset: u64) -> io::Result<usize> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Reader that serves small reads from cached blocks and refuses reads past
/// the read boundary.
#[derive(Debug)]
pub(super) struct BlockReader {
    source: Source,
    limit: u64,
    blocks: HashMap<u64, Vec<u8>>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl BlockReader {
    /// Reader of `source` that caches at most `cache_bytes` (at least two
    /// blocks).
    pub(super) fn new(source: Source, cache_bytes: u64) -> io::Result<Self> {
        let limit = source.len()?;
        let blocks = (cache_bytes / BLOCK_SIZE).max(MIN_CACHED_BLOCKS);
        Ok(Self {
            source,
            limit,
            blocks: HashMap::new(),
            order: VecDeque::new(),
            capacity: usize::try_from(blocks).unwrap_or(usize::MAX),
        })
    }

    /// Read boundary: no byte at or after it is read.
    pub(super) const fn limit(&self) -> u64 {
        self.limit
    }

    /// Lowers the read boundary; a higher value is ignored.
    pub(super) fn set_limit(&mut self, limit: u64) {
        if limit < self.limit {
            self.limit = limit;
            self.blocks.clear();
            self.order.clear();
        }
    }

    /// Reads `len` bytes at `offset`.
    pub(super) fn read(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Fault> {
        let end = offset.checked_add(len).ok_or(Fault::Corrupt)?;
        if end > self.limit {
            return Err(Fault::Beyond);
        }
        let size = usize::try_from(len).map_err(|_| Fault::Corrupt)?;
        if len > BLOCK_SIZE {
            let mut out = vec![0_u8; size];
            self.read_exact_at(&mut out, offset)?;
            return Ok(out);
        }
        let mut out = Vec::with_capacity(size);
        let mut position = offset;
        while position < end {
            let index = position / BLOCK_SIZE;
            let block_start = index * BLOCK_SIZE;
            let block = self.block(index)?;
            let from = usize::try_from(position - block_start).map_err(|_| Fault::Corrupt)?;
            let to = usize::try_from(end.min(block_start + BLOCK_SIZE) - block_start)
                .map_err(|_| Fault::Corrupt)?;
            out.extend_from_slice(block.get(from..to).ok_or(Fault::Truncated)?);
            position = block_start + to as u64;
        }
        Ok(out)
    }

    /// Reads a little-endian 64-bit value.
    pub(super) fn read_u64(&mut self, offset: u64) -> Result<u64, Fault> {
        let bytes = self.read(offset, 8)?;
        let array: [u8; 8] = bytes.try_into().map_err(|_| Fault::Truncated)?;
        Ok(u64::from_le_bytes(array))
    }

    /// Reads a little-endian 32-bit value.
    pub(super) fn read_u32(&mut self, offset: u64) -> Result<u32, Fault> {
        let bytes = self.read(offset, 4)?;
        let array: [u8; 4] = bytes.try_into().map_err(|_| Fault::Truncated)?;
        Ok(u32::from_le_bytes(array))
    }

    fn block(&mut self, index: u64) -> Result<&Vec<u8>, Fault> {
        if self.blocks.contains_key(&index) {
            if let Some(position) = self.order.iter().position(|cached| *cached == index) {
                self.order.remove(position);
            }
        } else {
            let start = index * BLOCK_SIZE;
            let len = BLOCK_SIZE.min(self.limit.saturating_sub(start));
            let mut data = vec![0_u8; usize::try_from(len).map_err(|_| Fault::Corrupt)?];
            self.read_exact_at(&mut data, start)?;
            if self.order.len() >= self.capacity
                && let Some(evicted) = self.order.pop_front()
            {
                self.blocks.remove(&evicted);
            }
            self.blocks.insert(index, data);
        }
        self.order.push_back(index);
        self.blocks.get(&index).ok_or(Fault::Corrupt)
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<(), Fault> {
        let mut done = 0;
        while done < buf.len() {
            let target = buf.get_mut(done..).ok_or(Fault::Corrupt)?;
            let position = offset.checked_add(done as u64).ok_or(Fault::Corrupt)?;
            match self.source.read_at(target, position) {
                Ok(0) => return Err(Fault::Truncated),
                Ok(count) => done += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(len: usize, cache: u64) -> BlockReader {
        let bytes: Vec<u8> = (0..len).map(|index| (index % 251) as u8).collect();
        BlockReader::new(Source::Memory(bytes), cache).unwrap()
    }

    #[test]
    fn reads_across_block_boundaries() {
        let mut reader = reader(3 * BLOCK_SIZE as usize, 0);
        let offset = BLOCK_SIZE - 3;
        let bytes = reader.read(offset, 6).unwrap();
        let expected: Vec<u8> = (offset..offset + 6)
            .map(|index| (index % 251) as u8)
            .collect();
        assert_eq!(bytes, expected);
        assert_eq!(
            reader.read_u32(0).unwrap(),
            u32::from_le_bytes([0, 1, 2, 3])
        );
        assert_eq!(
            reader.read_u64(8).unwrap(),
            u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15])
        );
    }

    #[test]
    fn large_reads_bypass_the_cache() {
        let mut reader = reader(3 * BLOCK_SIZE as usize, 0);
        let bytes = reader.read(1, BLOCK_SIZE + 10).unwrap();
        assert_eq!(bytes.len() as u64, BLOCK_SIZE + 10);
        assert!(reader.blocks.is_empty());
    }

    #[test]
    fn cache_keeps_at_most_capacity_blocks() {
        let mut reader = reader(8 * BLOCK_SIZE as usize, 3 * BLOCK_SIZE);
        for index in 0..8 {
            reader.read(index * BLOCK_SIZE, 1).unwrap();
        }
        reader.read(7 * BLOCK_SIZE, 1).unwrap();
        assert_eq!(reader.blocks.len(), 3);
        assert_eq!(reader.order.back(), Some(&7));
    }

    #[test]
    fn refuses_reads_beyond_limit() {
        let mut reader = reader(100, 0);
        assert!(matches!(reader.read(96, 8), Err(Fault::Beyond)));
        assert!(matches!(reader.read(u64::MAX, 2), Err(Fault::Corrupt)));
        reader.set_limit(50);
        reader.set_limit(80);
        assert_eq!(reader.limit(), 50);
        assert!(matches!(reader.read(48, 4), Err(Fault::Beyond)));
    }

    #[test]
    fn reads_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        std::fs::write(&path, b"0123456789").unwrap();
        let file = File::open(&path).unwrap();
        let mut reader = BlockReader::new(Source::File(file), 0).unwrap();
        assert_eq!(reader.read(2, 3).unwrap(), b"234");
    }
}
