// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion between document scalars and engine values.
//!
//! Numbers keep their source form: a lexeme that the binary form would
//! change (`1.10`, `1e400`, `.inf`) is read as [`Value::Decimal`], so setting
//! the value that was read changes no byte. Comparison of values for the
//! semantic guard treats `1`, `1.0` and `0x1` as the same number and a date
//! as the same as its text. [`needs_quotes`] decides whether a text written
//! as a plain scalar would be read back as text.
//!
//! ```
//! use nurt_contracts::value::Value;
//! use nurt_doc::value::{Syntax, equivalent, parse_number, render_scalar};
//!
//! let read = parse_number("1.10", Syntax::Json);
//! assert_eq!(read, Some(Value::Decimal("1.10".to_owned())));
//! assert!(equivalent(&Value::Decimal("1.10".to_owned()), &Value::Number(1.1)));
//! assert_eq!(render_scalar(&Value::Number(2.0), Syntax::Toml).as_deref(), Some("2.0"));
//! ```

mod apply;
mod compare;
mod number;
mod quote;

use nurt_contracts::value::{Fields, Value};

pub use apply::{apply_remove, apply_set};
pub use compare::{compare_numbers, equivalent, first_difference, numeric};
pub use number::{parse_number, render_scalar};
pub use quote::{PlainKind, QuoteRules, needs_quotes, yaml_plain_kind};

/// Number and keyword syntax of a document format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Syntax {
    /// JSON and JSONC (RFC 8259).
    Json,
    /// TOML 1.0.
    Toml,
    /// YAML 1.1 types (`yes`, `0o`-less octals, sexagesimal numbers).
    Yaml11,
    /// YAML 1.2 core schema.
    Yaml12,
    /// Formats without typed values (INI, property files, environment
    /// files, CSV): every value is text.
    Untyped,
}

/// Name of the type of a document value, as in the `value_type` field of
/// `item` records: `text`, `int`, `number`, `bool`, `null`, `list`, `map`,
/// `date` or `bytes`.
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::value::value_type;
///
/// assert_eq!(value_type(&Value::Decimal("1e400".to_owned())), "number");
/// ```
#[must_use]
pub const fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) | Value::BigInt(_) | Value::Size(_) => "int",
        Value::Number(_) | Value::Decimal(_) => "number",
        Value::Date(_) => "date",
        Value::Bytes(_) => "bytes",
        Value::List(_) => "list",
        Value::Map(_) | Value::Record(_) => "map",
        _ => "text",
    }
}

/// Converts a value of the language to the value a document stores: paths,
/// patterns, keys, enumeration values, regular expressions, ranges and
/// instants become text, durations become text in milliseconds (`1500ms`),
/// sizes become integers (bytes) and records
/// become maps of their fields.
///
/// ```
/// use nurt_contracts::value::{Size, Value};
/// use nurt_doc::value::to_document_value;
///
/// let size = Value::Size(Size::from_bytes(512));
/// assert_eq!(to_document_value(size), Value::Int(512));
/// ```
#[must_use]
pub fn to_document_value(value: Value) -> Value {
    match value {
        Value::Path(text) | Value::Glob(text) | Value::Key(text) | Value::Enum(text) => {
            Value::Text(text)
        }
        Value::Regex(regex) => Value::Text(regex.to_string()),
        Value::Range(range) => Value::Text(range.to_string()),
        Value::Instant(instant) => Value::Text(instant.to_string()),
        Value::Duration(duration) => Value::Text(format!("{}ms", duration.as_millis())),
        Value::Size(size) => {
            i64::try_from(size.bytes()).map_or_else(|_| Value::BigInt(size.bytes().to_string()), Value::Int)
        }
        Value::List(items) => Value::List(items.into_iter().map(to_document_value).collect()),
        Value::Map(fields) => Value::Map(document_fields(fields)),
        Value::Record(record) => Value::Map(document_fields(record.fields)),
        other => other,
    }
}

fn document_fields(fields: Fields) -> Fields {
    fields
        .into_iter()
        .map(|(name, value)| (name, to_document_value(value)))
        .collect()
}

/// Copy of `value` with maps and lists below `depth` levels emptied; `None`
/// keeps every level.
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::value::limit_depth;
///
/// let nested = Value::List(vec![Value::List(vec![Value::Int(1)])]);
/// assert_eq!(limit_depth(&nested, Some(1)), Value::List(vec![Value::List(Vec::new())]));
/// ```
#[must_use]
pub fn limit_depth(value: &Value, depth: Option<u32>) -> Value {
    let Some(depth) = depth else {
        return value.clone();
    };
    match value {
        Value::List(items) => Value::List(if depth == 0 {
            Vec::new()
        } else {
            items.iter().map(|item| limit_depth(item, Some(depth - 1))).collect()
        }),
        Value::Map(fields) => Value::Map(if depth == 0 {
            Fields::new()
        } else {
            fields
                .iter()
                .map(|(name, item)| (name.clone(), limit_depth(item, Some(depth - 1))))
                .collect()
        }),
        other => other.clone(),
    }
}
