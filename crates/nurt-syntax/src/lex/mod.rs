// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lexer driven by the parser.
//!
//! The [`Lexer`] reads program text lazily, one token per call of
//! [`Lexer::next`]. The parser passes a [`LexContext`] with every call: it
//! decides whether `/` starts a regular expression or is division. Line
//! breaks are tokens only where they end a statement.
//!
//! Literal tokens carry no value; the decoders [`decode_text`],
//! [`decode_raw`], [`decode_regex`], [`parse_int`], [`parse_number`],
//! [`parse_size`] and [`parse_duration`] turn the source fragment of a token
//! into a node of the surface tree.
//!
//! The byte ranges of the tokens, together with the whitespace, the line
//! breaks and the comments between them, cover the whole input without gaps
//! or overlaps.
//!
//! # Examples
//!
//! ```
//! use nurt_syntax::lex::{LexContext, LexOptions, Lexer, TokenKind};
//!
//! let mut lexer = Lexer::new("search /old\\(/ \"/srv/app\"\n", LexOptions::default());
//! let command = lexer.next(LexContext::Value)?;
//! assert_eq!(command.kind, TokenKind::Ident);
//! let pattern = lexer.next(LexContext::Operator)?;
//! assert_eq!(pattern.kind, TokenKind::Regex);
//! assert_eq!(pattern.range, 7..14);
//! assert_eq!(lexer.next(LexContext::Operator)?.kind, TokenKind::Text);
//! assert_eq!(lexer.next(LexContext::Operator)?.kind, TokenKind::Nl);
//! assert_eq!(lexer.next(LexContext::Value)?.kind, TokenKind::Eof);
//! # Ok::<(), nurt_syntax::lex::LexError>(())
//! ```

mod cursor;
mod error;
mod foreign;
mod number;
mod regex;
mod scan;
mod simple;
mod text;
mod token;

use std::ops::Range;

use nurt_contracts::ast::Span;

use self::cursor::Cursor;
use self::error::Fault;
use self::scan::{Lexeme, Rejected, Scanned, char_len_at};
use self::simple::Simple;

pub use self::error::{LexError, LexErrorData};
pub use self::number::{
    duration_ms, parse_duration, parse_int, parse_number, parse_size, size_bytes,
};
pub use self::regex::decode_regex;
pub use self::text::{
    decode_raw, decode_text, decode_text_keeping_unknown_escapes, invalid_escapes,
};
pub use self::token::{
    Comment, ForeignKind, Glued, KEYWORDS, LexContext, LexOptions, MAX_IDENTIFIER_LEN,
    MAX_SAFE_INTEGER, Token, TokenKind,
};

/// Byte order mark, skipped at the start of the input.
const BYTE_ORDER_MARK: char = '\u{FEFF}';

/// First line ignored at the start of the input.
const SHEBANG: &str = "#!";

/// Kind of the last token, which decides whether a line break ends a
/// statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Previous {
    /// No token yet.
    Start,
    /// A token after which the statement continues: `|>`, `=`, `,`.
    Continuation,
    /// A line break token.
    Newline,
    /// Any other token.
    Value,
}

/// State of a [`Lexer`] that [`Lexer::rewind`] restores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    position: usize,
    cursor: Cursor,
    depth: u32,
    previous: Previous,
    space_pending: bool,
    glued_word_at: Option<usize>,
    comments: usize,
}

/// Lexer of a program text, driven by the parser.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    text: &'a str,
    options: LexOptions,
    /// Whether a text with escapes that the language does not have is a
    /// text token instead of an `E_ESCAPE` error.
    unknown_escapes_kept: bool,
    position: usize,
    cursor: Cursor,
    /// Number of open brackets; line breaks inside brackets are skipped.
    depth: u32,
    previous: Previous,
    /// Whether the next token starts a line after a line break token.
    space_pending: bool,
    /// Start of letters that directly follow a recovered number.
    glued_word_at: Option<usize>,
    comments: Vec<Comment>,
}

/// Line break found while skipping whitespace and comments.
#[derive(Debug, Clone)]
struct LineBreak {
    range: Range<usize>,
    cursor: Cursor,
    space_before: bool,
}

/// Whitespace, line breaks and comments before a token.
#[derive(Debug, Clone, Default)]
struct Trivia {
    space: bool,
    line_break: Option<LineBreak>,
}

impl<'a> Lexer<'a> {
    /// Makes the lexer read a text with escapes that the language does not
    /// have (`\d`) as a text token whose value keeps the characters of the
    /// escape; an invalid `\u` escape stays an error.
    #[must_use]
    pub fn keeping_unknown_escapes(mut self) -> Self {
        self.unknown_escapes_kept = true;
        self
    }

    /// Lexer of `text`. A byte order mark at the start is skipped and not
    /// counted in columns; a first line that starts with `#!` is skipped.
    #[must_use]
    pub fn new(text: &'a str, options: LexOptions) -> Self {
        let content_start = if text.starts_with(BYTE_ORDER_MARK) {
            BYTE_ORDER_MARK.len_utf8()
        } else {
            0
        };
        let has_shebang = text
            .get(content_start..)
            .is_some_and(|rest| rest.starts_with(SHEBANG));
        let position = if has_shebang {
            scan::line_end(text, content_start)
        } else {
            content_start
        };
        Self {
            text,
            options,
            unknown_escapes_kept: false,
            position,
            cursor: Cursor::START.after(text.get(content_start..position).unwrap_or_default()),
            depth: 0,
            previous: Previous::Start,
            space_pending: true,
            glued_word_at: None,
            comments: Vec::new(),
        }
    }

    /// Reads the next token in `context`.
    ///
    /// After the end of the input every call returns a [`TokenKind::Eof`]
    /// token. After an error the lexer skips the erroneous fragment, so a
    /// further call continues behind it.
    ///
    /// # Errors
    ///
    /// In strict mode, returns `E_LEXICAL` for an invalid token and
    /// `E_ESCAPE` for an invalid escape sequence of a text. In tolerant mode
    /// never returns an error.
    pub fn next(&mut self, context: LexContext) -> Result<Token, LexError> {
        let trivia = self.skip_trivia();
        if let Some(line_break) = trivia.line_break.clone()
            && self.ends_statement()
        {
            return Ok(self.line_break_token(line_break));
        }
        let space_before = trivia.space || self.space_pending;
        self.space_pending = false;
        let start = self.position;
        if start >= self.text.len() {
            return Ok(self.end_token(space_before));
        }
        let scanned = self.scan(start, context, space_before);
        self.glued_word_at = None;
        match scanned {
            Ok(lexeme) => Ok(self.emit(start, lexeme, space_before)),
            Err(rejected) => self.reject(start, *rejected, space_before),
        }
    }

    /// Comments read so far, in source order; empty unless
    /// [`LexOptions::keep_comments`] is set.
    #[must_use]
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Current state, to return to with [`Lexer::rewind`].
    #[must_use]
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            position: self.position,
            cursor: self.cursor,
            depth: self.depth,
            previous: self.previous,
            space_pending: self.space_pending,
            glued_word_at: self.glued_word_at,
            comments: self.comments.len(),
        }
    }

    /// Returns to the state of `checkpoint`: the position, the bracket depth
    /// that controls line breaks, the state of the previous token and the
    /// comments read before the checkpoint.
    pub fn rewind(&mut self, checkpoint: Checkpoint) {
        self.position = checkpoint.position;
        self.cursor = checkpoint.cursor;
        self.depth = checkpoint.depth;
        self.previous = checkpoint.previous;
        self.space_pending = checkpoint.space_pending;
        self.glued_word_at = checkpoint.glued_word_at;
        self.comments.truncate(checkpoint.comments);
    }

    fn advance_to(&mut self, end: usize) {
        let end = end.clamp(self.position, self.text.len());
        self.cursor
            .advance(self.text.get(self.position..end).unwrap_or_default());
        self.position = end;
    }

    fn skip_trivia(&mut self) -> Trivia {
        let mut trivia = Trivia::default();
        loop {
            let bytes = self.text.as_bytes();
            match bytes.get(self.position) {
                Some(b' ' | b'\t') => {
                    let run = bytes
                        .get(self.position..)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|&&byte| byte == b' ' || byte == b'\t')
                        .count();
                    self.advance_to(self.position + run);
                    trivia.space = true;
                }
                Some(b'\n') => self.skip_line_break(&mut trivia, 1),
                Some(b'\r') if bytes.get(self.position + 1) == Some(&b'\n') => {
                    self.skip_line_break(&mut trivia, 2);
                }
                Some(b'#') => self.skip_comment(),
                _ => return trivia,
            }
        }
    }

    fn skip_line_break(&mut self, trivia: &mut Trivia, length: usize) {
        if trivia.line_break.is_none() {
            trivia.line_break = Some(LineBreak {
                range: self.position..self.position + length,
                cursor: self.cursor,
                space_before: trivia.space,
            });
        }
        self.advance_to(self.position + length);
        trivia.space = true;
    }

    fn skip_comment(&mut self) {
        let start = self.position;
        let start_cursor = self.cursor;
        let end = scan::line_end(self.text, start);
        self.advance_to(end);
        if self.options.keep_comments {
            self.comments.push(Comment {
                span: span(start_cursor, self.cursor),
                text: self.text.get(start + 1..end).unwrap_or_default().to_owned(),
            });
        }
    }

    /// Whether a line break at the current position ends a statement.
    fn ends_statement(&self) -> bool {
        self.depth == 0 && self.previous == Previous::Value && !self.continues_pipeline()
    }

    /// Whether the text at the current position continues the pipeline of
    /// the previous line: `|>`, or in tolerant mode a pipe of another shell.
    fn continues_pipeline(&self) -> bool {
        let rest = self.text.get(self.position..).unwrap_or_default();
        rest.starts_with("|>")
            || (self.options.tolerant
                && ((rest.starts_with('|') && !rest.starts_with("||")) || rest.starts_with("->")))
    }

    fn line_break_token(&mut self, line_break: LineBreak) -> Token {
        self.previous = Previous::Newline;
        self.space_pending = true;
        let length = u32::try_from(line_break.range.len()).unwrap_or(1);
        let start = line_break.cursor;
        let end = Cursor {
            line: start.line,
            column: start.column.saturating_add(length),
        };
        Token {
            kind: TokenKind::Nl,
            span: span(start, end),
            range: line_break.range,
            space_before: line_break.space_before,
            space_after: true,
            glued: Glued::None,
        }
    }

    fn end_token(&self, space_before: bool) -> Token {
        let end = self.text.len();
        Token {
            kind: TokenKind::Eof,
            span: span(self.cursor, self.cursor),
            range: end..end,
            space_before,
            space_after: true,
            glued: Glued::None,
        }
    }

    fn emit(&mut self, start: usize, lexeme: Lexeme, space_before: bool) -> Token {
        let end = if lexeme.end > start {
            lexeme.end
        } else {
            start + char_len_at(self.text, start)
        };
        let start_cursor = self.cursor;
        self.advance_to(end);
        self.update_state(&lexeme.kind);
        Token {
            kind: lexeme.kind,
            span: span(start_cursor, self.cursor),
            range: start..end,
            space_before,
            space_after: self.space_at(end),
            glued: self.glued_at(end),
        }
    }

    fn reject(
        &mut self,
        start: usize,
        rejected: Rejected,
        space_before: bool,
    ) -> Result<Token, LexError> {
        let minimum_end = start + char_len_at(self.text, start);
        if self.options.tolerant {
            let lexeme = match rejected.recovery {
                Some(lexeme) => lexeme,
                None => {
                    let end = rejected.fault.resume.max(minimum_end);
                    let error = rejected.fault.into_error(self.text, start, self.cursor);
                    Lexeme::new(end, TokenKind::Foreign(ForeignKind::Malformed(error)))
                }
            };
            if rejected.glued_word {
                self.glued_word_at = Some(lexeme.end);
            }
            return Ok(self.emit(start, lexeme, space_before));
        }
        let resume = rejected.fault.resume.max(minimum_end);
        let error = rejected.fault.into_error(self.text, start, self.cursor);
        self.advance_to(resume);
        self.previous = Previous::Value;
        Err(error)
    }

    fn update_state(&mut self, kind: &TokenKind) {
        match kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                self.depth = self.depth.saturating_add(1);
            }
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                self.depth = self.depth.saturating_sub(1);
            }
            _ => {}
        }
        self.previous = match kind {
            TokenKind::PipeGt
            | TokenKind::Eq
            | TokenKind::Comma
            | TokenKind::Foreign(
                ForeignKind::Pipe
                | ForeignKind::PipeSpaceGt
                | ForeignKind::PipeArrow
                | ForeignKind::Arrow,
            ) => Previous::Continuation,
            _ => Previous::Value,
        };
    }

    /// Whether whitespace, a comment or the end of the input is at `offset`.
    fn space_at(&self, offset: usize) -> bool {
        self.text
            .get(offset..)
            .and_then(|rest| rest.chars().next())
            .is_none_or(|next| next == '#' || foreign::is_space(next))
    }

    fn glued_at(&self, offset: usize) -> Glued {
        let bytes = self.text.as_bytes();
        match bytes.get(offset) {
            Some(b':') => Glued::Colon,
            Some(b'(') => Glued::LParen,
            Some(b'=') if bytes.get(offset + 1) != Some(&b'=') => Glued::Equals,
            _ => Glued::None,
        }
    }

    fn scan(&self, start: usize, context: LexContext, space_before: bool) -> Scanned {
        let Some(&first) = self.text.as_bytes().get(start) else {
            return Ok(Lexeme::new(start, TokenKind::Eof));
        };
        match first {
            b'"' => text::scan_text(self.text, start, self.unknown_escapes_kept),
            b'/' => self.scan_slash(start, context, space_before),
            b'0'..=b'9' => number::scan_number(self.text, start),
            // Letters glued to a number belong to the number, so `1r"q"` is a
            // number with a loose word, as the strict lexer reads it.
            b'r' if self.glued_word_at != Some(start)
                && text::raw_hashes(self.text, start).is_some() =>
            {
                text::scan_raw(self.text, start)
            }
            b'\'' => foreign::scan_single_quoted(self.text, start),
            b'-' => self.scan_minus(start, space_before),
            b'~' | b'.' if self.options.tolerant && context == LexContext::Value => self
                .scan_relative_path(start)
                .unwrap_or_else(|| self.scan_simple(start)),
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => self.scan_word(start),
            _ if foreign::is_extra_whitespace(char::from(first)) => {
                foreign::scan_whitespace(self.text, start)
            }
            _ if first.is_ascii() => self.scan_simple(start),
            _ => self.scan_unicode(start),
        }
    }

    fn scan_simple(&self, start: usize) -> Scanned {
        let rest = self.text.get(start..).unwrap_or_default();
        let Some((simple, length)) = simple::longest(rest) else {
            return Err(unexpected_character(self.text, start));
        };
        let end = start + length;
        if let Some(kind) = simple.kind() {
            return Ok(Lexeme::new(end, kind));
        }
        match simple.foreign() {
            Some((foreign, problem, message, fix)) => {
                let fault = Fault::lexical(start..end, problem, message, "syntax");
                let fault = if fix.is_empty() {
                    fault
                } else {
                    fault.with_fix(fix)
                };
                Err(scan::reject_with(
                    fault,
                    Lexeme::new(end, TokenKind::Foreign(foreign)),
                ))
            }
            None => self.scan_word(start),
        }
    }

    fn scan_slash(&self, start: usize, context: LexContext, space_before: bool) -> Scanned {
        let starts_regex = match context {
            LexContext::Value => true,
            LexContext::Operator => {
                self.options.arguments_by_whitespace
                    && space_before
                    && self
                        .text
                        .get(start + 1..)
                        .and_then(|rest| rest.chars().next())
                        .is_some_and(|next| !foreign::is_space(next))
            }
        };
        if !starts_regex {
            return Ok(Lexeme::new(start + 1, TokenKind::Slash));
        }
        regex::scan_regex(self.text, start).map_err(|mut rejected| {
            if regex::is_path_problem(rejected.fault.problem) {
                let end = foreign::bare_path_end(self.text, start).max(start + 1);
                rejected.recovery =
                    Some(Lexeme::new(end, TokenKind::Foreign(ForeignKind::BarePath)));
            }
            rejected
        })
    }

    /// Scans `-`, `->`, `--name` and, after whitespace, a flag `-Name` with an
    /// upper-case letter.
    fn scan_minus(&self, start: usize, space_before: bool) -> Scanned {
        let bytes = self.text.as_bytes();
        let word_start = start + 1;
        if space_before && bytes.get(word_start).is_some_and(u8::is_ascii_alphabetic) {
            let word_end = word_start
                + bytes
                    .get(word_start..)
                    .unwrap_or_default()
                    .iter()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
                    .count();
            let word = bytes.get(word_start..word_end).unwrap_or_default();
            if word.iter().any(u8::is_ascii_uppercase) {
                let fault = Fault::lexical(
                    start..word_end,
                    "flag",
                    "flags are not part of the language; write a named argument `name: value`",
                    "syntax",
                );
                return Err(scan::reject_with(
                    fault,
                    Lexeme::new(word_end, TokenKind::Foreign(ForeignKind::Flag)),
                ));
            }
        }
        self.scan_simple(start)
    }

    /// In tolerant mode and value context, scans `~/…`, `./…` and `../…`
    /// as an unquoted path when no regular expression can be read after the
    /// prefix.
    fn scan_relative_path(&self, start: usize) -> Option<Scanned> {
        let rest = self.text.get(start..)?;
        let slash = ["~/", "./", "../"]
            .iter()
            .find(|prefix| rest.starts_with(**prefix))
            .map(|prefix| start + prefix.len() - 1)?;
        if regex::scan_regex(self.text, slash).is_ok() {
            return None;
        }
        Some(Ok(Lexeme::new(
            foreign::bare_path_end(self.text, start),
            TokenKind::Foreign(ForeignKind::BarePath),
        )))
    }

    fn scan_word(&self, start: usize) -> Scanned {
        let rest = self.text.get(start..).unwrap_or_default();
        let Some((Simple::Word, length)) = simple::longest(rest) else {
            return Err(unexpected_character(self.text, start));
        };
        let end = start + length;
        if self.options.tolerant && length == 1 && self.is_drive_path(start, end) {
            return Ok(Lexeme::new(
                foreign::bare_path_end(self.text, start),
                TokenKind::Foreign(ForeignKind::BarePath),
            ));
        }
        if self
            .text
            .get(end..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(|next| !next.is_ascii() && next.is_alphanumeric())
        {
            return Err(foreign_word(self.text, start));
        }
        if self.glued_word_at == Some(start) {
            return Ok(Lexeme::new(end, TokenKind::Foreign(ForeignKind::LooseWord)));
        }
        let word = rest.get(..length).unwrap_or_default();
        classify_word(word, start..end)
    }

    /// Whether the single letter at `start` begins a Windows path `C:\…` or
    /// `C:/…`. A lower-case letter followed by `:/` begins a path only when
    /// no regular expression can be read after the colon.
    fn is_drive_path(&self, start: usize, end: usize) -> bool {
        let bytes = self.text.as_bytes();
        if bytes.get(end) != Some(&b':') {
            return false;
        }
        match bytes.get(end + 1) {
            Some(b'\\') => true,
            Some(b'/') => {
                bytes.get(start).is_some_and(u8::is_ascii_uppercase)
                    || regex::scan_regex(self.text, end + 1).is_err()
            }
            _ => false,
        }
    }

    fn scan_unicode(&self, start: usize) -> Scanned {
        let Some(first) = self.text.get(start..).and_then(|rest| rest.chars().next()) else {
            return Err(unexpected_character(self.text, start));
        };
        if foreign::is_extra_whitespace(first) {
            foreign::scan_whitespace(self.text, start)
        } else if foreign::is_typographic_quote(first) {
            foreign::scan_typographic(self.text, start)
        } else if first.is_alphanumeric() {
            Err(foreign_word(self.text, start))
        } else {
            Err(unexpected_character(self.text, start))
        }
    }
}

fn span(start: Cursor, end: Cursor) -> Span {
    Span {
        line: start.line,
        column: start.column,
        end_line: end.line,
        end_column: end.column,
    }
}

/// Classifies a word of ASCII letters, digits and `_`.
fn classify_word(word: &str, range: Range<usize>) -> Scanned {
    let loose = word.starts_with('_') || word.bytes().any(|byte| byte.is_ascii_uppercase());
    if loose {
        let name = foreign::snake_case(word);
        let fault = Fault::lexical(
            range.clone(),
            "name_case",
            format!("names are written in lower case with `_` between words: `{name}`"),
            "names",
        );
        let fault = if name.starts_with('_') {
            fault
        } else {
            fault.with_fix(name)
        };
        return Err(scan::reject_with(
            fault,
            Lexeme::new(range.end, TokenKind::Foreign(ForeignKind::LooseWord)),
        ));
    }
    if word.len() > MAX_IDENTIFIER_LEN {
        return Err(scan::reject(Fault::lexical(
            range,
            "name_too_long",
            format!("name is longer than {MAX_IDENTIFIER_LEN} characters; write a shorter name"),
            "names",
        )));
    }
    let kind = match word {
        "let" => TokenKind::Let,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "null" => TokenKind::Null,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        "in" => TokenKind::In,
        _ => TokenKind::Ident,
    };
    Ok(Lexeme::new(range.end, kind))
}

/// Rejects a word that contains letters or digits outside ASCII.
fn foreign_word(text: &str, start: usize) -> Box<Rejected> {
    let rest = text.get(start..).unwrap_or_default();
    let length = rest
        .char_indices()
        .find(|&(_, character)| !(character.is_alphanumeric() || character == '_'))
        .map_or(rest.len(), |(offset, _)| offset);
    let end = start + length.max(char_len_at(text, start));
    let fault = Fault::lexical(
        start..end,
        "non_ascii_name",
        "names use the letters a-z, digits and `_`; write other words as text in quotes \"…\"",
        "names",
    );
    scan::reject_with(
        fault,
        Lexeme::new(end, TokenKind::Foreign(ForeignKind::ForeignWord)),
    )
}

fn unexpected_character(text: &str, start: usize) -> Box<Rejected> {
    let character = text
        .get(start..)
        .and_then(|rest| rest.chars().next())
        .unwrap_or_default();
    foreign::lone_character(
        text,
        start,
        "unexpected_character",
        format!("unexpected character `{}`", character.escape_debug()),
    )
}

/// Reads all tokens of `text`, choosing the context of each token from the
/// previous one: [`LexContext::Operator`] after an identifier, a literal or a
/// closing bracket, [`LexContext::Value`] elsewhere. The parser drives the
/// lexer itself; this function serves tools that need the token stream
/// without parsing, such as syntax highlighting.
///
/// The last token is [`TokenKind::Eof`].
///
/// # Errors
///
/// Returns the first error of [`Lexer::next`].
///
/// # Examples
///
/// ```
/// use nurt_syntax::lex::{LexOptions, TokenKind, tokenize};
///
/// let tokens = tokenize("first 20 -1", LexOptions::default())?;
/// let kinds: Vec<_> = tokens.iter().map(|token| token.kind.clone()).collect();
/// assert_eq!(
///     kinds,
///     [TokenKind::Ident, TokenKind::Int, TokenKind::Minus, TokenKind::Int, TokenKind::Eof]
/// );
/// assert!(tokens[2].space_before && !tokens[2].space_after);
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn tokenize(text: &str, options: LexOptions) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer::new(text, options);
    let mut tokens = Vec::new();
    let mut context = LexContext::Value;
    loop {
        let token = lexer.next(context)?;
        context = context_after(&token.kind);
        let end = token.kind == TokenKind::Eof;
        tokens.push(token);
        if end {
            return Ok(tokens);
        }
    }
}

/// Byte ranges of the text literals of `text` that have an escape the
/// language does not have, such as `"\d+"`, in the order of the text.
///
/// The literals are found in the token stream of [`tokenize`]; the search
/// ends at the first token that is not valid for another reason.
///
/// # Examples
///
/// ```
/// use nurt_syntax::lex::unknown_escape_texts;
///
/// let text = r#"search "\d+" "/srv\n" "C:\proj""#;
/// let found: Vec<&str> = unknown_escape_texts(text)
///     .into_iter()
///     .map(|range| &text[range])
///     .collect();
/// assert_eq!(found, [r#""\d+""#, r#""C:\proj""#]);
/// ```
#[must_use]
pub fn unknown_escape_texts(text: &str) -> Vec<Range<usize>> {
    let mut lexer = Lexer::new(text, LexOptions::default()).keeping_unknown_escapes();
    let mut found = Vec::new();
    let mut context = LexContext::Value;
    while let Ok(token) = lexer.next(context) {
        context = context_after(&token.kind);
        if token.kind == TokenKind::Eof {
            break;
        }
        if token.kind == TokenKind::Text
            && text
                .get(token.range.clone())
                .is_some_and(text::has_unknown_escape)
        {
            found.push(token.range);
        }
    }
    found
}

/// Context of the token after a token of `kind`.
fn context_after(kind: &TokenKind) -> LexContext {
    match kind {
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
        | TokenKind::Foreign(
            ForeignKind::LooseWord
            | ForeignKind::ForeignWord
            | ForeignKind::SingleQuoted { .. }
            | ForeignKind::TypographicQuoted { .. }
            | ForeignKind::BarePath,
        ) => LexContext::Operator,
        _ => LexContext::Value,
    }
}
