// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-contracts::sql_text
// Responsibility:   Lexical reading of SQL text: statement boundaries and the kind of each.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Lexical reading of SQL text: statement boundaries and the kind of each
//! statement.
//!
//! The reading follows the tokens of SQLite: a semicolon inside a string
//! literal, a quoted identifier or a comment does not end a statement, and
//! the statements of a trigger body belong to their `CREATE TRIGGER`
//! statement. Identifiers may be quoted with double quotes, backticks or
//! brackets (SQLite grammar rule `nm`).
//!
//! The binder uses [`changes_database`] to decide, before any effect,
//! whether a `sql` call is a change; the `sql` command uses
//! [`statement_kinds`] to run each statement in the right place of its
//! transaction.

/// Words of a statement among which `TRIGGER` marks `CREATE [TEMP] TRIGGER`;
/// only the body of a trigger opens a block with `BEGIN`.
const TRIGGER_KEYWORD_WINDOW: usize = 3;

/// Pragmas that return rows when written as `pragma name(argument)`; every
/// other pragma with an argument or an assignment sets a value.
const QUERY_PRAGMAS: [&str; 10] = [
    "quick_check",
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "foreign_key_check",
    "integrity_check",
];

/// Pragmas that change the database or its files even without a value.
const ACTION_PRAGMAS: [&str; 4] = [
    "optimize",
    "shrink_memory",
    "incremental_vacuum",
    "wal_checkpoint",
];

/// Function that loads code into the process.
const EXTENSION_FUNCTION: &str = "load_extension";

/// Kind of one statement of a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    /// `SELECT`, `VALUES`, `EXPLAIN`, a read-only `WITH` and a pragma that
    /// only reads.
    Read,
    /// Any other statement that may change the database, including `SAVEPOINT`,
    /// `RELEASE` and `ROLLBACK TO`.
    Change,
    /// `BEGIN`, with any mode.
    Begin,
    /// `COMMIT` or `END`.
    Commit,
    /// `ROLLBACK` without `TO`.
    Rollback,
    /// `ATTACH`.
    Attach,
    /// `DETACH`.
    Detach,
    /// `VACUUM` without `INTO`.
    Vacuum,
    /// `VACUUM INTO`.
    VacuumInto,
    /// A pragma that sets a value or acts; it runs outside a transaction.
    Pragma,
    /// A statement that calls `load_extension`.
    Extension,
}

/// Number of statements in `query`, counting the statements of trigger
/// bodies as part of their `CREATE TRIGGER` statement.
///
/// # Examples
///
/// ```
/// use nurt_contracts::sql_text::statement_count;
///
/// assert_eq!(statement_count("select 1; select 2;"), 2);
/// assert_eq!(statement_count("select ';'"), 1);
/// ```
#[must_use]
pub fn statement_count(query: &str) -> usize {
    statements(query).len()
}

/// Kind of every statement of `query`, in order.
///
/// # Examples
///
/// ```
/// use nurt_contracts::sql_text::{StatementKind, statement_kinds};
///
/// assert_eq!(
///     statement_kinds("begin; select 1; vacuum"),
///     [StatementKind::Begin, StatementKind::Read, StatementKind::Vacuum]
/// );
/// ```
#[must_use]
pub fn statement_kinds(query: &str) -> Vec<StatementKind> {
    statements(query)
        .iter()
        .map(|statement| kind_of(&statement.tokens))
        .collect()
}

/// Text of every statement of `query`, in order, without the semicolon.
///
/// # Examples
///
/// ```
/// use nurt_contracts::sql_text::statement_texts;
///
/// assert_eq!(statement_texts("select 1; vacuum into 'a';"), ["select 1", "vacuum into 'a'"]);
/// ```
#[must_use]
pub fn statement_texts(query: &str) -> Vec<&str> {
    statements(query)
        .into_iter()
        .map(|statement| statement.text)
        .collect()
}

/// `true` when some statement of `query` may change a database, a
/// connection, a transaction or a file.
///
/// # Examples
///
/// ```
/// use nurt_contracts::sql_text::changes_database;
///
/// assert!(!changes_database("select * from users"));
/// assert!(changes_database("select 1; delete from users"));
/// ```
#[must_use]
pub fn changes_database(query: &str) -> bool {
    statement_kinds(query)
        .into_iter()
        .any(|kind| kind != StatementKind::Read)
}

/// One statement of a query: its tokens and its text.
struct Statement<'a> {
    tokens: Vec<Token<'a>>,
    text: &'a str,
}

fn statements(query: &str) -> Vec<Statement<'_>> {
    let mut found = Vec::new();
    let mut current: Vec<Token<'_>> = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_usize;
    let mut creates = false;
    let mut trigger = false;
    let mut tokens = Tokens::new(query);
    loop {
        tokens.skip_space_and_comments();
        let position = query.len() - tokens.rest.len();
        let Some(token) = tokens.next() else {
            break;
        };
        if token == Token::Symbol(';') {
            if depth == 0 && !current.is_empty() {
                found.push(Statement {
                    tokens: std::mem::take(&mut current),
                    text: query[start..position].trim(),
                });
            }
            continue;
        }
        if current.is_empty() {
            start = position;
            creates = token.is_keyword("CREATE");
            trigger = false;
        }
        if creates && current.len() < TRIGGER_KEYWORD_WINDOW && token.is_keyword("TRIGGER") {
            trigger = true;
        }
        let opens = token.is_keyword("CASE") || (trigger && token.is_keyword("BEGIN"));
        if opens {
            depth += 1;
        } else if token.is_keyword("END") && depth > 0 {
            depth -= 1;
        }
        current.push(token);
    }
    if !current.is_empty() {
        found.push(Statement {
            tokens: current,
            text: query[start..].trim(),
        });
    }
    found
}

fn kind_of(tokens: &[Token<'_>]) -> StatementKind {
    if tokens
        .iter()
        .any(|token| token.is_keyword(EXTENSION_FUNCTION))
    {
        return StatementKind::Extension;
    }
    let Some(Token::Word(first)) = tokens.first().copied() else {
        return StatementKind::Change;
    };
    match first.to_ascii_uppercase().as_str() {
        "SELECT" | "VALUES" | "EXPLAIN" => StatementKind::Read,
        "WITH" if with_changes(tokens) => StatementKind::Change,
        "WITH" => StatementKind::Read,
        "PRAGMA" if pragma_changes(&tokens[1..]) => StatementKind::Pragma,
        "PRAGMA" => StatementKind::Read,
        "BEGIN" => StatementKind::Begin,
        "COMMIT" | "END" => StatementKind::Commit,
        "ROLLBACK" if tokens.iter().any(|token| token.is_keyword("TO")) => StatementKind::Change,
        "ROLLBACK" => StatementKind::Rollback,
        "ATTACH" => StatementKind::Attach,
        "DETACH" => StatementKind::Detach,
        "VACUUM" if tokens.iter().any(|token| token.is_keyword("INTO")) => {
            StatementKind::VacuumInto
        }
        "VACUUM" => StatementKind::Vacuum,
        _ => StatementKind::Change,
    }
}

fn with_changes(tokens: &[Token<'_>]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        token.is_keyword("INSERT")
            || token.is_keyword("UPDATE")
            || token.is_keyword("DELETE")
            || (token.is_keyword("REPLACE")
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| next.is_keyword("INTO")))
    })
}

fn pragma_changes(tokens: &[Token<'_>]) -> bool {
    let mut name = None;
    let mut assigned = false;
    let mut called = false;
    for token in tokens {
        match token {
            Token::Word(word) => name = Some(*word),
            Token::Symbol('=') => {
                assigned = true;
                break;
            }
            Token::Symbol('(') => {
                called = true;
                break;
            }
            _ => {}
        }
    }
    let Some(name) = name else {
        return true;
    };
    let listed = |list: &[&str]| list.iter().any(|item| name.eq_ignore_ascii_case(item));
    if assigned {
        true
    } else if called {
        !listed(&QUERY_PRAGMAS)
    } else {
        listed(&ACTION_PRAGMAS)
    }
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
    fn begin_opens_a_block_only_in_a_trigger() {
        assert_eq!(statement_count("select begin from t; select 2"), 2);
        assert_eq!(statement_count("create table begin (x); select 1"), 2);
        assert_eq!(statement_count("begin; select 1; commit"), 3);
        let temporary = "create temp trigger t after insert on a begin select 1; end; select 2";
        assert_eq!(statement_count(temporary), 2);
        let guarded = "CREATE TRIGGER IF NOT EXISTS t BEGIN SELECT 1; END; SELECT 2";
        assert_eq!(statement_count(guarded), 2);
    }

    #[test]
    fn nested_case_expressions_do_not_end_the_statement_early() {
        assert_eq!(
            statement_count("select case when 1 then case when 2 then 3 end end; select 2"),
            2
        );
    }

    #[test]
    fn counts_multibyte_identifiers_as_words() {
        assert_eq!(statement_count("select zażółć from tabela; select 2"), 2);
    }

    #[test]
    fn ignores_semicolons_in_quotes_and_comments() {
        assert_eq!(statement_count("select [a;b], `c;d`, 'it''s;'"), 1);
        assert_eq!(statement_count("select 1 /* ; */ -- ;\n; select 2"), 2);
        assert_eq!(statement_count("/* unterminated ;"), 0);
        assert_eq!(statement_count("select 'unterminated ;"), 1);
        assert_eq!(statement_count(""), 0);
    }

    #[test]
    fn returns_the_text_of_each_statement() {
        assert_eq!(
            statement_texts(
                " -- c\n select ';' , 1 ; /* x */ create trigger t after insert on a begin select 1; end; select 2"
            ),
            [
                "select ';' , 1",
                "create trigger t after insert on a begin select 1; end",
                "select 2"
            ]
        );
        assert!(statement_texts(" ; ").is_empty());
    }

    #[test]
    fn classifies_reading_statements() {
        for query in [
            "select 1",
            "values (1)",
            "explain delete from t",
            "with a as (select 1) select * from a",
            "pragma user_version",
            "pragma main.table_info(t)",
            "select replace(a, 'x', 'y') from t",
        ] {
            assert!(!changes_database(query), "{query}");
        }
    }

    #[test]
    fn classifies_changing_statements() {
        for query in [
            "insert into t values (1)",
            "with a as (select 1) insert into t select * from a",
            "with a as (select 1) replace into t select * from a",
            "pragma user_version = 3",
            "pragma journal_mode(wal)",
            "pragma optimize",
            "pragma temp_store_directory = '/tmp'",
            "select load_extension('x')",
            "create trigger t after insert on a begin select 1; end",
            "savepoint s",
            "rollback to s",
            "(select 1)",
        ] {
            assert!(changes_database(query), "{query}");
        }
    }

    #[test]
    fn names_the_kind_of_each_statement() {
        assert_eq!(
            statement_kinds(
                "begin immediate; attach 'a' as b; detach b; commit; end; rollback; vacuum into 'x'; vacuum"
            ),
            [
                StatementKind::Begin,
                StatementKind::Attach,
                StatementKind::Detach,
                StatementKind::Commit,
                StatementKind::Commit,
                StatementKind::Rollback,
                StatementKind::VacuumInto,
                StatementKind::Vacuum,
            ]
        );
        assert_eq!(
            statement_kinds("select load_extension('x')"),
            [StatementKind::Extension]
        );
        assert!(statement_kinds(" ; -- nothing\n ;").is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
