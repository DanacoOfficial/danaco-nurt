// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Printing of expressions: precedence, parentheses, lists, maps and keys.

use std::collections::HashSet;
use std::fmt::Write as _;

use nurt_contracts::ast::{
    BinOp, Expr, Head, KeySeg, MapEntry, Pipeline, Ref, Span, UnOp, Wildcard,
};

use super::{MAX_LINE_WIDTH, MAX_NESTING_DEPTH, PrintError, literal};
use crate::parse::Variant;

/// Indentation of one nesting level of a multi-line list or map.
pub(super) const INDENT_WIDTH: usize = 2;

/// Binding strength accepted anywhere.
pub(super) const PREC_ANY: u8 = 0;
const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_NOT: u8 = 3;
const PREC_COMPARE: u8 = 4;
const PREC_ADD: u8 = 5;
const PREC_MUL: u8 = 6;
const PREC_NEG: u8 = 7;
const PREC_ATOM: u8 = 8;

const IDENT_PROBLEM: &str =
    "is not an identifier `[a-z][a-z0-9_]*` or is a reserved word; use a text value";
const NAME_PROBLEM: &str =
    "has a name that is not an identifier `[a-z][a-z0-9_]*` or is a reserved word; rename it";
const UNSAFE_PROBLEM: &str =
    "has a value outside 0..=2^53-1; write a negative value as unary `-` applied to its magnitude";
const UNKNOWN_PROBLEM: &str =
    "has a kind or operator this printer does not know; upgrade the printer";

/// Line breaking of lists and maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// Everything on one line.
    Flat,
    /// Lists and maps may break; `indent` is the indentation of the current
    /// line.
    Block {
        /// Indentation of the current line, in spaces.
        indent: usize,
    },
}

/// Layout mode and nesting depth of the node being printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ctx {
    pub(super) mode: Mode,
    pub(super) depth: usize,
}

impl Ctx {
    /// Context of a statement printed from column 0.
    pub(super) const TOP: Self = Self {
        mode: Mode::Block { indent: 0 },
        depth: 1,
    };

    pub(super) fn deeper(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }

    pub(super) fn flat(self) -> Self {
        Self {
            mode: Mode::Flat,
            ..self
        }
    }

    fn indented(self, indent: usize) -> Self {
        Self {
            mode: Mode::Block { indent },
            depth: self.depth + 1,
        }
    }
}

/// Output buffer that tracks the current column in characters.
#[derive(Debug, Default)]
pub(super) struct Out {
    pub(super) text: String,
    column: usize,
}

impl Out {
    pub(super) fn push(&mut self, piece: &str) {
        self.text.push_str(piece);
        match piece.rfind('\n') {
            Some(at) => self.column = piece[at + 1..].chars().count(),
            None => self.column += piece.chars().count(),
        }
    }

    fn newline(&mut self, indent: usize) {
        self.text.push('\n');
        self.text.extend(std::iter::repeat_n(' ', indent));
        self.column = indent;
    }
}

/// Returns `true` when `text` is one line that ends within the line width
/// when it starts at `column`.
pub(super) fn fits(text: &str, column: usize) -> bool {
    !text.contains('\n') && column + text.chars().count() <= MAX_LINE_WIDTH
}

/// First token of an expression that matters at an argument boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lead {
    /// `not`, which cannot continue a previous operand.
    Not,
    /// `..` of a range without a lower bound.
    OpenStart,
    /// Any other token.
    Other,
}

/// Returns the first token class of the printed form of `expr`.
pub(super) fn leading(expr: &Expr) -> Lead {
    let mut current = expr;
    loop {
        match current {
            Expr::Un { op: UnOp::Not, .. } => return Lead::Not,
            Expr::Range { lo: None, .. } => return Lead::OpenStart,
            Expr::Bin { op, l, .. } if precedence(l) >= operand_bounds(*op).0 && !ends_open(l) => {
                current = l;
            }
            _ => return Lead::Other,
        }
    }
}

/// Returns `true` when the left operand `left`, printed without its own
/// parentheses at the binding strength `min`, ends with `..`.
fn left_ends_open(left: &Expr, min: u8) -> bool {
    precedence(left) >= min && ends_open(left)
}

/// Returns `true` when the printed form of `expr` ends with `..` of a range
/// without an upper bound.
pub(super) fn ends_open(expr: &Expr) -> bool {
    let mut current = expr;
    loop {
        match current {
            Expr::Range { hi: None, .. } => return true,
            Expr::Bin { op, r, .. } if precedence(r) >= operand_bounds(*op).1 => current = r,
            Expr::Un {
                op: UnOp::Not, x, ..
            } if precedence(x) >= PREC_COMPARE => current = x,
            Expr::Un {
                op: UnOp::Neg, x, ..
            } if !neg_needs_parens(x) => current = x,
            _ => return false,
        }
    }
}

fn bin_precedence(op: BinOp) -> Option<u8> {
    match op {
        BinOp::Or => Some(PREC_OR),
        BinOp::And => Some(PREC_AND),
        BinOp::Eq
        | BinOp::Ne
        | BinOp::Lt
        | BinOp::Le
        | BinOp::Gt
        | BinOp::Ge
        | BinOp::Match
        | BinOp::NotMatch
        | BinOp::In => Some(PREC_COMPARE),
        BinOp::Add | BinOp::Sub => Some(PREC_ADD),
        BinOp::Mul | BinOp::Div => Some(PREC_MUL),
        _ => None,
    }
}

/// Smallest precedence of the left and the right operand printed without
/// parentheses. Comparisons are non-associative; every other level is
/// left-associative.
fn operand_bounds(op: BinOp) -> (u8, u8) {
    let level = bin_precedence(op).unwrap_or(PREC_COMPARE);
    if level == PREC_COMPARE {
        (level + 1, level + 1)
    } else {
        (level, level + 1)
    }
}

fn precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Bin { op, .. } => bin_precedence(*op).unwrap_or(PREC_COMPARE),
        Expr::Un { op: UnOp::Not, .. } => PREC_NOT,
        Expr::Un { .. } => PREC_NEG,
        _ => PREC_ATOM,
    }
}

/// A range directly under unary `-` would read as a range with a negative
/// bound, so it keeps its parentheses.
fn neg_needs_parens(operand: &Expr) -> bool {
    precedence(operand) < PREC_ATOM || matches!(operand, Expr::Range { .. })
}

/// Printer state: the variant and the first reason the tree cannot be read
/// back unchanged.
#[derive(Debug)]
pub(super) struct Printer {
    pub(super) variant: Variant,
    issue: Option<PrintError>,
}

impl Printer {
    pub(super) fn new(variant: Variant) -> Self {
        Self {
            variant,
            issue: None,
        }
    }

    pub(super) fn finish(self) -> Option<PrintError> {
        self.issue
    }

    pub(super) fn report(&mut self, error: PrintError) {
        if self.issue.is_none() {
            self.issue = Some(error);
        }
    }

    pub(super) fn unprintable(
        &mut self,
        node: &'static str,
        problem: &'static str,
        span: Option<Span>,
    ) {
        self.report(PrintError::Unprintable {
            node,
            problem,
            span,
        });
    }

    /// Returns `false` and records the failure when the context is deeper
    /// than the nesting limit.
    pub(super) fn enter(&mut self, ctx: Ctx) -> bool {
        if ctx.depth > MAX_NESTING_DEPTH {
            self.report(PrintError::TooDeep {
                limit: MAX_NESTING_DEPTH,
            });
            return false;
        }
        true
    }

    /// Prints `expr`, in parentheses when it binds looser than `min`.
    pub(super) fn expr(&mut self, expr: &Expr, min: u8, ctx: Ctx, out: &mut Out) {
        if !self.enter(ctx) {
            out.push("null");
            return;
        }
        if precedence(expr) < min {
            out.push("(");
            self.node(expr, ctx, out);
            out.push(")");
        } else {
            self.node(expr, ctx, out);
        }
    }

    fn node(&mut self, expr: &Expr, ctx: Ctx, out: &mut Out) {
        match expr {
            Expr::Ref(reference) => self.reference(reference, out),
            Expr::Key { segs, span } => self.key(segs, *span, ctx, out),
            Expr::List { items, .. } => self.list(items, ctx, out),
            Expr::Map { entries, span } => self.map(entries, *span, ctx, out),
            Expr::Spread { x, span } => {
                self.unprintable("spread", "is outside a list; spread only list items", *span);
                out.push("...");
                self.expr(x, PREC_ANY, ctx.deeper(), out);
            }
            Expr::Field { name, of, span } => self.field(name, of.as_deref(), *span, ctx, out),
            Expr::Call {
                function,
                args,
                span,
            } => self.call(function, args, *span, ctx, out),
            Expr::Un { op, x, span } => self.unary(*op, x, *span, ctx, out),
            Expr::Bin { op, l, r, span } => {
                if bin_precedence(*op).is_none() {
                    self.unprintable("bin", UNKNOWN_PROBLEM, *span);
                }
                self.binary(*op, l, r, ctx, out);
            }
            Expr::Sub { p, span } => self.sub(p, *span, ctx, out),
            other => self.scalar(other, out),
        }
    }

    fn scalar(&mut self, expr: &Expr, out: &mut Out) {
        match expr {
            Expr::Text { v, .. } => out.push(&literal::text(v)),
            Expr::Ident { v, span } => {
                if !literal::is_identifier(v) {
                    self.unprintable("ident", IDENT_PROBLEM, *span);
                }
                out.push(v);
            }
            Expr::Regex { v, flags, span } => {
                if let Some(problem) = self.regex_problem(v, flags) {
                    self.unprintable("regex", problem, *span);
                }
                out.push(&literal::regex(v, flags));
            }
            Expr::Bool { v, .. } => out.push(if *v { "true" } else { "false" }),
            Expr::Null { .. } => out.push("null"),
            Expr::Range { lo, hi, span } => self.range(*lo, *hi, *span, out),
            other => self.number(other, out),
        }
    }

    fn regex_problem(&self, pattern: &str, flags: &str) -> Option<&'static str> {
        let spaced = pattern.starts_with(char::is_whitespace);
        if spaced && self.variant != Variant::B {
            return Some(
                "starts with white space, which reads as division in variants A and C; escape the first character",
            );
        }
        literal::regex_problem(pattern, flags)
    }

    fn number(&mut self, expr: &Expr, out: &mut Out) {
        match expr {
            Expr::Int { v, span } => {
                if !literal::is_safe_integer(*v) {
                    self.unprintable("int", UNSAFE_PROBLEM, *span);
                }
                out.push(&v.to_string());
            }
            Expr::Num { v, span } => {
                if !(v.is_finite() && *v >= 0.0) {
                    self.unprintable(
                        "num",
                        "is negative or not finite; write a negative value as unary `-`",
                        *span,
                    );
                }
                out.push(&literal::number(*v));
            }
            Expr::Size { v, span } => {
                self.check_unsigned("size", *v, *span);
                out.push(&literal::size(*v));
            }
            Expr::Dur { v, span } => {
                self.check_unsigned("dur", *v, *span);
                out.push(&literal::duration(*v));
            }
            _ => {
                self.unprintable("expression", UNKNOWN_PROBLEM, None);
                out.push("null");
            }
        }
    }

    fn check_unsigned(&mut self, node: &'static str, value: u64, span: Option<Span>) {
        if value > literal::MAX_SAFE_INTEGER {
            self.unprintable(node, UNSAFE_PROBLEM, span);
        }
    }

    fn range(&mut self, lo: Option<i64>, hi: Option<i64>, span: Option<Span>, out: &mut Out) {
        if lo
            .into_iter()
            .chain(hi)
            .any(|bound| !literal::is_safe_integer(bound))
        {
            self.unprintable("range", UNSAFE_PROBLEM, span);
        }
        let mut text = String::new();
        if let Some(lo) = lo {
            let _ = write!(text, "{lo}");
        }
        text.push_str("..");
        if let Some(hi) = hi {
            let _ = write!(text, "{hi}");
        }
        out.push(&text);
    }

    pub(super) fn reference(&mut self, reference: &Ref, out: &mut Out) {
        let valid = literal::is_identifier(&reference.name)
            && reference.fields.iter().all(|f| literal::is_identifier(f));
        if !valid {
            self.unprintable("ref", NAME_PROBLEM, reference.span);
        }
        out.push("@");
        out.push(&reference.name);
        for field in &reference.fields {
            out.push(".");
            out.push(field);
        }
    }

    fn field(
        &mut self,
        name: &str,
        of: Option<&Expr>,
        span: Option<Span>,
        ctx: Ctx,
        out: &mut Out,
    ) {
        if !literal::is_identifier(name) {
            self.unprintable("field", NAME_PROBLEM, span);
        }
        let Some(of) = of else {
            self.unprintable(
                "field",
                "has no record expression and reads back as an identifier; set `of`",
                span,
            );
            out.push(name);
            return;
        };
        if matches!(
            of,
            Expr::Ident { .. } | Expr::Field { .. } | Expr::Call { .. }
        ) {
            self.expr(of, PREC_ANY, ctx.deeper(), out);
        } else {
            out.push("(");
            self.expr(of, PREC_ANY, ctx.deeper(), out);
            out.push(")");
        }
        out.push(".");
        out.push(name);
    }

    fn call(&mut self, function: &str, args: &[Expr], span: Option<Span>, ctx: Ctx, out: &mut Out) {
        if !literal::is_identifier(function) {
            self.unprintable("call", NAME_PROBLEM, span);
        }
        out.push(function);
        out.push("(");
        for (index, arg) in args.iter().enumerate() {
            if index > 0 {
                out.push(", ");
            }
            self.expr(arg, PREC_ANY, ctx.deeper(), out);
        }
        out.push(")");
    }

    fn unary(&mut self, op: UnOp, operand: &Expr, span: Option<Span>, ctx: Ctx, out: &mut Out) {
        match op {
            UnOp::Not => {
                out.push("not ");
                self.expr(operand, PREC_COMPARE, ctx.deeper(), out);
            }
            UnOp::Neg if matches!(operand, Expr::Range { .. }) => {
                self.unprintable(
                    "un",
                    "negates a range, which has no text form; negate an integer instead",
                    span,
                );
                out.push("-(");
                self.expr(operand, PREC_ANY, ctx.deeper(), out);
                out.push(")");
            }
            UnOp::Neg => {
                out.push("-");
                self.expr(operand, PREC_ATOM, ctx.deeper(), out);
            }
            _ => {
                self.unprintable("un", UNKNOWN_PROBLEM, span);
                out.push(op.as_str());
                out.push(" ");
                self.expr(operand, PREC_ATOM, ctx.deeper(), out);
            }
        }
    }

    /// A left operand that ends with `..` is parenthesized: the lexer reads
    /// the token after `..` as the start of a value, so `/` would open a
    /// regular expression.
    fn binary(&mut self, op: BinOp, left: &Expr, right: &Expr, ctx: Ctx, out: &mut Out) {
        let (left_min, right_min) = operand_bounds(op);
        if left_ends_open(left, left_min) {
            out.push("(");
            self.expr(left, PREC_ANY, ctx.deeper(), out);
            out.push(")");
        } else {
            self.expr(left, left_min, ctx.deeper(), out);
        }
        out.push(" ");
        out.push(op.as_str());
        out.push(" ");
        self.expr(right, right_min, ctx.deeper(), out);
    }

    fn list(&mut self, items: &[Expr], ctx: Ctx, out: &mut Out) {
        let Mode::Block { indent } = ctx.mode else {
            self.list_flat(items, ctx, out);
            return;
        };
        let nested = items.len() >= 2
            && items
                .iter()
                .any(|item| matches!(item, Expr::List { .. } | Expr::Map { .. }));
        if !nested {
            let mut flat = Out::default();
            self.list_flat(items, ctx, &mut flat);
            if items.is_empty() || fits(&flat.text, out.column) {
                out.push(&flat.text);
                return;
            }
        }
        out.push("[");
        let inner = ctx.indented(indent + INDENT_WIDTH);
        for (index, item) in items.iter().enumerate() {
            out.newline(indent + INDENT_WIDTH);
            self.list_item(item, inner, out);
            if index + 1 < items.len() {
                out.push(",");
            }
        }
        out.newline(indent);
        out.push("]");
    }

    fn list_flat(&mut self, items: &[Expr], ctx: Ctx, out: &mut Out) {
        out.push("[");
        let inner = ctx.flat().deeper();
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                out.push(", ");
            }
            self.list_item(item, inner, out);
        }
        out.push("]");
    }

    fn list_item(&mut self, item: &Expr, ctx: Ctx, out: &mut Out) {
        let Expr::Spread { x, .. } = item else {
            self.expr(item, PREC_ANY, ctx, out);
            return;
        };
        if self.enter(ctx) {
            out.push("...");
            self.expr(x, PREC_ANY, ctx.deeper(), out);
        } else {
            out.push("null");
        }
    }

    fn map(&mut self, entries: &[MapEntry], span: Option<Span>, ctx: Ctx, out: &mut Out) {
        let mut keys = HashSet::with_capacity(entries.len());
        if !entries.iter().all(|entry| keys.insert(entry.k.as_str())) {
            self.unprintable("map", "has a repeated key; keep one entry per key", span);
        }
        let Mode::Block { indent } = ctx.mode else {
            self.map_flat(entries, ctx, out);
            return;
        };
        let mut flat = Out::default();
        self.map_flat(entries, ctx, &mut flat);
        if entries.is_empty() || fits(&flat.text, out.column) {
            out.push(&flat.text);
            return;
        }
        out.push("{");
        let inner = ctx.indented(indent + INDENT_WIDTH);
        for (index, entry) in entries.iter().enumerate() {
            out.newline(indent + INDENT_WIDTH);
            self.map_entry(entry, inner, out);
            if index + 1 < entries.len() {
                out.push(",");
            }
        }
        out.newline(indent);
        out.push("}");
    }

    fn map_flat(&mut self, entries: &[MapEntry], ctx: Ctx, out: &mut Out) {
        out.push("{");
        let inner = ctx.flat().deeper();
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                out.push(", ");
            }
            self.map_entry(entry, inner, out);
        }
        out.push("}");
    }

    fn map_entry(&mut self, entry: &MapEntry, ctx: Ctx, out: &mut Out) {
        out.push(&literal::map_key(&entry.k));
        out.push(": ");
        self.expr(&entry.v, PREC_ANY, ctx, out);
    }

    /// Prints a structured key as the text of its key path.
    fn key(&mut self, segs: &[KeySeg], span: Option<Span>, ctx: Ctx, out: &mut Out) {
        self.unprintable(
            "key",
            "is a structured key and reads back as text; write the key path as text",
            span,
        );
        let mut path = String::new();
        for (index, seg) in segs.iter().enumerate() {
            match seg {
                KeySeg::Name(name) => {
                    let spelled = literal::key_name(name);
                    if index > 0 && !spelled.starts_with('[') {
                        path.push('.');
                    }
                    path.push_str(&spelled);
                }
                KeySeg::Index(position) => {
                    let _ = write!(path, "[{position}]");
                }
                KeySeg::Wild { wild } => path.push_str(wildcard(*wild, index > 0)),
                KeySeg::Filter { filter } => {
                    let mut condition = Out::default();
                    self.expr(filter, PREC_ANY, ctx.flat().deeper(), &mut condition);
                    let _ = write!(path, "[?{}]", condition.text);
                }
                _ => self.unprintable("key", UNKNOWN_PROBLEM, span),
            }
        }
        out.push(&literal::text(&path));
    }

    fn sub(&mut self, pipeline: &Pipeline, span: Option<Span>, ctx: Ctx, out: &mut Out) {
        self.check_sub(pipeline, span);
        out.push("(");
        self.pipeline_flat(pipeline, ctx.deeper(), out);
        out.push(")");
    }

    /// Records sub-pipelines whose parenthesized form reads back as an
    /// expression.
    pub(super) fn check_sub(&mut self, pipeline: &Pipeline, span: Option<Span>) {
        if !pipeline.stages.is_empty() {
            return;
        }
        match &pipeline.head {
            Head::Ref(_) => self.unprintable(
                "sub",
                "has only a reference and reads back as the reference; add a stage",
                span,
            ),
            Head::Stage(stage) if stage.args.is_empty() && self.variant != Variant::B => {
                self.unprintable(
                    "sub",
                    "has one stage without arguments and reads back as an identifier or a field",
                    span,
                );
            }
            _ => {}
        }
    }
}

fn wildcard(wild: Wildcard, after_segment: bool) -> &'static str {
    match wild {
        Wildcard::Deep if after_segment => ".**",
        Wildcard::Deep => "**",
        Wildcard::Append => "[+]",
        _ => "[*]",
    }
}
