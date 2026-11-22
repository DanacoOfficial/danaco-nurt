// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Statement boundaries of a query.
//!
//! The count follows the tokens of SQLite: a semicolon inside a string
//! literal, a quoted identifier or a comment does not end a statement, and
//! the statements of a trigger body belong to their `CREATE TRIGGER`
//! statement. Identifiers may be quoted with double quotes, backticks or
//! brackets (SQLite grammar rule `nm`).

/// Number of statements in `query`, counting the statements of trigger
/// bodies as part of their `CREATE TRIGGER` statement.
pub(crate) fn statement_count(query: &str) -> usize {
    let mut count = 0;
    let mut depth = 0_usize;
    let mut in_statement = false;
    for token in Tokens::new(query) {
        if token == Token::Symbol(';') {
            if depth == 0 && in_statement {
                count += 1;
                in_statement = false;
            }
            continue;
        }
        let opens = token.is_keyword("CASE") || (in_statement && token.is_keyword("BEGIN"));
        if opens {
            depth += 1;
        } else if token.is_keyword("END") && depth > 0 {
            depth -= 1;
        }
        in_statement = true;
    }
    count + usize::from(in_statement)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    /// Keyword or bare identifier.
    Word(&'a str),
    /// String literal or quoted identifier.
    Quoted,
    /// Any other character outside comments.
    Symbol(char),
}

impl Token<'_> {
    fn is_keyword(self, keyword: &str) -> bool {
        matches!(self, Token::Word(word) if word.eq_ignore_ascii_case(keyword))
    }
}

struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Tokens<'a> {
    fn new(sql: &'a str) -> Self {
        Self { rest: sql }
    }

    fn skip_space_and_comments(&mut self) {
        loop {
            let trimmed = self.rest.trim_start();
            if let Some(comment) = trimmed.strip_prefix("--") {
                self.rest = comment.find('\n').map_or("", |end| &comment[end..]);
            } else if let Some(comment) = trimmed.strip_prefix("/*") {
                self.rest = comment.find("*/").map_or("", |end| &comment[end + 2..]);
            } else {
                self.rest = trimmed;
                return;
            }
        }
    }

    /// Skips a quoted run that ends at `close`; a doubled `close` stands for
    /// one character, except in brackets, which have no escape.
    fn skip_quoted(&mut self, close: char, doubled: bool) {
        let mut chars = self.rest.char_indices();
        while let Some((index, character)) = chars.next() {
            if character != close {
                continue;
            }
            let after = index + character.len_utf8();
            if doubled && self.rest[after..].starts_with(close) {
                chars.next();
                continue;
            }
            self.rest = &self.rest[after..];
            return;
        }
        self.rest = "";
    }

    fn word(&mut self) -> &'a str {
        let end = self
            .rest
            .find(|character: char| !is_word_char(character))
            .unwrap_or(self.rest.len());
        let (word, rest) = self.rest.split_at(end);
        self.rest = rest;
        word
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.skip_space_and_comments();
        let first = self.rest.chars().next()?;
        if is_word_char(first) {
            return Some(Token::Word(self.word()));
        }
        self.rest = &self.rest[first.len_utf8()..];
        let (close, doubled) = match first {
            '\'' | '"' | '`' => (first, true),
            '[' => (']', false),
            other => return Some(Token::Symbol(other)),
        };
        self.skip_quoted(close, doubled);
        Some(Token::Quoted)
    }
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$' || !character.is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_statements_with_trigger_bodies() {
        assert_eq!(statement_count("select 1"), 1);
        assert_eq!(statement_count("select 1; select 2;"), 2);
        assert_eq!(statement_count(" ; -- nothing\n ;"), 0);
        assert_eq!(statement_count("select ';'; select \"a;b\""), 2);
        let trigger = "create trigger t after insert on a begin \
                       update a set x = case when 1 then 2 end; delete from b; end; select 1";
        assert_eq!(statement_count(trigger), 2);
    }

    #[test]
    fn ignores_semicolons_in_quotes_and_comments() {
        assert_eq!(statement_count("select [a;b], `c;d`, 'it''s;'"), 1);
        assert_eq!(statement_count("select 1 /* ; */ -- ;\n; select 2"), 2);
        assert_eq!(statement_count("/* unterminated ;"), 0);
        assert_eq!(statement_count("select 'unterminated ;"), 1);
        assert_eq!(statement_count(""), 0);
    }
}
