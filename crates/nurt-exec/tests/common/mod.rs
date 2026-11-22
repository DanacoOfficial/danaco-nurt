// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Shared fixtures of the end-to-end tests: a registry with commands of the
//! mock extension, a virtual world and builders of plans.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "each test file uses part of the fixtures; tests fail by panicking"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nurt_catalog::Registry;
use nurt_contracts::bound::{BoundArg, BoundExpr, BoundRef, BoundStage, Phase};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, Follow};
use nurt_contracts::journal::{Expect, FileOp, MutationTx, apply_ops};
use nurt_contracts::kinds::{EffectKind, Reversibility};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::stage::{
    BoundArgs, Demand, EffectOutcome, EffectPlan, EffectStage, ExecCtx, ExecStep, InputKinds,
    ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_exec::{ExecEnv, Outcome};
use nurt_plan::{OperationPlan, PipelinePlan, Plan, PlanStatement, PlannedStage};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::{JournalCall, RecordingJournal};
use nurt_testkit::vfs::{VirtualFs, test_path};

pub(crate) const ORIGINAL: &[u8] = b"original";
pub(crate) const SABOTAGED: &[u8] = b"changed by another program";

pub(crate) fn descriptor(command: &str, source: bool, input: InputKinds) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source,
        input,
    }
}

pub(crate) fn entry(path: &str, size: i64) -> Record {
    Record::new(RecordKind::Entry)
        .with("path", Value::Path(path.to_owned()))
        .with("size", Value::Int(size))
}

pub(crate) fn finding(path: &str) -> Record {
    Record::new(RecordKind::Finding).with("path", Value::Path(path.to_owned()))
}

pub(crate) fn path_of(record: &Record) -> String {
    record
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Source of fixed records; counts the records pulled by the consumer.
pub(crate) struct Source {
    pub(crate) command: String,
    pub(crate) records: Vec<Record>,
    pub(crate) pulled: Arc<AtomicUsize>,
}

impl Source {
    pub(crate) fn new(command: &str, records: Vec<Record>) -> Self {
        Self {
            command: command.to_owned(),
            records,
            pulled: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl StageImpl for Source {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(&self.command, true, InputKinds::None)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Source {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let pulled = Arc::clone(&self.pulled);
        Ok(StageOutput::records(RecordStream::new(
            self.records.clone().into_iter().map(move |record| {
                pulled.fetch_add(1, Ordering::SeqCst);
                Ok(record)
            }),
        )))
    }
}

/// Source whose stream fails with `E_STAGE_FAILED` after the given records.
pub(crate) struct FailingSource {
    pub(crate) command: String,
    pub(crate) records: Vec<Record>,
}

impl StageImpl for FailingSource {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(&self.command, true, InputKinds::None)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for FailingSource {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let items = self
            .records
            .clone()
            .into_iter()
            .map(Ok)
            .chain(std::iter::once(Err(Error::new(
                ErrorCode::StageFailed,
                "the source broke down",
            ))));
        Ok(StageOutput::records(RecordStream::new(items)))
    }
}

/// Adds the field `tag` to the records it serves.
pub(crate) struct Tagger {
    pub(crate) command: String,
    pub(crate) kind: RecordKind,
    pub(crate) tag: &'static str,
}

impl StageImpl for Tagger {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(&self.command, false, InputKinds::Only(vec![self.kind]))
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Tagger {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        input.map_records(|stream| {
            RecordStream::new(stream.map(|item| {
                item.map(|record| record.with("tag", Value::Text(self.tag.to_owned())))
            }))
        })
    }
}

/// How an effect changes its targets.
#[derive(Clone, Copy)]
pub(crate) enum Action {
    Write,
    Remove,
}

struct Targets(Vec<(AbsPath, Etag)>);

/// Changes the files of the records of one kind, checking the entity tag
/// read in the plan. With `sabotage` another program changes the last
/// target between the plan and the apply step.
pub(crate) struct FileEffect {
    pub(crate) command: String,
    pub(crate) kind: RecordKind,
    pub(crate) action: Action,
    pub(crate) sabotage: Option<Arc<VirtualFs>>,
}

impl FileEffect {
    pub(crate) fn new(command: &str, kind: RecordKind, action: Action) -> Self {
        Self {
            command: command.to_owned(),
            kind,
            action,
            sabotage: None,
        }
    }
}

impl StageImpl for FileEffect {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(&self.command, false, InputKinds::Only(vec![self.kind]))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for FileEffect {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let mut targets = Vec::new();
        let mut plan = EffectPlan::new();
        for item in input.into_records()? {
            let text = path_of(&item?);
            let path = AbsPath::new(&text)?;
            let etag = fs.etag(&path, Follow::NoFollow)?;
            let action = match self.action {
                Action::Write => ChangeAction::Modify,
                Action::Remove => ChangeAction::Delete,
            };
            plan.changes.push(Change::new(text.as_str(), action));
            targets.push((path, etag));
        }
        if let (Some(vfs), Some((path, _))) = (&self.sabotage, targets.last()) {
            vfs.write_file(path, SABOTAGED)?;
        }
        plan.bulk = targets.len() > 1;
        Ok(plan.with_state(Targets(targets)))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let Targets(targets) = plan
            .take_state::<Targets>()
            .ok_or_else(|| Error::new(ErrorCode::Internal, "the plan has no targets"))?;
        let ops = targets
            .into_iter()
            .map(|(path, etag)| {
                let content = format!("written {}", path.as_path().display()).into_bytes();
                match self.action {
                    Action::Write => FileOp::write(path, content),
                    Action::Remove => FileOp::remove(path),
                }
                .expect(Expect::Etag(etag))
            })
            .collect();
        apply_ops(tx, ops)?;
        let effect = Record::new(RecordKind::Effect)
            .with("status", Value::Enum("ok".to_owned()))
            .with("kind", Value::Text(self.command.clone()));
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(effect)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}

/// Launches a program: an `exec` record with the exit code, or a `job`
/// record that is still running.
pub(crate) struct Launcher {
    pub(crate) command: String,
    pub(crate) exit_code: i64,
    pub(crate) detached: bool,
}

impl StageImpl for Launcher {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(&self.command, true, InputKinds::None)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Launcher {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        Ok(EffectPlan::new())
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

pub(crate) struct World {
    pub(crate) clock: Arc<VirtualClock>,
    pub(crate) vfs: Arc<VirtualFs>,
    pub(crate) journal: Arc<RecordingJournal>,
}

impl World {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn file(&self, name: &str) -> String {
        let text = format!("/data/{name}");
        self.vfs
            .write_file(&test_path(&text).unwrap(), ORIGINAL)
            .unwrap();
        text
    }

    pub(crate) fn content(&self, path: &str) -> Vec<u8> {
        self.vfs.read_file(&test_path(path).unwrap()).unwrap()
    }

    pub(crate) fn exists(&self, path: &str) -> bool {
        self.vfs.exists(&test_path(path).unwrap())
    }

    /// Environment with the given implementations registered on top of the
    /// record stages of the crate. `mocks` names the commands of the mock
    /// extension to declare.
    pub(crate) fn env(&self, mocks: &[&str], stages: Vec<Arc<dyn StageImpl>>) -> ExecEnv {
        ExecEnv::new(
            registry(mocks, stages),
            self.vfs.clone(),
            self.journal.clone(),
            self.clock.clone(),
            Arc::new(TestHost::linux()),
        )
    }

    pub(crate) fn begins(&self) -> usize {
        self.journal
            .calls()
            .iter()
            .filter(|call| matches!(call, JournalCall::Begin { .. }))
            .count()
    }
}

pub(crate) fn registry(mocks: &[&str], stages: Vec<Arc<dyn StageImpl>>) -> Arc<Registry> {
    let mut registry = Registry::load_embedded().unwrap();
    if !mocks.is_empty() {
        let base = registry.command("count").cloned().unwrap();
        let specs = mocks
            .iter()
            .map(|name| {
                let mut spec = base.clone();
                spec.name = (*name).to_owned();
                spec.group.clear();
                spec
            })
            .collect();
        registry
            .register_extension("mock", "Test doubles of commands.", specs)
            .unwrap();
    }
    for implementation in nurt_exec::stages::implementations() {
        registry.register_impl(implementation).unwrap();
    }
    for implementation in stages {
        registry.register_impl(implementation).unwrap();
    }
    Arc::new(registry)
}

pub(crate) fn stage(name: &str, phase: Phase, args: Vec<BoundArg>) -> BoundStage {
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

pub(crate) fn count_stage() -> BoundStage {
    stage("count", Phase::Transform, Vec::new())
}

pub(crate) fn first(count: i64) -> BoundStage {
    stage(
        "first",
        Phase::Transform,
        vec![BoundArg::given("count", BoundExpr::Int { v: count })],
    )
}

pub(crate) fn step(phase: Phase) -> ExecStep {
    match phase {
        Phase::Check => ExecStep::Check,
        Phase::Mutate | Phase::Run => ExecStep::Plan,
        Phase::After => ExecStep::After,
        Phase::Verify => ExecStep::Verify,
        _ => ExecStep::Read,
    }
}

pub(crate) fn pipeline(head: Option<&str>, stages: Vec<BoundStage>) -> PipelinePlan {
    pipeline_with_limits(head, stages, &[])
}

/// A pipeline whose stages carry the given limits of their demand.
pub(crate) fn pipeline_with_limits(
    head: Option<&str>,
    stages: Vec<BoundStage>,
    limits: &[Option<u64>],
) -> PipelinePlan {
    let offset = u32::from(head.is_some());
    PipelinePlan {
        head: head.map(|name| BoundRef {
            name: name.to_owned(),
            fields: Vec::new(),
        }),
        stages: stages
            .into_iter()
            .zip(offset..)
            .enumerate()
            .map(|(position, (stage, index))| PlannedStage {
                index,
                step: step(stage.phase),
                stage,
                reversibility: Reversibility::Reversible,
                demand: Demand {
                    limit: limits.get(position).copied().flatten(),
                    ..Demand::default()
                },
                check: None,
            })
            .collect(),
        effect: EffectKind::Local,
        reversibility: Reversibility::Reversible,
        span: None,
    }
}

pub(crate) fn plan(statements: Vec<PlanStatement>) -> Plan {
    Plan {
        catalog_rev: "test".to_owned(),
        corr_rev: "test".to_owned(),
        statements,
        effect: EffectKind::Local,
        reversibility: Reversibility::Reversible,
        corrections: Vec::new(),
    }
}

pub(crate) fn single(pipeline: PipelinePlan) -> Plan {
    plan(vec![PlanStatement::Operation(OperationPlan {
        index: 0,
        pipeline,
    })])
}

pub(crate) fn error_of(outcome: &Outcome) -> &Error {
    outcome.error.as_ref().expect("the execution failed")
}
