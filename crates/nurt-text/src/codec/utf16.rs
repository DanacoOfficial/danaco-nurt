// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use super::Malformed;
use crate::eol::end_position;

const UNIT_SIZE: usize = 2;

/// Order of the two bytes of a UTF-16 code unit.
#[derive(Debug, Clone, Copy)]
pub(super) enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    fn unit(self, pair: [u8; UNIT_SIZE]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(pair),
            Self::Big => u16::from_be_bytes(pair),
        }
    }

    fn bytes(self, unit: u16) -> [u8; UNIT_SIZE] {
        match self {
            Self::Little => unit.to_le_bytes(),
            Self::Big => unit.to_be_bytes(),
        }
    }
}

/// Decodes UTF-16; an unpaired surrogate or a trailing odd byte is
/// malformed.
pub(super) fn decode(bytes: &[u8], order: ByteOrder) -> Result<String, Malformed> {
    let (pairs, tail) = bytes.as_chunks::<UNIT_SIZE>();
    let mut text = String::with_capacity(bytes.len());
    let mut offset = 0;
    for decoded in char::decode_utf16(pairs.iter().map(|&pair| order.unit(pair))) {
        let Ok(character) = decoded else {
            return Err(Malformed {
                offset,
                length: UNIT_SIZE,
                position: end_position(&text),
            });
        };
        text.push(character);
        offset += character.len_utf16() * UNIT_SIZE;
    }
    if !tail.is_empty() {
        return Err(Malformed {
            offset,
            length: tail.len(),
            position: end_position(&text),
        });
    }
    Ok(text)
}

/// Appends the UTF-16 form of `text` to `out`.
pub(super) fn encode(text: &str, order: ByteOrder, out: &mut Vec<u8>) {
    out.reserve(text.len() * UNIT_SIZE);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&order.bytes(unit));
    }
}
