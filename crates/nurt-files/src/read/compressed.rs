// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading of compressed files after decompression.
//!
//! A file is read after decompression when its name ends with an extension
//! of gzip, zstd, xz or bzip2 (`.gz`, `.zst`, `.xz`, `.bz2` and the tar
//! forms `.tgz`, `.tzst`, `.txz`, `.tbz2`) and its leading bytes carry the
//! signature of the same format. A file whose content does not match its
//! extension is read as it is.
//!
//! Decompression is streamed and stops with [`DecompressError::Limit`] as
//! soon as the output exceeds a limit, so a small file that expands to a
//! very large output does not exhaust memory.

use std::fmt;
use std::io::{self, Read};

use nurt_archive::codec::{self, Algorithm};
use nurt_contracts::error::ErrorCode;

/// Formats that are decompressed automatically.
pub const AUTO_ALGORITHMS: [Algorithm; 4] = [
    Algorithm::Gzip,
    Algorithm::Zstd,
    Algorithm::Xz,
    Algorithm::Bzip2,
];

/// Number of leading bytes that [`compression_of`] needs.
pub const SIGNATURE_LEN: usize = 6;

const TAR_EXTENSION: &str = "tar";

/// Returns the format in which a file is decompressed automatically.
///
/// `prefix` holds the first [`SIGNATURE_LEN`] bytes of the file, or the
/// whole file when it is shorter.
///
/// # Examples
///
/// ```
/// use nurt_archive::codec::Algorithm;
/// use nurt_files::read::compressed::compression_of;
///
/// assert_eq!(compression_of("app.log.gz", b"\x1f\x8b\x08\x00"), Some(Algorithm::Gzip));
/// assert_eq!(compression_of("app.log.gz", b"plain text"), None);
/// assert_eq!(compression_of("app.log", b"\x1f\x8b\x08\x00"), None);
/// ```
pub fn compression_of(file_name: &str, prefix: &[u8]) -> Option<Algorithm> {
    let (_, extension) = file_name.rsplit_once('.')?;
    let by_name = Algorithm::from_extension(extension)?;
    let by_content = codec::detect(prefix)?;
    (by_name == by_content && AUTO_ALGORITHMS.contains(&by_name)).then_some(by_name)
}

/// Name of the file after decompression: the name without the compression
/// extension, with `.tar` in place of the tar forms such as `.tgz`.
///
/// Returns `None` when the name has no compression extension or nothing
/// precedes it.
///
/// # Examples
///
/// ```
/// use nurt_files::read::compressed::decompressed_name;
///
/// assert_eq!(decompressed_name("report.pdf.gz").as_deref(), Some("report.pdf"));
/// assert_eq!(decompressed_name("backup.TGZ").as_deref(), Some("backup.tar"));
/// assert_eq!(decompressed_name("notes.txt"), None);
/// ```
pub fn decompressed_name(file_name: &str) -> Option<String> {
    let (stem, extension) = file_name.rsplit_once('.')?;
    let algorithm = Algorithm::from_extension(extension)?;
    if stem.is_empty() || stem.ends_with('/') {
        return None;
    }
    if extension.eq_ignore_ascii_case(algorithm.extension()) {
        Some(stem.to_owned())
    } else {
        Some(format!("{stem}.{TAR_EXTENSION}"))
    }
}

/// Failure to read decompressed content.
#[derive(Debug)]
#[non_exhaustive]
pub enum DecompressError {
    /// The decompressed content exceeds the limit.
    Limit {
        /// Limit in bytes.
        limit: u64,
    },
    /// The compressed data is damaged or truncated.
    Corrupt {
        /// Description of the defect.
        detail: String,
    },
    /// The compressed file cannot be read.
    Io(io::Error),
}

impl DecompressError {
    /// Error code of the language for this failure.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_files::read::compressed::DecompressError;
    ///
    /// assert_eq!(DecompressError::Limit { limit: 1024 }.code(), ErrorCode::Limit);
    /// ```
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Limit { .. } => ErrorCode::Limit,
            Self::Corrupt { .. } => ErrorCode::Format,
            Self::Io(_) => ErrorCode::Io,
        }
    }

    /// Classifies an error returned by [`DecompressedReader`].
    ///
    /// Errors of the underlying reader become [`DecompressError::Io`], the
    /// end of the limit becomes [`DecompressError::Limit`] and every other
    /// error comes from the decoder and becomes [`DecompressError::Corrupt`].
    pub fn from_io(error: io::Error) -> Self {
        if let Some(exceeded) = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<LimitExceeded>())
        {
            return Self::Limit {
                limit: exceeded.limit,
            };
        }
        let kind = error.kind();
        match error.into_inner() {
            Some(inner) => match inner.downcast::<SourceError>() {
                Ok(source) => Self::Io(source.0),
                Err(inner) => Self::Corrupt {
                    detail: inner.to_string(),
                },
            },
            None => Self::Corrupt {
                detail: kind.to_string(),
            },
        }
    }
}

impl fmt::Display for DecompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit { limit } => write!(
                f,
                "decompressed content exceeds {limit} bytes; read the file with decompress: none"
            ),
            Self::Corrupt { detail } => write!(
                f,
                "corrupt compressed data: {detail}; read the file with decompress: none"
            ),
            Self::Io(error) => write!(f, "cannot read the compressed file: {error}"),
        }
    }
}

impl std::error::Error for DecompressError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Limit { .. } | Self::Corrupt { .. } => None,
        }
    }
}

/// Marks the end of the output limit inside an [`io::Error`].
#[derive(Debug)]
struct LimitExceeded {
    limit: u64,
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decompressed content exceeds {} bytes", self.limit)
    }
}

impl std::error::Error for LimitExceeded {}

/// Marks an error of the compressed source inside an [`io::Error`], so that
/// it is not mistaken for damaged data after passing through a decoder.
#[derive(Debug)]
struct SourceError(io::Error);

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SourceError {}

struct Source<R>(R);

impl<R: Read> Read for Source<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .read(buf)
            .map_err(|error| io::Error::new(error.kind(), SourceError(error)))
    }
}

/// Reader of the decompressed content of a compressed stream, with a limit
/// on the size of the output.
///
/// Reading fails once the output would exceed the limit; the bytes up to
/// the limit are returned before the failure. Errors are classified with
/// [`DecompressError::from_io`].
pub struct DecompressedReader<'a> {
    decoder: Box<dyn Read + Send + 'a>,
    limit: u64,
    produced: u64,
    exceeded: bool,
}

impl fmt::Debug for DecompressedReader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecompressedReader")
            .field("limit", &self.limit)
            .field("produced", &self.produced)
            .finish_non_exhaustive()
    }
}

impl<'a> DecompressedReader<'a> {
    /// Decompresses `source` in the given format, allowing no more than
    /// `limit` bytes of output.
    ///
    /// # Errors
    ///
    /// Returns [`DecompressError::Corrupt`] when the decoder cannot be
    /// initialised.
    pub fn new<R: Read + Send + 'a>(
        algorithm: Algorithm,
        source: R,
        limit: u64,
    ) -> Result<Self, DecompressError> {
        let decoder =
            codec::decoder(algorithm, Source(source)).map_err(DecompressError::from_io)?;
        Ok(Self {
            decoder,
            limit,
            produced: 0,
            exceeded: false,
        })
    }

    /// Number of decompressed bytes returned so far.
    pub fn produced(&self) -> u64 {
        self.produced
    }

    /// Limit of the output in bytes.
    pub fn limit(&self) -> u64 {
        self.limit
    }
}

impl Read for DecompressedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.exceeded {
            return Err(io::Error::other(LimitExceeded { limit: self.limit }));
        }
        if buf.is_empty() {
            return Ok(0);
        }
        // One byte beyond the limit is requested to tell output that ends
        // exactly at the limit from output that continues past it.
        let allowed = self.limit.saturating_sub(self.produced).saturating_add(1);
        let len = usize::try_from(allowed).map_or(buf.len(), |allowed| allowed.min(buf.len()));
        let read = self.decoder.read(&mut buf[..len])?;
        let total = self.produced + read as u64;
        if total > self.limit {
            let kept = read - 1;
            self.produced = self.limit;
            self.exceeded = true;
            if kept == 0 {
                return Err(io::Error::other(LimitExceeded { limit: self.limit }));
            }
            return Ok(kept);
        }
        self.produced = total;
        Ok(read)
    }
}

/// Decompresses a whole stream into memory, allowing no more than `limit`
/// bytes of output.
///
/// # Errors
///
/// Returns [`DecompressError::Limit`] when the output exceeds the limit,
/// [`DecompressError::Corrupt`] for damaged or truncated data and
/// [`DecompressError::Io`] when `source` fails.
///
/// # Examples
///
/// ```
/// use std::io::Write;
///
/// use nurt_archive::codec::{self, Algorithm};
/// use nurt_files::read::compressed::{DecompressError, read_limited};
///
/// let mut encoder = codec::encoder(Algorithm::Gzip, None, Vec::new())?;
/// encoder.write_all(&[b'a'; 10_000])?;
/// let compressed = encoder.finish()?;
///
/// assert_eq!(read_limited(Algorithm::Gzip, &compressed[..], 10_000)?.len(), 10_000);
/// assert!(matches!(
///     read_limited(Algorithm::Gzip, &compressed[..], 9_999),
///     Err(DecompressError::Limit { limit: 9_999 })
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn read_limited<R: Read + Send>(
    algorithm: Algorithm,
    source: R,
    limit: u64,
) -> Result<Vec<u8>, DecompressError> {
    let mut reader = DecompressedReader::new(algorithm, source, limit)?;
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .map_err(DecompressError::from_io)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn compress(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut encoder = codec::encoder(algorithm, None, Vec::new()).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn sample_text() -> Vec<u8> {
        b"line of a log file\n".repeat(500)
    }

    #[test]
    fn every_automatic_format_round_trips_and_is_recognised() {
        let names = ["a.log.gz", "a.log.zst", "a.log.xz", "a.log.bz2"];
        for (algorithm, name) in AUTO_ALGORITHMS.into_iter().zip(names) {
            let compressed = compress(algorithm, &sample_text());
            assert_eq!(
                compression_of(name, &compressed[..SIGNATURE_LEN]),
                Some(algorithm)
            );
            assert_eq!(
                read_limited(algorithm, &compressed[..], u64::MAX).unwrap(),
                sample_text(),
                "{name}"
            );
        }
    }

    #[test]
    fn lz4_and_mismatched_extensions_are_not_decompressed() {
        let lz4 = compress(Algorithm::Lz4, b"data");
        assert_eq!(compression_of("a.lz4", &lz4), None);
        let gzip = compress(Algorithm::Gzip, b"data");
        assert_eq!(compression_of("a.zst", &gzip), None);
        assert_eq!(compression_of("archive", &gzip), None);
        assert_eq!(compression_of("A.GZ", &gzip), Some(Algorithm::Gzip));
    }

    #[test]
    fn decompressed_names_drop_extension() {
        assert_eq!(
            decompressed_name("dir/app.log.zst").as_deref(),
            Some("dir/app.log")
        );
        assert_eq!(decompressed_name("data.txz").as_deref(), Some("data.tar"));
        assert_eq!(decompressed_name(".gz"), None);
        assert_eq!(decompressed_name("dir/.gz"), None);
        assert_eq!(decompressed_name("gz"), None);
    }

    #[test]
    fn output_at_limit_is_accepted_and_beyond_is_rejected() {
        let compressed = compress(Algorithm::Zstd, &[0; 4096]);
        assert_eq!(
            read_limited(Algorithm::Zstd, &compressed[..], 4096)
                .unwrap()
                .len(),
            4096
        );
        let error = read_limited(Algorithm::Zstd, &compressed[..], 4095).unwrap_err();
        assert!(matches!(error, DecompressError::Limit { limit: 4095 }));
        assert_eq!(error.code(), ErrorCode::Limit);
        assert!(error.to_string().contains("decompress: none"));
    }

    #[test]
    fn expansion_bomb_stops_at_limit() {
        let bomb = compress(Algorithm::Bzip2, &vec![0; 8 * 1024 * 1024]);
        let mut reader = DecompressedReader::new(Algorithm::Bzip2, &bomb[..], 1000).unwrap();
        let mut output = Vec::new();
        let error = reader.read_to_end(&mut output).unwrap_err();
        assert_eq!(output.len(), 1000);
        assert_eq!(reader.produced(), reader.limit());
        assert!(matches!(
            DecompressError::from_io(error),
            DecompressError::Limit { limit: 1000 }
        ));
        assert!(format!("{reader:?}").contains("limit: 1000"));
    }

    #[test]
    fn damaged_and_truncated_data_is_corrupt() {
        let compressed = compress(Algorithm::Gzip, &sample_text());
        let mut damaged = compressed.clone();
        for byte in &mut damaged[12..40] {
            *byte ^= 0xa5;
        }
        let truncated = &compressed[..compressed.len() / 2];
        for data in [&damaged[..], truncated] {
            let error = read_limited(Algorithm::Gzip, data, u64::MAX).unwrap_err();
            assert!(matches!(error, DecompressError::Corrupt { .. }), "{error}");
            assert_eq!(error.code(), ErrorCode::Format);
        }
        let error = read_limited(Algorithm::Xz, &b"\xfd7zXZ\x00garbage"[..], u64::MAX).unwrap_err();
        assert!(matches!(error, DecompressError::Corrupt { .. }), "{error}");
    }

    #[test]
    fn failure_of_source_is_io_error() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "access denied",
                ))
            }
        }
        for algorithm in AUTO_ALGORITHMS {
            let error = DecompressedReader::new(algorithm, Failing, u64::MAX)
                .and_then(|mut reader| {
                    reader
                        .read_to_end(&mut Vec::new())
                        .map_err(DecompressError::from_io)
                })
                .unwrap_err();
            match &error {
                DecompressError::Io(inner) => {
                    assert_eq!(inner.kind(), io::ErrorKind::PermissionDenied);
                }
                other => panic!("{}: {other}", algorithm.name()),
            }
            assert_eq!(error.code(), ErrorCode::Io);
            assert!(std::error::Error::source(&error).is_some());
        }
    }

    #[test]
    fn empty_buffer_reads_nothing() {
        let compressed = compress(Algorithm::Gzip, b"abc");
        let mut reader = DecompressedReader::new(Algorithm::Gzip, &compressed[..], 10).unwrap();
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(reader.produced(), 0);
    }

    #[test]
    fn error_without_inner_value_is_corrupt() {
        let error = DecompressError::from_io(io::ErrorKind::UnexpectedEof.into());
        assert!(matches!(error, DecompressError::Corrupt { .. }));
        assert!(std::error::Error::source(&error).is_none());
    }
}
