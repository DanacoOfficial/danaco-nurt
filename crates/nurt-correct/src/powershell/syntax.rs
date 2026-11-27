// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::powershell::syntax
// Responsibility:   Statement structure of PowerShell text: statements, pipelines, commands with raw arguments and values.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Statement structure of PowerShell text: statements, pipelines, commands
//! with raw arguments and values.
//!
//! The parser does not know cmdlet signatures. A parameter token is kept
//! apart from the value that follows it; whether the parameter is a switch
//! or takes that value is decided when the arguments are bound against the
//! translation table.

use super::lex::{NumberLit, Redirect, StrPart, Token, TokenKind, VarRef};

/// A script: statements separated by `;` or line breaks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Script {
    pub(crate) statements: Vec<Statement>,
}

/// A statement with its byte range.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Statement {
    pub(crate) kind: StatementKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Kind of a statement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StatementKind {
    /// A pipeline.
    Pipeline(Pipeline),
    /// `$name = <pipeline>`.
    Assign {
        var: VarRef,
        var_start: usize,
        var_end: usize,
        value: Pipeline,
    },
}

/// Elements joined with `|`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pipeline {
    pub(crate) elements: Vec<Element>,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Element of a pipeline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Element {
    /// A command invocation.
    Command(Command),
    /// A value used as a pipeline element, `"text"`, `$x`, `(…)`.
    Expression(Value),
}

/// A command invocation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Command {
    /// Name as written, or the program value for `& program`.
    pub(crate) name: String,
    pub(crate) name_start: usize,
    pub(crate) name_end: usize,
    /// Invoked with the call operator `&`; the program is `program`.
    pub(crate) call: Option<Box<Value>>,
    pub(crate) args: Vec<RawArg>,
    pub(crate) redirections: Vec<Redirection>,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Argument of a command before binding.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawArg {
    /// `-Name` or `-Name:value`.
    Param {
        name: String,
        value: Option<Value>,
        start: usize,
        end: usize,
    },
    /// A value.
    Value(Value),
}

/// Output redirection with its target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Redirection {
    pub(crate) redirect: Redirect,
    pub(crate) target: Option<Value>,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// A value with its byte range.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Value {
    pub(crate) kind: ValueKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Kind of a value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ValueKind {
    /// Bare word in argument mode.
    Bare(String),
    /// Single-quoted text.
    Literal(String),
    /// Double-quoted text.
    Expandable(Vec<StrPart>),
    /// Number.
    Number(NumberLit),
    /// Variable.
    Variable(VarRef),
    /// Script block; the tokens between the braces.
    Block(Vec<Token>),
    /// `( … )`, `$( … )` or `@( … )`.
    Paren(Script),
    /// `@{ key = value; … }`.
    Hash(Vec<(String, Value)>),
    /// Comma-separated values.
    List(Vec<Value>),
    /// Type literal, `[int]`.
    Type(String),
    /// Member access or method call on a value.
    Member {
        base: Box<Value>,
        name: String,
        args: Option<Vec<Value>>,
    },
    /// Binary operation, `$a + $b`, `$x -replace 'a','b'`; the operator as
    /// written, with its dash for dash operators.
    Binary {
        op: String,
        left: Box<Value>,
        right: Box<Value>,
    },
}

impl Value {
    /// Text of a plain string value: a bare word, a single-quoted string or
    /// a double-quoted string without expansions.
    pub(crate) fn plain_text(&self) -> Option<String> {
        match &self.kind {
            ValueKind::Bare(text) | ValueKind::Literal(text) => Some(text.clone()),
            ValueKind::Expandable(parts) => match parts.as_slice() {
                [StrPart::Text(text)] => Some(text.clone()),
                [] => Some(String::new()),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Error of the statement parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub(crate) message: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Parses tokens into a script.
pub(crate) fn parse(tokens: &[Token]) -> Result<Script, SyntaxError> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let script = parser.script()?;
    match parser.peek() {
        Some(token) => Err(parser.unexpected(token)),
        None => Ok(script),
    }
}

/// Parses one value in expression mode starting at `pos`; returns the value
/// and the position after it.
pub(crate) fn parse_value(tokens: &[Token], pos: usize) -> Result<(Value, usize), SyntaxError> {
    let mut parser = Parser {
        tokens,
        pos,
        depth: 0,
    };
    let value = parser.value(true)?;
    Ok((value, parser.pos))
}

/// Deepest nesting of values that the parser reads; deeper text is an error
/// instead of an overflow of the stack.
pub(crate) const MAX_NESTING: usize = 64;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

fn is_terminator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Pipe
            | TokenKind::Semicolon
            | TokenKind::Newline
            | TokenKind::CloseParen
            | TokenKind::CloseBrace
    )
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&'a TokenKind> {
        self.peek().map(|token| &token.kind)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(token)
    }

    fn last_end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map_or(0, |token| token.end)
    }

    fn unexpected(&self, token: &Token) -> SyntaxError {
        SyntaxError {
            message: format!("unexpected {}", describe(&token.kind)),
            start: token.start,
            end: token.end,
        }
    }

    fn end_of_input(&self, what: &str) -> SyntaxError {
        let end = self.last_end();
        SyntaxError {
            message: format!("missing {what} at the end of the input"),
            start: end,
            end,
        }
    }

    fn skip_separators(&mut self) {
        while matches!(
            self.peek_kind(),
            Some(TokenKind::Semicolon | TokenKind::Newline)
        ) {
            self.pos += 1;
        }
    }

    fn script(&mut self) -> Result<Script, SyntaxError> {
        let mut statements = Vec::new();
        loop {
            self.skip_separators();
            match self.peek_kind() {
                None | Some(TokenKind::CloseParen | TokenKind::CloseBrace) => break,
                Some(_) => statements.push(self.statement()?),
            }
        }
        Ok(Script { statements })
    }

    fn statement(&mut self) -> Result<Statement, SyntaxError> {
        let first = self.peek().ok_or_else(|| self.end_of_input("statement"))?;
        let start = first.start;
        if let (TokenKind::Variable(var), Some(TokenKind::Assign)) = (
            &first.kind,
            self.tokens.get(self.pos + 1).map(|token| &token.kind),
        ) {
            self.pos += 2;
            while self.peek_kind() == Some(&TokenKind::Newline) {
                self.pos += 1;
            }
            let value = self.pipeline()?;
            let end = value.end;
            return Ok(Statement {
                kind: StatementKind::Assign {
                    var: var.clone(),
                    var_start: first.start,
                    var_end: first.end,
                    value,
                },
                start,
                end,
            });
        }
        let pipeline = self.pipeline()?;
        let end = pipeline.end;
        Ok(Statement {
            kind: StatementKind::Pipeline(pipeline),
            start,
            end,
        })
    }

    fn pipeline(&mut self) -> Result<Pipeline, SyntaxError> {
        let start = self
            .peek()
            .map(|token| token.start)
            .ok_or_else(|| self.end_of_input("pipeline"))?;
        let mut elements = vec![self.element()?];
        while self.peek_kind() == Some(&TokenKind::Pipe) {
            self.pos += 1;
            while self.peek_kind() == Some(&TokenKind::Newline) {
                self.pos += 1;
            }
            elements.push(self.element()?);
        }
        Ok(Pipeline {
            elements,
            start,
            end: self.last_end(),
        })
    }

    fn element(&mut self) -> Result<Element, SyntaxError> {
        let token = self.peek().ok_or_else(|| self.end_of_input("command"))?;
        match &token.kind {
            TokenKind::Ampersand => {
                self.pos += 1;
                let program = self.value(false)?;
                let mut command = Command {
                    name: program.plain_text().unwrap_or_default(),
                    name_start: program.start,
                    name_end: program.end,
                    call: Some(Box::new(program)),
                    args: Vec::new(),
                    redirections: Vec::new(),
                    start: token.start,
                    end: token.end,
                };
                self.arguments(&mut command)?;
                Ok(Element::Command(command))
            }
            TokenKind::Word(name) => {
                self.pos += 1;
                let mut command = Command {
                    name: name.clone(),
                    name_start: token.start,
                    name_end: token.end,
                    call: None,
                    args: Vec::new(),
                    redirections: Vec::new(),
                    start: token.start,
                    end: token.end,
                };
                self.arguments(&mut command)?;
                Ok(Element::Command(command))
            }
            kind if is_terminator(kind) => Err(self.unexpected(token)),
            _ => Ok(Element::Expression(self.expression()?)),
        }
    }

    fn arguments(&mut self, command: &mut Command) -> Result<(), SyntaxError> {
        while let Some(token) = self.peek() {
            match &token.kind {
                kind if is_terminator(kind) => break,
                TokenKind::Parameter { name, colon } => {
                    self.pos += 1;
                    let value = if *colon {
                        Some(self.value(false)?)
                    } else {
                        None
                    };
                    let end = value.as_ref().map_or(token.end, |value| value.end);
                    command.args.push(RawArg::Param {
                        name: name.clone(),
                        value,
                        start: token.start,
                        end,
                    });
                }
                TokenKind::Redirect(redirect) => {
                    self.pos += 1;
                    let target = if redirect.merge {
                        None
                    } else {
                        Some(self.value(false)?)
                    };
                    let end = target.as_ref().map_or(token.end, |value| value.end);
                    command.redirections.push(Redirection {
                        redirect: *redirect,
                        target,
                        start: token.start,
                        end,
                    });
                }
                _ => {
                    let value = self.list_value()?;
                    command.args.push(RawArg::Value(value));
                }
            }
        }
        command.end = self.last_end();
        Ok(())
    }

    /// A value followed by `, value` items.
    fn list_value(&mut self) -> Result<Value, SyntaxError> {
        let first = self.value(false)?;
        if self.peek_kind() != Some(&TokenKind::Comma) {
            return Ok(first);
        }
        let start = first.start;
        let mut items = vec![first];
        while self.peek_kind() == Some(&TokenKind::Comma) {
            self.pos += 1;
            while self.peek_kind() == Some(&TokenKind::Newline) {
                self.pos += 1;
            }
            items.push(self.value(false)?);
        }
        Ok(Value {
            kind: ValueKind::List(items),
            start,
            end: self.last_end(),
        })
    }

    /// A value followed by binary operators and further values, as in
    /// expression mode.
    fn expression(&mut self) -> Result<Value, SyntaxError> {
        let mut left = self.list_value_expr()?;
        let mut operators = 0_usize;
        while let Some(token) = self.peek() {
            let op = match &token.kind {
                TokenKind::Operator(op) => op.clone(),
                TokenKind::Parameter { name, .. } => format!("-{name}"),
                TokenKind::Word(word) if matches!(word.as_str(), "*" | "/" | "%") => word.clone(),
                _ => break,
            };
            operators += 1;
            if self.depth + operators > MAX_NESTING {
                return Err(too_deep(token));
            }
            self.pos += 1;
            while self.peek_kind() == Some(&TokenKind::Newline) {
                self.pos += 1;
            }
            let right = self.list_value_expr()?;
            let start = left.start;
            let end = right.end;
            left = Value {
                kind: ValueKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                start,
                end,
            };
        }
        Ok(left)
    }

    fn list_value_expr(&mut self) -> Result<Value, SyntaxError> {
        let first = self.value(true)?;
        if self.peek_kind() != Some(&TokenKind::Comma) {
            return Ok(first);
        }
        let start = first.start;
        let mut items = vec![first];
        while self.peek_kind() == Some(&TokenKind::Comma) {
            self.pos += 1;
            items.push(self.value(true)?);
        }
        Ok(Value {
            kind: ValueKind::List(items),
            start,
            end: self.last_end(),
        })
    }

    fn value(&mut self, expression_mode: bool) -> Result<Value, SyntaxError> {
        if self.depth >= MAX_NESTING {
            let token = self.peek().ok_or_else(|| self.end_of_input("value"))?;
            return Err(too_deep(token));
        }
        self.depth += 1;
        let value = self.nested_value(expression_mode);
        self.depth -= 1;
        value
    }

    fn nested_value(&mut self, expression_mode: bool) -> Result<Value, SyntaxError> {
        let token = self.next().ok_or_else(|| self.end_of_input("value"))?;
        let kind = match &token.kind {
            TokenKind::Word(word) => ValueKind::Bare(word.clone()),
            TokenKind::Operator(op) if !expression_mode => ValueKind::Bare(op.clone()),
            TokenKind::Literal(text) => ValueKind::Literal(text.clone()),
            TokenKind::Expandable(parts) => ValueKind::Expandable(parts.clone()),
            TokenKind::Number(number) => ValueKind::Number(*number),
            TokenKind::Variable(var) => ValueKind::Variable(var.clone()),
            TokenKind::TypeName(name) => ValueKind::Type(name.clone()),
            TokenKind::OpenBrace => ValueKind::Block(self.block_tokens(token)?),
            TokenKind::OpenParen | TokenKind::SubExprOpen | TokenKind::ArrayOpen => {
                let script = self.script()?;
                self.close(TokenKind::CloseParen, token)?;
                ValueKind::Paren(script)
            }
            TokenKind::HashOpen => ValueKind::Hash(self.hash_entries(token)?),
            _ => return Err(self.unexpected(token)),
        };
        let value = Value {
            kind,
            start: token.start,
            end: self.last_end(),
        };
        self.members(value)
    }

    fn close(&mut self, closer: TokenKind, opener: &Token) -> Result<(), SyntaxError> {
        match self.next() {
            Some(token) if token.kind == closer => Ok(()),
            Some(token) => Err(self.unexpected(token)),
            None => Err(SyntaxError {
                message: format!("{} without its closing pair", describe(&opener.kind)),
                start: opener.start,
                end: opener.end,
            }),
        }
    }

    fn block_tokens(&mut self, opener: &Token) -> Result<Vec<Token>, SyntaxError> {
        let mut depth = 1_usize;
        let start = self.pos;
        while let Some(token) = self.next() {
            match token.kind {
                TokenKind::OpenBrace | TokenKind::HashOpen => depth += 1,
                TokenKind::CloseBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self
                            .tokens
                            .get(start..self.pos - 1)
                            .unwrap_or_default()
                            .to_vec());
                    }
                }
                _ => {}
            }
        }
        Err(SyntaxError {
            message: "script block without closing brace".to_owned(),
            start: opener.start,
            end: opener.end,
        })
    }

    fn hash_entries(&mut self, opener: &Token) -> Result<Vec<(String, Value)>, SyntaxError> {
        let mut entries = Vec::new();
        loop {
            self.skip_separators();
            let token = self.next().ok_or_else(|| SyntaxError {
                message: "hash table without closing brace".to_owned(),
                start: opener.start,
                end: opener.end,
            })?;
            let key = match &token.kind {
                TokenKind::CloseBrace => return Ok(entries),
                TokenKind::Word(word) => word.clone(),
                TokenKind::Literal(text) => text.clone(),
                TokenKind::Expandable(parts) => match parts.as_slice() {
                    [StrPart::Text(text)] => text.clone(),
                    _ => return Err(self.unexpected(token)),
                },
                _ => return Err(self.unexpected(token)),
            };
            match self.next() {
                Some(token) if token.kind == TokenKind::Assign => {}
                Some(token) => return Err(self.unexpected(token)),
                None => return Err(self.end_of_input("=")),
            }
            let value = self.expression()?;
            entries.push((key, value));
        }
    }

    fn members(&mut self, mut value: Value) -> Result<Value, SyntaxError> {
        let mut accesses = 0_usize;
        while let Some(token) = self.peek() {
            let TokenKind::Member(name) = &token.kind else {
                break;
            };
            if token.start != value.end {
                break;
            }
            accesses += 1;
            if self.depth + accesses > MAX_NESTING {
                return Err(too_deep(token));
            }
            self.pos += 1;
            let mut end = token.end;
            let args = match self.peek() {
                Some(open) if open.kind == TokenKind::OpenParen && open.start == token.end => {
                    self.pos += 1;
                    let mut args = Vec::new();
                    while self.peek_kind() != Some(&TokenKind::CloseParen) {
                        args.push(self.expression()?);
                        if self.peek_kind() == Some(&TokenKind::Comma) {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    self.close(TokenKind::CloseParen, open)?;
                    end = self.last_end();
                    Some(args)
                }
                _ => None,
            };
            let start = value.start;
            value = Value {
                kind: ValueKind::Member {
                    base: Box::new(value),
                    name: name.clone(),
                    args,
                },
                start,
                end,
            };
        }
        Ok(value)
    }
}

fn too_deep(token: &Token) -> SyntaxError {
    SyntaxError {
        message: "values are nested too deeply".to_owned(),
        start: token.start,
        end: token.end,
    }
}

fn describe(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Word(word) => format!("word `{word}`"),
        TokenKind::Parameter { name, .. } => format!("parameter `-{name}`"),
        TokenKind::Literal(_) | TokenKind::Expandable(_) => "string".to_owned(),
        TokenKind::Number(_) => "number".to_owned(),
        TokenKind::Variable(var) => format!("variable `${}`", var.name),
        TokenKind::Member(name) => format!("member `.{name}`"),
        TokenKind::TypeName(name) => format!("type `[{name}]`"),
        TokenKind::Pipe => "`|`".to_owned(),
        TokenKind::Semicolon => "`;`".to_owned(),
        TokenKind::Newline => "line break".to_owned(),
        TokenKind::Ampersand => "`&`".to_owned(),
        TokenKind::Comma => "`,`".to_owned(),
        TokenKind::Assign => "`=`".to_owned(),
        TokenKind::OpenParen => "`(`".to_owned(),
        TokenKind::CloseParen => "`)`".to_owned(),
        TokenKind::OpenBrace => "`{`".to_owned(),
        TokenKind::CloseBrace => "`}`".to_owned(),
        TokenKind::SubExprOpen => "`$(`".to_owned(),
        TokenKind::ArrayOpen => "`@(`".to_owned(),
        TokenKind::HashOpen => "`@{`".to_owned(),
        TokenKind::Redirect(_) => "redirection".to_owned(),
        TokenKind::Operator(op) => format!("operator `{op}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::lex::lex;
    use super::*;

    fn script(input: &str) -> Script {
        let tokens = lex(input).unwrap_or_default();
        parse(&tokens).unwrap_or(Script {
            statements: Vec::new(),
        })
    }

    fn only_pipeline(input: &str) -> Pipeline {
        match script(input).statements.into_iter().next().map(|s| s.kind) {
            Some(StatementKind::Pipeline(pipeline)) => pipeline,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn command(element: &Element) -> &Command {
        match element {
            Element::Command(command) => command,
            Element::Expression(value) => panic!("unexpected expression {value:?}"),
        }
    }

    #[test]
    fn splits_statements_and_pipelines() {
        let parsed = script("a | b -X 1; c\n\nd");
        assert_eq!(parsed.statements.len(), 3);
        match &parsed.statements[0].kind {
            StatementKind::Pipeline(pipeline) => assert_eq!(pipeline.elements.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn keeps_parameters_apart_from_values() {
        let pipeline = only_pipeline("Copy-Item -Path a -Destination:b -Force");
        let args = &command(&pipeline.elements[0]).args;
        assert!(matches!(&args[0], RawArg::Param { name, value: None, .. } if name == "Path"));
        assert!(
            matches!(&args[1], RawArg::Value(Value { kind: ValueKind::Bare(text), .. }) if text == "a")
        );
        assert!(
            matches!(&args[2], RawArg::Param { name, value: Some(_), .. } if name == "Destination")
        );
        assert!(matches!(&args[3], RawArg::Param { name, value: None, .. } if name == "Force"));
    }

    #[test]
    fn comma_lists_span_white_space() {
        let pipeline = only_pipeline("Select-Object Id, Name, WorkingSet");
        let args = &command(&pipeline.elements[0]).args;
        assert!(
            matches!(&args[0], RawArg::Value(Value { kind: ValueKind::List(items), .. }) if items.len() == 3)
        );
    }

    #[test]
    fn assignment_takes_a_pipeline() {
        let parsed = script("$files = Get-ChildItem C:\\a | Sort-Object Name");
        match &parsed.statements[0].kind {
            StatementKind::Assign { var, value, .. } => {
                assert_eq!(var.name, "files");
                assert_eq!(value.elements.len(), 2);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn script_block_keeps_inner_tokens() {
        let pipeline = only_pipeline("Where-Object { $_.Length -gt 1MB }");
        match &command(&pipeline.elements[0]).args[0] {
            RawArg::Value(Value {
                kind: ValueKind::Block(tokens),
                ..
            }) => assert_eq!(tokens.len(), 4),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn method_call_on_parenthesized_command() {
        let tokens = lex("(Get-Date).AddDays(-1)").unwrap_or_default();
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
            depth: 0,
        };
        let value = parser.expression().ok();
        match value.map(|value| value.kind) {
            Some(ValueKind::Member {
                name,
                args: Some(args),
                base,
            }) => {
                assert_eq!(name, "AddDays");
                assert_eq!(args.len(), 1);
                assert!(matches!(base.kind, ValueKind::Paren(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn expression_element_with_operator() {
        let pipeline = only_pipeline("(Get-Content a) -replace 'x','y' | Set-Content a");
        match &pipeline.elements[0] {
            Element::Expression(Value {
                kind: ValueKind::Binary { op, right, .. },
                ..
            }) => {
                assert_eq!(op, "-replace");
                assert!(matches!(right.kind, ValueKind::List(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn call_operator_with_program_and_arguments() {
        let pipeline = only_pipeline("& 'C:\\Tools\\x.exe' a b");
        let element = command(&pipeline.elements[0]);
        assert!(element.call.is_some());
        assert_eq!(element.name, "C:\\Tools\\x.exe");
        assert_eq!(element.args.len(), 2);
    }

    #[test]
    fn hash_table_entries() {
        let pipeline = only_pipeline("iwr -Headers @{ Accept = 'text/plain'; 'X-Id' = \"7\" }");
        match &command(&pipeline.elements[0]).args[1] {
            RawArg::Value(Value {
                kind: ValueKind::Hash(entries),
                ..
            }) => {
                let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
                assert_eq!(keys, vec!["Accept", "X-Id"]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn redirection_target_is_read() {
        let pipeline = only_pipeline("Get-Process > C:\\p.txt");
        let element = command(&pipeline.elements[0]);
        assert_eq!(element.redirections.len(), 1);
        assert!(element.redirections[0].target.is_some());
    }

    #[test]
    fn reports_unbalanced_parenthesis() {
        let tokens = lex("Get-Item (a").unwrap_or_default();
        let error = parse(&tokens).err();
        assert!(error.is_some_and(|e| e.message.contains("without its closing pair")));
    }

    #[test]
    fn deeply_nested_values_are_a_syntax_error() {
        let nested = format!("a {}b{}", "(".repeat(10_000), ")".repeat(10_000));
        let tokens = lex(&nested).unwrap_or_default();
        let error = parse(&tokens).err();
        assert!(error.is_some_and(|e| e.message.contains("nested too deeply")));
        let hashes = format!("a {}", "@{ a = ".repeat(10_000));
        let tokens = lex(&hashes).unwrap_or_default();
        assert!(parse(&tokens).is_err());
    }

    #[test]
    fn long_chains_of_operators_and_members_are_a_syntax_error() {
        let operators = format!("1{}", " + 1".repeat(10_000));
        let tokens = lex(&operators).unwrap_or_default();
        let error = parse(&tokens).err();
        assert!(error.is_some_and(|e| e.message.contains("nested too deeply")));
        let members = format!("$x{}", ".a".repeat(10_000));
        let tokens = lex(&members).unwrap_or_default();
        let error = parse(&tokens).err();
        assert!(error.is_some_and(|e| e.message.contains("nested too deeply")));
    }

    #[test]
    fn moderately_nested_values_are_read() {
        let nested = format!("a {}b{}", "(".repeat(20), ")".repeat(20));
        let tokens = lex(&nested).unwrap_or_default();
        assert!(parse(&tokens).is_ok());
    }

    #[test]
    fn reports_stray_closing_brace() {
        let tokens = lex("a }").unwrap_or_default();
        assert!(parse(&tokens).is_err());
    }

    #[test]
    fn reports_missing_hash_assignment() {
        let tokens = lex("x @{ a 1 }").unwrap_or_default();
        assert!(parse(&tokens).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
