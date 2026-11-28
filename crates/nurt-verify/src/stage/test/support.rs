// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::stage::test::support
// Responsibility:   Execution context and files for the tests of this crate.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution context and files for the tests of this crate.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx};

struct FixedClock;

impl Clock for FixedClock {
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

    fn sleep(&self, _duration: Duration, _cancel: &CancelToken) -> Result<(), CancelReason> {
        Ok(())
    }
}

struct EmptyHost(HostInfo);

impl Host for EmptyHost {
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

/// Context whose state directory is `state`.
pub(crate) fn ctx(state: &Path) -> ExecCtx {
    let info = HostInfo {
        os: Os::Linux,
        os_version: "test".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "test".to_owned(),
        user: "test".to_owned(),
        home: PathBuf::from("/home/test"),
        temp: state.to_path_buf(),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    };
    ExecCtx::builder(Arc::new(FixedClock), Arc::new(EmptyHost(info)))
        .state_dir(state.to_path_buf())
        .build()
}

/// Writes a file, creating its directories.
pub(crate) fn write(root: &Path, relative: &str, content: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_default();
    }
    std::fs::write(&path, content).unwrap_or_default();
    path
}

/// Content of a file of the recorded outputs.
pub(crate) fn corpus(relative: &str) -> String {
    let path = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/data/test")
    .join(relative);
    std::fs::read_to_string(path).unwrap_or_default()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
