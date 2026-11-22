// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential tests of `nurt_text::codec` against `iconv`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;

use std::fs;
use std::path::Path;

use nurt_text::codec::{Codec, decode, encode};
use support_corpus::Rng;
use support_diff::{require, run_expect};

const ASCII: &str =
    " abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 .,;:!?-_()[]{}/+*=<>@#%&";
/// Polish letters, present in CP852, CP1250 and ISO-8859-2.
const POLISH: &str = "\u{105}\u{107}\u{119}\u{142}\u{144}\u{f3}\u{15b}\u{17a}\u{17c}\u{104}\u{106}\u{118}\u{141}\u{143}\u{d3}\u{15a}\u{179}\u{17b}";
/// Western European letters and the euro sign, present in Windows-1252.
const WESTERN: &str = "\u{e9}\u{e8}\u{fc}\u{df}\u{f1}\u{e7}\u{e0}\u{20ac}\u{c9}\u{dc}\u{d1}";
/// Characters outside the basic plane and from several scripts, for UTF-16.
const WIDE: &str = "\u{4e2d}\u{6587}\u{20ac}\u{1f600}\u{10348}\u{3b1}\u{142}";
const SAMPLES: u64 = 24;
const MIN_CHARS: u64 = 20;
const CHAR_SPREAD: u64 = 180;
const NEWLINE_ODDS: u64 = 12;
const FIRST_NON_ASCII: u8 = 0x80;

struct Case {
    codec: Codec,
    iconv: &'static str,
    alphabet: Vec<char>,
}

fn case(codec: Codec, iconv: &'static str, extra: &[&str]) -> Case {
    let mut alphabet: Vec<char> = ASCII.chars().collect();
    for part in extra {
        alphabet.extend(part.chars());
    }
    Case {
        codec,
        iconv,
        alphabet,
    }
}

fn cases() -> Vec<Case> {
    vec![
        case(Codec::Utf16Le, "UTF-16LE", &[POLISH, WESTERN, WIDE]),
        case(Codec::Utf16Be, "UTF-16BE", &[POLISH, WESTERN, WIDE]),
        case(Codec::Ibm852, "CP852", &[POLISH]),
        case(Codec::Windows1250, "CP1250", &[POLISH]),
        case(Codec::Iso8859_2, "ISO-8859-2", &[POLISH]),
        case(Codec::Windows1252, "WINDOWS-1252", &[WESTERN]),
    ]
}

fn sample(seed: u64, alphabet: &[char]) -> String {
    let mut rng = Rng::new(seed);
    let count = MIN_CHARS + rng.below(CHAR_SPREAD);
    let mut text = String::new();
    for _ in 0..count {
        if rng.below(NEWLINE_ODDS) == 0 {
            text.push('\n');
        } else {
            text.push(alphabet[rng.below(alphabet.len() as u64) as usize]);
        }
    }
    text.push('\n');
    text
}

/// Converts `input` with `iconv`; `None` when `iconv` rejects the input.
fn convert(iconv: &Path, dir: &Path, from: &str, to: &str, input: &[u8]) -> Option<Vec<u8>> {
    fs::write(dir.join("input.bin"), input).unwrap();
    let output = run_expect(iconv, &["-f", from, "-t", to, "input.bin"], dir, &[0, 1]);
    (output.code == Some(0)).then_some(output.stdout)
}

#[test]
fn encodes_like_iconv() {
    let Some(iconv) = require("iconv") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for case in cases() {
        let encoding = case.codec.text_encoding(false);
        for seed in 0..SAMPLES {
            let text = sample(seed, &case.alphabet);
            let reference = convert(&iconv, dir.path(), "UTF-8", case.iconv, text.as_bytes())
                .unwrap_or_else(|| panic!("iconv rejects the {} sample {seed}", case.iconv));
            assert_eq!(
                encode(&text, &encoding).unwrap(),
                reference,
                "{} sample {seed}",
                case.iconv
            );
        }
    }
}

#[test]
fn decodes_like_iconv() {
    let Some(iconv) = require("iconv") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for case in cases() {
        let encoding = case.codec.text_encoding(false);
        for seed in 0..SAMPLES {
            let text = sample(seed, &case.alphabet);
            let bytes = encode(&text, &encoding).unwrap();
            let reference = convert(&iconv, dir.path(), case.iconv, "UTF-8", &bytes)
                .unwrap_or_else(|| {
                    panic!("iconv rejects the {} bytes of sample {seed}", case.iconv)
                });
            assert_eq!(
                decode(&bytes, &encoding).unwrap().as_bytes(),
                reference.as_slice(),
                "{} sample {seed}",
                case.iconv
            );
        }
    }
}

#[test]
fn utf16_byte_order_mark_precedes_the_iconv_output() {
    let Some(iconv) = require("iconv") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let text = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}\u{1f600}\n";
    let marks: [(Codec, &str, [u8; 2]); 2] = [
        (Codec::Utf16Le, "UTF-16LE", [0xFF, 0xFE]),
        (Codec::Utf16Be, "UTF-16BE", [0xFE, 0xFF]),
    ];
    for (codec, name, mark) in marks {
        let mut expected = mark.to_vec();
        expected.extend(convert(&iconv, dir.path(), "UTF-8", name, text.as_bytes()).unwrap());
        let encoding = codec.text_encoding(true);
        let bytes = encode(text, &encoding).unwrap();
        assert_eq!(bytes, expected, "{name} with a byte order mark");
        assert_eq!(
            decode(&bytes, &encoding).unwrap(),
            text,
            "{name} round trip"
        );
    }
}

#[test]
fn single_byte_tables_match_iconv() {
    let Some(iconv) = require("iconv") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tables = [
        (Codec::Ibm852, "CP852"),
        (Codec::Windows1250, "CP1250"),
        (Codec::Iso8859_2, "ISO-8859-2"),
        (Codec::Windows1252, "WINDOWS-1252"),
    ];
    for (codec, name) in tables {
        let encoding = codec.text_encoding(false);
        let mut bytes = Vec::new();
        let mut text = String::new();
        for byte in FIRST_NON_ASCII..=u8::MAX {
            let Ok(decoded) = decode(&[byte], &encoding) else {
                continue;
            };
            let mut chars = decoded.chars();
            let (Some(character), None) = (chars.next(), chars.next()) else {
                continue;
            };
            if character.is_control() {
                continue;
            }
            bytes.push(byte);
            text.push(character);
        }
        let reference = convert(&iconv, dir.path(), "UTF-8", name, text.as_bytes())
            .unwrap_or_else(|| panic!("iconv cannot encode the {name} table decoded by Nurt"));
        assert_eq!(reference, bytes, "{name} table");
    }
}

#[test]
fn both_reject_characters_outside_the_code_page() {
    let Some(iconv) = require("iconv") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let rejected = "ab\u{4e2d}cd";
    for (codec, name) in [
        (Codec::Ibm852, "CP852"),
        (Codec::Windows1250, "CP1250"),
        (Codec::Iso8859_2, "ISO-8859-2"),
        (Codec::Windows1252, "WINDOWS-1252"),
    ] {
        assert!(
            convert(&iconv, dir.path(), "UTF-8", name, rejected.as_bytes()).is_none(),
            "iconv accepts the character for {name}"
        );
        assert!(
            encode(rejected, &codec.text_encoding(false)).is_err(),
            "Nurt accepts the character for {name}"
        );
    }
}
