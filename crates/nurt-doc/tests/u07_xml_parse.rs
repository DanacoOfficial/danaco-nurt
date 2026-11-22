// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parsing of XML documents into a lossless tree: byte ranges of every
//! syntactic part, rejection of malformed documents, key resolution and
//! the value model.

// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]
// One fixture is checked range by range in a single test.
#![allow(clippy::cognitive_complexity)]

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;
use nurt_doc::formats::xml::{
    CONSTRUCT_ENTITY_REFERENCE, EntitySource, Node, NodeId, Target, XmlTree,
};
use serde_json::json;

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn node_of(tree: &XmlTree, predicate: impl Fn(&Node) -> bool) -> &Node {
    tree.node_ids()
        .map(|id| tree.node(id))
        .find(|node| predicate(node))
        .unwrap()
}

#[test]
fn records_ranges_of_declaration_doctype_and_tags() {
    let text = "<?xml version=\"1.0\" encoding='UTF-8'?>\n<!DOCTYPE r [ <!ENTITY e \"v\"> ]>\n<r  a = \"1\"\n   b='2' ><x/><!--c--><?pi  data ?><![CDATA[d]]>t</r >";
    let tree = XmlTree::parse(text).unwrap();
    let declaration = tree.declaration().unwrap();
    assert_eq!(
        &text[declaration.span.clone()],
        "<?xml version=\"1.0\" encoding='UTF-8'?>"
    );
    assert_eq!(&text[declaration.attributes[1].value.clone()], "UTF-8");
    assert_eq!(declaration.attributes[1].quote, '\'');

    let doctype = tree.doctype().unwrap();
    assert_eq!(&text[doctype.name.clone()], "r");
    assert_eq!(
        &text[doctype.internal_subset.clone().unwrap()],
        " <!ENTITY e \"v\"> "
    );

    let root = tree.element(tree.root()).unwrap();
    assert_eq!(&text[root.start_tag.clone()], "<r  a = \"1\"\n   b='2' >");
    assert_eq!(&text[root.name.clone()], "r");
    let a = &root.attributes[0];
    assert_eq!(&text[a.leading.clone()], "  ");
    assert_eq!(&text[a.name.clone()], "a");
    assert_eq!(&text[a.equals.clone()], " = ");
    assert_eq!(&text[a.value.clone()], "1");
    assert_eq!(&text[a.span.clone()], "a = \"1\"");
    assert_eq!(&text[root.attributes[1].leading.clone()], "\n   ");
    assert_eq!(&text[root.trailing.clone()], " ");
    assert_eq!(&text[root.close.clone()], ">");
    assert_eq!(&text[root.end_tag.clone().unwrap()], "</r >");
    assert_eq!(root.span.end, text.len());

    let Node::Element(x) = node_of(
        &tree,
        |node| matches!(node, Node::Element(e) if e.is_empty_tag()),
    ) else {
        unreachable!()
    };
    assert_eq!(&text[x.close.clone()], "/>");
    let Node::Comment(comment) = node_of(&tree, |node| matches!(node, Node::Comment(_))) else {
        unreachable!()
    };
    assert_eq!(&text[comment.content.clone()], "c");
    let Node::ProcessingInstruction(pi) =
        node_of(&tree, |node| matches!(node, Node::ProcessingInstruction(_)))
    else {
        unreachable!()
    };
    assert_eq!(&text[pi.target.clone()], "pi");
    assert_eq!(&text[pi.content.clone()], "data ");
    let Node::CData(cdata) = node_of(&tree, |node| matches!(node, Node::CData(_))) else {
        unreachable!()
    };
    assert_eq!(&text[cdata.content.clone()], "d");
}

#[test]
fn node_ranges_tile_the_document() {
    let text = "\u{FEFF}<!-- a -->\n<r>\n  x &amp; <b c='1'/>y<![CDATA[z]]>\n</r>\n";
    let tree = XmlTree::parse(text).unwrap();
    let mut position = "\u{FEFF}".len();
    for id in tree.document_nodes() {
        let span = tree.node(*id).span();
        assert_eq!(span.start, position);
        position = span.end;
    }
    assert_eq!(position, text.len());
    let mut child_position = tree.element(tree.root()).unwrap().content().start;
    for child in tree.children(tree.root()) {
        let span = tree.node(*child).span();
        assert_eq!(span.start, child_position);
        child_position = span.end;
    }
}

#[test]
fn entity_and_text_events_form_one_text_node() {
    let tree = XmlTree::parse("<r>a &lt; b &#x41;</r>").unwrap();
    let children = tree.children(tree.root());
    assert_eq!(children.len(), 1);
    assert_eq!(
        tree.chunk_value(children[0]).unwrap().as_deref(),
        Some("a < b A")
    );
}

#[test]
fn rejects_malformed_documents_with_doc_parse() {
    let cases = [
        "",
        "<!-- only -->",
        "<a>",
        "<a></b>",
        "<a/><b/>",
        "text<a/>",
        "<a/>text",
        "<a x='1' x='2'/>",
        "<a x=1/>",
        "<a x/>",
        "<a x='1'y='2'/>",
        "<a x='<'/>",
        "<1a/>",
        "<a>&undefined;</a>",
        "<a>&amp</a>",
        "<a>& b</a>",
        "<a>&#0;</a>",
        "<a>&#xD800;</a>",
        "<a>&#xZZ;</a>",
        "<a>]]></a>",
        "<a><!-- a -- b --></a>",
        "<a><!-- a ---></a>",
        "<?xml version='1.0'?><?xml version='1.0'?><a/>",
        "<a/><?xml version='1.0'?>",
        "<?xml encoding='UTF-8'?><a/>",
        "<a>\u{1}</a>",
        "<![CDATA[x]]><a/>",
        "<a/><!DOCTYPE a>",
        "<!DOCTYPE a><!DOCTYPE a><a/>",
        "<a><?xml x?></a>",
        "<a><?1 x?></a>",
        "<a></a></a>",
        "<a><b></a></b>",
        "<a x='1' / >",
        "<a x=\"\u{FFFE}\"/>",
    ];
    for case in cases {
        let error = XmlTree::parse(case).expect_err(case);
        assert_eq!(error.code(), ErrorCode::DocParse, "{case}: {error}");
    }
}

#[test]
fn parse_errors_carry_line_and_column() {
    let error = XmlTree::parse("<a>\n  <b x='1' x='2'/>\n</a>").unwrap_err();
    let location = error.data().location.clone().unwrap();
    assert_eq!((location.line, location.column), (Some(2), Some(12)));
}

#[test]
fn line_and_column_count_unicode_scalars() {
    let tree = XmlTree::parse("<a>\r\n  <ł/><b/>\r\n</a>").unwrap();
    let b = tree.child_elements(tree.root()).nth(1).unwrap();
    assert_eq!(tree.line_column(tree.node(b).span().start), (2, 7));
}

#[test]
fn resolves_elements_attributes_and_text_chunks() {
    let tree = XmlTree::parse(
        "<r><a id='1'>x<b/>y<![CDATA[z]]></a><deps><dep><v>1</v></dep><dep><v>2</v></dep><dep><v>3</v></dep></deps></r>",
    )
    .unwrap();
    let read = |text: &str| tree.read(&key(text)).unwrap();
    assert_eq!(read("r.a.@id")[0].value, Value::Text("1".into()));
    assert_eq!(read("r.a.#text[1]")[0].value, Value::Text("yz".into()));
    assert_eq!(read("r.a.#text[-1]")[0].value, Value::Text("yz".into()));
    assert_eq!(read("r.a.#text[1]")[0].key, "r.a.#text[1]");
    assert_eq!(read("r.deps.dep[2].v")[0].value, Value::Text("3".into()));
    assert_eq!(read("r.deps.dep[2].v")[0].key, "r.deps.dep[2].v");
    assert_eq!(read("r.deps.dep[*].v").len(), 3);
    assert_eq!(read("r.deps[0].*").len(), 3);
    assert_eq!(read("r.deps.dep[*]").len(), 3);
    assert_eq!(read("r.**.v").len(), 3);
    assert_eq!(read("r.deps.dep[?v == \"2\"].v")[0].key, "r.deps.dep[1].v");
    assert_eq!(read("r.deps.dep[?v > 1]").len(), 2);
    assert!(matches!(read("$")[0].target, Target::Document));
}

#[test]
fn repeated_names_without_index_report_candidates() {
    let tree = XmlTree::parse("<r><a>x<b/>y</a><d/><d/></r>").unwrap();
    let error = tree.resolve(&key("r.d")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyConflict);
    assert_eq!(error.data().sample, ["r.d[0]", "r.d[1]"]);
    assert_eq!(
        error.data().details["candidates"],
        json!(["r.d[0]", "r.d[1]"])
    );
    let error = tree.resolve(&key("r.a.#text")).unwrap_err();
    assert_eq!(error.data().sample, ["r.a.#text[0]", "r.a.#text[1]"]);
}

#[test]
fn missing_keys_report_key_not_found() {
    let tree = XmlTree::parse("<r><a/></r>").unwrap();
    for text in [
        "x",
        "r.b",
        "r.a.@id",
        "r.a.#text",
        "r.a[1]",
        "#1:r",
        "r.a.@id.x",
    ] {
        let error = tree.resolve(&key(text)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::KeyNotFound, "{text}");
    }
}

#[test]
fn namespace_keys_use_document_prefixes_and_clark_names() {
    let text = r#"<pom:project xmlns:pom="urn:pom" xmlns:o="urn:o" xmlns="urn:d"><pom:version>1</pom:version><o:x o:k="v">2</o:x><plain>3</plain></pom:project>"#;
    let tree = XmlTree::parse(text).unwrap();
    let read = |text: &str| tree.read(&key(text)).unwrap()[0].value.clone();
    assert_eq!(read("pom:project.pom:version"), Value::Text("1".into()));
    assert_eq!(read("pom:project.version"), Value::Text("1".into()));
    assert_eq!(read("[\"{urn:pom}project\"][\"{urn:o}x\"]"), {
        let mut map = nurt_contracts::value::Fields::new();
        map.insert("+content".into(), Value::Text("2".into()));
        map.insert("+@o:k".into(), Value::Text("v".into()));
        Value::Map(map)
    });
    assert_eq!(read("pom:project.o:x.@{urn:o}k"), Value::Text("v".into()));
    assert_eq!(
        read("pom:project[\"{urn:d}plain\"]"),
        Value::Text("3".into())
    );
    let root = tree.root();
    assert_eq!(tree.namespace(root).as_deref(), Some("urn:pom"));
    assert_eq!(
        tree.lookup_namespace(root, Some("xml")).as_deref(),
        Some(nurt_doc::formats::xml::XML_NAMESPACE)
    );
}

#[test]
fn prefix_matches_by_namespace_when_written_differently() {
    let tree =
        XmlTree::parse(r#"<a:r xmlns:a="urn:x" xmlns:b="urn:x"><b:item>1</b:item></a:r>"#).unwrap();
    assert_eq!(tree.read(&key("a:r.a:item")).unwrap()[0].key, "a:r.b:item");
}

#[test]
fn comment_directly_before_element_is_reported() {
    let text =
        "<r>\n  <!-- first -->\n  <!-- second -->\n  <a/>\n\n  <!-- detached -->\n\n  <b/>\n</r>";
    let tree = XmlTree::parse(text).unwrap();
    assert_eq!(
        tree.read(&key("r.a")).unwrap()[0].comment.as_deref(),
        Some("first\nsecond")
    );
    assert_eq!(tree.read(&key("r.b")).unwrap()[0].comment, None);
}

#[test]
fn value_model_follows_yq_xml_convention() {
    let cases = [
        ("<a/>", json!({"a": null})),
        (
            "<r><a></a><b>  </b></r>",
            json!({"r": {"a": null, "b": null}}),
        ),
        (
            "<r><a x=\"1\">t</a></r>",
            json!({"r": {"a": {"+content": "t", "+@x": "1"}}}),
        ),
        (
            "<r>t1<b/>t2</r>",
            json!({"r": {"+content": ["t1", "t2"], "b": null}}),
        ),
        ("<r><b>1</b><b>2</b></r>", json!({"r": {"b": ["1", "2"]}})),
        (
            "<?xml version=\"1.0\"?><!-- c --><r xmlns:p=\"u\"><p:a>  x  </p:a><![CDATA[cd]]><?pi data?></r>",
            json!({"+p_xml": "version=\"1.0\"", "r": {"+content": "cd", "+@xmlns:p": "u", "p:a": "x", "+p_pi": "data"}}),
        ),
        (
            "<!DOCTYPE r><r>  <a> x </a>\n</r>",
            json!({"+directive": "DOCTYPE r", "r": {"a": "x"}}),
        ),
        ("<r>a&amp;b</r>", json!({"r": "a&b"})),
        (
            "<r><a>1</a><!-- k --><a>2</a></r>",
            json!({"r": {"a": ["1", "2"]}}),
        ),
        ("<r>a<![CDATA[b]]>c</r>", json!({"r": "abc"})),
        (
            "<r><?p one?><?p two?></r>",
            json!({"r": {"+p_p": ["one", "two"]}}),
        ),
        ("<r> line1\n  line2 </r>", json!({"r": "line1\n  line2"})),
        ("<r x=\"a&lt;&#65;\"/>", json!({"r": {"+@x": "a<A"}})),
        (
            "<r><a><b>1</b></a><a><b>2</b><b>3</b></a></r>",
            json!({"r": {"a": [{"b": "1"}, {"b": ["2", "3"]}]}}),
        ),
        (
            "<r><a x=\"1\"/><a>t</a></r>",
            json!({"r": {"a": [{"+@x": "1"}, "t"]}}),
        ),
        (
            "<r><a>1</a><b/><a>2</a></r>",
            json!({"r": {"a": ["1", "2"], "b": null}}),
        ),
        (
            "<r x=\"a\nb\tc\" y=\"&#10;z\"/>",
            json!({"r": {"+@x": "a b c", "+@y": "\nz"}}),
        ),
        ("<r>a\r\nb</r>", json!({"r": "a\nb"})),
    ];
    for (text, expected) in cases {
        let tree = XmlTree::parse(text).unwrap();
        assert_eq!(tree.to_value().unwrap().to_plain_json(), expected, "{text}");
    }
}

struct Entities;

impl EntitySource for Entities {
    fn resolve(&self, name: &str) -> Result<Option<String>> {
        Ok((name == "e").then(|| "expanded".to_owned()))
    }
}

#[test]
fn dtd_entities_need_an_entity_source() {
    let text = "<!DOCTYPE r [ <!ENTITY e \"expanded\"> ]><r><a>x &e; y</a><b c=\"&e;\"/></r>";
    let mut tree = XmlTree::parse(text).unwrap();
    let error = tree.read(&key("r.a")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedConstruct);
    assert_eq!(
        error.data().found.as_deref(),
        Some(CONSTRUCT_ENTITY_REFERENCE)
    );

    tree.set_entity_source(Arc::new(Entities));
    tree.check_references().unwrap();
    assert_eq!(
        tree.read(&key("r.a")).unwrap()[0].value,
        Value::Text("x expanded y".into())
    );
    assert_eq!(
        tree.read(&key("r.b.@c")).unwrap()[0].value,
        Value::Text("expanded".into())
    );

    let undeclared = XmlTree::parse("<!DOCTYPE r><r>&u;</r>").unwrap();
    let mut undeclared = undeclared;
    undeclared.set_entity_source(Arc::new(Entities));
    assert_eq!(
        undeclared.check_references().unwrap_err().code(),
        ErrorCode::DocParse
    );
}

#[test]
fn node_identifiers_are_in_document_order() {
    let tree = XmlTree::parse("<r><a/><b/></r>").unwrap();
    let ids: Vec<NodeId> = tree.child_elements(tree.root()).collect();
    assert!(ids[0].index() < ids[1].index());
    assert_eq!(tree.parent(ids[0]), Some(tree.root()));
    assert_eq!(tree.parent(tree.root()), None);
}
