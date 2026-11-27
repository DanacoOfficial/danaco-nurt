// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Read-only provider exposing archive contents as paths.
//!
//! An address names an archive file and a path inside it, separated by the
//! first `!`: `archive:/srv/a.zip!/dir/file`. Archives are read through
//! [`Fs::open_read`], so the provider works on every file system back end.
//! Supported formats are ZIP (stored, deflate and Zstandard members) and tar,
//! plain or compressed with gzip or Zstandard; the format is recognised by
//! the leading bytes of the file.
//!
//! Inner names containing `..`, `\`, a leading `/` or a drive prefix make
//! the archive unreadable with [`FsErrorKind::InvalidPath`]. Decompression is
//! bounded by [`ArchiveLimits`]: a stream that expands beyond the size limit
//! or beyond the ratio limit fails with [`FsErrorKind::Io`]. Damaged archives
//! fail with [`FsErrorKind::Io`] and never panic.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom, Take};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use nurt_contracts::fs::{
    AbsPath, FileId, FileKind, FileMeta, Fs, FsError, FsErrorKind, FsResult, Provider,
    ProviderCaps, ProviderChild, ProviderKey, ProviderScheme, ProviderSnapshot, ProviderValue,
    ReadHandle,
};

/// Scheme of the provider.
pub const SCHEME: &str = "archive";

const OP: &str = "archive";
const ZIP_MAGIC: &[u8] = b"PK";
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];
const ZSTD_MAGIC: &[u8] = &[0x28, 0xb5, 0x2f, 0xfd];
const MAGIC_LEN: usize = 4;
const ENTRY_MODE: u32 = 0o444;
const DIR_MODE: u32 = 0o555;
const NANOS_PER_SECOND: i64 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Bounds of decompression and of values returned by [`Provider::get`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Largest entry returned whole by [`Provider::get`]; larger entries are
    /// read with [`ArchiveProvider::open`]. Default 1 MiB.
    pub max_value_size: u64,
    /// Largest number of bytes one decompressed stream may produce. Default
    /// 64 GiB.
    pub max_expanded: u64,
    /// Largest ratio of produced to consumed bytes. Default 1000.
    pub max_ratio: u64,
    /// Produced bytes below which the ratio is not checked. Default 1 MiB.
    pub ratio_floor: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_value_size: MIB,
            max_expanded: 64 * GIB,
            max_ratio: 1_000,
            ratio_floor: MIB,
        }
    }
}

/// Archive file and path inside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveAddress {
    /// Archive file.
    pub archive: AbsPath,
    /// Components of the inner path; empty for the archive root.
    pub inner: Vec<String>,
}

impl ArchiveAddress {
    /// Parses `archive:<archive path>!<inner path>`; the `archive:` prefix is
    /// optional and an empty inner path addresses the root.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when the `!` separator is
    /// missing, the archive path is not absolute or the inner path contains
    /// `..` or `\`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::archive::ArchiveAddress;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let address = ArchiveAddress::parse("archive:/srv/a.zip!/dir/file.txt")?;
    /// assert_eq!(address.archive.as_path(), std::path::Path::new("/srv/a.zip"));
    /// assert_eq!(address.inner, ["dir", "file.txt"]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn parse(text: &str) -> FsResult<Self> {
        let body = text
            .strip_prefix(SCHEME)
            .and_then(|rest| rest.strip_prefix(':'))
            .unwrap_or(text);
        let (archive, inner) = body.split_once('!').ok_or_else(|| invalid(body))?;
        Ok(Self {
            archive: AbsPath::new(archive)?,
            inner: inner_components(inner.trim_start_matches('/').as_bytes())?,
        })
    }

    /// Provider key: the archive path in language form, then the inner
    /// components.
    #[must_use]
    pub fn to_key(&self) -> ProviderKey {
        let mut segments = vec![self.archive.to_language_string()];
        segments.extend(self.inner.iter().cloned());
        ProviderKey {
            scheme: ProviderScheme(SCHEME.to_owned()),
            segments,
            value: None,
        }
    }

    /// Address of a provider key produced by [`to_key`](Self::to_key).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] for another scheme, an empty
    /// key or invalid components.
    pub fn from_key(key: &ProviderKey) -> FsResult<Self> {
        let Some((archive, inner)) = key.segments.split_first() else {
            return Err(invalid(""));
        };
        if key.scheme.0 != SCHEME {
            return Err(invalid(&key.scheme.0));
        }
        let inner = inner_components(inner.join("/").as_bytes())?;
        Ok(Self {
            archive: AbsPath::new(archive)?,
            inner,
        })
    }
}

/// One member of an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Components of the inner path.
    pub path: Vec<String>,
    /// [`FileKind::File`], [`FileKind::Directory`], [`FileKind::Symlink`] or
    /// [`FileKind::Special`] (hard links and devices).
    pub kind: FileKind,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Stored size in bytes; equal to `size` in uncompressed tar archives.
    pub compressed_size: u64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub modified_ns: Option<i64>,
}

/// Provider of the `archive:` scheme.
///
/// Every call reads the archive anew; nothing is cached or extracted.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::fs::Provider;
/// use nurt_fs::archive::ArchiveProvider;
/// use nurt_fs::mem::MemFs;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let provider = ArchiveProvider::new(Arc::new(MemFs::new()?));
/// assert_eq!(provider.scheme().0, "archive");
/// assert!(!provider.caps().writable);
/// # Ok(())
/// # }
/// ```
pub struct ArchiveProvider {
    fs: Arc<dyn Fs>,
    limits: ArchiveLimits,
}

impl std::fmt::Debug for ArchiveProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveProvider")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl ArchiveProvider {
    /// Provider reading archives through `fs` with default limits.
    #[must_use]
    pub fn new(fs: Arc<dyn Fs>) -> Self {
        Self {
            fs,
            limits: ArchiveLimits::default(),
        }
    }

    /// Replaces the limits.
    #[must_use]
    pub fn with_limits(mut self, limits: ArchiveLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Every member of `archive`, with directories implied by member paths,
    /// in byte order of their paths.
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::open_read`], with [`FsErrorKind::Unsupported`] for an
    /// unknown format, [`FsErrorKind::InvalidPath`] for unsafe member names
    /// and [`FsErrorKind::Io`] for a damaged archive or an exceeded limit.
    pub fn entries(&self, archive: &AbsPath) -> FsResult<Vec<ArchiveEntry>> {
        Ok(self
            .index(archive)?
            .into_values()
            .map(|member| member.entry)
            .collect())
    }

    /// Metadata of the member at `address`; the root is a directory.
    ///
    /// # Errors
    ///
    /// Fails as [`entries`](Self::entries), and with
    /// [`FsErrorKind::NotFound`] for a missing member.
    pub fn entry(&self, address: &ArchiveAddress) -> FsResult<ArchiveEntry> {
        if address.inner.is_empty() {
            self.fs
                .metadata(&address.archive, nurt_contracts::fs::Follow::Follow)?;
            return Ok(directory(Vec::new()));
        }
        self.index(&address.archive)?
            .remove(&address.inner)
            .map(|member| member.entry)
            .ok_or_else(|| not_found(address))
    }

    /// Opens the content of a file member for streaming; the member is
    /// decompressed while it is read.
    ///
    /// # Errors
    ///
    /// Fails as [`entry`](Self::entry), with [`FsErrorKind::IsADirectory`]
    /// for a directory and [`FsErrorKind::Unsupported`] for links and
    /// unsupported compression methods. Reading fails with an error of kind
    /// [`io::ErrorKind::InvalidData`] when a limit is exceeded.
    pub fn open(&self, address: &ArchiveAddress) -> FsResult<Box<dyn ReadHandle>> {
        let member = self
            .index(&address.archive)?
            .remove(&address.inner)
            .ok_or_else(|| not_found(address))?;
        match member.entry.kind {
            FileKind::File => {}
            FileKind::Directory => return Err(fail(FsErrorKind::IsADirectory, address)),
            _ => return Err(fail(FsErrorKind::Unsupported, address)),
        }
        let source = Source {
            fs: Arc::clone(&self.fs),
            archive: address.archive.clone(),
            location: member.location,
            size: member.entry.size,
            limits: self.limits,
        };
        let reader = source.open().map_err(|error| io_fail(&error, address))?;
        Ok(Box::new(EntryReader {
            source,
            entry: member.entry,
            reader: Mutex::new(reader),
            position: 0,
        }))
    }

    fn index(&self, archive: &AbsPath) -> FsResult<BTreeMap<Vec<String>, Member>> {
        let mut handle = self.fs.open_read(archive)?;
        let mut magic = [0_u8; MAGIC_LEN];
        let read = read_prefix(&mut handle, &mut magic).map_err(|error| io_at(&error, archive))?;
        handle
            .seek(SeekFrom::Start(0))
            .map_err(|error| io_at(&error, archive))?;
        let prefix = magic.get(..read).unwrap_or_default();
        let members = match detect(prefix) {
            Format::Zip => zip_members(handle, archive)?,
            Format::Tar(compression) => tar_members(handle, compression, self.limits, archive)?,
        };
        Ok(with_parents(members))
    }
}

impl Provider for ArchiveProvider {
    fn scheme(&self) -> ProviderScheme {
        ProviderScheme(SCHEME.to_owned())
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            writable: false,
            hierarchical: true,
            persistent: true,
        }
    }

    /// Content of a file member as [`ProviderValue::Binary`] with raw type 0;
    /// `None` for directories and missing members. Members larger than
    /// [`ArchiveLimits::max_value_size`] fail with
    /// [`FsErrorKind::Unsupported`] and are read with
    /// [`ArchiveProvider::open`].
    fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>> {
        let address = ArchiveAddress::from_key(key)?;
        let entry = match self.entry(&address) {
            Ok(entry) => entry,
            Err(error) if error.kind == FsErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if entry.kind != FileKind::File {
            return Ok(None);
        }
        if entry.size > self.limits.max_value_size {
            return Err(fail(FsErrorKind::Unsupported, &address));
        }
        let mut bytes = Vec::new();
        self.open(&address)?
            .read_to_end(&mut bytes)
            .map_err(|error| io_fail(&error, &address))?;
        Ok(Some(ProviderValue::Binary { raw_type: 0, bytes }))
    }

    /// Children of a directory member; directories are keys, other members
    /// are values addressed by their full path.
    fn list(&self, key: &ProviderKey) -> FsResult<Vec<ProviderChild>> {
        let address = ArchiveAddress::from_key(key)?;
        if self.entry(&address)?.kind != FileKind::Directory {
            return Err(fail(FsErrorKind::NotADirectory, &address));
        }
        let depth = address.inner.len() + 1;
        Ok(self
            .entries(&address.archive)?
            .into_iter()
            .filter(|entry| entry.path.len() == depth && entry.path.starts_with(&address.inner))
            .map(|entry| {
                let child = ArchiveAddress {
                    archive: address.archive.clone(),
                    inner: entry.path,
                };
                ProviderChild {
                    key: child.to_key(),
                    is_key: entry.kind == FileKind::Directory,
                }
            })
            .collect())
    }

    fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot> {
        let address = ArchiveAddress::from_key(key)?;
        let existed = match self.entry(&address) {
            Ok(_) => true,
            Err(error) if error.kind == FsErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        let value = if existed { self.get(key)? } else { None };
        Ok(ProviderSnapshot {
            key: key.clone(),
            existed,
            value,
            subtree: None,
        })
    }
}

fn invalid(text: &str) -> FsError {
    FsError::new(
        FsErrorKind::InvalidPath,
        OP,
        Some(std::path::Path::new(text)),
    )
}

fn fail(kind: FsErrorKind, address: &ArchiveAddress) -> FsError {
    let mut path = address.archive.as_path().to_path_buf();
    path.push(address.inner.join("/"));
    FsError::new(kind, OP, Some(&path))
}

fn not_found(address: &ArchiveAddress) -> FsError {
    fail(FsErrorKind::NotFound, address)
}

fn io_at(error: &dyn std::fmt::Display, archive: &AbsPath) -> FsError {
    let mut failure = FsError::new(FsErrorKind::Io, OP, Some(archive.as_path()));
    failure.op = format!("{OP}: {error}");
    failure
}

fn io_fail(error: &dyn std::fmt::Display, address: &ArchiveAddress) -> FsError {
    io_at(error, &address.archive)
}

/// Components of an inner name; `.` and empty components are dropped.
fn inner_components(raw: &[u8]) -> FsResult<Vec<String>> {
    let text = String::from_utf8_lossy(raw);
    let bytes = text.as_bytes();
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':');
    if text.starts_with('/') || text.contains('\\') || drive {
        return Err(invalid(&text));
    }
    let mut out = Vec::new();
    for component in text.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err(invalid(&text)),
            name => out.push(name.to_owned()),
        }
    }
    Ok(out)
}

fn read_prefix(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while let Some(rest) = buf.get_mut(filled..) {
        if rest.is_empty() {
            break;
        }
        match reader.read(rest)? {
            0 => break,
            count => filled += count,
        }
    }
    Ok(filled)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Gzip,
    Zstd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    Tar(Compression),
}

fn detect(prefix: &[u8]) -> Format {
    if prefix.starts_with(ZIP_MAGIC) {
        Format::Zip
    } else if prefix.starts_with(GZIP_MAGIC) {
        Format::Tar(Compression::Gzip)
    } else if prefix.starts_with(ZSTD_MAGIC) {
        Format::Tar(Compression::Zstd)
    } else {
        Format::Tar(Compression::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZipMethod {
    Stored,
    Deflated,
    Zstd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Location {
    Zip {
        start: u64,
        stored: u64,
        method: Option<ZipMethod>,
    },
    Tar {
        compression: Compression,
        position: u64,
    },
    Implied,
}

struct Member {
    entry: ArchiveEntry,
    location: Location,
}

fn directory(path: Vec<String>) -> ArchiveEntry {
    ArchiveEntry {
        path,
        kind: FileKind::Directory,
        size: 0,
        compressed_size: 0,
        modified_ns: None,
    }
}

fn with_parents(members: Vec<Member>) -> BTreeMap<Vec<String>, Member> {
    let mut parents = BTreeSet::new();
    for member in &members {
        let path = &member.entry.path;
        for depth in 1..path.len() {
            parents.insert(path.get(..depth).unwrap_or_default().to_vec());
        }
    }
    let mut out: BTreeMap<Vec<String>, Member> = parents
        .into_iter()
        .map(|path| {
            let member = Member {
                entry: directory(path.clone()),
                location: Location::Implied,
            };
            (path, member)
        })
        .collect();
    for member in members {
        if !member.entry.path.is_empty() {
            out.insert(member.entry.path.clone(), member);
        }
    }
    out
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn zip_time(time: zip::DateTime) -> i64 {
    let days = days_from_civil(
        i64::from(time.year()),
        i64::from(time.month()),
        i64::from(time.day()),
    );
    let seconds = days * SECONDS_PER_DAY
        + i64::from(time.hour()) * 3_600
        + i64::from(time.minute()) * 60
        + i64::from(time.second());
    seconds * NANOS_PER_SECOND
}

fn zip_method(method: zip::CompressionMethod) -> Option<ZipMethod> {
    match method {
        zip::CompressionMethod::Stored => Some(ZipMethod::Stored),
        zip::CompressionMethod::Deflated => Some(ZipMethod::Deflated),
        zip::CompressionMethod::Zstd => Some(ZipMethod::Zstd),
        _ => None,
    }
}

fn zip_members(handle: Box<dyn ReadHandle>, archive: &AbsPath) -> FsResult<Vec<Member>> {
    let mut zip = zip::ZipArchive::new(handle).map_err(|error| io_at(&error, archive))?;
    let mut members = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        let file = zip
            .by_index_raw(index)
            .map_err(|error| io_at(&error, archive))?;
        let path = inner_components(file.name_raw())?;
        let entry = ArchiveEntry {
            path,
            kind: if file.is_dir() {
                FileKind::Directory
            } else {
                FileKind::File
            },
            size: file.size(),
            compressed_size: file.compressed_size(),
            modified_ns: file.last_modified().map(zip_time),
        };
        let start = file
            .data_start()
            .ok_or_else(|| io_at(&"member data offset is unknown", archive))?;
        let location = Location::Zip {
            start,
            stored: file.compressed_size(),
            method: zip_method(file.compression()),
        };
        members.push(Member { entry, location });
    }
    Ok(members)
}

fn tar_kind(kind: tar::EntryType) -> FileKind {
    if kind.is_dir() {
        FileKind::Directory
    } else if kind.is_symlink() {
        FileKind::Symlink
    } else if kind.is_file() || kind == tar::EntryType::Continuous || kind.is_gnu_sparse() {
        FileKind::File
    } else {
        FileKind::Special
    }
}

fn tar_members(
    handle: Box<dyn ReadHandle>,
    compression: Compression,
    limits: ArchiveLimits,
    archive: &AbsPath,
) -> FsResult<Vec<Member>> {
    let stream =
        decompressed(handle, compression, limits).map_err(|error| io_at(&error, archive))?;
    let mut tar = tar::Archive::new(stream);
    let mut members = Vec::new();
    for entry in tar.entries().map_err(|error| io_at(&error, archive))? {
        let entry = entry.map_err(|error| io_at(&error, archive))?;
        let kind = tar_kind(entry.header().entry_type());
        let size = if kind == FileKind::File {
            entry.size()
        } else {
            0
        };
        let modified_ns = entry
            .header()
            .mtime()
            .ok()
            .and_then(|seconds| i64::try_from(seconds).ok())
            .map(|seconds| seconds.saturating_mul(NANOS_PER_SECOND));
        let member = Member {
            entry: ArchiveEntry {
                path: inner_components(&entry.path_bytes())?,
                kind,
                size,
                compressed_size: size,
                modified_ns,
            },
            location: Location::Tar {
                compression,
                position: entry.raw_file_position(),
            },
        };
        members.push(member);
    }
    Ok(members)
}

/// Counts the bytes consumed from the archive file.
struct Counted<R> {
    inner: R,
    consumed: Arc<AtomicU64>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buf)?;
        self.consumed
            .fetch_add(u64::try_from(count).unwrap_or(u64::MAX), Ordering::Relaxed);
        Ok(count)
    }
}

/// Stops a decompressed stream that exceeds the limits.
struct Bounded<R> {
    inner: R,
    consumed: Arc<AtomicU64>,
    produced: u64,
    limit: u64,
    limits: ArchiveLimits,
}

impl<R: Read> Read for Bounded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buf)?;
        self.produced = self
            .produced
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        let consumed = self.consumed.load(Ordering::Relaxed).max(1);
        let ratio_exceeded = self.produced > self.limits.ratio_floor
            && self.produced / consumed > self.limits.max_ratio;
        if self.produced > self.limit || ratio_exceeded {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive member expands beyond the decompression limit; raise the archive limits to read it",
            ));
        }
        Ok(count)
    }
}

type Stream = Box<dyn Read + Send>;

fn bounded(inner: Stream, consumed: Arc<AtomicU64>, limit: u64, limits: ArchiveLimits) -> Stream {
    Box::new(Bounded {
        inner,
        consumed,
        produced: 0,
        limit: limit.min(limits.max_expanded),
        limits,
    })
}

fn decompressed(
    handle: Box<dyn ReadHandle>,
    compression: Compression,
    limits: ArchiveLimits,
) -> io::Result<Stream> {
    let consumed = Arc::new(AtomicU64::new(0));
    let source = Counted {
        inner: handle,
        consumed: Arc::clone(&consumed),
    };
    let inner: Stream = match compression {
        Compression::None => Box::new(source),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(source)),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(source)?),
    };
    Ok(bounded(inner, consumed, limits.max_expanded, limits))
}

/// Recipe for reading one member from the start.
struct Source {
    fs: Arc<dyn Fs>,
    archive: AbsPath,
    location: Location,
    size: u64,
    limits: ArchiveLimits,
}

impl Source {
    fn open(&self) -> io::Result<Stream> {
        let mut handle = self.fs.open_read(&self.archive).map_err(io::Error::other)?;
        match self.location {
            Location::Zip {
                start,
                stored,
                method,
            } => {
                handle.seek(SeekFrom::Start(start))?;
                let consumed = Arc::new(AtomicU64::new(0));
                let raw = Counted {
                    inner: handle.take(stored),
                    consumed: Arc::clone(&consumed),
                };
                let inner: Stream = match method {
                    Some(ZipMethod::Stored) => Box::new(raw),
                    Some(ZipMethod::Deflated) => Box::new(flate2::read::DeflateDecoder::new(raw)),
                    Some(ZipMethod::Zstd) => Box::new(zstd::stream::read::Decoder::new(raw)?),
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "compression method of the member is not supported",
                        ));
                    }
                };
                Ok(bounded(inner, consumed, self.size, self.limits))
            }
            Location::Tar {
                compression,
                position,
            } => {
                let mut stream = decompressed(handle, compression, self.limits)?;
                skip(&mut stream, position)?;
                Ok(Box::new(stream.take(self.size)))
            }
            Location::Implied => Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "member is a directory",
            )),
        }
    }
}

fn skip(stream: &mut Stream, count: u64) -> io::Result<()> {
    let skipped = io::copy(&mut Read::by_ref(stream).take(count), &mut io::sink())?;
    if skipped == count {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "archive ends inside a member",
        ))
    }
}

/// Streaming reader of one member. Seeking backwards reopens the member.
struct EntryReader {
    source: Source,
    entry: ArchiveEntry,
    reader: Mutex<Stream>,
    position: u64,
}

impl Read for EntryReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let reader = self
            .reader
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let count = reader.read(buf)?;
        self.position += u64::try_from(count).unwrap_or(u64::MAX);
        Ok(count)
    }
}

impl Seek for EntryReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let (base, offset) = match target {
            SeekFrom::Start(position) => (position, 0),
            SeekFrom::End(offset) => (self.entry.size, offset),
            SeekFrom::Current(offset) => (self.position, offset),
        };
        let wanted = base.checked_add_signed(offset).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the member",
            )
        })?;
        if wanted < self.position {
            *self
                .reader
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner) = self.source.open()?;
            self.position = 0;
        }
        let reader = self
            .reader
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let mut ahead: Take<&mut Stream> = Read::by_ref(reader).take(wanted - self.position);
        self.position += io::copy(&mut ahead, &mut io::sink())?;
        Ok(self.position)
    }
}

impl ReadHandle for EntryReader {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(FileMeta {
            kind: self.entry.kind,
            size: self.entry.size,
            mode: if self.entry.kind == FileKind::Directory {
                DIR_MODE
            } else {
                ENTRY_MODE
            },
            uid: None,
            gid: None,
            links: 1,
            modified_ns: self.entry.modified_ns.unwrap_or(0),
            accessed_ns: None,
            created_ns: None,
            changed_ns: None,
            file_id: FileId {
                volume: 0,
                low: 0,
                high: 0,
            },
            windows_attributes: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use nurt_contracts::fs::AbsPath;
    use proptest::prelude::*;

    use super::*;
    use crate::mem::MemFs;

    fn p(text: &str) -> AbsPath {
        AbsPath::for_test(text)
    }

    fn provider_with(name: &str, bytes: &[u8]) -> ArchiveProvider {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p(name), bytes).expect("seed");
        ArchiveProvider::new(Arc::new(fs))
    }

    fn address(text: &str) -> ArchiveAddress {
        ArchiveAddress::parse(text).expect("address")
    }

    fn zip_bytes(method: zip::CompressionMethod, members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        writer.add_directory("docs/", options).expect("dir");
        for (name, content) in members {
            writer.start_file(*name, options).expect("start");
            writer.write_all(content).expect("write");
        }
        writer.finish().expect("finish").into_inner()
    }

    fn tar_bytes(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(content.len()).expect("size"));
            header.set_mode(0o644);
            header.set_mtime(1_700_000_000);
            header.set_cksum();
            builder
                .append_data(&mut header, name, *content)
                .expect("append");
        }
        builder.into_inner().expect("finish")
    }

    /// Members used by the format tests: one nested file and one top file.
    const MEMBERS: &[(&str, &[u8])] = &[
        ("docs/readme.txt", b"hello archive"),
        ("top.bin", b"\x00\x01\x02"),
    ];

    fn check_list_and_read(provider: &ArchiveProvider, archive: &str) {
        let root = address(&format!("archive:{archive}!"));
        let children = provider.list(&root.to_key()).expect("list");
        let names: Vec<(String, bool)> = children
            .iter()
            .map(|child| {
                (
                    child.key.segments.last().cloned().unwrap_or_default(),
                    child.is_key,
                )
            })
            .collect();
        assert_eq!(
            names,
            [("docs".to_owned(), true), ("top.bin".to_owned(), false)]
        );
        let readme = address(&format!("{archive}!/docs/readme.txt"));
        let entry = provider.entry(&readme).expect("entry");
        assert_eq!(entry.kind, FileKind::File);
        assert_eq!(entry.size, 13);
        assert!(entry.modified_ns.is_some());
        let mut reader = provider.open(&readme).expect("open");
        let mut head = [0_u8; 5];
        reader.read_exact(&mut head).expect("read");
        assert_eq!(&head, b"hello");
        reader.seek(SeekFrom::Start(1)).expect("seek back");
        let mut rest = String::new();
        reader.read_to_string(&mut rest).expect("read");
        assert_eq!(rest, "ello archive");
        assert_eq!(reader.seek(SeekFrom::End(-7)).expect("seek"), 6);
        assert_eq!(reader.meta().expect("meta").size, 13);
        check_values_and_errors(provider, archive);
    }

    fn check_values_and_errors(provider: &ArchiveProvider, archive: &str) {
        let readme = address(&format!("{archive}!/docs/readme.txt"));
        assert_eq!(
            provider.get(&readme.to_key()).expect("get"),
            Some(ProviderValue::Binary {
                raw_type: 0,
                bytes: b"hello archive".to_vec()
            })
        );
        let docs = address(&format!("{archive}!/docs"));
        assert_eq!(provider.get(&docs.to_key()).expect("get"), None);
        assert_eq!(provider.list(&docs.to_key()).expect("list").len(), 1);
        let missing = address(&format!("{archive}!/missing"));
        let snapshot = provider.snapshot(&missing.to_key()).expect("snapshot");
        assert!(!snapshot.existed);
        assert!(
            provider
                .snapshot(&readme.to_key())
                .expect("snapshot")
                .existed
        );
        assert_eq!(provider.get(&missing.to_key()).expect("get"), None);
        assert_eq!(
            provider.open(&missing).err().map(|error| error.kind),
            Some(FsErrorKind::NotFound)
        );
        assert_eq!(
            provider.open(&docs).err().map(|error| error.kind),
            Some(FsErrorKind::IsADirectory)
        );
        assert_eq!(
            provider
                .list(&readme.to_key())
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::NotADirectory)
        );
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn zip_members_are_listed_and_streamed() {
        for method in [
            zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated,
            zip::CompressionMethod::Zstd,
        ] {
            let provider = provider_with("/a.zip", &zip_bytes(method, MEMBERS));
            check_list_and_read(&provider, "/a.zip");
        }
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn tar_gz_members_are_listed_and_streamed() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes(MEMBERS)).expect("compress");
        let provider = provider_with("/a.tar.gz", &encoder.finish().expect("finish"));
        check_list_and_read(&provider, "/a.tar.gz");
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn tar_zst_and_plain_tar_members_are_listed_and_streamed() {
        let plain = tar_bytes(MEMBERS);
        let compressed = zstd::encode_all(plain.as_slice(), 3).expect("compress");
        check_list_and_read(&provider_with("/a.tar.zst", &compressed), "/a.tar.zst");
        check_list_and_read(&provider_with("/a.tar", &plain), "/a.tar");
    }

    #[test]
    fn unsafe_member_names_are_rejected() {
        for name in ["../escape", "/abs/path", "dir\\file", "C:/x", "a/../../b"] {
            assert_eq!(
                inner_components(name.as_bytes())
                    .err()
                    .map(|error| error.kind),
                Some(FsErrorKind::InvalidPath),
                "{name}"
            );
        }
        assert_eq!(inner_components(b"./a//b/").expect("name"), ["a", "b"]);
        assert_eq!(inner_components(b"1:x").expect("name"), ["1:x"]);
        assert_eq!(inner_components(b"dir/c:x").expect("name"), ["dir", "c:x"]);
        assert!(inner_components(b"z:x").is_err());
        let provider = provider_with(
            "/evil.zip",
            &zip_bytes(zip::CompressionMethod::Stored, &[("../evil", b"x")]),
        );
        assert_eq!(
            provider
                .entries(&p("/evil.zip"))
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::InvalidPath)
        );
        assert_eq!(
            ArchiveAddress::parse("/a.zip!/../x")
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::InvalidPath)
        );
        assert_eq!(
            ArchiveAddress::parse("/a.zip")
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::InvalidPath)
        );
    }

    #[test]
    fn damaged_and_truncated_archives_fail_without_panic() {
        let zip = zip_bytes(zip::CompressionMethod::Deflated, MEMBERS);
        let half = zip.get(..zip.len() / 2).expect("half");
        let provider = provider_with("/cut.zip", half);
        assert_eq!(
            provider
                .entries(&p("/cut.zip"))
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::Io)
        );
        let mut tar = tar_bytes(MEMBERS);
        if let Some(byte) = tar.get_mut(150) {
            *byte ^= 0xff;
        }
        let provider = provider_with("/bad.tar", &tar);
        assert_eq!(
            provider
                .entries(&p("/bad.tar"))
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::Io)
        );
        let gz = provider_with("/bad.tar.gz", &[0x1f, 0x8b, 0, 1, 2, 3]);
        assert!(gz.entries(&p("/bad.tar.gz")).is_err());
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn decompression_beyond_the_limits_fails() {
        let zeros = vec![0_u8; 4 * 1024 * 1024];
        let bomb = zip_bytes(zip::CompressionMethod::Deflated, &[("zeros", &zeros)]);
        let limits = ArchiveLimits {
            max_ratio: 100,
            ratio_floor: 64 * 1024,
            ..ArchiveLimits::default()
        };
        let provider = provider_with("/bomb.zip", &bomb).with_limits(limits);
        let mut reader = provider.open(&address("/bomb.zip!/zeros")).expect("open");
        let error = reader.read_to_end(&mut Vec::new()).expect_err("limit");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            provider
                .get(&address("/bomb.zip!/zeros").to_key())
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::Unsupported)
        );
        let small = ArchiveLimits {
            max_expanded: 1024,
            ..ArchiveLimits::default()
        };
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(&tar_bytes(&[("zeros", &zeros)]))
            .expect("compress");
        let provider =
            provider_with("/bomb.tar.gz", &encoder.finish().expect("finish")).with_limits(small);
        assert_eq!(
            provider
                .entries(&p("/bomb.tar.gz"))
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::Io)
        );
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn addresses_round_trip_through_provider_keys() {
        let parsed = address("archive:/srv/a.zip!/dir/file");
        assert_eq!(
            ArchiveAddress::from_key(&parsed.to_key()).expect("key"),
            parsed
        );
        assert_eq!(address("/srv/a.zip!").inner, Vec::<String>::new());
        let mut other = parsed.to_key();
        other.scheme = ProviderScheme("registry".to_owned());
        assert!(ArchiveAddress::from_key(&other).is_err());
        other.segments.clear();
        assert!(ArchiveAddress::from_key(&other).is_err());
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        let provider = provider_with("/x", b"");
        assert!(format!("{provider:?}").starts_with("ArchiveProvider"));
        assert_eq!(
            provider.entry(&address("/x!")).expect("root").kind,
            FileKind::Directory
        );
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn tar_links_are_listed_as_links_and_cannot_be_opened() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        builder
            .append_link(&mut header, "link", "target")
            .expect("link");
        let provider = provider_with("/l.tar", &builder.into_inner().expect("finish"));
        let entries = provider.entries(&p("/l.tar")).expect("entries");
        assert_eq!(
            entries.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
            [FileKind::Symlink]
        );
        assert_eq!(
            provider
                .open(&address("/l.tar!/link"))
                .err()
                .map(|error| error.kind),
            Some(FsErrorKind::Unsupported)
        );
        assert_eq!(
            provider
                .get(&address("/l.tar!/link").to_key())
                .expect("get"),
            None
        );
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn multi_byte_member_names_are_listed_and_read() {
        let name = "za\u{17c}\u{f3}\u{142}\u{107}/\u{1f600}.txt";
        let provider = provider_with("/m.tar", &tar_bytes(&[(name, b"utf-8")]));
        let target = address(&format!("/m.tar!/{name}"));
        assert_eq!(
            target.inner,
            ["za\u{17c}\u{f3}\u{142}\u{107}", "\u{1f600}.txt"]
        );
        assert_eq!(
            provider.get(&target.to_key()).expect("get"),
            Some(ProviderValue::Binary {
                raw_type: 0,
                bytes: b"utf-8".to_vec()
            })
        );
        assert_eq!(
            inner_components(b"a\xffb").expect("lossy name"),
            ["a\u{fffd}b"]
        );
    }

    #[test]
    fn default_limits_are_one_mebibyte_sixty_four_gibibytes_and_ratio_one_thousand() {
        let limits = ArchiveLimits::default();
        assert_eq!(limits.max_value_size, 1 << 20);
        assert_eq!(limits.max_expanded, 64 << 30);
        assert_eq!((limits.max_ratio, limits.ratio_floor), (1_000, 1 << 20));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            for name in ["/r.zip", "/r.tar"] {
                let provider = provider_with(name, &bytes);
                if let Ok(entries) = provider.entries(&p(name)) {
                    for entry in entries.into_iter().filter(|entry| entry.kind == FileKind::File) {
                        let target = ArchiveAddress { archive: p(name), inner: entry.path };
                        if let Ok(mut reader) = provider.open(&target) {
                            let _ = reader.read_to_end(&mut Vec::new());
                        }
                    }
                }
            }
        }
    }
}
