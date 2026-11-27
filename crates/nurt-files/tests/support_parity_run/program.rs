// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/support_parity_run/program.rs
// Responsibility:   Parser of the program notation of the parity scenarios.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parser of the program notation of the parity scenarios.
//!
//! The notation is a subset of the language sufficient for the scenarios:
//! pipelines separated by `;`, stages separated by `|>`, a command name,
//! positional values and named values `name: value`. Values are text in
//! double quotes, regular expressions in slashes, integers, ranges `a..b`,
//! `true`/`false`, bare words, lists in brackets and maps in braces. A token
//! that fits none of these forms is kept as raw text, so that programs of
//! stages outside this crate still parse.

/// Literal value of the notation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Lit {
    /// Text in double quotes, escapes resolved.
    Str(String),
    /// Bare word.
    Ident(String),
    /// Integer.
    Int(i64),
    /// Number with a fraction.
    Float(f64),
    /// `true` or `false`.
    Bool(bool),
    /// Range with optional bounds.
    Range(Option<i64>, Option<i64>),
    /// Regular expression and its flags.
    Regex(String, String),
    /// List.
    List(Vec<Lit>),
    /// Map with keys in written order.
    Map(Vec<(String, Lit)>),
    /// Token of no other form.
    Raw(String),
}

/// One stage of a pipeline.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Call {
    /// Command name.
    pub(crate) command: String,
    /// Positional values in written order.
    pub(crate) positional: Vec<Lit>,
    /// Named values in written order.
    pub(crate) named: Vec<(String, Lit)>,
}

/// Stages connected by `|>`.
pub(crate) type Pipeline = Vec<Call>;

/// Parses a script of one or more pipelines.
///
/// # Errors
///
/// Returns a message with the character position of the first token that
/// does not fit the notation.
pub(crate) fn parse_script(text: &str) -> Result<Vec<Pipeline>, String> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    let mut pipelines = Vec::new();
    loop {
        parser.skip_ws();
        if parser.at_end() {
            break;
        }
        let mut pipeline = vec![parser.call()?];
        loop {
            parser.skip_ws();
            if parser.eat_str("|>") {
                pipeline.push(parser.call()?);
            } else {
                break;
            }
        }
        pipelines.push(pipeline);
        parser.skip_ws();
        if parser.at_end() {
            break;
        }
        if !parser.eat(';') {
            return Err(parser.error("expected `;`, `|>` or the end of the program"));
        }
    }
    if pipelines.is_empty() {
        return Err("empty program".to_owned());
    }
    Ok(pipelines)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

const ESCAPE_NEWLINE: char = 'n';
const ESCAPE_TAB: char = 't';
const ESCAPE_RETURN: char = 'r';
const ESCAPE_NUL: char = '0';
const ESCAPE_UNICODE: char = 'u';

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, expected: &str) -> bool {
        let matches = expected
            .chars()
            .enumerate()
            .all(|(offset, c)| self.peek_at(offset) == Some(c));
        if matches {
            self.pos += expected.chars().count();
        }
        matches
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn error(&self, message: &str) -> String {
        format!("{message} at character {}", self.pos)
    }

    fn at_boundary(&self) -> bool {
        match self.peek() {
            None => true,
            Some(c) => c.is_whitespace() || matches!(c, ',' | ']' | '}' | ';' | '|'),
        }
    }

    fn at_stage_end(&self) -> bool {
        self.at_end() || self.peek() == Some(';') || self.starts_with("|>")
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(offset, c)| self.peek_at(offset) == Some(c))
    }

    fn call(&mut self) -> Result<Call, String> {
        self.skip_ws();
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error("expected a command name"));
        }
        let command: String = self.chars[start..self.pos].iter().collect();
        let mut call = Call {
            command,
            positional: Vec::new(),
            named: Vec::new(),
        };
        loop {
            self.skip_ws();
            if self.at_stage_end() {
                break;
            }
            if let Some(name) = self.named_start() {
                self.skip_ws();
                let value = self.value()?;
                call.named.push((name, value));
            } else {
                call.positional.push(self.value()?);
            }
        }
        Ok(call)
    }

    /// Consumes `name:` followed by white space and returns the name.
    fn named_start(&mut self) -> Option<String> {
        let start = self.pos;
        if !self.peek().is_some_and(|c| c.is_alphabetic() || c == '_') {
            return None;
        }
        let mut end = start;
        while self
            .chars
            .get(end)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            end += 1;
        }
        let colon = self.chars.get(end) == Some(&':');
        let spaced = self.chars.get(end + 1).is_none_or(|c| c.is_whitespace());
        if colon && spaced {
            self.pos = end + 1;
            Some(self.chars[start..end].iter().collect())
        } else {
            None
        }
    }

    fn value(&mut self) -> Result<Lit, String> {
        self.skip_ws();
        match self.peek() {
            None => Err(self.error("expected a value")),
            Some('"') => self.string().map(Lit::Str),
            Some('/') => self.regex(),
            Some('[') => self.list(),
            Some('{') => self.map(),
            Some(c) if c.is_ascii_digit() || c == '-' || c == '.' => Ok(self.number_or_raw()),
            Some(c) if c.is_alphabetic() || c == '_' => Ok(self.word()),
            Some(_) => Ok(self.raw()),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut text = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error("unterminated text"));
            };
            self.pos += 1;
            match c {
                '"' => return Ok(text),
                '\\' => {
                    let Some(escaped) = self.peek() else {
                        return Err(self.error("unterminated escape"));
                    };
                    self.pos += 1;
                    match escaped {
                        ESCAPE_NEWLINE => text.push('\n'),
                        ESCAPE_TAB => text.push('\t'),
                        ESCAPE_RETURN => text.push('\r'),
                        ESCAPE_NUL => text.push('\0'),
                        ESCAPE_UNICODE => text.push(self.unicode_escape()?),
                        other => text.push(other),
                    }
                }
                other => text.push(other),
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        if !self.eat('{') {
            return Err(self.error("expected `{` after `\\u`"));
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        if !self.eat('}') {
            return Err(self.error("expected `}` after the code point"));
        }
        u32::from_str_radix(&digits, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| self.error("invalid code point"))
    }

    fn regex(&mut self) -> Result<Lit, String> {
        self.pos += 1;
        let mut pattern = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error("unterminated regular expression"));
            };
            self.pos += 1;
            match c {
                '/' => break,
                '\\' if self.peek() == Some('/') => {
                    self.pos += 1;
                    pattern.push('/');
                }
                '\\' => {
                    pattern.push('\\');
                    if let Some(next) = self.peek() {
                        self.pos += 1;
                        pattern.push(next);
                    }
                }
                other => pattern.push(other),
            }
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_lowercase()) {
            self.pos += 1;
        }
        let flags = self.chars[start..self.pos].iter().collect();
        Ok(Lit::Regex(pattern, flags))
    }

    fn list(&mut self) -> Result<Lit, String> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.eat(']') {
                return Ok(Lit::List(items));
            }
            items.push(self.value()?);
            self.skip_ws();
            if !self.eat(',') && self.peek() != Some(']') {
                return Err(self.error("expected `,` or `]`"));
            }
        }
    }

    fn map(&mut self) -> Result<Lit, String> {
        self.pos += 1;
        let mut entries = Vec::new();
        loop {
            self.skip_ws();
            if self.eat('}') {
                return Ok(Lit::Map(entries));
            }
            let key = if self.peek() == Some('"') {
                self.string()?
            } else {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
                {
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(self.error("expected a map key"));
                }
                self.chars[start..self.pos].iter().collect()
            };
            self.skip_ws();
            if !self.eat(':') {
                return Err(self.error("expected `:` after the map key"));
            }
            let value = self.value()?;
            entries.push((key, value));
            self.skip_ws();
            if !self.eat(',') && self.peek() != Some('}') {
                return Err(self.error("expected `,` or `}`"));
            }
        }
    }

    fn integer(&mut self) -> Option<i64> {
        let start = self.pos;
        if self.peek() == Some('-') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        match digits.parse() {
            Ok(number) => Some(number),
            Err(_) => {
                self.pos = start;
                None
            }
        }
    }

    fn number_or_raw(&mut self) -> Lit {
        let start = self.pos;
        let first = self.integer();
        let lit = if self.eat_str("..") {
            let second = self.integer();
            Some(Lit::Range(first, second))
        } else if let Some(whole) = first {
            if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
                let text: String = self.chars[start..self.pos].iter().collect();
                text.parse().ok().map(Lit::Float)
            } else {
                Some(Lit::Int(whole))
            }
        } else {
            None
        };
        match lit {
            Some(lit) if self.at_boundary() => lit,
            _ => {
                self.pos = start;
                self.raw()
            }
        }
    }

    fn word(&mut self) -> Lit {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            self.pos += 1;
        }
        if !self.at_boundary() {
            self.pos = start;
            return self.raw();
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        match word.as_str() {
            "true" => Lit::Bool(true),
            "false" => Lit::Bool(false),
            _ => Lit::Ident(word),
        }
    }

    fn raw(&mut self) -> Lit {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, ',' | ']' | '}' | ';'))
        {
            self.pos += 1;
        }
        if self.pos == start {
            self.pos += 1;
        }
        Lit::Raw(self.chars[start..self.pos].iter().collect())
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
