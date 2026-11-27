// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/keypath_properties.rs
// Responsibility:   Properties of key paths: generated round trips, serialization, schema and.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Properties of key paths: generated round trips, serialization, schema and
//! conversion from the structured key of the syntax tree.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::ast::{BinOp, Expr, KeySeg, UnOp, Wildcard};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{
    CompareOp, DocSelector, FilterExpr, FilterLiteral, KeyPath, KeyPathErrorKind, KeySegment,
};

/// Names that exercise quoting rules.
const NAMES: &[&str] = &[
    "a",
    "name",
    "0",
    "-1",
    "*",
    "**",
    "$",
    "$ref",
    "#1:x",
    "#*:y",
    "#text",
    "@attr",
    " sp",
    "sp ",
    "in ner",
    "a.b",
    "x[y]",
    "q\"t",
    "it's",
    "",
    "na\u{ef}ve",
    "tab\t",
    "\u{7}",
    "\\",
    "true",
    "and",
    "not",
    "x-y",
    "_u",
    "\u{65e5}\u{672c}",
    "+",
    "?",
    ":",
    "a:b",
];

const TEXTS: &[&str] = &["", "x", "]", "\"", "'", "\\", "a b", "\n", "\u{1F600}"];

const NUMBERS: &[f64] = &[0.5, -2.25, 1e300, 1e-7, 3.0, -0.0, 123_456.789];

/// Deterministic pseudo-random generator (64-bit linear congruential).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next()).unwrap() % n
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn gen_operand(rng: &mut Lcg) -> FilterExpr {
    match rng.below(7) {
        0 => FilterExpr::Field { path: Vec::new() },
        1 | 2 => FilterExpr::Field {
            path: (0..=rng.below(2))
                .map(|_| (*rng.pick(NAMES)).to_owned())
                .collect(),
        },
        3 => FilterExpr::Literal {
            value: FilterLiteral::Text((*rng.pick(TEXTS)).to_owned()),
        },
        4 => FilterExpr::Literal {
            value: FilterLiteral::Int(i64::try_from(rng.next()).unwrap() - (1 << 30)),
        },
        5 => FilterExpr::Literal {
            value: FilterLiteral::Number(*rng.pick(NUMBERS)),
        },
        _ => {
            let choices = [
                FilterLiteral::Bool(true),
                FilterLiteral::Bool(false),
                FilterLiteral::Null,
            ];
            FilterExpr::Literal {
                value: rng.pick(&choices).clone(),
            }
        }
    }
}

fn gen_filter(rng: &mut Lcg, depth: u32) -> FilterExpr {
    if depth == 0 {
        return gen_operand(rng);
    }
    let sub = |rng: &mut Lcg| Box::new(gen_filter(rng, depth - 1));
    match rng.below(6) {
        0 => gen_operand(rng),
        1 => {
            let op = *rng.pick(&[
                CompareOp::Eq,
                CompareOp::Ne,
                CompareOp::Lt,
                CompareOp::Le,
                CompareOp::Gt,
                CompareOp::Ge,
            ]);
            FilterExpr::Compare {
                op,
                left: sub(rng),
                right: sub(rng),
            }
        }
        2 => FilterExpr::And {
            left: sub(rng),
            right: sub(rng),
        },
        3 => FilterExpr::Or {
            left: sub(rng),
            right: sub(rng),
        },
        4 => FilterExpr::Not { operand: sub(rng) },
        _ => FilterExpr::Compare {
            op: CompareOp::Eq,
            left: Box::new(gen_operand(rng)),
            right: Box::new(gen_operand(rng)),
        },
    }
}

fn gen_key(rng: &mut Lcg) -> KeyPath {
    let document = match rng.below(5) {
        0 => Some(DocSelector::Index(u32::try_from(rng.below(5)).unwrap())),
        1 => Some(DocSelector::All),
        _ => None,
    };
    let count = rng.below(6);
    let mut segments = Vec::new();
    for position in 0..count {
        let segment = match rng.below(9) {
            0..=3 => KeySegment::Key((*rng.pick(NAMES)).to_owned()),
            4 => KeySegment::Index(i64::try_from(rng.below(2000)).unwrap() - 1000),
            5 => KeySegment::Wildcard,
            6 => KeySegment::Recursive,
            7 => KeySegment::Filter(gen_filter(rng, 3)),
            _ if position + 1 == count => KeySegment::Append,
            _ => KeySegment::Key((*rng.pick(NAMES)).to_owned()),
        };
        segments.push(segment);
    }
    KeyPath::new(document, segments).unwrap()
}

#[test]
fn generated_key_paths_round_trip() {
    let mut rng = Lcg(0x5eed_1234_abcd_0001);
    for _ in 0..5000 {
        let key = gen_key(&mut rng);
        let printed = key.to_string();
        let parsed = KeyPath::parse(&printed)
            .unwrap_or_else(|e| panic!("`{printed}` from {key:?} does not parse: {e}"));
        assert_eq!(parsed, key, "round trip of `{printed}`");
        assert_eq!(
            parsed.to_string(),
            printed,
            "printing is not idempotent for `{printed}`"
        );
    }
}

#[test]
fn every_single_name_round_trips_at_every_position() {
    for name in NAMES {
        for prefix in [
            None,
            Some(KeySegment::Key("p".to_owned())),
            Some(KeySegment::Index(0)),
        ] {
            for document in [None, Some(DocSelector::Index(3))] {
                let mut segments: Vec<KeySegment> = prefix.iter().cloned().collect();
                segments.push(KeySegment::Key((*name).to_owned()));
                let key = KeyPath::new(document, segments).unwrap();
                let printed = key.to_string();
                assert_eq!(KeyPath::parse(&printed).unwrap(), key, "`{printed}`");
            }
        }
    }
}

#[test]
fn serializes_as_canonical_text() {
    let key = KeyPath::parse("$.deps[0]['name']").unwrap();
    assert_eq!(
        serde_json::to_value(&key).unwrap(),
        serde_json::json!("deps[0].name")
    );
    let back: KeyPath = serde_json::from_value(serde_json::json!("$..x")).unwrap();
    assert_eq!(back.to_string(), "**.x");
    let error = serde_json::from_value::<KeyPath>(serde_json::json!("a.")).unwrap_err();
    assert!(error.to_string().contains("E_KEY_SYNTAX"));
}

#[test]
fn segment_and_error_types_serialize() {
    let key = KeyPath::parse("a[0][*][?x == 1]").unwrap();
    let value = serde_json::to_value(key.segments()).unwrap();
    assert_eq!(
        value,
        serde_json::json!([
            {"key": "a"},
            {"index": 0},
            "wildcard",
            {"filter": {"t": "compare", "op": "eq",
                "left": {"t": "field", "path": ["x"]},
                "right": {"t": "literal", "value": {"int": 1}}}}
        ])
    );
    let back: Vec<KeySegment> = serde_json::from_value(value).unwrap();
    assert_eq!(back, key.segments());

    let error = KeyPath::parse("a.").unwrap_err();
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(json["kind"], "trailing_dot");
    assert_eq!(json["fix"], "a");
}

#[test]
fn schema_is_a_string() {
    let schema = schemars::schema_for!(KeyPath);
    let json = serde_json::to_value(&schema).unwrap();
    assert_eq!(json["type"], "string");
}

#[test]
fn converts_to_contract_error() {
    let error: Error = KeyPath::parse("a..b").unwrap_err().into();
    assert_eq!(error.code(), ErrorCode::KeySyntax);
    assert_eq!(
        error.code().as_str(),
        nurt_contracts::keypath::KEY_SYNTAX_ERROR_CODE
    );
    let data = error.data();
    assert_eq!(data.details["key_column"], serde_json::json!(3));
    assert_eq!(data.details["key_fix"], serde_json::json!("a.**.b"));
}

fn ident(name: &str) -> Box<Expr> {
    Box::new(Expr::Ident {
        v: name.to_owned(),
        span: None,
    })
}

fn text(value: &str) -> Box<Expr> {
    Box::new(Expr::Text {
        v: value.to_owned(),
        span: None,
    })
}

#[test]
fn converts_structured_keys_of_the_syntax_tree() {
    let filter = Expr::Bin {
        op: BinOp::And,
        l: Box::new(Expr::Bin {
            op: BinOp::Eq,
            l: ident("name"),
            r: text("x"),
            span: None,
        }),
        r: Box::new(Expr::Un {
            op: UnOp::Not,
            x: Box::new(Expr::Field {
                name: "beta".to_owned(),
                of: Some(ident("flags")),
                span: None,
            }),
            span: None,
        }),
        span: None,
    };
    let segs = [
        KeySeg::Name("deps".to_owned()),
        KeySeg::Index(-1),
        KeySeg::Wild {
            wild: Wildcard::Level,
        },
        KeySeg::Wild {
            wild: Wildcard::Deep,
        },
        KeySeg::Filter {
            filter: Box::new(filter),
        },
    ];
    let key = KeyPath::from_key_segs(&segs).unwrap();
    assert_eq!(
        key.to_string(),
        "deps[-1][*].**[?name == \"x\" and not flags.beta]"
    );

    let append = [
        KeySeg::Wild {
            wild: Wildcard::Append,
        },
        KeySeg::Name("x".to_owned()),
    ];
    assert_eq!(
        KeyPath::from_key_segs(&append).unwrap_err().kind,
        KeyPathErrorKind::MisplacedAppend
    );
    assert_eq!(
        KeyPath::from_key_segs(&[]).unwrap_err().kind,
        KeyPathErrorKind::Empty
    );

    let regex = Expr::Bin {
        op: BinOp::Match,
        l: ident("name"),
        r: text("x"),
        span: None,
    };
    assert_eq!(
        KeyPath::from_key_segs(&[KeySeg::Filter {
            filter: Box::new(regex)
        }])
        .unwrap_err()
        .kind,
        KeyPathErrorKind::Unsupported
    );
}

fn field(name: &str) -> Box<FilterExpr> {
    Box::new(FilterExpr::Field {
        path: vec![name.to_owned()],
    })
}

/// Filter whose canonical text needs `levels` nested levels: negations
/// followed by a leaf.
fn negations(levels: usize) -> FilterExpr {
    let mut expr = FilterExpr::Field {
        path: vec!["x".to_owned()],
    };
    for _ in 0..levels {
        expr = FilterExpr::Not {
            operand: Box::new(expr),
        };
    }
    expr
}

/// Filter whose canonical text needs `levels` nested parenthesised groups:
/// a comparison whose left operand is a comparison, and so on.
fn groups(levels: usize) -> FilterExpr {
    let compare = |left: FilterExpr| FilterExpr::Compare {
        op: CompareOp::Eq,
        left: Box::new(left),
        right: field("x"),
    };
    let mut expr = compare(*field("x"));
    for _ in 0..levels {
        expr = compare(expr);
    }
    expr
}

#[test]
fn constructors_enforce_the_filter_depth_of_the_parser() {
    use nurt_contracts::keypath::MAX_FILTER_DEPTH;

    for build in [negations, groups] {
        let admitted = KeyPath::new(None, vec![KeySegment::Filter(build(MAX_FILTER_DEPTH))])
            .expect("the deepest admitted filter");
        let text = admitted.to_string();
        assert_eq!(KeyPath::parse(&text).unwrap(), admitted, "{text}");

        let error =
            KeyPath::new(None, vec![KeySegment::Filter(build(MAX_FILTER_DEPTH + 1))]).unwrap_err();
        assert_eq!(error.kind, KeyPathErrorKind::InvalidFilter);
        let mut key = KeyPath::root();
        let error = key
            .push(KeySegment::Filter(build(MAX_FILTER_DEPTH + 1)))
            .unwrap_err();
        assert_eq!(error.kind, KeyPathErrorKind::InvalidFilter);
        assert!(key.is_root());
    }
}

#[test]
fn a_negative_literal_of_the_syntax_tree_becomes_a_filter_literal() {
    let negated = |x: Expr| Expr::Un {
        op: UnOp::Neg,
        x: Box::new(x),
        span: None,
    };
    let compare = |right: Expr| KeySeg::Filter {
        filter: Box::new(Expr::Bin {
            op: BinOp::Gt,
            l: ident("x"),
            r: Box::new(right),
            span: None,
        }),
    };
    let int = |v: i64| Expr::Int { v, span: None };

    let key = KeyPath::from_key_segs(&[compare(negated(int(1)))]).unwrap();
    assert_eq!(key.to_string(), "[?x > -1]");
    assert_eq!(KeyPath::parse(&key.to_string()).unwrap(), key);

    let key =
        KeyPath::from_key_segs(&[compare(negated(Expr::Num { v: 2.5, span: None }))]).unwrap();
    assert_eq!(key.to_string(), "[?x > -2.5]");

    assert_eq!(
        KeyPath::from_key_segs(&[compare(negated(Expr::Ident {
            v: "y".to_owned(),
            span: None,
        }))])
        .unwrap_err()
        .kind,
        KeyPathErrorKind::Unsupported
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
