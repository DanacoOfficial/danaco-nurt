// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `schedule.remove`, `schedule.pause` and `schedule.resume`.

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetIndex};
use nurt_contracts::record::{Effect, EffectStatus, ScheduleState};
use nurt_contracts::report::{Change, ChangeAction, Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use super::{
    bool_arg, ids_arg, notify_changed, now, running_link, shown_state, store, target, text_arg,
};
use crate::errors::{conflict, failed, not_found};
use crate::module::ModuleRequest;
use crate::store::{Definition, Store, check_id, encode_definition};

const REMOVED: &str = "removed";
const UNCHANGED: &str = "unchanged";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Remove,
    Pause,
    Resume,
}

impl Op {
    const fn command(self) -> &'static str {
        match self {
            Self::Remove => "schedule.remove",
            Self::Pause => "schedule.pause",
            Self::Resume => "schedule.resume",
        }
    }

    const fn action(self) -> &'static str {
        match self {
            Self::Remove => "remove",
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

/// `schedule.remove`, `schedule.pause` or `schedule.resume`: changes the
/// definitions of the selected schedules in one operation of the journal.
///
/// `pause` applies only to active schedules and `resume` only to paused
/// ones; other selected schedules are reported as `unchanged` with their
/// state. Removing a schedule stops its running runs unless `stop_running`
/// is false.
#[derive(Debug, Clone, Copy)]
pub struct ChangeStage {
    op: Op,
}

#[derive(Debug)]
struct Planned {
    definition: Definition,
    before: ScheduleState,
    change: bool,
}

impl ChangeStage {
    /// `schedule.remove`.
    #[must_use]
    pub const fn remove() -> Self {
        Self { op: Op::Remove }
    }

    /// `schedule.pause`.
    #[must_use]
    pub const fn pause() -> Self {
        Self { op: Op::Pause }
    }

    /// `schedule.resume`.
    #[must_use]
    pub const fn resume() -> Self {
        Self { op: Op::Resume }
    }

    fn after(self, planned: &Planned) -> &'static str {
        if !planned.change {
            return UNCHANGED;
        }
        match self.op {
            Op::Remove => REMOVED,
            Op::Pause => ScheduleState::Paused.as_str(),
            Op::Resume => ScheduleState::Active.as_str(),
        }
    }
}

impl StageImpl for ChangeStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.op.command().to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for ChangeStage {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan, Error> {
        let store = store(ctx)?;
        let state = store.read_state()?;
        let selected = select(args, &store)?;
        let mut planned = Vec::with_capacity(selected.len());
        for definition in selected {
            let before = shown_state(&definition, state.schedules.get(&definition.id));
            let change = match self.op {
                Op::Remove => true,
                Op::Pause => before == ScheduleState::Active,
                Op::Resume => before == ScheduleState::Paused,
            };
            planned.push(Planned {
                definition,
                before,
                change,
            });
        }
        let mut plan = EffectPlan::new();
        for item in planned.iter().filter(|item| item.change) {
            let action = if self.op == Op::Remove {
                ChangeAction::Delete
            } else {
                ChangeAction::Modify
            };
            let mut change = Change::new(
                store
                    .definition_path(&item.definition.id)
                    .display()
                    .to_string(),
                action,
            );
            change.before = Some(item.before.as_str().to_owned());
            change.after = Some(self.after(item).to_owned());
            plan.changes.push(change);
        }
        plan.bulk = plan.changes.len() > 1;
        plan.preview = planned.iter().map(|item| self.effect(item, &[])).collect();
        Ok(plan.with_state(planned))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>, Error> {
        let planned: Vec<Planned> = plan.take_state().ok_or_else(|| {
            failed(format!(
                "the plan of `{}` has no schedules",
                self.op.command()
            ))
        })?;
        let store = store(ctx)?;
        let changed: Vec<&Planned> = planned.iter().filter(|item| item.change).collect();
        let action = if self.op == Op::Remove {
            TargetAction::Delete
        } else {
            TargetAction::Modify
        };
        let targets = changed
            .iter()
            .map(|item| {
                target(
                    &store.definition_path(&item.definition.id),
                    action.clone(),
                    Expect::Unchecked,
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        tx.preserve(&targets)?;
        let now = now(ctx);
        for (index, item) in changed.iter().enumerate() {
            let index = TargetIndex(u32::try_from(index).unwrap_or(u32::MAX));
            let mutation = match self.op {
                Op::Remove => Mutation::Remove,
                Op::Pause | Op::Resume => {
                    let mut definition = item.definition.clone();
                    definition.paused = self.op == Op::Pause;
                    definition.changed = now;
                    Mutation::WriteBytes(encode_definition(&definition)?)
                }
            };
            tx.apply(index, mutation)?;
        }
        let mut stopped_by_id = IndexMap::new();
        if self.op == Op::Remove && bool_arg(args, "stop_running", true)? {
            for item in &changed {
                let stopped = stop_runs(ctx, &item.definition.id)?;
                if !stopped.is_empty() {
                    tx.mark_irreversible(IrreversibleEntry {
                        reason: Irreversible::ProcessStopped,
                        path: Some(format!("schedule:{}", item.definition.id)),
                        raw_path: None,
                        count: u64::try_from(stopped.len()).ok(),
                        detail: Some(stopped.join(", ")),
                    });
                    stopped_by_id.insert(item.definition.id.clone(), stopped);
                }
            }
        }
        notify_changed(ctx);
        let records: Vec<Record> = planned
            .iter()
            .map(|item| {
                let stopped = stopped_by_id
                    .get(&item.definition.id)
                    .cloned()
                    .unwrap_or_default();
                self.effect(item, &stopped)
            })
            .collect();
        let mut outcome =
            EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}

impl ChangeStage {
    fn effect(&self, item: &Planned, stopped: &[String]) -> Record {
        let after = self.after(item);
        let mut details = IndexMap::new();
        details.insert("state".to_owned(), Value::Text(after.to_owned()));
        if !stopped.is_empty() {
            details.insert(
                "stopped_jobs".to_owned(),
                Value::List(stopped.iter().cloned().map(Value::Text).collect()),
            );
        }
        let mut effect = Effect {
            status: EffectStatus::Ok,
            kind: self.op.command().to_owned(),
            targets: Some(1),
            target: Some(format!("schedule:{}", item.definition.id)),
            action: Some(
                if item.change {
                    self.op.action()
                } else {
                    UNCHANGED
                }
                .to_owned(),
            ),
            before: Some(Value::Text(item.before.as_str().to_owned())),
            after: Some(Value::Map(details)),
            ..Effect::default()
        };
        if !stopped.is_empty() {
            effect.irreversible = Some(vec![IrreversibleEntry {
                reason: Irreversible::ProcessStopped,
                path: effect.target.clone(),
                raw_path: None,
                count: u64::try_from(stopped.len()).ok(),
                detail: Some(stopped.join(", ")),
            }]);
        }
        Record::from(effect)
    }
}

fn select(args: &BoundArgs, store: &Store) -> Result<Vec<Definition>, Error> {
    let ids = ids_arg(args)?;
    let all = bool_arg(args, "all", false)?;
    let origin = text_arg(args, "origin_session")?;
    if all && !ids.is_empty() {
        return Err(conflict(format!(
            "`{}` takes either `id` or `all: true`, not both",
            args.command()
        )));
    }
    if !all && ids.is_empty() && origin.is_none() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            format!(
                "`{}` needs `id`, `all: true` or `origin_session`",
                args.command()
            ),
        ));
    }
    for id in &ids {
        check_id(id)?;
    }
    let definitions = store.load_definitions()?.items;
    let missing: Vec<String> = ids
        .iter()
        .filter(|id| !definitions.iter().any(|definition| &definition.id == *id))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(not_found(&missing));
    }
    Ok(definitions
        .into_iter()
        .filter(|definition| ids.is_empty() || ids.contains(&definition.id))
        .filter(|definition| {
            origin
                .as_ref()
                .is_none_or(|origin| definition.session.as_ref() == Some(origin))
        })
        .collect())
}

fn stop_runs(ctx: &ExecCtx, id: &str) -> Result<Vec<String>, Error> {
    let Some(link) = running_link(ctx) else {
        return Ok(Vec::new());
    };
    let reply = link.request(ModuleRequest::new(
        "schedule.stop",
        serde_json::json!({ "id": id }),
    ))?;
    Ok(reply
        .get("stopped")
        .and_then(serde_json::Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}
