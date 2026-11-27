// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-archive::decompress
// Responsibility:   Whole-stream decompression, detection of the algorithm and the names of.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Whole-stream decompression, detection of the algorithm and the names of
//! decompressed files.

use std::io::{self, Cursor, Read, Write};

use crate::codec::{self, Algorithm, Error};

/// Number of leading bytes that identify every supported algorithm.
const SIGNATURE_LEN: usize = 6;

/// Size of the buffer used to move decompressed data to the output.
const COPY_BUFFER_LEN: usize = 64 * 1024;

/// Extensions that stand for a tar archive compressed with one algorithm.
const TAR_SHORT_EXTENSIONS: [&str; 4] = ["tgz", "tzst", "txz", "tbz2"];

/// Result of a successful decompression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decompressed {
    /// Algorithm of the input stream.
    pub algorithm: Algorithm,
    /// Number of compressed bytes consumed from the input.
    pub bytes_in: u64,
    /// Number of bytes written to the output.
    pub bytes_out: u64,
}

/// Returns the name of the decompressed form of `file_name` and the algorithm
/// its extension denotes.
///
/// The extension is compared without regard to case. The short forms `tgz`,
/// `tzst`, `txz` and `tbz2` are replaced by `tar`. Returns `None` when the
/// extension is not one of the supported algorithms or when nothing remains
/// of the name after removing it.
///
/// # Examples
///
/// ```
/// use nurt_archive::codec::Algorithm;
/// use nurt_archive::decompress::decompressed_name;
///
/// assert_eq!(decompressed_name("log.txt.gz"), Some(("log.txt".to_string(), Algorithm::Gzip)));
/// assert_eq!(decompressed_name("src.tgz"), Some(("src.tar".to_string(), Algorithm::Gzip)));
/// assert_eq!(decompressed_name(".gz"), None);
/// ```
pub fn decompressed_name(file_name: &str) -> Option<(String, Algorithm)> {
    let (stem, extension) = file_name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let algorithm = Algorithm::from_extension(extension)?;
    let is_tar_short_form = TAR_SHORT_EXTENSIONS
        .iter()
        .any(|short| extension.eq_ignore_ascii_case(short));
    let name = if is_tar_short_form {
        format!("{stem}.tar")
    } else {
        stem.to_string()
    };
    Some((name, algorithm))
}

/// Identifies the algorithm of a stream and returns a reader that yields the
/// whole stream again.
///
/// At most six bytes are consumed from `input` and replayed by the returned
/// reader.
///
/// # Errors
///
/// Returns [`Error::Format`] when the content matches no supported algorithm
/// and [`Error::Io`] when reading fails.
///
/// # Examples
///
/// ```
/// use std::io::Read;
/// use nurt_archive::codec::Algorithm;
/// use nurt_archive::compress::compress;
/// use nurt_archive::decompress::sniff;
///
/// let mut packed = Vec::new();
/// compress(Algorithm::Zstd, None, &b"data"[..], &mut packed)?;
/// let (algorithm, mut replay) = sniff(packed.as_slice())?;
/// assert_eq!(algorithm, Algorithm::Zstd);
/// let mut all = Vec::new();
/// replay.read_to_end(&mut all)?;
/// assert_eq!(all, packed);
/// # Ok::<(), nurt_archive::codec::Error>(())
/// ```
pub fn sniff<R: Read>(mut input: R) -> Result<(Algorithm, impl Read), Error> {
    let mut prefix = Vec::with_capacity(SIGNATURE_LEN);
    (&mut input)
        .take(SIGNATURE_LEN as u64)
        .read_to_end(&mut prefix)?;
    let algorithm = codec::detect(&prefix).ok_or_else(|| {
        Error::Format(
            "content matches no supported algorithm; name the algorithm explicitly".to_string(),
        )
    })?;
    Ok((algorithm, Cursor::new(prefix).chain(input)))
}

/// Decompresses `input` into `output`, taking the algorithm from the content.
///
/// Concatenated members are decoded as one stream. With a `limit`, output
/// beyond that many bytes is refused.
///
/// # Errors
///
/// Returns [`Error::Format`] for unrecognised, corrupt or truncated data,
/// [`Error::Limit`] when the output would exceed `limit` and [`Error::Io`]
/// when reading the input or writing the output fails.
///
/// # Examples
///
/// ```
/// use nurt_archive::codec::Algorithm;
/// use nurt_archive::compress::compress;
/// use nurt_archive::decompress::decompress;
///
/// let mut packed = Vec::new();
/// compress(Algorithm::Xz, None, &b"payload"[..], &mut packed)?;
/// let mut plain = Vec::new();
/// let result = decompress(packed.as_slice(), &mut plain, None)?;
/// assert_eq!(plain, b"payload");
/// assert_eq!(result.algorithm, Algorithm::Xz);
/// # Ok::<(), nurt_archive::codec::Error>(())
/// ```
pub fn decompress<R: Read + Send, W: Write>(
    input: R,
    output: W,
    limit: Option<u64>,
) -> Result<Decompressed, Error> {
    let (algorithm, replay) = sniff(input)?;
    decompress_as(algorithm, replay, output, limit)
}

/// Decompresses `input` into `output` as a stream of the given algorithm.
///
/// # Errors
///
/// Fails like [`decompress`], except that the algorithm is not detected.
pub fn decompress_as<R: Read + Send, W: Write>(
    algorithm: Algorithm,
    input: R,
    mut output: W,
    limit: Option<u64>,
) -> Result<Decompressed, Error> {
    let mut source = TrackedReader {
        inner: input,
        consumed: 0,
        failed: false,
    };
    let result = copy_decoded(algorithm, &mut source, &mut output, limit);
    match result {
        Ok(bytes_out) => Ok(Decompressed {
            algorithm,
            bytes_in: source.consumed,
            bytes_out,
        }),
        Err(CopyError::Output(error)) => Err(Error::Io(error)),
        Err(CopyError::Decode(error)) => Err(classify(error, source.failed, limit)),
    }
}

enum CopyError {
    Decode(io::Error),
    Output(io::Error),
}

fn copy_decoded<R: Read + Send, W: Write>(
    algorithm: Algorithm,
    input: &mut TrackedReader<R>,
    output: &mut W,
    limit: Option<u64>,
) -> Result<u64, CopyError> {
    let decoder = codec::decoder(algorithm, input).map_err(CopyError::Decode)?;
    let mut reader: Box<dyn Read + '_> = match limit {
        Some(limit) => Box::new(codec::limited(decoder, limit)),
        None => decoder,
    };
    let mut buffer = vec![0u8; COPY_BUFFER_LEN];
    let mut total = 0u64;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => return Ok(total),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(CopyError::Decode(error)),
        };
        output
            .write_all(&buffer[..count])
            .map_err(CopyError::Output)?;
        total += count as u64;
    }
}

fn classify(error: io::Error, input_failed: bool, limit: Option<u64>) -> Error {
    if codec::is_limit_exceeded(&error) {
        return Error::Limit {
            limit: limit.unwrap_or_default(),
        };
    }
    if input_failed {
        return Error::Io(error);
    }
    Error::Format(error.to_string())
}

struct TrackedReader<R> {
    inner: R,
    consumed: u64,
    failed: bool,
}

impl<R: Read> Read for TrackedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            Ok(count) => {
                self.consumed += count as u64;
                Ok(count)
            }
            Err(error) => {
                if error.kind() != io::ErrorKind::Interrupted {
                    self.failed = true;
                }
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::ALGORITHMS;
    use crate::compress::compress;

    fn pack(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut packed = Vec::new();
        compress(algorithm, None, data, &mut packed).unwrap();
        packed
    }

    fn noise(len: usize) -> Vec<u8> {
        let mut state = 0x9e37_79b9_u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 16) as u8
            })
            .collect()
    }

    #[test]
    fn strips_the_extension_and_reports_the_algorithm() {
        let cases = [
            ("a.txt.gz", "a.txt", Algorithm::Gzip),
            ("a.txt.zst", "a.txt", Algorithm::Zstd),
            ("a.txt.xz", "a.txt", Algorithm::Xz),
            ("a.txt.bz2", "a.txt", Algorithm::Bzip2),
            ("a.txt.lz4", "a.txt", Algorithm::Lz4),
            ("A.TXT.GZ", "A.TXT", Algorithm::Gzip),
        ];
        for (input, name, algorithm) in cases {
            assert_eq!(
                decompressed_name(input),
                Some((name.to_string(), algorithm))
            );
        }
    }

    #[test]
    fn expands_short_tar_extensions() {
        let cases = [
            ("a.tgz", Algorithm::Gzip),
            ("a.tzst", Algorithm::Zstd),
            ("a.txz", Algorithm::Xz),
            ("a.tbz2", Algorithm::Bzip2),
        ];
        for (input, algorithm) in cases {
            assert_eq!(
                decompressed_name(input),
                Some(("a.tar".to_string(), algorithm))
            );
        }
    }

    #[test]
    fn refuses_names_without_a_usable_extension() {
        for name in ["", "plain", "a.txt", ".gz", "a.", "gz"] {
            assert_eq!(decompressed_name(name), None, "{name:?}");
        }
    }

    #[test]
    fn removes_only_the_last_extension() {
        assert_eq!(
            decompressed_name("a.gz.gz"),
            Some(("a.gz".to_string(), Algorithm::Gzip))
        );
    }

    #[test]
    fn round_trips_every_algorithm_by_detection() {
        let data = noise(30_000);
        for algorithm in ALGORITHMS {
            let packed = pack(algorithm, &data);
            let mut plain = Vec::new();
            let result = decompress(packed.as_slice(), &mut plain, None).unwrap();
            assert_eq!(plain, data, "{}", algorithm.name());
            assert_eq!(result.algorithm, algorithm);
            assert_eq!(result.bytes_in, packed.len() as u64);
            assert_eq!(result.bytes_out, data.len() as u64);
        }
    }

    #[test]
    fn round_trips_every_level() {
        let data = noise(4_000);
        for algorithm in ALGORITHMS {
            let (low, high) = algorithm.level_range();
            for level in low..=high {
                let mut packed = Vec::new();
                compress(algorithm, Some(level), data.as_slice(), &mut packed).unwrap();
                let mut plain = Vec::new();
                decompress(packed.as_slice(), &mut plain, None).unwrap();
                assert_eq!(plain, data, "{} level {level}", algorithm.name());
            }
        }
    }

    #[test]
    fn rejects_unrecognised_content() {
        for input in [&b""[..], b"plain text", b"\x1f"] {
            let error = decompress(input, Vec::new(), None).unwrap_err();
            assert!(matches!(error, Error::Format(_)), "{input:?}");
        }
    }

    #[test]
    fn reports_corrupt_data_as_format_error() {
        let data = noise(8_000);
        for algorithm in ALGORITHMS {
            let mut packed = pack(algorithm, &data);
            let middle = packed.len() / 2;
            packed[middle] ^= 0xff;
            let error = decompress(packed.as_slice(), Vec::new(), None).unwrap_err();
            assert!(matches!(error, Error::Format(_)), "{}", algorithm.name());
        }
    }

    #[test]
    fn reports_truncated_data_as_format_error() {
        let data = noise(8_000);
        for algorithm in ALGORITHMS {
            let packed = pack(algorithm, &data);
            let cut = &packed[..packed.len() - 5];
            let error = decompress(cut, Vec::new(), None).unwrap_err();
            assert!(matches!(error, Error::Format(_)), "{}", algorithm.name());
        }
    }

    #[test]
    fn enforces_the_output_limit() {
        let bomb = vec![0u8; 2 * 1024 * 1024];
        for algorithm in ALGORITHMS {
            let packed = pack(algorithm, &bomb);
            let error = decompress(packed.as_slice(), Vec::new(), Some(1024)).unwrap_err();
            assert!(
                matches!(error, Error::Limit { limit: 1024 }),
                "{}",
                algorithm.name()
            );
        }
    }

    #[test]
    fn accepts_output_of_exactly_the_limit() {
        let data = vec![3u8; 5_000];
        let packed = pack(Algorithm::Gzip, &data);
        let mut plain = Vec::new();
        decompress(packed.as_slice(), &mut plain, Some(5_000)).unwrap();
        assert_eq!(plain, data);
    }

    #[test]
    fn decodes_concatenated_members() {
        for algorithm in ALGORITHMS {
            let mut packed = pack(algorithm, b"first;");
            packed.extend(pack(algorithm, b"second"));
            let mut plain = Vec::new();
            decompress(packed.as_slice(), &mut plain, None).unwrap();
            assert_eq!(plain, b"first;second", "{}", algorithm.name());
        }
    }

    #[test]
    fn reports_input_read_failure_as_io_error() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("device removed"))
            }
        }
        let prefix = pack(Algorithm::Gzip, &noise(1_000));
        let input = Cursor::new(prefix[..10].to_vec()).chain(Broken);
        let error = decompress(input, Vec::new(), None).unwrap_err();
        assert!(matches!(error, Error::Io(_)));
    }

    #[test]
    fn reports_output_write_failure_as_io_error() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let packed = pack(Algorithm::Zstd, b"content");
        let error = decompress(packed.as_slice(), Broken, None).unwrap_err();
        assert!(matches!(error, Error::Io(_)));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
