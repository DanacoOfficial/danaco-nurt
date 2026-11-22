// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Streaming compression codecs.
//!
//! Each [`Algorithm`] has a decoder that reads a compressed stream and an
//! encoder that writes one. Decoders accept concatenated members (gzip
//! multi-member files, several zstd frames, multi-stream xz and bzip2), as
//! produced by tools that append compressed output.

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

const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];
const ZSTD_MAGIC: &[u8] = &[0x28, 0xb5, 0x2f, 0xfd];
const XZ_MAGIC: &[u8] = &[0xfd, b'7', b'z', b'X', b'Z', 0x00];
const BZIP2_MAGIC: &[u8] = b"BZh";
const LZ4_MAGIC: &[u8] = &[0x04, 0x22, 0x4d, 0x18];

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

    fn magic(self) -> &'static [u8] {
        match self {
            Self::Gzip => GZIP_MAGIC,
            Self::Zstd => ZSTD_MAGIC,
            Self::Xz => XZ_MAGIC,
            Self::Bzip2 => BZIP2_MAGIC,
            Self::Lz4 => LZ4_MAGIC,
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
        .find(|algorithm| prefix.starts_with(algorithm.magic()))
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
        Algorithm::Lz4 => Box::new(lz4_flex::frame::FrameDecoder::new(inner)),
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
            EncoderKind::Zstd(zstd::stream::write::Encoder::new(inner, level)?)
        }
        Algorithm::Xz => EncoderKind::Xz(liblzma::write::XzEncoder::new(inner, level)),
        Algorithm::Bzip2 => EncoderKind::Bzip2(bzip2::write::BzEncoder::new(
            inner,
            bzip2::Compression::new(level),
        )),
        Algorithm::Lz4 => EncoderKind::Lz4(lz4_flex::frame::FrameEncoder::new(inner)),
    };
    Ok(Encoder { kind })
}

#[cfg(test)]
mod tests {
    use super::*;

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
