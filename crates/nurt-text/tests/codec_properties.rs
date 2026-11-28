// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/codec_properties.rs
// Responsibility:   Round-trip properties of the codecs, 10 000 cases each with a fixed seed.
// Dependencies:     proptest
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Round-trip properties of the codecs, 10 000 cases each with a fixed seed.

use nurt_text::codec::{Codec, decode, encode};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

const CASES: u32 = 10_000;
const SEED: u64 = 0x6e75_7274_7465_7874;

const SINGLE_BYTE: [Codec; 4] = [
    Codec::Windows1250,
    Codec::Iso8859_2,
    Codec::Windows1252,
    Codec::Ibm852,
];

const UNICODE: [Codec; 3] = [Codec::Utf8, Codec::Utf16Le, Codec::Utf16Be];

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: CASES,
        rng_seed: RngSeed::Fixed(SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn single_byte_encodings_round_trip_any_bytes(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        for codec in SINGLE_BYTE {
            let encoding = codec.text_encoding(false);
            let text = decode(&bytes, &encoding).unwrap();
            prop_assert_eq!(&encode(&text, &encoding).unwrap(), &bytes, "{}", codec.label());
        }
    }

    #[test]
    fn unicode_encodings_round_trip_any_text(text in any::<String>(), bom in any::<bool>()) {
        for codec in UNICODE {
            let encoding = codec.text_encoding(bom);
            let bytes = encode(&text, &encoding).unwrap();
            let decoded = decode(&bytes, &encoding).unwrap();
            prop_assert_eq!(&decoded, &text, "{}", codec.label());
            prop_assert_eq!(&encode(&decoded, &encoding).unwrap(), &bytes, "{}", codec.label());
        }
    }

    #[test]
    fn valid_utf16_bytes_round_trip(units in prop::collection::vec(any::<u16>(), 0..64)) {
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        let encoding = Codec::Utf16Le.text_encoding(false);
        let valid = char::decode_utf16(units.iter().copied()).all(|unit| unit.is_ok());
        match decode(&bytes, &encoding) {
            Ok(text) => {
                prop_assert!(valid);
                prop_assert_eq!(encode(&text, &encoding).unwrap(), bytes);
            }
            Err(_) => prop_assert!(!valid),
        }
    }

    #[test]
    fn decode_rejects_exactly_invalid_utf8(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let result = decode(&bytes, &Codec::Utf8.text_encoding(false));
        prop_assert_eq!(result.is_ok(), std::str::from_utf8(&bytes).is_ok());
    }

    #[test]
    fn representable_text_round_trips_in_single_byte_encodings(
        indices in prop::collection::vec(any::<u8>(), 0..128)
    ) {
        for codec in SINGLE_BYTE {
            let encoding = codec.text_encoding(false);
            let alphabet: Vec<char> = decode(&(0..=u8::MAX).collect::<Vec<u8>>(), &encoding)
                .unwrap()
                .chars()
                .collect();
            let text: String = indices.iter().map(|&index| alphabet[usize::from(index)]).collect();
            let bytes = encode(&text, &encoding).unwrap();
            prop_assert_eq!(&bytes, &indices, "{}", codec.label());
            prop_assert_eq!(&decode(&bytes, &encoding).unwrap(), &text, "{}", codec.label());
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
