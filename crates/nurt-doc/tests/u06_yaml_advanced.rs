// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sequences, anchors and aliases, merge keys, documents, comments, tags and
//! explicit keys of YAML documents.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "helpers outside test functions fail the test on unexpected input"
)]

mod u06_support;

use indexmap::IndexMap;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::{DocSelector, KeyPath};
use nurt_contracts::report::Notable;
use nurt_contracts::value::{Bytes, Value};
use nurt_doc::constructs::{
    YAML_ALIAS_EXPANSION_LIMIT, YAML_COMPLEX_KEY, YAML_MERGE_INHERITED_REMOVE,
};
use nurt_doc::edit::{ByteEdit, EditReport};
use nurt_doc::formats::yaml::anchor::{self, Anchors};
use nurt_doc::formats::yaml::comment::Comments;
use nurt_doc::formats::yaml::merge;
use nurt_doc::formats::yaml::model::YamlModel;
use nurt_doc::formats::yaml::multidoc;
use nurt_doc::formats::yaml::seq::{self, IndexTarget};
use nurt_doc::formats::yaml::tag::{self, CoreTag};
use nurt_doc::formats::yaml::{YamlDoc, YamlVersion};
use nurt_doc::model::Span;
use nurt_doc::model::{DocHandle, SetOptions};
use u06_support::{apply_edits, node_at, parse, reference};

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).expect("key path")
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn append(source: &str, path: &str, value: &Value) -> String {
    let model = parse(source);
    let plan =
        seq::plan_append(&model, node_at(&model, path), value, &key(path)).expect("append plan");
    let result = apply_edits(&model, &plan.edits);
    reference(&result).unwrap_or_else(|error| panic!("result does not parse: {error}\n{result}"));
    result
}

fn remove(source: &str, path: &str, index: i64) -> String {
    let model = parse(source);
    let plan =
        seq::plan_remove(&model, node_at(&model, path), index, &key(path)).expect("remove plan");
    let result = apply_edits(&model, &plan.edits);
    reference(&result).unwrap_or_else(|error| panic!("result does not parse: {error}\n{result}"));
    result
}

#[test]
fn appends_to_an_indented_block_list_after_the_inline_comment() {
    let result = append("a:\n  - x\n  - y  # last\nb: 1\n", "a", &text("z"));
    assert_eq!(result, "a:\n  - x\n  - y  # last\n  - z\nb: 1\n");
}

#[test]
fn appends_to_a_list_at_the_column_of_its_key() {
    let result = append("a:\n- x\nb: 1\n", "a", &text("z"));
    assert_eq!(result, "a:\n- x\n- z\nb: 1\n");
}

#[test]
fn appends_a_mapping_item_with_keys_aligned_to_its_siblings() {
    let mut fields = IndexMap::new();
    fields.insert("name".to_owned(), text("b"));
    fields.insert("size".to_owned(), Value::Int(2));
    let result = append(
        "l:\n  - name: a\n    size: 1\nk: v\n",
        "l",
        &Value::Map(fields),
    );
    assert_eq!(
        result,
        "l:\n  - name: a\n    size: 1\n  - name: b\n    size: 2\nk: v\n"
    );
}

#[test]
fn appends_before_the_comment_that_closes_a_list() {
    let result = append("a:\n  - x\n  # end of list\nb: 1\n", "a", &text("z"));
    assert_eq!(result, "a:\n  - x\n  - z\n  # end of list\nb: 1\n");
}

#[test]
fn appends_to_flow_lists_in_their_style() {
    assert_eq!(append("a: [x, y]\n", "a", &text("z")), "a: [x, y, z]\n");
    assert_eq!(append("a: []\n", "a", &text("z")), "a: [z]\n");
    assert_eq!(append("a: [ ]\n", "a", &text("z")), "a: [ z ]\n");
    assert_eq!(append("a: [x, y,]\n", "a", &text("z")), "a: [x, y, z,]\n");
    assert_eq!(
        append("a: [\n  x,\n  y\n]\n", "a", &text("z")),
        "a: [\n  x,\n  y,\n  z\n]\n"
    );
    assert_eq!(append("a: [x]\n", "a", &text("a, b")), "a: [x, \"a, b\"]\n");
}

#[test]
fn appends_to_an_empty_value_by_creating_a_list() {
    assert_eq!(append("a:\nb: 1\n", "a", &text("z")), "a:\n  - z\nb: 1\n");
    assert_eq!(
        append("a:  # none yet\nb: 1\n", "a", &text("z")),
        "a:  # none yet\n  - z\nb: 1\n"
    );
}

#[test]
fn appends_to_a_top_level_list_with_crlf_line_breaks() {
    assert_eq!(
        append("- a\r\n- b\r\n", "$", &text("c")),
        "- a\r\n- b\r\n- c\r\n"
    );
}

#[test]
fn appends_through_an_alias_to_the_anchored_list() {
    let source = "base: &b [x]\nuse: *b\n";
    let model = parse(source);
    let alias = node_at(&model, "use");
    let plan = seq::plan_append(&model, alias, &text("z"), &key("use[+]")).expect("append plan");
    assert_eq!(plan.target, node_at(&model, "base"));
    assert_eq!(
        apply_edits(&model, &plan.edits),
        "base: &b [x, z]\nuse: *b\n"
    );
    let shared = anchor::shared_by(&model, plan.target);
    assert_eq!(shared, vec![key("use")]);
}

#[test]
fn rejects_append_to_a_scalar_or_a_mapping() {
    let model = parse("a: 1\nm: {k: v}\n");
    for path in ["a", "m"] {
        let error = seq::plan_append(&model, node_at(&model, path), &text("z"), &key(path))
            .expect_err("not a list");
        assert_eq!(error.code, ErrorCode::KeyConflict);
    }
}

#[test]
fn selects_items_by_positive_and_negative_index_and_appends_at_the_length() {
    let model = parse("a: [x, y, z]\n");
    let list = node_at(&model, "a");
    let k = key("a[0]");
    assert_eq!(
        seq::index_target(&model, list, -1, &k).expect("last"),
        IndexTarget::Item(node_at(&model, "a[2]"))
    );
    assert_eq!(
        seq::index_target(&model, list, 1, &k).expect("second"),
        IndexTarget::Item(node_at(&model, "a[1]"))
    );
    assert_eq!(
        seq::index_target(&model, list, 3, &k).expect("length"),
        IndexTarget::Append
    );
    assert_eq!(
        seq::index_target(&model, list, 4, &k)
            .expect_err("past end")
            .code,
        ErrorCode::KeyConflict
    );
    assert_eq!(
        seq::index_target(&model, list, -4, &k)
            .expect_err("before start")
            .code,
        ErrorCode::KeyNotFound
    );
}

#[test]
fn removes_a_block_item_with_its_leading_and_inline_comments() {
    let source = "a:\n  # first\n  - x\n  - y  # second\n  - w\n";
    assert_eq!(remove(source, "a", 1), "a:\n  # first\n  - x\n  - w\n");
    assert_eq!(remove(source, "a", 0), "a:\n  - y  # second\n  - w\n");
    assert_eq!(
        remove(source, "a", -1),
        "a:\n  # first\n  - x\n  - y  # second\n"
    );
}

#[test]
fn keeps_a_comment_separated_from_the_removed_item_by_a_blank_line() {
    let result = remove("a:\n  - x\n  # about the list\n\n  - y\nb: 1\n", "a", 1);
    assert_eq!(result, "a:\n  - x\n  # about the list\n\nb: 1\n");
}

#[test]
fn removes_a_mapping_item_with_its_nested_lines() {
    let source = "l:\n  - name: a\n    tags:\n      - t\n      # end of tags\n  - name: b\n";
    assert_eq!(remove(source, "l", 0), "l:\n  - name: b\n");
}

#[test]
fn leaves_an_empty_flow_list_when_the_only_item_is_removed() {
    assert_eq!(remove("a:\n  - x\nb: 1\n", "a", 0), "a: []\nb: 1\n");
    assert_eq!(remove("a:  # kept\n  - x\n", "a", 0), "a: []  # kept\n");
    assert_eq!(remove("- x\n", "$", 0), "[]\n");
}

#[test]
fn writes_the_empty_list_after_the_properties_of_the_list() {
    assert_eq!(
        remove("e: &node # c\n - x\nf: 1\n", "e", 0),
        "e: &node [] # c\nf: 1\n"
    );
    assert_eq!(
        remove("e:\n !!seq # c\n - x\nf: 1\n", "e", 0),
        "e:\n !!seq [] # c\nf: 1\n"
    );
}

#[test]
fn removes_flow_items_with_their_separator() {
    assert_eq!(remove("a: [x, y, z]\n", "a", 1), "a: [x, z]\n");
    assert_eq!(remove("a: [x, y, z]\n", "a", 2), "a: [x, y]\n");
    assert_eq!(remove("a: [x]\n", "a", 0), "a: []\n");
    assert_eq!(remove("a: [x, y,]\n", "a", 1), "a: [x,]\n");
}

#[test]
fn removes_the_first_item_of_a_list_nested_on_the_dash_line() {
    assert_eq!(remove("- - a\n  - b\n", "[0]", 0), "- - b\n");
}

#[test]
fn resolves_aliases_to_the_closest_preceding_anchor() {
    let model = parse("a: &v one\nb: *v\nc: &v two\nd: *v\n");
    let anchors = Anchors::new(&model);
    assert_eq!(
        anchors.target(node_at(&model, "b")),
        Some(node_at(&model, "a"))
    );
    assert_eq!(
        anchors.target(node_at(&model, "d")),
        Some(node_at(&model, "c"))
    );
    assert_eq!(anchor::anchor_of(&model, node_at(&model, "a")), Some("v"));
    assert_eq!(anchor::alias_of(&model, node_at(&model, "d")), Some("v"));
    assert_eq!(anchor::alias_of(&model, node_at(&model, "a")), None);
}

#[test]
fn lists_alias_paths_that_see_a_change() {
    let model = parse("base: &b\n  x: 1\nuse: *b\nother: [*b]\n");
    let shared = anchor::shared_by(&model, node_at(&model, "base.x"));
    assert_eq!(shared, vec![key("other[0].x"), key("use.x")]);
    let mut report = EditReport::default();
    anchor::note_shared(&mut report, &shared);
    let entry = report
        .notable
        .iter()
        .find(|entry| entry.code == Notable::AliasShared)
        .expect("alias_shared");
    assert_eq!(entry.count, Some(2));
    assert!(
        entry
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("use.x"))
    );
    assert!(
        report
            .flags
            .iter()
            .any(|flag| flag == anchor::ALIAS_SHARED_FLAG)
    );
}

#[test]
fn follows_aliases_inside_aliased_nodes_and_merge_keys() {
    let model = parse("a: &a [1]\nb: &b [*a]\nc: *b\n");
    assert_eq!(
        anchor::shared_by(&model, node_at(&model, "a[0]")),
        vec![key("b[0][0]"), key("c[0][0]")]
    );
    let model = parse("base: &b {x: 1, y: 2}\no:\n  <<: *b\n  z: 3\np:\n  <<: *b\n  x: 9\n");
    assert_eq!(
        anchor::shared_by(&model, node_at(&model, "base.x")),
        vec![key("o.x")]
    );
    assert_eq!(
        anchor::shared_by(&model, node_at(&model, "base.y")),
        vec![key("o.y"), key("p.y")]
    );
}

#[test]
fn reports_nothing_for_a_node_without_aliases() {
    let model = parse("a: &unused [1]\nb: 2\n");
    assert!(anchor::shared_by(&model, node_at(&model, "a[0]")).is_empty());
    let mut report = EditReport::default();
    anchor::note_shared(&mut report, &[]);
    assert!(report.notable.is_empty());
}

#[test]
fn accepts_bounded_alias_expansion_and_rejects_exponential_expansion() {
    let small = parse("a: &a [x, x, x]\nb: &b [*a, *a, *a]\nc: [*b, *b, *b]\n");
    assert!(anchor::check_expansion(&small).is_ok());
    let mut laughs = String::from("l0: &l0 [lol, lol, lol, lol, lol, lol, lol, lol, lol]\n");
    for level in 1..=9 {
        let previous = level - 1;
        let refs = vec![format!("*l{previous}"); 9].join(", ");
        laughs.push_str(&format!("l{level}: &l{level} [{refs}]\n"));
    }
    let opened =
        YamlModel::parse(&laughs).and_then(|model| anchor::check_expansion(&model).map(|()| model));
    let error = opened.expect_err("expansion limit");
    assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
    assert_eq!(error.found.as_deref(), Some(YAML_ALIAS_EXPANSION_LIMIT));
}

#[test]
fn rejects_an_alias_inside_its_own_anchored_node() {
    let opened = YamlModel::parse("a: &a [x, *a]\n")
        .and_then(|model| anchor::check_expansion(&model).map(|()| model));
    let error = opened.expect_err("cycle");
    // An anchor is defined once its node ends, so the alias inside the node
    // names an anchor that is not defined yet: the parser refuses the
    // document before the expansion check sees a cycle.
    assert_eq!(error.code, ErrorCode::DocParse);
}

#[test]
fn prints_key_paths_of_nodes_with_document_selectors() {
    let model = parse("a: 1\n---\nb: [x, {c: d}]\n");
    assert_eq!(
        anchor::path_of(&model, node_at(&model, "#1:b[1].c")).to_string(),
        "#1:b[1].c"
    );
    assert_eq!(
        anchor::path_of(&model, node_at(&model, "a")).to_string(),
        "a"
    );
    let model = parse("\"a.b\": {c: 1}\n");
    assert_eq!(
        anchor::path_of(&model, node_at(&model, "[\"a.b\"].c")).to_string(),
        "[\"a.b\"].c"
    );
}

const MERGED: &str = "a: &a {p: 1, q: 1}\nb: &b {p: 2, r: 2}\nm:\n  <<: [*a, *b]\n  q: 9\n";

#[test]
fn merges_keys_with_earlier_sources_taking_precedence() {
    let model = parse(MERGED);
    let map = node_at(&model, "m");
    let inherited = merge::inherited_members(&model, map);
    let names: Vec<&str> = inherited
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, ["p", "r"]);
    assert_eq!(
        merge::inherited_value(&model, map, "p"),
        Some(node_at(&model, "a.p"))
    );
    assert_eq!(
        merge::inherited_value(&model, map, "r"),
        Some(node_at(&model, "b.r"))
    );
    assert_eq!(merge::inherited_value(&model, map, "q"), None);
    assert_eq!(
        merge::merge_sources(&model, map),
        vec![node_at(&model, "a"), node_at(&model, "b")]
    );
}

#[test]
fn merges_through_chained_merge_keys_and_inline_mappings() {
    let model = parse(
        "base: &base {name: base, port: 1}\nnested: &nested\n  <<: *base\n  level: two\ndeep:\n  <<: *nested\n  level: three\ninline:\n  <<: {a: 1, b: 2}\n  b: 3\n",
    );
    let deep = node_at(&model, "deep");
    assert_eq!(
        merge::inherited_value(&model, deep, "name"),
        Some(node_at(&model, "base.name"))
    );
    assert_eq!(merge::inherited_value(&model, deep, "level"), None);
    let inline = node_at(&model, "inline");
    let names: Vec<String> = merge::inherited_members(&model, inline)
        .into_iter()
        .map(|member| member.name)
        .collect();
    assert_eq!(names, ["a"]);
}

#[test]
fn writes_a_local_key_over_a_merged_key_and_leaves_the_sources_alone() {
    let mut doc = YamlDoc::parse(MERGED, YamlVersion::V1_2).unwrap();
    doc.set(
        &KeyPath::parse("m.p").unwrap(),
        &Value::Int(7),
        &SetOptions { create: true },
    )
    .unwrap();
    let text = String::from_utf8(doc.text().to_vec()).unwrap();
    assert!(text.starts_with("a: &a {p: 1, q: 1}\nb: &b {p: 2, r: 2}\nm:\n  <<: [*a, *b]\n"));
    let model = parse(&text);
    let map = node_at(&model, "m");
    assert!(merge::is_override(&model, map, "p"));
    assert_eq!(model.text(&model.node(node_at(&model, "m.p")).span), "7");
}

#[test]
fn recognises_overrides_of_merged_keys_and_refuses_to_remove_a_merged_key() {
    let model = parse(MERGED);
    let map = node_at(&model, "m");
    assert!(merge::is_override(&model, map, "p"));
    assert!(!merge::is_override(&model, map, "q"));
    assert!(!merge::is_override(&model, map, "missing"));
    let error = merge::check_remove(&model, map, "p").expect_err("merged key");
    assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
    assert_eq!(error.found.as_deref(), Some(YAML_MERGE_INHERITED_REMOVE));
    assert!(merge::check_remove(&model, map, "q").is_ok());
    assert!(merge::check_remove(&model, map, "missing").is_ok());
}

#[test]
fn names_explicit_and_complex_keys() {
    let model = parse(
        "? [a, b]\n: seq\n? {x: 1}\n: map\n? plain\n: value\n&k key: v\nother:\n  *k : again\n",
    );
    let names_of = |map| -> Vec<Option<String>> {
        model
            .node(map)
            .as_mapping()
            .expect("mapping")
            .entries
            .iter()
            .map(|entry| merge::key_name(&model, entry.key))
            .collect()
    };
    assert_eq!(
        names_of(model.documents()[0].root),
        [
            Some("[a, b]".to_owned()),
            Some("{x: 1}".to_owned()),
            Some("plain".to_owned()),
            Some("key".to_owned()),
            Some("other".to_owned())
        ]
    );
    assert_eq!(names_of(node_at(&model, "other")), [Some("key".to_owned())]);
}

#[test]
fn changes_the_value_under_an_explicit_key_but_not_the_key() {
    let source = "? [a, b]\n: old\nz: 1\n";
    let model = parse(source);
    let value = node_at(&model, "[\"[a, b]\"]");
    let span = model.node(value).span.clone();
    let value_edit = [ByteEdit::replace(Span::new(span.start, span.end), "new")];
    assert!(merge::guard_key_edits(&model, &value_edit).is_ok());
    assert_eq!(apply_edits(&model, &value_edit), "? [a, b]\n: new\nz: 1\n");
    let key_start = source.find('a').unwrap_or_default();
    let key_edit = [ByteEdit::replace(Span::new(key_start, key_start + 1), "c")];
    let error = merge::guard_key_edits(&model, &key_edit).expect_err("explicit key");
    assert_eq!(error.found.as_deref(), Some(YAML_COMPLEX_KEY));
    assert!(merge::check_new_key("short").is_ok());
    assert_eq!(
        merge::check_new_key("two\nlines")
            .expect_err("multi-line key")
            .found
            .as_deref(),
        Some(YAML_COMPLEX_KEY)
    );
    assert!(merge::check_new_key(&"k".repeat(merge::MAX_IMPLICIT_KEY_BYTES + 1)).is_err());
}

const STREAM: &str = "%YAML 1.2\n---\na: 1\n...\n# between\n---\nb: 2\n--- plain\n";

#[test]
fn selects_documents_of_a_stream() {
    let model = parse(STREAM);
    assert_eq!(model.documents().len(), 3);
    assert_eq!(multidoc::select(&model, None), [0]);
    assert_eq!(multidoc::select(&model, Some(DocSelector::Index(2))), [2]);
    assert!(multidoc::select(&model, Some(DocSelector::Index(3))).is_empty());
    assert_eq!(multidoc::select(&model, Some(DocSelector::All)), [0, 1, 2]);
    assert_eq!(multidoc::item_document(&model, 1), Some(1));
    assert_eq!(multidoc::item_document(&parse("a: 1\n"), 0), None);
    assert_eq!(multidoc::selector_of(0), None);
    assert_eq!(multidoc::selector_of(2), Some(DocSelector::Index(2)));
    let error = multidoc::document_not_found(&model, &key("#5:a"));
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert_eq!(error.candidates.len(), 3);
}

#[test]
fn reads_the_comment_of_a_document() {
    let model = parse("# file header\n\n# about a\na: 1\n---\n# second\n\nb: 2\n");
    assert_eq!(
        multidoc::document_comment(&model, 0).as_deref(),
        Some("file header")
    );
    assert_eq!(
        multidoc::document_comment(&model, 1).as_deref(),
        Some("second")
    );
}

#[test]
fn inserts_into_an_empty_document_after_its_marker() {
    let model = parse("---\n---\nb: 1\n");
    let (offset, newline) = multidoc::empty_root_insertion(&model, 0).expect("empty root");
    assert_eq!(offset, 3);
    assert!(newline);
    assert_eq!(multidoc::empty_root_insertion(&model, 1), None);
}

#[test]
fn binds_leading_inline_and_trailing_comments() {
    let source = "# port of the service\nport: 80  # default\nhost: a\nlist:\n  - x\n  # closes the list\nurl: \"http://h/#frag\"  # real\n";
    let model = parse(source);
    let comments = Comments::new(&model);
    let port = source.find("port:").unwrap_or_default();
    assert_eq!(
        comments.describe(port, port + 8).as_deref(),
        Some("port of the service\ndefault")
    );
    let host = source.find("host:").unwrap_or_default();
    assert_eq!(comments.describe(host, host + 7), None);
    let x = source.find("- x").unwrap_or_default();
    let trailing = comments.trailing(x + 3, 2);
    assert_eq!(comments.join(&trailing).as_deref(), Some("closes the list"));
    let url = source.find("url:").unwrap_or_default();
    assert_eq!(comments.describe(url, url + 21).as_deref(), Some("real"));
    let removal = comments.removal_span(port..port + 19, None);
    assert_eq!(
        &source[removal.clone()],
        "# port of the service\nport: 80  # default\n"
    );
}

#[test]
fn treats_hash_lines_of_block_scalars_as_content() {
    let source = "script: |\n  # not a comment\n  run\nnext: 1\n";
    let model = parse(source);
    assert!(model.comments().is_empty());
    let next = source.find("next:").unwrap_or_default();
    assert!(Comments::new(&model).leading(next).is_empty());
}

#[test]
fn keeps_tags_of_the_same_kind_and_removes_others() {
    let model = parse("a: !!int 5\nb: !Ref Foo\nc: !!binary aGVsbG8=\nd: !!str &s 10\ne: 3\n");
    let plan = tag::plan_set(&model, node_at(&model, "a"), &text("x"));
    assert_eq!(plan.removed.as_deref(), Some("!!int"));
    assert_eq!(
        apply_edits(&model, &plan.edits),
        "a: 5\nb: !Ref Foo\nc: !!binary aGVsbG8=\nd: !!str &s 10\ne: 3\n"
    );
    assert!(
        tag::plan_set(&model, node_at(&model, "a"), &Value::Int(7))
            .edits
            .is_empty()
    );
    assert!(
        tag::plan_set(&model, node_at(&model, "b"), &text("Other"))
            .edits
            .is_empty()
    );
    let collection = tag::plan_set(&model, node_at(&model, "b"), &Value::List(Vec::new()));
    assert_eq!(collection.removed.as_deref(), Some("!Ref"));
    let bytes = Value::Bytes(Bytes::new(b"bye".to_vec()));
    assert!(
        tag::plan_set(&model, node_at(&model, "c"), &bytes)
            .edits
            .is_empty()
    );
    let anchored = tag::plan_set(&model, node_at(&model, "d"), &Value::Int(1));
    assert_eq!(
        apply_edits(&model, &anchored.edits),
        "a: !!int 5\nb: !Ref Foo\nc: !!binary aGVsbG8=\nd: &s 10\ne: 3\n"
    );
    assert_eq!(
        tag::plan_set(&model, node_at(&model, "e"), &text("x")),
        tag::TagPlan::default()
    );
}

#[test]
fn removes_a_tag_at_the_end_of_its_line_with_the_space_before_it() {
    let model = parse("a: !!seq\n- x\n");
    let plan = tag::plan_set(&model, node_at(&model, "a"), &text("flat"));
    assert_eq!(apply_edits(&model, &plan.edits), "a:\n- x\n");
}

#[test]
fn types_scalars_with_core_tags() {
    assert_eq!(tag::typed_scalar(CoreTag::Str, "10"), Some(text("10")));
    assert_eq!(
        tag::typed_scalar(CoreTag::Int, "0x1F"),
        Some(Value::Int(31))
    );
    assert_eq!(
        tag::typed_scalar(CoreTag::Int, "-12"),
        Some(Value::Int(-12))
    );
    assert_eq!(
        tag::typed_scalar(CoreTag::Int, "99999999999999999999"),
        Some(Value::BigInt("99999999999999999999".to_owned()))
    );
    assert_eq!(tag::typed_scalar(CoreTag::Int, "1.5"), None);
    assert_eq!(
        tag::typed_scalar(CoreTag::Float, "4"),
        Some(Value::Number(4.0))
    );
    assert_eq!(
        tag::typed_scalar(CoreTag::Float, "1.10"),
        Some(Value::Decimal("1.10".to_owned()))
    );
    assert_eq!(
        tag::typed_scalar(CoreTag::Bool, "yes"),
        Some(Value::Bool(true))
    );
    assert_eq!(tag::typed_scalar(CoreTag::Null, ""), Some(Value::Null));
    assert_eq!(
        tag::typed_scalar(CoreTag::Binary, "aGVs\n  bG8="),
        Some(Value::Bytes(Bytes::new(b"hello".to_vec())))
    );
    assert_eq!(tag::typed_scalar(CoreTag::Binary, "not base64!"), None);
    assert_eq!(tag::typed_scalar(CoreTag::Map, "x"), None);
    assert_eq!(CoreTag::of("tag:yaml.org,2002:omap"), Some(CoreTag::Omap));
    assert_eq!(CoreTag::of("!Ref"), None);
}

#[test]
fn writes_binary_data_in_the_style_of_the_replaced_value() {
    let model = parse("a: !!binary aGVsbG8=\nb: !!binary |\n  aGVs\n  bG8=\nc: plain\n");
    let data = Bytes::new(b"hello!".to_vec());
    assert_eq!(
        tag::render_binary(&model, node_at(&model, "a"), &data),
        "aGVsbG8h"
    );
    assert_eq!(
        tag::render_binary(&model, node_at(&model, "b"), &data),
        "|\n  aGVs\n  bG8h"
    );
    assert_eq!(
        tag::render_binary(&model, node_at(&model, "c"), &data),
        "!!binary aGVsbG8h"
    );
}

#[test]
fn reads_tags_of_every_kind_from_the_model() {
    let model = parse(
        "%TAG !e! tag:example.com,2026:\n---\na: !e!widget {x: 1}\nb: !<tag:yaml.org,2002:str> v\nc: !!binary aGk=\nd: ! 12\n",
    );
    let tag_of = |path: &str| model.node(node_at(&model, path)).tag.clone().expect("tag");
    assert_eq!(tag_of("a").resolved, "tag:example.com,2026:widget");
    assert_eq!(tag_of("a").text, "!e!widget");
    assert_eq!(CoreTag::of(&tag_of("b").resolved), Some(CoreTag::Str));
    assert_eq!(CoreTag::of(&tag_of("c").resolved), Some(CoreTag::Binary));
    assert_eq!(tag_of("d").resolved, "!");
}
