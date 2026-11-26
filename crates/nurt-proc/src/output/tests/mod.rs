// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of output capture with spool files in temporary directories.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx, OutputStream};
use tempfile::TempDir;

use crate::iface::{OutputStore, OutputView, SpoolFactory};

mod budget;
mod corpus;
mod reads;
mod spool_files;
mod streams;
mod views;

/// Wall time of the virtual clock at the start of a test.
const START_NANOS: i128 = 1_700_000_000_000_000_000;

pub(super) struct VirtualClock {
    wall: Mutex<WallTime>,
    monotonic: Mutex<Duration>,
}

impl VirtualClock {
    fn new() -> Self {
        Self {
            wall: Mutex::new(WallTime::from_unix_nanos(START_NANOS)),
            monotonic: Mutex::new(Duration::ZERO),
        }
    }

    pub(super) fn advance(&self, by: Duration) {
        *self.monotonic.lock().unwrap() += by;
        let mut wall = self.wall.lock().unwrap();
        *wall = wall.saturating_add(by);
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        *self.wall.lock().unwrap()
    }

    fn monotonic(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }

    fn stamp(&self) -> ClockStamp {
        let monotonic = self.monotonic();
        ClockStamp {
            wall: self.now(),
            boot: BootId("test-boot".to_owned()),
            since_boot_with_suspend: monotonic,
            since_boot_active: monotonic,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        self.advance(duration);
        Ok(())
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

fn host(state: &std::path::Path) -> TestHost {
    TestHost {
        info: HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "user".to_owned(),
            home: state.to_path_buf(),
            temp: state.to_path_buf(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 2,
        },
    }
}

/// Spool factory over a temporary state directory.
pub(super) struct Bench {
    pub(super) dir: TempDir,
    pub(super) clock: Arc<VirtualClock>,
    pub(super) ctx: ExecCtx,
    pub(super) factory: Arc<dyn SpoolFactory>,
}

impl Bench {
    pub(super) fn new() -> Self {
        Self::with_config(Config::default())
    }

    pub(super) fn with_config(config: Config) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(VirtualClock::new());
        let ctx = ExecCtx::builder(clock.clone(), Arc::new(host(dir.path())))
            .config(Arc::new(config))
            .state_dir(dir.path().to_path_buf())
            .build();
        Self {
            dir,
            clock,
            ctx,
            factory: super::spool_factory(),
        }
    }

    pub(super) fn spool_dir(&self) -> PathBuf {
        self.dir.path().join("spool")
    }

    pub(super) fn create(&self, id: &str, merged: bool) -> Arc<dyn OutputStore> {
        self.factory.create(&self.ctx, id, merged).unwrap()
    }

    /// A finished run with the given output.
    pub(super) fn finished(&self, id: &str, stdout: &[u8], stderr: &[u8]) -> Arc<dyn OutputStore> {
        let store = self.create(id, false);
        store.write(OutputStream::Stdout, stdout).unwrap();
        store.write(OutputStream::Stderr, stderr).unwrap();
        store.finish(OutputStream::Stdout).unwrap();
        store.finish(OutputStream::Stderr).unwrap();
        store
    }
}

/// Lines `prefix 1` to `prefix count`, each ended by a line feed.
pub(super) fn numbered(prefix: &str, count: usize) -> String {
    (1..=count).map(|n| format!("{prefix} {n}\n")).collect()
}

pub(super) fn view() -> OutputView {
    OutputView::default()
}

/// Path of the `proc_fixture` example, which `cargo test` builds next to the
/// test binaries.
pub(super) fn fixture_program() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let target = exe
        .parent()
        .and_then(|deps| deps.parent())
        .unwrap()
        .to_path_buf();
    let path = target
        .join("examples")
        .join(format!("proc_fixture{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "{} is missing; run the tests with cargo test, which builds the example",
        path.display()
    );
    path
}
