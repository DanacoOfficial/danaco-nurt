// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::cmp::Ordering;

use nurt_contracts::keypath::{CompareOp, FilterExpr, FilterLiteral};
use nurt_contracts::value::Value;

use crate::value::{compare_numbers, equivalent, numeric};

/// Returns `true` when `expr` holds for `element`.
///
/// A bare field tests for existence; a missing field compared with `==` is
/// equal only to another missing field, and ordering comparisons hold only
/// between two numbers or two texts.
pub(crate) fn matches(expr: &FilterExpr, element: &Value) -> bool {
    match expr {
        FilterExpr::Field { path } => field(element, path).is_some(),
        FilterExpr::Literal { value } => !matches!(value, FilterLiteral::Bool(false) | FilterLiteral::Null),
        FilterExpr::Compare { op, left, right } => {
            compare(*op, operand(left, element).as_ref(), operand(right, element).as_ref())
        }
        FilterExpr::And { left, right } => matches(left, element) && matches(right, element),
        FilterExpr::Or { left, right } => matches(left, element) || matches(right, element),
        FilterExpr::Not { operand } => !matches(operand, element),
        _ => false,
    }
}

fn field<'v>(element: &'v Value, path: &[String]) -> Option<&'v Value> {
    path.iter().try_fold(element, |value, name| value.get(name))
}

fn operand(expr: &FilterExpr, element: &Value) -> Option<Value> {
    match expr {
        FilterExpr::Field { path } => field(element, path).cloned(),
        FilterExpr::Literal { value } => Some(literal(value)),
        other => Some(Value::Bool(matches(other, element))),
    }
}

fn literal(value: &FilterLiteral) -> Value {
    match value {
        FilterLiteral::Text(text) => Value::Text(text.clone()),
        FilterLiteral::Int(number) => Value::Int(*number),
        FilterLiteral::Number(number) => Value::Number(*number),
        FilterLiteral::Bool(flag) => Value::Bool(*flag),
        _ => Value::Null,
    }
}

fn compare(op: CompareOp, left: Option<&Value>, right: Option<&Value>) -> bool {
    match op {
        CompareOp::Eq => same(left, right),
        CompareOp::Ne => !same(left, right),
        CompareOp::Lt => order(left, right) == Some(Ordering::Less),
        CompareOp::Le => matches!(order(left, right), Some(Ordering::Less | Ordering::Equal)),
        CompareOp::Gt => order(left, right) == Some(Ordering::Greater),
        CompareOp::Ge => matches!(order(left, right), Some(Ordering::Greater | Ordering::Equal)),
    }
}

fn same(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => equivalent(left, right),
        _ => false,
    }
}

fn order(left: Option<&Value>, right: Option<&Value>) -> Option<Ordering> {
    let (left, right) = (left?, right?);
    if numeric(left) && numeric(right) {
        return compare_numbers(left, right);
    }
    match (text_of(left), text_of(right)) {
        (Some(left), Some(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Date(date) => Some(&date.text),
        other => other.as_str(),
    }
}
