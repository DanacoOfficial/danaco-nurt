// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of plans.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nurt_contracts::bound::Phase;
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::RollbackReason;
use nurt_contracts::report::{Notable, NotableEntry, ReportStatus};
use nurt_contracts::stage::{
    BoundArgs, EffectPlan, Event, ExecCtx, ExecFinished, ExecStarted, ExecStep, Extensions,
    StageFrame, StageImpl, StageInput,
};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};
use nurt_plan::{OperationPlan, PipelinePlan, Plan, PlanStatement, PlannedStage};
use rayon::{ThreadPool, ThreadPoolBuilder};
use serde_json::json;

use crate::effect::{AppliedEffect, ProgramSource, apply_effect, change_facts, target_text};
use crate::env::ExecEnv;
use crate::eval::ExprBinding;
use crate::flow::{
    Selection, Split, at_step, execute_split, guard_kinds, internal, read_interface, resolve,
    tag_output,
};
use crate::grouped::{Effect, plan_effect};
use crate::outcome::{EffectReport, OperationOutcome, Outcome, ResultData, StepTiming};
use crate::runner::{SubpipelineRunner, named_value};

/// State of a job that has not finished.
const JOB_RUNNING: &str = "running";
/// Status of an applied effect in an `effect` record.
const EFFECT_OK: &str = "ok";

/// Executor of plans.
///
/// The executor holds no state between executions; one executor serves any
/// number of executions, also concurrently.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_catalog::Registry;
/// use nurt_contracts::kinds::{EffectKind, Reversibility};
/// use nurt_contracts::report::ReportStatus;
/// use nurt_exec::{ExecEnv, Executor};
/// use nurt_plan::Plan;
/// use nurt_testkit::{clock::VirtualClock, host::TestHost, journal::NoopJournal, vfs::VirtualFs};
///
/// let clock = Arc::new(VirtualClock::new());
/// let fs = Arc::new(VirtualFs::new(clock.clone()));
/// let env = ExecEnv::new(
///     Arc::new(Registry::load_embedded()?),
///     fs.clone(),
///     Arc::new(NoopJournal::new(fs)),
///     clock,
///     Arc::new(TestHost::linux()),
/// );
/// let plan = Plan {
///     catalog_rev: String::new(),
///     corr_rev: String::new(),
///     statements: Vec::new(),
///     effect: EffectKind::Read,
///     reversibility: Reversibility::Read,
///     corrections: Vec::new(),
/// };
/// let outcome = Executor::new().run(&plan, &env);
/// assert_eq!(outcome.status, ReportStatus::Ok);
/// assert!(outcome.operations.is_empty());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Executor;

impl Executor {
    /// An executor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Executes `plan` in `env`.
    ///
    /// The statements run in program order. A named result is computed once
    /// and kept as a snapshot for every later reference. Every operation runs
    /// its pipeline: the stages before the effect as a lazy stream, the
    /// effect stage in a plan and an apply step with one journal transaction,
    /// the stages after the effect on the records of the effect, and
    /// `verify`, whose failure rolls the committed operation back. With the
    /// request option `preview` an operation stops after the plan of its
    /// effect. The first error ends the execution; in a script the following
    /// operations do not run.
    ///
    /// Events [`Event::ExecStarted`] and [`Event::ExecFinished`] enclose the
    /// events of the stages. The result holds the status, the error, the
    /// result and the effect of every executed operation and the timings;
    /// the report is assembled from it by the caller.
    #[must_use]
    pub fn run(&self, plan: &Plan, env: &ExecEnv) -> Outcome {
        let start = env.clock.monotonic();
        let mut outcome = Outcome::new(Instant::saturating_from_wall(env.clock.now()));
        env.events.emit(Event::ExecStarted(ExecStarted {
            ast_hash: env.ast_hash.clone(),
            effect: plan.effect,
        }));
        match Execution::prepare(plan, env) {
            Ok(execution) => execution.execute(&mut outcome),
            Err(error) => outcome.fail(error),
        }
        outcome.finished = Instant::saturating_from_wall(env.clock.now());
        outcome.elapsed = env.clock.monotonic().saturating_sub(start);
        env.events.emit(Event::ExecFinished(ExecFinished {
            status: outcome.status,
            error: outcome.error.as_ref().map(Error::code),
        }));
        outcome
    }
}

/// Implementation chosen for the input of a read stage.
enum Chosen<'a> {
    One(&'a Arc<dyn StageImpl>, StageInput<'a>),
    Split(Split),
}

/// Per-stage state of a pipeline: arguments, contexts and the selected
/// implementations, which the lazy streams borrow.
struct Stages {
    args: Vec<BoundArgs>,
    ctxs: Vec<ExecCtx>,
    impls: Vec<OnceLock<Arc<dyn StageImpl>>>,
}

/// Measures the time spent per step.
struct StepClock<'c> {
    clock: &'c dyn Clock,
    timings: Vec<StepTiming>,
}

impl<'c> StepClock<'c> {
    fn new(clock: &'c dyn Clock) -> Self {
        Self {
            clock,
            timings: Vec::new(),
        }
    }

    fn time<T>(&mut self, step: ExecStep, work: impl FnOnce() -> T) -> T {
        let start = self.clock.monotonic();
        let result = work();
        let elapsed = self.clock.monotonic().saturating_sub(start);
        self.timings.push(StepTiming { step, elapsed });
        result
    }
}

struct Execution<'e> {
    plan: &'e Plan,
    env: &'e ExecEnv,
    ctx: ExecCtx,
    binding: Arc<ExprBinding>,
    preview: bool,
    preserve_budget: Duration,
}

impl<'e> Execution<'e> {
    fn prepare(plan: &'e Plan, env: &'e ExecEnv) -> Result<Self> {
        let timeout = env.resolved_timeout()?;
        let limits = env.limits();
        let binding = Arc::new(match env.seed {
            Some(seed) => ExprBinding::with_seed(seed),
            None => ExprBinding::new(),
        });
        let preserve_budget = Duration::from_millis(env.config.run.preimage_budget.as_millis());
        let runner = SubpipelineRunner::new(Arc::clone(&env.registry), Arc::clone(&binding))
            .with_program(&plan.catalog_rev, &plan.corr_rev, plan.corrections.clone())
            .with_preserve_budget(preserve_budget);
        let mut extensions = Extensions::new();
        env.fill_extensions(&mut extensions);
        if let Some(pool) = thread_pool(env, limits.threads) {
            extensions.insert::<ThreadPool>(pool);
        }
        let mut builder = ExecCtx::builder(Arc::clone(&env.clock), Arc::clone(&env.host))
            .fs(Arc::clone(&env.fs))
            .config(Arc::clone(&env.config))
            .options(Arc::clone(&env.options))
            .events(Arc::clone(&env.events))
            .cancel(env.cancel.clone())
            .limits(limits)
            .evaluator(binding.clone())
            .pipelines(Arc::new(runner))
            .services(Arc::clone(&env.services))
            .extensions(extensions);
        if let Some(journal) = &env.journal {
            builder = builder.journal(Arc::clone(journal));
        }
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(dir) = &env.state_dir {
            builder = builder.state_dir(dir.clone());
        }
        Ok(Self {
            plan,
            env,
            ctx: builder.build(),
            binding,
            preview: env.preview(),
            preserve_budget,
        })
    }

    fn execute(&self, outcome: &mut Outcome) {
        for statement in &self.plan.statements {
            if let Err(error) = self.ctx.checkpoint() {
                outcome.fail(at_step(error, ExecStep::Read));
                return;
            }
            match statement {
                PlanStatement::Let(binding) => match self.snapshot(&binding.pipeline) {
                    Ok(value) => {
                        self.binding.define(binding.name.clone(), value);
                        outcome.lets.push(binding.name.clone());
                    }
                    Err(error) => {
                        outcome.fail(error);
                        return;
                    }
                },
                PlanStatement::Operation(operation) => {
                    let result = self.operation(operation);
                    let failed = result.error.is_some();
                    outcome.push(result);
                    if failed {
                        return;
                    }
                }
                _ => {
                    outcome.fail(internal("the plan holds a statement of an unknown kind"));
                    return;
                }
            }
        }
    }

    fn program(&self) -> ProgramSource<'_> {
        ProgramSource {
            registry: &self.env.registry,
            catalog_rev: &self.plan.catalog_rev,
            corr_rev: &self.plan.corr_rev,
            corrections: &self.plan.corrections,
        }
    }

    fn max_records(&self) -> u64 {
        self.ctx.limits().max_records
    }

    fn stages(&self, stages: &[PlannedStage]) -> Result<Stages> {
        let args = stages
            .iter()
            .map(|stage| {
                self.binding
                    .bind(&stage.stage, self.ctx.clock())
                    .map_err(|error| at_step(error, stage.step))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Stages {
            args,
            ctxs: stages
                .iter()
                .map(|stage| self.ctx.for_stage(stage.frame()))
                .collect(),
            impls: stages.iter().map(|_| OnceLock::new()).collect(),
        })
    }

    fn head(&self, pipeline: &PipelinePlan) -> Result<StageInput<'static>> {
        match &pipeline.head {
            Some(reference) => {
                Ok(ResultData::from_value(named_value(&self.binding, reference)?).into_input())
            }
            None => Ok(StageInput::none()),
        }
    }

    fn implementation<'a>(
        &self,
        state: &'a Stages,
        index: usize,
        command: &str,
        input: StageInput<'a>,
        limit: Option<u64>,
    ) -> Result<Chosen<'a>> {
        match resolve(
            &self.env.registry,
            command,
            input,
            limit,
            self.max_records(),
        )? {
            Selection::One(found, input) => {
                let cell = state
                    .impls
                    .get(index)
                    .ok_or_else(|| internal(format!("no state for stage {index}")))?;
                Ok(Chosen::One(cell.get_or_init(|| found), input))
            }
            Selection::Split(split) => Ok(Chosen::Split(split)),
        }
    }

    /// Chains the read stages with the given indexes into a lazy stream.
    ///
    /// A stage with several implementations reads its input before it
    /// produces output, because the records are split by their kind.
    fn chain<'a>(
        &self,
        stages: &'a [PlannedStage],
        state: &'a Stages,
        indexes: impl IntoIterator<Item = usize>,
        mut input: StageInput<'a>,
    ) -> Result<StageInput<'a>> {
        for index in indexes {
            let stage = &stages[index];
            let step = stage.step;
            let ctx = &state.ctxs[index];
            ctx.checkpoint().map_err(|error| at_step(error, step))?;
            let chosen = self
                .implementation(
                    state,
                    index,
                    stage.command(),
                    input,
                    input_limit(stages, index),
                )
                .map_err(|error| at_step(error, step))?;
            let output = match chosen {
                Chosen::One(implementation, resolved) => {
                    let read = read_interface(implementation, stage.command())?;
                    let guarded = guard_kinds(
                        resolved,
                        stage.command(),
                        &implementation.descriptor().input,
                    );
                    read.execute(&state.args[index], guarded, ctx)
                }
                Chosen::Split(split) => execute_split(
                    split,
                    stage.command(),
                    &state.args[index],
                    ctx,
                    self.max_records(),
                ),
            }
            .map_err(|error| at_step(error, step))?;
            input = tag_output(output, step, ctx);
        }
        Ok(input)
    }

    fn snapshot(&self, pipeline: &PipelinePlan) -> Result<Value> {
        let state = self.stages(&pipeline.stages)?;
        let input = self.head(pipeline)?;
        let output = self.chain(&pipeline.stages, &state, 0..pipeline.stages.len(), input)?;
        let data = ResultData::collect(output, self.max_records())
            .map_err(|error| at_step(error, ExecStep::Read))?;
        Ok(data.to_value())
    }

    fn operation(&self, operation: &OperationPlan) -> OperationOutcome {
        let mut outcome = OperationOutcome::new(operation.index);
        let mut clock = StepClock::new(self.ctx.clock());
        self.drive(&operation.pipeline, &mut outcome, &mut clock);
        outcome.timings = clock.timings;
        let mut notes = self.ctx.take_notes();
        notes.append(&mut outcome.notable);
        outcome.notable = notes;
        outcome
    }

    fn drive(
        &self,
        pipeline: &PipelinePlan,
        outcome: &mut OperationOutcome,
        clock: &mut StepClock<'_>,
    ) {
        let stages = &pipeline.stages;
        let prepared = self
            .stages(stages)
            .and_then(|state| Ok((state, self.head(pipeline)?)));
        let (state, input) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return outcome.fail(error),
        };
        let Some(effect_index) = stages.iter().position(PlannedStage::is_effect) else {
            let collected = clock.time(ExecStep::Read, || {
                self.chain(stages, &state, 0..stages.len(), input)
                    .and_then(|output| ResultData::collect(output, self.max_records()))
                    .map_err(|error| at_step(error, ExecStep::Read))
            });
            match collected {
                Ok(result) => outcome.result = result,
                Err(error) => outcome.fail(error),
            }
            return;
        };
        let planned = clock.time(ExecStep::Plan, || {
            self.chain(stages, &state, 0..effect_index, input)
                .and_then(|input| self.plan_effect(stages, &state, effect_index, input))
        });
        let (effect, plan) = match planned {
            Ok(planned) => planned,
            Err(error) => return outcome.fail(error),
        };
        let effect_stage = &stages[effect_index];
        let mut report = EffectReport::planned(effect_stage.command(), &plan);
        if self.preview {
            outcome.status = ReportStatus::Planned;
            outcome.result = ResultData::from_records(plan.preview);
            outcome.effect = Some(report);
            return;
        }
        let planned_changes = plan.changes.clone();
        let applied = clock.time(ExecStep::Apply, || {
            self.apply(pipeline, effect_stage, &effect, &state, effect_index, plan)
        });
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                outcome.effect = Some(report);
                return outcome.fail(error);
            }
        };
        record_applied(
            &mut report,
            applied.handle.clone(),
            &applied,
            planned_changes,
        );
        self.after_effect(
            stages,
            &state,
            effect_index,
            applied,
            report,
            outcome,
            clock,
        );
    }

    fn plan_effect(
        &self,
        stages: &[PlannedStage],
        state: &Stages,
        index: usize,
        input: StageInput<'_>,
    ) -> Result<(Effect, EffectPlan)> {
        let stage = &stages[index];
        let selection = resolve(
            &self.env.registry,
            stage.command(),
            input,
            input_limit(stages, index),
            self.max_records(),
        )
        .map_err(|error| at_step(error, ExecStep::Plan))?;
        plan_effect(
            selection,
            stage.command(),
            &state.args[index],
            &state.ctxs[index],
        )
        .map_err(|error| at_step(error, ExecStep::Plan))
    }

    fn apply(
        &self,
        pipeline: &PipelinePlan,
        stage: &PlannedStage,
        effect: &Effect,
        state: &Stages,
        index: usize,
        plan: EffectPlan,
    ) -> Result<AppliedEffect> {
        let journal = self.env.journal.as_deref().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                format!(
                    "`{}` needs the journal to change the world",
                    stage.command()
                ),
            )
            .with_stage(ExecStep::Apply)
        })?;
        let program = self.program();
        let document = program.document(
            pipeline.head.as_ref(),
            pipeline.stages.iter().map(|planned| &planned.stage),
            pipeline.span,
        );
        let budget = (stage.phase() == Phase::Run).then_some(self.preserve_budget);
        let bulk = plan.bulk || plan.changes.len() > 1;
        let ctx = state.ctxs[index].for_stage(StageFrame {
            step: ExecStep::Apply,
            ..stage.frame()
        });
        let request = program.begin_request(stage.command(), document, &ctx, bulk, budget);
        apply_effect(effect, &state.args[index], plan, journal, request, &ctx)
    }

    /// Runs the stages after the effect (`5a`) and `verify` (`6`).
    #[allow(
        clippy::too_many_arguments,
        reason = "the step continues the state of one operation"
    )]
    fn after_effect(
        &self,
        stages: &[PlannedStage],
        state: &Stages,
        effect_index: usize,
        applied: AppliedEffect,
        mut report: EffectReport,
        outcome: &mut OperationOutcome,
        clock: &mut StepClock<'_>,
    ) {
        let later: Vec<usize> = (effect_index + 1..stages.len()).collect();
        let records = applied.records;
        if report.detached || has_running_job(&records.records) {
            outcome.status = ReportStatus::Running;
            if !later.is_empty() {
                let mut deferred = NotableEntry::new(Notable::PostStagesDeferred);
                deferred.count = u64::try_from(later.len()).ok();
                outcome.notable.push(deferred);
            }
            outcome.result = records;
            outcome.effect = Some(report);
            return;
        }
        let (verify, after): (Vec<usize>, Vec<usize>) = later
            .into_iter()
            .partition(|&index| stages[index].step == ExecStep::Verify);
        let effect_records = records.clone();
        if after.is_empty() {
            outcome.result = records;
        } else {
            let result = clock.time(ExecStep::After, || {
                self.chain(stages, state, after, records.into_input())
                    .and_then(|output| ResultData::collect(output, self.max_records()))
            });
            match result {
                Ok(result) => outcome.result = result,
                Err(error) => {
                    outcome.result = effect_records;
                    outcome.error = Some(at_step(error, ExecStep::After).with_effect_applied(true));
                    outcome.effect = Some(report);
                    return;
                }
            }
        }
        for index in verify {
            let input = verify_input(&effect_records, &report);
            let checked = clock.time(ExecStep::Verify, || {
                self.chain(stages, state, [index], input.into_input())
                    .and_then(|output| ResultData::collect(output, self.max_records()))
            });
            if let Err(check) = checked {
                self.roll_back(&mut report, outcome, at_step(check, ExecStep::Verify));
                break;
            }
        }
        outcome.effect = Some(report);
    }

    /// Rolls back a committed operation whose verification failed.
    fn roll_back(&self, report: &mut EffectReport, outcome: &mut OperationOutcome, check: Error) {
        let Some(op) = report.handle.as_ref().and_then(|handle| handle.op) else {
            outcome.status = ReportStatus::RolledBack;
            outcome.error = Some(rolled_back(check, &[]));
            return;
        };
        let Some(journal) = self.env.journal.as_deref() else {
            outcome.status = ReportStatus::RolledBack;
            outcome.error = Some(rolled_back(check, &[]));
            return;
        };
        match journal.rollback_committed(op, RollbackReason::VerifyFailed) {
            Ok(undo) => {
                if !undo.unavailable.is_empty() {
                    let mut partial = NotableEntry::new(Notable::RollbackPartial);
                    partial.count = u64::try_from(undo.unavailable.len()).ok();
                    outcome.notable.push(partial);
                }
                report.add_irreversible(&undo.irreversible);
                let restored: Vec<String> = undo
                    .restored
                    .iter()
                    .map(|target| target_text(&target.path))
                    .collect();
                outcome.status = ReportStatus::RolledBack;
                outcome.error = Some(rolled_back(check, &restored));
                report.rollback = Some(undo);
            }
            Err(failure) => {
                let error = if failure.code() == ErrorCode::RollbackFailed {
                    failure
                } else {
                    Error::new(
                        ErrorCode::RollbackFailed,
                        "the verification failed and the effect could not be rolled back",
                    )
                    .with_detail("journal", json!(failure.code().as_str()))
                };
                outcome.status = ReportStatus::Partial;
                outcome.error = Some(
                    error
                        .with_stage(ExecStep::Verify)
                        .with_effect_applied(true)
                        .with_cause(check),
                );
            }
        }
    }
}

/// Largest number of records the stage at `index` needs from its input: the
/// demand of the stage before it.
fn input_limit(stages: &[PlannedStage], index: usize) -> Option<u64> {
    index
        .checked_sub(1)
        .and_then(|previous| stages.get(previous))
        .and_then(|stage| stage.demand.limit)
}

fn thread_pool(env: &ExecEnv, threads: u32) -> Option<Arc<ThreadPool>> {
    if threads <= 1 {
        return None;
    }
    if let Some(pool) = &env.pool {
        return Some(Arc::clone(pool));
    }
    ThreadPoolBuilder::new()
        .num_threads(usize::try_from(threads).ok()?)
        .build()
        .ok()
        .map(Arc::new)
}

fn record_applied(
    report: &mut EffectReport,
    handle: nurt_contracts::journal::OpHandle,
    applied: &AppliedEffect,
    planned: Vec<nurt_contracts::record::Change>,
) {
    let changes = if applied.changes.is_empty() {
        planned
    } else {
        applied.changes.clone()
    };
    report.applied = true;
    report.changes = change_facts(changes, Some(&handle));
    report.notable.extend(applied.notable.iter().cloned());
    report.add_irreversible(&applied.irreversible);
    report.add_irreversible(&handle.irreversible);
    report.bulk = report.bulk || handle.bulk;
    report.detached = applied.detached;
    report.handle = Some(handle);
}

fn rolled_back(check: Error, restored: &[String]) -> Error {
    Error::new(
        ErrorCode::RolledBack,
        "the verification failed and the effect was rolled back",
    )
    .with_stage(ExecStep::Verify)
    .with_detail("restored", json!(restored))
    .with_cause(check)
}

fn has_running_job(records: &[Record]) -> bool {
    records.iter().any(|record| {
        record.kind == RecordKind::Job
            && record
                .get("state")
                .and_then(Value::as_str)
                .is_some_and(|state| state == JOB_RUNNING)
    })
}

/// Input of `verify`: the `effect` records of the effect, or one `effect`
/// record describing the operation when the effect produced none.
fn verify_input(records: &ResultData, report: &EffectReport) -> ResultData {
    let effects: Vec<Record> = records
        .records
        .iter()
        .filter(|record| record.kind == RecordKind::Effect)
        .cloned()
        .collect();
    if !effects.is_empty() {
        return ResultData::from_records(effects);
    }
    let op = report
        .handle
        .as_ref()
        .and_then(|handle| handle.op)
        .and_then(|op| i64::try_from(op.0).ok());
    let targets = i64::try_from(report.changes.len()).unwrap_or(i64::MAX);
    let mut record = Record::new(RecordKind::Effect)
        .with("status", Value::Enum(EFFECT_OK.to_owned()))
        .with("kind", Value::Text(report.command.clone()))
        .with("targets", Value::Int(targets));
    if let Some(op) = op {
        record = record.with("op", Value::Int(op));
    }
    ResultData::from_records(vec![record])
}
