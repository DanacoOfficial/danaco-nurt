// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `schedule.run`.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageData, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use super::{now, running_link, schedule_record, store, text_arg};
use crate::action::PIPELINE_KIND;
use crate::errors::{busy, failed, not_found};
use crate::module::ModuleRequest;
use crate::store::{Definition, check_id};

/// `schedule.run`: runs the action of a schedule now, outside its run times
/// and its run limit.
///
/// With a running service the service executes the run, records it in the
/// history with `trigger: manual` and returns the output of the action.
/// Without the service the calling process runs a pipeline action itself;
/// such a run is not recorded in the history.
#[derive(Debug, Clone, Copy, Default)]
pub struct Run;

impl StageImpl for Run {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "schedule.run".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Run {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan, Error> {
        let definition = definition(args, ctx)?;
        let store = store(ctx)?;
        let state = store.read_state()?;
        let mut plan = EffectPlan::new();
        plan.preview.push(schedule_record(
            &definition,
            state.schedules.get(&definition.id),
            running_link(ctx).is_some(),
            now(ctx),
        ));
        Ok(plan.with_state(definition))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>, Error> {
        let definition: Definition = plan
            .take_state()
            .ok_or_else(|| failed("the plan of `schedule.run` has no schedule"))?;
        let records = match running_link(ctx) {
            Some(link) => {
                let reply = link.request(ModuleRequest::new(
                    "schedule.run",
                    serde_json::json!({ "id": definition.id }),
                ))?;
                output_records(&reply)
            }
            None => run_here(&definition, ctx)?,
        };
        Ok(EffectOutcome::new(StageOutput::records(
            RecordStream::from_records(records),
        )))
    }
}

fn definition(args: &BoundArgs, ctx: &ExecCtx) -> Result<Definition, Error> {
    let id = text_arg(args, "id")?
        .ok_or_else(|| Error::new(ErrorCode::MissingParam, "`schedule.run` needs a value for `id`"))?;
    check_id(&id)?;
    store(ctx)?
        .load_definition(&id)?
        .ok_or_else(|| not_found(std::slice::from_ref(&id)))
}

fn output_records(reply: &serde_json::Value) -> Vec<Record> {
    reply
        .get("output")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<Record>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn run_here(definition: &Definition, ctx: &ExecCtx) -> Result<Vec<Record>, Error> {
    let pipeline = match (&definition.pipeline, definition.action_kind.as_str()) {
        (Some(pipeline), PIPELINE_KIND) => pipeline,
        _ => {
            return Err(failed(format!(
                "the action kind `{}` of schedule `{}` runs only in the service; start it with \
                 `nurt service run`",
                definition.action_kind, definition.id
            )));
        }
    };
    let store = store(ctx)?;
    let Some(_lock) = store.try_run_lock(&definition.id)? else {
        return Err(busy(&definition.id, &definition.id));
    };
    let output = ctx
        .pipelines()?
        .run(pipeline, StageInput::none(), ctx)?;
    match output.data {
        StageData::Records(stream) => stream.collect_bounded(ctx.limits().max_records),
        _ => Ok(Vec::new()),
    }
}
