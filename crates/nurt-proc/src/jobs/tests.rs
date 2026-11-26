// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the job table, the run cycle, the supervisor and the job
//! commands, with test doubles of process control, output capture and
//! change detection.
//!
//! The test program is described by its arguments: `run <milliseconds>
//! <exit code> <lines> [<gate>]` runs for the given time, writes the given
//! number of lines `line 1`, `line 2`, … to standard output at its start and,
//! with an exit code other than 0, the line `failure` to standard error.
//! With a gate file it runs at least until that file exists, so a test
//! decides when it ends. The program `missing` cannot be started.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nurt_contracts::config::{Config, DurationSetting, Supervisor};
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{MutationTx, OpKind};
use nurt_contracts::record::{
    Change, ChangeAction, Effect, Exec, ExecShown, Job, JobState, KilledBy, Launcher, Signal,
    Tracking,
};
use nurt_contracts::report::{Irreversible, Notable};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, Event, EventSink, ExecCtx, Extensions,
    OutputStream, RecordStream, StageImpl, StageInput,
};
use nurt_contracts::value::{self, Record, RecordKind, Value};

use super::control::{self, ControlAction};
use super::host::LocalHost;
use super::journal::Unjournaled;
use super::runner::Launch;
use super::store::JobStore;
use super::table::{JobFile, JobTable, PROTOCOL};
use super::wire::StoredRequest;
use super::{Supervision, SupervisorCommand, run_host, supervisor};
use crate::iface::{
    ChangeTracker, Components, OutputSink, OutputStore, OutputView, PreparedScan, Priority,
    ProcessControl, ProcessEnd, ProcessLimits, RenderedOutput, ResourceUsage, RunHost, RunKind,
    RunRequest, RunWait, RunningProcess, ShowMode, SpawnRequest, SpoolFactory, StdinSource,
    TrackMode, TrackOutcome, TrackSpec,
};

/// Variable that carries the state directory to the test processes.
const STATE_ENV: &str = "NURT_JOBS_TEST_STATE";

/// Longest time a test waits for a job.
const PATIENCE: Duration = Duration::from_secs(20);

/// Step of the waiting loops of the tests.
const STEP: Duration = Duration::from_millis(10);

/// Explicitly set variable whose value must never reach a public record.
const SECRET_NAME: &str = "DEPLOY_TOKEN";
const SECRET_VALUE: &str = "s3cr3t-value-7f";

/// Name of the gate file in the test directory; a gated test program ends
/// once it exists.
const GATE: &str = "release";

struct SystemClock {
    origin: Instant,
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        WallTime::from_unix_nanos(i128::try_from(since.as_nanos()).unwrap_or(i128::MAX))
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

struct TestHost {
    info: HostInfo,
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, _name: &str) -> Option<std::ffi::OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        Vec::new()
    }
}

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }
}

impl EventSink for Recorder {
    fn emit(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
}

fn context_with(
    dir: &Path,
    config: Config,
    supervision: Option<Supervision>,
) -> (ExecCtx, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let mut extensions = Extensions::new();
    if let Some(supervision) = supervision {
        extensions.insert::<Supervision>(Arc::new(supervision));
    }
    let host = TestHost {
        info: HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "user".to_owned(),
            home: dir.to_path_buf(),
            temp: dir.to_path_buf(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        },
    };
    let ctx = ExecCtx::builder(
        Arc::new(SystemClock {
            origin: Instant::now(),
        }),
        Arc::new(host),
    )
    .state_dir(dir.to_path_buf())
    .config(Arc::new(config))
    .events(recorder.clone())
    .extensions(extensions)
    .build();
    (ctx, recorder)
}

fn context(dir: &Path) -> (ExecCtx, Arc<Recorder>) {
    context_with(dir, Config::default(), None)
}

/// Process control double; records the requests it starts.
#[derive(Default)]
struct FakeControl {
    spawned: Mutex<Vec<SpawnRequest>>,
}

impl ProcessControl for FakeControl {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn OutputSink>,
        cancel: CancelToken,
    ) -> Result<Box<dyn RunningProcess>> {
        if request.program.ends_with("missing") {
            return Err(Error::new(
                ErrorCode::ProgramNotFound,
                "program `missing` not found",
            ));
        }
        let number = |index: usize| -> u64 {
            request
                .args
                .get(index)
                .and_then(|arg| arg.to_str())
                .and_then(|arg| arg.parse().ok())
                .unwrap_or(0)
        };
        let run = Duration::from_millis(number(1));
        let code = i32::try_from(number(2)).unwrap_or(1);
        let gate = request.args.get(4).map(PathBuf::from);
        for line in 1..=number(3) {
            sink.write(OutputStream::Stdout, format!("line {line}\n").as_bytes())?;
        }
        if code != 0 {
            sink.write(OutputStream::Stderr, b"failure\n")?;
        }
        let process = FakeProcess {
            pid: 4000 + u32::try_from(self.spawned.lock().unwrap().len()).unwrap_or(0),
            started: wall_now(),
            left: run,
            gate,
            code,
            limit: request.limits.timeout,
            active: Duration::ZERO,
            last: Instant::now(),
            suspended: false,
            suspended_total: Duration::ZERO,
            cancel,
        };
        self.spawned.lock().unwrap().push(request);
        Ok(Box::new(process))
    }
}

fn wall_now() -> WallTime {
    SystemClock {
        origin: Instant::now(),
    }
    .now()
}

struct FakeProcess {
    pid: u32,
    started: WallTime,
    left: Duration,
    gate: Option<PathBuf>,
    code: i32,
    limit: Option<Duration>,
    active: Duration,
    last: Instant,
    suspended: bool,
    suspended_total: Duration,
    cancel: CancelToken,
}

impl FakeProcess {
    fn tick(&mut self) {
        let now = Instant::now();
        let passed = now.duration_since(self.last);
        self.last = now;
        if self.suspended {
            self.suspended_total += passed;
        } else {
            self.active += passed;
            self.left = self.left.saturating_sub(passed);
        }
    }

    fn end(
        &self,
        exit_code: Option<i32>,
        signal: Option<Signal>,
        killed_by: Option<KilledBy>,
    ) -> ProcessEnd {
        ProcessEnd {
            exit_code,
            signal,
            signal_number: None,
            killed_by,
            limit_hit: None,
            finished: wall_now(),
            usage: ResourceUsage::default(),
            suspended_total: self.suspended_total,
            stdin_closed_early: false,
        }
    }

    fn released(&self) -> bool {
        self.gate.as_ref().is_none_or(|gate| gate.exists())
    }
}

impl RunningProcess for FakeProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn started(&self) -> WallTime {
        self.started
    }

    fn wait(&mut self, timeout: Option<Duration>) -> Result<Option<ProcessEnd>> {
        let begun = Instant::now();
        loop {
            self.tick();
            if self.cancel.is_cancelled() {
                return Ok(Some(self.end(
                    None,
                    Some(Signal::Terminate),
                    Some(KilledBy::Cancel),
                )));
            }
            if self.limit.is_some_and(|limit| self.active >= limit) {
                return Ok(Some(self.end(
                    None,
                    Some(Signal::Kill),
                    Some(KilledBy::Timeout),
                )));
            }
            if self.left.is_zero() && !self.suspended && self.released() {
                return Ok(Some(self.end(Some(self.code), None, None)));
            }
            if timeout.is_some_and(|timeout| begun.elapsed() >= timeout) {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn stop(&mut self, signal: Signal, _grace: Duration) -> Result<ProcessEnd> {
        self.tick();
        Ok(self.end(None, Some(signal), None))
    }

    fn suspend(&mut self) -> Result<()> {
        self.tick();
        self.suspended = true;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.tick();
        self.suspended = false;
        Ok(())
    }
}

/// Output capture double: one file per stream in `<state>/spool`; cursors
/// count complete lines as `<stdout>.<stderr>`.
struct FileSpool;

struct FileStore {
    stdout: PathBuf,
    stderr: PathBuf,
}

fn spool_dir(ctx: &ExecCtx) -> PathBuf {
    ctx.state_dir().unwrap().join("spool")
}

impl SpoolFactory for FileSpool {
    fn create(&self, ctx: &ExecCtx, id: &str, _merge_stderr: bool) -> Result<Arc<dyn OutputStore>> {
        let dir = spool_dir(ctx);
        fs::create_dir_all(&dir).unwrap();
        let store = FileStore {
            stdout: dir.join(format!("{id}.stdout")),
            stderr: dir.join(format!("{id}.stderr")),
        };
        fs::write(&store.stdout, b"").unwrap();
        fs::write(&store.stderr, b"").unwrap();
        Ok(Arc::new(store))
    }

    fn open(&self, ctx: &ExecCtx, id: &str) -> Result<Arc<dyn OutputStore>> {
        let dir = spool_dir(ctx);
        let store = FileStore {
            stdout: dir.join(format!("{id}.stdout")),
            stderr: dir.join(format!("{id}.stderr")),
        };
        if !store.stdout.exists() {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no output of `{id}`"),
            ));
        }
        Ok(Arc::new(store))
    }

    fn remove(&self, ctx: &ExecCtx, id: &str) -> Result<()> {
        let dir = spool_dir(ctx);
        let _ = fs::remove_file(dir.join(format!("{id}.stdout")));
        let _ = fs::remove_file(dir.join(format!("{id}.stderr")));
        Ok(())
    }
}

impl FileStore {
    fn lines(&self, stream: OutputStream) -> Vec<String> {
        let path = match stream {
            OutputStream::Stdout => &self.stdout,
            OutputStream::Stderr => &self.stderr,
        };
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn shows(view: &OutputView, stream: OutputStream) -> bool {
        match view.show {
            ShowMode::Stdout => stream == OutputStream::Stdout,
            ShowMode::Stderr => stream == OutputStream::Stderr,
            ShowMode::None => false,
            _ => true,
        }
    }

    fn select(view: &OutputView, lines: &[String]) -> String {
        let filtered: Vec<&String> = lines
            .iter()
            .filter(|line| {
                view.filter
                    .as_ref()
                    .is_none_or(|filter| line.contains(filter.as_str()))
            })
            .collect();
        let tail = view
            .tail
            .map_or(filtered.len(), |tail| usize::try_from(tail).unwrap_or(0));
        let skip = filtered.len().saturating_sub(tail);
        filtered[skip..]
            .iter()
            .map(|line| format!("{line}\n"))
            .collect()
    }

    fn rendered(
        &self,
        view: &OutputView,
        out: &[String],
        err: &[String],
        all_out: usize,
        all_err: usize,
    ) -> RenderedOutput {
        let progress = view.progress.as_ref().and_then(|pattern| {
            out.iter()
                .rev()
                .find(|line| line.contains(pattern.as_str()))
                .map(|line| nurt_contracts::record::Progress {
                    percent: None,
                    line: line.clone(),
                })
        });
        RenderedOutput {
            stdout: if Self::shows(view, OutputStream::Stdout) {
                Self::select(view, out)
            } else {
                String::new()
            },
            stderr: if Self::shows(view, OutputStream::Stderr) {
                Self::select(view, err)
            } else {
                String::new()
            },
            stdout_bytes: fs::metadata(&self.stdout)
                .map(|meta| meta.len())
                .unwrap_or(0),
            stderr_bytes: fs::metadata(&self.stderr)
                .map(|meta| meta.len())
                .unwrap_or(0),
            stdout_lines: u64::try_from(all_out).unwrap_or(0),
            stderr_lines: u64::try_from(all_err).unwrap_or(0),
            decoded_as: "utf-8".to_owned(),
            shown: ExecShown::default(),
            progress,
            stdout_file: Some(self.stdout.clone()),
            stderr_file: Some(self.stderr.clone()),
            output_cursor: Some(format!("{all_out}.{all_err}")),
            ..RenderedOutput::default()
        }
    }
}

impl OutputSink for FileStore {
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()> {
        let path = match stream {
            OutputStream::Stdout => &self.stdout,
            OutputStream::Stderr => &self.stderr,
        };
        let mut file = OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(bytes).unwrap();
        Ok(())
    }

    fn finish(&self, _stream: OutputStream) -> Result<()> {
        Ok(())
    }

    fn last_activity(&self) -> Option<Duration> {
        None
    }
}

impl OutputStore for FileStore {
    fn render(&self, view: &OutputView, _exit_code: Option<i32>) -> Result<RenderedOutput> {
        let out = self.lines(OutputStream::Stdout);
        let err = self.lines(OutputStream::Stderr);
        Ok(self.rendered(view, &out, &err, out.len(), err.len()))
    }

    fn read_since(&self, cursor: Option<&str>, view: &OutputView) -> Result<RenderedOutput> {
        let out = self.lines(OutputStream::Stdout);
        let err = self.lines(OutputStream::Stderr);
        let (seen_out, seen_err) = cursor
            .and_then(|cursor| cursor.split_once('.'))
            .map_or((0, 0), |(left, right)| {
                (left.parse().unwrap_or(0), right.parse().unwrap_or(0))
            });
        let new_out = out[seen_out.min(out.len())..].to_vec();
        let new_err = err[seen_err.min(err.len())..].to_vec();
        Ok(self.rendered(view, &new_out, &new_err, out.len(), err.len()))
    }
}

/// Change detection double: reports one created file per watched directory
/// and records it in the transaction.
struct FakeTracker;

struct FakeScan {
    watch: Vec<PathBuf>,
    mode: TrackMode,
}

impl ChangeTracker for FakeTracker {
    fn prepare(
        &self,
        _ctx: &ExecCtx,
        spec: &TrackSpec,
        _tx: &mut dyn MutationTx,
    ) -> Result<Box<dyn PreparedScan>> {
        Ok(Box::new(FakeScan {
            watch: spec.watch.clone(),
            mode: spec.mode,
        }))
    }
}

impl PreparedScan for FakeScan {
    fn finish(self: Box<Self>, _ctx: &ExecCtx, tx: &mut dyn MutationTx) -> Result<TrackOutcome> {
        if self.mode == TrackMode::None {
            return Ok(TrackOutcome::default());
        }
        let mut changes = Vec::new();
        for dir in &self.watch {
            let path = dir.join("result.txt");
            tx.observe_external(&[nurt_contracts::journal::ExternalChange {
                path: nurt_contracts::fs::AbsPath::new(path.clone()).unwrap(),
                kind: nurt_contracts::journal::ExternalChangeKind::Created,
                declared: None,
            }])?;
            changes.push(Change {
                path: value::Path::from(path.to_string_lossy().into_owned()),
                action: ChangeAction::Create,
                ..Change::default()
            });
        }
        Ok(TrackOutcome {
            changes,
            tracking: Tracking::Scan,
            watch: self.watch.clone(),
            ..TrackOutcome::default()
        })
    }
}

fn components_with(control: Arc<FakeControl>) -> Components {
    Components {
        control,
        spool: Arc::new(FileSpool),
        tracker: Arc::new(FakeTracker),
    }
}

fn components() -> Components {
    components_with(Arc::new(FakeControl::default()))
}

/// Request of the test program: runs `millis`, exits with `code`, writes
/// `lines` lines.
fn request(dir: &Path, millis: u64, code: i32, lines: u32) -> RunRequest {
    let args: Vec<String> = vec![
        "run".to_owned(),
        millis.to_string(),
        code.to_string(),
        lines.to_string(),
    ];
    let mut command = vec!["fake".to_owned()];
    command.extend(args.iter().cloned());
    RunRequest {
        command,
        program: dir.join("fake"),
        env_keys: vec![SECRET_NAME.to_owned()],
        spawn: SpawnRequest {
            program: dir.join("fake"),
            args: args.into_iter().map(Into::into).collect(),
            launcher: Launcher::Direct,
            raw_command_line: None,
            cwd: dir.to_path_buf(),
            env: vec![(SECRET_NAME.into(), SECRET_VALUE.into())],
            stdin: StdinSource::Bytes(b"input".to_vec()),
            merge_stderr: false,
            limits: ProcessLimits {
                timeout: Some(Duration::from_secs(60)),
                idle_timeout: None,
                kill_grace: Duration::from_secs(1),
                max_cpu: None,
                max_memory: None,
                priority: Priority::Normal,
            },
        },
        track: TrackSpec {
            mode: TrackMode::Scan,
            watch: vec![dir.to_path_buf()],
            ..TrackSpec::default()
        },
        view: OutputView {
            show: ShowMode::Both,
            tail: Some(40),
            ..OutputView::default()
        },
        kind: RunKind::Run,
        label: Some("build".to_owned()),
        description: None,
        session: Some("s1".to_owned()),
        retry_of: None,
        external_effect: Vec::new(),
        finish_data: None,
    }
}

/// Request of the test program that writes `lines` lines and runs until the
/// gate file of `dir` exists.
fn gated(dir: &Path, lines: u32) -> RunRequest {
    let mut request = request(dir, 0, 0, lines);
    let gate = dir.join(GATE);
    request.command.push(gate.to_string_lossy().into_owned());
    request.spawn.args.push(gate.into_os_string());
    request
}

/// Lets the gated test programs of `dir` end.
fn release(dir: &Path) {
    fs::write(dir.join(GATE), b"").unwrap();
}

fn run(
    ctx: &ExecCtx,
    components: &Components,
    request: RunRequest,
    wait: Option<Duration>,
) -> Result<RunWait> {
    LocalHost::new(components.clone(), true).run(ctx, request, wait)
}

fn background(ctx: &ExecCtx, components: &Components, request: RunRequest) -> Job {
    match run(ctx, components, request, Some(Duration::ZERO)).unwrap() {
        RunWait::Background(job) => *job,
        RunWait::Finished(_) => panic!("the run did not move to the background"),
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let begun = Instant::now();
    while !condition() {
        assert!(begun.elapsed() < PATIENCE, "condition not reached in time");
        std::thread::sleep(STEP);
    }
}

fn job_file(dir: &Path, id: &str) -> JobFile {
    JobTable::at(dir.join("jobs"))
        .unwrap()
        .read(id)
        .unwrap()
        .unwrap()
}

fn wait_finished(dir: &Path, id: &str) -> JobFile {
    wait_until(|| job_file(dir, id).result.is_some());
    job_file(dir, id)
}

fn sample_file(dir: &Path, id: &str, state: JobState) -> JobFile {
    let request = request(dir, 0, 0, 0);
    JobFile {
        protocol: PROTOCOL,
        record: Job {
            id: id.to_owned(),
            command: request.command.clone(),
            state,
            ..Job::default()
        },
        command_name: "run".to_owned(),
        request: StoredRequest(request),
        wait: None,
        supervisor_pid: std::process::id(),
        detached: true,
        result: None,
        final_record: None,
    }
}

fn stage(components: &Components, command: &str) -> Arc<dyn StageImpl> {
    crate::stage::jobs::with_components(components)
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .unwrap()
}

fn text(value: &str) -> ArgValue {
    ArgValue::Value(Value::Text(value.to_owned()))
}

fn word(value: &str) -> ArgValue {
    ArgValue::Value(Value::Enum(value.to_owned()))
}

fn millis(value: i64) -> ArgValue {
    ArgValue::Value(Value::Duration(value::Duration::from_millis(value)))
}

fn read_jobs(
    components: &Components,
    ctx: &ExecCtx,
    args: &BoundArgs,
) -> Result<(Vec<Job>, Option<Record>)> {
    let jobs = stage(components, "jobs");
    let output = jobs
        .as_read()
        .unwrap()
        .execute(args, StageInput::none(), ctx)?;
    let header = output.header.clone();
    let records = StageInput::from(output).into_records()?;
    let jobs = records
        .map(|record| Job::try_from(record.unwrap()).unwrap())
        .collect();
    Ok((jobs, header))
}

fn effect_on(
    components: &Components,
    ctx: &ExecCtx,
    command: &str,
    args: &BoundArgs,
    jobs: Vec<Job>,
) -> Result<(
    Vec<Record>,
    nurt_contracts::stage::EffectOutcome<'static>,
    Vec<Irreversible>,
)> {
    let stage = stage(components, command);
    let effect = stage.as_effect().unwrap();
    let input = StageInput::records(RecordStream::from_records(
        jobs.into_iter().map(Record::from).collect(),
    ));
    let plan = effect.plan(args, input, ctx)?;
    let mut tx = Unjournaled::new(OpKind::command(command));
    let outcome = effect.apply(args, plan, &mut tx, ctx)?;
    let marked: Vec<Irreversible> = Box::new(tx)
        .commit()?
        .irreversible
        .into_iter()
        .map(|entry| entry.reason)
        .collect();
    let records: Vec<Record> = StageInput::from(outcome.output)
        .into_records()?
        .map(|record| record.unwrap())
        .collect();
    let rest = nurt_contracts::stage::EffectOutcome {
        output: nurt_contracts::stage::StageOutput::none(),
        changes: outcome.changes,
        notable: outcome.notable,
        irreversible: outcome.irreversible,
        detached: outcome.detached,
    };
    Ok((records, rest, marked))
}

mod arguments;
mod exchange;
mod records;
mod start;
mod tools;
mod transactions;

mod jobs_table {
    use super::*;

    #[test]
    fn allocates_increasing_identifiers_without_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        assert_eq!(table.allocate().unwrap(), "j1");
        let second = table.allocate().unwrap();
        assert_eq!(second, "j2");
        table
            .write(&sample_file(dir.path(), &second, JobState::Exited))
            .unwrap();
        table.remove(&second).unwrap();
        assert_eq!(table.allocate().unwrap(), "j3");
        assert!(table.ids().unwrap().is_empty());
    }

    #[test]
    fn allocates_unique_identifiers_from_many_threads() {
        let dir = tempfile::tempdir().unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let table = table.clone();
                std::thread::spawn(move || {
                    (0..10)
                        .map(|_| table.allocate().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: HashSet<String> = threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 80);
        assert!(ids.contains("j80"));
    }

    #[test]
    fn readers_never_see_a_partial_job_file() {
        let dir = tempfile::tempdir().unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        let mut file = sample_file(dir.path(), "j1", JobState::Exited);
        table.write(&file).unwrap();
        let writer = {
            let table = table.clone();
            std::thread::spawn(move || {
                for round in 0..200 {
                    file.record.label = Some("x".repeat(round));
                    table.write(&file).unwrap();
                }
            })
        };
        for _ in 0..200 {
            assert!(table.read("j1").unwrap().is_some());
        }
        writer.join().unwrap();
    }

    #[test]
    fn unknown_job_offers_the_nearest_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        for id in ["j1", "j5", "j6", "j9"] {
            table
                .write(&sample_file(dir.path(), id, JobState::Exited))
                .unwrap();
        }
        let error = table.get("j7").unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownJob);
        let programs: Vec<&str> = error
            .candidates
            .iter()
            .map(|candidate| candidate.program.as_str())
            .collect();
        assert_eq!(programs, ["jobs \"j6\"", "jobs \"j5\"", "jobs \"j9\""]);
        assert_eq!(table.get("build").unwrap_err().candidates.len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn keeps_environment_readable_by_the_account_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 0, 0, 1));
        wait_finished(dir.path(), &job.id);
        let jobs = dir.path().join("jobs");
        let mode = fs::metadata(jobs.join(format!("{}.input.json", job.id)))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = fs::metadata(&jobs).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
        let public = fs::read_to_string(jobs.join(format!("{}.json", job.id))).unwrap();
        assert!(!public.contains(SECRET_VALUE));
    }

    #[test]
    fn fails_without_a_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let bare = ExecCtx::builder(
            Arc::new(SystemClock {
                origin: Instant::now(),
            }),
            Arc::new(TestHost {
                info: ctx.host().info().clone(),
            }),
        )
        .build();
        assert_eq!(JobTable::open(&bare).unwrap_err().code, ErrorCode::Config);
    }
}

mod jobs_states {
    use super::*;

    #[test]
    fn records_exit_code_and_final_state_of_finished_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        for (code, state) in [(0, JobState::Exited), (3, JobState::Failed)] {
            let RunWait::Finished(result) =
                run(&ctx, &components, request(dir.path(), 20, code, 2), None).unwrap()
            else {
                panic!("the run moved to the background");
            };
            assert_eq!(result.end.exit_code, Some(code));
            assert_eq!(result.job, None);
            let table = JobTable::at(dir.path().join("jobs")).unwrap();
            let id = table.ids().unwrap().last().cloned().unwrap();
            let file = job_file(dir.path(), &id);
            assert_eq!(file.record.state, state);
            assert_eq!(file.record.exit_code, Some(i64::from(code)));
            assert!(!file.detached);
        }
    }

    #[test]
    fn ends_a_run_at_its_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let mut slow = request(dir.path(), 5_000, 0, 0);
        slow.spawn.limits.timeout = Some(Duration::from_millis(50));
        let RunWait::Finished(result) = run(&ctx, &components, slow, None).unwrap() else {
            panic!("the run moved to the background");
        };
        assert_eq!(result.end.killed_by, Some(KilledBy::Timeout));
        let file = job_file(dir.path(), "j1");
        assert_eq!(file.record.state, JobState::TimedOut);
    }

    #[test]
    fn reports_a_program_that_cannot_start() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let mut missing = request(dir.path(), 0, 0, 0);
        missing.spawn.program = dir.path().join("missing");
        let error = run(&ctx, &components(), missing, Some(Duration::from_secs(1))).unwrap_err();
        assert_eq!(error.code, ErrorCode::ProgramNotFound);
        assert!(
            JobTable::at(dir.path().join("jobs"))
                .unwrap()
                .ids()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn marks_a_running_job_without_supervisor_lost() {
        let dir = tempfile::tempdir().unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        table
            .write(&sample_file(dir.path(), "j1", JobState::Running))
            .unwrap();
        table
            .write(&sample_file(dir.path(), "j2", JobState::Suspended))
            .unwrap();
        assert_eq!(
            table.read("j1").unwrap().unwrap().record.state,
            JobState::Lost
        );
        assert_eq!(
            table.read("j2").unwrap().unwrap().record.state,
            JobState::Lost
        );
        assert_eq!(
            table.read_raw("j1").unwrap().unwrap().record.state,
            JobState::Lost
        );
    }

    #[test]
    fn keeps_a_supervised_job_running() {
        let dir = tempfile::tempdir().unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        let _lock = table.lock_job("j1").unwrap();
        table
            .write(&sample_file(dir.path(), "j1", JobState::Running))
            .unwrap();
        assert_eq!(
            table.read("j1").unwrap().unwrap().record.state,
            JobState::Running
        );
    }

    #[test]
    fn refuses_background_runs_without_supervisor() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.run.supervisor = Supervisor::Disabled;
        let command = SupervisorCommand {
            program: dir.path().join("nurt"),
            args: Vec::new(),
        };
        let (ctx, _) = context_with(dir.path(), config, Some(Supervision::Detached(command)));
        let host = run_host(&ctx, components());
        let error = host
            .run(&ctx, request(dir.path(), 0, 0, 0), Some(Duration::ZERO))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        let outcome = host
            .run(
                &ctx,
                request(dir.path(), 100, 0, 0),
                Some(Duration::from_millis(1)),
            )
            .unwrap();
        assert!(matches!(outcome, RunWait::Finished(_)));
    }

    #[test]
    fn records_changes_without_a_journal_as_irreversible() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("result.txt"), b"done").unwrap();
        let (ctx, _) = context(dir.path());
        let RunWait::Finished(result) =
            run(&ctx, &components(), request(dir.path(), 0, 0, 0), None).unwrap()
        else {
            panic!("the run moved to the background");
        };
        assert_eq!(result.op, None);
        assert_eq!(result.track.changes.len(), 1);
        let mut tx: Box<dyn MutationTx> = Box::new(Unjournaled::new(OpKind::command("run")));
        let applied = tx
            .observe_external(&[nurt_contracts::journal::ExternalChange {
                path: nurt_contracts::fs::AbsPath::new(dir.path().join("result.txt")).unwrap(),
                kind: nurt_contracts::journal::ExternalChangeKind::Created,
                declared: None,
            }])
            .unwrap();
        assert!(applied[0].after.exists);
        assert_eq!(applied[0].after.size, Some(4));
        let handle = tx.commit().unwrap();
        assert_eq!(handle.op, None);
        assert_eq!(handle.irreversible[0].reason, Irreversible::JournalInactive);
    }
}

mod jobs_jobs_views {
    use super::*;

    #[test]
    fn lists_running_jobs_before_finished_ones() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let finished = background(&ctx, &components, request(dir.path(), 0, 0, 0));
        wait_finished(dir.path(), &finished.id);
        let running = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        let (jobs, _) = read_jobs(&components, &ctx, &BoundArgs::new("jobs")).unwrap();
        let ids: Vec<&str> = jobs.iter().map(|job| job.id.as_str()).collect();
        assert_eq!(ids, [running.id.as_str(), finished.id.as_str()]);
        let only_exited = BoundArgs::new("jobs").with("state", word("exited"), false);
        let (jobs, _) = read_jobs(&components, &ctx, &only_exited).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, JobState::Exited);
        stop_all(&ctx, &components);
    }

    #[test]
    fn status_shows_the_last_five_lines() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 0, 0, 8));
        wait_finished(dir.path(), &job.id);
        let args = BoundArgs::new("jobs").with("id", text(&job.id), false);
        let (jobs, _) = read_jobs(&components, &ctx, &args).unwrap();
        assert_eq!(
            jobs[0].stdout.as_deref(),
            Some("line 4\nline 5\nline 6\nline 7\nline 8\n")
        );
        assert_eq!(jobs[0].stdout_lines, Some(8));
    }

    #[test]
    fn output_continues_from_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 0, 0, 3));
        wait_finished(dir.path(), &job.id);
        let first = BoundArgs::new("jobs")
            .with("id", text(&job.id), false)
            .with("view", word("output"), false);
        let (jobs, _) = read_jobs(&components, &ctx, &first).unwrap();
        assert_eq!(jobs[0].stdout.as_deref(), Some("line 1\nline 2\nline 3\n"));
        let cursor = jobs[0].output_cursor.clone().unwrap();
        let next = first.clone().with("since", text(&cursor), false);
        let (jobs, _) = read_jobs(&components, &ctx, &next).unwrap();
        assert_eq!(jobs[0].stdout.as_deref(), Some(""));
        assert_eq!(jobs[0].output_cursor.as_deref(), Some(cursor.as_str()));
        let stderr_only = first.with("stream", word("stderr"), false);
        let (jobs, _) = read_jobs(&components, &ctx, &stderr_only).unwrap();
        assert_eq!(jobs[0].stdout.as_deref(), Some(""));
    }

    #[test]
    fn result_needs_a_finished_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 1));
        let args = BoundArgs::new("jobs")
            .with("id", text(&job.id), false)
            .with("view", word("result"), false);
        let error = read_jobs(&components, &ctx, &args).unwrap_err();
        assert_eq!(error.code, ErrorCode::JobState);
        assert_eq!(
            error.fix,
            [format!("jobs \"{}\" view: result wait: 60s", job.id)]
        );
        stop_all(&ctx, &components);
        let (jobs, _) = read_jobs(&components, &ctx, &args).unwrap();
        assert_eq!(jobs[0].state, JobState::Stopped);
        assert_eq!(jobs[0].changes.as_ref().map(Vec::len), Some(1));
    }

    #[test]
    fn rejects_unknown_job_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let args = BoundArgs::new("jobs").with("id", text("j42"), false);
        assert_eq!(
            read_jobs(&components, &ctx, &args).unwrap_err().code,
            ErrorCode::UnknownJob
        );
    }
}

mod jobs_wait {
    use super::*;

    #[test]
    fn waits_for_the_end_of_a_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 200, 0, 1));
        let args = BoundArgs::new("jobs")
            .with("id", text(&job.id), false)
            .with("wait", millis(10_000), false);
        let (jobs, _) = read_jobs(&components, &ctx, &args).unwrap();
        assert_eq!(jobs[0].state, JobState::Exited);
    }

    #[test]
    fn returns_a_running_job_when_the_wait_elapses() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 1));
        let args = BoundArgs::new("jobs")
            .with("id", text(&job.id), false)
            .with("wait", millis(100), false);
        let begun = Instant::now();
        let (jobs, _) = read_jobs(&components, &ctx, &args).unwrap();
        assert_eq!(jobs[0].state, JobState::Running);
        assert!(begun.elapsed() >= Duration::from_millis(100));
        stop_all(&ctx, &components);
    }

    #[test]
    fn moves_a_run_to_the_background_after_its_wait() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let outcome = run(
            &ctx,
            &components,
            request(dir.path(), 10_000, 0, 1),
            Some(Duration::from_millis(100)),
        )
        .unwrap();
        let RunWait::Background(job) = outcome else {
            panic!("the run did not move to the background");
        };
        assert_eq!(job.state, JobState::Running);
        assert!(job.elapsed.as_millis() >= 100);
        assert!(job_file(dir.path(), &job.id).detached);
        stop_all(&ctx, &components);
        assert_eq!(
            job_file(dir.path(), &job.id).result.unwrap().0.job,
            Some(job.id.clone())
        );
    }
}

fn stop_all(ctx: &ExecCtx, components: &Components) {
    let (running, _) = read_jobs(
        components,
        ctx,
        &BoundArgs::new("jobs").with("state", word("running"), false),
    )
    .unwrap();
    if running.is_empty() {
        return;
    }
    let args = BoundArgs::new("stop").with("grace", millis(0), false);
    effect_on(components, ctx, "stop", &args, running).unwrap();
}

mod jobs_stop_job {
    use super::*;

    #[test]
    fn stops_a_running_job_irreversibly() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 1));
        let args = BoundArgs::new("stop").with("signal", word("interrupt"), false);
        let (records, outcome, marked) =
            effect_on(&components, &ctx, "stop", &args, vec![job.clone()]).unwrap();
        let effect = Effect::try_from(records[0].clone()).unwrap();
        let stopped = effect.job.unwrap();
        assert_eq!(stopped.state, JobState::Stopped);
        assert_eq!(stopped.killed_by, Some(KilledBy::Stop));
        assert_eq!(stopped.signal, Some(Signal::Interrupt));
        assert_eq!(
            effect.target.as_deref(),
            Some(format!("job:{}", job.id).as_str())
        );
        assert_eq!(outcome.irreversible[0].reason, Irreversible::ProcessStopped);
        assert_eq!(marked, [Irreversible::ProcessStopped]);
    }

    #[test]
    fn refuses_to_stop_a_finished_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 0, 0, 0));
        wait_finished(dir.path(), &job.id);
        let error = effect_on(
            &components,
            &ctx,
            "stop",
            &BoundArgs::new("stop"),
            vec![job],
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::JobState);
    }

    #[test]
    fn answers_a_stop_whose_job_ended_before_the_reply() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        let _lock = table.lock_job("j1").unwrap();
        let mut file = sample_file(dir.path(), "j1", JobState::Running);
        table.write(&file).unwrap();
        let requester = {
            let (ctx, table) = (ctx.clone(), table.clone());
            std::thread::spawn(move || {
                let stop = ControlAction::Stop {
                    signal: Signal::Terminate,
                    grace: Duration::ZERO,
                };
                control::send(&ctx, &table, "j1", stop)
            })
        };
        let mut taken = Vec::new();
        wait_until(|| {
            taken.extend(control::pending(&table, "j1"));
            !taken.is_empty()
        });
        file.record.state = JobState::Stopped;
        table.write(&file).unwrap();
        // The requester polls the job file several times while the final
        // state is written and the reply is not.
        std::thread::sleep(control::CONTROL_POLL * 10);
        control::reply(&table, "j1", &taken[0].0, Ok(file.record.clone()));
        let stopped = requester.join().unwrap().unwrap();
        assert_eq!(stopped.state, JobState::Stopped);
    }

    #[test]
    fn rejects_an_unknown_signal() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        let args = BoundArgs::new("stop").with("signal", word("usr1"), false);
        let error = effect_on(&components, &ctx, "stop", &args, vec![job]).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
        stop_all(&ctx, &components);
    }
}

mod jobs_stop_dispatch {
    use super::*;

    fn select<'a>(
        stages: &'a [Arc<dyn StageImpl>],
        command: &str,
        kind: RecordKind,
    ) -> Option<&'a Arc<dyn StageImpl>> {
        stages.iter().find(|stage| {
            let descriptor = stage.descriptor();
            descriptor.command == command && descriptor.input.accepts(kind)
        })
    }

    #[test]
    fn job_commands_accept_job_records_only() {
        let stages = crate::stage::jobs::with_components(&components());
        for command in ["stop", "suspend", "resume", "retry", "remove"] {
            let selected = select(&stages, command, RecordKind::Job).unwrap();
            assert!(selected.as_effect().is_some(), "{command}");
        }
        for command in ["stop", "suspend", "resume"] {
            let selected = select(&stages, command, RecordKind::Process).unwrap();
            assert!(selected.as_effect().is_some(), "{command}");
        }
        assert!(select(&stages, "retry", RecordKind::Process).is_none());
        assert!(select(&stages, "remove", RecordKind::Process).is_none());
        assert!(select(&stages, "remove", RecordKind::Entry).is_none());
        let jobs = select(&stages, "jobs", RecordKind::Job);
        assert!(jobs.is_none());
        assert!(
            stages
                .iter()
                .any(|stage| stage.descriptor().command == "jobs" && stage.descriptor().source)
        );
    }
}

mod jobs_retry {
    use super::*;

    #[test]
    fn repeats_the_command_with_the_same_directory_and_environment() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let control = Arc::new(FakeControl::default());
        let components = components_with(control.clone());
        let job = background(&ctx, &components, request(dir.path(), 0, 4, 1));
        wait_finished(dir.path(), &job.id);
        let args = BoundArgs::new("retry")
            .with("wait", word("forever"), false)
            .with("timeout", millis(30_000), false);
        let (records, outcome, _) =
            effect_on(&components, &ctx, "retry", &args, vec![job.clone()]).unwrap();
        let exec = Exec::try_from(records[0].clone()).unwrap();
        assert_eq!(exec.exit_code, 4);
        assert_eq!(exec.cwd.as_str(), dir.path().to_string_lossy());
        assert!(
            outcome
                .notable
                .iter()
                .any(|entry| entry.code == Notable::ExitNonzero)
        );
        assert!(!outcome.detached);
        let spawned = control.spawned.lock().unwrap();
        let repeated = spawned.last().unwrap();
        assert_eq!(
            repeated.env,
            vec![(SECRET_NAME.into(), SECRET_VALUE.into())]
        );
        assert_eq!(repeated.stdin, StdinSource::Bytes(b"input".to_vec()));
        let timeout = repeated.limits.timeout.unwrap();
        assert!(
            timeout <= Duration::from_secs(30) && timeout > Duration::from_secs(29),
            "{timeout:?}"
        );
        assert_eq!(repeated.cwd, dir.path());
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        let newest = table.ids().unwrap().last().cloned().unwrap();
        let file = job_file(dir.path(), &newest);
        assert_eq!(file.record.retry_of.as_deref(), Some(job.id.as_str()));
        assert_eq!(file.request.0.kind, RunKind::Retry);
    }

    #[test]
    fn continues_a_long_retry_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let mut slow = request(dir.path(), 10_000, 0, 0);
        slow.spawn.limits.timeout = Some(Duration::from_millis(20));
        let job = background(&ctx, &components, slow);
        wait_finished(dir.path(), &job.id);
        let args = BoundArgs::new("retry").with("wait", millis(0), false).with(
            "timeout",
            millis(60_000),
            false,
        );
        let (records, outcome, _) =
            effect_on(&components, &ctx, "retry", &args, vec![job]).unwrap();
        assert_eq!(records[0].kind, RecordKind::Job);
        assert!(outcome.detached);
        assert!(
            outcome
                .notable
                .iter()
                .any(|entry| entry.code == Notable::PostStagesDeferred)
        );
        stop_all(&ctx, &components);
    }

    #[test]
    fn refuses_to_repeat_a_running_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        let error = effect_on(
            &components,
            &ctx,
            "retry",
            &BoundArgs::new("retry"),
            vec![job.clone()],
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::JobState);
        assert_eq!(error.fix, [format!("jobs \"{}\" |> stop", job.id)]);
        stop_all(&ctx, &components);
    }
}

mod jobs_suspend_resume {
    use super::*;

    #[test]
    fn suspends_and_resumes_a_running_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, gated(dir.path(), 0));
        let (records, _, marked) = effect_on(
            &components,
            &ctx,
            "suspend",
            &BoundArgs::new("suspend"),
            vec![job.clone()],
        )
        .unwrap();
        let suspended = Effect::try_from(records[0].clone()).unwrap().job.unwrap();
        assert_eq!(suspended.state, JobState::Suspended);
        assert_eq!(marked, [Irreversible::ProcessStateChanged]);
        let again = effect_on(
            &components,
            &ctx,
            "suspend",
            &BoundArgs::new("suspend"),
            vec![job.clone()],
        )
        .unwrap_err();
        assert_eq!(again.code, ErrorCode::JobState);
        assert_eq!(again.fix, [format!("jobs \"{}\" |> resume", job.id)]);
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(
            job_file(dir.path(), &job.id).record.state,
            JobState::Suspended
        );
        let (records, _, _) = effect_on(
            &components,
            &ctx,
            "resume",
            &BoundArgs::new("resume"),
            vec![job.clone()],
        )
        .unwrap();
        assert_eq!(
            Effect::try_from(records[0].clone())
                .unwrap()
                .job
                .unwrap()
                .state,
            JobState::Running
        );
        release(dir.path());
        let file = wait_finished(dir.path(), &job.id);
        assert_eq!(file.record.state, JobState::Exited);
        let suspended_total = file.record.suspended_total.unwrap().as_millis();
        assert!(suspended_total >= 400, "{suspended_total}");
    }

    #[test]
    fn refuses_to_resume_a_running_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        let error = effect_on(
            &components,
            &ctx,
            "resume",
            &BoundArgs::new("resume"),
            vec![job.clone()],
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::JobState);
        assert_eq!(error.fix, [format!("jobs \"{}\" |> suspend", job.id)]);
        stop_all(&ctx, &components);
    }
}

mod jobs_remove {
    use nurt_contracts::fs::WriteHandle;
    use nurt_contracts::journal::{
        Applied, ExemptWrite, ExternalChange, Mutation, OpHandle, OpId, Preserved, RollbackReason,
        RollbackReport, TargetIndex, TargetSpec,
    };
    use nurt_contracts::report::IrreversibleEntry;

    use super::*;

    /// Transaction that records the name of every call a stage makes on it.
    #[derive(Default)]
    struct WatchedTx {
        calls: Vec<&'static str>,
    }

    fn journal_refused() -> Error {
        Error::new(ErrorCode::Unsupported, "the journal is not available")
    }

    impl MutationTx for WatchedTx {
        fn op(&self) -> Option<OpId> {
            None
        }

        fn preserve(&mut self, _targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
            self.calls.push("preserve");
            Err(journal_refused())
        }

        fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
            self.calls.push("stage");
            Err(journal_refused())
        }

        fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
            self.calls.push("apply");
            Err(journal_refused())
        }

        fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
            self.calls.push("observe_external");
            Err(journal_refused())
        }

        fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {
            self.calls.push("mark_irreversible");
        }

        fn record_exempt(&mut self, _write: ExemptWrite) {
            self.calls.push("record_exempt");
        }

        fn commit(self: Box<Self>) -> Result<OpHandle> {
            Err(journal_refused())
        }

        fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
            Err(journal_refused())
        }
    }

    /// What one `remove` call planned, produced and asked of the journal.
    struct Removal {
        records: Vec<Effect>,
        bulk: bool,
        planned_irreversible: usize,
        planned_changes: usize,
        irreversible: usize,
        changes: usize,
        journal_calls: Vec<&'static str>,
    }

    fn remove(components: &Components, ctx: &ExecCtx, jobs: Vec<Job>) -> Result<Removal> {
        let stage = stage(components, "remove");
        let effect = stage.as_effect().unwrap();
        let args = BoundArgs::new("remove");
        let input = StageInput::records(RecordStream::from_records(
            jobs.into_iter().map(Record::from).collect(),
        ));
        let plan = effect.plan(&args, input, ctx)?;
        let bulk = plan.bulk;
        let planned_irreversible = plan.irreversible.len();
        let planned_changes = plan.changes.len();
        let mut tx = WatchedTx::default();
        let outcome = effect.apply(&args, plan, &mut tx, ctx)?;
        let irreversible = outcome.irreversible.len();
        let changes = outcome.changes.len();
        let records = StageInput::from(outcome.output)
            .into_records()?
            .map(|record| Effect::try_from(record.unwrap()).unwrap())
            .collect();
        Ok(Removal {
            records,
            bulk,
            planned_irreversible,
            planned_changes,
            irreversible,
            changes,
            journal_calls: tx.calls,
        })
    }

    fn finished(ctx: &ExecCtx, components: &Components, dir: &Path) -> Job {
        let job = background(ctx, components, request(dir, 0, 0, 2));
        wait_finished(dir, &job.id);
        job
    }

    fn assert_outside_the_journal(removal: &Removal) {
        assert!(
            removal.journal_calls.is_empty(),
            "{:?}",
            removal.journal_calls
        );
        assert_eq!(removal.planned_irreversible, 0);
        assert_eq!(removal.planned_changes, 0);
        assert_eq!(removal.irreversible, 0);
        assert_eq!(removal.changes, 0);
    }

    fn assert_removed_effect(effect: &Effect, job: &Job) {
        assert_eq!(effect.kind, "remove");
        assert_eq!(effect.action.as_deref(), Some("remove"));
        assert_eq!(
            effect.target.as_deref(),
            Some(format!("job:{}", job.id).as_str())
        );
        assert!(effect.irreversible.is_none());
        assert_eq!(effect.job.as_ref().unwrap().state, JobState::Exited);
    }

    #[test]
    fn removes_a_finished_job_with_its_output_outside_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = finished(&ctx, &components, dir.path());
        let jobs = dir.path().join("jobs");
        let stdout = dir.path().join("spool").join(format!("{}.stdout", job.id));
        assert!(jobs.join(format!("{}.json", job.id)).exists());
        assert!(stdout.exists());
        let removal = remove(&components, &ctx, vec![job.clone()]).unwrap();
        assert_outside_the_journal(&removal);
        assert!(!removal.bulk);
        let [effect] = removal.records.as_slice() else {
            panic!("expected one effect record");
        };
        assert_removed_effect(effect, &job);
        assert!(!jobs.join(format!("{}.json", job.id)).exists());
        assert!(!jobs.join(format!("{}.input.json", job.id)).exists());
        assert!(!stdout.exists());
        assert_eq!(
            JobTable::at(jobs).unwrap().get(&job.id).unwrap_err().code,
            ErrorCode::UnknownJob
        );
    }

    #[test]
    fn removes_several_finished_jobs_as_one_bulk_operation() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let first = finished(&ctx, &components, dir.path());
        let second = finished(&ctx, &components, dir.path());
        let removal = remove(&components, &ctx, vec![first.clone(), second.clone()]).unwrap();
        assert!(removal.bulk);
        assert!(
            removal.journal_calls.is_empty(),
            "{:?}",
            removal.journal_calls
        );
        let targets: Vec<Option<&str>> = removal
            .records
            .iter()
            .map(|effect| effect.target.as_deref())
            .collect();
        assert_eq!(
            targets,
            [
                Some(format!("job:{}", first.id).as_str()),
                Some(format!("job:{}", second.id).as_str())
            ]
        );
        let (jobs, _) = read_jobs(&components, &ctx, &BoundArgs::new("jobs")).unwrap();
        assert!(jobs.is_empty());
    }

    #[test]
    fn refuses_to_remove_a_running_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        let error = remove(&components, &ctx, vec![job.clone()]).err().unwrap();
        assert_eq!(error.code, ErrorCode::JobState);
        assert_eq!(error.found.as_deref(), Some("running"));
        assert_eq!(error.fix, [format!("jobs \"{}\" |> stop", job.id)]);
        assert!(
            dir.path()
                .join("jobs")
                .join(format!("{}.json", job.id))
                .exists()
        );
        stop_all(&ctx, &components);
    }

    #[test]
    fn refuses_to_remove_a_suspended_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, gated(dir.path(), 0));
        effect_on(
            &components,
            &ctx,
            "suspend",
            &BoundArgs::new("suspend"),
            vec![job.clone()],
        )
        .unwrap();
        let error = remove(&components, &ctx, vec![job.clone()]).err().unwrap();
        assert_eq!(error.code, ErrorCode::JobState);
        assert_eq!(error.found.as_deref(), Some("suspended"));
        assert_eq!(error.fix, [format!("jobs \"{}\" |> stop", job.id)]);
        effect_on(
            &components,
            &ctx,
            "resume",
            &BoundArgs::new("resume"),
            vec![job.clone()],
        )
        .unwrap();
        release(dir.path());
        wait_finished(dir.path(), &job.id);
        let removal = remove(&components, &ctx, vec![job]).unwrap();
        assert_eq!(removal.records.len(), 1);
    }

    #[test]
    fn a_second_removal_reports_an_unknown_job() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = finished(&ctx, &components, dir.path());
        remove(&components, &ctx, vec![job.clone()]).unwrap();
        let error = remove(&components, &ctx, vec![job]).err().unwrap();
        assert_eq!(error.code, ErrorCode::UnknownJob);
    }

    #[test]
    fn removes_finished_jobs_older_than_the_output_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.output.spool_ttl = DurationSetting::from_millis(1);
        let (ctx, _) = context_with(dir.path(), config, None);
        let components = components();
        let finished = background(&ctx, &components, request(dir.path(), 0, 0, 1));
        wait_finished(dir.path(), &finished.id);
        let running = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        std::thread::sleep(Duration::from_millis(20));
        let (jobs, _) = read_jobs(&components, &ctx, &BoundArgs::new("jobs")).unwrap();
        let ids: Vec<&str> = jobs.iter().map(|job| job.id.as_str()).collect();
        assert_eq!(ids, [running.id.as_str()]);
        stop_all(&ctx, &components);
    }

    #[test]
    fn a_job_that_cannot_be_removed_does_not_stop_the_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.output.spool_ttl = DurationSetting::from_millis(1);
        let (ctx, _) = context_with(dir.path(), config, None);
        let components = components();
        let stuck = background(&ctx, &components, request(dir.path(), 0, 0, 1));
        wait_finished(dir.path(), &stuck.id);
        let removable = background(&ctx, &components, request(dir.path(), 0, 0, 1));
        wait_finished(dir.path(), &removable.id);
        let input = dir.path().join("jobs").join(format!("{}.input.json", stuck.id));
        fs::remove_file(&input).unwrap();
        fs::create_dir_all(input.join("blocker")).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let (jobs, _) = read_jobs(&components, &ctx, &BoundArgs::new("jobs")).unwrap();
        let ids: Vec<&str> = jobs.iter().map(|job| job.id.as_str()).collect();
        assert_eq!(ids, [stuck.id.as_str()]);
    }

    #[test]
    fn retry_without_the_stored_input_fails_instead_of_running_with_an_empty_one() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let job = finished(&ctx, &components, dir.path());
        let store = JobStore::open(&ctx, &components).unwrap();
        assert!(store.retry_request(&job.id, None).is_ok());
        fs::remove_file(
            dir.path()
                .join("jobs")
                .join(format!("{}.input.json", job.id)),
        )
        .unwrap();
        let error = store.retry_request(&job.id, None).unwrap_err();
        assert_eq!(error.code, ErrorCode::Io);
    }
}

mod jobs_events {
    use super::*;

    #[test]
    fn emits_start_output_progress_and_end_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, recorder) = context(dir.path());
        let mut with_progress = request(dir.path(), 300, 2, 3);
        with_progress.view.progress = Some("line 2".to_owned());
        let RunWait::Finished(result) = run(&ctx, &components(), with_progress, None).unwrap()
        else {
            panic!("the run moved to the background");
        };
        let events = recorder.events();
        let Event::JobStarted(started) = &events[0] else {
            panic!("the first event is not job.started");
        };
        assert_eq!(started.id, "j1");
        assert_eq!(started.label.as_deref(), Some("build"));
        let Some(Event::JobFinished(finished)) = events.last() else {
            panic!("the last event is not job.finished");
        };
        assert_eq!(finished.exit_code, Some(2));
        assert_eq!(finished.changes, 1);
        let mut lines = Vec::new();
        let mut cursors = Vec::new();
        for event in &events {
            if let Event::JobOutput(output) = event {
                if output.stream == OutputStream::Stdout {
                    lines.extend(output.lines.clone());
                }
                cursors.push(output.cursor.clone());
            }
        }
        assert_eq!(lines, ["line 1", "line 2", "line 3"]);
        assert_eq!(cursors.last().cloned(), result.output.output_cursor);
        assert!(events.iter().any(|event| matches!(event, Event::JobProgress(progress) if progress.line.as_deref() == Some("line 2"))));
    }

    #[test]
    fn never_puts_the_environment_into_events() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, recorder) = context(dir.path());
        run(&ctx, &components(), request(dir.path(), 0, 0, 1), None).unwrap();
        let text = serde_json::to_string(&recorder.events()).unwrap();
        assert!(!text.contains(SECRET_VALUE));
    }

    /// Checks that `events` hold no execution event and that the events of
    /// every job run from one `job.started` to one `job.finished`; returns
    /// the jobs in the order of their start.
    fn job_lifecycles(events: &[Event]) -> Vec<String> {
        let mut started: Vec<String> = Vec::new();
        let mut finished: HashSet<String> = HashSet::new();
        for event in events {
            let (id, starts, ends) = match event {
                Event::ExecStarted(_) | Event::ExecFinished(_) => {
                    panic!("a job emitted an execution event: {event:?}")
                }
                Event::JobStarted(job) => (&job.id, true, false),
                Event::JobOutput(output) => (&output.id, false, false),
                Event::JobProgress(progress) => (&progress.id, false, false),
                Event::JobFinished(job) => (&job.id, false, true),
                _ => continue,
            };
            if starts {
                assert!(!started.contains(id), "job {id} started twice");
                started.push(id.clone());
            } else {
                assert!(started.contains(id), "event of job {id} before job.started");
            }
            assert!(
                !finished.contains(id),
                "event of job {id} after job.finished"
            );
            if ends {
                finished.insert(id.clone());
            }
        }
        for id in &started {
            assert!(finished.contains(id), "job {id} has no job.finished");
        }
        started
    }

    fn has_finished(recorder: &Recorder, id: &str) -> bool {
        recorder
            .events()
            .iter()
            .any(|event| matches!(event, Event::JobFinished(job) if job.id == id))
    }

    #[test]
    fn a_foreground_run_emits_one_job_lifecycle_and_no_execution_events() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, recorder) = context(dir.path());
        run(&ctx, &components(), request(dir.path(), 0, 0, 2), None).unwrap();
        assert_eq!(job_lifecycles(&recorder.events()), ["j1"]);
    }

    #[test]
    fn a_background_run_and_its_retry_emit_separate_job_lifecycles() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, recorder) = context(dir.path());
        let components = components();
        let job = background(&ctx, &components, request(dir.path(), 0, 4, 1));
        wait_finished(dir.path(), &job.id);
        wait_until(|| has_finished(&recorder, &job.id));
        let args = BoundArgs::new("retry")
            .with("wait", word("forever"), false)
            .with("timeout", millis(30_000), false);
        let (records, _, _) =
            effect_on(&components, &ctx, "retry", &args, vec![job.clone()]).unwrap();
        assert_eq!(records[0].kind, RecordKind::Exec);
        let lifecycles = job_lifecycles(&recorder.events());
        assert_eq!(lifecycles.len(), 2);
        assert_eq!(lifecycles[0], job.id);
        assert_ne!(lifecycles[1], job.id);
    }
}

mod stage_registration {
    use super::*;
    use nurt_contracts::stage::StageRegistrar;

    /// Registrar that refuses the stages of one command.
    struct Refusing {
        command: &'static str,
        accepted: Vec<String>,
    }

    impl StageRegistrar for Refusing {
        fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
            let command = stage.descriptor().command;
            if command == self.command {
                return Err(Error::new(ErrorCode::UnknownCommand, "unknown command"));
            }
            self.accepted.push(command);
            Ok(())
        }
    }

    fn commands(stages: &[Arc<dyn StageImpl>]) -> Vec<String> {
        stages
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect()
    }

    #[test]
    fn registers_every_stage_in_the_order_of_the_list() {
        let mut registered: Vec<Arc<dyn StageImpl>> = Vec::new();
        crate::register(&mut registered).unwrap();
        assert_eq!(commands(&registered), commands(&crate::stages()));
        for command in [
            "run", "jobs", "stop", "retry", "suspend", "resume", "remove",
        ] {
            assert!(
                commands(&registered).iter().any(|found| found == command),
                "{command}"
            );
        }
    }

    #[test]
    fn stops_at_the_first_refused_stage() {
        let mut registrar = Refusing {
            command: "stop",
            accepted: Vec::new(),
        };
        let error = crate::register(&mut registrar).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownCommand);
        let expected: Vec<String> = commands(&crate::stages())
            .into_iter()
            .take_while(|command| command != "stop")
            .collect();
        assert_eq!(registrar.accepted, expected);
    }
}

/// Entry points of the processes started by the supervisor tests. Run
/// directly they return at once.
mod process_entries {
    use super::*;

    pub(super) fn supervisor_command() -> SupervisorCommand {
        SupervisorCommand {
            program: std::env::current_exe().unwrap(),
            args: [
                "--exact",
                "jobs::tests::process_entries::supervisor_process",
                "--ignored",
                "--test-threads=1",
            ]
            .iter()
            .map(Into::into)
            .collect(),
        }
    }

    #[test]
    #[ignore = "started as the supervisor process by the supervisor tests"]
    fn supervisor_process() {
        let Some(dir) = std::env::var_os(STATE_ENV) else {
            return;
        };
        let (ctx, _) = context(Path::new(&dir));
        let _ = supervisor::serve(&ctx, &components());
    }

    #[test]
    #[ignore = "started as the command line process by the supervisor tests"]
    fn command_line_process() {
        let Some(dir) = std::env::var_os(STATE_ENV) else {
            return;
        };
        let dir = PathBuf::from(dir);
        let (ctx, _) = context_with(
            &dir,
            Config::default(),
            Some(Supervision::Detached(supervisor_command())),
        );
        let host = run_host(&ctx, components());
        let RunWait::Background(job) = host
            .run(&ctx, gated(&dir, 2), Some(Duration::ZERO))
            .unwrap()
        else {
            panic!("the run did not move to the background");
        };
        fs::write(dir.join("started-job"), job.id).unwrap();
    }
}

mod jobs_supervisor_survives_cli {
    use std::process::{Command, Stdio};

    use super::process_entries::supervisor_command;
    use super::*;

    #[test]
    fn job_finishes_after_the_command_line_process_exits() {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "jobs::tests::process_entries::command_line_process",
                "--ignored",
                "--test-threads=1",
            ])
            .env(STATE_ENV, dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let id = fs::read_to_string(dir.path().join("started-job")).unwrap();
        assert_eq!(job_file(dir.path(), &id).record.state, JobState::Running);
        release(dir.path());
        let file = wait_finished(dir.path(), &id);
        assert_eq!(file.record.state, JobState::Exited);
        assert_eq!(file.record.exit_code, Some(0));
        assert!(file.detached);
        assert_ne!(file.supervisor_pid, std::process::id());
        assert_eq!(file.record.stdout_lines, Some(2));
    }

    #[test]
    fn job_is_lost_when_its_supervisor_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let command = supervisor_command();
        let mut supervisor = Command::new(&command.program)
            .args(&command.args)
            .env(STATE_ENV, dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        wait_until(|| table.host_running().unwrap());
        let (ctx, _) = context_with(
            dir.path(),
            Config::default(),
            Some(Supervision::Detached(command)),
        );
        let components = components();
        let host = run_host(&ctx, components.clone());
        let RunWait::Background(job) = host
            .run(
                &ctx,
                request(dir.path(), 30_000, 0, 1),
                Some(Duration::ZERO),
            )
            .unwrap()
        else {
            panic!("the run did not move to the background");
        };
        assert_eq!(
            job_file(dir.path(), &job.id).supervisor_pid,
            supervisor.id()
        );
        supervisor.kill().unwrap();
        supervisor.wait().unwrap();
        let (jobs, _) = read_jobs(&components, &ctx, &BoundArgs::new("jobs")).unwrap();
        assert_eq!(jobs[0].state, JobState::Lost);
        let error = effect_on(
            &components,
            &ctx,
            "stop",
            &BoundArgs::new("stop"),
            vec![*job],
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::JobState);
    }

    #[test]
    fn detached_run_that_ends_in_time_returns_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let command = supervisor_command();
        let mut supervisor = Command::new(&command.program)
            .args(&command.args)
            .env(STATE_ENV, dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let table = JobTable::at(dir.path().join("jobs")).unwrap();
        wait_until(|| table.host_running().unwrap());
        let (ctx, recorder) = context_with(
            dir.path(),
            Config::default(),
            Some(Supervision::Detached(command)),
        );
        let host = run_host(&ctx, components());
        let outcome = host
            .run(
                &ctx,
                request(dir.path(), 50, 5, 2),
                Some(Duration::from_secs(10)),
            )
            .unwrap();
        let RunWait::Finished(result) = outcome else {
            panic!("the run moved to the background");
        };
        assert_eq!(result.end.exit_code, Some(5));
        assert_eq!(result.output.stdout, "line 1\nline 2\n");
        let events = recorder.events();
        assert!(matches!(events.first(), Some(Event::JobStarted(_))));
        assert!(matches!(events.last(), Some(Event::JobFinished(_))));
        let mut missing = request(dir.path(), 0, 0, 0);
        missing.spawn.program = dir.path().join("missing");
        let error = host
            .run(&ctx, missing, Some(Duration::from_secs(1)))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProgramNotFound);
        supervisor.kill().unwrap();
        supervisor.wait().unwrap();
    }
}

mod jobs_matches_reference_behaviour {
    use super::*;

    #[test]
    fn stops_every_running_job_of_a_listing() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        for _ in 0..3 {
            background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        }
        stop_all(&ctx, &components);
        let (jobs, _) = read_jobs(&components, &ctx, &BoundArgs::new("jobs")).unwrap();
        assert!(jobs.iter().all(|job| job.state == JobState::Stopped));
        assert_eq!(jobs.len(), 3);
    }

    #[test]
    fn reports_exit_code_and_signal_of_finished_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let components = components();
        let failed = background(&ctx, &components, request(dir.path(), 0, 2, 0));
        let killed = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
        let args = BoundArgs::new("stop").with("signal", word("kill"), false);
        effect_on(&components, &ctx, "stop", &args, vec![killed.clone()]).unwrap();
        let failed = wait_finished(dir.path(), &failed.id).record;
        assert_eq!((failed.exit_code, failed.signal), (Some(2), None));
        let killed = job_file(dir.path(), &killed.id).record;
        assert_eq!(
            (killed.exit_code, killed.signal),
            (None, Some(Signal::Kill))
        );
    }

    #[test]
    fn result_reads_kept_output_with_another_filter_without_running_again() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let control = Arc::new(FakeControl::default());
        let components = components_with(control.clone());
        let job = background(&ctx, &components, request(dir.path(), 0, 0, 12));
        wait_finished(dir.path(), &job.id);
        let args = BoundArgs::new("jobs")
            .with("id", text(&job.id), false)
            .with("view", word("result"), false)
            .with("filter", text("line 1"), false);
        let (jobs, _) = read_jobs(&components, &ctx, &args).unwrap();
        assert_eq!(
            jobs[0].stdout.as_deref(),
            Some("line 1\nline 10\nline 11\nline 12\n")
        );
        assert_eq!(control.spawned.lock().unwrap().len(), 1);
    }

    #[test]
    fn keeps_the_request_of_every_run_for_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = context(dir.path());
        let launch = Launch::new(&ctx, request(dir.path(), 0, 0, 0), None);
        assert_eq!(launch.command_name, "run");
        let job = background(&ctx, &components(), request(dir.path(), 0, 0, 0));
        let file = wait_finished(dir.path(), &job.id);
        assert_eq!(file.request.0.spawn.env, Vec::new());
        assert_eq!(file.request.0.command, request(dir.path(), 0, 0, 0).command);
        assert_eq!(file.command_name, "run");
    }
}
