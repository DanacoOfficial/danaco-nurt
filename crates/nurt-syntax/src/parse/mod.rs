// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parsers of the syntax variants A, B and C.
//!
//! [`parse`] turns program text into a surface tree `nurt-ast/1` with spans.
//! [`parse_tolerant`] parses strictly first and, when strict parsing fails,
//! asks a [`RepairPolicy`] for edits of the text at the place of the failure
//! and tries them in a fixed order.
//!
//! # Examples
//!
//! ```
//! use nurt_syntax::parse::{ParseOptions, Variant, parse};
//!
//! let document = parse("find \"/srv/app\" |> first 20", Variant::A, &ParseOptions::default())?;
//! let json = serde_json::to_value(&document)?;
//! assert_eq!(json["program"]["statements"][0]["head"]["name"], "find");
//! assert_eq!(json["program"]["statements"][0]["stages"][0]["args"][0]["value"]["v"], 20);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod a;
mod b;
mod c;
pub(crate) mod error;
pub(crate) mod expr;
pub mod repair;

use std::ops::Range;

use nurt_contracts::ast::{
    AST_VERSION, AstDocument, Expr, Head, Let, Meta, Pipeline, Program, Ref, Span, Stage, Statement,
};

use crate::error::SyntaxError;
use crate::lex::{Checkpoint, Glued, LexContext, LexOptions, Lexer, Token, TokenKind};

pub use nurt_contracts::kinds::SyntaxVariant as Variant;
pub use repair::{AppliedRepair, Failure, NoRepair, Repair, RepairPolicy, Repaired};

/// Default of [`ParseOptions::max_repairs`].
pub const DEFAULT_MAX_REPAIRS: u8 = 3;

/// Default of [`ParseOptions::max_depth`]; equal to the nesting limit of the
/// tree decoders.
pub const DEFAULT_MAX_DEPTH: usize = 128;

/// Longest command name in characters: two identifiers of 64 characters
/// joined by `.`.
pub const MAX_COMMAND_NAME_LEN: usize = 129;

/// Options of [`parse`] and [`parse_tolerant`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseOptions {
    /// Whether the comments of the text are stored in `meta.comments`.
    pub keep_comments: bool,
    /// Whether the program text is stored in `meta.source`.
    pub keep_source: bool,
    /// Maximum number of repairs per statement in [`parse_tolerant`].
    pub max_repairs: u8,
    /// Maximum nesting depth of tree nodes below a statement.
    pub max_depth: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            keep_comments: false,
            keep_source: false,
            max_repairs: DEFAULT_MAX_REPAIRS,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

/// Parses program text of a syntax variant into a surface tree.
///
/// Every node of the tree carries its span. `meta` is present when
/// [`ParseOptions::keep_comments`] or [`ParseOptions::keep_source`] is set and
/// then always records the variant.
///
/// # Errors
///
/// Returns `E_LEXICAL` or `E_ESCAPE` for an invalid token and `E_SYNTAX` when
/// the text does not follow the grammar of the variant or nests deeper than
/// [`ParseOptions::max_depth`]. The error carries the position, the fragment
/// found, the expected tokens and, where the correction is certain, up to
/// three corrected program texts.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::ErrorCode;
/// use nurt_syntax::parse::{ParseOptions, Variant, parse};
///
/// let error = parse("filter a < b < c", Variant::A, &ParseOptions::default()).unwrap_err();
/// assert_eq!(error.code, ErrorCode::Syntax);
/// assert_eq!(error.fix, ["filter a < b and b < c"]);
/// ```
pub fn parse(
    text: &str,
    variant: Variant,
    options: &ParseOptions,
) -> Result<AstDocument, SyntaxError> {
    let mut parser = Parser::new(text, variant, options, false)?;
    let program = parse_variant(&mut parser)?;
    Ok(parser.document(program, options))
}

/// Parses program text and repairs it with `policy` when strict parsing
/// fails.
///
/// A text that parses strictly yields the same tree as [`parse`] and no
/// repairs. Otherwise the lexer runs in tolerant mode and, at each failure,
/// the edits proposed by `policy` are tried in the order `L10`, `L11`, `L12`,
/// `L13`, `L14`, `L15`, `L08`, `L09`, up to three per failure and up to
/// [`ParseOptions::max_repairs`] per statement. The first edited text that
/// parses wins; texts that parse into different trees are returned as
/// alternatives.
///
/// # Errors
///
/// Returns the error of strict parsing when no sequence of repairs yields a
/// tree.
///
/// # Examples
///
/// ```
/// use nurt_syntax::parse::{NoRepair, ParseOptions, Variant, parse_tolerant};
///
/// let repaired = parse_tolerant("count", Variant::A, &ParseOptions::default(), &NoRepair)?;
/// assert!(repaired.repairs.is_empty());
/// assert!(parse_tolerant("count )", Variant::A, &ParseOptions::default(), &NoRepair).is_err());
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub fn parse_tolerant(
    text: &str,
    variant: Variant,
    options: &ParseOptions,
    policy: &dyn RepairPolicy,
) -> Result<Repaired, SyntaxError> {
    repair::parse_with_repairs(text, variant, options, policy)
}

fn parse_variant(parser: &mut Parser<'_>) -> Result<Program, SyntaxError> {
    match parser.variant {
        Variant::A => a::parse_program(parser),
        Variant::C => c::parse_program(parser),
        Variant::B => Err(error::variant_unavailable(parser.variant)),
    }
}

/// A parsed piece of the tree with its span, its byte range in the text and
/// its height in tree levels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parsed<T> {
    pub(crate) value: T,
    pub(crate) span: Span,
    pub(crate) range: Range<usize>,
    pub(crate) height: usize,
}

/// A parsed expression.
pub(crate) type Node = Parsed<Expr>;

/// Stage notation of a syntax variant.
///
/// The statement, `let` and pipeline grammar and every expression are
/// shared; a variant decides how a stage and its arguments are written and
/// how arguments are delimited.
pub(crate) trait StageSyntax {
    /// Parses a stage; the current token is the first identifier of the
    /// command name.
    fn parse_stage(&self, p: &mut Parser<'_>) -> Result<Parsed<Stage>, SyntaxError>;

    /// Whether `next`, the token after a command name ending with `last`,
    /// starts the arguments of a stage. Inside parentheses this makes the
    /// content a sub-pipeline rather than an expression.
    fn starts_arguments(&self, last: &Token, next: &Token) -> bool;

    /// Whether whitespace separates arguments, so that `-` and `+` preceded
    /// but not followed by whitespace start a new argument.
    fn arguments_by_whitespace(&self) -> bool;
}

/// Saved state of a [`Parser`] for bounded lookahead.
#[derive(Debug, Clone)]
pub(crate) struct ParserCheckpoint {
    lexer: Checkpoint,
    current: Token,
    brackets: usize,
    trace: usize,
}

/// State of a parse: the token stream, the variant and the nesting depth.
pub(crate) struct Parser<'a> {
    text: &'a str,
    lexer: Lexer<'a>,
    variant: Variant,
    current: Token,
    max_depth: usize,
    nesting: usize,
    statement: usize,
    brackets: Vec<Token>,
    trace: Option<Vec<Token>>,
}

impl<'a> Parser<'a> {
    /// Starts a parse of `text`; `tolerant` selects the tolerant lexer and
    /// records the consumed tokens for repair policies.
    pub(crate) fn new(
        text: &'a str,
        variant: Variant,
        options: &ParseOptions,
        tolerant: bool,
    ) -> Result<Self, SyntaxError> {
        let lex_options = LexOptions {
            arguments_by_whitespace: variant != Variant::B,
            tolerant,
            keep_comments: options.keep_comments,
        };
        let mut lexer = Lexer::new(text, lex_options);
        let current = lexer.next(LexContext::Value)?;
        Ok(Self {
            text,
            lexer,
            variant,
            current,
            max_depth: options.max_depth,
            nesting: 0,
            statement: 0,
            brackets: Vec::new(),
            trace: tolerant.then(Vec::new),
        })
    }

    /// The program text.
    pub(crate) fn text(&self) -> &'a str {
        self.text
    }

    /// The current token.
    pub(crate) fn current(&self) -> &Token {
        &self.current
    }

    /// Kind of the current token.
    pub(crate) fn kind(&self) -> &TokenKind {
        &self.current.kind
    }

    /// Whether the current token has the kind.
    pub(crate) fn at(&self, kind: &TokenKind) -> bool {
        self.current.kind == *kind
    }

    /// Source text of a token.
    pub(crate) fn slice(&self, token: &Token) -> &'a str {
        self.text.get(token.range.clone()).unwrap_or_default()
    }

    /// Consumes the current token and reads the next one in `context`.
    ///
    /// # Errors
    ///
    /// Returns the lexical error of the next token.
    pub(crate) fn bump(&mut self, context: LexContext) -> Result<Token, SyntaxError> {
        let next = self.lexer.next(context)?;
        let consumed = std::mem::replace(&mut self.current, next);
        match consumed.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                self.brackets.push(consumed.clone());
            }
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                self.brackets.pop();
            }
            _ => {}
        }
        if let Some(trace) = &mut self.trace {
            trace.push(consumed.clone());
        }
        Ok(consumed)
    }

    /// Consumes the current token when it has the kind, otherwise fails with
    /// an error that names `expected`.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` when the current token has another kind, or the
    /// lexical error of the next token.
    pub(crate) fn expect(
        &mut self,
        kind: &TokenKind,
        context: LexContext,
        expected: &str,
    ) -> Result<Token, SyntaxError> {
        if self.at(kind) {
            self.bump(context)
        } else {
            Err(self.unexpected(&[expected]))
        }
    }

    /// Saves the state for [`Parser::rewind`].
    pub(crate) fn checkpoint(&self) -> ParserCheckpoint {
        ParserCheckpoint {
            lexer: self.lexer.checkpoint(),
            current: self.current.clone(),
            brackets: self.brackets.len(),
            trace: self.trace.as_ref().map_or(0, Vec::len),
        }
    }

    /// Restores a state saved by [`Parser::checkpoint`].
    pub(crate) fn rewind(&mut self, checkpoint: ParserCheckpoint) {
        self.lexer.rewind(checkpoint.lexer);
        self.current = checkpoint.current;
        self.brackets.truncate(checkpoint.brackets);
        if let Some(trace) = &mut self.trace {
            trace.truncate(checkpoint.trace);
        }
    }

    /// Enters a nested construct.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` when the nesting exceeds the maximum depth.
    pub(crate) fn enter(&mut self) -> Result<(), SyntaxError> {
        self.nesting += 1;
        if self.nesting > self.max_depth {
            Err(error::too_deep_at(self.current.span))
        } else {
            Ok(())
        }
    }

    /// Leaves a construct entered with [`Parser::enter`].
    pub(crate) fn leave(&mut self) {
        self.nesting = self.nesting.saturating_sub(1);
    }

    /// Checks the height of a node with the span.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` when the node nests deeper than the maximum depth.
    pub(crate) fn check_height(&self, height: usize, span: Span) -> Result<(), SyntaxError> {
        if height > self.max_depth {
            Err(error::too_deep_at(span))
        } else {
            Ok(())
        }
    }

    /// Error for an unexpected current token.
    pub(crate) fn unexpected(&self, expected: &[&str]) -> SyntaxError {
        error::unexpected(self, &self.current, expected)
    }

    /// Parses a command name `IDENT {"." IDENT}`; the token after the name is
    /// read in `context`. Returns the name, its first and its last token.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` when the current token is not an identifier, when
    /// a member after `.` is missing, separated by whitespace or a keyword,
    /// or when the name is longer than [`MAX_COMMAND_NAME_LEN`].
    pub(crate) fn parse_command_name(
        &mut self,
        context: LexContext,
    ) -> Result<(String, Token, Token), SyntaxError> {
        if self.current.kind != TokenKind::Ident {
            return Err(error::not_a_command(self));
        }
        let first = self.bump(context)?;
        let mut name = self.slice(&first).to_owned();
        let mut last = first.clone();
        while self.at(&TokenKind::Dot) && !self.current.space_before {
            self.bump(LexContext::Value)?;
            if self.current.kind != TokenKind::Ident || self.current.space_before {
                return Err(self.unexpected(&["identifier after `.`"]));
            }
            last = self.bump(context)?;
            name.push('.');
            name.push_str(self.slice(&last));
        }
        if name.chars().count() > MAX_COMMAND_NAME_LEN {
            return Err(error::name_too_long(self, &first, &last));
        }
        Ok((name, first, last))
    }

    /// Parses a program with the stage notation of a variant:
    /// `{NL} statement {NL {NL} statement} {NL} EOF`.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` for a program without statements, for two
    /// statements on one line and for every grammar error of a statement.
    pub(crate) fn parse_program_with(
        &mut self,
        syntax: &dyn StageSyntax,
    ) -> Result<Program, SyntaxError> {
        self.skip_line_breaks()?;
        if self.at(&TokenKind::Eof) {
            return Err(error::empty_program(self));
        }
        let mut statements = Vec::new();
        loop {
            statements.push(self.parse_statement(syntax)?);
            match self.current.kind {
                TokenKind::Eof => break,
                TokenKind::Nl => {
                    self.skip_line_breaks()?;
                    if self.at(&TokenKind::Eof) {
                        break;
                    }
                    self.statement += 1;
                }
                _ => return Err(error::statement_end(self)),
            }
        }
        Ok(Program { statements })
    }

    fn skip_line_breaks(&mut self) -> Result<(), SyntaxError> {
        while self.at(&TokenKind::Nl) {
            self.bump(LexContext::Value)?;
        }
        Ok(())
    }

    fn parse_statement(&mut self, syntax: &dyn StageSyntax) -> Result<Statement, SyntaxError> {
        if self.at(&TokenKind::Let) {
            return self.parse_let(syntax).map(Statement::Let);
        }
        match self.current.kind {
            TokenKind::Ident | TokenKind::At => {
                let pipeline = self.parse_pipeline(syntax)?;
                Ok(Statement::Pipeline(pipeline.value))
            }
            _ => Err(error::not_a_statement(self)),
        }
    }

    fn parse_let(&mut self, syntax: &dyn StageSyntax) -> Result<Let, SyntaxError> {
        let keyword = self.bump(LexContext::Value)?;
        if self.current.kind != TokenKind::Ident {
            return Err(error::bad_let_name(self));
        }
        let name = self.bump(LexContext::Value)?;
        self.expect(&TokenKind::Eq, LexContext::Value, "`=`")?;
        let pipeline = self.parse_pipeline(syntax)?;
        let span = join(keyword.span, pipeline.span);
        self.check_height(pipeline.height + 1, span)?;
        Ok(Let {
            name: self.slice(&name).to_owned(),
            value: pipeline.value,
            span: Some(span),
        })
    }

    /// Parses a pipeline `head {"|>" stage}`, where the head is a stage or a
    /// reference.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` for a reference after `|>` and for every grammar
    /// error of a stage.
    pub(crate) fn parse_pipeline(
        &mut self,
        syntax: &dyn StageSyntax,
    ) -> Result<Parsed<Pipeline>, SyntaxError> {
        let head = if self.at(&TokenKind::At) {
            let reference = self.parse_ref()?;
            Parsed {
                value: Head::Ref(reference.value),
                span: reference.span,
                range: reference.range,
                height: reference.height,
            }
        } else {
            let stage = syntax.parse_stage(self)?;
            Parsed {
                value: Head::Stage(stage.value),
                span: stage.span,
                range: stage.range,
                height: stage.height,
            }
        };
        self.parse_pipeline_tail(head, syntax)
    }

    /// Parses the stages after a head that is already parsed.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` for a reference after `|>` and for every grammar
    /// error of a stage.
    pub(crate) fn parse_pipeline_tail(
        &mut self,
        head: Parsed<Head>,
        syntax: &dyn StageSyntax,
    ) -> Result<Parsed<Pipeline>, SyntaxError> {
        let mut span = head.span;
        let mut range = head.range.clone();
        let mut height = head.height;
        let mut stages = Vec::new();
        while self.at(&TokenKind::PipeGt) {
            self.bump(LexContext::Value)?;
            if self.at(&TokenKind::At) {
                return Err(error::reference_after_pipe(self));
            }
            let stage = syntax.parse_stage(self)?;
            span = join(span, stage.span);
            range.end = stage.range.end;
            height = height.max(stage.height);
            stages.push(stage.value);
        }
        self.check_height(height + 1, span)?;
        Ok(Parsed {
            value: Pipeline {
                head: head.value,
                stages,
                span: Some(span),
            },
            span,
            range,
            height: height + 1,
        })
    }

    /// Parses a reference `@name {"." field}`.
    ///
    /// # Errors
    ///
    /// Returns `E_SYNTAX` when the name or a field is missing, separated by
    /// whitespace or a keyword, and for a call on a field.
    pub(crate) fn parse_ref(&mut self) -> Result<Parsed<Ref>, SyntaxError> {
        let at = self.bump(LexContext::Value)?;
        if self.current.kind != TokenKind::Ident || self.current.space_before {
            return Err(self.unexpected(&["result name after `@`"]));
        }
        let name = self.bump(LexContext::Operator)?;
        let mut last = name.clone();
        let mut fields = Vec::new();
        while self.at(&TokenKind::Dot) && !self.current.space_before {
            self.bump(LexContext::Value)?;
            if self.current.kind != TokenKind::Ident || self.current.space_before {
                return Err(self.unexpected(&["field name after `.`"]));
            }
            last = self.bump(LexContext::Operator)?;
            if last.glued == Glued::LParen {
                return Err(error::call_on_field(self, &last));
            }
            fields.push(self.slice(&last).to_owned());
        }
        if name.glued == Glued::LParen && fields.is_empty() {
            return Err(error::call_on_field(self, &name));
        }
        let span = join(at.span, last.span);
        Ok(Parsed {
            value: Ref {
                name: self.slice(&name).to_owned(),
                fields,
                span: Some(span),
            },
            span,
            range: at.range.start..last.range.end,
            height: 1,
        })
    }

    fn document(&self, program: Program, options: &ParseOptions) -> AstDocument {
        let meta = (options.keep_comments || options.keep_source).then(|| self.meta(options));
        AstDocument {
            nurt: AST_VERSION.to_owned(),
            program,
            meta,
        }
    }

    fn meta(&self, options: &ParseOptions) -> Meta {
        let mut meta = Meta::new();
        meta.insert(
            "variant".to_owned(),
            serde_json::to_value(self.variant).unwrap_or(serde_json::Value::Null),
        );
        if options.keep_comments {
            let comments = serde_json::to_value(self.lexer.comments())
                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
            meta.insert("comments".to_owned(), comments);
        }
        if options.keep_source {
            meta.insert(
                "source".to_owned(),
                serde_json::Value::String(self.text.to_owned()),
            );
        }
        meta
    }

    /// State of a failed parse for a repair policy: the tokens consumed so
    /// far in tolerant mode, the current token and the tokens after it, read
    /// in the context implied by the previous token.
    pub(crate) fn into_trace(mut self) -> Trace {
        let mut tokens = self.trace.take().unwrap_or_default();
        let index = tokens.len();
        let mut last = self.current.clone();
        tokens.push(last.clone());
        while last.kind != TokenKind::Eof {
            let context = if ends_operand(&last.kind) {
                LexContext::Operator
            } else {
                LexContext::Value
            };
            match self.lexer.next(context) {
                Ok(token) => {
                    last = token;
                    tokens.push(last.clone());
                }
                Err(_) => break,
            }
        }
        Trace {
            tokens,
            index,
            brackets: self.brackets,
            statement: self.statement,
        }
    }
}

/// Token stream and parser state at a failure; see [`Parser::into_trace`].
pub(crate) struct Trace {
    pub(crate) tokens: Vec<Token>,
    pub(crate) index: usize,
    pub(crate) brackets: Vec<Token>,
    pub(crate) statement: usize,
}

/// Whether a token of the kind completes an operand, so that the next token
/// is read as an operator.
pub(crate) fn ends_operand(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::Text
            | TokenKind::RawText
            | TokenKind::Regex
            | TokenKind::Int
            | TokenKind::Number
            | TokenKind::Size
            | TokenKind::Duration
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
    )
}

/// Span from the start of `first` to the end of `last`.
pub(crate) fn join(first: Span, last: Span) -> Span {
    Span {
        line: first.line,
        column: first.column,
        end_line: last.end_line,
        end_column: last.end_column,
    }
}
