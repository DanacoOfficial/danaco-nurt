// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Arithmetic of the language.
//!
//! | Operation | Operands | Result |
//! |---|---|---|
//! | `+ - * /` | `int` and `int` | `int`; `/` gives `number` when the division is not exact |
//! | `+ - * /` | `int` or `number` with `number` | `number` |
//! | `+ -` | `size` and `size` | `size` |
//! | `* /` | `size` and `int` or `number` | `size`; `size / size` is a `number` |
//! | `+ -` | `duration` and `duration` | `duration` |
//! | `* /` | `duration` and `int` or `number` | `duration`; `duration / duration` is a `number` |
//! | `-` | `instant` and `instant` | `duration` |
//! | `+ -` | `instant` and `duration` | `instant` |
//! | `+` | `text` and `text` | `text` |
//!
//! An operand `null` makes the result `null`. Multiplying or dividing a
//! size or a duration by a `number` rounds to the nearest byte or
//! millisecond; dividing by an `int` truncates towards zero.

use nurt_contracts::error::Result;
use nurt_contracts::value::{Duration, Size, Value};

use crate::errors::{division_by_zero, overflow, type_error};

/// 2^63 as a float, the first float above every `i64`.
const I64_LIMIT: f64 = 9_223_372_036_854_775_808.0;
/// 2^64 as a float, the first float above every `u64`.
const U64_LIMIT: f64 = 18_446_744_073_709_551_616.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "`+`",
            Self::Sub => "`-`",
            Self::Mul => "`*`",
            Self::Div => "`/`",
        }
    }

    fn apply(self, left: f64, right: f64) -> f64 {
        match self {
            Self::Add => left + right,
            Self::Sub => left - right,
            Self::Mul => left * right,
            Self::Div => left / right,
        }
    }

    /// Types accepted on the left.
    const fn left_types(self) -> &'static [&'static str] {
        match self {
            Self::Add => &["int", "number", "size", "duration", "instant", "text"],
            Self::Sub => &["int", "number", "size", "duration", "instant"],
            Self::Mul | Self::Div => &["int", "number", "size", "duration"],
        }
    }

    /// Types accepted on the right for a left operand of type `left`.
    fn right_types(self, left: &str) -> &'static [&'static str] {
        match (self, left) {
            (Self::Mul, "int" | "number") => &["int", "number", "size", "duration"],
            (_, "int" | "number") => &["int", "number"],
            (Self::Add | Self::Sub, "size") => &["size"],
            (Self::Mul, "size" | "duration") => &["int", "number"],
            (Self::Div, "size") => &["int", "number", "size"],
            (Self::Div, "duration") => &["int", "number", "duration"],
            (Self::Add, "duration") => &["duration", "instant"],
            (Self::Sub, "duration") | (Self::Add, "instant") => &["duration"],
            (Self::Sub, "instant") => &["instant", "duration"],
            (Self::Add, "text") => &["text"],
            _ => &[],
        }
    }
}

/// Sum of two values, `left + right`.
///
/// # Errors
///
/// * `E_TYPE` when the operands cannot be added; `expected` lists the
///   accepted types;
/// * `E_INVALID_VALUE` with the problem `overflow` when the result does not
///   fit its type.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::{Size, Value};
/// use nurt_types::add;
///
/// let total = add(&Value::Size(Size::from_bytes(1_000)), &Value::Size(Size::from_bytes(24)))?;
/// assert_eq!(total, Value::Size(Size::from_bytes(1_024)));
/// assert_eq!(add(&Value::from("a"), &Value::from("b"))?, Value::from("ab"));
/// assert!(add(&Value::from("a"), &Value::Int(1)).is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn add(left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    let op = Op::Add;
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => checked(a.checked_add(*b).map(Value::Int), op, "int"),
        (Value::Text(a), Value::Text(b)) => Ok(Value::Text(format!("{a}{b}"))),
        (Value::Size(a), Value::Size(b)) => checked(a.checked_add(*b).map(Value::Size), op, "size"),
        (Value::Duration(a), Value::Duration(b)) => {
            checked(a.checked_add(*b).map(Value::Duration), op, "duration")
        }
        (Value::Instant(a), Value::Duration(b)) | (Value::Duration(b), Value::Instant(a)) => {
            checked(a.checked_add(*b).map(Value::Instant), op, "instant")
        }
        _ => float_operation(op, left, right),
    }
}

/// Difference of two values, `left - right`.
///
/// # Errors
///
/// * `E_TYPE` when the operands cannot be subtracted;
/// * `E_INVALID_VALUE` with the problem `overflow` when the result does not
///   fit its type, including a negative size.
pub fn sub(left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    let op = Op::Sub;
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => checked(a.checked_sub(*b).map(Value::Int), op, "int"),
        (Value::Size(a), Value::Size(b)) => checked(a.checked_sub(*b).map(Value::Size), op, "size"),
        (Value::Duration(a), Value::Duration(b)) => {
            checked(a.checked_sub(*b).map(Value::Duration), op, "duration")
        }
        (Value::Instant(a), Value::Duration(b)) => {
            checked(a.checked_sub(*b).map(Value::Instant), op, "instant")
        }
        (Value::Instant(a), Value::Instant(b)) => {
            checked(a.duration_since(*b).map(Value::Duration), op, "duration")
        }
        _ => float_operation(op, left, right),
    }
}

/// Product of two values, `left * right`.
///
/// # Errors
///
/// * `E_TYPE` when the operands cannot be multiplied;
/// * `E_INVALID_VALUE` with the problem `overflow` when the result does not
///   fit its type, including a negative size.
pub fn mul(left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    let op = Op::Mul;
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => checked(a.checked_mul(*b).map(Value::Int), op, "int"),
        (Value::Size(size), Value::Int(factor)) | (Value::Int(factor), Value::Size(size)) => {
            let product = u64::try_from(*factor)
                .ok()
                .and_then(|factor| size.bytes().checked_mul(factor));
            checked(
                product.map(|bytes| Value::Size(Size::from_bytes(bytes))),
                op,
                "size",
            )
        }
        (Value::Size(size), Value::Number(factor)) | (Value::Number(factor), Value::Size(size)) => {
            scale_size(*size, *factor, op)
        }
        (Value::Duration(duration), Value::Int(factor))
        | (Value::Int(factor), Value::Duration(duration)) => {
            let product = duration.as_millis().checked_mul(*factor);
            checked(
                product.map(|millis| Value::Duration(Duration::from_millis(millis))),
                op,
                "duration",
            )
        }
        (Value::Duration(duration), Value::Number(factor))
        | (Value::Number(factor), Value::Duration(duration)) => {
            scale_duration(*duration, *factor, op)
        }
        _ => float_operation(op, left, right),
    }
}

/// Quotient of two values, `left / right`.
///
/// # Errors
///
/// * `E_TYPE` when the operands cannot be divided;
/// * `E_INVALID_VALUE` with the problem `division_by_zero` for a zero
///   divisor, and with the problem `overflow` when the result does not fit
///   its type.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::Value;
/// use nurt_types::div;
///
/// assert_eq!(div(&Value::Int(6), &Value::Int(3))?, Value::Int(2));
/// assert_eq!(div(&Value::Int(7), &Value::Int(2))?, Value::Number(3.5));
/// assert!(div(&Value::Int(1), &Value::Int(0)).is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn div(left: &Value, right: &Value) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Null);
    }
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => divide_ints(*a, *b),
        (Value::Size(size), Value::Int(divisor)) => divide_size(*size, *divisor),
        (Value::Size(size), Value::Number(divisor)) => {
            nonzero(*divisor)?;
            scale_size(*size, 1.0 / divisor, Op::Div)
        }
        (Value::Size(a), Value::Size(b)) => ratio(a.bytes() as f64, b.bytes() as f64),
        (Value::Duration(duration), Value::Int(divisor)) => {
            if *divisor == 0 {
                return Err(division_by_zero());
            }
            let quotient = duration.as_millis().checked_div(*divisor);
            checked(
                quotient.map(|millis| Value::Duration(Duration::from_millis(millis))),
                Op::Div,
                "duration",
            )
        }
        (Value::Duration(duration), Value::Number(divisor)) => {
            nonzero(*divisor)?;
            scale_duration(*duration, 1.0 / divisor, Op::Div)
        }
        (Value::Duration(a), Value::Duration(b)) => {
            ratio(a.as_millis() as f64, b.as_millis() as f64)
        }
        _ => float_operation(Op::Div, left, right),
    }
}

/// Arithmetic negation, `-value`, for `int`, `number` and `duration`.
///
/// # Errors
///
/// * `E_TYPE` for another type;
/// * `E_INVALID_VALUE` with the problem `overflow` for the smallest `int`
///   or `duration`.
pub fn neg(value: &Value) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Int(int) => checked(int.checked_neg().map(Value::Int), Op::Sub, "int"),
        Value::Number(float) => Ok(Value::Number(-float)),
        Value::Duration(duration) => {
            let negated = duration.as_millis().checked_neg();
            checked(
                negated.map(|millis| Value::Duration(Duration::from_millis(millis))),
                Op::Sub,
                "duration",
            )
        }
        Value::BigInt(text) => Ok(Value::BigInt(negate_text(text))),
        Value::Decimal(text) => Ok(Value::Decimal(negate_text(text))),
        other => Err(type_error(
            "unary `-`",
            &["int", "number", "duration"],
            other,
        )),
    }
}

fn negate_text(text: &str) -> String {
    text.strip_prefix('-')
        .map_or_else(|| format!("-{text}"), str::to_owned)
}

fn checked(result: Option<Value>, op: Op, type_name: &str) -> Result<Value> {
    result.ok_or_else(|| overflow(op.symbol(), type_name))
}

fn divide_ints(dividend: i64, divisor: i64) -> Result<Value> {
    if divisor == 0 {
        return Err(division_by_zero());
    }
    match dividend.checked_rem(divisor) {
        Some(0) => checked(
            dividend.checked_div(divisor).map(Value::Int),
            Op::Div,
            "int",
        ),
        Some(_) => finite(dividend as f64 / divisor as f64, Op::Div),
        None => Err(overflow(Op::Div.symbol(), "int")),
    }
}

fn divide_size(size: Size, divisor: i64) -> Result<Value> {
    if divisor == 0 {
        return Err(division_by_zero());
    }
    let quotient = u64::try_from(divisor)
        .ok()
        .and_then(|divisor| size.bytes().checked_div(divisor));
    checked(
        quotient.map(|bytes| Value::Size(Size::from_bytes(bytes))),
        Op::Div,
        "size",
    )
}

fn ratio(dividend: f64, divisor: f64) -> Result<Value> {
    nonzero(divisor)?;
    finite(dividend / divisor, Op::Div)
}

fn nonzero(divisor: f64) -> Result<()> {
    if divisor.classify() == std::num::FpCategory::Zero {
        Err(division_by_zero())
    } else {
        Ok(())
    }
}

fn scale_size(size: Size, factor: f64, op: Op) -> Result<Value> {
    let bytes = (size.bytes() as f64 * factor).round();
    if bytes.is_nan() || bytes < 0.0 || bytes >= U64_LIMIT {
        return Err(overflow(op.symbol(), "size"));
    }
    // The value is a whole number in [0, 2^64), so the conversion is exact.
    Ok(Value::Size(Size::from_bytes(bytes as u64)))
}

fn scale_duration(duration: Duration, factor: f64, op: Op) -> Result<Value> {
    let millis = (duration.as_millis() as f64 * factor).round();
    if millis.is_nan() || millis < -I64_LIMIT || millis >= I64_LIMIT {
        return Err(overflow(op.symbol(), "duration"));
    }
    // The value is a whole number in [-2^63, 2^63), so the conversion is exact.
    Ok(Value::Duration(Duration::from_millis(millis as i64)))
}

fn finite(result: f64, op: Op) -> Result<Value> {
    if result.is_finite() {
        Ok(Value::Number(result))
    } else {
        Err(overflow(op.symbol(), "number"))
    }
}

/// The number of an `int` or `number`, including integers and decimals kept
/// as source text.
fn as_float(value: &Value) -> Option<f64> {
    match value {
        Value::Int(int) => Some(*int as f64),
        Value::Number(float) => Some(*float),
        Value::BigInt(text) | Value::Decimal(text) => text.parse().ok(),
        _ => None,
    }
}

fn float_operation(op: Op, left: &Value, right: &Value) -> Result<Value> {
    match (as_float(left), as_float(right)) {
        (Some(dividend), Some(divisor)) if op == Op::Div => ratio(dividend, divisor),
        (Some(a), Some(b)) => finite(op.apply(a, b), op),
        _ => Err(operand_error(op, left, right)),
    }
}

fn operand_error(op: Op, left: &Value, right: &Value) -> nurt_contracts::error::Error {
    let accepted = op.right_types(left.type_name());
    if accepted.is_empty() {
        type_error(op.symbol(), op.left_types(), left)
    } else {
        type_error(
            &format!("{} with {}", op.symbol(), left.type_name()),
            accepted,
            right,
        )
    }
}
