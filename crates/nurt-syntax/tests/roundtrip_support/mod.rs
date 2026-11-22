// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Generator of surface trees inside the image of the parser.
//!
//! The trees cover every node kind of the surface tree except the
//! structured `key`, nest at most twelve node levels below a stage argument,
//! use identifiers that are not reserved words, unique map keys, spreads only
//! as list items, integers within `0..=2^53-1` and text with any Unicode
//! character, control characters included. Spans and metadata are absent.
//!
//! A test crate includes the generator with `mod roundtrip_support;` (or
//! `#[path = "roundtrip_support/mod.rs"] mod trees;` from another directory)
//! and draws documents from [`program`].

use nurt_contracts::ast::{
    Arg, AstDocument, BinOp, Expr, Head, Let, MapEntry, Pipeline, Program, Ref, Stage, Statement,
    UnOp, strip_for_hash,
};
use proptest::collection::vec;
use proptest::option;
use proptest::prelude::*;
use proptest::sample::select;

/// Reserved words of the language.
pub const KEYWORDS: [&str; 8] = ["let", "true", "false", "null", "and", "or", "not", "in"];

/// Largest integer of a tree, `2^53 - 1`.
pub const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// Levels of expression recursion; with the sub-pipelines in between, trees
/// stay within twelve node levels below a stage argument.
const EXPRESSION_LEVELS: u32 = 4;

/// Target number of expression nodes of one argument.
const EXPRESSION_NODES: u32 = 24;

/// Size units in bytes.
const SIZE_UNITS: [u64; 9] = [
    1,
    1_000,
    1_000_000,
    1_000_000_000,
    1_000_000_000_000,
    1 << 10,
    1 << 20,
    1 << 30,
    1 << 40,
];

/// Duration units in milliseconds.
const DURATION_UNITS: [u64; 6] = [1, 1_000, 60_000, 3_600_000, 86_400_000, 604_800_000];

const BINARY_OPERATORS: [BinOp; 15] = [
    BinOp::Eq,
    BinOp::Ne,
    BinOp::Lt,
    BinOp::Le,
    BinOp::Gt,
    BinOp::Ge,
    BinOp::Match,
    BinOp::NotMatch,
    BinOp::In,
    BinOp::And,
    BinOp::Or,
    BinOp::Add,
    BinOp::Sub,
    BinOp::Mul,
    BinOp::Div,
];

/// Restrictions of the generated trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// Every argument is named, as in variant C.
    pub named_only: bool,
    /// Text values may contain line feeds and carriage returns.
    pub line_breaks: bool,
}

impl Shape {
    /// Trees of variants A and B.
    pub const POSITIONAL: Self = Self {
        named_only: false,
        line_breaks: true,
    };
    /// Trees of variant C.
    pub const NAMED: Self = Self {
        named_only: true,
        line_breaks: true,
    };
    /// Trees whose printed text has no line break inside a literal.
    pub const SINGLE_LINE_TEXT: Self = Self {
        named_only: false,
        line_breaks: false,
    };
}

/// Hash form of a document: the JSON value without spans and metadata.
pub fn hash_form(document: &AstDocument) -> serde_json::Value {
    strip_for_hash(&serde_json::to_value(document).expect("tree serializes"))
}

/// Identifier that is not a reserved word.
pub fn identifier() -> BoxedStrategy<String> {
    "[a-z][a-z0-9_]{0,6}"
        .prop_map(|name| {
            if KEYWORDS.contains(&name.as_str()) {
                format!("{name}x")
            } else {
                name
            }
        })
        .boxed()
}

/// Identifier or reserved word, as accepted for parameter names and map keys.
fn word() -> BoxedStrategy<String> {
    prop_oneof![
        4 => identifier(),
        1 => select(KEYWORDS.to_vec()).prop_map(str::to_owned),
    ]
    .boxed()
}

fn command() -> BoxedStrategy<String> {
    prop_oneof![
        3 => identifier(),
        1 => (identifier(), identifier()).prop_map(|(space, name)| format!("{space}.{name}")),
    ]
    .boxed()
}

fn text_value(shape: Shape) -> BoxedStrategy<String> {
    let special = select(vec![
        '"', '\\', '#', '\n', '\r', '\t', '\0', '/', '{', '\u{7f}', 'ż', '\u{85}',
    ]);
    vec(prop_oneof![2 => any::<char>(), 1 => special], 0..10)
        .prop_map(move |chars| {
            chars
                .into_iter()
                .filter(|c| shape.line_breaks || !matches!(c, '\n' | '\r'))
                .collect()
        })
        .boxed()
}

fn regex() -> BoxedStrategy<Expr> {
    let piece = prop_oneof![
        4 => "[a-z0-9.*+?()|^$ /-]",
        1 => r"\\[a-z.()]",
        1 => Just("\\\\".to_owned()),
    ];
    ("[a-z]", vec(piece, 0..5), vec(any::<bool>(), 4))
        .prop_map(|(first, pieces, flags)| Expr::Regex {
            v: std::iter::once(first).chain(pieces).collect(),
            flags: "imsx"
                .chars()
                .zip(flags)
                .filter_map(|(flag, on)| on.then_some(flag))
                .collect(),
            span: None,
        })
        .boxed()
}

fn scaled(units: &'static [u64]) -> BoxedStrategy<u64> {
    let limit = u64::try_from(MAX_SAFE_INTEGER).unwrap_or(u64::MAX);
    prop_oneof![
        (1_u64..2_000, select(units)).prop_map(|(count, unit)| count * unit),
        0..=limit,
    ]
    .boxed()
}

fn number() -> BoxedStrategy<f64> {
    prop_oneof![
        (0_u32..10_000, 0_u32..1_000)
            .prop_map(|(whole, part)| f64::from(whole) + f64::from(part) / 1_000.0),
        any::<f64>().prop_map(|value| if value.is_finite() { value.abs() } else { 0.5 }),
    ]
    .boxed()
}

/// Reference to a named result with a field path.
pub fn reference() -> BoxedStrategy<Ref> {
    (identifier(), vec(identifier(), 0..3))
        .prop_map(|(name, fields)| Ref {
            name,
            fields,
            span: None,
        })
        .boxed()
}

fn leaf(shape: Shape) -> BoxedStrategy<Expr> {
    prop_oneof![
        text_value(shape).prop_map(|v| Expr::Text { v, span: None }),
        identifier().prop_map(|v| Expr::Ident { v, span: None }),
        regex(),
        prop_oneof![0_i64..100, 0..=MAX_SAFE_INTEGER].prop_map(|v| Expr::Int { v, span: None }),
        number().prop_map(|v| Expr::Num { v, span: None }),
        scaled(&SIZE_UNITS).prop_map(|v| Expr::Size { v, span: None }),
        scaled(&DURATION_UNITS).prop_map(|v| Expr::Dur { v, span: None }),
        any::<bool>().prop_map(|v| Expr::Bool { v, span: None }),
        Just(Expr::Null { span: None }),
        (option::of(0_i64..1_000), option::of(0_i64..1_000)).prop_map(|(lo, hi)| Expr::Range {
            lo,
            hi,
            span: None
        }),
        reference().prop_map(Expr::Ref),
    ]
    .boxed()
}

fn boxed(expr: Expr) -> Box<Expr> {
    Box::new(expr)
}

fn unique_entries(entries: Vec<(String, Expr)>) -> Vec<MapEntry> {
    let mut unique: Vec<MapEntry> = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        if unique.iter().all(|entry| entry.k != k) {
            unique.push(MapEntry { k, v });
        }
    }
    unique
}

/// Expression of a stage argument.
pub fn expression(shape: Shape) -> BoxedStrategy<Expr> {
    leaf(shape)
        .prop_recursive(EXPRESSION_LEVELS, EXPRESSION_NODES, 4, move |inner| {
            let item = prop_oneof![
                3 => inner.clone(),
                1 => inner.clone().prop_map(|x| Expr::Spread { x: boxed(x), span: None }),
            ];
            let key = prop_oneof![word(), text_value(shape)];
            prop_oneof![
                vec(item, 0..4).prop_map(|items| Expr::List { items, span: None }),
                vec((key, inner.clone()), 0..4).prop_map(|entries| Expr::Map {
                    entries: unique_entries(entries),
                    span: None
                }),
                (inner.clone(), identifier()).prop_map(|(of, name)| Expr::Field {
                    name,
                    of: Some(boxed(of)),
                    span: None
                }),
                (identifier(), vec(inner.clone(), 0..3)).prop_map(|(function, args)| Expr::Call {
                    function,
                    args,
                    span: None
                }),
                (any::<bool>(), inner.clone()).prop_map(|(negation, x)| Expr::Un {
                    op: if negation { UnOp::Not } else { UnOp::Neg },
                    x: boxed(x),
                    span: None
                }),
                (
                    select(BINARY_OPERATORS.to_vec()),
                    inner.clone(),
                    inner.clone()
                )
                    .prop_map(|(op, l, r)| {
                        Expr::Bin {
                            op,
                            l: boxed(l),
                            r: boxed(r),
                            span: None,
                        }
                    }),
                pipeline(shape, inner).prop_map(|p| Expr::Sub {
                    p: readable_sub(p),
                    span: None
                }),
            ]
        })
        .boxed()
}

/// Gives a sub-pipeline a stage after the head when its parenthesized form
/// would otherwise read as an expression.
fn readable_sub(mut pipeline: Pipeline) -> Pipeline {
    let lone = pipeline.stages.is_empty()
        && match &pipeline.head {
            Head::Stage(stage) => stage.args.is_empty(),
            _ => true,
        };
    if lone {
        pipeline.stages.push(Stage {
            name: "count".to_owned(),
            args: Vec::new(),
            span: None,
        });
    }
    pipeline
}

fn argument(shape: Shape, value: BoxedStrategy<Expr>) -> BoxedStrategy<Arg> {
    let named = (word(), value.clone()).prop_map(|(name, value)| Arg {
        name: Some(name),
        value,
        span: None,
    });
    if shape.named_only {
        named.boxed()
    } else {
        prop_oneof![
            2 => value.prop_map(|value| Arg { name: None, value, span: None }),
            1 => named,
        ]
        .boxed()
    }
}

fn stage(shape: Shape, value: BoxedStrategy<Expr>) -> BoxedStrategy<Stage> {
    (command(), vec(argument(shape, value), 0..3))
        .prop_map(|(name, args)| Stage {
            name,
            args,
            span: None,
        })
        .boxed()
}

fn pipeline(shape: Shape, value: BoxedStrategy<Expr>) -> BoxedStrategy<Pipeline> {
    let stage = stage(shape, value);
    let head = prop_oneof![
        3 => stage.clone().prop_map(Head::Stage),
        1 => reference().prop_map(Head::Ref),
    ];
    (head, vec(stage, 0..3))
        .prop_map(|(head, stages)| Pipeline {
            head,
            stages,
            span: None,
        })
        .boxed()
}

/// Program of one to three statements.
pub fn program(shape: Shape) -> BoxedStrategy<AstDocument> {
    let pipeline = pipeline(shape, expression(shape));
    let statement = prop_oneof![
        2 => pipeline.clone().prop_map(Statement::Pipeline),
        1 => (identifier(), pipeline).prop_map(|(name, value)| Statement::Let(Let {
            name,
            value,
            span: None
        })),
    ];
    vec(statement, 1..4)
        .prop_map(|statements| AstDocument::new(Program { statements }))
        .boxed()
}
