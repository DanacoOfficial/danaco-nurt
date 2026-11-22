// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Archive formats, members and listing.
//!
//! The format of an archive is recognised from its content, never from the
//! file name alone. Every format is read through the same visitor, so the
//! listing, the extraction plan and the content stream do not depend on the
//! container.

use std::borrow::Cow;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Instant;

use crate::codec::{self, Algorithm};

const ZIP_SIGNATURES: [&[u8]; 3] = [b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"];
const SEVEN_Z_SIGNATURE: &[u8] = &[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c];
const PREFIX_LEN: usize = 6;

/// Size of a tar header block.
pub(crate) const TAR_BLOCK: usize = 512;
const TAR_CHECKSUM: std::ops::Range<usize> = 148..156;
const TAR_CHECKSUM_BLANK: u32 = b' ' as u32;
const OCTAL_RADIX: u32 = 8;

/// Container format of an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// A zip archive, including `jar`, `whl` and `apk` files.
    Zip,
    /// A tar archive, optionally inside a compressed stream.
    Tar(Option<Algorithm>),
    /// A `7z` archive.
    SevenZ,
}

impl Format {
    /// Name used in command parameters and in the `format` field
    /// (`zip`, `tar`, `tar_gz`, `tar_zst`, `tar_xz`, `tar_bz2`, `tar_lz4`, `7z`).
    ///
    /// ```
    /// use nurt_archive::codec::Algorithm;
    /// use nurt_archive::list::Format;
    ///
    /// assert_eq!(Format::Tar(Some(Algorithm::Zstd)).name(), "tar_zst");
    /// ```
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar(None) => "tar",
            Self::Tar(Some(Algorithm::Gzip)) => "tar_gz",
            Self::Tar(Some(Algorithm::Zstd)) => "tar_zst",
            Self::Tar(Some(Algorithm::Xz)) => "tar_xz",
            Self::Tar(Some(Algorithm::Bzip2)) => "tar_bz2",
            Self::Tar(Some(Algorithm::Lz4)) => "tar_lz4",
            Self::SevenZ => "7z",
        }
    }

    /// Parses a name returned by [`Format::name`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }

    /// Format implied by the extension of a file name, compared without
    /// regard to case; used for `format: auto` when an archive is created.
    ///
    /// ```
    /// use nurt_archive::codec::Algorithm;
    /// use nurt_archive::list::Format;
    ///
    /// assert_eq!(Format::from_file_name("dist.tar.gz"), Some(Format::Tar(Some(Algorithm::Gzip))));
    /// assert_eq!(Format::from_file_name("app.JAR"), Some(Format::Zip));
    /// assert_eq!(Format::from_file_name("notes.txt"), None);
    /// ```
    #[must_use]
    pub fn from_file_name(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        let (stem, extension) = lower.rsplit_once('.')?;
        match extension {
            "zip" | "jar" | "whl" | "apk" => Some(Self::Zip),
            "tar" => Some(Self::Tar(None)),
            "7z" => Some(Self::SevenZ),
            "tgz" | "tzst" | "txz" | "tbz2" => {
                Algorithm::from_extension(extension).map(|algorithm| Self::Tar(Some(algorithm)))
            }
            "tbz" => Some(Self::Tar(Some(Algorithm::Bzip2))),
            _ => {
                let algorithm = Algorithm::from_extension(extension)?;
                stem.ends_with(".tar").then_some(Self::Tar(Some(algorithm)))
            }
        }
    }

    const ALL: [Self; 8] = [
        Self::Zip,
        Self::Tar(None),
        Self::Tar(Some(Algorithm::Gzip)),
        Self::Tar(Some(Algorithm::Zstd)),
        Self::Tar(Some(Algorithm::Xz)),
        Self::Tar(Some(Algorithm::Bzip2)),
        Self::Tar(Some(Algorithm::Lz4)),
        Self::SevenZ,
    ];
}

impl fmt::Display for Format {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Error of reading, planning or creating an archive.
///
/// [`ArchiveError::code`] maps every variant to an error code of the
/// language.
#[derive(Debug)]
#[non_exhaustive]
pub enum ArchiveError {
    /// The file is not an archive of a known format, or its structure is
    /// damaged.
    Format {
        /// Format, when it was recognised before the damage was found.
        format: Option<Format>,
        /// What is wrong.
        detail: String,
    },
    /// The format is recognised but this build cannot read or write it.
    Unsupported {
        /// Recognised format.
        format: Format,
    },
    /// The content of a member, or the header of a 7z archive, is encrypted.
    Encrypted {
        /// Path of the member inside the archive; empty when the header of
        /// the archive is encrypted.
        path: String,
    },
    /// Extraction would exceed a limit of [`crate::extract::Limits`], or a
    /// new 7z archive would exceed [`crate::create::MAX_SEVEN_Z_SIZE`].
    Limit {
        /// Exceeded limit.
        limit: LimitKind,
        /// Allowed value.
        allowed: u64,
        /// Value the archive declares or produced.
        actual: u64,
    },
    /// A path given for a new member cannot be stored in an archive.
    InvalidPath {
        /// The path as given.
        path: String,
        /// Why it is rejected.
        violation: crate::safety::Violation,
    },
    /// Two inputs of a new archive have the same path.
    DuplicatePath {
        /// The repeated path.
        path: String,
    },
    /// The compression level is outside the range of the format.
    InvalidLevel {
        /// Format of the new archive.
        format: Format,
        /// Requested level.
        level: u32,
        /// Lowest accepted level.
        min: u32,
        /// Highest accepted level.
        max: u32,
    },
    /// The archive or an input file changed while it was being read.
    Changed {
        /// Path of the member or of the input inside the archive.
        path: String,
        /// What differs.
        detail: String,
    },
    /// Input or output error of the underlying reader or writer.
    Io(io::Error),
}

/// Limit checked before and during extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// Total size of the extracted content in bytes.
    Size,
    /// Ratio of the extracted size to the size of the archive.
    Ratio,
    /// Size of a new 7z archive, which is assembled in memory before it is
    /// written.
    Memory,
}

impl LimitKind {
    /// Stable name of the limit (`size`, `ratio`, `memory`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Ratio => "ratio",
            Self::Memory => "memory",
        }
    }
}

impl ArchiveError {
    /// Error code of the language for this error.
    ///
    /// | Variant | Code |
    /// |---|---|
    /// | `Format`, `Unsupported` | `E_FORMAT` |
    /// | `Encrypted` | `E_ENCRYPTED` |
    /// | `Limit` | `E_LIMIT` |
    /// | `InvalidPath`, `DuplicatePath` | `E_ARG` |
    /// | `InvalidLevel` | `E_INVALID_VALUE` |
    /// | `Changed` | `E_STALE` |
    /// | `Io` | `E_IO` |
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Format { .. } | Self::Unsupported { .. } => ErrorCode::Format,
            Self::Encrypted { .. } => ErrorCode::Encrypted,
            Self::Limit { .. } => ErrorCode::Limit,
            Self::InvalidPath { .. } | Self::DuplicatePath { .. } => ErrorCode::Arg,
            Self::InvalidLevel { .. } => ErrorCode::InvalidValue,
            Self::Changed { .. } => ErrorCode::Stale,
            Self::Io(_) => ErrorCode::Io,
        }
    }

    pub(crate) fn format(format: Option<Format>, detail: impl Into<String>) -> Self {
        Self::Format {
            format,
            detail: detail.into(),
        }
    }

    /// Classifies an error raised while an archive is decoded: damaged or
    /// truncated data is a format error, everything else an I/O error.
    pub(crate) fn from_read(format: Format, error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::InvalidData
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::Other => Self::format(Some(format), error.to_string()),
            _ => Self::Io(error),
        }
    }
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format {
                format: Some(format),
                detail,
            } => write!(formatter, "damaged {format} archive: {detail}"),
            Self::Format {
                format: None,
                detail,
            } => write!(formatter, "not an archive of a known format: {detail}"),
            Self::Unsupported { format } => {
                write!(
                    formatter,
                    "{format} archives are not supported by this build"
                )
            }
            Self::Encrypted { path } if path.is_empty() => {
                formatter.write_str("the archive is encrypted; decrypt it with its own tool first")
            }
            Self::Encrypted { path } => write!(
                formatter,
                "member {path} is encrypted; decrypt the archive with its own tool first"
            ),
            Self::Limit {
                limit: LimitKind::Size,
                allowed,
                actual,
            } => write!(
                formatter,
                "extracted size {actual} bytes exceeds the limit of {allowed} bytes"
            ),
            Self::Limit {
                limit: LimitKind::Ratio,
                allowed,
                actual,
            } => write!(
                formatter,
                "compression ratio {actual} exceeds the limit of {allowed}; the archive may be a decompression bomb"
            ),
            Self::Limit {
                limit: LimitKind::Memory,
                allowed,
                actual,
            } => write!(
                formatter,
                "a 7z archive of {actual} bytes exceeds the in-memory limit of {allowed} bytes; create a zip or tar archive instead"
            ),
            Self::InvalidPath { path, violation } => write!(
                formatter,
                "path {path:?} cannot be stored in an archive: {}",
                violation.describe()
            ),
            Self::DuplicatePath { path } => {
                write!(
                    formatter,
                    "path {path} is given more than once; remove the duplicate"
                )
            }
            Self::InvalidLevel {
                format,
                level,
                min,
                max,
            } => write!(
                formatter,
                "level {level} is outside {min}..{max} for {format}"
            ),
            Self::Changed { path, detail } => {
                write!(
                    formatter,
                    "{path} changed while it was read: {detail}; run the command again"
                )
            }
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ArchiveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Type of an archive member.
///
/// The `type` field of an `archive_entry` record is `file` for
/// [`MemberKind::File`], `directory` for [`MemberKind::Directory`], `link`
/// for both kinds of links and `other` for [`MemberKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link; the target is in [`Member::link_target`].
    Symlink,
    /// Hard link to another member, named in [`Member::link_target`].
    HardLink,
    /// Device, FIFO or another special file.
    Other,
}

/// Member of an archive, as stored in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Full path inside the archive with `/` separators and without a
    /// trailing `/`, decoded as stored; see [`Member::name_is_exact`].
    pub path: String,
    /// Type.
    pub kind: MemberKind,
    /// Size after extraction in bytes.
    pub size: u64,
    /// Size inside the archive in bytes, when the format records it per
    /// member.
    pub packed: Option<u64>,
    /// Modification time.
    pub modified: Option<Instant>,
    /// Permission bits (`0o7777` mask), when the archive records them.
    pub mode: Option<u32>,
    /// Target of a symbolic link, or the archive path of the member a hard
    /// link refers to.
    pub link_target: Option<String>,
    /// Compression method of the member or of the whole stream.
    pub compression: Option<String>,
    /// The content is encrypted.
    pub encrypted: bool,
    /// CRC-32 checksum of the content, when the format records it.
    pub crc32: Option<u32>,
    pub(crate) exact_name: bool,
}

impl Member {
    /// Last component of [`Member::path`].
    #[must_use]
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// Whether the path and the link target were stored as valid UTF-8 and
    /// are reproduced exactly; `false` when invalid bytes were replaced with
    /// U+FFFD for display.
    #[must_use]
    pub fn name_is_exact(&self) -> bool {
        self.exact_name
    }

    pub(crate) fn new(path: Cow<'_, [u8]>, kind: MemberKind) -> Self {
        let (path, exact_name) = decode_name(path);
        Self {
            path,
            kind,
            size: 0,
            packed: None,
            modified: None,
            mode: None,
            link_target: None,
            compression: None,
            encrypted: false,
            crc32: None,
            exact_name,
        }
    }

    pub(crate) fn set_link_target(&mut self, target: Cow<'_, [u8]>) {
        let (target, exact) = decode_name(target);
        self.exact_name &= exact;
        self.link_target = Some(target);
    }
}

fn decode_name(bytes: Cow<'_, [u8]>) -> (String, bool) {
    let (mut text, exact) = match String::from_utf8(bytes.into_owned()) {
        Ok(text) => (text, true),
        Err(error) => (
            String::from_utf8_lossy(error.as_bytes()).into_owned(),
            false,
        ),
    };
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    (text, exact)
}

/// Access to the content of the member passed to a visitor.
pub(crate) trait Body {
    /// Opens the decoded content of the member; called at most once per
    /// member.
    fn open(&mut self) -> Result<&mut dyn Read, ArchiveError>;
}

/// Decision of a visitor after a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Stop,
}

/// Callback receiving the members of an archive in archive order.
pub(crate) type Visitor<'v> = dyn FnMut(&Member, &mut dyn Body) -> Result<Flow, ArchiveError> + 'v;

/// Recognises the format of an archive from its first bytes.
///
/// The reader is left at the start of the archive. A compressed stream is
/// reported as a tar archive only when the decompressed data starts with a
/// valid tar header.
///
/// # Errors
///
/// Returns [`ArchiveError::Format`] when the content matches no supported
/// format, and [`ArchiveError::Io`] when the reader fails.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
/// use nurt_archive::list::{detect_format, Format};
///
/// let empty_zip = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec();
/// assert_eq!(detect_format(&mut Cursor::new(empty_zip)).unwrap(), Format::Zip);
/// ```
pub fn detect_format<R: Read + Seek + Send>(reader: &mut R) -> Result<Format, ArchiveError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut prefix = Vec::with_capacity(TAR_BLOCK);
    reader
        .by_ref()
        .take(TAR_BLOCK as u64)
        .read_to_end(&mut prefix)?;
    reader.seek(SeekFrom::Start(0))?;
    let head = &prefix[..prefix.len().min(PREFIX_LEN)];
    if ZIP_SIGNATURES
        .iter()
        .any(|signature| head.starts_with(signature))
    {
        return Ok(Format::Zip);
    }
    if head.starts_with(SEVEN_Z_SIGNATURE) {
        return Ok(Format::SevenZ);
    }
    if let Some(algorithm) = codec::detect(head) {
        let format = Format::Tar(Some(algorithm));
        let mut block = Vec::with_capacity(TAR_BLOCK);
        codec::decoder(algorithm, reader.by_ref())?
            .take(TAR_BLOCK as u64)
            .read_to_end(&mut block)
            .map_err(|error| ArchiveError::from_read(format, error))?;
        reader.seek(SeekFrom::Start(0))?;
        return if is_tar_header(&block) {
            Ok(format)
        } else {
            Err(ArchiveError::format(
                None,
                format!(
                    "the {} stream does not contain a tar archive; use decompress",
                    algorithm.name()
                ),
            ))
        };
    }
    if is_tar_header(&prefix) {
        return Ok(Format::Tar(None));
    }
    Err(ArchiveError::format(
        None,
        "expected zip, tar, compressed tar or 7z content",
    ))
}

/// Whether a block is a tar header with a valid checksum, or the zero block
/// that ends an empty archive.
fn is_tar_header(block: &[u8]) -> bool {
    let Some(block) = block.get(..TAR_BLOCK) else {
        return false;
    };
    if block.iter().all(|byte| *byte == 0) {
        return true;
    }
    let Some(stored) = parse_octal(&block[TAR_CHECKSUM]) else {
        return false;
    };
    let sum: u32 = block
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if TAR_CHECKSUM.contains(&index) {
                TAR_CHECKSUM_BLANK
            } else {
                u32::from(*byte)
            }
        })
        .sum();
    stored == sum
}

fn parse_octal(field: &[u8]) -> Option<u32> {
    let text = std::str::from_utf8(field).ok()?;
    let digits = text.trim_matches(|c: char| c == ' ' || c == '\0');
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, OCTAL_RADIX).ok()
}

/// Recognises the format and passes every member to the visitor.
pub(crate) fn visit<R: Read + Seek + Send>(
    mut reader: R,
    visitor: &mut Visitor<'_>,
) -> Result<Format, ArchiveError> {
    let format = detect_format(&mut reader)?;
    match format {
        Format::Zip => crate::zip::visit(reader, visitor)?,
        Format::Tar(compression) => crate::tar::visit(reader, compression, visitor)?,
        Format::SevenZ => crate::sevenz::visit(reader, visitor)?,
    }
    Ok(format)
}

/// Lists the members of an archive in archive order.
///
/// The format is recognised from the content. `filter` receives
/// [`Member::path`] and selects the members passed to `emit`; the `glob`
/// parameter of `archive.list` is compiled into it by the caller, so the
/// glob syntax is the same as in the other file commands.
///
/// # Errors
///
/// Returns [`ArchiveError::Format`] for damaged or unknown content,
/// [`ArchiveError::Unsupported`] for a recognised format this build does
/// not read, [`ArchiveError::Io`] when the reader fails, and any error
/// returned by `emit`.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
/// use nurt_archive::create::{create, CreateOptions, Input};
/// use nurt_archive::list::{list, Format};
///
/// let inputs = vec![Input::file("docs/a.txt", 2), Input::directory("docs")];
/// let bytes = create(Vec::new(), &CreateOptions::new(Format::Zip), inputs, |_| {
///     Ok(Box::new(&b"hi"[..]))
/// })
/// .unwrap();
///
/// let mut paths = Vec::new();
/// let format = list(Cursor::new(bytes), None, |member| {
///     paths.push(member.path);
///     Ok(())
/// })
/// .unwrap();
/// assert_eq!(format, Format::Zip);
/// assert_eq!(paths, ["docs", "docs/a.txt"]);
/// ```
pub fn list<R, F>(
    reader: R,
    filter: Option<&dyn Fn(&str) -> bool>,
    mut emit: F,
) -> Result<Format, ArchiveError>
where
    R: Read + Seek + Send,
    F: FnMut(Member) -> Result<(), ArchiveError>,
{
    visit(reader, &mut |member, _body| {
        if filter.is_none_or(|selected| selected(&member.path)) {
            emit(member.clone())?;
        }
        Ok(Flow::Continue)
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn tar_with_file() -> Vec<u8> {
        let mut builder = ::tar::Builder::new(Vec::new());
        let mut header = ::tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, "a.txt", &b"abc"[..])
            .unwrap();
        builder.into_inner().unwrap()
    }

    fn compress(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut encoder = codec::encoder(algorithm, None, Vec::new()).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn detects_plain_and_compressed_tar() {
        let tar = tar_with_file();
        assert_eq!(
            detect_format(&mut Cursor::new(tar.clone())).unwrap(),
            Format::Tar(None)
        );
        for algorithm in codec::ALGORITHMS {
            let mut reader = Cursor::new(compress(algorithm, &tar));
            assert_eq!(
                detect_format(&mut reader).unwrap(),
                Format::Tar(Some(algorithm))
            );
            assert_eq!(reader.position(), 0);
        }
    }

    #[test]
    fn detects_seven_z_and_rejects_a_damaged_header() {
        let mut bytes = SEVEN_Z_SIGNATURE.to_vec();
        bytes.extend_from_slice(&[0; 26]);
        assert_eq!(
            detect_format(&mut Cursor::new(bytes.clone())).unwrap(),
            Format::SevenZ
        );
        let error = list(Cursor::new(bytes), None, |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            ArchiveError::Format {
                format: Some(Format::SevenZ),
                ..
            }
        ));
        assert_eq!(error.code(), ErrorCode::Format);
    }

    #[test]
    fn rejects_compressed_stream_without_tar() {
        let bytes = compress(Algorithm::Gzip, b"plain text, not a tar archive");
        let error = detect_format(&mut Cursor::new(bytes)).unwrap_err();
        assert!(error.to_string().contains("use decompress"), "{error}");
    }

    #[test]
    fn rejects_unknown_and_empty_content() {
        for bytes in [b"hello world".to_vec(), Vec::new()] {
            let error = detect_format(&mut Cursor::new(bytes)).unwrap_err();
            assert_eq!(error.code(), ErrorCode::Format);
        }
    }

    #[test]
    fn rejects_block_with_wrong_checksum() {
        let mut tar = tar_with_file();
        tar[0] ^= 0x01;
        assert!(detect_format(&mut Cursor::new(tar)).is_err());
    }

    #[test]
    fn maps_file_names_to_formats() {
        let cases = [
            ("a.zip", Some(Format::Zip)),
            ("a.whl", Some(Format::Zip)),
            ("a.apk", Some(Format::Zip)),
            ("a.tar", Some(Format::Tar(None))),
            ("a.tar.gz", Some(Format::Tar(Some(Algorithm::Gzip)))),
            ("a.tgz", Some(Format::Tar(Some(Algorithm::Gzip)))),
            ("a.tar.zst", Some(Format::Tar(Some(Algorithm::Zstd)))),
            ("a.tzst", Some(Format::Tar(Some(Algorithm::Zstd)))),
            ("a.tar.xz", Some(Format::Tar(Some(Algorithm::Xz)))),
            ("a.txz", Some(Format::Tar(Some(Algorithm::Xz)))),
            ("a.tar.bz2", Some(Format::Tar(Some(Algorithm::Bzip2)))),
            ("a.tbz2", Some(Format::Tar(Some(Algorithm::Bzip2)))),
            ("a.tbz", Some(Format::Tar(Some(Algorithm::Bzip2)))),
            ("a.7z", Some(Format::SevenZ)),
            ("a.gz", None),
            ("archive", None),
        ];
        for (name, expected) in cases {
            assert_eq!(Format::from_file_name(name), expected, "{name}");
        }
    }

    #[test]
    fn format_names_round_trip() {
        for format in Format::ALL {
            assert_eq!(Format::from_name(format.name()), Some(format));
        }
        assert_eq!(Format::from_name("rar"), None);
    }

    #[test]
    fn decodes_invalid_utf8_names_lossily() {
        let member = Member::new(Cow::Borrowed(b"dir/\xffname/"), MemberKind::Directory);
        assert_eq!(member.path, "dir/\u{fffd}name");
        assert_eq!(member.name(), "\u{fffd}name");
        assert!(!member.name_is_exact());
    }
}
