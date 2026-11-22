// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Creation of zip and tar archives from a list of inputs.
//!
//! The members are written in byte order of their archive paths, with the
//! times and permissions given by the caller and nothing taken from the
//! clock or the account, so the same inputs always produce the same bytes.
//! Content is read through a callback when the member is written, so only
//! one input is open at a time.

use std::io::{self, Read, Write};

use nurt_contracts::value::Instant;

use crate::list::{ArchiveError, Format};
use crate::safety::{self, Violation};
use crate::tar::TarSink;
use crate::zip::{self as zip_format, ZipSink};

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
    /// uncompressed, and the range of the stream algorithm for compressed
    /// tar; `None` selects the default of the format.
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
/// * [`ArchiveError::Unsupported`] for a format this build cannot write;
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
        format @ Format::SevenZ => Err(ArchiveError::Unsupported { format }),
    }
}

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
    fn seven_z_is_not_written_by_this_build() {
        let error = create(
            Vec::new(),
            &CreateOptions::new(Format::SevenZ),
            Vec::new(),
            |_| content(""),
        )
        .unwrap_err();
        assert!(matches!(error, ArchiveError::Unsupported { .. }));
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
