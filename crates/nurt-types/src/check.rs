// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of expressions that no record can avoid.
//!
//! When the type of a field of the input records is known and the other
//! operand is a literal, the evaluator raises the same error for every
//! record that has a value in the field. These functions find that error
//! without a record, by running the operator or the function of the
//! evaluator on a sample value of the type, so the binder can report it
//! before the program runs and with the same code, message and `expected`.

use jiff::tz::TimeZone;
use nurt_contracts::ast::BinOp;
use nurt_contracts::bound::{BoundExpr, BoundMapEntry};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::{Bytes, Duration, Fields, Instant, Range, Size, Value};

use crate::builtins::{self, Args, CallCtx};
use crate::coerce::{ValueType, coerce_in, converts_implicitly};
use crate::env::FixedEnv;
use crate::eval::{apply, is_comparison, is_text_literal, literal_value};
use crate::pattern::RegexCache;

/// An operand whose value is known without a record.
#[derive(Debug, Clone, Copy)]
pub enum Operand<'a> {
    /// A field of the input records that holds values of the type.
    Field(ValueType),
    /// A literal of a bound tree: a scalar, or a list or a map of literals.
    Literal(&'a BoundExpr),
}

/// Value of an operand: a sample of the type of a field, or the literal.
fn value_of(operand: Operand<'_>, tz: &TimeZone) -> Option<Value> {
    match operand {
        Operand::Field(ty) => sample(ty),
        Operand::Literal(expr) => literal(expr, tz),
    }
}

fn literal(expr: &BoundExpr, tz: &TimeZone) -> Option<Value> {
    match expr {
        BoundExpr::List { items } => items
            .iter()
            .map(|item| literal(item, tz))
            .collect::<Option<Vec<_>>>()
            .map(Value::List),
        BoundExpr::Map { entries } => entries
            .iter()
            .map(|BoundMapEntry { k, v }| literal(v, tz).map(|v| (k.clone(), v)))
            .collect::<Option<Fields>>()
            .map(Value::Map),
        other => literal_value(other, tz).ok(),
    }
}

/// A value of the type that no operator rejects for its value, only for its
/// type family: zero would turn a division into an error of the sample.
fn sample(ty: ValueType) -> Option<Value> {
    Some(match ty {
        ValueType::Bool => Value::Bool(false),
        ValueType::Int => Value::Int(1),
        ValueType::Number => Value::Number(1.0),
        ValueType::Text => Value::Text("a".to_owned()),
        ValueType::Enum => Value::Enum("a".to_owned()),
        ValueType::Size => Value::Size(Size::from_bytes(1)),
        ValueType::Duration => Value::Duration(Duration::from_millis(1)),
        ValueType::Instant => Value::Instant(Instant::UNIX_EPOCH),
        ValueType::Path => Value::Path("/a".to_owned()),
        ValueType::Glob => Value::Glob("*".to_owned()),
        ValueType::Key => Value::Key("a".to_owned()),
        ValueType::Bytes => Value::Bytes(Bytes::new(vec![1])),
        ValueType::Range => Value::Range(Range::single(1)),
        ValueType::List => Value::List(Vec::new()),
        ValueType::Map => Value::Map(Fields::new()),
        _ => return None,
    })
}

/// Error that evaluating `left op right` raises for every record, found
/// without a record; at least one operand must be a field.
///
/// * A text literal compared with a field of the type `size`, `duration`,
///   `instant`, `path`, `glob` or `key` must form a value of that type
///   (`E_INVALID_VALUE`, as the evaluator reports it).
/// * Operands of types that the operator does not accept are `E_TYPE`: an
///   ordering operator on two type families, `in` with a range and a field
///   that is not an `int`, arithmetic on types without a result, `~` on a
///   field that is not text.
///
/// Errors that depend on the value of the field (an overflow, a division by
/// zero) and operators that do not evaluate their operands (`and`, `or`) are
/// not reported. `None` when the comparison can succeed.
///
/// # Examples
///
/// ```rust
/// use jiff::tz::TimeZone;
/// use nurt_contracts::ast::BinOp;
/// use nurt_contracts::bound::BoundExpr;
/// use nurt_types::{Operand, ValueType, operation_error};
///
/// let text = BoundExpr::Text { v: "abc".to_owned() };
/// let field = Operand::Field(ValueType::Size);
/// let tz = TimeZone::UTC;
/// assert!(operation_error(BinOp::Gt, field, Operand::Literal(&text), &tz).is_some());
/// assert!(operation_error(BinOp::Add, field, Operand::Literal(&text), &tz).is_some());
/// let size = BoundExpr::Size { v: 5 };
/// assert!(operation_error(BinOp::Add, field, Operand::Literal(&size), &tz).is_none());
/// ```
#[must_use]
pub fn operation_error(
    op: BinOp,
    left: Operand<'_>,
    right: Operand<'_>,
    tz: &TimeZone,
) -> Option<Error> {
    if !matches!(left, Operand::Field(_)) && !matches!(right, Operand::Field(_)) {
        return None;
    }
    let mut values = [value_of(left, tz)?, value_of(right, tz)?];
    let sides = [left, right];
    for at in 0..2 {
        let (Operand::Literal(expr), Operand::Field(ty)) = (sides[at], sides[1 - at]) else {
            continue;
        };
        if is_comparison(op) && is_text_literal(expr) && converts_implicitly(ValueType::Text, ty) {
            match coerce_in(values[at].clone(), ty, tz) {
                Ok(converted) => values[at] = converted,
                Err(error) => return Some(error),
            }
        }
    }
    let [left, right] = &values;
    apply(op, left, right, &RegexCache::default())
        .err()
        .filter(|error| error.code == ErrorCode::Type)
}

/// Error that calling the built-in function `function` raises for every
/// record, found without a record; `None` when an argument is not known.
///
/// A call without a field argument is evaluated as written, so any error of
/// the evaluator is reported (a text that is not a duration in `ago("abc")`,
/// a wrong number of arguments). With a field argument only `E_TYPE` and
/// `E_ARITY` are reported, because other errors depend on the value of the
/// field.
///
/// # Examples
///
/// ```rust
/// use jiff::tz::TimeZone;
/// use nurt_contracts::bound::BoundExpr;
/// use nurt_types::{Operand, ValueType, function_error};
///
/// let tz = TimeZone::UTC;
/// let field = [Operand::Field(ValueType::Size)];
/// assert!(function_error("lower", &field, &tz).is_some());
/// let text = BoundExpr::Text { v: "abc".to_owned() };
/// assert!(function_error("ago", &[Operand::Literal(&text)], &tz).is_some());
/// ```
#[must_use]
pub fn function_error(function: &str, args: &[Operand<'_>], tz: &TimeZone) -> Option<Error> {
    let mut values = Vec::with_capacity(args.len());
    let mut literal_text = Vec::with_capacity(args.len());
    for arg in args {
        values.push(value_of(*arg, tz)?);
        literal_text.push(matches!(arg, Operand::Literal(BoundExpr::Text { .. })));
    }
    let with_field = args.iter().any(|arg| matches!(arg, Operand::Field(_)));
    let env = FixedEnv::new(Instant::UNIX_EPOCH, 0).with_tz(tz.clone());
    let regexes = RegexCache::default();
    let ctx = CallCtx {
        env: &env,
        regexes: &regexes,
    };
    builtins::call(function, &Args::new(values, literal_text), &ctx)
        .err()
        .filter(|error| !with_field || matches!(error.code, ErrorCode::Type | ErrorCode::Arity))
        .filter(|error| error.code != ErrorCode::UnknownName)
}
