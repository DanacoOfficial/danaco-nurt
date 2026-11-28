// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_b_props.rs
// Responsibility:   Properties of the call notation parser (variant B).
// Dependencies:     nurt_contracts, proptest, roundtrip_support, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Properties of the call notation parser (variant B): no panic on any
//! input, generated programs parse into the tree they were generated from,
//! and the variant B print of any tree in the image of the parser reads back
//! as that tree; parsed trees have nested and ordered spans.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod roundtrip_support;

use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Stage, Statement, strip_for_hash};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use nurt_syntax::print;
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestCaseError};
use roundtrip_support::{Shape, hash_form};
use serde_json::{Value, json};

/// Number of cases of every property in an optimized build.
const RELEASE_CASES: u32 = 100_000;
/// Number of cases of every property in a debug build, which runs about
/// twenty times slower; the optimized build checks the full number.
const DEBUG_CASES: u32 = 5_000;
/// Number of cases of every property.
const CASES: u32 = if cfg!(debug_assertions) {
    DEBUG_CASES
} else {
    RELEASE_CASES
};
/// Fixed seed, so that every run checks the same inputs.
const SEED: u64 = 0x6e75_7274_0b00;
/// Nesting levels of generated sub-pipelines.
const SUB_PIPELINE_LEVELS: u32 = 3;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(SEED),
        ..ProptestConfig::default()
    }
}

/// Program text and the surface tree it denotes, without spans.
#[derive(Debug, Clone)]
struct Fragment {
    text: String,
    tree: Value,
}

impl Fragment {
    fn new(text: impl Into<String>, tree: Value) -> Self {
        Self {
            text: text.into(),
            tree,
        }
    }
}

fn pick(words: &'static [&'static str]) -> impl Strategy<Value = &'static str> {
    prop::sample::select(words)
}

fn literal() -> impl Strategy<Value = Fragment> {
    prop_oneof![
        (0_u32..1_000).prop_map(|n| Fragment::new(n.to_string(), json!({"t": "int", "v": n}))),
        "[a-z /.*]{0,8}"
            .prop_map(|v| Fragment::new(format!("\"{v}\""), json!({"t": "text", "v": v}))),
        pick(&["size", "name", "count", "desc", "x"])
            .prop_map(|v| Fragment::new(v, json!({"t": "ident", "v": v}))),
        any::<bool>().prop_map(|v| Fragment::new(v.to_string(), json!({"t": "bool", "v": v}))),
        (1_u64..100)
            .prop_map(|n| Fragment::new(format!("{n}kb"), json!({"t": "size", "v": n * 1_000}))),
        (1_u64..60).prop_map(|n| Fragment::new(
            format!("ago({n}m)"),
            json!({"t": "call", "fn": "ago", "args": [{"t": "dur", "v": n * 60_000}]})
        )),
        Just(Fragment::new(
            "/re/i",
            json!({"t": "regex", "v": "re", "flags": "i"})
        )),
    ]
}

/// Binary operations; the spacing variants include a space only before the
/// operator, which variant B reads as a binary operator.
fn expression() -> impl Strategy<Value = Fragment> {
    let operand = prop_oneof![
        (0_u32..1_000).prop_map(|n| Fragment::new(n.to_string(), json!({"t": "int", "v": n}))),
        pick(&["size", "a"]).prop_map(|v| Fragment::new(v, json!({"t": "ident", "v": v}))),
    ];
    let binary = (
        operand.clone(),
        pick(&["==", ">", "+", "-", "*", "/"]),
        pick(&[" {} ", "{}", " {}"]),
        operand,
    )
        .prop_map(|(left, op, spacing, right)| {
            Fragment::new(
                format!("{}{}{}", left.text, spacing.replace("{}", op), right.text),
                json!({"t": "bin", "op": op, "l": left.tree, "r": right.tree}),
            )
        });
    let list = prop::collection::vec(literal(), 0..3).prop_map(|items| {
        Fragment::new(
            format!("[{}]", join(&items, ", ")),
            json!({"t": "list", "items": items.into_iter().map(|item| item.tree).collect::<Vec<_>>()}),
        )
    });
    prop_oneof![4 => literal(), 2 => binary, 1 => list]
}

fn join(fragments: &[Fragment], separator: &str) -> String {
    fragments
        .iter()
        .map(|fragment| fragment.text.as_str())
        .collect::<Vec<_>>()
        .join(separator)
}

/// Argument text with its tree, and whether it is named.
#[derive(Debug, Clone)]
struct Argument {
    fragment: Fragment,
    named: bool,
}

fn argument(value: BoxedStrategy<Fragment>) -> impl Strategy<Value = Argument> {
    (
        prop::option::weighted(
            0.4,
            pick(&["glob", "in", "not", "order", "exit_code", "cwd"]),
        ),
        value,
    )
        .prop_map(|(name, value)| match name {
            Some(name) => Argument {
                fragment: Fragment::new(
                    format!("{name}: {}", value.text),
                    json!({"name": name, "value": value.tree}),
                ),
                named: true,
            },
            None => Argument {
                fragment: Fragment::new(value.text, json!({"value": value.tree})),
                named: false,
            },
        })
}

/// A stage, and whether it reads as a sub-pipeline without parentheses in
/// value position (its name has a group or it has a named argument).
fn stage(value: BoxedStrategy<Fragment>) -> impl Strategy<Value = (Fragment, bool)> {
    (
        pick(&[
            "find",
            "filter",
            "count",
            "git.status",
            "code.rename",
            "run",
        ]),
        prop::collection::vec(argument(value), 0..4),
        any::<bool>(),
    )
        .prop_map(|(name, args, trailing_comma)| {
            let named = args.iter().any(|arg| arg.named);
            let fragments: Vec<Fragment> = args.into_iter().map(|arg| arg.fragment).collect();
            let comma = if trailing_comma && !fragments.is_empty() {
                ","
            } else {
                ""
            };
            let text = format!("{name}({}{comma})", join(&fragments, ", "));
            let tree = json!({"t": "stage", "name": name,
                "args": fragments.into_iter().map(|arg| arg.tree).collect::<Vec<_>>()});
            (Fragment::new(text, tree), named || name.contains('.'))
        })
}

fn pipeline(
    value: BoxedStrategy<Fragment>,
    separator: &'static str,
) -> impl Strategy<Value = Fragment> {
    (
        stage(value.clone()),
        prop::collection::vec(stage(value), 0..3),
    )
        .prop_map(move |((head, _), stages)| {
            let mut text = head.text;
            for (stage, _) in &stages {
                text.push_str(separator);
                text.push_str(&stage.text);
            }
            let tree = json!({"t": "pipeline", "head": head.tree,
                "stages": stages.into_iter().map(|(stage, _)| stage.tree).collect::<Vec<_>>()});
            Fragment::new(text, tree)
        })
}

/// Argument values: expressions and, nested up to a few levels, the three
/// written forms of a sub-pipeline.
fn value() -> BoxedStrategy<Fragment> {
    expression()
        .boxed()
        .prop_recursive(SUB_PIPELINE_LEVELS, 64, 4, |inner| {
            let multi_stage = (stage(inner.clone()), prop::collection::vec(stage(inner.clone()), 1..3))
                .prop_map(|((head, _), stages)| {
                    let text = std::iter::once(head.text.as_str())
                        .chain(stages.iter().map(|(stage, _)| stage.text.as_str()))
                        .collect::<Vec<_>>()
                        .join(" |> ");
                    let tree = json!({"t": "sub", "p": {"t": "pipeline", "head": head.tree,
                        "stages": stages.into_iter().map(|(stage, _)| stage.tree).collect::<Vec<_>>()}});
                    Fragment::new(text, tree)
                });
            let single_stage = (stage(inner.clone()), any::<bool>()).prop_map(|((stage, bare), parenthesize)| {
                let text = if bare && !parenthesize {
                    stage.text
                } else {
                    format!("({})", stage.text)
                };
                let tree = json!({"t": "sub", "p": {"t": "pipeline", "head": stage.tree, "stages": []}});
                Fragment::new(text, tree)
            });
            prop_oneof![3 => inner, 1 => multi_stage, 1 => single_stage]
        })
        .boxed()
}

fn program() -> impl Strategy<Value = Fragment> {
    let statement = (
        prop::option::weighted(0.2, pick(&["recent", "logs"])),
        pipeline(value(), " |> "),
        any::<bool>(),
    )
        .prop_map(|(name, pipeline, multiline)| {
            let text = if multiline {
                pipeline.text.replace(" |> ", "\n|> ")
            } else {
                pipeline.text
            };
            match name {
                Some(name) => Fragment::new(
                    format!("let {name} = {text}"),
                    json!({"t": "let", "name": name, "value": pipeline.tree}),
                ),
                None => Fragment::new(text, pipeline.tree),
            }
        });
    prop::collection::vec(statement, 1..3).prop_map(|statements| {
        Fragment::new(
            join(&statements, "\n"),
            json!({"statements": statements.into_iter().map(|s| s.tree).collect::<Vec<_>>()}),
        )
    })
}

type Position = (u64, u64);

fn span_of(node: &Value) -> Option<(Position, Position)> {
    let span = node.get("span")?.as_array()?;
    let number = |index: usize| span.get(index).and_then(Value::as_u64).unwrap();
    Some(((number(0), number(1)), (number(2), number(3))))
}

fn spanned_children<'a>(node: &'a Value, children: &mut Vec<&'a Value>) {
    let values: Vec<&Value> = match node {
        Value::Object(members) => members
            .iter()
            .filter(|(name, _)| name.as_str() != "span")
            .map(|(_, value)| value)
            .collect(),
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    };
    for value in values {
        if span_of(value).is_some() {
            children.push(value);
        } else {
            spanned_children(value, children);
        }
    }
}

fn spans_nested(node: &Value, outer: Option<(Position, Position)>) -> bool {
    let own = span_of(node);
    if let (Some((start, end)), Some((outer_start, outer_end))) = (own, outer)
        && (start < outer_start || outer_end < end || end < start)
    {
        return false;
    }
    let mut children = Vec::new();
    spanned_children(node, &mut children);
    let mut previous_end: Option<Position> = None;
    for child in children {
        let child_span = span_of(child);
        if let (Some(end), Some((start, _))) = (previous_end, child_span)
            && start < end
        {
            return false;
        }
        previous_end = child_span.map(|(_, end)| end);
        if !spans_nested(child, own.or(outer)) {
            return false;
        }
    }
    true
}

/// Reduces every sub-pipeline whose head is a stage to that stage alone, so
/// that sub-pipelines of one stage, with or without arguments, are printed
/// in their parenthesized form `name((stage))`.
fn single_stage_subs(document: &AstDocument) -> AstDocument {
    let mut document = document.clone();
    for statement in &mut document.program.statements {
        let pipeline = match statement {
            Statement::Pipeline(pipeline) => pipeline,
            Statement::Let(binding) => &mut binding.value,
            _ => continue,
        };
        trim_pipeline(pipeline);
    }
    document
}

fn trim_pipeline(pipeline: &mut Pipeline) {
    if let Head::Stage(stage) = &mut pipeline.head {
        trim_stage(stage);
    }
    for stage in &mut pipeline.stages {
        trim_stage(stage);
    }
}

fn trim_stage(stage: &mut Stage) {
    for arg in &mut stage.args {
        trim_expr(&mut arg.value);
    }
}

fn trim_expr(expr: &mut Expr) {
    match expr {
        Expr::Sub { p, .. } => {
            if matches!(p.head, Head::Stage(_)) {
                p.stages.clear();
            }
            trim_pipeline(p);
        }
        Expr::List { items, .. } | Expr::Call { args: items, .. } => {
            items.iter_mut().for_each(trim_expr);
        }
        Expr::Map { entries, .. } => {
            for entry in entries {
                trim_expr(&mut entry.v);
            }
        }
        Expr::Spread { x, .. } | Expr::Un { x, .. } => trim_expr(x),
        Expr::Field { of: Some(of), .. } => trim_expr(of),
        Expr::Bin { l, r, .. } => {
            trim_expr(l);
            trim_expr(r);
        }
        _ => {}
    }
}

/// Prints a tree in variant B, parses the text and compares the trees
/// without spans; the parsed spans must nest.
fn print_and_parse(document: &AstDocument) -> Result<(), TestCaseError> {
    let text = print::try_print(document, Variant::B)
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    let parsed = parse(&text, Variant::B, &ParseOptions::default())
        .map_err(|error| TestCaseError::fail(format!("{error}\n{text}")))?;
    prop_assert_eq!(hash_form(&parsed), hash_form(document), "{}", text);
    let tree = serde_json::to_value(&parsed).unwrap();
    prop_assert!(spans_nested(&tree, None), "{}: spans", text);
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn printed_trees_read_back_in_variant_b(
        document in roundtrip_support::program(Shape::POSITIONAL)
    ) {
        print_and_parse(&document)?;
        print_and_parse(&single_stage_subs(&document))?;
    }

    #[test]
    fn parser_never_panics_on_any_text(text in any::<String>()) {
        let _ = parse(&text, Variant::B, &ParseOptions::default());
    }

    #[test]
    fn parser_never_panics_on_call_notation_alphabet(
        text in "[a-z0-9_ .,:=|>@()\\[\\]{}\"/#\n+*!~<-]{0,48}"
    ) {
        let _ = parse(&text, Variant::B, &ParseOptions::default());
    }

    #[test]
    fn generated_programs_parse_into_their_trees(program in program()) {
        let document = parse(&program.text, Variant::B, &ParseOptions::default());
        prop_assert!(document.is_ok(), "{:?}: {:?}", program.text, document.err());
        let tree = serde_json::to_value(document.unwrap()).unwrap();
        prop_assert!(spans_nested(&tree, None), "{:?}: spans", program.text);
        prop_assert_eq!(&strip_for_hash(&tree)["program"], &program.tree, "{:?}", program.text);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
