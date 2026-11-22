// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Whole-stream compression and the names of compressed files.

use std::io::{self, Read, Write};

use crate::codec::{self, Algorithm, Error};

/// Sizes reported after a successful compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compressed {
    /// Number of bytes read from the input.
    pub bytes_in: u64,
    /// Number of bytes written to the output, including the stream trailer.
    pub bytes_out: u64,
}

/// Returns the name of the compressed form of `file_name`.
///
/// The extension of the algorithm is appended to the whole name, so
/// `data.tar` becomes `data.tar.gz` and a name that already carries the
/// extension is extended again. Returns `None` for an empty name.
///
/// # Examples
///
/// ```
/// use nurt_archive::codec::Algorithm;
/// use nurt_archive::compress::compressed_name;
///
/// assert_eq!(compressed_name("log.txt", Algorithm::Zstd).as_deref(), Some("log.txt.zst"));
/// assert_eq!(compressed_name("", Algorithm::Gzip), None);
/// ```
pub fn compressed_name(file_name: &str, algorithm: Algorithm) -> Option<String> {
    if file_name.is_empty() {
        return None;
    }
    Some(format!("{file_name}.{}", algorithm.extension()))
}

/// Compresses everything read from `input` and writes the stream to `output`.
///
/// `level` defaults to [`Algorithm::default_level`]. The output is not
/// flushed beyond the stream trailer; the caller owns `output` afterwards.
///
/// # Errors
///
/// Returns [`Error::InvalidLevel`] when the level is outside
/// [`Algorithm::level_range`] and [`Error::Io`] when reading or writing
/// fails.
///
/// # Examples
///
/// ```
/// use nurt_archive::codec::Algorithm;
/// use nurt_archive::compress::compress;
///
/// let mut packed = Vec::new();
/// let sizes = compress(Algorithm::Gzip, None, &b"hello hello hello"[..], &mut packed)?;
/// assert_eq!(sizes.bytes_in, 17);
/// assert_eq!(sizes.bytes_out, packed.len() as u64);
/// # Ok::<(), nurt_archive::codec::Error>(())
/// ```
pub fn compress<R: Read, W: Write>(
    algorithm: Algorithm,
    level: Option<u32>,
    mut input: R,
    output: W,
) -> Result<Compressed, Error> {
    let level = level.unwrap_or_else(|| algorithm.default_level());
    let (low, high) = algorithm.level_range();
    if !(low..=high).contains(&level) {
        return Err(Error::InvalidLevel {
            algorithm,
            level,
            low,
            high,
        });
    }
    let mut counter = CountingWriter {
        inner: output,
        written: 0,
    };
    let mut encoder = codec::encoder(algorithm, Some(level), &mut counter)?;
    let bytes_in = io::copy(&mut input, &mut encoder)?;
    encoder.finish()?;
    Ok(Compressed {
        bytes_in,
        bytes_out: counter.written,
    })
}

struct CountingWriter<W> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buf)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::ALGORITHMS;

    #[test]
    fn appends_the_extension_of_each_algorithm() {
        let expected = ["a.gz", "a.zst", "a.xz", "a.bz2", "a.lz4"];
        for (algorithm, name) in ALGORITHMS.into_iter().zip(expected) {
            assert_eq!(compressed_name("a", algorithm).as_deref(), Some(name));
        }
    }

    #[test]
    fn keeps_existing_extensions_and_hidden_names() {
        assert_eq!(
            compressed_name("a.tar", Algorithm::Xz).as_deref(),
            Some("a.tar.xz")
        );
        assert_eq!(
            compressed_name(".profile", Algorithm::Gzip).as_deref(),
            Some(".profile.gz")
        );
        assert_eq!(
            compressed_name("a.gz", Algorithm::Gzip).as_deref(),
            Some("a.gz.gz")
        );
    }

    #[test]
    fn rejects_an_empty_name() {
        assert_eq!(compressed_name("", Algorithm::Gzip), None);
    }

    #[test]
    fn rejects_a_level_outside_the_range() {
        let error = compress(Algorithm::Gzip, Some(10), &b"x"[..], Vec::new()).unwrap_err();
        assert!(matches!(error, Error::InvalidLevel { level: 10, .. }));
        let error = compress(Algorithm::Lz4, Some(1), &b"x"[..], Vec::new()).unwrap_err();
        assert!(matches!(error, Error::InvalidLevel { high: 0, .. }));
    }

    #[test]
    fn reports_input_and_output_sizes() {
        let data = vec![b'a'; 10_000];
        for algorithm in ALGORITHMS {
            let mut packed = Vec::new();
            let sizes = compress(algorithm, None, data.as_slice(), &mut packed).unwrap();
            assert_eq!(sizes.bytes_in, 10_000);
            assert_eq!(sizes.bytes_out, packed.len() as u64);
            assert!(sizes.bytes_out < sizes.bytes_in, "{}", algorithm.name());
        }
    }

    #[test]
    fn empty_input_produces_a_valid_stream() {
        for algorithm in ALGORITHMS {
            let mut packed = Vec::new();
            let sizes = compress(algorithm, None, io::empty(), &mut packed).unwrap();
            assert_eq!(sizes.bytes_in, 0);
            assert_eq!(codec::detect(&packed), Some(algorithm));
        }
    }

    #[test]
    fn write_failure_is_an_io_error() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let data = vec![1u8; 100_000];
        let error = compress(Algorithm::Gzip, None, data.as_slice(), Broken).unwrap_err();
        assert!(matches!(error, Error::Io(_)));
    }
}
