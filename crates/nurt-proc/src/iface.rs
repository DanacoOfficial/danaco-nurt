// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interfaces between process control, output capture, change detection and
//! job management.
//!
//! A run passes through four parts: the change tracker prepares the watched
//! scope, process control starts the program and feeds its output to the
//! output store, the run host waits for the end or moves the run to the
//! background, and the change tracker records what the program changed.
//! Each part depends on the others only through the types in this module.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::Result;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{
    Change, ExecShown, Job, KilledBy, Launcher, LimitHit, Preimage, Progress, Signal, Tracking,
};
use nurt_contracts::report::NotableEntry;
use nurt_contracts::stage::{CancelToken, ExecCtx, OutputStream};

/// Standard input of a started program.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum StdinSource {
    /// Empty input (`/dev/null`, `NUL`).
    #[default]
    Null,
    /// The given bytes, then end of input.
    Bytes(Vec<u8>),
    /// The content of a file, opened as the input handle.
    File(PathBuf),
}

/// Scheduling priority of a started program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Priority {
    /// Below normal.
    Low,
    /// Unchanged.
    #[default]
    Normal,
    /// Above normal; falls back to normal without the privilege.
    High,
    /// Unix niceness from 0 to 19.
    Nice(u8),
}

/// Limits applied to the whole process tree of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessLimits {
    /// Wall-clock limit; `None` waits without limit.
    pub(crate) timeout: Option<Duration>,
    /// Limit of time without output and without processor use.
    pub(crate) idle_timeout: Option<Duration>,
    /// Time between the termination request and the forced stop.
    pub(crate) kill_grace: Duration,
    /// Processor time of the whole tree.
    pub(crate) max_cpu: Option<Duration>,
    /// Memory of the whole tree, in bytes.
    pub(crate) max_memory: Option<u64>,
    /// Scheduling priority.
    pub(crate) priority: Priority,
}

/// Fully resolved request to start one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpawnRequest {
    /// Resolved program file.
    pub(crate) program: PathBuf,
    /// Arguments, without the program name.
    pub(crate) args: Vec<OsString>,
    /// How the program is started.
    pub(crate) launcher: Launcher,
    /// Exact command line for `CreateProcessW` when the arguments were quoted
    /// for the command interpreter; `None` lets process control quote `args`.
    pub(crate) raw_command_line: Option<OsString>,
    /// Working directory.
    pub(crate) cwd: PathBuf,
    /// Complete environment of the child.
    pub(crate) env: Vec<(OsString, OsString)>,
    /// Standard input.
    pub(crate) stdin: StdinSource,
    /// Standard error is written to the standard output stream.
    pub(crate) merge_stderr: bool,
    /// Limits of the process tree.
    pub(crate) limits: ProcessLimits,
}

/// Processor and memory use of a process tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ResourceUsage {
    /// User and system processor time.
    pub(crate) cpu_time: Duration,
    /// Peak resident memory, in bytes.
    pub(crate) max_rss: u64,
}

/// How a started program ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessEnd {
    /// Exit code; `None` when ended by a signal or stopped.
    pub(crate) exit_code: Option<i32>,
    /// Signal that ended the program.
    pub(crate) signal: Option<Signal>,
    /// Number of that signal.
    pub(crate) signal_number: Option<i32>,
    /// Reason the console ended the program, when it did.
    pub(crate) killed_by: Option<KilledBy>,
    /// Resource limit that ended the program.
    pub(crate) limit_hit: Option<LimitHit>,
    /// Time the program ended.
    pub(crate) finished: WallTime,
    /// Resource use of the whole tree.
    pub(crate) usage: ResourceUsage,
    /// Time spent suspended.
    pub(crate) suspended_total: Duration,
    /// The program closed its standard input before all given input was
    /// written.
    pub(crate) stdin_closed_early: bool,
}

/// Receiver of the output of a started program.
pub(crate) trait OutputSink: Send + Sync {
    /// Appends bytes read from one stream.
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()>;

    /// Marks the end of one stream.
    fn finish(&self, stream: OutputStream) -> Result<()>;

    /// Monotonic time of the last byte received; `None` before any output.
    /// Process control compares the value only with the previous one, so
    /// the clock of the sink does not have to be the clock of the caller.
    fn last_activity(&self) -> Option<Duration>;
}

/// Started program and its process tree.
pub(crate) trait RunningProcess: Send {
    /// Process identifier of the program.
    fn pid(&self) -> u32;

    /// Time the program was started.
    fn started(&self) -> WallTime;

    /// Waits up to `timeout` (`None`: until the end) while enforcing the
    /// limits; returns `None` when the program still runs.
    fn wait(&mut self, timeout: Option<Duration>) -> Result<Option<ProcessEnd>>;

    /// Requests termination, then stops the whole tree after `grace`.
    fn stop(&mut self, signal: Signal, grace: Duration) -> Result<ProcessEnd>;

    /// Suspends the whole tree.
    fn suspend(&mut self) -> Result<()>;

    /// Resumes the whole tree.
    fn resume(&mut self) -> Result<()>;
}

/// Starts programs.
pub(crate) trait ProcessControl: Send + Sync {
    /// Starts the program in its own process group or job object and feeds
    /// its output to `sink` until both streams end.
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn OutputSink>,
        cancel: CancelToken,
    ) -> Result<Box<dyn RunningProcess>>;
}

/// Output streams selected for a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ShowMode {
    /// Standard output, with standard error after a failure.
    #[default]
    Auto,
    /// Both streams.
    Both,
    /// Standard output only.
    Stdout,
    /// Standard error only.
    Stderr,
    /// Error lines with their context.
    Errors,
    /// Summary lines and the end of the output.
    Summary,
    /// Nothing.
    None,
}

/// What part of the captured output a response contains.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct OutputView {
    /// Selected streams.
    pub(crate) show: ShowMode,
    /// First lines kept.
    pub(crate) head: Option<u32>,
    /// Last lines kept.
    pub(crate) tail: Option<u32>,
    /// Regular expression selecting lines.
    pub(crate) filter: Option<String>,
    /// Lines of context around selected lines.
    pub(crate) context: u32,
    /// Repeated consecutive lines are collapsed.
    pub(crate) dedupe: bool,
    /// No output after exit code 0.
    pub(crate) quiet_on_success: bool,
    /// Patterns whose last matches form the summary lines.
    pub(crate) summary: Vec<String>,
    /// Pattern of progress lines.
    pub(crate) progress: Option<String>,
    /// Requested encoding; `None` detects it.
    pub(crate) encoding: Option<String>,
}

/// Output prepared for a response.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct RenderedOutput {
    /// Shown standard output.
    pub(crate) stdout: String,
    /// Shown standard error.
    pub(crate) stderr: String,
    /// Bytes written to standard output.
    pub(crate) stdout_bytes: u64,
    /// Bytes written to standard error.
    pub(crate) stderr_bytes: u64,
    /// Raw bytes of standard output (or of the merged output) that an
    /// incremental read covered, from its start cursor to its output cursor.
    pub(crate) stdout_read: u64,
    /// Raw bytes of standard error that an incremental read covered.
    pub(crate) stderr_read: u64,
    /// Lines written to standard output.
    pub(crate) stdout_lines: u64,
    /// Lines written to standard error.
    pub(crate) stderr_lines: u64,
    /// Encoding used to decode the output.
    pub(crate) decoded_as: String,
    /// What part of each stream is shown.
    pub(crate) shown: ExecShown,
    /// Last matches of the summary patterns.
    pub(crate) summary_lines: Vec<String>,
    /// Last progress line.
    pub(crate) progress: Option<Progress>,
    /// Combined output file, when the streams are merged.
    pub(crate) output_file: Option<PathBuf>,
    /// Standard output file.
    pub(crate) stdout_file: Option<PathBuf>,
    /// Standard error file.
    pub(crate) stderr_file: Option<PathBuf>,
    /// Cursor after the last complete line.
    pub(crate) output_cursor: Option<String>,
    /// SHA-256 of the whole output, hexadecimal.
    pub(crate) output_sha256: Option<String>,
    /// Notes about the output.
    pub(crate) notes: Vec<NotableEntry>,
}

/// Captured output of one run, kept in spool files.
pub(crate) trait OutputStore: OutputSink {
    /// Output of the finished or running program for a response.
    fn render(&self, view: &OutputView, exit_code: Option<i32>) -> Result<RenderedOutput>;

    /// Complete lines written after `cursor` (`None`: from the start).
    fn read_since(&self, cursor: Option<&str>, view: &OutputView) -> Result<RenderedOutput>;
}

/// Creates and reopens spool files.
pub(crate) trait SpoolFactory: Send + Sync {
    /// Creates the spool of a new run named `id`.
    fn create(&self, ctx: &ExecCtx, id: &str, merge_stderr: bool) -> Result<Arc<dyn OutputStore>>;

    /// Opens the spool of an earlier run.
    fn open(&self, ctx: &ExecCtx, id: &str) -> Result<Arc<dyn OutputStore>>;

    /// Removes the spool of a run.
    fn remove(&self, ctx: &ExecCtx, id: &str) -> Result<()>;
}

/// Requested change detection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TrackMode {
    /// Scan, accelerated by file system events where available.
    #[default]
    Auto,
    /// Scan before and after.
    Scan,
    /// File system events, verified by scanning the touched paths.
    Events,
    /// No detection.
    None,
}

/// Scope of change detection for one run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TrackSpec {
    /// Detection mode.
    pub(crate) mode: TrackMode,
    /// Watched directories.
    pub(crate) watch: Vec<PathBuf>,
    /// Ignored names within the watched directories.
    pub(crate) ignore: Vec<String>,
    /// Paths whose previous content is kept regardless of the limits.
    pub(crate) preserve: Vec<PathBuf>,
    /// The program changes exactly these files; their previous content is
    /// kept before the start.
    pub(crate) exact: Option<Vec<PathBuf>>,
    /// Keep previous content within the configured limits.
    pub(crate) preimage: bool,
    /// Compare content hashes, not only size and modification time.
    pub(crate) scan_hash: bool,
    /// Time limit of the whole run; the scans before and after the program
    /// end with it. `None` does not limit them.
    pub(crate) timeout: Option<Duration>,
}

/// What the change tracker found after a run.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TrackOutcome {
    /// Changed files.
    pub(crate) changes: Vec<Change>,
    /// Mechanism used.
    pub(crate) tracking: Tracking,
    /// Watched directories.
    pub(crate) watch: Vec<PathBuf>,
    /// Previous content kept before the run.
    pub(crate) preimage: Preimage,
    /// Files with kept previous content.
    pub(crate) preimage_entries: u64,
    /// Bytes of kept previous content.
    pub(crate) preimage_bytes: u64,
    /// Notes about the detection.
    pub(crate) notes: Vec<NotableEntry>,
}

/// State captured before a run.
pub(crate) trait PreparedScan: Send {
    /// Compares with the state after the run and records the changes in the
    /// transaction.
    fn finish(self: Box<Self>, ctx: &ExecCtx, tx: &mut dyn MutationTx) -> Result<TrackOutcome>;
}

/// Detects file changes made by programs.
pub(crate) trait ChangeTracker: Send + Sync {
    /// Captures the watched scope and keeps previous content before a run.
    fn prepare(
        &self,
        ctx: &ExecCtx,
        spec: &TrackSpec,
        tx: &mut dyn MutationTx,
    ) -> Result<Box<dyn PreparedScan>>;
}

/// Kind of the command that started a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RunKind {
    /// `run`.
    #[default]
    Run,
    /// `retry`.
    Retry,
    /// `test.run`, `test.failed`.
    Test,
    /// `analyze.check`, `analyze.format`.
    Analyze,
}

/// One run as requested by a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunRequest {
    /// Command as written, program first.
    pub(crate) command: Vec<String>,
    /// Program found in the search path, as reported; for a batch file the
    /// script, while `spawn.program` is the command interpreter.
    pub(crate) program: PathBuf,
    /// Names of the variables set by the command.
    pub(crate) env_keys: Vec<String>,
    /// Start request.
    pub(crate) spawn: SpawnRequest,
    /// Change detection.
    pub(crate) track: TrackSpec,
    /// Response view of the output.
    pub(crate) view: OutputView,
    /// Kind of the command.
    pub(crate) kind: RunKind,
    /// Label of the job.
    pub(crate) label: Option<String>,
    /// Description of the job.
    pub(crate) description: Option<String>,
    /// Session that started the run.
    pub(crate) session: Option<String>,
    /// Job repeated by this run.
    pub(crate) retry_of: Option<String>,
    /// Matches of the external effect classifier.
    pub(crate) external_effect: Vec<String>,
    /// Data passed to the job finisher of a tool run.
    pub(crate) finish_data: Option<serde_json::Value>,
}

/// Finished run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunResult {
    /// Process identifier.
    pub(crate) pid: u32,
    /// Start time.
    pub(crate) started: WallTime,
    /// How the program ended.
    pub(crate) end: ProcessEnd,
    /// Output for the response.
    pub(crate) output: RenderedOutput,
    /// Detected changes.
    pub(crate) track: TrackOutcome,
    /// Journal entry of the changes.
    pub(crate) op: Option<u64>,
    /// Job identifier, when the run was a job.
    pub(crate) job: Option<String>,
}

/// Outcome of waiting for a run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunWait {
    /// The run ended within the waiting time.
    Finished(Box<RunResult>),
    /// The run continues in the background.
    Background(Box<Job>),
}

/// Executes runs in this process or through the job supervisor.
pub(crate) trait RunHost: Send + Sync {
    /// Starts the run and waits up to `wait` (`None`: until the end).
    fn run(&self, ctx: &ExecCtx, request: RunRequest, wait: Option<Duration>) -> Result<RunWait>;
}

/// Parts used by the run host.
#[derive(Clone)]
pub(crate) struct Components {
    /// Process control.
    pub(crate) control: Arc<dyn ProcessControl>,
    /// Spool files.
    pub(crate) spool: Arc<dyn SpoolFactory>,
    /// Change detection.
    pub(crate) tracker: Arc<dyn ChangeTracker>,
}
