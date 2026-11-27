// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::json::reference
// Responsibility:   Reference reading with `serde_json`, independent of the parser used for.
// Dependencies:     indexmap, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reference reading with `serde_json`, independent of the parser used for
//! editing; edited documents are read again with it and compared with the
//! expected value.
//!
//! JSONC text is first turned into JSON of the same length: comments and
//! trailing commas become spaces, line breaks stay. Numbers are then passed
//! to `serde_json` as marked strings, so that lexemes outside the range of
//! `f64` and integers beyond 64 bits keep their exact text; every string of
//! the document gets a different marker, which keeps the two apart.

use std::borrow::Cow;

use indexmap::IndexMap;
use nurt_contracts::value::Value;

use super::Dialect;
use super::lexeme;

/// First character of a string that carries a number lexeme.
const NUMBER_MARK: char = '#';

/// First character of a string that carries document text.
const TEXT_MARK: char = '$';

/// Reads document text in the given dialect with `serde_json`. Fails with
/// the `serde_json` error message, or the message of an invalid number
/// lexeme, when the text is not valid in the dialect.
pub(crate) fn read(text: &str, dialect: Dialect) -> Result<Value, String> {
    let plain = match dialect {
        Dialect::Json => Cow::Borrowed(text),
        Dialect::Jsonc => Cow::Owned(strip_jsonc(text)),
    };
    let marked = mark(&plain)?;
    serde_json::from_str::<serde_json::Value>(&marked)
        .map(restore)
        .map_err(|error| error.to_string())
}

/// Wraps every number in a string starting with [`NUMBER_MARK`] and
/// prefixes every string with [`TEXT_MARK`].
fn mark(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 4);
    let mut pos = 0;
    while pos < bytes.len() {
        match bytes[pos] {
            b'"' => {
                let end = string_end(bytes, pos);
                out.push(b'"');
                out.push(TEXT_MARK as u8);
                out.extend_from_slice(&bytes[pos + 1..end]);
                pos = end;
            }
            b'-' | b'0'..=b'9' => {
                let len = bytes[pos..]
                    .iter()
                    .position(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
                    .unwrap_or(bytes.len() - pos);
                let number = &text[pos..pos + len];
                lexeme::validate_number(number).map_err(|error| error.message.to_owned())?;
                out.extend_from_slice(format!("\"{NUMBER_MARK}{number}\"").as_bytes());
                pos += len;
            }
            byte => {
                out.push(byte);
                pos += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|error| error.to_string())
}

/// Converts the marked `serde_json` value back.
fn restore(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::String(text) => {
            if let Some(number) = text.strip_prefix(NUMBER_MARK) {
                lexeme::number_value(number)
            } else {
                Value::Text(text.strip_prefix(TEXT_MARK).unwrap_or(&text).to_owned())
            }
        }
        serde_json::Value::Array(items) => Value::List(items.into_iter().map(restore).collect()),
        serde_json::Value::Object(fields) => {
            let mut map = IndexMap::with_capacity(fields.len());
            for (key, item) in fields {
                let key = key
                    .strip_prefix(TEXT_MARK)
                    .map_or_else(|| key.clone(), str::to_owned);
                map.insert(key, restore(item));
            }
            Value::Map(map)
        }
        other => Value::from_plain_json(other),
    }
}

/// Replaces comments and trailing commas of JSONC text with spaces.
fn strip_jsonc(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut pos = 0;
    while pos < bytes.len() {
        pos = match comment_end(&bytes, pos) {
            Some(end) => {
                blank(&mut bytes[pos..end]);
                end
            }
            None => match bytes[pos] {
                b'"' => string_end(&bytes, pos),
                b',' if closes_after(&bytes, pos + 1) => {
                    bytes[pos] = b' ';
                    pos + 1
                }
                _ => pos + 1,
            },
        };
    }
    String::from_utf8(bytes).unwrap_or_default()
}

/// End of the comment starting at `pos`, if one starts there; a line
/// comment ends before its line break.
fn comment_end(bytes: &[u8], pos: usize) -> Option<usize> {
    if bytes.get(pos) != Some(&b'/') {
        return None;
    }
    match bytes.get(pos + 1) {
        Some(b'/') => Some(
            bytes[pos..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |n| pos + n),
        ),
        Some(b'*') => Some(
            bytes[pos + 2..]
                .windows(2)
                .position(|pair| pair == b"*/")
                .map_or(bytes.len(), |n| pos + 2 + n + 2),
        ),
        _ => None,
    }
}

/// Turns every byte except line breaks into a space; multi-byte characters
/// become several spaces, so the result stays valid UTF-8.
fn blank(bytes: &mut [u8]) {
    for byte in bytes {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

/// Offset after the closing quote of the string starting at `pos`.
fn string_end(bytes: &[u8], mut pos: usize) -> usize {
    pos += 1;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' => pos += 2,
            b'"' => return pos + 1,
            _ => pos += 1,
        }
    }
    bytes.len()
}

/// The next character after whitespace and comments closes a container.
fn closes_after(bytes: &[u8], mut pos: usize) -> bool {
    loop {
        if let Some(end) = comment_end(bytes, pos) {
            pos = end;
            continue;
        }
        match bytes.get(pos) {
            Some(b' ' | b'\t' | b'\r' | b'\n') => pos += 1,
            Some(b'}' | b']') => return true,
            _ => return false,
        }
    }
}

/// Compares two values as JSON data: numbers by exact decimal value,
/// whatever their representation (`Int`, `BigInt`, `Number`, `Decimal`,
/// `1.10` equals `1.1`, `-0` equals `0`); maps by entries in order.
#[cfg(test)]
pub(crate) fn same_data(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_data(x, y))
        }
        (Value::Map(a), Value::Map(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ka, va), (kb, vb))| ka == kb && same_data(va, vb))
        }
        _ => match (number_text(left), number_text(right)) {
            (Some(a), Some(b)) => match (decimal(&a), decimal(&b)) {
                (Some(x), Some(y)) => x == y,
                _ => a == b,
            },
            (None, None) => left == right,
            _ => false,
        },
    }
}

#[cfg(test)]
fn number_text(value: &Value) -> Option<Cow<'_, str>> {
    match value {
        Value::Int(number) => Some(Cow::Owned(number.to_string())),
        Value::Number(number) => Some(Cow::Owned(lexeme::format_f64(*number))),
        Value::BigInt(text) | Value::Decimal(text) => Some(Cow::Borrowed(text)),
        _ => None,
    }
}

/// Exact value of a decimal number as sign, significant digits without
/// leading or trailing zeros, and power of ten; zero has no sign.
#[cfg(test)]
fn decimal(text: &str) -> Option<(bool, String, i64)> {
    let (negative, unsigned) = text
        .strip_prefix('-')
        .map_or((false, text), |rest| (true, rest));
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], unsigned[at + 1..].parse::<i64>().ok()?),
        None => (unsigned, 0),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{integer}{fraction}");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some((false, String::new(), 0));
    }
    let trimmed = significant.trim_end_matches('0');
    let shift = i64::try_from(significant.len() - trimmed.len()).ok()?;
    let fraction_len = i64::try_from(fraction.len()).ok()?;
    let power = exponent.checked_sub(fraction_len)?.checked_add(shift)?;
    Some((negative, trimmed.to_owned(), power))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_and_trailing_commas_keeping_offsets() {
        let text = "{\n  // c\n  \"a\": [1, 2,], /* b */\n  \"s\": \"//x,]\",\n}";
        let plain = strip_jsonc(text);
        assert_eq!(plain.len(), text.len());
        assert_eq!(
            plain,
            "{\n      \n  \"a\": [1, 2 ],        \n  \"s\": \"//x,]\" \n}"
        );
    }

    #[test]
    fn reads_jsonc_through_serde_json() {
        let value = read("[1, /* x */ 2,]", Dialect::Jsonc).unwrap();
        assert_eq!(value, Value::List(vec![Value::Int(1), Value::Int(2)]));
        assert!(read("[1, 2,]", Dialect::Json).is_err());
    }

    #[test]
    fn keeps_numbers_outside_the_binary_range() {
        let value = read(
            "{\"#a\": [1e400, 18446744073709551616, \"#1\"]}",
            Dialect::Json,
        )
        .unwrap();
        let expected = Value::Map(IndexMap::from([(
            "#a".to_owned(),
            Value::List(vec![
                Value::Decimal("1e400".to_owned()),
                Value::BigInt("18446744073709551616".to_owned()),
                Value::Text("#1".to_owned()),
            ]),
        )]));
        assert_eq!(value, expected);
        assert!(read("[01]", Dialect::Json).is_err());
    }

    #[test]
    fn compares_numbers_by_exact_value() {
        assert!(same_data(
            &Value::Decimal("1.10".to_owned()),
            &Value::Number(1.1)
        ));
        assert!(same_data(
            &Value::Int(3),
            &Value::Decimal("3.0e0".to_owned())
        ));
        assert!(same_data(&Value::Decimal("-0".to_owned()), &Value::Int(0)));
        assert!(same_data(
            &Value::Decimal("1e400".to_owned()),
            &Value::Decimal("10e399".to_owned())
        ));
        assert!(!same_data(
            &Value::Decimal("1e400".to_owned()),
            &Value::Decimal("2e400".to_owned())
        ));
        assert!(!same_data(
            &Value::BigInt("9007199254740993".to_owned()),
            &Value::Int(9_007_199_254_740_992)
        ));
        assert!(!same_data(&Value::Int(3), &Value::Text("3".to_owned())));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
