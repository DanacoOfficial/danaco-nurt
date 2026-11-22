// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Job commands through the public interface of the crate.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, InputKinds, StageInput,
};
use nurt_contracts::value::{RecordKind, Value};

struct TestClock {
    origin: Instant,
}

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(1_800_000_000_000_000_000)
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

struct TestHost {
    info: HostInfo,
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

fn context(state_dir: Option<&Path>) -> ExecCtx {
    let host = TestHost {
        info: HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "user".to_owned(),
            home: std::env::temp_dir(),
            temp: std::env::temp_dir(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        },
    };
    let builder = ExecCtx::builder(
        Arc::new(TestClock {
            origin: Instant::now(),
        }),
        Arc::new(host),
    );
    match state_dir {
        Some(dir) => builder.state_dir(dir.to_path_buf()).build(),
        None => builder.build(),
    }
}

#[test]
fn registers_the_job_commands() {
    let stages = nurt_proc::stages();
    let find = |command: &str| {
        stages
            .iter()
            .find(|stage| stage.descriptor().command == command)
            .unwrap_or_else(|| panic!("no stage `{command}`"))
    };
    let jobs = find("jobs").descriptor();
    assert!(jobs.source);
    assert_eq!(jobs.input, InputKinds::None);
    for command in ["stop", "suspend", "resume", "retry", "remove"] {
        let stage = find(command);
        assert_eq!(
            stage.descriptor().input,
            InputKinds::Only(vec![RecordKind::Job])
        );
        assert!(stage.as_effect().is_some());
    }
}

#[test]
fn lists_no_jobs_in_a_new_state_directory() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(Some(dir.path()));
    let stages = nurt_proc::stages();
    let jobs = stages
        .iter()
        .find(|stage| stage.descriptor().command == "jobs")
        .unwrap();
    let args = BoundArgs::new("jobs");
    let output = jobs
        .as_read()
        .unwrap()
        .execute(&args, StageInput::none(), &ctx)
        .unwrap();
    assert_eq!(StageInput::from(output).into_records().unwrap().count(), 0);
    let unknown =
        BoundArgs::new("jobs").with("id", ArgValue::Value(Value::Text("j3".to_owned())), false);
    let error = jobs
        .as_read()
        .unwrap()
        .execute(&unknown, StageInput::none(), &ctx)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownJob);
}

#[test]
fn supervisor_needs_a_state_directory() {
    let error = nurt_proc::run_supervisor(&context(None)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Config);
}
