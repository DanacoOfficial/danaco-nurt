// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::cmp::Ordering;

use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::Value;

/// Largest magnitude below which every integer is exact in `f64`.
const EXACT_FLOAT_LIMIT: f64 = 9_007_199_254_740_992.0;

/// Returns `true` for integers and numbers in any representation.
#[must_use]
pub const fn numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Int(_) | Value::BigInt(_) | Value::Number(_) | Value::Decimal(_)
    )
}

/// Order of two numeric values; `None` when either is not numeric or a
/// value is not a number (`nan`).
///
/// ```
/// use std::cmp::Ordering;
/// use nurt_contracts::value::Value;
/// use nurt_doc::value::compare_numbers;
///
/// let big = Value::BigInt("18446744073709551616".to_owned());
/// assert_eq!(compare_numbers(&Value::Int(3), &big), Some(Ordering::Less));
/// ```
#[must_use]
pub fn compare_numbers(left: &Value, right: &Value) -> Option<Ordering> {
    if let (Some(left), Some(right)) = (exact_integer(left), exact_integer(right)) {
        return Some(left.cmp(&right));
    }
    as_float(left)?.partial_cmp(&as_float(right)?)
}

/// Returns `true` when two document values mean the same.
///
/// Numbers are compared by value whatever their representation (`1`,
/// `1.0`, `1e0`, `1_000` and `1000`, the lexeme `1.10` and the number
/// `1.1`); two `nan` values are the same. A date is the same as its text.
/// Maps are compared by member name, regardless of the order of members;
/// lists element by element. Other values compare exactly.
///
/// ```
/// use nurt_contracts::value::{Date, DateKind, Value};
/// use nurt_doc::value::equivalent;
///
/// let date = Value::Date(Date { kind: DateKind::LocalDate, text: "2026-11-22".to_owned() });
/// assert!(equivalent(&date, &Value::Text("2026-11-22".to_owned())));
/// assert!(equivalent(&Value::Int(2), &Value::Number(2.0)));
/// assert!(!equivalent(&Value::Int(2), &Value::Text("2".to_owned())));
/// ```
#[must_use]
pub fn equivalent(left: &Value, right: &Value) -> bool {
    !differs(left, right, &mut Vec::new())
}

/// Key path of the first place where two values differ, `None` when they
/// are [`equivalent`].
///
/// Members of `left` are visited in order, then members only `right` has.
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::value::first_difference;
///
/// let left = Value::List(vec![Value::Int(1), Value::Int(2)]);
/// let right = Value::List(vec![Value::Int(1), Value::Int(3)]);
/// assert_eq!(first_difference(&left, &right).map(|key| key.to_string()).as_deref(), Some("[1]"));
/// ```
#[must_use]
pub fn first_difference(left: &Value, right: &Value) -> Option<KeyPath> {
    let mut path = Vec::new();
    differs(left, right, &mut path).then(|| KeyPath::new(None, path).unwrap_or_default())
}

/// Returns `true` when the values differ; `path` then holds the location of
/// the first difference.
fn differs(left: &Value, right: &Value, path: &mut Vec<KeySegment>) -> bool {
    match (left, right) {
        (Value::Map(left), Value::Map(right)) => {
            for (name, value) in left {
                path.push(KeySegment::Key(name.clone()));
                match right.get(name) {
                    Some(item) if !differs(value, item, path) => {}
                    _ => return true,
                }
                path.pop();
            }
            match right.keys().find(|name| !left.contains_key(*name)) {
                Some(name) => {
                    path.push(KeySegment::Key(name.clone()));
                    true
                }
                None => false,
            }
        }
        (Value::List(left), Value::List(right)) => {
            for (index, value) in left.iter().enumerate() {
                path.push(KeySegment::Index(to_index(index)));
                match right.get(index) {
                    Some(item) if !differs(value, item, path) => {}
                    _ => return true,
                }
                path.pop();
            }
            if right.len() > left.len() {
                path.push(KeySegment::Index(to_index(left.len())));
                return true;
            }
            false
        }
        _ => !scalar_equivalent(left, right),
    }
}

fn to_index(position: usize) -> i64 {
    i64::try_from(position).unwrap_or(i64::MAX)
}

fn scalar_equivalent(left: &Value, right: &Value) -> bool {
    if numeric(left) && numeric(right) {
        return compare_numbers(left, right) == Some(Ordering::Equal)
            || (as_float(left).is_some_and(f64::is_nan)
                && as_float(right).is_some_and(f64::is_nan));
    }
    match (text_form(left), text_form(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

/// Text of a text or date value.
fn text_form(value: &Value) -> Option<&str> {
    match value {
        Value::Text(text) => Some(text),
        Value::Date(date) => Some(&date.text),
        _ => None,
    }
}

/// Integer value of an integral number, `None` for fractions and values
/// outside 128 bits.
fn exact_integer(value: &Value) -> Option<i128> {
    match value {
        Value::Int(number) => Some(i128::from(*number)),
        Value::BigInt(digits) => digits.parse().ok(),
        Value::Number(number) => float_integer(*number),
        Value::Decimal(lexeme) => lexeme
            .replace('_', "")
            .parse::<i128>()
            .ok()
            .or_else(|| float_integer(lexeme.parse().ok()?)),
        _ => None,
    }
}

fn float_integer(number: f64) -> Option<i128> {
    // Integral and below 2^53 in magnitude, so the conversion is exact.
    (number.fract() == 0.0 && number.abs() < EXACT_FLOAT_LIMIT).then_some(number as i128)
}

fn as_float(value: &Value) -> Option<f64> {
    match value {
        Value::Int(number) => Some(*number as f64),
        Value::BigInt(digits) => digits.parse().ok(),
        Value::Number(number) => Some(*number),
        Value::Decimal(lexeme) => lexeme
            .replace('_', "")
            .to_ascii_lowercase()
            .replace(".inf", "inf")
            .replace(".nan", "nan")
            .parse()
            .ok(),
        _ => None,
    }
}
