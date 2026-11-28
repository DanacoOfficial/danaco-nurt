// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-net/tests/support/exec_ctx.rs
// Responsibility:   Execution context for tests of `nurt-net` commands: the real monotonic clock, a fixed host description and helpers for arguments and output.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution context for tests of `nurt-net` commands: the real monotonic
//! clock, a fixed host description and helpers for arguments and output.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "each test binary includes this module, uses a subset of its public helpers, and a failed helper fails the test"
)]

use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::Result;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, Event, EventSink, ExecCtx, ReadStage,
    StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};

/// Clock of the test process; waits block for real time.
pub struct SystemClock {
    origin: Instant,
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

/// Host description with fixed values.
pub struct TestHost(HostInfo);

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

/// Events collected by a test.
#[derive(Default)]
pub struct Events(pub Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
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

/// Context with the system clock, the test host and the given token.
pub fn ctx_with(cancel: CancelToken, events: Arc<Events>) -> ExecCtx {
    let clock = Arc::new(SystemClock {
        origin: Instant::now(),
    });
    ExecCtx::builder(clock, host())
        .cancel(cancel)
        .events(events)
        .build()
}

/// Context with the system clock and the test host.
pub fn ctx() -> ExecCtx {
    ctx_with(CancelToken::new(), Arc::new(Events::default()))
}

/// Arguments of `command` with the given values.
pub fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

/// Text value.
pub fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Runs a read stage without input and collects its records.
pub fn run(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let read: &dyn ReadStage = stage.as_read().expect("the command has a read interface");
    let output = read.execute(args, StageInput::none(), ctx)?;
    match output.data {
        StageData::Records(stream) => stream.collect(),
        other => panic!("expected records, got {other:?}"),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
