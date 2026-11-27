// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::bash::template
// Responsibility:   Result templates of the translation table.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Result templates of the translation table.
//!
//! A template is a pipeline in variant A in which `$name` marks a slot filled
//! from the translated command: `$1` to `$9` are operands, `$all`, `$rest`,
//! `$init` and `$last` are operand groups, and other names are bound by
//! options (`$value`) or by the interpreters of embedded languages. A slot
//! may name a conversion after a colon: `$1:text`, `$value:int`. A named
//! argument whose slot has no value is left out.

use std::collections::BTreeMap;

use nurt_contracts::ast::{Arg, BinOp, Expr, Head, MapEntry, Pipeline, Span, Stage, UnOp};

/// Conversion of a word bound to a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Conversion {
    /// A path: absolute, or resolved against the working directory.
    Path,
    /// Text with the exact bytes of the word.
    Text,
    /// A non-negative integer.
    Int,
    /// An identifier, lower-cased.
    Ident,
    /// A duration in seconds with an optional unit suffix.
    Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Slot {
    pub(crate) name: String,
    pub(crate) conversion: Conversion,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Const(Expr),
    Slot(Slot),
    List(Vec<Node>),
    Map(Vec<(String, Node)>),
    Call(String, Vec<Node>),
    Bin(BinOp, Box<Node>, Box<Node>),
    Not(Box<Node>),
    Range(Option<Box<Node>>, Option<Box<Node>>),
    Sub(TemplatePipeline),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemplateArg {
    pub(crate) name: Option<String>,
    pub(crate) value: Node,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemplateStage {
    pub(crate) name: String,
    pub(crate) args: Vec<TemplateArg>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemplatePipeline {
    pub(crate) stages: Vec<TemplateStage>,
}

/// Error of a template of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TemplateError(pub(crate) String);

/// Parses a template pipeline: `remove $all`, `find $1 |> remove`.
pub(crate) fn parse_pipeline(text: &str) -> Result<TemplatePipeline, TemplateError> {
    let mut parser = Parser::new(text)?;
    let pipeline = parser.pipeline()?;
    parser.end()?;
    Ok(pipeline)
}

/// Parses arguments added to a stage: `on_missing: skip hidden: false`.
pub(crate) fn parse_args(text: &str) -> Result<Vec<TemplateArg>, TemplateError> {
    let mut parser = Parser::new(text)?;
    let args = parser.args()?;
    parser.end()?;
    Ok(args)
}

/// Parses stages appended to a pipeline: `|> filter size > $size |> remove`.
pub(crate) fn parse_stages(text: &str) -> Result<Vec<TemplateStage>, TemplateError> {
    let mut parser = Parser::new(text)?;
    let mut stages = Vec::new();
    while parser.eat(&Tok::Pipe) {
        stages.push(parser.stage()?);
    }
    parser.end()?;
    Ok(stages)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Name(String),
    Text(String),
    Int(i64),
    Regex(String, String),
    Slot(Slot),
    Pipe,
    Colon,
    Comma,
    Dots,
    Open(char),
    Close(char),
    Op(BinOp),
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Result<Self, TemplateError> {
        Ok(Self {
            tokens: tokenize(text)?,
            pos: 0,
        })
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, token: &Tok) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Tok) -> Result<(), TemplateError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn error(&self, message: &str) -> TemplateError {
        TemplateError(format!("{message} at token {}", self.pos + 1))
    }

    fn end(&self) -> Result<(), TemplateError> {
        if self.pos == self.tokens.len() {
            Ok(())
        } else {
            Err(self.error("unexpected text after the template"))
        }
    }

    fn pipeline(&mut self) -> Result<TemplatePipeline, TemplateError> {
        let mut stages = vec![self.stage()?];
        while self.eat(&Tok::Pipe) {
            stages.push(self.stage()?);
        }
        Ok(TemplatePipeline { stages })
    }

    fn stage(&mut self) -> Result<TemplateStage, TemplateError> {
        let Some(Tok::Name(name)) = self.peek().cloned() else {
            return Err(self.error("expected a command name"));
        };
        self.pos += 1;
        let args = self.args()?;
        Ok(TemplateStage { name, args })
    }

    fn args(&mut self) -> Result<Vec<TemplateArg>, TemplateError> {
        let mut args = Vec::new();
        while let Some(token) = self.peek() {
            if matches!(token, Tok::Pipe | Tok::Close(_)) {
                break;
            }
            let named = matches!(
                (token, self.tokens.get(self.pos + 1)),
                (Tok::Name(_), Some(Tok::Colon))
            );
            let name = if named {
                let Some(Tok::Name(name)) = self.peek().cloned() else {
                    return Err(self.error("expected a parameter name"));
                };
                self.pos += 2;
                Some(name)
            } else {
                None
            };
            let value = self.expr()?;
            args.push(TemplateArg { name, value });
        }
        Ok(args)
    }

    fn expr(&mut self) -> Result<Node, TemplateError> {
        let mut left = self.conjunction()?;
        while self.eat(&Tok::Name("or".to_owned())) {
            let right = self.conjunction()?;
            left = Node::Bin(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn conjunction(&mut self) -> Result<Node, TemplateError> {
        let mut left = self.negation()?;
        while self.eat(&Tok::Name("and".to_owned())) {
            let right = self.negation()?;
            left = Node::Bin(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn negation(&mut self) -> Result<Node, TemplateError> {
        if self.eat(&Tok::Name("not".to_owned())) {
            return Ok(Node::Not(Box::new(self.negation()?)));
        }
        let left = self.primary()?;
        if let Some(Tok::Op(op)) = self.peek().cloned() {
            self.pos += 1;
            let right = self.primary()?;
            return Ok(Node::Bin(op, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn primary(&mut self) -> Result<Node, TemplateError> {
        if self.eat(&Tok::Dots) {
            let hi = self.atom()?;
            return Ok(Node::Range(None, Some(Box::new(hi))));
        }
        let node = self.atom()?;
        if self.eat(&Tok::Dots) {
            let bounded = matches!(self.peek(), Some(Tok::Int(_) | Tok::Slot(_)));
            let hi = if bounded {
                Some(Box::new(self.atom()?))
            } else {
                None
            };
            return Ok(Node::Range(Some(Box::new(node)), hi));
        }
        Ok(node)
    }

    fn atom(&mut self) -> Result<Node, TemplateError> {
        let Some(token) = self.peek().cloned() else {
            return Err(self.error("expected a value"));
        };
        self.pos += 1;
        match token {
            Tok::Text(v) => Ok(Node::Const(Expr::Text { v, span: None })),
            Tok::Int(v) => Ok(Node::Const(Expr::Int { v, span: None })),
            Tok::Regex(v, flags) => Ok(Node::Const(Expr::Regex {
                v,
                flags,
                span: None,
            })),
            Tok::Slot(slot) => Ok(Node::Slot(slot)),
            Tok::Name(name) => self.name(name),
            Tok::Open('[') => self.list(),
            Tok::Open('{') => self.map(),
            Tok::Open('(') => {
                let pipeline = self.pipeline()?;
                self.expect(&Tok::Close(')'))?;
                Ok(Node::Sub(pipeline))
            }
            _ => Err(self.error("expected a value")),
        }
    }

    fn name(&mut self, name: String) -> Result<Node, TemplateError> {
        match name.as_str() {
            "true" | "false" => {
                return Ok(Node::Const(Expr::Bool {
                    v: name == "true",
                    span: None,
                }));
            }
            "null" => return Ok(Node::Const(Expr::Null { span: None })),
            _ => {}
        }
        if self.eat(&Tok::Open('(')) {
            let mut args = Vec::new();
            while !self.eat(&Tok::Close(')')) {
                args.push(self.expr()?);
                self.eat(&Tok::Comma);
            }
            return Ok(Node::Call(name, args));
        }
        Ok(Node::Const(Expr::Ident {
            v: name,
            span: None,
        }))
    }

    fn list(&mut self) -> Result<Node, TemplateError> {
        let mut items = Vec::new();
        while !self.eat(&Tok::Close(']')) {
            items.push(self.expr()?);
            if !self.eat(&Tok::Comma) {
                self.expect(&Tok::Close(']'))?;
                break;
            }
        }
        Ok(Node::List(items))
    }

    fn map(&mut self) -> Result<Node, TemplateError> {
        let mut entries = Vec::new();
        while !self.eat(&Tok::Close('}')) {
            let key = match self.peek().cloned() {
                Some(Tok::Name(key) | Tok::Text(key)) => key,
                _ => return Err(self.error("expected a map key")),
            };
            self.pos += 1;
            self.expect(&Tok::Colon)?;
            entries.push((key, self.expr()?));
            if !self.eat(&Tok::Comma) {
                self.expect(&Tok::Close('}'))?;
                break;
            }
        }
        Ok(Node::Map(entries))
    }
}

fn tokenize(text: &str) -> Result<Vec<Tok>, TemplateError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let (token, next) = match c {
            ' ' | '\t' | '\n' => {
                i += 1;
                continue;
            }
            '|' if chars.get(i + 1) == Some(&'>') => (Tok::Pipe, i + 2),
            ':' => (Tok::Colon, i + 1),
            ',' => (Tok::Comma, i + 1),
            '.' if chars.get(i + 1) == Some(&'.') => (Tok::Dots, i + 2),
            '[' | '{' | '(' => (Tok::Open(c), i + 1),
            ']' | '}' | ')' => (Tok::Close(c), i + 1),
            '"' => read_text(&chars, i)?,
            '/' => read_regex(&chars, i)?,
            '$' => read_slot(&chars, i)?,
            '0'..='9' => read_int(&chars, i)?,
            c if c.is_ascii_lowercase() || c == '_' => read_name(&chars, i),
            _ => read_operator(&chars, i)?,
        };
        tokens.push(token);
        i = next;
    }
    Ok(tokens)
}

fn read_text(chars: &[char], start: usize) -> Result<(Tok, usize), TemplateError> {
    let mut text = String::new();
    let mut i = start + 1;
    loop {
        match chars.get(i) {
            None => return Err(TemplateError("unterminated text".to_owned())),
            Some('"') => return Ok((Tok::Text(text), i + 1)),
            Some('\\') => {
                let escaped = match chars.get(i + 1) {
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some(&c) => c,
                    None => return Err(TemplateError("unterminated text".to_owned())),
                };
                text.push(escaped);
                i += 2;
            }
            Some(&c) => {
                text.push(c);
                i += 1;
            }
        }
    }
}

fn read_regex(chars: &[char], start: usize) -> Result<(Tok, usize), TemplateError> {
    let mut pattern = String::new();
    let mut i = start + 1;
    loop {
        match chars.get(i) {
            None => return Err(TemplateError("unterminated regex".to_owned())),
            Some('/') => break,
            Some('\\') if chars.get(i + 1) == Some(&'/') => {
                pattern.push('/');
                i += 2;
            }
            Some(&c) => {
                pattern.push(c);
                i += 1;
            }
        }
    }
    i += 1;
    let mut flags = String::new();
    while let Some(&c) = chars.get(i) {
        if !"imsx".contains(c) {
            break;
        }
        flags.push(c);
        i += 1;
    }
    Ok((Tok::Regex(pattern, flags), i))
}

fn read_slot(chars: &[char], start: usize) -> Result<(Tok, usize), TemplateError> {
    let (name, mut i) = read_word(chars, start + 1);
    if name.is_empty() {
        return Err(TemplateError("empty slot name".to_owned()));
    }
    let mut conversion = Conversion::Path;
    if chars.get(i) == Some(&':') && chars.get(i + 1).is_some_and(char::is_ascii_lowercase) {
        let (kind, next) = read_word(chars, i + 1);
        conversion = match kind.as_str() {
            "path" => Conversion::Path,
            "text" => Conversion::Text,
            "int" => Conversion::Int,
            "ident" => Conversion::Ident,
            "duration" => Conversion::Duration,
            _ => return Err(TemplateError(format!("unknown slot conversion `{kind}`"))),
        };
        i = next;
    }
    Ok((Tok::Slot(Slot { name, conversion }), i))
}

fn read_word(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    let mut word = String::new();
    while let Some(&c) = chars.get(i) {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
            i += 1;
        } else {
            break;
        }
    }
    (word, i)
}

fn read_int(chars: &[char], start: usize) -> Result<(Tok, usize), TemplateError> {
    let (digits, i) = read_word(chars, start);
    let value = digits
        .parse()
        .map_err(|_| TemplateError(format!("invalid integer `{digits}`")))?;
    Ok((Tok::Int(value), i))
}

fn read_name(chars: &[char], start: usize) -> (Tok, usize) {
    let mut i = start;
    let mut name = String::new();
    while let Some(&c) = chars.get(i) {
        let dotted = c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || dotted {
            name.push(c);
            i += 1;
        } else {
            break;
        }
    }
    (Tok::Name(name), i)
}

fn read_operator(chars: &[char], start: usize) -> Result<(Tok, usize), TemplateError> {
    const OPERATORS: &[(&str, BinOp)] = &[
        ("==", BinOp::Eq),
        ("!=", BinOp::Ne),
        ("<=", BinOp::Le),
        (">=", BinOp::Ge),
        ("!~", BinOp::NotMatch),
        ("<", BinOp::Lt),
        (">", BinOp::Gt),
        ("~", BinOp::Match),
    ];
    let rest: String = chars[start..].iter().take(2).collect();
    OPERATORS
        .iter()
        .find(|(text, _)| rest.starts_with(text))
        .map(|&(text, op)| (Tok::Op(op), start + text.len()))
        .ok_or_else(|| TemplateError(format!("unexpected character `{}`", chars[start])))
}

/// Values bound to the slots of a template.
#[derive(Debug, Clone, Default)]
pub(crate) struct Bindings {
    slots: BTreeMap<String, Vec<Bound>>,
}

/// A value bound to a slot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Bound {
    /// A word of the command, converted by the slot.
    Word {
        text: String,
        span: Span,
        bare: bool,
    },
    /// A ready node.
    Node(Expr),
}

impl Bindings {
    pub(crate) fn set(&mut self, name: &str, values: Vec<Bound>) {
        self.slots.insert(name.to_owned(), values);
    }

    pub(crate) fn push(&mut self, name: &str, value: Bound) {
        self.slots.entry(name.to_owned()).or_default().push(value);
    }

    pub(crate) fn get(&self, name: &str) -> &[Bound] {
        self.slots.get(name).map_or(&[], Vec::as_slice)
    }
}

/// Converts a bound word into a node; implemented by the translator, which
/// knows the working directory and the call parameter `base`.
pub(crate) trait Convert {
    type Error;

    fn convert(&self, bound: &Bound, conversion: Conversion) -> Result<Expr, Self::Error>;

    fn missing(&self, slot: &str) -> Self::Error;
}

pub(crate) fn fill_pipeline<C: Convert>(
    template: &TemplatePipeline,
    bindings: &Bindings,
    convert: &C,
    span: Option<Span>,
) -> Result<Vec<Stage>, C::Error> {
    template
        .stages
        .iter()
        .map(|stage| fill_stage(stage, bindings, convert, span))
        .collect()
}

pub(crate) fn fill_stage<C: Convert>(
    stage: &TemplateStage,
    bindings: &Bindings,
    convert: &C,
    span: Option<Span>,
) -> Result<Stage, C::Error> {
    Ok(Stage {
        name: stage.name.clone(),
        args: fill_args(&stage.args, bindings, convert)?,
        span,
    })
}

pub(crate) fn fill_args<C: Convert>(
    args: &[TemplateArg],
    bindings: &Bindings,
    convert: &C,
) -> Result<Vec<Arg>, C::Error> {
    let mut filled = Vec::new();
    for arg in args {
        let values = fill_node(&arg.value, bindings, convert)?;
        match (&arg.name, values.len()) {
            (Some(_), 0) => {}
            (None, 0) => return Err(convert.missing(slot_name(&arg.value))),
            (Some(name), 1) => filled.extend(values.into_iter().map(|value| Arg {
                name: Some(name.clone()),
                span: expr_span(&value),
                value,
            })),
            (Some(name), _) => filled.push(Arg {
                name: Some(name.clone()),
                span: None,
                value: Expr::List {
                    items: values,
                    span: None,
                },
            }),
            (None, _) => filled.extend(values.into_iter().map(|value| Arg {
                name: None,
                span: expr_span(&value),
                value,
            })),
        }
    }
    Ok(filled)
}

fn slot_name(node: &Node) -> &str {
    match node {
        Node::Slot(slot) => &slot.name,
        _ => "value",
    }
}

/// Fills a node; a slot yields one node per bound value.
fn fill_node<C: Convert>(
    node: &Node,
    bindings: &Bindings,
    convert: &C,
) -> Result<Vec<Expr>, C::Error> {
    let single = |expr: Expr| Ok(vec![expr]);
    match node {
        Node::Const(expr) => single(expr.clone()),
        Node::Slot(slot) => bindings
            .get(&slot.name)
            .iter()
            .map(|bound| convert.convert(bound, slot.conversion))
            .collect(),
        Node::List(items) => {
            let mut filled = Vec::new();
            for item in items {
                filled.extend(fill_node(item, bindings, convert)?);
            }
            single(Expr::List {
                items: filled,
                span: None,
            })
        }
        Node::Map(entries) => {
            let mut filled = Vec::new();
            for (key, value) in entries {
                if let Some(v) = fill_one(value, bindings, convert)? {
                    filled.push(MapEntry {
                        k: key.clone(),
                        v,
                        span: None,
                    });
                }
            }
            single(Expr::Map {
                entries: filled,
                span: None,
            })
        }
        Node::Call(function, args) => {
            let mut filled = Vec::new();
            for arg in args {
                filled.extend(fill_node(arg, bindings, convert)?);
            }
            single(Expr::Call {
                function: function.clone(),
                args: filled,
                span: None,
            })
        }
        Node::Bin(op, left, right) => {
            let l = required(left, bindings, convert)?;
            let r = required(right, bindings, convert)?;
            single(Expr::Bin {
                op: *op,
                l: Box::new(l),
                r: Box::new(r),
                span: None,
            })
        }
        Node::Not(operand) => single(Expr::Un {
            op: UnOp::Not,
            x: Box::new(required(operand, bindings, convert)?),
            span: None,
        }),
        Node::Range(lo, hi) => single(Expr::Range {
            lo: bound_int(lo.as_deref(), bindings, convert)?,
            hi: bound_int(hi.as_deref(), bindings, convert)?,
            span: None,
        }),
        Node::Sub(pipeline) => {
            let mut stages = fill_pipeline(pipeline, bindings, convert, None)?.into_iter();
            let Some(head) = stages.next() else {
                return Err(convert.missing("pipeline"));
            };
            single(Expr::Sub {
                p: Pipeline {
                    head: Head::Stage(head),
                    stages: stages.collect(),
                    span: None,
                },
                span: None,
            })
        }
    }
}

fn fill_one<C: Convert>(
    node: &Node,
    bindings: &Bindings,
    convert: &C,
) -> Result<Option<Expr>, C::Error> {
    Ok(fill_node(node, bindings, convert)?.into_iter().next())
}

fn required<C: Convert>(node: &Node, bindings: &Bindings, convert: &C) -> Result<Expr, C::Error> {
    fill_one(node, bindings, convert)?.ok_or_else(|| convert.missing(slot_name(node)))
}

fn bound_int<C: Convert>(
    node: Option<&Node>,
    bindings: &Bindings,
    convert: &C,
) -> Result<Option<i64>, C::Error> {
    let Some(node) = node else {
        return Ok(None);
    };
    match required(node, bindings, convert)? {
        Expr::Int { v, .. } => Ok(Some(v)),
        _ => Err(convert.missing(slot_name(node))),
    }
}

pub(crate) fn expr_span(expr: &Expr) -> Option<Span> {
    match expr {
        Expr::Ref(reference) => reference.span,
        Expr::Text { span, .. }
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
        | Expr::Sub { span, .. } => *span,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::print_stage;
    use super::*;

    struct Plain;

    impl Convert for Plain {
        type Error = String;

        fn convert(&self, bound: &Bound, conversion: Conversion) -> Result<Expr, String> {
            match (bound, conversion) {
                (Bound::Node(expr), _) => Ok(expr.clone()),
                (Bound::Word { text, .. }, Conversion::Int) => text
                    .parse()
                    .map(|v| Expr::Int { v, span: None })
                    .map_err(|_| format!("not an int: {text}")),
                (Bound::Word { text, .. }, _) => Ok(Expr::Text {
                    v: text.clone(),
                    span: None,
                }),
            }
        }

        fn missing(&self, slot: &str) -> String {
            format!("missing {slot}")
        }
    }

    fn word(text: &str) -> Bound {
        Bound::Word {
            text: text.to_owned(),
            span: Span {
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 2,
            },
            bare: true,
        }
    }

    fn render(template: &str, bindings: &Bindings) -> Result<String, String> {
        let parsed = parse_pipeline(template).map_err(|e| e.0)?;
        let stages = fill_pipeline(&parsed, bindings, &Plain, None)?;
        Ok(stages
            .iter()
            .map(print_stage)
            .collect::<Vec<_>>()
            .join(" |> "))
    }

    #[test]
    fn slots_spread_into_positional_arguments() {
        let mut bindings = Bindings::default();
        bindings.set("all", vec![word("/a"), word("/b")]);
        assert_eq!(
            render("remove $all", &bindings).unwrap(),
            "remove \"/a\" \"/b\""
        );
    }

    #[test]
    fn named_argument_without_value_is_left_out() {
        let mut bindings = Bindings::default();
        bindings.set("1", vec![word("/f")]);
        bindings.set("user", vec![word("u")]);
        assert_eq!(
            render(
                "attributes $1 owner: $user:text group: $group:text",
                &bindings
            )
            .unwrap(),
            "attributes \"/f\" owner: \"u\""
        );
    }

    #[test]
    fn missing_positional_slot_is_an_error() {
        assert_eq!(
            render("read $1", &Bindings::default()).unwrap_err(),
            "missing 1"
        );
    }

    #[test]
    fn ranges_lists_maps_calls_and_conditions() {
        let mut bindings = Bindings::default();
        bindings.set("value", vec![word("10")]);
        bindings.set("1", vec![word("/f")]);
        assert_eq!(
            render("read $1 lines: ..$value:int", &bindings).unwrap(),
            "read \"/f\" lines: ..10"
        );
        assert_eq!(
            render("read $1 lines: $value:int..", &bindings).unwrap(),
            "read \"/f\" lines: 10.."
        );
        assert_eq!(
            render("run [\"x\", $1:text] env: {\"A\": $value:text}", &bindings).unwrap(),
            "run [\"x\", \"/f\"] env: {\"A\": \"10\"}"
        );
        assert_eq!(
            render(
                "filter size > 0 and not hidden |> filter text ~ /a\\/b/i",
                &bindings
            )
            .unwrap(),
            "filter size > 0 and not hidden |> filter text ~ /a\\/b/i"
        );
        assert_eq!(
            render("evaluate now()", &bindings).unwrap(),
            "evaluate now()"
        );
        assert_eq!(
            render(
                "write $1 from: (lines $1 |> count) mode: overwrite",
                &bindings
            )
            .unwrap(),
            "write \"/f\" from: (lines \"/f\" |> count) mode: overwrite"
        );
    }

    #[test]
    fn named_slot_with_several_values_becomes_a_list() {
        let mut bindings = Bindings::default();
        bindings.set("paths", vec![word("/a"), word("/b")]);
        assert_eq!(
            render("info in: $paths", &bindings).unwrap(),
            "info in: [\"/a\", \"/b\"]"
        );
    }

    #[test]
    fn args_and_stages_fragments() {
        let args = parse_args("on_missing: skip hidden: false").unwrap();
        assert_eq!(args.len(), 2);
        let stages = parse_stages("|> filter size > 1 |> remove").unwrap();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[1].name, "remove");
        assert!(parse_stages("").unwrap().is_empty());
    }

    #[test]
    fn malformed_templates_are_rejected() {
        assert!(parse_pipeline("read \"x").is_err());
        assert!(parse_pipeline("read $").is_err());
        assert!(parse_pipeline("read $1:float").is_err());
        assert!(parse_pipeline("read /x").is_err());
        assert!(parse_pipeline("read # x").is_err());
        assert!(parse_pipeline("read x )").is_err());
        assert!(parse_pipeline("|> read").is_err());
        assert!(parse_args("{1: x}").is_err());
        assert!(parse_pipeline("read 99999999999999999999").is_err());
    }

    #[test]
    fn constants_keep_their_kind() {
        let parsed =
            parse_pipeline("x a: true b: null c: \"t\\n\" d: [1, 2] e: git.status").unwrap();
        let args = &parsed.stages[0].args;
        assert_eq!(
            args[0].value,
            Node::Const(Expr::Bool {
                v: true,
                span: None
            })
        );
        assert_eq!(args[1].value, Node::Const(Expr::Null { span: None }));
        assert_eq!(
            args[2].value,
            Node::Const(Expr::Text {
                v: "t\n".to_owned(),
                span: None
            })
        );
        assert!(matches!(&args[3].value, Node::List(items) if items.len() == 2));
        assert_eq!(
            args[4].value,
            Node::Const(Expr::Ident {
                v: "git.status".to_owned(),
                span: None
            })
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
