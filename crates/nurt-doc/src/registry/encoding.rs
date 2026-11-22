// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::{Error, ErrorCode, Result};

use crate::style::UTF8_BOM;

/// Byte order mark of UTF-16 little endian.
const UTF16LE_BOM: &[u8] = b"\xFF\xFE";
/// Byte order mark of UTF-16 big endian.
const UTF16BE_BOM: &[u8] = b"\xFE\xFF";

/// Encoding of a document file that a format reads as UTF-8 text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextEncoding {
    /// UTF-8 without a byte order mark.
    #[default]
    Utf8,
    /// UTF-8 with a byte order mark.
    Utf8Bom,
    /// UTF-16 little endian with a byte order mark.
    Utf16Le,
    /// UTF-16 big endian with a byte order mark.
    Utf16Be,
}

impl TextEncoding {
    /// Name of the encoding in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8-bom",
            Self::Utf16Le => "utf-16le",
            Self::Utf16Be => "utf-16be",
        }
    }

    /// Detects the encoding from the byte order mark and decodes `bytes`
    /// into UTF-8 text without the mark.
    ///
    /// # Errors
    ///
    /// Fails with `E_ENCODING` when the bytes are not valid in the detected
    /// encoding, with the offset of the first invalid byte.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::registry::TextEncoding;
    ///
    /// let (encoding, text) = TextEncoding::decode(b"\xFF\xFEa\x00")?;
    /// assert_eq!(encoding, TextEncoding::Utf16Le);
    /// assert_eq!(text, b"a");
    /// assert_eq!(encoding.encode(&text), b"\xFF\xFEa\x00");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn decode(bytes: &[u8]) -> Result<(Self, Vec<u8>)> {
        if let Some(body) = bytes.strip_prefix(UTF8_BOM) {
            return utf8(body, UTF8_BOM.len()).map(|text| (Self::Utf8Bom, text));
        }
        if let Some(body) = bytes.strip_prefix(UTF16LE_BOM) {
            return utf16(body, u16::from_le_bytes).map(|text| (Self::Utf16Le, text));
        }
        if let Some(body) = bytes.strip_prefix(UTF16BE_BOM) {
            return utf16(body, u16::from_be_bytes).map(|text| (Self::Utf16Be, text));
        }
        utf8(bytes, 0).map(|text| (Self::Utf8, text))
    }

    /// Encodes UTF-8 `text` back into this encoding, with its byte order
    /// mark. Invalid UTF-8 sequences are replaced by U+FFFD in UTF-16.
    #[must_use]
    pub fn encode(self, text: &[u8]) -> Vec<u8> {
        match self {
            Self::Utf8 => text.to_vec(),
            Self::Utf8Bom => [UTF8_BOM, text].concat(),
            Self::Utf16Le => encode_utf16(text, UTF16LE_BOM, u16::to_le_bytes),
            Self::Utf16Be => encode_utf16(text, UTF16BE_BOM, u16::to_be_bytes),
        }
    }
}

fn invalid(offset: usize, encoding: &str) -> Error {
    Error::new(
        ErrorCode::Encoding,
        format!(
            "the file is not valid {encoding} at byte {offset}; convert it to UTF-8 or UTF-16 with a byte order mark"
        ),
    )
    .with_detail("byte", offset.into())
}

fn utf8(body: &[u8], base: usize) -> Result<Vec<u8>> {
    match std::str::from_utf8(body) {
        Ok(_) => Ok(body.to_vec()),
        Err(error) => Err(invalid(base + error.valid_up_to(), "UTF-8")),
    }
}

fn utf16(body: &[u8], unit: fn([u8; 2]) -> u16) -> Result<Vec<u8>> {
    let base = UTF16LE_BOM.len();
    if body.len() % 2 != 0 {
        return Err(invalid(base + body.len() - 1, "UTF-16"));
    }
    let units = body.chunks_exact(2).map(|pair| unit([pair[0], pair[1]]));
    let mut text = String::with_capacity(body.len() / 2);
    let mut offset = base;
    for decoded in char::decode_utf16(units) {
        let c = decoded.map_err(|_| invalid(offset, "UTF-16"))?;
        offset += c.len_utf16() * 2;
        text.push(c);
    }
    Ok(text.into_bytes())
}

fn encode_utf16(text: &[u8], bom: &[u8], bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
    let text = String::from_utf8_lossy(text);
    let mut out = Vec::with_capacity(bom.len() + text.len() * 2);
    out.extend_from_slice(bom);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&bytes(unit));
    }
    out
}
