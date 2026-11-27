// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::lex::token
// Responsibility:   Largest integer that a literal may denote, `2^53 - 1`.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::ops::Range;

use nurt_contracts::ast::Span;
use serde::Serialize;

use super::LexError;

/// Largest integer that a literal may denote, `2^53 - 1`.
///
/// Larger integers are not exactly representable in the canonical JSON
/// encoding (RFC 8785) of a syntax tree, so the lexer reports them as lexical
/// errors. Sizes and durations obey the same limit.
pub const MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;

/// Keywords of the language. Each has its own [`TokenKind`].
///
/// Command names are not keywords: they are identifiers reserved only in
/// command position.
pub const KEYWORDS: [&str; 8] = ["let", "true", "false", "null", "and", "or", "not", "in"];

/// Maximum length of an identifier in bytes.
pub const MAX_IDENTIFIER_LEN: usize = 64;

/// Token produced by the [`Lexer`](super::Lexer).
///
/// Literal tokens carry no value: the parser decodes the source fragment
/// `&text[token.range.clone()]` with [`decode_text`](super::decode_text),
/// [`decode_raw`](super::decode_raw), [`decode_regex`](super::decode_regex),
/// [`parse_int`](super::parse_int), [`parse_number`](super::parse_number),
/// [`parse_size`](super::parse_size) or [`parse_duration`](super::parse_duration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Kind of the token.
    pub kind: TokenKind,
    /// Position of the token; columns count Unicode scalar values and the
    /// end is exclusive.
    pub span: Span,
    /// Byte range of the token in the program text.
    pub range: Range<usize>,
    /// Whether a space, a tab or the start of a line precedes the token.
    pub space_before: bool,
    /// Whether a space, a tab, a line break, a comment or the end of the
    /// input follows the token.
    pub space_after: bool,
    /// Character that directly follows the token without a space.
    pub glued: Glued,
}

/// Character directly after a token, used to recognize named arguments
/// (`name:` in variants A and B, `name=` in variant C) and calls (`name(`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glued {
    /// No such character follows.
    None,
    /// `:` follows.
    Colon,
    /// `(` follows.
    LParen,
    /// `=` follows, not as part of `==`.
    Equals,
}

/// Context chosen by the parser for the next token.
///
/// The context decides only whether `/` starts a regular expression or is
/// the division operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LexContext {
    /// The parser expects a value: `/` always starts a regular expression.
    Value,
    /// The parser expects an operator. With
    /// [`LexOptions::arguments_by_whitespace`], `/` starts a regular
    /// expression when whitespace or the start of a line precedes it and no
    /// whitespace follows it (`find "/a" /re/`); otherwise, and always in
    /// variant B, `/` is division (`a / b`, `a/b`).
    Operator,
}

/// Options of the lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexOptions {
    /// Whether whitespace separates arguments: `true` for variants A and C,
    /// `false` for variant B, where commas separate arguments.
    pub arguments_by_whitespace: bool,
    /// Whether foreign tokens replace lexical errors. A tolerant lexer never
    /// returns an error; see [`ForeignKind`].
    pub tolerant: bool,
    /// Whether comments are recorded for [`Lexer::comments`](super::Lexer::comments).
    pub keep_comments: bool,
}

impl Default for LexOptions {
    /// Strict lexing of variant A without comments.
    fn default() -> Self {
        Self {
            arguments_by_whitespace: true,
            tolerant: false,
            keep_comments: false,
        }
    }
}

/// Comment of the program text, recorded in source order.
///
/// The parser stores comments in the member `comments` of the tree metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    /// Position of the comment, including the leading `#`.
    pub span: Span,
    /// Comment text without the leading `#` and without the line break.
    pub text: String,
}

/// Kind of a token.
///
/// The documentation of each kind states the exact extent of the token in
/// the source text. Whitespace (space, tab), line breaks that do not end a
/// statement, comments, a byte order mark at the start and a `#!` first line
/// are not tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// Identifier `[a-z][a-z0-9_]*`, no longer than [`MAX_IDENTIFIER_LEN`]
    /// bytes, other than a keyword. Command names with a namespace (`git.status`)
    /// are identifiers separated by [`TokenKind::Dot`].
    Ident,
    /// Keyword `let`.
    Let,
    /// Keyword `true`.
    True,
    /// Keyword `false`.
    False,
    /// Keyword `null`.
    Null,
    /// Keyword `and`.
    And,
    /// Keyword `or`.
    Or,
    /// Keyword `not`.
    Not,
    /// Keyword `in`.
    In,
    /// Text `"…"`, from the opening to the closing quote inclusive. The text
    /// may span lines. In tolerant mode the text may contain invalid escapes;
    /// see [`invalid_escapes`](super::invalid_escapes).
    Text,
    /// Raw text `r"…"`, `r#"…"#`, `r##"…"##`, from `r` to the last `#` of the
    /// closing delimiter inclusive.
    RawText,
    /// Regular expression `/…/flags`, from the opening `/` to the last flag
    /// inclusive.
    Regex,
    /// Integer `[0-9][0-9_]*` or `0x[0-9A-Fa-f][0-9A-Fa-f_]*`, without a
    /// sign.
    Int,
    /// Number with a fraction, `INT.INT`, without a sign or an exponent.
    Number,
    /// Size: an integer or a number directly followed by one of the units
    /// `b kb mb gb tb kib mib gib tib`.
    Size,
    /// Duration: one or more integers, each directly followed by one of the
    /// units `ms s m h d w` (`1h30m`).
    Duration,
    /// `|>`.
    PipeGt,
    /// `==`.
    EqEq,
    /// `!=`.
    NotEq,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `~`.
    Tilde,
    /// `!~`.
    NotTilde,
    /// `+`; never part of a number.
    Plus,
    /// `-`; never part of a number.
    Minus,
    /// `*`.
    Star,
    /// `/` as division; see [`LexContext`].
    Slash,
    /// `=`.
    Eq,
    /// `:`.
    Colon,
    /// `,`.
    Comma,
    /// `.`.
    Dot,
    /// `..`.
    DotDot,
    /// `...`.
    Ellipsis,
    /// `@`.
    At,
    /// `(`.
    LParen,
    /// `)`.
    RParen,
    /// `[`.
    LBracket,
    /// `]`.
    RBracket,
    /// `{`.
    LBrace,
    /// `}`.
    RBrace,
    /// Line break that ends a statement: `\n` or `\r\n`. Consecutive line
    /// breaks, blank lines and comment lines produce one token, which covers
    /// the first line break. No token is produced inside brackets, after
    /// `|>`, `=` or `,`, at the start of the input or when the next
    /// non-blank, non-comment line starts with `|>`.
    Nl,
    /// End of the input; an empty range at the end of the text.
    Eof,
    /// Foreign token, produced only in tolerant mode in place of a lexical
    /// error.
    Foreign(ForeignKind),
}

/// Foreign token of the tolerant mode.
///
/// Every foreign token keeps the exact source fragment in
/// [`Token::range`]. A text that the strict lexer accepts produces the same
/// tokens in both modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignKind {
    /// Text in single quotes `'…'` on one line; `content` is the byte range
    /// between the quotes.
    SingleQuoted {
        /// Byte range of the text between the quotes.
        content: Range<usize>,
    },
    /// Text in typographic quotes (`„…”`, `“…”`, `‚…‘`, `‘…’`, `«…»`) on one
    /// line; `content` is the byte range between the quotes.
    TypographicQuoted {
        /// Byte range of the text between the quotes.
        content: Range<usize>,
    },
    /// `|` not followed by `>`.
    Pipe,
    /// `|` and `>` separated by spaces or tabs, `| >`.
    PipeSpaceGt,
    /// `|->`.
    PipeArrow,
    /// `->`.
    Arrow,
    /// Command line flag: `--name`, or `-Name` with an upper-case letter
    /// after whitespace.
    Flag,
    /// `$`.
    Dollar,
    /// `;`.
    Semicolon,
    /// `&&`.
    AndAnd,
    /// `||`.
    OrOr,
    /// `&` not followed by `&`.
    Amp,
    /// `` ` ``.
    Backtick,
    /// Redirection or here-document operator, `>>` or `<<`.
    Redirect,
    /// Unquoted path: `/srv/app` where no regular expression can be read,
    /// `C:\…`, `C:/…`, and in value context `~/…`, `./…`, `../…`. The path
    /// ends before whitespace or one of ``"'`,;()[]{}|``.
    BarePath,
    /// Word of ASCII letters, digits and `_` that is not an identifier: it
    /// contains an upper-case letter (`Find`, `caseSensitive`), starts with
    /// `_`, or directly follows a number (`10MB`, `5min`).
    LooseWord,
    /// Word that contains a letter or digit outside ASCII.
    ForeignWord,
    /// Run of whitespace other than space, tab and line break: no-break
    /// space, zero-width characters, a byte order mark after the start, a
    /// carriage return outside `\r\n`.
    Whitespace,
    /// Text `"…` or raw text without the closing delimiter; the token
    /// extends to the end of the input.
    UnterminatedText {
        /// Byte offset of the end of the first line of the text, where a
        /// closing quote would end the text on its first line.
        first_line_end: usize,
    },
    /// Any other character that does not start a token, one per token.
    Other,
    /// Malformed literal or name, with the error the strict lexer reports.
    Malformed(LexError),
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
