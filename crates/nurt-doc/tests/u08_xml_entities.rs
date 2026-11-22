// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Entities, the document type declaration and the limits that protect
//! reading against entity expansion attacks.
// Helper functions of this test target are not `#[test]` functions, which
// `allow-unwrap-in-tests` covers.
#![allow(clippy::unwrap_used)]

mod u08_support;

use std::io::ErrorKind;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_doc::formats::xml::dtd::{
    AttributeDefault, AttributeType, DTD_INTERNAL_SUBSET, Dtd, EntityDefinition, ExternalId,
};
use nurt_doc::formats::xml::encoding::parse_document;
use nurt_doc::formats::xml::entity::{
    DtdEntities, ENTITY_MARKUP, EXTERNAL_ENTITY, MAX_ENTITY_DEPTH, MAX_EXPANSION_BYTES, Replacement,
};
use nurt_doc::formats::xml::{
    EntitySource, NOTABLE_ENTITY_PREFIX, Node, XmlChange, XmlEdit, XmlTree,
};
use u08_support::Codec;

/// Time within which a rejected expansion attack must fail.
const ATTACK_DEADLINE: Duration = Duration::from_secs(5);

/// Stack size of the thread that reads deeply nested documents; the size of
/// the main thread on common platforms.
const DEEP_STACK_BYTES: usize = 8 << 20;

/// Nesting depth of the deep element test.
const DEEP_ELEMENTS: usize = 10_000;

fn tree(text: &str) -> XmlTree {
    parse_document(text.as_bytes(), Some(&Codec)).unwrap().1
}

fn read_error(text: &str) -> Error {
    match parse_document(text.as_bytes(), Some(&Codec)) {
        Err(error) => error,
        Ok((_, tree)) => tree.to_value().unwrap_err(),
    }
}

fn value(text: &str) -> serde_json::Value {
    tree(text).to_value().unwrap().to_plain_json()
}

fn first_text(tree: &XmlTree) -> nurt_doc::formats::xml::NodeId {
    tree.node_ids()
        .find(|id| matches!(tree.node(*id), Node::Text(_)))
        .unwrap()
}

fn assert_construct(error: &Error, code: ErrorCode, construct: &str) {
    assert_eq!(error.code(), code, "{error}");
    assert_eq!(error.data().found.as_deref(), Some(construct), "{error}");
}

fn laughs(levels: usize, fan_out: usize) -> String {
    let mut subset = String::from("<!ENTITY l0 \"lol\">");
    for level in 1..levels {
        let refs = format!("&l{};", level - 1).repeat(fan_out);
        subset.push_str(&format!("<!ENTITY l{level} \"{refs}\">"));
    }
    format!("<!DOCTYPE a [{subset}]><a>&l{};</a>", levels - 1)
}

#[test]
fn dtd_reports_root_identifiers_and_internal_subset() {
    let text = r#"<!DOCTYPE book PUBLIC "-//Example//DTD Book//EN" "book.dtd" [<!ENTITY v "1">]>"#;
    let dtd = Dtd::parse(text, 10, false).unwrap();
    assert_eq!(dtd.root(), "book");
    assert_eq!(
        dtd.external_id(),
        Some(&ExternalId {
            public_id: Some("-//Example//DTD Book//EN".to_owned()),
            system_id: Some("book.dtd".to_owned()),
        })
    );
    assert_eq!(dtd.span(), 10..10 + text.len());
    let subset = dtd.internal_subset().unwrap();
    assert_eq!(
        &text[subset.start - 10..subset.end - 10],
        r#"<!ENTITY v "1">"#
    );
    assert!(dtd.has_unread_declarations());
    assert!(!dtd.is_standalone());
}

#[test]
fn dtd_keeps_the_first_declaration_of_an_entity() {
    let dtd = Dtd::parse(r#"<!DOCTYPE a [<!ENTITY e "1"><!ENTITY e "2">]>"#, 0, false).unwrap();
    assert_eq!(dtd.entities().len(), 1);
    assert_eq!(
        dtd.entity("e").unwrap().definition,
        EntityDefinition::Internal("1".to_owned())
    );
}

#[test]
fn dtd_classifies_entity_definitions() {
    let text = r#"<!DOCTYPE a [
        <!NOTATION png SYSTEM "image/png">
        <!ENTITY int "x&#38;#38;y&#10;">
        <!ENTITY ext SYSTEM "ext.xml">
        <!ENTITY pub PUBLIC "-//E//X//EN" "pub.xml">
        <!ENTITY pic SYSTEM "pic.png" NDATA png>
        <!ENTITY % par "ignored">
    ]>"#;
    let dtd = Dtd::parse(text, 0, false).unwrap();
    assert_eq!(
        dtd.entity("int").unwrap().definition,
        EntityDefinition::Internal("x&#38;y\n".to_owned())
    );
    assert!(matches!(
        dtd.entity("ext").unwrap().definition,
        EntityDefinition::External(_)
    ));
    assert!(matches!(
        dtd.entity("pub").unwrap().definition,
        EntityDefinition::External(_)
    ));
    assert!(matches!(
        &dtd.entity("pic").unwrap().definition,
        EntityDefinition::Unparsed { notation, .. } if notation == "png"
    ));
    assert!(dtd.entity("par").is_none());
    let span = dtd.entity("ext").unwrap().span.clone();
    assert_eq!(&text[span], r#"<!ENTITY ext SYSTEM "ext.xml">"#);
}

#[test]
fn dtd_reads_attribute_declarations() {
    let text = r#"<!DOCTYPE a [<!ATTLIST a id ID #REQUIRED kind (x|y) "x" fixed CDATA #FIXED "1" note CDATA #IMPLIED>]>"#;
    let dtd = Dtd::parse(text, 0, false).unwrap();
    let id = dtd.attribute("a", "id").unwrap();
    assert_eq!(id.kind, AttributeType::Id);
    assert!(id.kind.is_tokenized());
    assert_eq!(id.default, AttributeDefault::Required);
    let kind = dtd.attribute("a", "kind").unwrap();
    assert_eq!(
        kind.kind,
        AttributeType::Enumeration(vec!["x".to_owned(), "y".to_owned()])
    );
    assert!(matches!(
        &dtd.attribute("a", "fixed").unwrap().default,
        AttributeDefault::Value { literal, fixed: true, .. } if literal == "1"
    ));
    assert!(!dtd.attribute("a", "note").unwrap().kind.is_tokenized());
    assert_eq!(dtd.attributes_of("a").count(), 4);
    assert!(dtd.attribute("b", "id").is_none());
}

#[test]
fn declarations_after_a_parameter_entity_reference_are_not_processed() {
    let text = r#"<!DOCTYPE a [<!ENTITY % p SYSTEM "p.ent"> %p; <!ENTITY e "v">]>"#;
    let dtd = Dtd::parse(text, 0, false).unwrap();
    assert!(dtd.entity("e").is_none());
    let standalone = Dtd::parse(text, 0, true).unwrap();
    assert!(standalone.entity("e").is_some());
}

#[test]
fn predefined_and_character_references_are_replaced() {
    let json = value(r#"<a x="&lt;&#x41;&#66;">&amp;&#x1F600;&quot;</a>"#);
    assert_eq!(json["a"]["+@x"], "<AB");
    assert_eq!(json["a"]["+content"], "&\u{1F600}\"");
}

#[test]
fn internal_entities_expand_in_content_and_attributes() {
    let json = value(
        r#"<!DOCTYPE a [<!ENTITY v "1.0"><!ENTITY name "Nurt &v;">]><a ver="&v;">&name; &amp; more</a>"#,
    );
    assert_eq!(json["a"]["+@ver"], "1.0");
    assert_eq!(json["a"]["+content"], "Nurt 1.0 & more");
}

#[test]
fn entity_replacement_text_keeps_line_ends_and_converts_them_in_attributes() {
    let json = value("<!DOCTYPE a [<!ENTITY e \"x\r\ny\">]><a v=\"&e;\">&e;</a>");
    assert_eq!(json["a"]["+content"], "x\ny");
    assert_eq!(json["a"]["+@v"], "x y");
}

#[test]
fn tokenized_attribute_values_collapse_spaces() {
    let entities = DtdEntities::new(None);
    let decoded = entities
        .decode_attribute("  a \t b\r\n c ", 0, true)
        .unwrap();
    assert_eq!(decoded.value, "a b c");
    let cdata = entities.decode_attribute("  a \t b ", 0, false).unwrap();
    assert_eq!(cdata.value, "  a   b ");
}

#[test]
fn entity_source_reports_attribute_types_declared_as_tokenized() {
    let dtd = Dtd::parse(
        r#"<!DOCTYPE a [<!ATTLIST a id ID #IMPLIED kind (x|y) "x" note CDATA #IMPLIED><!ATTLIST a id CDATA #IMPLIED>]>"#,
        0,
        false,
    )
    .unwrap();
    let entities = DtdEntities::new(Some(dtd));
    assert!(EntitySource::tokenized(&entities, "a", "id"));
    assert!(entities.tokenized("a", "kind"));
    assert!(!entities.tokenized("a", "note"));
    assert!(!entities.tokenized("a", "other"));
    assert!(!entities.tokenized("b", "id"));
    assert!(!DtdEntities::new(None).tokenized("a", "id"));
}

#[test]
fn tokenized_attribute_values_are_normalized_when_read() {
    let json = value(concat!(
        r#"<!DOCTYPE a [<!ENTITY pad "  y  ">"#,
        r#"<!ATTLIST a id ID #IMPLIED refs IDREFS #IMPLIED note CDATA #IMPLIED>]>"#,
        "<a id=\"  x1 \" refs=\" x1 \t x2&pad;\r\n\" note=\"  keep  this \" other=\" as  is \"/>",
    ));
    assert_eq!(json["a"]["+@id"], "x1");
    assert_eq!(json["a"]["+@refs"], "x1 x2 y");
    assert_eq!(json["a"]["+@note"], "  keep  this ");
    assert_eq!(json["a"]["+@other"], " as  is ");
}

#[test]
fn tokenized_attribute_with_an_unchanged_value_keeps_its_bytes() {
    let text = r#"<!DOCTYPE a [<!ATTLIST a id ID #IMPLIED>]><a id=" x1 "><b>old</b></a>"#;
    let (encoding, tree) = parse_document(text.as_bytes(), Some(&Codec)).unwrap();
    let old = first_text(&tree);
    let change = tree.replace_text(old, "new").unwrap();
    let written = encoding
        .encode_change(&tree, &change, Some(&Codec))
        .unwrap();
    assert_eq!(
        String::from_utf8(written).unwrap(),
        text.replace("old", "new")
    );
}

#[test]
fn attribute_decoding_reports_entity_references_with_ranges() {
    let dtd = Dtd::parse(r#"<!DOCTYPE a [<!ENTITY e "v">]>"#, 0, false).unwrap();
    let entities = DtdEntities::new(Some(dtd));
    let decoded = entities.decode_attribute("x&e;&amp;", 100, false).unwrap();
    assert_eq!(decoded.value, "xv&");
    assert_eq!(decoded.entities.len(), 1);
    assert_eq!(decoded.entities[0].name, "e");
    assert_eq!(decoded.entities[0].span, 101..104);
}

#[test]
fn replacement_distinguishes_text_and_markup() {
    let dtd = Dtd::parse(
        r#"<!DOCTYPE a [<!ENTITY t "x &lt; y"><!ENTITY m "<b>&t;</b>">]>"#,
        0,
        false,
    )
    .unwrap();
    let entities = DtdEntities::new(Some(dtd));
    assert_eq!(
        entities.replacement("t", 0).unwrap(),
        Replacement::Text("x < y".to_owned())
    );
    assert_eq!(
        entities.replacement("m", 0).unwrap(),
        Replacement::Markup("<b>x &lt; y</b>".to_owned())
    );
    assert_eq!(
        entities.replacement("amp", 0).unwrap(),
        Replacement::Text("&".to_owned())
    );
}

#[test]
fn value_of_an_entity_with_markup_is_unsupported() {
    let error = read_error(r#"<!DOCTYPE a [<!ENTITY m "<b>x</b>">]><a>&m;</a>"#);
    assert_construct(&error, ErrorCode::UnsupportedConstruct, ENTITY_MARKUP);
}

#[test]
fn replacing_text_with_an_entity_reference_replaces_the_whole_node() {
    let tree = tree(r#"<!DOCTYPE a [<!ENTITY e "entity">]><a>before &e; after</a>"#);
    let text = first_text(&tree);
    let change = tree.replace_text(text, "new").unwrap();
    assert_eq!(
        change.apply(tree.text()).unwrap(),
        r#"<!DOCTYPE a [<!ENTITY e "entity">]><a>new</a>"#
    );
    assert_eq!(change.notable, vec![format!("{NOTABLE_ENTITY_PREFIX}e")]);
}

#[test]
fn unchanged_ranges_keep_references_byte_for_byte() {
    let text = r#"<!DOCTYPE a [<!ENTITY e "v">]><a><b>&e;&#x41;&lt;</b><c>old</c></a>"#;
    let (encoding, tree) = parse_document(text.as_bytes(), Some(&Codec)).unwrap();
    assert_eq!(
        encoding.encode(tree.text(), Some(&Codec)).unwrap(),
        text.as_bytes()
    );
    let old = tree
        .node_ids()
        .filter(|id| matches!(tree.node(*id), Node::Text(_)))
        .nth(1)
        .unwrap();
    let change = tree.replace_text(old, "new").unwrap();
    let written = encoding
        .encode_change(&tree, &change, Some(&Codec))
        .unwrap();
    assert_eq!(
        String::from_utf8(written).unwrap(),
        text.replace("<c>old</c>", "<c>new</c>")
    );
}

#[test]
fn external_file_entity_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "classified").unwrap();
    let url = format!("file://{}", secret.display());
    let text = format!(r#"<!DOCTYPE a [<!ENTITY x SYSTEM "{url}">]><a><b>&x;</b><c>ok</c></a>"#);
    let tree = tree(&text);
    let error = tree.to_value().unwrap_err();
    assert_construct(&error, ErrorCode::UnsupportedConstruct, EXTERNAL_ENTITY);
    assert!(!error.to_string().contains("classified"));
    let ok = tree
        .node_ids()
        .filter(|id| matches!(tree.node(*id), Node::Text(_)))
        .nth(1)
        .unwrap();
    assert_eq!(tree.chunk_value(ok).unwrap().as_deref(), Some("ok"));
}

#[test]
fn external_url_entity_is_never_fetched() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let text = format!(
        r#"<!DOCTYPE a [<!ENTITY x SYSTEM "http://127.0.0.1:{port}/x.xml"><!ENTITY y PUBLIC "-//E//Y//EN" "http://127.0.0.1:{port}/y.xml">]><a>&x;&y;</a>"#
    );
    let error = read_error(&text);
    assert_construct(&error, ErrorCode::UnsupportedConstruct, EXTERNAL_ENTITY);
    let accepted = listener.accept().map(drop).map_err(|error| error.kind());
    assert_eq!(accepted, Err(ErrorKind::WouldBlock));
}

#[test]
fn external_parameter_entity_makes_undeclared_entities_unsupported() {
    let error = read_error(
        r#"<!DOCTYPE a [<!ENTITY % ext SYSTEM "http://127.0.0.1:9/e.dtd"> %ext;]><a>&e;</a>"#,
    );
    assert_construct(&error, ErrorCode::UnsupportedConstruct, EXTERNAL_ENTITY);
}

#[test]
fn external_subset_makes_undeclared_entities_unsupported_unless_standalone() {
    let error = read_error(r#"<!DOCTYPE html SYSTEM "xhtml1-strict.dtd"><html>&nbsp;</html>"#);
    assert_construct(&error, ErrorCode::UnsupportedConstruct, EXTERNAL_ENTITY);
    let standalone = read_error(
        r#"<?xml version="1.0" standalone="yes"?><!DOCTYPE html SYSTEM "x.dtd"><html>&nbsp;</html>"#,
    );
    assert_eq!(standalone.code(), ErrorCode::DocParse);
}

#[test]
fn external_entity_in_an_attribute_is_malformed() {
    let error = read_error(r#"<!DOCTYPE a [<!ENTITY x SYSTEM "x.xml">]><a v="&x;"/>"#);
    assert_eq!(error.code(), ErrorCode::DocParse, "{error}");
}

#[test]
fn unparsed_entity_reference_is_malformed() {
    let error = read_error(
        r#"<!DOCTYPE a [<!NOTATION n SYSTEM "v"><!ENTITY u SYSTEM "u.bin" NDATA n>]><a>&u;</a>"#,
    );
    assert_eq!(error.code(), ErrorCode::DocParse, "{error}");
}

#[test]
fn recursive_entity_is_malformed() {
    let error = read_error(r#"<!DOCTYPE a [<!ENTITY a "&b;"><!ENTITY b "&a;">]><a>&a;</a>"#);
    assert_eq!(error.code(), ErrorCode::DocParse, "{error}");
}

#[test]
fn billion_laughs_fails_with_limit() {
    let started = Instant::now();
    let error = parse_document(laughs(10, 10).as_bytes(), Some(&Codec)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit, "{error}");
    assert!(started.elapsed() < ATTACK_DEADLINE);
}

#[test]
fn exponential_expansion_within_the_depth_limit_fails_with_limit() {
    let started = Instant::now();
    let error = parse_document(laughs(MAX_ENTITY_DEPTH, 10).as_bytes(), Some(&Codec)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit, "{error}");
    assert!(error.to_string().contains("bytes"), "{error}");
    assert!(started.elapsed() < ATTACK_DEADLINE);
}

#[test]
fn quadratic_blowup_fails_with_limit() {
    let block = "x".repeat(64 * 1024);
    let refs = "&big;".repeat(MAX_EXPANSION_BYTES / block.len() + 1);
    let text = format!(r#"<!DOCTYPE a [<!ENTITY big "{block}">]><a>{refs}</a>"#);
    let started = Instant::now();
    let error = parse_document(text.as_bytes(), Some(&Codec)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit, "{error}");
    assert!(started.elapsed() < ATTACK_DEADLINE);
}

#[test]
fn quadratic_blowup_in_attributes_fails_with_limit() {
    let block = "y".repeat(64 * 1024);
    let attributes: String = (0..=MAX_EXPANSION_BYTES / block.len())
        .map(|index| format!(" a{index}=\"&big;\""))
        .collect();
    let text = format!(r#"<!DOCTYPE a [<!ENTITY big "{block}">]><a{attributes}/>"#);
    let error = parse_document(text.as_bytes(), Some(&Codec)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit, "{error}");
}

#[test]
fn expansion_below_the_limit_can_be_read_repeatedly() {
    let block = "z".repeat(64 * 1024);
    let refs = "&big;".repeat(MAX_EXPANSION_BYTES / block.len() - 1);
    let text = format!(r#"<!DOCTYPE a [<!ENTITY big "{block}">]><a>{refs}</a>"#);
    let tree = tree(&text);
    for _ in 0..3 {
        let json = tree.to_value().unwrap().to_plain_json();
        assert!(json["a"].as_str().unwrap().len() < MAX_EXPANSION_BYTES);
    }
}

#[test]
fn nesting_up_to_the_depth_limit_is_expanded_and_deeper_nesting_fails() {
    let chain = |levels: usize| {
        let mut subset = String::from("<!ENTITY e1 \"x\">");
        for level in 2..=levels {
            subset.push_str(&format!("<!ENTITY e{level} \"&e{};\">", level - 1));
        }
        format!("<!DOCTYPE a [{subset}]><a>&e{levels};</a>")
    };
    assert_eq!(value(&chain(MAX_ENTITY_DEPTH))["a"], "x");
    let error = parse_document(chain(MAX_ENTITY_DEPTH + 1).as_bytes(), Some(&Codec)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit, "{error}");
}

#[test]
fn deeply_nested_elements_do_not_exhaust_the_stack() {
    let text = format!(
        "{}{}",
        "<e>".repeat(DEEP_ELEMENTS),
        "</e>".repeat(DEEP_ELEMENTS)
    );
    let outcome = std::thread::Builder::new()
        .stack_size(DEEP_STACK_BYTES)
        .spawn(move || {
            parse_document(text.as_bytes(), Some(&Codec))
                .and_then(|(_, tree)| tree.to_value())
                .map(drop)
        })
        .unwrap()
        .join()
        .unwrap();
    if let Err(error) = outcome {
        assert_eq!(error.code(), ErrorCode::Limit, "{error}");
    }
}

#[test]
fn edits_of_the_document_type_declaration_are_unsupported() {
    let text = r#"<!DOCTYPE a [<!ENTITY e "v">]><a>&e;</a>"#;
    let (encoding, tree) = parse_document(text.as_bytes(), Some(&Codec)).unwrap();
    let inside = text.find("\"v\"").unwrap();
    let mut change = XmlChange::default();
    change.push(XmlEdit::new(inside..inside + 3, "\"w\""));
    let error = encoding
        .encode_change(&tree, &change, Some(&Codec))
        .unwrap_err();
    assert_construct(&error, ErrorCode::UnsupportedConstruct, DTD_INTERNAL_SUBSET);
    let doctype_end = text.find("<a>").unwrap();
    let dtd = Dtd::parse(&text[..doctype_end], 0, false).unwrap();
    assert!(dtd.guard_edit(&(doctype_end..doctype_end)).is_ok());
    assert!(dtd.guard_edit(&(0..0)).is_ok());
    assert!(dtd.guard_edit(&(1..1)).is_err());
    assert!(dtd.guard_edit(&(doctype_end - 1..doctype_end + 2)).is_err());
}

#[test]
fn malformed_declarations_report_the_byte_offset() {
    let text = "<!DOCTYPE a [<!ENTITY e v>]>";
    let error = Dtd::parse(text, 40, false).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DocParse);
    let unquoted = 40 + text.find("v>").unwrap();
    assert_eq!(
        error.data().location.as_ref().unwrap().byte,
        u64::try_from(unquoted).ok()
    );
}
