// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u07_xml_edit.rs
// Responsibility:   Edits of XML documents: text and CDATA, attributes, empty elements, new and removed elements, namespaces and changes of the value type.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Edits of XML documents: text and CDATA, attributes, empty elements, new
//! and removed elements, namespaces and changes of the value type.
//!
//! Every edit is applied to the source text, the result is parsed again
//! and the addressed value is compared with the value that was set.

// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::{Fields, Value};
use nurt_doc::formats::xml::{
    CONSTRUCT_DOCUMENT_ROOT, CONSTRUCT_MIXED_CONTENT, CONSTRUCT_NAMESPACE_DECLARATION,
    CONSTRUCT_NESTED_LIST, XmlChange, XmlEdit, XmlTree,
};

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn map(members: &[(&str, Value)]) -> Value {
    let mut fields = Fields::new();
    for (name, value) in members {
        fields.insert((*name).to_owned(), value.clone());
    }
    Value::Map(fields)
}

/// Applies `set` and checks that the result parses again.
fn set_with(source: &str, path: &str, value: &Value, create: bool) -> (String, XmlChange) {
    let tree = XmlTree::parse(source).unwrap();
    let change = tree.set(&key(path), value, create).unwrap();
    let output = change.apply(source).unwrap();
    XmlTree::parse(&output).unwrap_or_else(|error| panic!("{error}\n{output}"));
    (output, change)
}

fn set(source: &str, path: &str, value: &Value) -> String {
    set_with(source, path, value, false).0
}

fn create(source: &str, path: &str, value: &Value) -> String {
    set_with(source, path, value, true).0
}

fn read(source: &str, path: &str) -> Value {
    XmlTree::parse(source).unwrap().read(&key(path)).unwrap()[0]
        .value
        .clone()
}

fn remove(source: &str, path: &str) -> String {
    let tree = XmlTree::parse(source).unwrap();
    tree.remove(&key(path)).unwrap().apply(source).unwrap()
}

fn set_error(
    source: &str,
    path: &str,
    value: &Value,
    create: bool,
) -> nurt_contracts::error::Error {
    XmlTree::parse(source)
        .unwrap()
        .set(&key(path), value, create)
        .unwrap_err()
}

#[test]
fn text_is_escaped_and_surrounding_whitespace_kept() {
    let out = set("<r><a>old</a></r>", "r.a", &text("x < y & z > w ]]> q"));
    assert_eq!(out, "<r><a>x &lt; y &amp; z &gt; w ]]&gt; q</a></r>");
    assert_eq!(read(&out, "r.a"), text("x < y & z > w ]]> q"));
    let out = set("<r>\n  <v>\n    1.0\n  </v>\n</r>", "r.v", &text("2.0"));
    assert_eq!(out, "<r>\n  <v>\n    2.0\n  </v>\n</r>");
}

#[test]
fn line_breaks_in_new_text_follow_the_document_line_ending() {
    let out = set("<r>\r\n  <a>x</a>\r\n</r>", "r.a", &text("1\n2"));
    assert_eq!(out, "<r>\r\n  <a>1\r\n2</a>\r\n</r>");
    assert_eq!(read(&out, "r.a"), text("1\n2"));
}

#[test]
fn setting_the_same_value_changes_nothing() {
    let source =
        "<r a='1'>\n  <b>x &amp; y</b>\n  <c><![CDATA[1 < 2]]></c>\n  <d/>\n  <e k='v'>t</e>\n</r>";
    let tree = XmlTree::parse(source).unwrap();
    for (path, value) in [
        ("r.@a", text("1")),
        ("r.b", text("x & y")),
        ("r.c", text("1 < 2")),
        ("r.d", Value::Null),
        ("r.e", map(&[("+content", text("t")), ("+@k", text("v"))])),
        ("r.e.#text", text("t")),
    ] {
        assert!(
            tree.set(&key(path), &value, false).unwrap().is_empty(),
            "{path}"
        );
    }
}

#[test]
fn cdata_stays_cdata_and_is_split_around_its_terminator() {
    let out = set("<r><s><![CDATA[a < b]]></s></r>", "r.s", &text("c && d"));
    assert_eq!(out, "<r><s><![CDATA[c && d]]></s></r>");
    let out = set("<r><s><![CDATA[a]]></s></r>", "r.s", &text("x]]>y"));
    assert_eq!(out, "<r><s><![CDATA[x]]]]><![CDATA[>y]]></s></r>");
    assert_eq!(read(&out, "r.s"), text("x]]>y"));
    let out = set(&out, "r.s", &text("z"));
    assert_eq!(out, "<r><s><![CDATA[z]]></s></r>");
    let out = set("<r><s>\n  <![CDATA[ a ]]>\n</s></r>", "r.s", &text("b"));
    assert_eq!(out, "<r><s>\n  <![CDATA[ b ]]>\n</s></r>");
}

#[test]
fn mixed_content_chunks_are_edited_separately() {
    let source = "<p>First <b>bold</b> then tail.</p>";
    let out = set(source, "p.#text[1]", &text("and more."));
    assert_eq!(out, "<p>First <b>bold</b> and more.</p>");
    let out = set(source, "p.#text[0]", &text("Start"));
    assert_eq!(out, "<p>Start <b>bold</b> then tail.</p>");
    let error = set_error(source, "p.#text", &text("x"), false);
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

#[test]
fn attribute_keeps_its_quote_and_escapes_markup() {
    let out = set(r#"<a x="1" y='2'/>"#, "a.@x", &text(r#"say "hi" & <go>"#));
    assert_eq!(out, r#"<a x="say &quot;hi&quot; &amp; &lt;go>" y='2'/>"#);
    let out = set(r#"<a x="1" y='2'/>"#, "a.@y", &text("it's \"ok\""));
    assert_eq!(out, r#"<a x="1" y='it&apos;s "ok"'/>"#);
    let out = set(r#"<a x="1"/>"#, "a.@x", &text("a\tb\nc"));
    assert_eq!(out, r#"<a x="a&#9;b&#10;c"/>"#);
    assert_eq!(read(&out, "a.@x"), text("a\tb\nc"));
}

#[test]
fn new_attribute_follows_the_last_with_its_layout() {
    let source = "<config\n    name='c'\n    version='1'>\n</config>";
    let out = create(source, "config.@enabled", &text("true"));
    assert_eq!(
        out,
        "<config\n    name='c'\n    version='1'\n    enabled='true'>\n</config>"
    );
    let out = create("<a x = \"1\"/>", "a.@y", &Value::Int(2));
    assert_eq!(out, "<a x = \"1\" y = \"2\"/>");
    let out = create("<r q='1'><a/></r>", "r.a.@y", &Value::Bool(true));
    assert_eq!(out, "<r q='1'><a y='true'/></r>");
}

#[test]
fn attribute_is_removed_with_the_whitespace_before_it() {
    assert_eq!(remove(r#"<a x="1"  y="2"/>"#, "a.@y"), r#"<a x="1"/>"#);
    assert_eq!(
        remove("<a\n  x=\"1\"\n  y=\"2\">t</a>", "a.@x"),
        "<a\n  y=\"2\">t</a>"
    );
}

#[test]
fn empty_element_is_expanded_for_text_or_children() {
    assert_eq!(set("<r><a/></r>", "r.a", &text("t")), "<r><a>t</a></r>");
    assert_eq!(
        set("<r><a x='1' /></r>", "r.a", &text("t")),
        "<r><a x='1'>t</a></r>"
    );
    assert_eq!(
        set("<r><a\n   x='1'\n/></r>", "r.a", &text("t")),
        "<r><a\n   x='1'\n>t</a></r>"
    );
    assert_eq!(
        create("<r><a/></r>", "r.a.#text", &text("t")),
        "<r><a>t</a></r>"
    );
    let out = create("<r>\n  <a/>\n</r>", "r.a.b", &text("1"));
    assert_eq!(out, "<r>\n  <a>\n    <b>1</b>\n  </a>\n</r>");
}

#[test]
fn new_element_takes_sibling_indentation() {
    let source = "<deps>\n\t<dep>a</dep>\n\t<dep>b</dep>\n</deps>\n";
    let (out, change) = set_with(source, "deps.dep[+]", &text("c"), false);
    assert_eq!(
        out,
        "<deps>\n\t<dep>a</dep>\n\t<dep>b</dep>\n\t<dep>c</dep>\n</deps>\n"
    );
    assert_eq!(change.keys_created, ["deps.dep[2]"]);
    assert_eq!(set(source, "deps.dep[2]", &text("c")), out);
    let error = set_error(source, "deps.dep[5]", &text("c"), false);
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

#[test]
fn new_element_follows_last_sibling_with_the_same_name() {
    let source = "<r>\n  <a>1</a>\n  <b/>\n</r>";
    assert_eq!(
        set(source, "r.a[+]", &text("2")),
        "<r>\n  <a>1</a>\n  <a>2</a>\n  <b/>\n</r>"
    );
    assert_eq!(
        create(source, "r.c", &text("3")),
        "<r>\n  <a>1</a>\n  <b/>\n  <c>3</c>\n</r>"
    );
}

#[test]
fn create_builds_missing_elements_as_a_block() {
    let source = "<project>\n    <name>x</name>\n</project>\n";
    let (out, change) = set_with(source, "project.build.plugins.plugin", &text("p"), true);
    assert_eq!(
        out,
        "<project>\n    <name>x</name>\n    <build>\n        <plugins>\n            <plugin>p</plugin>\n        </plugins>\n    </build>\n</project>\n"
    );
    assert_eq!(
        change.keys_created,
        [
            "project.build",
            "project.build.plugins",
            "project.build.plugins.plugin"
        ]
    );
    let missing = set_error(source, "project.build", &text("x"), false);
    assert_eq!(missing.code(), ErrorCode::KeyNotFound);
}

#[test]
fn new_element_in_single_line_document_has_no_line_breaks() {
    assert_eq!(
        create("<r><a>1</a></r>", "r.b", &text("2")),
        "<r><a>1</a><b>2</b></r>"
    );
    assert_eq!(
        create("<r>text</r>", "r.b", &text("2")),
        "<r>text<b>2</b></r>"
    );
}

#[test]
fn new_element_uses_the_parent_namespace() {
    let source = "<p:a xmlns:p=\"urn:p\">\n  <p:b>1</p:b>\n</p:a>";
    let out = set(source, "p:a.b[+]", &text("2"));
    assert_eq!(
        out,
        "<p:a xmlns:p=\"urn:p\">\n  <p:b>1</p:b>\n  <p:b>2</p:b>\n</p:a>"
    );
    let out = create(source, "p:a.c", &text("3"));
    assert_eq!(
        out,
        "<p:a xmlns:p=\"urn:p\">\n  <p:b>1</p:b>\n  <p:c>3</p:c>\n</p:a>"
    );
    let default = "<a xmlns=\"urn:d\" xmlns:o=\"urn:o\"><b/></a>";
    assert_eq!(
        create(default, "a.c", &text("1")),
        "<a xmlns=\"urn:d\" xmlns:o=\"urn:o\"><b/><c>1</c></a>"
    );
    assert_eq!(
        create(default, "a[\"{urn:o}x\"]", &text("1")),
        "<a xmlns=\"urn:d\" xmlns:o=\"urn:o\"><b/><o:x>1</o:x></a>"
    );
    assert_eq!(
        create(default, "a[\"{urn:d}y\"]", &text("1")),
        "<a xmlns=\"urn:d\" xmlns:o=\"urn:o\"><b/><y>1</y></a>"
    );
    let error = set_error(default, "a[\"{urn:none}z\"]", &text("1"), true);
    assert_eq!(
        error.data().found.as_deref(),
        Some(CONSTRUCT_NAMESPACE_DECLARATION)
    );
    let error = set_error(default, "a.u:z", &text("1"), true);
    assert_eq!(
        error.data().found.as_deref(),
        Some(CONSTRUCT_NAMESPACE_DECLARATION)
    );
}

#[test]
fn element_is_removed_with_its_indented_line_and_comments() {
    let source = "<r>\n  <!-- bound -->\n  <a/>\n  <b>x</b> <!-- trailing -->\n  <c/>\n</r>\n";
    assert_eq!(
        remove(source, "r.a"),
        "<r>\n  <b>x</b> <!-- trailing -->\n  <c/>\n</r>\n"
    );
    assert_eq!(
        remove(source, "r.b"),
        "<r>\n  <!-- bound -->\n  <a/>\n  <c/>\n</r>\n"
    );
    assert_eq!(remove("<r><a/><b/></r>", "r.a"), "<r><b/></r>");
    assert_eq!(remove("<r>\n  <a/><b/>\n</r>", "r.b"), "<r>\n  <a/>\n</r>");
    assert_eq!(remove("<r><a>x</a></r>", "r.a.#text"), "<r><a></a></r>");
    let error = XmlTree::parse(source)
        .unwrap()
        .remove(&key("r"))
        .unwrap_err();
    assert_eq!(error.data().found.as_deref(), Some(CONSTRUCT_DOCUMENT_ROOT));
}

#[test]
fn removing_all_repeated_elements_needs_a_wildcard() {
    let source = "<r>\n  <a>1</a>\n  <a>2</a>\n  <b/>\n</r>";
    let tree = XmlTree::parse(source).unwrap();
    assert_eq!(
        tree.remove(&key("r.a")).unwrap_err().code(),
        ErrorCode::KeyConflict
    );
    let out = tree.remove(&key("r.a[*]")).unwrap().apply(source).unwrap();
    assert_eq!(out, "<r>\n  <b/>\n</r>");
}

#[test]
fn map_value_replaces_scalar_content_with_children() {
    let source = "<r>\n  <a>1</a>\n</r>";
    let value = map(&[
        ("b", text("2")),
        ("@x", text("y")),
        ("c", Value::List(vec![text("3"), text("4")])),
    ]);
    let out = set(source, "r.a", &value);
    assert_eq!(
        out,
        "<r>\n  <a x=\"y\">\n    <b>2</b>\n    <c>3</c>\n    <c>4</c>\n  </a>\n</r>"
    );
    assert_eq!(
        read(&out, "r.a").to_plain_json(),
        serde_json::json!({"+@x": "y", "b": "2", "c": ["3", "4"]})
    );
    let back = set(&out, "r.a", &text("1"));
    assert_eq!(back, "<r>\n  <a x=\"y\">1</a>\n</r>");
}

#[test]
fn map_value_replaces_attributes_but_keeps_namespace_declarations() {
    let source = "<r xmlns:p='urn:p'><a p:k='1' z='2'>t</a></r>";
    let value = map(&[("+content", text("u")), ("+@p:k", text("3"))]);
    assert_eq!(
        set(source, "r.a", &value),
        "<r xmlns:p='urn:p'><a p:k='3'>u</a></r>"
    );
    let nested = map(&[
        ("+@xmlns:q", text("urn:q")),
        ("q:item", map(&[("@q:id", text("1"))])),
    ]);
    let out = create("<r/>", "r.n", &nested);
    assert_eq!(out, "<r><n xmlns:q=\"urn:q\"><q:item q:id=\"1\"/></n></r>");
}

#[test]
fn list_value_replaces_an_element_by_siblings() {
    let source = "<r>\n  <a>1</a>\n  <b/>\n</r>";
    let out = set(source, "r.a", &Value::List(vec![text("x"), Value::Null]));
    assert_eq!(out, "<r>\n  <a>x</a>\n  <a/>\n  <b/>\n</r>");
    assert_eq!(
        set(source, "r.a", &Value::List(Vec::new())),
        "<r>\n  <b/>\n</r>"
    );
}

#[test]
fn null_clears_content_and_keeps_attributes() {
    assert_eq!(
        set("<r><a k='1'>t<b/></a></r>", "r.a", &Value::Null),
        "<r><a k='1'></a></r>"
    );
}

#[test]
fn scalar_on_element_with_children_replaces_the_content() {
    assert_eq!(
        set("<r><a><b/>x</a></r>", "r.a", &Value::Int(7)),
        "<r><a>7</a></r>"
    );
}

#[test]
fn changes_without_layout_fail_with_construct_names() {
    let cases = [
        (
            "$",
            map(&[("r", Value::Null)]),
            CONSTRUCT_DOCUMENT_ROOT,
            false,
        ),
        (
            "r",
            Value::List(vec![text("1")]),
            CONSTRUCT_DOCUMENT_ROOT,
            false,
        ),
        ("x", text("1"), CONSTRUCT_DOCUMENT_ROOT, true),
        (
            "r.a",
            map(&[("+content", text("t")), ("b", text("1"))]),
            CONSTRUCT_MIXED_CONTENT,
            false,
        ),
        (
            "r.a",
            map(&[("+content", Value::List(vec![text("1"), text("2")]))]),
            CONSTRUCT_MIXED_CONTENT,
            false,
        ),
        (
            "r.a",
            Value::List(vec![Value::List(vec![text("1")])]),
            CONSTRUCT_NESTED_LIST,
            false,
        ),
    ];
    for (path, value, construct, create) in cases {
        let error = set_error("<r><a/></r>", path, &value, create);
        assert_eq!(error.code(), ErrorCode::UnsupportedConstruct, "{path}");
        assert_eq!(error.data().found.as_deref(), Some(construct), "{path}");
    }
}

#[test]
fn invalid_names_and_characters_are_rejected() {
    for (path, value) in [
        ("r.@a", text("\u{1}")),
        ("r[\"@1x\"]", text("1")),
        ("r.a", text("\u{FFFF}")),
    ] {
        let error = set_error("<r a='1'><a/></r>", path, &value, true);
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{path}");
    }
    let error = set_error("<r/>", "r[\"1x\"]", &text("1"), true);
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn wildcard_sets_every_match() {
    let source = "<r><a>1</a><a>2</a></r>";
    assert_eq!(set(source, "r.a[*]", &text("0")), "<r><a>0</a><a>0</a></r>");
}

#[test]
fn entity_text_is_replaced_whole_and_reported() {
    use std::sync::Arc;

    struct One;
    impl nurt_doc::formats::xml::EntitySource for One {
        fn resolve(&self, _name: &str) -> nurt_contracts::error::Result<Option<String>> {
            Ok(Some("E".to_owned()))
        }
    }
    let source = "<!DOCTYPE r [<!ENTITY e 'E'>]><r>a &e; b</r>";
    let mut tree = XmlTree::parse(source).unwrap();
    tree.set_entity_source(Arc::new(One));
    let change = tree.set(&key("r"), &text("plain"), false).unwrap();
    assert_eq!(
        change.apply(source).unwrap(),
        "<!DOCTYPE r [<!ENTITY e 'E'>]><r>plain</r>"
    );
    assert_eq!(change.notable, ["entity_replaced:e"]);
}

#[test]
fn overlapping_edits_are_rejected() {
    let mut change = XmlChange::default();
    change.push(XmlEdit::new(1..3, "x"));
    change.push(XmlEdit::new(2..4, "y"));
    assert_eq!(
        change.apply("<a/>xx").unwrap_err().code(),
        ErrorCode::KeyConflict
    );
    let mut change = XmlChange::default();
    change.push(XmlEdit::new(2..2, "x"));
    change.push(XmlEdit::new(2..2, "y"));
    assert_eq!(
        change.apply("<a/>").unwrap_err().code(),
        ErrorCode::KeyConflict
    );
    let mut change = XmlChange::default();
    change.push(XmlEdit::new(2..9, "x"));
    assert_eq!(
        change.apply("<a/>").unwrap_err().code(),
        ErrorCode::InvalidValue
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
