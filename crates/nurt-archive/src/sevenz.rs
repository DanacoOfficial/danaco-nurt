// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading and creating 7z archives.
//!
//! Entry paths are returned exactly as stored in the archive. The caller
//! must pass every path through the path safety checks of the crate before
//! it joins the path to a destination.

use std::fmt;
use std::io::{self, Read, Seek, Write};
use std::time::{Duration, SystemTime};

use sevenz_rust2::{ArchiveEntry, ArchiveReader, ArchiveWriter, NtTime, Password};

/// Upper bound for the memory a decoder may reserve, in kibibytes.
const MEMORY_LIMIT_KB: usize = 1024 * 1024;

/// Windows attribute bit that marks the high 16 bits as a Unix mode.
const ATTRIBUTE_UNIX_EXTENSION: u32 = 0x8000;
/// Windows attribute of a directory.
const ATTRIBUTE_DIRECTORY: u32 = 0x10;
/// Windows attribute of an ordinary file.
const ATTRIBUTE_ARCHIVE: u32 = 0x20;
/// Number of bits the Unix mode is shifted by inside the attribute word.
const MODE_SHIFT: u32 = 16;
/// Mask of the file type bits of a Unix mode.
const MODE_TYPE_MASK: u32 = 0o170_000;
/// File type bits of a symbolic link.
const MODE_SYMLINK: u32 = 0o120_000;
/// Mask of the permission bits of a Unix mode.
const MODE_PERMISSIONS: u32 = 0o7777;

/// Seconds between 1601-01-01 and 1970-01-01.
const NT_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;
/// Number of NT file time ticks per second.
const NT_TICKS_PER_SEC: u64 = 10_000_000;
/// Nanoseconds per NT file time tick.
const NANOS_PER_NT_TICK: u64 = 100;

/// Failure while reading or creating a 7z archive.
///
/// [`Error::Format`] maps to `E_FORMAT`, [`Error::Encrypted`] to
/// `E_ENCRYPTED`, [`Error::Limit`] to `E_LIMIT` and [`Error::Io`] to `E_IO`.
#[derive(Debug)]
pub enum Error {
    /// The data is not a valid 7z archive, is damaged, or uses an unsupported method.
    Format(String),
    /// The archive or its content is encrypted.
    Encrypted,
    /// The archive declares more data than the caller permits.
    Limit {
        /// Maximum number of bytes the caller permits.
        limit: u64,
    },
    /// Reading the source or writing the destination failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(reason) => write!(f, "invalid 7z archive: {reason}"),
            Self::Encrypted => f.write_str("7z archive is encrypted; decrypt it first"),
            Self::Limit { limit } => write!(
                f,
                "7z content exceeds {limit} bytes; raise the limit or check the source"
            ),
            Self::Io(error) => write!(f, "i/o failure: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<sevenz_rust2::Error> for Error {
    fn from(error: sevenz_rust2::Error) -> Self {
        use sevenz_rust2::Error as Source;
        match error {
            Source::PasswordRequired | Source::MaybeBadPassword(_) => Self::Encrypted,
            Source::MaxMemLimited { max_kb, .. } => Self::Limit {
                limit: (max_kb as u64).saturating_mul(1024),
            },
            Source::Io(inner, context) => match inner.kind() {
                io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData => {
                    Self::Format(describe(&inner, &context))
                }
                _ => Self::Io(inner),
            },
            Source::FileOpen(inner, _) => Self::Io(inner),
            other => Self::Format(other.to_string()),
        }
    }
}

fn describe(error: &io::Error, context: &str) -> String {
    if context.is_empty() {
        error.to_string()
    } else {
        format!("{context}: {error}")
    }
}

/// Kind of an archive entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link; the content of the entry is the link target.
    Symlink,
}

/// Metadata of one archive entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path as stored in the archive, not checked or normalised.
    pub path: String,
    /// Last component of the stored path.
    pub name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Compressed size in bytes; zero when the archive does not record it per entry.
    pub packed_size: u64,
    /// Modification time, when the archive records one.
    pub modified: Option<SystemTime>,
    /// Unix permission bits, when the archive records them.
    pub mode: Option<u32>,
    /// Kind of the entry.
    pub kind: EntryKind,
    /// Whether the content is encrypted.
    pub encrypted: bool,
}

/// Modification time and permission bits for a new entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metadata {
    /// Modification time to store.
    pub modified: Option<SystemTime>,
    /// Unix permission bits to store; bits above `0o7777` are ignored.
    pub mode: Option<u32>,
}

fn to_system_time(time: NtTime) -> Option<SystemTime> {
    let ticks = u64::from(time);
    let seconds = (ticks / NT_TICKS_PER_SEC).checked_sub(NT_EPOCH_OFFSET_SECS)?;
    let nanos = u32::try_from((ticks % NT_TICKS_PER_SEC) * NANOS_PER_NT_TICK).ok()?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(seconds, nanos))
}

fn convert(entry: &ArchiveEntry, encrypted: bool) -> Entry {
    let unix_mode = (entry.has_windows_attributes
        && entry.windows_attributes & ATTRIBUTE_UNIX_EXTENSION != 0)
        .then_some(entry.windows_attributes >> MODE_SHIFT);
    let kind = if entry.is_directory {
        EntryKind::Directory
    } else if unix_mode.is_some_and(|mode| mode & MODE_TYPE_MASK == MODE_SYMLINK) {
        EntryKind::Symlink
    } else {
        EntryKind::File
    };
    let path = entry.name.clone();
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    Entry {
        name,
        path,
        size: entry.size,
        packed_size: entry.compressed_size,
        modified: entry
            .has_last_modified_date
            .then(|| to_system_time(entry.last_modified_date))
            .flatten(),
        mode: unix_mode.map(|mode| mode & MODE_PERMISSIONS),
        kind,
        encrypted,
    }
}

fn open<R: Read + Seek>(source: R) -> Result<ArchiveReader<R>, Error> {
    Ok(ArchiveReader::new_with_memory_limit(
        source,
        Password::empty(),
        MEMORY_LIMIT_KB,
    )?)
}

/// Lists the entries of a 7z archive in archive order.
///
/// Markers of deleted entries are omitted.
///
/// # Errors
///
/// Returns [`Error::Format`] for data that is not a valid 7z archive,
/// [`Error::Encrypted`] when the header is encrypted and [`Error::Io`] when
/// reading fails. Entries with encrypted content are listed with
/// [`Entry::encrypted`] set.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
/// use nurt_archive::sevenz::{list, Creator, Metadata};
///
/// let mut creator = Creator::new(Cursor::new(Vec::new()))?;
/// creator.add_file("a/b.txt", Metadata::default(), &b"hello"[..])?;
/// let archive = creator.finish()?.into_inner();
/// let entries = list(Cursor::new(archive))?;
/// assert_eq!(entries[0].path, "a/b.txt");
/// assert_eq!(entries[0].size, 5);
/// # Ok::<(), nurt_archive::sevenz::Error>(())
/// ```
pub fn list<R: Read + Seek>(source: R) -> Result<Vec<Entry>, Error> {
    let reader = open(source)?;
    let archive = reader.archive();
    Ok(archive
        .files
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.is_anti_item)
        .map(|(index, entry)| convert(entry, archive.is_file_encrypted(index)))
        .collect())
}

/// Reads the content of every entry and passes it to `each`.
///
/// Entries are visited in archive order, directories included with empty
/// content. Nothing is written to disk. The callback may stop the traversal by
/// returning an error. When `limit` is set, the call fails before the first
/// callback if the declared sizes of all entries add up to more than `limit`.
///
/// # Errors
///
/// Returns [`Error::Encrypted`] when any entry is encrypted,
/// [`Error::Limit`] when the declared total size exceeds `limit`,
/// [`Error::Format`] for damaged data or a failed checksum, [`Error::Io`] for
/// read failures, and any error returned by `each`.
///
/// # Examples
///
/// ```
/// use std::io::{Cursor, Read};
/// use nurt_archive::sevenz::{read_entries, Creator, Metadata};
///
/// let mut creator = Creator::new(Cursor::new(Vec::new()))?;
/// creator.add_file("note.txt", Metadata::default(), &b"text"[..])?;
/// let archive = creator.finish()?.into_inner();
/// let mut total = 0;
/// read_entries(Cursor::new(archive), None, |_, content| {
///     let mut bytes = Vec::new();
///     content.read_to_end(&mut bytes)?;
///     total += bytes.len();
///     Ok(())
/// })?;
/// assert_eq!(total, 4);
/// # Ok::<(), nurt_archive::sevenz::Error>(())
/// ```
pub fn read_entries<R, F>(source: R, limit: Option<u64>, mut each: F) -> Result<(), Error>
where
    R: Read + Seek,
    F: FnMut(&Entry, &mut dyn Read) -> Result<(), Error>,
{
    let mut reader = open(source)?;
    let archive = reader.archive();
    let streamed = archive
        .files
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.has_stream);
    let mut declared: u64 = 0;
    for (index, entry) in streamed {
        if archive.is_file_encrypted(index) {
            return Err(Error::Encrypted);
        }
        declared = declared.saturating_add(entry.size);
    }
    if let Some(limit) = limit
        && declared > limit
    {
        return Err(Error::Limit { limit });
    }

    let mut failure: Option<Error> = None;
    let outcome = reader.for_each_entries(|entry, content| {
        if entry.is_anti_item {
            return Ok(true);
        }
        match each(&convert(entry, false), content) {
            Ok(()) => Ok(true),
            Err(error) => {
                failure = Some(error);
                Err(sevenz_rust2::Error::Unsupported("stopped by caller".into()))
            }
        }
    });
    match (failure, outcome) {
        (Some(error), _) => Err(error),
        (None, outcome) => Ok(outcome?),
    }
}

/// Builder that writes a 7z archive into a seekable destination.
///
/// Content is compressed with LZMA2, one block per entry. The destination
/// must support seeking because the header is written last; an in-memory
/// [`std::io::Cursor`] serves callers that only have a plain writer.
/// Entry paths are stored as given and are not checked.
pub struct Creator<W: Write + Seek> {
    writer: ArchiveWriter<W>,
}

impl<W: Write + Seek> Creator<W> {
    /// Starts an archive in `destination`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the destination cannot be written.
    pub fn new(destination: W) -> Result<Self, Error> {
        Ok(Self {
            writer: ArchiveWriter::new(destination)?,
        })
    }

    /// Adds a regular file whose content is read from `content`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when reading `content` or writing the archive fails.
    pub fn add_file<R: Read>(
        &mut self,
        path: &str,
        metadata: Metadata,
        content: R,
    ) -> Result<(), Error> {
        let entry = new_entry(path, metadata, false, ATTRIBUTE_ARCHIVE, 0);
        self.writer.push_archive_entry(entry, Some(content))?;
        Ok(())
    }

    /// Adds a directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when writing the archive fails.
    pub fn add_directory(&mut self, path: &str, metadata: Metadata) -> Result<(), Error> {
        let entry = new_entry(path, metadata, true, ATTRIBUTE_DIRECTORY, 0);
        self.writer.push_archive_entry(entry, None::<io::Empty>)?;
        Ok(())
    }

    /// Adds a symbolic link that points to `target`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when writing the archive fails.
    pub fn add_symlink(
        &mut self,
        path: &str,
        target: &str,
        metadata: Metadata,
    ) -> Result<(), Error> {
        let entry = new_entry(path, metadata, false, ATTRIBUTE_ARCHIVE, MODE_SYMLINK);
        self.writer
            .push_archive_entry(entry, Some(target.as_bytes()))?;
        Ok(())
    }

    /// Writes the header and returns the destination.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the header cannot be written.
    pub fn finish(self) -> Result<W, Error> {
        Ok(self.writer.finish()?)
    }
}

fn new_entry(
    path: &str,
    metadata: Metadata,
    directory: bool,
    base_attributes: u32,
    type_bits: u32,
) -> ArchiveEntry {
    let mut entry = if directory {
        ArchiveEntry::new_directory(path)
    } else {
        ArchiveEntry::new_file(path)
    };
    if let Some(time) = metadata.modified.and_then(|t| NtTime::try_from(t).ok()) {
        entry.last_modified_date = time;
        entry.has_last_modified_date = true;
    }
    let mode = metadata.mode.map(|mode| mode & MODE_PERMISSIONS);
    if mode.is_some() || type_bits != 0 {
        let unix = type_bits | mode.unwrap_or_default();
        entry.has_windows_attributes = true;
        entry.windows_attributes =
            base_attributes | ATTRIBUTE_UNIX_EXTENSION | (unix << MODE_SHIFT);
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::time::UNIX_EPOCH;

    use sevenz_rust2::encoder_options::AesEncoderOptions;
    use sevenz_rust2::{EncoderConfiguration, EncoderMethod};

    fn build(populate: impl FnOnce(&mut Creator<Cursor<Vec<u8>>>)) -> Vec<u8> {
        let mut creator = Creator::new(Cursor::new(Vec::new())).unwrap();
        populate(&mut creator);
        creator.finish().unwrap().into_inner()
    }

    fn collect(archive: &[u8], limit: Option<u64>) -> Result<Vec<(Entry, Vec<u8>)>, Error> {
        let mut seen = Vec::new();
        read_entries(Cursor::new(archive), limit, |entry, content| {
            let mut bytes = Vec::new();
            content.read_to_end(&mut bytes)?;
            seen.push((entry.clone(), bytes));
            Ok(())
        })?;
        Ok(seen)
    }

    fn encrypted_archive(encrypt_header: bool) -> Vec<u8> {
        let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.set_content_methods(vec![
            AesEncoderOptions::new("secret".into()).into(),
            EncoderConfiguration::new(EncoderMethod::LZMA2),
        ]);
        writer.set_encrypt_header(encrypt_header);
        writer
            .push_archive_entry(
                ArchiveEntry::new_file("hidden.txt"),
                Some(&b"classified"[..]),
            )
            .unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn lists_files_directories_and_symlinks_with_metadata() {
        let modified = UNIX_EPOCH + Duration::new(1_700_000_000, 500_000_000);
        let archive = build(|creator| {
            creator.add_directory("dir", Metadata::default()).unwrap();
            creator
                .add_file(
                    "dir/file.txt",
                    Metadata {
                        modified: Some(modified),
                        mode: Some(0o640),
                    },
                    &b"content"[..],
                )
                .unwrap();
            creator
                .add_symlink("dir/link", "file.txt", Metadata::default())
                .unwrap();
        });
        let entries = list(Cursor::new(archive)).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, EntryKind::Directory);
        assert_eq!(entries[1].path, "dir/file.txt");
        assert_eq!(entries[1].name, "file.txt");
        assert_eq!(entries[1].size, 7);
        assert_eq!(entries[1].mode, Some(0o640));
        assert_eq!(entries[1].modified, Some(modified));
        assert_eq!(entries[1].kind, EntryKind::File);
        assert!(!entries[1].encrypted);
        assert_eq!(entries[2].kind, EntryKind::Symlink);
    }

    #[test]
    fn reads_entry_content_in_archive_order() {
        let archive = build(|creator| {
            creator
                .add_file("a", Metadata::default(), &b"alpha"[..])
                .unwrap();
            creator
                .add_file("b", Metadata::default(), &b""[..])
                .unwrap();
            creator.add_symlink("c", "a", Metadata::default()).unwrap();
        });
        let seen = collect(&archive, None).unwrap();
        let contents: Vec<(&str, &[u8])> = seen
            .iter()
            .map(|(entry, bytes)| (entry.path.as_str(), bytes.as_slice()))
            .collect();
        assert_eq!(
            contents,
            vec![("a", &b"alpha"[..]), ("b", &b""[..]), ("c", &b"a"[..])]
        );
    }

    #[test]
    fn returns_hostile_paths_unchanged() {
        let archive = build(|creator| {
            creator
                .add_file("../../etc/passwd", Metadata::default(), &b"x"[..])
                .unwrap();
        });
        let entries = list(Cursor::new(archive)).unwrap();
        assert_eq!(entries[0].path, "../../etc/passwd");
        assert_eq!(entries[0].name, "passwd");
    }

    #[test]
    fn empty_archive_lists_no_entries() {
        let archive = build(|_| {});
        assert!(list(Cursor::new(archive)).unwrap().is_empty());
    }

    #[test]
    fn refuses_content_above_the_limit_before_reading() {
        let archive = build(|creator| {
            creator
                .add_file("big", Metadata::default(), &vec![0u8; 10_000][..])
                .unwrap();
        });
        let mut called = false;
        let error = read_entries(Cursor::new(&archive), Some(9_999), |_, _| {
            called = true;
            Ok(())
        })
        .unwrap_err();
        assert!(matches!(error, Error::Limit { limit: 9_999 }));
        assert!(!called);
        assert_eq!(collect(&archive, Some(10_000)).unwrap().len(), 1);
    }

    #[test]
    fn callback_error_stops_the_traversal_and_is_returned() {
        let archive = build(|creator| {
            creator
                .add_file("a", Metadata::default(), &b"1"[..])
                .unwrap();
            creator
                .add_file("b", Metadata::default(), &b"2"[..])
                .unwrap();
        });
        let mut visited = 0;
        let error = read_entries(Cursor::new(archive), None, |_, _| {
            visited += 1;
            Err(Error::Io(io::Error::other("destination full")))
        })
        .unwrap_err();
        assert!(matches!(error, Error::Io(_)));
        assert_eq!(visited, 1);
    }

    #[test]
    fn encrypted_content_is_listed_and_refused_on_read() {
        let archive = encrypted_archive(false);
        let entries = list(Cursor::new(&archive)).unwrap();
        assert!(entries[0].encrypted);
        assert!(matches!(collect(&archive, None), Err(Error::Encrypted)));
    }

    #[test]
    fn encrypted_header_is_reported_as_encrypted() {
        let archive = encrypted_archive(true);
        assert!(matches!(list(Cursor::new(&archive)), Err(Error::Encrypted)));
        assert!(matches!(collect(&archive, None), Err(Error::Encrypted)));
    }

    #[test]
    fn rejects_data_that_is_not_an_archive() {
        for data in [&b""[..], b"not a 7z archive at all, just text"] {
            let error = list(Cursor::new(data)).unwrap_err();
            assert!(matches!(error, Error::Format(_)), "{data:?}");
        }
    }

    #[test]
    fn rejects_truncated_archives() {
        let archive = build(|creator| {
            creator
                .add_file("a", Metadata::default(), &vec![1u8; 5_000][..])
                .unwrap();
        });
        for keep in [archive.len() - 1, archive.len() / 2, 20] {
            let error = collect(&archive[..keep], None).unwrap_err();
            assert!(matches!(error, Error::Format(_)), "kept {keep} bytes");
        }
    }

    #[test]
    fn detects_corrupted_content() {
        let data: Vec<u8> = (0..20_000u32)
            .map(|n| (n.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let mut archive = build(|creator| {
            creator
                .add_file("a", Metadata::default(), data.as_slice())
                .unwrap();
        });
        archive[100] ^= 0xff;
        assert!(collect(&archive, None).is_err());
    }
}
