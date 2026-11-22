// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text of new values in the style of the surrounding document.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Value;

use super::layout::Style;
use super::lexeme;

/// Value lowered to JSON: scalars are finished lexemes.
enum Json {
    Lexeme(String),
    Map(Vec<(String, Json)>),
    List(Vec<Json>),
}

/// Writes `value` as JSON text.
///
/// `indent` is the indentation of the line the value starts on; nested
/// entries of non-empty maps and lists go on their own lines indented by one
/// more unit when `multiline` is set, and on one line otherwise.
///
/// # Errors
///
/// `E_INVALID_VALUE` when a number has no JSON form (a non-finite number or
/// a decimal text outside the JSON number grammar).
pub(crate) fn render(value: &Value, indent: &str, style: &Style, multiline: bool) -> Result<String, Error> {
    let json = lower(value)?;
    let mut out = String::new();
    write(&json, indent, style, multiline, &mut out);
    Ok(out)
}

/// Text of an object member: quoted name, separator and value.
pub(crate) fn render_member(
    key: &str,
    value: &Value,
    indent: &str,
    style: &Style,
    multiline: bool,
) -> Result<String, Error> {
    Ok(format!(
        "{}{}{}",
        lexeme::quote(key),
        style.colon,
        render(value, indent, style, multiline)?
    ))
}

fn lower(value: &Value) -> Result<Json, Error> {
    Ok(match value {
        Value::Null => Json::Lexeme("null".to_owned()),
        Value::Bool(flag) => Json::Lexeme(flag.to_string()),
        Value::Int(number) => Json::Lexeme(number.to_string()),
        Value::BigInt(text) | Value::Decimal(text) => Json::Lexeme(number_lexeme(text)?),
        Value::Number(number) if number.is_finite() => Json::Lexeme(lexeme::format_f64(*number)),
        Value::Number(number) => return Err(not_a_number(&number.to_string())),
        Value::Text(text) => Json::Lexeme(lexeme::quote(text)),
        Value::List(items) => Json::List(items.iter().map(lower).collect::<Result<_, _>>()?),
        Value::Map(fields) => Json::Map(
            fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), lower(item)?)))
                .collect::<Result<_, Error>>()?,
        ),
        other => lower_plain(other.to_plain_json()),
    })
}

fn lower_plain(json: serde_json::Value) -> Json {
    match json {
        serde_json::Value::Null => Json::Lexeme("null".to_owned()),
        serde_json::Value::Bool(flag) => Json::Lexeme(flag.to_string()),
        serde_json::Value::Number(number) => Json::Lexeme(number.to_string()),
        serde_json::Value::String(text) => Json::Lexeme(lexeme::quote(&text)),
        serde_json::Value::Array(items) => Json::List(items.into_iter().map(lower_plain).collect()),
        serde_json::Value::Object(fields) => {
            Json::Map(fields.into_iter().map(|(key, item)| (key, lower_plain(item))).collect())
        }
    }
}

fn number_lexeme(text: &str) -> Result<String, Error> {
    lexeme::validate_number(text).map_err(|_| not_a_number(text))?;
    Ok(text.to_owned())
}

fn not_a_number(text: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("'{text}' has no JSON number form; pass a finite decimal number or a text value"),
    )
}

fn write(json: &Json, indent: &str, style: &Style, multiline: bool, out: &mut String) {
    match json {
        Json::Lexeme(text) => out.push_str(text),
        Json::Map(fields) if fields.is_empty() => out.push_str("{}"),
        Json::List(items) if items.is_empty() => out.push_str("[]"),
        Json::Map(fields) => {
            let entries = fields.iter().map(|(key, item)| (Some(key.as_str()), item));
            write_container(('{', '}'), entries, indent, style, multiline, out);
        }
        Json::List(items) => {
            let entries = items.iter().map(|item| (None, item));
            write_container(('[', ']'), entries, indent, style, multiline, out);
        }
    }
}

fn write_container<'j>(
    (open, close): (char, char),
    entries: impl Iterator<Item = (Option<&'j str>, &'j Json)>,
    indent: &str,
    style: &Style,
    multiline: bool,
    out: &mut String,
) {
    let inner = format!("{indent}{}", style.indent_unit);
    out.push(open);
    if !multiline {
        out.push_str(&style.bracket_space);
    }
    for (position, (key, item)) in entries.enumerate() {
        if position > 0 {
            out.push(',');
            if !multiline {
                out.push_str(&style.comma_space);
            }
        }
        if multiline {
            out.push_str(style.eol);
            out.push_str(&inner);
        }
        if let Some(key) = key {
            out.push_str(&lexeme::quote(key));
            out.push_str(&style.colon);
        }
        write(item, &inner, style, multiline, out);
    }
    if multiline {
        if style.trailing_comma {
            out.push(',');
        }
        out.push_str(style.eol);
        out.push_str(indent);
    } else {
        out.push_str(&style.bracket_space);
    }
    out.push(close);
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    fn style(trailing_comma: bool) -> Style {
        Style {
            indent_unit: "  ".to_owned(),
            eol: "\n",
            colon: ": ".to_owned(),
            comma_space: " ".to_owned(),
            bracket_space: String::new(),
            trailing_comma,
        }
    }

    fn sample() -> Value {
        let mut fields = IndexMap::new();
        fields.insert("a".to_owned(), Value::Int(1));
        fields.insert("b".to_owned(), Value::List(vec![Value::Bool(true), Value::Null]));
        fields.insert("c".to_owned(), Value::Map(IndexMap::new()));
        Value::Map(fields)
    }

    #[test]
    fn renders_nested_values_on_separate_lines() {
        let text = render(&sample(), "  ", &style(false), true).unwrap();
        assert_eq!(text, "{\n    \"a\": 1,\n    \"b\": [\n      true,\n      null\n    ],\n    \"c\": {}\n  }");
    }

    #[test]
    fn renders_nested_values_on_one_line() {
        let text = render(&sample(), "", &style(false), false).unwrap();
        assert_eq!(text, "{\"a\": 1, \"b\": [true, null], \"c\": {}}");
    }

    #[test]
    fn adds_trailing_commas_in_trailing_comma_style() {
        let value = Value::List(vec![Value::Int(1)]);
        assert_eq!(render(&value, "", &style(true), true).unwrap(), "[\n  1,\n]");
    }

    #[test]
    fn keeps_decimal_lexemes_and_rejects_non_json_numbers() {
        assert_eq!(render(&Value::Decimal("1.10".to_owned()), "", &style(false), false).unwrap(), "1.10");
        assert_eq!(render(&Value::Number(0.5), "", &style(false), false).unwrap(), "0.5");
        let error = render(&Value::Decimal("0x1F".to_owned()), "", &style(false), false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }
}
