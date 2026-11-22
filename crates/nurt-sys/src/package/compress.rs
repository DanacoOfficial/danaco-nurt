// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Transparent decompression of repository indexes.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

const GZIP: &[u8] = &[0x1f, 0x8b];
const ZSTD: &[u8] = &[0x28, 0xb5, 0x2f, 0xfd];
const XZ: &[u8] = &[0xfd, b'7', b'z', b'X', b'Z', 0x00];
const BZIP2: &[u8] = b"BZh";
const LZ4_FRAME: &[u8] = &[0x04, 0x22, 0x4d, 0x18];

/// Opens `path` and decodes it according to its signature.
///
/// The formats gzip, zstd, xz, bzip2 and the LZ4 frame are recognised; any
/// other content is returned as it is.
///
/// # Errors
///
/// Returns the error of the file system or of the decoder set-up.
pub(crate) fn open_decoded(path: &Path) -> io::Result<Box<dyn Read + Send>> {
    decode(BufReader::new(File::open(path)?))
}

/// Decodes `source` according to its signature.
///
/// # Errors
///
/// Returns the error of the source or of the decoder set-up.
pub(crate) fn decode<R: BufRead + Send + 'static>(mut source: R) -> io::Result<Box<dyn Read + Send>> {
    let head = source.fill_buf()?;
    if head.starts_with(GZIP) {
        Ok(Box::new(flate2::bufread::MultiGzDecoder::new(source)))
    } else if head.starts_with(ZSTD) {
        Ok(Box::new(zstd::stream::read::Decoder::with_buffer(source)?))
    } else if head.starts_with(XZ) {
        Ok(Box::new(liblzma::bufread::XzDecoder::new(source)))
    } else if head.starts_with(BZIP2) {
        Ok(Box::new(bzip2::bufread::BzDecoder::new(source)))
    } else if head.starts_with(LZ4_FRAME) {
        Ok(Box::new(lz4_flex::frame::FrameDecoder::new(source)))
    } else {
        Ok(Box::new(source))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;

    pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).expect("gzip write");
        encoder.finish().expect("gzip finish")
    }

    pub(crate) fn zstd(data: &[u8]) -> Vec<u8> {
        zstd::encode_all(data, 1).expect("zstd encode")
    }

    pub(crate) fn xz(data: &[u8]) -> Vec<u8> {
        let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 1);
        encoder.write_all(data).expect("xz write");
        encoder.finish().expect("xz finish")
    }

    pub(crate) fn bzip2(data: &[u8]) -> Vec<u8> {
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        encoder.write_all(data).expect("bzip2 write");
        encoder.finish().expect("bzip2 finish")
    }

    pub(crate) fn lz4(data: &[u8]) -> Vec<u8> {
        let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
        encoder.write_all(data).expect("lz4 write");
        encoder.finish().expect("lz4 finish")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read_all(bytes: Vec<u8>) -> String {
        let mut text = String::new();
        decode(Cursor::new(bytes))
            .expect("decoder")
            .read_to_string(&mut text)
            .expect("decoded text");
        text
    }

    #[test]
    fn decodes_every_supported_signature() {
        let data = b"Package: tool\n";
        assert_eq!(read_all(testing::gzip(data)), "Package: tool\n");
        assert_eq!(read_all(testing::zstd(data)), "Package: tool\n");
        assert_eq!(read_all(testing::xz(data)), "Package: tool\n");
        assert_eq!(read_all(testing::bzip2(data)), "Package: tool\n");
        assert_eq!(read_all(testing::lz4(data)), "Package: tool\n");
        assert_eq!(read_all(data.to_vec()), "Package: tool\n");
    }
}
