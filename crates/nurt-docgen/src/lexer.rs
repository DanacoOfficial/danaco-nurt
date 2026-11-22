// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lexer for programs in variant A of the language.
//!
//! The lexer serves the static check of documentation examples. It follows
//! the lexical rules of the grammar: texts, raw texts, regular expressions,
//! numbers with units, ranges, references, operators and brackets.

/// Kind of a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Identifier or dotted command name, including keywords such as `and`.
    Word(String),
    /// Text literal, with the unescaped content as written between the quotes.
    Text(String),
    /// Regular expression literal; the flags are not kept.
    Regex(String),
    /// Integer literal.
    Int(String),
    /// Decimal number literal.
    Number(String),
    /// Size literal such as `10mb`.
    Size(String),
    /// Duration literal such as `1h30m`.
    Duration(String),
    /// Range literal such as `1..5` or `..9`.
    Range(String),
    /// Reference to a named result, such as `@recent` or `@x.path`.
    Reference(String),
    /// Opening or closing bracket, comma or colon.
    Punct(char),
    /// Pipeline operator `|>`.
    Pipe,
    /// Operator such as `==`, `+` or `~`.
    Operator(String),
    /// Line break.
    Newline,
}

/// Token with the information whether whitespace precedes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Kind of the token.
    pub kind: Kind,
    /// Whitespace precedes the token.
    pub space_before: bool,
}

/// Size units accepted after a number.
const SIZE_UNITS: &[&str] = &["b", "kb", "mb", "gb", "tb", "kib", "mib", "gib", "tib"];
/// Duration units accepted after a number.
const DURATION_UNITS: &[&str] = &["ms", "s", "m", "h", "d", "w"];

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
    problems: Vec<String>,
}

/// Splits a program into tokens.
///
/// Returns the tokens and the lexical problems found; a program with
/// problems has an incomplete token list.
#[must_use]
pub fn tokenize(program: &str) -> (Vec<Token>, Vec<String>) {
    let mut lexer = Lexer {
        chars: program.chars().collect(),
        pos: 0,
        tokens: Vec::new(),
        problems: Vec::new(),
    };
    lexer.run();
    (lexer.tokens, lexer.problems)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl Lexer {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn push(&mut self, kind: Kind, space_before: bool) {
        self.tokens.push(Token { kind, space_before });
    }

    fn run(&mut self) {
        let mut space = true;
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                self.push(Kind::Newline, space);
                self.pos += 1;
                space = true;
            } else if c.is_whitespace() {
                self.pos += 1;
                space = true;
            } else if c == '#' {
                while self.peek(0).is_some_and(|x| x != '\n') {
                    self.pos += 1;
                }
            } else {
                if !self.token(c, space) {
                    return;
                }
                space = false;
            }
        }
    }

    fn token(&mut self, c: char, space: bool) -> bool {
        match c {
            '"' => self.text(space),
            '/' if self.regex_start(space) => self.regex(space),
            '|' if self.peek(1) == Some('>') => {
                self.pos += 2;
                self.push(Kind::Pipe, space);
                true
            }
            '@' => self.reference(space),
            '0'..='9' => self.number(space),
            '.' if self.peek(1) == Some('.') => self.number(space),
            c if is_ident_start(c) => self.word(space),
            '(' | ')' | '[' | ']' | '{' | '}' | ',' | ':' => {
                self.pos += 1;
                self.push(Kind::Punct(c), space);
                true
            }
            _ => self.operator(c, space),
        }
    }

    fn text(&mut self, space: bool) -> bool {
        self.pos += 1;
        let mut content = String::new();
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                '"' => {
                    self.push(Kind::Text(content), space);
                    return true;
                }
                '\\' => {
                    content.push('\\');
                    if let Some(next) = self.peek(0) {
                        content.push(next);
                        self.pos += 1;
                    }
                }
                '\n' => break,
                _ => content.push(c),
            }
        }
        self.problems.push("unterminated text".to_string());
        false
    }

    fn raw_text(&mut self, space: bool) -> bool {
        let start = self.pos;
        let mut offset = 1;
        while self.peek(offset) == Some('#') {
            offset += 1;
        }
        if self.peek(offset) != Some('"') {
            self.pos = start;
            return self.word_body(space);
        }
        let hashes = offset - 1;
        self.pos += offset + 1;
        let closing: Vec<char> = std::iter::once('"')
            .chain(std::iter::repeat_n('#', hashes))
            .collect();
        let mut content = String::new();
        while self.pos < self.chars.len() {
            if self.chars[self.pos..].starts_with(&closing) {
                self.pos += closing.len();
                self.push(Kind::Text(content), space);
                return true;
            }
            content.push(self.chars[self.pos]);
            self.pos += 1;
        }
        self.problems.push("unterminated raw text".to_string());
        false
    }

    fn regex_start(&self, space: bool) -> bool {
        let before_ok = space
            || self.tokens.last().is_none_or(|t| {
                matches!(
                    t.kind,
                    Kind::Punct('(' | '[' | '{' | ',' | ':') | Kind::Operator(_) | Kind::Pipe
                )
            });
        before_ok && self.peek(1).is_some_and(|c| !c.is_whitespace())
    }

    fn regex(&mut self, space: bool) -> bool {
        self.pos += 1;
        let mut content = String::new();
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                '\\' => {
                    content.push('\\');
                    if let Some(next) = self.peek(0) {
                        content.push(next);
                        self.pos += 1;
                    }
                }
                '/' => {
                    while self.peek(0).is_some_and(|f| "imsx".contains(f)) {
                        self.pos += 1;
                    }
                    self.push(Kind::Regex(content), space);
                    return true;
                }
                '\n' => break,
                _ => content.push(c),
            }
        }
        self.problems
            .push("unterminated regular expression".to_string());
        false
    }

    fn reference(&mut self, space: bool) -> bool {
        self.pos += 1;
        let mut name = String::new();
        while let Some(c) = self.peek(0) {
            if is_ident_char(c) || (c == '.' && self.peek(1).is_some_and(is_ident_start)) {
                name.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        if name.is_empty() {
            self.problems.push("reference without a name".to_string());
            return false;
        }
        self.push(Kind::Reference(name), space);
        true
    }

    fn number(&mut self, space: bool) -> bool {
        let start = self.pos;
        if self.peek(0) == Some('.') {
            self.pos += 2;
            let digits = self.digits();
            self.push(Kind::Range(format!("..{digits}")), space);
            return true;
        }
        let whole = self.digits();
        if self.peek(0) == Some('.') && self.peek(1) == Some('.') {
            self.pos += 2;
            let end = self.digits();
            self.push(Kind::Range(format!("{whole}..{end}")), space);
            return true;
        }
        let mut text = whole;
        let mut decimal = false;
        if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
            text.push('.');
            text.push_str(&self.digits());
            decimal = true;
        }
        let unit = self.unit_letters();
        if unit.is_empty() {
            let kind = if decimal {
                Kind::Number(text)
            } else {
                Kind::Int(text)
            };
            self.push(kind, space);
            return true;
        }
        self.classify_unit(&text, &unit, start, space)
    }

    fn classify_unit(&mut self, number: &str, unit: &str, start: usize, space: bool) -> bool {
        let literal = format!("{number}{unit}");
        if SIZE_UNITS.contains(&unit) {
            self.push(Kind::Size(literal), space);
            return true;
        }
        if DURATION_UNITS.contains(&unit) {
            let mut full = literal;
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                let more = self.digits();
                let next_unit = self.unit_letters();
                if !DURATION_UNITS.contains(&next_unit.as_str()) {
                    self.problems
                        .push(format!("invalid duration near {more}{next_unit}"));
                    return false;
                }
                full.push_str(&format!("{more}{next_unit}"));
            }
            self.push(Kind::Duration(full), space);
            return true;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        self.problems.push(format!("invalid number or unit {text}"));
        false
    }

    fn digits(&mut self) -> String {
        let mut digits = String::new();
        while let Some(c) = self.peek(0) {
            if c.is_ascii_digit() || c == '_' {
                digits.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        digits
    }

    fn unit_letters(&mut self) -> String {
        let mut unit = String::new();
        while let Some(c) = self.peek(0) {
            if c.is_ascii_lowercase() {
                unit.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        unit
    }

    fn word(&mut self, space: bool) -> bool {
        if self.peek(0) == Some('r') && matches!(self.peek(1), Some('"' | '#')) {
            return self.raw_text(space);
        }
        self.word_body(space)
    }

    fn word_body(&mut self, space: bool) -> bool {
        let mut name = String::new();
        while let Some(c) = self.peek(0) {
            let dotted = c == '.' && self.peek(1).is_some_and(is_ident_start) && !name.is_empty();
            if is_ident_char(c) || dotted {
                name.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        self.push(Kind::Word(name), space);
        true
    }

    fn operator(&mut self, c: char, space: bool) -> bool {
        let two: String = [c, self.peek(1).unwrap_or(' ')].iter().collect();
        if matches!(two.as_str(), "==" | "!=" | "<=" | ">=" | "!~") {
            self.pos += 2;
            self.push(Kind::Operator(two), space);
            return true;
        }
        if matches!(c, '<' | '>' | '~' | '+' | '-' | '*' | '/' | '=') {
            self.pos += 1;
            self.push(Kind::Operator(c.to_string()), space);
            return true;
        }
        self.problems.push(format!("unexpected character {c}"));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(program: &str) -> Vec<Kind> {
        let (tokens, problems) = tokenize(program);
        assert!(problems.is_empty(), "{problems:?}");
        tokens.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_command_text_and_named_argument() {
        assert_eq!(
            kinds("find \"/a\" glob: \"*.go\""),
            vec![
                Kind::Word("find".into()),
                Kind::Text("/a".into()),
                Kind::Word("glob".into()),
                Kind::Punct(':'),
                Kind::Text("*.go".into()),
            ]
        );
    }

    #[test]
    fn lexes_regex_after_space_and_division_between_spaces() {
        assert_eq!(
            kinds("search /a\\(/i"),
            vec![Kind::Word("search".into()), Kind::Regex("a\\(".into())]
        );
        assert_eq!(
            kinds("size / 2"),
            vec![
                Kind::Word("size".into()),
                Kind::Operator("/".into()),
                Kind::Int("2".into())
            ]
        );
    }

    #[test]
    fn lexes_sizes_durations_and_ranges() {
        assert_eq!(
            kinds("1mb 1h30m 1.5gib 3..5 ..9 40.. 7"),
            vec![
                Kind::Size("1mb".into()),
                Kind::Duration("1h30m".into()),
                Kind::Size("1.5gib".into()),
                Kind::Range("3..5".into()),
                Kind::Range("..9".into()),
                Kind::Range("40..".into()),
                Kind::Int("7".into()),
            ]
        );
    }

    #[test]
    fn lexes_raw_text_with_hashes() {
        assert_eq!(
            kinds("r#\"import \"fmt\"\"#"),
            vec![Kind::Text("import \"fmt\"".into())]
        );
    }

    #[test]
    fn lexes_pipe_reference_and_comment() {
        assert_eq!(
            kinds("@x.path |> count # note"),
            vec![
                Kind::Reference("x.path".into()),
                Kind::Pipe,
                Kind::Word("count".into())
            ]
        );
    }

    #[test]
    fn lexes_dotted_command_names() {
        assert_eq!(kinds("git.status"), vec![Kind::Word("git.status".into())]);
    }

    #[test]
    fn reports_unterminated_text() {
        let (_, problems) = tokenize("find \"/a");
        assert_eq!(problems, vec!["unterminated text"]);
    }

    #[test]
    fn reports_unknown_unit() {
        let (_, problems) = tokenize("first 10xx");
        assert_eq!(problems, vec!["invalid number or unit 10xx"]);
    }

    #[test]
    fn marks_whitespace_before_tokens() {
        let (tokens, _) = tokenize("ago(1h)");
        assert!(!tokens[1].space_before);
    }
}
