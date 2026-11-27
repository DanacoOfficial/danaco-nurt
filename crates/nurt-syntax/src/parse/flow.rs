// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::parse::flow
// Responsibility:   Parsers of the control constructs: `when`, `each`, `repeat`, `while`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsers of the control constructs: `when`, `each`, `repeat`, `while`,
//! `break`, `continue`, `steps`, `define` and `params`.
//!
//! The constructs are written the same way in the three syntax variants; only
//! the pipelines inside their blocks follow the notation of the variant. The
//! words that start a construct are contextual: they are reserved only where
//! a statement starts, so `steps` stays admissible as a parameter name.
//!
//! ```text
//! when COND { BLOCK } {otherwise when COND { BLOCK }} [otherwise { BLOCK }]
//! each EXPR as NAME {OPTION} { BLOCK }
//! repeat {OPTION} { BLOCK }        until: is required
//! while COND {OPTION} { BLOCK }
//! steps { BLOCK }
//! define NAME ( [PARAM {, PARAM}] ) { BLOCK }
//! params { {PARAM NL} }
//! PARAM  = NAME ":" TYPE ["=" EXPR]
//! OPTION = NAME [":" EXPR]
//! ```

use nurt_contracts::ast::{
    Arg, Branch, Define, Each, Expr, Jump, Param, Params, Repeat, Span, Statement, Steps, When,
    While,
};

use super::expr::ExprParser;
use super::{Parser, StageSyntax, join};
use crate::error::SyntaxError;
use crate::lex::{Glued, LexContext, Token, TokenKind};

/// Words that start a construct at the beginning of a statement.
const STARTERS: [&str; 9] = [
    "when", "each", "repeat", "while", "break", "continue", "steps", "define", "params",
];

/// Whether the current token starts a construct.
pub(super) fn starts_construct(p: &Parser<'_>) -> bool {
    p.kind() == &TokenKind::Ident && STARTERS.contains(&p.slice(p.current()))
}

/// Whether the current token is the identifier `word`.
fn at_word(p: &Parser<'_>, word: &str) -> bool {
    p.kind() == &TokenKind::Ident && p.slice(p.current()) == word
}

/// Parses the construct that starts at the current token.
pub(super) fn parse_construct(
    p: &mut Parser<'_>,
    syntax: &dyn StageSyntax,
) -> Result<Statement, SyntaxError> {
    let word = p.slice(p.current());
    match word {
        "when" => parse_when(p, syntax),
        "each" => parse_each(p, syntax),
        "repeat" => parse_repeat(p, syntax),
        "while" => parse_while(p, syntax),
        "steps" => parse_steps(p, syntax),
        "define" => parse_define(p, syntax),
        "params" => parse_params(p),
        _ => {
            let keyword = p.bump(LexContext::Operator)?;
            let jump = Jump {
                span: Some(keyword.span),
            };
            Ok(if word == "break" {
                Statement::Break(jump)
            } else {
                Statement::Continue(jump)
            })
        }
    }
}

fn skip_line_breaks(p: &mut Parser<'_>) -> Result<(), SyntaxError> {
    while p.at(&TokenKind::Nl) {
        p.bump(LexContext::Value)?;
    }
    Ok(())
}

/// Parses `{ statements }` and returns the statements with the closing brace.
fn parse_block(
    p: &mut Parser<'_>,
    syntax: &dyn StageSyntax,
) -> Result<(Vec<Statement>, Token), SyntaxError> {
    if !p.at(&TokenKind::LBrace) {
        return Err(p.unexpected(&["`{`"]));
    }
    p.lexer.mark_block();
    p.bump(LexContext::Value)?;
    p.enter()?;
    skip_line_breaks(p)?;
    let mut body = Vec::new();
    loop {
        if p.at(&TokenKind::RBrace) {
            if body.is_empty() {
                return Err(p.unexpected(&["a statement"]));
            }
            break;
        }
        if p.at(&TokenKind::Eof) {
            return Err(p.unexpected(&["a statement", "`}`"]));
        }
        body.push(p.parse_statement(syntax)?);
        match p.kind() {
            TokenKind::Nl => skip_line_breaks(p)?,
            TokenKind::RBrace => {}
            _ => return Err(super::error::statement_end(p)),
        }
    }
    let close = p.bump(LexContext::Operator)?;
    p.leave();
    Ok((body, close))
}

/// Parses the expression of a condition.
fn parse_condition(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Expr, SyntaxError> {
    let outer = std::mem::replace(&mut p.condition, true);
    let parsed = ExprParser::new(syntax).parse_expression(p);
    p.condition = outer;
    parsed.map(|node| node.value)
}

/// Parses `name: value` and bare `name` options up to the block.
fn parse_options(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Vec<Arg>, SyntaxError> {
    let mut options = Vec::new();
    while p.kind() == &TokenKind::Ident {
        let name = p.bump(LexContext::Value)?;
        let text = p.slice(&name).to_owned();
        if name.glued == Glued::Colon {
            p.bump(LexContext::Value)?;
            let outer = std::mem::replace(&mut p.condition, text == "until");
            let value = ExprParser::new(syntax).parse_expression(p);
            p.condition = outer;
            let value = value?;
            options.push(Arg {
                name: Some(text),
                span: Some(join(name.span, value.span)),
                value: value.value,
            });
        } else {
            options.push(Arg {
                name: Some(text),
                value: Expr::Bool {
                    v: true,
                    span: Some(name.span),
                },
                span: Some(name.span),
            });
        }
    }
    Ok(options)
}

fn parse_when(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    let mut branches = Vec::new();
    let mut otherwise = None;
    let mut start = keyword.span;
    let last;
    loop {
        let cond = parse_condition(p, syntax)?;
        let (body, close) = parse_block(p, syntax)?;
        branches.push(Branch {
            cond,
            body,
            span: Some(join(start, close.span)),
        });
        if !at_otherwise(p)? {
            last = close.span;
            break;
        }
        let word = p.bump(LexContext::Value)?;
        start = word.span;
        if at_word(p, "when") {
            p.bump(LexContext::Value)?;
            continue;
        }
        let (body, close) = parse_block(p, syntax)?;
        otherwise = Some(body);
        last = close.span;
        break;
    }
    Ok(Statement::When(When {
        branches,
        otherwise,
        span: Some(join(keyword.span, last)),
    }))
}

/// Whether `otherwise` follows, on the same line or after line breaks.
fn at_otherwise(p: &mut Parser<'_>) -> Result<bool, SyntaxError> {
    if at_word(p, "otherwise") {
        return Ok(true);
    }
    if !p.at(&TokenKind::Nl) {
        return Ok(false);
    }
    let checkpoint = p.checkpoint();
    skip_line_breaks(p)?;
    if at_word(p, "otherwise") {
        return Ok(true);
    }
    p.rewind(checkpoint);
    Ok(false)
}

fn parse_each(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    let source = ExprParser::new(syntax).parse_expression(p)?.value;
    if !at_word(p, "as") {
        return Err(p.unexpected(&["`as`"]));
    }
    p.bump(LexContext::Value)?;
    if p.kind() != &TokenKind::Ident {
        return Err(p.unexpected(&["a name for the record"]));
    }
    let name = p.bump(LexContext::Operator)?;
    let name = p.slice(&name).to_owned();
    let options = parse_options(p, syntax)?;
    let (body, close) = parse_block(p, syntax)?;
    Ok(Statement::Each(Each {
        source,
        name,
        options,
        body,
        span: Some(join(keyword.span, close.span)),
    }))
}

fn parse_repeat(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    let options = parse_options(p, syntax)?;
    let (body, close) = parse_block(p, syntax)?;
    Ok(Statement::Repeat(Repeat {
        options,
        body,
        span: Some(join(keyword.span, close.span)),
    }))
}

fn parse_while(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    let cond = parse_condition(p, syntax)?;
    let options = parse_options(p, syntax)?;
    let (body, close) = parse_block(p, syntax)?;
    Ok(Statement::While(While {
        cond,
        options,
        body,
        span: Some(join(keyword.span, close.span)),
    }))
}

fn parse_steps(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    let (body, close) = parse_block(p, syntax)?;
    Ok(Statement::Steps(Steps {
        body,
        span: Some(join(keyword.span, close.span)),
    }))
}

fn parse_define(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    if p.kind() != &TokenKind::Ident || p.current().glued != Glued::LParen {
        return Err(p.unexpected(&["a function name directly followed by `(`"]));
    }
    let name = p.bump(LexContext::Value)?;
    let name = p.slice(&name).to_owned();
    p.bump(LexContext::Value)?;
    let mut params = Vec::new();
    if !p.at(&TokenKind::RParen) {
        loop {
            params.push(parse_param(p, syntax)?);
            if !p.at(&TokenKind::Comma) {
                break;
            }
            p.bump(LexContext::Value)?;
        }
    }
    p.expect(&TokenKind::RParen, LexContext::Operator, "`,` or `)`")?;
    let (body, close) = parse_block(p, syntax)?;
    p.functions.insert(name.clone());
    Ok(Statement::Define(Define {
        name,
        params,
        body,
        span: Some(join(keyword.span, close.span)),
    }))
}

fn parse_params(p: &mut Parser<'_>) -> Result<Statement, SyntaxError> {
    let keyword = p.bump(LexContext::Value)?;
    if !p.at(&TokenKind::LBrace) {
        return Err(p.unexpected(&["`{`"]));
    }
    p.lexer.mark_block();
    p.bump(LexContext::Value)?;
    skip_line_breaks(p)?;
    let mut params = Vec::new();
    let no_syntax = super::a::SyntaxA;
    while !p.at(&TokenKind::RBrace) {
        if p.at(&TokenKind::Eof) {
            return Err(p.unexpected(&["a parameter", "`}`"]));
        }
        params.push(parse_param(p, &no_syntax)?);
        match p.kind() {
            TokenKind::Nl => skip_line_breaks(p)?,
            TokenKind::RBrace => {}
            _ => return Err(super::error::statement_end(p)),
        }
    }
    let close = p.bump(LexContext::Operator)?;
    Ok(Statement::Params(Params {
        params,
        span: Some(join(keyword.span, close.span)),
    }))
}

/// Parses `name: type [= default]`.
fn parse_param(p: &mut Parser<'_>, syntax: &dyn StageSyntax) -> Result<Param, SyntaxError> {
    if p.kind() != &TokenKind::Ident || p.current().glued != Glued::Colon {
        return Err(p.unexpected(&["a parameter name followed by `:`"]));
    }
    let name = p.bump(LexContext::Value)?;
    p.bump(LexContext::Value)?;
    let (ty, ty_span) = parse_type(p)?;
    let (default, end) = if p.at(&TokenKind::Eq) {
        p.bump(LexContext::Value)?;
        let value = ExprParser::new(syntax).parse_expression(p)?;
        (Some(value.value), value.span)
    } else {
        (None, ty_span)
    };
    Ok(Param {
        name: p.slice(&name).to_owned(),
        ty,
        default,
        span: Some(join(name.span, end)),
    })
}

/// Parses a type written as `name`, `name<type>` or `name(a, b)`.
fn parse_type(p: &mut Parser<'_>) -> Result<(String, Span), SyntaxError> {
    if p.kind() != &TokenKind::Ident {
        return Err(p.unexpected(&["a type name"]));
    }
    let first = p.bump(LexContext::Operator)?;
    let mut text = p.slice(&first).to_owned();
    let mut span = first.span;
    if p.at(&TokenKind::Lt) && !p.current().space_before {
        p.bump(LexContext::Value)?;
        let (inner, inner_span) = parse_type(p)?;
        if !p.at(&TokenKind::Gt) {
            return Err(p.unexpected(&["`>`"]));
        }
        let close = p.bump(LexContext::Operator)?;
        text = format!("{text}<{inner}>");
        span = join(span, join(inner_span, close.span));
    } else if p.at(&TokenKind::LParen) && !p.current().space_before {
        p.bump(LexContext::Value)?;
        let mut values = Vec::new();
        loop {
            if p.kind() != &TokenKind::Ident {
                return Err(p.unexpected(&["a value name"]));
            }
            let value = p.bump(LexContext::Operator)?;
            values.push(p.slice(&value).to_owned());
            if !p.at(&TokenKind::Comma) {
                break;
            }
            p.bump(LexContext::Value)?;
        }
        let close = p.expect(&TokenKind::RParen, LexContext::Operator, "`,` or `)`")?;
        text = format!("{text}({})", values.join(", "));
        span = join(span, close.span);
    }
    Ok((text, span))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
