// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The stages `group`, `summarize` and `distinct`.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, OrderKey, ReadStage, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo, StreamOrder,
};
use nurt_contracts::value::{Duration, Record, RecordKind, Size, Value};
use nurt_types::OrderedValue;

use super::common::{
    Description, Key, Parts, descriptor, flag, key_value, keys, len_u64, limit_error, missing_param,
};

const BY: &str = "by";
const KEEP_ITEMS: &str = "keep_items";
const FIELDS: &str = "fields";
const KEY_FIELD: &str = "key";
const COUNT_FIELD: &str = "count";
const ITEMS_FIELD: &str = "items";

/// Order of the output of `group` and `summarize`: ascending by the group
/// key.
fn key_order(field: &str) -> StreamOrder {
    StreamOrder {
        keys: vec![OrderKey {
            field: field.to_owned(),
            descending: false,
        }],
    }
}

fn group_keys(args: &BoundArgs) -> Result<Vec<Key>> {
    let keys = keys(args, BY)?;
    if keys.is_empty() {
        return Err(missing_param(args, BY));
    }
    Ok(keys)
}

/// Records of one group of `group`.
#[derive(Debug, Default)]
struct Members {
    count: u64,
    items: Vec<Value>,
}

/// `group by keep_items`: groups the records by the value of a field or an
/// expression and counts each group.
///
/// Every group becomes a `group` record `{key, count}`, with the records of
/// the group in `items` when `keep_items` is `true`. Groups are ordered by
/// their key in the total order of values, so the output does not depend on
/// the order of the input. Several keys form a list key. The number of
/// groups, and with `keep_items` the number of held records, is bounded by
/// `limits.max_records`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Group;

impl StageImpl for Group {
    fn descriptor(&self) -> StageDescriptor {
        descriptor("group", InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        group_keys(args)?;
        flag(args, KEEP_ITEMS)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Group {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let keys = group_keys(args)?;
        let keep_items = flag(args, KEEP_ITEMS)?;
        let max = ctx.limits().max_records;
        let parts = Parts::of(input, ctx)?;
        let mut groups: BTreeMap<OrderedValue, Members> = BTreeMap::new();
        let mut held = 0_u64;
        for item in parts.records {
            let record = item?;
            let members = groups
                .entry(OrderedValue(key_value(&keys, &record, ctx)?))
                .or_default();
            members.count += 1;
            if keep_items {
                members.items.push(Value::Record(record));
                held += 1;
            }
            if held.max(len_u64(groups.len())) > max {
                return Err(limit_error("group", max));
            }
        }
        let records = groups
            .into_iter()
            .map(|(key, members)| {
                let mut record = Record::new(RecordKind::Group)
                    .with(KEY_FIELD, key.0)
                    .with(COUNT_FIELD, Value::from(members.count));
                if keep_items {
                    record.set(ITEMS_FIELD, Value::List(members.items));
                }
                record
            })
            .collect();
        Ok(Description {
            header: None,
            order: Some(key_order(KEY_FIELD)),
            render: None,
        }
        .output(records))
    }
}

/// Running sum of the values of one field.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Sum {
    /// No value seen yet.
    #[default]
    Empty,
    /// Integers.
    Int(i128),
    /// Numbers, or integers mixed with numbers.
    Number(f64),
    /// Sizes in bytes.
    Size(u128),
    /// Durations in milliseconds.
    Duration(i128),
    /// Values that cannot be added: text, mixed families and others.
    NotNumeric,
}

impl Sum {
    fn add(self, value: &Value) -> Self {
        match (self, value) {
            (Self::Empty, Value::Int(number)) => Self::Int(i128::from(*number)),
            (Self::Empty, Value::Number(number)) => Self::Number(*number),
            (Self::Empty, Value::Size(size)) => Self::Size(u128::from(size.bytes())),
            (Self::Empty, Value::Duration(duration)) => {
                Self::Duration(i128::from(duration.as_millis()))
            }
            (Self::Int(sum), Value::Int(number)) => {
                Self::Int(sum.saturating_add(i128::from(*number)))
            }
            (Self::Int(sum), Value::Number(number)) => Self::Number(sum as f64 + number),
            (Self::Number(sum), Value::Int(number)) => Self::Number(sum + *number as f64),
            (Self::Number(sum), Value::Number(number)) => Self::Number(sum + number),
            (Self::Size(sum), Value::Size(size)) => {
                Self::Size(sum.saturating_add(u128::from(size.bytes())))
            }
            (Self::Duration(sum), Value::Duration(duration)) => {
                Self::Duration(sum.saturating_add(i128::from(duration.as_millis())))
            }
            _ => Self::NotNumeric,
        }
    }

    fn total(self) -> Value {
        match self {
            Self::Int(sum) => i64::try_from(sum).map_or(Value::Number(sum as f64), Value::Int),
            Self::Number(sum) => Value::Number(sum),
            Self::Size(sum) => {
                u64::try_from(sum).map_or(Value::Null, |bytes| Value::Size(Size::from_bytes(bytes)))
            }
            Self::Duration(sum) => i64::try_from(sum).map_or(Value::Null, |millis| {
                Value::Duration(Duration::from_millis(millis))
            }),
            Self::Empty | Self::NotNumeric => Value::Null,
        }
    }

    /// Mean of `count` values: a number for integers and numbers, a size or
    /// a duration rounded to the nearest unit for sizes and durations.
    fn average(self, count: u64) -> Value {
        if count == 0 {
            return Value::Null;
        }
        let divisor = u128::from(count);
        match self {
            Self::Int(sum) => Value::Number(sum as f64 / count as f64),
            Self::Number(sum) => Value::Number(sum / count as f64),
            Self::Size(sum) => u64::try_from((sum + divisor / 2) / divisor)
                .map_or(Value::Null, |bytes| Value::Size(Size::from_bytes(bytes))),
            Self::Duration(sum) => {
                let divisor = i128::from(count);
                let rounded = (sum + sum.signum() * divisor / 2) / divisor;
                i64::try_from(rounded).map_or(Value::Null, |millis| {
                    Value::Duration(Duration::from_millis(millis))
                })
            }
            Self::Empty | Self::NotNumeric => Value::Null,
        }
    }
}

/// Statistics of one field within one group.
#[derive(Debug, Clone, Default)]
struct Stats {
    count: u64,
    sum: Sum,
    min: Option<Value>,
    max: Option<Value>,
}

impl Stats {
    fn add(&mut self, value: Value) {
        if value.is_null() {
            return;
        }
        self.count += 1;
        self.sum = self.sum.add(&value);
        if self
            .min
            .as_ref()
            .is_none_or(|min| nurt_types::compare(&value, min).is_lt())
        {
            self.min = Some(value.clone());
        }
        if self
            .max
            .as_ref()
            .is_none_or(|max| nurt_types::compare(&value, max).is_gt())
        {
            self.max = Some(value);
        }
    }

    fn fill(self, record: &mut Record) {
        record.set(COUNT_FIELD, Value::from(self.count));
        record.set("sum", self.sum.total());
        record.set("min", self.min.unwrap_or_default());
        record.set("max", self.max.unwrap_or_default());
        record.set("average", self.sum.average(self.count));
    }
}

/// Records and field statistics of one group of `summarize`.
#[derive(Debug, Default)]
struct GroupStats {
    records: u64,
    fields: Vec<Stats>,
}

fn label(key: &Key, index: usize) -> String {
    key.field_name()
        .unwrap_or_else(|| format!("expression_{}", index + 1))
}

/// `summarize fields by`: computes `count`, `sum`, `min`, `max` and
/// `average` of fields, for all records or per group.
///
/// The output is one `summary` record per field, and per group when `by` is
/// given: `{<by>, field, count, sum, min, max, average}`. `count` is the
/// number of values other than `null`; `null` values are skipped. Integers
/// and numbers sum to a number or an integer and average to a number; sizes
/// and durations sum and average in their own unit. Fields of other types,
/// or of mixed types, have `sum` and `average` `null` and `min` and `max` by
/// the total order of values. Without `fields` each group gives one record
/// with the number of its records. Groups are ordered by key; an empty
/// input without `by` gives one record with `count` 0.
#[derive(Debug, Clone, Copy, Default)]
pub struct Summarize;

impl StageImpl for Summarize {
    fn descriptor(&self) -> StageDescriptor {
        descriptor("summarize", InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        keys(args, FIELDS)?;
        keys(args, BY)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Summarize {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let fields = keys(args, FIELDS)?;
        let by = keys(args, BY)?;
        let max = ctx.limits().max_records;
        let parts = Parts::of(input, ctx)?;
        let mut groups: BTreeMap<OrderedValue, GroupStats> = BTreeMap::new();
        for item in parts.records {
            let record = item?;
            let key = if by.is_empty() {
                Value::Null
            } else {
                key_value(&by, &record, ctx)?
            };
            let group = groups
                .entry(OrderedValue(key))
                .or_insert_with(|| GroupStats {
                    records: 0,
                    fields: vec![Stats::default(); fields.len()],
                });
            group.records += 1;
            for (stats, field) in group.fields.iter_mut().zip(&fields) {
                stats.add(field.eval(&record, ctx)?);
            }
            if len_u64(groups.len()) > max {
                return Err(limit_error("summarize", max));
            }
        }
        if by.is_empty() && groups.is_empty() {
            groups.insert(
                OrderedValue(Value::Null),
                GroupStats {
                    records: 0,
                    fields: vec![Stats::default(); fields.len()],
                },
            );
        }
        let by_field = match by.as_slice() {
            [] => None,
            [key] => Some(key.field_name().unwrap_or_else(|| KEY_FIELD.to_owned())),
            _ => Some(KEY_FIELD.to_owned()),
        };
        let records = summaries(groups, &fields, by_field.as_deref());
        Ok(Description {
            header: None,
            order: by_field.as_deref().map(key_order),
            render: None,
        }
        .output(records))
    }
}

fn summaries(
    groups: BTreeMap<OrderedValue, GroupStats>,
    fields: &[Key],
    by_field: Option<&str>,
) -> Vec<Record> {
    let mut records = Vec::new();
    for (key, group) in groups {
        let base = match by_field {
            Some(name) => Record::new(RecordKind::Summary).with(name, key.0),
            None => Record::new(RecordKind::Summary),
        };
        if fields.is_empty() {
            records.push(base.with(COUNT_FIELD, Value::from(group.records)));
            continue;
        }
        for (index, (stats, field)) in group.fields.into_iter().zip(fields).enumerate() {
            let mut record = base.clone().with("field", Value::from(label(field, index)));
            stats.fill(&mut record);
            records.push(record);
        }
    }
    records
}

/// `distinct by`: keeps the first record of each distinct value and adds
/// the number of its occurrences in the field `count`.
///
/// Records are distinct by the given fields or expressions, or by all their
/// fields and their kind when `by` is omitted; values compare by the total
/// order of values. The first occurrences keep their input order, so the
/// order declared by the producer stays valid. The number of distinct
/// records is bounded by `limits.max_records`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Distinct;

impl StageImpl for Distinct {
    fn descriptor(&self) -> StageDescriptor {
        descriptor("distinct", InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        keys(args, BY)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Distinct {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let by = keys(args, BY)?;
        let max = ctx.limits().max_records;
        let parts = Parts::of(input, ctx)?;
        let mut index: BTreeMap<OrderedValue, usize> = BTreeMap::new();
        let mut unique: Vec<(Record, u64)> = Vec::new();
        for item in parts.records {
            let record = item?;
            let key = if by.is_empty() {
                Value::Record(record.clone())
            } else {
                Value::List(
                    by.iter()
                        .map(|key| key.eval(&record, ctx))
                        .collect::<Result<Vec<_>>>()?,
                )
            };
            match index.entry(OrderedValue(key)) {
                Entry::Occupied(found) => {
                    if let Some((_, count)) = unique.get_mut(*found.get()) {
                        *count += 1;
                    }
                }
                Entry::Vacant(slot) => {
                    slot.insert(unique.len());
                    unique.push((record, 1));
                }
            }
            if len_u64(unique.len()) > max {
                return Err(limit_error("distinct", max));
            }
        }
        let records = unique
            .into_iter()
            .map(|(record, count)| record.with(COUNT_FIELD, Value::from(count)))
            .collect();
        Ok(Description {
            header: parts.header,
            order: parts.order,
            render: None,
        }
        .output(records))
    }
}
