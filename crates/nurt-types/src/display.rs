// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Short text form of any value, as shown in text reports.

use nurt_contracts::value::{Fields, Value};

use crate::units::{display_duration, display_instant, display_size};

/// Short form of a value in text reports.
///
/// Sizes, durations and instants use [`display_size`], [`display_duration`]
/// and [`display_instant`]; texts, paths, globs, keys and enumeration values
/// are written without quotes; bytes as base64; lists as `[a, b]`; maps and
/// records as `{name: value, …}`.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::{Size, Value};
/// use nurt_types::display_short;
///
/// assert_eq!(display_short(&Value::Size(Size::from_bytes(3_400_000))), "3.4mb");
/// let list = Value::List(vec![Value::Int(1), Value::Null]);
/// assert_eq!(display_short(&list), "[1, null]");
/// ```
#[must_use]
pub fn display_short(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(truth) => truth.to_string(),
        Value::Int(int) => int.to_string(),
        Value::Number(float) => float.to_string(),
        Value::BigInt(text) | Value::Decimal(text) => text.clone(),
        Value::Size(size) => display_size(*size),
        Value::Duration(duration) => display_duration(*duration),
        Value::Instant(instant) => display_instant(*instant),
        Value::Date(date) => date.text.clone(),
        Value::Regex(regex) => regex.to_string(),
        Value::Range(range) => range.to_string(),
        Value::Bytes(bytes) => bytes.to_base64(),
        Value::List(items) => {
            let items: Vec<String> = items.iter().map(display_short).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Map(map) => display_fields(map),
        Value::Record(record) => display_fields(&record.fields),
        other => other
            .as_str()
            .map_or_else(|| other.type_name().to_owned(), str::to_owned),
    }
}

fn display_fields(fields: &Fields) -> String {
    let entries: Vec<String> = fields
        .iter()
        .map(|(name, value)| format!("{name}: {}", display_short(value)))
        .collect();
    format!("{{{}}}", entries.join(", "))
}
