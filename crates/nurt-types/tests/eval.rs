// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-types/tests/eval.rs
// Responsibility:   Evaluation of bound expressions: operators, fields, named results and built-in functions.
// Dependencies:     jiff, nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Evaluation of bound expressions: operators, fields, named results and
//! built-in functions.

// Assertions unwrap results whose failure is the failure of the test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jiff::tz::{self, TimeZone};
use nurt_contracts::ast::{BinOp, UnOp};
use nurt_contracts::bound::{BoundExpr, BoundMapEntry, BoundPipeline, BoundRef};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Bytes, Duration, Instant, Range, Record, RecordKind, Size, Value};
use nurt_types::{EvalEnv, Evaluator, FUNCTIONS, FixedEnv};
use serde_json::json;

const NOW: &str = "2026-11-21T10:00:00Z";

fn instant(text: &str) -> Instant {
    text.parse().unwrap()
}

fn env() -> FixedEnv {
    let file = |path: &str, bytes: u64| {
        Value::Record(
            Record::new(RecordKind::Entry)
                .with("path", Value::Path(path.to_owned()))
                .with("size", Value::Size(Size::from_bytes(bytes))),
        )
    };
    FixedEnv::new(instant(NOW), 7)
        .with_name(
            "files",
            Value::List(vec![file("/srv/a.rs", 10), file("/srv/b.rs", 30)]),
        )
        .with_name("label", Value::from("7d"))
        .with_known_field(RecordKind::Entry, "link_target")
}

fn entry() -> Record {
    Record::new(RecordKind::Entry)
        .with("path", Value::Path("/srv/app/src/main.rs".to_owned()))
        .with("name", Value::from("main.rs"))
        .with("size", Value::Size(Size::from_bytes(2_048)))
        .with("modified", Value::Instant(instant("2026-11-10T08:00:00Z")))
        .with("type", Value::Enum("file".to_owned()))
        .with("hidden", Value::Bool(false))
}

fn eval(expr: &BoundExpr) -> Result<Value> {
    Evaluator::new().eval(expr, Some(&entry()), &env())
}

fn eval_ok(expr: &BoundExpr) -> Value {
    eval(expr).unwrap_or_else(|error| panic!("{error}: {expr:?}"))
}

fn eval_err(expr: &BoundExpr) -> Error {
    match eval(expr) {
        Ok(value) => panic!("expected an error, got {value:?} for {expr:?}"),
        Err(error) => error,
    }
}

fn field(name: &str) -> BoundExpr {
    BoundExpr::Field {
        name: name.to_owned(),
        of: None,
    }
}

fn text(value: &str) -> BoundExpr {
    BoundExpr::Text {
        v: value.to_owned(),
    }
}

fn int(value: i64) -> BoundExpr {
    BoundExpr::Int { v: value }
}

fn boolean(value: bool) -> BoundExpr {
    BoundExpr::Bool { v: value }
}

fn regex(pattern: &str, flags: &str) -> BoundExpr {
    BoundExpr::Regex {
        v: pattern.to_owned(),
        flags: flags.to_owned(),
    }
}

fn list(items: Vec<BoundExpr>) -> BoundExpr {
    BoundExpr::List { items }
}

fn spread(expr: BoundExpr) -> BoundExpr {
    BoundExpr::Spread { x: Box::new(expr) }
}

fn reference(name: &str, fields: &[&str]) -> BoundExpr {
    BoundExpr::Ref(BoundRef {
        name: name.to_owned(),
        fields: fields.iter().map(|field| (*field).to_owned()).collect(),
    })
}

fn bin(op: BinOp, left: BoundExpr, right: BoundExpr) -> BoundExpr {
    BoundExpr::Bin {
        op,
        l: Box::new(left),
        r: Box::new(right),
    }
}

fn un(op: UnOp, operand: BoundExpr) -> BoundExpr {
    BoundExpr::Un {
        op,
        x: Box::new(operand),
    }
}

fn call(function: &str, args: Vec<BoundExpr>) -> BoundExpr {
    BoundExpr::Call {
        function: function.to_owned(),
        args,
    }
}

fn division_by_zero() -> BoundExpr {
    bin(BinOp::Div, int(1), int(0))
}

#[test]
fn evaluates_every_literal_node() {
    let entries = vec![BoundMapEntry {
        k: "a".to_owned(),
        v: int(1),
    }];
    let cases = [
        (BoundExpr::Null, Value::Null),
        (boolean(true), Value::Bool(true)),
        (BoundExpr::Num { v: 1.5 }, Value::Number(1.5)),
        (
            BoundExpr::Path {
                v: "/srv".to_owned(),
            },
            Value::Path("/srv".to_owned()),
        ),
        (
            BoundExpr::Glob {
                v: "*.rs".to_owned(),
            },
            Value::Glob("*.rs".to_owned()),
        ),
        (
            BoundExpr::Key {
                v: "a.b".to_owned(),
            },
            Value::Key("a.b".to_owned()),
        ),
        (
            BoundExpr::Enum {
                v: "file".to_owned(),
            },
            Value::Enum("file".to_owned()),
        ),
        (
            BoundExpr::Instant { v: NOW.to_owned() },
            Value::Instant(instant(NOW)),
        ),
        (BoundExpr::Size { v: 10 }, Value::Size(Size::from_bytes(10))),
        (
            BoundExpr::Dur { v: 250 },
            Value::Duration(Duration::from_millis(250)),
        ),
        (
            BoundExpr::Range {
                lo: Some(1),
                hi: None,
            },
            Value::Range(Range::from_start(1)),
        ),
        (
            BoundExpr::Map { entries },
            Value::Map([("a".to_owned(), Value::Int(1))].into_iter().collect()),
        ),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval_ok(&expr), expected);
    }
}

#[test]
fn rejects_invalid_literal_nodes() {
    let cases = [
        (BoundExpr::Dur { v: u64::MAX }, ErrorCode::InvalidValue),
        (
            BoundExpr::Instant {
                v: "noon".to_owned(),
            },
            ErrorCode::InvalidValue,
        ),
        (
            BoundExpr::Range {
                lo: Some(5),
                hi: Some(1),
            },
            ErrorCode::InvalidValue,
        ),
        (regex("a", "q"), ErrorCode::Regex),
        (spread(list(Vec::new())), ErrorCode::Type),
    ];
    for (expr, code) in cases {
        assert_eq!(eval_err(&expr).code, code, "{expr:?}");
    }
    let sub = BoundExpr::Sub {
        p: BoundPipeline {
            head: nurt_contracts::bound::BoundHead::Ref(BoundRef {
                name: "files".to_owned(),
                fields: Vec::new(),
            }),
            stages: Vec::new(),
            span: None,
        },
    };
    assert_eq!(eval_err(&sub).code, ErrorCode::Type);
}

#[test]
fn compares_fields_with_literals_of_the_field_type() {
    let cases = [
        (
            bin(BinOp::Gt, field("size"), BoundExpr::Size { v: 1_000 }),
            true,
        ),
        (bin(BinOp::Lt, field("size"), text("4kb")), true),
        (bin(BinOp::Gt, field("modified"), text("2026-11-01")), true),
        (bin(BinOp::Le, text("2026-11-01"), field("modified")), true),
        (
            bin(BinOp::Eq, field("path"), text("/srv/app/src/main.rs")),
            true,
        ),
        (bin(BinOp::Eq, field("name"), text("main.rs")), true),
        (bin(BinOp::Ne, field("name"), text("lib.rs")), true),
        (
            bin(
                BinOp::Eq,
                field("type"),
                BoundExpr::Enum {
                    v: "directory".to_owned(),
                },
            ),
            false,
        ),
        (bin(BinOp::Eq, field("type"), text("file")), true),
        (bin(BinOp::Ge, int(2), BoundExpr::Num { v: 2.0 }), true),
        (bin(BinOp::Lt, field("link_target"), int(1)), false),
        (bin(BinOp::Eq, field("link_target"), BoundExpr::Null), true),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval_ok(&expr), Value::Bool(expected), "{expr:?}");
    }
}

#[test]
fn rejects_comparisons_across_type_families() {
    let error = eval_err(&bin(BinOp::Gt, field("size"), int(10)));
    assert_eq!(error.code, ErrorCode::Type);
    assert_eq!(error.expected, Some(json!(["size"])));
    assert_eq!(error.actual, Some(json!("int")));
    let error = eval_err(&bin(BinOp::Lt, int(1), text("a")));
    assert_eq!(error.expected, Some(json!(["int", "number"])));
    assert_eq!(
        eval_err(&bin(BinOp::Gt, field("modified"), text("garbage"))).code,
        ErrorCode::InvalidValue
    );
}

#[test]
fn reports_unknown_fields_with_the_fields_of_the_record() {
    let error = eval_err(&field("sise"));
    assert_eq!(error.code, ErrorCode::UnknownField);
    assert_eq!(error.found.as_deref(), Some("sise"));
    assert_eq!(
        error.expected,
        Some(json!([
            "path", "name", "size", "modified", "type", "hidden"
        ]))
    );
    assert!(error.message.contains("entry"), "{}", error.message);
    let outside = Evaluator::new()
        .eval(&field("size"), None, &env())
        .unwrap_err();
    assert_eq!(outside.code, ErrorCode::UnknownField);
}

#[test]
fn reads_fields_of_nested_values() {
    let map = BoundExpr::Map {
        entries: vec![BoundMapEntry {
            k: "inner".to_owned(),
            v: list(vec![int(4), int(5)]),
        }],
    };
    let inner = BoundExpr::Field {
        name: "inner".to_owned(),
        of: Some(Box::new(map.clone())),
    };
    let second = BoundExpr::Field {
        name: "1".to_owned(),
        of: Some(Box::new(inner)),
    };
    assert_eq!(eval_ok(&second), Value::Int(5));
    let missing = BoundExpr::Field {
        name: "absent".to_owned(),
        of: Some(Box::new(map)),
    };
    assert_eq!(eval_ok(&missing), Value::Null);
    let scalar = BoundExpr::Field {
        name: "x".to_owned(),
        of: Some(Box::new(int(1))),
    };
    assert_eq!(eval_err(&scalar).code, ErrorCode::Type);
}

#[test]
fn reads_named_results_and_spreads_lists() {
    let paths = eval_ok(&reference("files", &["path"]));
    assert_eq!(
        paths,
        Value::List(vec![
            Value::Path("/srv/a.rs".to_owned()),
            Value::Path("/srv/b.rs".to_owned())
        ])
    );
    let spread_list = list(vec![spread(reference("files", &["path"])), text("/x")]);
    assert_eq!(eval_ok(&spread_list).as_list().map(<[Value]>::len), Some(3));
    let largest = call("max", vec![spread(reference("files", &["size"]))]);
    assert_eq!(eval_ok(&largest), Value::Size(Size::from_bytes(30)));
    let first = reference("files", &["0", "size"]);
    assert_eq!(eval_ok(&first), Value::Size(Size::from_bytes(10)));
    let error = eval_err(&reference("fiels", &[]));
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert_eq!(error.found.as_deref(), Some("@fiels"));
    assert_eq!(eval_err(&spread_list_of(int(1))).code, ErrorCode::Type);
}

fn spread_list_of(expr: BoundExpr) -> BoundExpr {
    list(vec![spread(expr)])
}

#[test]
fn logical_operators_short_circuit() {
    let failing = bin(BinOp::Eq, division_by_zero(), int(1));
    let cases = [
        (bin(BinOp::And, boolean(false), failing.clone()), false),
        (bin(BinOp::Or, boolean(true), failing.clone()), true),
        (bin(BinOp::And, boolean(true), boolean(true)), true),
        (bin(BinOp::Or, boolean(false), boolean(false)), false),
        (bin(BinOp::And, BoundExpr::Null, boolean(true)), false),
        (bin(BinOp::Or, BoundExpr::Null, boolean(true)), true),
        (un(UnOp::Not, BoundExpr::Null), true),
        (un(UnOp::Not, field("hidden")), true),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval_ok(&expr), Value::Bool(expected), "{expr:?}");
    }
    assert_eq!(
        eval_err(&bin(BinOp::And, boolean(true), failing)).code,
        ErrorCode::InvalidValue
    );
    let error = eval_err(&bin(BinOp::Or, int(1), boolean(true)));
    assert_eq!(error.code, ErrorCode::Type);
    assert_eq!(error.expected, Some(json!(["bool"])));
    assert_eq!(eval_err(&un(UnOp::Not, int(1))).code, ErrorCode::Type);
}

#[test]
fn matches_text_against_regular_expressions() {
    let cases = [
        (bin(BinOp::Match, field("name"), regex(r"\.rs$", "")), true),
        (
            bin(BinOp::NotMatch, field("name"), regex(r"\.rs$", "")),
            false,
        ),
        (bin(BinOp::Match, field("path"), regex("MAIN", "i")), true),
        (bin(BinOp::Match, field("path"), regex("MAIN", "")), false),
        (
            bin(BinOp::Match, field("link_target"), regex("a", "")),
            false,
        ),
        (bin(BinOp::Match, field("type"), regex("^file$", "")), true),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval_ok(&expr), Value::Bool(expected), "{expr:?}");
    }
    let error = eval_err(&bin(BinOp::Match, field("name"), text("main")));
    assert_eq!(error.code, ErrorCode::Type);
    assert_eq!(error.expected, Some(json!(["regex"])));
    assert_eq!(
        eval_err(&bin(BinOp::Match, field("name"), regex("(", ""))).code,
        ErrorCode::Regex
    );
    assert_eq!(
        eval_err(&bin(BinOp::Match, field("size"), regex("a", ""))).code,
        ErrorCode::Type
    );
    assert_eq!(
        eval_err(&bin(BinOp::Match, field("name"), int(1))).code,
        ErrorCode::Type
    );
}

#[test]
fn tests_membership_in_lists_ranges_and_maps() {
    let map = BoundExpr::Map {
        entries: vec![BoundMapEntry {
            k: "a".to_owned(),
            v: int(1),
        }],
    };
    let cases = [
        (
            bin(BinOp::In, int(3), list(vec![int(1), int(2), int(3)])),
            true,
        ),
        (bin(BinOp::In, text("rs"), list(vec![text("go")])), false),
        (
            bin(
                BinOp::In,
                int(3),
                BoundExpr::Range {
                    lo: Some(1),
                    hi: Some(2),
                },
            ),
            false,
        ),
        (
            bin(
                BinOp::In,
                int(5),
                BoundExpr::Range {
                    lo: Some(5),
                    hi: None,
                },
            ),
            true,
        ),
        (
            bin(
                BinOp::In,
                BoundExpr::Null,
                BoundExpr::Range { lo: None, hi: None },
            ),
            false,
        ),
        (bin(BinOp::In, text("a"), map), true),
        (bin(BinOp::In, int(1), BoundExpr::Null), false),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval_ok(&expr), Value::Bool(expected), "{expr:?}");
    }
    let big = Value::BigInt("99999999999999999999".to_owned());
    let env = FixedEnv::new(instant(NOW), 0).with_name("big", big);
    let open_end = bin(
        BinOp::In,
        reference("big", &[]),
        BoundExpr::Range {
            lo: Some(0),
            hi: None,
        },
    );
    assert_eq!(
        Evaluator::new().eval(&open_end, None, &env).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        eval_err(&bin(BinOp::In, int(3), text("abc"))).code,
        ErrorCode::Type
    );
    assert_eq!(
        eval_err(&bin(
            BinOp::In,
            text("a"),
            BoundExpr::Range { lo: None, hi: None }
        ))
        .code,
        ErrorCode::Type
    );
}

#[test]
fn applies_arithmetic_operators() {
    let cases = [
        (
            bin(BinOp::Add, field("size"), BoundExpr::Size { v: 1_000 }),
            Value::Size(Size::from_bytes(3_048)),
        ),
        (bin(BinOp::Sub, int(5), int(7)), Value::Int(-2)),
        (bin(BinOp::Mul, int(6), int(7)), Value::Int(42)),
        (bin(BinOp::Div, int(7), int(2)), Value::Number(3.5)),
        (
            bin(BinOp::Sub, call("now", Vec::new()), field("modified")),
            Value::Duration(Duration::from_millis(957_600_000)),
        ),
        (un(UnOp::Neg, int(5)), Value::Int(-5)),
        (bin(BinOp::Add, text("a"), text("b")), Value::from("ab")),
    ];
    for (expr, expected) in cases {
        assert_eq!(eval_ok(&expr), expected, "{expr:?}");
    }
    assert_eq!(
        eval_err(&bin(BinOp::Sub, field("modified"), text("2026-11-01"))).code,
        ErrorCode::Type
    );
    assert_eq!(
        eval_err(&un(UnOp::Neg, field("name"))).code,
        ErrorCode::Type
    );
    let error = eval_err(&division_by_zero());
    assert_eq!(error.problem.as_deref(), Some("division_by_zero"));
}

#[test]
fn eval_bool_treats_null_as_false_and_rejects_other_types() {
    let evaluator = Evaluator::new();
    let record = entry();
    assert!(
        !evaluator
            .eval_bool(&BoundExpr::Null, Some(&record), &env())
            .unwrap()
    );
    assert!(
        evaluator
            .eval_bool(&field("type"), Some(&record), &env())
            .is_err()
    );
}

fn call_ok(function: &str, args: Vec<BoundExpr>) -> Value {
    eval_ok(&call(function, args))
}

fn call_code(function: &str, args: Vec<BoundExpr>) -> ErrorCode {
    eval_err(&call(function, args)).code
}

fn bytes(data: &[u8]) -> Value {
    Value::Bytes(Bytes::from(data))
}

#[test]
fn text_functions_keep_the_type_of_their_argument() {
    assert_eq!(
        call_ok("lower", vec![text("\u{c0}B")]),
        Value::from("\u{e0}b")
    );
    assert_eq!(call_ok("upper", vec![text("ab")]), Value::from("AB"));
    assert_eq!(call_ok("trim", vec![text("  a \n")]), Value::from("a"));
    assert_eq!(
        call_ok("lower", vec![field("path")]),
        Value::Path("/srv/app/src/main.rs".to_owned())
    );
    assert_eq!(
        call_ok("upper", vec![field("type")]),
        Value::Enum("FILE".to_owned())
    );
    assert_eq!(call_ok("lower", vec![BoundExpr::Null]), Value::Null);
    assert_eq!(call_code("lower", vec![int(1)]), ErrorCode::Type);
    assert_eq!(call_code("lower", Vec::new()), ErrorCode::Arity);
    assert_eq!(
        call_ok("replace_text", vec![text("a-b-c"), text("-"), text("+")]),
        Value::from("a+b+c")
    );
    assert_eq!(
        call_code("replace_text", vec![text("a"), text(""), text("b")]),
        ErrorCode::InvalidValue
    );
}

#[test]
fn len_counts_characters_items_fields_and_bytes() {
    assert_eq!(call_ok("len", vec![text("h\u{e9}llo")]), Value::Int(5));
    assert_eq!(
        call_ok("len", vec![list(vec![int(1), int(2)])]),
        Value::Int(2)
    );
    assert_eq!(
        call_ok("len", vec![reference("files", &["0"])]),
        Value::Int(2)
    );
    assert_eq!(
        call_ok("len", vec![call("base64_decode", vec![text("bnVydA==")])]),
        Value::Int(4)
    );
    assert_eq!(call_ok("len", vec![BoundExpr::Null]), Value::Null);
    assert_eq!(call_code("len", vec![int(1)]), ErrorCode::Type);
}

#[test]
fn numeric_functions_handle_bounds() {
    assert_eq!(call_ok("abs", vec![int(-5)]), Value::Int(5));
    assert_eq!(
        call_ok("abs", vec![BoundExpr::Num { v: -1.5 }]),
        Value::Number(1.5)
    );
    assert_eq!(
        call_ok("abs", vec![un(UnOp::Neg, BoundExpr::Dur { v: 3_000 })]),
        Value::Duration(Duration::from_millis(3_000))
    );
    assert_eq!(
        call_code("abs", vec![int(i64::MIN)]),
        ErrorCode::InvalidValue
    );
    assert_eq!(call_code("abs", vec![text("a")]), ErrorCode::Type);
    assert_eq!(
        call_ok("round", vec![BoundExpr::Num { v: 2.5 }]),
        Value::Int(3)
    );
    assert_eq!(
        call_ok("round", vec![BoundExpr::Num { v: -2.5 }]),
        Value::Int(-3)
    );
    assert_eq!(
        call_ok("round", vec![BoundExpr::Num { v: 1.25 }, int(1)]),
        Value::Number(1.3)
    );
    assert_eq!(call_ok("round", vec![int(7)]), Value::Int(7));
    assert_eq!(
        call_code("round", vec![BoundExpr::Num { v: 1e300 }]),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        call_code("round", vec![BoundExpr::Num { v: 1.5 }, int(16)]),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        call_code("round", vec![BoundExpr::Num { v: 1.5 }, text("a")]),
        ErrorCode::Type
    );
    assert_eq!(call_code("round", Vec::new()), ErrorCode::Arity);
}

#[test]
fn min_and_max_skip_null_and_need_one_type_family() {
    assert_eq!(call_ok("min", vec![int(3), int(1), int(2)]), Value::Int(1));
    assert_eq!(
        call_ok("max", vec![list(vec![int(1), int(5), int(2)])]),
        Value::Int(5)
    );
    assert_eq!(call_ok("min", vec![BoundExpr::Null, int(2)]), Value::Int(2));
    assert_eq!(call_ok("max", vec![list(Vec::new())]), Value::Null);
    assert_eq!(call_code("min", Vec::new()), ErrorCode::Arity);
    assert_eq!(call_code("min", vec![int(1), text("a")]), ErrorCode::Type);
}

#[test]
fn containment_functions_accept_text_lists_and_maps() {
    assert_eq!(
        call_ok("contains", vec![text("abc"), text("b")]),
        Value::Bool(true)
    );
    assert_eq!(
        call_ok("contains", vec![list(vec![int(1), int(2)]), int(2)]),
        Value::Bool(true)
    );
    assert_eq!(
        call_ok("contains", vec![BoundExpr::Null, text("a")]),
        Value::Bool(false)
    );
    assert_eq!(
        call_code("contains", vec![int(1), text("a")]),
        ErrorCode::Type
    );
    assert_eq!(
        call_ok("starts_with", vec![field("path"), text("/srv")]),
        Value::Bool(true)
    );
    assert_eq!(
        call_ok("ends_with", vec![field("name"), text(".rs")]),
        Value::Bool(true)
    );
    assert_eq!(
        call_ok("starts_with", vec![BoundExpr::Null, text("x")]),
        Value::Bool(false)
    );
    assert_eq!(
        call_code("ends_with", vec![field("name"), int(1)]),
        ErrorCode::Type
    );
}

#[test]
fn regex_function_builds_and_checks_expressions() {
    let built = call_ok("regex", vec![text("a+"), text("i")]);
    assert_eq!(
        built,
        Value::Regex(nurt_contracts::value::Regex::new("a+", "i").unwrap())
    );
    let matched = bin(
        BinOp::Match,
        text("AAA"),
        call("regex", vec![text("^a+$"), text("i")]),
    );
    assert_eq!(eval_ok(&matched), Value::Bool(true));
    assert_eq!(call_code("regex", vec![text("(")]), ErrorCode::Regex);
    assert_eq!(
        call_code("regex", vec![text("a"), text("q")]),
        ErrorCode::Regex
    );
    assert_eq!(call_code("regex", vec![int(1)]), ErrorCode::Type);
}

#[test]
fn time_functions_use_the_environment_clock() {
    let now = Value::Instant(instant(NOW));
    assert_eq!(call_ok("now", Vec::new()), now);
    assert_eq!(call_code("now", vec![int(1)]), ErrorCode::Arity);
    assert_eq!(
        call_ok("ago", vec![BoundExpr::Dur { v: 604_800_000 }]),
        Value::Instant(instant("2026-11-14T10:00:00Z"))
    );
    assert_eq!(
        call_ok("ago", vec![text("7d")]),
        Value::Instant(instant("2026-11-14T10:00:00Z"))
    );
    assert_eq!(
        call_ok("from_now", vec![text("2h")]),
        Value::Instant(instant("2026-11-21T12:00:00Z"))
    );
    assert_eq!(
        call_code("ago", vec![reference("label", &[])]),
        ErrorCode::Type
    );
    assert_eq!(
        call_code("ago", vec![text("seven days")]),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        call_code(
            "from_now",
            vec![BoundExpr::Dur {
                v: 9_000_000_000_000_000
            }]
        ),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        call_ok("time", vec![text("2026-11-21")]),
        Value::Instant(instant("2026-11-21T00:00:00Z"))
    );
    assert_eq!(
        call_code("time", vec![text("noon")]),
        ErrorCode::InvalidValue
    );
    assert_eq!(call_code("time", vec![int(1)]), ErrorCode::Type);
    let local = FixedEnv::new(instant(NOW), 0).with_tz(TimeZone::fixed(tz::offset(2)));
    let midnight = Evaluator::new()
        .eval(&call("time", vec![text("2026-11-21")]), None, &local)
        .unwrap();
    assert_eq!(midnight, Value::Instant(instant("2026-11-20T22:00:00Z")));
}

#[test]
fn path_functions_work_on_canonical_paths() {
    assert_eq!(
        call_ok("parent", vec![text("/srv/app")]),
        Value::Path("/srv".to_owned())
    );
    assert_eq!(
        call_ok("parent", vec![field("path")]),
        Value::Path("/srv/app/src".to_owned())
    );
    assert_eq!(
        call_ok("parent", vec![BoundExpr::Path { v: "/".to_owned() }]),
        Value::Null
    );
    assert_eq!(call_code("parent", vec![int(1)]), ErrorCode::Type);
    assert_eq!(
        call_ok("join_path", vec![text("/srv"), text("app"), text("../lib")]),
        Value::Path("/srv/lib".to_owned())
    );
    assert_eq!(call_code("join_path", vec![text("/srv")]), ErrorCode::Arity);
    assert_eq!(
        call_code("join_path", vec![text("/srv"), int(1)]),
        ErrorCode::Type
    );
}

#[test]
fn encoding_functions_convert_text_and_bytes() {
    assert_eq!(
        call_ok("base64_encode", vec![text("nurt")]),
        Value::from("bnVydA==")
    );
    assert_eq!(
        call_ok("base64_decode", vec![text("bnVydA==")]),
        bytes(b"nurt")
    );
    assert_eq!(
        call_code("base64_decode", vec![text("!!")]),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        call_ok(
            "base64_encode",
            vec![call("base64_decode", vec![text("AAE=")])]
        ),
        Value::from("AAE=")
    );
    assert_eq!(call_ok("hex", vec![text("A")]), Value::from("41"));
    assert_eq!(call_ok("hex", vec![int(255)]), Value::from("ff"));
    assert_eq!(call_ok("hex", vec![int(-255)]), Value::from("-ff"));
    assert_eq!(
        call_ok("hex", vec![call("base64_decode", vec![text("D6A=")])]),
        Value::from("0fa0")
    );
    assert_eq!(
        call_code("hex", vec![BoundExpr::Num { v: 1.0 }]),
        ErrorCode::Type
    );
    assert_eq!(
        call_ok("idna_encode", vec![text("b\u{fc}cher.de")]),
        Value::from("xn--bcher-kva.de")
    );
    assert_eq!(
        call_ok("idna_decode", vec![text("xn--bcher-kva.de")]),
        Value::from("b\u{fc}cher.de")
    );
    assert_eq!(
        call_code("idna_encode", vec![text("bad host")]),
        ErrorCode::InvalidValue
    );
    assert_eq!(call_code("idna_decode", vec![int(1)]), ErrorCode::Type);
}

#[test]
fn random_functions_are_reproducible_for_a_seed() {
    let evaluate = |expr: &BoundExpr| {
        Evaluator::new()
            .eval(expr, None, &FixedEnv::new(instant(NOW), 99))
            .unwrap()
    };
    let id = evaluate(&call("uuid", Vec::new()));
    assert_eq!(id, evaluate(&call("uuid", Vec::new())));
    let id = id.as_str().unwrap().to_owned();
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().nth(14), Some('4'));
    let Value::Number(fraction) = evaluate(&call("random", Vec::new())) else {
        panic!("random() is not a number");
    };
    assert!((0.0..1.0).contains(&fraction));
    let die = call(
        "random",
        vec![BoundExpr::Range {
            lo: Some(1),
            hi: Some(6),
        }],
    );
    for seed in 0..32 {
        let env = FixedEnv::new(instant(NOW), seed);
        let Value::Int(roll) = Evaluator::new().eval(&die, None, &env).unwrap() else {
            panic!("random(range) is not an int");
        };
        assert!((1..=6).contains(&roll));
    }
    let full = call(
        "random",
        vec![BoundExpr::Range {
            lo: Some(i64::MIN),
            hi: Some(i64::MAX),
        }],
    );
    assert!(matches!(evaluate(&full), Value::Int(_)));
    assert_eq!(
        call_code(
            "random",
            vec![BoundExpr::Range {
                lo: Some(5),
                hi: None
            }]
        ),
        ErrorCode::InvalidValue
    );
    assert_eq!(call_code("random", vec![int(1)]), ErrorCode::Type);
    assert_eq!(call_code("uuid", vec![int(1)]), ErrorCode::Arity);
}

#[test]
fn every_listed_function_is_known_and_others_are_not() {
    assert_eq!(FUNCTIONS.len(), 26);
    for function in FUNCTIONS {
        let result = eval(&call(
            function,
            vec![
                BoundExpr::Null,
                BoundExpr::Null,
                BoundExpr::Null,
                BoundExpr::Null,
            ],
        ));
        if let Err(error) = result {
            assert_ne!(error.code, ErrorCode::UnknownName, "{function}");
        }
    }
    let error = eval_err(&call("lowercase", vec![text("A")]));
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert_eq!(error.problem.as_deref(), Some("unknown_function"));
}

#[test]
fn environment_defaults_give_utc_and_no_names() {
    struct Minimal;
    impl EvalEnv for Minimal {
        fn now(&self) -> Instant {
            Instant::UNIX_EPOCH
        }

        fn random_u64(&self) -> u64 {
            0
        }
    }
    let env = Minimal;
    assert_eq!(env.named("files"), None);
    assert!(!env.is_known_field(RecordKind::Entry, "size"));
    assert_eq!(
        Evaluator::new()
            .eval(&call("time", vec![text("1970-01-02")]), None, &env)
            .unwrap(),
        Value::Instant(instant("1970-01-02T00:00:00Z"))
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
