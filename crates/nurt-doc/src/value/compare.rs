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
    if let (Some(left), Some(right)) = (Decimal::of(left), Decimal::of(right)) {
        return Some(left.compare(&right));
    }
    as_float(left)?.partial_cmp(&as_float(right)?)
}

/// Largest exponent of a lexeme compared as an exact decimal; larger ones
/// are compared as floating point numbers.
const MAX_EXACT_EXPONENT: i64 = 1_000_000;

/// Exact value of a finite number of any representation and size, without
/// rounding: `0.DIGITS × 10^order`. `digits` holds no leading and no
/// trailing zeros and is empty for zero, which is never negative.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    digits: String,
    order: i64,
}

impl Decimal {
    /// Exact value of a numeric `value`; `None` for other values, for
    /// `nan` and infinities, and for lexemes this form does not cover.
    fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Int(number) => Self::parse(&number.to_string()),
            Value::BigInt(digits) => Self::parse(digits),
            Value::Decimal(lexeme) => Self::parse(&lexeme.replace('_', "")),
            Value::Number(number) if number.is_finite() => Self::parse(&format!("{number:?}")),
            _ => None,
        }
    }

    /// Value of `[+-]digits[.digits][(e|E)[+-]digits]`.
    fn parse(text: &str) -> Option<Self> {
        let (negative, unsigned) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
            Some(at) => (&unsigned[..at], unsigned[at + 1..].parse::<i64>().ok()?),
            None => (unsigned, 0),
        };
        if exponent.unsigned_abs() > MAX_EXACT_EXPONENT.unsigned_abs() {
            return None;
        }
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        if whole.len() + fraction.len() == 0 || !all_digits(whole) || !all_digits(fraction) {
            return None;
        }
        let joined = format!("{whole}{fraction}");
        let significant = joined.trim_start_matches('0');
        let leading = i64::try_from(joined.len() - significant.len()).ok()?;
        let digits = significant.trim_end_matches('0');
        if digits.is_empty() {
            return Some(Self {
                negative: false,
                digits: String::new(),
                order: 0,
            });
        }
        let order = i64::try_from(whole.len()).ok()? - leading + exponent;
        Some(Self {
            negative,
            digits: digits.to_owned(),
            order,
        })
    }

    fn compare(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (negative, _) => {
                let magnitude = match (self.digits.is_empty(), other.digits.is_empty()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => self
                        .order
                        .cmp(&other.order)
                        .then_with(|| self.digits.cmp(&other.digits)),
                };
                if negative { magnitude.reverse() } else { magnitude }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn big(text: &str) -> Value {
        Value::BigInt(text.to_owned())
    }

    #[test]
    fn integers_beyond_128_bits_are_compared_digit_by_digit() {
        let low = big(&format!("1{}", "0".repeat(40)));
        let high = big(&format!("1{}1", "0".repeat(39)));
        assert_eq!(compare_numbers(&low, &high), Some(Ordering::Less));
        assert_eq!(compare_numbers(&high, &low), Some(Ordering::Greater));
        assert_eq!(compare_numbers(&low, &low.clone()), Some(Ordering::Equal));
        assert!(!equivalent(&low, &high));
    }

    #[test]
    fn integers_beyond_the_range_of_a_float_stay_distinct() {
        let first = big(&format!("1{}", "0".repeat(400)));
        let second = big(&format!("2{}", "0".repeat(400)));
        assert_eq!(compare_numbers(&first, &second), Some(Ordering::Less));
        assert!(!equivalent(&first, &second));
        assert!(equivalent(&first, &first.clone()));
    }

    #[test]
    fn negative_and_mixed_representations_order_correctly() {
        let negative = big(&format!("-1{}", "0".repeat(40)));
        let smaller = big(&format!("-2{}", "0".repeat(40)));
        assert_eq!(compare_numbers(&smaller, &negative), Some(Ordering::Less));
        assert_eq!(
            compare_numbers(&negative, &Value::Int(5)),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_numbers(&Value::Int(5), &negative),
            Some(Ordering::Greater)
        );
        let decimal = Value::Decimal("9".repeat(45));
        let nines = big(&"9".repeat(45));
        assert!(equivalent(&decimal, &nines));
        assert!(!equivalent(&decimal, &big(&format!("{}8", "9".repeat(44)))));
    }

    #[test]
    fn long_decimal_fractions_are_compared_exactly() {
        let short = Value::Decimal("0.1000000000000000".to_owned());
        let longer = Value::Decimal("0.1000000000000000000001".to_owned());
        assert_eq!(compare_numbers(&short, &longer), Some(Ordering::Less));
        assert!(!equivalent(&short, &longer));
        assert!(!equivalent(&Value::Number(0.1), &longer));
        assert!(equivalent(&Value::Number(0.1), &short));
        let left = Value::from_plain_json(serde_json::json!({"a": [1.5]}));
        let right = Value::Map(
            [(
                "a".to_owned(),
                Value::List(vec![Value::Decimal("1.5000000000000000001".to_owned())]),
            )]
            .into_iter()
            .collect(),
        );
        let path = first_difference(&left, &right).map(|key| key.to_string());
        assert_eq!(path.as_deref(), Some("a[0]"));
    }

    #[test]
    fn decimals_with_exponents_signs_and_zeros_are_ordered_by_value() {
        let decimal = |text: &str| Value::Decimal(text.to_owned());
        let cases = [
            ("1e5", "100000.0", Ordering::Equal),
            ("1e-3", "0.001", Ordering::Equal),
            ("0.0", "-0.0", Ordering::Equal),
            ("-1.5", "-1.25", Ordering::Less),
            ("-1e400", "1e-400", Ordering::Less),
            ("1e400", "1e399", Ordering::Greater),
            ("0.5", ".5", Ordering::Equal),
            ("123456.789e-3", "123.456789", Ordering::Equal),
        ];
        for (left, right, expected) in cases {
            assert_eq!(
                compare_numbers(&decimal(left), &decimal(right)),
                Some(expected),
                "{left} {right}"
            );
        }
    }

    #[test]
    fn numbers_compare_by_value_in_any_representation() {
        assert!(equivalent(&Value::Int(1), &Value::Number(1.0)));
        assert!(equivalent(
            &Value::Decimal("1_000".to_owned()),
            &Value::Int(1000)
        ));
        assert!(equivalent(
            &Value::Decimal("1.10".to_owned()),
            &Value::Number(1.1)
        ));
        assert!(equivalent(
            &Value::Decimal(".nan".to_owned()),
            &Value::Number(f64::NAN)
        ));
        assert!(!equivalent(&Value::Int(2), &Value::Text("2".to_owned())));
        assert_eq!(compare_numbers(&Value::Int(1), &Value::Text("1".to_owned())), None);
    }

    #[test]
    fn first_difference_names_the_member_and_the_index() {
        let left = Value::from_plain_json(serde_json::json!({"a": [1, {"b": 2}], "c": 1}));
        let right = Value::from_plain_json(serde_json::json!({"a": [1, {"b": 3}], "c": 1}));
        let path = first_difference(&left, &right).map(|key| key.to_string());
        assert_eq!(path.as_deref(), Some("a[1].b"));
        let extra = Value::from_plain_json(serde_json::json!({"a": [1, {"b": 2}], "c": 1, "d": 0}));
        let path = first_difference(&left, &extra).map(|key| key.to_string());
        assert_eq!(path.as_deref(), Some("d"));
        assert_eq!(first_difference(&left, &left.clone()), None);
    }

    #[test]
    fn map_order_does_not_matter_but_list_order_does() {
        let left = Value::from_plain_json(serde_json::json!({"a": 1, "b": 2}));
        let right = Value::from_plain_json(serde_json::json!({"b": 2, "a": 1}));
        assert!(equivalent(&left, &right));
        let first = Value::from_plain_json(serde_json::json!([1, 2]));
        let second = Value::from_plain_json(serde_json::json!([2, 1]));
        assert!(!equivalent(&first, &second));
    }
}
