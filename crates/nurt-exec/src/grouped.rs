// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Planning and applying of effect stages whose input mixes record kinds.
//!
//! An effect stage with several implementations serves each kind of record
//! with its own implementation. The groups are planned one after another and
//! applied through one journal transaction: the targets of every group are
//! declared and preserved before the first group changes anything, and the
//! groups change their targets one after another in the order of the groups.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetIndex, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, ExecCtx, ExecStep, RecordStream, StageData, StageImpl,
    StageOutput,
};

use crate::flow::{Selection, at_step, effect_interface, guard_kinds, internal};

/// The effect stage of an operation: one implementation, or one per group of
/// record kinds.
pub(crate) struct Effect {
    command: String,
    implementations: Vec<Arc<dyn StageImpl>>,
}

/// Plans of the groups of an effect, kept as the state of the merged plan.
struct GroupPlans(Vec<EffectPlan>);

/// Plans the effect stage of `command` for the selected input.
///
/// The plan of a mixed input merges the plans of the groups: changes, notes,
/// irreversible effects and preview records follow the order of the groups.
///
/// # Errors
///
/// Fails with the error of the planning of a group; nothing was changed.
pub(crate) fn plan_effect(
    selection: Selection<'_>,
    command: &str,
    args: &BoundArgs,
    ctx: &ExecCtx,
) -> Result<(Effect, EffectPlan)> {
    match selection {
        Selection::One(implementation, input) => {
            let input = guard_kinds(input, command, &implementation.descriptor().input);
            let plan = effect_interface(&implementation, command)?.plan(args, input, ctx)?;
            let effect = Effect {
                command: command.to_owned(),
                implementations: vec![implementation],
            };
            Ok((effect, plan))
        }
        Selection::Split(split) => {
            let mut implementations = Vec::new();
            let mut plans = Vec::new();
            let mut merged = EffectPlan::new();
            for (implementation, input) in split.into_inputs() {
                let plan = effect_interface(&implementation, command)?.plan(args, input, ctx)?;
                merged.changes.extend(plan.changes.iter().cloned());
                merged.notable.extend(plan.notable.iter().cloned());
                merged
                    .irreversible
                    .extend(plan.irreversible.iter().cloned());
                merged.preview.extend(plan.preview.iter().cloned());
                merged.bulk |= plan.bulk;
                implementations.push(implementation);
                plans.push(plan);
            }
            merged.bulk |= merged.changes.len() > 1;
            let effect = Effect {
                command: command.to_owned(),
                implementations,
            };
            Ok((effect, merged.with_state(GroupPlans(plans))))
        }
    }
}

impl Effect {
    /// Applies the planned effect through `tx`.
    ///
    /// # Errors
    ///
    /// Fails with the error of the first group that failed; the caller rolls
    /// the transaction back.
    pub(crate) fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        if let [implementation] = self.implementations.as_slice() {
            return effect_interface(implementation, &self.command)?.apply(args, plan, tx, ctx);
        }
        let GroupPlans(plans) = plan
            .take_state::<GroupPlans>()
            .ok_or_else(|| internal("the plan of a mixed effect has no group plans"))?;
        let turns = Turns::new(self.implementations.len());
        let shared = Mutex::new(tx);
        let results = thread::scope(|scope| {
            let handles: Vec<_> = self
                .implementations
                .iter()
                .zip(plans)
                .enumerate()
                .map(|(group, (implementation, plan))| {
                    let (turns, shared, command) = (&turns, &shared, self.command.as_str());
                    scope.spawn(move || {
                        let mut done = Finish::new(turns, group);
                        let result = effect_interface(implementation, command).and_then(|effect| {
                            let mut proxy = GroupTx {
                                turns,
                                group,
                                shared,
                                cleared: false,
                            };
                            effect.apply(args, plan, &mut proxy, ctx)
                        });
                        done.finish(result.is_err());
                        result
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(internal("a stage of a mixed effect panicked")))
                })
                .collect::<Vec<_>>()
        });
        merge(results, turns.first_failure())
    }
}

fn merge<'a>(
    results: Vec<Result<EffectOutcome<'a>>>,
    failed: Option<usize>,
) -> Result<EffectOutcome<'a>> {
    let mut outcomes = Vec::new();
    let mut primary = None;
    let mut secondary = None;
    for (group, result) in results.into_iter().enumerate() {
        match result {
            Ok(outcome) => outcomes.push(outcome),
            Err(error) if Some(group) == failed => primary = Some(error),
            Err(error) => secondary = secondary.or(Some(error)),
        }
    }
    if let Some(error) = primary.or(secondary) {
        return Err(error);
    }
    let mut streams = Vec::new();
    let mut header = None;
    let mut merged = EffectOutcome::new(StageOutput::none());
    for outcome in outcomes {
        let EffectOutcome {
            output,
            changes,
            notable,
            irreversible,
            detached,
        } = outcome;
        header = header.or(output.header);
        if let StageData::Records(stream) = output.data {
            streams.push(stream);
        }
        merged.changes.extend(changes);
        merged.notable.extend(notable);
        merged.irreversible.extend(irreversible);
        merged.detached |= detached;
    }
    merged.output = StageOutput::records(RecordStream::new(streams.into_iter().flatten()));
    merged.output.header = header;
    Ok(merged)
}

/// Progress of a group through the protocol of the transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Progress {
    /// The group declares and preserves its targets.
    Preserving,
    /// The group changes its targets, or waits for its turn to do so.
    Changing,
    /// The group returned.
    Done,
}

struct TurnState {
    progress: Vec<Progress>,
    failed: Option<usize>,
}

/// Order of the groups in the protocol: a group preserves after the groups
/// before it reached the first change, and changes after every group has
/// preserved and the groups before it are done.
struct Turns {
    state: Mutex<TurnState>,
    changed: Condvar,
}

impl Turns {
    fn new(groups: usize) -> Self {
        Self {
            state: Mutex::new(TurnState {
                progress: vec![Progress::Preserving; groups],
                failed: None,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TurnState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, group: usize, progress: Progress, failed: bool) {
        let mut state = self.lock();
        if let Some(slot) = state.progress.get_mut(group) {
            *slot = progress;
        }
        if failed && state.failed.is_none() {
            state.failed = Some(group);
        }
        drop(state);
        self.changed.notify_all();
    }

    fn first_failure(&self) -> Option<usize> {
        self.lock().failed
    }

    fn wait_until(&self, ready: impl Fn(&[Progress]) -> bool) -> Result<()> {
        let mut state = self.lock();
        loop {
            if state.failed.is_some() {
                return Err(Error::new(
                    ErrorCode::Cancelled,
                    "another group of the operation failed",
                ));
            }
            if ready(&state.progress) {
                return Ok(());
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Marks a group as done when its thread ends, also by a panic.
struct Finish<'t> {
    turns: &'t Turns,
    group: usize,
    finished: bool,
}

impl<'t> Finish<'t> {
    fn new(turns: &'t Turns, group: usize) -> Self {
        Self {
            turns,
            group,
            finished: false,
        }
    }

    fn finish(&mut self, failed: bool) {
        self.finished = true;
        self.turns.set(self.group, Progress::Done, failed);
    }
}

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.turns.set(self.group, Progress::Done, true);
        }
    }
}

/// The transaction as seen by one group: calls of all groups reach the one
/// transaction in the order of [`Turns`].
struct GroupTx<'t, 'x> {
    turns: &'t Turns,
    group: usize,
    shared: &'t Mutex<&'x mut dyn MutationTx>,
    cleared: bool,
}

impl GroupTx<'_, '_> {
    fn with<T>(&self, call: impl FnOnce(&mut dyn MutationTx) -> T) -> T {
        let mut tx = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        call(&mut **tx)
    }

    fn before_preserve(&self) -> Result<()> {
        let group = self.group;
        self.turns.wait_until(|progress| {
            progress
                .iter()
                .take(group)
                .all(|state| *state >= Progress::Changing)
        })
    }

    fn before_change(&mut self) -> Result<()> {
        if self.cleared {
            return Ok(());
        }
        let group = self.group;
        self.turns.set(group, Progress::Changing, false);
        self.turns.wait_until(|progress| {
            progress.iter().all(|state| *state >= Progress::Changing)
                && progress
                    .iter()
                    .take(group)
                    .all(|state| *state == Progress::Done)
        })?;
        self.cleared = true;
        Ok(())
    }
}

impl MutationTx for GroupTx<'_, '_> {
    fn op(&self) -> Option<OpId> {
        self.with(|tx| tx.op())
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        self.before_preserve()?;
        self.with(|tx| tx.preserve(targets))
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        self.before_change()?;
        self.with(|tx| tx.stage(target))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.before_change()?;
        self.with(|tx| tx.apply(target, mutation))
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.before_change()?;
        self.with(|tx| tx.observe_external(changes))
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.with(|tx| tx.mark_irreversible(marker));
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.with(|tx| tx.record_exempt(write));
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Err(at_step(
            internal("a stage must not commit the operation"),
            ExecStep::Apply,
        ))
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Err(at_step(
            internal("a stage must not roll back the operation"),
            ExecStep::Apply,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_ordered_by_protocol_phase() {
        assert!(Progress::Preserving < Progress::Changing);
        assert!(Progress::Changing < Progress::Done);
    }

    #[test]
    fn waiting_ends_with_an_error_when_a_group_failed() {
        let turns = Turns::new(2);
        turns.set(1, Progress::Done, true);
        let error = turns
            .wait_until(|progress| progress.iter().all(|state| *state == Progress::Done))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert_eq!(turns.first_failure(), Some(1));
    }

    #[test]
    fn dropped_guard_marks_the_group_failed() {
        let turns = Turns::new(1);
        drop(Finish::new(&turns, 0));
        assert_eq!(turns.first_failure(), Some(0));
    }

    #[test]
    fn waiting_returns_at_once_when_the_condition_holds() {
        let turns = Turns::new(2);
        turns
            .wait_until(|progress| progress.iter().all(|state| *state == Progress::Preserving))
            .unwrap();
        assert_eq!(turns.first_failure(), None);
    }

    #[test]
    fn merge_joins_the_changes_and_flags_of_all_groups() {
        use nurt_contracts::record::{Change, ChangeAction};

        let mut first = EffectOutcome::new(StageOutput::none());
        first.changes.push(Change::new("/a", ChangeAction::Modify));
        let mut second = EffectOutcome::new(StageOutput::none());
        second.changes.push(Change::new("/b", ChangeAction::Delete));
        second.detached = true;
        let merged = merge(vec![Ok(first), Ok(second)], None).unwrap();
        assert_eq!(merged.changes.len(), 2);
        assert_eq!(merged.changes[1].path.as_str(), "/b");
        assert!(merged.detached);
    }

    #[test]
    fn merge_prefers_the_error_of_the_first_failed_group() {
        let results: Vec<Result<EffectOutcome<'_>>> = vec![
            Err(Error::new(ErrorCode::Cancelled, "aborted")),
            Err(Error::new(ErrorCode::Io, "failed")),
        ];
        assert_eq!(merge(results, Some(1)).unwrap_err().code(), ErrorCode::Io);
        let outcome = merge(vec![Ok(EffectOutcome::new(StageOutput::none()))], None).unwrap();
        assert!(!outcome.detached);
    }
}
