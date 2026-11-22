// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Evaluator of bound expressions.

use std::cmp::Ordering;
use std::iter;

use jiff::tz::TimeZone;
use nurt_contracts::ast::{BinOp, UnOp};
use nurt_contracts::bound::{BoundExpr, BoundMapEntry, BoundRef};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Duration, Fields, Range, Record, Regex, Size, Value};
use serde_json::json;

use crate::access::{get_field, get_path};
use crate::arith::{add, div, mul, neg, sub};
use crate::builtins::{self, Args, CallCtx};
use crate::coerce::{ValueType, coerce_in, converts_implicitly, text_is_not_regex};
use crate::compare::{comparable, compare, equal, family_types};
use crate::env::EvalEnv;
use crate::errors::{TEXT_TYPES, invalid_value, overflow, type_error, unknown_field, unsupported};
use crate::pattern::RegexCache;
use crate::units::parse_instant;

/// Evaluator of bound expressions: `ƒ` parameters, `filter`, `select`,
/// `sort` and the arguments computed per record.
///
/// The operator rules are listed in the crate documentation, section
/// "Expressions". The evaluator keeps a cache of compiled regular
/// expressions; one evaluator serves any number of threads and evaluations.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::ast::BinOp;
/// use nurt_contracts::bound::BoundExpr;
/// use nurt_contracts::value::{Instant, Record, RecordKind, Value};
/// use nurt_types::{Evaluator, FixedEnv};
///
/// let record = Record::new(RecordKind::Entry).with("name", Value::from("main.rs"));
/// let expr = BoundExpr::Bin {
///     op: BinOp::Match,
///     l: Box::new(BoundExpr::Field { name: "name".to_owned(), of: None }),
///     r: Box::new(BoundExpr::Regex { v: r"\.rs$".to_owned(), flags: String::new() }),
/// };
/// let env = FixedEnv::new(Instant::UNIX_EPOCH, 0);
/// assert!(Evaluator::new().eval_bool(&expr, Some(&record), &env)?);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Default)]
pub struct Evaluator {
    regexes: RegexCache,
}

impl Evaluator {
    /// Evaluator with an empty cache of regular expressions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Value of `expr` for `record`; `record` is `None` outside a record
    /// stream.
    ///
    /// # Errors
    ///
    /// * `E_UNKNOWN_FIELD` for a field the record does not have, with its
    ///   fields in `expected`, and for a field read without a record;
    /// * `E_UNKNOWN_NAME` for an unknown named result or function;
    /// * `E_TYPE` for operands and arguments of types the operator or the
    ///   function does not accept, with the accepted types in `expected`;
    /// * `E_ARITY` for a wrong number of arguments;
    /// * `E_REGEX` for a regular expression that does not compile;
    /// * `E_INVALID_VALUE` for a text that does not form a value of its
    ///   place, for arguments outside their range, and with the problems
    ///   `overflow` and `division_by_zero` for arithmetic;
    /// * `E_INTERNAL` for an expression node this version does not know.
    pub fn eval(
        &self,
        expr: &BoundExpr,
        record: Option<&Record>,
        env: &dyn EvalEnv,
    ) -> Result<Value> {
        Scope {
            regexes: &self.regexes,
            record,
            env,
        }
        .eval(expr)
    }

    /// Truth value of a condition: `true` only for `true`; `null` is
    /// `false`.
    ///
    /// # Errors
    ///
    /// The errors of [`Evaluator::eval`], and `E_TYPE` when the value is
    /// neither a `bool` nor `null`.
    pub fn eval_bool(
        &self,
        expr: &BoundExpr,
        record: Option<&Record>,
        env: &dyn EvalEnv,
    ) -> Result<bool> {
        truth("the condition", self.eval(expr, record, env)?)
    }
}

/// One evaluation: the record, the environment and the shared cache.
struct Scope<'a> {
    regexes: &'a RegexCache,
    record: Option<&'a Record>,
    env: &'a dyn EvalEnv,
}

impl Scope<'_> {
    fn eval(&self, expr: &BoundExpr) -> Result<Value> {
        match expr {
            BoundExpr::List { items } => self.list(items).map(Value::List),
            BoundExpr::Map { entries } => self.map(entries),
            BoundExpr::Ref(reference) => self.reference(reference),
            BoundExpr::Field { name, of } => self.field(name, of.as_deref()),
            BoundExpr::Call { function, args } => self.call(function, args),
            BoundExpr::Un { op, x } => self.unary(*op, x),
            BoundExpr::Bin { op, l, r } => self.binary(*op, l, r),
            BoundExpr::Spread { .. } => Err(Error::new(
                ErrorCode::Type,
                "a spread `...` is allowed only in a list or in the arguments of a call",
            )),
            BoundExpr::Sub { .. } => Err(Error::new(
                ErrorCode::Type,
                "a pipeline is not a value of an expression",
            )
            .with_actual(json!("pipeline"))),
            literal => literal_value(literal, &self.env.tz()),
        }
    }

    fn list(&self, items: &[BoundExpr]) -> Result<Vec<Value>> {
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            match item {
                BoundExpr::Spread { x } => values.extend(self.spread(x)?),
                other => values.push(self.eval(other)?),
            }
        }
        Ok(values)
    }

    fn spread(&self, expr: &BoundExpr) -> Result<Vec<Value>> {
        match self.eval(expr)? {
            Value::List(items) => Ok(items),
            Value::Null => Ok(Vec::new()),
            other => Err(type_error("a spread `...`", &["list"], &other)),
        }
    }

    fn map(&self, entries: &[BoundMapEntry]) -> Result<Value> {
        entries
            .iter()
            .map(|entry| Ok((entry.k.clone(), self.eval(&entry.v)?)))
            .collect::<Result<Fields>>()
            .map(Value::Map)
    }

    fn reference(&self, reference: &BoundRef) -> Result<Value> {
        let value = self.env.named(&reference.name).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownName,
                format!("unknown name `@{}`", reference.name),
            )
            .with_found(format!("@{}", reference.name))
        })?;
        get_path(value, &reference.fields)
    }

    fn field(&self, name: &str, of: Option<&BoundExpr>) -> Result<Value> {
        if let Some(base) = of {
            return get_field(&self.eval(base)?, name);
        }
        let record = self.record.ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownField,
                format!("field `{name}` is read outside a record stream"),
            )
            .with_found(name)
        })?;
        match record.get(name) {
            Some(value) => Ok(value.clone()),
            None if self.env.is_known_field(record.kind, name) => Ok(Value::Null),
            None => Err(unknown_field(
                name,
                Some(record.kind.as_str()),
                record.field_names(),
            )),
        }
    }

    fn call(&self, function: &str, args: &[BoundExpr]) -> Result<Value> {
        let mut values = Vec::with_capacity(args.len());
        let mut literal_text = Vec::with_capacity(args.len());
        for arg in args {
            if let BoundExpr::Spread { x } = arg {
                let items = self.spread(x)?;
                literal_text.extend(iter::repeat_n(false, items.len()));
                values.extend(items);
            } else {
                literal_text.push(matches!(arg, BoundExpr::Text { .. }));
                values.push(self.eval(arg)?);
            }
        }
        let ctx = CallCtx {
            env: self.env,
            regexes: self.regexes,
        };
        builtins::call(function, &Args::new(values, literal_text), &ctx)
    }

    fn unary(&self, op: UnOp, operand: &BoundExpr) -> Result<Value> {
        let value = self.eval(operand)?;
        match op {
            UnOp::Not => truth("`not`", value).map(|truth| Value::Bool(!truth)),
            UnOp::Neg => neg(&value),
            _ => Err(unsupported("the unary operator")),
        }
    }

    fn binary(&self, op: BinOp, left: &BoundExpr, right: &BoundExpr) -> Result<Value> {
        match op {
            BinOp::And => self.logical("`and`", left, right, false),
            BinOp::Or => self.logical("`or`", left, right, true),
            _ => {
                let (left, right) = self.operands(op, left, right)?;
                apply(op, &left, &right, self.regexes)
            }
        }
    }

    /// `and` stops at a left `false`, `or` at a left `true`: `decisive` is
    /// the left value that decides the result alone.
    fn logical(
        &self,
        operator: &str,
        left: &BoundExpr,
        right: &BoundExpr,
        decisive: bool,
    ) -> Result<Value> {
        if truth(operator, self.eval(left)?)? == decisive {
            return Ok(Value::Bool(decisive));
        }
        truth(operator, self.eval(right)?).map(Value::Bool)
    }

    /// Evaluates both operands; in comparisons, converts a text literal to
    /// the type of the other operand.
    fn operands(&self, op: BinOp, left: &BoundExpr, right: &BoundExpr) -> Result<(Value, Value)> {
        let left_value = self.eval(left)?;
        let right_value = self.eval(right)?;
        if !is_comparison(op) {
            return Ok((left_value, right_value));
        }
        let tz = self.env.tz();
        let left_value = if is_text_literal(left) {
            convert_literal(left_value, &right_value, &tz)?
        } else {
            left_value
        };
        let right_value = if is_text_literal(right) {
            convert_literal(right_value, &left_value, &tz)?
        } else {
            right_value
        };
        Ok((left_value, right_value))
    }
}

fn literal_value(expr: &BoundExpr, tz: &TimeZone) -> Result<Value> {
    Ok(match expr {
        BoundExpr::Null => Value::Null,
        BoundExpr::Bool { v } => Value::Bool(*v),
        BoundExpr::Int { v } => Value::Int(*v),
        BoundExpr::Num { v } => Value::Number(*v),
        BoundExpr::Text { v } => Value::Text(v.clone()),
        BoundExpr::Path { v } => Value::Path(v.clone()),
        BoundExpr::Glob { v } => Value::Glob(v.clone()),
        BoundExpr::Key { v } => Value::Key(v.clone()),
        BoundExpr::Enum { v } => Value::Enum(v.clone()),
        BoundExpr::Instant { v } => {
            Value::Instant(parse_instant(v, tz).map_err(|error| invalid_value(&error))?)
        }
        BoundExpr::Size { v } => Value::Size(Size::from_bytes(*v)),
        BoundExpr::Dur { v } => Value::Duration(Duration::from_millis(
            i64::try_from(*v).map_err(|_| overflow("the duration literal", "duration"))?,
        )),
        BoundExpr::Regex { v, flags } => {
            Value::Regex(Regex::new(v, flags).map_err(|error| {
                Error::new(ErrorCode::Regex, error.to_string()).with_help("regex")
            })?)
        }
        BoundExpr::Range { lo, hi } => {
            Value::Range(Range::new(*lo, *hi).map_err(|error| invalid_value(&error))?)
        }
        _ => return Err(unsupported("the expression node")),
    })
}

fn truth(operator: &str, value: Value) -> Result<bool> {
    match value {
        Value::Bool(truth) => Ok(truth),
        Value::Null => Ok(false),
        other => Err(type_error(operator, &["bool"], &other)),
    }
}

const fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

const fn is_text_literal(expr: &BoundExpr) -> bool {
    matches!(expr, BoundExpr::Text { .. })
}

fn convert_literal(literal: Value, other: &Value, tz: &TimeZone) -> Result<Value> {
    match ValueType::of(other) {
        Some(target) if converts_implicitly(ValueType::Text, target) => {
            coerce_in(literal, target, tz)
        }
        _ => Ok(literal),
    }
}

fn apply(op: BinOp, left: &Value, right: &Value, regexes: &RegexCache) -> Result<Value> {
    match op {
        BinOp::Eq => Ok(Value::Bool(equal(left, right))),
        BinOp::Ne => Ok(Value::Bool(!equal(left, right))),
        BinOp::Lt => order("`<`", left, right, Ordering::is_lt),
        BinOp::Le => order("`<=`", left, right, Ordering::is_le),
        BinOp::Gt => order("`>`", left, right, Ordering::is_gt),
        BinOp::Ge => order("`>=`", left, right, Ordering::is_ge),
        BinOp::Match => matches(left, right, regexes).map(Value::Bool),
        BinOp::NotMatch => matches(left, right, regexes).map(|found| Value::Bool(!found)),
        BinOp::In => membership(left, right).map(Value::Bool),
        BinOp::Add => add(left, right),
        BinOp::Sub => sub(left, right),
        BinOp::Mul => mul(left, right),
        BinOp::Div => div(left, right),
        _ => Err(unsupported("the binary operator")),
    }
}

fn order(operator: &str, left: &Value, right: &Value, test: fn(Ordering) -> bool) -> Result<Value> {
    if left.is_null() || right.is_null() {
        return Ok(Value::Bool(false));
    }
    if !comparable(left, right) {
        return Err(type_error(
            &format!("{operator} with {}", left.type_name()),
            &family_types(left),
            right,
        ));
    }
    Ok(Value::Bool(test(compare(left, right))))
}

fn matches(left: &Value, right: &Value, regexes: &RegexCache) -> Result<bool> {
    let regex = match right {
        Value::Regex(regex) => regex,
        Value::Text(_) => return Err(text_is_not_regex()),
        other => return Err(type_error("the right operand of `~`", &["regex"], other)),
    };
    if left.is_null() {
        return Ok(false);
    }
    let text = left
        .as_str()
        .ok_or_else(|| type_error("the left operand of `~`", TEXT_TYPES, left))?;
    Ok(regexes.compile(regex)?.is_match(text))
}

fn membership(left: &Value, right: &Value) -> Result<bool> {
    match right {
        Value::Null => Ok(false),
        Value::List(items) => Ok(items.iter().any(|item| equal(left, item))),
        Value::Map(map) => Ok(left.as_str().is_some_and(|key| map.contains_key(key))),
        Value::Range(range) => match left {
            Value::Null => Ok(false),
            Value::Int(int) => Ok(range.contains(*int)),
            Value::BigInt(text) if text.starts_with('-') => Ok(range.start().is_none()),
            Value::BigInt(_) => Ok(range.end().is_none()),
            other => Err(type_error("`in` with a range", &["int"], other)),
        },
        other => Err(type_error(
            "the right operand of `in`",
            &["list", "range", "map"],
            other,
        )),
    }
}
