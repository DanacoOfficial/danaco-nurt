// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Round trips JSON ↔ CBOR ↔ tree of the golden trees and of generated trees.
//!
//! The properties run `PROPTEST_CASES` cases, 100 000 by default.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use std::collections::BTreeMap;
use std::fs;

use hash_support::{Format, Tree, all_trees, decode_cbor, decode_cbor_strict, decode_json};
use nurt_contracts::ast::{
    Arg, AstDocument, BinOp, Expr, Head, KeySeg, MapEntry, Meta, Pipeline, Program, Ref, Span,
    Stage, Statement, UnOp, Wildcard,
};
use nurt_contracts::bound::{
    BoundArg, BoundDocument, BoundExpr, BoundHead, BoundLet, BoundMapEntry, BoundPipeline,
    BoundProgram, BoundRef, BoundStage, BoundStatement, Phase,
};
use nurt_contracts::kinds::EffectKind;
use nurt_syntax::cbor::encode;
use nurt_syntax::hash::ast_hash;
use nurt_syntax::validate::{MAX_SAFE_INTEGER, is_reserved_word};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

/// Number of generated cases of every property unless `PROPTEST_CASES` is set.
const DEFAULT_CASES: u32 = 100_000;

fn config() -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(DEFAULT_CASES);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

#[test]
fn every_golden_tree_survives_json_and_cbor_round_trips() {
    let trees = all_trees();
    assert!(!trees.is_empty());
    for golden in trees {
        let name = golden.path.display().to_string();
        let tree = golden.decode();
        let again = decode_json(golden.format, &tree.to_json().to_string()).unwrap();
        assert_eq!(again, tree, "{name}: JSON round trip");
        let bytes = again.cbor();
        let from_cbor = decode_cbor(golden.format, &bytes).unwrap();
        assert_eq!(from_cbor, tree, "{name}: CBOR round trip");
        assert_eq!(
            decode_cbor_strict(golden.format, &bytes).unwrap(),
            tree,
            "{name}"
        );
        assert_eq!(from_cbor.cbor(), bytes, "{name}: repeated encoding");
        assert_eq!(
            encode(&golden.json()).unwrap(),
            bytes,
            "{name}: encoding of the JSON value"
        );
        let hash = tree.hash();
        assert_eq!(from_cbor.hash(), hash, "{name}: hash after CBOR");
        if let Ok(recorded) = fs::read_to_string(golden.hash_path()) {
            assert_eq!(recorded.trim_end(), hash, "{name}: recorded hash");
        }
    }
}

fn safe_integer() -> impl Strategy<Value = i64> {
    -MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER
}

fn natural() -> impl Strategy<Value = i64> {
    0..=MAX_SAFE_INTEGER
}

fn unsigned() -> impl Strategy<Value = u64> {
    natural().prop_map(i64::unsigned_abs)
}

fn identifier() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,5}".prop_filter("not a reserved word", |name| !is_reserved_word(name))
}

fn text() -> impl Strategy<Value = String> {
    "\\PC{0,6}"
}

fn finite() -> impl Strategy<Value = f64> {
    prop_oneof![
        any::<f64>().prop_filter("finite", |value| value.is_finite()),
        prop::sample::select(vec![3.0, -0.0, 0.5, 1.1]),
    ]
}

fn flags() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["", "i", "m", "ix", "ims", "imsx", "sx"]).prop_map(str::to_owned)
}

fn span() -> impl Strategy<Value = Option<Span>> {
    prop::option::of((1_u32..40, 1_u32..80, 0_u32..3, 0_u32..20).prop_map(
        |(line, column, lines, width)| {
            let end_column = if lines == 0 {
                column + width
            } else {
                1 + width
            };
            Span::try_from([line, column, line + lines, end_column]).unwrap()
        },
    ))
}

fn bin_op() -> impl Strategy<Value = BinOp> {
    prop::sample::select(vec![
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
    ])
}

fn un_op() -> impl Strategy<Value = UnOp> {
    prop::sample::select(vec![UnOp::Not, UnOp::Neg])
}

fn surface_ref() -> impl Strategy<Value = Ref> {
    (
        identifier(),
        prop::collection::vec(identifier(), 0..3),
        span(),
    )
        .prop_map(|(name, fields, span)| Ref { name, fields, span })
}

fn surface_leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        (text(), span()).prop_map(|(v, span)| Expr::Text { v, span }),
        (identifier(), span()).prop_map(|(v, span)| Expr::Ident { v, span }),
        ("[a-z.*]{1,4}", flags(), span()).prop_map(|(v, flags, span)| Expr::Regex {
            v,
            flags,
            span
        }),
        (natural(), span()).prop_map(|(v, span)| Expr::Int { v, span }),
        (finite(), span()).prop_map(|(v, span)| Expr::Num { v, span }),
        (unsigned(), span()).prop_map(|(v, span)| Expr::Size { v, span }),
        (unsigned(), span()).prop_map(|(v, span)| Expr::Dur { v, span }),
        (any::<bool>(), span()).prop_map(|(v, span)| Expr::Bool { v, span }),
        span().prop_map(|span| Expr::Null { span }),
        surface_ref().prop_map(Expr::Ref),
        (
            prop::option::of(natural()),
            prop::option::of(natural()),
            span()
        )
            .prop_map(|(lo, hi, span)| Expr::Range { lo, hi, span }),
    ]
}

fn key_segment(expr: BoxedStrategy<Expr>) -> impl Strategy<Value = KeySeg> {
    prop_oneof![
        identifier().prop_map(KeySeg::Name),
        safe_integer().prop_map(KeySeg::Index),
        prop::sample::select(vec![Wildcard::Level, Wildcard::Deep, Wildcard::Append])
            .prop_map(|wild| KeySeg::Wild { wild }),
        expr.prop_map(|filter| KeySeg::Filter {
            filter: Box::new(filter)
        }),
    ]
}

fn surface_expr() -> BoxedStrategy<Expr> {
    surface_leaf()
        .prop_recursive(3, 24, 3, |inner| {
            let item = prop_oneof![
                3 => inner.clone(),
                1 => (inner.clone(), span()).prop_map(|(x, span)| Expr::Spread { x: Box::new(x), span }),
            ];
            prop_oneof![
                (prop::collection::vec(item, 0..3), span())
                    .prop_map(|(items, span)| Expr::List { items, span }),
                (prop::collection::btree_map(identifier(), inner.clone(), 0..3), span()).prop_map(
                    |(entries, span)| Expr::Map {
                        entries: entries
                            .into_iter()
                            .map(|(k, v)| MapEntry { k, v, span: None })
                            .collect(),
                        span,
                    }
                ),
                (identifier(), prop::option::of(inner.clone()), span()).prop_map(
                    |(name, of, span)| Expr::Field {
                        name,
                        of: of.map(Box::new),
                        span
                    }
                ),
                (identifier(), prop::collection::vec(inner.clone(), 0..3), span()).prop_map(
                    |(function, args, span)| Expr::Call {
                        function,
                        args,
                        span
                    }
                ),
                (un_op(), inner.clone(), span()).prop_map(|(op, x, span)| Expr::Un {
                    op,
                    x: Box::new(x),
                    span
                }),
                (bin_op(), inner.clone(), inner.clone(), span()).prop_map(|(op, l, r, span)| {
                    Expr::Bin {
                        op,
                        l: Box::new(l),
                        r: Box::new(r),
                        span,
                    }
                }),
                (prop::collection::vec(key_segment(inner.clone()), 1..3), span())
                    .prop_map(|(segs, span)| Expr::Key { segs, span }),
                (surface_ref(), prop::collection::vec(surface_stage(inner), 0..2), span())
                    .prop_map(|(head, stages, span)| Expr::Sub {
                        p: Pipeline {
                            head: Head::Ref(head),
                            stages,
                            span,
                        },
                        span: None,
                    }),
            ]
        })
        .boxed()
}

fn command_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,4}(\\.[a-z][a-z0-9_]{0,4}){0,2}"
}

fn surface_stage(expr: BoxedStrategy<Expr>) -> impl Strategy<Value = Stage> {
    let arg = (prop::option::of(identifier()), expr, span()).prop_map(|(name, value, span)| Arg {
        name,
        value,
        span,
    });
    (command_name(), prop::collection::vec(arg, 0..3), span())
        .prop_map(|(name, args, span)| Stage { name, args, span })
}

fn surface_pipeline() -> impl Strategy<Value = Pipeline> {
    let head = prop_oneof![
        surface_stage(surface_expr()).prop_map(Head::Stage),
        surface_ref().prop_map(Head::Ref),
    ];
    (
        head,
        prop::collection::vec(surface_stage(surface_expr()), 0..3),
        span(),
    )
        .prop_map(|(head, stages, span)| Pipeline { head, stages, span })
}

fn meta() -> impl Strategy<Value = Option<Meta>> {
    prop::option::of(
        prop::collection::btree_map(text(), (text(), natural()), 0..3).prop_map(|members| {
            members
                .into_iter()
                .map(|(name, (text, number))| (name, serde_json::json!([text, number])))
                .collect()
        }),
    )
}

fn surface_document() -> impl Strategy<Value = AstDocument> {
    let statement = prop_oneof![
        surface_pipeline().prop_map(Statement::Pipeline),
        (identifier(), surface_pipeline(), span()).prop_map(|(name, value, span)| {
            Statement::Let(nurt_contracts::ast::Let { name, value, span })
        }),
    ];
    (prop::collection::vec(statement, 1..3), meta()).prop_map(|(statements, meta)| {
        let mut document = AstDocument::new(Program { statements });
        document.meta = meta;
        document
    })
}

fn instant() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "2026-11-22T10:00:00Z",
        "2024-02-29T23:59:59.5Z",
        "2026-01-01T00:00:00.25Z",
    ])
    .prop_map(str::to_owned)
}

fn bound_leaf() -> impl Strategy<Value = BoundExpr> {
    prop_oneof![
        any::<()>().prop_map(|()| BoundExpr::Null),
        any::<bool>().prop_map(|v| BoundExpr::Bool { v }),
        safe_integer().prop_map(|v| BoundExpr::Int { v }),
        finite().prop_map(|v| BoundExpr::Num { v }),
        text().prop_map(|v| BoundExpr::Text { v }),
        "/[a-z]{1,4}(/[a-z ]{1,4}){0,2}".prop_map(|v| BoundExpr::Path { v }),
        "[a-z*/]{1,6}".prop_map(|v| BoundExpr::Glob { v }),
        "[a-z.]{1,6}".prop_map(|v| BoundExpr::Key { v }),
        identifier().prop_map(|v| BoundExpr::Enum { v }),
        instant().prop_map(|v| BoundExpr::Instant { v }),
        unsigned().prop_map(|v| BoundExpr::Size { v }),
        unsigned().prop_map(|v| BoundExpr::Dur { v }),
        ("[a-z]{1,3}", flags()).prop_map(|(v, flags)| BoundExpr::Regex { v, flags }),
        (
            prop::option::of(safe_integer()),
            prop::option::of(safe_integer())
        )
            .prop_map(|(lo, hi)| BoundExpr::Range { lo, hi }),
        bound_ref().prop_map(BoundExpr::Ref),
    ]
}

fn bound_ref() -> impl Strategy<Value = BoundRef> {
    (identifier(), prop::collection::vec(identifier(), 0..2))
        .prop_map(|(name, fields)| BoundRef { name, fields })
}

fn bound_expr() -> BoxedStrategy<BoundExpr> {
    bound_leaf()
        .prop_recursive(3, 24, 3, |inner| {
            let item = prop_oneof![
                3 => inner.clone(),
                1 => inner.clone().prop_map(|x| BoundExpr::Spread { x: Box::new(x) }),
            ];
            prop_oneof![
                prop::collection::vec(item, 0..3).prop_map(|items| BoundExpr::List { items }),
                prop::collection::btree_map(identifier(), inner.clone(), 0..3).prop_map(
                    |entries| BoundExpr::Map {
                        entries: entries
                            .into_iter()
                            .map(|(k, v)| BoundMapEntry { k, v })
                            .collect(),
                    }
                ),
                (identifier(), prop::option::of(inner.clone())).prop_map(|(name, of)| {
                    BoundExpr::Field {
                        name,
                        of: of.map(Box::new),
                    }
                }),
                (identifier(), prop::collection::vec(inner.clone(), 0..3))
                    .prop_map(|(function, args)| BoundExpr::Call { function, args }),
                (un_op(), inner.clone()).prop_map(|(op, x)| BoundExpr::Un { op, x: Box::new(x) }),
                (bin_op(), inner.clone(), inner.clone()).prop_map(|(op, l, r)| BoundExpr::Bin {
                    op,
                    l: Box::new(l),
                    r: Box::new(r),
                }),
                (
                    bound_ref(),
                    prop::collection::vec(bound_stage(inner, Phase::Transform), 0..2)
                )
                    .prop_map(|(head, stages)| BoundExpr::Sub {
                        p: BoundPipeline {
                            head: BoundHead::Ref(head),
                            stages,
                            span: None,
                        },
                    }),
            ]
        })
        .boxed()
}

fn effect_for(phase: Phase) -> BoxedStrategy<EffectKind> {
    match phase {
        Phase::Mutate => {
            prop::sample::select(vec![EffectKind::Local, EffectKind::External]).boxed()
        }
        Phase::Run => prop::sample::select(vec![EffectKind::Process, EffectKind::External]).boxed(),
        _ => prop::sample::select(vec![EffectKind::Read]).boxed(),
    }
}

fn bound_stage(expr: BoxedStrategy<BoundExpr>, phase: Phase) -> impl Strategy<Value = BoundStage> {
    let args = prop::collection::btree_map(identifier(), (expr, any::<bool>(), span()), 0..3);
    (
        "[a-z][a-z0-9_]{0,4}(\\.[a-z][a-z0-9_]{0,4})?",
        effect_for(phase),
        any::<bool>(),
        args,
        span(),
    )
        .prop_map(move |(name, effect, waits, args, span)| BoundStage {
            name,
            effect,
            phase,
            waits,
            args: args
                .into_iter()
                .map(|(name, (value, default, span))| BoundArg {
                    name,
                    value,
                    default,
                    span: if default { None } else { span },
                })
                .collect::<Vec<_>>(),
            span,
        })
}

fn phases_of(stages: &BTreeMap<usize, Phase>) -> Vec<Phase> {
    stages.values().copied().collect()
}

/// Pipeline of the given shape: a source head, transformations and checks,
/// an optional effect, `after` stages and an optional `verify`.
fn bound_pipeline(effect: bool) -> impl Strategy<Value = BoundPipeline> {
    let before = prop::collection::vec(
        prop::sample::select(vec![Phase::Transform, Phase::Check]),
        0..3,
    );
    let effect_phase = prop::sample::select(vec![Phase::Mutate, Phase::Run]);
    let after = 0_usize..3;
    let verify = any::<bool>();
    (before, effect_phase, after, verify, span()).prop_flat_map(
        move |(before, effect_phase, after, verify, span)| {
            let mut shape: BTreeMap<usize, Phase> = BTreeMap::new();
            for phase in before {
                shape.insert(shape.len(), phase);
            }
            if effect {
                shape.insert(shape.len(), effect_phase);
                for _ in 0..after {
                    shape.insert(shape.len(), Phase::After);
                }
                if verify {
                    shape.insert(shape.len(), Phase::Verify);
                }
            }
            let stages: Vec<_> = phases_of(&shape)
                .into_iter()
                .map(|phase| bound_stage(bound_expr(), phase).boxed())
                .collect();
            let head = prop_oneof![
                bound_stage(bound_expr(), Phase::Source).prop_map(BoundHead::Stage),
                bound_ref().prop_map(BoundHead::Ref),
            ];
            (head, stages).prop_map(move |(head, stages)| BoundPipeline { head, stages, span })
        },
    )
}

fn bound_document() -> impl Strategy<Value = BoundDocument> {
    let statement = prop_oneof![
        any::<bool>()
            .prop_flat_map(|effect| bound_pipeline(effect).prop_map(BoundStatement::Pipeline)),
        (identifier(), bound_pipeline(false), span()).prop_map(|(name, value, span)| {
            BoundStatement::Let(BoundLet { name, value, span })
        }),
    ];
    (prop::collection::vec(statement, 1..3), meta()).prop_map(|(statements, meta)| {
        let mut document = BoundDocument::new("cat-1", "corr-1", BoundProgram { statements });
        document.meta = meta;
        document
    })
}

fn check_round_trip(tree: &Tree, format: Format) -> Result<(), TestCaseError> {
    let bytes = tree.cbor();
    prop_assert_eq!(&tree.cbor(), &bytes);
    let decoded =
        decode_cbor(format, &bytes).map_err(|error| TestCaseError::fail(error.to_string()))?;
    prop_assert_eq!(&decoded, tree);
    let strict = decode_cbor_strict(format, &bytes)
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    prop_assert_eq!(&strict, tree);
    let from_json = decode_json(format, &tree.to_json().to_string())
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    prop_assert_eq!(&from_json, tree);
    prop_assert_eq!(decoded.hash(), tree.hash());
    let without_layout = hash_support::without_layout(&tree.to_json());
    let plain = decode_json(format, &without_layout.to_string())
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    prop_assert_eq!(plain.hash(), tree.hash());
    Ok(())
}

#[test]
fn generated_surface_trees_survive_cbor_and_keep_their_hash() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&surface_document(), |document| {
            check_round_trip(&Tree::Surface(document), Format::Surface)
        })
        .unwrap();
}

#[test]
fn generated_bound_trees_survive_cbor_and_keep_their_hash() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&bound_document(), |document| {
            prop_assert!(ast_hash(&document).is_ok());
            check_round_trip(&Tree::Bound(document), Format::Bound)
        })
        .unwrap();
}
