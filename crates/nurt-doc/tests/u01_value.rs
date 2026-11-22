// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Number lexemes, scalar rendering, value comparison, quoting rules and
//! value edits.
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use std::cmp::Ordering;

use nurt_contracts::config::YamlVersion;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::{Date, DateKind, Record, RecordKind, Size, Value};
use nurt_doc::model::{FlatSemantics, MapSemantics};
use nurt_doc::value::{
    PlainKind, QuoteRules, Syntax, apply_remove, apply_set, compare_numbers, equivalent,
    first_difference, limit_depth, needs_quotes, numeric, parse_number, render_scalar,
    to_document_value, value_type, yaml_plain_kind,
};
use serde_json::json;

fn decimal(lexeme: &str) -> Value {
    Value::Decimal(lexeme.to_owned())
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn plain(value: serde_json::Value) -> Value {
    Value::from_plain_json(value)
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).expect("valid key path")
}

fn date(text: &str) -> Value {
    Value::Date(Date {
        kind: DateKind::LocalDate,
        text: text.to_owned(),
    })
}

fn check_numbers(syntax: Syntax, cases: &[(&str, Option<Value>)]) {
    for (lexeme, expected) in cases {
        assert_eq!(
            &parse_number(lexeme, syntax),
            expected,
            "{lexeme} in {syntax:?}"
        );
    }
}

#[test]
fn json_numbers_keep_lexemes_that_binary_form_changes() {
    check_numbers(
        Syntax::Json,
        &[
            ("0", Some(Value::Int(0))),
            ("-12", Some(Value::Int(-12))),
            ("1.5", Some(Value::Number(1.5))),
            ("1.10", Some(decimal("1.10"))),
            ("1e400", Some(decimal("1e400"))),
            ("1E2", Some(decimal("1E2"))),
            ("1e+5", Some(decimal("1e+5"))),
            (
                "18446744073709551616",
                Some(Value::BigInt("18446744073709551616".to_owned())),
            ),
            ("01", None),
            ("1.", None),
            ("-", None),
            ("1e++5", None),
            ("0x10", None),
        ],
    );
}

#[test]
fn toml_numbers_accept_prefixes_and_underscores() {
    check_numbers(
        Syntax::Toml,
        &[
            ("0x1F", Some(Value::Int(31))),
            ("0o17", Some(Value::Int(15))),
            ("0b101", Some(Value::Int(5))),
            ("1_000", Some(Value::Int(1000))),
            ("-7", Some(Value::Int(-7))),
            ("2.75", Some(Value::Number(2.75))),
            ("1e3", Some(decimal("1e3"))),
            ("1_0.5", Some(decimal("1_0.5"))),
            ("inf", Some(decimal("inf"))),
            ("-nan", Some(decimal("-nan"))),
            ("+0x1", None),
            ("1__0", None),
            ("_1", None),
            ("01", None),
            ("1._5", None),
        ],
    );
}

#[test]
fn yaml_numbers_follow_the_schema_version() {
    check_numbers(
        Syntax::Yaml12,
        &[
            ("0o17", Some(Value::Int(15))),
            ("0x1f", Some(Value::Int(31))),
            ("017", Some(Value::Int(17))),
            ("1:30", None),
            (".inf", Some(decimal(".inf"))),
            ("-.inf", Some(decimal("-.inf"))),
            (".nan", Some(decimal(".nan"))),
            ("yes", None),
        ],
    );
    check_numbers(
        Syntax::Yaml11,
        &[
            ("1:30", Some(Value::Int(90))),
            ("-1:00", Some(Value::Int(-60))),
            ("017", Some(Value::Int(15))),
            ("0b1_0", Some(Value::Int(2))),
            ("1_000", Some(Value::Int(1000))),
            ("190:20:30.15", Some(decimal("190:20:30.15"))),
            ("2.5", Some(Value::Number(2.5))),
        ],
    );
    check_numbers(Syntax::Untyped, &[("1", None)]);
}

#[test]
fn scalars_render_in_the_syntax_of_the_format() {
    let cases = [
        (Value::Null, Syntax::Toml, None),
        (Value::Null, Syntax::Untyped, Some("")),
        (Value::Null, Syntax::Json, Some("null")),
        (Value::Bool(false), Syntax::Toml, Some("false")),
        (Value::Int(-3), Syntax::Json, Some("-3")),
        (
            Value::BigInt("99999999999999999999".to_owned()),
            Syntax::Json,
            Some("99999999999999999999"),
        ),
        (Value::Number(2.0), Syntax::Json, Some("2.0")),
        (Value::Number(1e300), Syntax::Json, Some("1e300")),
        (Value::Number(1e-7), Syntax::Yaml11, Some("1.0e-7")),
        (Value::Number(f64::NAN), Syntax::Json, None),
        (decimal("1.10"), Syntax::Json, Some("1.10")),
        (decimal(".inf"), Syntax::Json, None),
        (decimal(".inf"), Syntax::Untyped, Some(".inf")),
        (date("2026-11-22"), Syntax::Toml, Some("2026-11-22")),
        (text("a"), Syntax::Json, None),
    ];
    for (value, syntax, expected) in cases {
        assert_eq!(
            render_scalar(&value, syntax).as_deref(),
            expected,
            "{value:?} in {syntax:?}"
        );
    }
}

#[test]
fn rendered_numbers_parse_back_to_equivalent_values() {
    for number in [0.5, -2.25, 1e21, 1.5e-9, 123_456.789] {
        for syntax in [Syntax::Json, Syntax::Toml, Syntax::Yaml11, Syntax::Yaml12] {
            let rendered =
                render_scalar(&Value::Number(number), syntax).expect("finite number renders");
            let parsed = parse_number(&rendered, syntax).expect("rendered number parses");
            assert!(
                equivalent(&parsed, &Value::Number(number)),
                "{rendered} in {syntax:?}"
            );
        }
    }
}

#[test]
fn numbers_compare_across_representations() {
    assert!(numeric(&decimal("1e3")));
    assert!(!numeric(&text("1")));
    assert!(equivalent(&Value::Int(1000), &decimal("1_000")));
    assert!(equivalent(&Value::Int(2), &Value::Number(2.0)));
    assert!(equivalent(&decimal("1.10"), &Value::Number(1.1)));
    assert!(equivalent(&decimal(".nan"), &Value::Number(f64::NAN)));
    assert!(!equivalent(&Value::Int(2), &text("2")));
    let big = Value::BigInt("18446744073709551616".to_owned());
    assert_eq!(compare_numbers(&Value::Int(3), &big), Some(Ordering::Less));
    assert_eq!(
        compare_numbers(&Value::Number(2.5), &Value::Int(2)),
        Some(Ordering::Greater)
    );
    assert_eq!(compare_numbers(&Value::Int(1), &text("1")), None);
}

#[test]
fn first_difference_names_the_differing_key() {
    let left = plain(json!({"a": {"b": 1, "c": 2}}));
    let changed = plain(json!({"a": {"b": 1, "c": 3}}));
    let extra = plain(json!({"a": {"b": 1, "c": 2}, "d": 4}));
    let reordered = plain(json!({"a": {"c": 2, "b": 1}}));
    let missing = plain(json!({"a": {"b": 1}}));
    let as_text = |value: &Value| first_difference(&left, value).map(|path| path.to_string());
    assert_eq!(as_text(&left), None);
    assert_eq!(as_text(&changed).as_deref(), Some("a.c"));
    assert_eq!(as_text(&extra).as_deref(), Some("d"));
    assert_eq!(as_text(&missing).as_deref(), Some("a.c"));
    assert_eq!(as_text(&reordered), None);
    assert!(!equivalent(
        &plain(json!({"a": [1, 2]})),
        &plain(json!({"a": [1, 3]}))
    ));
    let shorter = plain(json!([1]));
    let longer = plain(json!([1, 2]));
    let difference =
        |left: &Value, right: &Value| first_difference(left, right).map(|path| path.to_string());
    assert_eq!(difference(&shorter, &longer).as_deref(), Some("[1]"));
    assert_eq!(difference(&longer, &shorter).as_deref(), Some("[1]"));
}

#[test]
fn dates_equal_their_text() {
    assert!(equivalent(&date("2026-11-22"), &text("2026-11-22")));
    assert!(!equivalent(&date("2026-11-22"), &text("2026-11-23")));
}

#[test]
fn yaml_plain_scalars_resolve_by_version() {
    let cases = [
        ("Yes", PlainKind::Bool, PlainKind::Str),
        ("off", PlainKind::Bool, PlainKind::Str),
        ("true", PlainKind::Bool, PlainKind::Bool),
        ("~", PlainKind::Null, PlainKind::Null),
        ("", PlainKind::Null, PlainKind::Null),
        ("0o17", PlainKind::Str, PlainKind::Int),
        ("017", PlainKind::Int, PlainKind::Int),
        ("1e3", PlainKind::Str, PlainKind::Float),
        ("1.0e+3", PlainKind::Float, PlainKind::Float),
        ("+12", PlainKind::Int, PlainKind::Int),
        (".NaN", PlainKind::Float, PlainKind::Float),
        ("-.inf", PlainKind::Float, PlainKind::Float),
        ("1:30", PlainKind::Int, PlainKind::Str),
        ("2026-11-22", PlainKind::Timestamp, PlainKind::Str),
        ("2026-11-22T10:00:00Z", PlainKind::Timestamp, PlainKind::Str),
        (
            "2026-11-22 10:00:00.5 +01:00",
            PlainKind::Timestamp,
            PlainKind::Str,
        ),
        ("2026-11-22T10:00", PlainKind::Str, PlainKind::Str),
        ("hello", PlainKind::Str, PlainKind::Str),
    ];
    for (text, yaml11, yaml12) in cases {
        assert_eq!(
            yaml_plain_kind(text, YamlVersion::V1_1),
            yaml11,
            "{text:?} in 1.1"
        );
        assert_eq!(
            yaml_plain_kind(text, YamlVersion::V1_2),
            yaml12,
            "{text:?} in 1.2"
        );
    }
}

#[test]
fn yaml_text_is_quoted_when_any_version_would_type_it() {
    let quoted = [
        "yes",
        "1.0",
        "null",
        "2026-11-22",
        "~",
        "",
        "a: b",
        "a #b",
        "- x",
        "x:",
        "<<",
        "tab\there",
        " lead",
        "&anchor",
        "0o17",
        "line\nbreak",
    ];
    for text in quoted {
        assert!(needs_quotes(text, QuoteRules::Yaml), "{text:?}");
    }
    for text in ["plain text", "a:b", "a#b", "version-2", "zażółć"] {
        assert!(!needs_quotes(text, QuoteRules::Yaml), "{text:?}");
    }
}

#[test]
fn ini_and_env_values_are_quoted_for_comment_and_shell_characters() {
    for text in [" a", "a ", "a;b", "a#b", "'x", "[x", "say \"hi\""] {
        assert!(needs_quotes(text, QuoteRules::Ini), "{text:?}");
    }
    assert!(!needs_quotes("plain value", QuoteRules::Ini));
    for text in ["a b", "$HOME", "a#b", "x;y", "`cmd`", "a\\b", "\u{1}"] {
        assert!(needs_quotes(text, QuoteRules::Env), "{text:?}");
    }
    assert!(!needs_quotes("plain_value-1.2", QuoteRules::Env));
}

#[test]
fn language_values_become_document_values() {
    assert_eq!(
        to_document_value(Value::Path("a/b".to_owned())),
        text("a/b")
    );
    assert_eq!(
        to_document_value(Value::Size(Size::from_bytes(512))),
        Value::Int(512)
    );
    let record = Record::new(RecordKind::Item).with("size", Value::Size(Size::from_bytes(2)));
    assert_eq!(
        to_document_value(Value::Record(record)),
        plain(json!({"size": 2}))
    );
    assert_eq!(
        to_document_value(Value::List(vec![Value::Key("k".to_owned())])),
        plain(json!(["k"]))
    );
    assert_eq!(to_document_value(Value::Int(1)), Value::Int(1));
}

#[test]
fn value_types_use_record_names() {
    assert_eq!(value_type(&decimal("1e400")), "number");
    assert_eq!(value_type(&Value::BigInt("1".to_owned())), "int");
    assert_eq!(value_type(&Value::Null), "null");
    assert_eq!(value_type(&plain(json!({}))), "map");
    assert_eq!(value_type(&plain(json!([]))), "list");
    assert_eq!(value_type(&Value::Path("x".to_owned())), "text");
}

#[test]
fn depth_limit_empties_deeper_containers() {
    let nested = plain(json!({"a": {"b": [1]}, "c": 1}));
    assert_eq!(limit_depth(&nested, None), nested);
    assert_eq!(
        limit_depth(&nested, Some(1)),
        plain(json!({"a": {}, "c": 1}))
    );
    assert_eq!(
        limit_depth(&nested, Some(2)),
        plain(json!({"a": {"b": []}, "c": 1}))
    );
    assert_eq!(limit_depth(&nested, Some(0)), plain(json!({})));
}

#[test]
fn set_creates_missing_maps_and_lists() {
    let semantics = MapSemantics::LAST_WINS;
    let mut root = Value::Null;
    apply_set(&mut root, &key("a.b[0]"), Value::Int(1), &semantics).expect("creates");
    apply_set(&mut root, &key("a.c.d"), text("x"), &semantics).expect("creates");
    apply_set(&mut root, &key("a.b[+]"), Value::Int(2), &semantics).expect("appends");
    apply_set(&mut root, &key("a.b[2]"), Value::Int(3), &semantics).expect("appends at end");
    apply_set(&mut root, &key("a.b[-1]"), Value::Int(30), &semantics).expect("replaces last");
    apply_set(&mut root, &key("a.b.0"), Value::Int(10), &semantics).expect("replaces by name");
    apply_set(&mut root, &key("e[0].f"), Value::Bool(true), &semantics)
        .expect("creates list of maps");
    assert_eq!(
        root,
        plain(json!({"a": {"b": [10, 2, 30], "c": {"d": "x"}}, "e": [{"f": true}]}))
    );
}

#[test]
fn set_rejects_paths_through_scalars_and_patterns() {
    let semantics = MapSemantics::LAST_WINS;
    let mut root = plain(json!({"a": 1, "l": [1]}));
    let through =
        apply_set(&mut root, &key("a.b"), Value::Int(2), &semantics).expect_err("a is an int");
    assert_eq!(through.code, ErrorCode::KeyConflict);
    assert_eq!(through.found.as_deref(), Some("int"));
    let pattern =
        apply_set(&mut root, &key("l[*]"), Value::Int(2), &semantics).expect_err("wildcard");
    assert_eq!(pattern.found.as_deref(), Some("pattern"));
    let beyond =
        apply_set(&mut root, &key("l[5]"), Value::Int(2), &semantics).expect_err("beyond end");
    assert_eq!(beyond.code, ErrorCode::KeyNotFound);
    let named =
        apply_set(&mut root, &key("l.x"), Value::Int(2), &semantics).expect_err("name on list");
    assert_eq!(named.code, ErrorCode::KeyConflict);
}

#[test]
fn set_addresses_documents_of_a_stream() {
    let semantics = MapSemantics::LAST_WINS;
    let mut documents = plain(json!([{"kind": "A"}, {"kind": "B"}]));
    apply_set(&mut documents, &key("#1:kind"), text("C"), &semantics).expect("second document");
    assert_eq!(documents, plain(json!([{"kind": "A"}, {"kind": "C"}])));
    let all = apply_set(&mut documents, &key("#*:kind"), text("D"), &semantics)
        .expect_err("every document");
    assert_eq!(all.code, ErrorCode::KeyConflict);
    let missing = apply_set(&mut documents, &key("#4:kind"), text("D"), &semantics)
        .expect_err("no fifth document");
    assert_eq!(missing.code, ErrorCode::KeyNotFound);
}

#[test]
fn flat_semantics_sets_dotted_member() {
    let mut root = plain(json!({}));
    apply_set(
        &mut root,
        &key("server.port"),
        Value::Int(80),
        &FlatSemantics,
    )
    .expect("sets");
    assert_eq!(root, plain(json!({"server.port": 80})));
}

#[test]
fn remove_deletes_members_and_elements() {
    let semantics = MapSemantics::LAST_WINS;
    let mut root = plain(json!({"a": 1, "b": [1, 2, 3], "c": 3}));
    apply_remove(&mut root, &key("a"), &semantics).expect("removes member");
    apply_remove(&mut root, &key("b[-1]"), &semantics).expect("removes last element");
    apply_remove(&mut root, &key("b.0"), &semantics).expect("removes element by name");
    assert_eq!(root, plain(json!({"b": [2], "c": 3})));
    let missing = apply_remove(&mut root, &key("x"), &semantics).expect_err("no member x");
    assert_eq!(missing.code, ErrorCode::KeyNotFound);
    let deep = apply_remove(&mut root, &key("x.y"), &semantics).expect_err("no member x");
    assert_eq!(deep.code, ErrorCode::KeyNotFound);
    let through = apply_remove(&mut root, &key("c.d"), &semantics).expect_err("c is an int");
    assert_eq!(through.code, ErrorCode::KeyConflict);
}

#[test]
fn removing_the_document_root_leaves_null() {
    let mut root = plain(json!({"a": 1}));
    apply_remove(&mut root, &KeyPath::root(), &MapSemantics::LAST_WINS).expect("removes root");
    assert_eq!(root, Value::Null);
}
