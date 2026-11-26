// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of plans with mock commands on a virtual file system and a
//! recording journal; `assert`, `verify` and `count` are the record stages
//! of the crate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_catalog::Registry;
use nurt_contracts::bound::{
    BoundArg, BoundExpr, BoundHead, BoundPipeline, BoundRef, BoundStage, Phase,
};
use nurt_contracts::config::Config;
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, FileKind, Follow};
use nurt_contracts::journal::{Expect, FileOp, MutationTx, TargetPath, apply_ops};
use nurt_contracts::kinds::{EffectKind, Reversibility};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::{Notable, ReportStatus};
use nurt_contracts::stage::{
    BoundArgs, CancelReason, CancelToken, DeliveredEvent, Demand, EffectOutcome, EffectPlan,
    EffectStage, Event, ExecCtx, ExecStep, InputKinds, ReadStage, RecordStream, StageDescriptor,
    StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_exec::{ExecEnv, Executor, Outcome, SequencedEvents, ordered_filter_map};
use nurt_plan::{LetPlan, OperationPlan, PipelinePlan, Plan, PlanStatement, PlannedStage};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::{JournalCall, RecordingJournal, TxState};
use nurt_testkit::vfs::{VirtualFs, test_path};

const LIST: &str = "mock.list";
const MAP: &str = "mock.map";
const WRITE: &str = "mock.write";
const RUN: &str = "mock.run";
const STOP: &str = "mock.stop";
const ORIGINAL: &[u8] = b"original";
const PATHS: &str = "paths";

fn descriptor(command: &str, source: bool, input: InputKinds) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source,
        input,
    }
}

fn entry(path: &str, size: i64) -> Record {
    Record::new(RecordKind::Entry)
        .with("path", Value::Path(path.to_owned()))
        .with("size", Value::Int(size))
}

fn number(record: &Record) -> i64 {
    match record.get("size") {
        Some(Value::Int(n)) => *n,
        _ => -1,
    }
}

/// Source of `entry` records: the paths of the argument `paths`, or the
/// fixed records. Counts its calls; may advance the clock or cancel the
/// execution while producing records.
#[derive(Default)]
struct Source {
    records: Vec<Record>,
    calls: Arc<AtomicUsize>,
    tick: Option<(Arc<VirtualClock>, Duration)>,
    cancel_after: Option<(usize, CancelToken)>,
}

impl StageImpl for Source {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(LIST, true, InputKinds::None)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Source {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let records = match args.value(PATHS) {
            Some(Value::List(paths)) => paths
                .iter()
                .filter_map(Value::as_str)
                .map(|path| entry(path, 1))
                .collect(),
            _ => self.records.clone(),
        };
        Ok(StageOutput::records(RecordStream::new(
            records.into_iter().enumerate().map(move |(index, record)| {
                if let Some((clock, step)) = &self.tick {
                    clock.advance(*step);
                }
                if let Some((after, token)) = &self.cancel_after
                    && index == *after
                {
                    token.cancel(CancelReason::Requested);
                }
                Ok(record)
            }),
        )))
    }
}

/// Keeps records whose size is not a multiple of 3 and adds `double`, on the
/// thread pool of the execution.
struct Mapper;

impl StageImpl for Mapper {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(MAP, false, InputKinds::Any)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Mapper {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        input.map_records(|stream| {
            ordered_filter_map(stream, ctx, |record| {
                let n = number(&record);
                let mut spin = 0_u64;
                for step in 0..u64::try_from(n % 11).unwrap_or(0) * 300 {
                    spin = std::hint::black_box(spin.wrapping_add(step));
                }
                std::hint::black_box(spin);
                Ok((n % 3 != 0).then(|| record.with("double", Value::Int(n * 2))))
            })
        })
    }
}

struct WriteState(Vec<(AbsPath, Etag, Vec<u8>)>);

/// Writes `written <path>` into every listed file, checking the entity tag
/// read in the plan. With `sabotage` another program changes the last
/// target between the plan and the apply step; with `cancel` the execution is
/// cancelled and with `tick` the clock advances at the end of the plan.
#[derive(Default)]
struct Writer {
    sabotage: Option<Arc<VirtualFs>>,
    cancel: Option<CancelToken>,
    tick: Option<(Arc<VirtualClock>, Duration)>,
}

impl StageImpl for Writer {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(WRITE, false, InputKinds::Only(vec![RecordKind::Entry]))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Writer {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let mut targets = Vec::new();
        let mut changes = Vec::new();
        for item in input.into_records()? {
            let record = item?;
            let text = record
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let path = AbsPath::new(&text)?;
            let etag = fs.etag(&path, Follow::NoFollow)?;
            changes.push(Change::new(text.as_str(), ChangeAction::Modify));
            targets.push((path, etag, format!("written {text}").into_bytes()));
        }
        if let (Some(vfs), Some((path, _, _))) = (&self.sabotage, targets.last()) {
            vfs.write_file(path, b"changed by another program")?;
        }
        if let Some(token) = &self.cancel {
            token.cancel(CancelReason::Requested);
        }
        if let Some((clock, step)) = &self.tick {
            clock.advance(*step);
        }
        let mut plan = EffectPlan::new();
        plan.bulk = targets.len() > 1;
        plan.changes = changes;
        Ok(plan.with_state(WriteState(targets)))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let WriteState(targets) = plan
            .take_state::<WriteState>()
            .ok_or_else(|| Error::new(ErrorCode::Internal, "the plan has no targets"))?;
        let ops = targets
            .into_iter()
            .map(|(path, etag, content)| FileOp::write(path, content).expect(Expect::Etag(etag)))
            .collect();
        apply_ops(tx, ops)?;
        let effect = Record::new(RecordKind::Effect)
            .with("status", Value::Enum("ok".to_owned()))
            .with("kind", Value::Text(WRITE.to_owned()));
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(effect)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}

/// Runs a program: an `exec` record with the exit code, or a `job` record
/// still running in the background.
struct Runner {
    exit_code: i64,
    detached: bool,
}

impl StageImpl for Runner {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(RUN, true, InputKinds::None)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Runner {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let mut plan = EffectPlan::new();
        plan.preview =
            vec![Record::new(RecordKind::Exec).with("command", Value::Text("make".to_owned()))];
        Ok(plan)
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let record = if self.detached {
            Record::new(RecordKind::Job)
                .with("id", Value::Text("j1".to_owned()))
                .with("state", Value::Enum("running".to_owned()))
        } else {
            Record::new(RecordKind::Exec).with("exit_code", Value::Int(self.exit_code))
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        outcome.detached = self.detached;
        Ok(outcome)
    }
}

/// Cancels the execution when it runs and passes its input on.
struct Stopper {
    cancel: CancelToken,
}

impl StageImpl for Stopper {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(STOP, false, InputKinds::Any)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Stopper {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        self.cancel.cancel(CancelReason::Requested);
        input.map_records(|records| records)
    }
}

struct World {
    clock: Arc<VirtualClock>,
    vfs: Arc<VirtualFs>,
    journal: Arc<RecordingJournal>,
}

impl World {
    fn new() -> Self {
        let clock = Arc::new(VirtualClock::new());
        let vfs = Arc::new(VirtualFs::new(clock.clone()));
        vfs.create_dir_all(&test_path("/data").unwrap()).unwrap();
        let journal = Arc::new(RecordingJournal::keeping(vfs.clone()));
        Self {
            clock,
            vfs,
            journal,
        }
    }

    fn file(&self, name: &str) -> String {
        let text = format!("/data/{name}");
        self.vfs
            .write_file(&test_path(&text).unwrap(), ORIGINAL)
            .unwrap();
        text
    }

    fn content(&self, path: &str) -> Vec<u8> {
        self.vfs.read_file(&test_path(path).unwrap()).unwrap()
    }

    fn env(&self, stages: Vec<Arc<dyn StageImpl>>) -> ExecEnv {
        ExecEnv::new(
            registry(stages),
            self.vfs.clone(),
            self.journal.clone(),
            self.clock.clone(),
            Arc::new(TestHost::linux()),
        )
    }

    fn begins(&self) -> usize {
        self.journal
            .calls()
            .iter()
            .filter(|call| matches!(call, JournalCall::Begin { .. }))
            .count()
    }
}

fn registry(stages: Vec<Arc<dyn StageImpl>>) -> Arc<Registry> {
    let mut registry = Registry::load_embedded().unwrap();
    let base = registry.command("count").cloned().unwrap();
    let specs = [LIST, MAP, WRITE, RUN, STOP]
        .into_iter()
        .map(|name| {
            let mut spec = base.clone();
            spec.name = name.to_owned();
            spec.group.clear();
            spec
        })
        .collect();
    registry
        .register_extension("mock", "Test doubles of commands.", specs)
        .unwrap();
    for implementation in nurt_exec::stages::implementations() {
        registry.register_impl(implementation).unwrap();
    }
    for implementation in stages {
        registry.register_impl(implementation).unwrap();
    }
    Arc::new(registry)
}

fn stage(name: &str, phase: Phase, args: Vec<BoundArg>) -> BoundStage {
    BoundStage {
        name: name.to_owned(),
        effect: if phase.is_effect() {
            EffectKind::Local
        } else {
            EffectKind::Read
        },
        phase,
        waits: false,
        args,
        span: None,
    }
}

fn list(paths: &[&str]) -> BoundStage {
    let items = paths
        .iter()
        .map(|path| BoundExpr::Path {
            v: (*path).to_owned(),
        })
        .collect();
    stage(
        LIST,
        Phase::Source,
        vec![BoundArg::given(PATHS, BoundExpr::List { items })],
    )
}

fn source() -> BoundStage {
    stage(LIST, Phase::Source, Vec::new())
}

fn range(low: i64, high: i64) -> BoundExpr {
    BoundExpr::Range {
        lo: Some(low),
        hi: Some(high),
    }
}

fn assert(phase: Phase, condition: &str, value: BoundExpr) -> BoundStage {
    stage("assert", phase, vec![BoundArg::given(condition, value)])
}

fn verify(head: BoundStage, stages: Vec<BoundStage>) -> BoundStage {
    let check = BoundPipeline {
        head: BoundHead::Stage(head),
        stages,
        span: None,
    };
    stage(
        "verify",
        Phase::Verify,
        vec![BoundArg::given("check", BoundExpr::Sub { p: check })],
    )
}

fn step(phase: Phase) -> ExecStep {
    match phase {
        Phase::Check => ExecStep::Check,
        Phase::Mutate | Phase::Run => ExecStep::Plan,
        Phase::After => ExecStep::After,
        Phase::Verify => ExecStep::Verify,
        _ => ExecStep::Read,
    }
}

fn pipeline(head: Option<&str>, stages: Vec<BoundStage>) -> PipelinePlan {
    let offset = u32::from(head.is_some());
    PipelinePlan {
        head: head.map(|name| BoundRef {
            name: name.to_owned(),
            fields: Vec::new(),
        }),
        stages: stages
            .into_iter()
            .zip(offset..)
            .map(|(stage, index)| PlannedStage {
                index,
                step: step(stage.phase),
                stage,
                reversibility: Reversibility::Reversible,
                demand: Demand::default(),
                check: None,
            })
            .collect(),
        effect: EffectKind::Local,
        reversibility: Reversibility::Reversible,
        span: None,
    }
}

fn operation(index: u32, stages: Vec<BoundStage>) -> PlanStatement {
    PlanStatement::Operation(OperationPlan {
        index,
        pipeline: pipeline(None, stages),
    })
}

fn plan(statements: Vec<PlanStatement>) -> Plan {
    Plan {
        catalog_rev: "test".to_owned(),
        corr_rev: "test".to_owned(),
        statements,
        effect: EffectKind::Local,
        reversibility: Reversibility::Reversible,
        corrections: Vec::new(),
    }
}

fn single(stages: Vec<BoundStage>) -> Plan {
    plan(vec![operation(0, stages)])
}

fn numbered(count: i64) -> Vec<Record> {
    (0..count)
        .map(|n| entry(&format!("/data/f{n}"), n))
        .collect()
}

fn error_of(outcome: &Outcome) -> &Error {
    outcome.error.as_ref().expect("the execution failed")
}

fn config(edit: impl FnOnce(&mut Config)) -> Arc<Config> {
    let mut config = Config::default();
    edit(&mut config);
    Arc::new(config)
}

#[test]
fn reads_transforms_and_counts_records() {
    let world = World::new();
    let env = world.env(vec![
        Arc::new(Source {
            records: numbered(6),
            ..Source::default()
        }),
        Arc::new(Mapper),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            source(),
            stage(MAP, Phase::Transform, Vec::new()),
            stage("count", Phase::Transform, Vec::new()),
        ]),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.status, ReportStatus::Ok);
    let last = outcome.last().unwrap();
    assert_eq!(last.result.value, Some(Value::Int(4)));
    assert_eq!(last.timings[0].step, ExecStep::Read);
    assert_eq!(world.begins(), 0);
}

#[test]
fn failed_check_before_the_effect_changes_nothing() {
    let world = World::new();
    let file = world.file("a.txt");
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            assert(Phase::Check, "count", range(5, 5)),
            stage(WRITE, Phase::Mutate, Vec::new()),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(error.stage(), Some(ExecStep::Check));
    assert!(error.expected.is_some() && error.actual.is_some());
    assert_eq!(outcome.status, ReportStatus::Failed);
    assert_eq!(world.begins(), 0);
    assert!(world.journal.transactions().is_empty());
    assert_eq!(world.content(&file), ORIGINAL);
}

#[test]
fn preview_stops_after_the_plan_of_the_effect() {
    let world = World::new();
    let files = [world.file("a.txt"), world.file("b.txt")];
    let options = RequestOptions {
        preview: Some(true),
        ..RequestOptions::default()
    };
    let env = world
        .env(vec![
            Arc::new(Source::default()),
            Arc::new(Writer::default()),
        ])
        .with_options(Arc::new(options));
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&files[0], &files[1]]),
            stage(WRITE, Phase::Mutate, Vec::new()),
        ]),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.status, ReportStatus::Planned);
    let effect = outcome.last().unwrap().effect.as_ref().unwrap();
    assert!(!effect.applied);
    assert!(effect.handle.is_none());
    assert!(effect.bulk);
    assert_eq!(effect.changes.len(), 2);
    assert_eq!(world.begins(), 0);
    assert_eq!(world.content(&files[0]), ORIGINAL);
}

#[test]
fn writes_many_targets_in_one_transaction() {
    let world = World::new();
    let files = [
        world.file("a.txt"),
        world.file("b.txt"),
        world.file("c.txt"),
    ];
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&files[0], &files[1], &files[2]]),
            stage(WRITE, Phase::Mutate, Vec::new()),
        ]),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].state, TxState::Committed);
    assert_eq!(transactions[0].applied.len(), 3);
    let operation = outcome.last().unwrap();
    let effect = operation.effect.as_ref().unwrap();
    assert!(effect.applied && effect.bulk);
    assert!(effect.handle.as_ref().unwrap().op.is_some());
    assert!(
        effect
            .changes
            .iter()
            .all(|facts| facts.entry == Some(FileKind::File))
    );
    assert!(effect.changes.iter().all(|facts| !facts.replaced_existing));
    assert_eq!(operation.result.records[0].kind, RecordKind::Effect);
    let steps: Vec<ExecStep> = operation.timings.iter().map(|timing| timing.step).collect();
    assert_eq!(steps, [ExecStep::Plan, ExecStep::Apply]);
    for file in &files {
        assert_eq!(world.content(file), format!("written {file}").into_bytes());
    }
}

#[test]
fn stale_target_rolls_back_the_whole_operation() {
    let world = World::new();
    let files = [world.file("a.txt"), world.file("b.txt")];
    let writer = Writer {
        sabotage: Some(world.vfs.clone()),
        ..Writer::default()
    };
    let env = world.env(vec![Arc::new(Source::default()), Arc::new(writer)]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&files[0], &files[1]]),
            stage(WRITE, Phase::Mutate, Vec::new()),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(error.stage(), Some(ExecStep::Apply));
    assert_eq!(outcome.status, ReportStatus::Failed);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].state, TxState::RolledBack);
    assert_eq!(world.content(&files[0]), ORIGINAL);
    assert_eq!(world.content(&files[1]), b"changed by another program");
}

#[test]
fn failed_assertion_after_a_run_keeps_the_effect() {
    let world = World::new();
    let env = world.env(vec![Arc::new(Runner {
        exit_code: 3,
        detached: false,
    })]);
    let outcome = Executor::new().run(
        &single(vec![
            stage(RUN, Phase::Run, Vec::new()),
            assert(Phase::After, "exit_code", range(0, 0)),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(error.stage(), Some(ExecStep::After));
    assert!(error.effect_applied);
    assert_eq!(outcome.status, ReportStatus::Ok);
    let operation = outcome.last().unwrap();
    assert!(operation.effect.as_ref().unwrap().applied);
    assert_eq!(operation.result.records[0].kind, RecordKind::Exec);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].state, TxState::Committed);
}

#[test]
fn running_job_defers_the_stages_after_the_effect() {
    let world = World::new();
    let env = world.env(vec![Arc::new(Runner {
        exit_code: 0,
        detached: true,
    })]);
    let outcome = Executor::new().run(
        &single(vec![
            stage(RUN, Phase::Run, Vec::new()),
            assert(Phase::After, "exit_code", range(0, 0)),
        ]),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.status, ReportStatus::Running);
    let operation = outcome.last().unwrap();
    assert!(operation.effect.as_ref().unwrap().detached);
    let deferred = operation
        .notable
        .iter()
        .find(|entry| entry.code == Notable::PostStagesDeferred)
        .unwrap();
    assert_eq!(deferred.count, Some(1));
}

#[test]
fn failed_verification_rolls_the_operation_back() {
    let world = World::new();
    let file = world.file("a.txt");
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            stage(WRITE, Phase::Mutate, Vec::new()),
            verify(assert(Phase::Check, "count", range(2, 2)), Vec::new()),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert_eq!(error.stage(), Some(ExecStep::Verify));
    assert_eq!(
        error.cause.as_ref().map(Error::code),
        Some(ErrorCode::Assert)
    );
    assert_eq!(outcome.status, ReportStatus::RolledBack);
    assert_eq!(world.content(&file), ORIGINAL);
    let effect = outcome.last().unwrap().effect.as_ref().unwrap();
    assert!(effect.rollback.is_some());
    assert_eq!(world.journal.transactions()[0].state, TxState::RolledBack);
}

#[test]
fn passing_verification_keeps_the_effect() {
    let world = World::new();
    let file = world.file("a.txt");
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            stage(WRITE, Phase::Mutate, Vec::new()),
            verify(assert(Phase::Check, "count", range(1, 1)), Vec::new()),
        ]),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.status, ReportStatus::Ok);
    assert_eq!(world.content(&file), format!("written {file}").into_bytes());
    let steps: Vec<ExecStep> = outcome
        .last()
        .unwrap()
        .timings
        .iter()
        .map(|t| t.step)
        .collect();
    assert_eq!(steps, [ExecStep::Plan, ExecStep::Apply, ExecStep::Verify]);
}

#[test]
fn verification_runs_a_program_as_an_operation_of_its_own() {
    let world = World::new();
    let file = world.file("a.txt");
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
        Arc::new(Runner {
            exit_code: 0,
            detached: false,
        }),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            stage(WRITE, Phase::Mutate, Vec::new()),
            verify(
                stage(RUN, Phase::Run, Vec::new()),
                vec![assert(Phase::After, "exit_code", range(0, 0))],
            ),
        ]),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 2);
    assert!(transactions.iter().all(|tx| tx.state == TxState::Committed));
}

#[test]
fn failed_rollback_lists_the_targets() {
    let world = World::new();
    let file = world.file("a.txt");
    world
        .journal
        .fail_rollback_of(TargetPath::Fs(test_path(&file).unwrap()));
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            stage(WRITE, Phase::Mutate, Vec::new()),
            verify(
                assert(Phase::Check, "empty", BoundExpr::Bool { v: true }),
                Vec::new(),
            ),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::RollbackFailed);
    assert_eq!(error.stage(), Some(ExecStep::Verify));
    assert!(error.effect_applied);
    assert!(error.details["failed"].to_string().contains("a.txt"));
    assert_eq!(outcome.status, ReportStatus::Partial);
}

#[test]
fn time_limit_ends_the_program_with_timeout() {
    let world = World::new();
    let env = world
        .env(vec![Arc::new(Source {
            records: numbered(10),
            tick: Some((world.clock.clone(), Duration::from_millis(400))),
            ..Source::default()
        })])
        .with_timeout(Duration::from_secs(1));
    let outcome = Executor::new().run(&single(vec![source()]), &env);
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Timeout);
    assert_eq!(error.stage(), Some(ExecStep::Read));
    assert_eq!(outcome.status, ReportStatus::Failed);
}

#[test]
fn cancellation_ends_the_program() {
    let world = World::new();
    let cancel = CancelToken::new();
    let env = world
        .env(vec![Arc::new(Source {
            records: numbered(10),
            cancel_after: Some((2, cancel.clone())),
            ..Source::default()
        })])
        .with_cancel(cancel);
    let outcome = Executor::new().run(&single(vec![source()]), &env);
    assert_eq!(error_of(&outcome).code(), ErrorCode::Cancelled);
}

#[test]
fn cancelled_execution_runs_no_stage() {
    let world = World::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Requested);
    let env = world
        .env(vec![Arc::new(Source {
            calls: calls.clone(),
            ..Source::default()
        })])
        .with_cancel(cancel);
    let outcome = Executor::new().run(&single(vec![source()]), &env);
    assert_eq!(error_of(&outcome).code(), ErrorCode::Cancelled);
    assert!(outcome.operations.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_after_the_plan_stops_before_the_apply_step() {
    let world = World::new();
    let file = world.file("a.txt");
    let cancel = CancelToken::new();
    let writer = Writer {
        cancel: Some(cancel.clone()),
        ..Writer::default()
    };
    let env = world
        .env(vec![Arc::new(Source::default()), Arc::new(writer)])
        .with_cancel(cancel);
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            stage(WRITE, Phase::Mutate, Vec::new()),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert_eq!(error.stage(), Some(ExecStep::Plan));
    assert_eq!(outcome.status, ReportStatus::Failed);
    assert!(!outcome.last().unwrap().effect.as_ref().unwrap().applied);
    assert_eq!(world.begins(), 0);
    assert_eq!(world.content(&file), ORIGINAL);
}

#[test]
fn time_limit_elapsed_during_the_plan_stops_before_the_apply_step() {
    let world = World::new();
    let file = world.file("a.txt");
    let writer = Writer {
        tick: Some((world.clock.clone(), Duration::from_secs(2))),
        ..Writer::default()
    };
    let env = world
        .env(vec![Arc::new(Source::default()), Arc::new(writer)])
        .with_timeout(Duration::from_secs(1));
    let outcome = Executor::new().run(
        &single(vec![
            list(&[&file]),
            stage(WRITE, Phase::Mutate, Vec::new()),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Timeout);
    assert_eq!(error.stage(), Some(ExecStep::Plan));
    assert_eq!(outcome.status, ReportStatus::Failed);
    assert_eq!(world.begins(), 0);
    assert_eq!(world.content(&file), ORIGINAL);
}

#[test]
fn stop_after_the_effect_fails_the_program_and_keeps_the_effect() {
    let world = World::new();
    let cancel = CancelToken::new();
    let env = world
        .env(vec![
            Arc::new(Runner {
                exit_code: 0,
                detached: false,
            }),
            Arc::new(Stopper {
                cancel: cancel.clone(),
            }),
        ])
        .with_cancel(cancel);
    let outcome = Executor::new().run(
        &single(vec![
            stage(RUN, Phase::Run, Vec::new()),
            stage(STOP, Phase::After, Vec::new()),
        ]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert_eq!(error.stage(), Some(ExecStep::After));
    assert!(error.effect_applied);
    assert_eq!(outcome.status, ReportStatus::Failed);
    let operation = outcome.last().unwrap();
    assert!(operation.effect.as_ref().unwrap().applied);
    assert_eq!(operation.result.records[0].kind, RecordKind::Exec);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].state, TxState::Committed);
}

#[test]
fn record_limit_ends_the_program_with_limit() {
    let world = World::new();
    let env = world
        .env(vec![Arc::new(Source {
            records: numbered(5),
            ..Source::default()
        })])
        .with_config(config(|config| config.limits.max_records = 3));
    let outcome = Executor::new().run(&single(vec![source()]), &env);
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Limit);
    assert_eq!(error.stage(), Some(ExecStep::Read));
}

#[test]
fn result_does_not_depend_on_the_number_of_threads() {
    let run = |threads: u32| {
        let world = World::new();
        let env = world
            .env(vec![
                Arc::new(Source {
                    records: numbered(3000),
                    ..Source::default()
                }),
                Arc::new(Mapper),
            ])
            .with_config(config(|config| config.limits.threads = Some(threads)));
        let outcome = Executor::new().run(
            &single(vec![source(), stage(MAP, Phase::Transform, Vec::new())]),
            &env,
        );
        assert!(outcome.is_ok(), "{:?}", outcome.error);
        outcome.last().unwrap().result.records.clone()
    };
    let sequential = run(1);
    assert_eq!(sequential.len(), 2000);
    assert_eq!(sequential, run(8));
}

#[test]
fn named_result_is_computed_once() {
    let world = World::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let env = world.env(vec![Arc::new(Source {
        records: numbered(3),
        calls: calls.clone(),
        ..Source::default()
    })]);
    let program = plan(vec![
        PlanStatement::Let(LetPlan {
            name: "files".to_owned(),
            pipeline: pipeline(None, vec![source()]),
            span: None,
        }),
        PlanStatement::Operation(OperationPlan {
            index: 0,
            pipeline: pipeline(
                Some("files"),
                vec![stage("count", Phase::Transform, Vec::new())],
            ),
        }),
        PlanStatement::Operation(OperationPlan {
            index: 1,
            pipeline: pipeline(
                Some("files"),
                vec![assert(Phase::Check, "count", range(3, 3))],
            ),
        }),
    ]);
    let outcome = Executor::new().run(&program, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.lets, ["files"]);
    assert_eq!(outcome.operations.len(), 2);
    assert_eq!(outcome.operations[0].result.value, Some(Value::Int(3)));
    assert_eq!(outcome.operations[1].result.records.len(), 3);
}

#[test]
fn unknown_named_result_fails_the_operation() {
    let world = World::new();
    let env = world.env(Vec::new());
    let program = plan(vec![PlanStatement::Operation(OperationPlan {
        index: 0,
        pipeline: pipeline(
            Some("missing"),
            vec![stage("count", Phase::Transform, Vec::new())],
        ),
    })]);
    let outcome = Executor::new().run(&program, &env);
    assert_eq!(error_of(&outcome).code(), ErrorCode::UnknownName);
}

#[test]
fn script_runs_each_operation_as_a_transaction_of_its_own() {
    let world = World::new();
    let files = [world.file("a.txt"), world.file("b.txt")];
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let program = plan(vec![
        operation(
            0,
            vec![list(&[&files[0]]), stage(WRITE, Phase::Mutate, Vec::new())],
        ),
        operation(
            1,
            vec![list(&[&files[1]]), stage(WRITE, Phase::Mutate, Vec::new())],
        ),
    ]);
    let outcome = Executor::new().run(&program, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.operations.len(), 2);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 2);
    assert_ne!(transactions[0].op, transactions[1].op);
    assert_eq!(world.journal.committed().len(), 2);
}

#[test]
fn script_stops_at_the_first_error() {
    let world = World::new();
    let files = [world.file("a.txt"), world.file("b.txt")];
    let env = world.env(vec![
        Arc::new(Source::default()),
        Arc::new(Writer::default()),
    ]);
    let program = plan(vec![
        operation(
            0,
            vec![
                list(&[&files[0]]),
                assert(Phase::Check, "count", range(5, 5)),
                stage(WRITE, Phase::Mutate, Vec::new()),
            ],
        ),
        operation(
            1,
            vec![list(&[&files[1]]), stage(WRITE, Phase::Mutate, Vec::new())],
        ),
    ]);
    let outcome = Executor::new().run(&program, &env);
    assert_eq!(error_of(&outcome).code(), ErrorCode::Assert);
    assert_eq!(outcome.operations.len(), 1);
    assert_eq!(world.begins(), 0);
    assert_eq!(world.content(&files[1]), ORIGINAL);
}

#[test]
fn events_are_delivered_in_sequence_from_start_to_finish() {
    let world = World::new();
    let delivered: Arc<Mutex<Vec<DeliveredEvent>>> = Arc::default();
    let target = delivered.clone();
    let events = SequencedEvents::new(world.clock.clone(), Some("r1".to_owned()), move |event| {
        target.lock().unwrap().push(event);
    });
    let env = world
        .env(vec![Arc::new(Source {
            records: numbered(2),
            ..Source::default()
        })])
        .with_events(Arc::new(events))
        .with_ast_hash("sha256:00");
    let outcome = Executor::new().run(&single(vec![source()]), &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let delivered = delivered.lock().unwrap();
    let seqs: Vec<u64> = delivered.iter().map(|event| event.seq).collect();
    assert_eq!(seqs, [1, 2]);
    assert!(
        matches!(&delivered[0].event, Event::ExecStarted(started) if started.ast_hash.as_deref() == Some("sha256:00"))
    );
    assert!(
        matches!(&delivered[1].event, Event::ExecFinished(finished) if finished.status == ReportStatus::Ok && finished.error.is_none())
    );
    assert!(
        delivered
            .iter()
            .all(|event| event.request.as_deref() == Some("r1"))
    );
}

#[test]
fn record_of_a_foreign_kind_is_a_type_error() {
    let world = World::new();
    let file = world.file("a.txt");
    let env = world.env(vec![
        Arc::new(Source {
            records: vec![
                entry(&file, 1),
                Record::new(RecordKind::Exec).with("exit_code", Value::Int(0)),
            ],
            ..Source::default()
        }),
        Arc::new(Writer::default()),
    ]);
    let outcome = Executor::new().run(
        &single(vec![source(), stage(WRITE, Phase::Mutate, Vec::new())]),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Type);
    assert_eq!(world.begins(), 0);
}
