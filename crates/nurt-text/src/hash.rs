// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content digests.
//!
//! [`Hasher`] computes a digest incrementally, so content of any size is
//! hashed in fixed memory. [`content_hash`] and [`content_hash_reader`]
//! compute the SHA-256 [`ContentHash`] carried by entity tags and by the
//! journal.

use std::fmt;
use std::io::{self, Read, Write};
use std::str::FromStr;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::ContentHash;
use sha2::Digest as _;

/// Length of the longest digest, SHA-512, in bytes.
const MAX_DIGEST_SIZE: usize = 64;

/// Size of the buffer used to read content.
const READ_BUFFER_SIZE: usize = 64 * 1024;

/// Digest algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    /// SHA-256 (FIPS 180-4).
    Sha256,
    /// SHA-512 (FIPS 180-4).
    Sha512,
    /// SHA-1 (FIPS 180-4), for compatibility with existing checksums.
    Sha1,
    /// MD5 (RFC 1321), for compatibility with existing checksums.
    Md5,
    /// BLAKE3 with the default 256-bit output.
    Blake3,
    /// CRC-32 with the polynomial of IEEE 802.3, as in zlib, gzip and zip;
    /// rendered as eight hexadecimal digits in big-endian byte order.
    Crc32,
}

impl HashAlgorithm {
    /// Every algorithm, in the order of the catalog.
    pub const ALL: [Self; 6] = [
        Self::Sha256,
        Self::Sha512,
        Self::Sha1,
        Self::Md5,
        Self::Blake3,
        Self::Crc32,
    ];

    /// Name of the algorithm, as in the `algorithm` parameter and in the
    /// prefix of a digest.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::hash::HashAlgorithm;
    ///
    /// assert_eq!(HashAlgorithm::Blake3.name(), "blake3");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
            Self::Sha1 => "sha1",
            Self::Md5 => "md5",
            Self::Blake3 => "blake3",
            Self::Crc32 => "crc32",
        }
    }

    /// Length of the digest in bytes.
    #[must_use]
    pub const fn digest_size(self) -> usize {
        match self {
            Self::Sha256 | Self::Blake3 => 32,
            Self::Sha512 => 64,
            Self::Sha1 => 20,
            Self::Md5 => 16,
            Self::Crc32 => 4,
        }
    }
}

impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for HashAlgorithm {
    type Err = UnknownAlgorithm;

    /// Parses the name of an algorithm; the match is exact.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|algorithm| algorithm.name() == name)
            .ok_or_else(|| UnknownAlgorithm {
                name: name.to_owned(),
            })
    }
}

/// Error of parsing a [`HashAlgorithm`]; converts to `E_UNKNOWN_VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAlgorithm {
    /// The name as given.
    pub name: String,
}

impl fmt::Display for UnknownAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown hash algorithm `{}`: use one of ", self.name)?;
        for (index, algorithm) in HashAlgorithm::ALL.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            f.write_str(algorithm.name())?;
        }
        Ok(())
    }
}

impl std::error::Error for UnknownAlgorithm {}

impl From<UnknownAlgorithm> for Error {
    fn from(error: UnknownAlgorithm) -> Self {
        let message = error.to_string();
        Self::new(ErrorCode::UnknownValue, message)
            .with_expected(HashAlgorithm::ALL.map(HashAlgorithm::name).to_vec().into())
            .with_actual(error.name.into())
    }
}

#[derive(Clone)]
enum State {
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Sha1(sha1::Sha1),
    Md5(md5::Md5),
    Blake3(Box<blake3::Hasher>),
    Crc32(crc32fast::Hasher),
}

/// Incremental digest computation.
///
/// The hasher also implements [`Write`], so [`io::copy`] can feed it.
///
/// # Examples
///
/// ```
/// use nurt_text::hash::{HashAlgorithm, Hasher};
///
/// let mut hasher = Hasher::new(HashAlgorithm::Sha256);
/// hasher.update(b"a");
/// hasher.update(b"bc");
/// assert_eq!(
///     hasher.finalize().to_string(),
///     "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
/// );
/// ```
#[derive(Clone)]
pub struct Hasher {
    state: State,
}

impl Hasher {
    /// Starts a digest.
    #[must_use]
    pub fn new(algorithm: HashAlgorithm) -> Self {
        let state = match algorithm {
            HashAlgorithm::Sha256 => State::Sha256(sha2::Sha256::new()),
            HashAlgorithm::Sha512 => State::Sha512(sha2::Sha512::new()),
            HashAlgorithm::Sha1 => State::Sha1(sha1::Sha1::new()),
            HashAlgorithm::Md5 => State::Md5(md5::Md5::new()),
            HashAlgorithm::Blake3 => State::Blake3(Box::new(blake3::Hasher::new())),
            HashAlgorithm::Crc32 => State::Crc32(crc32fast::Hasher::new()),
        };
        Self { state }
    }

    /// Algorithm of the digest.
    #[must_use]
    pub fn algorithm(&self) -> HashAlgorithm {
        match self.state {
            State::Sha256(_) => HashAlgorithm::Sha256,
            State::Sha512(_) => HashAlgorithm::Sha512,
            State::Sha1(_) => HashAlgorithm::Sha1,
            State::Md5(_) => HashAlgorithm::Md5,
            State::Blake3(_) => HashAlgorithm::Blake3,
            State::Crc32(_) => HashAlgorithm::Crc32,
        }
    }

    /// Adds content.
    pub fn update(&mut self, bytes: &[u8]) {
        match &mut self.state {
            State::Sha256(state) => state.update(bytes),
            State::Sha512(state) => state.update(bytes),
            State::Sha1(state) => state.update(bytes),
            State::Md5(state) => state.update(bytes),
            State::Blake3(state) => {
                state.update(bytes);
            }
            State::Crc32(state) => state.update(bytes),
        }
    }

    /// Adds the rest of the content of a reader and returns the number of
    /// bytes read.
    ///
    /// # Errors
    ///
    /// Returns the first error of the reader other than
    /// [`io::ErrorKind::Interrupted`], after which the reading is retried.
    pub fn update_reader<R: Read>(&mut self, reader: R) -> io::Result<u64> {
        read_chunks(reader, |chunk| self.update(chunk))
    }

    /// Completes the digest.
    #[must_use]
    pub fn finalize(self) -> Digest {
        let algorithm = self.algorithm();
        let mut bytes = [0_u8; MAX_DIGEST_SIZE];
        let size = algorithm.digest_size();
        let output = &mut bytes[..size];
        match self.state {
            State::Sha256(state) => output.copy_from_slice(&state.finalize()),
            State::Sha512(state) => output.copy_from_slice(&state.finalize()),
            State::Sha1(state) => output.copy_from_slice(&state.finalize()),
            State::Md5(state) => output.copy_from_slice(&state.finalize()),
            State::Blake3(state) => output.copy_from_slice(state.finalize().as_bytes()),
            State::Crc32(state) => output.copy_from_slice(&state.finalize().to_be_bytes()),
        }
        Digest { algorithm, bytes }
    }
}

impl fmt::Debug for Hasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hasher")
            .field("algorithm", &self.algorithm())
            .finish_non_exhaustive()
    }
}

impl Write for Hasher {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Completed digest.
///
/// The text form, given by [`Display`](fmt::Display), is the algorithm name,
/// `:` and the digest in lower-case hexadecimal digits, as in the `hash`
/// field of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest {
    algorithm: HashAlgorithm,
    bytes: [u8; MAX_DIGEST_SIZE],
}

impl Digest {
    /// Algorithm of the digest.
    #[must_use]
    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    /// The digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.algorithm.digest_size()]
    }

    /// The digest in lower-case hexadecimal digits, without the algorithm
    /// name.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(self.algorithm.digest_size() * 2);
        for byte in self.as_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// The digest as a [`ContentHash`]; `None` unless the algorithm is
    /// SHA-256.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::hash::{HashAlgorithm, Hasher, content_hash};
    ///
    /// let digest = Hasher::new(HashAlgorithm::Sha256).finalize();
    /// assert_eq!(digest.content_hash(), Some(content_hash(b"")));
    /// ```
    #[must_use]
    pub fn content_hash(&self) -> Option<ContentHash> {
        match self.algorithm {
            HashAlgorithm::Sha256 => self.as_bytes().try_into().ok().map(ContentHash),
            _ => None,
        }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm.name(), self.to_hex())
    }
}

/// Computes the digest of the content of a reader.
///
/// # Errors
///
/// Returns the first error of the reader other than
/// [`io::ErrorKind::Interrupted`].
///
/// # Examples
///
/// ```
/// use nurt_text::hash::{HashAlgorithm, hash_reader};
///
/// let digest = hash_reader(HashAlgorithm::Crc32, &b"123456789"[..])?;
/// assert_eq!(digest.to_string(), "crc32:cbf43926");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn hash_reader<R: Read>(algorithm: HashAlgorithm, reader: R) -> io::Result<Digest> {
    let mut hasher = Hasher::new(algorithm);
    hasher.update_reader(reader)?;
    Ok(hasher.finalize())
}

/// SHA-256 content hash of bytes, as carried by entity tags.
///
/// # Examples
///
/// ```
/// use nurt_text::hash::content_hash;
///
/// assert_eq!(
///     content_hash(b"").to_string(),
///     "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
/// );
/// ```
#[must_use]
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(sha2::Sha256::digest(bytes).into())
}

/// SHA-256 content hash of the content of a reader, as carried by entity
/// tags.
///
/// # Errors
///
/// Returns the first error of the reader other than
/// [`io::ErrorKind::Interrupted`].
pub fn content_hash_reader<R: Read>(reader: R) -> io::Result<ContentHash> {
    let mut hasher = sha2::Sha256::new();
    read_chunks(reader, |chunk| hasher.update(chunk))?;
    Ok(ContentHash(hasher.finalize().into()))
}

/// Passes the content of a reader to `sink` in chunks and returns the number
/// of bytes read; reading is retried after [`io::ErrorKind::Interrupted`].
fn read_chunks<R: Read>(mut reader: R, mut sink: impl FnMut(&[u8])) -> io::Result<u64> {
    let mut buffer = vec![0_u8; READ_BUFFER_SIZE];
    let mut total = 0_u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(total),
            Ok(read) => {
                sink(&buffer[..read]);
                total += read as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Input of the official BLAKE3 test vectors: byte `i` is `i % 251`.
    fn blake3_input(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    fn hex_of(algorithm: HashAlgorithm, bytes: &[u8]) -> String {
        let mut hasher = Hasher::new(algorithm);
        hasher.update(bytes);
        hasher.finalize().to_hex()
    }

    /// Reader that returns one byte per call and interrupts every other
    /// call.
    struct Trickle<'a> {
        rest: &'a [u8],
        interrupt: bool,
    }

    impl Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.interrupt = !self.interrupt;
            if self.interrupt {
                return Err(io::ErrorKind::Interrupted.into());
            }
            match (self.rest.split_first(), buffer.first_mut()) {
                (Some((&byte, rest)), Some(slot)) => {
                    *slot = byte;
                    self.rest = rest;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    struct Failing;

    impl Read for Failing {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("device error"))
        }
    }

    #[test]
    fn sha256_matches_fips_180_vectors() {
        let cases = [
            (
                &b""[..],
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex_of(HashAlgorithm::Sha256, input), expected);
        }
    }

    #[test]
    fn sha256_of_one_million_a_matches_fips_180_vector() {
        let input = vec![b'a'; 1_000_000];
        assert_eq!(
            hex_of(HashAlgorithm::Sha256, &input),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha512_matches_fips_180_vectors() {
        let cases = [
            (
                &b""[..],
                "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
                 47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
            ),
            (
                b"abc",
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                 2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
            (
                b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmno\
                  ijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
                "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018\
                 501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex_of(HashAlgorithm::Sha512, input), expected);
        }
    }

    #[test]
    fn sha1_matches_fips_180_vectors() {
        let cases = [
            (&b""[..], "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            (b"abc", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "84983e441c3bd26ebaae4aa1f95129e5e54670f1",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex_of(HashAlgorithm::Sha1, input), expected);
        }
    }

    #[test]
    fn md5_matches_rfc_1321_vectors() {
        let cases = [
            (&b""[..], "d41d8cd98f00b204e9800998ecf8427e"),
            (b"a", "0cc175b9c0f1b6a831c399e269772661"),
            (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
            (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                b"abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex_of(HashAlgorithm::Md5, input), expected);
        }
    }

    #[test]
    fn blake3_matches_official_vectors() {
        let cases = [
            (
                0,
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            ),
            (
                1,
                "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
            ),
            (
                1023,
                "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
            ),
            (
                1024,
                "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
            ),
            (
                1025,
                "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
            ),
            (
                2048,
                "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
            ),
            (
                8193,
                "bab6c09cb8ce8cf459261398d2e7aef35700bf488116ceb94a36d0f5f1b7bc3b",
            ),
            (
                31744,
                "62b6960e1a44bcc1eb1a611a8d6235b6b4b78f32e7abc4fb4c6cdcce94895c47",
            ),
        ];
        for (len, expected) in cases {
            assert_eq!(
                hex_of(HashAlgorithm::Blake3, &blake3_input(len)),
                expected,
                "{len}"
            );
        }
    }

    #[test]
    fn crc32_matches_check_value() {
        assert_eq!(hex_of(HashAlgorithm::Crc32, b"123456789"), "cbf43926");
        assert_eq!(hex_of(HashAlgorithm::Crc32, b""), "00000000");
    }

    #[test]
    fn digest_has_algorithm_prefix_and_length() {
        for algorithm in HashAlgorithm::ALL {
            let digest = Hasher::new(algorithm).finalize();
            assert_eq!(digest.algorithm(), algorithm);
            assert_eq!(digest.as_bytes().len(), algorithm.digest_size());
            let text = digest.to_string();
            let (name, hex) = text.split_once(':').unwrap();
            assert_eq!(name, algorithm.name());
            assert_eq!(hex.len(), algorithm.digest_size() * 2);
        }
    }

    #[test]
    fn incremental_updates_equal_one_update() {
        let input = blake3_input(100_000);
        for algorithm in HashAlgorithm::ALL {
            let mut hasher = Hasher::new(algorithm);
            for chunk in input.chunks(4_093) {
                hasher.update(chunk);
            }
            assert_eq!(
                hasher.finalize().to_hex(),
                hex_of(algorithm, &input),
                "{algorithm}"
            );
        }
    }

    #[test]
    fn reader_hashing_retries_interrupted_reads() {
        let reader = Trickle {
            rest: b"abc",
            interrupt: false,
        };
        let digest = hash_reader(HashAlgorithm::Sha1, reader).unwrap();
        assert_eq!(digest.to_hex(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn reader_hashing_counts_bytes() {
        let input = blake3_input(200_000);
        let mut hasher = Hasher::new(HashAlgorithm::Md5);
        assert_eq!(hasher.update_reader(&input[..]).unwrap(), 200_000);
        assert_eq!(
            hasher.finalize().to_hex(),
            hex_of(HashAlgorithm::Md5, &input)
        );
    }

    #[test]
    fn reader_errors_are_returned() {
        let error = hash_reader(HashAlgorithm::Sha256, Failing).unwrap_err();
        assert_eq!(error.to_string(), "device error");
        assert!(content_hash_reader(Failing).is_err());
    }

    #[test]
    fn hasher_accepts_io_copy() {
        let mut hasher = Hasher::new(HashAlgorithm::Blake3);
        io::copy(&mut &blake3_input(1025)[..], &mut hasher).unwrap();
        hasher.flush().unwrap();
        assert_eq!(
            hasher.finalize().to_hex(),
            "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"
        );
    }

    #[test]
    fn content_hash_matches_sha256_digest() {
        let input = b"Zaz\xc3\xb3\xc5\x82\xc4\x87\r\n";
        let expected = content_hash(input);
        assert_eq!(content_hash_reader(&input[..]).unwrap(), expected);
        let digest = hash_reader(HashAlgorithm::Sha256, &input[..]).unwrap();
        assert_eq!(digest.content_hash(), Some(expected));
        assert_eq!(digest.to_string(), expected.to_string());
        assert_eq!(ContentHash::parse(&expected.to_string()), Ok(expected));
    }

    #[test]
    fn only_sha256_digest_is_a_content_hash() {
        for algorithm in HashAlgorithm::ALL {
            let digest = Hasher::new(algorithm).finalize();
            assert_eq!(
                digest.content_hash().is_some(),
                algorithm == HashAlgorithm::Sha256
            );
        }
    }

    #[test]
    fn parses_algorithm_names() {
        for algorithm in HashAlgorithm::ALL {
            assert_eq!(algorithm.name().parse(), Ok(algorithm));
            assert_eq!(algorithm.to_string(), algorithm.name());
        }
        assert_eq!(
            "SHA256".parse::<HashAlgorithm>(),
            Err(UnknownAlgorithm {
                name: "SHA256".to_owned()
            })
        );
    }

    #[test]
    fn unknown_algorithm_converts_to_unknown_value() {
        let error: Error = "sha3".parse::<HashAlgorithm>().unwrap_err().into();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        assert_eq!(
            error.message,
            "unknown hash algorithm `sha3`: use one of sha256, sha512, sha1, md5, blake3, crc32"
        );
        assert_eq!(error.actual, Some("sha3".into()));
    }

    #[test]
    fn hasher_debug_shows_algorithm() {
        let hasher = Hasher::new(HashAlgorithm::Crc32);
        assert_eq!(format!("{hasher:?}"), "Hasher { algorithm: Crc32, .. }");
        let copy = hasher.clone();
        assert_eq!(copy.algorithm(), HashAlgorithm::Crc32);
    }
}
