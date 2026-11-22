// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading and writing zip archives.
//!
//! Member times are read from the extended timestamp field when present and
//! otherwise from the MS-DOS date and time, which has no UTC offset and is
//! read as UTC. New archives store both, so times survive a round trip with
//! one-second precision.

use std::borrow::Cow;
use std::io::{self, Read, Seek, Write};

use nurt_contracts::value::Instant;
use zip::result::ZipError;
use zip::write::{FullFileOptions, SimpleFileOptions, StreamWriter};
use zip::{CompressionMethod, DateTime, ExtraField, System, ZipArchive, ZipWriter};

use crate::create::{Input, InputKind};
use crate::list::{ArchiveError, Body, Flow, Format, Member, MemberKind, Visitor};

/// Longest symbolic link target read from an archive.
const MAX_LINK_TARGET: u64 = 4096;
const PERMISSION_BITS: u32 = 0o7777;
const FILE_TYPE_BITS: u32 = 0o170_000;
const TYPE_REGULAR: u32 = 0o100_000;
const TYPE_DIRECTORY: u32 = 0o040_000;
const TYPE_SYMLINK: u32 = 0o120_000;
/// Deflate levels accepted for new archives; level 0 stores members
/// uncompressed.
pub(crate) const LEVELS: (u32, u32) = (0, 9);
const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const SECONDS_PER_HOUR: i64 = 3_600;
const SECONDS_PER_MINUTE: i64 = 60;

/// Passes every member of a zip archive to the visitor in the order of the
/// central directory.
pub(crate) fn visit<R: Read + Seek>(
    reader: R,
    visitor: &mut Visitor<'_>,
) -> Result<(), ArchiveError> {
    let mut archive = ZipArchive::new(reader).map_err(read_error)?;
    for index in 0..archive.len() {
        let member = read_member(&mut archive, index)?;
        let mut body = if member.encrypted {
            ZipBody::Encrypted(member.path.clone())
        } else {
            ZipBody::Closed(&mut archive, index)
        };
        if visitor(&member, &mut body)? == Flow::Stop {
            break;
        }
    }
    Ok(())
}

fn read_member<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    index: usize,
) -> Result<Member, ArchiveError> {
    let file = archive.by_index_raw(index).map_err(read_error)?;
    let unix_mode = file.unix_mode();
    let kind = if file.is_dir() {
        MemberKind::Directory
    } else {
        match unix_mode.map(|mode| mode & FILE_TYPE_BITS) {
            None | Some(0 | TYPE_REGULAR) => MemberKind::File,
            Some(TYPE_DIRECTORY) => MemberKind::Directory,
            Some(TYPE_SYMLINK) => MemberKind::Symlink,
            Some(_) => MemberKind::Other,
        }
    };
    let mut member = match file.name() {
        Ok(name) => Member::new(Cow::Borrowed(name.as_bytes()), kind),
        Err(_) => Member::new(Cow::Borrowed(file.name_raw()), kind),
    };
    member.size = file.size();
    member.packed = Some(file.compressed_size());
    member.mode = unix_mode.map(|mode| mode & PERMISSION_BITS);
    member.compression = Some(method_name(file.compression()).to_owned());
    member.encrypted = file.encrypted();
    member.crc32 = Some(file.crc32());
    let extended = file.extra_data_fields().find_map(|field| match field {
        ExtraField::ExtendedTimestamp(timestamp) => timestamp.mod_time(),
        _ => None,
    });
    member.modified = match extended {
        Some(seconds) => Instant::from_unix_seconds(i64::from(seconds)),
        None => file.last_modified().and_then(dos_to_instant),
    };
    drop(file);
    if kind == MemberKind::Symlink && !member.encrypted {
        let mut target = Vec::new();
        archive
            .by_index(index)
            .map_err(read_error)?
            .take(MAX_LINK_TARGET + 1)
            .read_to_end(&mut target)
            .map_err(|error| ArchiveError::from_read(Format::Zip, error))?;
        if target.len() as u64 > MAX_LINK_TARGET {
            return Err(ArchiveError::format(
                Some(Format::Zip),
                format!(
                    "link target of {} is longer than {MAX_LINK_TARGET} bytes",
                    member.path
                ),
            ));
        }
        member.set_link_target(Cow::Owned(target));
    }
    Ok(member)
}

/// Content of one zip member, decompressed when first opened.
enum ZipBody<'a, R: Read> {
    Closed(&'a mut ZipArchive<R>, usize),
    Open(zip::read::ZipFile<'a, R>),
    Encrypted(String),
    Unavailable,
}

impl<R: Read + Seek> Body for ZipBody<'_, R> {
    fn open(&mut self) -> Result<&mut dyn Read, ArchiveError> {
        match std::mem::replace(self, Self::Unavailable) {
            Self::Closed(archive, index) => {
                *self = Self::Open(archive.by_index(index).map_err(read_error)?);
            }
            Self::Encrypted(path) => return Err(ArchiveError::Encrypted { path }),
            other => *self = other,
        }
        match self {
            Self::Open(file) => Ok(file),
            _ => Err(ArchiveError::format(
                Some(Format::Zip),
                "member content is not available",
            )),
        }
    }
}

fn read_error(error: ZipError) -> ArchiveError {
    match error {
        ZipError::Io(error) => ArchiveError::from_read(Format::Zip, error),
        ZipError::CompressionMethodNotSupported(method) => ArchiveError::format(
            Some(Format::Zip),
            format!("compression method {method} is not supported"),
        ),
        other => ArchiveError::format(Some(Format::Zip), other.to_string()),
    }
}

fn write_error(error: ZipError) -> ArchiveError {
    match error {
        ZipError::Io(error) => ArchiveError::Io(error),
        other => ArchiveError::format(Some(Format::Zip), other.to_string()),
    }
}

/// Name of a zip compression method as shown in the `compression` field.
fn method_name(method: CompressionMethod) -> String {
    match method.to_string().as_str() {
        "Deflated" => "deflate".to_owned(),
        name => name.to_lowercase(),
    }
}

/// Writer of a new zip archive that needs no seeking; sizes and checksums
/// follow each member in a data descriptor.
pub(crate) struct ZipSink<W: Write> {
    writer: ZipWriter<StreamWriter<W>>,
    method: CompressionMethod,
    level: Option<i64>,
}

impl<W: Write> ZipSink<W> {
    /// Starts an archive; the level must be within [`LEVELS`].
    pub(crate) fn new(out: W, level: Option<u32>) -> Self {
        let (method, level) = match level {
            Some(0) => (CompressionMethod::Stored, None),
            Some(level) => (CompressionMethod::DEFLATE, Some(i64::from(level))),
            None => (CompressionMethod::DEFLATE, None),
        };
        Self {
            writer: ZipWriter::new_stream(out),
            method,
            level,
        }
    }

    /// Appends one member; `content` is read only for regular files.
    pub(crate) fn add(
        &mut self,
        input: &Input,
        content: &mut dyn Read,
    ) -> Result<(), ArchiveError> {
        let options = self.options(input).map_err(write_error)?;
        match &input.kind {
            InputKind::Directory => self
                .writer
                .add_directory(input.path.as_str(), options)
                .map_err(write_error),
            InputKind::Symlink(target) => self
                .writer
                .add_symlink(input.path.as_str(), target.as_str(), options)
                .map_err(write_error),
            InputKind::File { .. } => {
                self.writer
                    .start_file(input.path.as_str(), options)
                    .map_err(write_error)?;
                io::copy(content, &mut self.writer)?;
                Ok(())
            }
        }
    }

    fn options(&self, input: &Input) -> Result<FullFileOptions<'static, 'static>, ZipError> {
        let size = match input.kind {
            InputKind::File { size } => size,
            InputKind::Directory | InputKind::Symlink(_) => 0,
        };
        let mut options = SimpleFileOptions::default()
            .system(System::Unix)
            .compression_method(self.method)
            .compression_level(self.level)
            .last_modified_time(instant_to_dos(input.modified))
            .large_file(size >= zip::ZIP64_BYTES_THR);
        if let Some(mode) = input.mode {
            options = options.unix_permissions(mode);
        }
        let seconds = input
            .modified
            .and_then(|instant| u32::try_from(unix_seconds(instant)).ok());
        let options = options.into_full_options();
        match seconds {
            Some(seconds) => options.with_extended_timestamp(Some(seconds), None, None),
            None => Ok(options),
        }
    }

    /// Writes the central directory and returns the output.
    pub(crate) fn finish(self) -> Result<W, ArchiveError> {
        Ok(self.writer.finish().map_err(write_error)?.into_inner())
    }
}

fn unix_seconds(instant: Instant) -> i64 {
    instant.as_unix_nanos().div_euclid(NANOSECONDS_PER_SECOND)
}

fn dos_to_instant(time: DateTime) -> Option<Instant> {
    if !time.is_valid() {
        return None;
    }
    let days = days_from_civil(
        i64::from(time.year()),
        i64::from(time.month()),
        i64::from(time.day()),
    );
    let seconds = days * SECONDS_PER_DAY
        + i64::from(time.hour()) * SECONDS_PER_HOUR
        + i64::from(time.minute()) * SECONDS_PER_MINUTE
        + i64::from(time.second());
    Instant::from_unix_seconds(seconds)
}

/// MS-DOS time of an instant in UTC; instants outside 1980..2107 and
/// missing times become the earliest MS-DOS time.
fn instant_to_dos(instant: Option<Instant>) -> DateTime {
    let Some(instant) = instant else {
        return DateTime::default();
    };
    let seconds = unix_seconds(instant);
    let (year, month, day) = civil_from_days(seconds.div_euclid(SECONDS_PER_DAY));
    let of_day = seconds.rem_euclid(SECONDS_PER_DAY);
    let fields = (
        u16::try_from(year),
        u8::try_from(month),
        u8::try_from(day),
        u8::try_from(of_day / SECONDS_PER_HOUR),
        u8::try_from(of_day % SECONDS_PER_HOUR / SECONDS_PER_MINUTE),
        u8::try_from(of_day % SECONDS_PER_MINUTE),
    );
    match fields {
        (Ok(year), Ok(month), Ok(day), Ok(hour), Ok(minute), Ok(second)) => {
            DateTime::from_date_and_time(year, month, day, hour, minute, second).unwrap_or_default()
        }
        _ => DateTime::default(),
    }
}

/// Years in a cycle of the proleptic Gregorian calendar.
const YEARS_PER_ERA: i64 = 400;
/// Days in [`YEARS_PER_ERA`] years.
const DAYS_PER_ERA: i64 = 146_097;
/// Days from 0000-03-01 to 1970-01-01.
const EPOCH_FROM_MARCH_ZERO: i64 = 719_468;
const DAYS_PER_YEAR: i64 = 365;

/// Days since 1970-01-01 of a proleptic Gregorian date.
///
/// The `days_from_civil` algorithm of H. Hinnant ("chrono-Compatible
/// Low-Level Date Algorithms"): years start on 1 March, so the leap day is
/// the last day of a year, and month lengths from March follow
/// `(153 * month + 2) / 5`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(YEARS_PER_ERA);
    let year_of_era = year.rem_euclid(YEARS_PER_ERA);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * DAYS_PER_YEAR + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - EPOCH_FROM_MARCH_ZERO
}

/// Year, month and day of a day count; the inverse of [`days_from_civil`]
/// (`civil_from_days` of the same paper).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + EPOCH_FROM_MARCH_ZERO;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524
        - day_of_era / (DAYS_PER_ERA - 1))
        / DAYS_PER_YEAR;
    let day_of_year =
        day_of_era - (DAYS_PER_YEAR * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * YEARS_PER_ERA + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip() {
        let cases = [
            ((1970, 1, 1), 0),
            ((2000, 2, 29), 11_016),
            ((1980, 1, 1), 3_652),
            ((2107, 12, 31), 50_402),
            ((1969, 12, 31), -1),
        ];
        for ((year, month, day), days) in cases {
            assert_eq!(
                days_from_civil(year, month, day),
                days,
                "{year}-{month}-{day}"
            );
            assert_eq!(civil_from_days(days), (year, month, day));
        }
    }

    #[test]
    fn dos_times_convert_as_utc() {
        let instant = Instant::from_unix_seconds(1_700_000_000).unwrap();
        let dos = instant_to_dos(Some(instant));
        assert_eq!(
            (
                dos.year(),
                dos.month(),
                dos.day(),
                dos.hour(),
                dos.minute(),
                dos.second()
            ),
            (2023, 11, 14, 22, 13, 20)
        );
        assert_eq!(dos_to_instant(dos), Some(instant));
    }

    #[test]
    fn times_outside_dos_range_use_the_earliest_time() {
        let before = Instant::from_unix_seconds(0).unwrap();
        assert_eq!(instant_to_dos(Some(before)), DateTime::default());
        assert_eq!(instant_to_dos(None), DateTime::default());
    }

    #[test]
    fn names_compression_methods() {
        assert_eq!(method_name(CompressionMethod::Stored), "stored");
        assert_eq!(method_name(CompressionMethod::DEFLATE), "deflate");
        assert_eq!(method_name(CompressionMethod::Zstd), "zstd");
    }
}
