// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_c_limits.rs
// Responsibility:   Nesting limits of the variant C parser.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Nesting limits of the variant C parser.
//!
//! The parser rejects nesting deeper than the limit of the tree decoders with
//! `E_SYNTAX` and never overflows the stack of a thread with the default
//! stack size of the test harness.

use nurt_contracts::ast::{Expr, Head, Statement};
use nurt_contracts::error::ErrorCode;
use nurt_syntax::parse::{DEFAULT_MAX_DEPTH, ParseOptions, Variant, parse};

fn parse_c(
    text: &str,
) -> Result<nurt_contracts::ast::AstDocument, nurt_syntax::error::SyntaxError> {
    parse(text, Variant::C, &ParseOptions::default())
}

fn parenthesized(depth: usize) -> String {
    format!(
        "filter condition={}x{}",
        "(".repeat(depth),
        ")".repeat(depth)
    )
}

#[test]
fn parentheses_below_the_limit_are_accepted() {
    let document = parse_c(&parenthesized(DEFAULT_MAX_DEPTH / 2)).expect("nesting is accepted");
    let Statement::Pipeline(pipeline) = &document.program.statements[0] else {
        panic!("not a pipeline");
    };
    let Head::Stage(stage) = &pipeline.head else {
        panic!("head is not a stage");
    };
    assert!(matches!(&stage.args[0].value, Expr::Ident { v, .. } if v == "x"));
}

#[test]
fn deep_parentheses_are_rejected_without_overflow() {
    let error = parse_c(&parenthesized(DEFAULT_MAX_DEPTH * 100)).expect_err("nesting is rejected");
    assert_eq!(error.code, ErrorCode::Syntax);
}

#[test]
fn deep_sub_pipelines_are_rejected_without_overflow() {
    let mut text = String::from("count");
    for _ in 0..DEFAULT_MAX_DEPTH * 10 {
        text = format!("@x |> verify check=({text} |> count)");
    }
    let error = parse_c(&text).expect_err("nesting is rejected");
    assert_eq!(error.code, ErrorCode::Syntax);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
