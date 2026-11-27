// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u08_xml_encoding.rs
// Responsibility:   Declared encodings of XML documents: detection, decoding, encoding back.
// Dependencies:     nurt_contracts, u08_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Declared encodings of XML documents: detection, decoding, encoding back
//! to the same bytes, and character references for characters that the
//! encoding cannot represent.
//!
//! UTF-16 fixtures are built in the tests, because the files of the corpus
//! carry a comment in ASCII that UTF-16 content cannot hold.
// Helper functions of this test target are not `#[test]` functions, which
// `allow-unwrap-in-tests` covers.
#![allow(clippy::unwrap_used)]

mod u08_support;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::services::{TextCodecService, TextDetection, TextEncoding};
use nurt_doc::formats::xml::encoding::{XmlContext, XmlEncoding, parse_document};
use nurt_doc::formats::xml::{PI_KEY_PREFIX, XmlChange, XmlEdit, XmlTree};
use u08_support::Codec;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: [u8; 2] = [0xFF, 0xFE];
const UTF16BE_BOM: [u8; 2] = [0xFE, 0xFF];

/// Declaration that the UTF-16 copies of corpus files replace.
const UTF8_DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

/// Minimum number of UTF-16 copies of corpus files.
const MIN_UTF16_COPIES: usize = 40;

/// Codec that adds a byte when encoding, so that decoded text never
/// encodes back to the same bytes.
struct LossyCodec;

impl TextCodecService for LossyCodec {
    fn detect(&self, bytes: &[u8]) -> TextDetection {
        Codec.detect(bytes)
    }

    fn decode(&self, bytes: &[u8], encoding: &TextEncoding) -> Result<String> {
        Codec.decode(bytes, encoding)
    }

    fn encode(&self, text: &str, encoding: &TextEncoding) -> Result<Vec<u8>> {
        let mut bytes = Codec.encode(text, encoding)?;
        bytes.push(b' ');
        Ok(bytes)
    }
}

fn single_byte(text: &str, label: &str) -> Vec<u8> {
    let (bytes, _, unmappable) = encoding_rs::Encoding::for_label(label.as_bytes())
        .unwrap()
        .encode(text);
    assert!(!unmappable, "{text} is not representable in {label}");
    bytes.into_owned()
}

fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend(if big_endian { UTF16BE_BOM } else { UTF16LE_BOM });
    }
    for unit in text.encode_utf16() {
        bytes.extend(if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        });
    }
    bytes
}

fn first_text(tree: &XmlTree) -> Option<String> {
    tree.chunk_value(tree.text_chunks(tree.root())[0]).unwrap()
}

/// Value of a document without its processing instructions, which hold the
/// declared encoding.
fn content_value(tree: &XmlTree) -> std::result::Result<serde_json::Value, ErrorCode> {
    let mut json = tree
        .to_value()
        .map_err(|error| error.code())?
        .to_plain_json();
    if let Some(object) = json.as_object_mut() {
        object.retain(|key, _| !key.starts_with(PI_KEY_PREFIX));
    }
    Ok(json)
}

fn edit_of(text: &str, marker: &str, replacement: &str) -> XmlChange {
    let start = text.find(marker).unwrap();
    let mut change = XmlChange::default();
    change.push(XmlEdit::new(start..start + marker.len(), replacement));
    change
}

/// ISO-8859-2 document with the markers `AV` (attribute value), `TX`
/// (text), `CM` (comment) and `QQ` (CDATA section).
fn iso_8859_2_document() -> (XmlEncoding, XmlTree, String) {
    let text =
        r#"<?xml version="1.0" encoding="ISO-8859-2"?><a v="AV">TX<!--CM--><![CDATA[QQ]]></a>"#;
    let (encoding, tree) = parse_document(&single_byte(text, "iso-8859-2"), Some(&Codec)).unwrap();
    (encoding, tree, text.to_owned())
}

#[test]
fn single_byte_encodings_decode_and_encode_back_to_the_same_bytes() {
    let cases = [
        (
            "ISO-8859-2",
            "iso-8859-2",
            "Plze\u{148}, Gy\u{151}r, \u{17D}ilina",
        ),
        ("latin2", "iso-8859-2", "\u{10C}akovec"),
        (
            "windows-1250",
            "windows-1250",
            "\u{201E}Brno\u{201C} \u{2013} Ko\u{161}ice",
        ),
        ("cp1250", "windows-1250", "\u{DA}st\u{ED}"),
        (
            "windows-1252",
            "windows-1252",
            "Preis 12 \u{20AC} \u{2013} \u{153}uvre",
        ),
    ];
    for (declared, label, value) in cases {
        let text =
            format!(r#"<?xml version="1.0" encoding="{declared}"?><a v="{value}">{value}</a>"#);
        let bytes = single_byte(&text, label);
        let (encoding, tree) = parse_document(&bytes, Some(&Codec)).unwrap();
        assert_eq!(encoding.label(), label);
        assert!(!encoding.is_unicode());
        assert_eq!(first_text(&tree).as_deref(), Some(value));
        assert_eq!(encoding.encode(tree.text(), Some(&Codec)).unwrap(), bytes);
    }
}

#[test]
fn iso_8859_1_maps_every_byte_to_the_code_point_of_the_same_value() {
    let mut bytes = br#"<?xml version="1.0" encoding="ISO-8859-1"?><a>"#.to_vec();
    bytes.extend([0x80, 0xE9, 0xFF]);
    bytes.extend(b"</a>");
    let (encoding, tree) = parse_document(&bytes, None).unwrap();
    assert_eq!(encoding.label(), "iso-8859-1");
    assert_eq!(first_text(&tree).as_deref(), Some("\u{80}\u{E9}\u{FF}"));
    assert_eq!(encoding.encode(tree.text(), None).unwrap(), bytes);
}

#[test]
fn us_ascii_accepts_only_ascii_bytes() {
    let ok = br#"<?xml version="1.0" encoding="US-ASCII"?><a>&#xE9;</a>"#;
    let (encoding, tree) = parse_document(ok, None).unwrap();
    assert_eq!(encoding.label(), "us-ascii");
    assert_eq!(first_text(&tree).as_deref(), Some("\u{E9}"));
    let bad = b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?><a>\xE9</a>";
    let error = parse_document(bad, None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DocParse);
    let position = bad.iter().position(|byte| *byte == 0xE9).unwrap();
    assert_eq!(
        error.data().location.as_ref().unwrap().byte,
        u64::try_from(position).ok()
    );
}

#[test]
fn invalid_utf8_is_rejected_with_the_byte_offset() {
    let bytes = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><a>\xFF</a>";
    let error = parse_document(bytes, None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DocParse);
    let position = bytes.iter().position(|byte| *byte == 0xFF).unwrap();
    assert_eq!(
        error.data().location.as_ref().unwrap().byte,
        u64::try_from(position).ok()
    );
}

#[test]
fn utf8_byte_order_mark_is_kept() {
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend(br#"<?xml version="1.0" encoding="UTF-8"?><a>x</a>"#);
    let (encoding, tree) = parse_document(&bytes, None).unwrap();
    assert!(encoding.has_bom());
    assert!(tree.text().starts_with("<?xml"));
    assert_eq!(encoding.encode(tree.text(), None).unwrap(), bytes);
}

#[test]
fn utf16_is_detected_with_and_without_a_byte_order_mark() {
    let text = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a v=\"\u{3A9}\">\u{1F600} \u{4E2D}</a>";
    let cases = [
        (false, true, "utf-16le"),
        (true, true, "utf-16be"),
        (false, false, "utf-16le"),
        (true, false, "utf-16be"),
    ];
    for (big_endian, bom, label) in cases {
        let bytes = utf16(text, big_endian, bom);
        let (encoding, tree) = parse_document(&bytes, Some(&Codec)).unwrap();
        assert_eq!(encoding.label(), label);
        assert_eq!(encoding.has_bom(), bom);
        assert!(encoding.is_unicode());
        assert_eq!(tree.text(), text);
        assert_eq!(encoding.encode(tree.text(), Some(&Codec)).unwrap(), bytes);
    }
}

#[test]
fn utf16_copies_of_the_corpus_read_like_the_utf8_originals() {
    let dir = tempfile::tempdir().unwrap();
    let mut copies = 0;
    for path in u08_support::files_below(&u08_support::corpus_dir("xml/advanced/dtd")) {
        let original = std::fs::read(&path).unwrap();
        let text = String::from_utf8(original.clone()).unwrap();
        let Some(rest) = text.strip_prefix(UTF8_DECLARATION) else {
            continue;
        };
        let (_, utf8_tree) = parse_document(&original, Some(&Codec)).unwrap();
        let utf16_text = format!(r#"<?xml version="1.0" encoding="UTF-16"?>{rest}"#);
        for (big_endian, suffix) in [(false, "le"), (true, "be")] {
            let name = path.file_stem().unwrap().to_string_lossy();
            let copy = dir.path().join(format!("{name}-utf-16{suffix}.xml"));
            std::fs::write(&copy, utf16(&utf16_text, big_endian, true)).unwrap();
            let bytes = std::fs::read(&copy).unwrap();
            let (encoding, tree) = parse_document(&bytes, Some(&Codec)).unwrap();
            assert_eq!(encoding.encode(tree.text(), Some(&Codec)).unwrap(), bytes);
            assert_eq!(
                content_value(&tree),
                content_value(&utf8_tree),
                "{}",
                copy.display()
            );
            copies += 1;
        }
    }
    assert!(copies >= MIN_UTF16_COPIES, "{copies} copies");
}

#[test]
fn declared_encoding_that_contradicts_the_content_is_rejected() {
    let mut utf8_bom = UTF8_BOM.to_vec();
    utf8_bom.extend(br#"<?xml version="1.0" encoding="ISO-8859-2"?><a/>"#);
    let utf16_declared_utf8 = utf16(r#"<?xml version="1.0" encoding="UTF-8"?><a/>"#, false, true);
    let single_byte_declared_utf16 = br#"<?xml version="1.0" encoding="UTF-16"?><a/>"#.to_vec();
    for bytes in [utf8_bom, utf16_declared_utf8, single_byte_declared_utf16] {
        let error = XmlEncoding::detect(&bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse, "{error}");
        assert!(error.to_string().contains("contradicts"), "{error}");
    }
}

#[test]
fn ucs4_and_ebcdic_documents_are_rejected() {
    let heads = [
        [0x00, 0x00, 0x00, 0x3C],
        [0x3C, 0x00, 0x00, 0x00],
        [0x00, 0x00, 0xFE, 0xFF],
        [0xFF, 0xFE, 0x00, 0x00],
        [0x4C, 0x6F, 0xA7, 0x94],
    ];
    for head in heads {
        let error = XmlEncoding::detect(&head).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse, "{head:?}");
    }
}

#[test]
fn encodings_without_a_supported_label_are_rejected() {
    for label in [
        "x-unknown-1",
        "replacement",
        "x-user-defined",
        "UTF-7",
        "ISO-2022-JP",
        "1abc",
    ] {
        let text = format!(r#"<?xml version="1.0" encoding="{label}"?><a/>"#);
        let error = XmlEncoding::detect(text.as_bytes()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse, "{label}");
    }
}

#[test]
fn codec_encodings_require_the_text_codec_service() {
    let bytes = single_byte(
        r#"<?xml version="1.0" encoding="windows-1250"?><a>Brno</a>"#,
        "windows-1250",
    );
    let error = parse_document(&bytes, None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DocParse);
    assert!(error.to_string().contains("text codec service"), "{error}");
}

#[test]
fn codec_that_does_not_encode_back_to_the_same_bytes_fails_with_e_encoding() {
    let bytes = single_byte(
        r#"<?xml version="1.0" encoding="windows-1250"?><a>Brno</a>"#,
        "windows-1250",
    );
    let error = parse_document(&bytes, Some(&LossyCodec)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encoding, "{error}");
}

#[test]
fn unrepresentable_characters_become_character_references_in_text_and_attributes() {
    let (encoding, tree, text) = iso_8859_2_document();
    let mut change = edit_of(&text, "TX", "\u{3A9}\u{17E}");
    change.merge(edit_of(&text, "AV", "\u{20AC}"));
    let written = encoding
        .encode_change(&tree, &change, Some(&Codec))
        .unwrap();
    let expected = text
        .replace("TX", "&#x03A9;\u{17E}")
        .replace("AV", "&#x20AC;");
    assert_eq!(written, single_byte(&expected, "iso-8859-2"));
    let (_, reread) = parse_document(&written, Some(&Codec)).unwrap();
    assert_eq!(first_text(&reread).as_deref(), Some("\u{3A9}\u{17E}"));
    let json = reread.to_value().unwrap().to_plain_json();
    assert_eq!(json["a"]["+@v"], "\u{20AC}");
}

#[test]
fn unrepresentable_characters_in_comments_and_cdata_fail_with_e_encoding() {
    let (encoding, tree, text) = iso_8859_2_document();
    for marker in ["CM", "QQ"] {
        let error = encoding
            .encode_change(&tree, &edit_of(&text, marker, "\u{3A9}"), Some(&Codec))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding, "{marker}: {error}");
    }
    let written = encoding
        .encode_change(&tree, &edit_of(&text, "CM", "\u{17E}"), Some(&Codec))
        .unwrap();
    assert!(written.contains(&0xBE));
}

#[test]
fn escaping_depends_on_the_encoding_and_the_context() {
    let utf8 = XmlEncoding::detect(b"<a/>").unwrap();
    assert_eq!(utf8.label(), "utf-8");
    assert_eq!(
        utf8.escape_unrepresentable("\u{4E2D}", XmlContext::Comment, None)
            .unwrap(),
        "\u{4E2D}"
    );
    let ascii = XmlEncoding::detect(br#"<?xml version="1.0" encoding="us-ascii"?><a/>"#).unwrap();
    assert_eq!(
        ascii
            .escape_unrepresentable("a\u{E9}", XmlContext::Attribute, None)
            .unwrap(),
        "a&#x00E9;"
    );
    assert_eq!(
        ascii
            .escape_unrepresentable("plain", XmlContext::Name, None)
            .unwrap(),
        "plain"
    );
    for context in [
        XmlContext::Name,
        XmlContext::ProcessingInstruction,
        XmlContext::CData,
    ] {
        let error = ascii
            .escape_unrepresentable("\u{E9}", context, None)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding, "{context:?}");
    }
}

#[test]
fn xml_declaration_is_reported() {
    let text = r#"<?xml version="1.0" encoding="windows-1250" standalone="yes"?><a/>"#;
    let encoding = XmlEncoding::detect(text.as_bytes()).unwrap();
    let declaration = encoding.declaration().unwrap();
    assert_eq!(declaration.version, "1.0");
    assert_eq!(declaration.encoding.as_deref(), Some("windows-1250"));
    assert_eq!(declaration.standalone, Some(true));
    assert_eq!(declaration.span, 0..text.find("?>").unwrap() + 2);
    assert!(
        XmlEncoding::detect(b"<a/>")
            .unwrap()
            .declaration()
            .is_none()
    );
}

#[test]
fn malformed_xml_declarations_are_rejected() {
    let declarations = [
        "<?xml encoding='UTF-8'?>",
        "<?xml version='2.0'?>",
        "<?xml version='1.0' standalone='maybe'?>",
        "<?xml version='1.0' encoding=UTF-8?>",
        "<?xml version='1.0'",
    ];
    for declaration in declarations {
        let bytes = format!("{declaration}<a/>");
        let error = XmlEncoding::detect(bytes.as_bytes()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse, "{declaration}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
