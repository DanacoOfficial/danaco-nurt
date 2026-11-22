// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! IBM code page 852, the DOS Central European code page.
//!
//! Bytes below `0x80` are ASCII, including the control characters, as in the
//! Unicode Consortium mapping `CP852.TXT` and in `iconv`. The upper half uses
//! the tables of `oem_cp`, which are generated from that mapping and define
//! all 128 bytes, so decoding never fails.

use oem_cp::code_table::{DECODING_TABLE_CP852, ENCODING_TABLE_CP852};

use super::Unmappable;

const UPPER_HALF: u8 = 0x80;

pub(super) fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| decode_byte(byte)).collect()
}

fn decode_byte(byte: u8) -> char {
    match byte.checked_sub(UPPER_HALF) {
        Some(index) => DECODING_TABLE_CP852[usize::from(index)],
        None => char::from(byte),
    }
}

pub(super) fn encode(text: &str, out: &mut Vec<u8>) -> Result<(), Unmappable> {
    out.reserve(text.len());
    for (offset, character) in text.char_indices() {
        let byte = match u8::try_from(character) {
            Ok(ascii) if ascii < UPPER_HALF => Some(ascii),
            _ => ENCODING_TABLE_CP852.get(&character).copied(),
        };
        match byte {
            Some(byte) => out.push(byte),
            None => return Err(Unmappable { offset, character }),
        }
    }
    Ok(())
}
