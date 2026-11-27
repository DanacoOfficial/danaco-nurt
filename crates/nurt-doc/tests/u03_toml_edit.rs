// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u03_toml_edit.rs
// Responsibility:   Changes to TOML documents through the API of `TomlDocument`: new keys and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Changes to TOML documents through the API of `TomlDocument`: new keys and
//! tables, replacement of maps and lists, arrays of tables, removals and the
//! refused changes with their construct names.
// Helpers outside `#[test]` functions fail the test on unexpected errors.
#![allow(clippy::unwrap_used, clippy::panic)]

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::{Date, DateKind, Fields, Value};
use nurt_doc::formats::toml::TomlDocument;

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn list(values: Vec<Value>) -> Value {
    Value::List(values)
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<Fields>(),
    )
}

fn parse(content: &str) -> TomlDocument {
    TomlDocument::parse(content).unwrap()
}

/// Text after setting `value` at `path` with `create`.
fn set(content: &str, path: &str, value: &Value) -> String {
    let mut doc = parse(content);
    doc.set(&key(path), value, true)
        .unwrap_or_else(|error| panic!("{path}: {error:?}"));
    doc.text().to_owned()
}

fn set_error(content: &str, path: &str, value: &Value, create: bool) -> Error {
    let mut doc = parse(content);
    let error = doc.set(&key(path), value, create).unwrap_err();
    assert_eq!(doc.text(), content, "a refused change keeps the text");
    error
}

fn remove(content: &str, path: &str) -> String {
    let mut doc = parse(content);
    doc.remove(&key(path)).unwrap();
    doc.text().to_owned()
}

fn construct(error: &Error) -> &str {
    assert_eq!(error.code(), ErrorCode::UnsupportedConstruct);
    error.found.as_deref().unwrap()
}

fn value_of(content: &str, path: &str) -> Value {
    parse(content).get(&key(path)).unwrap().remove(0).value
}

#[test]
fn new_key_goes_to_the_end_of_a_table_with_a_header() {
    let out = set("[a]\nx = 1\n\n[b]\ny = 2\n", "a.z", &Value::Int(3));
    assert_eq!(out, "[a]\nx = 1\nz = 3\n\n[b]\ny = 2\n");
}

#[test]
fn new_key_of_the_root_goes_before_the_first_header() {
    let out = set("a = 1\n\n[t]\nx = 1\n", "b", &Value::Bool(true));
    assert_eq!(out, "a = 1\nb = true\n\n[t]\nx = 1\n");
}

#[test]
fn new_key_in_an_empty_table_follows_its_header() {
    let out = set("[a]\n\n[b]\ny = 2\n", "a.x", &Value::Int(1));
    assert!(out.starts_with("[a]\nx = 1\n"), "{out}");
}

#[test]
fn new_key_in_a_dotted_table_is_a_dotted_key() {
    let out = set("a.b = 1\n\n[t]\nx = 1\n", "a.c", &Value::Int(2));
    assert_eq!(out, "a.b = 1\na.c = 2\n\n[t]\nx = 1\n");
}

#[test]
fn new_dotted_key_below_a_dotted_table_keeps_the_prefix() {
    let out = set("a.b.c = 1\n", "a.b.d", &Value::Int(2));
    assert_eq!(out, "a.b.c = 1\na.b.d = 2\n");
}

#[test]
fn new_table_is_written_as_a_header_at_the_end() {
    let out = set(
        "[a]\nx = 1\n",
        "b",
        &map(&[("y", Value::Int(2)), ("z", text("s"))]),
    );
    assert_eq!(out, "[a]\nx = 1\n\n[b]\ny = 2\nz = \"s\"\n");
}

#[test]
fn new_nested_table_key_creates_a_header_with_the_path() {
    let out = set("[a]\nx = 1\n", "b.c.d", &Value::Int(2));
    assert_eq!(out, "[a]\nx = 1\n\n[b.c]\nd = 2\n");
}

#[test]
fn new_table_after_a_document_without_final_line_break() {
    let out = set("[a]\nx = 1", "b", &map(&[("y", Value::Int(2))]));
    assert_eq!(out, "[a]\nx = 1\n\n[b]\ny = 2\n");
}

#[test]
fn new_key_keeps_crlf_line_breaks() {
    let out = set("[a]\r\nx = 1\r\n", "a.y", &Value::Int(2));
    assert_eq!(out, "[a]\r\nx = 1\r\ny = 2\r\n");
}

#[test]
fn new_key_copies_the_layout_of_the_last_pair() {
    let out = set("[a]\n  x   =   1\n", "a.y", &Value::Int(2));
    assert_eq!(out, "[a]\n  x   =   1\n  y   =   2\n");
}

#[test]
fn new_key_in_an_inline_table_stays_inline() {
    let out = set("t = { a = 1 }\n", "t.b", &Value::Int(2));
    assert_eq!(
        parse(&out).to_value(),
        map(&[("t", map(&[("a", Value::Int(1)), ("b", Value::Int(2))]))])
    );
    assert!(out.starts_with("t = {"), "{out}");
}

#[test]
fn new_key_below_an_implicit_table_creates_a_header_for_it() {
    let out = set("[a.b]\nx = 1\n", "a.y", &Value::Int(2));
    assert_eq!(
        parse(&out).to_value(),
        map(&[(
            "a",
            map(&[("b", map(&[("x", Value::Int(1))])), ("y", Value::Int(2))])
        )])
    );
}

#[test]
fn key_below_a_value_is_a_key_conflict() {
    let error = set_error("a = 1\n", "a.b", &Value::Int(1), true);
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

#[test]
fn new_key_addressed_with_an_index_after_the_existing_part_is_a_key_conflict() {
    let error = set_error("[a]\nx = 1\n", "a.n[0].k", &Value::Int(1), true);
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

#[test]
fn missing_key_without_create_is_reported() {
    let error = set_error("a = 1\n", "b", &Value::Int(1), false);
    assert_eq!(error.code(), ErrorCode::KeyNotFound);
}

#[test]
fn replacing_a_scalar_keeps_the_string_style() {
    let out = set("a = 'old'\nb = \"\"\"\nold\"\"\"\n", "a", &text("new"));
    assert_eq!(out, "a = 'new'\nb = \"\"\"\nold\"\"\"\n");
    let out = set("b = \"\"\"\nold\"\"\"\n", "b", &text("line 1\nline 2"));
    assert_eq!(out, "b = \"\"\"\nline 1\nline 2\"\"\"\n");
}

#[test]
fn replacing_a_literal_string_falls_back_to_basic_when_it_does_not_fit() {
    let out = set("a = 'old'\n", "a", &text("it's"));
    assert_eq!(out, "a = \"it's\"\n");
}

#[test]
fn control_characters_are_escaped() {
    let out = set(
        "a = 1\n",
        "a",
        &text("tab\t nl\n cr\r bell\u{7} nul\u{0} q\" bs\\ \u{8}\u{c}"),
    );
    assert_eq!(
        parse(&out).to_value(),
        map(&[(
            "a",
            text("tab\t nl\n cr\r bell\u{7} nul\u{0} q\" bs\\ \u{8}\u{c}")
        )])
    );
    assert!(out.contains("\\u0007") && out.contains("\\u0000"), "{out}");
}

#[test]
fn key_names_are_quoted_when_they_are_not_bare() {
    let out = set(
        "a = 1\n",
        "b",
        &map(&[
            ("with space", Value::Int(1)),
            ("plain-1_x", Value::Int(2)),
            ("quo\"te", Value::Int(3)),
            ("", Value::Int(4)),
        ]),
    );
    assert!(out.contains("\"with space\" = 1"), "{out}");
    assert!(out.contains("plain-1_x = 2"), "{out}");
    assert!(out.contains("\"quo\\\"te\" = 3"), "{out}");
    assert!(out.contains("\"\" = 4"), "{out}");
}

#[test]
fn dates_numbers_and_text_forms_are_written_by_value_type() {
    let date = |kind, text: &str| {
        Value::Date(Date {
            kind,
            text: text.to_owned(),
        })
    };
    let values = map(&[
        (
            "odt",
            date(DateKind::OffsetDateTime, "1979-05-27T07:32:00Z"),
        ),
        ("ldt", date(DateKind::LocalDateTime, "1979-05-27T07:32:00")),
        ("ld", date(DateKind::LocalDate, "1979-05-27")),
        ("lt", date(DateKind::LocalTime, "07:32:00")),
        ("inf", Value::Number(f64::INFINITY)),
        ("nan", Value::Number(f64::NAN)),
        ("neg", Value::Number(-1.5)),
        ("whole", Value::Number(2.0)),
        ("key", Value::Key("k".to_owned())),
        ("big", Value::BigInt("42".to_owned())),
        ("dec", Value::Decimal("0.5".to_owned())),
    ]);
    let out = set("a = 1\n", "t", &values);
    let read = parse(&out).to_value();
    let Value::Map(root) = read else { panic!() };
    let Value::Map(t) = &root["t"] else { panic!() };
    assert_eq!(t["inf"], Value::Number(f64::INFINITY));
    assert!(matches!(t["nan"], Value::Number(n) if n.is_nan()));
    assert_eq!(t["whole"], Value::Number(2.0));
    assert_eq!(t["key"], text("k"));
    assert_eq!(t["big"], Value::Int(42));
    assert_eq!(t["dec"], Value::Number(0.5));
    assert_eq!(t["ld"], date(DateKind::LocalDate, "1979-05-27"));
    assert_eq!(t["lt"], date(DateKind::LocalTime, "07:32:00"));
}

#[test]
fn values_without_a_toml_form_are_refused_with_their_construct() {
    let doc = "a = 1\n";
    assert_eq!(
        construct(&set_error(doc, "a", &Value::Null, false)),
        "toml.null"
    );
    let huge = Value::BigInt("99999999999999999999".to_owned());
    assert_eq!(
        construct(&set_error(doc, "a", &huge, false)),
        "toml.value_type"
    );
    let decimal = Value::Decimal("x".to_owned());
    assert_eq!(
        construct(&set_error(doc, "a", &decimal, false)),
        "toml.value_type"
    );
    let nested = list(vec![Value::Null]);
    assert_eq!(construct(&set_error(doc, "a", &nested, false)), "toml.null");
    let fields = map(&[("n", Value::Null)]);
    assert_eq!(construct(&set_error(doc, "a", &fields, false)), "toml.null");
    let range = Value::Bytes(vec![1].into());
    assert_eq!(
        construct(&set_error(doc, "a", &range, false)),
        "toml.value_type"
    );
}

#[test]
fn inline_table_is_replaced_member_by_member() {
    let out = set(
        "t = { a = 1, b = 2 }\nu = 0\n",
        "t",
        &map(&[("a", Value::Int(10)), ("c", Value::Int(3))]),
    );
    assert_eq!(
        parse(&out).to_value(),
        map(&[
            ("t", map(&[("a", Value::Int(10)), ("c", Value::Int(3))])),
            ("u", Value::Int(0))
        ])
    );
}

#[test]
fn table_with_a_header_is_replaced_member_by_member() {
    let out = set(
        "[t]\na = 1\nb = 2\n\n[u]\nx = 1\n",
        "t",
        &map(&[("a", Value::Int(1)), ("c", Value::Int(3))]),
    );
    assert_eq!(out, "[t]\na = 1\nc = 3\n\n[u]\nx = 1\n");
}

#[test]
fn array_is_replaced_element_by_element() {
    let content = "a = [1, 2, 3]\nb = 1\n";
    let grown = set(
        content,
        "a",
        &list(vec![
            Value::Int(1),
            Value::Int(5),
            Value::Int(3),
            Value::Int(4),
        ]),
    );
    assert_eq!(grown, "a = [1, 5, 3, 4]\nb = 1\n");
    let shrunk = set(content, "a", &list(vec![Value::Int(1)]));
    assert_eq!(shrunk, "a = [1]\nb = 1\n");
    let emptied = set(content, "a", &list(vec![]));
    assert_eq!(
        parse(&emptied).to_value(),
        map(&[("a", list(vec![])), ("b", Value::Int(1))])
    );
}

#[test]
fn multi_line_array_gains_an_element_on_its_own_line() {
    let content = "a = [\n  1,\n  2,\n]\n";
    let out = set(content, "a[+]", &Value::Int(3));
    assert_eq!(out, "a = [\n  1,\n  2,\n  3,\n]\n");
}

#[test]
fn append_adds_to_an_array_and_creates_a_missing_one() {
    assert_eq!(set("a = [1]\n", "a[+]", &Value::Int(2)), "a = [1, 2]\n");
    assert_eq!(set("a = [1]\n", "a[1]", &Value::Int(2)), "a = [1, 2]\n");
    let created = set("x = 1\n", "b[+]", &Value::Int(2));
    assert_eq!(
        parse(&created).to_value(),
        map(&[("x", Value::Int(1)), ("b", list(vec![Value::Int(2)]))])
    );
}

#[test]
fn append_to_a_value_that_is_not_an_array_is_a_key_conflict() {
    let error = set_error("a = 1\n", "a[+]", &Value::Int(2), true);
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

const TABLES: &str = "[[p]]\nn = 1\n\n[[p]]\nn = 2\n\n[q]\nx = 1\n";

#[test]
fn array_of_tables_gains_a_header() {
    let out = set(TABLES, "p[+]", &map(&[("n", Value::Int(3))]));
    assert_eq!(
        out,
        "[[p]]\nn = 1\n\n[[p]]\nn = 2\n\n[[p]]\nn = 3\n\n[q]\nx = 1\n"
    );
    assert_eq!(set(TABLES, "p[2]", &map(&[("n", Value::Int(3))])), out);
}

#[test]
fn array_of_tables_at_the_end_of_the_document_gains_a_header() {
    let out = set("[[p]]\nn = 1\n", "p[+]", &map(&[("n", Value::Int(2))]));
    assert_eq!(out, "[[p]]\nn = 1\n\n[[p]]\nn = 2\n");
}

#[test]
fn array_of_tables_is_replaced_element_by_element() {
    let items = list(vec![
        map(&[("n", Value::Int(7))]),
        map(&[("n", Value::Int(8))]),
        map(&[("n", Value::Int(9))]),
    ]);
    let grown = set(TABLES, "p", &items);
    assert_eq!(
        parse(&grown).to_value(),
        map(&[("p", items.clone()), ("q", map(&[("x", Value::Int(1))]))])
    );
    let shrunk = set(TABLES, "p", &list(vec![map(&[("n", Value::Int(5))])]));
    assert_eq!(shrunk, "[[p]]\nn = 5\n\n[q]\nx = 1\n");
}

#[test]
fn array_of_tables_refuses_empty_lists_and_scalars() {
    let error = set_error(TABLES, "p", &list(vec![]), false);
    assert_eq!(construct(&error), "toml.empty_array_of_tables");
    let error = set_error(TABLES, "p", &list(vec![Value::Int(1)]), false);
    assert_eq!(construct(&error), "toml.table_to_value");
    let error = set_error(TABLES, "p", &Value::Int(1), false);
    assert_eq!(construct(&error), "toml.table_to_value");
    let error = set_error(TABLES, "p[+]", &Value::Int(1), true);
    assert_eq!(construct(&error), "toml.table_to_value");
}

#[test]
fn table_replaced_by_a_scalar_is_refused() {
    let error = set_error("[t]\nx = 1\n", "t", &text("s"), false);
    assert_eq!(construct(&error), "toml.table_to_value");
}

#[test]
fn table_below_an_array_of_tables_is_addressed_through_the_last_element() {
    let content = "[[p]]\nn = 1\n\n[[p]]\nn = 2\n";
    let out = set(content, "p[1].extra", &map(&[("k", Value::Int(1))]));
    assert_eq!(
        parse(&out).to_value(),
        map(&[(
            "p",
            list(vec![
                map(&[("n", Value::Int(1))]),
                map(&[
                    ("n", Value::Int(2)),
                    ("extra", map(&[("k", Value::Int(1))]))
                ]),
            ])
        )])
    );
    let first = set(content, "p[0].extra", &map(&[("k", Value::Int(1))]));
    assert_eq!(value_of(&first, "p[0].extra.k"), Value::Int(1));
}

#[test]
fn nested_array_of_tables_gains_a_header_below_its_parent() {
    let content = "[[p]]\nn = 1\n[[p.c]]\nm = 1\n";
    let out = set(content, "p[0].c[+]", &map(&[("m", Value::Int(2))]));
    assert_eq!(out, "[[p]]\nn = 1\n[[p.c]]\nm = 1\n\n[[p.c]]\nm = 2\n");
}

#[test]
fn removing_a_key_takes_its_line_and_comments_above() {
    let out = remove("[a]\n# about x\nx = 1\ny = 2\n", "a.x");
    assert_eq!(out, "[a]\ny = 2\n");
}

#[test]
fn removing_a_table_takes_its_header_and_pairs() {
    let out = remove("a = 0\n\n[t]\nx = 1\n\n[u]\ny = 1\n", "t");
    assert_eq!(
        parse(&out).to_value(),
        map(&[("a", Value::Int(0)), ("u", map(&[("y", Value::Int(1))]))])
    );
    assert!(!out.contains("[t]"), "{out}");
}

#[test]
fn removing_array_elements_and_tables_of_an_array() {
    assert_eq!(remove("a = [1, 2, 3]\n", "a[1]"), "a = [1, 3]\n");
    let out = remove(TABLES, "p[0]");
    assert_eq!(out, "[[p]]\nn = 2\n\n[q]\nx = 1\n");
    let all = remove(TABLES, "p");
    assert_eq!(
        parse(&all).to_value(),
        map(&[("q", map(&[("x", Value::Int(1))]))])
    );
}

#[test]
fn removing_a_member_of_an_inline_table() {
    let out = remove("t = { a = 1, b = 2 }\n", "t.a");
    assert_eq!(
        parse(&out).to_value(),
        map(&[("t", map(&[("b", Value::Int(2))]))])
    );
}

#[test]
fn removing_the_root_or_a_missing_key_fails() {
    let mut doc = parse("a = 1\n");
    assert_eq!(
        doc.remove(&KeyPath::parse("$").unwrap())
            .unwrap_err()
            .code(),
        ErrorCode::KeyConflict
    );
    assert_eq!(
        doc.remove(&key("missing")).unwrap_err().code(),
        ErrorCode::KeyNotFound
    );
}

#[test]
fn setting_the_current_value_changes_nothing() {
    let mut doc = parse("a = 0x10\n");
    assert_eq!(doc.set(&key("a"), &Value::Int(16), false).unwrap(), 1);
    assert_eq!(doc.text(), "a = 0x10\n");
}

#[test]
fn get_reads_dates_numbers_and_tables_with_their_positions() {
    assert_eq!(value_of("a = 0x1F\n", "a"), Value::Int(31));
    assert_eq!(value_of("a = 1_000\n", "a"), Value::Int(1000));
    assert_eq!(
        value_of("d = 1979-05-27\n", "d"),
        Value::Date(Date {
            kind: DateKind::LocalDate,
            text: "1979-05-27".to_owned()
        })
    );
    let doc = parse("# about t\n[t]\nx = 1\n");
    let found = doc.get(&key("t")).unwrap();
    assert_eq!(found[0].value, map(&[("x", Value::Int(1))]));
    assert_eq!(
        doc.get(&key("missing")).unwrap_err().code(),
        ErrorCode::KeyNotFound
    );
}

#[test]
fn malformed_documents_are_rejected_as_parse_errors() {
    for content in ["a = \n", "[a\n", "a = 1\na = 2\n", "[a]\n[a]\n"] {
        let error = TomlDocument::parse(content).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse, "{content:?}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
