// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/codec_corpus.rs
// Responsibility:   Polish text in every supported encoding, checked against reference bytes produced by `iconv` and against byte-exact round trips.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Polish text in every supported encoding, checked against reference bytes
//! produced by `iconv` and against byte-exact round trips.
//!
//! Non-ASCII characters are written as escapes so that the sources stay
//! ASCII.

use nurt_contracts::services::{LineEnding, TextEncoding};
use nurt_text::codec::{Codec, CodecError, decode, encode};
use nurt_text::eol;

/// The 18 Polish letters with diacritical marks, upper case, then lower
/// case.
const POLISH_LETTERS: &str = "\u{104}\u{106}\u{118}\u{141}\u{143}\u{d3}\u{15a}\u{179}\u{17b}\
                              \u{105}\u{107}\u{119}\u{142}\u{144}\u{f3}\u{15b}\u{17a}\u{17c}";

/// Box drawing, shade and block characters of code page 852.
const BOX_DRAWING: &str = "\u{250c}\u{2500}\u{2510}\u{2502}\u{2514}\u{2518}\u{2554}\u{2550}\
                           \u{2557}\u{2551}\u{255a}\u{255d}\u{2591}\u{2592}\u{2593}\u{2588}";

/// The word `Zazolc` with its diacritical marks.
const ZAZOLC: &str = "Za\u{17c}\u{f3}\u{142}\u{107}";

/// The word `Gesla` with its diacritical marks.
const GESLA: &str = "G\u{119}\u{15b}l\u{105}";

/// Three Polish pangrams, each containing every Polish letter, with every
/// line terminator, a blank line and no final terminator.
const CORPUS: &str = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}.\r\n\
                      Pchn\u{105}\u{107} w t\u{119} \u{142}\u{f3}d\u{17a} je\u{17c}a lub \
                      o\u{15b}m skrzy\u{144} fig.\n\
                      \r\n\
                      M\u{118}\u{17b}NY B\u{104}D\u{179}, CHRO\u{143} PU\u{141}K TW\u{d3}J I \
                      SZE\u{15a}\u{106} FLAG.\r\
                      last line";

fn plain(codec: Codec) -> TextEncoding {
    codec.text_encoding(false)
}

#[test]
fn polish_letters_encode_to_reference_bytes() {
    let cases: [(Codec, &[u8]); 3] = [
        (
            Codec::Windows1250,
            b"\xa5\xc6\xca\xa3\xd1\xd3\x8c\x8f\xaf\xb9\xe6\xea\xb3\xf1\xf3\x9c\x9f\xbf",
        ),
        (
            Codec::Iso8859_2,
            b"\xa1\xc6\xca\xa3\xd1\xd3\xa6\xac\xaf\xb1\xe6\xea\xb3\xf1\xf3\xb6\xbc\xbf",
        ),
        (
            Codec::Ibm852,
            b"\xa4\x8f\xa8\x9d\xe3\xe0\x97\x8d\xbd\xa5\x86\xa9\x88\xe4\xa2\x98\xab\xbe",
        ),
    ];
    for (codec, expected) in cases {
        let encoding = plain(codec);
        assert_eq!(
            encode(POLISH_LETTERS, &encoding).unwrap(),
            expected,
            "{}",
            codec.label()
        );
        assert_eq!(
            decode(expected, &encoding).unwrap(),
            POLISH_LETTERS,
            "{}",
            codec.label()
        );
    }
}

#[test]
fn ibm852_box_drawing_encodes_to_reference_bytes() {
    let expected = b"\xda\xc4\xbf\xb3\xc0\xd9\xc9\xcd\xbb\xba\xc8\xbc\xb0\xb1\xb2\xdb";
    let encoding = plain(Codec::Ibm852);
    assert_eq!(encode(BOX_DRAWING, &encoding).unwrap(), expected);
    assert_eq!(decode(expected, &encoding).unwrap(), BOX_DRAWING);
}

/// First line of `dir` on a Windows console with the Polish locale, which
/// writes code page 852.
#[test]
fn ibm852_decodes_windows_console_listing() {
    let listing = b" Zawarto\x98\x86 katalogu C:\\proj\r\n";
    let text = decode(listing, &plain(Codec::Ibm852)).unwrap();
    assert_eq!(text, " Zawarto\u{15b}\u{107} katalogu C:\\proj\r\n");
}

#[test]
fn box_drawing_is_unrepresentable_in_windows_code_pages() {
    for codec in [Codec::Windows1250, Codec::Windows1252, Codec::Iso8859_2] {
        let error = encode(BOX_DRAWING, &plain(codec)).unwrap_err();
        assert!(
            matches!(
                error,
                CodecError::Unrepresentable {
                    character: '\u{250c}',
                    offset: 0,
                    ..
                }
            ),
            "{}",
            codec.label()
        );
    }
}

#[test]
fn windows_1252_encodes_western_text_to_reference_bytes() {
    let text = "caf\u{e9} \u{20ac}";
    let encoding = plain(Codec::Windows1252);
    assert_eq!(encode(text, &encoding).unwrap(), b"caf\xe9 \x80");
    assert_eq!(decode(b"caf\xe9 \x80", &encoding).unwrap(), text);
}

#[test]
fn utf16_encodes_to_reference_bytes() {
    let little = b"Z\x00a\x00\x7c\x01\xf3\x00\x42\x01\x07\x01 \x00\x3d\xd8\x00\xde";
    let big = b"\x00Z\x00a\x01\x7c\x00\xf3\x01\x42\x01\x07\x00 \xd8\x3d\xde\x00";
    let text = format!("{ZAZOLC} \u{1F600}");
    assert_eq!(encode(&text, &plain(Codec::Utf16Le)).unwrap(), little);
    assert_eq!(encode(&text, &plain(Codec::Utf16Be)).unwrap(), big);
    assert_eq!(decode(little, &plain(Codec::Utf16Le)).unwrap(), text);
    assert_eq!(decode(big, &plain(Codec::Utf16Be)).unwrap(), text);
}

#[test]
fn corpus_round_trips_in_every_encoding_that_represents_it() {
    for codec in Codec::ALL {
        if codec == Codec::Windows1252 {
            continue;
        }
        let boms: &[bool] = if codec.bom().is_some() {
            &[false, true]
        } else {
            &[false]
        };
        for &bom in boms {
            let encoding = codec.text_encoding(bom);
            let bytes = encode(CORPUS, &encoding).unwrap();
            assert_eq!(
                bom,
                Codec::from_bom(&bytes) == Some(codec),
                "{}",
                codec.label()
            );
            let text = decode(&bytes, &encoding).unwrap();
            assert_eq!(text, CORPUS, "{} bom {bom}", codec.label());
            assert_eq!(
                encode(&text, &encoding).unwrap(),
                bytes,
                "{} bom {bom}",
                codec.label()
            );
        }
    }
}

#[test]
fn corpus_keeps_mixed_line_endings_after_decoding() {
    for codec in [Codec::Utf16Le, Codec::Windows1250, Codec::Ibm852] {
        let encoding = plain(codec);
        let text = decode(&encode(CORPUS, &encoding).unwrap(), &encoding).unwrap();
        assert_eq!(eol::detect(text.as_bytes()), LineEnding::Mixed);
        assert!(!text.ends_with('\n'));
        assert_eq!(eol::lines(&text).count(), 5);
    }
}

#[test]
fn empty_content_round_trips_in_every_encoding() {
    for codec in Codec::ALL {
        let encoding = plain(codec);
        assert_eq!(decode(b"", &encoding).unwrap(), "");
        assert!(encode("", &encoding).unwrap().is_empty());
        if let Some(bom) = codec.bom() {
            let encoding = codec.text_encoding(true);
            assert_eq!(encode("", &encoding).unwrap(), bom);
            assert_eq!(decode(bom, &encoding).unwrap(), "");
        }
    }
}

#[test]
fn polish_letters_are_unrepresentable_in_windows_1252_with_position() {
    let text = "Pchn\u{105}\u{107}\r\nw t\u{119}";
    let error = encode(text, &plain(Codec::Windows1252)).unwrap_err();
    let CodecError::Unrepresentable {
        character,
        position,
        ..
    } = error
    else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(character, '\u{105}');
    assert_eq!((position.line, position.column), (1, 5));
}

#[test]
fn edit_of_windows_1250_file_keeps_encoding_and_line_endings() {
    let original_bytes = b"Za\xbf\xf3\xb3\xe6\r\nsecond\r\nZa\xbf\xf3\xb3\xe6";
    let encoding = plain(Codec::Windows1250);
    let original = decode(original_bytes, &encoding).unwrap();
    let edited = original
        .replace(ZAZOLC, GESLA)
        .replace("second\r\n", "second\nnew\n");
    let written = encode(&eol::preserve(&original, &edited), &encoding).unwrap();
    assert_eq!(
        written,
        b"G\xea\x9cl\xb9\r\nsecond\r\nnew\r\nG\xea\x9cl\xb9"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
