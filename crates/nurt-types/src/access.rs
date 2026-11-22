// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Access to the fields of records and maps by name and by path.

use nurt_contracts::error::Result;
use nurt_contracts::value::Value;

use crate::errors::{type_error, unknown_field};

/// Names of the fields of a record or the keys of a map, in order; empty
/// for other values.
///
/// The list is the `expected` member of `E_UNKNOWN_FIELD` and the field list
/// of `describe`.
#[must_use]
pub fn field_names(value: &Value) -> Vec<&str> {
    match value {
        Value::Record(record) => record.field_names().collect(),
        Value::Map(map) => map.keys().map(String::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Value of one path segment of `value`.
///
/// * record: the field; a missing field is `E_UNKNOWN_FIELD`;
/// * map: the entry; a missing key gives `null`, because maps have no fixed
///   keys;
/// * list: a segment of decimal digits is an index (`null` past the end);
///   any other segment reads the field of every item and gives the list of
///   results;
/// * `null`: `null`.
///
/// # Errors
///
/// * `E_UNKNOWN_FIELD` with the fields of the record in `expected`;
/// * `E_TYPE` when the value has no fields.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::{Record, RecordKind, Value};
/// use nurt_types::get_field;
///
/// let record = Record::new(RecordKind::Map).with("name", Value::from("a.txt"));
/// let list = Value::List(vec![Value::Record(record.clone()), Value::Record(record)]);
/// let names = get_field(&list, "name")?;
/// assert_eq!(names, Value::List(vec![Value::from("a.txt"), Value::from("a.txt")]));
/// assert!(get_field(&list, "size").is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn get_field(value: &Value, name: &str) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Record(record) => record
            .get(name)
            .cloned()
            .ok_or_else(|| unknown_field(name, Some(record.kind.as_str()), record.field_names())),
        Value::Map(map) => Ok(map.get(name).cloned().unwrap_or(Value::Null)),
        Value::List(items) => list_field(items, name),
        other => Err(type_error(
            &format!("field `{name}`"),
            &["record", "map", "list"],
            other,
        )),
    }
}

/// Value at a path of segments, each read with [`get_field`]: `a.b.0`.
///
/// # Errors
///
/// As [`get_field`], for the first segment that fails.
pub fn get_path<S: AsRef<str>>(value: &Value, path: &[S]) -> Result<Value> {
    path.iter().try_fold(value.clone(), |current, segment| {
        get_field(&current, segment.as_ref())
    })
}

fn list_field(items: &[Value], name: &str) -> Result<Value> {
    if !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()) {
        let item = name
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get(index));
        return Ok(item.cloned().unwrap_or(Value::Null));
    }
    items
        .iter()
        .map(|item| get_field(item, name))
        .collect::<Result<Vec<_>>>()
        .map(Value::List)
}
