// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! TOML text of values and keys.

use nurt_contracts::error::Error;
use nurt_contracts::value::{Date, DateKind, Fields, Value};

use crate::formats::ini::support::errors::unsupported;

/// Construct name: `null`, which TOML cannot hold.
pub(crate) const NULL: &str = "toml.null";
/// Construct name: a value of a type TOML cannot hold.
pub(crate) const VALUE_TYPE: &str = "toml.value_type";

/// Converts a value to the form it has after it is written and read back.
/// Fails with `E_UNSUPPORTED_CONSTRUCT` `toml.null` for `null` and
/// `toml.value_type` for sizes, durations, ranges, regular expressions,
/// bytes and integers outside 64 bits.
pub(crate) fn normalize(value: &Value) -> Result<Value, Error> {
    Ok(match value {
        Value::Text(_) | Value::Int(_) | Value::Number(_) | Value::Bool(_) | Value::Date(_) => {
            value.clone()
        }
        Value::Path(path) => Value::Text(path.as_str().to_owned()),
        Value::Key(text) | Value::Enum(text) | Value::Glob(text) => Value::Text(text.clone()),
        Value::Instant(instant) => Value::Date(Date {
            kind: DateKind::OffsetDateTime,
            text: instant.to_string(),
        }),
        Value::BigInt(text) => Value::Int(text.parse().map_err(|_| {
            unsupported(VALUE_TYPE, "TOML integers have 64 bits; write a smaller integer or a text")
        })?),
        Value::Decimal(text) => Value::Number(text.parse().map_err(|_| {
            unsupported(VALUE_TYPE, "the decimal number cannot be written as a TOML float")
        })?),
        Value::List(items) => Value::List(items.iter().map(normalize).collect::<Result<_, _>>()?),
        Value::Map(fields) => Value::Map(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), normalize(v)?)))
                .collect::<Result<Fields, Error>>()?,
        ),
        Value::Null => {
            return Err(unsupported(NULL, "TOML has no null; remove the key instead"));
        }
        _ => {
            return Err(unsupported(
                VALUE_TYPE,
                "the value type has no TOML form; write a text, number, boolean, date, list or map",
            ));
        }
    })
}

/// Structural equality in which `NaN` equals `NaN` and map members are
/// compared without regard to order.
pub(crate) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Value::Map(x), Value::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

/// Quoting style of a TOML string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringStyle {
    Basic,
    Literal,
    MultiBasic,
    MultiLiteral,
}

impl StringStyle {
    /// Style of the raw text of a string.
    pub(crate) fn of(raw: &str) -> Option<Self> {
        if raw.starts_with("\"\"\"") {
            Some(Self::MultiBasic)
        } else if raw.starts_with("'''") {
            Some(Self::MultiLiteral)
        } else if raw.starts_with('"') {
            Some(Self::Basic)
        } else if raw.starts_with('\'') {
            Some(Self::Literal)
        } else {
            None
        }
    }
}

/// Writes a string in `style` when the style can hold it, otherwise as a
/// basic string. A multi-line string keeps a line break after its opening
/// delimiter when `leading_newline` is set.
pub(crate) fn string(text: &str, style: StringStyle, leading_newline: bool) -> String {
    let lead = if leading_newline { "\n" } else { "" };
    match style {
        StringStyle::Literal if literal_fits(text, false) => format!("'{text}'"),
        StringStyle::MultiLiteral if literal_fits(text, true) && !text.contains("'''") && !text.ends_with('\'') => {
            format!("'''{lead}{text}'''")
        }
        StringStyle::MultiBasic => format!("\"\"\"{lead}{}\"\"\"", escape_basic(text, true)),
        _ => format!("\"{}\"", escape_basic(text, false)),
    }
}

fn literal_fits(text: &str, multiline: bool) -> bool {
    text.chars().all(|c| {
        let quote_ok = c != '\'' || multiline;
        let control_ok = c == '\t' || (multiline && c == '\n') || !c.is_control();
        quote_ok && control_ok
    })
}

fn escape_basic(text: &str, multiline: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' if multiline => out.push('\n'),
            '\n' => out.push_str("\\n"),
            '\t' => out.push('\t'),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            _ if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            _ => out.push(c),
        }
    }
    out
}

/// Writes an integer in the radix of `old_raw` (`0x`, `0o`, `0b`) when the
/// value is not negative, otherwise in decimal.
pub(crate) fn integer(value: i64, old_raw: Option<&str>) -> String {
    let raw = old_raw.unwrap_or_default();
    match (raw.get(..2), value >= 0) {
        (Some("0x"), true) if raw[2..].chars().any(|c| c.is_ascii_uppercase()) => {
            format!("0x{value:X}")
        }
        (Some("0x"), true) => format!("0x{value:x}"),
        (Some("0o"), true) => format!("0o{value:o}"),
        (Some("0b"), true) => format!("0b{value:b}"),
        _ => value.to_string(),
    }
}

/// Writes a float so that it reads back as the same number.
pub(crate) fn float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let text = format!("{value:?}");
    if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    }
}

/// Inline TOML text of a normalized value; `old_raw` is the raw text of the
/// value being replaced, whose string style and integer radix are kept.
pub(crate) fn inline(value: &Value, old_raw: Option<&str>) -> String {
    match value {
        Value::Text(text) => {
            let raw = old_raw.unwrap_or_default();
            let style = StringStyle::of(raw).unwrap_or(StringStyle::Basic);
            let lead = matches!(style, StringStyle::MultiBasic | StringStyle::MultiLiteral)
                && raw.get(3..).is_some_and(|rest| rest.starts_with(['\n', '\r']));
            string(text, style, lead)
        }
        Value::Int(int) => integer(*int, old_raw),
        Value::Number(number) => float(*number),
        Value::Bool(flag) => flag.to_string(),
        Value::Date(date) => date.text.clone(),
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(|i| inline(i, None)).collect::<Vec<_>>().join(", ")
        ),
        Value::Map(fields) if fields.is_empty() => "{}".to_owned(),
        Value::Map(fields) => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(k, v)| format!("{} = {}", key(k), inline(v, None)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => String::new(),
    }
}

/// Writes a key: bare when it holds only ASCII letters, digits, `_` and
/// `-`, otherwise as a basic string.
pub(crate) fn key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        name.to_owned()
    } else {
        format!("\"{}\"", escape_basic(name, false))
    }
}

/// Writes a dotted key of several names.
pub(crate) fn dotted(names: &[String]) -> String {
    names.iter().map(|n| key(n)).collect::<Vec<_>>().join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_string_style_when_it_holds_the_value() {
        assert_eq!(inline(&Value::Text("a b".into()), Some("'x'")), "'a b'");
        assert_eq!(inline(&Value::Text("it's".into()), Some("'x'")), "\"it's\"");
        assert_eq!(
            inline(&Value::Text("l1\nl2".into()), Some("'''\nx'''")),
            "'''\nl1\nl2'''"
        );
        assert_eq!(
            inline(&Value::Text("q\"\n".into()), Some("\"\"\"x\"\"\"")),
            "\"\"\"q\\\"\n\"\"\""
        );
        assert_eq!(inline(&Value::Text("tab\tz\u{1}".into()), None), "\"tab\tz\\u0001\"");
        assert_eq!(string("end'", StringStyle::MultiLiteral, false), "\"end'\"");
    }

    #[test]
    fn keeps_the_integer_radix_and_writes_floats_that_read_back() {
        assert_eq!(integer(255, Some("0xFF")), "0xFF");
        assert_eq!(integer(255, Some("0xdead_beef")), "0xff");
        assert_eq!(integer(8, Some("0o7")), "0o10");
        assert_eq!(integer(5, Some("0b1")), "0b101");
        assert_eq!(integer(-5, Some("0x1")), "-5");
        assert_eq!(float(1.0), "1.0");
        assert_eq!(float(1e300), "1e300");
        assert_eq!(float(f64::NEG_INFINITY), "-inf");
        assert_eq!(float(f64::NAN), "nan");
    }

    #[test]
    fn writes_keys_bare_or_quoted() {
        assert_eq!(key("serde_json-1"), "serde_json-1");
        assert_eq!(key("a.b"), "\"a.b\"");
        assert_eq!(key(""), "\"\"");
        assert_eq!(dotted(&["a".into(), "b c".into()]), "a.\"b c\"");
    }

    #[test]
    fn normalizes_values_and_rejects_null() {
        assert_eq!(normalize(&Value::BigInt("12".into())).unwrap(), Value::Int(12));
        assert!(normalize(&Value::BigInt("99999999999999999999".into())).is_err());
        assert_eq!(normalize(&Value::Null).unwrap_err().found.as_deref(), Some(NULL));
        let mut fields = Fields::new();
        fields.insert("a".into(), Value::List(vec![Value::Int(1)]));
        let map = Value::Map(fields);
        assert_eq!(inline(&map, None), "{ a = [1] }");
        assert!(same(&Value::Number(f64::NAN), &Value::Number(f64::NAN)));
    }
}
