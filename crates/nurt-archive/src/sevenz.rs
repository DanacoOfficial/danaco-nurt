// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-archive::sevenz
// Responsibility:   Reading and creating 7z archives.
// Dependencies:     nurt_contracts, sevenz_rust2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reading and creating 7z archives.
//!
//! Entry paths are returned exactly as stored in the archive. [`list`] and
//! [`read_entries`] do not check them; the listing, the extraction plan and
//! the content stream of [`crate::list`] and [`crate::extract`] pass them
//! through the checks of [`crate::safety`] like the paths of zip and tar
//! archives.
//!
//! Entries are visited in the order of the archive header. Content is
//! decoded one block at a time, and the content of an entry inside a solid
//! block is reached only by decoding the entries before it.

use std::borrow::Cow;
use std::fmt;
use std::io::{self, Read, Seek, Write};
use std::ops::Range;
use std::time::{Duration, SystemTime};

use nurt_contracts::value::Instant;
use sevenz_rust2::encoder_options::Lzma2Options;
use sevenz_rust2::{
    Archive, ArchiveEntry, ArchiveWriter, Block, BlockDecoder, EncoderMethod, NtTime, Password,
};

use crate::codec;
use crate::list::{ArchiveError, Body, Flow, Format, Member, MemberKind, Visitor};

/// Lowest and highest compression level accepted by [`Creator::with_level`]:
/// the LZMA2 presets of the 7z format.
pub const LEVELS: (u32, u32) = (0, 9);

/// Upper bound for the memory a decoder may reserve, in kibibytes.
const MEMORY_LIMIT_KB: usize = 1024 * 1024;
/// Number of bytes in a kibibyte.
const BYTES_PER_KB: u64 = 1024;
/// Content is decoded on the calling thread.
const DECODER_THREADS: u32 = 1;
/// Longest symbolic link target read from an archive, in bytes.
const MAX_LINK_TARGET: u64 = 4096;
/// Reason passed to the decoder when a callback ends the traversal.
const STOPPED_BY_CALLER: &str = "stopped by caller";

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
    /// The data is not a valid 7z archive, is damaged, fails a checksum, or
    /// uses an unsupported method.
    Format(String),
    /// The archive or its content is encrypted.
    Encrypted,
    /// The archive declares more data than the caller permits, or a decoder
    /// needs more memory than permitted.
    Limit {
        /// Maximum number of bytes permitted.
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
                limit: (max_kb as u64).saturating_mul(BYTES_PER_KB),
            },
            Source::Io(inner, context) => {
                // Checksum failures of the decoder reach the caller as I/O
                // errors that wrap the decoder's own error.
                let from_decoder = inner.get_ref().is_some_and(|source| source.is::<Source>());
                match inner.kind() {
                    io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::InvalidData
                    | io::ErrorKind::InvalidInput => Self::Format(describe(&inner, &context)),
                    _ if from_decoder => Self::Format(describe(&inner, &context)),
                    _ => Self::Io(inner),
                }
            }
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

/// Converts a read error into the error type of the archive commands.
fn read_error(error: Error) -> ArchiveError {
    match error {
        Error::Format(detail) => ArchiveError::format(Some(Format::SevenZ), detail),
        Error::Encrypted => ArchiveError::Encrypted {
            path: String::new(),
        },
        Error::Limit { limit } => ArchiveError::format(
            Some(Format::SevenZ),
            format!("decoding needs more than {limit} bytes of memory"),
        ),
        Error::Io(error) => ArchiveError::Io(error),
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

/// Converts a failure of reading or decoding an archive.
///
/// The decoders report damaged data as I/O errors of kind `Other`; while
/// an archive is read, such an error is a format error, as in the zip and
/// tar readers of the crate.
fn decode_failure(error: sevenz_rust2::Error) -> Error {
    match Error::from(error) {
        Error::Io(error) if error.kind() == io::ErrorKind::Other => {
            Error::Format(error.to_string())
        }
        other => other,
    }
}

fn read_archive<R: Read + Seek>(source: &mut R, password: &Password) -> Result<Archive, Error> {
    Archive::read_with_memory_limit(source, password, MEMORY_LIMIT_KB).map_err(decode_failure)
}

fn is_encrypted(block: &Block) -> bool {
    block
        .coders
        .iter()
        .any(|coder| coder.encoder_method_id() == EncoderMethod::ID_AES256_SHA256)
}

/// Names of the coders of a block in lower case, joined with `+`.
fn method_names(block: &Block) -> String {
    block
        .coders
        .iter()
        .filter_map(|coder| EncoderMethod::by_id(coder.encoder_method_id()))
        .map(|method| method.name().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("+")
}

/// Content passed to the callback of [`drive`].
enum Content<'c> {
    /// Decoded content; empty for directories and empty files.
    Data(&'c mut dyn Read),
    /// The entry belongs to an encrypted block, which is not decoded.
    Encrypted,
}

/// Failure of [`drive`]: from the decoder or from the callback.
enum DriveError<E> {
    Archive(sevenz_rust2::Error),
    Callback(E),
}

/// Passes every entry except deletion markers to `each`, in header order.
///
/// Entries outside blocks (directories and empty files between blocks) are
/// passed with empty content at their position in the header, unlike the
/// traversal of the decoder, which passes them after all blocks.
fn drive<R, E, F>(
    source: &mut R,
    archive: &Archive,
    password: &Password,
    mut each: F,
) -> Result<(), DriveError<E>>
where
    R: Read + Seek,
    F: FnMut(usize, &ArchiveEntry, Content<'_>) -> Result<Flow, E>,
{
    let mut next = 0;
    for block in 0..archive.blocks.len() {
        let Some(&start) = archive.stream_map.block_first_file_index.get(block) else {
            break;
        };
        let decoder = BlockDecoder::new_with_memory_limit(
            DECODER_THREADS,
            MEMORY_LIMIT_KB,
            block,
            archive,
            password,
            source,
        );
        let end = start + decoder.entry_count();
        if start == end {
            continue;
        }
        if visit_undecoded(archive, next..start, false, &mut each)? == Flow::Stop {
            return Ok(());
        }
        next = end;
        let flow = if is_encrypted(&archive.blocks[block]) {
            visit_undecoded(archive, start..end, true, &mut each)?
        } else {
            decode_block(decoder, start, &mut each)?
        };
        if flow == Flow::Stop {
            return Ok(());
        }
    }
    visit_undecoded(archive, next..archive.files.len(), false, &mut each).map(drop)
}

/// Passes entries without decoding: entries outside blocks with empty
/// content, and the entries of an encrypted block.
fn visit_undecoded<E, F>(
    archive: &Archive,
    indices: Range<usize>,
    encrypted: bool,
    each: &mut F,
) -> Result<Flow, DriveError<E>>
where
    F: FnMut(usize, &ArchiveEntry, Content<'_>) -> Result<Flow, E>,
{
    for index in indices {
        let Some(entry) = archive.files.get(index) else {
            break;
        };
        if entry.is_anti_item {
            continue;
        }
        let mut empty: &[u8] = &[];
        let content = if encrypted && entry.has_stream {
            Content::Encrypted
        } else {
            Content::Data(&mut empty)
        };
        if each(index, entry, content).map_err(DriveError::Callback)? == Flow::Stop {
            return Ok(Flow::Stop);
        }
    }
    Ok(Flow::Continue)
}

/// Decodes one block and passes its entries, starting at file index
/// `start`. Content a callback leaves unread is decoded and verified by the
/// decoder before the next entry.
fn decode_block<R, E, F>(
    decoder: BlockDecoder<'_, R>,
    start: usize,
    each: &mut F,
) -> Result<Flow, DriveError<E>>
where
    R: Read + Seek,
    F: FnMut(usize, &ArchiveEntry, Content<'_>) -> Result<Flow, E>,
{
    let mut index = start;
    let mut failure = None;
    let mut flow = Flow::Continue;
    let outcome = decoder.for_each_entries(&mut |entry, content| {
        let current = index;
        index += 1;
        if entry.is_anti_item {
            return Ok(true);
        }
        match each(current, entry, Content::Data(content)) {
            Ok(Flow::Continue) => Ok(true),
            Ok(Flow::Stop) => {
                flow = Flow::Stop;
                Ok(false)
            }
            Err(error) => {
                failure = Some(error);
                Err(sevenz_rust2::Error::Unsupported(STOPPED_BY_CALLER.into()))
            }
        }
    });
    if let Some(error) = failure {
        return Err(DriveError::Callback(error));
    }
    outcome.map_err(DriveError::Archive)?;
    Ok(flow)
}

/// Lists the entries of a 7z archive in archive order.
///
/// Markers of deleted entries are omitted. Only the header is read.
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
pub fn list<R: Read + Seek>(mut source: R) -> Result<Vec<Entry>, Error> {
    let archive = read_archive(&mut source, &Password::empty())?;
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
/// read failures, and any error returned by `each`. Damaged content can
/// also surface as an error of the content reader inside `each`, of kind
/// [`io::ErrorKind::InvalidData`] or [`io::ErrorKind::Other`].
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
pub fn read_entries<R, F>(mut source: R, limit: Option<u64>, mut each: F) -> Result<(), Error>
where
    R: Read + Seek,
    F: FnMut(&Entry, &mut dyn Read) -> Result<(), Error>,
{
    let password = Password::empty();
    let archive = read_archive(&mut source, &password)?;
    let mut declared: u64 = 0;
    for (index, entry) in archive.files.iter().enumerate() {
        if !entry.has_stream {
            continue;
        }
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
    drive(
        &mut source,
        &archive,
        &password,
        |_, entry, content| match content {
            Content::Data(content) => {
                each(&convert(entry, false), content).map(|()| Flow::Continue)
            }
            Content::Encrypted => Err(Error::Encrypted),
        },
    )
    .map_err(|error| match error {
        DriveError::Archive(error) => decode_failure(error),
        DriveError::Callback(error) => error,
    })
}

/// Content of a member passed to an archive visitor.
enum EntryBody<'c> {
    Content(&'c mut dyn Read),
    Encrypted(&'c str),
}

impl Body for EntryBody<'_> {
    fn open(&mut self) -> Result<&mut dyn Read, ArchiveError> {
        match self {
            Self::Content(content) => Ok(&mut **content),
            Self::Encrypted(path) => Err(ArchiveError::Encrypted {
                path: (*path).to_owned(),
            }),
        }
    }
}

fn to_member(entry: &Entry, raw: &ArchiveEntry) -> Member {
    let kind = match entry.kind {
        EntryKind::File => MemberKind::File,
        EntryKind::Directory => MemberKind::Directory,
        EntryKind::Symlink => MemberKind::Symlink,
    };
    let mut member = Member::new(Cow::Borrowed(entry.path.as_bytes()), kind);
    member.size = entry.size;
    member.packed = (entry.packed_size > 0).then_some(entry.packed_size);
    member.modified = entry.modified.and_then(Instant::from_system_time);
    member.mode = entry.mode;
    member.encrypted = entry.encrypted;
    member.crc32 = raw.has_crc.then(|| u32::try_from(raw.crc).ok()).flatten();
    member
}

fn read_link_target(content: &mut dyn Read, path: &str) -> Result<Vec<u8>, ArchiveError> {
    let mut target = Vec::new();
    content
        .take(MAX_LINK_TARGET + 1)
        .read_to_end(&mut target)
        .map_err(|error| ArchiveError::from_read(Format::SevenZ, error))?;
    if target.len() as u64 > MAX_LINK_TARGET {
        return Err(ArchiveError::format(
            Some(Format::SevenZ),
            format!("the link target of {path} is longer than {MAX_LINK_TARGET} bytes"),
        ));
    }
    Ok(target)
}

/// Passes every member of a 7z archive to the visitor of [`crate::list`].
///
/// The content of a regular file is limited to its declared size. The
/// target of a symbolic link is read from its content before the visitor
/// sees the member, so the path checks can evaluate it. Content the visitor
/// does not read is still decoded and its checksum verified, because a
/// solid block can only be read in order.
pub(crate) fn visit<R: Read + Seek>(
    mut source: R,
    visitor: &mut Visitor<'_>,
) -> Result<(), ArchiveError> {
    let password = Password::empty();
    let archive = read_archive(&mut source, &password).map_err(read_error)?;
    let methods: Vec<String> = archive.blocks.iter().map(method_names).collect();
    drive(&mut source, &archive, &password, |index, raw, content| {
        let encrypted = matches!(content, Content::Encrypted);
        let mut member = to_member(&convert(raw, encrypted), raw);
        member.compression = archive
            .stream_map
            .file_block_index
            .get(index)
            .copied()
            .flatten()
            .and_then(|block| methods.get(block))
            .filter(|names| !names.is_empty())
            .cloned();
        match content {
            Content::Encrypted => visitor(&member, &mut EntryBody::Encrypted(&member.path)),
            Content::Data(content) if member.kind == MemberKind::Symlink => {
                let target = read_link_target(content, &member.path)?;
                member.set_link_target(Cow::Owned(target));
                let mut empty = io::empty();
                visitor(&member, &mut EntryBody::Content(&mut empty))
            }
            Content::Data(content) => {
                let mut limited = codec::limited(content, member.size);
                visitor(&member, &mut EntryBody::Content(&mut limited))
            }
        }
    })
    .map_err(|error| match error {
        DriveError::Archive(error) => read_error(decode_failure(error)),
        DriveError::Callback(error) => error,
    })
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
    /// Starts an archive in `destination` at the default level of the format.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the destination cannot be written.
    pub fn new(destination: W) -> Result<Self, Error> {
        Ok(Self {
            writer: ArchiveWriter::new(destination)?,
        })
    }

    /// Starts an archive in `destination` compressed at `level`, an LZMA2
    /// preset in the range [`LEVELS`]; a higher level is treated as the
    /// highest one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the destination cannot be written.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    /// use nurt_archive::sevenz::{list, Creator, Metadata};
    ///
    /// let mut creator = Creator::with_level(Cursor::new(Vec::new()), 1)?;
    /// creator.add_file("a.txt", Metadata::default(), &b"aaaa"[..])?;
    /// let archive = creator.finish()?.into_inner();
    /// assert_eq!(list(Cursor::new(archive))?[0].size, 4);
    /// # Ok::<(), nurt_archive::sevenz::Error>(())
    /// ```
    pub fn with_level(destination: W, level: u32) -> Result<Self, Error> {
        let mut writer = ArchiveWriter::new(destination)?;
        writer.set_content_methods(vec![Lzma2Options::from_level(level.min(LEVELS.1)).into()]);
        Ok(Self { writer })
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

    use nurt_contracts::error::ErrorCode;
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

    fn members(archive: &[u8]) -> Result<Vec<Member>, ArchiveError> {
        let mut found = Vec::new();
        crate::list::list(Cursor::new(archive), None, |member| {
            found.push(member);
            Ok(())
        })?;
        Ok(found)
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

    /// Pseudo-random bytes that LZMA2 cannot compress.
    fn noise(len: u32) -> Vec<u8> {
        (0..len)
            .map(|n| (n.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect()
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
    fn entries_between_blocks_keep_their_header_position() {
        let archive = build(|creator| {
            creator.add_directory("d", Metadata::default()).unwrap();
            creator
                .add_file("d/a", Metadata::default(), &b"1"[..])
                .unwrap();
            creator
                .add_file("d/empty", Metadata::default(), &b""[..])
                .unwrap();
            creator.add_directory("e", Metadata::default()).unwrap();
            creator
                .add_file("e/b", Metadata::default(), &b"2"[..])
                .unwrap();
            creator.add_directory("f", Metadata::default()).unwrap();
        });
        let expected = ["d", "d/a", "d/empty", "e", "e/b", "f"];
        let read: Vec<String> = collect(&archive, None)
            .unwrap()
            .into_iter()
            .map(|(entry, _)| entry.path)
            .collect();
        assert_eq!(read, expected);
        let visited: Vec<String> = members(&archive)
            .unwrap()
            .into_iter()
            .map(|member| member.path)
            .collect();
        assert_eq!(visited, expected);
    }

    #[test]
    fn visitor_receives_members_with_method_checksum_and_link_target() {
        let archive = build(|creator| {
            creator.add_directory("dir", Metadata::default()).unwrap();
            creator
                .add_file("dir/file.txt", Metadata::default(), &b"content"[..])
                .unwrap();
            creator
                .add_symlink("dir/link", "file.txt", Metadata::default())
                .unwrap();
        });
        let found = members(&archive).unwrap();
        assert_eq!(found[0].kind, MemberKind::Directory);
        assert_eq!(found[0].compression, None);
        assert_eq!(found[1].kind, MemberKind::File);
        assert_eq!(found[1].size, 7);
        assert_eq!(found[1].compression.as_deref(), Some("lzma2"));
        assert!(found[1].crc32.is_some());
        assert!(found[1].name_is_exact());
        assert_eq!(found[2].kind, MemberKind::Symlink);
        assert_eq!(found[2].link_target.as_deref(), Some("file.txt"));
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
        assert!(list(Cursor::new(&archive)).unwrap().is_empty());
        assert!(members(&archive).unwrap().is_empty());
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
        let found = members(&archive).unwrap();
        assert!(found[0].encrypted);
        let methods = found[0].compression.clone().unwrap_or_default();
        assert!(methods.contains("aes256_sha256"), "{methods}");
        assert!(methods.contains("lzma2"), "{methods}");
    }

    #[test]
    fn encrypted_header_is_reported_as_encrypted() {
        let archive = encrypted_archive(true);
        assert!(matches!(list(Cursor::new(&archive)), Err(Error::Encrypted)));
        assert!(matches!(collect(&archive, None), Err(Error::Encrypted)));
        let error = members(&archive).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encrypted);
        assert!(
            error.to_string().starts_with("the archive is encrypted"),
            "{error}"
        );
    }

    #[test]
    fn rejects_data_that_is_not_an_archive() {
        for data in [&b""[..], b"not a 7z archive, plain text"] {
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
            let error = members(&archive[..keep]).unwrap_err();
            assert_eq!(error.code(), ErrorCode::Format, "kept {keep} bytes");
        }
    }

    #[test]
    fn corrupted_content_is_a_format_error() {
        let data = noise(20_000);
        let mut archive = build(|creator| {
            creator
                .add_file("a", Metadata::default(), data.as_slice())
                .unwrap();
        });
        archive[100] ^= 0xff;
        let mut failed_read = None;
        let result = read_entries(Cursor::new(&archive), None, |_, content| {
            failed_read = io::copy(content, &mut io::sink()).err();
            Ok(())
        });
        let damaged = failed_read.is_some() || matches!(result, Err(Error::Format(_)));
        assert!(damaged, "{result:?}");
        let error = members(&archive).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format, "{error}");
    }

    #[test]
    fn overlong_link_target_is_a_format_error() {
        let target = "t".repeat(5_000);
        let archive = build(|creator| {
            creator
                .add_symlink("link", &target, Metadata::default())
                .unwrap();
        });
        let error = members(&archive).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
        assert!(error.to_string().contains("link target of link"), "{error}");
    }

    #[test]
    fn level_selects_the_lzma2_preset() {
        let text = "the same line of text, again and again\n".repeat(2_000);
        let archives: Vec<Vec<u8>> = [0, 6]
            .into_iter()
            .map(|level| {
                let mut creator = Creator::with_level(Cursor::new(Vec::new()), level).unwrap();
                creator
                    .add_file("t.txt", Metadata::default(), text.as_bytes())
                    .unwrap();
                creator.finish().unwrap().into_inner()
            })
            .collect();
        assert_ne!(archives[0], archives[1]);
        for archive in &archives {
            let seen = collect(archive, None).unwrap();
            assert_eq!(seen[0].1, text.as_bytes());
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
