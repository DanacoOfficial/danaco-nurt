// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::processes::testing
// Responsibility:   Test doubles shared by the tests of `processes`, `ports` and `disks`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test doubles shared by the tests of `processes`, `ports` and `disks`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::Result;
use nurt_contracts::stage::{
    BoundArgs, CancelReason, CancelToken, Demand, ExecCtx, StageFrame, StageImpl, StageInput,
};
use nurt_contracts::value::Record;

type SleepHook = Box<dyn FnMut(Duration) + Send>;

/// Clock whose time moves only when a stage sleeps; a hook runs on every
/// sleep so that a test can change the system tree between two readings.
pub(crate) struct VirtualClock {
    wall: Mutex<i128>,
    monotonic: Mutex<Duration>,
    sleeps: AtomicUsize,
    hook: Mutex<Option<SleepHook>>,
    cancel_on_sleep: Mutex<Option<CancelToken>>,
}

impl VirtualClock {
    /// A clock at the wall time `unix_seconds`.
    pub(crate) fn at(unix_seconds: i64) -> Arc<Self> {
        Arc::new(Self {
            wall: Mutex::new(i128::from(unix_seconds) * 1_000_000_000),
            monotonic: Mutex::new(Duration::from_secs(1)),
            sleeps: AtomicUsize::new(0),
            hook: Mutex::new(None),
            cancel_on_sleep: Mutex::new(None),
        })
    }

    pub(crate) fn on_sleep(&self, hook: impl FnMut(Duration) + Send + 'static) {
        *self.hook.lock().unwrap() = Some(Box::new(hook));
    }

    /// Cancels `token` when a stage starts to sleep.
    pub(crate) fn cancel_on_sleep(&self, token: CancelToken) {
        *self.cancel_on_sleep.lock().unwrap() = Some(token);
    }

    pub(crate) fn sleeps(&self) -> usize {
        self.sleeps.load(Ordering::SeqCst)
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(*self.wall.lock().unwrap())
    }

    fn monotonic(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("boot".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        self.sleeps.fetch_add(1, Ordering::SeqCst);
        if let Some(token) = self.cancel_on_sleep.lock().unwrap().take() {
            token.cancel(CancelReason::Requested);
        }
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        *self.monotonic.lock().unwrap() += duration;
        *self.wall.lock().unwrap() += i128::try_from(duration.as_nanos()).unwrap();
        if let Some(hook) = self.hook.lock().unwrap().as_mut() {
            hook(duration);
        }
        Ok(())
    }
}

/// Clock of the running system, for tests that read the real process table.
pub(crate) struct SystemClock {
    origin: std::time::Instant,
}

impl SystemClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: std::time::Instant::now(),
        })
    }
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        WallTime::from_unix_nanos(i128::try_from(since.as_nanos()).unwrap())
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("boot".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.wait_timeout(duration).map_or(Ok(()), Err)
    }
}

/// Host with fixed facts.
pub(crate) struct TestHost {
    info: HostInfo,
}

impl TestHost {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            info: HostInfo {
                os: Os::Linux,
                os_version: "6.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "host".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from("/home/user"),
                temp: PathBuf::from("/tmp"),
                path_style: PathStyle::Posix,
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 1,
            },
        })
    }
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

/// Execution context with `clock`, the demand of the following stages and
/// the cancellation token `cancel`.
pub(crate) fn context(clock: Arc<dyn Clock>, demand: Demand, cancel: CancelToken) -> ExecCtx {
    ExecCtx::builder(clock, TestHost::new())
        .cancel(cancel)
        .frame(StageFrame {
            demand,
            ..StageFrame::default()
        })
        .build()
}

/// Demand that names only `fields`.
pub(crate) fn only(fields: &[&str]) -> Demand {
    Demand {
        fields: nurt_contracts::stage::FieldDemand::Only(
            fields.iter().map(|field| (*field).to_owned()).collect(),
        ),
        ..Demand::default()
    }
}

/// Runs a source stage and collects its records.
pub(crate) fn run(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let output = stage
        .as_read()
        .expect("read stage")
        .execute(args, StageInput::none(), ctx)?;
    match output.data {
        nurt_contracts::stage::StageData::Records(stream) => stream.collect(),
        other => panic!("unexpected output {other:?}"),
    }
}

/// Writes `content` to `path` under `root`, creating the directories.
#[cfg(unix)]
pub(crate) fn write(root: &std::path::Path, path: &str, content: impl AsRef<[u8]>) {
    let target = root.join(path.trim_start_matches('/'));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, content).unwrap();
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
