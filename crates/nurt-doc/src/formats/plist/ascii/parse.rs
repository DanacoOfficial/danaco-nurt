// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of ASCII (OpenStep) property lists that keeps the byte range of
//! every key, value, separator and bracket.
//!
//! A value is a string, quoted with `"` or `'` or unquoted, data in angle
//! brackets (`<0fbd 7a>`), an array `( a, b )` with an optional trailing
//! comma, or a dictionary `{ key = value; }`. The GNUstep typed values
//! `<*I12>`, `<*R1.5>`, `<*BY>` and `<*D2026-01-31 12:00:00 +0000>` are read
//! as integer, real, truth value and date. Comments `//` and `/* */` may
//! stand wherever white space may. A file whose top level is a sequence of
//! `key = value;` entries without braces, a strings file, is a dictionary.
//!
//! Unquoted strings consist of ASCII letters, digits and `_ $ + / : . -`,
//! the set accepted by CoreFoundation.

use std::ops::Range;

use nurt_contracts::error::Error;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Bytes, Date, DateKind, Fields, Value};

use crate::model::doc_parse;

/// Deepest nesting of arrays and dictionaries accepted.
pub(crate) const MAX_DEPTH: usize = 512;

/// Code points of the NeXTSTEP encoding for bytes 0x80 to 0xFF, used by
/// octal escapes such as `\303`, from the NeXTSTEP mapping table of the
/// Unicode Consortium.
const NEXTSTEP_HIGH: [char; 128] = [
    '\u{A0}', '\u{C0}', '\u{C1}', '\u{C2}', '\u{C3}', '\u{C4}', '\u{C5}', '\u{C7}', '\u{C8}',
    '\u{C9}', '\u{CA}', '\u{CB}', '\u{CC}', '\u{CD}', '\u{CE}', '\u{CF}', '\u{D0}', '\u{D1}',
    '\u{D2}', '\u{D3}', '\u{D4}', '\u{D5}', '\u{D6}', '\u{D9}', '\u{DA}', '\u{DB}', '\u{DC}',
    '\u{DD}', '\u{DE}', '\u{B5}', '\u{D7}', '\u{F7}', '\u{A9}', '\u{A1}', '\u{A2}', '\u{A3}',
    '\u{2044}', '\u{A5}', '\u{192}', '\u{A7}', '\u{A4}', '\u{2019}', '\u{201C}', '\u{AB}',
    '\u{2039}', '\u{203A}', '\u{FB01}', '\u{FB02}', '\u{AE}', '\u{2013}', '\u{2020}', '\u{2021}',
    '\u{B7}', '\u{A6}', '\u{B6}', '\u{2022}', '\u{201A}', '\u{201E}', '\u{201D}', '\u{BB}',
    '\u{2026}', '\u{2030}', '\u{AC}', '\u{BF}', '\u{B9}', '\u{2CB}', '\u{B4}', '\u{2C6}',
    '\u{2DC}', '\u{AF}', '\u{2D8}', '\u{2D9}', '\u{A8}', '\u{B2}', '\u{2DA}', '\u{B8}', '\u{B3}',
    '\u{2DD}', '\u{2DB}', '\u{2C7}', '\u{2014}', '\u{B1}', '\u{BC}', '\u{BD}', '\u{BE}', '\u{E0}',
    '\u{E1}', '\u{E2}', '\u{E3}', '\u{E4}', '\u{E5}', '\u{E7}', '\u{E8}', '\u{E9}', '\u{EA}',
    '\u{EB}', '\u{EC}', '\u{C6}', '\u{ED}', '\u{AA}', '\u{EE}', '\u{EF}', '\u{F0}', '\u{F1}',
    '\u{141}', '\u{D8}', '\u{152}', '\u{BA}', '\u{F2}', '\u{F3}', '\u{F4}', '\u{F5}', '\u{F6}',
    '\u{E6}', '\u{F9}', '\u{FA}', '\u{FB}', '\u{131}', '\u{FC}', '\u{FD}', '\u{142}', '\u{F8}',
    '\u{153}', '\u{DF}', '\u{FE}', '\u{FF}', '\u{FFFD}', '\u{FFFD}',
];

/// First NeXTSTEP code outside ASCII.
const NEXTSTEP_HIGH_START: u32 = 0x80;

/// Hexadecimal digits of a `\U` escape.
const UNICODE_ESCAPE_DIGITS: usize = 4;

/// Largest number of digits of an octal escape.
const OCTAL_ESCAPE_DIGITS: usize = 3;

/// Value with its byte range.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    /// Bytes of the value; for a strings file, from the first entry to the
    /// end of the last one.
    pub(crate) span: Range<usize>,
    /// Kind and content.
    pub(crate) body: Body,
}

/// Kind and content of an [`Item`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Body {
    /// String after unescaping.
    String(String),
    /// Data in angle brackets.
    Data(Vec<u8>),
    /// GNUstep typed value.
    Typed(Value),
    /// Array elements in order.
    Array(Vec<Element>),
    /// Dictionary entries in order.
    Dict(Vec<Entry>),
}

/// Element of an array.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Element {
    /// The value.
    pub(crate) value: Item,
    /// Offset of the comma after the value, when there is one.
    pub(crate) comma: Option<usize>,
}

/// Entry of a dictionary.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    /// The key, always a string.
    pub(crate) key: Item,
    /// Text of the key.
    pub(crate) name: String,
    /// The value.
    pub(crate) value: Item,
    /// Offset after the closing `;`.
    pub(crate) end: usize,
}

impl Item {
    /// Engine value of the item.
    pub(crate) fn to_value(&self) -> Value {
        match &self.body {
            Body::String(text) => Value::Text(text.clone()),
            Body::Data(bytes) => Value::Bytes(Bytes::new(bytes.clone())),
            Body::Typed(value) => value.clone(),
            Body::Array(elements) => Value::List(
                elements
                    .iter()
                    .map(|element| element.value.to_value())
                    .collect(),
            ),
            Body::Dict(entries) => Value::Map(
                entries
                    .iter()
                    .map(|entry| (entry.name.clone(), entry.value.to_value()))
                    .collect::<Fields>(),
            ),
        }
    }
}

/// Parses an ASCII property list. A file holding only white space and
/// comments is an empty dictionary.
///
/// # Errors
///
/// Returns `E_DOC_PARSE` with the line and column of the first error.
pub(crate) fn parse(text: &[u8]) -> Result<Item, Error> {
    let mut parser = Parser {
        text,
        position: 0,
        depth: 0,
    };
    parser.skip_space()?;
    if parser.at_end() {
        let end = parser.position;
        return Ok(Item {
            span: end..end,
            body: Body::Dict(Vec::new()),
        });
    }
    let start = parser.position;
    let first = parser.value()?;
    parser.skip_space()?;
    if parser.at_end() {
        return Ok(first);
    }
    if parser.peek() == Some(b'=') && matches!(first.body, Body::String(_)) {
        parser.position = start;
        let entries = parser.entries(None)?;
        let end = entries.last().map_or(start, |entry| entry.end);
        return Ok(Item {
            span: start..end,
            body: Body::Dict(entries),
        });
    }
    Err(parser.error("unexpected text after the root value; a property list holds one value"))
}

struct Parser<'a> {
    text: &'a [u8],
    position: usize,
    depth: usize,
}

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.position >= self.text.len()
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.position).copied()
    }

    fn error(&self, message: &str) -> Error {
        doc_parse(
            &DocFormatId::PLIST,
            self.text,
            Some(self.position.min(self.text.len())),
            message,
        )
    }

    fn skip_space(&mut self) -> Result<(), Error> {
        loop {
            match self.peek() {
                Some(byte) if is_space(byte) => self.position += 1,
                Some(b'/') if self.text.get(self.position + 1) == Some(&b'/') => {
                    while self.peek().is_some_and(|byte| byte != b'\n') {
                        self.position += 1;
                    }
                }
                Some(b'/') if self.text.get(self.position + 1) == Some(&b'*') => {
                    let body = self.position + 2;
                    let close = find(&self.text[body..], b"*/")
                        .ok_or_else(|| self.error("unterminated comment; close it with */"))?;
                    self.position = body + close + 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn expect(&mut self, byte: u8, message: &str) -> Result<usize, Error> {
        self.skip_space()?;
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(self.position - 1)
        } else {
            Err(self.error(message))
        }
    }

    fn value(&mut self) -> Result<Item, Error> {
        self.skip_space()?;
        let start = self.position;
        let body = match self.peek() {
            Some(b'{') => self.nested(Self::dict)?,
            Some(b'(') => self.nested(Self::array)?,
            Some(b'<') => self.angle()?,
            Some(quote @ (b'"' | b'\'')) => self.quoted(quote)?,
            Some(byte) if is_unquoted(byte) => self.unquoted(),
            Some(_) => {
                return Err(self.error(
                    "expected a value: a string, <data>, ( array ) or { dictionary }; quote strings with other characters",
                ));
            }
            None => {
                return Err(self.error("unexpected end of the property list; a value is missing"));
            }
        };
        Ok(Item {
            span: start..self.position,
            body,
        })
    }

    fn nested(&mut self, parse: fn(&mut Self) -> Result<Body, Error>) -> Result<Body, Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("arrays and dictionaries are nested deeper than 512 levels"));
        }
        let body = parse(self);
        self.depth -= 1;
        body
    }

    fn dict(&mut self) -> Result<Body, Error> {
        self.position += 1;
        let entries = self.entries(Some(b'}'))?;
        self.expect(b'}', "expected } to close the dictionary")?;
        Ok(Body::Dict(entries))
    }

    /// Entries up to `close`, or to the end of the text without `close`.
    fn entries(&mut self, close: Option<u8>) -> Result<Vec<Entry>, Error> {
        let mut entries = Vec::new();
        loop {
            self.skip_space()?;
            if self.peek() == close || (close.is_none() && self.at_end()) {
                return Ok(entries);
            }
            let key = self.value()?;
            let Body::String(name) = &key.body else {
                self.position = key.span.start;
                return Err(self.error("a dictionary key must be a string"));
            };
            let name = name.clone();
            self.expect(b'=', "expected = after the dictionary key")?;
            let value = self.value()?;
            let end = self.expect(b';', "expected ; after the dictionary value")? + 1;
            entries.push(Entry {
                key,
                name,
                value,
                end,
            });
        }
    }

    fn array(&mut self) -> Result<Body, Error> {
        self.position += 1;
        let mut elements: Vec<Element> = Vec::new();
        loop {
            self.skip_space()?;
            if self.peek() == Some(b')') {
                self.position += 1;
                return Ok(Body::Array(elements));
            }
            if elements
                .last()
                .is_some_and(|element| element.comma.is_none())
            {
                return Err(self.error("expected , or ) after the array element"));
            }
            let value = self.value()?;
            self.skip_space()?;
            let comma = (self.peek() == Some(b',')).then_some(self.position);
            if comma.is_some() {
                self.position += 1;
            }
            elements.push(Element { value, comma });
        }
    }

    fn angle(&mut self) -> Result<Body, Error> {
        let body = self.position + 1;
        let close = self.text[body..]
            .iter()
            .position(|&byte| byte == b'>')
            .ok_or_else(|| self.error("unterminated data; close it with >"))?;
        let inner = &self.text[body..body + close];
        let parsed = match inner.strip_prefix(b"*") {
            Some(typed) => typed_value(typed),
            None => hex_data(inner),
        };
        let parsed = parsed.map_err(|message| self.error(message))?;
        self.position = body + close + 1;
        Ok(parsed)
    }

    fn unquoted(&mut self) -> Body {
        let start = self.position;
        while self.peek().is_some_and(is_unquoted) {
            self.position += 1;
        }
        Body::String(String::from_utf8_lossy(&self.text[start..self.position]).into_owned())
    }

    fn quoted(&mut self, quote: u8) -> Result<Body, Error> {
        self.position += 1;
        let mut text: Vec<u8> = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string; close it with a quote")),
                Some(byte) if byte == quote => {
                    self.position += 1;
                    break;
                }
                Some(b'\\') => {
                    self.position += 1;
                    let character = self.escape()?;
                    text.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                }
                Some(byte) => {
                    text.push(byte);
                    self.position += 1;
                }
            }
        }
        Ok(Body::String(String::from_utf8_lossy(&text).into_owned()))
    }

    fn escape(&mut self) -> Result<char, Error> {
        let Some(byte) = self.peek() else {
            return Err(self.error("unterminated escape sequence"));
        };
        self.position += 1;
        let character = match byte {
            b'a' => '\u{7}',
            b'b' => '\u{8}',
            b'f' => '\u{C}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'v' => '\u{B}',
            b'U' | b'u' => self.unicode_escape()?,
            b'0'..=b'7' => self.octal_escape(byte),
            other => char::from(other),
        };
        Ok(character)
    }

    fn unicode_escape(&mut self) -> Result<char, Error> {
        let unit = self
            .text
            .get(self.position..self.position + UNICODE_ESCAPE_DIGITS)
            .filter(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .ok_or_else(|| self.error("\\U must be followed by four hexadecimal digits"))?;
        self.position += UNICODE_ESCAPE_DIGITS;
        Ok(char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    fn octal_escape(&mut self, first: u8) -> char {
        let mut code = u32::from(first - b'0');
        for _ in 1..OCTAL_ESCAPE_DIGITS {
            match self.peek() {
                Some(digit @ b'0'..=b'7') => {
                    code = code * 8 + u32::from(digit - b'0');
                    self.position += 1;
                }
                _ => break,
            }
        }
        nextstep_char(code)
    }
}

/// Character of a NeXTSTEP code.
fn nextstep_char(code: u32) -> char {
    code.checked_sub(NEXTSTEP_HIGH_START)
        .and_then(|index| NEXTSTEP_HIGH.get(usize::try_from(index).ok()?).copied())
        .unwrap_or_else(|| char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER))
}

fn typed_value(inner: &[u8]) -> Result<Body, &'static str> {
    let (&letter, rest) = inner.split_first().ok_or("typed value <*…> is empty")?;
    let rest = std::str::from_utf8(rest).map_err(|_| "typed value <*…> is not text")?;
    let value = match letter {
        b'I' => rest
            .trim()
            .parse()
            .map(Value::Int)
            .map_err(|_| "<*I…> must hold an integer")?,
        b'R' => rest
            .trim()
            .parse()
            .map(Value::Number)
            .map_err(|_| "<*R…> must hold a number")?,
        b'B' => match rest.trim() {
            "Y" => Value::Bool(true),
            "N" => Value::Bool(false),
            _ => return Err("<*B…> must hold Y or N"),
        },
        b'D' => Value::Date(Date {
            kind: DateKind::OffsetDateTime,
            text: rest.to_owned(),
        }),
        _ => return Err("typed value must be <*I…>, <*R…>, <*B…> or <*D…>"),
    };
    Ok(Body::Typed(value))
}

fn hex_data(inner: &[u8]) -> Result<Body, &'static str> {
    let digits: Vec<u8> = inner
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if !digits.len().is_multiple_of(2) {
        return Err("data must hold an even number of hexadecimal digits");
    }
    digits
        .chunks(2)
        .map(|pair| Ok((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect::<Result<Vec<u8>, _>>()
        .map(Body::Data)
}

fn hex_digit(byte: u8) -> Result<u8, &'static str> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
        .ok_or("data may hold only hexadecimal digits and white space")
}

/// White space between tokens.
pub(crate) const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0B' | b'\x0C')
}

/// Byte of an unquoted string.
pub(crate) const fn is_unquoted(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'+' | b'/' | b':' | b'.' | b'-')
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Value {
        parse(text.as_bytes()).unwrap().to_value()
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn map(value: Value) -> Fields {
        match value {
            Value::Map(fields) => fields,
            other => panic!("map expected, found {other:?}"),
        }
    }

    #[test]
    fn reads_dictionaries_arrays_and_strings() {
        let fields = map(value(
            "{ name = \"My App\"; list = (a, 'b c', ); nested = { k = v; }; }",
        ));
        assert_eq!(fields["name"], text("My App"));
        assert_eq!(fields["list"], Value::List(vec![text("a"), text("b c")]));
        assert_eq!(
            fields.keys().collect::<Vec<_>>(),
            ["name", "list", "nested"]
        );
    }

    #[test]
    fn reads_data_typed_values_and_escapes() {
        let fields = map(value(
            "{ d = <0fbd 7a>; i = <*I-3>; r = <*R1.5>; b = <*BY>; n = <*BN>; t = <*D2026-01-31 12:00:00 +0000>; s = \"a\\n\\U00e9\\101\\341\\\"\"; }",
        ));
        assert_eq!(
            fields["d"],
            Value::Bytes(Bytes::new(vec![0x0f, 0xbd, 0x7a]))
        );
        assert_eq!(fields["i"], Value::Int(-3));
        assert_eq!(fields["r"], Value::Number(1.5));
        assert_eq!(fields["b"], Value::Bool(true));
        assert_eq!(fields["n"], Value::Bool(false));
        assert!(
            matches!(&fields["t"], Value::Date(date) if date.text == "2026-01-31 12:00:00 +0000")
        );
        assert_eq!(fields["s"], text("a\n\u{e9}A\u{c6}\""));
    }

    #[test]
    fn comments_are_white_space_and_urls_stay_unquoted() {
        let fields = map(value(
            "// head\n{ /* a */ url = http://example.org/x; // tail\n}",
        ));
        assert_eq!(fields["url"], text("http://example.org/x"));
    }

    #[test]
    fn strings_file_is_a_braceless_dictionary() {
        let item = parse(b"\"a\" = \"1\";\nb = 2;\n").unwrap();
        assert_eq!(item.span, 0..17);
        assert_eq!(map(item.to_value())["b"], text("2"));
        assert_eq!(
            parse(b"  // only a comment\n").unwrap().to_value(),
            Value::Map(Fields::new())
        );
    }

    #[test]
    fn records_ranges_of_entries_and_commas() {
        let source = "{ a = (x, y); }";
        let item = parse(source.as_bytes()).unwrap();
        let Body::Dict(entries) = &item.body else {
            panic!("map expected")
        };
        assert_eq!(&source[entries[0].key.span.clone()], "a");
        assert_eq!(&source[entries[0].value.span.clone()], "(x, y)");
        assert_eq!(entries[0].end, 13);
        let Body::Array(elements) = &entries[0].value.body else {
            panic!("array expected")
        };
        assert_eq!(elements[0].comma, Some(8));
        assert_eq!(elements[1].comma, None);
    }

    #[test]
    fn reports_errors_with_position() {
        let cases = [
            "{ a = b }",
            "( a b )",
            "{ (a) = b; }",
            "<abc>",
            "<zz>",
            "\"open",
            "{ a = b; } c",
            "/* x",
            "{ a = ",
            "<*Ix>",
            "<*Q1>",
            "<*BX>",
            "<*>",
            "\"\\U12\"",
            "\"\\U+123\"",
            "\"\\U-123\"",
            "\"\\u 123\"",
            "#",
        ];
        for source in cases {
            let error = parse(source.as_bytes()).unwrap_err();
            assert_eq!(error.code().as_str(), "E_DOC_PARSE", "input {source:?}");
        }
    }

    #[test]
    fn rejects_excessive_nesting() {
        let source = "(".repeat(MAX_DEPTH + 1);
        assert_eq!(
            parse(source.as_bytes()).unwrap_err().code().as_str(),
            "E_DOC_PARSE"
        );
    }

    #[test]
    fn nextstep_codes_map_to_unicode() {
        assert_eq!(nextstep_char(0x41), 'A');
        assert_eq!(nextstep_char(0x80), '\u{A0}');
        assert_eq!(nextstep_char(0xFF), '\u{FFFD}');
    }
}
