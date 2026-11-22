// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test doubles shared by the integration tests: a virtual clock, a host, a
//! journal transaction that applies changes to a temporary directory and
//! can undo them, a pipeline runner, a run context factory, an event
//! recorder and a service link to an in-process schedule module.

#![allow(dead_code, reason = "every test file uses a different part of the support code")]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use nurt_contracts::bound::{BoundArg, BoundExpr, BoundHead, BoundPipeline, BoundStage, Phase};
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{
    Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId,
    PreserveMode, Preserved, RollbackReason, RollbackReport, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, EffectStage, ExecCtx, Extensions,
    PipelineRunner, RecordStream, StageData, StageInput, StageOutput,
};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};
use nurt_sched::action::pipeline_hash;
use nurt_sched::module::{
    ModuleEvent, ModuleEvents, ModuleRequest, RunContextFactory, RunContextRequest, ScheduleModule,
    ServiceLink, ServiceModule,
};
use nurt_sched::store::{Definition, FORMAT, OnMissed, Overlap};
use nurt_sched::trigger::TriggerDef;

/// Real-time bound for waiting on a condition reached by another thread.
pub const WAIT: Duration = Duration::from_secs(10);

/// Instant of an RFC 3339 text.
pub fn at(text: &str) -> Instant {
    text.parse().unwrap()
}

/// Minutes as a duration.
pub fn minutes(count: u64) -> Duration {
    Duration::from_secs(count * 60)
}

/// Virtual clock. In advancing mode `sleep` moves the time forward at once;
/// in frozen mode it waits until the cancellation token is cancelled.
pub struct VirtualClock {
    now: Mutex<i128>,
    monotonic: Mutex<Duration>,
    advancing: bool,
}

impl VirtualClock {
    pub fn frozen(start: Instant) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(i128::from(start.as_unix_nanos())),
            monotonic: Mutex::new(Duration::ZERO),
            advancing: false,
        })
    }

    pub fn set(&self, instant: Instant) {
        *self.now.lock().unwrap() = i128::from(instant.as_unix_nanos());
    }

    pub fn advance(&self, duration: Duration) {
        *self.now.lock().unwrap() += i128::try_from(duration.as_nanos()).unwrap();
        *self.monotonic.lock().unwrap() += duration;
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(*self.now.lock().unwrap())
    }

    fn monotonic(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> std::result::Result<(), CancelReason> {
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        if self.advancing {
            self.advance(duration);
            return Ok(());
        }
        match cancel.wait_timeout(WAIT) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

/// Host whose local time zone is `timezone`.
pub struct TestHost {
    info: HostInfo,
}

impl TestHost {
    pub fn new(timezone: &str) -> Arc<Self> {
        Arc::new(Self {
            info: HostInfo {
                os: Os::Linux,
                os_version: "test".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "test".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from("/home/user"),
                temp: PathBuf::from("/tmp"),
                path_style: PathStyle::Posix,
                timezone: timezone.to_owned(),
                locale: None,
                cpus: 1,
            },
        })
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Journal transaction that applies changes to the file system and keeps
/// the previous content of every target for [`TestTx::undo`].
#[derive(Default)]
pub struct TestTx {
    targets: Vec<(PathBuf, Option<Vec<u8>>, bool)>,
    pub irreversible: Vec<IrreversibleEntry>,
}

impl TestTx {
    /// Paths declared by the transaction, in declaration order.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.targets.iter().map(|(path, _, _)| path.clone()).collect()
    }

    /// Restores every target, in reverse order.
    pub fn undo(&self) {
        for (path, before, existed) in self.targets.iter().rev() {
            match (before, existed) {
                (Some(bytes), _) => fs::write(path, bytes).unwrap(),
                (None, true) => {}
                (None, false) => {
                    if path.is_dir() {
                        fs::remove_dir_all(path).unwrap();
                    } else if path.exists() {
                        fs::remove_file(path).unwrap();
                    }
                }
            }
        }
    }
}

fn fs_path(path: &TargetPath) -> PathBuf {
    match path {
        TargetPath::Fs(path) => path.as_path().to_path_buf(),
        TargetPath::Provider(_) => panic!("schedule commands change files only"),
    }
}

impl MutationTx for TestTx {
    fn op(&self) -> Option<OpId> {
        Some(OpId(1))
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let first = self.targets.len();
        for target in targets {
            let path = fs_path(&target.path);
            let existed = path.exists();
            let before = if path.is_file() { Some(fs::read(&path).unwrap()) } else { None };
            self.targets.push((path, before, existed));
        }
        Ok((first..self.targets.len())
            .map(|index| Preserved {
                index: TargetIndex(u32::try_from(index).unwrap()),
                volume: None,
                mode: PreserveMode::Copy,
                bytes_held: 0,
                before: BeforeState::default(),
                irreversible: None,
                notable: Vec::new(),
            })
            .collect())
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(Error::new(ErrorCode::Unsupported, "the test transaction does not stage files"))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        let (path, _, _) = &self.targets[usize::try_from(target.0).unwrap()];
        match mutation {
            Mutation::WriteBytes(bytes) => fs::write(path, bytes).unwrap(),
            Mutation::Remove => fs::remove_file(path).unwrap(),
            Mutation::CreateDir { .. } => fs::create_dir(path).unwrap(),
            other => panic!("unexpected mutation {other:?}"),
        }
        Ok(Applied {
            index: target,
            after: nurt_contracts::journal::AfterState::default(),
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Ok(Vec::new())
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Err(Error::new(ErrorCode::Unsupported, "the test transaction is not committed"))
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Ok(RollbackReport {
            op: None,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

/// Behaviour of a test pipeline runner.
pub type Behaviour = dyn Fn(&ExecCtx) -> Result<Vec<Record>> + Send + Sync;

/// Pipeline runner that answers with the records of a closure.
pub struct TestRunner {
    behaviour: Box<Behaviour>,
    pub calls: Mutex<Vec<Option<Duration>>>,
}

impl TestRunner {
    pub fn new(behaviour: impl Fn(&ExecCtx) -> Result<Vec<Record>> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            behaviour: Box::new(behaviour),
            calls: Mutex::new(Vec::new()),
        })
    }

    /// A runner whose program prints `stdout` and exits with `code`.
    pub fn printing(stdout: &'static str, code: i64) -> Arc<Self> {
        Self::new(move |_| Ok(vec![exec_record(stdout, code)]))
    }
}

impl PipelineRunner for TestRunner {
    fn run<'a>(
        &'a self,
        _pipeline: &'a BoundPipeline,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        self.calls.lock().unwrap().push(ctx.remaining());
        let records = (self.behaviour)(ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// `exec` record of a program that printed `stdout` and exited with `code`.
pub fn exec_record(stdout: &str, code: i64) -> Record {
    Record::new(RecordKind::Exec)
        .with("exit_code", Value::Int(code))
        .with("timed_out", Value::Bool(false))
        .with("stdout", Value::Text(stdout.to_owned()))
        .with("stderr", Value::Text(String::new()))
}

/// Run context factory that builds contexts with a test runner.
pub struct TestFactory {
    clock: Arc<VirtualClock>,
    runner: Arc<TestRunner>,
    pub requests: Mutex<Vec<RunContextRequest>>,
}

impl TestFactory {
    pub fn new(clock: Arc<VirtualClock>, runner: Arc<TestRunner>) -> Arc<Self> {
        Arc::new(Self {
            clock,
            runner,
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl RunContextFactory for TestFactory {
    fn context(&self, request: &RunContextRequest) -> Result<ExecCtx> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(ExecCtx::builder(self.clock.clone(), TestHost::new("UTC"))
            .pipelines(self.runner.clone())
            .cancel(request.cancel.clone())
            .timeout(request.timeout)
            .build())
    }
}

/// Event receiver that keeps every event.
#[derive(Default)]
pub struct TestEvents {
    events: Mutex<Vec<ModuleEvent>>,
    changed: Condvar,
}

impl TestEvents {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Waits until `count` events named `name` were published and returns
    /// them.
    pub fn wait_for(&self, name: &str, count: usize) -> Vec<ModuleEvent> {
        let guard = self.events.lock().unwrap();
        let (guard, _) = self
            .changed
            .wait_timeout_while(guard, WAIT, |events| {
                events.iter().filter(|event| event.event == name).count() < count
            })
            .unwrap();
        guard.iter().filter(|event| event.event == name).cloned().collect()
    }

    pub fn named(&self, name: &str) -> Vec<ModuleEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.event == name)
            .cloned()
            .collect()
    }
}

impl ModuleEvents for TestEvents {
    fn publish(&self, event: ModuleEvent) {
        self.events.lock().unwrap().push(event);
        self.changed.notify_all();
    }
}

/// Service link to a schedule module running in the test process.
pub struct TestLink {
    pub module: Option<ScheduleModule>,
    pub requests: Mutex<Vec<ModuleRequest>>,
}

impl TestLink {
    pub fn to(module: ScheduleModule) -> Arc<Self> {
        Arc::new(Self {
            module: Some(module),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub fn stopped() -> Arc<Self> {
        Arc::new(Self {
            module: None,
            requests: Mutex::new(Vec::new()),
        })
    }

    pub fn ops(&self) -> Vec<String> {
        self.requests.lock().unwrap().iter().map(|request| request.op.clone()).collect()
    }
}

impl ServiceLink for TestLink {
    fn running(&self) -> bool {
        self.module.is_some()
    }

    fn request(&self, request: ModuleRequest) -> Result<serde_json::Value> {
        self.requests.lock().unwrap().push(request.clone());
        match &self.module {
            Some(module) => module.request(request),
            None => Err(Error::new(ErrorCode::Io, "the service does not run")),
        }
    }
}

/// Pipeline `run ["true"] cwd: "/tmp"` used as the action of test schedules.
pub fn pipeline(program: &str) -> BoundPipeline {
    BoundPipeline {
        head: BoundHead::Stage(BoundStage {
            name: "run".to_owned(),
            effect: EffectKind::Process,
            phase: Phase::Run,
            waits: false,
            args: vec![
                BoundArg::given(
                    "command",
                    BoundExpr::List {
                        items: vec![BoundExpr::Text { v: program.to_owned() }],
                    },
                ),
                BoundArg::given("cwd", BoundExpr::Path { v: "/tmp".to_owned() }),
            ],
            span: None,
        }),
        stages: Vec::new(),
        span: None,
    }
}

/// Definition of a schedule running [`pipeline`], with the defaults of
/// `schedule.create`.
pub fn definition(id: &str, trigger: TriggerDef, created: Instant) -> Definition {
    let action = pipeline("true");
    Definition {
        format: FORMAT.to_owned(),
        id: id.to_owned(),
        label: format!("test {id}"),
        trigger,
        action_kind: "pipeline".to_owned(),
        ast_hash: pipeline_hash(&action).unwrap(),
        pipeline: Some(action),
        action_args: None,
        action: "run [\"true\"]".to_owned(),
        on_missed: OnMissed::Skip,
        misfire: nurt_contracts::value::Duration::from_millis(120_000),
        overlap: Overlap::Skip,
        timeout: nurt_contracts::value::Duration::from_millis(3_600_000),
        max_runs: None,
        expires: None,
        paused: false,
        created,
        changed: created,
        session: None,
        base: None,
        project_roots: Vec::new(),
    }
}

/// Interval trigger without limits.
pub fn every(interval: Duration) -> TriggerDef {
    TriggerDef {
        kind: nurt_contracts::record::TriggerKind::Every,
        expression: nurt_sched::trigger::format_duration(interval),
        timezone: "UTC".to_owned(),
    }
}

/// Execution context of the schedule commands on `state_dir`.
pub fn exec_ctx(state_dir: &Path, clock: Arc<VirtualClock>, extensions: Extensions) -> ExecCtx {
    exec_ctx_in_session(state_dir, clock, extensions, None)
}

/// [`exec_ctx`] with a session in the request options.
pub fn exec_ctx_in_session(
    state_dir: &Path,
    clock: Arc<VirtualClock>,
    extensions: Extensions,
    session: Option<&str>,
) -> ExecCtx {
    let options = nurt_contracts::envelope::RequestOptions {
        session: session.map(str::to_owned),
        ..Default::default()
    };
    ExecCtx::builder(clock, TestHost::new("Europe/Warsaw"))
        .state_dir(state_dir.to_path_buf())
        .extensions(extensions)
        .options(Arc::new(options))
        .build()
}

/// Extensions with a service link.
pub fn with_link(link: Arc<TestLink>) -> Extensions {
    let mut extensions = Extensions::new();
    extensions.insert::<dyn ServiceLink>(link);
    extensions
}

/// Arguments of a command.
pub fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(command);
    for (name, value) in values {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

/// Arguments of `schedule.create` with the test pipeline as the action.
pub fn create_args(values: &[(&str, Value)]) -> BoundArgs {
    let mut args = args("schedule.create", values);
    args.insert("action", ArgValue::Pipeline(pipeline("true")), false);
    args
}

/// Plans and applies an effect stage; returns the output records.
pub fn apply(stage: &dyn EffectStage, args: &BoundArgs, tx: &mut TestTx, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let plan = stage.plan(args, StageInput::none(), ctx)?;
    let outcome = stage.apply(args, plan, tx, ctx)?;
    records(outcome.output.data)
}

/// Records of a stage output.
pub fn records(data: StageData<'_>) -> Result<Vec<Record>> {
    match data {
        StageData::Records(stream) => stream.collect(),
        _ => Ok(Vec::new()),
    }
}

/// Text field of a record.
pub fn text(record: &Record, name: &str) -> String {
    match record.get(name) {
        Some(Value::Text(text) | Value::Enum(text)) => text.clone(),
        other => panic!("field {name} is {other:?}"),
    }
}
