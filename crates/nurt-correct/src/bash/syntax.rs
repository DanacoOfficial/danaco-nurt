// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Structure of shell commands: lists, and-or chains, pipelines and simple
//! commands with redirections.
//!
//! Compound commands (`for`, `while`, `until`, `if`, `case`, functions,
//! subshells and groups) are recognised only to be reported; their bodies
//! are not parsed.

use nurt_contracts::ast::Span;

use super::lex::{Operator, PosixToken, PosixTokenKind, Redirect, Word};

/// Separator after an item of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Separator {
    Semicolon,
    Background,
    Newline,
}

/// Connector between pipelines of an and-or chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Connector {
    And,
    Or,
}

#[derive(Debug, Clone)]
pub(crate) struct Script {
    pub(crate) items: Vec<Item>,
}

/// An and-or chain with the separator that ends it.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub(crate) first: Pipe,
    pub(crate) rest: Vec<(Connector, Pipe)>,
    pub(crate) separator: Option<Separator>,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct Pipe {
    pub(crate) negated: bool,
    pub(crate) commands: Vec<Command>,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) enum Command {
    Simple(Simple),
    Compound { keyword: String, span: Span },
}

#[derive(Debug, Clone)]
pub(crate) struct Simple {
    pub(crate) assignments: Vec<Assignment>,
    pub(crate) words: Vec<Located<Word>>,
    pub(crate) redirects: Vec<Redirection>,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct Located<T> {
    pub(crate) value: T,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct Assignment {
    pub(crate) name: String,
    pub(crate) value: Word,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct Redirection {
    pub(crate) redirect: Redirect,
    pub(crate) target: Option<Located<Word>>,
    pub(crate) span: Span,
}

/// Error of the command structure, with the position of the offending token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub(crate) message: &'static str,
    pub(crate) span: Span,
}

const COMPOUND_KEYWORDS: &[&str] = &[
    "for", "while", "until", "if", "case", "select", "function", "{", "[[",
];

pub(crate) fn parse(tokens: &[PosixToken]) -> Result<Script, SyntaxError> {
    Parser { tokens, pos: 0 }.script()
}

struct Parser<'t> {
    tokens: &'t [PosixToken],
    pos: usize,
}

impl Parser<'_> {
    fn script(mut self) -> Result<Script, SyntaxError> {
        let mut items = Vec::new();
        loop {
            self.skip_blank();
            if self.pos >= self.tokens.len() {
                break;
            }
            items.push(self.item()?);
        }
        Ok(Script { items })
    }

    fn skip_blank(&mut self) {
        while let Some(token) = self.tokens.get(self.pos) {
            if matches!(token.kind, PosixTokenKind::Newline | PosixTokenKind::Comment(_)) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<&PosixToken> {
        self.tokens.get(self.pos)
    }

    fn item(&mut self) -> Result<Item, SyntaxError> {
        let first = self.pipe()?;
        let mut span = first.span;
        let mut rest = Vec::new();
        loop {
            let connector = match self.peek().map(|token| &token.kind) {
                Some(PosixTokenKind::Operator(Operator::And)) => Connector::And,
                Some(PosixTokenKind::Operator(Operator::Or)) => Connector::Or,
                _ => break,
            };
            self.pos += 1;
            self.skip_blank();
            let pipe = self.pipe()?;
            span = join(span, pipe.span);
            rest.push((connector, pipe));
        }
        let separator = self.separator()?;
        Ok(Item {
            first,
            rest,
            separator,
            span,
        })
    }

    fn separator(&mut self) -> Result<Option<Separator>, SyntaxError> {
        if matches!(
            self.peek().map(|token| &token.kind),
            Some(PosixTokenKind::Comment(_))
        ) {
            self.pos += 1;
        }
        let Some(token) = self.peek() else {
            return Ok(None);
        };
        let separator = match &token.kind {
            PosixTokenKind::Operator(Operator::Semicolon) => Separator::Semicolon,
            PosixTokenKind::Operator(Operator::Background) => Separator::Background,
            PosixTokenKind::Newline => Separator::Newline,
            _ => return Err(unexpected(token)),
        };
        self.pos += 1;
        Ok(Some(separator))
    }

    fn pipe(&mut self) -> Result<Pipe, SyntaxError> {
        let negated = self.bang();
        let first = self.command()?;
        let mut span = command_span(&first);
        let mut commands = vec![first];
        while matches!(
            self.peek().map(|token| &token.kind),
            Some(PosixTokenKind::Operator(Operator::Pipe | Operator::PipeAll))
        ) {
            self.pos += 1;
            self.skip_blank();
            let command = self.command()?;
            span = join(span, command_span(&command));
            commands.push(command);
        }
        Ok(Pipe {
            negated,
            commands,
            span,
        })
    }

    fn bang(&mut self) -> bool {
        let is_bang = matches!(
            self.peek().map(|token| &token.kind),
            Some(PosixTokenKind::Word(word)) if word.raw == "!"
        );
        if is_bang {
            self.pos += 1;
        }
        is_bang
    }

    fn command(&mut self) -> Result<Command, SyntaxError> {
        let Some(token) = self.peek() else {
            let span = self.tokens.last().map_or(EMPTY_SPAN, |token| token.span);
            return Err(SyntaxError {
                message: "expected a command",
                span,
            });
        };
        match &token.kind {
            PosixTokenKind::Operator(Operator::OpenParen) => Ok(self.compound("(")),
            PosixTokenKind::Word(word) if is_compound_start(&word.raw, self.next_kind(1)) => {
                let keyword = word.raw.clone();
                Ok(self.compound(&keyword))
            }
            PosixTokenKind::Word(_) | PosixTokenKind::Redirect(_) => self.simple(),
            _ => Err(unexpected(token)),
        }
    }

    fn next_kind(&self, offset: usize) -> Option<&PosixTokenKind> {
        self.tokens.get(self.pos + offset).map(|token| &token.kind)
    }

    /// Consumes the rest of the input as one compound command.
    fn compound(&mut self, keyword: &str) -> Command {
        let start = self.tokens[self.pos].span;
        let end = self.tokens.last().map_or(start, |token| token.span);
        self.pos = self.tokens.len();
        Command::Compound {
            keyword: keyword.to_owned(),
            span: join(start, end),
        }
    }

    fn simple(&mut self) -> Result<Command, SyntaxError> {
        let start = self.tokens[self.pos].span;
        let mut span = start;
        let mut assignments = Vec::new();
        let mut words: Vec<Located<Word>> = Vec::new();
        let mut redirects = Vec::new();
        while let Some(token) = self.peek() {
            match &token.kind {
                PosixTokenKind::Word(word) => {
                    let located = Located {
                        value: word.clone(),
                        span: token.span,
                    };
                    span = join(span, token.span);
                    self.pos += 1;
                    match assignment(&located) {
                        Some(found) if words.is_empty() => assignments.push(found),
                        _ => words.push(located),
                    }
                }
                PosixTokenKind::Redirect(redirect) => {
                    let redirection = self.redirection(redirect.clone(), token.span)?;
                    span = join(span, redirection.span);
                    redirects.push(redirection);
                }
                _ => break,
            }
        }
        Ok(Command::Simple(Simple {
            assignments,
            words,
            redirects,
            span,
        }))
    }

    fn redirection(&mut self, redirect: Redirect, span: Span) -> Result<Redirection, SyntaxError> {
        self.pos += 1;
        if redirect.here_doc.is_some() {
            return Ok(Redirection {
                redirect,
                target: None,
                span,
            });
        }
        match self.peek() {
            Some(PosixToken {
                kind: PosixTokenKind::Word(word),
                span: target_span,
            }) => {
                let target = Located {
                    value: word.clone(),
                    span: *target_span,
                };
                let span = join(span, *target_span);
                self.pos += 1;
                Ok(Redirection {
                    redirect,
                    target: Some(target),
                    span,
                })
            }
            _ => Err(SyntaxError {
                message: "redirection without a target",
                span,
            }),
        }
    }
}

const EMPTY_SPAN: Span = Span {
    line: 1,
    column: 1,
    end_line: 1,
    end_column: 1,
};

fn is_compound_start(word: &str, next: Option<&PosixTokenKind>) -> bool {
    COMPOUND_KEYWORDS.contains(&word)
        || matches!(next, Some(PosixTokenKind::Operator(Operator::OpenParen)))
}

fn assignment(word: &Located<Word>) -> Option<Assignment> {
    let raw = &word.value.raw;
    let (name, _) = raw.split_once('=')?;
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
    if !valid {
        return None;
    }
    let value = super::lex::lex(&raw[name.len() + 1..])
        .ok()
        .and_then(|tokens| match tokens.into_iter().next() {
            Some(PosixToken {
                kind: PosixTokenKind::Word(word),
                ..
            }) => Some(word),
            None => Some(Word {
                raw: String::new(),
                parts: Vec::new(),
            }),
            Some(_) => None,
        })?;
    Some(Assignment {
        name: name.to_owned(),
        value,
        span: word.span,
    })
}

fn unexpected(token: &PosixToken) -> SyntaxError {
    SyntaxError {
        message: "unexpected token",
        span: token.span,
    }
}

pub(crate) fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(simple) => simple.span,
        Command::Compound { span, .. } => *span,
    }
}

/// Smallest span covering both spans.
pub(crate) fn join(a: Span, b: Span) -> Span {
    let (start_line, start_column) = (a.line, a.column).min((b.line, b.column));
    let (end_line, end_column) = (a.end_line, a.end_column).max((b.end_line, b.end_column));
    Span {
        line: start_line,
        column: start_column,
        end_line,
        end_column,
    }
}

#[cfg(test)]
mod tests {
    use super::super::lex::lex;
    use super::*;

    fn script(input: &str) -> Script {
        parse(&lex(input).unwrap()).unwrap()
    }

    fn simple(command: &Command) -> &Simple {
        match command {
            Command::Simple(simple) => simple,
            Command::Compound { .. } => panic!("compound"),
        }
    }

    #[test]
    fn and_or_chain_with_pipes() {
        let parsed = script("cd /a && ls -l | grep x || echo no; rm f &");
        assert_eq!(parsed.items.len(), 2);
        let first = &parsed.items[0];
        assert_eq!(first.rest.len(), 2);
        assert_eq!(first.rest[0].0, Connector::And);
        assert_eq!(first.rest[0].1.commands.len(), 2);
        assert_eq!(first.rest[1].0, Connector::Or);
        assert_eq!(first.separator, Some(Separator::Semicolon));
        assert_eq!(parsed.items[1].separator, Some(Separator::Background));
    }

    #[test]
    fn assignments_precede_words() {
        let parsed = script("FOO=1 BAR='a b' make X=2");
        let command = simple(&parsed.items[0].first.commands[0]);
        assert_eq!(command.assignments.len(), 2);
        assert_eq!(command.assignments[1].value.literal().as_deref(), Some("a b"));
        assert_eq!(command.words.len(), 2);
        assert_eq!(command.words[1].value.raw, "X=2");
    }

    #[test]
    fn redirections_take_targets() {
        let parsed = script("echo x > /a/f 2>&1");
        let command = simple(&parsed.items[0].first.commands[0]);
        assert_eq!(command.redirects.len(), 2);
        assert_eq!(
            command.redirects[0].target.as_ref().unwrap().value.raw,
            "/a/f"
        );
        assert_eq!(command.redirects[1].redirect.fd, Some(2));
        assert_eq!(command.words.len(), 2);
    }

    #[test]
    fn compound_commands_are_reported() {
        let parsed = script("for f in *.txt; do rm $f; done");
        assert!(matches!(
            &parsed.items[0].first.commands[0],
            Command::Compound { keyword, .. } if keyword == "for"
        ));
        let function = script("build() { make; }");
        assert!(matches!(&function.items[0].first.commands[0], Command::Compound { .. }));
        let subshell = script("(cd /a; ls)");
        assert!(matches!(&subshell.items[0].first.commands[0], Command::Compound { keyword, .. } if keyword == "("));
    }

    #[test]
    fn negated_pipeline() {
        let parsed = script("! grep x /a");
        assert!(parsed.items[0].first.negated);
    }

    #[test]
    fn missing_redirection_target_is_an_error() {
        let error = parse(&lex("echo x >").unwrap()).unwrap_err();
        assert_eq!(error.message, "redirection without a target");
    }

    #[test]
    fn dangling_operator_is_an_error() {
        assert!(parse(&lex("ls &&").unwrap()).is_err());
        assert!(parse(&lex("ls ) x").unwrap()).is_err());
        assert!(parse(&lex("| ls").unwrap()).is_err());
    }

    #[test]
    fn comments_end_items() {
        let parsed = script("ls /a # note\nls /b");
        assert_eq!(parsed.items.len(), 2);
    }

    #[test]
    fn join_covers_both_spans() {
        let a = Span { line: 1, column: 5, end_line: 1, end_column: 8 };
        let b = Span { line: 1, column: 1, end_line: 1, end_column: 3 };
        assert_eq!(join(a, b), Span { line: 1, column: 1, end_line: 1, end_column: 8 });
    }
}
