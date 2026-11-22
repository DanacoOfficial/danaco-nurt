// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Streaming compression codecs.
//!
//! Each [`Algorithm`] has a decoder that reads a compressed stream and an
//! encoder that writes one. Decoders accept concatenated members (gzip
//! multi-member files, several zstd frames, multi-stream xz and bzip2), as
//! produced by tools that append compressed output.

use std::fmt;
use std::io::{self, Read, Write};

/// Compression algorithm of a single stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Algorithm {
    /// The gzip format (RFC 1952).
    Gzip,
    /// Zstandard (RFC 8878).
    Zstd,
    /// The `.xz` container with LZMA2 data.
    Xz,
    /// The bzip2 format.
    Bzip2,
    /// The frame format of the `lz4` tool.
    Lz4,
}

/// Every algorithm, in the order used for detection and listings.
pub const ALGORITHMS: [Algorithm; 5] = [
    Algorithm::Gzip,
    Algorithm::Zstd,
    Algorithm::Xz,
    Algorithm::Bzip2,
    Algorithm::Lz4,
];

const GZIP_SIGNATURE: &[u8] = &[0x1f, 0x8b];
const ZSTD_SIGNATURE: &[u8] = &[0x28, 0xb5, 0x2f, 0xfd];
const XZ_SIGNATURE: &[u8] = &[0xfd, b'7', b'z', b'X', b'Z', 0x00];
const BZIP2_SIGNATURE: &[u8] = b"BZh";
const LZ4_SIGNATURE: &[u8] = &[0x04, 0x22, 0x4d, 0x18];

impl Algorithm {
    /// Name used in command parameters (`gzip`, `zstd`, `xz`, `bzip2`, `lz4`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Zstd => "zstd",
            Self::Xz => "xz",
            Self::Bzip2 => "bzip2",
            Self::Lz4 => "lz4",
        }
    }

    /// File name extension without the leading dot (`gz`, `zst`, `xz`, `bz2`, `lz4`).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gzip => "gz",
            Self::Zstd => "zst",
            Self::Xz => "xz",
            Self::Bzip2 => "bz2",
            Self::Lz4 => "lz4",
        }
    }

    /// Parses a parameter value as returned by [`Algorithm::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        ALGORITHMS
            .into_iter()
            .find(|algorithm| algorithm.name() == name)
    }

    /// Maps a file name extension, compared without regard to case.
    pub fn from_extension(extension: &str) -> Option<Self> {
        let lower = extension.to_ascii_lowercase();
        match lower.as_str() {
            "gz" | "tgz" => Some(Self::Gzip),
            "zst" | "tzst" => Some(Self::Zstd),
            "xz" | "txz" => Some(Self::Xz),
            "bz2" | "tbz2" => Some(Self::Bzip2),
            "lz4" => Some(Self::Lz4),
            _ => None,
        }
    }

    /// Compression level range accepted by the encoder, inclusive.
    pub fn level_range(self) -> (u32, u32) {
        match self {
            Self::Gzip | Self::Xz => (0, 9),
            Self::Zstd => (1, 22),
            Self::Bzip2 => (1, 9),
            Self::Lz4 => (0, 0),
        }
    }

    /// Level used when the caller does not choose one.
    pub fn default_level(self) -> u32 {
        match self {
            Self::Gzip | Self::Xz => 6,
            Self::Zstd => 3,
            Self::Bzip2 => 9,
            Self::Lz4 => 0,
        }
    }

    fn signature(self) -> &'static [u8] {
        match self {
            Self::Gzip => GZIP_SIGNATURE,
            Self::Zstd => ZSTD_SIGNATURE,
            Self::Xz => XZ_SIGNATURE,
            Self::Bzip2 => BZIP2_SIGNATURE,
            Self::Lz4 => LZ4_SIGNATURE,
        }
    }
}

/// Recognises a compressed stream by its leading bytes.
///
/// Six bytes are enough for every supported algorithm; a shorter prefix
/// matches only when it contains the whole signature.
pub fn detect(prefix: &[u8]) -> Option<Algorithm> {
    ALGORITHMS
        .into_iter()
        .find(|algorithm| prefix.starts_with(algorithm.signature()))
}

/// Returns a reader that yields the decompressed content of `inner`.
///
/// # Errors
///
/// Fails when the decoder cannot be initialised; errors in the compressed
/// data are reported by the returned reader.
pub fn decoder<'a, R: Read + Send + 'a>(
    algorithm: Algorithm,
    inner: R,
) -> io::Result<Box<dyn Read + Send + 'a>> {
    Ok(match algorithm {
        Algorithm::Gzip => Box::new(flate2::read::MultiGzDecoder::new(inner)),
        Algorithm::Zstd => Box::new(zstd::stream::read::Decoder::new(inner)?),
        Algorithm::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(inner)),
        Algorithm::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(inner)),
        Algorithm::Lz4 => Box::new(Lz4Reader::new(inner)),
    })
}

/// Writer that compresses into an inner writer.
///
/// [`Encoder::finish`] writes the stream trailer; dropping the encoder
/// without calling it leaves the output truncated.
pub struct Encoder<W: Write> {
    kind: EncoderKind<W>,
}

enum EncoderKind<W: Write> {
    Gzip(flate2::write::GzEncoder<W>),
    Zstd(zstd::stream::write::Encoder<'static, W>),
    Xz(liblzma::write::XzEncoder<W>),
    Bzip2(bzip2::write::BzEncoder<W>),
    Lz4(lz4_flex::frame::FrameEncoder<W>),
}

impl<W: Write> Encoder<W> {
    /// Completes the stream and returns the inner writer.
    ///
    /// # Errors
    ///
    /// Returns an error when the trailer cannot be written to the inner
    /// writer.
    pub fn finish(self) -> io::Result<W> {
        match self.kind {
            EncoderKind::Gzip(encoder) => encoder.finish(),
            EncoderKind::Zstd(encoder) => encoder.finish(),
            EncoderKind::Xz(encoder) => encoder.finish(),
            EncoderKind::Bzip2(encoder) => encoder.finish(),
            EncoderKind::Lz4(encoder) => encoder.finish().map_err(io::Error::other),
        }
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.kind {
            EncoderKind::Gzip(encoder) => encoder.write(buf),
            EncoderKind::Zstd(encoder) => encoder.write(buf),
            EncoderKind::Xz(encoder) => encoder.write(buf),
            EncoderKind::Bzip2(encoder) => encoder.write(buf),
            EncoderKind::Lz4(encoder) => encoder.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.kind {
            EncoderKind::Gzip(encoder) => encoder.flush(),
            EncoderKind::Zstd(encoder) => encoder.flush(),
            EncoderKind::Xz(encoder) => encoder.flush(),
            EncoderKind::Bzip2(encoder) => encoder.flush(),
            EncoderKind::Lz4(encoder) => encoder.flush(),
        }
    }
}

/// Returns an encoder that compresses into `inner`.
///
/// The level defaults to [`Algorithm::default_level`]. The `lz4` frame
/// format has no levels and accepts only `None` or `Some(0)`.
///
/// # Errors
///
/// Fails with [`io::ErrorKind::InvalidInput`] when the level is outside
/// [`Algorithm::level_range`], or when the encoder cannot be initialised.
pub fn encoder<W: Write>(
    algorithm: Algorithm,
    level: Option<u32>,
    inner: W,
) -> io::Result<Encoder<W>> {
    let level = level.unwrap_or_else(|| algorithm.default_level());
    let (low, high) = algorithm.level_range();
    if !(low..=high).contains(&level) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "level {level} is outside {low}..{high} for {}",
                algorithm.name()
            ),
        ));
    }
    let kind = match algorithm {
        Algorithm::Gzip => EncoderKind::Gzip(flate2::write::GzEncoder::new(
            inner,
            flate2::Compression::new(level),
        )),
        Algorithm::Zstd => {
            let level = i32::try_from(level).map_err(io::Error::other)?;
            let mut encoder = zstd::stream::write::Encoder::new(inner, level)?;
            encoder.include_checksum(true)?;
            EncoderKind::Zstd(encoder)
        }
        Algorithm::Xz => EncoderKind::Xz(liblzma::write::XzEncoder::new(inner, level)),
        Algorithm::Bzip2 => EncoderKind::Bzip2(bzip2::write::BzEncoder::new(
            inner,
            bzip2::Compression::new(level),
        )),
        Algorithm::Lz4 => EncoderKind::Lz4(lz4_flex::frame::FrameEncoder::with_frame_info(
            lz4_flex::frame::FrameInfo::new().content_checksum(true),
            inner,
        )),
    };
    Ok(Encoder { kind })
}

const LZ4_FRAME_SIGNATURE: u32 = 0x184d_2204;
const LZ4_FLAG_DICTIONARY_ID: u8 = 0x01;
const LZ4_FLAG_CONTENT_CHECKSUM: u8 = 0x04;
const LZ4_FLAG_CONTENT_SIZE: u8 = 0x08;
const LZ4_FLAG_BLOCK_CHECKSUM: u8 = 0x10;
const LZ4_UNCOMPRESSED_BLOCK_BIT: u32 = 0x8000_0000;
const LZ4_CONTENT_SIZE_LEN: u64 = 8;
const LZ4_DICTIONARY_ID_LEN: u64 = 4;
const LZ4_HEADER_CHECKSUM_LEN: u64 = 1;
const LZ4_CHECKSUM_LEN: u64 = 4;

#[derive(Debug, Clone, Copy)]
enum Lz4Phase {
    Signature,
    Descriptor,
    BlockHeader,
    Skip { remaining: u64, then: Lz4Next },
    Untracked,
}

#[derive(Debug, Clone, Copy)]
enum Lz4Next {
    BlockHeader,
    FrameEnd,
}

/// Follows the structure of an lz4 frame stream while the bytes pass through.
///
/// The frame decoder reports a stream cut at a block boundary as a normal end
/// of data. The tracker tells a complete stream from a truncated one.
#[derive(Debug)]
struct Lz4Tracker {
    phase: Lz4Phase,
    head: [u8; 4],
    filled: usize,
    flags: u8,
}

impl Lz4Tracker {
    fn new() -> Self {
        Self {
            phase: Lz4Phase::Signature,
            head: [0; 4],
            filled: 0,
            flags: 0,
        }
    }

    fn at_frame_boundary(&self) -> bool {
        match self.phase {
            Lz4Phase::Signature => self.filled == 0,
            Lz4Phase::Untracked => true,
            _ => false,
        }
    }

    fn feed(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            match self.phase {
                Lz4Phase::Untracked => return,
                Lz4Phase::Skip { remaining, then } => {
                    let take = remaining.min(data.len() as u64);
                    data = &data[take as usize..];
                    self.skip(remaining - take, then);
                }
                Lz4Phase::Signature | Lz4Phase::BlockHeader | Lz4Phase::Descriptor => {
                    let wanted = match self.phase {
                        Lz4Phase::Descriptor => 2,
                        _ => 4,
                    };
                    let take = (wanted - self.filled).min(data.len());
                    self.head[self.filled..self.filled + take].copy_from_slice(&data[..take]);
                    self.filled += take;
                    data = &data[take..];
                    if self.filled == wanted {
                        self.filled = 0;
                        self.gathered();
                    }
                }
            }
        }
    }

    fn skip(&mut self, remaining: u64, then: Lz4Next) {
        self.phase = if remaining > 0 {
            Lz4Phase::Skip { remaining, then }
        } else {
            match then {
                Lz4Next::BlockHeader => Lz4Phase::BlockHeader,
                Lz4Next::FrameEnd => Lz4Phase::Signature,
            }
        };
    }

    fn gathered(&mut self) {
        match self.phase {
            Lz4Phase::Signature => {
                self.phase = if u32::from_le_bytes(self.head) == LZ4_FRAME_SIGNATURE {
                    Lz4Phase::Descriptor
                } else {
                    Lz4Phase::Untracked
                };
            }
            Lz4Phase::Descriptor => {
                self.flags = self.head[0];
                let mut rest = LZ4_HEADER_CHECKSUM_LEN;
                if self.flags & LZ4_FLAG_CONTENT_SIZE != 0 {
                    rest += LZ4_CONTENT_SIZE_LEN;
                }
                if self.flags & LZ4_FLAG_DICTIONARY_ID != 0 {
                    rest += LZ4_DICTIONARY_ID_LEN;
                }
                self.skip(rest, Lz4Next::BlockHeader);
            }
            _ => {
                let header = u32::from_le_bytes(self.head);
                if header == 0 {
                    let trailer = if self.flags & LZ4_FLAG_CONTENT_CHECKSUM != 0 {
                        LZ4_CHECKSUM_LEN
                    } else {
                        0
                    };
                    self.skip(trailer, Lz4Next::FrameEnd);
                } else {
                    let mut length = u64::from(header & !LZ4_UNCOMPRESSED_BLOCK_BIT);
                    if self.flags & LZ4_FLAG_BLOCK_CHECKSUM != 0 {
                        length += LZ4_CHECKSUM_LEN;
                    }
                    self.skip(length, Lz4Next::BlockHeader);
                }
            }
        }
    }
}

struct Lz4Source<R> {
    inner: R,
    tracker: Lz4Tracker,
    ended: bool,
}

impl<R: Read> Read for Lz4Source<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buf)?;
        if count == 0 && !buf.is_empty() {
            self.ended = true;
        }
        self.tracker.feed(&buf[..count]);
        Ok(count)
    }
}

/// Reader for a sequence of lz4 frames that rejects a truncated stream.
struct Lz4Reader<R: Read> {
    decoder: lz4_flex::frame::FrameDecoder<Lz4Source<R>>,
}

impl<R: Read> Lz4Reader<R> {
    fn new(inner: R) -> Self {
        let source = Lz4Source {
            inner,
            tracker: Lz4Tracker::new(),
            ended: false,
        };
        Self {
            decoder: lz4_flex::frame::FrameDecoder::new(source),
        }
    }
}

impl<R: Read> Read for Lz4Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let count = self.decoder.read(buf)?;
            if count > 0 || buf.is_empty() {
                return Ok(count);
            }
            let source = self.decoder.get_ref();
            if !source.tracker.at_frame_boundary() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "lz4 stream ends inside a frame",
                ));
            }
            if source.ended {
                return Ok(0);
            }
        }
    }
}

/// Error returned by a [`LimitedReader`] whose source is longer than the limit.
///
/// The error is carried inside an [`io::Error`]; [`is_limit_exceeded`]
/// recognises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitExceeded {
    /// Maximum number of bytes the reader was allowed to yield.
    pub limit: u64,
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "output exceeds the limit of {} bytes", self.limit)
    }
}

impl std::error::Error for LimitExceeded {}

/// Reader that fails once its source yields more than a fixed number of bytes.
///
/// A source of exactly `limit` bytes is read to the end without an error.
#[derive(Debug)]
pub struct LimitedReader<R> {
    inner: R,
    limit: u64,
    remaining: u64,
}

/// Wraps `inner` so that reading more than `limit` bytes fails.
///
/// The failure is an [`io::Error`] of kind [`io::ErrorKind::Other`] that wraps
/// [`LimitExceeded`]. Use it on decoders to stop compression bombs.
pub fn limited<R: Read>(inner: R, limit: u64) -> LimitedReader<R> {
    LimitedReader {
        inner,
        limit,
        remaining: limit,
    }
}

/// Reports whether `error` was produced by a [`LimitedReader`].
pub fn is_limit_exceeded(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<LimitExceeded>())
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::other(LimitExceeded { limit: self.limit })),
            };
        }
        let window = usize::try_from(self.remaining).map_or(buf.len(), |n| n.min(buf.len()));
        let count = self.inner.read(&mut buf[..window])?;
        self.remaining -= count as u64;
        Ok(count)
    }
}

/// Failure of a whole-stream compression or decompression.
///
/// Each variant maps to one error code: [`Error::Format`] to `E_FORMAT`,
/// [`Error::Limit`] to `E_LIMIT`, [`Error::Io`] to `E_IO` and
/// [`Error::InvalidLevel`] to `E_ARG`.
#[derive(Debug)]
pub enum Error {
    /// The input is not a valid stream of the expected algorithm, or it ends early.
    Format(String),
    /// The decompressed output is larger than the permitted size.
    Limit {
        /// Maximum number of output bytes.
        limit: u64,
    },
    /// Reading the input or writing the output failed.
    Io(io::Error),
    /// The compression level is outside the range of the algorithm.
    InvalidLevel {
        /// Algorithm the level was given for.
        algorithm: Algorithm,
        /// Requested level.
        level: u32,
        /// Lowest accepted level.
        low: u32,
        /// Highest accepted level.
        high: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(reason) => write!(f, "invalid compressed data: {reason}"),
            Self::Limit { limit } => write!(
                f,
                "decompressed size exceeds {limit} bytes; raise the limit or check the source"
            ),
            Self::Io(error) => write!(f, "i/o failure: {error}"),
            Self::InvalidLevel {
                algorithm,
                level,
                low,
                high,
            } => write!(
                f,
                "level {level} is outside {low}..{high} for {}; choose a level in range",
                algorithm.name()
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn compress_all(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut encoder = encoder(algorithm, None, Vec::new()).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn sample(len: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            })
            .collect()
    }

    fn decode_all(algorithm: Algorithm, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        decoder(algorithm, data)?.read_to_end(&mut output)?;
        Ok(output)
    }

    #[test]
    fn concatenated_members_decode_as_one_stream() {
        let first = sample(3_000);
        let second = b"second member".to_vec();
        for algorithm in ALGORITHMS {
            let mut joined = compress_all(algorithm, &first);
            joined.extend(compress_all(algorithm, &second));
            let mut expected = first.clone();
            expected.extend(&second);
            assert_eq!(
                decode_all(algorithm, &joined).unwrap(),
                expected,
                "{}",
                algorithm.name()
            );
        }
    }

    #[test]
    fn every_level_round_trips() {
        let data = sample(5_000);
        for algorithm in ALGORITHMS {
            let (low, high) = algorithm.level_range();
            for level in low..=high {
                let mut encoder = encoder(algorithm, Some(level), Vec::new()).unwrap();
                encoder.write_all(&data).unwrap();
                let compressed = encoder.finish().unwrap();
                assert_eq!(
                    decode_all(algorithm, &compressed).unwrap(),
                    data,
                    "{} level {level}",
                    algorithm.name()
                );
            }
        }
    }

    #[test]
    fn truncated_streams_fail() {
        let data = sample(4_000);
        for algorithm in ALGORITHMS {
            let compressed = compress_all(algorithm, &data);
            let cut = &compressed[..compressed.len() - 6];
            assert!(decode_all(algorithm, cut).is_err(), "{}", algorithm.name());
        }
    }

    #[test]
    fn corrupted_streams_fail() {
        let data = sample(4_000);
        for algorithm in ALGORITHMS {
            let mut compressed = compress_all(algorithm, &data);
            let middle = compressed.len() / 2;
            compressed[middle] ^= 0xff;
            assert!(
                decode_all(algorithm, &compressed).is_err(),
                "{}",
                algorithm.name()
            );
        }
    }

    #[test]
    fn garbage_is_rejected_by_every_decoder() {
        let garbage = b"this is not compressed data at all";
        for algorithm in ALGORITHMS {
            assert!(
                decode_all(algorithm, garbage).is_err(),
                "{}",
                algorithm.name()
            );
        }
    }

    #[test]
    fn limited_reader_passes_input_up_to_the_limit() {
        let data = vec![7u8; 100];
        let mut output = Vec::new();
        limited(data.as_slice(), 100)
            .read_to_end(&mut output)
            .unwrap();
        assert_eq!(output, data);
    }

    #[test]
    fn limited_reader_fails_beyond_the_limit() {
        let data = vec![7u8; 101];
        let error = limited(data.as_slice(), 100)
            .read_to_end(&mut Vec::new())
            .unwrap_err();
        assert!(is_limit_exceeded(&error));
    }

    #[test]
    fn limited_decoder_stops_a_compression_bomb() {
        let bomb = vec![0u8; 4 * 1024 * 1024];
        for algorithm in ALGORITHMS {
            let compressed = compress_all(algorithm, &bomb);
            let reader = decoder(algorithm, compressed.as_slice()).unwrap();
            let error = limited(reader, 64 * 1024)
                .read_to_end(&mut Vec::new())
                .unwrap_err();
            assert!(is_limit_exceeded(&error), "{}", algorithm.name());
        }
    }

    #[test]
    fn other_io_errors_are_not_limit_errors() {
        assert!(!is_limit_exceeded(&io::Error::other("unrelated")));
    }

    fn round_trip(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut encoder = encoder(algorithm, None, Vec::new()).unwrap();
        encoder.write_all(data).unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(detect(&compressed), Some(algorithm));
        let mut output = Vec::new();
        decoder(algorithm, compressed.as_slice())
            .unwrap()
            .read_to_end(&mut output)
            .unwrap();
        output
    }

    #[test]
    fn every_algorithm_round_trips_and_is_detected() {
        let data: Vec<u8> = (0..20_000u32).flat_map(|n| n.to_le_bytes()).collect();
        for algorithm in ALGORITHMS {
            assert_eq!(round_trip(algorithm, &data), data, "{}", algorithm.name());
        }
    }

    #[test]
    fn empty_input_round_trips() {
        for algorithm in ALGORITHMS {
            assert!(
                round_trip(algorithm, b"").is_empty(),
                "{}",
                algorithm.name()
            );
        }
    }

    #[test]
    fn rejects_level_outside_range() {
        let error = encoder(Algorithm::Zstd, Some(23), Vec::new())
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn names_and_extensions_map_back() {
        for algorithm in ALGORITHMS {
            assert_eq!(Algorithm::from_name(algorithm.name()), Some(algorithm));
            assert_eq!(
                Algorithm::from_extension(algorithm.extension()),
                Some(algorithm)
            );
        }
    }
}
