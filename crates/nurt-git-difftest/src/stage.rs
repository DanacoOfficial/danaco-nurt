// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of a command through its stage implementation, the way the
//! executor runs it: static check, then the read interface, or plan and apply
//! inside a journal transaction.

use std::collections::BTreeMap;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::journal::{MutationTx, OpId, OpKind, RollbackReason};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectStage, ExecCtx, StageData, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Value;

use crate::journal::RecordingJournal;

/// Stage implementations available to the scenarios, by command name.
#[derive(Clone, Default)]
pub struct CommandTable {
    stages: BTreeMap<String, Arc<dyn StageImpl>>,
}

impl std::fmt::Debug for CommandTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.stages.keys()).finish()
    }
}

impl CommandTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an implementation under the command name of its descriptor,
    /// replacing an earlier one.
    pub fn insert(&mut self, stage: Arc<dyn StageImpl>) {
        self.stages.insert(stage.descriptor().command, stage);
    }

    /// Implementation of `command`.
    #[must_use]
    pub fn get(&self, command: &str) -> Option<&Arc<dyn StageImpl>> {
        self.stages.get(command)
    }

    /// Names of the commands in the table.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.stages.keys().map(String::as_str)
    }
}

/// How the command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageResult {
    /// Error code; `None` when the command succeeded.
    pub code: Option<ErrorCode>,
    /// Error message.
    pub message: Option<String>,
    /// Committed journal operation, when the command changed something.
    pub committed: Option<OpId>,
    /// Number of output records.
    pub records: usize,
    /// The effect interface was used.
    pub effect: bool,
}

impl StageResult {
    fn failed(error: &Error, committed: Option<OpId>, effect: bool) -> Self {
        Self {
            code: Some(error.code()),
            message: Some(error.data().message.clone()),
            committed,
            records: 0,
            effect,
        }
    }

    fn succeeded(records: usize, committed: Option<OpId>, effect: bool) -> Self {
        Self {
            code: None,
            message: None,
            committed,
            records,
            effect,
        }
    }
}

/// Runs one stage with bound arguments.
///
/// The effect interface is used when the effect of the selected action is
/// not `read`; the transaction is committed after success and after an error
/// that reports an applied effect (`E_GIT_CONFLICT`), and rolled back
/// otherwise.
#[must_use]
pub fn run_stage(
    stage: &dyn StageImpl,
    spec: &StageSpec,
    args: &BoundArgs,
    ctx: &ExecCtx,
    journal: &RecordingJournal,
) -> StageResult {
    let check = CheckCtx::new(spec, ctx.host(), ctx.config());
    let wants_effect = effect_of(spec, args) != EffectKind::Read || stage.as_read().is_none();
    if let Err(error) = stage.check(args, &check) {
        return StageResult::failed(&error, None, wants_effect);
    }
    if wants_effect {
        if let Some(effect) = stage.as_effect() {
            return run_effect(effect, spec, args, ctx, journal);
        }
    }
    let Some(read) = stage.as_read() else {
        let error = Error::new(ErrorCode::StageFailed, "the stage has no interface for this action");
        return StageResult::failed(&error, None, wants_effect);
    };
    match read.execute(args, StageInput::none(), ctx).and_then(drain) {
        Ok(records) => StageResult::succeeded(records, None, false),
        Err(error) => StageResult::failed(&error, None, false),
    }
}

fn run_effect(
    effect: &dyn EffectStage,
    spec: &StageSpec,
    args: &BoundArgs,
    ctx: &ExecCtx,
    journal: &RecordingJournal,
) -> StageResult {
    let plan = match effect.plan(args, StageInput::none(), ctx) {
        Ok(plan) => plan,
        Err(error) => return StageResult::failed(&error, None, true),
    };
    let mut tx: Box<dyn MutationTx + '_> = Box::new(journal.begin(op_kind(spec, args)));
    match effect.apply(args, plan, tx.as_mut(), ctx) {
        Ok(outcome) => {
            let records = drain(outcome.output);
            match (records, tx.commit()) {
                (Ok(records), Ok(handle)) => StageResult::succeeded(records, handle.op, true),
                (Err(error), Ok(handle)) => StageResult::failed(&error, handle.op, true),
                (_, Err(error)) => StageResult::failed(&error, None, true),
            }
        }
        Err(error) if error.data().effect_applied => {
            let committed = tx.commit().ok().and_then(|handle| handle.op);
            StageResult::failed(&error, committed, true)
        }
        Err(error) => {
            let _ = tx.rollback(RollbackReason::TargetFailed);
            StageResult::failed(&error, None, true)
        }
    }
}

fn drain(output: StageOutput<'_>) -> nurt_contracts::error::Result<usize> {
    match output.data {
        StageData::None => Ok(0),
        StageData::Value(_) => Ok(1),
        StageData::Records(stream) => {
            let mut count = 0;
            for record in stream {
                record?;
                count += 1;
            }
            Ok(count)
        }
    }
}

/// Text of an enumeration or text argument.
fn word<'a>(args: &'a BoundArgs, name: &str) -> Option<&'a str> {
    match args.value(name) {
        Some(Value::Enum(text) | Value::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}

/// Effect of the action selected by the arguments: the first entry of
/// `effects` whose action is the value of an enumeration argument or the
/// name of a given argument, otherwise the default effect of the command.
fn effect_of(spec: &StageSpec, args: &BoundArgs) -> EffectKind {
    spec.effects
        .iter()
        .find(|entry| {
            args.is_given(&entry.action)
                || spec
                    .params
                    .iter()
                    .any(|param| word(args, &param.name) == Some(entry.action.as_str()))
        })
        .map_or(spec.effect, |entry| entry.effect)
}

fn op_kind(spec: &StageSpec, args: &BoundArgs) -> OpKind {
    match word(args, "action") {
        Some(action) => OpKind::with_action(&spec.name, action),
        None => OpKind::command(&spec.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind::Catalog;

    #[test]
    fn read_actions_select_the_read_interface() {
        let catalog = Catalog::git().unwrap();
        let spec = catalog.command("git.stash").unwrap();
        let root = std::env::temp_dir();
        let list = catalog
            .bind("git.stash", &toml::from_str("action = \"list\"").unwrap(), &root)
            .unwrap();
        assert_eq!(effect_of(spec, &list.args), EffectKind::Read);
        let push = catalog
            .bind("git.stash", &toml::from_str("action = \"push\"").unwrap(), &root)
            .unwrap();
        assert_eq!(effect_of(spec, &push.args), EffectKind::Local);
        assert_eq!(op_kind(spec, &push.args).action.as_deref(), Some("push"));
    }

    #[test]
    fn empty_table_has_no_commands() {
        let table = CommandTable::new();
        assert!(table.get("git.status").is_none());
        assert_eq!(table.names().count(), 0);
        assert_eq!(format!("{table:?}"), "{}");
    }
}
