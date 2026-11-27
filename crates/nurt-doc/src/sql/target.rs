// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Files that `ATTACH` and `VACUUM INTO` name.
//!
//! The reading works on the expanded text of a prepared statement, in which
//! SQLite has replaced every bound parameter by its literal, so a file given
//! as a literal or as a text parameter is found before the statement runs.
//! An expression that is neither, such as a concatenation, is not found; the
//! caller then learns the file after the statement ran or reports the
//! effect as untracked.

/// File of `ATTACH [DATABASE] '<file>' AS <name>`; `None` for another
/// statement and for a file that is not a text literal.
pub(super) fn attached_file(sql: &str) -> Option<String> {
    let mut scan = Scan::new(sql);
    scan.keyword("ATTACH")?;
    if scan
        .peek_word()
        .is_some_and(|word| word.eq_ignore_ascii_case("DATABASE"))
    {
        scan.word();
    }
    let file = scan.literal()?;
    scan.keyword("AS")?;
    Some(file)
}

/// Text of the expression that names the file of `ATTACH [DATABASE] <expr>
/// AS <name>`; `None` for another statement. The expression is the text up to
/// the last `AS` outside quotes and parentheses.
pub(super) fn attached_expression(sql: &str) -> Option<String> {
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
            && body[index..].len() >= 2
            && body.is_char_boundary(index + 2)
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
    let expression = body[..end?].trim();
    (!expression.is_empty()).then(|| expression.to_owned())
}

/// File of `VACUUM [<schema>] INTO '<file>'`; `None` for another statement
/// and for a file that is not a text literal.
pub(super) fn vacuum_target(sql: &str) -> Option<String> {
    let mut scan = Scan::new(sql);
    scan.keyword("VACUUM")?;
    if !scan
        .peek_word()
        .is_some_and(|word| word.eq_ignore_ascii_case("INTO"))
    {
        scan.identifier()?;
    }
    scan.keyword("INTO")?;
    let file = scan.literal()?;
    scan.ends().then_some(file)
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
    fn reads_the_attached_file() {
        assert_eq!(
            attached_file("ATTACH DATABASE '/tmp/a b.db' AS other").as_deref(),
            Some("/tmp/a b.db")
        );
        assert_eq!(
            attached_file("  attach 'it''s.db' as o").as_deref(),
            Some("it's.db")
        );
        assert_eq!(attached_file("attach (select 1) as o"), None);
        assert_eq!(attached_file("attach NULL as o"), None);
        assert_eq!(attached_file("select 1"), None);
    }

    #[test]
    fn reads_the_attach_expression() {
        assert_eq!(
            attached_expression("ATTACH DATABASE '/t/' || 'x.db' AS o").as_deref(),
            Some("'/t/' || 'x.db'")
        );
        assert_eq!(
            attached_expression("attach (select 'a as b') as o").as_deref(),
            Some("(select 'a as b')")
        );
        assert_eq!(attached_expression("attach 'x' alias"), None);
        assert_eq!(attached_expression("select 1"), None);
    }

    #[test]
    fn reads_the_vacuum_target() {
        assert_eq!(
            vacuum_target("VACUUM INTO '/tmp/copy.db'").as_deref(),
            Some("/tmp/copy.db")
        );
        assert_eq!(
            vacuum_target("vacuum main into 'c.db'").as_deref(),
            Some("c.db")
        );
        assert_eq!(
            vacuum_target("vacuum \"my db\" into 'c.db'").as_deref(),
            Some("c.db")
        );
        assert_eq!(vacuum_target("vacuum"), None);
        assert_eq!(vacuum_target("vacuum into 'a' || 'b'"), None);
        assert_eq!(vacuum_target("vacuum into 'a';").as_deref(), Some("a"));
        assert_eq!(vacuum_target("vacuum into NULL"), None);
    }
}
