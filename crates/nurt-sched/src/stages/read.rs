// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sched::stages::read
// Responsibility:   `schedule.list` and `schedule.history`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `schedule.list` and `schedule.history`.

use nurt_contracts::error::Error;
use nurt_contracts::record::ScheduleState;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, OrderKey, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StreamOrder,
};
use nurt_contracts::value::{Record, Value};

use super::{now, run_record, running_link, schedule_record, shown_state, store, text_arg};
use crate::errors::{invalid, not_found};
use crate::store::{RunRecord, check_id};

const DEFAULT_HISTORY_LIMIT: i64 = 20;
const ALL_STATES: &str = "all";

/// `schedule.list`: lists schedules with their next run and state.
#[derive(Debug, Clone, Copy, Default)]
pub struct List;

/// `schedule.history`: lists past runs, newest first.
#[derive(Debug, Clone, Copy, Default)]
pub struct History;

impl StageImpl for List {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "schedule.list".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for List {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>, Error> {
        let store = store(ctx)?;
        let id = text_arg(args, "id")?;
        if let Some(id) = &id {
            check_id(id)?;
        }
        let wanted = text_arg(args, "state")?.unwrap_or_else(|| ALL_STATES.to_owned());
        let wanted = match wanted.as_str() {
            ALL_STATES => None,
            other => Some(
                ScheduleState::ALL
                    .iter()
                    .copied()
                    .find(|state| state.as_str() == other)
                    .ok_or_else(|| invalid(format!("unknown schedule state `{other}`")))?,
            ),
        };
        let origin = text_arg(args, "origin_session")?;
        let definitions = store.load_definitions()?.items;
        let state = store.read_state()?;
        if let Some(id) = &id
            && !definitions.iter().any(|definition| &definition.id == id)
        {
            return Err(not_found(std::slice::from_ref(id)));
        }
        let running = running_link(ctx).is_some();
        let now = now(ctx);
        let records: Vec<Record> = definitions
            .iter()
            .filter(|definition| id.as_ref().is_none_or(|id| &definition.id == id))
            .filter(|definition| {
                origin
                    .as_ref()
                    .is_none_or(|origin| definition.session.as_ref() == Some(origin))
            })
            .filter(|definition| {
                wanted.is_none_or(|wanted| {
                    shown_state(definition, state.schedules.get(&definition.id)) == wanted
                })
            })
            .map(|definition| {
                schedule_record(
                    definition,
                    state.schedules.get(&definition.id),
                    running,
                    now,
                )
            })
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

impl StageImpl for History {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "schedule.history".to_owned(),
            source: true,
            input: InputKinds::None,
        }
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
    ) -> Result<StageOutput<'a>, Error> {
        let store = store(ctx)?;
        let id = text_arg(args, "id")?;
        let limit = match args.value("limit") {
            Some(Value::Int(limit)) if *limit >= 1 => *limit,
            Some(Value::Int(_)) => return Err(invalid("`limit` must be at least 1")),
            _ => DEFAULT_HISTORY_LIMIT,
        };
        let state = store.read_state()?;
        if let Some(id) = &id {
            check_id(id)?;
            let known = state.history.contains_key(id) || store.load_definition(id)?.is_some();
            if !known {
                return Err(not_found(std::slice::from_ref(id)));
            }
        }
        let runs = newest_first(
            state
                .history
                .iter()
                .filter(|(schedule, _)| id.as_ref().is_none_or(|id| *schedule == id))
                .flat_map(|(_, runs)| runs.iter()),
            usize::try_from(limit).unwrap_or(usize::MAX),
        );
        let records = runs.iter().map(run_record).collect();
        Ok(
            StageOutput::records(RecordStream::from_records(records)).with_order(StreamOrder {
                keys: vec![OrderKey {
                    field: "started".to_owned(),
                    descending: true,
                }],
            }),
        )
    }
}

/// Runs ordered from the newest start, at most `limit`.
fn newest_first<'a>(runs: impl Iterator<Item = &'a RunRecord>, limit: usize) -> Vec<RunRecord> {
    let mut runs: Vec<RunRecord> = runs.cloned().collect();
    runs.sort_by(|a, b| {
        b.started
            .cmp(&a.started)
            .then_with(|| b.run.cmp(&a.run))
            .then_with(|| a.schedule.cmp(&b.schedule))
    });
    runs.truncate(limit);
    runs
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
