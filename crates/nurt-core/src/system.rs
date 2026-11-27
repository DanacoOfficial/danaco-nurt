// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::system
// Responsibility:   Clock and host of the running process, read from the operating system.
// Dependencies:     nurt_contracts, nurt_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Clock and host of the running process, read from the operating system.
//!
//! [`EngineConfig::system`](crate::EngineConfig::system) assembles them with an
//! engine.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Longest interval between two checks of the cancellation token while
/// sleeping.
const SLEEP_SLICE: Duration = Duration::from_millis(10);
const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";
const UNKNOWN: &str = "unknown";

/// Whether the output code page of the console of this process is UTF-8. On a
/// system other than Windows the answer is `false`.
#[must_use]
pub fn console_output_is_utf8() -> bool {
    nurt_sys::console::output_is_utf8()
}

/// Clock of the running process.
///
/// The monotonic reading counts from the creation of the clock and runs while
/// the computer is awake.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// Creates a clock whose monotonic reading starts at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => WallTime::from_unix_nanos(nanos(after)),
            Err(before) => WallTime::from_unix_nanos(-nanos(before.duration())),
        }
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        let elapsed = self.monotonic();
        ClockStamp {
            wall: self.now(),
            boot: BootId(read_trimmed(BOOT_ID_FILE).unwrap_or_else(|| UNKNOWN.to_owned())),
            since_boot_with_suspend: elapsed,
            since_boot_active: elapsed,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        let deadline = Instant::now() + duration;
        loop {
            if cancel.is_cancelled() {
                return Err(cancel.reason().unwrap_or(CancelReason::Requested));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(SLEEP_SLICE));
        }
    }
}

fn nanos(duration: Duration) -> i128 {
    i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
}

/// Facts about the host and the environment of the running process.
#[derive(Debug, Clone)]
pub struct SystemHost {
    info: HostInfo,
}

impl SystemHost {
    /// Reads the facts about the host. A fact that the host does not report
    /// is `unknown` or empty.
    #[must_use]
    pub fn detect() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        };
        let info = HostInfo {
            os,
            os_version: read_trimmed("/proc/sys/kernel/osrelease")
                .unwrap_or_else(|| UNKNOWN.to_owned()),
            architecture: std::env::consts::ARCH.to_owned(),
            hostname: read_trimmed("/proc/sys/kernel/hostname")
                .or_else(|| var("HOSTNAME"))
                .or_else(|| var("COMPUTERNAME"))
                .unwrap_or_else(|| "localhost".to_owned()),
            user: var("USER")
                .or_else(|| var("USERNAME"))
                .unwrap_or_else(|| UNKNOWN.to_owned()),
            home: var("HOME")
                .or_else(|| var("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_default(),
            temp: std::env::temp_dir(),
            path_style: if cfg!(windows) {
                PathStyle::Windows
            } else {
                PathStyle::Posix
            },
            timezone: var("TZ").unwrap_or_else(|| "UTC".to_owned()),
            locale: var("LC_ALL").or_else(|| var("LANG")),
            cpus: std::thread::available_parallelism()
                .ok()
                .and_then(|count| u32::try_from(count.get()).ok())
                .unwrap_or(1),
        };
        Self { info }
    }
}

impl Host for SystemHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_time_does_not_decrease() {
        let clock = SystemClock::new();
        let first = clock.monotonic();
        assert!(clock.monotonic() >= first);
    }

    #[test]
    fn wall_time_is_after_the_epoch() {
        assert!(SystemClock::new().now() > WallTime::UNIX_EPOCH);
    }

    #[test]
    fn sleep_ends_early_when_cancelled() {
        let clock = SystemClock::new();
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Requested);
        let result = clock.sleep(Duration::from_secs(30), &cancel);
        assert_eq!(result, Err(CancelReason::Requested));
    }

    #[test]
    fn sleep_completes_without_cancellation() {
        let clock = SystemClock::new();
        let result = clock.sleep(Duration::from_millis(5), &CancelToken::new());
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn host_reports_at_least_one_processor() {
        let host = SystemHost::detect();
        assert!(host.info().cpus >= 1);
        assert!(!host.info().architecture.is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
