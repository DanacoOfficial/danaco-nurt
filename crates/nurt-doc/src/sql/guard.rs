// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lexical checks of single SQL statements before they run.
//!
//! The checks read the statement text as prepared by SQLite, so a statement
//! boundary is always the one SQLite uses. `ATTACH` and `DETACH` are
//! top-level statements only, `VACUUM INTO` is the only use of `INTO` in a
//! `VACUUM` statement, and a pragma name may be written as an identifier, a
//! quoted identifier or a string literal (SQLite grammar rule `nm`).

/// Pragmas that make SQLite create or use files outside the database
/// directory.
const DENIED_PRAGMAS: &[&str] = &["temp_store_directory", "data_store_directory"];

/// Why a statement is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Denial {
    /// `ATTACH` or `DETACH`.
    Attach(&'static str),
    /// `VACUUM INTO`.
    VacuumInto,
    /// A pragma from [`DENIED_PRAGMAS`].
    Pragma(String),
    /// A statement that ends or starts the transaction of a change.
    Transaction(String),
}

/// Checks one prepared statement; `writing` refuses transaction control as
/// well, because a change runs in one transaction opened by `sql`.
pub(super) fn check(sql: &str, writing: bool) -> Option<Denial> {
    let mut tokens = Tokens::new(sql);
    let Some(Token::Word(first)) = tokens.next() else {
        return None;
    };
    let keyword = first.to_ascii_uppercase();
    match keyword.as_str() {
        "ATTACH" => Some(Denial::Attach("ATTACH")),
        "DETACH" => Some(Denial::Attach("DETACH")),
        "VACUUM" => tokens
            .any(|token| token.is_keyword("INTO"))
            .then_some(Denial::VacuumInto),
        "PRAGMA" => pragma_name(&mut tokens)
            .filter(|name| DENIED_PRAGMAS.iter().any(|denied| name.eq_ignore_ascii_case(denied)))
            .map(Denial::Pragma),
        "BEGIN" | "COMMIT" | "END" if writing => Some(Denial::Transaction(keyword)),
        "ROLLBACK" if writing && !tokens.any(|token| token.is_keyword("TO")) => {
            Some(Denial::Transaction(keyword))
        }
        _ => None,
    }
}

/// Number of statements in `query`, counting the statements of trigger
/// bodies as part of their `CREATE TRIGGER` statement.
pub(super) fn statement_count(query: &str) -> usize {
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

fn pragma_name(tokens: &mut Tokens<'_>) -> Option<String> {
    let first = tokens.next()?.into_name()?;
    match tokens.next() {
        Some(Token::Symbol('.')) => tokens.next()?.into_name(),
        _ => Some(first),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token<'a> {
    /// Keyword or bare identifier.
    Word(&'a str),
    /// Identifier in double quotes, backticks or brackets, unquoted.
    Quoted(String),
    /// String literal, unquoted.
    Text(String),
    /// Any other character outside comments.
    Symbol(char),
}

impl Token<'_> {
    fn is_keyword(&self, keyword: &str) -> bool {
        matches!(self, Token::Word(word) if word.eq_ignore_ascii_case(keyword))
    }

    fn into_name(self) -> Option<String> {
        match self {
            Token::Word(word) => Some(word.to_owned()),
            Token::Quoted(name) | Token::Text(name) => Some(name),
            Token::Symbol(_) => None,
        }
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

    /// Reads a quoted run that ends at `close`; a doubled `close` stands for
    /// one character, except in brackets, which have no escape.
    fn quoted(&mut self, close: char, doubled: bool) -> String {
        let mut text = String::new();
        let mut chars = self.rest.char_indices();
        while let Some((index, character)) = chars.next() {
            if character != close {
                text.push(character);
                continue;
            }
            let after = index + character.len_utf8();
            if doubled && self.rest[after..].starts_with(close) {
                text.push(close);
                chars.next();
                continue;
            }
            self.rest = &self.rest[after..];
            return text;
        }
        self.rest = "";
        text
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
        let token = match first {
            '\'' => Token::Text(self.quoted('\'', true)),
            '"' => Token::Quoted(self.quoted('"', true)),
            '`' => Token::Quoted(self.quoted('`', true)),
            '[' => Token::Quoted(self.quoted(']', false)),
            other => Token::Symbol(other),
        };
        Some(token)
    }
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$' || !character.is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_attach_and_detach_after_comments() {
        assert_eq!(
            check("/* x */ -- y\n attach 'other.db' as o", false),
            Some(Denial::Attach("ATTACH"))
        );
        assert_eq!(check("DETACH o", true), Some(Denial::Attach("DETACH")));
    }

    #[test]
    fn refuses_vacuum_into_only() {
        assert_eq!(check("vacuum main into '/tmp/copy.db'", true), Some(Denial::VacuumInto));
        assert_eq!(check("VACUUM", true), None);
        assert_eq!(check("vacuum \"into\"", true), None);
    }

    #[test]
    fn refuses_directory_pragmas_in_every_spelling() {
        let denied = Some(Denial::Pragma("temp_store_directory".to_owned()));
        assert_eq!(check("PRAGMA temp_store_directory = '/tmp'", false), denied);
        assert_eq!(check("pragma 'temp_store_directory'='/tmp'", false), denied);
        assert_eq!(check("pragma [temp_store_directory]", false), denied);
        assert_eq!(
            check("pragma main.\"data_store_directory\"", false),
            Some(Denial::Pragma("data_store_directory".to_owned()))
        );
        assert_eq!(check("pragma main.journal_mode = wal", true), None);
        assert_eq!(check("pragma", true), None);
    }

    #[test]
    fn refuses_transaction_control_only_when_writing() {
        assert_eq!(check("begin", true), Some(Denial::Transaction("BEGIN".to_owned())));
        assert_eq!(check("commit", true), Some(Denial::Transaction("COMMIT".to_owned())));
        assert_eq!(check("rollback", true), Some(Denial::Transaction("ROLLBACK".to_owned())));
        assert_eq!(check("rollback transaction to s1", true), None);
        assert_eq!(check("savepoint s1", true), None);
        assert_eq!(check("begin", false), None);
    }

    #[test]
    fn accepts_ordinary_statements_and_empty_text() {
        assert_eq!(check("select 'attach' from [vacuum]", false), None);
        assert_eq!(check("", false), None);
        assert_eq!(check("-- only a comment", false), None);
        assert_eq!(check("/* unterminated", false), None);
        assert_eq!(check("select 'unterminated", false), None);
    }

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
}
