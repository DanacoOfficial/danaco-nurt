// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The stages `filter` and `select`.

use nurt_contracts::bound::BoundExpr;
use nurt_contracts::error::Result;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind, Value};

use super::common::{
    Description, Parts, descriptor, field_path, field_value, fused, missing_param, require_present,
    split_path, truth, type_error,
};

const FILTER: &str = "filter";
const CONDITION: &str = "condition";

/// `filter condition`: keeps the records for which the condition is `true`.
///
/// The condition is evaluated on each record by the expression evaluator of
/// the context; `null` counts as `false` and any other value than a `bool`
/// fails with `E_TYPE`. The stream stays lazy and keeps its header, order and
/// presentation hint, so `filter` works on file records before an effect and
/// on the records of an effect (`exec`, `job`, `test_result`, `finding`,
/// `effect`) after it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filter;

impl StageImpl for Filter {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(FILTER, InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        require_present(args, CONDITION)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Filter {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        require_present(args, CONDITION)?;
        input.map_records(|records| {
            fused(records.checked(ctx).filter_map(move |item| {
                item.and_then(|record| {
                    let value = args
                        .eval(CONDITION, Some(&record), ctx)?
                        .unwrap_or_default();
                    Ok(truth(args, CONDITION, &value)?.then_some(record))
                })
                .transpose()
            }))
        })
    }
}

const SELECT: &str = "select";
const FIELDS: &str = "fields";
const COLUMN_TYPES: &[&str] = &["field", "named_expression"];

/// One output field of `select`.
#[derive(Debug, Clone, PartialEq)]
enum Column {
    /// A field of the input record under the name of its last segment.
    Keep { path: Vec<String>, name: String },
    /// A field computed per record.
    Compute { name: String, expr: BoundExpr },
    /// A field with the same value in every record.
    Constant { name: String, value: Value },
}

impl Column {
    fn keep(path: Vec<String>) -> Self {
        let name = path.last().cloned().unwrap_or_default();
        Self::Keep { path, name }
    }

    /// Whether the column keeps a top-level field of the input record.
    fn keeps_field(&self) -> bool {
        matches!(self, Self::Keep { path, .. } if path.len() == 1)
    }

    fn value(&self, record: &Record, ctx: &ExecCtx) -> Result<(&str, Value)> {
        match self {
            Self::Keep { path, name } => Ok((name, field_value(record, path)?)),
            Self::Compute { name, expr } => {
                Ok((name, ctx.evaluator()?.eval(expr, Some(record), ctx)?))
            }
            Self::Constant { name, value } => Ok((name, value.clone())),
        }
    }
}

fn columns(args: &BoundArgs) -> Result<Vec<Column>> {
    let mut columns = Vec::new();
    match args.get(FIELDS) {
        Some(ArgValue::Expr(expr)) => columns_of_expr(args, expr, &mut columns)?,
        Some(ArgValue::Value(value)) => columns_of_value(args, value, &mut columns)?,
        Some(ArgValue::Pipeline(_)) => {
            return Err(type_error(args, FIELDS, COLUMN_TYPES, "pipeline"));
        }
        _ => {}
    }
    if columns.is_empty() {
        return Err(missing_param(args, FIELDS));
    }
    Ok(columns)
}

fn columns_of_expr(args: &BoundArgs, expr: &BoundExpr, columns: &mut Vec<Column>) -> Result<()> {
    match expr {
        BoundExpr::List { items } => {
            for item in items {
                columns_of_expr(args, item, columns)?;
            }
        }
        BoundExpr::Map { entries } => {
            columns.extend(entries.iter().map(|entry| Column::Compute {
                name: entry.k.clone(),
                expr: entry.v.clone(),
            }));
        }
        other => {
            let path = field_path(other)
                .ok_or_else(|| type_error(args, FIELDS, COLUMN_TYPES, "expression"))?;
            columns.push(Column::keep(path));
        }
    }
    Ok(())
}

fn columns_of_value(args: &BoundArgs, value: &Value, columns: &mut Vec<Column>) -> Result<()> {
    match value {
        Value::Null => {}
        Value::List(items) => {
            for item in items {
                columns_of_value(args, item, columns)?;
            }
        }
        Value::Map(entries) => {
            columns.extend(entries.iter().map(|(name, value)| Column::Constant {
                name: name.clone(),
                value: value.clone(),
            }));
        }
        other => {
            let text = other
                .as_str()
                .ok_or_else(|| type_error(args, FIELDS, COLUMN_TYPES, other.type_name()))?;
            columns.push(Column::keep(split_path(text)));
        }
    }
    Ok(())
}

fn project(
    columns: &[Column],
    kind: Option<RecordKind>,
    record: &Record,
    ctx: &ExecCtx,
) -> Result<Record> {
    let mut output = Record::new(kind.unwrap_or(record.kind));
    for column in columns {
        let (name, value) = column.value(record, ctx)?;
        output.set(name, value);
    }
    Ok(output)
}

/// `select fields`: keeps the named fields of each record, in the given
/// order, and adds computed fields written as `name: expression`.
///
/// Records that only keep top-level fields keep their kind; records with a
/// computed field, a constant field or a field read from inside another
/// field (`a.b`) become `map` records. A field the record does not have fails
/// with `E_UNKNOWN_FIELD` and the list of its fields. The header of the
/// stream is kept; the order and the presentation hint are not, because they
/// may name fields that are no longer present.
#[derive(Debug, Clone, Copy, Default)]
pub struct Select;

impl StageImpl for Select {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(SELECT, InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        columns(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Select {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let columns = columns(args)?;
        let kind = (!columns.iter().all(Column::keeps_field)).then_some(RecordKind::Map);
        let parts = Parts::of(input, ctx)?;
        let records = fused(
            parts
                .records
                .map(move |item| item.and_then(|record| project(&columns, kind, &record, ctx))),
        );
        let description = Description {
            header: parts.header,
            ..Description::default()
        };
        Ok(description.stream(records))
    }
}
