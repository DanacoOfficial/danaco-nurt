// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::host::test_support
// Responsibility:   Test doubles of the clock, the host and the process host, shared by the tests of `host`, `environment`, `accounts` and `open_files`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test doubles of the clock, the host and the process host, shared by the
//! tests of `host`, `environment`, `accounts` and `open_files`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Process;
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};

/// Directory of the static fixtures of these tests.
pub(crate) fn fixtures() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d7/u02")
}

/// Clock fixed at one wall time.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FixedClock(pub(crate) WallTime);

impl FixedClock {
    /// Clock at the current system time.
    pub(crate) fn now() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| i128::try_from(elapsed.as_nanos()).unwrap_or_default())
            .unwrap_or_default();
        Self(WallTime::from_unix_nanos(nanos))
    }
}

impl Clock for FixedClock {
    fn now(&self) -> WallTime {
        self.0
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.0,
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(
        &self,
        _duration: Duration,
        cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        cancel.reason().map_or(Ok(()), Err)
    }
}

/// Host with fixed facts and environment.
#[derive(Debug, Clone)]
pub(crate) struct TestHost {
    pub(crate) info: HostInfo,
    pub(crate) vars: Vec<(OsString, OsString)>,
}

impl TestHost {
    /// Host of the operating system of the test, with home and temporary
    /// directories at `home`.
    pub(crate) fn new(home: &Path) -> Self {
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        };
        Self {
            info: HostInfo {
                os,
                os_version: "1.0".to_owned(),
                architecture: "amd64".to_owned(),
                hostname: "build-host".to_owned(),
                user: "builder".to_owned(),
                home: home.to_path_buf(),
                temp: home.join("tmp"),
                path_style: if cfg!(windows) {
                    PathStyle::Windows
                } else {
                    PathStyle::Posix
                },
                timezone: "America/New_York".to_owned(),
                locale: None,
                cpus: 4,
            },
            vars: Vec::new(),
        }
    }

    /// The same host with the environment variable `name` set.
    pub(crate) fn with_var(
        mut self,
        name: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Self {
        self.vars.push((name.into(), value.into()));
        self
    }

    /// The same host reporting another operating system.
    pub(crate) fn with_os(mut self, os: Os) -> Self {
        self.info.os = os;
        self
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        self.vars
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.vars.clone()
    }
}

/// Process host that counts calls and starts nothing.
#[derive(Debug, Default)]
pub(crate) struct CountingProcessHost {
    pub(crate) calls: AtomicUsize,
}

impl CountingProcessHost {
    fn refuse(&self) -> Error {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Error::new(
            ErrorCode::StageFailed,
            "starting programs is not allowed in this test",
        )
    }

    /// Number of calls made.
    pub(crate) fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ProcessHost for CountingProcessHost {
    fn resolve_program(&self, _program: &str, _cwd: &Path) -> Result<PathBuf> {
        Err(self.refuse())
    }

    fn run_program(&self, _spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        Err(self.refuse())
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(self.refuse())
    }
}

/// Execution context with `host` and the fixed current time.
pub(crate) fn context(host: TestHost) -> ExecCtx {
    ExecCtx::builder(Arc::new(FixedClock::now()), Arc::new(host)).build()
}

/// Bound arguments of `command` from name and value pairs.
pub(crate) fn args(command: &str, values: Vec<(&str, Value)>) -> BoundArgs {
    values
        .into_iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(name, ArgValue::Value(value), false)
        })
}

/// Runs the source `stage` and collects its records.
pub(crate) fn run(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let read = stage
        .as_read()
        .ok_or_else(|| Error::new(ErrorCode::StageFailed, "not a read stage"))?;
    let output = read.execute(args, StageInput::none(), ctx)?;
    StageInput::from(output).into_records()?.collect()
}

/// Typed `process` records of `records`.
pub(crate) fn processes(records: Vec<Record>) -> Vec<Process> {
    records
        .into_iter()
        .map(|record| Process::try_from(record).unwrap())
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
