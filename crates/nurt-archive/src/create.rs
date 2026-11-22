// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Creation of zip, tar and 7z archives from a list of inputs.
//!
//! The members are written in byte order of their archive paths, with the
//! times and permissions given by the caller and nothing taken from the
//! clock or the account, so the same inputs always produce the same bytes.
//! Content is read through a callback when the member is written, so only
//! one input is open at a time.
//!
//! A 7z archive ends with a header that refers back to the content, which
//! requires a seekable destination. It is therefore assembled in memory, up
//! to [`MAX_SEVEN_Z_SIZE`] bytes, and copied to the output when complete.

use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use nurt_contracts::value::Instant;

use crate::list::{ArchiveError, Format, LimitKind};
use crate::safety::{self, Violation};
use crate::sevenz::{self, Creator, Metadata};
use crate::tar::TarSink;
use crate::zip::{self as zip_format, ZipSink};

/// Largest 7z archive [`create`] assembles in memory: 1 GiB.
pub const MAX_SEVEN_Z_SIZE: u64 = 1 << 30;

/// Type of a new member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    /// Regular file of the given size; the content must have exactly this
    /// length.
    File {
        /// Size in bytes.
        size: u64,
    },
    /// Directory.
    Directory,
    /// Symbolic link stored with its target. Whether links are stored or
    /// followed (`follow_links`) is decided by the caller, which passes a
    /// followed link as a [`InputKind::File`] or [`InputKind::Directory`].
    Symlink(String),
}

/// One member of a new archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Path inside the archive, relative, with `/` separators.
    pub path: String,
    /// Type.
    pub kind: InputKind,
    /// Permission bits; a default of the format is used when absent.
    pub mode: Option<u32>,
    /// Modification time; the earliest time of the format is used when
    /// absent.
    pub modified: Option<Instant>,
}

impl Input {
    /// Regular file of `size` bytes.
    #[must_use]
    pub fn file(path: impl Into<String>, size: u64) -> Self {
        Self::new(path.into(), InputKind::File { size })
    }

    /// Directory.
    #[must_use]
    pub fn directory(path: impl Into<String>) -> Self {
        Self::new(path.into(), InputKind::Directory)
    }

    /// Symbolic link to `target`.
    #[must_use]
    pub fn symlink(path: impl Into<String>, target: impl Into<String>) -> Self {
        Self::new(path.into(), InputKind::Symlink(target.into()))
    }

    /// Sets the permission bits.
    #[must_use]
    pub fn with_mode(mut self, mode: u32) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Sets the modification time.
    #[must_use]
    pub fn with_modified(mut self, modified: Instant) -> Self {
        self.modified = Some(modified);
        self
    }

    fn new(path: String, kind: InputKind) -> Self {
        Self {
            path,
            kind,
            mode: None,
            modified: None,
        }
    }
}

/// Format and compression level of a new archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateOptions {
    /// Container format.
    pub format: Format,
    /// Compression level: `0..=9` for zip, where 0 stores members
    /// uncompressed, the LZMA2 preset `0..=9` for 7z, and the range of the
    /// stream algorithm for compressed tar; `None` selects the default of
    /// the format.
    pub level: Option<u32>,
}

impl CreateOptions {
    /// Options for `format` at its default level.
    #[must_use]
    pub fn new(format: Format) -> Self {
        Self {
            format,
            level: None,
        }
    }
}

/// Writes a new archive with the given members to `out` and returns it.
///
/// Member paths are normalised (`.` and empty components are dropped) and
/// sorted in byte order. `open` is called once for every regular file, in
/// archive order, and must yield exactly the `size` of the file.
///
/// # Errors
///
/// * [`ArchiveError::InvalidPath`] when a path is empty, absolute, starts
///   with a drive letter or contains `..`, `\` or NUL;
/// * [`ArchiveError::DuplicatePath`] when two inputs have the same path;
/// * [`ArchiveError::InvalidLevel`] when the level is outside the range of
///   the format;
/// * [`ArchiveError::Changed`] when a file yields a different number of
///   bytes than its size;
/// * [`ArchiveError::Limit`] with [`LimitKind::Memory`] when a 7z archive
///   would exceed [`MAX_SEVEN_Z_SIZE`];
/// * [`ArchiveError::Io`] when `open`, a content reader or `out` fails.
///
/// # Examples
///
/// ```
/// use nurt_archive::codec::Algorithm;
/// use nurt_archive::create::{create, CreateOptions, Input};
/// use nurt_archive::list::Format;
///
/// let options = CreateOptions::new(Format::Tar(Some(Algorithm::Zstd)));
/// let inputs = vec![Input::file("b.txt", 1), Input::file("./a.txt", 1)];
/// let archive = create(Vec::new(), &options, inputs, |_| Ok(Box::new(&b"x"[..]))).unwrap();
/// assert!(!archive.is_empty());
/// ```
pub fn create<'r, W, F>(
    out: W,
    options: &CreateOptions,
    inputs: Vec<Input>,
    mut open: F,
) -> Result<W, ArchiveError>
where
    W: Write,
    F: FnMut(&Input) -> io::Result<Box<dyn Read + 'r>>,
{
    let inputs = normalise(inputs)?;
    match options.format {
        Format::Zip => {
            let (min, max) = zip_format::LEVELS;
            if let Some(level) = options.level.filter(|level| !(min..=max).contains(level)) {
                return Err(ArchiveError::InvalidLevel {
                    format: Format::Zip,
                    level,
                    min,
                    max,
                });
            }
            write_members(ZipSink::new(out, options.level), &inputs, &mut open)
        }
        Format::Tar(compression) => write_members(
            TarSink::new(out, compression, options.level)?,
            &inputs,
            &mut open,
        ),
        Format::SevenZ => {
            let (min, max) = sevenz::LEVELS;
            if let Some(level) = options.level.filter(|level| !(min..=max).contains(level)) {
                return Err(ArchiveError::InvalidLevel {
                    format: Format::SevenZ,
                    level,
                    min,
                    max,
                });
            }
            write_seven_z(out, options.level, &inputs, &mut open, MAX_SEVEN_Z_SIZE)
        }
    }
}

/// Writes a 7z archive through an in-memory buffer of at most `limit`
/// bytes. The declared content is checked against the limit before any
/// input is opened.
fn write_seven_z<'r, W, F>(
    out: W,
    level: Option<u32>,
    inputs: &[Input],
    open: &mut F,
    limit: u64,
) -> Result<W, ArchiveError>
where
    W: Write,
    F: FnMut(&Input) -> io::Result<Box<dyn Read + 'r>>,
{
    let declared = inputs
        .iter()
        .map(|input| match &input.kind {
            InputKind::File { size } => *size,
            InputKind::Symlink(target) => target.len() as u64,
            InputKind::Directory => 0,
        })
        .fold(0_u64, u64::saturating_add);
    if declared > limit {
        return Err(ArchiveError::Limit {
            limit: LimitKind::Memory,
            allowed: limit,
            actual: declared,
        });
    }
    let buffer = BoundedBuffer {
        cursor: Cursor::new(Vec::new()),
        limit,
    };
    let creator = match level {
        Some(level) => Creator::with_level(buffer, level),
        None => Creator::new(buffer),
    }
    .map_err(|error| seven_z_write_error(error, limit))?;
    write_members(
        SevenZSink {
            creator,
            out,
            limit,
        },
        inputs,
        open,
    )
}

/// Converts a failure of the 7z writer, recognising a full buffer.
fn seven_z_write_error(error: sevenz::Error, limit: u64) -> ArchiveError {
    match error {
        sevenz::Error::Io(error) => {
            let full = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<BufferFull>())
                .map(|full| full.attempted);
            match full {
                Some(attempted) => ArchiveError::Limit {
                    limit: LimitKind::Memory,
                    allowed: limit,
                    actual: attempted,
                },
                None => ArchiveError::Io(error),
            }
        }
        other => ArchiveError::Io(io::Error::other(other.to_string())),
    }
}

/// Writer of a 7z archive into a [`BoundedBuffer`], copied to `out` when
/// the archive is complete.
struct SevenZSink<W> {
    creator: Creator<BoundedBuffer>,
    out: W,
    limit: u64,
}

impl<W: Write> Sink<W> for SevenZSink<W> {
    fn add(&mut self, input: &Input, content: &mut dyn Read) -> Result<(), ArchiveError> {
        let metadata = Metadata {
            modified: input.modified.and_then(Instant::to_system_time),
            mode: input.mode,
        };
        let result = match &input.kind {
            InputKind::File { .. } => self.creator.add_file(&input.path, metadata, content),
            InputKind::Directory => self.creator.add_directory(&input.path, metadata),
            InputKind::Symlink(target) => self.creator.add_symlink(&input.path, target, metadata),
        };
        result.map_err(|error| seven_z_write_error(error, self.limit))
    }

    fn finish(self) -> Result<W, ArchiveError> {
        let buffer = self
            .creator
            .finish()
            .map_err(|error| seven_z_write_error(error, self.limit))?;
        let mut out = self.out;
        out.write_all(buffer.cursor.get_ref())?;
        Ok(out)
    }
}

/// In-memory destination that refuses to grow beyond `limit` bytes.
struct BoundedBuffer {
    cursor: Cursor<Vec<u8>>,
    limit: u64,
}

impl Write for BoundedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let end = self.cursor.position().saturating_add(buf.len() as u64);
        if end > self.limit {
            return Err(io::Error::other(BufferFull { attempted: end }));
        }
        self.cursor.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedBuffer {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}

/// Error of a [`BoundedBuffer`] asked to grow beyond its limit, carried
/// inside an [`io::Error`] through the 7z writer.
#[derive(Debug)]
struct BufferFull {
    attempted: u64,
}

impl fmt::Display for BufferFull {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the 7z archive needs {} bytes of memory",
            self.attempted
        )
    }
}

impl std::error::Error for BufferFull {}

fn normalise(inputs: Vec<Input>) -> Result<Vec<Input>, ArchiveError> {
    let mut normalised = Vec::with_capacity(inputs.len());
    for mut input in inputs {
        let path = match safety::components(&input.path) {
            Ok(parts) if parts.is_empty() => Err(Violation::InvalidName),
            Ok(parts) => Ok(parts.join("/")),
            Err(violation) => Err(violation),
        };
        match path {
            Ok(path) => input.path = path,
            Err(violation) => {
                return Err(ArchiveError::InvalidPath {
                    path: input.path,
                    violation,
                });
            }
        }
        normalised.push(input);
    }
    normalised.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    if let Some(pair) = normalised
        .windows(2)
        .find(|pair| pair[0].path == pair[1].path)
    {
        return Err(ArchiveError::DuplicatePath {
            path: pair[0].path.clone(),
        });
    }
    Ok(normalised)
}

/// Writer of one archive format.
trait Sink<W> {
    fn add(&mut self, input: &Input, content: &mut dyn Read) -> Result<(), ArchiveError>;
    fn finish(self) -> Result<W, ArchiveError>;
}

impl<W: Write> Sink<W> for ZipSink<W> {
    fn add(&mut self, input: &Input, content: &mut dyn Read) -> Result<(), ArchiveError> {
        ZipSink::add(self, input, content)
    }

    fn finish(self) -> Result<W, ArchiveError> {
        ZipSink::finish(self)
    }
}

impl<W: Write> Sink<W> for TarSink<W> {
    fn add(&mut self, input: &Input, content: &mut dyn Read) -> Result<(), ArchiveError> {
        Ok(TarSink::add(self, input, content)?)
    }

    fn finish(self) -> Result<W, ArchiveError> {
        Ok(TarSink::finish(self)?)
    }
}

fn write_members<'r, W, S, F>(
    mut sink: S,
    inputs: &[Input],
    open: &mut F,
) -> Result<W, ArchiveError>
where
    S: Sink<W>,
    F: FnMut(&Input) -> io::Result<Box<dyn Read + 'r>>,
{
    for input in inputs {
        let InputKind::File { size } = input.kind else {
            sink.add(input, &mut io::empty())?;
            continue;
        };
        let mut content = ExactReader::new(open(input)?, size);
        let result = sink.add(input, &mut content);
        if let Some(detail) = content.mismatch {
            return Err(ArchiveError::Changed {
                path: input.path.clone(),
                detail: detail.to_owned(),
            });
        }
        result?;
    }
    sink.finish()
}

/// Reader that yields exactly the declared number of bytes, records a
/// content of a different length in [`ExactReader::mismatch`] and marks
/// failures of the inner reader in [`ExactReader::read_failed`].
pub(crate) struct ExactReader<R> {
    inner: R,
    remaining: u64,
    pub(crate) mismatch: Option<&'static str>,
    pub(crate) read_failed: bool,
}

impl<R: Read> ExactReader<R> {
    pub(crate) fn new(inner: R, size: u64) -> Self {
        Self {
            inner,
            remaining: size,
            mismatch: None,
            read_failed: false,
        }
    }

    fn fail(&mut self, detail: &'static str) -> io::Error {
        self.mismatch = Some(detail);
        io::Error::new(io::ErrorKind::InvalidData, detail)
    }

    fn read_inner(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner
            .read(buf)
            .inspect_err(|_| self.read_failed = true)
    }
}

impl<R: Read> Read for ExactReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            let mut probe = [0_u8; 1];
            return match self.read_inner(&mut probe)? {
                0 => Ok(0),
                _ => Err(self.fail("the content is longer than the declared size")),
            };
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let limit = usize::try_from(self.remaining).map_or(buf.len(), |left| left.min(buf.len()));
        let count = self.read_inner(&mut buf[..limit])?;
        if count == 0 {
            return Err(self.fail("the content is shorter than the declared size"));
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::codec::Algorithm;

    fn content(text: &'static str) -> io::Result<Box<dyn Read>> {
        Ok(Box::new(text.as_bytes()))
    }

    #[test]
    fn rejects_unsafe_input_paths() {
        for (path, violation) in [
            ("../x", Violation::ParentComponent),
            ("/x", Violation::Absolute),
            ("C:/x", Violation::DriveLetter),
            ("a\\b", Violation::Backslash),
            ("./", Violation::InvalidName),
        ] {
            let error = create(
                Vec::new(),
                &CreateOptions::new(Format::Zip),
                vec![Input::directory(path)],
                |_| content(""),
            )
            .unwrap_err();
            assert!(
                matches!(&error, ArchiveError::InvalidPath { violation: found, .. } if *found == violation),
                "{path}: {error}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_paths_after_normalisation() {
        let error = create(
            Vec::new(),
            &CreateOptions::new(Format::Tar(None)),
            vec![Input::directory("a/b"), Input::directory("./a//b/")],
            |_| content(""),
        )
        .unwrap_err();
        assert!(matches!(error, ArchiveError::DuplicatePath { path } if path == "a/b"));
    }

    #[test]
    fn reports_files_that_changed_size() {
        for (size, text) in [(5, "abc"), (2, "abc")] {
            for format in [Format::Zip, Format::Tar(None)] {
                let error = create(
                    Vec::new(),
                    &CreateOptions::new(format),
                    vec![Input::file("f", size)],
                    |_| content(text),
                )
                .unwrap_err();
                assert!(
                    matches!(&error, ArchiveError::Changed { path, .. } if path == "f"),
                    "{format}: {error}"
                );
                assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Stale);
            }
        }
    }

    #[test]
    fn rejects_zip_level_outside_range() {
        let options = CreateOptions {
            format: Format::Zip,
            level: Some(10),
        };
        let error = create(Vec::new(), &options, Vec::new(), |_| content("")).unwrap_err();
        assert!(matches!(error, ArchiveError::InvalidLevel { max: 9, .. }));
    }

    #[test]
    fn rejects_seven_z_level_outside_range() {
        let options = CreateOptions {
            format: Format::SevenZ,
            level: Some(10),
        };
        let error = create(Vec::new(), &options, Vec::new(), |_| content("")).unwrap_err();
        assert!(matches!(
            error,
            ArchiveError::InvalidLevel {
                format: Format::SevenZ,
                max: 9,
                ..
            }
        ));
    }

    #[test]
    fn seven_z_content_above_the_memory_limit_is_refused_before_reading() {
        let mut opened = false;
        let error = write_seven_z(
            Vec::new(),
            None,
            &[Input::file("big", 100)],
            &mut |_: &Input| -> io::Result<Box<dyn Read>> {
                opened = true;
                content("")
            },
            64,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ArchiveError::Limit {
                limit: LimitKind::Memory,
                allowed: 64,
                actual: 100,
            }
        ));
        assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Limit);
        assert!(!opened);
    }

    #[test]
    fn seven_z_archive_outgrowing_the_buffer_fails_with_limit() {
        let error = write_seven_z(
            Vec::new(),
            None,
            &[Input::file("small", 10)],
            &mut |_: &Input| content("0123456789"),
            64,
        )
        .unwrap_err();
        assert!(
            matches!(
                &error,
                ArchiveError::Limit {
                    limit: LimitKind::Memory,
                    allowed: 64,
                    actual,
                } if *actual > 64
            ),
            "{error}"
        );
    }

    #[test]
    fn seven_z_reports_files_that_changed_size() {
        for (size, text) in [(5, "abc"), (2, "abc")] {
            let error = create(
                Vec::new(),
                &CreateOptions::new(Format::SevenZ),
                vec![Input::file("f", size)],
                |_| content(text),
            )
            .unwrap_err();
            assert!(
                matches!(&error, ArchiveError::Changed { path, .. } if path == "f"),
                "{size}: {error}"
            );
        }
    }

    #[test]
    fn seven_z_output_is_reproducible() {
        let inputs = vec![
            Input::file("b", 1).with_mode(0o600),
            Input::directory("a"),
            Input::file("a/c", 1).with_modified(Instant::from_unix_nanos(1_500_000_000)),
            Input::symlink("l", "b"),
        ];
        let mut reversed = inputs.clone();
        reversed.reverse();
        let options = CreateOptions::new(Format::SevenZ);
        let first = create(Vec::new(), &options, inputs, |_| content("x")).unwrap();
        let second = create(Vec::new(), &options, reversed, |_| content("x")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn output_does_not_depend_on_input_order() {
        let inputs = vec![
            Input::file("b", 1).with_mode(0o600),
            Input::directory("a"),
            Input::file("a/c", 1),
            Input::symlink("l", "b"),
        ];
        let mut reversed = inputs.clone();
        reversed.reverse();
        for format in [
            Format::Zip,
            Format::Tar(None),
            Format::Tar(Some(Algorithm::Gzip)),
        ] {
            let options = CreateOptions::new(format);
            let first = create(Vec::new(), &options, inputs.clone(), |_| content("x")).unwrap();
            let second = create(Vec::new(), &options, reversed.clone(), |_| content("x")).unwrap();
            assert_eq!(first, second, "{format}");
            let mut paths = Vec::new();
            crate::list::list(Cursor::new(first), None, |member| {
                paths.push(member.path);
                Ok(())
            })
            .unwrap();
            assert_eq!(paths, ["a", "a/c", "b", "l"], "{format}");
        }
    }
}
