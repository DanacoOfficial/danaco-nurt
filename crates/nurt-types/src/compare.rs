// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Equality and the total order of values.
//!
//! Values of different type families are ordered by family:
//! `null` < `bool` < numbers < `size` < `duration` < `instant` < `date` <
//! text and enumeration values < `path` < `glob` < `key` < `regex` <
//! `bytes` < `range` < `list` < `map` < `record`. Within a family:
//!
//! * numbers (`int`, `number` and the integers and decimals kept as source
//!   text) compare by their mathematical value without loss of precision;
//!   a NaN is greater than every other number and equal to itself;
//! * text, enumeration values, paths, globs and keys compare by Unicode code
//!   points, without locale rules;
//! * lists compare element by element, then by length; maps and records by
//!   their entries in order (records first by kind);
//! * ranges by the start (an open start first), then by the end (an open
//!   end last).

use std::cmp::Ordering;

use nurt_contracts::value::{Fields, Range, Value};

const RANK_NULL: u8 = 0;
const RANK_BOOL: u8 = 1;
const RANK_NUMBER: u8 = 2;
const RANK_SIZE: u8 = 3;
const RANK_DURATION: u8 = 4;
const RANK_INSTANT: u8 = 5;
const RANK_DATE: u8 = 6;
const RANK_TEXT: u8 = 7;
const RANK_PATH: u8 = 8;
const RANK_GLOB: u8 = 9;
const RANK_KEY: u8 = 10;
const RANK_REGEX: u8 = 11;
const RANK_BYTES: u8 = 12;
const RANK_RANGE: u8 = 13;
const RANK_LIST: u8 = 14;
const RANK_MAP: u8 = 15;
const RANK_RECORD: u8 = 16;
const RANK_OTHER: u8 = 17;

/// 2^63 as a float: the first float above every `i64`.
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
const MANTISSA_BITS: u32 = 52;
const EXPONENT_MASK: u64 = 0x7ff;
const EXPONENT_BIAS: i64 = 1_075;

/// Total order of values; see the module documentation for the rules.
///
/// The order is reflexive, antisymmetric and transitive over all values,
/// so it serves `sort`, `distinct` and `group`.
///
/// # Examples
///
/// ```rust
/// use std::cmp::Ordering;
/// use nurt_contracts::value::Value;
/// use nurt_types::compare;
///
/// assert_eq!(compare(&Value::Int(2), &Value::Number(2.5)), Ordering::Less);
/// assert_eq!(compare(&Value::Int(1), &Value::Number(1.0)), Ordering::Equal);
/// assert_eq!(compare(&Value::Null, &Value::Bool(false)), Ordering::Less);
/// assert_eq!(compare(&Value::from("Z"), &Value::from("a")), Ordering::Less);
/// ```
#[must_use]
pub fn compare(left: &Value, right: &Value) -> Ordering {
    let (left_rank, right_rank) = (rank(left), rank(right));
    if left_rank != right_rank {
        return left_rank.cmp(&right_rank);
    }
    compare_same_family(left, right)
}

/// Equality of the language (`==`): [`compare`] gives `Equal`.
///
/// An `int` equals a `number` of the same value, and a text equals an
/// enumeration value with the same text.
#[must_use]
pub fn equal(left: &Value, right: &Value) -> bool {
    compare(left, right) == Ordering::Equal
}

/// Whether the two values belong to the same type family, so that `<`,
/// `<=`, `>` and `>=` apply to them.
#[must_use]
pub fn comparable(left: &Value, right: &Value) -> bool {
    rank(left) == rank(right)
}

/// Type names of the family of `value`, as listed in `E_TYPE` errors of
/// ordering operators.
pub(crate) fn family_types(value: &Value) -> Vec<&'static str> {
    match rank(value) {
        RANK_NUMBER => vec!["int", "number"],
        RANK_TEXT => vec!["text", "enum"],
        _ => vec![value.type_name()],
    }
}

/// Value ordered and compared by [`compare`], for use as a key of ordered
/// collections.
///
/// # Examples
///
/// ```rust
/// use std::collections::BTreeSet;
/// use nurt_contracts::value::Value;
/// use nurt_types::OrderedValue;
///
/// let distinct: BTreeSet<OrderedValue> =
///     [Value::Int(1), Value::Number(1.0), Value::Int(0)].into_iter().map(OrderedValue).collect();
/// assert_eq!(distinct.len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct OrderedValue(pub Value);

impl PartialEq for OrderedValue {
    fn eq(&self, other: &Self) -> bool {
        equal(&self.0, &other.0)
    }
}

impl Eq for OrderedValue {}

impl PartialOrd for OrderedValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedValue {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(&self.0, &other.0)
    }
}

impl From<Value> for OrderedValue {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

const fn rank(value: &Value) -> u8 {
    match value {
        Value::Null => RANK_NULL,
        Value::Bool(_) => RANK_BOOL,
        Value::Int(_) | Value::BigInt(_) | Value::Number(_) | Value::Decimal(_) => RANK_NUMBER,
        Value::Size(_) => RANK_SIZE,
        Value::Duration(_) => RANK_DURATION,
        Value::Instant(_) => RANK_INSTANT,
        Value::Date(_) => RANK_DATE,
        Value::Text(_) | Value::Enum(_) => RANK_TEXT,
        Value::Path(_) => RANK_PATH,
        Value::Glob(_) => RANK_GLOB,
        Value::Key(_) => RANK_KEY,
        Value::Regex(_) => RANK_REGEX,
        Value::Bytes(_) => RANK_BYTES,
        Value::Range(_) => RANK_RANGE,
        Value::List(_) => RANK_LIST,
        Value::Map(_) => RANK_MAP,
        Value::Record(_) => RANK_RECORD,
        _ => RANK_OTHER,
    }
}

fn compare_same_family(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Size(a), Value::Size(b)) => a.cmp(b),
        (Value::Duration(a), Value::Duration(b)) => a.cmp(b),
        (Value::Instant(a), Value::Instant(b)) => a.cmp(b),
        (Value::Date(a), Value::Date(b)) => a.text.cmp(&b.text),
        (Value::Regex(a), Value::Regex(b)) => a
            .pattern
            .cmp(&b.pattern)
            .then_with(|| a.flags.to_string().cmp(&b.flags.to_string())),
        (Value::Bytes(a), Value::Bytes(b)) => a.as_slice().cmp(b.as_slice()),
        (Value::Range(a), Value::Range(b)) => compare_ranges(*a, *b),
        (Value::List(a), Value::List(b)) => compare_lists(a, b),
        (Value::Map(a), Value::Map(b)) => compare_fields(a, b),
        (Value::Record(a), Value::Record(b)) => a
            .kind
            .cmp(&b.kind)
            .then_with(|| compare_fields(&a.fields, &b.fields)),
        _ => compare_scalars(left, right),
    }
}

fn compare_scalars(left: &Value, right: &Value) -> Ordering {
    if let (Some(a), Some(b)) = (Number::of(left), Number::of(right)) {
        return compare_numbers(a, b);
    }
    match (left.as_str(), right.as_str()) {
        (Some(a), Some(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

fn compare_ranges(left: Range, right: Range) -> Ordering {
    let start = |range: Range| range.start().map_or((0, 0), |value| (1, value));
    let end = |range: Range| range.end().map_or((1, 0), |value| (0, value));
    start(left)
        .cmp(&start(right))
        .then_with(|| end(left).cmp(&end(right)))
}

fn compare_lists(left: &[Value], right: &[Value]) -> Ordering {
    left.iter()
        .zip(right)
        .map(|(a, b)| compare(a, b))
        .find(|ordering| ordering.is_ne())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn compare_fields(left: &Fields, right: &Fields) -> Ordering {
    left.iter()
        .zip(right)
        .map(|((left_key, left_value), (right_key, right_value))| {
            left_key
                .cmp(right_key)
                .then_with(|| compare(left_value, right_value))
        })
        .find(|ordering| ordering.is_ne())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

/// Numeric payload of a value of the number family.
#[derive(Clone, Copy)]
enum Number<'a> {
    Int(i64),
    Float(f64),
    Text(&'a str),
}

impl<'a> Number<'a> {
    fn of(value: &'a Value) -> Option<Self> {
        match value {
            Value::Int(int) => Some(Self::Int(*int)),
            Value::Number(float) => Some(Self::Float(*float)),
            Value::BigInt(text) | Value::Decimal(text) => Some(Self::Text(text)),
            _ => None,
        }
    }
}

fn compare_numbers(left: Number<'_>, right: Number<'_>) -> Ordering {
    match (left, right) {
        (Number::Int(a), Number::Int(b)) => a.cmp(&b),
        (Number::Int(a), Number::Float(b)) => compare_int_float(a, b),
        (Number::Float(a), Number::Int(b)) => compare_int_float(b, a).reverse(),
        (Number::Float(a), Number::Float(b)) => compare_floats(a, b),
        _ => compare_decimals(Decimal::of(left), Decimal::of(right)),
    }
}

/// Exact comparison of an integer with a float.
fn compare_int_float(int: i64, float: f64) -> Ordering {
    if float.is_nan() || float >= TWO_POW_63 {
        return Ordering::Less;
    }
    if float < -TWO_POW_63 {
        return Ordering::Greater;
    }
    let whole = float.trunc();
    // The float lies in [-2^63, 2^63), so its integer part converts exactly.
    let whole_int = whole as i64;
    int.cmp(&whole_int)
        .then_with(|| whole.partial_cmp(&float).unwrap_or(Ordering::Equal))
}

fn compare_floats(left: f64, right: f64) -> Ordering {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}

/// Malformed numbers and NaN sort after every well-formed number.
fn compare_decimals(left: Option<Decimal>, right: Option<Decimal>) -> Ordering {
    match (left, right) {
        (Some(a), Some(b)) => a.cmp(&b),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

/// Decimal number `0.d1 d2 d3 … × 10^exponent`, normalised: no leading or
/// trailing zero digits, zero has no digits and is not negative.
#[derive(Debug, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    digits: Vec<u8>,
    exponent: i64,
}

impl Decimal {
    fn of(number: Number<'_>) -> Option<Self> {
        match number {
            Number::Int(int) => Self::parse(&int.to_string()),
            Number::Float(float) => Self::of_float(float),
            Number::Text(text) => Self::parse(text),
        }
    }

    /// Integer-valued floats convert exactly; other finite floats convert
    /// through their shortest round-trip form, which lies on the same side
    /// of every integer and every other float as the float itself.
    fn of_float(float: f64) -> Option<Self> {
        if float.is_nan() {
            return None;
        }
        if float.is_infinite() {
            return Some(Self {
                negative: float < 0.0,
                digits: vec![1],
                exponent: i64::MAX,
            });
        }
        if float.fract() == 0.0 {
            let digits = exact_integer_digits(float);
            let text = if float.is_sign_negative() {
                format!("-{digits}")
            } else {
                digits
            };
            return Self::parse(&text);
        }
        Self::parse(&format!("{float:e}"))
    }

    /// Parses `-?[0-9]*(.[0-9]*)?([eE][+-]?[0-9]+)?` with at least one digit.
    fn parse(text: &str) -> Option<Self> {
        let (negative, body) = match text.strip_prefix('-') {
            Some(body) => (true, body),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (mantissa, exponent) = match body.split_once(['e', 'E']) {
            Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
            None => (body, 0),
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let all_digits = whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit());
        if !all_digits || (whole.is_empty() && fraction.is_empty()) {
            return None;
        }
        let mut digits: Vec<u8> = whole
            .bytes()
            .chain(fraction.bytes())
            .map(|byte| byte - b'0')
            .collect();
        let leading = digits.iter().take_while(|&&digit| digit == 0).count();
        let exponent = exponent
            .checked_add(i64::try_from(whole.len()).ok()?)?
            .checked_sub(i64::try_from(leading).ok()?)?;
        digits.drain(..leading);
        while digits.last() == Some(&0) {
            digits.pop();
        }
        if digits.is_empty() {
            return Some(Self {
                negative: false,
                digits,
                exponent: 0,
            });
        }
        Some(Self {
            negative,
            digits,
            exponent,
        })
    }

    fn sign(&self) -> i8 {
        match (self.digits.is_empty(), self.negative) {
            (true, _) => 0,
            (false, true) => -1,
            (false, false) => 1,
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        let (left, right) = (self.sign(), other.sign());
        if left != right || left == 0 {
            return left.cmp(&right);
        }
        let magnitude = self
            .exponent
            .cmp(&other.exponent)
            .then_with(|| self.digits.cmp(&other.digits));
        if left < 0 {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}

/// Decimal digits of the absolute value of an integer-valued finite float.
fn exact_integer_digits(float: f64) -> String {
    let bits = float.to_bits();
    let biased = i64::try_from((bits >> MANTISSA_BITS) & EXPONENT_MASK).unwrap_or(0);
    let fraction = bits & ((1_u64 << MANTISSA_BITS) - 1);
    let (mantissa, shift) = if biased == 0 {
        (fraction, 1 - EXPONENT_BIAS)
    } else {
        (fraction | (1_u64 << MANTISSA_BITS), biased - EXPONENT_BIAS)
    };
    if shift <= 0 {
        let divisor = u32::try_from(-shift).unwrap_or(u32::MAX);
        return mantissa.checked_shr(divisor).unwrap_or(0).to_string();
    }
    // Little-endian decimal digits, doubled once per binary exponent step.
    let mut digits: Vec<u8> = mantissa
        .to_string()
        .bytes()
        .rev()
        .map(|byte| byte - b'0')
        .collect();
    for _ in 0..shift {
        let mut carry = 0;
        for digit in &mut digits {
            let doubled = *digit * 2 + carry;
            *digit = doubled % 10;
            carry = doubled / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
    }
    digits
        .iter()
        .rev()
        .map(|digit| char::from(b'0' + digit))
        .collect()
}
