// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion between property list values and engine values.
//!
//! | Property list | Engine value | `value_type` |
//! |---|---|---|
//! | `string` | `Text` | `text` |
//! | `integer` | `Int`, or `BigInt` above `i64::MAX` | `int` |
//! | `real` | `Number` | `number` |
//! | `true`, `false` | `Bool` | `bool` |
//! | `date` | `Date` with the RFC 3339 text in UTC | `date` |
//! | `data` | `Bytes` | `bytes` |
//! | `UID` (binary only) | `Int` with the tag `uid` | `int` |
//! | `array` | `List` | `list` |
//! | `dict` | `Map`, keys in document order | `map` |
//!
//! A new value takes the type of the engine value, with two exceptions that
//! keep the type of the replaced node: an integer stored over a `real` stays
//! `real`, and a non-negative integer stored over a `UID` stays a `UID`.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::{Bytes, Date, DateKind, Fields, Value};
use plist::{Dictionary, Integer, Uid};

/// Tag of values read from a `UID` node.
pub(crate) const UID_TAG: &str = "uid";

/// Type of the replaced node where it decides the type of the new value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hint {
    /// The node is a `real`.
    Real,
    /// The node is a `UID`.
    Uid,
    /// Any other node, or no node.
    Other,
}

impl Hint {
    /// Hint for a replaced value.
    pub(crate) const fn of(value: &plist::Value) -> Self {
        match value {
            plist::Value::Real(_) => Self::Real,
            plist::Value::Uid(_) => Self::Uid,
            _ => Self::Other,
        }
    }
}

/// Engine value of a property list value.
pub(crate) fn to_value(value: &plist::Value) -> Value {
    match value {
        plist::Value::String(text) => Value::Text(text.clone()),
        plist::Value::Integer(integer) => integer_value(*integer),
        plist::Value::Real(number) => Value::Number(*number),
        plist::Value::Boolean(flag) => Value::Bool(*flag),
        plist::Value::Date(date) => date_value(*date),
        plist::Value::Data(bytes) => Value::Bytes(Bytes::new(bytes.clone())),
        plist::Value::Uid(uid) => uid_value(*uid),
        plist::Value::Array(items) => Value::List(items.iter().map(to_value).collect()),
        plist::Value::Dictionary(dictionary) => Value::Map(
            dictionary
                .iter()
                .map(|(key, item)| (key.clone(), to_value(item)))
                .collect::<Fields>(),
        ),
        _ => Value::Null,
    }
}

fn integer_value(integer: Integer) -> Value {
    integer
        .as_signed()
        .map_or_else(|| Value::BigInt(integer.to_string()), Value::Int)
}

/// Engine value of a `UID`.
pub(crate) fn uid_value(uid: Uid) -> Value {
    i64::try_from(uid.get()).map_or_else(|_| Value::BigInt(uid.get().to_string()), Value::Int)
}

/// Engine value of a date: RFC 3339 text in UTC.
pub(crate) fn date_value(date: plist::Date) -> Value {
    Value::Date(Date {
        kind: DateKind::OffsetDateTime,
        text: date.to_xml_format(),
    })
}

/// Property list value for `value`, stored over a node described by `hint`.
///
/// # Errors
///
/// Returns `E_TYPE` for a value without a property list type (null, regular
/// expression, range) and `E_INVALID_VALUE` for an integer outside the
/// 64-bit range or a date that is not RFC 3339.
pub(crate) fn from_value(value: &Value, hint: Hint) -> Result<plist::Value, Error> {
    let converted = match value {
        Value::Text(text)
        | Value::Path(text)
        | Value::Glob(text)
        | Value::Key(text)
        | Value::Enum(text) => plist::Value::String(text.clone()),
        Value::Int(number) => integer_over(*number, hint),
        Value::BigInt(text) => plist::Value::Integer(big_integer(text)?),
        Value::Number(number) => plist::Value::Real(*number),
        Value::Decimal(text) => plist::Value::Real(decimal(text)?),
        Value::Bool(flag) => plist::Value::Boolean(*flag),
        Value::Size(size) => plist::Value::Integer(Integer::from(size.bytes())),
        Value::Duration(duration) => plist::Value::Integer(Integer::from(duration.as_millis())),
        Value::Date(date) => plist::Value::Date(parse_date(&date.text)?),
        Value::Instant(instant) => plist::Value::Date(parse_date(&instant.to_string())?),
        Value::Bytes(bytes) => plist::Value::Data(bytes.as_slice().to_vec()),
        Value::List(items) => plist::Value::Array(
            items
                .iter()
                .map(|item| from_value(item, Hint::Other))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(fields) => plist::Value::Dictionary(dictionary(fields.iter())?),
        Value::Record(record) => plist::Value::Dictionary(dictionary(
            record
                .field_names()
                .filter_map(|name| Some((name, record.get(name)?))),
        )?),
        other => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "a property list cannot hold a value of type {}; pass text, a number, a date, bytes, a list or a map",
                    other.type_name()
                ),
            ));
        }
    };
    Ok(converted)
}

/// Integer stored over a node of the given type.
///
/// A `real` node keeps its type; the conversion rounds integers beyond 2^53
/// to the nearest double, as storing them in a `real` requires.
fn integer_over(number: i64, hint: Hint) -> plist::Value {
    match hint {
        #[allow(
            clippy::cast_precision_loss,
            reason = "a real node holds a double, so the integer is rounded as the type requires"
        )]
        Hint::Real => plist::Value::Real(number as f64),
        Hint::Uid if number >= 0 => plist::Value::Uid(Uid::new(number.unsigned_abs())),
        Hint::Uid | Hint::Other => plist::Value::Integer(Integer::from(number)),
    }
}

fn dictionary<'a, K>(fields: impl Iterator<Item = (K, &'a Value)>) -> Result<Dictionary, Error>
where
    K: AsRef<str>,
{
    let mut dictionary = Dictionary::new();
    for (key, item) in fields {
        dictionary.insert(key.as_ref().to_owned(), from_value(item, Hint::Other)?);
    }
    Ok(dictionary)
}

fn big_integer(text: &str) -> Result<Integer, Error> {
    text.parse::<u64>()
        .map(Integer::from)
        .or_else(|_| text.parse::<i64>().map(Integer::from))
        .map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "integer {text} is outside the 64-bit range of a property list integer; pass a smaller integer or a real"
                ),
            )
        })
}

fn decimal(text: &str) -> Result<f64, Error> {
    text.parse::<f64>().map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("{text} is not a decimal number; pass a number such as 1.5"),
        )
    })
}

/// Date from RFC 3339 text.
///
/// # Errors
///
/// Returns `E_INVALID_VALUE` for text that is not an RFC 3339 date and time.
pub(crate) fn parse_date(text: &str) -> Result<plist::Date, Error> {
    plist::Date::from_xml_format(text).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "{text} is not a property list date; pass an RFC 3339 date and time such as 2026-01-31T12:00:00Z"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::{Duration, Instant, Record, RecordKind, Size};

    use super::*;

    #[test]
    fn integer_over_real_and_uid_keeps_the_node_type() {
        assert_eq!(
            from_value(&Value::Int(2), Hint::Real).unwrap(),
            plist::Value::Real(2.0)
        );
        assert_eq!(
            from_value(&Value::Int(4), Hint::Uid).unwrap(),
            plist::Value::Uid(Uid::new(4))
        );
        assert_eq!(
            from_value(&Value::Int(-4), Hint::Uid).unwrap(),
            plist::Value::Integer((-4).into())
        );
        assert_eq!(
            from_value(&Value::Int(5), Hint::Other).unwrap(),
            plist::Value::Integer(5.into())
        );
        assert_eq!(Hint::of(&plist::Value::Real(1.0)), Hint::Real);
        assert_eq!(Hint::of(&plist::Value::Boolean(true)), Hint::Other);
    }

    #[test]
    fn values_round_trip_with_their_types() {
        let mut dictionary = Dictionary::new();
        dictionary.insert("z".into(), plist::Value::Integer(1.into()));
        dictionary.insert("a".into(), plist::Value::Real(1.0));
        dictionary.insert(
            "d".into(),
            plist::Value::Date(parse_date("2026-01-31T12:00:00Z").unwrap()),
        );
        dictionary.insert("b".into(), plist::Value::Data(vec![0, 255]));
        dictionary.insert("u".into(), plist::Value::Integer(u64::MAX.into()));
        dictionary.insert(
            "l".into(),
            plist::Value::Array(vec![plist::Value::Boolean(false)]),
        );
        let original = plist::Value::Dictionary(dictionary);
        let value = to_value(&original);
        let Value::Map(fields) = &value else {
            panic!("map expected")
        };
        assert_eq!(
            fields.keys().collect::<Vec<_>>(),
            ["z", "a", "d", "b", "u", "l"]
        );
        assert_eq!(fields["a"], Value::Number(1.0));
        assert_eq!(fields["u"], Value::BigInt(u64::MAX.to_string()));
        assert_eq!(from_value(&value, Hint::Other).unwrap(), original);
    }

    #[test]
    fn uid_values_are_integers() {
        assert_eq!(to_value(&plist::Value::Uid(Uid::new(9))), Value::Int(9));
        assert_eq!(
            uid_value(Uid::new(u64::MAX)),
            Value::BigInt(u64::MAX.to_string())
        );
    }

    #[test]
    fn rejects_null_bad_dates_and_huge_integers() {
        assert_eq!(
            from_value(&Value::Null, Hint::Other)
                .unwrap_err()
                .code()
                .as_str(),
            "E_TYPE"
        );
        let date = Value::Date(Date {
            kind: DateKind::LocalDate,
            text: "2026-01-31".into(),
        });
        assert_eq!(
            from_value(&date, Hint::Other).unwrap_err().code().as_str(),
            "E_INVALID_VALUE"
        );
        let huge = Value::BigInt("99999999999999999999".into());
        assert_eq!(
            from_value(&huge, Hint::Other).unwrap_err().code().as_str(),
            "E_INVALID_VALUE"
        );
        let decimal = Value::Decimal("1.25".into());
        assert_eq!(
            from_value(&decimal, Hint::Other).unwrap(),
            plist::Value::Real(1.25)
        );
    }

    #[test]
    fn named_text_values_are_strings() {
        for value in [
            Value::Path("a/b".into()),
            Value::Glob("*.c".into()),
            Value::Key("a.b".into()),
            Value::Enum("on".into()),
        ] {
            assert_eq!(
                from_value(&value, Hint::Other).unwrap(),
                plist::Value::String(value.as_str().unwrap().to_owned())
            );
        }
    }

    #[test]
    fn sizes_durations_and_instants_have_property_list_types() {
        assert_eq!(
            from_value(&Value::Size(Size::from_bytes(2048)), Hint::Other).unwrap(),
            plist::Value::Integer(2048_u64.into())
        );
        assert_eq!(
            from_value(&Value::Duration(Duration::from_millis(1500)), Hint::Other).unwrap(),
            plist::Value::Integer(1500_i64.into())
        );
        assert_eq!(
            from_value(&Value::Instant(Instant::UNIX_EPOCH), Hint::Other).unwrap(),
            plist::Value::Date(parse_date("1970-01-01T00:00:00Z").unwrap())
        );
    }

    #[test]
    fn records_become_dictionaries_in_field_order() {
        let record = Record::new(RecordKind::ALL[0])
            .with("z", Value::Int(1))
            .with("a", Value::Bool(true));
        let plist::Value::Dictionary(dictionary) =
            from_value(&Value::Record(record), Hint::Other).unwrap()
        else {
            panic!("dictionary expected")
        };
        assert_eq!(dictionary.keys().collect::<Vec<_>>(), ["z", "a"]);
        assert_eq!(dictionary.get("a"), Some(&plist::Value::Boolean(true)));
    }

    #[test]
    fn rejects_decimals_that_are_not_numbers() {
        let error = from_value(&Value::Decimal("1,5".into()), Hint::Other).unwrap_err();
        assert_eq!(error.code().as_str(), "E_INVALID_VALUE");
        assert!(error.to_string().contains("1.5"), "{error}");
        let error = from_value(&Value::List(vec![Value::Null]), Hint::Other).unwrap_err();
        assert_eq!(error.code().as_str(), "E_TYPE");
    }
}
