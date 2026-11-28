// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::jq::convert
// Responsibility:   Conversion between language values and `jq` values.
// Dependencies:     jaq_json, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conversion between language values and `jq` values.
//!
//! Numbers keep their exact form in both directions: integers outside the
//! 64-bit range and decimal literals such as `1.10` are carried as their
//! source text. Values without a JSON counterpart follow the plain JSON form
//! of the language: instants and dates as RFC 3339 text, sizes in bytes,
//! durations in milliseconds and binary data as standard base64 text.

use jaq_json::{Map, Num, Rc, Val};
use nurt_contracts::record::ValueType;
use nurt_contracts::value::{Bytes, Fields, Value};

/// Converts a language value to a `jq` value.
pub(crate) fn to_val(value: &Value) -> Val {
    match value {
        Value::Null => Val::Null,
        Value::Bool(flag) => Val::Bool(*flag),
        Value::Int(number) => Val::Num(Num::from_integral(*number)),
        Value::BigInt(text) | Value::Decimal(text) => number_from_text(text),
        Value::Number(number) => Val::from(*number),
        Value::Text(text) => Val::from(text.clone()),
        Value::Bytes(bytes) => Val::from(bytes.to_base64()),
        Value::List(items) => Val::Arr(Rc::new(items.iter().map(to_val).collect())),
        Value::Map(fields) => object(fields),
        Value::Record(record) => object(&record.fields),
        other => json_to_val(other.to_plain_json()),
    }
}

fn object(fields: &Fields) -> Val {
    let mut map = Map::default();
    for (name, value) in fields {
        map.insert(Val::from(name.clone()), to_val(value));
    }
    Val::obj(map)
}

fn number_from_text(text: &str) -> Val {
    jaq_json::read::parse_single_num(text.as_bytes())
        .map_or_else(|| Val::from(text.to_owned()), Val::Num)
}

fn json_to_val(json: serde_json::Value) -> Val {
    match json {
        serde_json::Value::Null => Val::Null,
        serde_json::Value::Bool(flag) => Val::Bool(flag),
        serde_json::Value::Number(number) => number_from_text(&number.to_string()),
        serde_json::Value::String(text) => Val::from(text),
        serde_json::Value::Array(items) => {
            Val::Arr(Rc::new(items.into_iter().map(json_to_val).collect()))
        }
        serde_json::Value::Object(members) => {
            let mut map = Map::default();
            for (name, value) in members {
                map.insert(Val::from(name), json_to_val(value));
            }
            Val::obj(map)
        }
    }
}

/// Converts a `jq` value to a language value.
///
/// Object keys that are not strings, which `jq` filters cannot produce from
/// documents but may construct, are written as their JSON text.
pub(crate) fn from_val(val: &Val) -> Value {
    match val {
        Val::Null => Value::Null,
        Val::Bool(flag) => Value::Bool(*flag),
        Val::Num(number) => from_num(number),
        Val::TStr(text) => Value::Text(String::from_utf8_lossy(text).into_owned()),
        Val::BStr(bytes) => Value::Bytes(Bytes::new(bytes.to_vec())),
        Val::Arr(items) => Value::List(items.iter().map(from_val).collect()),
        Val::Obj(members) => {
            let mut fields = Fields::new();
            for (name, value) in members.iter() {
                fields.insert(key_text(name), from_val(value));
            }
            Value::Map(fields)
        }
    }
}

/// Text of an object key or a path step.
pub(crate) fn key_text(val: &Val) -> String {
    match val {
        Val::TStr(text) | Val::BStr(text) => String::from_utf8_lossy(text).into_owned(),
        other => other.to_string(),
    }
}

fn from_num(number: &Num) -> Value {
    match number {
        Num::Int(int) => {
            i64::try_from(*int).map_or_else(|_| Value::BigInt(int.to_string()), Value::Int)
        }
        Num::BigInt(int) => {
            let text = int.to_string();
            text.parse::<i64>().map_or(Value::BigInt(text), Value::Int)
        }
        Num::Float(float) => from_float(*float),
        Num::Dec(text) => Value::Decimal(text.to_string()),
    }
}

/// `jq` prints NaN as `null` and infinities as the largest finite numbers.
fn from_float(float: f64) -> Value {
    if float.is_nan() {
        Value::Null
    } else if float.is_infinite() {
        Value::Number(f64::MAX.copysign(float))
    } else {
        Value::Number(float)
    }
}

/// Type of a value as reported in the `value_type` field of `item` records.
#[must_use]
pub fn value_type(value: &Value) -> ValueType {
    match value {
        Value::Null => ValueType::Null,
        Value::Bool(_) => ValueType::Bool,
        Value::Int(_) | Value::BigInt(_) => ValueType::Int,
        Value::Number(_) => ValueType::Number,
        Value::Decimal(text) if is_integer_text(text) => ValueType::Int,
        Value::Decimal(_) => ValueType::Number,
        Value::List(_) => ValueType::List,
        Value::Map(_) | Value::Record(_) => ValueType::Map,
        Value::Bytes(_) => ValueType::Bytes,
        Value::Date(_) | Value::Instant(_) => ValueType::Date,
        _ => ValueType::Text,
    }
}

fn is_integer_text(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::value::Record;
    use nurt_contracts::value::RecordKind;

    fn round_trip(value: Value) -> Value {
        from_val(&to_val(&value))
    }

    #[test]
    fn keeps_exact_numbers() {
        assert_eq!(round_trip(Value::Int(-7)), Value::Int(-7));
        assert_eq!(
            round_trip(Value::Decimal("1.10".to_owned())),
            Value::Decimal("1.10".to_owned())
        );
        let big = "123456789012345678901234567890".to_owned();
        assert_eq!(round_trip(Value::BigInt(big.clone())), Value::BigInt(big));
        assert_eq!(round_trip(Value::Number(0.5)), Value::Number(0.5));
    }

    #[test]
    fn converts_containers_in_order() {
        let mut fields = Fields::new();
        fields.insert("z".to_owned(), Value::Bool(true));
        fields.insert(
            "a".to_owned(),
            Value::List(vec![Value::Null, Value::Text("x".to_owned())]),
        );
        let map = Value::Map(fields);
        assert_eq!(round_trip(map.clone()), map);
        let record = Record::new(RecordKind::Map).with("n", Value::Int(1));
        let mut expected = Fields::new();
        expected.insert("n".to_owned(), Value::Int(1));
        assert_eq!(round_trip(Value::Record(record)), Value::Map(expected));
    }

    #[test]
    fn writes_bytes_as_base64_text() {
        let bytes = Value::Bytes(Bytes::new(b"hi".to_vec()));
        assert_eq!(round_trip(bytes), Value::Text("aGk=".to_owned()));
        assert_eq!(
            from_val(&Val::byte_str(b"hi".to_vec())),
            Value::Bytes(Bytes::new(b"hi".to_vec()))
        );
    }

    #[test]
    fn maps_non_finite_floats_like_jq() {
        assert_eq!(from_float(f64::NAN), Value::Null);
        assert_eq!(from_float(f64::NEG_INFINITY), Value::Number(-f64::MAX));
    }

    #[test]
    fn reports_value_types() {
        assert_eq!(
            value_type(&Value::Decimal("-12".to_owned())),
            ValueType::Int
        );
        assert_eq!(
            value_type(&Value::Decimal("1e5".to_owned())),
            ValueType::Number
        );
        assert_eq!(value_type(&Value::Text(String::new())), ValueType::Text);
        assert_eq!(
            value_type(&Value::Bytes(Bytes::new(Vec::new()))),
            ValueType::Bytes
        );
        assert_eq!(value_type(&Value::Map(Fields::new())), ValueType::Map);
    }

    #[test]
    fn names_non_text_keys_by_json() {
        assert_eq!(key_text(&Val::from(3.5)), "3.5");
        assert_eq!(key_text(&Val::from("k".to_owned())), "k");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
