// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! POSIX shell lexer for commands written in bash notation.
//!
//! The lexer reads one command line or a short script with the quoting rules
//! of the POSIX shell: `'…'` is literal, `"…"` processes the escapes `\$`,
//! `` \` ``, `\"`, `\\` and a line continuation and expands `$`, and `\`
//! outside quotes escapes the next character. Word bytes are kept exactly as
//! written inside quotes; every token carries its position in the input.

use std::fmt;

use nurt_contracts::ast::Span;

/// Token of a command in POSIX shell notation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosixToken {
    /// Kind and content of the token.
    pub kind: PosixTokenKind,
    /// Position in the input; columns count Unicode scalar values from 1 and
    /// the end is exclusive.
    pub span: Span,
}

/// Kind of a [`PosixToken`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PosixTokenKind {
    /// A word: a command name, an argument or an assignment.
    Word(Word),
    /// A control operator.
    Operator(Operator),
    /// A redirection operator with its optional file descriptor.
    Redirect(Redirect),
    /// End of a line outside quotes.
    Newline,
    /// A comment, without the leading `#`.
    Comment(String),
}

/// Control operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Operator {
    /// `&&`.
    And,
    /// `||`.
    Or,
    /// `;`.
    Semicolon,
    /// `;;`, the end of a `case` branch.
    CaseEnd,
    /// `|`.
    Pipe,
    /// `|&`, a pipe that also carries standard error.
    PipeAll,
    /// `&`.
    Background,
    /// `(`.
    OpenParen,
    /// `)`.
    CloseParen,
}

impl Operator {
    /// Returns the operator as written.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::And => "&&",
            Self::Or => "||",
            Self::Semicolon => ";",
            Self::CaseEnd => ";;",
            Self::Pipe => "|",
            Self::PipeAll => "|&",
            Self::Background => "&",
            Self::OpenParen => "(",
            Self::CloseParen => ")",
        }
    }
}

/// Redirection operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// File descriptor written before the operator (`2` in `2>`).
    pub fd: Option<u32>,
    /// The operator.
    pub op: RedirectOp,
    /// Body of a here-document; present for `<<` and `<<-`.
    pub here_doc: Option<HereDoc>,
}

/// Kind of a redirection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedirectOp {
    /// `>`.
    Out,
    /// `>>`.
    Append,
    /// `>|`.
    Clobber,
    /// `<`.
    In,
    /// `<>`.
    ReadWrite,
    /// `<<`.
    HereDoc,
    /// `<<-`, a here-document with leading tabs removed.
    HereDocStrip,
    /// `<<<`.
    HereString,
    /// `>&`.
    DupOut,
    /// `<&`.
    DupIn,
    /// `&>`.
    OutErr,
    /// `&>>`.
    AppendOutErr,
}

impl RedirectOp {
    /// Returns the operator as written.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Out => ">",
            Self::Append => ">>",
            Self::Clobber => ">|",
            Self::In => "<",
            Self::ReadWrite => "<>",
            Self::HereDoc => "<<",
            Self::HereDocStrip => "<<-",
            Self::HereString => "<<<",
            Self::DupOut => ">&",
            Self::DupIn => "<&",
            Self::OutErr => "&>",
            Self::AppendOutErr => "&>>",
        }
    }
}

/// Body of a here-document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HereDoc {
    /// Delimiter word after quote removal.
    pub delimiter: String,
    /// Whether the delimiter was quoted; the body is then not expanded.
    pub quoted: bool,
    /// Body lines, each followed by a line feed.
    pub body: String,
    /// Position of the body.
    pub span: Span,
}

/// A shell word split into literal parts and expansions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The word as written.
    pub raw: String,
    /// Parts in order.
    pub parts: Vec<WordPart>,
}

/// Part of a [`Word`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WordPart {
    /// Literal text after quote removal.
    Literal {
        /// The text.
        text: String,
        /// How the text was quoted.
        quoting: Quoting,
    },
    /// Parameter expansion, `$NAME` or `${NAME}`.
    Variable {
        /// Name, or the whole content of `${…}` when it is not a plain name.
        name: String,
        /// Whether braces were used.
        braced: bool,
        /// Whether the expansion is inside double quotes.
        quoted: bool,
    },
    /// Command substitution `$(…)` or `` `…` ``, or arithmetic `$((…))`.
    Substitution {
        /// Text between the delimiters.
        body: String,
        /// Whether the backtick form was used.
        backtick: bool,
        /// Whether the substitution is arithmetic.
        arithmetic: bool,
        /// Whether the substitution is inside double quotes.
        quoted: bool,
    },
}

/// Quoting of a literal part of a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Quoting {
    /// Unquoted: pattern characters keep their meaning.
    None,
    /// Inside `'…'`.
    Single,
    /// Inside `"…"`.
    Double,
    /// A single character escaped with `\`.
    Escaped,
}

impl Word {
    /// Returns the word after quote removal when it contains no expansion.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_correct::bash::{PosixTokenKind, lex};
    ///
    /// let tokens = lex("rm 'my file.txt'")?;
    /// let PosixTokenKind::Word(word) = &tokens[1].kind else { unreachable!() };
    /// assert_eq!(word.literal().as_deref(), Some("my file.txt"));
    /// # Ok::<(), nurt_correct::bash::LexError>(())
    /// ```
    #[must_use]
    pub fn literal(&self) -> Option<String> {
        let mut text = String::new();
        for part in &self.parts {
            match part {
                WordPart::Literal { text: piece, .. } => text.push_str(piece),
                WordPart::Variable { .. } | WordPart::Substitution { .. } => return None,
            }
        }
        Some(text)
    }

    /// Whether the word contains a parameter expansion or a substitution.
    #[must_use]
    pub fn has_expansion(&self) -> bool {
        self.parts
            .iter()
            .any(|part| !matches!(part, WordPart::Literal { .. }))
    }

    /// Whether an unquoted part contains a pattern character `*`, `?` or `[`.
    #[must_use]
    pub fn has_unquoted_glob(&self) -> bool {
        self.parts.iter().any(|part| match part {
            WordPart::Literal {
                text,
                quoting: Quoting::None,
            } => text.contains(['*', '?', '[']),
            _ => false,
        })
    }

    /// Whether every part of the word is unquoted literal text.
    #[must_use]
    pub fn is_bare(&self) -> bool {
        self.parts.iter().all(|part| {
            matches!(
                part,
                WordPart::Literal {
                    quoting: Quoting::None,
                    ..
                }
            )
        })
    }
}

/// Error of the POSIX lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    /// What is wrong.
    pub message: String,
    /// Line of the error, from 1.
    pub line: u32,
    /// Column of the error, from 1, in Unicode scalar values.
    pub column: u32,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for LexError {}

/// Splits a command in POSIX shell notation into tokens.
///
/// # Errors
///
/// Returns [`LexError`] for an unterminated quote, substitution or
/// here-document.
///
/// # Examples
///
/// ```
/// use nurt_correct::bash::{Operator, PosixTokenKind, lex};
///
/// let tokens = lex("cd /srv/app && go test ./...")?;
/// assert_eq!(tokens.len(), 6);
/// assert_eq!(tokens[2].kind, PosixTokenKind::Operator(Operator::And));
/// assert_eq!(tokens[2].span.column, 13);
/// # Ok::<(), nurt_correct::bash::LexError>(())
/// ```
pub fn lex(input: &str) -> Result<Vec<PosixToken>, LexError> {
    Lexer::new(input).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    column: u32,
    tokens: Vec<PosixToken>,
    pending_here_docs: Vec<usize>,
}

#[derive(Clone, Copy)]
struct Mark {
    line: u32,
    column: u32,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
            tokens: Vec::new(),
            pending_here_docs: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Vec<PosixToken>, LexError> {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\\' if self.peek(1) == Some('\n') => {
                    self.bump();
                    self.bump();
                }
                '\n' => self.newline()?,
                '#' => self.comment(),
                '&' | '|' | ';' | '(' | ')' | '<' | '>' => self.operator(None)?,
                _ => self.word()?,
            }
        }
        if let Some(&index) = self.pending_here_docs.first() {
            let span = self.tokens[index].span;
            return Err(error_at(
                span.line,
                span.column,
                "here-document without a body",
            ));
        }
        Ok(self.tokens)
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn mark(&self) -> Mark {
        Mark {
            line: self.line,
            column: self.column,
        }
    }

    fn span_from(&self, start: Mark) -> Span {
        Span {
            line: start.line,
            column: start.column,
            end_line: self.line,
            end_column: self.column,
        }
    }

    fn push(&mut self, kind: PosixTokenKind, start: Mark) {
        let span = self.span_from(start);
        self.tokens.push(PosixToken { kind, span });
    }

    fn newline(&mut self) -> Result<(), LexError> {
        let start = self.mark();
        self.bump();
        self.push(PosixTokenKind::Newline, start);
        let pending = std::mem::take(&mut self.pending_here_docs);
        for index in pending {
            self.here_doc_body(index)?;
        }
        Ok(())
    }

    fn comment(&mut self) {
        let start = self.mark();
        self.bump();
        let mut text = String::new();
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                break;
            }
            text.push(c);
            self.bump();
        }
        self.push(PosixTokenKind::Comment(text), start);
    }

    fn operator(&mut self, fd: Option<(u32, Mark)>) -> Result<(), LexError> {
        let start = fd.map_or_else(|| self.mark(), |(_, mark)| mark);
        let fd = fd.map(|(number, _)| number);
        let three: String = (0..3).filter_map(|offset| self.peek(offset)).collect();
        let (text, kind) = classify_operator(&three, fd);
        for _ in 0..text.chars().count() {
            self.bump();
        }
        let is_here_doc = matches!(
            kind,
            PosixTokenKind::Redirect(Redirect {
                op: RedirectOp::HereDoc | RedirectOp::HereDocStrip,
                ..
            })
        );
        self.push(kind, start);
        if is_here_doc {
            self.here_doc_delimiter()?;
        }
        Ok(())
    }

    fn here_doc_delimiter(&mut self) -> Result<(), LexError> {
        while matches!(self.peek(0), Some(' ' | '\t')) {
            self.bump();
        }
        let operator_index = self.tokens.len() - 1;
        let start = self.mark();
        let word = self.read_word(start)?;
        let quoted = word.parts.iter().any(|part| {
            !matches!(
                part,
                WordPart::Literal {
                    quoting: Quoting::None,
                    ..
                }
            )
        });
        let delimiter = word.literal().unwrap_or_else(|| word.raw.clone());
        if delimiter.is_empty() {
            return Err(error_at(
                start.line,
                start.column,
                "here-document without a delimiter",
            ));
        }
        let span = self.span_from(start);
        if let PosixTokenKind::Redirect(redirect) = &mut self.tokens[operator_index].kind {
            redirect.here_doc = Some(HereDoc {
                delimiter,
                quoted,
                body: String::new(),
                span,
            });
        }
        self.pending_here_docs.push(operator_index);
        Ok(())
    }

    fn here_doc_body(&mut self, index: usize) -> Result<(), LexError> {
        let PosixTokenKind::Redirect(redirect) = &self.tokens[index].kind else {
            return Ok(());
        };
        let strip = redirect.op == RedirectOp::HereDocStrip;
        let delimiter = redirect
            .here_doc
            .as_ref()
            .map(|doc| doc.delimiter.clone())
            .unwrap_or_default();
        let start = self.mark();
        let mut body = String::new();
        loop {
            if self.peek(0).is_none() {
                let span = self.tokens[index].span;
                return Err(error_at(
                    span.line,
                    span.column,
                    "unterminated here-document",
                ));
            }
            let line = self.read_line();
            let content = if strip {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            };
            if content == delimiter {
                break;
            }
            body.push_str(content);
            body.push('\n');
        }
        let span = self.span_from(start);
        if let PosixTokenKind::Redirect(redirect) = &mut self.tokens[index].kind
            && let Some(doc) = &mut redirect.here_doc
        {
            doc.body = body;
            doc.span = span;
        }
        Ok(())
    }

    fn read_line(&mut self) -> String {
        let mut line = String::new();
        while let Some(c) = self.bump() {
            if c == '\n' {
                break;
            }
            line.push(c);
        }
        line.strip_suffix('\r').map(str::to_owned).unwrap_or(line)
    }

    fn word(&mut self) -> Result<(), LexError> {
        let start = self.mark();
        let word = self.read_word(start)?;
        let io_number = matches!(self.peek(0), Some('<' | '>'))
            && !word.raw.is_empty()
            && word.raw.bytes().all(|b| b.is_ascii_digit());
        if io_number && let Ok(fd) = word.raw.parse::<u32>() {
            return self.operator(Some((fd, start)));
        }
        self.push(PosixTokenKind::Word(word), start);
        Ok(())
    }

    fn read_word(&mut self, start: Mark) -> Result<Word, LexError> {
        let mut builder = WordBuilder::default();
        let begin = self.pos;
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' | '\n' | '\r' | '&' | '|' | ';' | '(' | ')' | '<' | '>' => break,
                '\'' => self.single_quoted(&mut builder)?,
                '"' => self.double_quoted(&mut builder)?,
                '\\' => self.escaped(&mut builder),
                '$' => self.dollar(&mut builder, false)?,
                '`' => self.backtick(&mut builder, false)?,
                _ => {
                    self.bump();
                    builder.literal(c, Quoting::None);
                }
            }
        }
        if self.pos == begin {
            return Err(error_at(start.line, start.column, "expected a word"));
        }
        let raw: String = self.chars[begin..self.pos].iter().collect();
        Ok(builder.finish(raw))
    }

    fn single_quoted(&mut self, builder: &mut WordBuilder) -> Result<(), LexError> {
        let start = self.mark();
        self.bump();
        builder.open(Quoting::Single);
        loop {
            match self.bump() {
                None => return Err(error_at(start.line, start.column, "unterminated `'`")),
                Some('\'') => return Ok(()),
                Some(c) => builder.literal(c, Quoting::Single),
            }
        }
    }

    fn double_quoted(&mut self, builder: &mut WordBuilder) -> Result<(), LexError> {
        let start = self.mark();
        self.bump();
        builder.open(Quoting::Double);
        loop {
            match self.peek(0) {
                None => return Err(error_at(start.line, start.column, "unterminated `\"`")),
                Some('"') => {
                    self.bump();
                    return Ok(());
                }
                Some('\\') => self.double_quoted_escape(builder),
                Some('$') => self.dollar(builder, true)?,
                Some('`') => self.backtick(builder, true)?,
                Some(c) => {
                    self.bump();
                    builder.literal(c, Quoting::Double);
                }
            }
        }
    }

    fn double_quoted_escape(&mut self, builder: &mut WordBuilder) {
        self.bump();
        match self.peek(0) {
            Some('\n') => {
                self.bump();
            }
            Some(c @ ('$' | '`' | '"' | '\\')) => {
                self.bump();
                builder.literal(c, Quoting::Double);
            }
            _ => builder.literal('\\', Quoting::Double),
        }
    }

    fn escaped(&mut self, builder: &mut WordBuilder) {
        self.bump();
        match self.bump() {
            Some('\n') | None => {}
            Some(c) => builder.literal(c, Quoting::Escaped),
        }
    }

    fn dollar(&mut self, builder: &mut WordBuilder, quoted: bool) -> Result<(), LexError> {
        let start = self.mark();
        match self.peek(1) {
            Some('(') if self.peek(2) == Some('(') => {
                self.bump();
                let body = self.balanced(start, '(', ')')?;
                let inner = body
                    .strip_prefix('(')
                    .and_then(|rest| rest.strip_suffix(')'))
                    .unwrap_or(&body)
                    .to_owned();
                builder.part(WordPart::Substitution {
                    body: inner,
                    backtick: false,
                    arithmetic: true,
                    quoted,
                });
            }
            Some('(') => {
                self.bump();
                let body = self.balanced(start, '(', ')')?;
                builder.part(WordPart::Substitution {
                    body,
                    backtick: false,
                    arithmetic: false,
                    quoted,
                });
            }
            Some('{') => {
                self.bump();
                let name = self.balanced(start, '{', '}')?;
                builder.part(WordPart::Variable {
                    name,
                    braced: true,
                    quoted,
                });
            }
            Some(c) if c == '_' || c.is_ascii_alphabetic() => {
                self.bump();
                let mut name = String::new();
                while let Some(c) = self.peek(0) {
                    if c == '_' || c.is_ascii_alphanumeric() {
                        name.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                builder.part(WordPart::Variable {
                    name,
                    braced: false,
                    quoted,
                });
            }
            Some(c) if c.is_ascii_digit() || "@*#?$!-".contains(c) => {
                self.bump();
                self.bump();
                builder.part(WordPart::Variable {
                    name: c.to_string(),
                    braced: false,
                    quoted,
                });
            }
            _ => {
                self.bump();
                let quoting = if quoted {
                    Quoting::Double
                } else {
                    Quoting::None
                };
                builder.literal('$', quoting);
            }
        }
        Ok(())
    }

    /// Reads from an opening delimiter to the matching closing one and
    /// returns the text between them; quotes inside are skipped as units.
    fn balanced(&mut self, start: Mark, open: char, close: char) -> Result<String, LexError> {
        self.bump();
        let mut depth = 1_usize;
        let mut text = String::new();
        let mut quote: Option<char> = None;
        loop {
            let Some(c) = self.bump() else {
                return Err(error_at(
                    start.line,
                    start.column,
                    "unterminated substitution",
                ));
            };
            match (quote, c) {
                (Some(q), _) if c == q => quote = None,
                (Some(_), '\\') | (None, '\\') => {
                    text.push(c);
                    if let Some(next) = self.bump() {
                        text.push(next);
                    }
                    continue;
                }
                (Some(_), _) => {}
                (None, '\'' | '"') => quote = Some(c),
                (None, _) if c == open => depth += 1,
                (None, _) if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(text);
                    }
                }
                (None, _) => {}
            }
            text.push(c);
        }
    }

    fn backtick(&mut self, builder: &mut WordBuilder, quoted: bool) -> Result<(), LexError> {
        let start = self.mark();
        self.bump();
        let mut body = String::new();
        loop {
            match self.bump() {
                None => return Err(error_at(start.line, start.column, "unterminated `` ` ``")),
                Some('`') => break,
                Some('\\') => match self.bump() {
                    Some(c @ ('`' | '\\' | '$')) => body.push(c),
                    Some(c) => {
                        body.push('\\');
                        body.push(c);
                    }
                    None => body.push('\\'),
                },
                Some(c) => body.push(c),
            }
        }
        builder.part(WordPart::Substitution {
            body,
            backtick: true,
            arithmetic: false,
            quoted,
        });
        Ok(())
    }
}

fn classify_operator(next: &str, fd: Option<u32>) -> (&'static str, PosixTokenKind) {
    const TABLE: &[(&str, Option<Operator>, Option<RedirectOp>)] = &[
        ("&>>", None, Some(RedirectOp::AppendOutErr)),
        ("<<<", None, Some(RedirectOp::HereString)),
        ("<<-", None, Some(RedirectOp::HereDocStrip)),
        ("&&", Some(Operator::And), None),
        ("||", Some(Operator::Or), None),
        (";;", Some(Operator::CaseEnd), None),
        ("|&", Some(Operator::PipeAll), None),
        ("&>", None, Some(RedirectOp::OutErr)),
        (">>", None, Some(RedirectOp::Append)),
        (">|", None, Some(RedirectOp::Clobber)),
        (">&", None, Some(RedirectOp::DupOut)),
        ("<<", None, Some(RedirectOp::HereDoc)),
        ("<>", None, Some(RedirectOp::ReadWrite)),
        ("<&", None, Some(RedirectOp::DupIn)),
        ("&", Some(Operator::Background), None),
        ("|", Some(Operator::Pipe), None),
        (";", Some(Operator::Semicolon), None),
        ("(", Some(Operator::OpenParen), None),
        (")", Some(Operator::CloseParen), None),
        (">", None, Some(RedirectOp::Out)),
        ("<", None, Some(RedirectOp::In)),
    ];
    let allows_fd = |op: RedirectOp| !matches!(op, RedirectOp::OutErr | RedirectOp::AppendOutErr);
    for &(text, operator, redirect) in TABLE {
        if !next.starts_with(text) {
            continue;
        }
        match (operator, redirect) {
            (Some(operator), _) if fd.is_none() => {
                return (text, PosixTokenKind::Operator(operator));
            }
            (_, Some(op)) if fd.is_none() || allows_fd(op) => {
                let redirect = Redirect {
                    fd,
                    op,
                    here_doc: None,
                };
                return (text, PosixTokenKind::Redirect(redirect));
            }
            _ => {}
        }
    }
    let redirect = Redirect {
        fd,
        op: RedirectOp::Out,
        here_doc: None,
    };
    (">", PosixTokenKind::Redirect(redirect))
}

fn error_at(line: u32, column: u32, message: &str) -> LexError {
    LexError {
        message: message.to_owned(),
        line,
        column,
    }
}

#[derive(Default)]
struct WordBuilder {
    parts: Vec<WordPart>,
}

impl WordBuilder {
    /// Starts a quoted literal so that an empty pair of quotes still yields
    /// an (empty) literal part.
    fn open(&mut self, quoting: Quoting) {
        self.parts.push(WordPart::Literal {
            text: String::new(),
            quoting,
        });
    }

    fn literal(&mut self, c: char, quoting: Quoting) {
        if let Some(WordPart::Literal {
            text,
            quoting: last,
        }) = self.parts.last_mut()
            && *last == quoting
            && quoting != Quoting::Escaped
        {
            text.push(c);
            return;
        }
        self.parts.push(WordPart::Literal {
            text: c.to_string(),
            quoting,
        });
    }

    fn part(&mut self, part: WordPart) {
        self.parts.push(part);
    }

    fn finish(self, raw: String) -> Word {
        Word {
            raw,
            parts: self.parts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(input: &str) -> Vec<Word> {
        lex(input)
            .unwrap()
            .into_iter()
            .filter_map(|token| match token.kind {
                PosixTokenKind::Word(word) => Some(word),
                _ => None,
            })
            .collect()
    }

    fn kinds(input: &str) -> Vec<PosixTokenKind> {
        lex(input)
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn single_quotes_keep_bytes() {
        let word = &words(r#"grep 'a\b"$x' f"#)[1];
        assert_eq!(word.literal().as_deref(), Some(r#"a\b"$x"#));
        assert!(!word.has_expansion());
    }

    #[test]
    fn double_quotes_process_posix_escapes_only() {
        let word = &words(r#"echo "a\$b\"c\\d\e""#)[1];
        assert_eq!(word.literal().as_deref(), Some(r#"a$b"c\d\e"#));
    }

    #[test]
    fn double_quotes_expand_variables() {
        let word = &words(r#"rm "$BUILD/x""#)[1];
        assert!(word.has_expansion());
        assert_eq!(
            word.parts[1],
            WordPart::Variable {
                name: "BUILD".to_owned(),
                braced: false,
                quoted: true
            }
        );
    }

    #[test]
    fn backslash_outside_quotes_escapes_one_character() {
        let word = &words(r"rm my\ file\*")[1];
        assert_eq!(word.literal().as_deref(), Some("my file*"));
        assert!(!word.has_unquoted_glob());
    }

    #[test]
    fn unquoted_glob_is_detected() {
        assert!(words("rm /a/*.tmp")[1].has_unquoted_glob());
        assert!(!words("rm '/a/*.tmp'")[1].has_unquoted_glob());
    }

    #[test]
    fn line_continuation_joins_words() {
        let all = words("rm \\\n/a");
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].literal().as_deref(), Some("/a"));
    }

    #[test]
    fn control_operators_are_recognised() {
        let operators: Vec<Operator> = lex("a && b || c ; d | e & f |& g ;;")
            .unwrap()
            .into_iter()
            .filter_map(|token| match token.kind {
                PosixTokenKind::Operator(op) => Some(op),
                _ => None,
            })
            .collect();
        assert_eq!(
            operators,
            vec![
                Operator::And,
                Operator::Or,
                Operator::Semicolon,
                Operator::Pipe,
                Operator::Background,
                Operator::PipeAll,
                Operator::CaseEnd
            ]
        );
    }

    #[test]
    fn redirections_with_descriptors() {
        let redirects: Vec<(Option<u32>, RedirectOp)> = lex("cmd > a >> b 2> c 2>&1 < d &> e")
            .unwrap()
            .into_iter()
            .filter_map(|token| match token.kind {
                PosixTokenKind::Redirect(r) => Some((r.fd, r.op)),
                _ => None,
            })
            .collect();
        assert_eq!(
            redirects,
            vec![
                (None, RedirectOp::Out),
                (None, RedirectOp::Append),
                (Some(2), RedirectOp::Out),
                (Some(2), RedirectOp::DupOut),
                (None, RedirectOp::In),
                (None, RedirectOp::OutErr),
            ]
        );
    }

    #[test]
    fn digits_separated_from_operator_stay_a_word() {
        let all = words("head -n 2 > f");
        assert_eq!(all[2].raw, "2");
    }

    #[test]
    fn substitutions_are_parts() {
        let word = &words("rm $(find /a -name x) `ls`")[1];
        assert_eq!(
            word.parts[0],
            WordPart::Substitution {
                body: "find /a -name x".to_owned(),
                backtick: false,
                arithmetic: false,
                quoted: false
            }
        );
        let tick = &words("rm `ls /a`")[1];
        assert!(matches!(
            &tick.parts[0],
            WordPart::Substitution { backtick: true, .. }
        ));
    }

    #[test]
    fn arithmetic_and_braced_expansions() {
        let word = &words("echo $((1+2))${HOME}$1")[1];
        assert!(
            matches!(&word.parts[0], WordPart::Substitution { arithmetic: true, body, .. } if body == "1+2")
        );
        assert!(
            matches!(&word.parts[1], WordPart::Variable { braced: true, name, .. } if name == "HOME")
        );
        assert!(matches!(&word.parts[2], WordPart::Variable { name, .. } if name == "1"));
    }

    #[test]
    fn lone_dollar_is_literal() {
        assert_eq!(words("echo a$")[1].literal().as_deref(), Some("a$"));
    }

    #[test]
    fn comments_run_to_end_of_line() {
        let all = kinds("ls /a # list it\nls /b");
        assert!(matches!(&all[2], PosixTokenKind::Comment(text) if text == " list it"));
        assert_eq!(all[3], PosixTokenKind::Newline);
    }

    #[test]
    fn here_document_body_is_attached() {
        let tokens = lex("cat > /a/f <<'EOF'\nline \"1\"\n$x\nEOF\n").unwrap();
        let doc = tokens
            .iter()
            .find_map(|token| match &token.kind {
                PosixTokenKind::Redirect(Redirect {
                    here_doc: Some(doc),
                    ..
                }) => Some(doc),
                _ => None,
            })
            .unwrap();
        assert_eq!(doc.delimiter, "EOF");
        assert!(doc.quoted);
        assert_eq!(doc.body, "line \"1\"\n$x\n");
        assert_eq!(doc.span.line, 2);
    }

    #[test]
    fn here_document_strip_removes_tabs() {
        let tokens = lex("cat <<-END\n\tx\n\tEND\n").unwrap();
        let doc = tokens
            .iter()
            .find_map(|token| match &token.kind {
                PosixTokenKind::Redirect(Redirect {
                    here_doc: Some(doc),
                    ..
                }) => Some(doc),
                _ => None,
            })
            .unwrap();
        assert!(!doc.quoted);
        assert_eq!(doc.body, "x\n");
    }

    #[test]
    fn positions_count_characters() {
        let tokens = lex("rm \"zażółć\" /b").unwrap();
        assert_eq!(
            tokens[1].span,
            Span {
                line: 1,
                column: 4,
                end_line: 1,
                end_column: 12
            }
        );
        assert_eq!(tokens[2].span.column, 13);
    }

    #[test]
    fn positions_follow_lines() {
        let tokens = lex("ls /a\nrm /b").unwrap();
        assert_eq!(tokens[3].span.line, 2);
        assert_eq!(tokens[3].span.column, 1);
    }

    #[test]
    fn unterminated_quotes_are_errors() {
        assert_eq!(lex("echo 'abc").unwrap_err().column, 6);
        assert!(lex("echo \"abc").is_err());
        assert!(lex("echo $(ls").is_err());
        assert!(lex("echo `ls").is_err());
        assert!(lex("cat <<EOF\nabc\n").is_err());
        assert!(lex("cat <<EOF").is_err());
    }

    #[test]
    fn empty_quotes_give_an_empty_word() {
        let word = &words("echo ''")[1];
        assert_eq!(word.literal().as_deref(), Some(""));
    }

    #[test]
    fn lex_error_displays_position() {
        let error = lex("echo 'x").unwrap_err();
        assert_eq!(error.to_string(), "1:6: unterminated `'`");
    }
}
