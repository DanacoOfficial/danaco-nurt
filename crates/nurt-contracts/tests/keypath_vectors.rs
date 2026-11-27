// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/keypath_vectors.rs
// Responsibility:   Test vectors of the key path grammar: canonical forms, segments, errors.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test vectors of the key path grammar: canonical forms, segments, errors
//! and the parse, print, parse round trip.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::keypath::{
    CompareOp, DocSelector, FilterExpr, FilterLiteral, KeyPath, KeyPathErrorKind, KeySegment,
};

/// Valid key paths and their canonical form.
const CANONICAL: &[(&str, &str)] = &[
    ("scripts.test", "scripts.test"),
    ("deps[0].name", "deps[0].name"),
    ("a[*]", "a[*]"),
    ("a.*", "a[*]"),
    ("a.**", "a.**"),
    ("a[?name == \"x\"]", "a[?name == \"x\"]"),
    ("deps[+]", "deps[+]"),
    ("[\"a.b\"].c", "[\"a.b\"].c"),
    ("$.a.b", "a.b"),
    ("#1:spec.replicas", "#1:spec.replicas"),
    ("project.@version", "project.@version"),
    ("a[-1]", "a[-1]"),
    ("a", "a"),
    ("$", "$"),
    ("$['a']", "a"),
    ("$[\"a b\"]", "a b"),
    ("a b.c", "a b.c"),
    ("[' a']", "[\" a\"]"),
    ("a.0", "a.0"),
    ("a[ 0 ]", "a[0]"),
    ("a[007]", "a[7]"),
    ("a[-0]", "a[0]"),
    ("$..name", "**.name"),
    ("$..*", "**[*]"),
    ("$.a..b", "a.**.b"),
    ("$..[0]", "**[0]"),
    ("$.*", "[*]"),
    ("*", "[*]"),
    ("**", "**"),
    ("**.version", "**.version"),
    ("[*].name", "[*].name"),
    ("a[**]", "a.**"),
    ("#*:kind", "#*:kind"),
    ("#0:a", "a"),
    ("#12:a[1]", "#12:a[1]"),
    ("#1:$", "#1:$"),
    ("#0:$", "$"),
    ("#2:$.a", "#2:a"),
    ("$schema", "$schema"),
    ("[\"$\"]", "[\"$\"]"),
    ("definitions.$ref", "definitions.$ref"),
    ("#text", "#text"),
    ("#1", "#1"),
    ("[\"#1:x\"]", "[\"#1:x\"]"),
    ("a[\"#1:x\"]", "a.#1:x"),
    ("#1:#2:x", "#1:#2:x"),
    ("a[\"*\"]", "a[\"*\"]"),
    ("a[\"**\"]", "a[\"**\"]"),
    ("a[\"\"]", "a[\"\"]"),
    ("a['it\\'s']", "a[\"it's\"]"),
    ("a[\"line\\nbreak\"]", "a[\"line\\nbreak\"]"),
    ("a[\"tab\\there\"]", "a[\"tab\\there\"]"),
    ("a[\"\\u0041\"]", "a.A"),
    ("a[\"\\u{1F600}\"]", "a.\u{1F600}"),
    ("a[\"\\ud83d\\ude00\"]", "a.\u{1F600}"),
    ("a[\"\\u{7}\"]", "a[\"\\u{7}\"]"),
    ("a[\"x\\/y\"]", "a.x/y"),
    ("na\u{ef}ve.caf\u{e9}", "na\u{ef}ve.caf\u{e9}"),
    ("\u{65e5}\u{672c}.\u{8a9e}", "\u{65e5}\u{672c}.\u{8a9e}"),
    ("a.x\\y", "a.x\\y"),
    ("a.b c .d", "a[\"b c \"].d"),
    ("a[?@ == 1]", "a[?@ == 1]"),
    ("a[?@.name == 'x']", "a[?name == \"x\"]"),
    ("a[?name=='x'&&size>2]", "a[?name == \"x\" and size > 2]"),
    ("a[?a || b && c]", "a[?a or b and c]"),
    ("a[?(a || b) && c]", "a[?(a or b) and c]"),
    ("a[?!active]", "a[?not active]"),
    ("a[?not (x == 1)]", "a[?not (x == 1)]"),
    ("a[?!(x == 1)]", "a[?not (x == 1)]"),
    ("a[?not x == 1]", "a[?not (x == 1)]"),
    ("a[?x != null]", "a[?x != null]"),
    ("a[?v >= -1.5]", "a[?v >= -1.5]"),
    ("a[?v < 1e3]", "a[?v < 1000.0]"),
    ("a[?v <= 2.50]", "a[?v <= 2.5]"),
    ("a[?flag == true]", "a[?flag == true]"),
    ("a[?flag == false]", "a[?flag == false]"),
    ("a[?meta.version > 2]", "a[?meta.version > 2]"),
    ("a[?@[\"a b\"] == 1]", "a[?@[\"a b\"] == 1]"),
    ("a[?@['true'] == 1]", "a[?@[\"true\"] == 1]"),
    ("a[?x-y == 1]", "a[?x-y == 1]"),
    ("a[?a and b or c]", "a[?a and b or c]"),
    ("a[?a or (b or c)]", "a[?a or (b or c)]"),
    ("a[?(a or b) or c]", "a[?a or b or c]"),
    ("a[?a and (b and c)]", "a[?a and (b and c)]"),
    ("a[?not not a]", "a[?not not a]"),
    ("a[?(x == 1) == true]", "a[?(x == 1) == true]"),
    ("a[?name == \"]\"]", "a[?name == \"]\"]"),
    (
        "a[?n == 9223372036854775807]",
        "a[?n == 9223372036854775807]",
    ),
    (
        "a[?n == 9223372036854775808]",
        "a[?n == 9.223372036854776e18]",
    ),
    (
        "a[?n == -9223372036854775808]",
        "a[?n == -9223372036854775808]",
    ),
    ("a[?'x' == name]", "a[?\"x\" == name]"),
    ("a[? name  ==  \"x\" ]", "a[?name == \"x\"]"),
    ("a[?active]", "a[?active]"),
    ("a[*][?x == 1][0]", "a[*][?x == 1][0]"),
    ("a[0][1][2]", "a[0][1][2]"),
    ("a.b.c.d.e", "a.b.c.d.e"),
    ("$[0]", "[0]"),
    ("[0]", "[0]"),
    ("[-1].x", "[-1].x"),
    ("$.a[+]", "a[+]"),
    ("[+]", "[+]"),
    ("a.-1", "a.-1"),
    ("a.@context", "a.@context"),
];

/// Invalid key paths: kind, byte offset and correction.
const ERRORS: &[(&str, KeyPathErrorKind, usize, Option<&str>)] = &[
    ("", KeyPathErrorKind::Empty, 0, None),
    ("a.", KeyPathErrorKind::TrailingDot, 1, Some("a")),
    (".a", KeyPathErrorKind::LeadingDot, 0, Some("a")),
    ("a..b", KeyPathErrorKind::EmptyName, 2, Some("a.**.b")),
    ("a.[0]", KeyPathErrorKind::DotBeforeBracket, 1, Some("a[0]")),
    ("a[", KeyPathErrorKind::UnexpectedEnd, 1, None),
    ("a[?]", KeyPathErrorKind::EmptyFilter, 3, None),
    ("a[]", KeyPathErrorKind::EmptyBracket, 2, Some("a[*]")),
    ("a[b]", KeyPathErrorKind::UnquotedName, 2, Some("a[\"b\"]")),
    ("a[0", KeyPathErrorKind::UnexpectedEnd, 3, None),
    ("a[\"x", KeyPathErrorKind::UnterminatedString, 2, None),
    ("a[\"\\q\"]", KeyPathErrorKind::InvalidEscape, 3, None),
    (
        "a[99999999999999999999]",
        KeyPathErrorKind::IndexOutOfRange,
        2,
        None,
    ),
    ("a[1:3]", KeyPathErrorKind::Unsupported, 3, None),
    ("a[0,1]", KeyPathErrorKind::Unsupported, 3, None),
    ("a[:3]", KeyPathErrorKind::Unsupported, 2, None),
    ("a['x','y']", KeyPathErrorKind::Unsupported, 5, None),
    ("a[+].b", KeyPathErrorKind::MisplacedAppend, 5, None),
    ("a[+][0]", KeyPathErrorKind::MisplacedAppend, 4, None),
    ("#1:", KeyPathErrorKind::MissingPath, 3, Some("#1:$")),
    (
        "it's",
        KeyPathErrorKind::CharacterRequiresQuoting,
        2,
        Some("[\"it's\"]"),
    ),
    (
        "a]",
        KeyPathErrorKind::CharacterRequiresQuoting,
        1,
        Some("[\"a]\"]"),
    ),
    (
        "a.b\"c",
        KeyPathErrorKind::CharacterRequiresQuoting,
        3,
        Some("a[\"b\\\"c\"]"),
    ),
    (
        "a[0]b",
        KeyPathErrorKind::UnexpectedCharacter,
        4,
        Some("a[0].b"),
    ),
    (
        "a[?x = 1]",
        KeyPathErrorKind::InvalidFilter,
        5,
        Some("a[?x == 1]"),
    ),
    ("a[?x == ]", KeyPathErrorKind::InvalidFilter, 8, None),
    ("a[?x == 1 == 2]", KeyPathErrorKind::InvalidFilter, 10, None),
    ("a[?length(@) > 1]", KeyPathErrorKind::Unsupported, 9, None),
    ("a[?@.x == $.y]", KeyPathErrorKind::Unsupported, 10, None),
    (
        "a[?(x == 1]",
        KeyPathErrorKind::UnexpectedCharacter,
        10,
        None,
    ),
    ("a[?x == 1e400]", KeyPathErrorKind::InvalidNumber, 8, None),
    ("a[?x == 1e]", KeyPathErrorKind::InvalidNumber, 10, None),
    ("a[?x == -]", KeyPathErrorKind::InvalidNumber, 8, None),
    ("a[?and]", KeyPathErrorKind::InvalidFilter, 3, None),
    ("a[?@.]", KeyPathErrorKind::InvalidFilter, 5, None),
    (
        "a[?@.items[0] == 1]",
        KeyPathErrorKind::InvalidFilter,
        11,
        None,
    ),
    ("a[-]", KeyPathErrorKind::UnexpectedCharacter, 3, None),
    ("a[+1]", KeyPathErrorKind::UnexpectedCharacter, 3, None),
    ("$.a.", KeyPathErrorKind::TrailingDot, 3, Some("$.a")),
    ("$..", KeyPathErrorKind::EmptyName, 3, None),
    ("$a.", KeyPathErrorKind::TrailingDot, 2, Some("$a")),
    (
        "a[\"\\u{110000}\"]",
        KeyPathErrorKind::InvalidEscape,
        3,
        None,
    ),
    ("a[\"\\ud800\"]", KeyPathErrorKind::InvalidEscape, 3, None),
    ("a[\"\\u12\"]", KeyPathErrorKind::InvalidEscape, 3, None),
    ("#99999999999:a", KeyPathErrorKind::IndexOutOfRange, 0, None),
    ("a[?x == 1", KeyPathErrorKind::UnexpectedEnd, 9, None),
];

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap_or_else(|e| panic!("`{text}` should parse: {e}"))
}

fn name(text: &str) -> KeySegment {
    KeySegment::Key(text.to_owned())
}

fn field(path: &[&str]) -> Box<FilterExpr> {
    Box::new(FilterExpr::Field {
        path: path.iter().map(|s| (*s).to_owned()).collect(),
    })
}

fn text(value: &str) -> Box<FilterExpr> {
    Box::new(FilterExpr::Literal {
        value: FilterLiteral::Text(value.to_owned()),
    })
}

#[test]
fn there_are_enough_vectors() {
    assert!(CANONICAL.len() + ERRORS.len() >= 60);
}

#[test]
fn canonical_forms() {
    for (input, canonical) in CANONICAL {
        let parsed = key(input);
        assert_eq!(
            parsed.to_string(),
            *canonical,
            "canonical form of `{input}`"
        );
    }
}

#[test]
fn round_trip_of_every_vector() {
    for (input, canonical) in CANONICAL {
        let parsed = key(input);
        let printed = parsed.to_string();
        let reparsed = key(&printed);
        assert_eq!(
            reparsed, parsed,
            "`{input}` changed after printing as `{printed}`"
        );
        assert_eq!(
            reparsed.to_string(),
            *canonical,
            "printing `{input}` is not idempotent"
        );
    }
}

#[test]
fn error_vectors() {
    for (input, kind, offset, fix) in ERRORS {
        let error = KeyPath::parse(input).expect_err(input);
        assert_eq!(error.kind, *kind, "kind for `{input}`: {error}");
        assert_eq!(error.offset, *offset, "offset for `{input}`: {error}");
        assert_eq!(error.fix.as_deref(), *fix, "fix for `{input}`: {error}");
        assert_eq!(error.code(), "E_KEY_SYNTAX");
        if let Some(fix) = fix
            && !fix.is_empty()
        {
            assert!(
                KeyPath::parse(fix).is_ok(),
                "fix `{fix}` for `{input}` must parse"
            );
        }
    }
}

#[test]
fn reference_segments() {
    assert_eq!(
        key("scripts.test").segments(),
        &[name("scripts"), name("test")]
    );
    assert_eq!(
        key("deps[0].name").segments(),
        &[name("deps"), KeySegment::Index(0), name("name")]
    );
    assert_eq!(key("a[*]").segments(), &[name("a"), KeySegment::Wildcard]);
    assert_eq!(key("a.**").segments(), &[name("a"), KeySegment::Recursive]);
    assert_eq!(
        key("deps[+]").segments(),
        &[name("deps"), KeySegment::Append]
    );
    assert_eq!(key("[\"a.b\"].c").segments(), &[name("a.b"), name("c")]);
    assert_eq!(key("$.a.b").segments(), &[name("a"), name("b")]);
    assert_eq!(key("a[-1]").segments(), &[name("a"), KeySegment::Index(-1)]);
    assert_eq!(
        key("project.@version").segments(),
        &[name("project"), name("@version")]
    );
    assert_eq!(key("a.0").segments(), &[name("a"), name("0")]);
    assert_eq!(key("a[0]").segments(), &[name("a"), KeySegment::Index(0)]);
    assert_eq!(key("$..x").segments(), &[KeySegment::Recursive, name("x")]);
    assert!(key("$").segments().is_empty());
}

#[test]
fn reference_filter() {
    let parsed = key("a[?name == \"x\"]");
    let expected = FilterExpr::Compare {
        op: CompareOp::Eq,
        left: field(&["name"]),
        right: text("x"),
    };
    assert_eq!(
        parsed.segments(),
        &[name("a"), KeySegment::Filter(expected)]
    );

    let parsed = key("a[?@ != 'x' && !(n > 2)]");
    let expected = FilterExpr::And {
        left: Box::new(FilterExpr::Compare {
            op: CompareOp::Ne,
            left: field(&[]),
            right: text("x"),
        }),
        right: Box::new(FilterExpr::Not {
            operand: Box::new(FilterExpr::Compare {
                op: CompareOp::Gt,
                left: field(&["n"]),
                right: Box::new(FilterExpr::Literal {
                    value: FilterLiteral::Int(2),
                }),
            }),
        }),
    };
    assert_eq!(parsed.segments()[1], KeySegment::Filter(expected));
}

#[test]
fn document_selectors() {
    assert_eq!(
        key("#1:spec.replicas").document(),
        Some(DocSelector::Index(1))
    );
    assert_eq!(key("#*:kind").document(), Some(DocSelector::All));
    assert_eq!(key("#0:kind").document(), None);
    assert_eq!(key("#0:kind"), key("kind"));
    assert_eq!(key("#text").document(), None);
    assert_eq!(
        KeyPath::new(Some(DocSelector::Index(0)), vec![name("a")]).unwrap(),
        key("a")
    );
}

#[test]
fn error_columns_count_characters() {
    let error = KeyPath::parse("na\u{ef}ve.").unwrap_err();
    assert_eq!(error.kind, KeyPathErrorKind::TrailingDot);
    assert_eq!(error.offset, 6);
    assert_eq!(error.column, 6);
    assert!(error.to_string().starts_with("E_KEY_SYNTAX at column 6"));
}

#[test]
fn deep_filters_are_rejected() {
    let deep = format!("a[?{}x{}]", "(".repeat(80), ")".repeat(80));
    assert_eq!(
        KeyPath::parse(&deep).unwrap_err().kind,
        KeyPathErrorKind::InvalidFilter
    );
    let shallow = format!("a[?{}x{}]", "(".repeat(20), ")".repeat(20));
    assert_eq!(key(&shallow).to_string(), "a[?x]");
    let negations = format!("a[?{}x]", "!".repeat(80));
    assert_eq!(
        KeyPath::parse(&negations).unwrap_err().kind,
        KeyPathErrorKind::InvalidFilter
    );
}

#[test]
fn builder_rules() {
    let mut built = KeyPath::root();
    assert!(built.is_root());
    built.push(name("deps")).unwrap();
    built.push(KeySegment::Append).unwrap();
    assert!(built.ends_with_append());
    assert_eq!(
        built.push(name("x")).unwrap_err().kind,
        KeyPathErrorKind::MisplacedAppend
    );
    assert_eq!(built.to_string(), "deps[+]");
    assert_eq!(built.parent().unwrap().to_string(), "deps");
    assert_eq!(KeyPath::root().parent(), None);

    let not_a_number = FilterExpr::Literal {
        value: FilterLiteral::Number(f64::NAN),
    };
    assert_eq!(
        KeyPath::new(None, vec![KeySegment::Filter(not_a_number)])
            .unwrap_err()
            .kind,
        KeyPathErrorKind::InvalidNumber
    );
}

#[test]
fn concreteness() {
    assert!(key("a.b[0]").is_concrete());
    assert!(key("$").is_concrete());
    assert!(!key("a[*]").is_concrete());
    assert!(!key("a.**").is_concrete());
    assert!(!key("a[?x]").is_concrete());
    assert!(!key("a[+]").is_concrete());
    assert!(!key("#*:a").is_concrete());
    assert!(key("#1:a").is_concrete());
}

#[test]
fn parse_traits_agree() {
    let parsed: KeyPath = "a.b".parse().unwrap();
    assert_eq!(parsed, KeyPath::try_from("$.a.b").unwrap());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
