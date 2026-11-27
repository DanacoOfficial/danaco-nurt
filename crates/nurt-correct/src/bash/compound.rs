// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::bash::compound
// Responsibility:   Structure of compound shell commands: `if`, `for`, `while`, `until`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Structure of compound shell commands: `if`, `for`, `while`, `until`,
//! `case`, function definitions and groups, with their bodies.
//!
//! Simple commands and pipelines stay as source spans; the translator reads
//! the text of a span with the translation table of simple commands.

use nurt_contracts::ast::Span;

use super::lex::{Operator, PosixToken, PosixTokenKind, Quoting, Word, WordPart};
use super::syntax::{Connector, join};

/// A list of commands.
#[derive(Debug, Clone, Default)]
pub(crate) struct Block {
    pub(crate) entries: Vec<Entry>,
}

/// And-or chain of commands.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) first: Node,
    pub(crate) rest: Vec<(Connector, Node)>,
    pub(crate) background: bool,
}

impl Entry {
    pub(crate) fn span(&self) -> Span {
        self.rest
            .iter()
            .fold(self.first.span(), |span, (_, node)| join(span, node.span()))
    }
}

/// A command of a list.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    /// A simple command or a pipeline of simple commands.
    Simple(Simple),
    If {
        arms: Vec<(Block, Block)>,
        other: Option<Block>,
        span: Span,
    },
    For {
        name: String,
        items: Option<Vec<Word>>,
        body: Block,
        span: Span,
    },
    While {
        until: bool,
        condition: Block,
        body: Block,
        span: Span,
    },
    Case {
        subject: Word,
        arms: Vec<CaseArm>,
        span: Span,
    },
    Function {
        name: String,
        span: Span,
    },
    Group {
        body: Block,
        span: Span,
    },
    /// A construct without a translation, such as a subshell.
    Unsupported {
        keyword: String,
        span: Span,
    },
}

impl Node {
    pub(crate) fn span(&self) -> Span {
        match self {
            Self::Simple(simple) => simple.span,
            Self::If { span, .. }
            | Self::For { span, .. }
            | Self::While { span, .. }
            | Self::Case { span, .. }
            | Self::Function { span, .. }
            | Self::Group { span, .. }
            | Self::Unsupported { span, .. } => *span,
        }
    }
}

/// A simple command with the words of its first command.
#[derive(Debug, Clone)]
pub(crate) struct Simple {
    pub(crate) span: Span,
    pub(crate) words: Vec<Word>,
    pub(crate) negated: bool,
    /// The words are the content of `[[ … ]]`.
    pub(crate) double_bracket: bool,
}

/// An arm of a `case` command.
#[derive(Debug, Clone)]
pub(crate) struct CaseArm {
    pub(crate) patterns: Vec<Word>,
    pub(crate) body: Block,
}

/// Error of the structure of compound commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseError {
    pub(crate) message: &'static str,
    pub(crate) span: Span,
}

/// Whether the tokens contain a compound command.
pub(crate) fn has_compound(tokens: &[PosixToken]) -> bool {
    let mut at_start = true;
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            PosixTokenKind::Word(word) if at_start => {
                let next = tokens.get(index + 1).map(|t| &t.kind);
                if is_keyword(&word.raw)
                    || matches!(next, Some(PosixTokenKind::Operator(Operator::OpenParen)))
                {
                    return true;
                }
                at_start = false;
            }
            PosixTokenKind::Operator(
                Operator::Semicolon | Operator::And | Operator::Or | Operator::Pipe,
            )
            | PosixTokenKind::Newline => at_start = true,
            PosixTokenKind::Operator(Operator::OpenParen) if at_start => return true,
            _ => at_start = false,
        }
    }
    false
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "for" | "while" | "until" | "case" | "function" | "{" | "select"
    )
}

pub(crate) fn parse(tokens: &[PosixToken]) -> Result<Block, ParseError> {
    let mut parser = Parser { tokens, pos: 0 };
    let block = parser.block(&[])?;
    match parser.peek() {
        None => Ok(block),
        Some(token) => Err(ParseError {
            message: "unexpected token",
            span: token.span,
        }),
    }
}

struct Parser<'t> {
    tokens: &'t [PosixToken],
    pos: usize,
}

const BLOCK_END: &[&str] = &["then", "elif", "else", "fi", "do", "done", "esac", "}"];

impl Parser<'_> {
    fn peek(&self) -> Option<&PosixToken> {
        self.tokens.get(self.pos)
    }

    fn kind(&self, offset: usize) -> Option<&PosixTokenKind> {
        self.tokens.get(self.pos + offset).map(|token| &token.kind)
    }

    fn last_span(&self) -> Span {
        self.tokens
            .get(self.pos.saturating_sub(1))
            .or_else(|| self.tokens.last())
            .map_or(EMPTY, |token| token.span)
    }

    fn error<T>(&self, message: &'static str) -> Result<T, ParseError> {
        Err(ParseError {
            message,
            span: self.peek().map_or_else(|| self.last_span(), |t| t.span),
        })
    }

    fn word_is(&self, text: &str) -> bool {
        matches!(self.kind(0), Some(PosixTokenKind::Word(word)) if word.raw == text)
    }

    fn skip_separators(&mut self) {
        while matches!(
            self.kind(0),
            Some(
                PosixTokenKind::Newline
                    | PosixTokenKind::Comment(_)
                    | PosixTokenKind::Operator(Operator::Semicolon)
            )
        ) {
            self.pos += 1;
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(
            self.kind(0),
            Some(PosixTokenKind::Newline | PosixTokenKind::Comment(_))
        ) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, keyword: &'static str) -> Result<(), ParseError> {
        self.skip_separators();
        if self.word_is(keyword) {
            self.pos += 1;
            Ok(())
        } else {
            self.error(keyword)
        }
    }

    /// Reads commands until one of the stop words (or the end).
    fn block(&mut self, stops: &[&str]) -> Result<Block, ParseError> {
        let mut entries = Vec::new();
        loop {
            self.skip_separators();
            let Some(token) = self.peek() else { break };
            if let PosixTokenKind::Word(word) = &token.kind
                && (stops.contains(&word.raw.as_str()) || BLOCK_END.contains(&word.raw.as_str()))
            {
                break;
            }
            if matches!(
                token.kind,
                PosixTokenKind::Operator(Operator::CaseEnd | Operator::CloseParen)
            ) {
                break;
            }
            entries.push(self.entry()?);
        }
        Ok(Block { entries })
    }

    fn entry(&mut self) -> Result<Entry, ParseError> {
        let first = self.node()?;
        let mut rest = Vec::new();
        loop {
            let connector = match self.kind(0) {
                Some(PosixTokenKind::Operator(Operator::And)) => Connector::And,
                Some(PosixTokenKind::Operator(Operator::Or)) => Connector::Or,
                _ => break,
            };
            self.pos += 1;
            self.skip_newlines();
            rest.push((connector, self.node()?));
        }
        let background = matches!(
            self.kind(0),
            Some(PosixTokenKind::Operator(Operator::Background))
        );
        if background {
            self.pos += 1;
        }
        Ok(Entry {
            first,
            rest,
            background,
        })
    }

    fn node(&mut self) -> Result<Node, ParseError> {
        let Some(token) = self.peek() else {
            return self.error("expected a command");
        };
        let span = token.span;
        match &token.kind {
            PosixTokenKind::Operator(Operator::OpenParen) => {
                self.pos = self.tokens.len();
                Ok(Node::Unsupported {
                    keyword: "(".to_owned(),
                    span,
                })
            }
            PosixTokenKind::Word(word) => {
                let raw = word.raw.clone();
                match raw.as_str() {
                    "if" => self.if_command(),
                    "for" => self.for_command(),
                    "while" | "until" => self.while_command(raw == "until"),
                    "case" => self.case_command(),
                    "function" => self.function_keyword(),
                    "{" => self.group(),
                    "select" | "coproc" => {
                        self.pos = self.tokens.len();
                        Ok(Node::Unsupported { keyword: raw, span })
                    }
                    _ if self.is_function_header() => self.function_header(),
                    _ => self.simple(),
                }
            }
            PosixTokenKind::Redirect(_) => self.simple(),
            _ => self.error("expected a command"),
        }
    }

    fn is_function_header(&self) -> bool {
        matches!(
            self.kind(1),
            Some(PosixTokenKind::Operator(Operator::OpenParen))
        ) && matches!(
            self.kind(2),
            Some(PosixTokenKind::Operator(Operator::CloseParen))
        )
    }

    fn if_command(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        self.pos += 1;
        let mut arms = Vec::new();
        let mut other = None;
        let mut condition = self.block(&["then"])?;
        loop {
            self.expect("then")?;
            let body = self.block(&[])?;
            arms.push((condition, body));
            self.skip_separators();
            if self.word_is("elif") {
                self.pos += 1;
                condition = self.block(&["then"])?;
            } else if self.word_is("else") {
                self.pos += 1;
                other = Some(self.block(&[])?);
                self.expect("fi")?;
                break;
            } else {
                self.expect("fi")?;
                break;
            }
        }
        Ok(Node::If {
            arms,
            other,
            span: join(start, self.last_span()),
        })
    }

    fn last_or_current(&self) -> Span {
        self.peek().map_or(EMPTY, |token| token.span)
    }

    fn for_command(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        self.pos += 1;
        let name = match self.kind(0) {
            Some(PosixTokenKind::Word(word)) if is_name(&word.raw) => word.raw.clone(),
            _ => {
                self.pos = self.tokens.len();
                return Ok(Node::Unsupported {
                    keyword: "for ((".to_owned(),
                    span: start,
                });
            }
        };
        self.pos += 1;
        let mut items = None;
        if self.word_is("in") {
            self.pos += 1;
            let mut words = Vec::new();
            while let Some(PosixTokenKind::Word(word)) = self.kind(0) {
                words.push(word.clone());
                self.pos += 1;
            }
            items = Some(words);
        }
        self.expect("do")?;
        let body = self.block(&[])?;
        self.expect("done")?;
        Ok(Node::For {
            name,
            items,
            body,
            span: join(start, self.last_span()),
        })
    }

    fn while_command(&mut self, until: bool) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        self.pos += 1;
        let condition = self.block(&["do"])?;
        self.expect("do")?;
        let body = self.block(&[])?;
        self.expect("done")?;
        Ok(Node::While {
            until,
            condition,
            body,
            span: join(start, self.last_span()),
        })
    }

    fn case_command(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        self.pos += 1;
        let Some(PosixTokenKind::Word(subject)) = self.kind(0) else {
            return self.error("expected a word after case");
        };
        let subject = subject.clone();
        self.pos += 1;
        self.skip_newlines();
        if !self.word_is("in") {
            return self.error("expected in");
        }
        self.pos += 1;
        let mut arms = Vec::new();
        loop {
            self.skip_separators();
            if self.word_is("esac") {
                self.pos += 1;
                break;
            }
            if matches!(
                self.kind(0),
                Some(PosixTokenKind::Operator(Operator::OpenParen))
            ) {
                self.pos += 1;
            }
            let mut patterns = Vec::new();
            loop {
                let Some(PosixTokenKind::Word(word)) = self.kind(0) else {
                    return self.error("expected a pattern");
                };
                patterns.push(word.clone());
                self.pos += 1;
                match self.kind(0) {
                    Some(PosixTokenKind::Operator(Operator::Pipe)) => self.pos += 1,
                    Some(PosixTokenKind::Operator(Operator::CloseParen)) => {
                        self.pos += 1;
                        break;
                    }
                    _ => return self.error("expected ) after the pattern"),
                }
            }
            let body = self.block(&[])?;
            if matches!(
                self.kind(0),
                Some(PosixTokenKind::Operator(Operator::CaseEnd))
            ) {
                self.pos += 1;
            }
            arms.push(CaseArm { patterns, body });
        }
        Ok(Node::Case {
            subject,
            arms,
            span: join(start, self.last_span()),
        })
    }

    fn function_keyword(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        self.pos += 1;
        let Some(PosixTokenKind::Word(word)) = self.kind(0) else {
            return self.error("expected a function name");
        };
        let name = word.raw.clone();
        self.pos += 1;
        if matches!(
            self.kind(0),
            Some(PosixTokenKind::Operator(Operator::OpenParen))
        ) {
            self.pos += 2;
        }
        self.function_body(name, start)
    }

    fn function_header(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        let Some(PosixTokenKind::Word(word)) = self.kind(0) else {
            return self.error("expected a function name");
        };
        let name = word.raw.clone();
        self.pos += 3;
        self.function_body(name, start)
    }

    fn function_body(&mut self, name: String, start: Span) -> Result<Node, ParseError> {
        self.skip_newlines();
        if !self.word_is("{") {
            return self.error("expected { after the function header");
        }
        self.pos += 1;
        self.block(&[])?;
        self.expect("}")?;
        Ok(Node::Function {
            name,
            span: join(start, self.last_span()),
        })
    }

    fn group(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        self.pos += 1;
        let body = self.block(&[])?;
        self.expect("}")?;
        Ok(Node::Group {
            body,
            span: join(start, self.last_span()),
        })
    }

    fn simple(&mut self) -> Result<Node, ParseError> {
        let start = self.last_or_current();
        let negated = self.word_is("!");
        if negated {
            self.pos += 1;
        }
        let double_bracket = self.word_is("[[");
        let mut words = Vec::new();
        let mut first_command = true;
        let mut end = start;
        while let Some(token) = self.peek() {
            match &token.kind {
                PosixTokenKind::Word(word) => {
                    if double_bracket && word.raw == "]]" {
                        end = token.span;
                        self.pos += 1;
                        break;
                    }
                    if first_command && !(double_bracket && words.is_empty() && word.raw == "[[") {
                        words.push(word.clone());
                    }
                    if !double_bracket && self.at_unquoted_keyword(&word.raw, words.len()) {
                        break;
                    }
                }
                PosixTokenKind::Redirect(_) => {}
                PosixTokenKind::Operator(Operator::Pipe | Operator::PipeAll) => {
                    first_command = false;
                }
                PosixTokenKind::Operator(
                    operator @ (Operator::And
                    | Operator::Or
                    | Operator::OpenParen
                    | Operator::CloseParen),
                ) if double_bracket => {
                    let text = operator.as_str();
                    words.push(Word {
                        raw: text.to_owned(),
                        parts: vec![WordPart::Literal {
                            text: text.to_owned(),
                            quoting: Quoting::None,
                        }],
                    });
                }
                PosixTokenKind::Operator(_)
                | PosixTokenKind::Newline
                | PosixTokenKind::Comment(_) => {
                    break;
                }
            }
            end = token.span;
            self.pos += 1;
        }
        if words.is_empty() && !double_bracket {
            return self.error("expected a command");
        }
        Ok(Node::Simple(Simple {
            span: join(start, end),
            words,
            negated,
            double_bracket,
        }))
    }

    /// Keywords end a simple command only where a command could start; a
    /// word later in the command is an argument.
    fn at_unquoted_keyword(&self, _word: &str, _count: usize) -> bool {
        false
    }
}

const EMPTY: Span = Span {
    line: 1,
    column: 1,
    end_line: 1,
    end_column: 1,
};

fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::super::lex::lex;
    use super::*;

    fn block(input: &str) -> Block {
        parse(&lex(input).expect("lexes")).expect("parses")
    }

    #[test]
    fn if_with_elif_and_else() {
        let b = block("if [ -f /a ]; then rm /a; elif [ -d /b ]; then rmdir /b; else echo x; fi");
        match &b.entries[0].first {
            Node::If { arms, other, .. } => {
                assert_eq!(arms.len(), 2);
                assert!(other.is_some());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn loops_case_and_functions_parse() {
        let b = block(
            "for f in /a/*.csv; do gzip \"$f\"; done\nwhile [ -f /x ]; do rm /x; done\n\
             case $m in a|b) echo 1;; *) echo 2;; esac\nclean() {\n rm /a\n}\nfunction g { rm /b; }",
        );
        assert_eq!(b.entries.len(), 5);
        assert!(matches!(&b.entries[3].first, Node::Function { name, .. } if name == "clean"));
        assert!(matches!(&b.entries[4].first, Node::Function { name, .. } if name == "g"));
        match &b.entries[2].first {
            Node::Case { arms, .. } => {
                assert_eq!(arms[0].patterns.len(), 2);
                assert_eq!(arms.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_fi_is_an_error() {
        let tokens = lex("if [ -f /a ]; then rm /a").expect("lexes");
        assert!(parse(&tokens).is_err());
    }

    #[test]
    fn plain_commands_are_not_compound() {
        assert!(!has_compound(&lex("rm -rf /a && mkdir /a").expect("lexes")));
        assert!(has_compound(
            &lex("cd /a && for f in x; do rm $f; done").expect("lexes")
        ));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
