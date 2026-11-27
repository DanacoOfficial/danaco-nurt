// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Files that `ATTACH` and `VACUUM INTO` name.
//!
//! The reading works on the expanded text of a prepared statement, in which
//! SQLite has replaced every bound parameter by its literal. A literal is
//! read as it is; any other expression is computed by the caller with a
//! `SELECT` of the same expression before the statement runs.

/// Parts of `ATTACH [DATABASE] <expression> AS <alias>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AttachParts {
    /// Expression that names the file.
    pub(super) expression: String,
    /// Text after the last `AS`: the name of the database.
    pub(super) alias: String,
}

/// Parts of an `ATTACH` statement; `None` for another statement. The
/// expression is the text up to the last `AS` outside quotes and
/// parentheses.
pub(super) fn attach_parts(sql: &str) -> Option<AttachParts> {
    let mut scan = Scan::new(sql);
    scan.keyword("ATTACH")?;
    if scan
        .peek_word()
        .is_some_and(|word| word.eq_ignore_ascii_case("DATABASE"))
    {
        scan.word();
    }
    let body = scan.rest;
    let mut depth = 0_usize;
    let mut quote: Option<char> = None;
    let mut end = None;
    let mut previous = ' ';
    for (index, character) in body.char_indices() {
        match quote {
            Some(close) => {
                if character == close {
                    quote = None;
                }
            }
            None => match character {
                '\'' | '"' | '`' => quote = Some(character),
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            },
        }
        let word_start = !previous.is_alphanumeric() && previous != '_';
        if quote.is_none()
            && depth == 0
            && word_start
            && body.is_char_boundary(index + 2)
            && body.len() >= index + 2
            && body[index..index + 2].eq_ignore_ascii_case("as")
            && body[index + 2..]
                .chars()
                .next()
                .is_some_and(|next| !(next.is_alphanumeric() || next == '_'))
        {
            end = Some(index);
        }
        previous = character;
    }
    let end = end?;
    let expression = body[..end].trim();
    let alias = body[end + 2..].trim().trim_end_matches(';').trim();
    (!expression.is_empty() && !alias.is_empty()).then(|| AttachParts {
        expression: expression.to_owned(),
        alias: alias.to_owned(),
    })
}

/// Parts of `VACUUM [<schema>] INTO <expression>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VacuumParts {
    /// Schema as written, when given.
    pub(super) schema: Option<String>,
    /// Expression that names the new file.
    pub(super) expression: String,
}

/// Parts of a `VACUUM INTO` statement; `None` for another statement.
pub(super) fn vacuum_parts(sql: &str) -> Option<VacuumParts> {
    let mut scan = Scan::new(sql);
    scan.keyword("VACUUM")?;
    let mut schema = None;
    if !scan
        .peek_word()
        .is_some_and(|word| word.eq_ignore_ascii_case("INTO"))
    {
        let before = scan.rest.trim_start();
        scan.identifier()?;
        let used = before.len() - scan.rest.len();
        schema = Some(before[..used].trim().to_owned());
    }
    scan.keyword("INTO")?;
    let expression = scan.rest.trim().trim_end_matches(';').trim();
    (!expression.is_empty()).then(|| VacuumParts {
        schema,
        expression: expression.to_owned(),
    })
}

/// Text of a single quoted literal; `None` for any other expression.
pub(super) fn literal_of(expression: &str) -> Option<String> {
    let mut scan = Scan::new(expression);
    let text = scan.literal()?;
    scan.ends().then_some(text)
}

struct Scan<'a> {
    rest: &'a str,
}

impl<'a> Scan<'a> {
    fn new(sql: &'a str) -> Self {
        Self { rest: sql }
    }

    fn skip(&mut self) {
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

    fn peek_word(&mut self) -> Option<&'a str> {
        self.skip();
        let end = self
            .rest
            .find(|character: char| !(character.is_alphanumeric() || character == '_'))
            .unwrap_or(self.rest.len());
        (end > 0).then(|| &self.rest[..end])
    }

    fn word(&mut self) -> Option<&'a str> {
        let word = self.peek_word()?;
        self.rest = &self.rest[word.len()..];
        Some(word)
    }

    fn keyword(&mut self, keyword: &str) -> Option<()> {
        self.word()
            .filter(|word| word.eq_ignore_ascii_case(keyword))
            .map(|_| ())
    }

    /// `true` when only a semicolon or nothing follows.
    fn ends(&mut self) -> bool {
        self.skip();
        self.rest.is_empty() || self.rest.starts_with(';')
    }

    /// A bare or quoted name.
    fn identifier(&mut self) -> Option<()> {
        self.skip();
        match self.rest.chars().next()? {
            '"' | '`' | '[' => {
                let first = self.rest.chars().next()?;
                let close = if first == '[' { ']' } else { first };
                let end = self.rest[1..].find(close)?;
                self.rest = &self.rest[end + 2..];
                Some(())
            }
            _ => self.word().map(|_| ()),
        }
    }

    /// A single-quoted text literal; a doubled quote stands for one quote.
    fn literal(&mut self) -> Option<String> {
        self.skip();
        let body = self.rest.strip_prefix('\'')?;
        let mut text = String::new();
        let mut chars = body.char_indices().peekable();
        while let Some((index, character)) = chars.next() {
            if character != '\'' {
                text.push(character);
                continue;
            }
            if chars.next_if(|(_, next)| *next == '\'').is_some() {
                text.push('\'');
                continue;
            }
            self.rest = &body[index + 1..];
            return Some(text);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_attach_parts() {
        let parts = attach_parts("ATTACH DATABASE '/t/' || 'x.db' AS o").unwrap();
        assert_eq!(parts.expression, "'/t/' || 'x.db'");
        assert_eq!(parts.alias, "o");
        let parts = attach_parts("attach (select 'a as b') as \"my db\";").unwrap();
        assert_eq!(parts.expression, "(select 'a as b')");
        assert_eq!(parts.alias, "\"my db\"");
        assert_eq!(attach_parts("attach 'x' alias"), None);
        assert_eq!(attach_parts("select 1"), None);
    }

    #[test]
    fn reads_the_vacuum_parts() {
        let parts = vacuum_parts("VACUUM INTO '/tmp/copy.db'").unwrap();
        assert_eq!(
            (parts.schema, parts.expression.as_str()),
            (None, "'/tmp/copy.db'")
        );
        let parts = vacuum_parts("vacuum \"my db\" into ?;").unwrap();
        assert_eq!(parts.schema.as_deref(), Some("\"my db\""));
        assert_eq!(parts.expression, "?");
        assert_eq!(vacuum_parts("vacuum"), None);
        assert_eq!(vacuum_parts("vacuum main"), None);
    }

    #[test]
    fn reads_a_single_literal() {
        assert_eq!(literal_of("'it''s.db'").as_deref(), Some("it's.db"));
        assert_eq!(literal_of("'a' || 'b'"), None);
        assert_eq!(literal_of("(select 1)"), None);
    }
}
