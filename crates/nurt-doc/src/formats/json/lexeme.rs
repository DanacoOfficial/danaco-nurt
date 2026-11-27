// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::json::lexeme
// Responsibility:   Number and string lexemes of RFC 8259.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Number and string lexemes of RFC 8259.
//!
//! Numbers keep their source text: a lexeme becomes [`Value::Int`] or
//! [`Value::Number`] only when printing the binary form gives the same text
//! back, so reading and writing a value unchanged never alters bytes.

use std::fmt::Write as _;

use nurt_contracts::value::Value;

/// Reason a lexeme is not valid JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexemeError {
    /// Byte offset of the offending character, relative to the lexeme start.
    pub(crate) offset: usize,
    /// Description of the problem in the form of an error message.
    pub(crate) message: &'static str,
}

impl LexemeError {
    const fn new(offset: usize, message: &'static str) -> Self {
        Self { offset, message }
    }
}

/// Checks a number lexeme against the grammar of RFC 8259, section 6.
pub(crate) fn validate_number(text: &str) -> Result<(), LexemeError> {
    let bytes = text.as_bytes();
    let mut pos = usize::from(bytes.first() == Some(&b'-'));
    match bytes.get(pos) {
        Some(b'0') => {
            pos += 1;
            if bytes.get(pos).is_some_and(u8::is_ascii_digit) {
                return Err(LexemeError::new(
                    pos,
                    "leading zeros are not allowed in numbers",
                ));
            }
        }
        Some(b'1'..=b'9') => pos = skip_digits(bytes, pos),
        _ => return Err(LexemeError::new(pos, "a number needs a digit here")),
    }
    if bytes.get(pos) == Some(&b'.') {
        pos += 1;
        pos = require_digits(bytes, pos, "a fraction needs at least one digit")?;
    }
    if matches!(bytes.get(pos), Some(b'e' | b'E')) {
        pos += 1;
        if matches!(bytes.get(pos), Some(b'+' | b'-')) {
            pos += 1;
        }
        pos = require_digits(bytes, pos, "an exponent needs at least one digit")?;
    }
    if pos == bytes.len() {
        Ok(())
    } else {
        Err(LexemeError::new(pos, "unexpected character in number"))
    }
}

fn skip_digits(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
        pos += 1;
    }
    pos
}

fn require_digits(bytes: &[u8], pos: usize, message: &'static str) -> Result<usize, LexemeError> {
    let end = skip_digits(bytes, pos);
    if end == pos {
        Err(LexemeError::new(pos, message))
    } else {
        Ok(end)
    }
}

/// Checks the source text of a string, quotes included, against RFC 8259,
/// section 7: double quotes, no raw control characters and only the escapes
/// the RFC defines.
pub(crate) fn validate_string(text: &str) -> Result<(), LexemeError> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'"') {
        return Err(LexemeError::new(
            0,
            "strings must be enclosed in double quotes",
        ));
    }
    let mut pos = 1;
    while let Some(&byte) = bytes.get(pos) {
        match byte {
            b'"' if pos + 1 == bytes.len() => return Ok(()),
            b'"' => return Err(LexemeError::new(pos, "unexpected quote inside string")),
            b'\\' => pos = validate_escape(bytes, pos)?,
            0x00..=0x1F => {
                return Err(LexemeError::new(
                    pos,
                    "control characters must be escaped in strings",
                ));
            }
            _ => pos += 1,
        }
    }
    Err(LexemeError::new(pos, "string is not terminated"))
}

/// Validates the escape starting at `pos` (the backslash) and returns the
/// position after it.
fn validate_escape(bytes: &[u8], pos: usize) -> Result<usize, LexemeError> {
    match bytes.get(pos + 1) {
        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => Ok(pos + 2),
        Some(b'u') => {
            let digits = bytes.get(pos + 2..pos + UNICODE_ESCAPE_LEN);
            if digits.is_some_and(|d| d.iter().all(u8::is_ascii_hexdigit)) {
                Ok(pos + UNICODE_ESCAPE_LEN)
            } else {
                Err(LexemeError::new(
                    pos,
                    "\\u must be followed by four hexadecimal digits",
                ))
            }
        }
        _ => Err(LexemeError::new(
            pos,
            "invalid escape; use \\\" \\\\ \\/ \\b \\f \\n \\r \\t or \\uXXXX",
        )),
    }
}

/// Length of a `\uXXXX` escape in bytes.
const UNICODE_ESCAPE_LEN: usize = 6;

/// Converts a valid number lexeme into the value that prints back as the
/// same text.
pub(crate) fn number_value(text: &str) -> Value {
    let integral = !text.contains(['.', 'e', 'E']);
    if integral {
        if let Ok(value) = text.parse::<i64>() {
            if value.to_string() == text {
                return Value::Int(value);
            }
        } else {
            return Value::BigInt(text.to_owned());
        }
    }
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() && format_f64(value) == text => Value::Number(value),
        _ => Value::Decimal(text.to_owned()),
    }
}

/// Prints a finite `f64` in the shortest form that reads back as the same
/// number, as `serde_json` does.
pub(crate) fn format_f64(value: f64) -> String {
    serde_json::Number::from_f64(value).map_or_else(String::new, |number| number.to_string())
}

/// Writes `text` as a JSON string with the minimal escaping RFC 8259
/// requires: the quote, the backslash and control characters.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_rfc_number_forms() {
        for text in [
            "0",
            "-0",
            "12",
            "-1.5",
            "1e9",
            "1E+2",
            "0.5e-3",
            "123456789012345678901234567890",
        ] {
            assert_eq!(validate_number(text), Ok(()), "{text}");
        }
    }

    #[test]
    fn rejects_non_rfc_number_forms() {
        for text in [
            "01", "-", "1.", ".5", "+1", "1e", "1e+", "0x1F", "Infinity", "1.0.0", "-01",
        ] {
            assert!(validate_number(text).is_err(), "{text}");
        }
    }

    #[test]
    fn keeps_number_lexemes_that_binary_forms_would_change() {
        assert_eq!(number_value("42"), Value::Int(42));
        assert_eq!(number_value("-7"), Value::Int(-7));
        assert_eq!(number_value("1.5"), Value::Number(1.5));
        assert_eq!(number_value("1.10"), Value::Decimal("1.10".to_owned()));
        assert_eq!(number_value("1e400"), Value::Decimal("1e400".to_owned()));
        assert_eq!(number_value("1E2"), Value::Decimal("1E2".to_owned()));
        assert_eq!(number_value("-0"), Value::Decimal("-0".to_owned()));
        assert_eq!(
            number_value("18446744073709551616"),
            Value::BigInt("18446744073709551616".to_owned())
        );
    }

    #[test]
    fn validates_string_escapes() {
        assert_eq!(validate_string(r#""a\"b\\c\/dé""#), Ok(()));
        assert!(validate_string(r#""\x41""#).is_err());
        assert!(validate_string(r#""\u12""#).is_err());
        assert!(validate_string("\"a\tb\"").is_err());
        assert!(validate_string("'a'").is_err());
        assert!(validate_string("\"abc").is_err());
    }

    #[test]
    fn quotes_with_minimal_escaping() {
        assert_eq!(quote("a/b é"), "\"a/b é\"");
        assert_eq!(quote("q\"\\\n\u{1}"), "\"q\\\"\\\\\\n\\u0001\"");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
