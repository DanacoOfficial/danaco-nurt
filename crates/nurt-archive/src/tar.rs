// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-archive::tar
// Responsibility:   Reading and writing tar archives, plain or inside a compressed stream.
// Dependencies:     nurt_contracts, tar
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reading and writing tar archives, plain or inside a compressed stream.
//!
//! Long names and link targets are read from GNU and PAX extension headers;
//! a PAX `mtime` record takes precedence over the header time. New archives
//! use GNU headers with user and group identifiers 0 and no account names,
//! so the output depends only on the inputs.

use std::borrow::Cow;
use std::io::{self, Read, Write};

use ::tar::{Archive, Builder, Entry, EntryType, Header};
use nurt_contracts::value::Instant;

use crate::codec::{self, Algorithm};
use crate::create::{Input, InputKind};
use crate::list::{ArchiveError, Body, Flow, Format, Member, MemberKind, Visitor};

const PERMISSION_BITS: u32 = 0o7777;
const DEFAULT_FILE_MODE: u32 = 0o644;
const DEFAULT_DIRECTORY_MODE: u32 = 0o755;
const DEFAULT_SYMLINK_MODE: u32 = 0o777;
const PAX_MTIME: &str = "mtime";
const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;
const NANOSECOND_DIGITS: usize = 9;
const DECIMAL_RADIX: i64 = 10;

/// Passes every member of a tar archive to the visitor in archive order.
pub(crate) fn visit<R: Read + Send>(
    reader: R,
    compression: Option<Algorithm>,
    visitor: &mut Visitor<'_>,
) -> Result<(), ArchiveError> {
    let format = Format::Tar(compression);
    match compression {
        None => visit_stream(reader, format, visitor),
        Some(algorithm) => visit_stream(codec::decoder(algorithm, reader)?, format, visitor),
    }
}

fn visit_stream<S: Read>(
    stream: S,
    format: Format,
    visitor: &mut Visitor<'_>,
) -> Result<(), ArchiveError> {
    let mut archive = Archive::new(stream);
    let entries = archive
        .entries()
        .map_err(|error| ArchiveError::from_read(format, error))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| ArchiveError::from_read(format, error))?;
        let Some(member) = read_member(&mut entry, format)? else {
            continue;
        };
        if visitor(&member, &mut TarBody(&mut entry))? == Flow::Stop {
            break;
        }
    }
    Ok(())
}

/// Member described by an entry; `None` for a global PAX header, which
/// carries no member.
fn read_member<S: Read>(
    entry: &mut Entry<'_, S>,
    format: Format,
) -> Result<Option<Member>, ArchiveError> {
    let header = entry.header();
    let path = entry.path_bytes();
    let kind = match header.entry_type() {
        EntryType::Regular if path.ends_with(b"/") => MemberKind::Directory,
        EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => MemberKind::File,
        EntryType::Directory => MemberKind::Directory,
        EntryType::Symlink => MemberKind::Symlink,
        EntryType::Link => MemberKind::HardLink,
        EntryType::XGlobalHeader => return Ok(None),
        _ => MemberKind::Other,
    };
    let mut member = Member::new(path, kind);
    // Some writers leave the mode or the time field blank; such a member
    // has no mode or time rather than making the archive unreadable.
    member.mode = header.mode().ok().map(|mode| mode & PERMISSION_BITS);
    let header_time = header
        .mtime()
        .ok()
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(Instant::from_unix_seconds);
    let read_error = |error| ArchiveError::from_read(format, error);
    if matches!(kind, MemberKind::Symlink | MemberKind::HardLink)
        && let Some(target) = entry.link_name_bytes()
    {
        member.set_link_target(Cow::Owned(target.into_owned()));
    }
    if kind == MemberKind::File {
        member.size = entry.size();
    }
    if let Format::Tar(Some(algorithm)) = format {
        member.compression = Some(algorithm.name().to_owned());
    }
    member.modified = pax_mtime(entry).map_err(read_error)?.or(header_time);
    Ok(Some(member))
}

fn pax_mtime<S: Read>(entry: &mut Entry<'_, S>) -> io::Result<Option<Instant>> {
    let Some(extensions) = entry.pax_extensions()? else {
        return Ok(None);
    };
    for extension in extensions {
        let extension = extension?;
        if extension.key() == Ok(PAX_MTIME) {
            return Ok(extension.value().ok().and_then(parse_pax_time));
        }
    }
    Ok(None)
}

/// Parses a PAX time, decimal seconds since the epoch with an optional
/// fraction (`1700000000.25`, `-1.5`).
fn parse_pax_time(text: &str) -> Option<Instant> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let seconds: i64 = whole.parse().ok()?;
    let digits = &fraction[..fraction.len().min(NANOSECOND_DIGITS)];
    let mut nanos: i64 = if digits.is_empty() {
        0
    } else {
        digits.parse().ok()?
    };
    for _ in digits.len()..NANOSECOND_DIGITS {
        nanos *= DECIMAL_RADIX;
    }
    let total = seconds
        .checked_mul(NANOSECONDS_PER_SECOND)?
        .checked_add(nanos)?;
    Some(Instant::from_unix_nanos(if negative {
        -total
    } else {
        total
    }))
}

/// Content of the current tar entry.
struct TarBody<'e, 'a, S: Read>(&'e mut Entry<'a, S>);

impl<S: Read> Body for TarBody<'_, '_, S> {
    fn open(&mut self) -> Result<&mut dyn Read, ArchiveError> {
        Ok(self.0)
    }
}

/// Output of a tar archive, compressed or not.
enum TarOut<W: Write> {
    Plain(W),
    Compressed(Box<codec::Encoder<W>>),
}

impl<W: Write> Write for TarOut<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(out) => out.write(buf),
            Self::Compressed(out) => out.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(out) => out.flush(),
            Self::Compressed(out) => out.flush(),
        }
    }
}

/// Writer of a new tar archive.
pub(crate) struct TarSink<W: Write> {
    builder: Builder<TarOut<W>>,
}

impl<W: Write> TarSink<W> {
    /// Starts an archive compressed with `compression` at `level`.
    ///
    /// # Errors
    ///
    /// Fails with [`ArchiveError::InvalidLevel`] when the level is outside
    /// the range of the algorithm or a level is given without compression.
    pub(crate) fn new(
        out: W,
        compression: Option<Algorithm>,
        level: Option<u32>,
    ) -> Result<Self, ArchiveError> {
        let format = Format::Tar(compression);
        let out = match compression {
            None => match level {
                None => TarOut::Plain(out),
                Some(level) => {
                    return Err(ArchiveError::InvalidLevel {
                        format,
                        level,
                        min: 0,
                        max: 0,
                    });
                }
            },
            Some(algorithm) => {
                let (min, max) = algorithm.level_range();
                if let Some(level) = level.filter(|level| !(min..=max).contains(level)) {
                    return Err(ArchiveError::InvalidLevel {
                        format,
                        level,
                        min,
                        max,
                    });
                }
                TarOut::Compressed(Box::new(codec::encoder(algorithm, level, out)?))
            }
        };
        Ok(Self {
            builder: Builder::new(out),
        })
    }

    /// Appends one member; `content` is read only for regular files.
    pub(crate) fn add(&mut self, input: &Input, content: &mut dyn Read) -> io::Result<()> {
        let mut header = Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        let seconds = input.modified.map_or(0, |instant| {
            instant.as_unix_nanos().div_euclid(NANOSECONDS_PER_SECOND)
        });
        header.set_mtime(u64::try_from(seconds).unwrap_or(0));
        let path = input.path.as_str();
        match &input.kind {
            InputKind::File { size } => {
                header.set_entry_type(EntryType::Regular);
                header.set_mode(input.mode.unwrap_or(DEFAULT_FILE_MODE));
                header.set_size(*size);
                self.builder.append_data(&mut header, path, content)
            }
            InputKind::Directory => {
                header.set_entry_type(EntryType::Directory);
                header.set_mode(input.mode.unwrap_or(DEFAULT_DIRECTORY_MODE));
                header.set_size(0);
                self.builder.append_data(&mut header, path, io::empty())
            }
            InputKind::Symlink(target) => {
                header.set_entry_type(EntryType::Symlink);
                header.set_mode(input.mode.unwrap_or(DEFAULT_SYMLINK_MODE));
                header.set_size(0);
                self.builder.append_link(&mut header, path, target.as_str())
            }
        }
    }

    /// Writes the end-of-archive blocks and the compressed stream trailer
    /// and returns the output.
    pub(crate) fn finish(self) -> io::Result<W> {
        match self.builder.into_inner()? {
            TarOut::Plain(out) => Ok(out),
            TarOut::Compressed(encoder) => encoder.finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pax_times() {
        let cases = [
            ("1700000000", Some(1_700_000_000_000_000_000)),
            ("1700000000.25", Some(1_700_000_000_250_000_000)),
            ("1.1234567891", Some(1_123_456_789)),
            ("-1.5", Some(-1_500_000_000)),
            ("", None),
            ("12a", None),
            ("1.x", None),
            (".5", None),
        ];
        for (text, nanos) in cases {
            assert_eq!(
                parse_pax_time(text),
                nanos.map(Instant::from_unix_nanos),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_level_for_plain_tar() {
        let error = TarSink::new(Vec::new(), None, Some(3)).err().unwrap();
        assert!(matches!(error, ArchiveError::InvalidLevel { .. }));
    }

    #[test]
    fn rejects_level_outside_algorithm_range() {
        let error = TarSink::new(Vec::new(), Some(Algorithm::Gzip), Some(10))
            .err()
            .unwrap();
        assert!(matches!(
            error,
            ArchiveError::InvalidLevel {
                level: 10,
                min: 0,
                max: 9,
                ..
            }
        ));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
