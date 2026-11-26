// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion between language values and SQLite values.
//!
//! SQLite stores five storage classes. Results map `INTEGER` to `int`,
//! `REAL` to `number`, `TEXT` to `text`, `BLOB` to `bytes` and `NULL` to
//! `null`. A `REAL` infinity has no `number` form and becomes the text `Inf`
//! or `-Inf`, as printed by the `sqlite3` program; text that is not valid
//! UTF-8 becomes `bytes`.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Bytes, Value};
use rusqlite::types::{Value as SqlValue, ValueRef};

/// Converts a parameter value for binding; `label` names the parameter in
/// errors: its number across all statements for a list, its placeholder
/// for a map.
pub(super) fn to_sqlite(value: &Value, label: &str) -> Result<SqlValue> {
    let converted = match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Int(integer) => SqlValue::Integer(*integer),
        Value::Number(number) if number.is_nan() => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "parameter {label} is not a number (NaN), which SQLite stores as NULL; pass null or text"
                ),
            )
            .with_problem("parameter_value"));
        }
        Value::Number(number) => SqlValue::Real(*number),
        Value::Text(text)
        | Value::Decimal(text)
        | Value::Path(text)
        | Value::Glob(text)
        | Value::Key(text)
        | Value::Enum(text) => SqlValue::Text(text.clone()),
        Value::Size(size) => {
            SqlValue::Integer(i64::try_from(size.bytes()).map_err(|_| out_of_range(label, "size"))?)
        }
        Value::Duration(duration) => SqlValue::Integer(duration.as_millis()),
        Value::Instant(instant) => SqlValue::Text(instant.to_string()),
        Value::Date(date) => SqlValue::Text(date.text.clone()),
        Value::Regex(regex) => SqlValue::Text(regex.pattern.clone()),
        Value::Bytes(bytes) => SqlValue::Blob(bytes.as_slice().to_vec()),
        Value::BigInt(_) => return Err(out_of_range(label, "int")),
        other => {
            return Err(Error::new(
                ErrorCode::Sql,
                format!(
                    "parameter {label} has type {}; bind null, bool, int, number, text or bytes",
                    other.type_name()
                ),
            )
            .with_problem("parameter_type"));
        }
    };
    Ok(converted)
}

/// Converts one result cell.
pub(super) fn from_sqlite(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(integer) => Value::Int(integer),
        ValueRef::Real(real) if real.is_finite() => Value::Number(real),
        ValueRef::Real(real) if real.is_sign_negative() => Value::Text("-Inf".to_owned()),
        ValueRef::Real(_) => Value::Text("Inf".to_owned()),
        ValueRef::Text(text) => match std::str::from_utf8(text) {
            Ok(text) => Value::Text(text.to_owned()),
            Err(_) => Value::Bytes(Bytes::new(text.to_vec())),
        },
        ValueRef::Blob(blob) => Value::Bytes(Bytes::new(blob.to_vec())),
    }
}

fn out_of_range(label: &str, type_name: &str) -> Error {
    Error::new(
        ErrorCode::Sql,
        format!(
            "parameter {label} is a {type_name} outside the 64-bit integer range of SQLite; pass it as text"
        ),
    )
    .with_problem("parameter_range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::value::{Date, DateKind, Duration, Fields, Instant, Regex, Size};

    #[test]
    fn binds_scalars_by_storage_class() {
        assert_eq!(
            to_sqlite(&Value::Bool(true), "1").ok(),
            Some(SqlValue::Integer(1))
        );
        assert_eq!(
            to_sqlite(&Value::Size(Size::from_bytes(4096)), "1").ok(),
            Some(SqlValue::Integer(4096))
        );
        assert_eq!(
            to_sqlite(&Value::Duration(Duration::from_millis(1500)), "1").ok(),
            Some(SqlValue::Integer(1500))
        );
        assert_eq!(
            to_sqlite(&Value::Decimal("1.10".to_owned()), "`:price`").ok(),
            Some(SqlValue::Text("1.10".to_owned()))
        );
    }

    #[test]
    fn rejects_values_without_storage_class() {
        let error = to_sqlite(&Value::List(Vec::new()), "`:ids`").err();
        assert_eq!(error.as_ref().map(Error::code), Some(ErrorCode::Sql));
        assert!(
            error
                .as_ref()
                .is_some_and(|error| error.message.starts_with("parameter `:ids` has type"))
        );
        let big = to_sqlite(&Value::BigInt("99999999999999999999".to_owned()), "2").err();
        assert_eq!(
            big.map(|error| error.problem.clone()),
            Some(Some("parameter_range".to_owned()))
        );
        let size = to_sqlite(&Value::Size(Size::from_bytes(u64::MAX)), "1").err();
        assert_eq!(size.map(|error| error.code()), Some(ErrorCode::Sql));
    }

    #[test]
    fn maps_infinite_and_invalid_text_results() {
        assert_eq!(
            from_sqlite(ValueRef::Real(f64::INFINITY)),
            Value::Text("Inf".to_owned())
        );
        assert_eq!(
            from_sqlite(ValueRef::Real(f64::NEG_INFINITY)),
            Value::Text("-Inf".to_owned())
        );
        assert_eq!(
            from_sqlite(ValueRef::Text(&[0xff, 0x00])),
            Value::Bytes(Bytes::new(vec![0xff, 0x00]))
        );
    }

    #[test]
    fn rejects_nan_and_binds_infinity() {
        let error = to_sqlite(&Value::Number(f64::NAN), "`:x`").unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("parameter_value"));
        assert!(error.message.contains("`:x`"), "{}", error.message);
        assert_eq!(
            to_sqlite(&Value::Number(f64::INFINITY), "1").ok(),
            Some(SqlValue::Real(f64::INFINITY))
        );
    }

    #[test]
    fn binds_null_int_number_and_bytes() {
        assert_eq!(to_sqlite(&Value::Null, "1").ok(), Some(SqlValue::Null));
        assert_eq!(
            to_sqlite(&Value::Int(-7), "1").ok(),
            Some(SqlValue::Integer(-7))
        );
        assert_eq!(
            to_sqlite(&Value::Number(2.5), "1").ok(),
            Some(SqlValue::Real(2.5))
        );
        assert_eq!(
            to_sqlite(&Value::Bool(false), "1").ok(),
            Some(SqlValue::Integer(0))
        );
        assert_eq!(
            to_sqlite(&Value::Bytes(Bytes::new(vec![0, 1, 2])), "1").ok(),
            Some(SqlValue::Blob(vec![0, 1, 2]))
        );
    }

    #[test]
    fn binds_text_like_values_as_text() {
        let cases = [
            Value::Text("a".to_owned()),
            Value::Path("a".to_owned()),
            Value::Glob("a".to_owned()),
            Value::Key("a".to_owned()),
            Value::Enum("a".to_owned()),
        ];
        for value in cases {
            assert_eq!(
                to_sqlite(&value, "1").ok(),
                Some(SqlValue::Text("a".to_owned())),
                "{}",
                value.type_name()
            );
        }
    }

    #[test]
    fn binds_instant_date_and_regex_as_text() {
        let instant = Instant::UNIX_EPOCH;
        assert_eq!(
            to_sqlite(&Value::Instant(instant), "1").ok(),
            Some(SqlValue::Text(instant.to_string()))
        );
        let date = Date {
            kind: DateKind::LocalDate,
            text: "2026-11-21".to_owned(),
        };
        assert_eq!(
            to_sqlite(&Value::Date(date), "1").ok(),
            Some(SqlValue::Text("2026-11-21".to_owned()))
        );
        let regex = Regex::new("^a+$", "i").ok();
        assert!(regex.is_some());
        if let Some(regex) = regex {
            assert_eq!(
                to_sqlite(&Value::Regex(regex), "1").ok(),
                Some(SqlValue::Text("^a+$".to_owned()))
            );
        }
    }

    #[test]
    fn reports_parameter_type_problem_with_label() {
        let error = to_sqlite(&Value::Map(Fields::default()), "3").err();
        assert_eq!(
            error.as_ref().and_then(|error| error.problem.as_deref()),
            Some("parameter_type")
        );
        assert!(
            error
                .as_ref()
                .is_some_and(|error| error.message.contains("parameter 3 has type"))
        );
    }

    #[test]
    fn maps_scalar_results() {
        assert_eq!(from_sqlite(ValueRef::Null), Value::Null);
        assert_eq!(
            from_sqlite(ValueRef::Integer(i64::MIN)),
            Value::Int(i64::MIN)
        );
        assert_eq!(from_sqlite(ValueRef::Real(1.5)), Value::Number(1.5));
        assert_eq!(
            from_sqlite(ValueRef::Text("zażółć".as_bytes())),
            Value::Text("zażółć".to_owned())
        );
        assert_eq!(
            from_sqlite(ValueRef::Blob(&[9, 8])),
            Value::Bytes(Bytes::new(vec![9, 8]))
        );
    }
}
