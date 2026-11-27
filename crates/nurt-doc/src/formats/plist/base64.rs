// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::plist::base64
// Responsibility:   Base64 of `data` elements in XML property lists (RFC 4648, standard alphabet with padding).
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Base64 of `data` elements in XML property lists (RFC 4648, standard
//! alphabet with padding).

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Padding character.
const PAD: u8 = b'=';

/// Bits in one Base64 digit.
const DIGIT_BITS: u32 = 6;

/// Mask of the low six bits.
const DIGIT_MASK: u32 = 0x3F;

/// Encodes `bytes`.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().enumerate().fold(0u32, |group, (index, byte)| {
            group | (u32::from(*byte) << (16 - 8 * u32::try_from(index).unwrap_or(0)))
        });
        for position in 0..4usize {
            if position <= chunk.len() {
                let shift = 18 - DIGIT_BITS * u32::try_from(position).unwrap_or(0);
                let digit = usize::try_from((group >> shift) & DIGIT_MASK).unwrap_or(0);
                out.push(char::from(ALPHABET[digit]));
            } else {
                out.push(char::from(PAD));
            }
        }
    }
    out
}

/// Decodes Base64 text, ignoring ASCII white space; `None` for text that is
/// not Base64.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let body = digits
        .strip_suffix(b"==")
        .or_else(|| digits.strip_suffix(b"="))
        .unwrap_or(&digits);
    if body.contains(&PAD) || body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let mut group = 0u32;
    let mut bits = 0u32;
    for &byte in body {
        let digit = ALPHABET.iter().position(|&candidate| candidate == byte)?;
        group = (group << DIGIT_BITS) | u32::try_from(digit).ok()?;
        bits += DIGIT_BITS;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((group >> bits) & 0xFF).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rfc_4648_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in vectors {
            assert_eq!(encode(plain.as_bytes()), encoded);
            assert_eq!(decode(encoded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn decodes_wrapped_text_and_rejects_invalid_text() {
        assert_eq!(decode("Zm9v\n\tYmFy\n").unwrap(), b"foobar");
        assert_eq!(decode("Zg").unwrap(), b"f");
        assert!(decode("Z").is_none());
        assert!(decode("Zm=v").is_none());
        assert!(decode("Zm9*").is_none());
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
