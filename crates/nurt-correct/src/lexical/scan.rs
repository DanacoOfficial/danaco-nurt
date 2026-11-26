// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lossless scanner of program text for correction.
//!
//! The scanner splits any text into consecutive pieces that cover it without
//! gaps. It never fails: notation that the language does not have (single
//! quotes, typographic quotes, bare paths, Unicode spaces) becomes a piece of
//! its own kind, so that the correction rules can rewrite it. Literal bounds
//! follow the lexical rules of the language: text, raw text and regular
//! expressions are recognized first, and nothing inside them is a token.

use std::ops::Range;

use unicode_normalization::char::is_combining_mark;

/// Byte order mark, ignored at the start of a program.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// Characters of zero width that correction removes outside literals.
const INVISIBLE: [char; 5] = [
    '\u{200b}',
    '\u{200c}',
    '\u{200d}',
    '\u{2060}',
    BYTE_ORDER_MARK,
];

/// Line separators that correction turns into a line feed: the Unicode ones
/// and a carriage return that does not precede a line feed, which would
/// otherwise read as a space and join two statements.
const LINE_SEPARATORS: [char; 4] = ['\r', '\u{85}', '\u{2028}', '\u{2029}'];

/// Flags of a regular expression of the language.
const REGEX_FLAGS: &str = "imsx";

/// Characters after which `/` starts a regular expression.
const REGEX_PRECEDERS: &str = "([{,:=~!<>+-*|&";

/// Pieces after which a word stands where a command name does; `>` is the
/// second half of a pipe written `| >`.
const HEAD_PRECEDERS: [&str; 8] = ["|>", "|", "->", "|->", ";", "(", "{", ">"];

/// Closing brackets, which end an operand.
const CLOSING_BRACKETS: [&str; 3] = [")", "]", "}"];

/// Spaces that make a slash after an operand a division; other Unicode
/// spaces keep a following regular expression whole.
const ASCII_SPACES: [char; 4] = [' ', '\t', '\n', '\r'];

/// Multi-character operators, longest first.
const OPERATORS: [&str; 15] = [
    "|->", "...", "|>", "->", "==", "!=", "<=", ">=", "!~", "..", "&&", "||", ">>", "<<", "::",
];

/// Opening typographic quotes and the characters that close them.
const TYPOGRAPHIC_QUOTES: [(char, &str); 8] = [
    ('\u{201e}', "\u{201d}\u{201c}"),
    ('\u{201c}', "\u{201d}\u{201c}"),
    ('\u{201d}', "\u{201d}"),
    ('\u{201a}', "\u{2018}\u{2019}"),
    ('\u{2018}', "\u{2019}\u{2018}"),
    ('\u{2019}', "\u{2019}"),
    ('\u{ab}', "\u{bb}"),
    ('\u{bb}', "\u{ab}"),
];

/// Kind of a piece of program text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PieceKind {
    /// Byte order mark at the start of the text.
    ByteOrderMark,
    /// Spaces and tabulators.
    Space,
    /// Other Unicode white space.
    ForeignSpace,
    /// Unicode line or paragraph separator, or a carriage return without a
    /// line feed.
    LineSeparator,
    /// Character of zero width.
    Invisible,
    /// Line feed, with a preceding carriage return if present.
    Newline,
    /// Comment from `#` to the end of the line.
    Comment,
    /// Text `"…"`.
    Text { terminated: bool },
    /// Raw text `r"…"`, `r#"…"#`.
    Raw { terminated: bool },
    /// Regular expression `/…/flags`.
    Regex,
    /// Unquoted path: `/srv/app`, `C:\x`, `C:/x`, `./x`, `../x`.
    BarePath,
    /// Text in single quotes.
    SingleQuoted,
    /// Text in typographic quotes.
    Typographic,
    /// ASCII word: letters, digits and underscores, starting with a letter or
    /// an underscore.
    Word,
    /// Word with characters outside ASCII.
    ForeignWord,
    /// Number, size or duration, possibly with letters attached.
    Number,
    /// Operator or bracket.
    Punct,
    /// Any other character.
    Other,
}

impl PieceKind {
    /// Whether the piece is a literal whose bytes correction keeps.
    pub(crate) fn is_literal(self) -> bool {
        matches!(
            self,
            Self::Text { .. }
                | Self::Raw { .. }
                | Self::Regex
                | Self::BarePath
                | Self::SingleQuoted
                | Self::Typographic
                | Self::Comment
        )
    }

    /// Whether the piece separates tokens without meaning of its own.
    pub(crate) fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::ByteOrderMark
                | Self::Space
                | Self::ForeignSpace
                | Self::Invisible
                | Self::Comment
        )
    }

    /// Whether the piece ends a complete operand.
    pub(crate) fn ends_operand(self) -> bool {
        matches!(
            self,
            Self::Text { .. }
                | Self::Raw { .. }
                | Self::Regex
                | Self::BarePath
                | Self::SingleQuoted
                | Self::Typographic
                | Self::Word
                | Self::Number
        )
    }
}

/// Consecutive part of a program text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub(crate) kind: PieceKind,
    pub(crate) range: Range<usize>,
}

impl Piece {
    pub(crate) fn text<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.range.clone()).unwrap_or_default()
    }
}

/// Splits a program text into pieces.
pub(crate) fn scan(text: &str) -> Vec<Piece> {
    let mut scanner = Scanner {
        text,
        offset: 0,
        pieces: Vec::new(),
    };
    if text.starts_with(BYTE_ORDER_MARK) {
        scanner.push(PieceKind::ByteOrderMark, BYTE_ORDER_MARK.len_utf8());
    }
    while let Some(c) = scanner.peek() {
        scanner.step(c);
    }
    scanner.pieces
}

struct Scanner<'a> {
    text: &'a str,
    offset: usize,
    pieces: Vec<Piece>,
}

impl Scanner<'_> {
    fn rest(&self) -> &str {
        self.text.get(self.offset..).unwrap_or_default()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn push(&mut self, kind: PieceKind, len: usize) {
        let end = (self.offset + len.max(1)).min(self.text.len());
        let end = ceil_char_boundary(self.text, end);
        self.pieces.push(Piece {
            kind,
            range: self.offset..end,
        });
        self.offset = end;
    }

    fn previous_char(&self) -> Option<char> {
        self.text
            .get(..self.offset)
            .and_then(|before| before.chars().next_back())
    }

    fn step(&mut self, c: char) {
        let rest = self.rest();
        match c {
            ' ' | '\t' => {
                let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                self.push(PieceKind::Space, len);
            }
            '\n' => self.push(PieceKind::Newline, 1),
            '\r' if rest.starts_with("\r\n") => self.push(PieceKind::Newline, 2),
            '#' => {
                let len = rest.find(['\n', '\r']).unwrap_or(rest.len());
                self.push(PieceKind::Comment, len);
            }
            '"' => {
                let (len, terminated) = text_len(rest);
                self.push(PieceKind::Text { terminated }, len);
            }
            'r' if raw_opening(rest).is_some() => {
                let (len, terminated) = raw_len(rest);
                self.push(PieceKind::Raw { terminated }, len);
            }
            '\'' => self.quoted_or_other(single_quoted_len(rest), PieceKind::SingleQuoted, c),
            '/' => self.slash(),
            '.' if self.at_value_start() && relative_path_prefix(rest) => {
                self.push(PieceKind::BarePath, bare_path_len(rest));
            }
            _ if c.is_ascii_digit() => self.push(PieceKind::Number, number_len(rest)),
            _ if self.at_value_start() && drive_path_prefix(rest) => {
                self.push(PieceKind::BarePath, bare_path_len(rest));
            }
            _ if c.is_alphabetic() || c == '_' => self.word(),
            _ if LINE_SEPARATORS.contains(&c) => self.push(PieceKind::LineSeparator, c.len_utf8()),
            _ if INVISIBLE.contains(&c) => self.push(PieceKind::Invisible, c.len_utf8()),
            _ if c.is_whitespace() => self.push(PieceKind::ForeignSpace, c.len_utf8()),
            _ if typographic_closers(c).is_some() => {
                let len = self.typographic_len(c);
                self.quoted_or_other(len, PieceKind::Typographic, c);
            }
            _ if c.is_ascii_punctuation() => {
                let len = OPERATORS
                    .iter()
                    .find(|operator| rest.starts_with(**operator))
                    .map_or(1, |operator| operator.len());
                self.push(PieceKind::Punct, len);
            }
            _ => self.push(PieceKind::Other, c.len_utf8()),
        }
    }

    fn quoted_or_other(&mut self, len: Option<usize>, kind: PieceKind, c: char) {
        let at_word_boundary = self
            .previous_char()
            .is_none_or(|before| !is_word_char(before));
        match len {
            Some(len) if at_word_boundary => self.push(kind, len),
            _ => self.push(PieceKind::Other, c.len_utf8()),
        }
    }

    fn typographic_len(&self, opening: char) -> Option<usize> {
        let closers = typographic_closers(opening)?;
        let rest = self.rest();
        let body = rest.get(opening.len_utf8()..)?;
        let line_end = body.find(['\n', '\r']).unwrap_or(body.len());
        let line = body.get(..line_end)?;
        let (index, closer) = line.char_indices().find(|(_, c)| closers.contains(*c))?;
        let len = opening.len_utf8() + index + closer.len_utf8();
        let after = rest.get(len..).and_then(|tail| tail.chars().next());
        after.is_none_or(|next| !is_word_char(next)).then_some(len)
    }

    fn word(&mut self) {
        let rest = self.rest();
        let len = rest
            .char_indices()
            .find(|&(_, c)| !(is_word_char(c) || is_combining_mark(c)))
            .map_or(rest.len(), |(index, _)| index);
        let word = rest.get(..len).unwrap_or_default();
        let kind = if word.is_ascii() {
            PieceKind::Word
        } else {
            PieceKind::ForeignWord
        };
        self.push(kind, len);
    }

    /// A slash is division only after an operand, either glued to it or
    /// with spaces on both sides; the parser reads a regular expression
    /// after a command name even with a space after the slash. Where the
    /// two readings are close, the slash starts a literal, so that no rule
    /// changes the bytes of a regular expression.
    fn slash(&mut self) {
        let rest = self.rest();
        let spaced_after = rest
            .get(1..)
            .and_then(|tail| tail.chars().next())
            .is_none_or(|next| ASCII_SPACES.contains(&next));
        let glued = self
            .previous_char()
            .is_some_and(|before| !before.is_whitespace() && !REGEX_PRECEDERS.contains(before));
        if self.after_operand() && (glued || spaced_after) {
            self.push(PieceKind::Punct, 1);
            return;
        }
        match regex_len(rest) {
            Some(len) => self.push(PieceKind::Regex, len),
            None if spaced_after => self.push(PieceKind::Punct, 1),
            None => self.push(PieceKind::BarePath, bare_path_len(rest)),
        }
    }

    /// Significant pieces before the current offset, the nearest first.
    fn significant_before(&self) -> impl Iterator<Item = &Piece> {
        self.pieces
            .iter()
            .rev()
            .filter(|piece| !piece.kind.is_trivia())
    }

    /// Whether the nearest significant piece is a complete operand other
    /// than a command name.
    fn after_operand(&self) -> bool {
        let Some(previous) = self.significant_before().next() else {
            return false;
        };
        let closing = previous.kind == PieceKind::Punct
            && CLOSING_BRACKETS.contains(&previous.text(self.text));
        (previous.kind.ends_operand() || closing) && !self.after_head()
    }

    /// Whether the nearest significant piece is a word at the start of a
    /// statement or a stage, where a command name stands.
    fn after_head(&self) -> bool {
        let mut before = self.significant_before();
        let is_word = before
            .next()
            .is_some_and(|piece| matches!(piece.kind, PieceKind::Word | PieceKind::ForeignWord));
        is_word
            && before.next().is_none_or(|piece| {
                piece.kind == PieceKind::Newline
                    || piece.kind == PieceKind::LineSeparator
                    || (piece.kind == PieceKind::Punct
                        && HEAD_PRECEDERS.contains(&piece.text(self.text)))
            })
    }

    fn at_value_start(&self) -> bool {
        self.previous_char()
            .is_none_or(|before| before.is_whitespace() || "([{,:=".contains(before))
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn ceil_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn typographic_closers(opening: char) -> Option<&'static str> {
    TYPOGRAPHIC_QUOTES
        .iter()
        .find(|(quote, _)| *quote == opening)
        .map(|(_, closers)| *closers)
}

/// Length of a text literal starting at `"`, and whether it is closed.
fn text_len(rest: &str) -> (usize, bool) {
    let mut chars = rest.char_indices().skip(1);
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' => return (index + 1, true),
            _ => {}
        }
    }
    (rest.len(), false)
}

/// Number of `#` of a raw text opening `r#…#"`, if `rest` starts with one.
pub(crate) fn raw_opening(rest: &str) -> Option<usize> {
    let after_r = rest.strip_prefix('r')?;
    let hashes = after_r.len() - after_r.trim_start_matches('#').len();
    after_r.get(hashes..)?.starts_with('"').then_some(hashes)
}

/// Length of a raw text literal, and whether it is closed.
fn raw_len(rest: &str) -> (usize, bool) {
    let Some(hashes) = raw_opening(rest) else {
        return (rest.len(), false);
    };
    let body_start = 1 + hashes + 1;
    let closing = format!("\"{}", "#".repeat(hashes));
    match rest.get(body_start..).and_then(|body| body.find(&closing)) {
        Some(index) => (body_start + index + closing.len(), true),
        None => (rest.len(), false),
    }
}

/// Length of a text in single quotes on one line, if closed.
fn single_quoted_len(rest: &str) -> Option<usize> {
    let body = rest.get(1..)?;
    let line_end = body.find(['\n', '\r']).unwrap_or(body.len());
    let close = body.get(..line_end)?.find('\'')?;
    let len = 1 + close + 1;
    let after = rest.get(len..).and_then(|tail| tail.chars().next());
    after.is_none_or(|next| !is_word_char(next)).then_some(len)
}

/// Length of a valid regular expression `/…/flags` at the start of `rest`.
pub(crate) fn regex_len(rest: &str) -> Option<usize> {
    let mut chars = rest.char_indices().skip(1);
    let close = loop {
        let (index, c) = chars.next()?;
        match c {
            '\\' => {
                let (_, escaped) = chars.next()?;
                if escaped == '\n' {
                    return None;
                }
            }
            '/' => break index,
            '\n' | '\r' => return None,
            _ => {}
        }
    };
    if close == 1 {
        return None;
    }
    let after = rest.get(close + 1..)?;
    let flags_len = after
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(after.len());
    let flags = after.get(..flags_len)?;
    let valid_flags = flags.chars().all(|flag| REGEX_FLAGS.contains(flag))
        && flags
            .char_indices()
            .all(|(index, flag)| !flags.get(..index).unwrap_or_default().contains(flag));
    let next = after.get(flags_len..).and_then(|tail| tail.chars().next());
    let delimited = next.is_none_or(|c| !(c == '/' || c == '\\' || c == '.' || is_word_char(c)));
    (valid_flags && delimited).then_some(close + 1 + flags_len)
}

/// Whether `rest` starts with a drive letter and a separator: `C:\`, `C:/`.
fn drive_path_prefix(rest: &str) -> bool {
    let mut chars = rest.chars();
    match (chars.next(), chars.next(), chars.next(), chars.next()) {
        (Some(letter), Some(':'), Some('\\'), next) => {
            letter.is_ascii_alphabetic() && next.is_none_or(|c| !c.is_whitespace())
        }
        (Some(letter), Some(':'), Some('/'), Some(next)) => {
            letter.is_ascii_uppercase() && next != '/' && !next.is_whitespace()
        }
        _ => false,
    }
}

/// Whether `rest` starts a relative path: `./x`, `../x`.
fn relative_path_prefix(rest: &str) -> bool {
    let after_dots = rest.strip_prefix("../").or_else(|| rest.strip_prefix("./"));
    after_dots
        .and_then(|tail| tail.chars().next())
        .is_some_and(|c| !c.is_whitespace())
}

/// Length of an unquoted path: up to white space, a quote, a comma, a pipe or
/// a closing bracket.
fn bare_path_len(rest: &str) -> usize {
    rest.find(|c: char| c.is_whitespace() || "\"'`,|)]};".contains(c) || INVISIBLE.contains(&c))
        .unwrap_or(rest.len())
}

/// Length of a number with attached letters: `10`, `1.5gib`, `1h30m`, `10M`.
fn number_len(rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let continues = byte.is_ascii_alphanumeric()
            || byte == b'_'
            || (byte == b'.' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit));
        if !continues {
            break;
        }
        index += 1;
    }
    index.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(PieceKind, &str)> {
        scan(text)
            .into_iter()
            .map(|piece| {
                let slice = piece.text(text);
                (piece.kind, slice)
            })
            .collect()
    }

    fn significant(text: &str) -> Vec<(PieceKind, &str)> {
        kinds(text)
            .into_iter()
            .filter(|(kind, _)| !kind.is_trivia())
            .collect()
    }

    #[test]
    fn pieces_cover_the_text_without_gaps() {
        let text = "find /srv/app glob: '*.go' | filter size > 10 MB # x\n„a” \u{a0}é";
        let pieces = scan(text);
        let mut offset = 0;
        for piece in &pieces {
            assert_eq!(piece.range.start, offset);
            offset = piece.range.end;
        }
        assert_eq!(offset, text.len());
    }

    #[test]
    fn literals_hide_operators_inside() {
        assert_eq!(
            significant(r##"replace "a -> b" r#"x | y"# /x|y/"##),
            vec![
                (PieceKind::Word, "replace"),
                (PieceKind::Text { terminated: true }, "\"a -> b\""),
                (PieceKind::Raw { terminated: true }, "r#\"x | y\"#"),
                (PieceKind::Regex, "/x|y/"),
            ]
        );
    }

    #[test]
    fn slash_after_operand_without_space_is_division() {
        assert_eq!(
            significant("filter a/b (c)/d"),
            vec![
                (PieceKind::Word, "filter"),
                (PieceKind::Word, "a"),
                (PieceKind::Punct, "/"),
                (PieceKind::Word, "b"),
                (PieceKind::Punct, "("),
                (PieceKind::Word, "c"),
                (PieceKind::Punct, ")"),
                (PieceKind::Punct, "/"),
                (PieceKind::Word, "d"),
            ]
        );
    }

    #[test]
    fn slash_between_spaced_operands_is_division() {
        assert_eq!(
            significant("filter a / b/"),
            vec![
                (PieceKind::Word, "filter"),
                (PieceKind::Word, "a"),
                (PieceKind::Punct, "/"),
                (PieceKind::Word, "b"),
                (PieceKind::Punct, "/"),
            ]
        );
    }

    #[test]
    fn slash_after_command_name_starts_a_regex() {
        for (text, regex) in [
            ("search/ oldName/ \"/srv\"", "/ oldName/"),
            ("search/ Pending/i \"/srv\"", "/ Pending/i"),
            ("find \"/a\" | > search / x/", "/ x/"),
            ("find \"/a\" |> search /\u{a0}x/", "/\u{a0}x/"),
        ] {
            let pieces = significant(text);
            assert!(
                pieces.contains(&(PieceKind::Regex, regex)),
                "{text}: {pieces:?}"
            );
        }
    }

    #[test]
    fn unicode_space_after_slash_keeps_the_regex_whole() {
        assert!(
            significant("find \"/a\" /\u{2003}x/").contains(&(PieceKind::Regex, "/\u{2003}x/"))
        );
    }

    #[test]
    fn spaced_slash_without_closing_is_punctuation() {
        assert_eq!(
            significant("search / x"),
            vec![
                (PieceKind::Word, "search"),
                (PieceKind::Punct, "/"),
                (PieceKind::Word, "x"),
            ]
        );
    }

    #[test]
    fn regex_with_invalid_flags_is_a_bare_path() {
        assert_eq!(
            significant("find /srv/app /etc/ /a/i"),
            vec![
                (PieceKind::Word, "find"),
                (PieceKind::BarePath, "/srv/app"),
                (PieceKind::Regex, "/etc/"),
                (PieceKind::Regex, "/a/i"),
            ]
        );
    }

    #[test]
    fn drive_and_relative_paths_are_bare_paths() {
        assert_eq!(
            significant(r"list C:\proj C:/x ./src ../up"),
            vec![
                (PieceKind::Word, "list"),
                (PieceKind::BarePath, r"C:\proj"),
                (PieceKind::BarePath, "C:/x"),
                (PieceKind::BarePath, "./src"),
                (PieceKind::BarePath, "../up"),
            ]
        );
    }

    #[test]
    fn foreign_quotes_and_operators_are_recognized() {
        assert_eq!(
            significant("'*.go' „a b” | > |-> -> ||"),
            vec![
                (PieceKind::SingleQuoted, "'*.go'"),
                (PieceKind::Typographic, "„a b”"),
                (PieceKind::Punct, "|"),
                (PieceKind::Punct, ">"),
                (PieceKind::Punct, "|->"),
                (PieceKind::Punct, "->"),
                (PieceKind::Punct, "||"),
            ]
        );
    }

    #[test]
    fn apostrophe_inside_a_word_is_not_a_quote() {
        assert_eq!(
            significant("don't"),
            vec![
                (PieceKind::Word, "don"),
                (PieceKind::Other, "'"),
                (PieceKind::Word, "t")
            ]
        );
    }

    #[test]
    fn unicode_spaces_are_classified() {
        assert_eq!(
            kinds("a\u{a0}b\u{200b}c\u{2028}"),
            vec![
                (PieceKind::Word, "a"),
                (PieceKind::ForeignSpace, "\u{a0}"),
                (PieceKind::Word, "b"),
                (PieceKind::Invisible, "\u{200b}"),
                (PieceKind::Word, "c"),
                (PieceKind::LineSeparator, "\u{2028}"),
            ]
        );
    }

    #[test]
    fn unterminated_text_runs_to_the_end() {
        assert_eq!(
            significant("replace \"x\" \"new(\nin: 1"),
            vec![
                (PieceKind::Word, "replace"),
                (PieceKind::Text { terminated: true }, "\"x\""),
                (PieceKind::Text { terminated: false }, "\"new(\nin: 1"),
            ]
        );
    }

    #[test]
    fn numbers_keep_attached_units_and_stop_at_ranges() {
        assert_eq!(
            significant("10mb 1.5gib 1..5 10M"),
            vec![
                (PieceKind::Number, "10mb"),
                (PieceKind::Number, "1.5gib"),
                (PieceKind::Number, "1"),
                (PieceKind::Punct, ".."),
                (PieceKind::Number, "5"),
                (PieceKind::Number, "10M"),
            ]
        );
    }

    #[test]
    fn a_lone_carriage_return_is_a_line_separator_and_a_line_end_pair_is_a_newline() {
        assert_eq!(
            kinds("a\rb"),
            vec![
                (PieceKind::Word, "a"),
                (PieceKind::LineSeparator, "\r"),
                (PieceKind::Word, "b"),
            ]
        );
        assert_eq!(
            kinds("a\r\nb"),
            vec![
                (PieceKind::Word, "a"),
                (PieceKind::Newline, "\r\n"),
                (PieceKind::Word, "b"),
            ]
        );
    }

    #[test]
    fn quotes_without_a_closing_quote_are_other_characters() {
        assert_eq!(
            significant("„abc"),
            vec![(PieceKind::Other, "„"), (PieceKind::Word, "abc")]
        );
        assert_eq!(
            significant("'abc"),
            vec![(PieceKind::Other, "'"), (PieceKind::Word, "abc")]
        );
    }

    #[test]
    fn numbers_stop_before_multibyte_characters_and_words_keep_combining_marks() {
        assert_eq!(
            significant("10é"),
            vec![(PieceKind::Number, "10"), (PieceKind::ForeignWord, "é")]
        );
        assert_eq!(
            significant("e\u{301}x"),
            vec![(PieceKind::ForeignWord, "e\u{301}x")]
        );
    }

    #[test]
    fn byte_order_mark_and_comments_are_trivia() {
        assert_eq!(
            kinds("\u{feff}a # „x”\r\nb"),
            vec![
                (PieceKind::ByteOrderMark, "\u{feff}"),
                (PieceKind::Word, "a"),
                (PieceKind::Space, " "),
                (PieceKind::Comment, "# „x”"),
                (PieceKind::Newline, "\r\n"),
                (PieceKind::Word, "b"),
            ]
        );
    }
}
