// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Doubles for the tests of the administration stages: a scripted process
//! host, a host of a chosen system and a transaction that records markers.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetIndex, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

use super::System;
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec, Services,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelToken, ExecCtx, StageData, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime(0)
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime(0),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }
    fn sleep(
        &self,
        _duration: Duration,
        _cancel: &CancelToken,
    ) -> std::result::Result<(), nurt_contracts::stage::CancelReason> {
        Ok(())
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

/// Reply of the scripted host (the last one registered wins) to the programs whose command line starts with
/// `prefix`.
struct Reply {
    prefix: String,
    code: i32,
    stdout: String,
    stderr: String,
}

/// Process host that answers by command line and records every call.
#[derive(Default)]
pub(crate) struct FakeProcess {
    replies: Mutex<Vec<Reply>>,
    lines: Mutex<Vec<String>>,
    stdin: Mutex<Vec<String>>,
    missing: Mutex<Vec<String>>,
}

impl FakeProcess {
    /// Host without scripted replies: every program succeeds with no output.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Reply with `stdout` and code 0 to command lines that start with `prefix`.
    pub(crate) fn reply(&self, prefix: &str, stdout: &str) -> &Self {
        self.reply_with(prefix, 0, stdout, "")
    }

    /// Reply with a failure to command lines that start with `prefix`.
    pub(crate) fn fail(&self, prefix: &str, code: i32, stderr: &str) -> &Self {
        self.reply_with(prefix, code, "", stderr)
    }

    fn reply_with(&self, prefix: &str, code: i32, stdout: &str, stderr: &str) -> &Self {
        self.replies.lock().expect("replies").push(Reply {
            prefix: prefix.to_owned(),
            code,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        });
        self
    }

    /// The program does not exist.
    pub(crate) fn without(&self, program: &str) -> &Self {
        self.missing
            .lock()
            .expect("missing")
            .push(program.to_owned());
        self
    }

    /// Command lines run so far, in order.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("lines").clone()
    }

    /// Texts written to standard input so far, in order.
    pub(crate) fn stdin(&self) -> Vec<String> {
        self.stdin.lock().expect("stdin").clone()
    }
}

impl ProcessHost for FakeProcess {
    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        let line = std::iter::once(spec.program.as_str())
            .chain(spec.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        if self
            .missing
            .lock()
            .expect("missing")
            .contains(&spec.program)
        {
            return Err(Error::new(ErrorCode::ProgramNotFound, "program not found"));
        }
        self.lines.lock().expect("lines").push(line.clone());
        if let nurt_contracts::services::ProgramInput::Bytes(bytes) = &spec.stdin {
            self.stdin
                .lock()
                .expect("stdin")
                .push(String::from_utf8_lossy(bytes).into_owned());
        }
        let replies = self.replies.lock().expect("replies");
        let reply = replies
            .iter()
            .rev()
            .find(|reply| line.starts_with(&reply.prefix));
        Ok(match reply {
            Some(reply) => ProgramOutcome {
                exit_code: Some(reply.code),
                stdout: reply.stdout.clone().into_bytes(),
                stderr: reply.stderr.clone().into_bytes(),
                ..ProgramOutcome::default()
            },
            None => ProgramOutcome {
                exit_code: Some(0),
                ..ProgramOutcome::default()
            },
        })
    }

    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        if self
            .missing
            .lock()
            .expect("missing")
            .iter()
            .any(|name| name == program)
        {
            return Err(Error::new(ErrorCode::ProgramNotFound, "program not found"));
        }
        Ok(PathBuf::from(program))
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(ErrorCode::Spawn, "not available in tests"))
    }
}

/// Execution context of a host of system `os` over `process`.
pub(crate) fn ctx(system: System, process: &Arc<FakeProcess>) -> ExecCtx {
    let os = match system {
        System::Linux => Os::Linux,
        System::Macos => Os::Macos,
        System::Windows => Os::Windows,
    };
    let host = TestHost(HostInfo {
        os,
        os_version: "test".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "host".to_owned(),
        user: "user".to_owned(),
        home: PathBuf::from(if os == Os::Windows {
            "C:\\Users\\user"
        } else {
            "/home/user"
        }),
        temp: PathBuf::from("/tmp"),
        path_style: if os == Os::Windows {
            PathStyle::Windows
        } else {
            PathStyle::Posix
        },
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    });
    let process: Arc<dyn ProcessHost> = process.clone();
    ExecCtx::builder(Arc::new(TestClock), Arc::new(host))
        .services(Arc::new(Services::new().with_process_host(process)))
        .build()
}

/// Input of a source stage.
pub(crate) fn input() -> StageInput<'static> {
    StageInput {
        data: StageData::None,
        header: None,
        order: None,
        render: None,
    }
}

/// Arguments of `command` with the given values, all as given by the user.
pub(crate) fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

/// Text value.
pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Enumeration value.
pub(crate) fn word(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

/// List of texts.
pub(crate) fn texts(values: &[&str]) -> Value {
    Value::List(values.iter().map(|value| text(value)).collect())
}

/// Records of a stage output.
pub(crate) fn records(output: StageOutput<'_>) -> Vec<Record> {
    match output.data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>>>().expect("records"),
        _ => panic!("expected records"),
    }
}

/// Transaction that records the markers of the stage.
#[derive(Default)]
pub(crate) struct RecordingTx {
    /// Markers received.
    pub(crate) markers: Vec<IrreversibleEntry>,
}

fn unavailable<T>() -> Result<T> {
    Err(Error::new(
        ErrorCode::StageFailed,
        "not used by administration stages",
    ))
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        None
    }
    fn preserve(&mut self, _targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        unavailable()
    }
    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        unavailable()
    }
    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        unavailable()
    }
    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        unavailable()
    }
    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.markers.push(marker);
    }
    fn record_exempt(&mut self, _write: ExemptWrite) {}
    fn commit(self: Box<Self>) -> Result<OpHandle> {
        unavailable()
    }
    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        unavailable()
    }
}
