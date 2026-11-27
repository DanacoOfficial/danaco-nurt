// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_a_repair.rs
// Responsibility:   Tolerant parsing: the order of repairs, alternatives, the limits per.
// Dependencies:     nurt_contracts, parse_a_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tolerant parsing: the order of repairs, alternatives, the limits per
//! statement and the information given to a repair policy.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod parse_a_support;

use std::cell::{Cell, RefCell};

use nurt_contracts::ast::strip_for_hash;
use nurt_syntax::lex::{ForeignKind, Token, TokenKind};
use nurt_syntax::parse::repair::MAX_REPAIRS_PER_PROGRAM;
use nurt_syntax::parse::{
    Failure, NoRepair, ParseOptions, Repair, RepairPolicy, Variant, parse, parse_tolerant,
};
use parse_a_support::parse_a;

/// Closes the innermost open bracket at the end of the program.
struct CloseInnermost;

impl RepairPolicy for CloseInnermost {
    fn propose(&self, failure: &Failure, _tokens: &[Token], text: &str) -> Vec<Repair> {
        let Some(open) = failure.brackets.last() else {
            return Vec::new();
        };
        if failure.token.kind != TokenKind::Eof {
            return Vec::new();
        }
        let closing = match open.kind {
            TokenKind::LParen => ")",
            TokenKind::LBracket => "]",
            _ => "}",
        };
        vec![Repair::insert("L10", text.len(), closing)]
    }
}

/// Replaces a foreign `|` with `|>`.
struct PipeToOperator;

impl RepairPolicy for PipeToOperator {
    fn propose(&self, failure: &Failure, tokens: &[Token], _text: &str) -> Vec<Repair> {
        assert_eq!(tokens[failure.index], failure.token);
        match failure.token.kind {
            TokenKind::Foreign(ForeignKind::Pipe) => {
                vec![Repair::replace("L13", failure.token.range.clone(), "|>")]
            }
            _ => Vec::new(),
        }
    }
}

/// Proposes fixed repairs and records every failure it sees.
struct Fixed {
    repairs: Vec<Repair>,
    calls: Cell<usize>,
    failures: RefCell<Vec<Failure>>,
}

impl Fixed {
    fn new(repairs: Vec<Repair>) -> Self {
        Self {
            repairs,
            calls: Cell::new(0),
            failures: RefCell::new(Vec::new()),
        }
    }
}

impl RepairPolicy for Fixed {
    fn propose(&self, failure: &Failure, _tokens: &[Token], _text: &str) -> Vec<Repair> {
        self.calls.set(self.calls.get() + 1);
        self.failures.borrow_mut().push(failure.clone());
        self.repairs.clone()
    }
}

fn tolerant(
    text: &str,
    policy: &dyn RepairPolicy,
) -> Result<nurt_syntax::parse::Repaired, nurt_syntax::error::SyntaxError> {
    parse_tolerant(text, Variant::A, &ParseOptions::default(), policy)
}

#[test]
fn valid_text_yields_the_strict_tree_without_repairs() {
    let text = "find \"/a\" |> filter size > 1mb";
    let repaired = tolerant(text, &CloseInnermost).expect("parses");
    assert_eq!(repaired.document, parse_a(text).expect("parses"));
    assert!(repaired.repairs.is_empty());
    assert!(repaired.alternatives.is_empty());
    assert_eq!(repaired.source, text);
}

#[test]
fn closes_an_unterminated_sub_pipeline_at_the_end() {
    let text = r#"search /x/ "/a" |> replace "y" |> verify (run ["go","build"] cwd: "/a""#;
    let repaired = tolerant(text, &CloseInnermost).expect("repaired");
    assert_eq!(repaired.source, format!("{text})"));
    assert_eq!(repaired.repairs.len(), 1);
    let repair = &repaired.repairs[0];
    assert_eq!(repair.rule, "L10");
    assert_eq!((repair.at.line, repair.at.column), (Some(1), Some(71)));
    assert_eq!((repair.from.as_str(), repair.to.as_str()), ("", ")"));
    assert_eq!(
        repaired.document,
        parse_a(&repaired.source).expect("parses")
    );
}

#[test]
fn applies_several_repairs_up_to_the_limit_per_statement() {
    let repaired = tolerant("x ([(a", &CloseInnermost).expect("repaired");
    assert_eq!(repaired.source, "x ([(a)])");
    assert_eq!(repaired.repairs.len(), 3);
    let error = tolerant("x (([(a", &CloseInnermost).expect_err("four repairs needed");
    assert_eq!(error, parse_a("x (([(a").expect_err("invalid"));
    let one = ParseOptions {
        max_repairs: 1,
        ..ParseOptions::default()
    };
    assert!(parse_tolerant("x ((a", Variant::A, &one, &CloseInnermost).is_err());
}

#[test]
fn counts_repairs_per_statement() {
    let text = "x ((a\n";
    let repaired = tolerant(&format!("y [[b\n{text}"), &CloseInnermost);
    assert!(repaired.is_err(), "brackets stay open across lines");
    let repaired = tolerant("x (a)\ny ((b", &CloseInnermost).expect("repaired");
    assert_eq!(repaired.repairs.len(), 2);
}

#[test]
fn replaces_foreign_pipes_and_maps_positions_to_the_original_text() {
    let repaired = tolerant("a | b | c", &PipeToOperator).expect("repaired");
    assert_eq!(repaired.source, "a |> b |> c");
    let columns: Vec<Option<u32>> = repaired.repairs.iter().map(|r| r.at.column).collect();
    assert_eq!(columns, [Some(3), Some(7)]);
    assert!(
        repaired
            .repairs
            .iter()
            .all(|r| r.from == "|" && r.to == "|>")
    );
}

#[test]
fn applies_at_most_the_repairs_of_the_program_limit() {
    let line = "a | b\n";
    let within = line.repeat(MAX_REPAIRS_PER_PROGRAM);
    let repaired = tolerant(&within, &PipeToOperator).expect("repaired");
    assert_eq!(repaired.repairs.len(), MAX_REPAIRS_PER_PROGRAM);
    let beyond = line.repeat(MAX_REPAIRS_PER_PROGRAM + 1);
    let error = tolerant(&beyond, &PipeToOperator).expect_err("one repair too many");
    assert_eq!(error, parse_a(&beyond).expect_err("invalid"));
}

#[test]
fn tries_rules_in_the_fixed_order_and_keeps_other_trees_as_alternatives() {
    let text = "x (a";
    let policy = Fixed::new(vec![
        Repair::replace("L08", 2..4, "\"(a\""),
        Repair::insert("L99", 4, "))"),
        Repair::insert("L10", 4, ")"),
    ]);
    let repaired = tolerant(text, &policy).expect("repaired");
    assert_eq!(repaired.source, "x (a)");
    assert_eq!(repaired.repairs[0].rule, "L10");
    assert_eq!(repaired.alternatives.len(), 1);
    assert_eq!(repaired.alternatives[0].source, "x \"(a\"");
    assert_eq!(repaired.alternatives[0].repairs[0].rule, "L08");
}

#[test]
fn drops_alternatives_that_yield_the_same_tree() {
    let policy = Fixed::new(vec![
        Repair::insert("L10", 4, ")"),
        Repair::insert("L11", 4, " )"),
    ]);
    let repaired = tolerant("x (a", &policy).expect("repaired");
    assert!(repaired.alternatives.is_empty());
}

#[test]
fn describes_the_failure_to_the_policy() {
    let policy = Fixed::new(Vec::new());
    assert!(tolerant("verify (run [1, 2", &policy).is_err());
    let failures = policy.failures.borrow();
    let failure = &failures[0];
    assert_eq!(failure.token.kind, TokenKind::Eof);
    let brackets: Vec<&TokenKind> = failure.brackets.iter().map(|t| &t.kind).collect();
    assert_eq!(brackets, [&TokenKind::LParen, &TokenKind::LBracket]);
    assert_eq!(failure.statement, 0);
    assert_eq!(failure.expected, ["`,`", "`]`"]);
    let policy = Fixed::new(Vec::new());
    assert!(tolerant("count\nfind \"/a\" )", &policy).is_err());
    assert_eq!(policy.failures.borrow()[0].statement, 1);
}

#[test]
fn a_text_accepted_only_by_the_tolerant_lexer_keeps_the_strict_error() {
    let text = "nottrue/re/i 1r#\"q\"#glob:\"/a\"";
    let strict = parse_a(text).expect_err("strict lexical error");
    let options = ParseOptions::default();
    let tolerant = parse_tolerant(text, Variant::A, &options, &NoRepair).expect_err("no tree");
    assert_eq!(strict, tolerant);
}

#[test]
fn bounds_the_number_of_attempts() {
    let policy = Fixed::new(vec![
        Repair::insert("L10", 0, " "),
        Repair::insert("L11", 0, "  "),
        Repair::insert("L12", 0, "   "),
    ]);
    assert!(tolerant("count )", &policy).is_err());
    assert!(
        policy.calls.get() <= 1 + 3 + 9,
        "{} calls",
        policy.calls.get()
    );
}

#[test]
#[expect(
    clippy::reversed_empty_ranges,
    reason = "a reversed range is the invalid repair under test"
)]
fn ignores_repairs_outside_the_text() {
    let policy = Fixed::new(vec![
        Repair::insert("L10", 99, ")"),
        Repair::delete("L12", 3..1),
    ]);
    assert!(tolerant("x (a", &policy).is_err());
}

#[test]
fn disabled_repairs_and_the_empty_policy_return_the_strict_error() {
    let strict = parse("x (a", Variant::A, &ParseOptions::default()).expect_err("invalid");
    assert_eq!(tolerant("x (a", &NoRepair).expect_err("invalid"), strict);
    let none = ParseOptions {
        max_repairs: 0,
        ..ParseOptions::default()
    };
    let error = parse_tolerant("x (a", Variant::A, &none, &CloseInnermost).expect_err("invalid");
    assert_eq!(error, strict);
}

#[test]
fn repaired_trees_ignore_spans_when_compared() {
    let repaired = tolerant("x (a", &CloseInnermost).expect("repaired");
    let tree = serde_json::to_value(&repaired.document).expect("tree");
    let expected = serde_json::to_value(parse_a("x a").expect("parses")).expect("tree");
    assert_eq!(strip_for_hash(&tree), strip_for_hash(&expected));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
