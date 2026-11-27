// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::output::cursor
// Responsibility:   Read positions in spool files, output cursors and reading of complete.
// Dependencies:     nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Read positions in spool files, output cursors and reading of complete
//! lines from a position.
//!
//! A cursor is the text `c_<spool>.<out-offset>.<out-line>.<err-offset>.<err-line>`:
//! a short hash of the spool name, then the byte offset and the line number
//! after the last line returned from each stream. Merged output uses the
//! first pair. Offsets always point to the start of a line, so reading from
//! a cursor neither repeats nor skips lines while the program keeps writing.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::OutputStream;
use sha2::{Digest, Sha256};

use super::decode::LineUnit;

/// Prefix of every output cursor.
const PREFIX: &str = "c_";

/// Size of the read buffer.
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Bytes of one line kept for a response; longer lines are shortened.
pub(super) const MAX_LINE_BYTES: usize = 64 * 1024;

/// Position after a number of complete lines of one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Position {
    /// Byte offset in the spool file.
    pub(super) offset: u64,
    /// Lines before the offset.
    pub(super) line: u64,
}

/// Decoded output cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Cursor {
    /// Position in standard output or in the merged output.
    pub(super) stdout: Position,
    /// Position in standard error.
    pub(super) stderr: Position,
}

/// Short hash of a spool name, which ties a cursor to its spool.
pub(super) fn spool_tag(id: &str) -> String {
    const TAG_BYTES: usize = 4;
    Sha256::digest(id.as_bytes())
        .iter()
        .take(TAG_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Cursor {
    /// Position in `stream`; merged output uses the standard output position.
    pub(super) fn position(&self, stream: OutputStream) -> Position {
        match stream {
            OutputStream::Stdout => self.stdout,
            OutputStream::Stderr => self.stderr,
        }
    }

    /// Moves the position in `stream`.
    pub(super) fn set_position(&mut self, stream: OutputStream, position: Position) {
        match stream {
            OutputStream::Stdout => self.stdout = position,
            OutputStream::Stderr => self.stderr = position,
        }
    }

    /// Text of the cursor for the spool with the tag `tag`.
    pub(super) fn encode(&self, tag: &str) -> String {
        format!(
            "{PREFIX}{tag}.{}.{}.{}.{}",
            self.stdout.offset, self.stdout.line, self.stderr.offset, self.stderr.line
        )
    }

    /// Reads a cursor issued for the spool with the tag `tag`.
    ///
    /// # Errors
    ///
    /// `E_INVALID_VALUE` when the text is not a cursor of this spool.
    pub(super) fn parse(text: &str, tag: &str) -> Result<Self> {
        let invalid = || {
            Error::new(
                ErrorCode::InvalidValue,
                "since: not an output cursor of this job; pass output_cursor of an earlier result",
            )
            .with_found(text)
        };
        let body = text
            .trim()
            .strip_prefix(PREFIX)
            .and_then(|body| body.strip_prefix(tag))
            .and_then(|body| body.strip_prefix('.'))
            .ok_or_else(invalid)?;
        let numbers = body
            .split('.')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| invalid())?;
        match numbers[..] {
            [out_offset, out_line, err_offset, err_line] => Ok(Self {
                stdout: Position {
                    offset: out_offset,
                    line: out_line,
                },
                stderr: Position {
                    offset: err_offset,
                    line: err_line,
                },
            }),
            _ => Err(invalid()),
        }
    }
}

/// One line read from a spool file, without its line feed.
#[derive(Debug, Default)]
pub(super) struct RawLine {
    /// Bytes of the line, up to [`MAX_LINE_BYTES`].
    pub(super) bytes: Vec<u8>,
    /// The line was longer than [`MAX_LINE_BYTES`].
    pub(super) shortened: bool,
}

/// Reads complete lines of one spool file from a position.
///
/// A last line without a line feed is returned only when the stream has
/// ended; while the program runs it may still grow.
pub(super) struct LineReader {
    file: File,
    unit: LineUnit,
    ended: bool,
    position: Position,
    buffer: Vec<u8>,
    start: usize,
    end: usize,
    eof: bool,
}

impl LineReader {
    /// Opens `path` at `position`.
    ///
    /// # Errors
    ///
    /// The I/O error of opening or positioning the file.
    pub(super) fn open(
        path: &Path,
        position: Position,
        unit: LineUnit,
        ended: bool,
    ) -> io::Result<Self> {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(position.offset))?;
        Ok(Self {
            file,
            unit,
            ended,
            position,
            buffer: vec![0; READ_BUFFER_BYTES],
            start: 0,
            end: 0,
            eof: false,
        })
    }

    /// Position after the last line returned.
    pub(super) fn position(&self) -> Position {
        self.position
    }

    /// The next complete line; with `keep` false the bytes are skipped and
    /// only the position advances.
    ///
    /// # Errors
    ///
    /// The I/O error of reading the file.
    pub(super) fn next_line(&mut self, keep: bool) -> io::Result<Option<RawLine>> {
        let mut line = RawLine::default();
        let mut consumed: u64 = 0;
        loop {
            self.fill()?;
            let window = &self.buffer[self.start..self.end];
            if window.is_empty() {
                if consumed > 0 && self.ended {
                    self.advance(consumed);
                    return Ok(Some(line));
                }
                return Ok(None);
            }
            let (take, found) = match find_line_end(window, self.unit) {
                Some(index) => (index + unit_width(self.unit), Some(index)),
                None => (usable_length(window.len(), self.unit, self.eof), None),
            };
            if keep {
                let content = found.unwrap_or(take);
                append(&mut line, &window[..content]);
            }
            self.start += take;
            consumed += take as u64;
            if found.is_some() {
                self.advance(consumed);
                return Ok(Some(line));
            }
        }
    }

    fn advance(&mut self, bytes: u64) {
        self.position.offset += bytes;
        self.position.line += 1;
    }

    /// Makes the buffer hold at least one whole line unit unless the file
    /// ends first.
    fn fill(&mut self) -> io::Result<()> {
        let needed = unit_width(self.unit);
        if self.end - self.start >= needed || self.eof {
            return Ok(());
        }
        self.buffer.copy_within(self.start..self.end, 0);
        self.end -= self.start;
        self.start = 0;
        while self.end < needed && !self.eof {
            let read = self.file.read(&mut self.buffer[self.end..])?;
            if read == 0 {
                self.eof = true;
            }
            self.end += read;
        }
        Ok(())
    }
}

fn unit_width(unit: LineUnit) -> usize {
    match unit {
        LineUnit::Byte => 1,
        LineUnit::Utf16Le | LineUnit::Utf16Be => 2,
    }
}

/// Bytes of `window` that can be taken without splitting a line unit.
fn usable_length(length: usize, unit: LineUnit, eof: bool) -> usize {
    if unit == LineUnit::Byte || eof {
        length
    } else {
        length - length % 2
    }
}

/// Index of the line feed in `window`, which starts at a unit boundary.
fn find_line_end(window: &[u8], unit: LineUnit) -> Option<usize> {
    match unit {
        LineUnit::Byte => memchr::memchr(b'\n', window),
        LineUnit::Utf16Le => window
            .as_chunks::<2>()
            .0
            .iter()
            .position(|pair| *pair == [b'\n', 0])
            .map(|pair| pair * 2),
        LineUnit::Utf16Be => window
            .as_chunks::<2>()
            .0
            .iter()
            .position(|pair| *pair == [0, b'\n'])
            .map(|pair| pair * 2),
    }
}

fn append(line: &mut RawLine, bytes: &[u8]) {
    let room = MAX_LINE_BYTES.saturating_sub(line.bytes.len());
    if bytes.len() > room {
        line.shortened = true;
    }
    line.bytes
        .extend_from_slice(&bytes[..bytes.len().min(room)]);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn lines(path: &Path, from: Position, unit: LineUnit, ended: bool) -> (Vec<String>, Position) {
        let mut reader = LineReader::open(path, from, unit, ended).unwrap();
        let mut out = Vec::new();
        while let Some(line) = reader.next_line(true).unwrap() {
            out.push(String::from_utf8_lossy(&line.bytes).into_owned());
        }
        (out, reader.position())
    }

    #[test]
    fn round_trips_cursor_text() {
        let tag = spool_tag("j7");
        let cursor = Cursor {
            stdout: Position {
                offset: 120,
                line: 4,
            },
            stderr: Position { offset: 9, line: 1 },
        };
        let text = cursor.encode(&tag);
        assert!(text.starts_with("c_"));
        assert_eq!(Cursor::parse(&text, &tag).unwrap(), cursor);
    }

    #[test]
    fn rejects_cursor_of_other_spool_or_malformed() {
        let text = Cursor {
            stdout: Position::default(),
            stderr: Position::default(),
        }
        .encode(&spool_tag("j1"));
        for bad in [
            text.as_str(),
            "c_",
            "x",
            &format!("c_{}.1.2.3", spool_tag("j2")),
            &format!("c_{}.1.2.3.x", spool_tag("j2")),
        ] {
            let error = Cursor::parse(bad, &spool_tag("j2")).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{bad}");
        }
    }

    #[test]
    fn holds_back_unterminated_line_until_stream_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        std::fs::write(&path, b"one\ntwo\nthr").unwrap();
        let (running, position) = lines(&path, Position::default(), LineUnit::Byte, false);
        assert_eq!(running, ["one", "two"]);
        assert_eq!(position, Position { offset: 8, line: 2 });
        let (ended, end) = lines(&path, position, LineUnit::Byte, true);
        assert_eq!(ended, ["thr"]);
        assert_eq!(
            end,
            Position {
                offset: 11,
                line: 3
            }
        );
    }

    #[test]
    fn splits_utf16_lines_across_buffer_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        let long = "x".repeat(READ_BUFFER_BYTES / 2 - 1);
        let text = format!("{long}\nshort\n\u{a0a}tail");
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        std::fs::write(&path, &bytes).unwrap();
        let (out, end) = lines(&path, Position::default(), LineUnit::Utf16Le, true);
        assert_eq!(out.len(), 3);
        assert_eq!(end.offset, bytes.len() as u64);
        let be: Vec<u8> = "a\nb".encode_utf16().flat_map(u16::to_be_bytes).collect();
        std::fs::write(&path, &be).unwrap();
        let (_, end) = lines(&path, Position::default(), LineUnit::Utf16Be, false);
        assert_eq!(end, Position { offset: 4, line: 1 });
    }

    #[test]
    fn shortens_long_lines_and_skips_without_keeping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        let mut file = File::create(&path).unwrap();
        file.write_all(&vec![b'a'; MAX_LINE_BYTES + 10]).unwrap();
        file.write_all(b"\nb\n").unwrap();
        let mut reader =
            LineReader::open(&path, Position::default(), LineUnit::Byte, true).unwrap();
        let first = reader.next_line(true).unwrap().unwrap();
        assert!(first.shortened);
        assert_eq!(first.bytes.len(), MAX_LINE_BYTES);
        let second = reader.next_line(false).unwrap().unwrap();
        assert!(second.bytes.is_empty());
        assert!(reader.next_line(true).unwrap().is_none());
        assert_eq!(reader.position().line, 2);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
