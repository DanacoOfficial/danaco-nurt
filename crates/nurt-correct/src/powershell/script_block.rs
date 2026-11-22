// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Script blocks of `Where-Object` and `ForEach-Object`.
//!
//! A `Where-Object` block becomes the condition of a `filter` stage when it
//! consists of comparisons of properties of `$_` with values, joined with
//! `-and`, `-or` and `-not`. String comparisons of PowerShell ignore case;
//! the translation keeps that meaning with `lower(…)` or the regex flag `i`,
//! or states the difference as an assumption. Any other block has no
//! translation.

use nurt_contracts::ast::{BinOp, Expr, Span, UnOp};
use nurt_contracts::error::Error;

use super::convert;
use super::lex::{Token, TokenKind};
use super::rules::{Session, ident, text};
use super::syntax::{self, Command, Element, RawArg, StatementKind, Value, ValueKind};
use super::table::{FieldMap, Fold, Operator, Pattern, Record};

/// Help topic of `filter`.
const HELP_FILTER: &str = "help filter";

/// Function that folds text to lower case.
const LOWER_FUNCTION: &str = "lower";

/// Regex flag that ignores case.
const IGNORE_CASE_FLAG: &str = "i";

/// A translated condition with the assumptions it makes.
pub(crate) struct Condition {
    pub(crate) expr: Expr,
    pub(crate) assumptions: Vec<String>,
}

/// Translates the arguments of `Where-Object`: a script block or the
/// simplified syntax `Where-Object Property -Operator Value`.
pub(crate) fn where_condition(
    session: &Session<'_>,
    command: &Command,
    record: &Record,
) -> Result<Condition, Error> {
    let mut block = None;
    let mut property: Option<&Value> = None;
    let mut value: Option<&Value> = None;
    let mut operator: Option<(String, usize, usize)> = None;
    let mut negate = false;
    let mut args = command.args.iter().peekable();
    while let Some(arg) = args.next() {
        match arg {
            RawArg::Value(found) => match &found.kind {
                ValueKind::Block(tokens) if block.is_none() => block = Some(tokens),
                _ if property.is_none() => property = Some(found),
                _ if value.is_none() => value = Some(found),
                _ => return Err(extra(session, found.start, found.end)),
            },
            RawArg::Param {
                name,
                value: inline,
                start,
                end,
            } => {
                let next = inline.as_ref().or_else(|| match args.peek().copied() {
                    Some(RawArg::Value(next)) => Some(next),
                    _ => None,
                });
                match name.to_ascii_lowercase().as_str() {
                    "filterscript" => {
                        if let Some(Value {
                            kind: ValueKind::Block(tokens),
                            ..
                        }) = next
                        {
                            block = Some(tokens);
                            consume(inline.is_none(), &mut args);
                        }
                    }
                    "property" => {
                        property = next;
                        consume(inline.is_none(), &mut args);
                    }
                    "value" => {
                        value = next;
                        consume(inline.is_none(), &mut args);
                    }
                    "not" => negate = true,
                    other => operator = Some((other.to_owned(), *start, *end)),
                }
            }
        }
    }
    let mut parser = Parser {
        session,
        record,
        tokens: Vec::new(),
        pos: 0,
        assumptions: Vec::new(),
    };
    let expr = match (block, property) {
        (Some(tokens), None) => {
            parser.tokens = tokens
                .iter()
                .filter(|token| token.kind != TokenKind::Newline)
                .cloned()
                .collect();
            let expr = parser.or()?;
            if let Some(token) = parser.tokens.get(parser.pos) {
                return Err(unsupported(session, token.start, token.end));
            }
            expr
        }
        (None, Some(property)) => {
            let field = parser.property_operand(property)?;
            let expr = match (operator, value) {
                (Some((name, start, end)), Some(value)) => {
                    let op = session
                        .table
                        .operator(&name)
                        .ok_or_else(|| unknown_operator(session, &name, start, end))?;
                    parser.compare(field, op, value)?
                }
                (None, None) => parser.truthy(&field, property)?,
                _ => {
                    return Err(session.uncorrectable(
                        "incomplete_condition",
                        "the condition needs a property, an operator and a value",
                        command.start,
                        command.end,
                        HELP_FILTER,
                    ));
                }
            };
            if negate { not(expr) } else { expr }
        }
        _ => {
            return Err(session.uncorrectable(
                "missing_condition",
                "Where-Object needs a script block or a property",
                command.start,
                command.end,
                HELP_FILTER,
            ));
        }
    };
    Ok(Condition {
        expr,
        assumptions: parser.assumptions,
    })
}

fn consume<'a>(take: bool, args: &mut std::iter::Peekable<std::slice::Iter<'a, RawArg>>) {
    if take {
        let _ = args.next_if(|next| matches!(next, RawArg::Value(_)));
    }
}

fn extra(session: &Session<'_>, start: usize, end: usize) -> Error {
    session.uncorrectable(
        "extra_argument",
        "Where-Object takes one condition",
        start,
        end,
        HELP_FILTER,
    )
}

fn unsupported(session: &Session<'_>, start: usize, end: usize) -> Error {
    session.uncorrectable(
        "script_block",
        "the script block has no Nurt form; a filter compares fields with values, joined with and, or and not",
        start,
        end,
        HELP_FILTER,
    )
}

fn unknown_operator(session: &Session<'_>, name: &str, start: usize, end: usize) -> Error {
    session.uncorrectable(
        "operator",
        format!(
            "-{name} has no Nurt form in a filter; use comparisons, -like, -match, -contains or -in"
        ),
        start,
        end,
        HELP_FILTER,
    )
}

fn not(expr: Expr) -> Expr {
    Expr::Un {
        op: UnOp::Not,
        x: Box::new(expr),
        span: None,
    }
}

fn bin(op: BinOp, l: Expr, r: Expr, span: Span) -> Expr {
    Expr::Bin {
        op,
        l: Box::new(l),
        r: Box::new(r),
        span: Some(span),
    }
}

fn call(function: &str, arg: Expr, span: Span) -> Expr {
    Expr::Call {
        function: function.to_owned(),
        args: vec![arg],
        span: Some(span),
    }
}

/// A property of `$_` resolved to its field, or `$_` itself.
struct FieldRef<'r> {
    map: Option<&'r FieldMap>,
    expr: Expr,
}

/// An operand of a comparison.
enum Operand {
    Condition(Expr),
    Value(Value),
}

struct Parser<'s, 'r> {
    session: &'s Session<'s>,
    record: &'r Record,
    tokens: Vec<Token>,
    pos: usize,
    assumptions: Vec<String>,
}

impl<'r> Parser<'_, 'r> {
    fn peek_param(&self) -> Option<(&str, usize, usize)> {
        match self.tokens.get(self.pos) {
            Some(Token {
                kind: TokenKind::Parameter { name, .. },
                start,
                end,
                ..
            }) => Some((name.as_str(), *start, *end)),
            _ => None,
        }
    }

    fn take_keyword(&mut self, keyword: &str) -> bool {
        if self
            .peek_param()
            .is_some_and(|(name, _, _)| name.eq_ignore_ascii_case(keyword))
        {
            self.pos += 1;
            return true;
        }
        false
    }

    fn or(&mut self) -> Result<Expr, Error> {
        let mut left = self.and()?;
        while self.take_keyword("or") {
            let right = self.and()?;
            left = Expr::Bin {
                op: BinOp::Or,
                l: Box::new(left),
                r: Box::new(right),
                span: None,
            };
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, Error> {
        let mut left = self.negation()?;
        while self.take_keyword("and") {
            let right = self.negation()?;
            left = Expr::Bin {
                op: BinOp::And,
                l: Box::new(left),
                r: Box::new(right),
                span: None,
            };
        }
        Ok(left)
    }

    fn negation(&mut self) -> Result<Expr, Error> {
        let bang = matches!(self.tokens.get(self.pos), Some(Token { kind: TokenKind::Word(word), .. }) if word == "!");
        if bang || self.take_keyword("not") {
            if bang {
                self.pos += 1;
            }
            return Ok(not(self.negation()?));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, Error> {
        let left = self.primary()?;
        let operator = self
            .peek_param()
            .filter(|(name, _, _)| {
                !["and", "or", "not"]
                    .iter()
                    .any(|keyword| name.eq_ignore_ascii_case(keyword))
            })
            .map(|(name, start, end)| (name.to_owned(), start, end));
        match (left, operator) {
            (Operand::Condition(expr), None) => Ok(expr),
            (Operand::Value(value), None) => {
                let field = self.property_operand(&value)?;
                self.truthy(&field, &value)
            }
            (Operand::Value(value), Some((name, start, end))) => {
                self.pos += 1;
                let op = self
                    .session
                    .table
                    .operator(&name)
                    .ok_or_else(|| unknown_operator(self.session, &name, start, end))?;
                let right = match self.primary()? {
                    Operand::Value(right) => right,
                    Operand::Condition(_) => return Err(unsupported(self.session, start, end)),
                };
                let field = self.property_operand(&value)?;
                self.compare(field, op, &right)
            }
            (Operand::Condition(_), Some((_, start, end))) => {
                Err(unsupported(self.session, start, end))
            }
        }
    }

    fn primary(&mut self) -> Result<Operand, Error> {
        let Some(token) = self.tokens.get(self.pos).cloned() else {
            let end = self.tokens.last().map_or(0, |token| token.end);
            return Err(unsupported(self.session, end, end));
        };
        if token.kind == TokenKind::OpenParen {
            let inner_is_command = matches!(
                self.tokens.get(self.pos + 1),
                Some(Token {
                    kind: TokenKind::Word(_),
                    ..
                })
            );
            if !inner_is_command {
                let close = self
                    .matching_paren()
                    .ok_or_else(|| unsupported(self.session, token.start, token.end))?;
                let inner: Vec<Token> = self
                    .tokens
                    .get(self.pos + 1..close)
                    .unwrap_or_default()
                    .to_vec();
                let mut nested = Parser {
                    session: self.session,
                    record: self.record,
                    tokens: inner,
                    pos: 0,
                    assumptions: Vec::new(),
                };
                let expr = nested.or()?;
                if let Some(extra) = nested.tokens.get(nested.pos) {
                    return Err(unsupported(self.session, extra.start, extra.end));
                }
                self.assumptions.extend(nested.assumptions);
                self.pos = close + 1;
                return Ok(Operand::Condition(expr));
            }
        }
        let (value, next) = syntax::parse_value(&self.tokens, self.pos)
            .map_err(|error| unsupported(self.session, error.start, error.end))?;
        self.pos = next;
        if matches!(
            self.tokens.get(self.pos),
            Some(Token {
                kind: TokenKind::Comma,
                ..
            })
        ) {
            let start = value.start;
            let mut items = vec![value];
            while matches!(
                self.tokens.get(self.pos),
                Some(Token {
                    kind: TokenKind::Comma,
                    ..
                })
            ) {
                let (item, next) = syntax::parse_value(&self.tokens, self.pos + 1)
                    .map_err(|error| unsupported(self.session, error.start, error.end))?;
                self.pos = next;
                items.push(item);
            }
            let end = items.last().map_or(start, |item| item.end);
            return Ok(Operand::Value(Value {
                kind: ValueKind::List(items),
                start,
                end,
            }));
        }
        Ok(Operand::Value(value))
    }

    fn matching_paren(&self) -> Option<usize> {
        let mut depth = 0_usize;
        for (index, token) in self.tokens.iter().enumerate().skip(self.pos) {
            match token.kind {
                TokenKind::OpenParen | TokenKind::SubExprOpen | TokenKind::ArrayOpen => depth += 1,
                TokenKind::CloseParen => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `$_.Property`, `$_`, or a bare property name of the simplified
    /// syntax.
    fn property_operand(&self, value: &Value) -> Result<FieldRef<'r>, Error> {
        let span = self.session.span(value.start, value.end);
        let property = match &value.kind {
            ValueKind::Member {
                base,
                name,
                args: None,
            } if matches!(&base.kind, ValueKind::Variable(var) if var.is_pipeline_item()) => {
                name.clone()
            }
            ValueKind::Variable(var) if var.is_pipeline_item() => {
                return match &self.record.self_field {
                    Some(field) => Ok(FieldRef {
                        map: None,
                        expr: ident(field.clone(), span),
                    }),
                    None => Err(self.session.uncorrectable(
                        "whole_object",
                        format!(
                            "the whole {} object has no Nurt value; compare one of its properties",
                            self.record.name
                        ),
                        value.start,
                        value.end,
                        "help describe",
                    )),
                };
            }
            ValueKind::Bare(name) => name.clone(),
            _ => {
                return Err(self.session.uncorrectable(
                    "comparison_operand",
                    "a filter compares a property of $_ with a value",
                    value.start,
                    value.end,
                    HELP_FILTER,
                ));
            }
        };
        let map = self.record.field(&property).ok_or_else(|| {
            self.session
                .unknown_property(&property, self.record, value.start, value.end)
        })?;
        let base = ident(map.field.clone(), span);
        let expr = match &map.function {
            Some(function) => call(function, base, span),
            None => base,
        };
        Ok(FieldRef {
            map: Some(map),
            expr,
        })
    }

    /// A property used as a condition on its own.
    fn truthy(&self, field: &FieldRef<'_>, value: &Value) -> Result<Expr, Error> {
        let span = self.session.span(value.start, value.end);
        match field.map {
            Some(map) if map.equals.is_some() => Ok(bin(
                BinOp::Eq,
                field.expr.clone(),
                ident(map.equals.clone().unwrap_or_default(), span),
                span,
            )),
            Some(map) if map.ty.as_deref() == Some("bool") => Ok(field.expr.clone()),
            _ => Err(self.session.uncorrectable(
                "property_condition",
                "compare the property with a value",
                value.start,
                value.end,
                HELP_FILTER,
            )),
        }
    }

    fn compare(
        &mut self,
        field: FieldRef<'_>,
        op: &Operator,
        right: &Value,
    ) -> Result<Expr, Error> {
        let span = self.session.span(right.start, right.end);
        if let Some(equals) = field.map.and_then(|map| map.equals.clone()) {
            if !matches!(op.op, BinOp::Eq | BinOp::Ne) {
                return Err(self.literal_error(right, "the property is compared with -eq or -ne"));
            }
            let flag = match &right.kind {
                ValueKind::Variable(var) if var.name.eq_ignore_ascii_case("true") => true,
                ValueKind::Variable(var) if var.name.eq_ignore_ascii_case("false") => false,
                _ => {
                    return Err(
                        self.literal_error(right, "the property is compared with $true or $false")
                    );
                }
            };
            let positive = (op.op == BinOp::Eq) == flag;
            let op = if positive { BinOp::Eq } else { BinOp::Ne };
            return Ok(bin(op, field.expr, ident(equals, span), span));
        }
        let mut left = field.expr;
        let right_expr = match op.pattern {
            Some(Pattern::Wildcard) => {
                let pattern = self.text_literal(right)?;
                Expr::Regex {
                    v: convert::wildcard_regex(&pattern),
                    flags: self.flags(op),
                    span: Some(span),
                }
            }
            Some(Pattern::Regex) => {
                let pattern = self.text_literal(right)?;
                regex::Regex::new(&pattern).map_err(|_| {
                    self.literal_error(
                        right,
                        "the regular expression uses a construct Nurt does not support",
                    )
                })?;
                Expr::Regex {
                    v: pattern,
                    flags: self.flags(op),
                    span: Some(span),
                }
            }
            None => {
                let (expr, folded) = self.value(field.map, op, right)?;
                if folded {
                    left = call(LOWER_FUNCTION, left, span);
                }
                expr
            }
        };
        let expr = if op.swap {
            bin(op.op, right_expr, left, span)
        } else {
            bin(op.op, left, right_expr, span)
        };
        Ok(if op.negate { not(expr) } else { expr })
    }

    fn flags(&self, op: &Operator) -> String {
        if op.fold == Some(Fold::Flag) {
            IGNORE_CASE_FLAG.to_owned()
        } else {
            String::new()
        }
    }

    fn literal_error(&self, value: &Value, message: &str) -> Error {
        self.session.uncorrectable(
            "comparison_value",
            message.to_owned(),
            value.start,
            value.end,
            HELP_FILTER,
        )
    }

    fn text_literal(&self, value: &Value) -> Result<String, Error> {
        value
            .plain_text()
            .ok_or_else(|| self.literal_error(value, "the pattern must be literal text"))
    }

    fn assume(&mut self, text: &str) {
        if !self.assumptions.iter().any(|known| known == text) {
            self.assumptions.push(text.to_owned());
        }
    }

    /// The right operand of a comparison; the flag tells whether the left
    /// side has to be folded to lower case.
    fn value(
        &mut self,
        map: Option<&FieldMap>,
        op: &Operator,
        value: &Value,
    ) -> Result<(Expr, bool), Error> {
        let span = self.session.span(value.start, value.end);
        match &value.kind {
            ValueKind::List(items) => {
                let mut converted = Vec::new();
                let mut folded = false;
                for item in items {
                    let (expr, fold) = self.value(map, op, item)?;
                    folded |= fold;
                    converted.push(expr);
                }
                Ok((
                    Expr::List {
                        items: converted,
                        span: Some(span),
                    },
                    folded,
                ))
            }
            ValueKind::Number(number) => {
                let is_size = map.is_some_and(|map| map.ty.as_deref() == Some("size"));
                let expr = match (is_size, number.bytes(), number.int) {
                    (true, Some(bytes), _) => Expr::Size {
                        v: bytes,
                        span: Some(span),
                    },
                    (false, Some(bytes), _) if number.exponent > 0 => Expr::Int {
                        v: i64::try_from(bytes)
                            .map_err(|_| self.literal_error(value, "the number is too large"))?,
                        span: Some(span),
                    },
                    (_, None, _) if number.exponent > 0 => {
                        return Err(self.literal_error(value, "the number is too large"));
                    }
                    (_, _, Some(int)) if int >= 0 => Expr::Int {
                        v: int,
                        span: Some(span),
                    },
                    (_, _, None) if number.value >= 0.0 => Expr::Num {
                        v: number.value,
                        span: Some(span),
                    },
                    _ => Expr::Un {
                        op: UnOp::Neg,
                        x: Box::new(match number.int {
                            Some(int) => Expr::Int {
                                v: int.saturating_neg(),
                                span: Some(span),
                            },
                            None => Expr::Num {
                                v: -number.value,
                                span: Some(span),
                            },
                        }),
                        span: Some(span),
                    },
                };
                Ok((expr, false))
            }
            ValueKind::Variable(var) if var.scope.is_none() => {
                match var.name.to_ascii_lowercase().as_str() {
                    "true" => Ok((
                        Expr::Bool {
                            v: true,
                            span: Some(span),
                        },
                        false,
                    )),
                    "false" => Ok((
                        Expr::Bool {
                            v: false,
                            span: Some(span),
                        },
                        false,
                    )),
                    "null" => Ok((Expr::Null { span: Some(span) }, false)),
                    _ => Err(self.literal_error(
                        value,
                        "variables are not expanded; compare with a literal value",
                    )),
                }
            }
            ValueKind::Paren(_) | ValueKind::Member { .. } => Ok((self.instant(value)?, false)),
            _ => {
                let written = self.text_literal(value)?;
                self.text_value(map, op, &written, value)
            }
        }
    }

    fn text_value(
        &mut self,
        map: Option<&FieldMap>,
        op: &Operator,
        written: &str,
        value: &Value,
    ) -> Result<(Expr, bool), Error> {
        let span = self.session.span(value.start, value.end);
        if let Some(map) = map {
            if !map.values.is_empty() {
                let mapped = map
                    .values
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(written))
                    .map(|(_, value)| value.clone());
                return match mapped {
                    Some(mapped) => Ok((ident(mapped, span), false)),
                    None => {
                        let names: Vec<&str> =
                            map.values.iter().map(|(name, _)| name.as_str()).collect();
                        Err(self.session.uncorrectable(
                            "comparison_value",
                            format!("{} has the values {}", map.property, names.join(", ")),
                            value.start,
                            value.end,
                            HELP_FILTER,
                        ))
                    }
                };
            }
            if let Some(prefix) = &map.prefix
                && let Some(stripped) = written.strip_prefix(prefix.as_str())
            {
                let assumption = self.session.table.comparison.prefix_assumption.clone();
                self.assume(&assumption);
                return self.fold_text(op, stripped, span);
            }
        }
        self.fold_text(op, written, span)
    }

    fn fold_text(
        &mut self,
        op: &Operator,
        written: &str,
        span: Span,
    ) -> Result<(Expr, bool), Error> {
        if !convert::has_cased(written) {
            return Ok((text(written, span), false));
        }
        match op.fold {
            Some(Fold::Lower) => {
                let expr = if written.to_lowercase() == written {
                    text(written, span)
                } else {
                    call(LOWER_FUNCTION, text(written, span), span)
                };
                Ok((expr, true))
            }
            Some(Fold::Assume) => {
                let assumption = self.session.table.comparison.text_assumption.clone();
                self.assume(&assumption);
                Ok((text(written, span), false))
            }
            _ => Ok((text(written, span), false)),
        }
    }

    /// `(Get-Date)` and `(Get-Date).AddDays(-n)`.
    fn instant(&self, value: &Value) -> Result<Expr, Error> {
        let span = self.session.span(value.start, value.end);
        let dates = &self.session.table.dates;
        let now = || Expr::Call {
            function: dates.now.clone(),
            args: Vec::new(),
            span: Some(span),
        };
        let is_now = |value: &Value| match &value.kind {
            ValueKind::Paren(script) => match script.statements.as_slice() {
                [statement] => match &statement.kind {
                    StatementKind::Pipeline(pipeline) => matches!(
                        pipeline.elements.as_slice(),
                        [Element::Command(command)] if command.name.eq_ignore_ascii_case(&dates.cmdlet) && command.args.is_empty()
                    ),
                    StatementKind::Assign { .. } => false,
                },
                _ => false,
            },
            _ => false,
        };
        if is_now(value) {
            return Ok(now());
        }
        let ValueKind::Member {
            base,
            name,
            args: Some(args),
        } = &value.kind
        else {
            return Err(self.literal_error(
                value,
                "compare with a literal value or with (Get-Date).AddDays(-n)",
            ));
        };
        let unit = dates.unit(name).filter(|_| is_now(base));
        let count = match args.as_slice() {
            [
                Value {
                    kind: ValueKind::Number(number),
                    ..
                },
            ] if number.exponent == 0 => Some(number.value),
            _ => None,
        };
        let (Some(unit), Some(count)) = (unit, count) else {
            return Err(self.literal_error(value, "only (Get-Date).AddDays(-n), AddHours, AddMinutes, AddSeconds and AddMilliseconds have a Nurt form"));
        };
        if count == 0.0 {
            return Ok(now());
        }
        if count > 0.0 {
            return Err(self.literal_error(
                value,
                "a filter compares with past instants; use a negative count",
            ));
        }
        let ms = convert::duration_ms(-count, unit).ok_or_else(|| {
            self.literal_error(value, "the duration must be a whole number of milliseconds")
        })?;
        Ok(Expr::Call {
            function: dates.ago.clone(),
            args: vec![Expr::Dur {
                v: ms,
                span: Some(span),
            }],
            span: Some(span),
        })
    }
}

/// Properties selected by `ForEach-Object { $_.Name }` or
/// `ForEach-Object Name`; `None` for any other block.
pub(crate) fn selected_properties(command: &Command) -> Option<Vec<String>> {
    match command.args.as_slice() {
        [
            RawArg::Value(Value {
                kind: ValueKind::Block(tokens),
                ..
            }),
        ] => {
            let significant: Vec<&Token> = tokens
                .iter()
                .filter(|token| token.kind != TokenKind::Newline)
                .collect();
            match significant.as_slice() {
                [
                    Token {
                        kind: TokenKind::Variable(var),
                        ..
                    },
                    Token {
                        kind: TokenKind::Member(name),
                        ..
                    },
                ] if var.is_pipeline_item() => Some(vec![name.clone()]),
                _ => None,
            }
        }
        [RawArg::Value(value)] => match &value.kind {
            ValueKind::Bare(name) => Some(vec![name.clone()]),
            _ => None,
        },
        [
            RawArg::Param {
                name, value: None, ..
            },
            RawArg::Value(value),
        ] if name.eq_ignore_ascii_case("MemberName") => value.plain_text().map(|name| vec![name]),
        _ => None,
    }
}

/// The only command of a `ForEach-Object` block, for a proposal that
/// replaces the loop by a stage.
pub(crate) fn single_command(command: &Command) -> Option<Command> {
    let tokens = command.args.iter().find_map(|arg| match arg {
        RawArg::Value(Value {
            kind: ValueKind::Block(tokens),
            ..
        }) => Some(tokens),
        _ => None,
    })?;
    let script = syntax::parse(tokens).ok()?;
    let [statement] = script.statements.as_slice() else {
        return None;
    };
    let StatementKind::Pipeline(pipeline) = &statement.kind else {
        return None;
    };
    match pipeline.elements.as_slice() {
        [Element::Command(inner)] => Some(inner.clone()),
        _ => None,
    }
}
