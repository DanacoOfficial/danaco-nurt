// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `history` command: journal entries as `operation` records,
//! latest first.

use nurt_contracts::error::Result;
use nurt_contracts::journal::{HistoryQuery, OpId, OpState};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, OrderKey, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo, StreamOrder,
};
use nurt_contracts::value::Record;

use super::Params;

const NAME: &str = "history";

/// Number of operations listed without `limit`.
const DEFAULT_LIMIT: u32 = 20;

const STATES: [(&str, Option<OpState>); 5] = [
    ("any", None),
    ("available", Some(OpState::Available)),
    ("partial", Some(OpState::Partial)),
    ("expired", Some(OpState::Expired)),
    ("undone", Some(OpState::Undone)),
];

/// Arguments of `history`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HistoryArgs {
    id: Option<OpId>,
    limit: u32,
    state: Option<OpState>,
    before: Option<OpId>,
    since: Option<OpId>,
    path: Option<String>,
    deleted: bool,
    details: bool,
}

/// Reads the arguments of `history`.
pub(super) fn parse(args: &BoundArgs) -> Result<HistoryArgs> {
    let params = Params::new(args);
    let id = params.op("id")?;
    let limit = match params.int("limit")? {
        None => DEFAULT_LIMIT,
        Some(number) => u32::try_from(number)
            .ok()
            .filter(|limit| *limit > 0)
            .ok_or_else(|| {
                params.invalid(format!(
                    "`limit` {number} is out of range; give a number from 1 to {}",
                    u32::MAX
                ))
            })?,
    };
    Ok(HistoryArgs {
        id,
        limit,
        state: params.choice("state", &STATES)?.flatten(),
        before: params.op("before")?,
        since: params.op("since")?,
        path: params.path("path")?,
        deleted: params.flag("deleted")?,
        details: params.flag("details")?,
    })
}

/// The `history` command.
pub(super) struct History;

impl StageImpl for History {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let parsed = parse(args)?;
        Ok(super::static_info(parsed.path.into_iter().collect()))
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for History {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let parsed = parse(args)?;
        let store = super::store(ctx)?;
        let query = HistoryQuery {
            id: parsed.id,
            limit: Some(parsed.limit),
            state: parsed.state,
            before: parsed.before,
            since: parsed.since,
            path: super::resolve_opt(ctx, parsed.path.as_deref())?,
            deleted_only: parsed.deleted,
        };
        let operations = store.history(&query)?;
        if parsed.details
            && operations.is_empty()
            && let Some(id) = parsed.id
        {
            store.details(id)?;
        }
        let mut listed = Vec::with_capacity(operations.len());
        for operation in operations {
            ctx.checkpoint()?;
            listed.push(Record::from(operation));
        }
        let order = StreamOrder {
            keys: vec![OrderKey {
                field: "id".to_owned(),
                descending: true,
            }],
        };
        Ok(StageOutput::records(RecordStream::from_records(listed)).with_order(order))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::record::Operation;
    use nurt_contracts::value::Value;

    use super::super::testing::{Env, call, read};
    use super::*;

    #[test]
    fn lists_operations_latest_first() {
        let env = Env::new();
        let a = env.seed("a.txt", b"a");
        let b = env.seed("b.txt", b"b");
        env.write(&a, b"A");
        env.write(&b, b"B");
        let ctx = env.ctx(NAME);
        let listed = read(&History, &call(NAME, &[]), &ctx).unwrap();
        let ids: Vec<_> = listed.iter().map(|record| record.get("id")).collect();
        assert_eq!(ids, [Some(&Value::Int(2)), Some(&Value::Int(1))]);
        assert!(listed[0].get("items").is_none());

        let only = call(NAME, &[("path", Value::Path(a.to_string()))]);
        let listed = read(&History, &only, &ctx).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].get("id"), Some(&Value::Int(1)));
    }

    #[test]
    fn records_with_and_without_details_follow_the_operation_contract() {
        let env = Env::new();
        let a = env.seed("a.txt", b"a");
        let b = env.seed("b.txt", b"b");
        env.write(&a, b"A");
        env.write(&b, b"B");
        let ctx = env.ctx(NAME);
        let plain = read(&History, &call(NAME, &[]), &ctx).unwrap();
        let detailed = read(
            &History,
            &call(NAME, &[("details", Value::Bool(true))]),
            &ctx,
        )
        .unwrap();
        let one = read(
            &History,
            &call(
                NAME,
                &[("id", Value::Int(1)), ("details", Value::Bool(true))],
            ),
            &ctx,
        )
        .unwrap();
        assert_eq!(plain.len(), 2);
        assert_eq!(detailed, plain);
        assert_eq!(one.len(), 1);
        for record in plain.iter().chain(&detailed).chain(&one) {
            let typed = Operation::try_from(record.clone()).unwrap();
            assert_eq!(Record::from(typed), *record);
        }
    }

    #[test]
    fn details_of_an_unknown_operation_fail() {
        let env = Env::new();
        let ctx = env.ctx(NAME);
        let args = call(
            NAME,
            &[("id", Value::Int(9)), ("details", Value::Bool(true))],
        );
        let error = read(&History, &args, &ctx).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalNoop);
        let listed = read(&History, &call(NAME, &[("id", Value::Int(9))]), &ctx).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn reads_the_catalog_defaults() {
        let parsed = parse(&call(NAME, &[])).unwrap();
        assert_eq!(parsed.limit, DEFAULT_LIMIT);
        assert_eq!(parsed.state, None);
        let error = parse(&call(NAME, &[("limit", Value::Int(0))])).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
        let available = call(NAME, &[("state", Value::Enum("available".to_owned()))]);
        assert_eq!(parse(&available).unwrap().state, Some(OpState::Available));
    }
}
