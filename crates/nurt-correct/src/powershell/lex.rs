// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tokenizer for the subset of PowerShell that the habit translation reads.
//!
//! The tokenizer follows the argument mode of PowerShell: a bare word ends at
//! white space or at one of `| ; & ( ) { } ,`, a token that starts with a dash
//! and a letter is a parameter, and numbers may carry the size multipliers
//! `KB`, `MB`, `GB`, `TB` and `PB`. Typographic quotes and dashes are accepted
//! where PowerShell accepts them. Every token keeps its byte range and its
//! position in Unicode scalar values, so translated nodes point into the
//! input text.

use std::fmt;

use nurt_contracts::ast::Span;

/// Base of the PowerShell size multipliers: `1KB` is 1024 bytes.
pub const SIZE_MULTIPLIER_BASE: u64 = 1024;

/// Size multiplier suffixes in increasing order; the exponent of
/// [`SIZE_MULTIPLIER_BASE`] is the index plus one.
const SIZE_SUFFIXES: [&str; 5] = ["kb", "mb", "gb", "tb", "pb"];

/// Radix of hexadecimal integer literals, `0x1F`.
const HEX_RADIX: u32 = 16;

/// Characters PowerShell accepts as a single quotation mark.
const SINGLE_QUOTES: [char; 4] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}'];

/// Characters PowerShell accepts as a double quotation mark.
const DOUBLE_QUOTES: [char; 4] = ['"', '\u{201C}', '\u{201D}', '\u{201E}'];

/// Characters PowerShell accepts as the dash of a parameter or operator.
const DASHES: [char; 4] = ['-', '\u{2013}', '\u{2014}', '\u{2015}'];

/// A token with its kind and its place in the input.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// Kind and value of the token.
    pub kind: TokenKind,
    /// Byte offset of the first byte.
    pub start: usize,
    /// Byte offset after the last byte.
    pub end: usize,
    /// Position of the token in lines and Unicode scalar values.
    pub span: Span,
}

/// Kind of a token.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TokenKind {
    /// Bare word with backtick escapes resolved: `Get-ChildItem`, `C:\proj`,
    /// `*.go`, `%`.
    Word(String),
    /// Parameter or operator name after a dash, `-Recurse`, `-gt`; `colon`
    /// is set for the form `-Name:value`.
    Parameter {
        /// Name without the dash.
        name: String,
        /// Whether the name is followed directly by `:`.
        colon: bool,
    },
    /// Single-quoted text; `''` stands for one quotation mark.
    Literal(String),
    /// Double-quoted text or here-string with its expandable parts.
    Expandable(Vec<StrPart>),
    /// Number, possibly with a size multiplier.
    Number(NumberLit),
    /// Variable reference, `$x`, `$_`, `$env:NAME`.
    Variable(VarRef),
    /// Member access directly after a value, `.Length`.
    Member(String),
    /// Type literal, `[int]`.
    TypeName(String),
    /// `|`.
    Pipe,
    /// `;`.
    Semicolon,
    /// Line break.
    Newline,
    /// `&`, the call operator.
    Ampersand,
    /// `,`.
    Comma,
    /// `=`.
    Assign,
    /// `(`.
    OpenParen,
    /// `)`.
    CloseParen,
    /// `{`.
    OpenBrace,
    /// `}`.
    CloseBrace,
    /// `$(`, start of a subexpression.
    SubExprOpen,
    /// `@(`, start of an array subexpression.
    ArrayOpen,
    /// `@{`, start of a hash table.
    HashOpen,
    /// Output redirection, `>`, `>>`, `2>`, `2>&1`.
    Redirect(Redirect),
    /// Operator outside the dash family: `+`, `*`, `/`, `!`, `::`, `..`,
    /// `+=`.
    Operator(String),
}

/// Output redirection operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Redirect {
    /// Stream number; `None` for the success stream written as `>`.
    pub stream: Option<char>,
    /// `>>`: append instead of overwrite.
    pub append: bool,
    /// `2>&1`: merge into the success stream instead of a file.
    pub merge: bool,
}

/// Part of an expandable string.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum StrPart {
    /// Literal text with escapes resolved.
    Text(String),
    /// Expanded variable.
    Variable(VarRef),
    /// Subexpression `$( … )`, as written between the parentheses.
    SubExpr(String),
}

/// Reference to a variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarRef {
    /// Scope or drive before `:`, `env` in `$env:PATH`.
    pub scope: Option<String>,
    /// Name, `_` for the pipeline variable.
    pub name: String,
}

impl VarRef {
    /// Whether the variable is the current pipeline object, `$_` or
    /// `$PSItem`.
    #[must_use]
    pub fn is_pipeline_item(&self) -> bool {
        self.scope.is_none() && (self.name == "_" || self.name.eq_ignore_ascii_case("PSItem"))
    }

    /// Whether the variable is an environment variable, `$env:NAME`.
    #[must_use]
    pub fn is_environment(&self) -> bool {
        self.scope
            .as_deref()
            .is_some_and(|scope| scope.eq_ignore_ascii_case("env"))
    }
}

/// Numeric literal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberLit {
    /// Integer value before the multiplier, when the literal has no fraction.
    pub int: Option<i64>,
    /// Value before the multiplier.
    pub value: f64,
    /// Exponent of [`SIZE_MULTIPLIER_BASE`] given by the suffix; 0 without a
    /// suffix.
    pub exponent: u32,
}

impl NumberLit {
    /// Value in bytes of a non-negative literal with a size multiplier, when
    /// it is a whole number of bytes.
    #[must_use]
    pub fn bytes(&self) -> Option<u64> {
        let factor = SIZE_MULTIPLIER_BASE.checked_pow(self.exponent)?;
        match self.int {
            Some(int) => u64::try_from(int).ok()?.checked_mul(factor),
            None => {
                let product = self.value * factor as f64;
                let whole = product.trunc();
                (product >= 0.0 && whole == product && product < u64::MAX as f64)
                    .then(|| format!("{whole:.0}").parse().ok())
                    .flatten()
            }
        }
    }
}

/// Error of the tokenizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    /// What is wrong.
    pub kind: LexErrorKind,
    /// Byte offset where the faulty construct starts.
    pub offset: usize,
    /// Position where the faulty construct starts.
    pub span: Span,
}

/// Kind of a tokenizer error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LexErrorKind {
    /// A quoted string or here-string has no closing quotation mark.
    UnterminatedString,
    /// A block comment `<# … #>` has no end.
    UnterminatedComment,
    /// `${` has no closing brace.
    UnterminatedVariable,
    /// `$(` inside a string has no closing parenthesis.
    UnterminatedSubExpression,
    /// A character that neither starts a token nor belongs to a word.
    UnexpectedCharacter,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            LexErrorKind::UnterminatedString => "string without closing quotation mark",
            LexErrorKind::UnterminatedComment => "block comment without closing #>",
            LexErrorKind::UnterminatedVariable => "variable name without closing brace",
            LexErrorKind::UnterminatedSubExpression => "subexpression without closing parenthesis",
            LexErrorKind::UnexpectedCharacter => "character that starts no token",
        };
        write!(f, "{what} at {}:{}", self.span.line, self.span.column)
    }
}

impl std::error::Error for LexError {}

/// Splits PowerShell text into tokens.
///
/// Comments and line continuations are dropped; line breaks are kept as
/// [`TokenKind::Newline`] because they separate statements.
///
/// # Errors
///
/// Returns [`LexError`] for a string, block comment, braced variable name or
/// string subexpression without its closing character.
///
/// # Examples
///
/// ```
/// use nurt_correct::powershell::lex::{TokenKind, lex};
///
/// let tokens = lex("Get-ChildItem -Path C:\\proj -Recurse")?;
/// assert_eq!(tokens[0].kind, TokenKind::Word("Get-ChildItem".to_owned()));
/// assert!(matches!(&tokens[1].kind, TokenKind::Parameter { name, .. } if name == "Path"));
/// assert_eq!(tokens[2].kind, TokenKind::Word("C:\\proj".to_owned()));
/// # Ok::<(), nurt_correct::powershell::lex::LexError>(())
/// ```
pub fn lex(input: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(input).run()
}

/// Line and column tracker over byte offsets.
#[derive(Debug, Clone)]
pub(crate) struct Positions {
    line_starts: Vec<usize>,
}

impl Positions {
    pub(crate) fn new(input: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(input.match_indices('\n').map(|(index, _)| index + 1));
        Self { line_starts }
    }

    /// Line and column (both from 1) of a byte offset.
    pub(crate) fn at(&self, input: &str, offset: usize) -> (u32, u32) {
        let line_index = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        let column = input
            .get(line_start..offset)
            .map_or(0, |prefix| prefix.chars().count());
        (to_u32(line_index + 1), to_u32(column + 1))
    }

    /// Span of a byte range.
    pub(crate) fn span(&self, input: &str, start: usize, end: usize) -> Span {
        let (line, column) = self.at(input, start);
        let (end_line, end_column) = self.at(input, end.max(start));
        Span {
            line,
            column,
            end_line,
            end_column,
        }
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    positions: Positions,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            positions: Positions::new(input),
            tokens: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Vec<Token>, LexError> {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                '\n' => {
                    self.bump();
                    self.push(TokenKind::Newline, start);
                }
                c if c.is_whitespace() || c == '\u{FEFF}' => {
                    self.bump();
                }
                '`' if self
                    .peek_at(1)
                    .is_some_and(|next| next == '\n' || next == '\r') =>
                {
                    self.bump();
                    self.skip_line_break();
                }
                '#' => self.skip_line_comment(),
                '<' if self.peek_at(1) == Some('#') => self.skip_block_comment()?,
                _ => self.token(c, start)?,
            }
        }
        Ok(self.tokens)
    }

    fn token(&mut self, c: char, start: usize) -> Result<(), LexError> {
        match c {
            '|' => self.single(TokenKind::Pipe, start),
            ';' => self.single(TokenKind::Semicolon, start),
            ',' => self.single(TokenKind::Comma, start),
            '(' => self.single(TokenKind::OpenParen, start),
            ')' => {
                self.single(TokenKind::CloseParen, start);
                self.members();
            }
            '{' => self.single(TokenKind::OpenBrace, start),
            '}' => self.single(TokenKind::CloseBrace, start),
            '&' if self.peek_at(1) == Some('&') => self.operator(start, 2),
            '&' => self.single(TokenKind::Ampersand, start),
            '@' => self.at_sign(start)?,
            '$' => self.dollar(start)?,
            '>' => self.redirect(None, start),
            '0'..='9' | '*' if self.peek_at(1) == Some('>') => self.redirect(Some(c), start),
            '[' => self.type_name(start)?,
            _ if SINGLE_QUOTES.contains(&c) => self.literal(start)?,
            _ if DOUBLE_QUOTES.contains(&c) => self.expandable(start)?,
            _ if DASHES.contains(&c) => self.dash(start)?,
            _ => self.bare(start)?,
        }
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.input
            .get(self.pos..)
            .and_then(|rest| rest.chars().next())
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.input
            .get(self.pos..)
            .and_then(|rest| rest.chars().nth(n))
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        let span = self.positions.span(self.input, start, self.pos);
        self.tokens.push(Token {
            kind,
            start,
            end: self.pos,
            span,
        });
    }

    fn single(&mut self, kind: TokenKind, start: usize) {
        self.bump();
        self.push(kind, start);
    }

    fn operator(&mut self, start: usize, chars: usize) {
        for _ in 0..chars {
            self.bump();
        }
        let text = self
            .input
            .get(start..self.pos)
            .unwrap_or_default()
            .to_owned();
        self.push(TokenKind::Operator(text), start);
    }

    fn error(&self, kind: LexErrorKind, offset: usize) -> LexError {
        LexError {
            kind,
            offset,
            span: self.positions.span(self.input, offset, offset),
        }
    }

    fn skip_line_break(&mut self) {
        if self.peek() == Some('\r') {
            self.bump();
        }
        if self.peek() == Some('\n') {
            self.bump();
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.bump();
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), LexError> {
        let start = self.pos;
        match self
            .input
            .get(self.pos + 2..)
            .and_then(|rest| rest.find("#>"))
        {
            Some(end) => {
                self.pos += 2 + end + 2;
                Ok(())
            }
            None => Err(self.error(LexErrorKind::UnterminatedComment, start)),
        }
    }

    fn at_sign(&mut self, start: usize) -> Result<(), LexError> {
        match self.peek_at(1) {
            Some('(') => self.operator_kind(TokenKind::ArrayOpen, start, 2),
            Some('{') => self.operator_kind(TokenKind::HashOpen, start, 2),
            Some(q) if SINGLE_QUOTES.contains(&q) || DOUBLE_QUOTES.contains(&q) => {
                return self.here_string(start, DOUBLE_QUOTES.contains(&q));
            }
            _ => self.bare(start)?,
        }
        Ok(())
    }

    fn operator_kind(&mut self, kind: TokenKind, start: usize, chars: usize) {
        for _ in 0..chars {
            self.bump();
        }
        self.push(kind, start);
    }

    fn redirect(&mut self, stream: Option<char>, start: usize) {
        if stream.is_some() {
            self.bump();
        }
        self.bump();
        let mut redirect = Redirect {
            stream,
            append: false,
            merge: false,
        };
        if self.peek() == Some('>') {
            self.bump();
            redirect.append = true;
        } else if self.peek() == Some('&') && self.peek_at(1) == Some('1') {
            self.bump();
            self.bump();
            redirect.merge = true;
        }
        self.push(TokenKind::Redirect(redirect), start);
    }

    fn type_name(&mut self, start: usize) -> Result<(), LexError> {
        let rest = self.input.get(self.pos + 1..).unwrap_or_default();
        let close = rest.find(']');
        let name = close.and_then(|end| rest.get(..end)).filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '[' | ','))
        });
        match name {
            Some(name) => {
                let name = name.to_owned();
                self.pos += 1 + name.len() + 1;
                self.push(TokenKind::TypeName(name), start);
                self.members();
                Ok(())
            }
            None => self.bare(start),
        }
    }

    fn dash(&mut self, start: usize) -> Result<(), LexError> {
        let next = self.peek_at(1);
        if next.is_some_and(|c| c.is_ascii_digit()) {
            return self.bare(start);
        }
        if !next.is_some_and(|c| c.is_alphabetic() || c == '_' || c == '?') {
            let long_option = next.is_some_and(|c| DASHES.contains(&c))
                && self.peek_at(2).is_some_and(char::is_alphanumeric);
            if long_option {
                return self.bare(start);
            }
            if next.is_some_and(|c| DASHES.contains(&c) || c == '=') {
                self.operator(start, 2);
            } else {
                self.operator(start, 1);
            }
            return Ok(());
        }
        self.bump();
        let name_start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?')
        {
            self.bump();
        }
        let name = self
            .input
            .get(name_start..self.pos)
            .unwrap_or_default()
            .to_owned();
        let colon = self.peek() == Some(':');
        if colon {
            self.bump();
        }
        self.push(TokenKind::Parameter { name, colon }, start);
        Ok(())
    }

    /// Reads a bare word; a character that ends a word without starting a
    /// token of its own is an error, so that every token consumes input.
    fn bare(&mut self, start: usize) -> Result<(), LexError> {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if ends_bare_word(c) || (c == '>' && !text.is_empty()) {
                break;
            }
            if c == '`' {
                self.bump();
                if let Some(escaped) = self.bump() {
                    text.push(escaped);
                }
                continue;
            }
            text.push(c);
            self.bump();
        }
        if self.pos == start {
            return Err(self.error(LexErrorKind::UnexpectedCharacter, start));
        }
        let kind = match text.as_str() {
            "=" => TokenKind::Assign,
            "+" | "+=" | "::" | ".." => TokenKind::Operator(text),
            _ => parse_number(&text).map_or(TokenKind::Word(text), TokenKind::Number),
        };
        self.push(kind, start);
        Ok(())
    }

    fn literal(&mut self, start: usize) -> Result<(), LexError> {
        self.bump();
        let mut text = String::new();
        loop {
            match self.bump() {
                Some(c) if SINGLE_QUOTES.contains(&c) => {
                    if self
                        .peek()
                        .is_some_and(|next| SINGLE_QUOTES.contains(&next))
                    {
                        self.bump();
                        text.push(c);
                    } else {
                        break;
                    }
                }
                Some(c) => text.push(c),
                None => return Err(self.error(LexErrorKind::UnterminatedString, start)),
            }
        }
        self.push(TokenKind::Literal(text), start);
        self.members();
        Ok(())
    }

    fn expandable(&mut self, start: usize) -> Result<(), LexError> {
        self.bump();
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error(LexErrorKind::UnterminatedString, start)),
                Some(c) if DOUBLE_QUOTES.contains(&c) => {
                    self.bump();
                    if self
                        .peek()
                        .is_some_and(|next| DOUBLE_QUOTES.contains(&next))
                    {
                        self.bump();
                        text.push(c);
                    } else {
                        break;
                    }
                }
                Some('`') => {
                    self.bump();
                    if let Some(escaped) = self.bump() {
                        text.push(backtick_escape(escaped));
                    }
                }
                Some('$') => self.string_dollar(&mut parts, &mut text)?,
                Some(c) => {
                    self.bump();
                    text.push(c);
                }
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(StrPart::Text(text));
        }
        self.push(TokenKind::Expandable(parts), start);
        self.members();
        Ok(())
    }

    fn here_string(&mut self, start: usize, expandable: bool) -> Result<(), LexError> {
        let close = if expandable { "\"@" } else { "'@" };
        self.bump();
        self.bump();
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.bump();
        }
        self.skip_line_break();
        let body_start = self.pos;
        let rest = self.input.get(body_start..).unwrap_or_default();
        let terminator = if rest.starts_with(close) {
            Some((0, 0))
        } else {
            ["\r\n", "\n"]
                .iter()
                .filter_map(|line_end| {
                    rest.find(&format!("{line_end}{close}"))
                        .map(|index| (index, line_end.len()))
                })
                .min()
        };
        let Some((body_len, line_end_len)) = terminator else {
            return Err(self.error(LexErrorKind::UnterminatedString, start));
        };
        let body = rest.get(..body_len).unwrap_or_default().to_owned();
        self.pos = body_start + body_len + line_end_len + close.len();
        let parts = if expandable && body.contains(['$', '`']) {
            expand_here_body(&body)
        } else {
            vec![StrPart::Text(body)]
        };
        let kind = if expandable {
            TokenKind::Expandable(parts)
        } else {
            TokenKind::Literal(body_text(parts))
        };
        self.push(kind, start);
        Ok(())
    }

    fn string_dollar(
        &mut self,
        parts: &mut Vec<StrPart>,
        text: &mut String,
    ) -> Result<(), LexError> {
        let start = self.pos;
        match self.peek_at(1) {
            Some('(') => {
                self.bump();
                self.bump();
                let inner_start = self.pos;
                let mut depth = 1_usize;
                while depth > 0 {
                    match self.bump() {
                        Some('(') => depth += 1,
                        Some(')') => depth -= 1,
                        Some(_) => {}
                        None => {
                            return Err(self.error(LexErrorKind::UnterminatedSubExpression, start));
                        }
                    }
                }
                let inner = self
                    .input
                    .get(inner_start..self.pos - 1)
                    .unwrap_or_default();
                flush_text(parts, text);
                parts.push(StrPart::SubExpr(inner.to_owned()));
            }
            Some(c) if starts_variable(c) => {
                self.bump();
                let var = self.variable_name(start)?;
                flush_text(parts, text);
                parts.push(StrPart::Variable(var));
            }
            _ => {
                self.bump();
                text.push('$');
            }
        }
        Ok(())
    }

    fn dollar(&mut self, start: usize) -> Result<(), LexError> {
        match self.peek_at(1) {
            Some('(') => self.operator_kind(TokenKind::SubExprOpen, start, 2),
            Some(c) if starts_variable(c) => {
                self.bump();
                let var = self.variable_name(start)?;
                self.push(TokenKind::Variable(var), start);
                self.members();
            }
            _ => return self.bare(start),
        }
        Ok(())
    }

    /// Reads a variable name after `$`.
    fn variable_name(&mut self, start: usize) -> Result<VarRef, LexError> {
        if self.peek() == Some('{') {
            self.bump();
            let name_start = self.pos;
            while self.peek().is_some_and(|c| c != '}') {
                self.bump();
            }
            if self.bump() != Some('}') {
                return Err(self.error(LexErrorKind::UnterminatedVariable, start));
            }
            let raw = self.input.get(name_start..self.pos - 1).unwrap_or_default();
            return Ok(split_scope(raw));
        }
        if let Some(c) = self.peek().filter(|c| matches!(c, '_' | '$' | '?' | '^'))
            && (c != '_' || !self.peek_at(1).is_some_and(is_name_char))
        {
            self.bump();
            return Ok(VarRef {
                scope: None,
                name: c.to_string(),
            });
        }
        let name_start = self.pos;
        while self.peek().is_some_and(is_name_char) {
            self.bump();
        }
        let mut scope = None;
        if self.peek() == Some(':') && self.peek_at(1).is_some_and(is_name_char) {
            scope = Some(
                self.input
                    .get(name_start..self.pos)
                    .unwrap_or_default()
                    .to_owned(),
            );
            self.bump();
            let inner_start = self.pos;
            while self.peek().is_some_and(is_name_char) {
                self.bump();
            }
            let name = self
                .input
                .get(inner_start..self.pos)
                .unwrap_or_default()
                .to_owned();
            return Ok(VarRef { scope, name });
        }
        let name = self
            .input
            .get(name_start..self.pos)
            .unwrap_or_default()
            .to_owned();
        Ok(VarRef {
            scope: scope.take(),
            name,
        })
    }

    /// Reads member accesses written directly after a value, `.Length`,
    /// `.AddDays`, and the static member operator `::`.
    fn members(&mut self) {
        loop {
            let start = self.pos;
            if self.peek() == Some(':') && self.peek_at(1) == Some(':') {
                self.operator(start, 2);
                continue;
            }
            if self.peek() != Some('.')
                || !self
                    .peek_at(1)
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                return;
            }
            self.bump();
            let name_start = self.pos;
            while self.peek().is_some_and(is_name_char) {
                self.bump();
            }
            let name = self
                .input
                .get(name_start..self.pos)
                .unwrap_or_default()
                .to_owned();
            self.push(TokenKind::Member(name), start);
        }
    }
}

fn body_text(parts: Vec<StrPart>) -> String {
    parts
        .into_iter()
        .filter_map(|part| match part {
            StrPart::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn flush_text(parts: &mut Vec<StrPart>, text: &mut String) {
    if !text.is_empty() {
        parts.push(StrPart::Text(std::mem::take(text)));
    }
}

fn expand_here_body(body: &str) -> Vec<StrPart> {
    let quoted = format!("\"{}\"", body.replace('"', "\"\""));
    match Lexer::new(&quoted).run() {
        Ok(tokens) => match tokens.into_iter().next().map(|token| token.kind) {
            Some(TokenKind::Expandable(parts)) => parts,
            _ => vec![StrPart::Text(body.to_owned())],
        },
        Err(_) => vec![StrPart::Text(body.to_owned())],
    }
}

fn split_scope(raw: &str) -> VarRef {
    match raw.split_once(':') {
        Some((scope, name)) if !scope.is_empty() => VarRef {
            scope: Some(scope.to_owned()),
            name: name.to_owned(),
        },
        _ => VarRef {
            scope: None,
            name: raw.to_owned(),
        },
    }
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_variable(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '{' | '$' | '?' | '^')
}

fn ends_bare_word(c: char) -> bool {
    c.is_whitespace()
        || matches!(c, '|' | ';' | '&' | '(' | ')' | '{' | '}' | ',')
        || SINGLE_QUOTES.contains(&c)
        || DOUBLE_QUOTES.contains(&c)
}

/// Resolves a backtick escape inside a double-quoted string.
fn backtick_escape(c: char) -> char {
    match c {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '0' => '\0',
        'a' => '\u{7}',
        'b' => '\u{8}',
        'e' => '\u{1B}',
        'f' => '\u{C}',
        'v' => '\u{B}',
        other => other,
    }
}

/// Parses a numeric literal with an optional size multiplier; `None` when
/// the word is not a number.
fn parse_number(word: &str) -> Option<NumberLit> {
    let (negative, body) = match word.strip_prefix(DASHES) {
        Some(rest) => (true, rest),
        None => (false, word),
    };
    let lower = body.to_ascii_lowercase();
    let (digits, exponent) = SIZE_SUFFIXES
        .iter()
        .zip(1_u32..)
        .find_map(|(suffix, exponent)| lower.strip_suffix(suffix).map(|rest| (rest, exponent)))
        .unwrap_or((lower.as_str(), 0));
    if digits.is_empty() {
        return None;
    }
    let sign = if negative { -1 } else { 1 };
    if let Some(hex) = digits.strip_prefix("0x") {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let int = i64::from_str_radix(hex, HEX_RADIX).ok()? * sign;
        return Some(NumberLit {
            int: Some(int),
            value: int as f64,
            exponent,
        });
    }
    if !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        || digits.matches('.').count() > 1
        || !digits.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        || digits.ends_with('.')
    {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    let int = if digits.contains('.') {
        None
    } else {
        Some(digits.parse::<i64>().ok()? * sign)
    };
    Some(NumberLit {
        int,
        value: if negative { -value } else { value },
        exponent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<TokenKind> {
        lex(input)
            .map(|tokens| tokens.into_iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    }

    fn word(text: &str) -> TokenKind {
        TokenKind::Word(text.to_owned())
    }

    fn param(name: &str) -> TokenKind {
        TokenKind::Parameter {
            name: name.to_owned(),
            colon: false,
        }
    }

    #[test]
    fn splits_command_with_parameters_and_bare_path() {
        assert_eq!(
            kinds(r"gci -Path C:\proj -Recurse"),
            vec![
                word("gci"),
                param("Path"),
                word(r"C:\proj"),
                param("Recurse")
            ]
        );
    }

    #[test]
    fn single_quoted_string_keeps_bytes_and_doubles_quote() {
        assert_eq!(
            kinds("'it''s a\\b'"),
            vec![TokenKind::Literal("it's a\\b".to_owned())]
        );
    }

    #[test]
    fn double_quoted_string_resolves_backtick_escapes() {
        assert_eq!(
            kinds("\"a`tb`n`\"c\"\"d\""),
            vec![TokenKind::Expandable(vec![StrPart::Text(
                "a\tb\n\"c\"d".to_owned()
            )])]
        );
    }

    #[test]
    fn double_quoted_string_expands_variables_and_subexpressions() {
        let parts = match kinds("\"$env:BUILD\\* $x $(Get-Date)\"").into_iter().next() {
            Some(TokenKind::Expandable(parts)) => parts,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            parts,
            vec![
                StrPart::Variable(VarRef {
                    scope: Some("env".to_owned()),
                    name: "BUILD".to_owned()
                }),
                StrPart::Text("\\* ".to_owned()),
                StrPart::Variable(VarRef {
                    scope: None,
                    name: "x".to_owned()
                }),
                StrPart::Text(" ".to_owned()),
                StrPart::SubExpr("Get-Date".to_owned()),
            ]
        );
    }

    #[test]
    fn lone_dollar_in_string_is_text() {
        assert_eq!(
            kinds("\"5 $ total\""),
            vec![TokenKind::Expandable(vec![StrPart::Text(
                "5 $ total".to_owned()
            )])]
        );
    }

    #[test]
    fn parameter_with_colon_value() {
        assert_eq!(
            kinds("-Confirm:$false"),
            vec![
                TokenKind::Parameter {
                    name: "Confirm".to_owned(),
                    colon: true
                },
                TokenKind::Variable(VarRef {
                    scope: None,
                    name: "false".to_owned()
                }),
            ]
        );
    }

    #[test]
    fn typographic_dash_starts_parameter() {
        assert_eq!(kinds("\u{2013}Recurse"), vec![param("Recurse")]);
    }

    #[test]
    fn typographic_quotes_delimit_strings() {
        assert_eq!(
            kinds("\u{2018}a\u{2019} \u{201C}b\u{201D}"),
            vec![
                TokenKind::Literal("a".to_owned()),
                TokenKind::Expandable(vec![StrPart::Text("b".to_owned())])
            ]
        );
    }

    #[test]
    fn size_suffixes_are_binary_multipliers() {
        let tokens = kinds("1MB 1.5gb 10kb 2PB");
        let bytes: Vec<Option<u64>> = tokens
            .iter()
            .map(|kind| match kind {
                TokenKind::Number(number) => number.bytes(),
                _ => None,
            })
            .collect();
        assert_eq!(
            bytes,
            vec![
                Some(1_048_576),
                Some(1_610_612_736),
                Some(10_240),
                Some(2_251_799_813_685_248)
            ]
        );
    }

    #[test]
    fn plain_and_negative_numbers() {
        assert_eq!(
            kinds("5 -1 0x10 2.5"),
            vec![
                TokenKind::Number(NumberLit {
                    int: Some(5),
                    value: 5.0,
                    exponent: 0
                }),
                TokenKind::Number(NumberLit {
                    int: Some(-1),
                    value: -1.0,
                    exponent: 0
                }),
                TokenKind::Number(NumberLit {
                    int: Some(16),
                    value: 16.0,
                    exponent: 0
                }),
                TokenKind::Number(NumberLit {
                    int: None,
                    value: 2.5,
                    exponent: 0
                }),
            ]
        );
    }

    #[test]
    fn words_that_look_numeric_stay_words() {
        assert_eq!(
            kinds("7zip 1.2.3 1MBx"),
            vec![word("7zip"), word("1.2.3"), word("1MBx")]
        );
    }

    #[test]
    fn pipeline_variable_with_members() {
        assert_eq!(
            kinds("$_.Length.Value"),
            vec![
                TokenKind::Variable(VarRef {
                    scope: None,
                    name: "_".to_owned()
                }),
                TokenKind::Member("Length".to_owned()),
                TokenKind::Member("Value".to_owned()),
            ]
        );
    }

    #[test]
    fn braced_variable_and_scoped_variable() {
        assert_eq!(
            kinds("${my var} $env:Path"),
            vec![
                TokenKind::Variable(VarRef {
                    scope: None,
                    name: "my var".to_owned()
                }),
                TokenKind::Variable(VarRef {
                    scope: Some("env".to_owned()),
                    name: "Path".to_owned()
                }),
            ]
        );
    }

    #[test]
    fn unterminated_braced_variable_is_an_error() {
        let error = lex("${abc").err();
        assert_eq!(
            error.map(|e| e.kind),
            Some(LexErrorKind::UnterminatedVariable)
        );
    }

    #[test]
    fn blocks_subexpressions_arrays_and_hash_tables() {
        assert_eq!(
            kinds("{ } $( ) @( ) @{ a = 1 }"),
            vec![
                TokenKind::OpenBrace,
                TokenKind::CloseBrace,
                TokenKind::SubExprOpen,
                TokenKind::CloseParen,
                TokenKind::ArrayOpen,
                TokenKind::CloseParen,
                TokenKind::HashOpen,
                word("a"),
                TokenKind::Assign,
                TokenKind::Number(NumberLit {
                    int: Some(1),
                    value: 1.0,
                    exponent: 0
                }),
                TokenKind::CloseBrace,
            ]
        );
    }

    #[test]
    fn paren_followed_by_method_member() {
        assert_eq!(
            kinds("(Get-Date).AddDays(-1)"),
            vec![
                TokenKind::OpenParen,
                word("Get-Date"),
                TokenKind::CloseParen,
                TokenKind::Member("AddDays".to_owned()),
                TokenKind::OpenParen,
                TokenKind::Number(NumberLit {
                    int: Some(-1),
                    value: -1.0,
                    exponent: 0
                }),
                TokenKind::CloseParen,
            ]
        );
    }

    #[test]
    fn separators_redirections_and_call_operator() {
        assert_eq!(
            kinds("& a | b; c > f >> g 2>&1\n"),
            vec![
                TokenKind::Ampersand,
                word("a"),
                TokenKind::Pipe,
                word("b"),
                TokenKind::Semicolon,
                word("c"),
                TokenKind::Redirect(Redirect {
                    stream: None,
                    append: false,
                    merge: false
                }),
                word("f"),
                TokenKind::Redirect(Redirect {
                    stream: None,
                    append: true,
                    merge: false
                }),
                word("g"),
                TokenKind::Redirect(Redirect {
                    stream: Some('2'),
                    append: false,
                    merge: true
                }),
                TokenKind::Newline,
            ]
        );
    }

    #[test]
    fn comments_and_line_continuations_are_dropped() {
        assert_eq!(
            kinds("a `\n b # note\n<# block\n #> c"),
            vec![word("a"), word("b"), TokenKind::Newline, word("c")]
        );
    }

    #[test]
    fn unterminated_strings_and_comments_are_errors() {
        assert_eq!(
            lex("'abc").err().map(|e| e.kind),
            Some(LexErrorKind::UnterminatedString)
        );
        assert_eq!(
            lex("\"abc").err().map(|e| e.kind),
            Some(LexErrorKind::UnterminatedString)
        );
        assert_eq!(
            lex("<# abc").err().map(|e| e.kind),
            Some(LexErrorKind::UnterminatedComment)
        );
        assert_eq!(
            lex("\"$(abc\"").err().map(|e| e.kind),
            Some(LexErrorKind::UnterminatedSubExpression)
        );
    }

    #[test]
    fn here_strings_keep_their_body() {
        assert_eq!(
            kinds("@'\nline 'one'\n'@"),
            vec![TokenKind::Literal("line 'one'".to_owned())]
        );
        assert_eq!(
            kinds("@\"\na `$b\n\"@"),
            vec![TokenKind::Expandable(vec![StrPart::Text(
                "a $b".to_owned()
            )])]
        );
    }

    #[test]
    fn empty_here_strings_have_an_empty_body() {
        assert_eq!(kinds("@'\n'@"), vec![TokenKind::Literal(String::new())]);
        assert_eq!(
            kinds("@\"\r\n\"@ x"),
            vec![
                TokenKind::Expandable(vec![StrPart::Text(String::new())]),
                word("x")
            ]
        );
    }

    #[test]
    fn hexadecimal_words_with_a_sign_or_other_characters_stay_words() {
        assert_eq!(
            kinds("0x-5 0x+5 0xZZ 0x"),
            vec![word("0x-5"), word("0x+5"), word("0xZZ"), word("0x")]
        );
    }

    #[test]
    fn type_literal_and_static_member() {
        assert_eq!(
            kinds("[Environment]::GetEnvironmentVariable"),
            vec![
                TokenKind::TypeName("Environment".to_owned()),
                TokenKind::Operator("::".to_owned()),
                word("GetEnvironmentVariable"),
            ]
        );
    }

    #[test]
    fn spans_count_unicode_scalar_values() {
        let tokens = lex("'Zürich' -Path\nx").unwrap_or_default();
        let spans: Vec<[u32; 4]> = tokens.iter().map(|token| token.span.into()).collect();
        assert_eq!(
            spans,
            vec![[1, 1, 1, 9], [1, 10, 1, 15], [1, 15, 2, 1], [2, 1, 2, 2]]
        );
    }

    #[test]
    fn standalone_assign_and_operators() {
        assert_eq!(
            kinds("$x = a + b"),
            vec![
                TokenKind::Variable(VarRef {
                    scope: None,
                    name: "x".to_owned()
                }),
                TokenKind::Assign,
                word("a"),
                TokenKind::Operator("+".to_owned()),
                word("b"),
            ]
        );
    }
}
