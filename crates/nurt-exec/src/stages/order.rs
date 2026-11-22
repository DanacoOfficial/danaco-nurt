// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The stages `sort`, `first`, `last`, `skip` and `count`.

use std::cmp::Ordering;
use std::collections::VecDeque;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, OrderKey, ReadStage, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo, StreamOrder,
};
use nurt_contracts::value::{Record, Value};
use serde_json::json;

use super::common::{
    Description, Key, Parts, count_param, descriptor, fused, keys, known_value, len_u64,
    limit_error, missing_param,
};

const SORT: &str = "sort";
const BY: &str = "by";
const ORDER: &str = "order";
const ASCENDING: &str = "asc";
const DESCENDING: &str = "desc";

/// Name of the static detail through which `first` reports the number of
/// records the rest of the pipeline uses, so that sources can stop early.
pub const LIMIT_DETAIL: &str = "limit";

fn sort_keys(args: &BoundArgs) -> Result<Vec<Key>> {
    let keys = keys(args, BY)?;
    if keys.is_empty() {
        return Err(missing_param(args, BY));
    }
    Ok(keys)
}

fn descending(args: &BoundArgs) -> Result<bool> {
    let Some(value) = known_value(args, ORDER) else {
        return Ok(false);
    };
    match value.as_str() {
        Some(ASCENDING) => Ok(false),
        Some(DESCENDING) => Ok(true),
        _ => Err(Error::new(
            ErrorCode::UnknownValue,
            format!(
                "`sort` orders by `asc` or `desc`, found {}",
                nurt_types::display_short(value)
            ),
        )
        .with_expected(json!([ASCENDING, DESCENDING]))
        .with_found(nurt_types::display_short(value))),
    }
}

fn compare_keys(left: &[Value], right: &[Value]) -> Ordering {
    left.iter()
        .zip(right)
        .map(|(left, right)| nurt_types::compare(left, right))
        .find(|ordering| ordering.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// Order declared for the output when every key is a field.
fn declared_order(keys: &[Key], descending: bool) -> Option<StreamOrder> {
    let keys = keys
        .iter()
        .map(|key| key.field_name().map(|field| OrderKey { field, descending }))
        .collect::<Option<Vec<_>>>()?;
    Some(StreamOrder { keys })
}

/// `sort by order`: orders the records by one or more keys.
///
/// Keys are fields or expressions, the first key first; values compare by
/// the total order of values, so records with keys of different types still
/// sort deterministically (`null` first in ascending order). The sort is
/// stable: records with equal keys keep their input order in both
/// directions. The stage holds all records within `limits.max_records`
/// (`E_LIMIT` beyond). When the producer already declared an order that
/// starts with the requested one, the stream passes through unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sort;

impl StageImpl for Sort {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(SORT, InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        sort_keys(args)?;
        descending(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Sort {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let keys = sort_keys(args)?;
        let descending = descending(args)?;
        let order = declared_order(&keys, descending);
        let parts = Parts::of(input, ctx)?;
        if let (Some(wanted), Some(known)) = (&order, &parts.order)
            && known.keys.starts_with(&wanted.keys)
        {
            return Ok(Description {
                header: parts.header,
                order: parts.order.clone(),
                render: None,
            }
            .stream(parts.records));
        }
        let (records, description) = parts.collect(ctx)?;
        let mut keyed = records
            .into_iter()
            .map(|record| {
                let values = keys
                    .iter()
                    .map(|key| key.eval(&record, ctx))
                    .collect::<Result<Vec<_>>>()?;
                Ok((values, record))
            })
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_by(|(left, _), (right, _)| {
            let ordering = compare_keys(left, right);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        let sorted = keyed.into_iter().map(|(_, record)| record).collect();
        Ok(Description {
            header: description.header,
            order,
            render: None,
        }
        .output(sorted))
    }
}

const COUNT: &str = "count";

fn limit_details(args: &BoundArgs) -> Result<StaticInfo> {
    let count = count_param(args, COUNT)?;
    let mut info = StaticInfo::default();
    info.details
        .insert(LIMIT_DETAIL.to_owned(), Value::from(count));
    Ok(info)
}

/// `first count`: keeps the first `count` records.
///
/// The stage is lazy: it pulls no more than `count` records from its input,
/// so a source is never read past them, and `first 0` reads nothing. The
/// static check reports the count as the detail [`LIMIT_DETAIL`], which the
/// planner passes to the sources as the record limit of their demand.
#[derive(Debug, Clone, Copy, Default)]
pub struct First;

impl StageImpl for First {
    fn descriptor(&self) -> StageDescriptor {
        descriptor("first", InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        limit_details(args)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for First {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let count = usize::try_from(count_param(args, COUNT)?).unwrap_or(usize::MAX);
        input.map_records(|records| fused(records.checked(ctx).take(count)))
    }
}

/// `skip count`: drops the first `count` records and passes the rest.
///
/// The stage is lazy and pulls the dropped records one at a time; an error
/// among them still ends the stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct Skip;

impl StageImpl for Skip {
    fn descriptor(&self) -> StageDescriptor {
        descriptor("skip", InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        count_param(args, COUNT)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Skip {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let count = count_param(args, COUNT)?;
        input.map_records(|records| {
            let mut dropped = 0_u64;
            fused(records.checked(ctx).filter(move |item| {
                if item.is_err() || dropped >= count {
                    return true;
                }
                dropped += 1;
                false
            }))
        })
    }
}

/// `last count`: keeps the last `count` records.
///
/// The stage reads its whole input and holds no more than `count` records
/// at a time; a count above `limits.max_records` fails with `E_LIMIT` once
/// the stream holds more records than the limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct Last;

impl StageImpl for Last {
    fn descriptor(&self) -> StageDescriptor {
        descriptor("last", InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        count_param(args, COUNT)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Last {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let count = count_param(args, COUNT)?;
        let max = ctx.limits().max_records;
        let parts = Parts::of(input, ctx)?;
        let mut tail: VecDeque<Record> = VecDeque::new();
        for item in parts.records {
            tail.push_back(item?);
            if len_u64(tail.len()) > count {
                tail.pop_front();
            }
            if len_u64(tail.len()) > max {
                return Err(limit_error("last", max));
            }
        }
        Ok(Description {
            header: parts.header,
            order: parts.order,
            render: parts.render,
        }
        .output(tail.into()))
    }
}

/// `count`: the number of input records as an `int`.
///
/// The stage reads the whole input without holding it. It counts file
/// records before an effect as well as the records of an effect after it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Count;

impl StageImpl for Count {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(COUNT, InputKinds::Any)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Count {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let mut count = 0_u64;
        for item in input.into_records()?.checked(ctx) {
            item?;
            count += 1;
        }
        Ok(StageOutput::value(Value::from(count)))
    }
}
