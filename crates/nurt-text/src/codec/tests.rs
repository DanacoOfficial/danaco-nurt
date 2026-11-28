// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::codec::tests
// Responsibility:   Implements the module nurt-text::codec::tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::services::TextEncoding;

use super::*;

fn plain(codec: Codec) -> TextEncoding {
    codec.text_encoding(false)
}

fn with_bom(codec: Codec) -> TextEncoding {
    codec.text_encoding(true)
}

#[test]
fn labels_are_canonical_and_resolve_to_themselves() {
    for codec in Codec::ALL {
        assert_eq!(Codec::from_label(codec.label()), Ok(codec));
        assert_eq!(codec.label(), codec.label().to_ascii_lowercase());
    }
}

#[test]
fn labels_match_whatwg_names() {
    for codec in Codec::ALL {
        if let Some(encoding) = codec.whatwg() {
            assert_eq!(encoding.name().to_ascii_lowercase(), codec.label());
        }
    }
}

#[test]
fn resolves_aliases_case_and_whitespace() {
    let cases = [
        ("UTF8", Codec::Utf8),
        (" utf-16 ", Codec::Utf16Le),
        ("UTF-16BE", Codec::Utf16Be),
        ("cp1250", Codec::Windows1250),
        ("latin2", Codec::Iso8859_2),
        ("ISO_8859-2", Codec::Iso8859_2),
        ("latin1", Codec::Windows1252),
        ("ascii", Codec::Windows1252),
        ("IBM852", Codec::Ibm852),
        ("cp852", Codec::Ibm852),
        ("852", Codec::Ibm852),
        ("csPCp852", Codec::Ibm852),
    ];
    for (label, codec) in cases {
        assert_eq!(Codec::from_label(label), Ok(codec), "{label}");
    }
}

#[test]
fn rejects_unsupported_labels() {
    for label in [
        "",
        "auto",
        "koi8-r",
        "shift_jis",
        "replacement",
        "x-user-defined",
    ] {
        assert_eq!(
            Codec::from_label(label),
            Err(CodecError::UnknownEncoding {
                label: label.to_owned()
            }),
            "{label}"
        );
    }
}

#[test]
fn only_unicode_encodings_have_a_bom() {
    assert_eq!(Codec::Utf8.bom(), Some(&b"\xEF\xBB\xBF"[..]));
    assert_eq!(Codec::Utf16Le.bom(), Some(&b"\xFF\xFE"[..]));
    assert_eq!(Codec::Utf16Be.bom(), Some(&b"\xFE\xFF"[..]));
    for codec in [
        Codec::Windows1250,
        Codec::Iso8859_2,
        Codec::Windows1252,
        Codec::Ibm852,
    ] {
        assert_eq!(codec.bom(), None);
    }
}

#[test]
fn finds_encoding_from_bom() {
    assert_eq!(Codec::from_bom(b"\xEF\xBB\xBFx"), Some(Codec::Utf8));
    assert_eq!(Codec::from_bom(b"\xFE\xFF\x00x"), Some(Codec::Utf16Be));
    assert_eq!(Codec::from_bom(b"\xFF\xFE"), Some(Codec::Utf16Le));
    assert_eq!(Codec::from_bom(b"\xEF\xBB"), None);
    assert_eq!(Codec::from_bom(b""), None);
}

#[test]
fn rejects_bom_for_single_byte_encodings() {
    let error = decode(b"a", &with_bom(Codec::Windows1250)).unwrap_err();
    assert_eq!(
        error,
        CodecError::BomNotSupported {
            codec: Codec::Windows1250
        }
    );
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(encode("a", &with_bom(Codec::Ibm852)).is_err());
}

#[test]
fn decode_removes_requested_bom() {
    assert_eq!(
        decode(b"\xEF\xBB\xBFa", &with_bom(Codec::Utf8)).unwrap(),
        "a"
    );
    assert_eq!(
        decode(b"\xFF\xFEa\x00", &with_bom(Codec::Utf16Le)).unwrap(),
        "a"
    );
    assert_eq!(
        decode(b"\xFE\xFF\x00a", &with_bom(Codec::Utf16Be)).unwrap(),
        "a"
    );
}

#[test]
fn decode_keeps_unrequested_bom_as_character() {
    let text = decode(b"\xEF\xBB\xBFa", &plain(Codec::Utf8)).unwrap();
    assert_eq!(text, "\u{FEFF}a");
    assert_eq!(
        encode(&text, &plain(Codec::Utf8)).unwrap(),
        b"\xEF\xBB\xBFa"
    );
}

#[test]
fn decode_requires_requested_bom() {
    let error = decode(b"a", &with_bom(Codec::Utf8)).unwrap_err();
    assert_eq!(error, CodecError::MissingBom { codec: Codec::Utf8 });
    assert_eq!(error.code(), ErrorCode::Encoding);
}

#[test]
fn encode_writes_requested_bom() {
    assert_eq!(encode("", &with_bom(Codec::Utf8)).unwrap(), b"\xEF\xBB\xBF");
    assert_eq!(
        encode("a", &with_bom(Codec::Utf16Le)).unwrap(),
        b"\xFF\xFEa\x00"
    );
    assert_eq!(
        encode("a", &with_bom(Codec::Utf16Be)).unwrap(),
        b"\xFE\xFF\x00a"
    );
    assert_eq!(encode("a", &plain(Codec::Utf16Be)).unwrap(), b"\x00a");
}

#[test]
fn utf8_malformed_sequence_reports_offset_and_position() {
    let error = decode(b"ok\r\nza\xC5\xBC\xC3\x28", &plain(Codec::Utf8)).unwrap_err();
    assert_eq!(
        error,
        CodecError::Malformed {
            codec: Codec::Utf8,
            offset: 8,
            bytes: vec![0xC3],
            position: Position { line: 2, column: 4 },
        }
    );
}

#[test]
fn utf8_truncated_sequence_at_end_is_malformed() {
    let error = decode(b"\xEF\xBB\xBFab\xE2\x82", &with_bom(Codec::Utf8)).unwrap_err();
    assert_eq!(
        error,
        CodecError::Malformed {
            codec: Codec::Utf8,
            offset: 5,
            bytes: vec![0xE2, 0x82],
            position: Position { line: 1, column: 3 },
        }
    );
}

#[test]
fn utf16_decodes_surrogate_pairs() {
    let bytes = b"\x3D\xD8\x00\xDE\x0A\x00";
    assert_eq!(
        decode(bytes, &plain(Codec::Utf16Le)).unwrap(),
        "\u{1F600}\n"
    );
    assert_eq!(
        encode("\u{1F600}\n", &plain(Codec::Utf16Le)).unwrap(),
        bytes
    );
    let big = b"\xD8\x3D\xDE\x00";
    assert_eq!(decode(big, &plain(Codec::Utf16Be)).unwrap(), "\u{1F600}");
}

#[test]
fn utf16_unpaired_surrogate_is_malformed() {
    let lone_high = b"a\x00\n\x00\x3D\xD8b\x00";
    let error = decode(lone_high, &plain(Codec::Utf16Le)).unwrap_err();
    assert_eq!(
        error,
        CodecError::Malformed {
            codec: Codec::Utf16Le,
            offset: 4,
            bytes: vec![0x3D, 0xD8],
            position: Position { line: 2, column: 1 },
        }
    );
    let lone_low = b"\xFE\xFF\xDE\x00";
    let error = decode(lone_low, &with_bom(Codec::Utf16Be)).unwrap_err();
    assert!(matches!(error, CodecError::Malformed { offset: 2, .. }));
}

#[test]
fn utf16_odd_length_is_malformed() {
    let error = decode(b"a\x00b", &plain(Codec::Utf16Le)).unwrap_err();
    assert_eq!(
        error,
        CodecError::Malformed {
            codec: Codec::Utf16Le,
            offset: 2,
            bytes: vec![b'b'],
            position: Position { line: 1, column: 2 },
        }
    );
}

#[test]
fn single_byte_encodings_decode_every_byte() {
    let all: Vec<u8> = (0..=u8::MAX).collect();
    for codec in [
        Codec::Windows1250,
        Codec::Iso8859_2,
        Codec::Windows1252,
        Codec::Ibm852,
    ] {
        let text = decode(&all, &plain(codec)).unwrap();
        assert_eq!(text.chars().count(), all.len(), "{}", codec.label());
        assert_eq!(
            encode(&text, &plain(codec)).unwrap(),
            all,
            "{}",
            codec.label()
        );
    }
}

#[test]
fn unrepresentable_character_reports_position() {
    let error = encode("Kůň\r\nžluťoučký →", &plain(Codec::Windows1250)).unwrap_err();
    assert_eq!(
        error,
        CodecError::Unrepresentable {
            codec: Codec::Windows1250,
            character: '→',
            offset: 21,
            position: Position {
                line: 2,
                column: 11
            },
        }
    );
    let error = encode("Kůň", &plain(Codec::Windows1252)).unwrap_err();
    assert!(matches!(
        error,
        CodecError::Unrepresentable {
            character: 'ů',
            position: Position { line: 1, column: 2 },
            ..
        }
    ));
    let error = encode("ok\n\u{1F600}", &plain(Codec::Ibm852)).unwrap_err();
    assert!(matches!(
        error,
        CodecError::Unrepresentable {
            character: '\u{1F600}',
            offset: 3,
            position: Position { line: 2, column: 1 },
            ..
        }
    ));
}

#[test]
fn unknown_encoding_converts_to_unknown_value() {
    let error: Error = Codec::from_label("ebcdic").unwrap_err().into();
    assert_eq!(error.code(), ErrorCode::UnknownValue);
    assert_eq!(
        error.message,
        "unknown encoding `ebcdic`: use one of utf-8, utf-16le, utf-16be, windows-1250, \
         iso-8859-2, windows-1252, ibm852"
    );
    assert_eq!(error.actual, Some("ebcdic".into()));
    assert!(error.expected.is_some());
}

#[test]
fn unrepresentable_converts_to_encoding_error_with_location_and_fix() {
    let error = encode("a\nb€", &plain(Codec::Iso8859_2))
        .unwrap_err()
        .into_error(Some("C:/proj/say \"hi\".txt"));
    assert_eq!(error.code(), ErrorCode::Encoding);
    assert_eq!(
        error.message,
        "character `€` (U+20AC) at line 2, column 2 cannot be represented in `iso-8859-2`: \
         convert the file to utf-8 or remove the character"
    );
    assert_eq!(error.found.as_deref(), Some("€"));
    assert_eq!(error.problem.as_deref(), Some("unrepresentable_character"));
    let location = error.location.as_ref().unwrap();
    assert_eq!(location.path, "C:/proj/say \"hi\".txt");
    assert_eq!(
        (location.line, location.column, location.byte),
        (Some(2), Some(2), Some(3))
    );
    assert_eq!(
        error.fix,
        ["convert \"C:/proj/say \\\"hi\\\".txt\" encoding: \"utf-8\""]
    );
    assert_eq!(error.details.get("character"), Some(&"U+20AC".into()));
}

#[test]
fn content_errors_without_path_have_no_location_or_fix() {
    let error: Error = encode("€", &plain(Codec::Ibm852)).unwrap_err().into();
    assert!(error.location.is_none());
    assert!(error.fix.is_empty());
    let error: Error = decode(b"\xFF", &plain(Codec::Utf8)).unwrap_err().into();
    assert!(error.location.is_none());
}

#[test]
fn malformed_converts_to_encoding_error() {
    let error = decode(b"a\n\xFF", &plain(Codec::Utf8))
        .unwrap_err()
        .into_error(Some("/srv/a.txt"));
    assert_eq!(error.code(), ErrorCode::Encoding);
    assert_eq!(
        error.message,
        "invalid `utf-8` byte sequence `ff` at line 2, column 1: read the file with its actual \
         encoding"
    );
    assert_eq!(error.found.as_deref(), Some("ff"));
    assert_eq!(error.details.get("byte"), Some(&2_u64.into()));
    assert_eq!(error.location.as_ref().and_then(|at| at.line), Some(2));
}

#[test]
fn missing_bom_converts_to_encoding_error() {
    let error = decode(b"a", &with_bom(Codec::Utf16Le))
        .unwrap_err()
        .into_error(Some("/srv/a.txt"));
    assert_eq!(error.code(), ErrorCode::Encoding);
    assert_eq!(
        error.message,
        "content does not start with the `utf-16le` byte order mark: decode it without the byte \
         order mark"
    );
    assert_eq!(error.problem.as_deref(), Some("missing_bom"));
    assert_eq!(error.location.as_ref().and_then(|at| at.byte), Some(0));
}

#[test]
fn bom_not_supported_converts_to_invalid_value() {
    let error: Error = Codec::from_encoding(&with_bom(Codec::Windows1252))
        .unwrap_err()
        .into();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(
        error.message,
        "encoding `windows-1252` has no byte order mark: set bom to false"
    );
}

#[test]
fn control_characters_are_escaped_in_messages() {
    let error = CodecError::Unrepresentable {
        codec: Codec::Ibm852,
        character: '\u{7}',
        offset: 0,
        position: Position::START,
    };
    assert!(error.to_string().starts_with("character `\\u{7}` (U+0007)"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
