// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::parse::a
// Responsibility:   Stage syntax of variant A: positional and `name: value` arguments.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Stage syntax of variant A: positional and `name: value` arguments.
//!
//! A stage is `COMMAND {argument}`. Whitespace separates arguments; a named
//! argument is a word directly followed by `:` and an expression. A
//! positional argument ends at the first token that cannot continue its
//! expression.

use nurt_contracts::ast::{Arg, Expr, Program, Stage};

use super::expr::ExprParser;
use super::{Parsed, Parser, StageSyntax, error, join};
use crate::error::SyntaxError;
use crate::lex::{Glued, LexContext, Token, TokenKind};

/// Stage notation of variant A.
pub(crate) struct SyntaxA;

impl StageSyntax for SyntaxA {
    fn parse_stage(&self, p: &mut Parser<'_>) -> Result<Parsed<Stage>, SyntaxError> {
        let (name, first, last) = p.parse_command_name(LexContext::Value)?;
        let expressions = ExprParser::new(self);
        let mut span = join(first.span, last.span);
        let mut end = last.range.end;
        let mut height = 0;
        let mut args = Vec::new();
        while starts_argument(p.current()) {
            let arg = parse_argument(p, &expressions)?;
            span = join(span, arg.span);
            end = arg.range.end;
            height = height.max(arg.height);
            args.push(arg.value);
        }
        let height = height + 1;
        p.check_height(height, span)?;
        Ok(Parsed {
            value: Stage {
                name,
                args,
                span: Some(span),
            },
            span,
            range: first.range.start..end,
            height,
        })
    }

    fn starts_arguments(&self, last: &Token, next: &Token) -> bool {
        last.glued != Glued::LParen && starts_argument(next)
    }

    fn arguments_by_whitespace(&self) -> bool {
        true
    }
}

/// Parses a program of variant A.
///
/// # Errors
///
/// Returns the first lexical or grammar error of the text.
pub(crate) fn parse_program(p: &mut Parser<'_>) -> Result<Program, SyntaxError> {
    p.parse_program_with(&SyntaxA)
}

/// Whether the token starts an argument of variant A: a literal, `[`, `{`,
/// `@`, `(`, `..`, `...`, an identifier, `not`, a word directly followed by
/// `:`, or `-` and `+` preceded but not followed by whitespace.
pub(crate) fn starts_argument(token: &Token) -> bool {
    match token.kind {
        TokenKind::Ident
        | TokenKind::Text
        | TokenKind::RawText
        | TokenKind::Regex
        | TokenKind::Int
        | TokenKind::Number
        | TokenKind::Size
        | TokenKind::Duration
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Null
        | TokenKind::LBracket
        | TokenKind::LBrace
        | TokenKind::At
        | TokenKind::LParen
        | TokenKind::DotDot
        | TokenKind::Ellipsis
        | TokenKind::Not => true,
        TokenKind::Minus | TokenKind::Plus => token.space_before && !token.space_after,
        TokenKind::Let | TokenKind::And | TokenKind::Or | TokenKind::In => {
            token.glued == Glued::Colon
        }
        _ => false,
    }
}

/// Whether the token names an argument: an identifier or a keyword
/// directly followed by `:`.
fn names_argument(token: &Token) -> bool {
    token.glued == Glued::Colon
        && matches!(
            token.kind,
            TokenKind::Ident
                | TokenKind::Let
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Not
                | TokenKind::In
        )
}

fn parse_argument(
    p: &mut Parser<'_>,
    expressions: &ExprParser<'_>,
) -> Result<Parsed<Arg>, SyntaxError> {
    if names_argument(p.current()) {
        return parse_named_argument(p, expressions);
    }
    let value = expressions.parse_expression(p)?;
    if p.at(&TokenKind::Colon) && matches!(value.value, Expr::Ident { .. }) {
        return Err(error::space_before_colon(p, &value.range));
    }
    Ok(Parsed {
        value: Arg {
            name: None,
            value: value.value,
            span: Some(value.span),
        },
        span: value.span,
        range: value.range,
        height: value.height,
    })
}

fn parse_named_argument(
    p: &mut Parser<'_>,
    expressions: &ExprParser<'_>,
) -> Result<Parsed<Arg>, SyntaxError> {
    let name = p.bump(LexContext::Value)?;
    let colon = p.bump(LexContext::Value)?;
    let starts_value =
        starts_argument(p.current()) || matches!(p.kind(), TokenKind::Minus | TokenKind::Plus);
    if !starts_value || names_argument(p.current()) {
        return Err(error::missing_value(p, &name, &colon));
    }
    let value = expressions.parse_expression(p)?;
    let span = join(name.span, value.span);
    Ok(Parsed {
        value: Arg {
            name: Some(p.slice(&name).to_owned()),
            value: value.value,
            span: Some(span),
        },
        span,
        range: name.range.start..value.range.end,
        height: value.height,
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
