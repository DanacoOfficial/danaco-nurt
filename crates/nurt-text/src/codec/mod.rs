// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::codec
// Responsibility:   Character encodings of text files.
// Dependencies:     encoding_rs, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Character encodings of text files.
//!
//! [`decode`] and [`encode`] convert between bytes and text for the encodings
//! of [`Codec`]. For bytes that are valid in an encoding,
//! `encode(decode(bytes))` returns the same bytes, including the byte order
//! mark, every line terminator and a missing final terminator. Content for
//! which this cannot hold fails with [`CodecError`] instead of being changed.
//!
//! Labels follow the WHATWG Encoding Standard, lowercase, with `ibm852` for
//! the DOS Central European code page, which the standard does not define.

mod error;
mod ibm852;
mod single_byte;
mod utf16;

use encoding_rs::Encoding;
use nurt_contracts::services::TextEncoding;

pub use self::error::CodecError;
use self::utf16::ByteOrder;
use crate::eol::Position;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const UTF16LE_BOM: &[u8] = b"\xFF\xFE";
const UTF16BE_BOM: &[u8] = b"\xFE\xFF";

/// Supported character encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// `utf-8`.
    Utf8,
    /// `utf-16le`.
    Utf16Le,
    /// `utf-16be`.
    Utf16Be,
    /// `windows-1250`, the Windows Central European code page.
    Windows1250,
    /// `iso-8859-2`, Latin-2.
    Iso8859_2,
    /// `windows-1252`, the Windows Western European code page.
    Windows1252,
    /// `ibm852`, the DOS Central European code page (CP852).
    Ibm852,
}

impl Codec {
    /// Every supported encoding.
    pub const ALL: [Self; 7] = [
        Self::Utf8,
        Self::Utf16Le,
        Self::Utf16Be,
        Self::Windows1250,
        Self::Iso8859_2,
        Self::Windows1252,
        Self::Ibm852,
    ];

    /// Canonical label of the encoding.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::codec::Codec;
    ///
    /// assert_eq!(Codec::Ibm852.label(), "ibm852");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16Le => "utf-16le",
            Self::Utf16Be => "utf-16be",
            Self::Windows1250 => "windows-1250",
            Self::Iso8859_2 => "iso-8859-2",
            Self::Windows1252 => "windows-1252",
            Self::Ibm852 => "ibm852",
        }
    }

    /// Finds the encoding of a label.
    ///
    /// The label is matched without regard to case and surrounding
    /// whitespace. Besides the canonical labels, the aliases of the WHATWG
    /// Encoding Standard are accepted (`utf8`, `latin2`, `cp1250`; `utf-16`
    /// denotes `utf-16le`, and `latin1` and `ascii` denote `windows-1252`),
    /// as are `cp852` and `852` for `ibm852`.
    ///
    /// # Errors
    ///
    /// Fails with [`CodecError::UnknownEncoding`] when the label denotes no
    /// supported encoding.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::codec::Codec;
    ///
    /// assert_eq!(Codec::from_label("CP1250")?, Codec::Windows1250);
    /// assert_eq!(Codec::from_label("cp852")?, Codec::Ibm852);
    /// assert!(Codec::from_label("koi8-r").is_err());
    /// # Ok::<(), nurt_text::codec::CodecError>(())
    /// ```
    pub fn from_label(label: &str) -> Result<Self, CodecError> {
        let normalized = label.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "ibm852" | "cp852" | "852" | "cspcp852") {
            return Ok(Self::Ibm852);
        }
        Encoding::for_label_no_replacement(normalized.as_bytes())
            .and_then(|encoding| {
                Self::ALL
                    .into_iter()
                    .find(|codec| codec.whatwg() == Some(encoding))
            })
            .ok_or_else(|| CodecError::UnknownEncoding {
                label: label.to_owned(),
            })
    }

    /// Finds the encoding of a [`TextEncoding`] and checks its byte order
    /// mark flag.
    ///
    /// # Errors
    ///
    /// Fails with [`CodecError::UnknownEncoding`] for an unsupported label
    /// and with [`CodecError::BomNotSupported`] when `bom` is set for an
    /// encoding without a byte order mark.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::services::TextEncoding;
    /// use nurt_text::codec::Codec;
    ///
    /// let encoding = TextEncoding { label: "utf-16be".to_owned(), bom: true };
    /// assert_eq!(Codec::from_encoding(&encoding)?, Codec::Utf16Be);
    /// # Ok::<(), nurt_text::codec::CodecError>(())
    /// ```
    pub fn from_encoding(encoding: &TextEncoding) -> Result<Self, CodecError> {
        let codec = Self::from_label(&encoding.label)?;
        if encoding.bom && codec.bom().is_none() {
            return Err(CodecError::BomNotSupported { codec });
        }
        Ok(codec)
    }

    /// The encoding as a [`TextEncoding`] with the given byte order mark
    /// flag.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::codec::Codec;
    ///
    /// let encoding = Codec::Utf8.text_encoding(true);
    /// assert_eq!((encoding.label.as_str(), encoding.bom), ("utf-8", true));
    /// ```
    #[must_use]
    pub fn text_encoding(self, bom: bool) -> TextEncoding {
        TextEncoding {
            label: self.label().to_owned(),
            bom,
        }
    }

    /// Byte order mark of the encoding; `None` for single-byte encodings.
    #[must_use]
    pub const fn bom(self) -> Option<&'static [u8]> {
        match self {
            Self::Utf8 => Some(UTF8_BOM),
            Self::Utf16Le => Some(UTF16LE_BOM),
            Self::Utf16Be => Some(UTF16BE_BOM),
            Self::Windows1250 | Self::Iso8859_2 | Self::Windows1252 | Self::Ibm852 => None,
        }
    }

    /// Encoding whose byte order mark starts `bytes`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::codec::Codec;
    ///
    /// assert_eq!(Codec::from_bom(b"\xFF\xFEa\x00"), Some(Codec::Utf16Le));
    /// assert_eq!(Codec::from_bom(b"plain"), None);
    /// ```
    #[must_use]
    pub fn from_bom(bytes: &[u8]) -> Option<Self> {
        [Self::Utf8, Self::Utf16Le, Self::Utf16Be]
            .into_iter()
            .find(|codec| codec.bom().is_some_and(|bom| bytes.starts_with(bom)))
    }

    /// Encoding of `encoding_rs` with the same WHATWG name, used to resolve
    /// labels.
    fn whatwg(self) -> Option<&'static Encoding> {
        match self {
            Self::Utf8 => Some(encoding_rs::UTF_8),
            Self::Utf16Le => Some(encoding_rs::UTF_16LE),
            Self::Utf16Be => Some(encoding_rs::UTF_16BE),
            Self::Windows1250 => Some(encoding_rs::WINDOWS_1250),
            Self::Iso8859_2 => Some(encoding_rs::ISO_8859_2),
            Self::Windows1252 => Some(encoding_rs::WINDOWS_1252),
            Self::Ibm852 => None,
        }
    }
}

/// Byte sequence that is not valid in the encoding.
struct Malformed {
    /// Offset of the sequence in the decoded bytes.
    offset: usize,
    /// Length of the sequence.
    length: usize,
    /// Position after the last character decoded before the sequence.
    position: Position,
}

/// Character that the encoding cannot represent.
struct Unmappable {
    /// Byte offset of the character in the text.
    offset: usize,
    character: char,
}

/// Decodes text, without the byte order mark.
///
/// When `encoding.bom` is set, the content must start with the byte order
/// mark of the encoding, which is removed. When it is not set, a leading
/// byte order mark is kept as the character U+FEFF.
///
/// # Errors
///
/// Fails with [`CodecError::UnknownEncoding`] or
/// [`CodecError::BomNotSupported`] for an invalid `encoding`, with
/// [`CodecError::MissingBom`] when `encoding.bom` is set and the content does
/// not start with the byte order mark, and with [`CodecError::Malformed`] at
/// the first byte sequence that is not valid in the encoding.
///
/// # Examples
///
/// ```
/// use nurt_text::codec::{Codec, decode};
///
/// let text = decode(b"K\xf9\xf2\r\n", &Codec::Windows1250.text_encoding(false))?;
/// assert_eq!(text, "Kůň\r\n");
/// # Ok::<(), nurt_text::codec::CodecError>(())
/// ```
pub fn decode(bytes: &[u8], encoding: &TextEncoding) -> Result<String, CodecError> {
    let codec = Codec::from_encoding(encoding)?;
    let body = match codec.bom() {
        Some(bom) if encoding.bom => bytes
            .strip_prefix(bom)
            .ok_or(CodecError::MissingBom { codec })?,
        _ => bytes,
    };
    let decoded = match codec {
        Codec::Utf8 => decode_utf8(body),
        Codec::Utf16Le => utf16::decode(body, ByteOrder::Little),
        Codec::Utf16Be => utf16::decode(body, ByteOrder::Big),
        Codec::Windows1250 => single_byte::decode(body, encoding_rs::WINDOWS_1250),
        Codec::Iso8859_2 => single_byte::decode(body, encoding_rs::ISO_8859_2),
        Codec::Windows1252 => single_byte::decode(body, encoding_rs::WINDOWS_1252),
        Codec::Ibm852 => Ok(ibm852::decode(body)),
    };
    decoded.map_err(|malformed| {
        let start = bytes.len() - body.len() + malformed.offset;
        let sequence = bytes
            .get(start..start + malformed.length)
            .unwrap_or_default();
        CodecError::Malformed {
            codec,
            offset: start as u64,
            bytes: sequence.to_vec(),
            position: malformed.position,
        }
    })
}

/// Encodes text, writing a byte order mark when `encoding.bom` is set.
///
/// # Errors
///
/// Fails with [`CodecError::UnknownEncoding`] or
/// [`CodecError::BomNotSupported`] for an invalid `encoding`, and with
/// [`CodecError::Unrepresentable`] at the first character the encoding
/// cannot represent; no bytes are returned in that case.
///
/// # Examples
///
/// ```
/// use nurt_text::codec::{Codec, CodecError, encode};
///
/// let bytes = encode("Kůň", &Codec::Ibm852.text_encoding(false))?;
/// assert_eq!(bytes, b"K\x85\xe5");
///
/// let error = encode("a\n€", &Codec::Ibm852.text_encoding(false)).unwrap_err();
/// assert!(matches!(error, CodecError::Unrepresentable { character: '€', .. }));
/// # Ok::<(), nurt_text::codec::CodecError>(())
/// ```
pub fn encode(text: &str, encoding: &TextEncoding) -> Result<Vec<u8>, CodecError> {
    let codec = Codec::from_encoding(encoding)?;
    let mut out = Vec::with_capacity(text.len() + UTF8_BOM.len());
    if let (true, Some(bom)) = (encoding.bom, codec.bom()) {
        out.extend_from_slice(bom);
    }
    let encoded = match codec {
        Codec::Utf8 => {
            out.extend_from_slice(text.as_bytes());
            Ok(())
        }
        Codec::Utf16Le => {
            utf16::encode(text, ByteOrder::Little, &mut out);
            Ok(())
        }
        Codec::Utf16Be => {
            utf16::encode(text, ByteOrder::Big, &mut out);
            Ok(())
        }
        Codec::Windows1250 => single_byte::encode(text, encoding_rs::WINDOWS_1250, &mut out),
        Codec::Iso8859_2 => single_byte::encode(text, encoding_rs::ISO_8859_2, &mut out),
        Codec::Windows1252 => single_byte::encode(text, encoding_rs::WINDOWS_1252, &mut out),
        Codec::Ibm852 => ibm852::encode(text, &mut out),
    };
    encoded.map_err(|unmappable| CodecError::Unrepresentable {
        codec,
        character: unmappable.character,
        offset: unmappable.offset as u64,
        position: text
            .get(..unmappable.offset)
            .map_or(Position::START, crate::eol::end_position),
    })?;
    Ok(out)
}

fn decode_utf8(bytes: &[u8]) -> Result<String, Malformed> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) => {
            let offset = error.valid_up_to();
            let length = error
                .error_len()
                .unwrap_or(bytes.len().saturating_sub(offset));
            let position = bytes
                .get(..offset)
                .and_then(|valid| std::str::from_utf8(valid).ok())
                .map_or(Position::START, crate::eol::end_position);
            Err(Malformed {
                offset,
                length,
                position,
            })
        }
    }
}

#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
