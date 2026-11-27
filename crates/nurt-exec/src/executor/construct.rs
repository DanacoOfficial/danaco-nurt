// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::executor::construct
// Responsibility:   Execution of the control constructs: conditions, loops, `steps` and functions.
// Dependencies:     nurt_contracts, nurt_plan, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution of the control constructs: conditions, loops, `steps` and
//! functions.
//!
//! A construct that changes the world is one journal entry. The executor
//! opens one transaction for it, every effect of its parts is applied in that
//! transaction, and one commit closes it. A failure of any part rolls the
//! transaction back, so the state after equals the state before, except for
//! the parts the catalog marks irreversible, which the report lists. A
//! construct that only reads opens no transaction. In a preview every part
//! is planned and nothing is applied.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use nurt_contracts::bound::{BoundCall, BoundExpr, BoundLimit, BoundParam, BoundStatement};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{
    BeginRequest, MutationTx, OpHandle, OpKind, RollbackReason, SessionId,
};
use nurt_contracts::kinds::{EffectKind, Reversibility};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, ReportPart, ReportStatus};
use nurt_contracts::stage::{ExecStep, StageInput};
use nurt_contracts::value::{Fields, Record, RecordKind, Value};
use nurt_plan::{
    ConstructPlan, ConstructShape, LetPlan, OperationPlan, PipelinePlan, PlanStatement,
};
use serde_json::json;

use super::{Execution, FunctionDef, RunState};
use crate::effect::target_text;
use crate::flow::{at_step, internal};
use crate::outcome::{ConstructOutcome, EffectReport, OperationOutcome, Outcome, ResultData};

/// State and outcome of one iteration that ran on its own thread.
type Finished = (RunState, Result<()>);

/// What a block asks of the loop around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Next,
    Break,
    Continue,
}

const EXIT_CODE: &str = "exit_code";
const SUB_PREFIX: &str = "~sub";
const POLL: Duration = Duration::from_millis(50);

/// What the entry of a construct left behind.
struct Settled {
    handle: Option<OpHandle>,
    failure: Option<Error>,
    rolled: Option<Vec<String>>,
    not_restored: Vec<String>,
    state: RunState,
    opened: bool,
    deferred: Option<Error>,
}

impl<'e> Execution<'e> {
    /// Runs a control construct at the top of the program.
    pub(super) fn construct(&self, plan: &'e ConstructPlan, outcome: &mut Outcome) {
        match &plan.shape {
            ConstructShape::Define { .. } => {
                self.register(plan);
                return;
            }
            ConstructShape::Params { .. } => return,
            _ => {}
        }
        let operation = self.entry(
            plan.name(),
            &plan.program,
            plan.changes(),
            plan.index,
            announce_construct(plan),
            || self.run_construct(plan).map(drop),
        );
        outcome.push(operation);
    }

    /// Runs a pipeline that starts with a call of a function at the top of
    /// the program: the call is one operation, one journal entry.
    pub(super) fn call_operation(&self, operation: &'e OperationPlan) -> OperationOutcome {
        let program = bound_pipeline(&operation.pipeline);
        let name = operation
            .pipeline
            .call
            .as_ref()
            .map_or("call", |call| call.name.as_str());
        let changes = operation.pipeline.effect != nurt_contracts::kinds::EffectKind::Read;
        let mut result = ResultData::default();
        let announced = announce_pipeline(&operation.pipeline);
        let mut finished = self.entry(name, &program, changes, operation.index, announced, || {
            let outcome = self.operation(operation);
            result = outcome.result.clone();
            match outcome.error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        });
        if finished.error.is_none() {
            finished.result = result;
        }
        finished
    }

    /// Runs a `let` whose pipeline starts with a call of a function. A call
    /// that changes the world is one journal entry, like any construct;
    /// `None` when it failed, with the failure in the outcome.
    pub(super) fn call_let(&self, binding: &'e LetPlan, outcome: &mut Outcome) -> Option<Value> {
        let changes = binding.pipeline.effect != nurt_contracts::kinds::EffectKind::Read;
        if !changes {
            return match self.snapshot(&binding.pipeline) {
                Ok(value) => Some(value),
                Err(error) => {
                    outcome.fail(error);
                    None
                }
            };
        }
        let program = bound_pipeline(&binding.pipeline);
        let name = binding
            .pipeline
            .call
            .as_ref()
            .map_or("call", |call| call.name.as_str());
        let mut value = None;
        let announced = announce_pipeline(&binding.pipeline);
        let operation = self.entry(name, &program, true, u32::MAX, announced, || {
            value = Some(self.snapshot(&binding.pipeline)?);
            Ok(())
        });
        let failed = operation.error.is_some();
        outcome.push(operation);
        if failed { None } else { value }
    }

    fn register(&self, plan: &'e ConstructPlan) {
        if let ConstructShape::Define { name, params, body } = &plan.shape {
            self.functions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(name.clone(), FunctionDef { params, body });
        }
    }

    /// Runs `body` as one journal entry when it changes the world and as a
    /// plain run when it only reads, previews or already is part of an entry.
    fn entry(
        &self,
        name: &str,
        program: &BoundStatement,
        changes: bool,
        index: u32,
        announced: Vec<IrreversibleEntry>,
        body: impl FnOnce() -> Result<()>,
    ) -> OperationOutcome {
        let mut operation = OperationOutcome::new(index);
        let nested = self
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        let opened = changes && !self.preview && !nested;
        let outer = self.take_state();
        if opened {
            let Some(journal) = self.env.journal.as_deref() else {
                self.restore_state(outer);
                operation.fail(
                    Error::new(
                        ErrorCode::StageFailed,
                        format!("`{name}` needs the journal to change the world"),
                    )
                    .with_stage(ExecStep::Apply),
                );
                return operation;
            };
            let source = self.program();
            let request = BeginRequest {
                kind: OpKind::command(name),
                session: self.ctx.options().session.clone().map(SessionId),
                program: source.construct_document(program),
                corrections: self.plan.corrections.clone(),
                bulk: true,
                counts_as_step: true,
                preserve_budget: Some(self.preserve_budget),
            };
            let mut tx = journal.begin(request);
            tx.allow_incremental();
            *self.shared.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        }
        self.restore_state(RunState {
            path: outer.path.clone(),
            depth: outer.depth,
            exit_code: outer.exit_code,
            announced,
            ..RunState::default()
        });
        let mut failure = body().err();
        let mut handle: Option<OpHandle> = None;
        let mut rolled: Option<Vec<String>> = None;
        let mut not_restored: Vec<String> = Vec::new();
        if opened {
            let tx = self
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(tx) = tx {
                match failure.take() {
                    None => match tx.commit() {
                        Ok(done) => handle = Some(done),
                        Err(error) => failure = Some(at_step(error, ExecStep::Apply)),
                    },
                    Some(cause) => {
                        let reason = if self.ctx.cancel().is_cancelled() {
                            RollbackReason::Cancelled
                        } else {
                            RollbackReason::TargetFailed
                        };
                        match tx.rollback(reason) {
                            Ok(report) => {
                                rolled = Some(report.restored.iter().map(target_text).collect());
                                not_restored = report.failed.iter().map(target_text).collect();
                                failure = Some(cause);
                            }
                            Err(error) => {
                                failure = Some(at_step(error, ExecStep::Apply).with_cause(cause));
                            }
                        }
                    }
                }
            }
        }
        let mut finished = self.take_state();
        let deferred = finished.deferred.take();
        let mut parts = std::mem::take(&mut finished.parts);
        let mut kept = outer;
        kept.last_value = finished.last_value.clone();
        kept.exit_code = finished.exit_code;
        kept.completed = finished.completed.or(kept.completed);
        kept.handle = finished.handle.clone().or(kept.handle.take());
        self.restore_state(kept);
        self.settle(
            &mut operation,
            name,
            &mut parts,
            Settled {
                handle,
                failure,
                rolled,
                not_restored,
                state: finished,
                opened,
                deferred,
            },
        );
        operation
    }

    /// Writes the outcome of an entry into the operation.
    fn settle(
        &self,
        operation: &mut OperationOutcome,
        name: &str,
        parts: &mut Vec<ReportPart>,
        settled: Settled,
    ) {
        let Settled {
            handle,
            failure,
            rolled,
            not_restored,
            state,
            opened,
            deferred,
        } = settled;
        let RunState {
            changes,
            notable,
            mut irreversible,
            announced,
            last,
            handle: kept_handle,
            completed,
            ..
        } = state;
        let handle = handle.or(kept_handle);
        // The parts the plan marks irreversible are listed whether or not they
        // ran; the entries the parts reported are added.
        let mut listed = announced;
        for entry in irreversible.drain(..) {
            if !listed_has(&listed, &entry) {
                listed.push(entry);
            }
        }
        let irreversible = listed;
        let changed = !changes.is_empty() || !irreversible.is_empty() || handle.is_some();
        operation.result = last;
        operation.notable = notable;
        if changed {
            operation.effect = Some(EffectReport {
                command: name.to_owned(),
                applied: opened && handle.is_some() && rolled.is_none() && !self.preview,
                changes,
                notable: Vec::new(),
                irreversible: irreversible.clone(),
                preview: Vec::new(),
                bulk: true,
                detached: false,
                handle,
                rollback: None,
            });
        }
        if self.preview {
            operation.status = ReportStatus::Planned;
        }
        if failure.is_none()
            && let Some(error) = deferred
        {
            let kept = handle_op(&operation.effect);
            operation.status = if kept && completed.is_some_and(|done| done > 0) {
                ReportStatus::Partial
            } else {
                ReportStatus::Failed
            };
            operation.error = Some(error);
        }
        if let Some(error) = failure {
            match rolled {
                Some(restored) => {
                    for part in parts.iter_mut() {
                        if part.kind == "step" || part.kind == "iteration" {
                            part.rolled_back = !part.irreversible;
                        }
                    }
                    operation.status = if not_restored.is_empty() {
                        ReportStatus::RolledBack
                    } else {
                        ReportStatus::Failed
                    };
                    operation.error = Some(construct_rolled_back(
                        error,
                        &restored,
                        &not_restored,
                        &irreversible,
                    ));
                }
                None => {
                    operation.status = if completed.is_some_and(|done| done > 0) {
                        ReportStatus::Partial
                    } else {
                        ReportStatus::Failed
                    };
                    operation.error = Some(error);
                }
            }
        }
        if !parts.is_empty() {
            operation.construct = Some(ConstructOutcome {
                name: name.to_owned(),
                parts: std::mem::take(parts),
            });
        }
    }

    fn take_state(&self) -> RunState {
        std::mem::take(&mut *self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn restore_state(&self, state: RunState) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }

    fn with_state<T>(&self, change: impl FnOnce(&mut RunState) -> T) -> T {
        change(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn push_part(&self, part: ReportPart) {
        self.with_state(|state| state.parts.push(part));
    }

    fn at(&self, tail: &str) -> String {
        self.with_state(|state| {
            let mut at = state.path.join(".");
            if !at.is_empty() && !tail.is_empty() {
                at.push('.');
            }
            at.push_str(tail);
            at
        })
    }

    fn enter_path(&self, segment: impl Into<String>) {
        self.with_state(|state| state.path.push(segment.into()));
    }

    fn leave_path(&self) {
        self.with_state(|state| {
            state.path.pop();
        });
    }

    /// Runs the construct and returns what it asks of the loop around it.
    pub(super) fn run_construct(&self, plan: &'e ConstructPlan) -> Result<Control> {
        match &plan.shape {
            ConstructShape::When {
                branches,
                otherwise,
            } => self.run_when(branches, otherwise.as_deref()),
            ConstructShape::Each {
                source,
                name,
                parallel,
                atomic,
                body,
            } => self.run_each(source, name, *parallel, *atomic, body),
            ConstructShape::Repeat {
                until,
                every_ms,
                limit,
                atomic,
                body,
            } => self.run_repeat(until, *every_ms, *limit, *atomic, body),
            ConstructShape::While {
                cond,
                limit,
                atomic,
                body,
            } => self.run_while(cond, *limit, *atomic, body),
            ConstructShape::Steps { body } => {
                self.enter_path("steps");
                let control = self.run_block(body);
                self.leave_path();
                control
            }
            ConstructShape::Define { .. } => {
                self.register(plan);
                Ok(Control::Next)
            }
            ConstructShape::Params { .. } => Ok(Control::Next),
            ConstructShape::Break => Ok(Control::Break),
            ConstructShape::Continue => Ok(Control::Continue),
            _ => Err(internal("the plan holds a construct of an unknown kind")),
        }
    }

    fn run_when(
        &self,
        branches: &'e [nurt_plan::BranchPlan],
        otherwise: Option<&'e [PlanStatement]>,
    ) -> Result<Control> {
        for (index, branch) in branches.iter().enumerate() {
            if self.evaluate_condition(&branch.cond)? {
                let number = u64::try_from(index + 1).unwrap_or(u64::MAX);
                let mut part = ReportPart::done(self.at("when"), "branch");
                part.condition = Some(true);
                part.branch = Some(number);
                self.push_part(part);
                self.enter_path(format!("when.branch[{number}]"));
                let control = self.run_block(&branch.body);
                self.leave_path();
                return control;
            }
        }
        let mut part = ReportPart::done(self.at("when"), "branch");
        match otherwise {
            Some(body) => {
                let number = u64::try_from(branches.len() + 1).unwrap_or(u64::MAX);
                part.branch = Some(number);
                self.push_part(part);
                self.enter_path(format!("when.branch[{number}]"));
                let control = self.run_block(body);
                self.leave_path();
                control
            }
            None => {
                part.condition = Some(false);
                self.push_part(part);
                Ok(Control::Next)
            }
        }
    }

    fn run_each(
        &self,
        source: &BoundExpr,
        name: &str,
        parallel: Option<u32>,
        atomic: bool,
        body: &'e [PlanStatement],
    ) -> Result<Control> {
        let items = self.evaluate_sequence(source)?;
        let mut note = None;
        if let Some(workers) = parallel.filter(|workers| *workers > 1 && atomic) {
            match self.try_parallel(name, &items, workers, body)? {
                Some(count) => {
                    let mut part = ReportPart::done(self.at("each"), "loop");
                    part.iterations = Some(count);
                    part.note = Some(format!("ran up to {workers} iterations at a time"));
                    self.push_part(part);
                    return Ok(Control::Next);
                }
                None => {
                    note = Some(
                        "parallel: the targets of the iterations overlap or are unknown, ran in order"
                            .to_owned(),
                    );
                }
            }
        } else if parallel.is_some_and(|workers| workers > 1) {
            note = Some("parallel: atomic: false runs in order".to_owned());
        }
        let mut count: u64 = 0;
        for item in items {
            self.checkpoint()?;
            count += 1;
            let outer = self.binding.names();
            self.binding.define(name.to_owned(), item);
            self.enter_path(format!("each.iteration[{count}]"));
            let control = self.iteration(atomic, count, body);
            self.leave_path();
            self.binding.restore_names(outer);
            if control? == Control::Break {
                break;
            }
        }
        let mut part = ReportPart::done(self.at("each"), "loop");
        part.iterations = Some(count);
        part.note = note;
        self.push_part(part);
        Ok(Control::Next)
    }

    /// Runs the iterations of an `each` concurrently, at most `workers` at a
    /// time, when their targets are disjoint; `None` when they are not or
    /// cannot be known, and the caller runs them in order. The targets are
    /// found before the start by planning every iteration without applying
    /// it. The effects of all iterations are applied in the one transaction
    /// of the loop.
    fn try_parallel(
        &self,
        name: &str,
        items: &[Value],
        workers: u32,
        body: &'e [PlanStatement],
    ) -> Result<Option<u64>> {
        if self.preview || items.len() < 2 || contains_jump(body) {
            return Ok(None);
        }
        let mut claimed: Vec<(String, usize)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let probe = self.fork(true)?;
            probe.binding.define(name.to_owned(), item.clone());
            if probe.run_block(body).is_err() {
                return Ok(None);
            }
            let state = probe.take_state();
            if !state.irreversible.is_empty() || state.exit_code.is_some() {
                return Ok(None);
            }
            for change in &state.changes {
                claimed.push((change.change.path.to_string(), index));
            }
        }
        if overlapping(&claimed) {
            return Ok(None);
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let failed = std::sync::atomic::AtomicBool::new(false);
        let results: Mutex<Vec<Option<Finished>>> =
            Mutex::new((0..items.len()).map(|_| None).collect());
        let width = usize::try_from(workers).unwrap_or(1).min(items.len());
        let started: Result<()> = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..width {
                let worker = || -> Result<()> {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if index >= items.len() || failed.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            return Ok(());
                        }
                        let fork = self.fork(false)?;
                        fork.binding.define(name.to_owned(), items[index].clone());
                        fork.enter_path(format!("each.iteration[{}]", index + 1));
                        let outcome = fork
                            .checkpoint()
                            .and_then(|()| fork.run_block(body).map(drop));
                        if outcome.is_err() {
                            failed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        let state = fork.take_state();
                        results.lock().unwrap_or_else(PoisonError::into_inner)[index] =
                            Some((state, outcome));
                    }
                };
                handles.push(scope.spawn(worker));
            }
            let mut first = Ok(());
            for handle in handles {
                match handle.join() {
                    Ok(Err(error)) if first.is_ok() => first = Err(error),
                    Err(_) if first.is_ok() => first = Err(internal("an iteration panicked")),
                    _ => {}
                }
            }
            first
        });
        let mut count = 0;
        let mut failure = started.err();
        for (index, slot) in results
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .enumerate()
        {
            let Some((state, outcome)) = slot else {
                continue;
            };
            count += 1;
            self.with_state(|mine| {
                mine.parts.extend(state.parts);
                mine.changes.extend(state.changes);
                mine.notable.extend(state.notable);
                mine.irreversible.extend(state.irreversible);
                mine.last = state.last;
            });
            if let Err(error) = outcome
                && failure.is_none()
            {
                failure = Some(error.with_detail("iteration", json!(index + 1)));
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(Some(count)),
        }
    }

    fn checkpoint(&self) -> Result<()> {
        self.ctx
            .checkpoint()
            .map_err(|error| at_step(error, ExecStep::Read))
    }

    /// Runs the body of one iteration. In a loop with `atomic: false` the
    /// iteration sets a savepoint of the transaction of the loop: when it
    /// fails, only its own changes are restored, the loop ends, and the
    /// iterations completed before stay in the one entry, which reports the
    /// failure.
    fn iteration(&self, atomic: bool, number: u64, body: &'e [PlanStatement]) -> Result<Control> {
        let kept = !atomic && !self.preview && self.with_tx(|tx| tx.savepoint()).is_some();
        if !kept {
            return self.run_block(body);
        }
        match self.run_block(body) {
            Ok(control) => {
                self.with_tx(|tx| tx.release_savepoint());
                self.with_state(|state| state.completed = Some(number));
                // A nested loop that failed has ended the whole construct.
                let failed = self.with_state(|state| state.deferred.is_some());
                Ok(if failed { Control::Break } else { control })
            }
            Err(error) => {
                if let Some(Err(failure)) = self.with_tx(|tx| tx.rollback_to_savepoint()) {
                    return Err(failure.with_cause(error));
                }
                let mut part = ReportPart::done(self.at(""), "iteration");
                part.ok = false;
                part.rolled_back = true;
                part.error = Some(error.code().as_str().to_owned());
                self.push_part(part);
                self.with_state(|state| {
                    state.completed = Some(number - 1);
                    state.deferred = Some(error.with_detail("iteration", json!(number)));
                });
                Ok(Control::Break)
            }
        }
    }

    /// Runs `change` on the transaction of the construct, when there is one.
    fn with_tx<T>(&self, change: impl FnOnce(&mut dyn MutationTx) -> T) -> Option<T> {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared.as_mut().map(|tx| change(tx.as_mut()))
    }

    /// Copies the place of the construct of `parent` into this fork.
    pub(super) fn with_state_path(&self, parent: &Self) {
        let path = parent.with_state(|state| state.path.clone());
        self.with_state(|state| state.path = path);
    }

    fn loop_limits(&self, limit: Option<BoundLimit>) -> (u64, Duration) {
        let config = &self.env.config.loop_limits;
        let mut iterations = config.max_iterations;
        let mut time = Duration::from_millis(config.max_time.as_millis());
        // An explicit limit replaces both defaults: the other dimension is
        // then not limited by the configuration.
        match limit {
            Some(BoundLimit::Iterations { n }) => {
                iterations = n;
                time = Duration::MAX;
            }
            Some(BoundLimit::Time { ms }) => {
                time = Duration::from_millis(ms);
                iterations = u64::MAX;
            }
            _ => {}
        }
        (iterations, time)
    }

    fn run_repeat(
        &self,
        until: &BoundExpr,
        every_ms: Option<u64>,
        limit: Option<BoundLimit>,
        atomic: bool,
        body: &'e [PlanStatement],
    ) -> Result<Control> {
        let (max_iterations, max_time) = self.loop_limits(limit);
        let started = self.ctx.clock().monotonic();
        let mut count: u64 = 0;
        loop {
            self.checkpoint()?;
            if count >= max_iterations || self.elapsed(started) >= max_time {
                return Err(limit_error("repeat", count, "the condition was never met"));
            }
            count += 1;
            self.enter_path(format!("repeat.iteration[{count}]"));
            let control = self.iteration(atomic, count, body);
            self.leave_path();
            if control? == Control::Break {
                break;
            }
            if self.preview || self.evaluate_condition(until)? {
                break;
            }
            if let Some(interval) = every_ms {
                let left = max_time.saturating_sub(self.elapsed(started));
                self.pause(Duration::from_millis(interval).min(left))?;
            }
        }
        let mut part = ReportPart::done(self.at("repeat"), "loop");
        part.iterations = Some(count);
        if self.preview {
            part.note = Some("preview plans one iteration".to_owned());
        }
        self.push_part(part);
        Ok(Control::Next)
    }

    fn run_while(
        &self,
        cond: &BoundExpr,
        limit: Option<BoundLimit>,
        atomic: bool,
        body: &'e [PlanStatement],
    ) -> Result<Control> {
        let (max_iterations, max_time) = self.loop_limits(limit);
        let started = self.ctx.clock().monotonic();
        let mut count: u64 = 0;
        let mut note = None;
        loop {
            self.checkpoint()?;
            if count >= max_iterations || self.elapsed(started) >= max_time {
                if self.evaluate_condition(cond)? {
                    note =
                        Some("the limit ended the loop while the condition still held".to_owned());
                }
                break;
            }
            if !self.evaluate_condition(cond)? {
                break;
            }
            count += 1;
            self.enter_path(format!("while.iteration[{count}]"));
            let control = self.iteration(atomic, count, body);
            self.leave_path();
            if control? == Control::Break {
                break;
            }
            if self.preview {
                note = Some("preview plans one iteration".to_owned());
                break;
            }
        }
        let mut part = ReportPart::done(self.at("while"), "loop");
        part.iterations = Some(count);
        part.note = note;
        self.push_part(part);
        Ok(Control::Next)
    }

    fn elapsed(&self, since: Duration) -> Duration {
        self.ctx.clock().monotonic().saturating_sub(since)
    }

    fn pause(&self, duration: Duration) -> Result<()> {
        let mut left = duration;
        while !left.is_zero() {
            let slice = left.min(POLL);
            if self.ctx.clock().sleep(slice, self.ctx.cancel()).is_err() {
                return Err(match self.ctx.checkpoint() {
                    Err(error) => at_step(error, ExecStep::Read),
                    Ok(()) => internal("the pause was interrupted"),
                });
            }
            left = left.saturating_sub(slice);
        }
        Ok(())
    }

    /// Runs statements in a scope of their own: names defined inside are
    /// gone after the block.
    pub(super) fn run_block(&self, body: &'e [PlanStatement]) -> Result<Control> {
        let outer = self.binding.names();
        let control = self.run_statements(body);
        self.binding.restore_names(outer);
        control
    }

    fn run_statements(&self, body: &'e [PlanStatement]) -> Result<Control> {
        for (index, statement) in body.iter().enumerate() {
            self.checkpoint()?;
            match statement {
                PlanStatement::Let(binding) if binding.pipeline.effect_stage().is_some() => {
                    let outcome = self.operation_of(0, &binding.pipeline);
                    let value = self.record_step(&binding.pipeline, &outcome, index + 1)?;
                    self.binding.define(binding.name.clone(), value);
                }
                PlanStatement::Let(binding) => {
                    let value = self.snapshot(&binding.pipeline)?;
                    self.binding.define(binding.name.clone(), value.clone());
                    self.with_state(|state| state.last_value = Some(value));
                }
                PlanStatement::Operation(operation) => self.step(operation, index + 1)?,
                PlanStatement::Construct(inner) => match self.run_construct(inner)? {
                    Control::Next => {}
                    other => return Ok(other),
                },
                _ => return Err(internal("the plan holds a statement of an unknown kind")),
            }
        }
        Ok(Control::Next)
    }

    /// Runs one pipeline of a construct and records its effects.
    fn step(&self, operation: &'e OperationPlan, number: usize) -> Result<()> {
        let outcome = self.operation(operation);
        self.record_step(&operation.pipeline, &outcome, number)
            .map(drop)
    }

    /// Records the part of a step and its effects; returns the value the step
    /// gives to a `let`: the only record of its result, or the list.
    fn record_step(
        &self,
        pipeline: &PipelinePlan,
        outcome: &OperationOutcome,
        number: usize,
    ) -> Result<Value> {
        let command = pipeline
            .effect_stage()
            .map(|stage| stage.command().to_owned())
            .or_else(|| pipeline.call.as_ref().map(|call| call.name.clone()))
            .or_else(|| {
                pipeline
                    .stages
                    .first()
                    .map(|stage| stage.command().to_owned())
            });
        let mut part = ReportPart::done(self.at(&format!("step[{number}]")), "step");
        part.command = command;
        // A stage written with `preview` only plans: its changes are not part
        // of the effects of the construct.
        let planned = outcome.status == ReportStatus::Planned && !self.preview;
        if let Some(effect) = &outcome.effect {
            part.targets = Some(u64::try_from(effect.changes.len()).unwrap_or(u64::MAX));
            part.irreversible = !effect.irreversible.is_empty();
        }
        part.irreversible |= !announce_pipeline(pipeline).is_empty();
        if planned {
            part.note = Some("planned, not applied".to_owned());
        }
        let exit = exit_code(&outcome.result);
        let failure = outcome.error.clone();
        self.with_state(|state| {
            if let Some(effect) = outcome.effect.as_ref().filter(|_| !planned) {
                state.changes.extend(effect.changes.clone());
                state.irreversible.extend(effect.irreversible.clone());
                state.notable.extend(effect.notable.clone());
            }
            state.notable.extend(outcome.notable.clone());
            if let Some(code) = exit {
                state.exit_code = Some(code);
            }
            state.last_value = Some(outcome.result.to_value());
            state.last = outcome.result.clone();
        });
        if let Some(error) = &failure {
            part.ok = false;
            part.error = Some(error.code().as_str().to_owned());
        }
        self.push_part(part);
        match failure {
            Some(error) => Err(error),
            None => Ok(match outcome.result.records.as_slice() {
                [record] if outcome.result.value.is_none() => Value::Record(record.clone()),
                _ => outcome.result.to_value(),
            }),
        }
    }

    /// Calls a defined function with the arguments of the call and returns
    /// the value of its last statement.
    pub(super) fn call_function(&self, call: &BoundCall) -> Result<Value> {
        let definition = self
            .functions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&call.name)
            .copied()
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::UnknownName,
                    format!("the function `{}` is not defined", call.name),
                )
                .with_stage(ExecStep::Plan)
            })?;
        let max_depth = u64::from(self.env.config.function.max_depth);
        let depth = self.with_state(|state| {
            state.depth += 1;
            state.depth
        });
        let result = if u64::from(depth) > max_depth {
            Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "the function `{}` is nested deeper than function.max_depth ({max_depth})",
                    call.name
                ),
            )
            .with_stage(ExecStep::Plan)
            .with_detail("depth", json!(depth)))
        } else {
            self.invoke(call, definition)
        };
        self.with_state(|state| state.depth -= 1);
        result
    }

    fn invoke(&self, call: &BoundCall, definition: FunctionDef<'e>) -> Result<Value> {
        let mut names = Fields::new();
        for param in definition.params {
            let value = match call.args.iter().find(|arg| arg.name == param.name) {
                Some(arg) => self
                    .binding
                    .evaluate(&arg.value, None, self.ctx.clock())
                    .map_err(|error| at_step(error, ExecStep::Plan))?,
                None => default_of(param, &self.binding, self.ctx.clock())?,
            };
            names.insert(param.name.clone(), value);
        }
        let outer = self.binding.replace_names(names);
        let (saved, saved_exit) = self.with_state(|state| {
            state.path.push(format!("call {}", call.name));
            (state.last_value.take(), state.exit_code.take())
        });
        let control = self.run_block(definition.body);
        let value = self.with_state(|state| {
            state.path.pop();
            state.exit_code = saved_exit;
            std::mem::replace(&mut state.last_value, saved)
        });
        self.binding.restore_names(outer);
        control?;
        Ok(value.unwrap_or(Value::Null))
    }

    /// Value of a `bool` expression before a branch or an iteration.
    fn evaluate_condition(&self, expr: &BoundExpr) -> Result<bool> {
        match self.evaluate_raw(expr, false)? {
            Value::Bool(value) => Ok(value),
            other => Err(Error::new(
                ErrorCode::Type,
                format!("a condition is `bool`, found {}", other.type_name()),
            )
            .with_stage(ExecStep::Plan)),
        }
    }

    /// The items an `each` visits.
    fn evaluate_sequence(&self, expr: &BoundExpr) -> Result<Vec<Value>> {
        match self.evaluate_raw(expr, true)? {
            Value::List(items) => Ok(items),
            Value::Null => Ok(Vec::new()),
            other => Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`each` visits a list of records, found {}",
                    other.type_name()
                ),
            )
            .with_stage(ExecStep::Plan)),
        }
    }

    /// Evaluates an expression of a construct: a reading pipeline in
    /// parentheses is run and its result is the value, `exists` asks the
    /// file system, `exit_code` is the exit code of the last program run.
    fn evaluate_raw(&self, expr: &BoundExpr, whole_list: bool) -> Result<Value> {
        let outer = self.binding.names();
        let mut counter = 0usize;
        let value = self
            .resolve(expr, &mut counter, whole_list)
            .and_then(|resolved| {
                if let Some(code) = self.with_state(|state| state.exit_code) {
                    self.binding.define(EXIT_CODE.to_owned(), Value::Int(code));
                }
                self.binding
                    .evaluate(&resolved, None, self.ctx.clock())
                    .map_err(|error| at_step(error, ExecStep::Plan))
            });
        self.binding.restore_names(outer);
        value
    }

    fn resolve(
        &self,
        expr: &BoundExpr,
        counter: &mut usize,
        whole_list: bool,
    ) -> Result<BoundExpr> {
        Ok(match expr {
            BoundExpr::Sub { p } => {
                let output = self
                    .ctx
                    .pipelines()?
                    .run(p, StageInput::none(), &self.ctx)
                    .map_err(|error| at_step(error, ExecStep::Read))?;
                let data = ResultData::collect(output.into(), self.max_records())
                    .map_err(|error| at_step(error, ExecStep::Read))?;
                let value = if whole_list && *counter == 0 {
                    data.to_value()
                } else {
                    scalar_of(&data)
                };
                let name = format!("{SUB_PREFIX}{counter}");
                *counter += 1;
                self.binding.define(name.clone(), value);
                BoundExpr::Ref(nurt_contracts::bound::BoundRef {
                    name,
                    fields: Vec::new(),
                })
            }
            BoundExpr::Call { function, args } if function == "exists" => {
                let [BoundExpr::Path { v }] = args.as_slice() else {
                    return Err(internal("`exists` takes one path"));
                };
                BoundExpr::Bool { v: self.exists(v)? }
            }
            BoundExpr::Un { op, x } => BoundExpr::Un {
                op: *op,
                x: Box::new(self.resolve(x, counter, false)?),
            },
            BoundExpr::Bin { op, l, r } => BoundExpr::Bin {
                op: *op,
                l: Box::new(self.resolve(l, counter, false)?),
                r: Box::new(self.resolve(r, counter, false)?),
            },
            BoundExpr::Field { name, of: Some(of) } => BoundExpr::Field {
                name: name.clone(),
                of: Some(Box::new(self.resolve(of, counter, false)?)),
            },
            BoundExpr::Call { function, args } => BoundExpr::Call {
                function: function.clone(),
                args: args
                    .iter()
                    .map(|arg| self.resolve(arg, counter, false))
                    .collect::<Result<Vec<_>>>()?,
            },
            BoundExpr::List { items } => BoundExpr::List {
                items: items
                    .iter()
                    .map(|item| self.resolve(item, counter, false))
                    .collect::<Result<Vec<_>>>()?,
            },
            other => other.clone(),
        })
    }

    fn exists(&self, path: &str) -> Result<bool> {
        let absolute = nurt_contracts::fs::AbsPath::new(path)
            .map_err(|error| Error::new(ErrorCode::InvalidValue, error.to_string()))?;
        match self
            .env
            .fs
            .metadata(&absolute, nurt_contracts::fs::Follow::NoFollow)
        {
            Ok(_) => Ok(true),
            Err(error) if error.kind == nurt_contracts::fs::FsErrorKind::NotFound => Ok(false),
            Err(error) => Err(Error::new(ErrorCode::Io, format!("{path}: {error}"))),
        }
    }
}

/// Whether a block holds `break` or `continue` at any depth: the order of the
/// iterations then matters.
fn contains_jump(body: &[PlanStatement]) -> bool {
    body.iter().any(|statement| match statement {
        PlanStatement::Construct(construct) => {
            matches!(
                construct.shape,
                ConstructShape::Break | ConstructShape::Continue
            ) || construct.blocks().into_iter().any(contains_jump)
        }
        _ => false,
    })
}

/// Components of a path for the comparison of targets: both separators
/// split, and case is folded, so that a case-insensitive volume is treated as
/// overlapping, which is the cautious answer.
fn components(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether two different iterations claim the same target, or one claims a
/// target inside another: every path is compared with all its ancestors and
/// all its descendants among the claims of other iterations.
fn overlapping(claimed: &[(String, usize)]) -> bool {
    use std::collections::{HashMap, HashSet};
    let mut exact: HashMap<Vec<String>, HashSet<usize>> = HashMap::new();
    let mut below: HashMap<Vec<String>, HashSet<usize>> = HashMap::new();
    let parsed: Vec<(Vec<String>, usize)> = claimed
        .iter()
        .map(|(path, iteration)| (components(path), *iteration))
        .collect();
    for (path, iteration) in &parsed {
        exact.entry(path.clone()).or_default().insert(*iteration);
        for length in 0..path.len() {
            below
                .entry(path[..length].to_vec())
                .or_default()
                .insert(*iteration);
        }
    }
    let others = |set: Option<&HashSet<usize>>, iteration: usize| {
        set.is_some_and(|set| set.iter().any(|other| *other != iteration))
    };
    parsed.iter().any(|(path, iteration)| {
        others(exact.get(path), *iteration)
            || others(below.get(path), *iteration)
            || (0..path.len()).any(|length| others(exact.get(&path[..length]), *iteration))
    })
}

/// Whether `listed` has an entry of the same reason and detail as `entry`.
fn listed_has(listed: &[IrreversibleEntry], entry: &IrreversibleEntry) -> bool {
    listed
        .iter()
        .any(|known| known.reason == entry.reason && known.detail == entry.detail)
}

/// The irreversible stages of a pipeline, from its plan.
fn announce_pipeline(pipeline: &PipelinePlan) -> Vec<IrreversibleEntry> {
    let mut found: Vec<IrreversibleEntry> = pipeline
        .all_stages()
        .into_iter()
        .filter(|stage| {
            stage.reversibility == Reversibility::Irreversible
                || matches!(
                    stage.stage.effect,
                    EffectKind::Process | EffectKind::External
                )
        })
        .map(|stage| irreversible_part(stage.command()))
        .collect();
    if let Some(call) = &pipeline.call
        && pipeline.reversibility == Reversibility::Irreversible
    {
        found.push(irreversible_part(&call.name));
    }
    found
}

/// The irreversible stages of the pipelines of a construct, from its plan.
fn announce_construct(plan: &ConstructPlan) -> Vec<IrreversibleEntry> {
    let mut found: Vec<IrreversibleEntry> = Vec::new();
    for pipeline in plan.pipelines() {
        for entry in announce_pipeline(pipeline) {
            if !listed_has(&found, &entry) {
                found.push(entry);
            }
        }
    }
    found
}

fn irreversible_part(command: &str) -> IrreversibleEntry {
    let mut entry = IrreversibleEntry::new(Irreversible::ExternalEffect);
    entry.detail = Some(format!("`{command}` cannot be rolled back"));
    entry
}

/// Whether the effect of an operation holds a committed journal entry.
fn handle_op(effect: &Option<EffectReport>) -> bool {
    effect
        .as_ref()
        .and_then(|effect| effect.handle.as_ref())
        .and_then(|handle| handle.op)
        .is_some()
}

/// Value of a reading pipeline inside an expression: the only field of the
/// only record, the only record, or the list of records.
fn scalar_of(data: &ResultData) -> Value {
    if let Some(value) = &data.value {
        return value.clone();
    }
    match data.records.as_slice() {
        [record] => single_field(record),
        _ => data.to_value(),
    }
}

fn single_field(record: &Record) -> Value {
    let mut fields = record
        .fields
        .iter()
        .filter(|(_, value)| !matches!(value, Value::Null));
    match (fields.next(), fields.next()) {
        (Some((_, value)), None) => value.clone(),
        _ => Value::Record(record.clone()),
    }
}

fn exit_code(result: &ResultData) -> Option<i64> {
    result
        .records
        .iter()
        .filter(|record| record.kind == RecordKind::Exec)
        .find_map(|record| match record.get(EXIT_CODE) {
            Some(Value::Int(code)) => Some(*code),
            _ => None,
        })
}

fn default_of(
    param: &BoundParam,
    binding: &crate::eval::ExprBinding,
    clock: &dyn nurt_contracts::env::Clock,
) -> Result<Value> {
    match &param.default {
        Some(default) => binding
            .evaluate(default, None, clock)
            .map_err(|error| at_step(error, ExecStep::Plan)),
        None => Err(Error::new(
            ErrorCode::MissingParam,
            format!("the parameter `{}` has no value", param.name),
        )
        .with_stage(ExecStep::Plan)),
    }
}

fn limit_error(construct: &str, iterations: u64, reason: &str) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!("`{construct}` reached its limit after {iterations} iteration(s): {reason}"),
    )
    .with_stage(ExecStep::Plan)
    .with_detail("iterations", json!(iterations))
}

/// The error of a construct that was rolled back: what was restored, and
/// what could not be.
fn construct_rolled_back(
    cause: Error,
    restored: &[String],
    not_restored: &[String],
    irreversible: &[IrreversibleEntry],
) -> Error {
    let (code, message) = if not_restored.is_empty() {
        (
            ErrorCode::RolledBack,
            "a part of the construct failed and its changes were rolled back",
        )
    } else {
        (
            ErrorCode::RollbackFailed,
            "a part of the construct failed and some changes could not be restored",
        )
    };
    let mut error = Error::new(code, message)
        .with_stage(ExecStep::Apply)
        .with_detail("restored", json!(restored));
    if !not_restored.is_empty() {
        error = error.with_detail("not_restored", json!(not_restored));
    }
    if !irreversible.is_empty() {
        error = error.with_detail("not_rolled_back", json!(irreversible));
    }
    error.with_cause(cause)
}

/// The bound pipeline of a planned one, as the journal records it.
fn bound_pipeline(pipeline: &PipelinePlan) -> BoundStatement {
    use nurt_contracts::bound::{BoundHead, BoundPipeline, BoundRef};
    let mut stages: Vec<_> = pipeline
        .stages
        .iter()
        .map(|planned| planned.stage.clone())
        .collect();
    let head = match (&pipeline.call, &pipeline.head) {
        (Some(call), _) => BoundHead::Call(call.clone()),
        (None, Some(reference)) => BoundHead::Ref(reference.clone()),
        (None, None) if !stages.is_empty() => BoundHead::Stage(stages.remove(0)),
        (None, None) => BoundHead::Ref(BoundRef {
            name: String::new(),
            fields: Vec::new(),
        }),
    };
    BoundStatement::Pipeline(BoundPipeline {
        head,
        stages,
        span: pipeline.span,
    })
}

#[cfg(test)]
mod tests {
    use super::overlapping;

    fn claims(items: &[(&str, usize)]) -> Vec<(String, usize)> {
        items
            .iter()
            .map(|(path, n)| ((*path).to_owned(), *n))
            .collect()
    }

    #[test]
    fn a_path_and_a_descendant_overlap_even_when_other_names_sort_between() {
        assert!(overlapping(&claims(&[
            ("/srv/a", 1),
            ("/srv/a.log", 2),
            ("/srv/a/x", 3),
        ])));
    }

    #[test]
    fn disjoint_paths_and_the_same_iteration_do_not_overlap() {
        assert!(!overlapping(&claims(&[
            ("/srv/a", 1),
            ("/srv/a.log", 2),
            ("/srv/ab/x", 3),
        ])));
        assert!(!overlapping(&claims(&[("/srv/a", 1), ("/srv/a/x", 1)])));
    }

    #[test]
    fn the_same_target_overlaps() {
        assert!(overlapping(&claims(&[("/srv/a", 1), ("/srv/a", 2)])));
    }

    #[test]
    fn windows_separators_and_case_are_compared_cautiously() {
        assert!(overlapping(&claims(&[
            ("C:\\Srv\\A", 1),
            ("c:/srv/a/x", 2)
        ])));
        assert!(overlapping(&claims(&[("/Srv/A", 1), ("/srv/a", 2)])));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
