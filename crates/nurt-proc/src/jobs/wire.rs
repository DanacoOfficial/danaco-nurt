// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Serialized form of run requests and run results.
//!
//! Job files keep the request of every run so that `retry` can repeat it, and
//! the detached supervisor receives requests and returns results through
//! files of the state directory. The types of the run interfaces carry no
//! serialization of their own; the definitions below mirror them field by
//! field.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::record::{
    Change, ExecDiagnostics, ExecShown, KilledBy, Launcher, LimitHit, Preimage, Progress, Signal,
    Tracking,
};
use nurt_contracts::report::NotableEntry;
use serde::{Deserialize, Serialize};

use crate::iface::{
    OutputView, Priority, ProcessEnd, ProcessLimits, RenderedOutput, ResourceUsage, RunKind,
    RunRequest, RunResult, ShowMode, SpawnRequest, StdinSource, TrackMode, TrackOutcome, TrackSpec,
};

/// Serialized form of a shared environment: the list of its variables.
mod shared_env {
    use std::ffi::OsString;
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(
        env: &Arc<[(OsString, OsString)]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        env.as_ref().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<[(OsString, OsString)]>, D::Error> {
        Vec::<(OsString, OsString)>::deserialize(deserializer).map(Arc::from)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "StdinSource", rename_all = "snake_case")]
enum StdinSourceDef {
    Null,
    Bytes(Vec<u8>),
    File(PathBuf),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Priority", rename_all = "snake_case")]
enum PriorityDef {
    Low,
    Normal,
    High,
    Nice(u8),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ProcessLimits")]
struct ProcessLimitsDef {
    timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    kill_grace: Duration,
    max_cpu: Option<Duration>,
    max_memory: Option<u64>,
    #[serde(with = "PriorityDef")]
    priority: Priority,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SpawnRequest")]
struct SpawnRequestDef {
    program: PathBuf,
    args: Vec<OsString>,
    launcher: Launcher,
    raw_command_line: Option<OsString>,
    cwd: PathBuf,
    #[serde(with = "shared_env")]
    env: Arc<[(OsString, OsString)]>,
    #[serde(with = "StdinSourceDef")]
    stdin: StdinSource,
    merge_stderr: bool,
    #[serde(with = "ProcessLimitsDef")]
    limits: ProcessLimits,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "TrackMode", rename_all = "snake_case")]
enum TrackModeDef {
    Auto,
    Scan,
    Events,
    None,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "TrackSpec")]
struct TrackSpecDef {
    #[serde(with = "TrackModeDef")]
    mode: TrackMode,
    watch: Vec<PathBuf>,
    ignore: Vec<String>,
    preserve: Vec<PathBuf>,
    exact: Option<Vec<PathBuf>>,
    preimage: bool,
    scan_hash: bool,
    #[serde(default)]
    timeout: Option<Duration>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ShowMode", rename_all = "snake_case")]
enum ShowModeDef {
    Auto,
    Both,
    Stdout,
    Stderr,
    Errors,
    Summary,
    None,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "OutputView")]
struct OutputViewDef {
    #[serde(with = "ShowModeDef")]
    show: ShowMode,
    head: Option<u32>,
    tail: Option<u32>,
    filter: Option<String>,
    context: u32,
    dedupe: bool,
    #[serde(default)]
    max_bytes: u32,
    #[serde(default)]
    compact: bool,
    quiet_on_success: bool,
    summary: Vec<String>,
    progress: Option<String>,
    encoding: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RunKind", rename_all = "snake_case")]
pub(crate) enum RunKindDef {
    Run,
    Retry,
    Test,
    Analyze,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RunRequest")]
pub(crate) struct RunRequestDef {
    command: Vec<String>,
    program: PathBuf,
    env_keys: Vec<String>,
    #[serde(with = "SpawnRequestDef")]
    spawn: SpawnRequest,
    #[serde(with = "TrackSpecDef")]
    track: TrackSpec,
    #[serde(with = "OutputViewDef")]
    view: OutputView,
    #[serde(with = "RunKindDef")]
    kind: RunKind,
    label: Option<String>,
    description: Option<String>,
    session: Option<String>,
    retry_of: Option<String>,
    external_effect: Vec<String>,
    finish_data: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ResourceUsage")]
struct ResourceUsageDef {
    cpu_time: Duration,
    max_rss: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ProcessEnd")]
struct ProcessEndDef {
    exit_code: Option<i32>,
    signal: Option<Signal>,
    signal_number: Option<i32>,
    killed_by: Option<KilledBy>,
    limit_hit: Option<LimitHit>,
    finished: WallTime,
    #[serde(with = "ResourceUsageDef")]
    usage: ResourceUsage,
    suspended_total: Duration,
    stdin_closed_early: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RenderedOutput")]
struct RenderedOutputDef {
    stdout: String,
    stderr: String,
    stdout_bytes: u64,
    stderr_bytes: u64,
    #[serde(default)]
    stdout_read: u64,
    #[serde(default)]
    stderr_read: u64,
    stdout_lines: u64,
    stderr_lines: u64,
    decoded_as: String,
    shown: ExecShown,
    summary_lines: Vec<String>,
    progress: Option<Progress>,
    output_file: Option<PathBuf>,
    stdout_file: Option<PathBuf>,
    stderr_file: Option<PathBuf>,
    output_cursor: Option<String>,
    output_sha256: Option<String>,
    notes: Vec<NotableEntry>,
    #[serde(default)]
    diagnostics: Option<ExecDiagnostics>,
    #[serde(default)]
    continuation: Option<crate::iface::LinesContinuation>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "TrackOutcome")]
struct TrackOutcomeDef {
    changes: Vec<Change>,
    tracking: Tracking,
    watch: Vec<PathBuf>,
    preimage: Preimage,
    preimage_entries: u64,
    preimage_bytes: u64,
    notes: Vec<NotableEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RunResult")]
pub(crate) struct RunResultDef {
    pid: u32,
    started: WallTime,
    #[serde(with = "ProcessEndDef")]
    end: ProcessEnd,
    #[serde(with = "RenderedOutputDef")]
    output: RenderedOutput,
    #[serde(with = "TrackOutcomeDef")]
    track: TrackOutcome,
    op: Option<u64>,
    job: Option<String>,
}

/// Environment and standard input of a run, kept apart from the job record
/// in a file readable by the account of the engine only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredInput {
    /// Complete environment of the program.
    pub(crate) env: Vec<(OsString, OsString)>,
    /// Standard input.
    #[serde(with = "StdinSourceDef")]
    pub(crate) stdin: StdinSource,
}

/// A run request in serialized form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredRequest(#[serde(with = "RunRequestDef")] pub(crate) RunRequest);

/// A finished run in serialized form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredResult(#[serde(with = "RunResultDef")] pub(crate) RunResult);
