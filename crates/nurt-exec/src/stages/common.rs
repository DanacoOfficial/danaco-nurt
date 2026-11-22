// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Argument access, per-record keys, stream adapters and errors shared by
//! the record stages.

use std::fmt;

use nurt_contracts::bound::BoundExpr;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, ExecCtx, InputKinds, RecordStream, RenderHint, StageData, StageDescriptor,
    StageInput, StageOutput, StreamOrder,
};
use nurt_contracts::value::{Record, Value};
use serde_json::json;

/// Largest number of items in the `sample` of an error.
pub(crate) const SAMPLE_SIZE: usize = 5;

/// Field that names the file or target of a record.
pub(crate) const PATH_FIELD: &str = "path";

/// Field that holds the line or row number of a record.
pub(crate) const NUMBER_FIELD: &str = "no";

/// Separator of the segments of a field path written as text (`a.b`).
const PATH_SEPARATOR: char = '.';

/// Descriptor of a record stage that is not a source.
pub(crate) fn descriptor(command: &str, input: InputKinds) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source: false,
        input,
    }
}

/// `E_MISSING_PARAM` for a parameter without a value.
pub(crate) fn missing_param(args: &BoundArgs, name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        format!("`{}` needs a value for `{name}`", args.command()),
    )
    .with_expected(json!([name]))
    .with_help(args.command().to_owned())
}

/// `E_TYPE` for an argument of another type than the accepted ones.
pub(crate) fn type_error(args: &BoundArgs, name: &str, expected: &[&str], found: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`{}` expects {} for `{name}`, found {found}",
            args.command(),
            expected.join(" or ")
        ),
    )
    .with_expected(json!(expected))
    .with_found(found.to_owned())
}

/// `E_INVALID_VALUE` for an argument of the right type outside its range.
pub(crate) fn invalid_value(message: impl Into<String>, found: impl fmt::Display) -> Error {
    Error::new(ErrorCode::InvalidValue, message).with_found(found.to_string())
}

/// `E_LIMIT` for a stage that would hold more records than allowed.
pub(crate) fn limit_error(command: &str, max: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!("`{command}` would hold more than {max} records; narrow the input first"),
    )
    .with_detail("max_records", json!(max))
}

/// `E_UNKNOWN_FIELD` for a field the record does not have, with the fields
/// of the record in `expected`.
pub(crate) fn unknown_field(record: &Record, name: &str) -> Error {
    let fields: Vec<&str> = record.field_names().collect();
    Error::new(
        ErrorCode::UnknownField,
        format!("`{}` records have no field `{name}`", record.kind),
    )
    .with_found(name.to_owned())
    .with_expected(json!(fields))
}

/// `E_TYPE` for a record of a kind the stage does not read.
pub(crate) fn kind_error(command: &str, record: &Record, accepted: &[&str]) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`{command}` reads {} records, found `{}` records",
            accepted.join(" or "),
            record.kind
        ),
    )
    .with_expected(json!(accepted))
    .with_found(record.kind.as_str().to_owned())
}

/// Whether the parameter has a value: an expression, a pipeline or a value
/// other than `null`.
pub(crate) fn is_present(args: &BoundArgs, name: &str) -> bool {
    match args.get(name) {
        Some(ArgValue::Value(value)) => !value.is_null(),
        Some(ArgValue::Expr(_) | ArgValue::Pipeline(_)) => true,
        _ => false,
    }
}

/// Fails with `E_MISSING_PARAM` unless the parameter has a value.
pub(crate) fn require_present(args: &BoundArgs, name: &str) -> Result<()> {
    if is_present(args, name) {
        Ok(())
    } else {
        Err(missing_param(args, name))
    }
}

/// Value of a parameter known before the stage runs; `None` for `null`.
pub(crate) fn known_value<'a>(args: &'a BoundArgs, name: &str) -> Option<&'a Value> {
    args.value(name).filter(|value| !value.is_null())
}

/// A record count that must be present and not negative.
pub(crate) fn count_param(args: &BoundArgs, name: &str) -> Result<u64> {
    match known_value(args, name) {
        None => Err(missing_param(args, name)),
        Some(Value::Int(count)) => u64::try_from(*count).map_err(|_| {
            invalid_value(
                format!(
                    "`{}` needs a count of 0 or more for `{name}`, found {count}",
                    args.command()
                ),
                count,
            )
        }),
        Some(other) => Err(type_error(args, name, &["int"], other.type_name())),
    }
}

/// A flag; `false` when it has no value.
pub(crate) fn flag(args: &BoundArgs, name: &str) -> Result<bool> {
    match known_value(args, name) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(type_error(args, name, &["bool"], other.type_name())),
    }
}

/// Truth value of a condition: `null` counts as `false`.
pub(crate) fn truth(args: &BoundArgs, name: &str, value: &Value) -> Result<bool> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::Null => Ok(false),
        other => Err(type_error(args, name, &["bool"], other.type_name())),
    }
}

/// Length of a collection as `u64`.
pub(crate) fn len_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Value computed for each record: a field read by its path or an
/// expression evaluated by the evaluator of the context.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Key {
    /// Field path, the field of the record first.
    Field(Vec<String>),
    /// Expression.
    Expr(BoundExpr),
}

impl Key {
    /// Field path written with `.`, when the key is a field.
    pub(crate) fn field_name(&self) -> Option<String> {
        match self {
            Self::Field(path) => Some(path.join(".")),
            Self::Expr(_) => None,
        }
    }

    /// Value of the key for `record`.
    pub(crate) fn eval(&self, record: &Record, ctx: &ExecCtx) -> Result<Value> {
        match self {
            Self::Field(path) => field_value(record, path),
            Self::Expr(expr) => ctx.evaluator()?.eval(expr, Some(record), ctx),
        }
    }
}

/// Value at a field path of a record.
///
/// The first segment must be a field of the record (`E_UNKNOWN_FIELD`
/// otherwise); later segments read into the value as the expression `a.b`
/// does.
pub(crate) fn field_value(record: &Record, path: &[String]) -> Result<Value> {
    let Some((first, rest)) = path.split_first() else {
        return Ok(Value::Null);
    };
    let value = record
        .get(first)
        .cloned()
        .ok_or_else(|| unknown_field(record, first))?;
    rest.iter().try_fold(value, |current, segment| {
        nurt_types::get_field(&current, segment)
    })
}

/// Splits a field path written as text.
pub(crate) fn split_path(text: &str) -> Vec<String> {
    text.split(PATH_SEPARATOR).map(str::to_owned).collect()
}

/// Field path of an expression that only reads a field, also through other
/// fields (`a.b`); names written as text, keys or enumeration values count
/// as field paths.
pub(crate) fn field_path(expr: &BoundExpr) -> Option<Vec<String>> {
    match expr {
        BoundExpr::Field { name, of: None } => Some(vec![name.clone()]),
        BoundExpr::Field {
            name,
            of: Some(inner),
        } => {
            let mut path = field_path(inner)?;
            path.push(name.clone());
            Some(path)
        }
        BoundExpr::Text { v } | BoundExpr::Key { v } | BoundExpr::Enum { v } => Some(split_path(v)),
        _ => None,
    }
}

/// Keys of a parameter of type `field`, `list<field>` or `expression`; empty
/// when the parameter has no value.
///
/// A list expression gives one key per item; a value known before the stage
/// runs must be a field name or a list of field names.
pub(crate) fn keys(args: &BoundArgs, name: &str) -> Result<Vec<Key>> {
    match args.get(name) {
        Some(ArgValue::Expr(BoundExpr::List { items })) => Ok(items.iter().map(key_of).collect()),
        Some(ArgValue::Expr(expr)) => Ok(vec![key_of(expr)]),
        Some(ArgValue::Value(value)) => keys_of_value(args, name, value),
        Some(ArgValue::Pipeline(_)) => Err(type_error(args, name, &["field"], "pipeline")),
        _ => Ok(Vec::new()),
    }
}

fn key_of(expr: &BoundExpr) -> Key {
    field_path(expr).map_or_else(|| Key::Expr(expr.clone()), Key::Field)
}

fn keys_of_value(args: &BoundArgs, name: &str, value: &Value) -> Result<Vec<Key>> {
    let field = |item: &Value| {
        item.as_str()
            .map(|text| Key::Field(split_path(text)))
            .ok_or_else(|| type_error(args, name, &["field"], item.type_name()))
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::List(items) => items.iter().map(field).collect(),
        other => field(other).map(|key| vec![key]),
    }
}

/// Values of several keys for one record: the single value for one key, a
/// list for more.
pub(crate) fn key_value(keys: &[Key], record: &Record, ctx: &ExecCtx) -> Result<Value> {
    match keys {
        [key] => key.eval(record, ctx),
        _ => keys
            .iter()
            .map(|key| key.eval(record, ctx))
            .collect::<Result<Vec<_>>>()
            .map(Value::List),
    }
}

/// Short text that identifies a record in the `sample` of an error: the
/// path and number when the record has them, the short form of the record
/// otherwise.
pub(crate) fn sample_item(record: &Record) -> String {
    let path = record.get(PATH_FIELD).and_then(Value::as_str);
    let number = record.get(NUMBER_FIELD).and_then(Value::as_int);
    match (path, number) {
        (Some(path), Some(number)) => format!("{path}:{number}"),
        (Some(path), None) => path.to_owned(),
        _ => nurt_types::display_short(&Value::Record(record.clone())),
    }
}

/// The input of a record stage split into the record stream and the
/// description of the stream.
pub(crate) struct Parts<'a> {
    /// Input records, checked against cancellation on every pull.
    pub(crate) records: RecordStream<'a>,
    /// Header record of the stream.
    pub(crate) header: Option<Record>,
    /// Order of the stream known to its producer.
    pub(crate) order: Option<StreamOrder>,
    /// Presentation hint of the producer.
    pub(crate) render: Option<RenderHint>,
}

impl<'a> Parts<'a> {
    /// Splits the input; fails with `E_TYPE` when it is not a record stream.
    pub(crate) fn of(input: StageInput<'a>, ctx: &'a ExecCtx) -> Result<Self> {
        let StageInput {
            data,
            header,
            order,
            render,
        } = input;
        let records = StageInput {
            data,
            header: None,
            order: None,
            render: None,
        }
        .into_records()?;
        Ok(Self {
            records: records.checked(ctx),
            header,
            order,
            render,
        })
    }

    /// Reads every record, within the record limit of the context.
    pub(crate) fn collect(self, ctx: &ExecCtx) -> Result<(Vec<Record>, Description)> {
        let description = Description {
            header: self.header,
            order: self.order,
            render: self.render,
        };
        let records = self.records.collect_bounded(ctx.limits().max_records)?;
        Ok((records, description))
    }
}

/// Header, order and presentation hint of a stream.
#[derive(Debug, Clone, Default)]
pub(crate) struct Description {
    /// Header record.
    pub(crate) header: Option<Record>,
    /// Order of the stream.
    pub(crate) order: Option<StreamOrder>,
    /// Presentation hint.
    pub(crate) render: Option<RenderHint>,
}

impl Description {
    /// Output of records held in memory with this description.
    pub(crate) fn output<'a>(self, records: Vec<Record>) -> StageOutput<'a> {
        self.stream(RecordStream::from_records(records))
    }

    /// Output of a stream with this description.
    pub(crate) fn stream(self, records: RecordStream<'_>) -> StageOutput<'_> {
        StageOutput {
            data: StageData::Records(records),
            header: self.header,
            order: self.order,
            render: self.render,
        }
    }
}

/// Wraps an iterator so that the stream ends after its first error.
pub(crate) fn fused<'a, I>(iter: I) -> RecordStream<'a>
where
    I: Iterator<Item = Result<Record>> + Send + 'a,
{
    RecordStream::new(Fused {
        inner: iter,
        done: false,
    })
}

struct Fused<I> {
    inner: I,
    done: bool,
}

impl<I: Iterator<Item = Result<Record>>> Iterator for Fused<I> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let item = self.inner.next();
        if !matches!(item, Some(Ok(_))) {
            self.done = true;
        }
        item
    }
}
