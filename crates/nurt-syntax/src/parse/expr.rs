// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Expressions, lists, maps, ranges and sub-pipelines shared by all variants.
//!
//! Binding strength, from the loosest: `or`; `and`; unary `not`; the
//! non-associative comparisons `== != < <= > >= ~ !~ in`; `+ -`; `* /`;
//! unary `-`; field access and calls. Binary levels associate to the left.
//!
//! Binary operators are parsed by precedence climbing, and the recursive
//! functions pass nodes in boxes and build them in separate functions, so
//! that one level of bracket nesting costs a few small stack frames even
//! without optimization.

use std::collections::HashSet;
use std::ops::Range;

use nurt_contracts::ast::{BinOp, Expr, Head, MapEntry, Pipeline, Ref, Span, Stage, UnOp};

use super::{MAX_COMMAND_NAME_LEN, Node, Parsed, Parser, StageSyntax, Variant, error, join};
use crate::error::SyntaxError;
use crate::lex::{self, Glued, LexContext, Token, TokenKind};

/// Binding strength of `or`.
const OR: u8 = 1;
/// Binding strength of `and`.
const AND: u8 = 2;
/// Binding strength of the unary `not`.
const NOT: u8 = 3;
/// Binding strength of the comparisons.
const COMPARISON: u8 = 4;
/// Binding strength of `+` and `-`.
const ADDITIVE: u8 = 5;
/// Binding strength of `*` and `/`.
const MULTIPLICATIVE: u8 = 6;

/// A parsed expression on the heap.
type Boxed = Box<Node>;

/// Parser of expressions, parameterized by the stage notation of a variant.
///
/// The notation decides two things: whether `-` and `+` preceded but not
/// followed by whitespace end an expression (variants A and C), and how a
/// stage of a sub-pipeline in parentheses is written.
pub(crate) struct ExprParser<'s> {
    syntax: &'s dyn StageSyntax,
}

impl<'s> ExprParser<'s> {
    /// Expression parser for the stage notation.
    pub(crate) fn new(syntax: &'s dyn StageSyntax) -> Self {
        Self { syntax }
    }

    /// Parses an expression. The expression ends before the first token that
    /// cannot continue it, which ends a positional argument.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` for every grammar error of the expression and the
    /// lexical errors of its tokens.
    pub(crate) fn parse_expression(&self, p: &mut Parser<'_>) -> Result<Node, SyntaxError> {
        self.parse_level(p, OR).map(|node| *node)
    }

    /// Parses an expression whose binary operators bind at least as strongly
    /// as `min`.
    fn parse_level(&self, p: &mut Parser<'_>, min: u8) -> Result<Boxed, SyntaxError> {
        let mut left = self.parse_operand(p, min)?;
        while let Some((op, level)) = self.binary_operator(p.current()) {
            if level < min {
                break;
            }
            p.bump(LexContext::Value)?;
            let right = if level == COMPARISON {
                self.parse_comparison_operand(p)?
            } else {
                self.parse_level(p, level + 1)?
            };
            if level == COMPARISON
                && let Some(next) = comparison_operator(p.current())
            {
                return Err(error::chained_comparison(p, p.current(), next, &right));
            }
            left = binary(p, op, left, right)?;
        }
        Ok(left)
    }

    /// Parses the right operand of a comparison, which may not be a `not`
    /// expression without parentheses.
    fn parse_comparison_operand(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        if is_operator_word(p.current(), &TokenKind::Not) {
            let operand = self.parse_operand(p, NOT)?;
            return Err(error::not_in_comparison(p, &operand));
        }
        self.parse_level(p, ADDITIVE)
    }

    /// Parses an operand of a binary level: `not` and its comparison where
    /// `not` may stand, otherwise a unary expression.
    fn parse_operand(&self, p: &mut Parser<'_>, min: u8) -> Result<Boxed, SyntaxError> {
        if min > NOT || !is_operator_word(p.current(), &TokenKind::Not) {
            return self.parse_unary(p);
        }
        self.parse_not(p)
    }

    fn parse_not(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let operator = p.bump(LexContext::Value)?;
        p.enter()?;
        if is_operator_word(p.current(), &TokenKind::Not) {
            return Err(self.double_not(p));
        }
        let operand = self.parse_level(p, COMPARISON)?;
        p.leave();
        unary(p, UnOp::Not, &operator, operand)
    }

    /// Error for `not not x`, with the extent of the inner `not` expression.
    fn double_not(&self, p: &mut Parser<'_>) -> SyntaxError {
        let inner = p.current().clone();
        match self.parse_operand(p, NOT) {
            Ok(operand) => error::double_not(p, &inner, &operand),
            Err(error) => error,
        }
    }

    /// The binary operator of a token and its binding strength; `None` when
    /// the token ends the expression.
    fn binary_operator(&self, token: &Token) -> Option<(BinOp, u8)> {
        match token.kind {
            TokenKind::Or if is_operator_word(token, &TokenKind::Or) => Some((BinOp::Or, OR)),
            TokenKind::And if is_operator_word(token, &TokenKind::And) => Some((BinOp::And, AND)),
            TokenKind::Plus if !self.starts_new_argument(token) => Some((BinOp::Add, ADDITIVE)),
            TokenKind::Minus if !self.starts_new_argument(token) => Some((BinOp::Sub, ADDITIVE)),
            TokenKind::Star => Some((BinOp::Mul, MULTIPLICATIVE)),
            TokenKind::Slash => Some((BinOp::Div, MULTIPLICATIVE)),
            _ => comparison_operator(token).map(|op| (op, COMPARISON)),
        }
    }

    fn parse_unary(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        match p.kind() {
            TokenKind::Minus => self.parse_negation(p),
            TokenKind::Plus => Err(error::unary_plus(p)),
            _ => self.parse_postfix(p),
        }
    }

    fn parse_negation(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let operator = p.bump(LexContext::Value)?;
        p.enter()?;
        if p.at(&TokenKind::Minus) {
            return Err(self.double_minus(p, &operator));
        }
        let operand = self.parse_postfix(p)?;
        p.leave();
        if matches!(operand.value, Expr::Range { .. }) {
            return Err(error::negative_range(p, &operator, &operand));
        }
        unary(p, UnOp::Neg, &operator, operand)
    }

    /// Error for `- -x`, with the extent of the inner negation.
    fn double_minus(&self, p: &mut Parser<'_>, outer: &Token) -> SyntaxError {
        let inner = p.current().clone();
        match self.parse_unary(p) {
            Ok(operand) => error::double_minus(p, outer, &inner, &operand),
            Err(error) => error,
        }
    }

    fn parse_postfix(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let mut node = self.parse_primary(p)?;
        while p.at(&TokenKind::Dot) && !p.current().space_before {
            node = field_access(p, node)?;
        }
        if p.at(&TokenKind::DotDot) && !p.current().space_before {
            return Err(error::range_bound(p, node.span, &node.range));
        }
        Ok(node)
    }

    fn parse_primary(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        match p.kind() {
            TokenKind::Int => parse_int_or_range(p),
            TokenKind::Text
            | TokenKind::RawText
            | TokenKind::Regex
            | TokenKind::Number
            | TokenKind::Size
            | TokenKind::Duration
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null => parse_literal(p),
            TokenKind::Ident if p.current().glued == Glued::LParen => self.parse_call(p),
            TokenKind::Ident if p.condition && p.slice(p.current()) == "exists" => {
                self.parse_exists(p)
            }
            TokenKind::Ident => parse_ident(p),
            TokenKind::At => parse_reference(p),
            TokenKind::DotDot => parse_range_after(p, None),
            TokenKind::LBracket => self.parse_list(p),
            TokenKind::LBrace => self.parse_map(p),
            TokenKind::LParen => self.parse_parenthesized(p),
            TokenKind::Ellipsis => Err(self.spread_outside_list(p)),
            TokenKind::Let | TokenKind::And | TokenKind::Or | TokenKind::Not | TokenKind::In => {
                Err(error::keyword_as_value(p))
            }
            _ => Err(error::missing_operand(p)),
        }
    }

    /// Parses `exists <operand>` of a condition into a call of `exists`.
    fn parse_exists(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let word = p.bump(LexContext::Value)?;
        let operand = self.parse_postfix(p)?;
        let span = join(word.span, operand.span);
        let range = word.range.start..operand.range.end;
        let height = operand.height + 1;
        p.check_height(height, span)?;
        Ok(Box::new(Parsed {
            value: Expr::Call {
                function: "exists".to_owned(),
                args: vec![operand.value],
                span: Some(span),
            },
            span,
            range,
            height,
        }))
    }

    /// Error for `...x` outside a list, with the extent of the spread.
    fn spread_outside_list(&self, p: &mut Parser<'_>) -> SyntaxError {
        if let Err(error) = p.enter() {
            return error;
        }
        match self.parse_list_item(p) {
            Ok(spread) => error::spread_outside_list(p, spread.span, &spread.range),
            Err(error) => error,
        }
    }

    fn parse_call(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let name = p.bump(LexContext::Value)?;
        p.bump(LexContext::Value)?;
        p.enter()?;
        let mut args = Vec::new();
        let mut height = 0;
        if !p.at(&TokenKind::RParen) {
            loop {
                let arg = self.parse_level(p, OR)?;
                height = height.max(arg.height);
                args.push(arg.value);
                if !p.at(&TokenKind::Comma) {
                    break;
                }
                let comma = p.bump(LexContext::Value)?;
                if p.at(&TokenKind::RParen) {
                    return Err(error::trailing_comma_in_call(p, &comma));
                }
            }
        }
        let close = p.expect(&TokenKind::RParen, LexContext::Operator, "`,` or `)`")?;
        p.leave();
        call_node(p, &name, &close, args, height)
    }

    fn parse_list(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let open = p.bump(LexContext::Value)?;
        p.enter()?;
        let mut items = Vec::new();
        let mut height = 0;
        while !p.at(&TokenKind::RBracket) {
            let item = self.parse_list_item(p)?;
            height = height.max(item.height);
            items.push(item.value);
            if p.at(&TokenKind::Comma) {
                p.bump(LexContext::Value)?;
            } else if !p.at(&TokenKind::RBracket) {
                return Err(error::missing_separator(p, "`]`"));
            }
        }
        let close = p.bump(LexContext::Operator)?;
        p.leave();
        list_node(p, &open, &close, items, height)
    }

    fn parse_list_item(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        if !p.at(&TokenKind::Ellipsis) {
            return self.parse_level(p, OR);
        }
        let operator = p.bump(LexContext::Value)?;
        let operand = self.parse_level(p, OR)?;
        spread_node(p, &operator, operand)
    }

    fn parse_map(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let open = p.bump(LexContext::Value)?;
        p.enter()?;
        let mut entries = Vec::new();
        let mut keys = HashSet::new();
        let mut height = 0;
        while !p.at(&TokenKind::RBrace) {
            let (key, key_token) = parse_map_key(p)?;
            p.expect(&TokenKind::Colon, LexContext::Value, "`:`")?;
            let value = self.parse_level(p, OR)?;
            if !keys.insert(key.clone()) {
                return Err(error::duplicate_key(p, &key_token, &key));
            }
            height = height.max(value.height);
            entries.push(MapEntry {
                k: key,
                v: value.value,
                span: Some(join(key_token.span, value.span)),
            });
            if p.at(&TokenKind::Comma) {
                p.bump(LexContext::Value)?;
            } else if !p.at(&TokenKind::RBrace) {
                return Err(error::missing_separator(p, "`}`"));
            }
        }
        let close = p.bump(LexContext::Operator)?;
        p.leave();
        map_node(p, &open, &close, entries, height)
    }

    /// Parses `( … )`: a sub-pipeline when the content starts with a command
    /// followed by an argument or has `|>` at its top level, otherwise a
    /// grouped expression whose span includes the parentheses.
    fn parse_parenthesized(&self, p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
        let open = p.bump(LexContext::Value)?;
        p.enter()?;
        let inner = self.parse_parenthesized_content(p)?;
        let close = p.expect(&TokenKind::RParen, LexContext::Operator, "`)`")?;
        p.leave();
        parenthesized_node(p, &open, &close, inner)
    }

    fn parse_parenthesized_content(&self, p: &mut Parser<'_>) -> Result<Inner, SyntaxError> {
        if self.starts_sub_pipeline(p) {
            return Ok(Inner::Pipeline(Box::new(p.parse_pipeline(self.syntax)?)));
        }
        let expression = self.parse_level(p, OR)?;
        if !p.at(&TokenKind::PipeGt) {
            return Ok(Inner::Expression(expression));
        }
        let Parsed {
            value, span, range, ..
        } = *expression;
        if let Expr::Sub { p: inner, .. } = value {
            let head = Parsed {
                value: inner.head,
                span,
                range,
                height: 1,
            };
            let mut tail = p.parse_pipeline_tail(head, self.syntax)?;
            tail.value.stages.splice(0..0, inner.stages);
            return Ok(Inner::Pipeline(Box::new(tail)));
        }
        let head = into_head(
            p,
            Parsed {
                value,
                span,
                range,
                height: 1,
            },
        )?;
        Ok(Inner::Pipeline(Box::new(
            p.parse_pipeline_tail(head, self.syntax)?,
        )))
    }

    /// Whether the content of parentheses starts with a command followed by
    /// a token that starts an argument; the tokens read ahead are rewound.
    fn starts_sub_pipeline(&self, p: &mut Parser<'_>) -> bool {
        if p.kind() != &TokenKind::Ident {
            return false;
        }
        let checkpoint = p.checkpoint();
        let starts = match p.parse_command_name(LexContext::Operator) {
            Ok((_, _, last)) => self.syntax.starts_arguments(&last, p.current()),
            Err(_) => false,
        };
        p.rewind(checkpoint);
        starts
    }

    /// Whether `-` or `+` ends the expression and starts a new argument:
    /// whitespace precedes it and does not follow it.
    fn starts_new_argument(&self, operator: &Token) -> bool {
        self.syntax.arguments_by_whitespace() && operator.space_before && !operator.space_after
    }
}

/// Content of parentheses.
enum Inner {
    Expression(Boxed),
    Pipeline(Box<Parsed<Pipeline>>),
}

/// Turns the expression before `|>` inside parentheses into the head of a
/// pipeline: an identifier or a chain of field accesses is a command, a
/// reference is a reference. A stage of variant B always has parentheses,
/// so there a bare command name before `|>` is an error.
fn into_head(p: &Parser<'_>, expression: Node) -> Result<Parsed<Head>, SyntaxError> {
    let head = match expression.value {
        Expr::Ref(reference) => Head::Ref(reference),
        other => match command_name(&other) {
            Some(name) if name.chars().count() > MAX_COMMAND_NAME_LEN => {
                return Err(error::bad_pipeline_head(
                    p,
                    expression.span,
                    &expression.range,
                ));
            }
            Some(name) if p.variant == Variant::B => {
                return Err(error::stage_without_parentheses(
                    p,
                    &name,
                    expression.span,
                    &expression.range,
                ));
            }
            Some(name) => Head::Stage(Stage {
                name,
                args: Vec::new(),
                span: Some(expression.span),
            }),
            None => {
                return Err(error::bad_pipeline_head(
                    p,
                    expression.span,
                    &expression.range,
                ));
            }
        },
    };
    Ok(Parsed {
        value: head,
        span: expression.span,
        range: expression.range,
        height: 1,
    })
}

fn command_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Ident { v, .. } => Some(v.clone()),
        Expr::Field {
            name, of: Some(of), ..
        } => command_name(of).map(|prefix| format!("{prefix}.{name}")),
        _ => None,
    }
}

/// Whether the token is the keyword `kind` used as an operator; a keyword
/// directly followed by `:` or `=` names an argument.
pub(crate) fn is_operator_word(token: &Token, kind: &TokenKind) -> bool {
    token.kind == *kind && !matches!(token.glued, Glued::Colon | Glued::Equals)
}

fn comparison_operator(token: &Token) -> Option<BinOp> {
    let op = match token.kind {
        TokenKind::EqEq => BinOp::Eq,
        TokenKind::NotEq => BinOp::Ne,
        TokenKind::Lt => BinOp::Lt,
        TokenKind::Le => BinOp::Le,
        TokenKind::Gt => BinOp::Gt,
        TokenKind::Ge => BinOp::Ge,
        TokenKind::Tilde => BinOp::Match,
        TokenKind::NotTilde => BinOp::NotMatch,
        TokenKind::In if is_operator_word(token, &TokenKind::In) => BinOp::In,
        _ => return None,
    };
    Some(op)
}

/// Builds a node after checking its height.
fn node(
    p: &Parser<'_>,
    value: Expr,
    span: Span,
    range: Range<usize>,
    height: usize,
) -> Result<Boxed, SyntaxError> {
    p.check_height(height, span)?;
    Ok(Box::new(Parsed {
        value,
        span,
        range,
        height,
    }))
}

/// Takes the expression out of a boxed node, leaving `null` in its place.
fn take_value(node: &mut Boxed) -> Expr {
    std::mem::replace(&mut node.value, Expr::Null { span: None })
}

/// Stores a node in the allocation of `slot` after checking its height, so
/// that wrapping an operand does not allocate a new node.
fn rebuild(
    p: &Parser<'_>,
    mut slot: Boxed,
    value: Expr,
    span: Span,
    range: Range<usize>,
    height: usize,
) -> Result<Boxed, SyntaxError> {
    p.check_height(height, span)?;
    *slot = Parsed {
        value,
        span,
        range,
        height,
    };
    Ok(slot)
}

fn binary(
    p: &Parser<'_>,
    op: BinOp,
    mut left: Boxed,
    mut right: Boxed,
) -> Result<Boxed, SyntaxError> {
    let span = join(left.span, right.span);
    let range = left.range.start..right.range.end;
    let height = left.height.max(right.height) + 1;
    let value = Expr::Bin {
        op,
        l: Box::new(take_value(&mut left)),
        r: Box::new(take_value(&mut right)),
        span: Some(span),
    };
    rebuild(p, left, value, span, range, height)
}

fn unary(
    p: &Parser<'_>,
    op: UnOp,
    operator: &Token,
    mut operand: Boxed,
) -> Result<Boxed, SyntaxError> {
    let span = join(operator.span, operand.span);
    let range = operator.range.start..operand.range.end;
    let height = operand.height + 1;
    let value = Expr::Un {
        op,
        x: Box::new(take_value(&mut operand)),
        span: Some(span),
    };
    rebuild(p, operand, value, span, range, height)
}

fn spread_node(p: &Parser<'_>, operator: &Token, mut operand: Boxed) -> Result<Boxed, SyntaxError> {
    let span = join(operator.span, operand.span);
    let range = operator.range.start..operand.range.end;
    let height = operand.height + 1;
    let value = Expr::Spread {
        x: Box::new(take_value(&mut operand)),
        span: Some(span),
    };
    rebuild(p, operand, value, span, range, height)
}

/// Reads `.name` after an operand and wraps the operand in a field access.
fn field_access(p: &mut Parser<'_>, mut record: Boxed) -> Result<Boxed, SyntaxError> {
    p.bump(LexContext::Value)?;
    if p.kind() != &TokenKind::Ident || p.current().space_before {
        return Err(p.unexpected(&["field name after `.`"]));
    }
    let name = p.bump(LexContext::Operator)?;
    if name.glued == Glued::LParen {
        return Err(error::call_on_field(p, &name));
    }
    let span = join(record.span, name.span);
    let range = record.range.start..name.range.end;
    let height = record.height + 1;
    let value = Expr::Field {
        name: p.slice(&name).to_owned(),
        of: Some(Box::new(take_value(&mut record))),
        span: Some(span),
    };
    rebuild(p, record, value, span, range, height)
}

fn call_node(
    p: &Parser<'_>,
    name: &Token,
    close: &Token,
    args: Vec<Expr>,
    height: usize,
) -> Result<Boxed, SyntaxError> {
    let span = join(name.span, close.span);
    let value = Expr::Call {
        function: p.slice(name).to_owned(),
        args,
        span: Some(span),
    };
    node(
        p,
        value,
        span,
        name.range.start..close.range.end,
        height + 1,
    )
}

fn list_node(
    p: &Parser<'_>,
    open: &Token,
    close: &Token,
    items: Vec<Expr>,
    height: usize,
) -> Result<Boxed, SyntaxError> {
    let span = join(open.span, close.span);
    let value = Expr::List {
        items,
        span: Some(span),
    };
    node(
        p,
        value,
        span,
        open.range.start..close.range.end,
        height + 1,
    )
}

fn map_node(
    p: &Parser<'_>,
    open: &Token,
    close: &Token,
    entries: Vec<MapEntry>,
    height: usize,
) -> Result<Boxed, SyntaxError> {
    let span = join(open.span, close.span);
    let value = Expr::Map {
        entries,
        span: Some(span),
    };
    node(
        p,
        value,
        span,
        open.range.start..close.range.end,
        height + 1,
    )
}

fn parenthesized_node(
    p: &Parser<'_>,
    open: &Token,
    close: &Token,
    inner: Inner,
) -> Result<Boxed, SyntaxError> {
    let span = join(open.span, close.span);
    let range = open.range.start..close.range.end;
    match inner {
        Inner::Pipeline(pipeline) => {
            let height = pipeline.height + 1;
            let value = Expr::Sub {
                p: pipeline.value,
                span: Some(span),
            };
            node(p, value, span, range, height)
        }
        Inner::Expression(mut expression) => {
            set_span(&mut expression.value, span);
            expression.span = span;
            expression.range = range;
            Ok(expression)
        }
    }
}

fn leaf(value: Expr, token: &Token) -> Boxed {
    Box::new(Parsed {
        value,
        span: token.span,
        range: token.range.clone(),
        height: 1,
    })
}

fn parse_ident(p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
    let token = p.bump(LexContext::Operator)?;
    let value = Expr::Ident {
        v: p.slice(&token).to_owned(),
        span: Some(token.span),
    };
    Ok(leaf(value, &token))
}

fn parse_reference(p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
    let reference = p.parse_ref()?;
    Ok(Box::new(Parsed {
        value: Expr::Ref(reference.value),
        span: reference.span,
        range: reference.range,
        height: reference.height,
    }))
}

/// Parses a literal token with the decoder of its kind.
fn parse_literal(p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
    let token = p.bump(LexContext::Operator)?;
    let source = p.slice(&token);
    let decoded = match token.kind {
        TokenKind::True => Ok(Expr::Bool {
            v: true,
            span: None,
        }),
        TokenKind::False => Ok(Expr::Bool {
            v: false,
            span: None,
        }),
        TokenKind::Null => Ok(Expr::Null { span: None }),
        TokenKind::Text => p.decode_text(source),
        TokenKind::RawText => lex::decode_raw(source),
        TokenKind::Regex => lex::decode_regex(source),
        TokenKind::Int => lex::parse_int(source),
        TokenKind::Number => lex::parse_number(source),
        TokenKind::Size => lex::parse_size(source),
        TokenKind::Duration => lex::parse_duration(source),
        _ => return Err(error::unexpected(p, &token, &["value"])),
    };
    let mut value = decoded.map_err(|lexical| error::literal(p, &token, lexical))?;
    set_span(&mut value, token.span);
    Ok(leaf(value, &token))
}

/// Parses an integer and, when `..` follows without whitespace, the range
/// that starts with it.
fn parse_int_or_range(p: &mut Parser<'_>) -> Result<Boxed, SyntaxError> {
    let literal = parse_literal(p)?;
    if !p.at(&TokenKind::DotDot) || p.current().space_before {
        return Ok(literal);
    }
    match literal.value {
        Expr::Int { v, .. } => parse_range_after(p, Some((v, literal.span, literal.range.start))),
        _ => Ok(literal),
    }
}

/// Parses the rest of a range `[INT] ".." [INT]`; the current token is `..`
/// and `low` is the lower bound already read with its span and start.
/// Bounds are written without whitespace around `..`.
fn parse_range_after(
    p: &mut Parser<'_>,
    low: Option<(i64, Span, usize)>,
) -> Result<Boxed, SyntaxError> {
    let dots = p.bump(LexContext::Value)?;
    let (lo, start_span, start) = match low {
        Some((v, span, start)) => (Some(v), span, start),
        None => (None, dots.span, dots.range.start),
    };
    let mut end_span = dots.span;
    let mut end = dots.range.end;
    let mut hi = None;
    if !p.current().space_before {
        if p.at(&TokenKind::Int) {
            let bound = parse_literal(p)?;
            if let Expr::Int { v, .. } = bound.value {
                hi = Some(v);
            }
            end_span = bound.span;
            end = bound.range.end;
        } else if matches!(
            p.kind(),
            TokenKind::Number | TokenKind::Size | TokenKind::Duration
        ) {
            let token = p.current();
            return Err(error::range_bound(p, token.span, &token.range));
        }
    }
    if p.at(&TokenKind::DotDot) && !p.current().space_before {
        return Err(p.unexpected(&["end of the range"]));
    }
    let span = join(start_span, end_span);
    let value = Expr::Range {
        lo,
        hi,
        span: Some(span),
    };
    node(p, value, span, start..end, 1)
}

/// Parses a map key: an identifier, a keyword or a text.
fn parse_map_key(p: &mut Parser<'_>) -> Result<(String, Token), SyntaxError> {
    let key = match p.kind() {
        TokenKind::Ident
        | TokenKind::Let
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Null
        | TokenKind::And
        | TokenKind::Or
        | TokenKind::Not
        | TokenKind::In => p.slice(p.current()).to_owned(),
        TokenKind::Text | TokenKind::RawText => text_key(p)?,
        _ => return Err(p.unexpected(&["map key", "`}`"])),
    };
    let token = p.bump(LexContext::Value)?;
    Ok((key, token))
}

/// Decoded value of a text written as a map key.
fn text_key(p: &Parser<'_>) -> Result<String, SyntaxError> {
    let token = p.current();
    let source = p.slice(token);
    let decoded = if token.kind == TokenKind::Text {
        p.decode_text(source)
    } else {
        lex::decode_raw(source)
    };
    match decoded {
        Ok(Expr::Text { v, .. }) => Ok(v),
        Ok(_) => Err(p.unexpected(&["map key"])),
        Err(lexical) => Err(error::literal(p, token, lexical)),
    }
}

/// Replaces the span of an expression.
fn set_span(expression: &mut Expr, new: Span) {
    match expression {
        Expr::Ref(Ref { span, .. })
        | Expr::Text { span, .. }
        | Expr::Ident { span, .. }
        | Expr::Regex { span, .. }
        | Expr::Int { span, .. }
        | Expr::Num { span, .. }
        | Expr::Size { span, .. }
        | Expr::Dur { span, .. }
        | Expr::Bool { span, .. }
        | Expr::Null { span }
        | Expr::Key { span, .. }
        | Expr::List { span, .. }
        | Expr::Map { span, .. }
        | Expr::Spread { span, .. }
        | Expr::Field { span, .. }
        | Expr::Call { span, .. }
        | Expr::Un { span, .. }
        | Expr::Bin { span, .. }
        | Expr::Range { span, .. }
        | Expr::Sub { span, .. } => *span = Some(new),
        _ => {}
    }
}
