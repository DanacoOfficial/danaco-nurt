// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! A sample implementation of `first` runs on an in-memory record stream with
//! an execution context composed of test doubles.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::services::{FileListService, ServiceKind, Services, WalkIter, WalkSpec};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, DeliveredEvent, EffectPlan, EffectStage, Event,
    EventSink, ExecCtx, ExecFinished, ExecStarted, ExecStep, Extensions, InputKinds, Progress,
    ReadStage, RecordStream, StageData, StageDescriptor, StageFrame, StageImpl, StageInput,
    StageOutput, StageRegistrar,
};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};

fn entry(no: i64) -> Record {
    let mut record = Record::new(RecordKind::Entry);
    record.set("no", Value::Int(no));
    record
}

fn count_arg(count: i64) -> BoundArgs {
    BoundArgs::new("first").with("count", ArgValue::Value(Value::Int(count)), false)
}

struct VirtualClock {
    wall: Mutex<WallTime>,
    monotonic: Mutex<Duration>,
}

impl VirtualClock {
    fn new() -> Self {
        Self {
            wall: Mutex::new(WallTime::from_unix_nanos(1_700_000_000_000_000_000)),
            monotonic: Mutex::new(Duration::ZERO),
        }
    }

    fn advance(&self, by: Duration) {
        *self.monotonic.lock().unwrap() += by;
        let mut wall = self.wall.lock().unwrap();
        *wall = wall.saturating_add(by);
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        *self.wall.lock().unwrap()
    }

    fn monotonic(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }

    fn stamp(&self) -> ClockStamp {
        let monotonic = self.monotonic();
        ClockStamp {
            wall: self.now(),
            boot: BootId("test-boot".to_owned()),
            since_boot_with_suspend: monotonic,
            since_boot_active: monotonic,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        self.advance(duration);
        Ok(())
    }
}

struct TestHost {
    info: HostInfo,
    vars: HashMap<String, String>,
}

impl TestHost {
    fn new() -> Self {
        Self {
            info: HostInfo {
                os: Os::Linux,
                os_version: "6.8.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "test".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from("/home/user"),
                temp: PathBuf::from("/tmp"),
                path_style: PathStyle::Posix,
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 2,
            },
            vars: HashMap::from([("LANG".to_owned(), "C.UTF-8".to_owned())]),
        }
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        self.vars.get(name).map(OsString::from)
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.vars
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }
}

#[derive(Default)]
struct CollectEvents {
    events: Mutex<Vec<Event>>,
}

impl EventSink for CollectEvents {
    fn emit(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
}

struct First;

impl StageImpl for First {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "first".to_owned(),
            source: false,
            input: InputKinds::Any,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for First {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> nurt_contracts::error::Result<StageOutput<'a>> {
        let count = args.int("count")?.unwrap_or(0).max(0);
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        ctx.progress("take", 0, Some(u64::try_from(count).unwrap_or(u64::MAX)));
        input.map_records(|stream| RecordStream::new(stream.checked(ctx).take(count)))
    }
}

struct Plan;

impl StageImpl for Plan {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "touch".to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }
}

struct TestCtx {
    clock: Arc<VirtualClock>,
    events: Arc<CollectEvents>,
    ctx: ExecCtx,
}

fn test_ctx(timeout: Option<Duration>) -> TestCtx {
    let clock = Arc::new(VirtualClock::new());
    let events = Arc::new(CollectEvents::default());
    let mut builder = ExecCtx::builder(clock.clone(), Arc::new(TestHost::new()))
        .events(events.clone())
        .frame(StageFrame {
            command: "first".to_owned(),
            ..StageFrame::default()
        });
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    TestCtx {
        clock,
        events,
        ctx: builder.build(),
    }
}

fn records_of(output: StageOutput<'_>) -> Vec<Record> {
    match output.data {
        StageData::Records(stream) => stream.map(Result::unwrap).collect(),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn first_takes_the_leading_records_in_order() {
    let test = test_ctx(None);
    let args = count_arg(3);
    let input = StageInput::records(RecordStream::from_records((1..=5).map(entry).collect()));
    let stage: Arc<dyn StageImpl> = Arc::new(First);
    let read = stage.as_read().expect("first is a read stage");
    let output = read.execute(&args, input, &test.ctx).unwrap();
    assert_eq!(records_of(output), vec![entry(1), entry(2), entry(3)]);
    assert!(stage.as_effect().is_none());
}

#[test]
fn first_pulls_only_the_records_it_returns() {
    let test = test_ctx(None);
    let args = count_arg(2);
    let pulled = Arc::new(AtomicUsize::new(0));
    let counter = pulled.clone();
    let source = (1..=1_000_000).map(move |no| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(entry(no))
    });
    let input = StageInput::records(RecordStream::new(source));
    let output = First.execute(&args, input, &test.ctx).unwrap();
    assert_eq!(records_of(output).len(), 2);
    assert_eq!(pulled.load(Ordering::SeqCst), 2);
}

#[test]
fn first_keeps_header_and_order_of_the_input() {
    let test = test_ctx(None);
    let args = count_arg(1);
    let mut input = StageInput::records(RecordStream::from_records(vec![entry(1), entry(2)]));
    input.header = Some(entry(0));
    let output = First.execute(&args, input, &test.ctx).unwrap();
    assert_eq!(output.header, Some(entry(0)));
}

#[test]
fn first_without_input_records_fails() {
    let test = test_ctx(None);
    let args = count_arg(1);
    assert!(First.execute(&args, StageInput::none(), &test.ctx).is_err());
}

#[test]
fn cancellation_ends_the_stream_with_one_error() {
    let test = test_ctx(None);
    let args = count_arg(10);
    let input = StageInput::records(RecordStream::from_records((1..=5).map(entry).collect()));
    let output = First.execute(&args, input, &test.ctx).unwrap();
    let StageData::Records(mut stream) = output.data else {
        panic!("expected records");
    };
    assert_eq!(stream.next().unwrap().unwrap(), entry(1));
    test.ctx.cancel().cancel(CancelReason::Requested);
    assert!(stream.next().unwrap().is_err());
    assert!(stream.next().is_none());
}

#[test]
fn passing_the_deadline_ends_the_stream() {
    let test = test_ctx(Some(Duration::from_secs(5)));
    let args = count_arg(10);
    let input = StageInput::records(RecordStream::from_records((1..=5).map(entry).collect()));
    let output = First.execute(&args, input, &test.ctx).unwrap();
    let StageData::Records(mut stream) = output.data else {
        panic!("expected records");
    };
    assert!(stream.next().unwrap().is_ok());
    test.clock.advance(Duration::from_secs(6));
    assert!(stream.next().unwrap().is_err());
    assert_eq!(test.ctx.cancel().reason(), Some(CancelReason::Timeout));
}

#[test]
fn sleep_stops_at_the_deadline_on_a_virtual_clock() {
    let test = test_ctx(Some(Duration::from_secs(5)));
    test.ctx.sleep(Duration::from_secs(2)).unwrap();
    assert_eq!(test.ctx.remaining(), Some(Duration::from_secs(3)));
    assert!(test.ctx.sleep(Duration::from_secs(60)).is_err());
    assert_eq!(test.clock.monotonic(), Duration::from_secs(5));
    assert_eq!(test.ctx.cancel().reason(), Some(CancelReason::Timeout));
}

#[test]
fn child_tokens_follow_their_parent() {
    let root = CancelToken::new();
    let job = root.child();
    let grandchild = job.child();
    assert!(!grandchild.is_cancelled());
    root.cancel(CancelReason::Shutdown);
    assert_eq!(grandchild.reason(), Some(CancelReason::Shutdown));
    assert_eq!(
        job.wait_timeout(Duration::from_secs(10)),
        Some(CancelReason::Shutdown)
    );
    let late = root.child();
    assert!(late.is_cancelled());
    assert!(late.check().is_err());
}

#[test]
fn waiting_on_a_token_wakes_up_on_cancel() {
    let token = CancelToken::new();
    let other = token.clone();
    let waiter = std::thread::spawn(move || other.wait_timeout(Duration::from_secs(60)));
    token.cancel(CancelReason::Requested);
    assert_eq!(waiter.join().unwrap(), Some(CancelReason::Requested));
    assert_eq!(
        CancelToken::new().wait_timeout(Duration::from_millis(1)),
        None
    );
}

#[test]
fn progress_events_reach_the_sink() {
    let test = test_ctx(None);
    let args = count_arg(4);
    let input = StageInput::records(RecordStream::empty());
    let _ = records_of(First.execute(&args, input, &test.ctx).unwrap());
    let events = test.events.events.lock().unwrap();
    assert_eq!(
        events.as_slice(),
        [Event::Progress(Progress {
            command: "first".to_owned(),
            phase: "take".to_owned(),
            done: 0,
            total: Some(4),
            bytes: None,
            message: None,
        })]
    );
}

#[test]
fn events_have_a_type_member_and_round_trip() {
    let event = Event::Progress(Progress {
        command: "git.clone".to_owned(),
        phase: "receive".to_owned(),
        done: 10,
        total: Some(40),
        bytes: Some(4096),
        message: None,
    });
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["t"], "progress");
    assert_eq!(json["done"], 10);
    let back: Event = serde_json::from_value(json).unwrap();
    assert_eq!(back, event);
}

#[test]
fn delivered_events_carry_sequence_time_and_request() {
    let started = Event::ExecStarted(ExecStarted {
        ast_hash: Some("sha256:00".to_owned()),
        effect: EffectKind::Local,
    });
    let json = serde_json::to_value(&started).unwrap();
    assert_eq!(json["t"], "exec.started");
    assert_eq!(json["effect"], "local");

    let finished = DeliveredEvent {
        seq: 7,
        time: Instant::from_unix_nanos(1_500_000_000),
        request: Some("req-1".to_owned()),
        event: Event::ExecFinished(ExecFinished {
            status: ReportStatus::Failed,
            error: Some(ErrorCode::Stale),
        }),
    };
    let json = serde_json::to_value(&finished).unwrap();
    assert_eq!(json["t"], "exec.finished");
    assert_eq!(json["seq"], 7);
    assert_eq!(json["time"], "1970-01-01T00:00:01.500Z");
    assert_eq!(json["request"], "req-1");
    assert_eq!(json["status"], "failed");
    assert_eq!(json["error"], "E_STALE");
    let back: DeliveredEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, finished);
}

struct Named(&'static str);

impl StageImpl for Named {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.0.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }
}

fn register_group(registrar: &mut dyn StageRegistrar) -> nurt_contracts::error::Result<()> {
    registrar.register(Arc::new(Named("git.status")))?;
    registrar.register(Arc::new(Named("git.diff")))
}

struct EmptyCatalog;

impl StageRegistrar for EmptyCatalog {
    fn register(&mut self, stage: Arc<dyn StageImpl>) -> nurt_contracts::error::Result<()> {
        Err(Error::new(
            ErrorCode::UnknownCommand,
            format!("unknown command `{}`", stage.descriptor().command),
        ))
    }
}

#[test]
fn command_crates_register_stages_through_the_registrar() {
    let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
    register_group(&mut stages).unwrap();
    let commands: Vec<String> = stages
        .iter()
        .map(|stage| stage.descriptor().command)
        .collect();
    assert_eq!(commands, ["git.status", "git.diff"]);

    let error = register_group(&mut EmptyCatalog).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
}

#[test]
fn execution_steps_serialize_as_step_numbers() {
    assert_eq!(serde_json::to_value(ExecStep::After).unwrap(), "5a");
    assert_eq!(serde_json::to_value(ExecStep::Plan).unwrap(), "4");
    let step: ExecStep = serde_json::from_value(serde_json::json!("6")).unwrap();
    assert_eq!(step, ExecStep::Verify);
    assert!(ExecStep::Apply.changes_world());
    assert!(!ExecStep::After.changes_world());
}

#[test]
fn per_stage_context_shares_cancellation_and_notes() {
    let test = test_ctx(None);
    let stage_ctx = test.ctx.for_stage(StageFrame {
        index: 2,
        command: "sort".to_owned(),
        step: ExecStep::After,
        ..StageFrame::default()
    });
    assert_eq!(stage_ctx.step(), ExecStep::After);
    assert_eq!(stage_ctx.frame().index, 2);
    stage_ctx.cancel().cancel(CancelReason::Requested);
    assert!(test.ctx.checkpoint().is_err());
    assert!(stage_ctx.fs().is_none());
    assert!(stage_ctx.require_fs().is_err());
    assert!(stage_ctx.evaluator().is_err());
    assert!(stage_ctx.take_notes().is_empty());
    assert_eq!(
        stage_ctx.host().env_var("LANG"),
        Some(OsString::from("C.UTF-8"))
    );
}

#[test]
fn bound_args_report_given_and_defaulted_values() {
    let args = BoundArgs::new("find")
        .with("depth", ArgValue::Value(Value::Int(2)), false)
        .with("hidden", ArgValue::Value(Value::Bool(true)), true)
        .with("glob", ArgValue::Absent, false);
    assert_eq!(
        args.names().collect::<Vec<_>>(),
        ["depth", "hidden", "glob"]
    );
    assert!(args.is_given("depth"));
    assert!(!args.is_given("hidden"));
    assert!(args.is_defaulted("hidden"));
    assert!(!args.is_given("glob"));
    assert_eq!(args.int("depth").unwrap(), Some(2));
    assert_eq!(args.bool("hidden").unwrap(), Some(true));
    assert!(args.text("depth").is_err());
    assert!(args.require("glob").is_err());
    let json = serde_json::to_value(&args).unwrap();
    let back: BoundArgs = serde_json::from_value(json).unwrap();
    assert_eq!(back, args);
}

#[test]
fn records_beyond_the_limit_are_an_error() {
    let stream = RecordStream::from_records((1..=3).map(entry).collect());
    assert!(stream.collect_bounded(2).is_err());
    let stream = RecordStream::from_records((1..=3).map(entry).collect());
    assert_eq!(stream.collect_bounded(3).unwrap().len(), 3);
}

#[test]
fn effect_plans_carry_private_state() {
    #[derive(Debug, PartialEq)]
    struct Targets(Vec<String>);

    let mut plan = EffectPlan::new().with_state(Targets(vec!["/tmp/a".to_owned()]));
    assert!(plan.state::<String>().is_none());
    assert_eq!(
        plan.state::<Targets>(),
        Some(&Targets(vec!["/tmp/a".to_owned()]))
    );
    assert!(plan.take_state::<String>().is_none());
    assert_eq!(
        plan.take_state::<Targets>(),
        Some(Targets(vec!["/tmp/a".to_owned()]))
    );
    assert!(plan.state::<Targets>().is_none());
    let stage: Box<dyn StageImpl> = Box::new(Plan);
    assert!(stage.as_read().is_none());
    assert!(stage.as_effect().is_none());
    assert!(stage.descriptor().input.accepts(RecordKind::Entry));
}

/// File listing service that finds no files.
struct NoFiles;

impl FileListService for NoFiles {
    fn walk<'a>(&'a self, _spec: &WalkSpec) -> nurt_contracts::error::Result<WalkIter<'a>> {
        Ok(Box::new(std::iter::empty()))
    }
}

#[test]
fn service_accessors_delegate_to_the_registry() {
    let test = test_ctx(None);
    assert!(test.ctx.services().available().is_empty());
    assert!(test.ctx.files().is_none());

    let services = Services::new().with_file_list(Arc::new(NoFiles));
    let ctx = ExecCtx::builder(Arc::new(VirtualClock::new()), Arc::new(TestHost::new()))
        .services(Arc::new(services))
        .build();
    assert_eq!(ctx.services().available(), [ServiceKind::FileList]);
    assert!(ctx.files().is_some());
    assert!(ctx.doc().is_none());
    assert!(ctx.code().is_none());
    assert!(ctx.process().is_none());
    let stage_ctx = ctx.for_stage(StageFrame::default());
    assert!(stage_ctx.services().has(ServiceKind::FileList));
}

#[test]
fn extensions_hold_trait_objects() {
    trait Counter: Send + Sync {
        fn value(&self) -> u32;
    }
    struct Seven;
    impl Counter for Seven {
        fn value(&self) -> u32 {
            7
        }
    }

    let mut extensions = Extensions::new();
    assert!(extensions.is_empty());
    extensions.insert::<dyn Counter>(Arc::new(Seven));
    assert!(extensions.contains::<dyn Counter>());
    assert_eq!(extensions.get::<dyn Counter>().map(|c| c.value()), Some(7));
    assert!(extensions.get::<String>().is_none());
    let ctx = ExecCtx::builder(Arc::new(VirtualClock::new()), Arc::new(TestHost::new()))
        .extensions(extensions)
        .build();
    assert_eq!(ctx.extensions().len(), 1);
}

#[test]
fn interfaces_are_thread_safe_objects() {
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn StageImpl>();
    assert_send_sync::<dyn ReadStage>();
    assert_send_sync::<dyn EffectStage>();
    assert_send_sync::<dyn EventSink>();
    assert_send_sync::<dyn Clock>();
    assert_send_sync::<dyn Host>();
    assert_send_sync::<ExecCtx>();
    assert_send_sync::<CancelToken>();
    fn assert_send<T: Send>() {}
    assert_send::<RecordStream<'static>>();
    assert_send::<StageInput<'static>>();
    assert_send::<StageOutput<'static>>();
}
