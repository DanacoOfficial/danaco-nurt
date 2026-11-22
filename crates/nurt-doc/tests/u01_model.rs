// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Node tree, key path resolution and key errors.
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;
use nurt_doc::model::{
    DocTree, FlatSemantics, FormatSemantics, LineIndex, MapSemantics, NodeId, NodeKind, NodeSpec,
    ScalarStyle, Span, TreeBuilder, ambiguous_key, doc_parse, key_not_found,
};
use serde_json::json;

/// Node of a test document.
enum N {
    Scalar(Value),
    Map(Vec<(&'static str, N)>),
    List(Vec<N>),
    Comment,
}

fn int(value: i64) -> N {
    N::Scalar(Value::Int(value))
}

fn text(value: &str) -> N {
    N::Scalar(Value::Text(value.to_owned()))
}

/// Tree of `documents` over a blank text; every node gets its own span.
fn tree_of(documents: Vec<N>) -> DocTree {
    let blank = vec![b' '; 4096];
    let mut builder = TreeBuilder::new(&blank);
    let mut next = 0;
    for document in documents {
        add(&mut builder, None, None, document, &mut next);
    }
    builder.finish()
}

fn add(
    builder: &mut TreeBuilder<'_>,
    parent: Option<NodeId>,
    name: Option<&str>,
    node: N,
    next: &mut usize,
) -> NodeId {
    let span = Span::new(*next, *next + 1);
    *next += 2;
    let (kind, value) = match &node {
        N::Scalar(value) => (NodeKind::Scalar, Some(value.clone())),
        N::Map(_) => (NodeKind::Map, None),
        N::List(_) => (NodeKind::List, None),
        N::Comment => (NodeKind::Comment, None),
    };
    let mut spec = NodeSpec::new(kind, span);
    if let Some(name) = name {
        spec = spec.with_name(name);
    }
    if let Some(value) = value {
        spec = spec.with_value(value);
    }
    let id = builder.node(parent, spec);
    match node {
        N::Map(members) => {
            for (member, child) in members {
                add(builder, Some(id), Some(member), child, next);
            }
        }
        N::List(items) => {
            for item in items {
                add(builder, Some(id), None, item, next);
            }
        }
        N::Scalar(_) | N::Comment => {}
    }
    id
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).expect("valid key path")
}

fn keys(tree: &DocTree, path: &str, semantics: &dyn FormatSemantics) -> Vec<String> {
    tree.resolve(&key(path), semantics)
        .expect("key resolves")
        .into_iter()
        .map(|resolved| resolved.key.to_string())
        .collect()
}

fn value_at(tree: &DocTree, path: &str, semantics: &dyn FormatSemantics) -> Value {
    let found = tree.resolve(&key(path), semantics).expect("key resolves");
    assert_eq!(found.len(), 1, "{path} selects one node");
    tree.value_of(found[0].node, semantics, None)
}

fn sample() -> DocTree {
    tree_of(vec![N::Map(vec![
        (
            "a",
            N::Map(vec![("b", N::List(vec![int(1), int(2), int(3)]))]),
        ),
        (
            "deps",
            N::List(vec![
                N::Map(vec![("name", text("serde")), ("version", text("1.0"))]),
                N::Map(vec![("name", text("toml")), ("version", text("0.9"))]),
            ]),
        ),
        ("note", N::Comment),
        ("port", int(8080)),
    ])])
}

#[test]
fn resolves_names_and_indexes_to_canonical_keys() {
    let tree = sample();
    let semantics = MapSemantics::LAST_WINS;
    assert_eq!(keys(&tree, "a.b[1]", &semantics), ["a.b[1]"]);
    assert_eq!(keys(&tree, "a.b[-1]", &semantics), ["a.b[2]"]);
    assert_eq!(keys(&tree, "a.b.0", &semantics), ["a.b[0]"]);
    assert_eq!(value_at(&tree, "a.b[1]", &semantics), Value::Int(2));
    assert_eq!(value_at(&tree, "port", &semantics), Value::Int(8080));
}

#[test]
fn missing_key_and_out_of_range_index_select_nothing() {
    let tree = sample();
    let semantics = MapSemantics::LAST_WINS;
    assert!(keys(&tree, "a.missing", &semantics).is_empty());
    assert!(keys(&tree, "a.b[3]", &semantics).is_empty());
    assert!(keys(&tree, "a.b[-4]", &semantics).is_empty());
    assert!(keys(&tree, "a.b[+]", &semantics).is_empty());
}

#[test]
fn wildcard_selects_members_without_comments() {
    let tree = sample();
    let semantics = MapSemantics::LAST_WINS;
    assert_eq!(
        keys(&tree, "a.b[*]", &semantics),
        ["a.b[0]", "a.b[1]", "a.b[2]"]
    );
    assert_eq!(keys(&tree, "[*]", &semantics), ["a", "deps", "port"]);
}

#[test]
fn recursive_segment_selects_node_and_descendants_once() {
    let tree = sample();
    let semantics = MapSemantics::LAST_WINS;
    let all = keys(&tree, "**", &semantics);
    assert_eq!(all.len(), 14, "{all:?}");
    assert_eq!(
        keys(&tree, "**.version", &semantics),
        ["deps[0].version", "deps[1].version"]
    );
    assert_eq!(keys(&tree, "a.**.b", &semantics), ["a.b"]);
}

#[test]
fn filter_selects_list_elements_by_field() {
    let tree = sample();
    let semantics = MapSemantics::LAST_WINS;
    assert_eq!(
        keys(&tree, "deps[?name == \"serde\"].version", &semantics),
        ["deps[0].version"]
    );
    assert_eq!(
        keys(&tree, "deps[?version > \"0.95\"]", &semantics),
        ["deps[0]"]
    );
    assert_eq!(
        keys(&tree, "deps[?not (name == \"serde\")]", &semantics),
        ["deps[1]"]
    );
    assert_eq!(keys(&tree, "deps[?name]", &semantics).len(), 2);
    assert!(keys(&tree, "deps[?missing == 1]", &semantics).is_empty());
    assert_eq!(
        keys(&tree, "a.b[?@ >= 2]", &semantics),
        ["a.b[1]", "a.b[2]"]
    );
}

#[test]
fn name_through_scalar_is_key_conflict() {
    let tree = sample();
    let error = tree
        .resolve(&key("port.x"), &MapSemantics::LAST_WINS)
        .expect_err("port is a scalar");
    assert_eq!(error.code, ErrorCode::KeyConflict);
    assert_eq!(error.found.as_deref(), Some("scalar"));
    assert!(error.message.contains("port.x"), "{}", error.message);
}

#[test]
fn index_on_map_is_key_conflict() {
    let tree = sample();
    let error = tree
        .resolve(&key("a[0]"), &MapSemantics::LAST_WINS)
        .expect_err("a is a map");
    assert_eq!(error.code, ErrorCode::KeyConflict);
    assert_eq!(error.found.as_deref(), Some("map"));
}

#[test]
fn pattern_through_scalar_selects_nothing() {
    let tree = sample();
    assert!(keys(&tree, "[*].x", &MapSemantics::LAST_WINS).is_empty());
}

fn repeated() -> DocTree {
    tree_of(vec![N::Map(vec![
        ("dep", N::Map(vec![("version", text("1"))])),
        ("dep", N::Map(vec![("version", text("2"))])),
        ("dep", N::Map(vec![("version", text("3"))])),
        ("name", text("app")),
    ])])
}

#[test]
fn repeated_name_without_index_lists_candidates() {
    let tree = repeated();
    let error = tree
        .resolve(&key("dep.version"), &MapSemantics::CONFLICT)
        .expect_err("dep is repeated");
    assert_eq!(error.code, ErrorCode::KeyConflict);
    let candidates: Vec<&str> = error
        .candidates
        .iter()
        .map(|candidate| candidate.program.as_str())
        .collect();
    assert_eq!(candidates, ["dep[0]", "dep[1]", "dep[2]"]);
    assert_eq!(error.candidates[0].rank, 1);
}

#[test]
fn repeated_name_is_addressed_by_index_and_wildcard() {
    let tree = repeated();
    let semantics = MapSemantics::CONFLICT;
    assert_eq!(
        keys(&tree, "dep[2].version", &semantics),
        ["dep[2].version"]
    );
    assert_eq!(keys(&tree, "dep[-1]", &semantics), ["dep[2]"]);
    assert_eq!(
        keys(&tree, "dep[*]", &semantics),
        ["dep[0]", "dep[1]", "dep[2]"]
    );
    assert_eq!(keys(&tree, "name[0]", &semantics), ["name"]);
    assert_eq!(keys(&tree, "dep[*].version", &semantics).len(), 3);
}

#[test]
fn repeated_names_become_a_list_in_values() {
    let tree = repeated();
    let root = tree.roots()[0];
    let value = tree.value_of(root, &MapSemantics::CONFLICT, None);
    let expected =
        json!({"dep": [{"version": "1"}, {"version": "2"}, {"version": "3"}], "name": "app"});
    assert_eq!(value, Value::from_plain_json(expected));
}

#[test]
fn duplicate_name_keeps_last_member_under_last_wins() {
    let tree = tree_of(vec![N::Map(vec![("a", int(1)), ("a", int(2))])]);
    let semantics = MapSemantics::LAST_WINS;
    assert_eq!(value_at(&tree, "a", &semantics), Value::Int(2));
    let root = tree.roots()[0];
    assert_eq!(
        tree.value_of(root, &semantics, None),
        Value::from_plain_json(json!({"a": 2}))
    );
}

#[test]
fn flat_semantics_joins_names_with_dots() {
    let tree = tree_of(vec![N::Map(vec![
        ("server.port", int(80)),
        ("server", text("web")),
    ])]);
    let semantics = FlatSemantics;
    assert_eq!(keys(&tree, "server.port", &semantics), ["server.port"]);
    assert_eq!(
        keys(&tree, "[\"server.port\"]", &semantics),
        ["server.port"]
    );
    assert_eq!(
        value_at(&tree, "server", &semantics),
        Value::Text("web".to_owned())
    );
}

#[test]
fn alias_resolves_through_its_target() {
    let blank = vec![b' '; 64];
    let mut builder = TreeBuilder::new(&blank);
    let root = builder.node(None, NodeSpec::new(NodeKind::Map, Span::new(0, 40)));
    let base = builder.node(
        Some(root),
        NodeSpec::new(NodeKind::Map, Span::new(1, 10))
            .with_name("base")
            .with_anchor("b"),
    );
    builder.node(
        Some(base),
        NodeSpec::new(NodeKind::Scalar, Span::new(2, 3))
            .with_name("x")
            .with_value(Value::Int(7)),
    );
    let alias = builder.node(
        Some(root),
        NodeSpec::new(NodeKind::Scalar, Span::new(11, 13)).with_name("copy"),
    );
    builder.set_alias(alias, base);
    let tree = builder.finish();
    let semantics = MapSemantics::LAST_WINS;
    assert_eq!(tree.target(alias), base);
    assert_eq!(tree.target(base), base);
    assert_eq!(keys(&tree, "copy.x", &semantics), ["base.x"]);
    assert_eq!(
        tree.value_of(alias, &semantics, None),
        Value::from_plain_json(json!({"x": 7}))
    );
}

#[test]
fn alias_cycle_does_not_loop() {
    let blank = vec![b' '; 16];
    let mut builder = TreeBuilder::new(&blank);
    let root = builder.node(None, NodeSpec::new(NodeKind::Map, Span::new(0, 10)));
    let first = builder.node(
        Some(root),
        NodeSpec::new(NodeKind::Scalar, Span::new(1, 2)).with_name("a"),
    );
    let second = builder.node(
        Some(root),
        NodeSpec::new(NodeKind::Scalar, Span::new(3, 4)).with_name("b"),
    );
    builder.set_alias(first, second);
    builder.set_alias(second, first);
    let tree = builder.finish();
    let target = tree.target(first);
    assert!(target == first || target == second);
    assert_eq!(
        tree.value_of(first, &MapSemantics::LAST_WINS, None),
        Value::Null
    );
}

#[test]
fn document_selector_picks_documents_of_a_stream() {
    let tree = tree_of(vec![
        N::Map(vec![("kind", text("Service"))]),
        N::Map(vec![("kind", text("Deployment"))]),
    ]);
    let semantics = MapSemantics::LAST_WINS;
    assert_eq!(
        value_at(&tree, "kind", &semantics),
        Value::Text("Service".to_owned())
    );
    assert_eq!(
        value_at(&tree, "#1:kind", &semantics),
        Value::Text("Deployment".to_owned())
    );
    assert_eq!(keys(&tree, "#*:kind", &semantics), ["kind", "#1:kind"]);
    assert!(keys(&tree, "#5:kind", &semantics).is_empty());
    let second = tree.resolve(&key("#1:kind"), &semantics).expect("resolves")[0].node;
    assert_eq!(tree.document_of(second), Some(1));
}

#[test]
fn depth_limits_expanded_levels() {
    let tree = sample();
    let root = tree.roots()[0];
    let semantics = MapSemantics::LAST_WINS;
    let shallow = tree.value_of(root, &semantics, Some(1));
    let expected = json!({"a": {}, "deps": [], "port": 8080});
    assert_eq!(shallow, Value::from_plain_json(expected));
    assert_eq!(
        tree.value_of(root, &semantics, Some(0)),
        Value::from_plain_json(json!({}))
    );
}

#[test]
fn similar_keys_rank_members_by_edit_distance() {
    let tree = sample();
    let similar = tree.similar_keys(&key("a.bb"), &MapSemantics::LAST_WINS);
    assert_eq!(
        similar.first().map(ToString::to_string).as_deref(),
        Some("a.b")
    );
    let top = tree.similar_keys(&key("prot"), &MapSemantics::LAST_WINS);
    assert_eq!(
        top.first().map(ToString::to_string).as_deref(),
        Some("port")
    );
}

#[test]
fn tree_reports_size_roots_and_nodes() {
    let tree = sample();
    assert!(!tree.is_empty());
    assert_eq!(tree.len(), tree.iter().count());
    assert_eq!(tree.roots().len(), 1);
    let comments = tree
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Comment)
        .count();
    assert_eq!(comments, 1);
    assert!(DocTree::default().is_empty());
}

#[test]
fn builder_computes_lines_and_character_columns() {
    let text = "a: 1\néü: 22\n".as_bytes();
    let mut builder = TreeBuilder::new(text);
    let root = builder.node(None, NodeSpec::new(NodeKind::Map, Span::new(0, text.len())));
    let member = builder.node(
        Some(root),
        NodeSpec::new(NodeKind::Scalar, Span::new(5, 13))
            .with_name("éü")
            .with_key_span(Span::new(5, 9))
            .with_value_span(Span::new(11, 13))
            .with_value(Value::Int(22))
            .with_comment("count")
            .with_tag("!!int"),
    );
    let tree = builder.finish();
    let node = &tree[member];
    assert_eq!((node.start.line, node.start.column), (2, 5));
    assert_eq!((node.end.line, node.end.column), (2, 7));
    assert_eq!(node.comment.as_deref(), Some("count"));
    assert_eq!(node.tag.as_deref(), Some("!!int"));
    let region = tree.text_span(text, node.value_span);
    assert_eq!((region.bytes.start, region.bytes.end), (11, 13));
    assert_eq!(tree.lines().line_count(), 3);
}

#[test]
fn line_index_maps_offsets_past_the_end_to_last_line() {
    let text = b"x\r\ny";
    let lines = LineIndex::new(text);
    assert_eq!(lines.line_of(0), 0);
    assert_eq!(lines.line_of(3), 1);
    assert_eq!(lines.line_of(100), 1);
    assert_eq!(lines.line_start(1), 3);
    assert_eq!(lines.line_start(9), text.len());
    let position = lines.position(text, 2);
    assert_eq!((position.line, position.column), (1, 3));
    let invalid = b"\xFF\xFEz";
    let position = LineIndex::new(invalid).position(invalid, 3);
    assert_eq!(position.column, 4);
}

#[test]
fn spans_compare_join_and_convert() {
    let span = Span::new(5, 2);
    assert_eq!((span.start, span.end), (2, 5));
    assert!(Span::at(4).is_empty());
    assert!(span.covers(Span::at(5)));
    assert!(!span.covers(Span::new(1, 3)));
    assert!(!span.contains(5));
    assert_eq!(span.join(Span::new(7, 9)), Span::new(2, 9));
    let range = span.to_byte_range();
    assert_eq!((range.start, range.end), (2, 5));
}

#[test]
fn kinds_and_styles_describe_themselves() {
    assert_eq!(NodeKind::Element.as_str(), "element");
    assert!(NodeKind::Element.is_named_container());
    assert!(!NodeKind::List.is_named_container());
    assert!(NodeKind::List.is_container());
    assert!(!NodeKind::Attribute.is_container());
    assert_eq!(ScalarStyle::SingleQuoted.quote(), Some('\''));
    assert_eq!(ScalarStyle::Literal.quote(), None);
}

#[test]
fn doc_parse_error_names_format_and_position() {
    let json = DocFormatId::new("json").expect("valid identifier");
    let without = doc_parse(&json, b"{", None, "unexpected end");
    assert_eq!(without.code, ErrorCode::DocParse);
    assert_eq!(without.message, "invalid json document: unexpected end");
    let with = doc_parse(&json, b"{\n  1}", Some(4), "expected a member name");
    assert_eq!(with.details.get("line"), Some(&json!(2)));
    assert_eq!(with.details.get("column"), Some(&json!(3)));
    assert_eq!(with.details.get("byte"), Some(&json!(4)));
}

#[test]
fn key_errors_cap_candidates() {
    let wanted = key("dep");
    let candidates: Vec<KeyPath> = (0..20).map(|index| key(&format!("dep[{index}]"))).collect();
    let ambiguous = ambiguous_key(&wanted, &candidates);
    assert_eq!(ambiguous.candidates.len(), 8);
    assert!(
        ambiguous.message.contains("20 members"),
        "{}",
        ambiguous.message
    );
    let missing = key_not_found(&wanted, &candidates[..2]);
    assert_eq!(missing.code, ErrorCode::KeyNotFound);
    assert_eq!(missing.candidates.len(), 2);
    assert!(
        missing.message.ends_with("set `create: true`"),
        "{}",
        missing.message
    );
}
