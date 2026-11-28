// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::sources::rpm::header
// Responsibility:   Parser of RPM headers as stored in package databases.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of RPM headers as stored in package databases.
//!
//! A stored header has no lead and no magic: a big-endian count of index
//! entries, the length of the data store, the index entries of 16 bytes
//! (`tag`, `type`, `offset`, `count`) and the data store. Every offset and
//! count is checked against the data store; a header that violates a bound
//! is rejected as a whole.

use crate::model::LocalizedText;

/// Tag of the table of locales of `I18NSTRING` values.
pub const TAG_I18N_TABLE: u32 = 100;
/// Tag `NAME`.
pub const TAG_NAME: u32 = 1000;
/// Tag `VERSION`.
pub const TAG_VERSION: u32 = 1001;
/// Tag `RELEASE`.
pub const TAG_RELEASE: u32 = 1002;
/// Tag `EPOCH`.
pub const TAG_EPOCH: u32 = 1003;
/// Tag `SUMMARY`.
pub const TAG_SUMMARY: u32 = 1004;
/// Tag `DESCRIPTION`.
pub const TAG_DESCRIPTION: u32 = 1005;
/// Tag `INSTALLTIME`.
pub const TAG_INSTALL_TIME: u32 = 1008;
/// Tag `SIZE`.
pub const TAG_SIZE: u32 = 1009;
/// Tag `LICENSE`.
pub const TAG_LICENSE: u32 = 1014;
/// Tag `GROUP`.
pub const TAG_GROUP: u32 = 1016;
/// Tag `URL`.
pub const TAG_URL: u32 = 1020;
/// Tag `ARCH`.
pub const TAG_ARCH: u32 = 1022;
/// Tag `SOURCERPM`.
pub const TAG_SOURCE_RPM: u32 = 1044;
/// Tag `DIRINDEXES`.
pub const TAG_DIR_INDEXES: u32 = 1116;
/// Tag `BASENAMES`.
pub const TAG_BASENAMES: u32 = 1117;
/// Tag `DIRNAMES`.
pub const TAG_DIRNAMES: u32 = 1118;
/// Tag `LONGSIZE`.
pub const TAG_LONG_SIZE: u32 = 5009;

/// Type `CHAR`.
pub const TYPE_CHAR: u32 = 1;
/// Type `INT8`.
pub const TYPE_INT8: u32 = 2;
/// Type `INT16`.
pub const TYPE_INT16: u32 = 3;
/// Type `INT32`.
pub const TYPE_INT32: u32 = 4;
/// Type `INT64`.
pub const TYPE_INT64: u32 = 5;
/// Type `STRING`.
pub const TYPE_STRING: u32 = 6;
/// Type `BIN`.
pub const TYPE_BIN: u32 = 7;
/// Type `STRING_ARRAY`.
pub const TYPE_STRING_ARRAY: u32 = 8;
/// Type `I18NSTRING`.
pub const TYPE_I18N_STRING: u32 = 9;

/// Largest number of index entries accepted.
pub const MAX_ENTRIES: u32 = 0xffff;
/// Largest data store accepted, in bytes.
pub const MAX_DATA: u32 = 256 * 1024 * 1024;
/// Size of one index entry.
pub const ENTRY_SIZE: usize = 16;
/// Size of the two leading counts.
const PREAMBLE: usize = 8;

/// One index entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    /// Tag.
    pub tag: u32,
    /// Data type.
    pub kind: u32,
    /// Offset in the data store.
    pub offset: u32,
    /// Number of values.
    pub count: u32,
}

/// Parsed header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    entries: Vec<IndexEntry>,
    data: Vec<u8>,
}

/// Why a header was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HeaderError(pub String);

fn error(message: &str) -> HeaderError {
    HeaderError(message.to_owned())
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

impl Header {
    /// Parses a stored header.
    ///
    /// # Errors
    ///
    /// Fails when the counts exceed the limits or the blob, or an index
    /// entry points outside the data store.
    pub fn parse(blob: &[u8]) -> Result<Self, HeaderError> {
        let count = be32(blob, 0).ok_or_else(|| error("header shorter than its preamble"))?;
        let length = be32(blob, 4).ok_or_else(|| error("header shorter than its preamble"))?;
        if count == 0 || count > MAX_ENTRIES || length > MAX_DATA {
            return Err(error("header counts out of range"));
        }
        let index_end = PREAMBLE + count as usize * ENTRY_SIZE;
        let data_end = index_end + length as usize;
        let data = blob
            .get(index_end..data_end)
            .ok_or_else(|| error("header data store exceeds the blob"))?;
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            let at = PREAMBLE + index * ENTRY_SIZE;
            let entry = IndexEntry {
                tag: be32(blob, at).unwrap_or_default(),
                kind: be32(blob, at + 4).unwrap_or_default(),
                offset: be32(blob, at + 8).unwrap_or_default(),
                count: be32(blob, at + 12).unwrap_or_default(),
            };
            if entry.offset >= length && entry.count > 0 {
                return Err(error("index entry points outside the data store"));
            }
            entries.push(entry);
        }
        Ok(Self {
            entries,
            data: data.to_vec(),
        })
    }

    /// Index entries in stored order.
    #[must_use]
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    fn entry(&self, tag: u32) -> Option<&IndexEntry> {
        self.entries.iter().find(|entry| entry.tag == tag)
    }

    /// Strings of a `STRING`, `STRING_ARRAY` or `I18NSTRING` entry; `None`
    /// when the tag is absent, has another type or its strings are not
    /// terminated within the data store.
    #[must_use]
    pub fn strings(&self, tag: u32) -> Option<Vec<String>> {
        let entry = self.entry(tag)?;
        let wanted = match entry.kind {
            TYPE_STRING => 1,
            TYPE_STRING_ARRAY | TYPE_I18N_STRING => entry.count as usize,
            _ => return None,
        };
        let mut at = entry.offset as usize;
        let mut out = Vec::with_capacity(wanted.min(self.data.len()));
        for _ in 0..wanted {
            let rest = self.data.get(at..)?;
            let end = memchr::memchr(0, rest)?;
            out.push(String::from_utf8_lossy(&rest[..end]).into_owned());
            at += end + 1;
        }
        Some(out)
    }

    /// First string of a string entry.
    #[must_use]
    pub fn string(&self, tag: u32) -> Option<String> {
        self.strings(tag)?
            .into_iter()
            .next()
            .filter(|text| !text.is_empty())
    }

    /// Integers of an `INT8`, `INT16`, `INT32` or `INT64` entry.
    #[must_use]
    pub fn integers(&self, tag: u32) -> Option<Vec<u64>> {
        let entry = self.entry(tag)?;
        let width: usize = match entry.kind {
            TYPE_CHAR | TYPE_INT8 => 1,
            TYPE_INT16 => 2,
            TYPE_INT32 => 4,
            TYPE_INT64 => 8,
            _ => return None,
        };
        let start = entry.offset as usize;
        let end = start.checked_add(width.checked_mul(entry.count as usize)?)?;
        let bytes = self.data.get(start..end)?;
        Some(
            bytes
                .chunks_exact(width)
                .map(|chunk| {
                    chunk
                        .iter()
                        .fold(0u64, |value, byte| value << 8 | u64::from(*byte))
                })
                .collect(),
        )
    }

    /// First integer of an integer entry.
    #[must_use]
    pub fn integer(&self, tag: u32) -> Option<u64> {
        self.integers(tag)?.first().copied()
    }

    /// Text of an `I18NSTRING` entry with its translations; the locale `C`
    /// is English.
    #[must_use]
    pub fn localized(&self, tag: u32) -> Option<LocalizedText> {
        let values = self.strings(tag)?;
        let locales = self.strings(TAG_I18N_TABLE).unwrap_or_default();
        let mut text: Option<LocalizedText> = None;
        for (index, value) in values.into_iter().enumerate() {
            if value.is_empty() {
                continue;
            }
            let locale = locales.get(index).map_or("C", String::as_str);
            let lang = if locale == "C" || locale.is_empty() {
                "en".to_owned()
            } else {
                locale
                    .split(['_', '.', '@'])
                    .next()
                    .unwrap_or(locale)
                    .to_owned()
            };
            match text.as_mut() {
                None => text = Some(LocalizedText::in_lang(value, &lang)),
                Some(text) => text.set(&lang, value),
            }
        }
        text
    }

    /// Full paths of the files of the package, from `DIRNAMES`,
    /// `DIRINDEXES` and `BASENAMES`; entries with an index out of range are
    /// skipped.
    #[must_use]
    pub fn file_paths(&self) -> Vec<String> {
        let (Some(dirs), Some(indexes), Some(names)) = (
            self.strings(TAG_DIRNAMES),
            self.integers(TAG_DIR_INDEXES),
            self.strings(TAG_BASENAMES),
        ) else {
            return Vec::new();
        };
        names
            .iter()
            .zip(indexes)
            .filter_map(|(name, index)| {
                let dir = dirs.get(usize::try_from(index).ok()?)?;
                Some(format!("{dir}{name}"))
            })
            .collect()
    }

    /// Version as rpm prints it: `[epoch:]version-release`.
    #[must_use]
    pub fn full_version(&self) -> Option<String> {
        let version = self.string(TAG_VERSION)?;
        let mut text = match self.integer(TAG_EPOCH) {
            Some(epoch) => format!("{epoch}:{version}"),
            None => version,
        };
        if let Some(release) = self.string(TAG_RELEASE) {
            text.push('-');
            text.push_str(&release);
        }
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(entries: &[(u32, u32, u32, u32)], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((entries.len() as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        for (tag, kind, offset, count) in entries {
            for value in [tag, kind, offset, count] {
                out.extend(value.to_be_bytes());
            }
        }
        out.extend(data);
        out
    }

    #[test]
    fn parses_strings_and_integers() {
        let data = b"bash\0\0\0\0\0\0\0\x05";
        let header = Header::parse(&blob(
            &[(TAG_NAME, TYPE_STRING, 0, 1), (TAG_EPOCH, TYPE_INT32, 8, 1)],
            data,
        ))
        .unwrap();
        assert_eq!(header.string(TAG_NAME).as_deref(), Some("bash"));
        assert_eq!(header.integer(TAG_EPOCH), Some(5));
        assert_eq!(header.entries().len(), 2);
        assert_eq!(header.string(TAG_VERSION), None);
        assert_eq!(header.integers(TAG_NAME), None);
    }

    #[test]
    fn rejects_bad_bounds() {
        assert!(Header::parse(&[0, 0]).is_err());
        assert!(Header::parse(&blob(&[], b"")).is_err());
        assert!(Header::parse(&blob(&[(TAG_NAME, TYPE_STRING, 9, 1)], b"abc\0")).is_err());
        let mut short = blob(&[(TAG_NAME, TYPE_STRING, 0, 1)], b"abc\0");
        short.truncate(short.len() - 1);
        assert!(Header::parse(&short).is_err());
    }

    #[test]
    fn unterminated_string_is_absent() {
        let header = Header::parse(&blob(&[(TAG_NAME, TYPE_STRING, 0, 1)], b"abc")).unwrap();
        assert_eq!(header.string(TAG_NAME), None);
    }

    #[test]
    fn integer_entry_beyond_data_is_absent() {
        let header = Header::parse(&blob(&[(TAG_EPOCH, TYPE_INT32, 0, 4)], b"\0\0\0\x01")).unwrap();
        assert_eq!(header.integers(TAG_EPOCH), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
