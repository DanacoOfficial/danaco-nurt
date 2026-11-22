// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use indexmap::IndexMap;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;

use super::editor::apply;
use super::{Dialect, JsonDocument, reference};

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn int(number: i64) -> Value {
    Value::Int(number)
}

fn text(content: &str) -> Value {
    Value::Text(content.to_owned())
}

/// Applies an operation and checks that the result parses in the dialect
/// and reads the same with the reference parser.
fn checked(document: &JsonDocument, outcome: &super::Outcome) -> String {
    let edited = apply(document.text(), &outcome.changes).unwrap();
    let reparsed = JsonDocument::parse(edited.clone(), document.dialect())
        .unwrap_or_else(|error| panic!("edited text does not parse: {error:?}\n{edited}"));
    let reference = reference::read(&edited, document.dialect()).unwrap();
    assert!(reference::same_data(&reparsed.to_value(), &reference), "{edited}");
    edited
}

fn set_in(source: &str, dialect: Dialect, path: &str, value: &Value) -> String {
    let document = JsonDocument::parse(source, dialect).unwrap();
    let outcome = document.plan_set(&key(path), value, true).unwrap();
    let edited = checked(&document, &outcome);
    if !path.ends_with("[+]") {
        let reread = JsonDocument::parse(edited.clone(), dialect).unwrap();
        let Ok(super::Target::Node { id, .. }) = reread.locate(&key(path)) else {
            panic!("value not found after set");
        };
        assert_eq!(&reread.value_of(id), value);
    }
    edited
}

fn set(source: &str, path: &str, value: &Value) -> String {
    set_in(source, Dialect::Json, path, value)
}

fn remove_in(source: &str, dialect: Dialect, path: &str) -> String {
    let document = JsonDocument::parse(source, dialect).unwrap();
    let outcome = document.plan_remove(&key(path)).unwrap();
    checked(&document, &outcome)
}

fn remove(source: &str, path: &str) -> String {
    remove_in(source, Dialect::Json, path)
}

#[test]
fn replaces_a_scalar_without_touching_other_bytes() {
    let source = "{\n  \"a\": 1.10,\n  \"b\": \"x\",\n  \"c\": [1e400]\n}";
    assert_eq!(set(source, "b", &text("y")), "{\n  \"a\": 1.10,\n  \"b\": \"y\",\n  \"c\": [1e400]\n}");
}

#[test]
fn setting_the_current_value_changes_nothing() {
    let source = "{\"a\": 1.10, \"s\": \"\\u00e9\\/\", \"n\": 18446744073709551616}";
    let document = JsonDocument::parse(source, Dialect::Json).unwrap();
    for (path, value) in [
        ("a", Value::Decimal("1.10".to_owned())),
        ("s", text("é/")),
        ("n", Value::BigInt("18446744073709551616".to_owned())),
    ] {
        assert!(document.plan_set(&key(path), &value, true).unwrap().changes.is_empty(), "{path}");
    }
}

#[test]
fn adds_a_member_on_a_new_line_with_sibling_indentation() {
    assert_eq!(set("{\n  \"a\": 1\n}\n", "b", &int(2)), "{\n  \"a\": 1,\n  \"b\": 2\n}\n");
    assert_eq!(set("{\n\t\"a\": 1\n}", "b", &int(2)), "{\n\t\"a\": 1,\n\t\"b\": 2\n}");
    assert_eq!(set("{\r\n    \"a\": 1\r\n}\r\n", "b", &int(2)), "{\r\n    \"a\": 1,\r\n    \"b\": 2\r\n}\r\n");
}

#[test]
fn adds_a_member_on_one_line_in_single_line_documents() {
    assert_eq!(set("{\"a\":1}", "b", &int(2)), "{\"a\":1,\"b\":2}");
    assert_eq!(set("{ \"a\": 1 }", "b", &int(2)), "{ \"a\": 1, \"b\": 2 }");
    assert_eq!(set("{}", "a", &int(1)), "{\"a\": 1}");
}

#[test]
fn keeps_a_comment_after_the_last_member_on_its_line() {
    let source = "{\n  \"a\": 1 // first\n}";
    assert_eq!(
        set_in(source, Dialect::Jsonc, "b", &int(2)),
        "{\n  \"a\": 1, // first\n  \"b\": 2\n}"
    );
}

#[test]
fn follows_the_trailing_comma_style_of_jsonc_files() {
    let source = "{\n  \"a\": 1,\n}";
    assert_eq!(set_in(source, Dialect::Jsonc, "b", &int(2)), "{\n  \"a\": 1,\n  \"b\": 2,\n}");
    assert_eq!(set_in("[1, 2,]", Dialect::Jsonc, "[+]", &int(3)), "[1, 2, 3,]");
}

#[test]
fn expands_an_empty_container_in_a_multi_line_document() {
    let source = "{\n  \"a\": [],\n  \"b\": {}\n}";
    assert_eq!(set(source, "a[+]", &int(1)), "{\n  \"a\": [\n    1\n  ],\n  \"b\": {}\n}");
    assert_eq!(set(source, "b.c", &text("x")), "{\n  \"a\": [],\n  \"b\": {\n    \"c\": \"x\"\n  }\n}");
}

#[test]
fn appends_to_lists_in_the_style_of_their_elements() {
    assert_eq!(set("[1, 2]", "[+]", &int(3)), "[1, 2, 3]");
    assert_eq!(set("[\n  1,\n  2\n]", "[+]", &int(3)), "[\n  1,\n  2,\n  3\n]");
    assert_eq!(set("{\"l\": [1]}", "l[1]", &int(2)), "{\"l\": [1, 2]}");
}

#[test]
fn rejects_an_index_beyond_the_end_of_a_list() {
    let document = JsonDocument::parse("[1]", Dialect::Json).unwrap();
    let error = document.plan_set(&key("[3]"), &int(2), true).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

#[test]
fn creates_intermediate_members() {
    let document = JsonDocument::parse("{}", Dialect::Json).unwrap();
    let outcome = document.plan_set(&key("a.b[0]"), &int(1), true).unwrap();
    assert!(outcome.keys_created);
    assert_eq!(checked(&document, &outcome), "{\"a\": {\"b\": [1]}}");
    let error = document.plan_set(&key("a.b"), &int(1), false).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyNotFound);
}

#[test]
fn changes_the_type_of_a_value_in_the_style_of_the_context() {
    let mut fields = IndexMap::new();
    fields.insert("x".to_owned(), Value::List(vec![int(1), int(2)]));
    let map = Value::Map(fields);
    assert_eq!(set("{\"a\": 1}", "a", &map), "{\"a\": {\"x\": [1, 2]}}");
    assert_eq!(
        set("{\n  \"a\": 1\n}", "a", &map),
        "{\n  \"a\": {\n    \"x\": [\n      1,\n      2\n    ]\n  }\n}"
    );
    assert_eq!(set("{\"a\": {\"b\": [1]}}", "a", &Value::Null), "{\"a\": null}");
}

#[test]
fn writes_new_strings_with_minimal_escaping() {
    assert_eq!(set("{\"a\": 1}", "a", &text("é \"q\" /\n")), "{\"a\": \"é \\\"q\\\" /\\n\"}");
}

#[test]
fn changes_the_last_occurrence_of_a_repeated_member() {
    let source = "{\"a\": 1, \"a\": 2}";
    let document = JsonDocument::parse(source, Dialect::Json).unwrap();
    assert_eq!(document.to_value().get("a"), Some(&int(2)));
    let outcome = document.plan_set(&key("a"), &int(3), true).unwrap();
    assert!(outcome.duplicate_key);
    assert_eq!(checked(&document, &outcome), "{\"a\": 1, \"a\": 3}");
}

#[test]
fn removes_members_with_their_separator() {
    let source = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}";
    assert_eq!(remove(source, "a"), "{\n  \"b\": 2,\n  \"c\": 3\n}");
    assert_eq!(remove(source, "b"), "{\n  \"a\": 1,\n  \"c\": 3\n}");
    assert_eq!(remove(source, "c"), "{\n  \"a\": 1,\n  \"b\": 2\n}");
    assert_eq!(remove("{\"a\": 1, \"b\": 2, \"c\": 3}", "b"), "{\"a\": 1, \"c\": 3}");
    assert_eq!(remove("{\"a\": 1, \"b\": 2}", "a"), "{\"b\": 2}");
    assert_eq!(remove("{\"a\": 1, \"b\": 2}", "b"), "{\"a\": 1}");
    assert_eq!(remove("[1, 2, 3]", "[-1]"), "[1, 2]");
}

#[test]
fn removing_the_only_member_leaves_an_empty_container() {
    assert_eq!(remove("{\n  \"a\": {\n    \"b\": 1\n  }\n}", "a.b"), "{\n  \"a\": {}\n}");
    assert_eq!(remove("{ \"a\": 1 }", "a"), "{}");
}

#[test]
fn removes_comments_attached_to_a_member() {
    let source = "{\n  \"a\": 1,\n\n  // kept: separated by a blank line\n\n  // about b\n  /* more about b */\n  \"b\": 2, // b\n  \"c\": 3\n}";
    assert_eq!(
        remove_in(source, Dialect::Jsonc, "b"),
        "{\n  \"a\": 1,\n\n  // kept: separated by a blank line\n\n  \"c\": 3\n}"
    );
}

#[test]
fn removing_the_last_member_keeps_the_trailing_comma_style() {
    let source = "{\n  \"a\": 1,\n  \"b\": 2,\n}";
    assert_eq!(remove_in(source, Dialect::Jsonc, "b"), "{\n  \"a\": 1,\n}");
    assert_eq!(remove("{\n  \"a\": 1,\n  \"b\": 2\n}", "b"), "{\n  \"a\": 1\n}");
}

#[test]
fn keeps_a_missing_final_newline() {
    assert_eq!(set("{\n  \"a\": 1\n}", "b", &int(2)), "{\n  \"a\": 1,\n  \"b\": 2\n}");
    assert_eq!(remove("{\n  \"a\": 1,\n  \"b\": 2\n}", "a"), "{\n  \"b\": 2\n}");
}

#[test]
fn reports_syntax_errors_with_their_position() {
    let error = JsonDocument::parse("{\n  \"a\": 1, // c\n}", Dialect::Json).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DocParse);
    let location = error.data().location.clone().unwrap();
    assert_eq!((location.line, location.column), (Some(2), Some(11)));
    for source in ["[1,]", "{\"a\" 1}", "[01]", "[\"\t\"]", "\u{0C}[]", "{'a': 1}", "", "[1] x"] {
        assert!(JsonDocument::parse(source, Dialect::Json).is_err(), "{source:?}");
    }
    assert!(JsonDocument::parse("[1, /* c */ 2,] // end", Dialect::Jsonc).is_ok());
    assert!(JsonDocument::parse("[1,,2]", Dialect::Jsonc).is_err());
}

#[test]
fn rejects_documents_nested_beyond_the_limit() {
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    assert!(JsonDocument::parse(deep, Dialect::Json).is_err());
    let shallow = format!("{}{}", "[".repeat(100), "]".repeat(100));
    assert!(JsonDocument::parse(shallow, Dialect::Json).is_ok());
}

#[test]
fn detects_the_dialect_from_the_file_name() {
    use std::path::Path;
    let cases = [
        ("a/package.json", Some(Dialect::Json)),
        ("tsconfig.json", Some(Dialect::Jsonc)),
        ("tsconfig.build.json", Some(Dialect::Jsonc)),
        (".vscode/launch.json", Some(Dialect::Jsonc)),
        (".devcontainer/devcontainer.json", Some(Dialect::Jsonc)),
        (".eslintrc.json", Some(Dialect::Jsonc)),
        ("x.jsonc", Some(Dialect::Jsonc)),
        ("x.json5", None),
        ("x.yaml", None),
    ];
    for (path, expected) in cases {
        assert_eq!(Dialect::detect(Path::new(path)), expected, "{path}");
    }
}
