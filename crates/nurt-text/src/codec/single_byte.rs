// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::codec::single_byte
// Responsibility:   Single-byte encodings of the WHATWG Encoding Standard.
// Dependencies:     encoding_rs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Single-byte encodings of the WHATWG Encoding Standard.
//!
//! The standard maps every byte of `windows-1250`, `iso-8859-2` and
//! `windows-1252` to a character, bytes undefined by the vendor tables
//! included (to C1 control characters), and each encoder is the inverse of
//! its decoder. Decoding these encodings therefore never fails and the round
//! trip is exact for all 256 byte values.

use encoding_rs::{DecoderResult, EncoderResult, Encoding};

use super::{Malformed, Unmappable};
use crate::eol::end_position;

pub(super) fn decode(bytes: &[u8], encoding: &'static Encoding) -> Result<String, Malformed> {
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let capacity = decoder
        .max_utf8_buffer_length_without_replacement(bytes.len())
        .unwrap_or(bytes.len());
    let mut text = String::with_capacity(capacity);
    let mut consumed = 0;
    loop {
        let rest = bytes.get(consumed..).unwrap_or_default();
        let (result, read) = decoder.decode_to_string_without_replacement(rest, &mut text, true);
        consumed += read;
        match result {
            DecoderResult::InputEmpty => return Ok(text),
            DecoderResult::OutputFull => text.reserve(rest.len().saturating_sub(read).max(1)),
            DecoderResult::Malformed(length, after) => {
                let end = consumed - usize::from(after);
                return Err(Malformed {
                    offset: end - usize::from(length),
                    length: usize::from(length),
                    position: end_position(&text),
                });
            }
        }
    }
}

pub(super) fn encode(
    text: &str,
    encoding: &'static Encoding,
    out: &mut Vec<u8>,
) -> Result<(), Unmappable> {
    let mut encoder = encoding.new_encoder();
    let mut consumed = 0;
    loop {
        let rest = text.get(consumed..).unwrap_or_default();
        let needed = encoder
            .max_buffer_length_from_utf8_without_replacement(rest.len())
            .unwrap_or(rest.len());
        out.reserve(needed);
        let (result, read) = encoder.encode_from_utf8_to_vec_without_replacement(rest, out, true);
        consumed += read;
        match result {
            EncoderResult::InputEmpty => return Ok(()),
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(character) => {
                return Err(Unmappable {
                    offset: consumed - character.len_utf8(),
                    character,
                });
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
