// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of CIM property values to language values.
//!
//! The COM layer reads every `VARIANT` into a [`RawValue`] together with the
//! CIM type reported by `IWbemClassObject::Get`; this module applies the
//! rules that depend on the CIM type. WMI delivers 64-bit integers
//! (`CIM_SINT64`, `CIM_UINT64`) and `CIM_DATETIME` values as strings, so a
//! string is converted by its CIM type and not by its `VARIANT` type.

use nurt_contracts::value::{Fields, Value};

use super::datetime::{self, CimTime};

/// `CIM_SINT64` of `CIMTYPE_ENUMERATION` (`wbemcli.h`).
const CIM_SINT64: i32 = 20;
/// `CIM_UINT64` of `CIMTYPE_ENUMERATION`.
const CIM_UINT64: i32 = 21;
/// `CIM_DATETIME` of `CIMTYPE_ENUMERATION`.
const CIM_DATETIME: i32 = 101;
/// `CIM_FLAG_ARRAY`, set on the CIM type of array properties.
const CIM_FLAG_ARRAY: i32 = 0x2000;

/// Deepest level of embedded objects converted to maps; deeper objects are
/// left out.
pub(crate) const MAX_DEPTH: usize = 4;

/// Name of the field that holds the class of an embedded object.
pub(crate) const CLASS_FIELD: &str = "__class";

/// Value of a CIM property as read from a `VARIANT`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawValue {
    /// `VT_EMPTY` or `VT_NULL`.
    Null,
    /// `VT_BOOL`.
    Bool(bool),
    /// `VT_I1`, `VT_I2`, `VT_I4`, `VT_I8`, `VT_INT`.
    Signed(i64),
    /// `VT_UI1`, `VT_UI2`, `VT_UI4`, `VT_UI8`, `VT_UINT`.
    Unsigned(u64),
    /// `VT_R4`, `VT_R8`.
    Real(f64),
    /// `VT_BSTR`.
    Text(String),
    /// `VT_ARRAY` of any of the other kinds.
    Array(Vec<RawValue>),
    /// Embedded object (`VT_UNKNOWN` holding an `IWbemClassObject`).
    Object(RawObject),
}

/// Embedded CIM object.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawObject {
    /// Value of the system property `__CLASS`.
    pub(crate) class: Option<String>,
    /// Non-system properties in the order of the provider.
    pub(crate) properties: Vec<RawProperty>,
}

/// One property with its CIM type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawProperty {
    /// Property name.
    pub(crate) name: String,
    /// CIM type as reported by `IWbemClassObject::Get`, with
    /// `CIM_FLAG_ARRAY` for arrays.
    pub(crate) cim_type: i32,
    /// Value.
    pub(crate) value: RawValue,
}

/// Converts the properties of an object at nesting level `depth` (0 for
/// the properties of a result object). Properties without a value are left
/// out.
pub(crate) fn fields(properties: Vec<RawProperty>, depth: usize) -> Fields {
    let mut fields = Fields::new();
    for property in properties {
        if let Some(value) = convert(property.value, property.cim_type, depth) {
            fields.insert(property.name, value);
        }
    }
    fields
}

/// Converts one value; `None` for a null value and for an object nested
/// deeper than [`MAX_DEPTH`].
pub(crate) fn convert(value: RawValue, cim_type: i32, depth: usize) -> Option<Value> {
    let base_type = cim_type & !CIM_FLAG_ARRAY;
    match value {
        RawValue::Null => None,
        RawValue::Bool(flag) => Some(Value::Bool(flag)),
        RawValue::Signed(number) => Some(Value::Int(number)),
        RawValue::Unsigned(number) => Some(unsigned(number)),
        RawValue::Real(number) if number.is_finite() => Some(Value::Number(number)),
        RawValue::Real(number) => Some(Value::Text(number.to_string())),
        RawValue::Text(text) => Some(text_value(text, base_type)),
        RawValue::Array(items) => Some(Value::List(
            items
                .into_iter()
                .map(|item| convert(item, base_type, depth).unwrap_or(Value::Null))
                .collect(),
        )),
        RawValue::Object(object) if depth < MAX_DEPTH => {
            let mut map = Fields::new();
            if let Some(class) = object.class {
                map.insert(CLASS_FIELD.to_owned(), Value::Text(class));
            }
            map.extend(fields(object.properties, depth + 1));
            Some(Value::Map(map))
        }
        RawValue::Object(_) => None,
    }
}

fn unsigned(number: u64) -> Value {
    i64::try_from(number).map_or_else(|_| Value::BigInt(number.to_string()), Value::Int)
}

fn text_value(text: String, base_type: i32) -> Value {
    match base_type {
        CIM_SINT64 => text.parse().map_or(Value::Text(text), Value::Int),
        CIM_UINT64 => text.parse().map_or(Value::Text(text), unsigned),
        CIM_DATETIME => match datetime::parse(&text) {
            Some(CimTime::Instant(instant)) => Value::Instant(instant),
            Some(CimTime::Interval(duration)) => Value::Duration(duration),
            None => Value::Text(text),
        },
        _ => Value::Text(text),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::Duration;

    use super::*;

    /// `CIM_STRING` of `CIMTYPE_ENUMERATION`.
    const CIM_STRING: i32 = 8;
    /// `CIM_UINT32` of `CIMTYPE_ENUMERATION`.
    const CIM_UINT32: i32 = 19;
    /// `CIM_OBJECT` of `CIMTYPE_ENUMERATION`.
    const CIM_OBJECT: i32 = 13;

    fn property(name: &str, cim_type: i32, value: RawValue) -> RawProperty {
        RawProperty {
            name: name.to_owned(),
            cim_type,
            value,
        }
    }

    fn nested(depth: usize) -> RawValue {
        let mut value = RawValue::Text("leaf".to_owned());
        for level in 0..depth {
            value = RawValue::Object(RawObject {
                class: Some(format!("Level{level}")),
                properties: vec![property("Inner", CIM_OBJECT, value)],
            });
        }
        value
    }

    #[test]
    fn cim_variant_conversion_table() {
        let text = |value: &str| RawValue::Text(value.to_owned());
        let cases: Vec<(RawValue, i32, Option<Value>)> = vec![
            (RawValue::Null, CIM_STRING, None),
            (RawValue::Bool(true), 11, Some(Value::Bool(true))),
            (RawValue::Signed(-5), 3, Some(Value::Int(-5))),
            (RawValue::Unsigned(7), CIM_UINT32, Some(Value::Int(7))),
            (
                RawValue::Unsigned(u64::MAX),
                CIM_UINT64,
                Some(Value::BigInt(u64::MAX.to_string())),
            ),
            (RawValue::Real(0.5), 5, Some(Value::Number(0.5))),
            (
                RawValue::Real(f64::NAN),
                5,
                Some(Value::Text("NaN".to_owned())),
            ),
            (
                text("Running"),
                CIM_STRING,
                Some(Value::Text("Running".to_owned())),
            ),
            (text("-42"), CIM_SINT64, Some(Value::Int(-42))),
            (
                text("17179869184"),
                CIM_UINT64,
                Some(Value::Int(17_179_869_184)),
            ),
            (
                text("18446744073709551615"),
                CIM_UINT64,
                Some(Value::BigInt("18446744073709551615".to_owned())),
            ),
            (text("n/a"), CIM_UINT64, Some(Value::Text("n/a".to_owned()))),
            (
                text("20261121103000.000000+000"),
                CIM_DATETIME,
                Some(Value::Instant("2026-11-21T10:30:00Z".parse().unwrap())),
            ),
            (
                text("00000000000130.000000:000"),
                CIM_DATETIME,
                Some(Value::Duration(Duration::from_millis(90_000))),
            ),
            (
                text("2026112110****.******+***"),
                CIM_DATETIME,
                Some(Value::Text("2026112110****.******+***".to_owned())),
            ),
            (
                RawValue::Array(vec![text("1"), RawValue::Null, text("3")]),
                CIM_UINT64 | CIM_FLAG_ARRAY,
                Some(Value::List(vec![Value::Int(1), Value::Null, Value::Int(3)])),
            ),
        ];
        for (raw, cim_type, expected) in cases {
            let described = format!("{raw:?} as {cim_type}");
            assert_eq!(convert(raw, cim_type, 0), expected, "{described}");
        }
    }

    #[test]
    fn cim_embedded_objects_become_maps_up_to_depth_four() {
        let converted = convert(nested(5), CIM_OBJECT, 0).unwrap();
        let mut level = &converted;
        for expected in (1..5).rev() {
            let Value::Map(map) = level else {
                panic!("expected a map at Level{expected}, got {level:?}");
            };
            assert_eq!(map[CLASS_FIELD], Value::Text(format!("Level{expected}")));
            match map.get("Inner") {
                Some(inner) => level = inner,
                None => {
                    assert_eq!(expected, 1, "only the fifth level is left out");
                    return;
                }
            }
        }
        panic!("the object nested below level four was not left out");
    }

    #[test]
    fn cim_fields_skip_null_properties_and_keep_order() {
        let converted = fields(
            vec![
                property("Name", CIM_STRING, RawValue::Text("spooler".to_owned())),
                property("Description", CIM_STRING, RawValue::Null),
                property("ProcessId", CIM_UINT32, RawValue::Unsigned(812)),
            ],
            0,
        );
        let names: Vec<&str> = converted.keys().map(String::as_str).collect();
        assert_eq!(names, ["Name", "ProcessId"]);
        assert_eq!(converted["ProcessId"], Value::Int(812));
    }
}
