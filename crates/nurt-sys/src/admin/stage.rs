// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::stage
// Responsibility:   The stage of an administration command: plan, preview and apply.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The stage of an administration command: plan, preview and apply.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{FileOp, MutationTx, TargetPath};
use nurt_contracts::record::{Change, ChangeAction, Effect, EffectStatus};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Value};

use super::sys::{CtxSys, Sys, run_ok};
use super::{COMMANDS, Plan, StateChange, plan};

/// An administration command; see the module documentation for the list.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
///
/// let stages = nurt_sys::admin::stages();
/// assert_eq!(stages[0].descriptor().command, "accounts.create");
/// assert!(stages[0].descriptor().source);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminCommand {
    command: &'static str,
}

/// The stages of the administration commands, in catalog order.
#[must_use]
pub(super) fn stages() -> Vec<Arc<dyn StageImpl>> {
    COMMANDS
        .iter()
        .map(|command| Arc::new(AdminCommand { command }) as Arc<dyn StageImpl>)
        .collect()
}

impl AdminCommand {
    /// Name of the command.
    #[must_use]
    pub const fn command(&self) -> &'static str {
        self.command
    }
}

impl StageImpl for AdminCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, _args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for AdminCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let system = CtxSys::new(ctx);
        let planned = plan(&system, args)?;
        Ok(effect_plan(self.command, &planned))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan.take_state::<Plan>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "the administration command has no plan",
            )
        })?;
        let system = CtxSys::new(ctx);
        apply_plan(self.command, &system, planned, tx)
    }
}

fn effect_plan(command: &str, planned: &Plan) -> EffectPlan {
    let mut effect = EffectPlan::new();
    effect.bulk = planned.changes.len() + planned.files.len() > 1;
    if let Some(reason) = &planned.unchanged {
        let mut note = NotableEntry::new(Notable::Unchanged);
        note.detail = Some(reason.clone());
        effect.notable.push(note);
        return effect.with_state(Plan {
            calls: Vec::new(),
            files: Vec::new(),
            changes: Vec::new(),
            irreversible: None,
            unchanged: Some(reason.clone()),
            notes: Vec::new(),
        });
    }
    for state in &planned.changes {
        effect.changes.push(state_change(state));
        effect
            .preview
            .push(effect_record(command, state, EffectStatus::Planned).into());
    }
    for file in &planned.files {
        effect.changes.push(file_change(file));
    }
    for line in &planned.notes {
        let mut preview = Effect {
            status: EffectStatus::Planned,
            kind: command.to_owned(),
            ..Effect::default()
        };
        preview.action = Some(line.clone());
        effect.preview.push(preview.into());
    }
    if let Some(reason) = &planned.irreversible {
        effect.irreversible.push(marker(reason));
    }
    effect.with_state(planned.clone())
}

fn apply_plan<'a>(
    command: &str,
    system: &dyn Sys,
    planned: Plan,
    tx: &mut dyn MutationTx,
) -> Result<EffectOutcome<'a>> {
    let mut outcome = EffectOutcome::new(StageOutput::none());
    if planned.unchanged.is_some() {
        return Ok(outcome);
    }
    if let Some(reason) = &planned.irreversible {
        tx.mark_irreversible(marker(reason));
        outcome.irreversible.push(marker(reason));
    }
    let preserved = if planned.files.is_empty() {
        Vec::new()
    } else {
        let specs: Vec<_> = planned.files.iter().map(|op| op.target.clone()).collect();
        tx.preserve(&specs)?
    };
    for (done, call) in planned.calls.iter().enumerate() {
        run_ok(system, call).map_err(|error| error.with_effect_applied(done > 0))?;
    }
    for (op, kept) in planned.files.iter().zip(&preserved) {
        tx.apply(kept.index, op.mutation.clone())?;
        outcome.changes.push(file_change(op));
    }
    let mut records: Vec<Record> = Vec::new();
    for state in &planned.changes {
        outcome.changes.push(state_change(state));
        records.push(effect_record(command, state, EffectStatus::Ok).into());
    }
    outcome.output = StageOutput::records(RecordStream::from_records(records));
    Ok(outcome)
}

fn marker(reason: &str) -> IrreversibleEntry {
    let mut entry = IrreversibleEntry::new(Irreversible::ExternalEffect);
    entry.detail = Some(reason.to_owned());
    entry
}

fn state_change(state: &StateChange) -> Change {
    let mut change = Change::new(state.target.clone(), ChangeAction::State);
    change.before.clone_from(&state.before);
    change.after.clone_from(&state.after);
    change
}

fn file_change(op: &FileOp) -> Change {
    let path = match &op.target.path {
        TargetPath::Fs(path) => path.to_language_string(),
        TargetPath::Provider(_) => String::new(),
    };
    let action = match op.target.action {
        nurt_contracts::journal::TargetAction::Delete => ChangeAction::Delete,
        nurt_contracts::journal::TargetAction::Create => ChangeAction::Create,
        _ => ChangeAction::Modify,
    };
    Change::new(path, action)
}

fn effect_record(command: &str, state: &StateChange, status: EffectStatus) -> Effect {
    Effect {
        status,
        kind: command.to_owned(),
        target: Some(state.target.clone()),
        action: Some(command.to_owned()),
        before: state.before.clone().map(Value::Text),
        after: state.after.clone().map(Value::Text),
        ..Effect::default()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
