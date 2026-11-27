// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u01_keypath.rs
// Responsibility:   Key path vectors of `tests/d5/data/u01/keypath.toml` and the round trip.
// Dependencies:     KeyPathErrorKind, nurt_contracts, proptest, serde_json, toml_edit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Key path vectors of `tests/d5/data/u01/keypath.toml` and the round trip
//! of the canonical printer.
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use std::path::PathBuf;

use nurt_contracts::ast::{BinOp, Expr, KeySeg, UnOp, Wildcard};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{
    CompareOp, DocSelector, FilterExpr, FilterLiteral, KeyPath, KeyPathErrorKind, KeySegment,
    MAX_FILTER_DEPTH,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use serde_json::json;
use toml_edit::{DocumentMut, InlineTable};

/// Smallest number of vectors in the file.
const MIN_VECTORS: usize = 200;
/// Cases of the printer round trip property.
const PROPERTY_CASES: u32 = 2_000;
/// Seed of the property, fixed so that runs are reproducible.
const PROPERTY_SEED: u64 = 0x6E75_7274;

fn vectors() -> DocumentMut {
    let path = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/data/u01/keypath.toml");
    std::fs::read_to_string(&path)
        .expect("vector file is readable")
        .parse()
        .expect("vector file is valid TOML")
}

fn entries<'d>(document: &'d DocumentMut, name: &str) -> Vec<&'d InlineTable> {
    document[name]
        .as_array()
        .expect("array of vectors")
        .iter()
        .map(|entry| entry.as_inline_table().expect("vector is an inline table"))
        .collect()
}

fn field(entry: &InlineTable, name: &str) -> Option<String> {
    entry
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

#[test]
fn vector_file_has_at_least_two_hundred_vectors() {
    let document = vectors();
    let total = entries(&document, "valid").len() + entries(&document, "invalid").len();
    assert!(total >= MIN_VECTORS, "{total} vectors");
}

#[test]
fn valid_vectors_print_canonically_and_parse_back() {
    let document = vectors();
    let mut failures = Vec::new();
    for entry in entries(&document, "valid") {
        let input = field(entry, "input").expect("input");
        let canonical = field(entry, "canonical").expect("canonical");
        let key = match KeyPath::parse(&input) {
            Ok(key) => key,
            Err(error) => {
                failures.push(format!("{input:?} fails: {error}"));
                continue;
            }
        };
        let printed = key.to_string();
        if printed != canonical {
            failures.push(format!(
                "{input:?} prints {printed:?}, expected {canonical:?}"
            ));
            continue;
        }
        match KeyPath::parse(&printed) {
            Ok(again) if again == key => {}
            Ok(again) => {
                failures.push(format!("{printed:?} parses to {again:?}, expected {key:?}"))
            }
            Err(error) => failures.push(format!("canonical {printed:?} fails: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn invalid_vectors_fail_with_key_syntax_kind_and_fix() {
    let document = vectors();
    let mut failures = Vec::new();
    for entry in entries(&document, "invalid") {
        let input = field(entry, "input").expect("input");
        let error = match KeyPath::parse(&input) {
            Ok(key) => {
                failures.push(format!("{input:?} parses to {key}"));
                continue;
            }
            Err(error) => error,
        };
        assert_eq!(error.code(), "E_KEY_SYNTAX");
        if let Some(kind) = field(entry, "kind") {
            let actual = serde_json::to_value(error.kind).expect("kind serializes");
            if actual != serde_json::Value::String(kind.clone()) {
                failures.push(format!("{input:?} fails as {actual}, expected {kind}"));
            }
        }
        if let Some(fix) = field(entry, "fix")
            && error.fix.as_deref() != Some(fix.as_str())
        {
            failures.push(format!(
                "{input:?} suggests {:?}, expected {fix:?}",
                error.fix
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn key_syntax_error_reports_column_of_problem() {
    let error = KeyPath::parse("a.[0]").expect_err("dot before bracket");
    assert!(error.column >= 2, "column {}", error.column);
    assert!(error.to_string().starts_with("E_KEY_SYNTAX at column"));
}

#[test]
fn malformed_keys_fail_with_the_kind_of_the_problem() {
    use KeyPathErrorKind::{
        CharacterRequiresQuoting, EmptyName, IndexOutOfRange, InvalidEscape, InvalidFilter,
        InvalidNumber, UnexpectedCharacter, UnexpectedEnd, Unsupported,
    };
    let deep = format!("a[?{}x]", "!".repeat(MAX_FILTER_DEPTH + 1));
    let cases = [
        (deep.as_str(), InvalidFilter),
        ("a[?(x", UnexpectedEnd),
        ("#99999999999:a", IndexOutOfRange),
        ("$..", EmptyName),
        ("a[:2]", Unsupported),
        ("a[0,1]", Unsupported),
        ("a[0 x]", UnexpectedCharacter),
        ("$['a']b", UnexpectedCharacter),
        ("a[?x == 1 == 2]", InvalidFilter),
        ("a[?x = 1]", InvalidFilter),
        ("a[?$.x == 1]", Unsupported),
        ("a[?and]", InvalidFilter),
        ("a[?x. == 1]", InvalidFilter),
        ("a[?x[0] == 1]", InvalidFilter),
        ("a[?x == 1e]", InvalidNumber),
        ("a[?x == -]", InvalidNumber),
        ("a[?x == 1e999]", InvalidNumber),
        ("a[?f(x)]", Unsupported),
        (r#"a["\u{}"]"#, InvalidEscape),
        (r#"a["\u{110000}"]"#, InvalidEscape),
        (r#"a["\u{12"]"#, InvalidEscape),
        (r#"a["\u{1234567}"]"#, InvalidEscape),
        (r#"a["\uZZZZ"]"#, InvalidEscape),
        (r#"a["\ud83d"]"#, InvalidEscape),
        (r#"a["\ud83d\u0041"]"#, InvalidEscape),
        (r#"a["\udc00"]"#, InvalidEscape),
        (r#"a.b"c"#, CharacterRequiresQuoting),
    ];
    for (input, kind) in cases {
        let error = KeyPath::parse(input).expect_err(input);
        assert_eq!(error.kind, kind, "{input}");
    }
    let equals = KeyPath::parse("a[?x = 1]").expect_err("single equals sign");
    assert_eq!(equals.fix.as_deref(), Some("a[?x == 1]"));
    let quote = KeyPath::parse(r#"a.b"c"#).expect_err("quote in a name");
    assert_eq!(quote.fix.as_deref(), Some(r#"a["b\"c"]"#));
}

#[test]
fn escapes_recursion_and_filters_print_canonically() {
    let cases = [
        (r#"a["\'\/\r\b\f"]"#, r#"a["'/\r\u{8}\u{c}"]"#),
        (
            r#"a["\u00e9\u{1F600}\ud83d\ude00"]"#,
            "a.\u{e9}\u{1F600}\u{1F600}",
        ),
        ("a[?not not x]", "a[?not not x]"),
        (r#"a[?b["c d"] == 1]"#, r#"a[?b["c d"] == 1]"#),
        ("a[**]", "a.**"),
        ("$..[0]", "**[0]"),
        ("$..a", "**.a"),
        ("a[?x == 1e+5]", "a[?x == 100000.0]"),
    ];
    for (input, canonical) in cases {
        let key = KeyPath::parse(input).expect(input);
        assert_eq!(key.to_string(), canonical, "{input}");
        assert_eq!(KeyPath::parse(canonical).as_ref(), Ok(&key), "{canonical}");
    }
}

fn filter_field(name: &str) -> Box<FilterExpr> {
    Box::new(FilterExpr::Field {
        path: vec![name.to_owned()],
    })
}

fn number(value: f64) -> Box<FilterExpr> {
    Box::new(FilterExpr::Literal {
        value: FilterLiteral::Number(value),
    })
}

#[test]
fn key_path_values_check_segments_and_convert() {
    let root = KeyPath::root();
    assert!(root.is_root() && root.is_concrete());
    let concrete: KeyPath = "a[0].b".parse().expect("valid key path");
    assert!(concrete.is_concrete() && !concrete.is_root());
    assert!(!KeyPath::try_from("a[*]").expect("valid").is_concrete());
    assert!(!KeyPath::parse("#*:a").expect("valid").is_concrete());

    let mut appended = KeyPath::parse("a[+]").expect("valid key path");
    let error = appended
        .push(KeySegment::Key("b".to_owned()))
        .expect_err("segment after [+]");
    assert_eq!(error.kind, KeyPathErrorKind::MisplacedAppend);

    let infinite = FilterExpr::Not {
        operand: Box::new(FilterExpr::Or {
            left: Box::new(FilterExpr::Literal {
                value: FilterLiteral::Bool(true),
            }),
            right: Box::new(FilterExpr::And {
                left: Box::new(FilterExpr::Literal {
                    value: FilterLiteral::Null,
                }),
                right: Box::new(FilterExpr::Compare {
                    op: CompareOp::Gt,
                    left: filter_field("x"),
                    right: number(f64::INFINITY),
                }),
            }),
        }),
    };
    let error = KeyPath::new(None, vec![KeySegment::Filter(infinite)]).expect_err("infinity");
    assert_eq!(error.kind, KeyPathErrorKind::InvalidNumber);
}

#[test]
fn filtered_key_paths_serialize_as_text_and_errors_convert() {
    let finite = FilterExpr::Compare {
        op: CompareOp::Le,
        left: filter_field("x"),
        right: number(1.5),
    };
    assert_eq!(finite.to_string(), "x <= 1.5");
    let key = KeyPath::new(
        Some(DocSelector::Index(0)),
        vec![KeySegment::Key("a".to_owned()), KeySegment::Filter(finite)],
    )
    .expect("finite filter");
    assert_eq!(key.document(), None);
    assert_eq!(key.to_string(), "a[?x <= 1.5]");
    assert_eq!(
        serde_json::to_value(&key).expect("serializes"),
        json!("a[?x <= 1.5]")
    );
    let read: KeyPath = serde_json::from_value(json!("a[?x <= 1.5]")).expect("deserializes");
    assert_eq!(read, key);
    assert!(serde_json::from_value::<KeyPath>(json!("a.")).is_err());

    let error = KeyPath::parse("a.").expect_err("trailing dot");
    assert_eq!(
        error.to_string(),
        "E_KEY_SYNTAX at column 2: a key path does not end with `.` (did you mean `a`?)"
    );
    let converted = Error::from(error);
    assert_eq!(converted.code, ErrorCode::KeySyntax);
    assert_eq!(converted.details.get("key_column"), Some(&json!(2)));
    assert_eq!(converted.details.get("key_fix"), Some(&json!("a")));
}

fn ident(name: &str) -> Expr {
    Expr::Ident {
        v: name.to_owned(),
        span: None,
    }
}

fn int(value: i64) -> Expr {
    Expr::Int {
        v: value,
        span: None,
    }
}

fn bin(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::Bin {
        op,
        l: Box::new(left),
        r: Box::new(right),
        span: None,
    }
}

fn filter(expr: Expr) -> KeySeg {
    KeySeg::Filter {
        filter: Box::new(expr),
    }
}

#[test]
fn structured_keys_convert_to_key_paths() {
    let version = Expr::Field {
        name: "version".to_owned(),
        of: Some(Box::new(Expr::Field {
            name: "meta".to_owned(),
            of: None,
            span: None,
        })),
        span: None,
    };
    let condition = bin(
        BinOp::Or,
        bin(
            BinOp::And,
            bin(
                BinOp::Eq,
                ident("name"),
                Expr::Text {
                    v: "x".to_owned(),
                    span: None,
                },
            ),
            bin(BinOp::Ne, version, Expr::Null { span: None }),
        ),
        Expr::Un {
            op: UnOp::Not,
            x: Box::new(bin(BinOp::Lt, ident("n"), int(3))),
            span: None,
        },
    );
    let segments = [
        KeySeg::Name("deps".to_owned()),
        KeySeg::Index(-1),
        KeySeg::Wild {
            wild: Wildcard::Level,
        },
        KeySeg::Wild {
            wild: Wildcard::Deep,
        },
        filter(condition),
    ];
    let key = KeyPath::from_key_segs(&segments).expect("converts");
    assert_eq!(
        key.to_string(),
        r#"deps[-1][*].**[?name == "x" and meta.version != null or not (n < 3)]"#
    );

    let ranges = bin(
        BinOp::And,
        bin(BinOp::Le, ident("a"), Expr::Num { v: 1.5, span: None }),
        bin(
            BinOp::And,
            bin(
                BinOp::Gt,
                ident("b"),
                Expr::Bool {
                    v: true,
                    span: None,
                },
            ),
            bin(BinOp::Ge, ident("c"), int(0)),
        ),
    );
    let append = KeySeg::Wild {
        wild: Wildcard::Append,
    };
    let key = KeyPath::from_key_segs(&[KeySeg::Name("list".to_owned()), filter(ranges), append])
        .expect("converts");
    assert_eq!(
        key.to_string(),
        "list[?a <= 1.5 and (b > true and c >= 0)][+]"
    );
}

#[test]
fn structured_keys_outside_the_filter_subset_are_rejected() {
    use KeyPathErrorKind::{Empty, MisplacedAppend, Unsupported};
    let name = || KeySeg::Name("a".to_owned());
    let append = || KeySeg::Wild {
        wild: Wildcard::Append,
    };
    let field_of_text = Expr::Field {
        name: "x".to_owned(),
        of: Some(Box::new(Expr::Text {
            v: "t".to_owned(),
            span: None,
        })),
        span: None,
    };
    let regex = Expr::Regex {
        v: "a.*".to_owned(),
        flags: String::new(),
        span: None,
    };
    let cases = [
        (Vec::new(), Empty),
        (vec![append(), name()], MisplacedAppend),
        (vec![name(), filter(field_of_text)], Unsupported),
        (vec![name(), filter(regex)], Unsupported),
        (
            vec![name(), filter(bin(BinOp::Add, int(1), int(2)))],
            Unsupported,
        ),
    ];
    for (segments, kind) in cases {
        let error = KeyPath::from_key_segs(&segments).expect_err("rejected");
        assert_eq!(error.kind, kind, "{segments:?}");
    }
}

fn name() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-z][a-z0-9_-]{0,6}",
        2 => "[ -~]{1,6}",
        1 => "\\PC{1,4}",
    ]
}

fn segment() -> impl Strategy<Value = KeySegment> {
    prop_oneof![
        4 => name().prop_map(KeySegment::Key),
        2 => any::<i64>().prop_map(KeySegment::Index),
        1 => Just(KeySegment::Wildcard),
        1 => Just(KeySegment::Recursive),
    ]
}

fn document() -> impl Strategy<Value = Option<DocSelector>> {
    proptest::option::of(prop_oneof![
        (1_u32..64).prop_map(DocSelector::Index),
        Just(DocSelector::All),
    ])
}

fn key_path() -> impl Strategy<Value = KeyPath> {
    (
        document(),
        proptest::collection::vec(segment(), 0..6),
        any::<bool>(),
    )
        .prop_filter_map(
            "segments form a valid key path",
            |(document, mut segments, append)| {
                if append {
                    segments.push(KeySegment::Append);
                }
                KeyPath::new(document, segments).ok()
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: PROPERTY_CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        ..ProptestConfig::default()
    })]

    #[test]
    fn printed_key_path_parses_to_the_same_key_path(key in key_path()) {
        let printed = key.to_string();
        let parsed = KeyPath::parse(&printed);
        prop_assert_eq!(parsed.as_ref().ok(), Some(&key), "printed as {:?}: {:?}", printed, parsed);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
