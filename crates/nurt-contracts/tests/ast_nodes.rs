// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/ast_nodes.rs
// Responsibility:   Node by node checks of the surface tree and the bound tree: the JSON form.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Node by node checks of the surface tree and the bound tree: the JSON form
//! of every expression kind, rejected members, spans and the hash form.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::ast::{
    self, AST_FORMAT, AST_HASH_PREFIX, AST_MEDIA_TYPE_CBOR, AST_MEDIA_TYPE_JSON, AST_VERSION, Arg,
    AstDocument, BinOp, Expr, Head, KEYWORDS, KeySeg, Let, MAX_IDENTIFIER_LENGTH, MAX_SAFE_INTEGER,
    MapEntry, Pipeline, Program, Ref, Span, Stage, Statement, UnOp, Wildcard,
};
use nurt_contracts::bound::{
    BOUND_FORMAT, BOUND_KINDS, BOUND_MEDIA_TYPE_CBOR, BOUND_MEDIA_TYPE_JSON, BOUND_VERSION,
    BoundArg, BoundDocument, BoundExpr, BoundHead, BoundMapEntry, BoundPipeline, BoundProgram,
    BoundRef, BoundStage, BoundStatement, Phase,
};
use nurt_contracts::kinds::EffectKind;
use serde_json::{Value, json};

fn text(value: &str) -> Expr {
    Expr::Text {
        v: value.to_owned(),
        span: None,
    }
}

fn int(value: i64) -> Expr {
    Expr::Int {
        v: value,
        span: None,
    }
}

fn stage(name: &str, args: Vec<Arg>) -> Stage {
    Stage {
        name: name.to_owned(),
        args,
        span: None,
    }
}

fn pipeline(name: &str) -> Pipeline {
    Pipeline {
        head: Head::Stage(stage(name, Vec::new())),
        stages: Vec::new(),
        span: None,
    }
}

#[test]
fn format_constants_name_the_documents() {
    assert_eq!(AST_FORMAT, "nurt-ast/1");
    assert_eq!(BOUND_FORMAT, "nurt-bound/1");
    assert_eq!(AST_VERSION, "1.1");
    assert_eq!(BOUND_VERSION, "1.1");
    assert_eq!(AST_HASH_PREFIX, "sha256:");
    assert_eq!(AST_MEDIA_TYPE_JSON, "application/vnd.nurt.ast+json; v=1");
    assert_eq!(AST_MEDIA_TYPE_CBOR, "application/vnd.nurt.ast+cbor; v=1");
    assert_eq!(
        BOUND_MEDIA_TYPE_JSON,
        "application/vnd.nurt.bound+json; v=1"
    );
    assert_eq!(
        BOUND_MEDIA_TYPE_CBOR,
        "application/vnd.nurt.bound+cbor; v=1"
    );
    assert_eq!(MAX_SAFE_INTEGER, 9_007_199_254_740_991);
    assert_eq!(MAX_IDENTIFIER_LENGTH, 64);
    assert_eq!(
        KEYWORDS,
        ["let", "true", "false", "null", "and", "or", "not", "in"]
    );
    assert_eq!(BOUND_KINDS.len(), 35);
}

#[test]
fn every_expression_kind_has_its_json_form() {
    let recent = Ref {
        name: "recent".to_owned(),
        fields: vec!["path".to_owned()],
        span: None,
    };
    let cases = vec![
        (text("a"), json!({"t": "text", "v": "a"})),
        (
            Expr::Ident {
                v: "desc".to_owned(),
                span: None,
            },
            json!({"t": "ident", "v": "desc"}),
        ),
        (
            Expr::Regex {
                v: "a+".to_owned(),
                flags: "i".to_owned(),
                span: None,
            },
            json!({"t": "regex", "v": "a+", "flags": "i"}),
        ),
        (int(-3), json!({"t": "int", "v": -3})),
        (
            Expr::Num { v: 2.5, span: None },
            json!({"t": "num", "v": 2.5}),
        ),
        (
            Expr::Size {
                v: 10_000_000,
                span: None,
            },
            json!({"t": "size", "v": 10_000_000}),
        ),
        (
            Expr::Dur {
                v: 90_000,
                span: None,
            },
            json!({"t": "dur", "v": 90_000}),
        ),
        (
            Expr::Bool {
                v: true,
                span: None,
            },
            json!({"t": "bool", "v": true}),
        ),
        (Expr::Null { span: None }, json!({"t": "null"})),
        (
            Expr::Range {
                lo: Some(10),
                hi: None,
                span: None,
            },
            json!({"t": "range", "lo": 10}),
        ),
        (
            Expr::Range {
                lo: None,
                hi: Some(9),
                span: None,
            },
            json!({"t": "range", "hi": 9}),
        ),
        (
            Expr::Un {
                op: UnOp::Neg,
                x: Box::new(int(1)),
                span: None,
            },
            json!({"t": "un", "op": "neg", "x": {"t": "int", "v": 1}}),
        ),
        (
            Expr::Bin {
                op: BinOp::NotMatch,
                l: Box::new(text("a")),
                r: Box::new(text("b")),
                span: None,
            },
            json!({
                "t": "bin", "op": "!~",
                "l": {"t": "text", "v": "a"}, "r": {"t": "text", "v": "b"}
            }),
        ),
        (
            Expr::Call {
                function: "ago".to_owned(),
                args: vec![Expr::Dur {
                    v: 3_600_000,
                    span: None,
                }],
                span: None,
            },
            json!({"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 3_600_000}]}),
        ),
        (
            Expr::Field {
                name: "size".to_owned(),
                of: None,
                span: None,
            },
            json!({"t": "field", "name": "size"}),
        ),
        (
            Expr::Field {
                name: "path".to_owned(),
                of: Some(Box::new(Expr::Ref(recent.clone()))),
                span: None,
            },
            json!({
                "t": "field", "name": "path",
                "of": {"t": "ref", "name": "recent", "fields": ["path"]}
            }),
        ),
        (
            Expr::Spread {
                x: Box::new(Expr::Ref(recent)),
                span: None,
            },
            json!({"t": "spread", "x": {"t": "ref", "name": "recent", "fields": ["path"]}}),
        ),
        (
            Expr::List {
                items: Vec::new(),
                span: None,
            },
            json!({"t": "list", "items": []}),
        ),
        (
            Expr::Map {
                entries: vec![MapEntry {
                    k: "replace".to_owned(),
                    v: text("a"),
                    span: None,
                }],
                span: None,
            },
            json!({"t": "map", "entries": [{"k": "replace", "v": {"t": "text", "v": "a"}}]}),
        ),
        (
            Expr::Key {
                segs: vec![
                    KeySeg::Name("a".to_owned()),
                    KeySeg::Index(-1),
                    KeySeg::Wild {
                        wild: Wildcard::Deep,
                    },
                ],
                span: None,
            },
            json!({"t": "key", "segs": ["a", -1, {"wild": "**"}]}),
        ),
        (
            Expr::Sub {
                p: pipeline("count"),
                span: None,
            },
            json!({
                "t": "sub",
                "p": {
                    "t": "pipeline",
                    "head": {"t": "stage", "name": "count", "args": []},
                    "stages": []
                }
            }),
        ),
    ];
    for (expression, expected) in cases {
        assert_eq!(serde_json::to_value(&expression).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<Expr>(expected).unwrap(),
            expression
        );
    }
}

#[test]
fn expressions_with_unknown_members_or_kinds_are_rejected() {
    for value in [
        json!({"t": "text", "v": "a", "extra": 1}),
        json!({"t": "text"}),
        json!({"t": "int", "v": "1"}),
        json!({"t": "size", "v": -1}),
        json!({"t": "path", "v": "/a"}),
        json!({"v": "a"}),
        json!({"t": "bin", "op": "===", "l": {"t": "null"}, "r": {"t": "null"}}),
        json!({"t": "un", "op": "-", "x": {"t": "null"}}),
        json!({"t": "key", "segs": [{"wild": "?"}]}),
        json!({"t": "key", "segs": [1.5]}),
    ] {
        assert!(
            serde_json::from_value::<Expr>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn key_segments_tell_names_indexes_wildcards_and_filters_apart() {
    let segments = [
        (KeySeg::Name("scripts".to_owned()), json!("scripts")),
        (KeySeg::Index(0), json!(0)),
        (KeySeg::Index(-2), json!(-2)),
        (
            KeySeg::Wild {
                wild: Wildcard::Level,
            },
            json!({"wild": "*"}),
        ),
        (
            KeySeg::Wild {
                wild: Wildcard::Append,
            },
            json!({"wild": "+"}),
        ),
        (
            KeySeg::Filter {
                filter: Box::new(Expr::Bin {
                    op: BinOp::Eq,
                    l: Box::new(Expr::Ident {
                        v: "name".to_owned(),
                        span: None,
                    }),
                    r: Box::new(text("x")),
                    span: None,
                }),
            },
            json!({"filter": {
                "t": "bin", "op": "==",
                "l": {"t": "ident", "v": "name"}, "r": {"t": "text", "v": "x"}
            }}),
        ),
    ];
    for (segment, expected) in segments {
        assert_eq!(serde_json::to_value(&segment).unwrap(), expected);
        assert_eq!(serde_json::from_value::<KeySeg>(expected).unwrap(), segment);
    }
    for invalid in [
        json!({"wild": "**", "extra": true}),
        json!({"wild": "x"}),
        json!({}),
        json!(null),
        json!(true),
    ] {
        assert!(
            serde_json::from_value::<KeySeg>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn operators_are_written_as_in_program_text() {
    let binary = [
        (BinOp::Eq, "=="),
        (BinOp::Ne, "!="),
        (BinOp::Lt, "<"),
        (BinOp::Le, "<="),
        (BinOp::Gt, ">"),
        (BinOp::Ge, ">="),
        (BinOp::Match, "~"),
        (BinOp::NotMatch, "!~"),
        (BinOp::In, "in"),
        (BinOp::And, "and"),
        (BinOp::Or, "or"),
        (BinOp::Add, "+"),
        (BinOp::Sub, "-"),
        (BinOp::Mul, "*"),
        (BinOp::Div, "/"),
    ];
    for (operator, written) in binary {
        assert_eq!(operator.as_str(), written);
        assert_eq!(operator.to_string(), written);
        assert_eq!(serde_json::to_value(operator).unwrap(), json!(written));
        assert_eq!(
            serde_json::from_value::<BinOp>(json!(written)).unwrap(),
            operator
        );
    }
    assert_eq!(UnOp::Not.as_str(), "not");
    assert_eq!(UnOp::Neg.as_str(), "-");
    assert_eq!(UnOp::Neg.to_string(), "-");
    assert_eq!(serde_json::to_value(UnOp::Not).unwrap(), json!("not"));
    assert_eq!(serde_json::to_value(UnOp::Neg).unwrap(), json!("neg"));
}

#[test]
fn spans_are_four_positions_from_one() {
    let cases = [
        (json!([1, 1, 1, 1]), true),
        (json!([1, 10, 2, 1]), true),
        (json!(vec![u32::MAX; 4]), true),
        (json!([0, 1, 1, 1]), false),
        (json!([1, 0, 1, 1]), false),
        (json!([1, 1, 0, 1]), false),
        (json!([1, 1, 1, 0]), false),
        (json!([2, 1, 1, 9]), false),
        (json!([1, 5, 1, 4]), false),
        (json!([1, 1, 1]), false),
        (json!([1, 1, 1, 1, 1]), false),
        (json!([-1, 1, 1, 1]), false),
        (json!([1.5, 1, 1, 1]), false),
        (json!("1:1-1:1"), false),
    ];
    for (value, valid) in cases {
        let parsed = serde_json::from_value::<Span>(value.clone());
        assert_eq!(parsed.is_ok(), valid, "{value}");
        if let Ok(span) = parsed {
            assert_eq!(serde_json::to_value(span).unwrap(), value);
        }
    }
    let span: Span = serde_json::from_value(json!([3, 7, 4, 2])).unwrap();
    assert_eq!(
        (span.line, span.column, span.end_line, span.end_column),
        (3, 7, 4, 2)
    );
}

#[test]
fn spans_are_kept_in_nodes_and_dropped_from_the_hash_form() {
    let document = json!({
        "nurt": "1.0",
        "program": {"statements": [{
            "t": "pipeline",
            "head": {
                "t": "stage", "name": "find", "span": [1, 1, 1, 5],
                "args": [{"value": {"t": "text", "v": "/srv", "span": [1, 6, 1, 12]}, "span": [1, 6, 1, 12]}]
            },
            "stages": [],
            "span": [1, 1, 1, 12]
        }]}
    });
    let typed: AstDocument = serde_json::from_value(document.clone()).unwrap();
    let Statement::Pipeline(pipeline) = &typed.program.statements[0] else {
        panic!("expected a pipeline");
    };
    assert_eq!(pipeline.span.map(|span| span.end_column), Some(12));
    assert_eq!(serde_json::to_value(&typed).unwrap(), document);

    let stripped = ast::strip_for_hash(&document);
    let text = stripped.to_string();
    assert!(!text.contains("span"), "{text}");
    assert!(text.contains("\"/srv\""), "{text}");
}

#[test]
fn the_hash_form_strips_meta_only_at_the_root() {
    let tree = json!({
        "span": 1,
        "meta": {"source": "x"},
        "keep": {"span": 2, "meta": 3, "list": [{"span": 4, "a": 5}]}
    });
    assert_eq!(
        ast::strip_for_hash(&tree),
        json!({"keep": {"meta": 3, "list": [{"a": 5}]}})
    );
    assert_eq!(ast::strip_for_hash(&json!(5)), json!(5));
    assert_eq!(ast::strip_for_hash(&json!("span")), json!("span"));
    assert_eq!(ast::strip_for_hash(&json!(null)), json!(null));
    assert_eq!(
        ast::strip_for_hash(&json!([{"span": 1, "x": 1}, {"meta": 1}])),
        json!([{"x": 1}, {"meta": 1}])
    );
}

#[test]
fn a_named_result_and_a_reference_have_their_json_form() {
    let statement = Statement::Let(Let {
        name: "recent".to_owned(),
        value: pipeline("find"),
        span: None,
    });
    let expected = json!({
        "t": "let",
        "name": "recent",
        "value": {
            "t": "pipeline",
            "head": {"t": "stage", "name": "find", "args": []},
            "stages": []
        }
    });
    assert_eq!(serde_json::to_value(&statement).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<Statement>(expected).unwrap(),
        statement
    );

    let reference = Pipeline {
        head: Head::Ref(Ref {
            name: "recent".to_owned(),
            fields: Vec::new(),
            span: None,
        }),
        stages: vec![stage("count", Vec::new())],
        span: None,
    };
    let value = serde_json::to_value(Statement::Pipeline(reference)).unwrap();
    assert_eq!(
        value,
        json!({
            "t": "pipeline",
            "head": {"t": "ref", "name": "recent"},
            "stages": [{"t": "stage", "name": "count", "args": []}]
        })
    );
}

#[test]
fn nodes_without_their_kind_member_are_rejected() {
    for value in [
        json!({"t": "let", "name": "a", "value": {"head": {"t": "stage", "name": "find", "args": []}, "stages": []}}),
        json!({"t": "let", "name": "a", "value": {"t": "stage", "name": "find", "args": []}}),
        json!({"t": "pipeline", "head": {"name": "find", "args": []}, "stages": []}),
        json!({"t": "pipeline", "head": {"t": "stage", "name": "find", "args": []}, "stages": [{"name": "count", "args": []}]}),
        json!({"t": "pipeline", "head": {"t": "stage", "name": "find", "args": []}, "stages": [{"t": "ref", "name": "a"}]}),
        json!({"t": "block", "statements": []}),
    ] {
        assert!(
            serde_json::from_value::<Statement>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn a_document_keeps_free_form_metadata() {
    let mut document = AstDocument::new(Program {
        statements: vec![Statement::Pipeline(pipeline("count"))],
    });
    assert_eq!(document.nurt, AST_VERSION);
    assert!(document.meta.is_none());
    assert!(
        serde_json::to_value(&document)
            .unwrap()
            .get("meta")
            .is_none()
    );

    let meta = json!({"source": "count", "variant": "a", "custom": [1, 2]});
    document.meta = meta.as_object().cloned();
    let value = serde_json::to_value(&document).unwrap();
    assert_eq!(value["meta"], meta);
    assert_eq!(
        serde_json::from_value::<AstDocument>(value).unwrap(),
        document
    );

    let unknown_root = json!({
        "nurt": "1.0",
        "program": {"statements": []},
        "bound": "1.0"
    });
    assert!(serde_json::from_value::<AstDocument>(unknown_root).is_err());
}

fn bound_stage() -> BoundStage {
    BoundStage {
        name: "ports".to_owned(),
        effect: EffectKind::Read,
        phase: Phase::Source,
        waits: false,
        args: vec![
            BoundArg::given(
                "state",
                BoundExpr::Enum {
                    v: "listen".to_owned(),
                },
            ),
            BoundArg::from_default("limit", BoundExpr::Null),
        ],
        span: None,
    }
}

#[test]
fn bound_arguments_mark_values_taken_from_defaults() {
    let given = BoundArg::given("a", BoundExpr::Null);
    let defaulted = BoundArg::from_default("b", BoundExpr::Null);
    assert!(!given.default && given.span.is_none());
    assert!(defaulted.default && defaulted.span.is_none());
    assert_eq!(
        serde_json::to_value(&given).unwrap(),
        json!({"name": "a", "value": {"t": "null"}})
    );
    assert_eq!(
        serde_json::to_value(&defaulted).unwrap(),
        json!({"name": "b", "value": {"t": "null"}, "default": true})
    );
    let explicit: BoundArg =
        serde_json::from_value(json!({"name": "a", "value": {"t": "null"}, "default": false}))
            .unwrap();
    assert_eq!(explicit, given);
}

#[test]
fn bound_stages_write_waits_only_when_true() {
    let mut stage = bound_stage();
    let value = serde_json::to_value(&stage).unwrap();
    assert_eq!(value["effect"], "read");
    assert_eq!(value["phase"], "source");
    assert!(value.get("waits").is_none());
    stage.waits = true;
    stage.phase = Phase::Run;
    stage.effect = EffectKind::Process;
    let value = serde_json::to_value(&stage).unwrap();
    assert_eq!(value["waits"], true);
    assert_eq!(value["phase"], "run");
    assert_eq!(value["effect"], "process");
    assert_eq!(serde_json::from_value::<BoundStage>(value).unwrap(), stage);
}

#[test]
fn phases_have_names_and_two_of_them_change_the_world() {
    let phases = [
        (Phase::Source, "source", false),
        (Phase::Transform, "transform", false),
        (Phase::Check, "check", false),
        (Phase::Mutate, "mutate", true),
        (Phase::Run, "run", true),
        (Phase::After, "after", false),
        (Phase::Verify, "verify", false),
    ];
    for (phase, name, effect) in phases {
        assert_eq!(serde_json::to_value(phase).unwrap(), json!(name));
        assert_eq!(phase.is_effect(), effect, "{name}");
    }
}

#[test]
fn every_bound_value_kind_has_its_json_form() {
    let pipeline = BoundPipeline {
        head: BoundHead::Stage(bound_stage()),
        stages: Vec::new(),
        span: None,
    };
    let cases = vec![
        (BoundExpr::Null, json!({"t": "null"})),
        (
            BoundExpr::Bool { v: false },
            json!({"t": "bool", "v": false}),
        ),
        (BoundExpr::Int { v: -9 }, json!({"t": "int", "v": -9})),
        (BoundExpr::Num { v: 0.5 }, json!({"t": "num", "v": 0.5})),
        (
            BoundExpr::Path {
                v: "/srv/app".to_owned(),
            },
            json!({"t": "path", "v": "/srv/app"}),
        ),
        (
            BoundExpr::Glob {
                v: "*.go".to_owned(),
            },
            json!({"t": "glob", "v": "*.go"}),
        ),
        (
            BoundExpr::Key {
                v: "scripts.test".to_owned(),
            },
            json!({"t": "key", "v": "scripts.test"}),
        ),
        (
            BoundExpr::Instant {
                v: "2026-03-01T12:00:00Z".to_owned(),
            },
            json!({"t": "instant", "v": "2026-03-01T12:00:00Z"}),
        ),
        (
            BoundExpr::Size { v: 1_024 },
            json!({"t": "size", "v": 1_024}),
        ),
        (BoundExpr::Dur { v: 250 }, json!({"t": "dur", "v": 250})),
        (
            BoundExpr::Regex {
                v: "a/b".to_owned(),
                flags: String::new(),
            },
            json!({"t": "regex", "v": "a/b", "flags": ""}),
        ),
        (
            BoundExpr::Range {
                lo: Some(1),
                hi: Some(1),
            },
            json!({"t": "range", "lo": 1, "hi": 1}),
        ),
        (
            BoundExpr::List {
                items: vec![BoundExpr::Int { v: 1 }],
            },
            json!({"t": "list", "items": [{"t": "int", "v": 1}]}),
        ),
        (
            BoundExpr::Map {
                entries: vec![BoundMapEntry {
                    k: "with".to_owned(),
                    v: BoundExpr::Null,
                }],
            },
            json!({"t": "map", "entries": [{"k": "with", "v": {"t": "null"}}]}),
        ),
        (
            BoundExpr::Ref(BoundRef {
                name: "recent".to_owned(),
                fields: vec!["path".to_owned()],
            }),
            json!({"t": "ref", "name": "recent", "fields": ["path"]}),
        ),
        (
            BoundExpr::Un {
                op: UnOp::Not,
                x: Box::new(BoundExpr::Bool { v: true }),
            },
            json!({"t": "un", "op": "not", "x": {"t": "bool", "v": true}}),
        ),
        (
            BoundExpr::Bin {
                op: BinOp::In,
                l: Box::new(BoundExpr::Int { v: 1 }),
                r: Box::new(BoundExpr::Range {
                    lo: None,
                    hi: Some(9),
                }),
            },
            json!({
                "t": "bin", "op": "in",
                "l": {"t": "int", "v": 1}, "r": {"t": "range", "hi": 9}
            }),
        ),
        (
            BoundExpr::Spread {
                x: Box::new(BoundExpr::Null),
            },
            json!({"t": "spread", "x": {"t": "null"}}),
        ),
        (
            BoundExpr::Field {
                name: "size".to_owned(),
                of: None,
            },
            json!({"t": "field", "name": "size"}),
        ),
        (
            BoundExpr::Call {
                function: "ago".to_owned(),
                args: vec![BoundExpr::Dur { v: 1 }],
            },
            json!({"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 1}]}),
        ),
        (
            BoundExpr::Sub { p: pipeline },
            json!({
                "t": "sub",
                "p": {
                    "t": "pipeline",
                    "head": {
                        "t": "stage", "name": "ports", "effect": "read", "phase": "source",
                        "args": [
                            {"name": "state", "value": {"t": "enum", "v": "listen"}},
                            {"name": "limit", "value": {"t": "null"}, "default": true}
                        ]
                    },
                    "stages": []
                }
            }),
        ),
    ];
    for (expression, expected) in cases {
        assert_eq!(serde_json::to_value(&expression).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<BoundExpr>(expected).unwrap(),
            expression
        );
    }
}

#[test]
fn bound_values_reject_surface_kinds_and_unknown_members() {
    for value in [
        json!({"t": "text", "v": "a", "span": [1, 1, 1, 2]}),
        json!({"t": "ident", "v": "desc"}),
        json!({"t": "path", "v": "/a", "span": [1, 1, 1, 2]}),
        json!({"t": "enum", "v": "a", "extra": 1}),
        json!({"t": "int"}),
        json!({"t": "int", "v": 1.5}),
        json!({"v": "a"}),
    ] {
        assert!(
            serde_json::from_value::<BoundExpr>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn a_bound_document_records_the_revisions_it_was_bound_with() {
    let document = BoundDocument::new(
        "cat-1",
        "corr-1",
        BoundProgram {
            statements: vec![BoundStatement::Pipeline(BoundPipeline {
                head: BoundHead::Stage(bound_stage()),
                stages: Vec::new(),
                span: None,
            })],
        },
    );
    assert_eq!(document.nurt, AST_VERSION);
    assert_eq!(document.bound, BOUND_VERSION);
    let value = serde_json::to_value(&document).unwrap();
    assert_eq!(value["catalog_rev"], "cat-1");
    assert_eq!(value["corr_rev"], "corr-1");
    assert!(value.get("meta").is_none());
    assert_eq!(
        serde_json::from_value::<BoundDocument>(value.clone()).unwrap(),
        document
    );

    for member in ["bound", "catalog_rev", "corr_rev", "program"] {
        let mut incomplete: Value = value.clone();
        incomplete.as_object_mut().unwrap().remove(member);
        assert!(
            serde_json::from_value::<BoundDocument>(incomplete).is_err(),
            "{member}"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
