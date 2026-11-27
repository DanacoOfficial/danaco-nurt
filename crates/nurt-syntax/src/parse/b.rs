// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage syntax of variant B: call notation `command(arguments)`.
//!
//! Only stages differ from the word notation; the program, `let`,
//! pipelines and expressions share its grammar and produce the same tree.
//!
//! ```text
//! stage    = COMMAND "(" [ arglist ] ")"
//! arglist  = argument { "," argument } [ "," ]
//! argument = WORD ":" value | value
//! ```
//!
//! A stage always has parentheses (`remove()`); a space before `(` is
//! accepted and line breaks inside them are ignored. Arguments end only at
//! `,` or `)`, so `first(20 -1)` subtracts and `filter(size /2)` divides. A
//! named argument is a word, a keyword included, directly followed by `:`.
//! `name(…)` is a stage at the start of a statement, after `let name =` and
//! after `|>`; as a value it is a function call unless it is a sub-pipeline.
//!
//! # Sub-pipeline arguments
//!
//! A value is a sub-pipeline in three forms:
//!
//! 1. the argument holds `|>` outside brackets and starts with a stage or a
//!    reference: `verify(run(["go"]) |> assert(exit_code: 0))`;
//! 2. the argument is exactly one call whose name contains `.` or whose
//!    arguments include a named one: `verify(git.status())`;
//! 3. parentheses enclose exactly one stage, `verify((count()))`, or a
//!    pipeline.
//!
//! Any other call is a function call (`verify(f(x))`), a bare name is an
//! identifier (`verify(count)`) and other parentheses group an expression
//! (`(x -1)`, `(f(x) + 1)`). Recognition looks ahead to the end of the
//! argument or of the parentheses, at a cost linear in their length.
//!
//! The printer mirrors the rule: it writes a sub-pipeline with stages after
//! its head in form 1 and a sub-pipeline of one stage in form 3. A
//! sub-pipeline of a reference alone has no form of its own, since `(@x)`
//! reads back as the reference.
//!
//! # Spans
//!
//! A stage covers its name through the closing parenthesis. A sub-pipeline
//! in form 1 or 2 and its pipeline cover the argument from its first to its
//! last token; in form 3 the sub-pipeline covers the parentheses.

use nurt_contracts::ast::{Arg, Expr, Program, Stage};
use nurt_contracts::error::At;

use super::error::{at_of, edit};
use super::expr::ExprParser;
use super::{Node, Parsed, Parser, StageSyntax, ends_operand, join};
use crate::error::SyntaxError;
use crate::lex::{Glued, LexContext, LexOptions, Lexer, Token, TokenKind};

/// Parses a program written in the call notation.
///
/// # Errors
///
/// Returns a [`SyntaxError`] with the code `E_SYNTAX`, `E_LEXICAL` or
/// `E_ESCAPE` at the first fault of the text.
pub(crate) fn parse_program(p: &mut Parser<'_>) -> Result<Program, SyntaxError> {
    let notation = CallNotation { text: p.text() };
    p.parse_program_with(&notation)
}

/// Stage syntax of the call notation over the program text, which the
/// recognition of sub-pipelines reads ahead.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallNotation<'t> {
    text: &'t str,
}

impl<'t> CallNotation<'t> {
    /// Call notation over the program text.
    pub(crate) fn new(text: &'t str) -> Self {
        Self { text }
    }
}

impl StageSyntax for CallNotation<'_> {
    fn parse_stage(&self, p: &mut Parser<'_>) -> Result<Parsed<Stage>, SyntaxError> {
        let (name, first, last) = p.parse_command_name(LexContext::Operator)?;
        if !p.at(&TokenKind::LParen) {
            return Err(missing_parentheses(p, &name, &first, &last));
        }
        p.bump(LexContext::Value)?;
        let (args, close, height) = self.parse_arguments(p)?;
        let span = join(first.span, close.span);
        let height = height + 1;
        p.check_height(height, span)?;
        Ok(Parsed {
            value: Stage {
                name,
                args,
                span: Some(span),
            },
            span,
            range: first.range.start..close.range.end,
            height,
        })
    }

    /// Parenthesized content is a sub-pipeline when its leading call fills
    /// the parentheses or is followed by `|>`; `(f(x) + 1)` is an expression.
    fn starts_arguments(&self, _last: &Token, next: &Token) -> bool {
        if next.kind != TokenKind::LParen {
            return false;
        }
        let tokens = scan(self.text, next.range.start);
        matching_close(&tokens, 0).is_some_and(|close| {
            tokens
                .get(close + 1)
                .is_none_or(|after| after.kind == TokenKind::PipeGt)
        })
    }

    fn arguments_by_whitespace(&self) -> bool {
        false
    }
}

impl CallNotation<'_> {
    /// Parses the arguments after `(` through the closing `)` and returns
    /// them with the closing parenthesis and their greatest height.
    fn parse_arguments(&self, p: &mut Parser<'_>) -> Result<(Vec<Arg>, Token, usize), SyntaxError> {
        let mut args = Vec::new();
        let mut height = 0;
        loop {
            if p.at(&TokenKind::RParen) {
                let close = p.bump(LexContext::Operator)?;
                return Ok((args, close, height));
            }
            let (arg, value) = self.parse_argument(p)?;
            height = height.max(value.height);
            let bare_name = arg.name.is_none() && matches!(arg.value, Expr::Ident { .. });
            args.push(arg);
            match p.kind() {
                TokenKind::Comma => {
                    p.bump(LexContext::Value)?;
                }
                TokenKind::RParen => {}
                _ => return Err(argument_end_error(p, value.range.end, bare_name)),
            }
        }
    }

    /// Parses `WORD ":" value` or `value` and returns the argument with the
    /// extent of its value.
    fn parse_argument(&self, p: &mut Parser<'_>) -> Result<(Arg, Parsed<()>), SyntaxError> {
        let first = p.current().clone();
        let named = is_word(&first.kind) && first.glued == Glued::Colon;
        if named {
            p.bump(LexContext::Value)?;
            p.bump(LexContext::Value)?;
        }
        let Parsed {
            value,
            span,
            range,
            height,
        } = self.parse_value(p)?;
        let arg = Arg {
            name: named.then(|| p.slice(&first).to_owned()),
            value,
            span: Some(if named { join(first.span, span) } else { span }),
        };
        let extent = Parsed {
            value: (),
            span,
            range,
            height,
        };
        Ok((arg, extent))
    }

    /// Parses an argument value: a sub-pipeline in form 1 or 2, or an
    /// expression, which covers form 3.
    fn parse_value(&self, p: &mut Parser<'_>) -> Result<Node, SyntaxError> {
        let tokens = scan(self.text, p.current().range.start);
        if !Shape::of(&tokens).is_sub_pipeline() {
            return ExprParser::new(self).parse_expression(p);
        }
        p.enter()?;
        let pipeline = p.parse_pipeline(self)?;
        p.leave();
        let height = pipeline.height + 1;
        p.check_height(height, pipeline.span)?;
        Ok(Parsed {
            value: Expr::Sub {
                p: pipeline.value,
                span: Some(pipeline.span),
            },
            span: pipeline.span,
            range: pipeline.range,
            height,
        })
    }
}

/// Structure of the tokens of one argument.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Shape {
    /// `|>` stands outside brackets.
    pipe: bool,
    /// The argument starts with `@`.
    reference: bool,
    /// The argument starts with `COMMAND (`.
    call: bool,
    /// The command name contains `.`.
    group: bool,
    /// The parentheses of the call hold a named argument.
    named: bool,
    /// The closing parenthesis of the call ends the argument.
    whole: bool,
}

impl Shape {
    fn of(tokens: &[Token]) -> Self {
        let mut shape = Self {
            pipe: has_top_level_pipe(tokens),
            reference: tokens
                .first()
                .is_some_and(|token| token.kind == TokenKind::At),
            ..Self::default()
        };
        if let Some((open, group)) = call_start(tokens) {
            shape.call = true;
            shape.group = group;
            let close = matching_close(tokens, open);
            shape.whole = close.is_some_and(|close| close + 1 == tokens.len());
            let inner = tokens
                .get(open + 1..close.unwrap_or(tokens.len()))
                .unwrap_or(&[]);
            shape.named = has_top_level_name(inner);
        }
        shape
    }

    /// Whether the argument is a sub-pipeline in form 1 or 2.
    fn is_sub_pipeline(self) -> bool {
        (self.pipe && (self.call || self.reference))
            || (self.call && self.whole && (self.group || self.named))
    }
}

/// Tokens of `text` from byte `start` to the end of the argument or of the
/// parenthesized content that starts there: the first `,` or unmatched
/// closing bracket outside brackets, the end of the input or the first
/// lexical error, none of them included. Line breaks are skipped, as they
/// are inside the parentheses of an argument list.
fn scan(text: &str, start: usize) -> Vec<Token> {
    let rest = text.get(start..).unwrap_or_default();
    let mut lexer = Lexer::new(rest, CALL_NOTATION_LEXING);
    let mut tokens = Vec::new();
    let mut depth = 0_usize;
    let mut context = LexContext::Value;
    while let Ok(token) = lexer.next(context) {
        match token.kind {
            TokenKind::Eof => break,
            TokenKind::Nl => continue,
            TokenKind::Comma if depth == 0 => break,
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                match depth.checked_sub(1) {
                    Some(outer) => depth = outer,
                    None => break,
                }
            }
            _ => {}
        }
        context = if ends_operand(&token.kind) {
            LexContext::Operator
        } else {
            LexContext::Value
        };
        tokens.push(token);
    }
    tokens
}

/// Strict lexing of the call notation without comments.
const CALL_NOTATION_LEXING: LexOptions = LexOptions {
    arguments_by_whitespace: false,
    tolerant: false,
    keep_comments: false,
};

/// Index of the bracket that closes the bracket at `open`.
fn matching_close(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
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

fn has_top_level_pipe(tokens: &[Token]) -> bool {
    top_level(tokens).any(|token| token.kind == TokenKind::PipeGt)
}

/// Whether an argument list holds a named argument outside nested brackets.
fn has_top_level_name(tokens: &[Token]) -> bool {
    top_level(tokens).any(|token| is_word(&token.kind) && token.glued == Glued::Colon)
}

/// Tokens outside brackets; brackets themselves are skipped.
fn top_level(tokens: &[Token]) -> impl Iterator<Item = &Token> {
    let mut depth = 0_usize;
    tokens.iter().filter(move |token| match token.kind {
        TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
            depth += 1;
            false
        }
        TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
            depth = depth.saturating_sub(1);
            false
        }
        _ => depth == 0,
    })
}

/// Index of `(` after a command name at the start of `tokens`, and whether
/// the name contains `.`.
fn call_start(tokens: &[Token]) -> Option<(usize, bool)> {
    let mut index = 0;
    loop {
        if tokens.get(index)?.kind != TokenKind::Ident {
            return None;
        }
        let next = tokens.get(index + 1)?;
        match next.kind {
            TokenKind::LParen => return Some((index + 1, index > 0)),
            TokenKind::Dot if !next.space_before => index += 2,
            _ => return None,
        }
    }
}

/// Whether a token is a word that may name a parameter: an identifier or a
/// keyword.
fn is_word(kind: &TokenKind) -> bool {
    matches!(
        kind,
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

/// Whether a token can start the value of an argument.
fn starts_value(kind: &TokenKind) -> bool {
    matches!(
        kind,
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
            | TokenKind::Not
            | TokenKind::At
            | TokenKind::LBracket
            | TokenKind::LBrace
    )
}

/// Error for a stage name that is not followed by `(`. The fix appends `()`
/// when the stage ends there.
fn missing_parentheses(p: &Parser<'_>, name: &str, first: &Token, last: &Token) -> SyntaxError {
    let ends_stage = matches!(
        p.kind(),
        TokenKind::Nl | TokenKind::Eof | TokenKind::PipeGt | TokenKind::RParen | TokenKind::Comma
    );
    let (message, fix) = if ends_stage {
        (
            format!("command `{name}` has no arguments: write `{name}()`"),
            Some(edit(p.text(), &(last.range.end..last.range.end), "()")),
        )
    } else {
        (
            format!("arguments of `{name}` are written in parentheses: `{name}(…)`"),
            None,
        )
    };
    let length = u32::try_from(name.chars().count()).unwrap_or(u32::MAX);
    error(
        At::new(first.span.line, first.span.column, length),
        name,
        &message,
        "missing_parentheses",
        &["`(`"],
        fix,
    )
}

/// Error for the current token, which neither separates nor closes the
/// arguments; `end` is the byte offset after the last argument. `bare_name`
/// tells whether the last argument is a bare identifier, which a following
/// `=` or a detached `:` marks as a parameter name.
fn argument_end_error(p: &Parser<'_>, end: usize, bare_name: bool) -> SyntaxError {
    let token = p.current();
    let text = p.text();
    let (message, problem, fix) = match token.kind {
        TokenKind::PipeGt => (
            "missing `)` before `|>`; close the arguments of the stage",
            "unclosed_arguments",
            Some(edit(text, &(end..end), ")")),
        ),
        TokenKind::Eof => (
            "missing `)` at the end of the arguments",
            "unclosed_arguments",
            Some(edit(text, &(end..end), ")")),
        ),
        TokenKind::Eq if bare_name => (
            "a named argument is written `name: value`; replace `=` with `:`",
            "equals_in_named_argument",
            Some(edit(
                text,
                &(end..after_spaces(text, token.range.end)),
                ": ",
            )),
        ),
        TokenKind::Colon if bare_name => (
            "write `:` directly after the parameter name",
            "space_before_colon",
            Some(edit(text, &(end..token.range.start), "")),
        ),
        _ if starts_value(&token.kind) => (
            "missing `,` between arguments",
            "missing_comma",
            Some(edit(
                text,
                &(end..end),
                if token.space_before { "," } else { ", " },
            )),
        ),
        _ => (
            "expected `,` or `)` after the argument",
            "unexpected_token",
            None,
        ),
    };
    let found = match token.kind {
        TokenKind::Eof => "end of input",
        _ => p.slice(token),
    };
    error(
        at_of(p, token),
        found,
        message,
        problem,
        &["`,`", "`)`"],
        fix,
    )
}

/// Syntax error with the position, the fragment found, the expected tokens
/// and an optional ready fix.
fn error(
    at: At,
    found: &str,
    message: &str,
    problem: &str,
    expected: &[&str],
    fix: Option<String>,
) -> SyntaxError {
    let error = SyntaxError::syntax(message)
        .with_at(at)
        .with_found(found)
        .with_problem(problem)
        .with_expected(expected.iter().copied());
    match fix {
        Some(fix) => error.with_fix(fix),
        None => error,
    }
}

/// Byte offset of the first character at or after `from` that is neither a
/// space nor a tab.
fn after_spaces(text: &str, from: usize) -> usize {
    text.get(from..).map_or(from, |rest| {
        from + rest.len() - rest.trim_start_matches([' ', '\t']).len()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(argument: &str) -> Shape {
        Shape::of(&scan(argument, 0))
    }

    fn kinds(text: &str, start: usize) -> Vec<TokenKind> {
        scan(text, start)
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn scan_stops_at_comma_or_unmatched_bracket_outside_brackets() {
        assert_eq!(
            kinds("f(a, b), c", 0),
            [
                TokenKind::Ident,
                TokenKind::LParen,
                TokenKind::Ident,
                TokenKind::Comma,
                TokenKind::Ident,
                TokenKind::RParen
            ]
        );
        assert_eq!(kinds("x) |> y", 0), [TokenKind::Ident]);
        assert_eq!(kinds("[1, 2]]", 0).len(), 5);
    }

    #[test]
    fn scan_skips_line_breaks_and_comments() {
        assert_eq!(
            kinds("a # note\n\n|> b()", 0),
            [
                TokenKind::Ident,
                TokenKind::PipeGt,
                TokenKind::Ident,
                TokenKind::LParen,
                TokenKind::RParen
            ]
        );
    }

    #[test]
    fn scan_reads_slash_after_operand_as_division() {
        assert_eq!(
            kinds("size /2/ 3", 0),
            [
                TokenKind::Ident,
                TokenKind::Slash,
                TokenKind::Int,
                TokenKind::Slash,
                TokenKind::Int
            ]
        );
        assert_eq!(kinds("/re/i", 0), [TokenKind::Regex]);
    }

    #[test]
    fn scan_stops_at_lexical_error() {
        assert_eq!(kinds("a \"\\q\" b", 0), [TokenKind::Ident]);
        assert!(kinds("", 0).is_empty());
        assert!(kinds("abc", 10).is_empty());
    }

    #[test]
    fn argument_with_pipe_after_stage_or_reference_is_sub_pipeline() {
        assert!(shape("run([\"go\"]) |> count()").is_sub_pipeline());
        assert!(shape("@x |> count()").is_sub_pipeline());
        assert!(shape("git.status() |> count()").is_sub_pipeline());
        assert!(!shape("x |> count()").is_sub_pipeline());
        assert!(!shape("[run() |> count()]").is_sub_pipeline());
    }

    #[test]
    fn whole_call_with_group_or_named_argument_is_sub_pipeline() {
        assert!(shape("git.status()").is_sub_pipeline());
        assert!(shape("a.b.c(1)").is_sub_pipeline());
        assert!(shape("run([\"go\"], cwd: \"/a\")").is_sub_pipeline());
        assert!(shape("run(in: 1)").is_sub_pipeline());
        assert!(!shape("f(x)").is_sub_pipeline());
        assert!(!shape("f()").is_sub_pipeline());
        assert!(!shape("f(g(a: 1))").is_sub_pipeline());
        assert!(!shape("f({a: 1})").is_sub_pipeline());
        assert!(!shape("git.status() + 1").is_sub_pipeline());
        assert!(!shape("git . status()").is_sub_pipeline());
        assert!(!shape("count").is_sub_pipeline());
    }

    #[test]
    fn unclosed_call_is_not_whole() {
        let unclosed = shape("git.status(1");
        assert!(unclosed.call && unclosed.group && !unclosed.whole);
        assert!(!unclosed.is_sub_pipeline());
    }

    #[test]
    fn matching_close_skips_nested_brackets() {
        let tokens = scan("(a, [b], {c: (d)}) e", 0);
        assert_eq!(matching_close(&tokens, 0), Some(14));
        assert_eq!(matching_close(&scan("(a", 0), 0), None);
    }

    #[test]
    fn top_level_name_ignores_nested_maps() {
        assert!(shape("f(a, in: 1)").named);
        assert!(!shape("f({in: 1})").named);
        assert!(!shape("f(a : 1)").named);
        assert!(!has_top_level_name(&scan("{in: 1}", 0)));
    }

    #[test]
    fn spaces_after_offset_are_skipped() {
        assert_eq!(after_spaces("a = \t b", 3), 6);
        assert_eq!(after_spaces("a=", 2), 2);
        assert_eq!(after_spaces("a", 5), 5);
    }
}
