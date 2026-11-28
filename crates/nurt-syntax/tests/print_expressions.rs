// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/print_expressions.rs
// Responsibility:   Parentheses, argument boundaries, line layout and trees outside the parser image.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parentheses, argument boundaries, line layout and trees outside the
//! parser image.
//!
//! Every printed text that the parser of variant A should accept is also
//! parsed back and compared with the tree.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::{
    Arg, AstDocument, BinOp, Expr, Head, KeySeg, Let, MapEntry, Pipeline, Program, Ref, Stage,
    Statement, UnOp, strip_for_hash,
};
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print::{self, MAX_LINE_WIDTH, MAX_NESTING_DEPTH, PrintError};

fn ident(name: &str) -> Expr {
    Expr::Ident {
        v: name.to_owned(),
        span: None,
    }
}

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

fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Bin {
        op,
        l: Box::new(l),
        r: Box::new(r),
        span: None,
    }
}

fn not(x: Expr) -> Expr {
    Expr::Un {
        op: UnOp::Not,
        x: Box::new(x),
        span: None,
    }
}

fn neg(x: Expr) -> Expr {
    Expr::Un {
        op: UnOp::Neg,
        x: Box::new(x),
        span: None,
    }
}

fn range(lo: Option<i64>, hi: Option<i64>) -> Expr {
    Expr::Range { lo, hi, span: None }
}

fn list(items: Vec<Expr>) -> Expr {
    Expr::List { items, span: None }
}

fn map(entries: &[(&str, Expr)]) -> Expr {
    Expr::Map {
        entries: entries
            .iter()
            .map(|(k, v)| MapEntry {
                k: (*k).to_owned(),
                v: v.clone(),
                span: None,
            })
            .collect(),
        span: None,
    }
}

fn field(of: Expr, name: &str) -> Expr {
    Expr::Field {
        name: name.to_owned(),
        of: Some(Box::new(of)),
        span: None,
    }
}

fn reference(name: &str, fields: &[&str]) -> Ref {
    Ref {
        name: name.to_owned(),
        fields: fields.iter().map(|f| (*f).to_owned()).collect(),
        span: None,
    }
}

fn pos(value: Expr) -> Arg {
    Arg {
        name: None,
        value,
        span: None,
    }
}

fn named(name: &str, value: Expr) -> Arg {
    Arg {
        name: Some(name.to_owned()),
        value,
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

fn pipeline(head: Stage, stages: Vec<Stage>) -> Pipeline {
    Pipeline {
        head: Head::Stage(head),
        stages,
        span: None,
    }
}

fn sub(pipeline: Pipeline) -> Expr {
    Expr::Sub {
        p: pipeline,
        span: None,
    }
}

fn program(statements: Vec<Statement>) -> AstDocument {
    AstDocument::new(Program { statements })
}

fn one(head: Stage) -> AstDocument {
    program(vec![Statement::Pipeline(pipeline(head, Vec::new()))])
}

fn hash_form(document: &AstDocument) -> serde_json::Value {
    strip_for_hash(&serde_json::to_value(document).expect("tree serializes"))
}

/// Prints `document` in variant A, checks the text and that it reads back.
fn assert_a(document: &AstDocument, expected: &str) {
    let printed = print::try_print(document, Variant::A).expect("printable tree");
    assert_eq!(printed, expected);
    let parsed = parse::parse(&printed, Variant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{error}\n{printed}"));
    assert_eq!(hash_form(&parsed), hash_form(document), "{printed}");
}

fn assert_expr(expr: Expr, expected: &str) {
    assert_eq!(print::print_expr(&expr, Variant::A), expected);
    assert_a(
        &one(stage("filter", vec![pos(expr)])),
        &format!("filter {expected}\n"),
    );
}

#[test]
fn not_binds_looser_than_comparisons() {
    let compare = bin(
        BinOp::Gt,
        ident("size"),
        Expr::Size {
            v: 1_000_000,
            span: None,
        },
    );
    assert_expr(not(compare.clone()), "not size > 1mb");
    assert_expr(bin(BinOp::Eq, not(ident("a")), ident("b")), "(not a) == b");
    assert_expr(bin(BinOp::Eq, ident("a"), not(ident("b"))), "a == (not b)");
    assert_expr(not(not(ident("x"))), "not (not x)");
    assert_expr(
        bin(BinOp::And, not(ident("a")), compare),
        "not a and size > 1mb",
    );
}

#[test]
fn comparisons_are_non_associative() {
    let inner = bin(BinOp::Lt, ident("a"), ident("b"));
    assert_expr(bin(BinOp::Eq, inner.clone(), ident("c")), "(a < b) == c");
    assert_expr(bin(BinOp::Eq, ident("c"), inner), "c == (a < b)");
}

#[test]
fn left_associative_levels_parenthesize_the_right_operand() {
    let ab = bin(BinOp::Sub, ident("a"), ident("b"));
    assert_expr(bin(BinOp::Sub, ab.clone(), ident("c")), "a - b - c");
    assert_expr(bin(BinOp::Sub, ident("c"), ab), "c - (a - b)");
    let or = bin(BinOp::Or, ident("a"), ident("b"));
    assert_expr(bin(BinOp::And, or.clone(), ident("c")), "(a or b) and c");
    assert_expr(bin(BinOp::Or, ident("c"), or), "c or (a or b)");
    let sum = bin(BinOp::Add, int(1), int(2));
    assert_expr(bin(BinOp::Mul, sum, int(3)), "(1 + 2) * 3");
    assert_expr(
        bin(BinOp::Add, int(1), bin(BinOp::Mul, int(2), int(3))),
        "1 + 2 * 3",
    );
    assert_expr(bin(BinOp::Div, ident("a"), ident("b")), "a / b");
}

#[test]
fn unary_minus_is_attached_and_nests_with_parentheses() {
    assert_expr(neg(int(1)), "-1");
    assert_expr(neg(neg(ident("x"))), "-(-x)");
    assert_expr(neg(bin(BinOp::Add, int(1), int(2))), "-(1 + 2)");
    assert_expr(bin(BinOp::Sub, ident("a"), neg(int(1))), "a - -1");
    assert_expr(bin(BinOp::Mul, neg(ident("a")), ident("b")), "-a * b");
    assert_expr(neg(field(ident("file"), "size")), "-file.size");
    assert_eq!(
        print::print_expr(&neg(range(Some(1), Some(5))), Variant::A),
        "-(1..5)"
    );
}

#[test]
fn field_access_parenthesizes_operands_other_than_names_and_calls() {
    let call = Expr::Call {
        function: "ago".to_owned(),
        args: vec![Expr::Dur {
            v: 86_400_000,
            span: None,
        }],
        span: None,
    };
    assert_expr(field(ident("file"), "size"), "file.size");
    assert_expr(field(field(ident("a"), "b"), "c"), "a.b.c");
    assert_expr(field(call, "x"), "ago(1d).x");
    assert_expr(field(Expr::Ref(reference("x", &[])), "path"), "(@x).path");
    assert_expr(field(text("abc"), "len"), "(\"abc\").len");
    assert_expr(
        Expr::Ref(reference("recent", &["path", "name"])),
        "@recent.path.name",
    );
}

#[test]
fn argument_boundaries_of_variant_a_read_back() {
    assert_a(
        &one(stage("first", vec![pos(int(20)), pos(neg(int(1)))])),
        "first 20 -1\n",
    );
    assert_a(
        &one(stage("first", vec![pos(bin(BinOp::Sub, int(20), int(1)))])),
        "first 20 - 1\n",
    );
    let regex = Expr::Regex {
        v: "re".to_owned(),
        flags: "i".to_owned(),
        span: None,
    };
    assert_a(
        &one(stage("search", vec![pos(ident("x")), pos(regex)])),
        "search x /re/i\n",
    );
    assert_a(
        &one(stage("check", vec![pos(ident("a")), pos(not(ident("b")))])),
        "check a (not b)\n",
    );
    assert_a(
        &one(stage("check", vec![pos(not(ident("b")))])),
        "check not b\n",
    );
    assert_a(
        &one(stage("lines", vec![pos(range(Some(2), None)), pos(int(5))])),
        "lines (2..) 5\n",
    );
    assert_a(
        &one(stage("lines", vec![pos(int(5)), pos(range(None, Some(9)))])),
        "lines 5 (..9)\n",
    );
    assert_a(
        &one(stage("lines", vec![pos(range(Some(2), None))])),
        "lines 2..\n",
    );
    assert_a(
        &one(stage("replace", vec![named("in", list(vec![text("/a")]))])),
        "replace in: [\"/a\"]\n",
    );
}

#[test]
fn regular_expression_starting_with_a_hash_reads_back_after_another_argument() {
    let regex = Expr::Regex {
        v: "#include".to_owned(),
        flags: "i".to_owned(),
        span: None,
    };
    assert_a(
        &one(stage("search", vec![pos(text("/src")), pos(regex)])),
        "search \"/src\" /#include/i\n",
    );
}

#[test]
fn single_stage_sub_pipeline_guards_its_first_argument() {
    let verify = |args| {
        one(stage(
            "verify",
            vec![pos(sub(pipeline(stage("check", args), Vec::new())))],
        ))
    };
    assert_a(&verify(vec![pos(ident("x"))]), "verify (check x)\n");
    assert_a(&verify(vec![pos(neg(int(1)))]), "verify (check -1)\n");
    assert_a(
        &verify(vec![pos(not(ident("x")))]),
        "verify (check (not x))\n",
    );
    assert_a(
        &verify(vec![pos(range(None, Some(3)))]),
        "verify (check (..3))\n",
    );
    assert_a(
        &verify(vec![named("exit_code", int(0))]),
        "verify (check exit_code: 0)\n",
    );
}

#[test]
fn lists_break_when_nested_or_too_long() {
    assert_expr(list(vec![int(1), int(2)]), "[1, 2]");
    assert_expr(list(vec![list(vec![int(1)])]), "[[1]]");
    assert_a(
        &one(stage(
            "info",
            vec![pos(list(vec![list(vec![int(1)]), int(2)]))],
        )),
        "info [\n  [1],\n  2\n]\n",
    );
    assert_a(
        &one(stage(
            "info",
            vec![pos(list(vec![
                map(&[("k", int(1))]),
                list(vec![list(vec![int(1)]), int(2)]),
            ]))],
        )),
        "info [\n  {k: 1},\n  [\n    [1],\n    2\n  ]\n]\n",
    );
    let fits = "x".repeat(MAX_LINE_WIDTH - "info [\"\"]".len());
    assert_a(
        &one(stage("info", vec![pos(list(vec![text(&fits)]))])),
        &format!("info [\"{fits}\"]\n"),
    );
    let long = format!("{fits}x");
    assert_a(
        &one(stage("info", vec![pos(list(vec![text(&long)]))])),
        &format!("info [\n  \"{long}\"\n]\n"),
    );
}

#[test]
fn line_width_counts_unicode_scalar_values() {
    let wide = "ż".repeat(MAX_LINE_WIDTH - "info [\"\"]".len());
    assert_a(
        &one(stage("info", vec![pos(list(vec![text(&wide)]))])),
        &format!("info [\"{wide}\"]\n"),
    );
}

#[test]
fn maps_break_only_when_too_long() {
    assert_a(
        &one(stage(
            "info",
            vec![pos(map(&[("in", int(1)), ("key with space", int(2))]))],
        )),
        "info {in: 1, \"key with space\": 2}\n",
    );
    let value = "v".repeat(MAX_LINE_WIDTH);
    assert_a(
        &one(stage(
            "info",
            vec![pos(map(&[("a", text(&value)), ("b", int(1))]))],
        )),
        &format!("info {{\n  a: \"{value}\",\n  b: 1\n}}\n"),
    );
}

#[test]
fn pipelines_stack_when_the_head_has_arguments_or_the_line_is_long() {
    let head = stage("find", vec![pos(text("/srv"))]);
    let document = program(vec![Statement::Pipeline(pipeline(
        head,
        vec![stage("count", Vec::new())],
    ))]);
    assert_a(&document, "find \"/srv\"\n|> count\n");
    let short = program(vec![Statement::Pipeline(pipeline(
        stage("jobs", Vec::new()),
        vec![stage("count", Vec::new())],
    ))]);
    assert_a(&short, "jobs |> count\n");
    let name = "f".repeat(MAX_LINE_WIDTH / 2);
    let long = program(vec![Statement::Pipeline(pipeline(
        stage("jobs", Vec::new()),
        vec![
            stage("filter", vec![pos(ident(&name))]),
            stage("sort", vec![pos(ident(&name))]),
        ],
    ))]);
    assert_a(&long, &format!("jobs\n|> filter {name}\n|> sort {name}\n"));
    let stacked = Statement::Let(Let {
        name: "recent".to_owned(),
        value: Pipeline {
            head: Head::Ref(reference("x", &[])),
            stages: vec![stage("count", Vec::new())],
            span: None,
        },
        span: None,
    });
    assert_a(&program(vec![stacked]), "let recent = @x |> count\n");
}

#[test]
fn literals_print_in_canonical_spelling() {
    assert_expr(Expr::Num { v: 3.0, span: None }, "3.0");
    assert_expr(Expr::Num { v: 0.1, span: None }, "0.1");
    assert_expr(
        Expr::Size {
            v: 1_000_000,
            span: None,
        },
        "1mb",
    );
    assert_expr(
        Expr::Size {
            v: 1024,
            span: None,
        },
        "1kib",
    );
    assert_expr(
        Expr::Size {
            v: 1500,
            span: None,
        },
        "1.5kb",
    );
    assert_expr(
        Expr::Dur {
            v: 604_800_000,
            span: None,
        },
        "7d",
    );
    assert_expr(
        Expr::Dur {
            v: 90_000,
            span: None,
        },
        "1m30s",
    );
    assert_expr(Expr::Dur { v: 0, span: None }, "0s");
    assert_expr(text("C:\\proj"), "r\"C:\\proj\"");
    assert_expr(text("tab\there"), "\"tab\\there\"");
    assert_expr(
        Expr::Bool {
            v: true,
            span: None,
        },
        "true",
    );
    assert_expr(Expr::Null { span: None }, "null");
    assert_expr(range(None, None), "..");
}

fn unprintable_node(expr: Expr) -> &'static str {
    match print::try_print(&one(stage("info", vec![pos(expr)])), Variant::A) {
        Err(PrintError::Unprintable { node, .. }) => node,
        other => panic!("expected an unprintable node, got {other:?}"),
    }
}

#[test]
fn values_outside_the_parser_image_are_rejected() {
    let cases = [
        (int(-1), "int"),
        (int(1 << 53), "int"),
        (
            Expr::Num {
                v: -0.5,
                span: None,
            },
            "num",
        ),
        (
            Expr::Size {
                v: 1 << 60,
                span: None,
            },
            "size",
        ),
        (range(Some(-1), None), "range"),
        (neg(range(Some(1), Some(5))), "un"),
        (ident("in"), "ident"),
        (ident("Upper"), "ident"),
        (
            Expr::Spread {
                x: Box::new(ident("a")),
                span: None,
            },
            "spread",
        ),
        (
            Expr::Field {
                name: "a".to_owned(),
                of: None,
                span: None,
            },
            "field",
        ),
        (map(&[("a", int(1)), ("a", int(2))]), "map"),
        (
            Expr::Key {
                segs: vec![KeySeg::Name("a".to_owned())],
                span: None,
            },
            "key",
        ),
    ];
    for (expr, node) in cases {
        let shown = format!("{expr:?}");
        assert_eq!(unprintable_node(expr), node, "{shown}");
    }
}

#[test]
fn regular_expressions_without_a_text_form_are_rejected() {
    for (pattern, flags) in [
        ("", ""),
        ("a\nb", ""),
        ("a\\", ""),
        ("a\\/b", ""),
        ("a", "xi"),
        (" a", ""),
    ] {
        let regex = Expr::Regex {
            v: pattern.to_owned(),
            flags: flags.to_owned(),
            span: None,
        };
        assert_eq!(unprintable_node(regex), "regex", "{pattern:?} {flags:?}");
    }
}

#[test]
fn sub_pipelines_programs_and_commands_outside_the_parser_image_are_rejected() {
    assert_eq!(
        unprintable_node(sub(pipeline(stage("count", Vec::new()), Vec::new()))),
        "sub"
    );
    let ref_only = sub(Pipeline {
        head: Head::Ref(reference("x", &[])),
        stages: Vec::new(),
        span: None,
    });
    assert_eq!(unprintable_node(ref_only), "sub");
    assert!(matches!(
        print::try_print(&program(Vec::new()), Variant::A),
        Err(PrintError::Unprintable {
            node: "program",
            ..
        })
    ));
    assert!(matches!(
        print::try_print(&one(stage("Bad", Vec::new())), Variant::A),
        Err(PrintError::Unprintable { node: "stage", .. })
    ));
}

#[test]
fn single_stage_sub_without_arguments_has_a_variant_b_form() {
    let document = one(stage(
        "verify",
        vec![pos(sub(pipeline(stage("count", Vec::new()), Vec::new())))],
    ));
    assert_eq!(
        print::try_print(&document, Variant::B),
        Ok("verify((count()))\n".to_owned())
    );
}

#[test]
fn nesting_beyond_the_limit_is_reported_without_overflow() {
    let mut expr = int(1);
    for _ in 0..(MAX_NESTING_DEPTH * 4) {
        expr = neg(expr);
    }
    let document = one(stage("info", vec![pos(expr)]));
    assert_eq!(
        print::try_print(&document, Variant::A),
        Err(PrintError::TooDeep {
            limit: MAX_NESTING_DEPTH
        })
    );
    assert!(print::print(&document, Variant::A).contains("null"));
}

#[test]
fn left_operand_ending_with_an_open_range_is_parenthesized() {
    assert_expr(bin(BinOp::Div, range(None, None), text("")), "(..) / \"\"");
    assert_expr(
        bin(
            BinOp::Div,
            bin(BinOp::Add, int(1), range(Some(2), None)),
            int(3),
        ),
        "(1 + 2..) / 3",
    );
    assert_expr(bin(BinOp::Div, int(3), range(Some(2), None)), "3 / 2..");
}

#[test]
fn let_names_outside_the_identifier_shape_are_rejected() {
    for name in ["in", "Recent", "1x"] {
        let document = program(vec![Statement::Let(Let {
            name: name.to_owned(),
            value: pipeline(stage("jobs", Vec::new()), Vec::new()),
            span: None,
        })]);
        assert!(
            matches!(
                print::try_print(&document, Variant::A),
                Err(PrintError::Unprintable { node: "let", .. })
            ),
            "{name}"
        );
    }
}

#[test]
fn argument_names_outside_the_identifier_shape_are_rejected_in_every_variant() {
    let document = one(stage("find", vec![named("Glob", text("*.go"))]));
    for variant in [Variant::A, Variant::B, Variant::C] {
        assert!(
            matches!(
                print::try_print(&document, variant),
                Err(PrintError::Unprintable {
                    node: "argument",
                    ..
                })
            ),
            "{variant:?}"
        );
    }
}

/// `verify (verify (… (first 1)))` with `levels` nested sub-pipelines; every
/// sub-pipeline has the stages `extra` after its head.
fn nested_subs(levels: usize, extra: &[Stage]) -> AstDocument {
    let mut value = int(1);
    let mut name = "first";
    for _ in 0..levels {
        value = sub(pipeline(stage(name, vec![pos(value)]), extra.to_vec()));
        name = "verify";
    }
    one(stage("verify", vec![pos(value)]))
}

#[test]
fn nested_sub_pipelines_beyond_the_limit_are_reported_without_overflow() {
    assert_a(&nested_subs(2, &[]), "verify (verify (first 1))\n");
    let count = [stage("count", Vec::new())];
    assert_eq!(
        print::try_print(&nested_subs(2, &count), Variant::B),
        Ok("verify(verify(first(1) |> count()) |> count())\n".to_owned())
    );
    for (variant, extra) in [(Variant::A, &[][..]), (Variant::B, &count[..])] {
        let document = nested_subs(MAX_NESTING_DEPTH, extra);
        assert_eq!(
            print::try_print(&document, variant),
            Err(PrintError::TooDeep {
                limit: MAX_NESTING_DEPTH
            }),
            "{variant:?}"
        );
        assert!(
            print::print(&document, variant).contains("null"),
            "{variant:?}"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
