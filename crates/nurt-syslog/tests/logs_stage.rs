#![allow(
// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! The stage of `logs` over journal files produced by the test writer and
//! over a scripted store.

#[path = "support/journal_writer.rs"]
mod journal_writer;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant as StdInstant, SystemTime};

use journal_writer::{Entry, Options, Writer, sample_entries, system_root};
use nurt_contracts::config::Config;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::LogLevel;
use nurt_contracts::report::Notable;
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, CheckCtx, Demand, Event, EventSink, ExecCtx,
    InputKinds, StageData, StageFrame, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Instant, Record, RecordKind, Regex as RegexValue, Value};
use nurt_syslog::cursor::Cursor;
use nurt_syslog::event::{LogEvent, StoreKind};
use nurt_syslog::journald::JournaldStore;
use nurt_syslog::limits::{DEFAULT_LIMIT, LogLimits};
use nurt_syslog::query::{LogNotice, LogQuery, LogStore, LogStream, NoticeCode, Notices};
use nurt_syslog::stage::{COMMAND, FIRST_PAGE, LogsStage};

/// Reception time of the first sample entry, in microseconds.
const START: u64 = 1_700_000_000_000_000;
/// Distance between sample entries, in microseconds.
const STEP: u64 = 1_000;
const NANOS_PER_MICRO: u64 = 1_000;
/// Sample entries of every test journal unless stated otherwise.
const ENTRIES: u64 = 16;
/// Priority of the sample entries that `level: info` excludes.
const DEBUG_PRIORITY: u64 = 7;
const PRIORITIES: u64 = 8;

struct SystemClock {
    origin: StdInstant,
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        WallTime::from_unix_nanos(i128::try_from(nanos).unwrap_or(i128::MAX))
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(
        &self,
        duration: Duration,
        cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        match cancel.wait_timeout(duration) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

struct TestHost(HostInfo);

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

fn host() -> Arc<TestHost> {
    Arc::new(TestHost(HostInfo {
        os: Os::Linux,
        os_version: "6.8.0".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "host".to_owned(),
        user: "user".to_owned(),
        home: "/home/user".into(),
        temp: std::env::temp_dir(),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    }))
}

/// Diagnostic events collected by a test.
#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Log(log) => Some(log.message.clone()),
                _ => None,
            })
            .collect()
    }

    fn starting_with(&self, code: NoticeCode) -> Vec<String> {
        self.messages()
            .into_iter()
            .filter(|message| message.starts_with(code.as_str()))
            .collect()
    }
}

struct Harness {
    ctx: ExecCtx,
    events: Arc<Events>,
}

fn harness_with(cancel: CancelToken, used: Option<u64>) -> Harness {
    let events = Arc::new(Events::default());
    let frame = StageFrame {
        command: COMMAND.to_owned(),
        demand: Demand {
            limit: used,
            ..Demand::default()
        },
        ..StageFrame::default()
    };
    let clock = Arc::new(SystemClock {
        origin: StdInstant::now(),
    });
    let ctx = ExecCtx::builder(clock, host())
        .cancel(cancel)
        .events(events.clone())
        .frame(frame)
        .build();
    Harness { ctx, events }
}

fn harness() -> Harness {
    harness_with(CancelToken::new(), None)
}

/// Arguments as the binder gives them: every parameter of the catalog, with
/// the defaults of `level` and `limit`, then the given values.
fn args(given: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(COMMAND);
    for name in ["source", "unit", "since", "until", "message"] {
        args.insert(name, ArgValue::Absent, false);
    }
    args.insert(
        "level",
        ArgValue::Value(Value::Enum("info".to_owned())),
        true,
    );
    args.insert(
        "limit",
        ArgValue::Value(Value::Int(i64::from(DEFAULT_LIMIT))),
        true,
    );
    for (name, value) in given {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn level(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

fn at(micros: u64) -> Value {
    Value::Instant(Instant::from_unix_nanos(
        i64::try_from(micros * NANOS_PER_MICRO).unwrap(),
    ))
}

fn execute<'a>(
    stage: &'a LogsStage,
    args: &'a BoundArgs,
    ctx: &'a ExecCtx,
) -> Result<StageOutput<'a>> {
    stage
        .as_read()
        .expect("`logs` has a read interface")
        .execute(args, StageInput::none(), ctx)
}

fn run(stage: &LogsStage, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    match execute(stage, args, ctx)?.data {
        StageData::Records(stream) => stream.collect(),
        other => panic!("expected records, got {other:?}"),
    }
}

fn field(record: &Record, name: &str) -> Option<String> {
    match record.get(name) {
        Some(Value::Text(value)) => Some(value.clone()),
        _ => None,
    }
}

fn messages(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| field(record, "message").unwrap())
        .collect()
}

fn names(indexes: impl IntoIterator<Item = u64>) -> Vec<String> {
    indexes
        .into_iter()
        .map(|index| format!("event {index}"))
        .collect()
}

fn journal(entries: &[Entry]) -> Vec<u8> {
    Writer::build(Options::default(), entries)
}

/// System root whose journal directory holds the given files.
fn root_with(files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let directory = system_root(root.path(), files);
    (root, directory)
}

fn sample_root() -> (tempfile::TempDir, PathBuf) {
    root_with(&[("system.journal", journal(&sample_entries(ENTRIES, START)))])
}

fn stage_over(root: &Path) -> LogsStage {
    let store = JournaldStore::new(root, LogLimits::default());
    LogsStage::with_store(Arc::new(store), LogLimits::default())
}

#[cfg(unix)]
fn denied(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    fs::File::open(path).is_err()
}

#[test]
fn returns_newest_events_first_with_catalog_defaults() {
    let (root, _) = sample_root();
    let h = harness();
    let records = run(&stage_over(root.path()), &args(&[]), &h.ctx).unwrap();
    let expected = (0..ENTRIES)
        .rev()
        .filter(|index| index % PRIORITIES != DEBUG_PRIORITY);
    assert_eq!(messages(&records), names(expected));
    assert!(
        records
            .iter()
            .all(|record| record.kind == RecordKind::LogEvent)
    );
    assert!(field(&records[0], "cursor").unwrap().starts_with("v1:j:"));
    assert!(h.events.messages().is_empty());
    assert!(h.ctx.take_notes().is_empty());
}

#[test]
fn filters_by_unit_source_level_and_message() {
    let (root, _) = sample_root();
    let stage = stage_over(root.path());
    let h = harness();
    let by_unit = run(
        &stage,
        &args(&[("unit", text("ssh")), ("level", level("warning"))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&by_unit), names([12, 10, 8, 4, 2, 0]));
    let pattern = RegexValue::new("^event 1[13]$", "").unwrap();
    let by_source = run(
        &stage,
        &args(&[("source", text("cron")), ("message", Value::Regex(pattern))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&by_source), names([13, 11]));
    let by_glob = run(
        &stage,
        &args(&[("unit", text("*.service")), ("level", level("debug"))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&by_glob), names((0..ENTRIES).rev()));
}

#[test]
fn unit_parameter_equals_filter_on_unit_field() {
    let (root, _) = sample_root();
    let stage = stage_over(root.path());
    let h = harness();
    let all = run(&stage, &args(&[("level", level("debug"))]), &h.ctx).unwrap();
    let filtered: Vec<Record> = all
        .into_iter()
        .filter(|record| field(record, "unit").as_deref() == Some("cron.service"))
        .collect();
    let direct = run(
        &stage,
        &args(&[("unit", text("cron.service")), ("level", level("debug"))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&direct), messages(&filtered));
    assert_eq!(direct.len(), 8);
}

#[test]
fn since_is_inclusive_and_until_exclusive() {
    let (root, _) = sample_root();
    let h = harness();
    let records = run(
        &stage_over(root.path()),
        &args(&[
            ("since", at(START + 4 * STEP)),
            ("until", at(START + 10 * STEP)),
            ("level", level("debug")),
        ]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&records), names((4..10).rev()));
}

#[test]
fn limit_ends_output_and_cursor_continues_after_last_event() {
    let (root, _) = sample_root();
    let stage = stage_over(root.path());
    let h = harness();
    let first = run(
        &stage,
        &args(&[("limit", Value::Int(3)), ("level", level("debug"))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&first), names([15, 14, 13]));
    let cursor = field(&first[2], "cursor").unwrap();
    let reached = h.events.starting_with(NoticeCode::LimitReached);
    assert_eq!(reached.len(), 1);
    assert!(reached[0].contains("more than 3 events"));
    assert!(reached[0].contains(&cursor));
    let next = run(
        &stage,
        &args(&[
            ("limit", Value::Int(3)),
            ("level", level("debug")),
            ("cursor", text(&cursor)),
        ]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&next), names([12, 11, 10]));
}

#[test]
fn limit_outside_range_fails_with_e_limit() {
    let (root, _) = sample_root();
    let stage = stage_over(root.path());
    let h = harness();
    for value in [0, 100_001] {
        let error = run(&stage, &args(&[("limit", Value::Int(value))]), &h.ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit, "{value}");
    }
}

#[test]
fn pages_of_the_store_join_without_gaps_or_repeats() {
    let count = u64::from(FIRST_PAGE) * 2 + 500;
    let (root, _) = root_with(&[("system.journal", journal(&sample_entries(count, START)))]);
    let h = harness();
    let records = run(
        &stage_over(root.path()),
        &args(&[
            ("limit", Value::Int(i64::try_from(count).unwrap())),
            ("level", level("debug")),
        ]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&records), names((0..count).rev()));
    assert!(h.events.starting_with(NoticeCode::LimitReached).is_empty());
}

#[test]
fn small_demand_reads_small_pages_and_reports_each_notice_once() {
    let (root, _) = root_with(&[
        ("system.journal", journal(&sample_entries(ENTRIES, START))),
        ("user-1000.journal", b"not a journal file".to_vec()),
    ]);
    let h = harness_with(CancelToken::new(), Some(2));
    let records = run(
        &stage_over(root.path()),
        &args(&[("level", level("debug"))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&records), names((0..ENTRIES).rev()));
    let skipped = h.events.starting_with(NoticeCode::FileSkipped);
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].contains("user-1000.journal"));
}

#[test]
fn unknown_unit_gives_empty_result_and_notice() {
    let (root, _) = sample_root();
    let h = harness();
    let records = run(
        &stage_over(root.path()),
        &args(&[("unit", text("ssd"))]),
        &h.ctx,
    )
    .unwrap();
    assert!(records.is_empty());
    assert_eq!(h.events.starting_with(NoticeCode::UnknownName).len(), 1);
}

#[test]
fn missing_journal_fails_with_e_not_found() {
    let root = tempfile::tempdir().unwrap();
    let h = harness();
    let error = run(&stage_over(root.path()), &args(&[]), &h.ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn only_damaged_files_fail_with_e_format() {
    let (root, _) = root_with(&[("system.journal", vec![0x5a; 512])]);
    let h = harness();
    let error = run(&stage_over(root.path()), &args(&[]), &h.ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Format);
}

#[test]
fn notices_are_reported_when_output_is_dropped_early() {
    let (root, _) = root_with(&[
        ("system.journal", journal(&sample_entries(ENTRIES, START))),
        ("user-1000.journal", vec![0x5a; 512]),
    ]);
    let stage = stage_over(root.path());
    let h = harness();
    let args = args(&[]);
    let output = execute(&stage, &args, &h.ctx).unwrap();
    let StageData::Records(mut stream) = output.data else {
        panic!("expected records");
    };
    assert!(stream.next().unwrap().is_ok());
    assert!(h.events.messages().is_empty());
    drop(stream);
    assert_eq!(h.events.starting_with(NoticeCode::FileSkipped).len(), 1);
}

#[cfg(unix)]
#[test]
fn unreadable_journal_files_are_skipped_with_notable_entry() {
    let (root, directory) = root_with(&[
        ("system.journal", journal(&sample_entries(ENTRIES, START))),
        (
            "user-1000.journal",
            journal(&sample_entries(ENTRIES, START + ENTRIES * STEP)),
        ),
    ]);
    if !denied(&directory.join("user-1000.journal")) {
        return;
    }
    let h = harness();
    let records = run(
        &stage_over(root.path()),
        &args(&[("level", level("debug"))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&records), names((0..ENTRIES).rev()));
    let notes = h.ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::PermissionDeniedSkipped);
    assert_eq!(notes[0].count, Some(1));
}

#[cfg(unix)]
#[test]
fn no_readable_journal_file_fails_with_e_permission() {
    let (root, directory) = sample_root();
    if !denied(&directory.join("system.journal")) {
        return;
    }
    let h = harness();
    let error = run(&stage_over(root.path()), &args(&[]), &h.ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Permission);
}

#[test]
fn cancellation_ends_output_between_events() {
    let (root, _) = sample_root();
    let stage = stage_over(root.path());
    let token = CancelToken::new();
    let h = harness_with(token.clone(), None);
    let args = args(&[]);
    let output = execute(&stage, &args, &h.ctx).unwrap();
    let StageData::Records(mut stream) = output.data else {
        panic!("expected records");
    };
    assert!(stream.next().unwrap().is_ok());
    token.cancel(CancelReason::Requested);
    let error = stream.next().unwrap().unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert!(stream.next().is_none());
}

#[test]
fn cancelled_execution_fails_before_first_record() {
    let (root, _) = sample_root();
    let token = CancelToken::new();
    token.cancel(CancelReason::Requested);
    let h = harness_with(token, None);
    let error = run(&stage_over(root.path()), &args(&[]), &h.ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

/// Store over fixed events of one channel, newest first, that adds the same
/// notices to every query and records the limit and cursor of each query.
struct ScriptedStore {
    events: u64,
    notices: Vec<LogNotice>,
    failure: Option<ErrorCode>,
    error_after: Option<u64>,
    queries: Mutex<Vec<(u32, Option<String>)>>,
}

impl ScriptedStore {
    fn new(events: u64) -> Self {
        Self {
            events,
            notices: Vec::new(),
            failure: None,
            error_after: None,
            queries: Mutex::new(Vec::new()),
        }
    }

    fn event(record_id: u64) -> LogEvent {
        let micros = START + record_id * STEP;
        LogEvent::new(
            Instant::from_unix_nanos(i64::try_from(micros * NANOS_PER_MICRO).unwrap()),
            "app",
            6,
            &format!("event {record_id}"),
            StoreKind::EventLog,
            Cursor::EventLog {
                channel: "System".to_owned(),
                record_id,
            },
        )
    }
}

impl LogStore for ScriptedStore {
    fn query<'a>(&'a self, query: &'a LogQuery) -> Result<LogStream<'a>> {
        self.queries
            .lock()
            .unwrap()
            .push((query.limit, query.cursor.as_ref().map(ToString::to_string)));
        if let Some(code) = self.failure {
            return Err(Error::new(code, "the store is not available"));
        }
        let start = match &query.cursor {
            Some(Cursor::EventLog { record_id, .. }) => *record_id,
            _ => self.events,
        };
        let notices = Notices::new();
        for notice in &self.notices {
            notices.add(notice.clone());
        }
        let error_after = self.error_after;
        let events = (0..start).rev().map(move |record_id| {
            if error_after.is_some_and(|after| record_id < after) {
                Err(Error::new(ErrorCode::Io, "reading the channel failed"))
            } else {
                Ok(Self::event(record_id))
            }
        });
        Ok(LogStream::new(events, notices, query.limit))
    }
}

fn notice(code: NoticeCode, path: Option<&str>, detail: Option<&str>, count: u64) -> LogNotice {
    let mut notice = LogNotice::new(code);
    notice.path = path.map(str::to_owned);
    notice.detail = detail.map(str::to_owned);
    notice.count = Some(count);
    notice
}

fn scripted(store: &Arc<ScriptedStore>) -> LogsStage {
    let shared: Arc<dyn LogStore + Send + Sync> = store.clone();
    LogsStage::with_store(shared, LogLimits::default())
}

#[test]
fn pages_double_and_merge_notices_of_repeated_queries() {
    let mut store = ScriptedStore::new(20);
    store.notices = vec![
        notice(NoticeCode::Unformatted, None, None, 1),
        notice(
            NoticeCode::FileSkipped,
            Some("/j/a.journal"),
            Some("corrupt"),
            1,
        ),
        notice(NoticeCode::UnknownName, None, Some("sshd"), 1),
        notice(NoticeCode::CursorApproximate, None, None, 1),
        notice(NoticeCode::PermissionDeniedSkipped, None, None, 2),
        notice(NoticeCode::EventsLost, None, None, 3),
    ];
    let store = Arc::new(store);
    let h = harness_with(CancelToken::new(), Some(2));
    let records = run(&scripted(&store), &args(&[]), &h.ctx).unwrap();
    assert_eq!(messages(&records), names((0..20).rev()));
    let queries = store.queries.lock().unwrap().clone();
    let limits: Vec<u32> = queries.iter().map(|(limit, _)| *limit).collect();
    assert_eq!(limits, [2, 4, 8, 16]);
    assert_eq!(queries[0].1, None);
    assert_eq!(queries[1].1.as_deref(), Some("v1:w:System:18"));
    assert_eq!(queries[3].1.as_deref(), Some("v1:w:System:6"));
    let unformatted = h.events.starting_with(NoticeCode::Unformatted);
    assert_eq!(unformatted, ["log_unformatted (4)"]);
    for code in [
        NoticeCode::FileSkipped,
        NoticeCode::UnknownName,
        NoticeCode::CursorApproximate,
        NoticeCode::EventsLost,
    ] {
        assert_eq!(h.events.starting_with(code).len(), 1, "{code:?}");
    }
    assert!(h.events.starting_with(NoticeCode::LimitReached).is_empty());
    let notes = h.ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::PermissionDeniedSkipped);
    assert_eq!(notes[0].count, Some(2));
}

#[test]
fn limit_of_the_execution_ends_the_last_page() {
    let store = Arc::new(ScriptedStore::new(20));
    let h = harness_with(CancelToken::new(), Some(2));
    let records = run(
        &scripted(&store),
        &args(&[("limit", Value::Int(5))]),
        &h.ctx,
    )
    .unwrap();
    assert_eq!(messages(&records), names([19, 18, 17, 16, 15]));
    let limits: Vec<u32> = store
        .queries
        .lock()
        .unwrap()
        .iter()
        .map(|(limit, _)| *limit)
        .collect();
    assert_eq!(limits, [2, 3]);
    let reached = h.events.starting_with(NoticeCode::LimitReached);
    assert_eq!(reached.len(), 1);
    assert!(reached[0].contains("more than 5 events"));
    assert!(reached[0].contains("v1:w:System:15"));
}

#[test]
fn store_errors_end_execution_before_first_record() {
    for code in [
        ErrorCode::Permission,
        ErrorCode::Privilege,
        ErrorCode::NotFound,
        ErrorCode::Format,
    ] {
        let mut store = ScriptedStore::new(4);
        store.failure = Some(code);
        let h = harness();
        let error = run(&scripted(&Arc::new(store)), &args(&[]), &h.ctx).unwrap_err();
        assert_eq!(error.code(), code);
    }
}

#[test]
fn stream_error_follows_the_events_read_before_it() {
    let mut store = ScriptedStore::new(6);
    store.error_after = Some(4);
    let store = Arc::new(store);
    let h = harness();
    let stage = scripted(&store);
    let args = args(&[]);
    let output = execute(&stage, &args, &h.ctx).unwrap();
    let StageData::Records(stream) = output.data else {
        panic!("expected records");
    };
    let items: Vec<Result<Record>> = stream.collect();
    assert_eq!(items.len(), 3);
    assert_eq!(
        field(items[1].as_ref().unwrap(), "message").unwrap(),
        "event 4"
    );
    assert_eq!(items[2].as_ref().unwrap_err().code(), ErrorCode::Io);
}

#[test]
fn error_of_a_later_page_ends_the_output() {
    let mut store = ScriptedStore::new(10);
    store.error_after = Some(5);
    let store = Arc::new(store);
    let h = harness_with(CancelToken::new(), Some(2));
    let stage = scripted(&store);
    let args = args(&[]);
    let output = execute(&stage, &args, &h.ctx).unwrap();
    let StageData::Records(stream) = output.data else {
        panic!("expected records");
    };
    let items: Vec<Result<Record>> = stream.collect();
    assert_eq!(items.len(), 6);
    assert!(items[..5].iter().all(Result::is_ok));
    assert_eq!(items[5].as_ref().unwrap_err().code(), ErrorCode::Io);
}

fn spec() -> StageSpec {
    serde_json::from_value(serde_json::json!({
        "name": COMMAND,
        "category": "source",
        "reversibility": "read",
        "effect": "read",
        "implementation": "builtin",
        "input": ["none"],
        "output": ["log_event"],
        "summary": "Read events from the system log.",
    }))
    .unwrap()
}

#[test]
fn static_check_rejects_invalid_arguments() {
    let stage = LogsStage::with_store(Arc::new(ScriptedStore::new(0)), LogLimits::default());
    let spec = spec();
    let host = host();
    let config = Config::default();
    let ctx = CheckCtx::new(&spec, host.as_ref(), &config);
    assert!(stage.check(&args(&[]), &ctx).is_ok());
    let cases = [
        ("limit", Value::Int(0), ErrorCode::Limit),
        ("message", text("("), ErrorCode::Regex),
        ("level", level("verbose"), ErrorCode::InvalidValue),
        ("unit", text("ssh["), ErrorCode::PatternSyntax),
        ("since", text("yesterday"), ErrorCode::Type),
    ];
    for (name, value, code) in cases {
        let error = stage.check(&args(&[(name, value)]), &ctx).unwrap_err();
        assert_eq!(error.code(), code, "{name}");
    }
}

#[test]
fn stage_is_a_source_of_log_events() {
    let stage = LogsStage::default();
    let descriptor = stage.descriptor();
    assert_eq!(descriptor.command, COMMAND);
    assert!(descriptor.source);
    assert_eq!(descriptor.input, InputKinds::None);
    assert!(stage.as_effect().is_none());
    assert!(format!("{stage:?}").contains("LogsStage"));
}

/// Parameters of `logs` in `catalog/system.toml` as `(name, type, default)`.
fn catalog_parameters() -> Vec<(String, String, Option<String>)> {
    let path = format!(
        "{}/../../catalog/system.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let catalog = fs::read_to_string(path).unwrap();
    let mut command = String::new();
    let mut in_param = false;
    let mut params: Vec<(String, String, Option<String>)> = Vec::new();
    for line in catalog.lines().map(str::trim) {
        if line == "[[command]]" {
            command.clear();
            in_param = false;
            continue;
        }
        if line == "[[command.param]]" {
            in_param = true;
            if command == COMMAND {
                params.push((String::new(), String::new(), None));
            }
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        let value = value.trim_matches('"').to_owned();
        match (in_param, key) {
            (false, "name") => command = value,
            (true, _) if command == COMMAND => {
                let param = params.last_mut().unwrap();
                match key {
                    "name" => param.0 = value,
                    "type" => param.1 = value,
                    "default" => param.2 = Some(value),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    params
}

#[test]
fn parameters_and_defaults_match_the_catalog() {
    let params = catalog_parameters();
    let declared: Vec<(&str, &str)> = params
        .iter()
        .map(|(name, kind, _)| (name.as_str(), kind.as_str()))
        .collect();
    assert_eq!(
        declared,
        [
            ("source", "text"),
            ("unit", "text"),
            ("level", "enum(critical|error|warning|info|debug)"),
            ("since", "instant"),
            ("until", "instant"),
            ("message", "regex"),
            ("limit", "int"),
        ]
    );
    let default_of = |wanted: &str| {
        params
            .iter()
            .find(|(name, _, _)| name == wanted)
            .and_then(|(_, _, default)| default.clone())
    };
    let defaults = LogQuery::default();
    assert_eq!(default_of("level").as_deref(), Some("info"));
    assert_eq!(defaults.level, LogLevel::Info);
    assert_eq!(default_of("limit"), Some(defaults.limit.to_string()));
    for (name, kind, _) in &params {
        if let Some(values) = kind
            .strip_prefix("enum(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            for value in values.split('|') {
                let given = args(&[(name, level(value))]);
                assert!(
                    LogQuery::from_args(&given, &LogLimits::default()).is_ok(),
                    "{value}"
                );
            }
        }
    }
}
