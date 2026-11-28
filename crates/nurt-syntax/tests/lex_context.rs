// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/lex_context.rs
// Responsibility:   Context of `/` chosen by the parser and the spacing of `+` and `-`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Context of `/` chosen by the parser and the spacing of `+` and `-`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_syntax::lex::{LexContext, LexOptions, Lexer, Token, TokenKind};

const VARIANT_A: LexOptions = LexOptions {
    arguments_by_whitespace: true,
    tolerant: false,
    keep_comments: false,
};

const VARIANT_B: LexOptions = LexOptions {
    arguments_by_whitespace: false,
    tolerant: false,
    keep_comments: false,
};

/// Reads the operand `a` and then the token at `/` in operator context.
fn after_operand(text: &str, options: LexOptions) -> Token {
    let mut lexer = Lexer::new(text, options);
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Ident
    );
    lexer.next(LexContext::Operator).unwrap()
}

#[test]
fn reads_division_between_spaced_or_glued_operands() {
    for text in ["a / b", "a/b", "a/ b"] {
        let token = after_operand(text, VARIANT_A);
        assert_eq!(token.kind, TokenKind::Slash, "{text}");
        assert_eq!(token.range.len(), 1, "{text}");
    }
}

#[test]
fn reads_a_regular_expression_argument_after_whitespace() {
    let token = after_operand("x /re/", VARIANT_A);
    assert_eq!((token.kind, token.range), (TokenKind::Regex, 2..6));
    let token = after_operand("search /oldName\\(/i", VARIANT_A);
    assert_eq!(token.kind, TokenKind::Regex);
}

#[test]
fn reads_a_regular_expression_argument_that_starts_with_a_hash() {
    let token = after_operand("x /#include/", VARIANT_A);
    assert_eq!((token.kind, token.range), (TokenKind::Regex, 2..12));
    let token = after_operand("x /#re/ix", VARIANT_A);
    assert_eq!((token.kind, token.range), (TokenKind::Regex, 2..9));
}

#[test]
fn reads_division_when_a_hash_follows_a_slash_without_leading_whitespace() {
    for (text, options) in [("x/#note", VARIANT_A), ("x /#re/", VARIANT_B)] {
        let token = after_operand(text, options);
        assert_eq!(token.kind, TokenKind::Slash, "{text}");
        assert_eq!(token.range.len(), 1, "{text}");
    }
}

#[test]
fn always_reads_division_in_operator_context_of_variant_b() {
    let token = after_operand("x /re/", VARIANT_B);
    assert_eq!((token.kind, token.range), (TokenKind::Slash, 2..3));
}

#[test]
fn always_reads_a_regular_expression_in_value_context() {
    for options in [VARIANT_A, VARIANT_B] {
        let mut lexer = Lexer::new("/ a/", options);
        let token = lexer.next(LexContext::Value).unwrap();
        assert_eq!((token.kind, token.range), (TokenKind::Regex, 0..4));
    }
    let mut lexer = Lexer::new("search(/re/i, \"/srv\")", VARIANT_B);
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Ident
    );
    assert_eq!(
        lexer.next(LexContext::Operator).unwrap().kind,
        TokenKind::LParen
    );
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Regex
    );
}

#[test]
fn reads_division_before_the_end_of_the_input() {
    let token = after_operand("a /", VARIANT_A);
    assert_eq!(token.kind, TokenKind::Slash);
}

#[test]
fn reports_spacing_of_plus_and_minus_for_argument_boundaries() {
    let mut lexer = Lexer::new("first 20 -1", VARIANT_A);
    let kinds: Vec<_> = (0..4)
        .map(|index| {
            let context = if index % 2 == 0 {
                LexContext::Value
            } else {
                LexContext::Operator
            };
            lexer.next(context).unwrap()
        })
        .collect();
    assert_eq!(kinds[2].kind, TokenKind::Minus);
    assert!(kinds[2].space_before && !kinds[2].space_after);
    assert_eq!(kinds[3].kind, TokenKind::Int);

    for (text, before, after) in [
        ("a - 1", true, true),
        ("a-1", false, false),
        ("a -1", true, false),
        ("a- 1", false, true),
        ("a + 1", true, true),
        ("a +1", true, false),
    ] {
        let token = after_operand(text, VARIANT_A);
        assert!(
            matches!(token.kind, TokenKind::Minus | TokenKind::Plus),
            "{text}"
        );
        assert_eq!(
            (token.space_before, token.space_after),
            (before, after),
            "{text}"
        );
    }
}

#[test]
fn never_joins_a_sign_with_a_number() {
    let mut lexer = Lexer::new("-1.5", VARIANT_A);
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Minus
    );
    let number = lexer.next(LexContext::Value).unwrap();
    assert_eq!((number.kind, number.range), (TokenKind::Number, 1..4));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
