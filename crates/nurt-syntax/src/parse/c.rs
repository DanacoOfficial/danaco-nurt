// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage syntax of variant C: `name=value` arguments only.
//!
//! ```text
//! stage      = COMMAND { assignment } ;
//! COMMAND    = IDENT { "." IDENT } ;
//! assignment = WORD "=" expression ;
//! ```
//!
//! `WORD` is an identifier or a keyword, so `in="…"` names the parameter
//! `in`. Spaces around `=` are accepted and a line break after `=` is
//! ignored. After a value, a keyword is an operator of that value unless `=`
//! follows it directly: `with="b" in=["/a"]` has two arguments. The value is
//! an expression of the shared expression grammar and ends before the first
//! token that cannot continue it: the next assignment, `|>`, a line break,
//! `)`, the end of the input, a value after a complete operand, or `-` and
//! `+` preceded but not followed by whitespace.
//!
//! A value without a parameter name is an error. The parser never supplies
//! the name, because only the command catalog knows the positional
//! parameters. `name: value` is an error whose correction replaces `:` with
//! `=`.
//!
//! # Sub-pipelines
//!
//! Content in parentheses is a sub-pipeline when it has `|>` after its first
//! expression, or when it starts with a command name followed by a token
//! that starts an argument: an assignment, or a value that variant A would
//! read as a positional argument and variant C rejects. Any other content is
//! a parenthesized expression: `(remove)` is the identifier `remove` and
//! `(size > 1mb)` is a comparison. Stages of a sub-pipeline are written in
//! variant C, so a valid program has an assignment after the command name.

use std::ops::Range;

use nurt_contracts::ast::{Arg, Program, Span, Stage};
use nurt_contracts::error::At;

use super::a::starts_argument;
use super::expr::ExprParser;
use super::{Parsed, Parser, StageSyntax, error, join};
use crate::error::SyntaxError;
use crate::lex::{Glued, LexContext, Token, TokenKind};

/// Message of the error for a value without a parameter name.
const POSITIONAL_MESSAGE: &str = "variant C accepts only name=value arguments; \
     write the parameter name and `=` before the value";

/// Message of the error for `name: value`.
const COLON_MESSAGE: &str = "a named argument is written `name=value` in variant C; \
     replace `:` with `=`";

/// Stage notation of variant C.
pub(crate) struct SyntaxC;

impl StageSyntax for SyntaxC {
    fn parse_stage(&self, p: &mut Parser<'_>) -> Result<Parsed<Stage>, SyntaxError> {
        let (name, first, last) = p.parse_command_name(LexContext::Value)?;
        let expressions = ExprParser::new(self);
        let mut span = join(first.span, last.span);
        let mut end = last.range.end;
        let mut height = 0;
        let mut args = Vec::new();
        while let Some(arg) = parse_argument(p, &expressions)? {
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
        last.glued != Glued::LParen && (names_parameter(next) || starts_argument(next))
    }

    fn arguments_by_whitespace(&self) -> bool {
        true
    }
}

/// Parses a program of variant C.
///
/// # Errors
///
/// Returns the first lexical or grammar error of the text.
pub(crate) fn parse_program(p: &mut Parser<'_>) -> Result<Program, SyntaxError> {
    p.parse_program_with(&SyntaxC)
}

/// Whether the token is a word: an identifier or a keyword.
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

/// Whether the token is a word directly followed by `=`.
fn names_parameter(token: &Token) -> bool {
    is_word(&token.kind) && token.glued == Glued::Equals
}

/// How the current token continues a stage.
enum Next {
    /// An assignment starts.
    Assignment,
    /// A `name: value` argument; the error is ready.
    Colon(SyntaxError),
    /// A value without a parameter name starts.
    Positional,
    /// The stage ends.
    End,
}

/// Parses the next argument of a stage, or returns `None` when the stage
/// ends at the current token.
fn parse_argument(
    p: &mut Parser<'_>,
    expressions: &ExprParser<'_>,
) -> Result<Option<Parsed<Arg>>, SyntaxError> {
    match classify(p)? {
        Next::Assignment => parse_assignment(p, expressions).map(Some),
        Next::Colon(error) => Err(error),
        Next::Positional => Err(positional(p, expressions)),
        Next::End => Ok(None),
    }
}

/// Decides how the current token continues a stage, reading no more than
/// one token ahead; the parser state is unchanged afterwards.
fn classify(p: &mut Parser<'_>) -> Result<Next, SyntaxError> {
    let token = p.current().clone();
    if is_word(&token.kind) {
        match token.glued {
            Glued::Equals => return Ok(Next::Assignment),
            Glued::Colon => return Ok(Next::Colon(colon(p, &token)?)),
            Glued::LParen => {}
            Glued::None => {
                if let Some(next) = spaced_separator(p, &token)? {
                    return Ok(next);
                }
            }
        }
    }
    Ok(if starts_argument(&token) {
        Next::Positional
    } else {
        Next::End
    })
}

/// Looks past a word followed by whitespace for `=` or `:`; the parser
/// state is unchanged afterwards.
fn spaced_separator(p: &mut Parser<'_>, word: &Token) -> Result<Option<Next>, SyntaxError> {
    let checkpoint = p.checkpoint();
    p.bump(LexContext::Operator)?;
    let next = match p.kind() {
        TokenKind::Eq => Some(Next::Assignment),
        TokenKind::Colon => Some(Next::Colon(colon_error(p, word)?)),
        _ => None,
    };
    p.rewind(checkpoint);
    Ok(next)
}

/// Parses `WORD "=" expression`; the current token is the word.
fn parse_assignment(
    p: &mut Parser<'_>,
    expressions: &ExprParser<'_>,
) -> Result<Parsed<Arg>, SyntaxError> {
    let name = p.bump(LexContext::Operator)?;
    let equals = p.bump(LexContext::Value)?;
    if !starts_value(p.current()) {
        return Err(error::missing_value(p, &name, &equals));
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

/// Whether the token starts the value of an assignment. A word directly
/// followed by `=` starts the next assignment, not a value.
fn starts_value(token: &Token) -> bool {
    !names_parameter(token)
        && (starts_argument(token) || matches!(token.kind, TokenKind::Minus | TokenKind::Plus))
}

/// Error for a value without a parameter name; the current token starts the
/// value and stays current, so that a repair policy sees the failure there.
fn positional(p: &mut Parser<'_>, expressions: &ExprParser<'_>) -> SyntaxError {
    let token = p.current().clone();
    let checkpoint = p.checkpoint();
    let extent = match expressions.parse_expression(p) {
        Ok(value) => (value.span, value.range),
        Err(_) => (token.span, token.range.clone()),
    };
    p.rewind(checkpoint);
    let (span, range) = extent;
    let written = p.text().get(range).unwrap_or_default();
    SyntaxError::syntax(POSITIONAL_MESSAGE)
        .with_at(at_fragment(span, written))
        .with_found(written)
        .with_problem("positional_argument")
        .with_expected(["name=value", "`|>`", "end of line"])
}

/// Error for `name:` written directly before its value; the current token is
/// the name.
fn colon(p: &mut Parser<'_>, name: &Token) -> Result<SyntaxError, SyntaxError> {
    let checkpoint = p.checkpoint();
    p.bump(LexContext::Operator)?;
    let error = colon_error(p, name);
    p.rewind(checkpoint);
    error
}

/// Builds the error for a colon after a parameter name; the current token is
/// the colon. The correction replaces everything from the end of the name to
/// the value on the same line with `=`.
fn colon_error(p: &mut Parser<'_>, name: &Token) -> Result<SyntaxError, SyntaxError> {
    let colon = p.bump(LexContext::Value)?;
    let value_start = match p.kind() {
        TokenKind::Nl | TokenKind::Eof => colon.range.end,
        _ => p.current().range.start,
    };
    let replaced: Range<usize> = name.range.end..value_start;
    let written = p
        .text()
        .get(name.range.start..colon.range.end)
        .unwrap_or_default();
    let span = join(name.span, colon.span);
    Ok(SyntaxError::syntax(COLON_MESSAGE)
        .with_at(at_fragment(span, written))
        .with_found(written)
        .with_problem("colon_in_named_argument")
        .with_expected(["`=`"])
        .with_fix(error::edit(p.text(), &replaced, "=")))
}

/// Position of a written fragment that starts at the span.
fn at_fragment(span: Span, written: &str) -> At {
    At::new(
        span.line,
        span.column,
        u32::try_from(written.chars().count()).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::ast::{Expr, Head, Pipeline, Statement};
    use nurt_contracts::error::ErrorCode;

    use super::*;
    use crate::parse::{ParseOptions, Variant};

    fn parse_c(text: &str) -> Result<Program, SyntaxError> {
        let options = ParseOptions::default();
        let mut parser = Parser::new(text, Variant::C, &options, false)?;
        parse_program(&mut parser)
    }

    fn program(text: &str) -> Program {
        parse_c(text).unwrap_or_else(|error| panic!("{text:?}: {error:?}"))
    }

    fn failure(text: &str) -> SyntaxError {
        match parse_c(text) {
            Ok(program) => panic!("{text:?} parses as {program:?}"),
            Err(error) => error,
        }
    }

    fn first_pipeline(program: &Program) -> &Pipeline {
        match &program.statements[0] {
            Statement::Pipeline(pipeline) => pipeline,
            Statement::Let(binding) => &binding.value,
            other => panic!("unexpected statement {other:?}"),
        }
    }

    fn head(text: &str) -> Stage {
        let program = program(text);
        match &first_pipeline(&program).head {
            Head::Stage(stage) => stage.clone(),
            other => panic!("head is not a stage: {other:?}"),
        }
    }

    fn names(stage: &Stage) -> Vec<&str> {
        stage
            .args
            .iter()
            .map(|arg| arg.name.as_deref().unwrap_or("<positional>"))
            .collect()
    }

    fn position(error: &SyntaxError) -> (u32, u32) {
        let at = error.at.expect("error has a position");
        (at.line, at.column)
    }

    #[test]
    fn stage_without_arguments_is_its_command_name() {
        let stage = head("remove");
        assert_eq!(stage.name, "remove");
        assert!(stage.args.is_empty());
    }

    #[test]
    fn assignment_names_its_argument() {
        let stage = head(r#"code.symbols name="oldName" kind=function"#);
        assert_eq!(stage.name, "code.symbols");
        assert_eq!(names(&stage), ["name", "kind"]);
    }

    #[test]
    fn keyword_before_equals_names_a_parameter() {
        let stage = head(r#"replace pattern="a" with="b" in=["/a"]"#);
        assert_eq!(names(&stage), ["pattern", "with", "in"]);
        assert!(matches!(&stage.args[1].value, Expr::Text { v, .. } if v == "b"));
    }

    #[test]
    fn spaces_around_equals_are_accepted() {
        let stage = head(r#"find path = "/a" glob ="*" depth= 1"#);
        assert_eq!(names(&stage), ["path", "glob", "depth"]);
    }

    #[test]
    fn keyword_with_spaced_equals_after_the_command_names_a_parameter() {
        let stage = head(r#"replace in = ["/a"] with="b""#);
        assert_eq!(names(&stage), ["in", "with"]);
    }

    #[test]
    fn keyword_with_spaced_equals_after_a_value_is_an_operator() {
        let error = failure(r#"replace with="b" in = ["/a"]"#);
        assert_eq!(error.code, ErrorCode::Syntax);
        assert_eq!(position(&error), (1, 21));
    }

    #[test]
    fn line_break_after_equals_is_ignored() {
        let stage = head("find path=\n  \"/a\" glob=\"*\"");
        assert_eq!(names(&stage), ["path", "glob"]);
    }

    #[test]
    fn value_spans_several_tokens_up_to_the_next_assignment() {
        let stage = head("filter condition=size > 1mb and modified > ago(24h) hidden=true");
        assert_eq!(names(&stage), ["condition", "hidden"]);
    }

    #[test]
    fn repeated_names_are_kept_for_binding() {
        assert_eq!(names(&head("first count=1 count=2")), ["count", "count"]);
    }

    #[test]
    fn value_without_name_is_rejected_without_guessing() {
        let error = failure(r#"find "/srv/app""#);
        assert_eq!(error.code, ErrorCode::Syntax);
        assert_eq!(error.problem.as_deref(), Some("positional_argument"));
        assert_eq!(position(&error), (1, 6));
        assert_eq!(error.found.as_deref(), Some(r#""/srv/app""#));
        assert!(error.fix.is_empty());
    }

    #[test]
    fn negative_number_after_a_space_is_a_value_without_name() {
        let error = failure("first count=20 -1");
        assert_eq!(error.problem.as_deref(), Some("positional_argument"));
        assert_eq!(position(&error), (1, 16));
        assert_eq!(error.found.as_deref(), Some("-1"));
    }

    #[test]
    fn word_after_a_complete_value_is_a_value_without_name() {
        let error = failure("sort by=size desc");
        assert_eq!(error.problem.as_deref(), Some("positional_argument"));
        assert_eq!(error.found.as_deref(), Some("desc"));
    }

    #[test]
    fn invalid_value_without_name_reports_its_first_token() {
        let error = failure("find +1");
        assert_eq!(error.problem.as_deref(), Some("positional_argument"));
        assert_eq!(error.found.as_deref(), Some("+"));
    }

    #[test]
    fn colon_is_corrected_to_equals() {
        let error = failure(r#"find path: "/a" |> count"#);
        assert_eq!(error.problem.as_deref(), Some("colon_in_named_argument"));
        assert_eq!(position(&error), (1, 6));
        assert_eq!(error.fix, [r#"find path="/a" |> count"#]);
    }

    #[test]
    fn colon_after_a_space_is_corrected_to_equals() {
        let error = failure(r#"find path : "/a""#);
        assert_eq!(error.problem.as_deref(), Some("colon_in_named_argument"));
        assert_eq!(error.fix, [r#"find path="/a""#]);
    }

    #[test]
    fn colon_at_the_end_of_a_line_is_corrected_to_equals() {
        let error = failure("find path:\n");
        assert_eq!(error.fix, ["find path=\n"]);
    }

    #[test]
    fn equals_without_value_is_rejected() {
        let error = failure("find path=");
        assert_eq!(error.problem.as_deref(), Some("missing_value"));
        let error = failure(r#"find path= glob="*""#);
        assert_eq!(error.problem.as_deref(), Some("missing_value"));
    }

    #[test]
    fn group_with_command_and_assignment_is_a_sub_pipeline() {
        let stage = head(r#"verify check=(run command=["go"] cwd="/a")"#);
        let Expr::Sub { p, .. } = &stage.args[0].value else {
            panic!("{:?}", stage.args[0].value);
        };
        assert!(p.stages.is_empty());
    }

    #[test]
    fn group_with_command_and_spaced_assignment_is_a_sub_pipeline() {
        let stage = head(r#"verify check=(run command = ["go"])"#);
        assert!(matches!(&stage.args[0].value, Expr::Sub { .. }));
    }

    #[test]
    fn group_with_command_and_keyword_assignment_is_a_sub_pipeline() {
        let stage = head(r#"verify check=(replace in=["/a"] with="b")"#);
        assert!(matches!(&stage.args[0].value, Expr::Sub { .. }));
    }

    #[test]
    fn group_with_a_pipe_is_a_sub_pipeline() {
        let stage = head(r#"verify check=(find path="/a" |> count)"#);
        let Expr::Sub { p, .. } = &stage.args[0].value else {
            panic!("{:?}", stage.args[0].value);
        };
        assert_eq!(p.stages.len(), 1);
    }

    #[test]
    fn single_word_in_parentheses_is_an_identifier() {
        let stage = head("verify check=(remove)");
        assert!(matches!(&stage.args[0].value, Expr::Ident { v, .. } if v == "remove"));
    }

    #[test]
    fn expression_in_parentheses_is_grouping() {
        for text in [
            "filter condition=(size > 1mb)",
            "filter condition=(name in [\"a\"])",
            "filter condition=(ago(1h))",
        ] {
            let stage = head(text);
            assert!(!matches!(&stage.args[0].value, Expr::Sub { .. }), "{text}");
        }
    }

    #[test]
    fn stages_of_a_sub_pipeline_accept_only_assignments() {
        let error = failure(r#"verify check=(run ["go"] |> count)"#);
        assert_eq!(error.problem.as_deref(), Some("positional_argument"));
        assert_eq!(position(&error), (1, 19));
    }

    #[test]
    fn stage_ends_at_a_separator_that_the_program_reports() {
        let error = failure(r#"find path="/a", glob="*""#);
        assert_eq!(error.problem.as_deref(), Some("comma_between_arguments"));
        assert_eq!(position(&error), (1, 15));
    }

    #[test]
    fn named_argument_span_covers_name_and_value() {
        let stage = head(r#"find path = "/srv/app" depth=1"#);
        let span = |line, column, end_line, end_column| {
            Some(Span {
                line,
                column,
                end_line,
                end_column,
            })
        };
        assert_eq!(stage.args[0].span, span(1, 6, 1, 23));
        assert_eq!(stage.args[1].span, span(1, 24, 1, 31));
        assert_eq!(stage.span, span(1, 1, 1, 31));
    }

    #[test]
    fn stage_syntax_delimits_arguments_by_whitespace() {
        assert!(SyntaxC.arguments_by_whitespace());
    }
}
