// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use logos::Logos;

use super::token::{ForeignKind, TokenKind};

/// Tokens of fixed form recognized by longest match: operators, punctuation,
/// words and the foreign operators of other shells.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Simple {
    #[token("|>")]
    PipeGt,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token("<=")]
    Le,
    #[token(">")]
    Gt,
    #[token(">=")]
    Ge,
    #[token("~")]
    Tilde,
    #[token("!~")]
    NotTilde,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("=")]
    Eq,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("..")]
    DotDot,
    #[token("...")]
    Ellipsis,
    #[token("@")]
    At,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    /// Word of ASCII letters, digits and `_`; classified by the lexer.
    #[regex("[A-Za-z_][A-Za-z0-9_]*")]
    Word,
    #[token("|")]
    Pipe,
    #[regex(r"\|[ \t]+>")]
    PipeSpaceGt,
    #[token("|->")]
    PipeArrow,
    #[token("->")]
    Arrow,
    #[regex("--[A-Za-z][A-Za-z0-9_-]*")]
    LongFlag,
    #[token("$")]
    Dollar,
    #[token(";")]
    Semicolon,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("&")]
    Amp,
    #[token("`")]
    Backtick,
    #[token(">>")]
    #[token("<<")]
    Redirect,
    #[token("!")]
    Bang,
}

/// Longest simple token at the start of `rest` and its length.
pub(crate) fn longest(rest: &str) -> Option<(Simple, usize)> {
    let mut lexer = Simple::lexer(rest);
    match lexer.next() {
        Some(Ok(simple)) => Some((simple, lexer.span().end)),
        _ => None,
    }
}

impl Simple {
    /// Token kind of a valid operator or punctuation token; `None` for words
    /// and foreign operators.
    pub(crate) fn kind(self) -> Option<TokenKind> {
        let kind = match self {
            Self::PipeGt => TokenKind::PipeGt,
            Self::EqEq => TokenKind::EqEq,
            Self::NotEq => TokenKind::NotEq,
            Self::Lt => TokenKind::Lt,
            Self::Le => TokenKind::Le,
            Self::Gt => TokenKind::Gt,
            Self::Ge => TokenKind::Ge,
            Self::Tilde => TokenKind::Tilde,
            Self::NotTilde => TokenKind::NotTilde,
            Self::Plus => TokenKind::Plus,
            Self::Minus => TokenKind::Minus,
            Self::Star => TokenKind::Star,
            Self::Eq => TokenKind::Eq,
            Self::Colon => TokenKind::Colon,
            Self::Comma => TokenKind::Comma,
            Self::Dot => TokenKind::Dot,
            Self::DotDot => TokenKind::DotDot,
            Self::Ellipsis => TokenKind::Ellipsis,
            Self::At => TokenKind::At,
            Self::LParen => TokenKind::LParen,
            Self::RParen => TokenKind::RParen,
            Self::LBracket => TokenKind::LBracket,
            Self::RBracket => TokenKind::RBracket,
            Self::LBrace => TokenKind::LBrace,
            Self::RBrace => TokenKind::RBrace,
            _ => return None,
        };
        Some(kind)
    }

    /// Foreign kind of an operator of another shell, with the problem name,
    /// the message and the replacement of the strict error; an empty
    /// replacement means none.
    pub(crate) fn foreign(self) -> Option<(ForeignKind, &'static str, &'static str, &'static str)> {
        let foreign = match self {
            Self::Pipe => (
                ForeignKind::Pipe,
                "pipe",
                "stages are joined with `|>`, not `|`",
                "|>",
            ),
            Self::PipeSpaceGt => (
                ForeignKind::PipeSpaceGt,
                "pipe",
                "write the pipeline operator `|>` without a space",
                "|>",
            ),
            Self::PipeArrow => (
                ForeignKind::PipeArrow,
                "pipe",
                "stages are joined with `|>`, not `|->`",
                "|>",
            ),
            Self::Arrow => (
                ForeignKind::Arrow,
                "pipe",
                "stages are joined with `|>`, not `->`",
                "|>",
            ),
            Self::LongFlag => (
                ForeignKind::Flag,
                "flag",
                "flags are not part of the language; write a named argument `name: value`",
                "",
            ),
            Self::Dollar => (
                ForeignKind::Dollar,
                "variable",
                "`$` variables are not part of the language; name a result with `let` and refer to it with `@name`",
                "",
            ),
            Self::Semicolon => (
                ForeignKind::Semicolon,
                "separator",
                "statements are separated by line breaks, not `;`",
                "",
            ),
            Self::AndAnd => (
                ForeignKind::AndAnd,
                "shell_operator",
                "`&&` is not part of the language; write `and` in expressions or one statement per line",
                "",
            ),
            Self::OrOr => (
                ForeignKind::OrOr,
                "shell_operator",
                "`||` is not part of the language; write `or` in expressions",
                "",
            ),
            Self::Amp => (
                ForeignKind::Amp,
                "shell_operator",
                "`&` is not part of the language; run a program in the background with `run … background: true`",
                "",
            ),
            Self::Backtick => (
                ForeignKind::Backtick,
                "shell_operator",
                "command substitution is not part of the language; name a result with `let`",
                "",
            ),
            Self::Redirect => (
                ForeignKind::Redirect,
                "redirect",
                "redirection is not part of the language; write to a file with `write`",
                "",
            ),
            Self::Bang => (
                ForeignKind::Other,
                "bang",
                "`!` stands alone; write `not` for negation, `!=` or `!~` for comparison",
                "",
            ),
            _ => return None,
        };
        Some(foreign)
    }
}
