// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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

use std::sync::PoisonError;
use std::time::Duration;

use nurt_contracts::bound::{BoundCall, BoundExpr, BoundLimit, BoundParam, BoundStatement};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{BeginRequest, OpHandle, OpKind, RollbackReason, SessionId};
use nurt_contracts::report::{IrreversibleEntry, ReportPart, ReportStatus};
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
        // The iterations of a loop that is not atomic are entries of their own.
        let own_entries = matches!(
            plan.shape,
            ConstructShape::Each { atomic: false, .. }
                | ConstructShape::Repeat { atomic: false, .. }
                | ConstructShape::While { atomic: false, .. }
        );
        let changes = plan.changes() && !own_entries;
        let operation = self.entry(plan.name(), &plan.program, changes, plan.index, || {
            self.run_construct(plan).map(drop)
        });
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
        let mut finished = self.entry(name, &program, changes, operation.index, || {
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
        let operation = self.entry(name, &program, true, u32::MAX, || {
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
        } = settled;
        let RunState {
            changes,
            notable,
            irreversible,
            last,
            handle: kept_handle,
            completed,
            ..
        } = state;
        let handle = handle.or(kept_handle);
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
                atomic,
                body,
                ..
            } => self.run_each(plan, source, name, *atomic, body),
            ConstructShape::Repeat {
                until,
                every_ms,
                limit,
                atomic,
                body,
            } => self.run_repeat(plan, until, *every_ms, *limit, *atomic, body),
            ConstructShape::While {
                cond,
                limit,
                atomic,
                body,
            } => self.run_while(plan, cond, *limit, *atomic, body),
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
        plan: &'e ConstructPlan,
        source: &BoundExpr,
        name: &str,
        atomic: bool,
        body: &'e [PlanStatement],
    ) -> Result<Control> {
        let items = self.evaluate_sequence(source)?;
        let mut count: u64 = 0;
        for item in items {
            self.checkpoint()?;
            count += 1;
            let outer = self.binding.names();
            self.binding.define(name.to_owned(), item);
            self.enter_path(format!("each.iteration[{count}]"));
            let control = self.iteration(plan, atomic, count, body);
            self.leave_path();
            self.binding.restore_names(outer);
            if control? == Control::Break {
                break;
            }
        }
        let mut part = ReportPart::done(self.at("each"), "loop");
        part.iterations = Some(count);
        self.push_part(part);
        Ok(Control::Next)
    }

    fn checkpoint(&self) -> Result<()> {
        self.ctx
            .checkpoint()
            .map_err(|error| at_step(error, ExecStep::Read))
    }

    /// Runs the body of one iteration. An iteration of a loop that is not
    /// atomic is an entry of its own, so a failure keeps the iterations that
    /// were completed before it.
    fn iteration(
        &self,
        plan: &'e ConstructPlan,
        atomic: bool,
        number: u64,
        body: &'e [PlanStatement],
    ) -> Result<Control> {
        let separate = !atomic
            && plan.changes()
            && !self.preview
            && self
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_none();
        if !separate {
            return self.run_block(body);
        }
        let mut control = Control::Next;
        let inner = self.entry(plan.name(), &plan.program, true, plan.index, || {
            control = self.run_block(body)?;
            Ok(())
        });
        let mut part = ReportPart::done(self.at("each"), "iteration");
        part.op = inner
            .effect
            .as_ref()
            .and_then(|effect| effect.handle.as_ref())
            .and_then(|handle| handle.op)
            .map(|op| op.0);
        part.targets = inner
            .effect
            .as_ref()
            .map(|effect| u64::try_from(effect.changes.len()).unwrap_or(u64::MAX));
        self.with_state(|state| {
            if let Some(effect) = &inner.effect {
                state.changes.extend(effect.changes.clone());
                state.irreversible.extend(effect.irreversible.clone());
                if effect.handle.is_some() {
                    state.handle.clone_from(&effect.handle);
                }
            }
            state.notable.extend(inner.notable.clone());
            state.last = inner.result.clone();
        });
        if let Some(construct) = inner.construct {
            self.with_state(|state| state.parts.extend(construct.parts));
        }
        let failed = inner.error;
        if let Some(error) = &failed {
            part.ok = false;
            part.error = Some(error.code().as_str().to_owned());
            part.rolled_back = true;
            self.with_state(|state| state.completed = Some(number - 1));
        } else {
            self.with_state(|state| state.completed = Some(number));
        }
        self.push_part(part);
        match failed {
            Some(error) => Err(error.with_detail("iteration", json!(number))),
            None => Ok(control),
        }
    }

    fn loop_limits(&self, limit: Option<BoundLimit>) -> (u64, Duration) {
        let config = &self.env.config.loop_limits;
        let mut iterations = config.max_iterations;
        let mut time = Duration::from_millis(config.max_time.as_millis());
        match limit {
            Some(BoundLimit::Iterations { n }) => iterations = n,
            Some(BoundLimit::Time { ms }) => time = Duration::from_millis(ms),
            _ => {}
        }
        (iterations, time)
    }

    fn run_repeat(
        &self,
        plan: &'e ConstructPlan,
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
            let control = self.iteration(plan, atomic, count, body);
            self.leave_path();
            if control? == Control::Break {
                break;
            }
            if self.preview || self.evaluate_condition(until)? {
                break;
            }
            if let Some(interval) = every_ms {
                self.pause(Duration::from_millis(interval))?;
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
        plan: &'e ConstructPlan,
        cond: &BoundExpr,
        limit: Option<BoundLimit>,
        atomic: bool,
        body: &'e [PlanStatement],
    ) -> Result<Control> {
        let (max_iterations, max_time) = self.loop_limits(limit);
        let started = self.ctx.clock().monotonic();
        let mut count: u64 = 0;
        let mut note = None;
        while self.evaluate_condition(cond)? {
            self.checkpoint()?;
            if count >= max_iterations || self.elapsed(started) >= max_time {
                note = Some("the limit ended the loop while the condition still held".to_owned());
                break;
            }
            count += 1;
            self.enter_path(format!("while.iteration[{count}]"));
            let control = self.iteration(plan, atomic, count, body);
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
        if let Some(effect) = &outcome.effect {
            part.targets = Some(u64::try_from(effect.changes.len()).unwrap_or(u64::MAX));
            part.irreversible = !effect.irreversible.is_empty();
        }
        let exit = exit_code(&outcome.result);
        let failure = outcome.error.clone();
        self.with_state(|state| {
            if let Some(effect) = &outcome.effect {
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
        let saved = self.with_state(|state| {
            state.path.push(format!("call {}", call.name));
            state.last_value.take()
        });
        let control = self.run_block(definition.body);
        let value = self.with_state(|state| {
            state.path.pop();
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
