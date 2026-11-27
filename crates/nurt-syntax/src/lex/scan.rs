// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::lex::scan
// Responsibility:   Token found by a scanner: its kind and the byte offset of its end.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use super::error::Fault;
use super::token::TokenKind;

/// Token found by a scanner: its kind and the byte offset of its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lexeme {
    pub(crate) end: usize,
    pub(crate) kind: TokenKind,
}

impl Lexeme {
    pub(crate) const fn new(end: usize, kind: TokenKind) -> Self {
        Self { end, kind }
    }
}

/// Fragment that the strict lexer rejects, with the token that the tolerant
/// lexer returns instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rejected {
    pub(crate) fault: Fault,
    /// Token of the tolerant mode; `None` produces a malformed token over the
    /// faulty fragment.
    pub(crate) recovery: Option<Lexeme>,
    /// Whether the letters directly after the recovered token form a loose
    /// word, as in `10MB` or `5min`.
    pub(crate) glued_word: bool,
}

/// Outcome of scanning one token.
pub(crate) type Scanned = Result<Lexeme, Box<Rejected>>;

/// Rejection without a dedicated foreign token.
pub(crate) fn reject(fault: Fault) -> Box<Rejected> {
    Box::new(Rejected {
        fault,
        recovery: None,
        glued_word: false,
    })
}

/// Rejection recovered by `lexeme` in tolerant mode.
pub(crate) fn reject_with(fault: Fault, lexeme: Lexeme) -> Box<Rejected> {
    Box::new(Rejected {
        fault,
        recovery: Some(lexeme),
        glued_word: false,
    })
}

/// Byte offset of the end of the line that contains `from`, before `\r\n`
/// or `\n`, or the end of the text.
pub(crate) fn line_end(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let Some(offset) = bytes
        .get(from..)
        .and_then(|rest| rest.iter().position(|&byte| byte == b'\n'))
    else {
        return text.len();
    };
    let newline = from + offset;
    if newline > from && bytes.get(newline - 1) == Some(&b'\r') {
        newline - 1
    } else {
        newline
    }
}

/// Byte length of the character at `offset`, 1 when there is none.
pub(crate) fn char_len_at(text: &str, offset: usize) -> usize {
    text.get(offset..)
        .and_then(|rest| rest.chars().next())
        .map_or(1, char::len_utf8)
}

/// Text literal `"…"` or raw text with the smallest number of `#` that
/// denotes `value` without escapes.
pub(crate) fn quote(value: &str) -> String {
    if !value.contains(['"', '\\']) {
        return format!("\"{value}\"");
    }
    raw(value)
}

/// Raw text with the smallest number of `#` that contains `value`.
pub(crate) fn raw(value: &str) -> String {
    let mut hashes = String::new();
    loop {
        let closing = format!("\"{hashes}");
        if !value.contains(closing.as_str()) {
            return format!("r{hashes}\"{value}\"{hashes}");
        }
        hashes.push('#');
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
