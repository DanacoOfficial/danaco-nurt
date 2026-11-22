// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Hexadecimal dump of content in the layout of `xxd`.
//!
//! Each row shows the offset of its first byte as at least eight lowercase
//! hexadecimal digits, a colon, sixteen bytes in groups of two and the same
//! bytes as ASCII, with `.` in place of bytes outside the printable range:
//!
//! ```text
//! 00000000: 4865 6c6c 6f2c 2077 6f72 6c64 210a       Hello, world!.
//! ```
//!
//! Rows are produced from a reader one at a time, so content of any size is
//! dumped in constant memory. Row numbers start at zero; row `n` begins at
//! byte offset `16 * n`.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

/// Number of bytes shown in one row.
pub const BYTES_PER_ROW: usize = 16;

const ROW_BYTES: u64 = BYTES_PER_ROW as u64;
const GROUP_BYTES: usize = 2;
const OFFSET_DIGITS: usize = 8;
const HEX_COLUMN_WIDTH: usize = BYTES_PER_ROW * 2 + BYTES_PER_ROW / GROUP_BYTES - 1;
const PRINTABLE: std::ops::RangeInclusive<u8> = b' '..=b'~';

/// Number of rows in the dump of content of `len` bytes.
///
/// # Examples
///
/// ```
/// use nurt_files::read::hex::row_count;
///
/// assert_eq!(row_count(0), 0);
/// assert_eq!(row_count(16), 1);
/// assert_eq!(row_count(17), 2);
/// ```
pub fn row_count(len: u64) -> u64 {
    len.div_ceil(ROW_BYTES)
}

/// One row of a dump: up to [`BYTES_PER_ROW`] bytes and their offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    offset: u64,
    bytes: [u8; BYTES_PER_ROW],
    len: usize,
}

impl HexRow {
    /// Offset of the first byte of the row in the content.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Bytes of the row; fewer than [`BYTES_PER_ROW`] only in the last row.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Formats the row as `xxd` does, without a line terminator.
impl fmt::Display for HexRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:0width$x}: ", self.offset, width = OFFSET_DIGITS)?;
        let mut written = 0;
        for (index, byte) in self.bytes().iter().enumerate() {
            if index > 0 && index % GROUP_BYTES == 0 {
                f.write_str(" ")?;
                written += 1;
            }
            write!(f, "{byte:02x}")?;
            written += 2;
        }
        write!(f, "{:pad$}  ", "", pad = HEX_COLUMN_WIDTH - written)?;
        for &byte in self.bytes() {
            let shown = if PRINTABLE.contains(&byte) {
                char::from(byte)
            } else {
                '.'
            };
            write!(f, "{shown}")?;
        }
        Ok(())
    }
}

/// Iterator over the rows of a dump of a reader.
///
/// The iterator ends after the last byte of the reader or after the first
/// error. Combine it with [`Iterator::take`] to dump a range of rows.
///
/// # Examples
///
/// ```
/// use nurt_files::read::hex::HexRows;
///
/// let rows: Vec<String> = HexRows::new(&b"Hello, world!\n"[..])
///     .map(|row| row.map(|row| row.to_string()))
///     .collect::<Result<_, _>>()?;
/// assert_eq!(
///     rows,
///     ["00000000: 4865 6c6c 6f2c 2077 6f72 6c64 210a       Hello, world!."]
/// );
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct HexRows<R> {
    reader: R,
    offset: u64,
    finished: bool,
}

impl<R: Read> HexRows<R> {
    /// Dumps a reader from its current position, which is shown as offset 0.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            offset: 0,
            finished: false,
        }
    }

    /// Dumps a reader from row `row`, reading and discarding the bytes of the
    /// rows before it.
    ///
    /// # Errors
    ///
    /// Returns the error of the reader while skipping.
    pub fn starting_at_row(mut reader: R, row: u64) -> io::Result<Self> {
        let offset = row.saturating_mul(ROW_BYTES);
        let skipped = io::copy(&mut (&mut reader).take(offset), &mut io::sink())?;
        Ok(Self {
            reader,
            offset: skipped,
            finished: skipped < offset,
        })
    }

    fn fill(&mut self, buffer: &mut [u8; BYTES_PER_ROW]) -> io::Result<usize> {
        let mut filled = 0;
        while filled < BYTES_PER_ROW {
            match self.reader.read(&mut buffer[filled..]) {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(filled)
    }
}

impl<R: Read + Seek> HexRows<R> {
    /// Dumps a seekable reader from row `row`, counted from the start of the
    /// content, without reading the rows before it.
    ///
    /// # Errors
    ///
    /// Returns the error of the reader while seeking.
    pub fn seek_to_row(mut reader: R, row: u64) -> io::Result<Self> {
        let len = reader.seek(SeekFrom::End(0))?;
        let offset = row.saturating_mul(ROW_BYTES).min(len);
        reader.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            reader,
            offset,
            finished: offset == len,
        })
    }
}

impl<R: Read> Iterator for HexRows<R> {
    type Item = io::Result<HexRow>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let mut bytes = [0; BYTES_PER_ROW];
        match self.fill(&mut bytes) {
            Ok(0) => {
                self.finished = true;
                None
            }
            Ok(len) => {
                let row = HexRow {
                    offset: self.offset,
                    bytes,
                    len,
                };
                self.offset += len as u64;
                self.finished = len < BYTES_PER_ROW;
                Some(Ok(row))
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn dump(bytes: &[u8]) -> Vec<String> {
        HexRows::new(bytes)
            .map(|row| row.unwrap().to_string())
            .collect()
    }

    #[test]
    fn full_row_matches_xxd() {
        assert_eq!(
            dump(b"#include <stdio.h>"),
            [
                "00000000: 2369 6e63 6c75 6465 203c 7374 6469 6f2e  #include <stdio.",
                "00000010: 683e                                     h>",
            ]
        );
    }

    #[test]
    fn odd_length_row_pads_ascii_column() {
        assert_eq!(
            dump(b"abc"),
            ["00000000: 6162 63                                  abc"]
        );
    }

    #[test]
    fn bytes_outside_printable_range_show_as_dots() {
        assert_eq!(
            dump(&[0x00, 0x1f, 0x20, 0x7e, 0x7f, 0xff]),
            ["00000000: 001f 207e 7fff                           .. ~.."]
        );
    }

    #[test]
    fn empty_content_has_no_rows() {
        assert!(dump(b"").is_empty());
        assert_eq!(row_count(0), 0);
    }

    #[test]
    fn offsets_wider_than_eight_digits_grow() {
        let row = HexRow {
            offset: 0x1_0000_0000,
            bytes: [b'a'; BYTES_PER_ROW],
            len: 1,
        };
        assert!(row.to_string().starts_with("100000000: 61 "));
    }

    #[test]
    fn starting_row_skips_by_reading() {
        let content: Vec<u8> = (0..=255).collect();
        let rows: Vec<HexRow> = HexRows::starting_at_row(&content[..], 14)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].offset(), 0xe0);
        assert_eq!(rows[1].bytes()[15], 0xff);
    }

    #[test]
    fn starting_row_past_end_yields_nothing() {
        let mut rows = HexRows::starting_at_row(&b"short"[..], 3).unwrap();
        assert!(rows.next().is_none());
    }

    #[test]
    fn seek_to_row_matches_reading_skip() {
        let content: Vec<u8> = (0..100).collect();
        let sought: Vec<String> = HexRows::seek_to_row(Cursor::new(&content), 2)
            .unwrap()
            .take(2)
            .map(|row| row.unwrap().to_string())
            .collect();
        let read: Vec<String> = HexRows::starting_at_row(&content[..], 2)
            .unwrap()
            .take(2)
            .map(|row| row.unwrap().to_string())
            .collect();
        assert_eq!(sought, read);
        assert!(sought[0].starts_with("00000020: 2021 "));
        assert!(
            HexRows::seek_to_row(Cursor::new(&content), 7)
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn short_reads_still_fill_rows() {
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let Some((&first, rest)) = self.0.split_first() else {
                    return Ok(0);
                };
                buf[0] = first;
                self.0 = rest;
                Ok(1)
            }
        }
        let rows: Vec<HexRow> = HexRows::new(OneByte(&[7; 20]))
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows.iter().map(|row| row.bytes().len()).collect::<Vec<_>>(),
            [16, 4]
        );
    }

    #[test]
    fn reader_error_ends_iteration() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("device error"))
            }
        }
        let mut rows = HexRows::new(Failing);
        assert!(rows.next().unwrap().is_err());
        assert!(rows.next().is_none());
    }

    #[test]
    fn row_count_rounds_up() {
        assert_eq!(row_count(1), 1);
        assert_eq!(row_count(32), 2);
        assert_eq!(row_count(33), 3);
    }
}
