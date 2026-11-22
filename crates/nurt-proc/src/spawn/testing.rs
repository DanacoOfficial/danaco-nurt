// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test doubles of the execution context and of the parts a run uses.
//!
//! [`StdControl`] starts programs with `std::process::Command`, without
//! process groups or limits, so the arguments, environment, working directory
//! and input prepared for a run reach a real program. [`DirectRunHost`] runs
//! requests through it and returns their complete output.

use std::ffi::OsString;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::config::Config;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetIndex, TargetSpec,
};
use nurt_contracts::record::{Job, JobState, KilledBy, Signal};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx, OutputStream};

use crate::iface::{
    OutputSink, ProcessControl, ProcessEnd, RenderedOutput, RunHost, RunRequest, RunResult,
    RunWait, RunningProcess, SpawnRequest, StdinSource, TrackOutcome,
};

/// Size of the read buffer of an output stream.
const READ_BUFFER: usize = 64 * 1024;
/// Interval between checks of a program that runs with a waiting limit.
const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Process identifier reported for runs that move to the background.
const BACKGROUND_PID: u32 = 4242;

/// Path of the `proc_fixture` example built next to the test binaries.
pub(crate) fn fixture_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let target = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(PathBuf::from)
        .unwrap_or_default();
    target
        .join("examples")
        .join(format!("proc_fixture{}", std::env::consts::EXE_SUFFIX))
}

/// Wall time now.
pub(crate) fn wall_now() -> WallTime {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    WallTime::from_unix_nanos(i128::try_from(nanos).unwrap_or(i128::MAX))
}

/// Clock that follows the system clock.
pub(crate) struct TestClock {
    origin: Instant,
}

impl TestClock {
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        wall_now()
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

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.wait_timeout(duration).map_or(Ok(()), Err)
    }
}

/// Host with the given operating system and environment.
pub(crate) struct TestHost {
    info: HostInfo,
    vars: Vec<(OsString, OsString)>,
}

impl TestHost {
    pub(crate) fn new(os: Os, vars: &[(&str, &str)]) -> Self {
        let windows = os == Os::Windows;
        Self {
            info: HostInfo {
                os,
                os_version: "1.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "test".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from(if windows {
                    "C:/Users/user"
                } else {
                    "/home/user"
                }),
                temp: std::env::temp_dir(),
                path_style: if windows {
                    PathStyle::Windows
                } else {
                    PathStyle::Posix
                },
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 2,
            },
            vars: vars
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect(),
        }
    }

    /// Host of the running system whose environment holds `vars` and the
    /// variables a program needs to start there.
    pub(crate) fn current(vars: &[(&str, &str)]) -> Self {
        let os = if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        };
        let mut host = Self::new(os, vars);
        for name in ["SystemRoot", "ComSpec", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                host.vars.push((OsString::from(name), value));
            }
        }
        host
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        self.vars
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, value)| value.clone())
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.vars.clone()
    }
}

/// Execution context with `host`, `config` and `options`.
pub(crate) fn context(host: TestHost, config: Config, options: RequestOptions) -> ExecCtx {
    ExecCtx::builder(Arc::new(TestClock::new()), Arc::new(host))
        .config(Arc::new(config))
        .options(Arc::new(options))
        .build()
}

/// Output kept in memory.
#[derive(Default)]
pub(crate) struct MemorySink {
    streams: Mutex<(Vec<u8>, Vec<u8>)>,
}

impl MemorySink {
    pub(crate) fn stdout(&self) -> Vec<u8> {
        self.streams
            .lock()
            .map(|streams| streams.0.clone())
            .unwrap_or_default()
    }

    pub(crate) fn stderr(&self) -> Vec<u8> {
        self.streams
            .lock()
            .map(|streams| streams.1.clone())
            .unwrap_or_default()
    }
}

impl OutputSink for MemorySink {
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()> {
        let mut streams = self.streams.lock().map_err(|_| poisoned())?;
        match stream {
            OutputStream::Stdout => streams.0.extend_from_slice(bytes),
            OutputStream::Stderr => streams.1.extend_from_slice(bytes),
        }
        Ok(())
    }

    fn finish(&self, _stream: OutputStream) -> Result<()> {
        Ok(())
    }

    fn last_activity(&self) -> Option<Duration> {
        None
    }
}

fn poisoned() -> Error {
    Error::new(ErrorCode::StageFailed, "a test lock was poisoned")
}

/// Starts programs with `std::process::Command`.
pub(crate) struct StdControl;

impl ProcessControl for StdControl {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn OutputSink>,
        _cancel: CancelToken,
    ) -> Result<Box<dyn RunningProcess>> {
        Ok(Box::new(self.start(&request, sink)?))
    }
}

impl StdControl {
    /// Starts the program of `request`.
    pub(crate) fn start(
        &self,
        request: &SpawnRequest,
        sink: Arc<dyn OutputSink>,
    ) -> Result<StdProcess> {
        let mut command = Command::new(&request.program);
        add_arguments(&mut command, request);
        command
            .current_dir(&request.cwd)
            .env_clear()
            .envs(request.env.iter().map(|(name, value)| (name, value)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let input = match &request.stdin {
            StdinSource::Null => {
                command.stdin(Stdio::null());
                None
            }
            StdinSource::Bytes(bytes) => {
                command.stdin(Stdio::piped());
                Some(bytes.clone())
            }
            StdinSource::File(path) => {
                let file = std::fs::File::open(path)
                    .map_err(|error| Error::new(ErrorCode::Spawn, error.to_string()))?;
                command.stdin(Stdio::from(file));
                None
            }
        };
        let mut child = command
            .spawn()
            .map_err(|error| Error::new(ErrorCode::Spawn, error.to_string()))?;
        let closed_early = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            let flag = Arc::clone(&closed_early);
            threads.push(std::thread::spawn(move || {
                if let Err(error) = stdin.write_all(&bytes)
                    && error.kind() == ErrorKind::BrokenPipe
                {
                    flag.store(true, Ordering::SeqCst);
                }
            }));
        }
        let merged = request.merge_stderr;
        if let Some(stdout) = child.stdout.take() {
            threads.push(pump(stdout, OutputStream::Stdout, Arc::clone(&sink)));
        }
        if let Some(stderr) = child.stderr.take() {
            let stream = if merged {
                OutputStream::Stdout
            } else {
                OutputStream::Stderr
            };
            threads.push(pump(stderr, stream, sink));
        }
        Ok(StdProcess {
            child,
            threads,
            started: wall_now(),
            closed_early,
        })
    }
}

#[cfg(windows)]
fn add_arguments(command: &mut Command, request: &SpawnRequest) {
    use std::os::windows::process::CommandExt;
    match &request.raw_command_line {
        Some(line) => {
            let text = line.to_string_lossy();
            command.raw_arg(text.strip_prefix("cmd.exe ").unwrap_or(&text));
        }
        None => {
            command.args(&request.args);
        }
    }
}

#[cfg(not(windows))]
fn add_arguments(command: &mut Command, request: &SpawnRequest) {
    command.args(&request.args);
}

fn pump(
    mut reader: impl Read + Send + 'static,
    stream: OutputStream,
    sink: Arc<dyn OutputSink>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = vec![0; READ_BUFFER];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 || sink.write(stream, &buffer[..read]).is_err() {
                break;
            }
        }
        let _ = sink.finish(stream);
    })
}

/// Program started by [`StdControl`].
pub(crate) struct StdProcess {
    child: Child,
    threads: Vec<JoinHandle<()>>,
    started: WallTime,
    closed_early: Arc<AtomicBool>,
}

impl StdProcess {
    /// The program closed its input before all input was written.
    pub(crate) fn closed_input_early(&self) -> bool {
        self.closed_early.load(Ordering::SeqCst)
    }

    fn end(&mut self, status: std::process::ExitStatus) -> ProcessEnd {
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        ProcessEnd {
            exit_code: status.code(),
            signal: None,
            signal_number: signal_number(status),
            killed_by: None,
            limit_hit: None,
            finished: wall_now(),
            usage: crate::iface::ResourceUsage::default(),
            suspended_total: Duration::ZERO,
            stdin_closed_early: self.closed_input_early(),
        }
    }
}

#[cfg(unix)]
fn signal_number(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn signal_number(_status: std::process::ExitStatus) -> Option<i32> {
    None
}

fn io_error(error: &std::io::Error) -> Error {
    Error::new(ErrorCode::Io, error.to_string())
}

impl RunningProcess for StdProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn started(&self) -> WallTime {
        self.started
    }

    fn wait(&mut self, timeout: Option<Duration>) -> Result<Option<ProcessEnd>> {
        let Some(timeout) = timeout else {
            let status = self.child.wait().map_err(|error| io_error(&error))?;
            return Ok(Some(self.end(status)));
        };
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|error| io_error(&error))? {
                return Ok(Some(self.end(status)));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn stop(&mut self, _signal: Signal, _grace: Duration) -> Result<ProcessEnd> {
        self.child.kill().map_err(|error| io_error(&error))?;
        let status = self.child.wait().map_err(|error| io_error(&error))?;
        let mut end = self.end(status);
        end.killed_by = Some(KilledBy::Stop);
        Ok(end)
    }

    fn suspend(&mut self) -> Result<()> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "test processes cannot be suspended",
        ))
    }

    fn resume(&mut self) -> Result<()> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "test processes cannot be resumed",
        ))
    }
}

/// Runs a request to its end through [`StdControl`].
pub(crate) fn run_direct(request: &RunRequest) -> Result<(RunResult, bool)> {
    let sink = Arc::new(MemorySink::default());
    let mut process = StdControl.start(&request.spawn, Arc::clone(&sink) as Arc<dyn OutputSink>)?;
    let end = process
        .wait(None)?
        .ok_or_else(|| Error::new(ErrorCode::StageFailed, "the program did not end"))?;
    let stdout = String::from_utf8_lossy(&sink.stdout()).into_owned();
    let stderr = String::from_utf8_lossy(&sink.stderr()).into_owned();
    let output = RenderedOutput {
        stdout_bytes: stdout.len() as u64,
        stderr_bytes: stderr.len() as u64,
        stdout_lines: stdout.lines().count() as u64,
        stderr_lines: stderr.lines().count() as u64,
        stdout,
        stderr,
        decoded_as: "utf-8".to_owned(),
        ..RenderedOutput::default()
    };
    let result = RunResult {
        pid: process.pid(),
        started: process.started(),
        end,
        output,
        track: TrackOutcome {
            watch: request.track.watch.clone(),
            ..TrackOutcome::default()
        },
        op: None,
        job: Some("j1".to_owned()),
    };
    Ok((result, process.closed_input_early()))
}

/// How [`DirectRunHost`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HostMode {
    /// Run the program to its end.
    #[default]
    Run,
    /// Report every run as moved to the background without starting it.
    Background,
    /// Run the program and report it as ended by the console.
    KilledBy(KilledBy),
}

/// Run host that records the requests and runs them through [`StdControl`].
#[derive(Default)]
pub(crate) struct DirectRunHost {
    pub(crate) mode: HostMode,
    pub(crate) requests: Mutex<Vec<(RunRequest, Option<Duration>)>>,
}

impl DirectRunHost {
    pub(crate) fn new(mode: HostMode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn recorded(&self) -> Vec<(RunRequest, Option<Duration>)> {
        self.requests
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }
}

impl RunHost for DirectRunHost {
    fn run(&self, _ctx: &ExecCtx, request: RunRequest, wait: Option<Duration>) -> Result<RunWait> {
        self.requests
            .lock()
            .map_err(|_| poisoned())?
            .push((request.clone(), wait));
        match self.mode {
            HostMode::Background => Ok(RunWait::Background(Box::new(Job {
                id: "j1".to_owned(),
                command: request.command.clone(),
                pid: u64::from(BACKGROUND_PID),
                state: JobState::Running,
                label: request.label.clone(),
                ..Job::default()
            }))),
            HostMode::Run => Ok(RunWait::Finished(Box::new(run_direct(&request)?.0))),
            HostMode::KilledBy(by) => {
                let (mut result, _) = run_direct(&request)?;
                result.end.exit_code = None;
                result.end.killed_by = Some(by);
                Ok(RunWait::Finished(Box::new(result)))
            }
        }
    }
}

/// Transaction that accepts only irreversible markers.
#[derive(Default)]
pub(crate) struct NullTx {
    pub(crate) markers: Vec<IrreversibleEntry>,
}

fn not_expected() -> Error {
    Error::new(ErrorCode::StageFailed, "the stage changed files directly")
}

impl MutationTx for NullTx {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, _targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        Err(not_expected())
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(not_expected())
    }

    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        Err(not_expected())
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Err(not_expected())
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.markers.push(marker);
    }

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Err(not_expected())
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Err(not_expected())
    }
}
