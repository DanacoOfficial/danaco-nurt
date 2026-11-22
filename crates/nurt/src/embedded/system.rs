// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Clock and host of the operating system for an embedded engine.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

const UNKNOWN: &str = "unknown";
const DEFAULT_HOST_NAME: &str = "localhost";
const DEFAULT_TIMEZONE: &str = "UTC";
const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";
const OS_RELEASE_FILE: &str = "/proc/sys/kernel/osrelease";
const HOST_NAME_FILE: &str = "/proc/sys/kernel/hostname";

/// Clock backed by [`SystemTime`] and [`Instant`].
///
/// The monotonic reading counts from the creation of the clock. Time spent
/// suspended is counted only where [`Instant`] counts it.
#[derive(Debug)]
pub(super) struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub(super) fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => i128::try_from(after.as_nanos()).unwrap_or(i128::MAX),
            Err(before) => -i128::try_from(before.duration().as_nanos()).unwrap_or(i128::MAX),
        };
        WallTime::from_unix_nanos(nanos)
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
        match cancel.wait_timeout(duration) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

/// Host read from the process and the environment variables.
#[derive(Debug)]
pub(super) struct SystemHost {
    info: HostInfo,
}

impl SystemHost {
    pub(super) fn detect() -> Self {
        let temp = std::env::temp_dir();
        let home = var("HOME")
            .or_else(|| var("USERPROFILE"))
            .map_or_else(|| temp.clone(), PathBuf::from);
        let (os, path_style) = if cfg!(windows) {
            (Os::Windows, PathStyle::Windows)
        } else if cfg!(target_os = "macos") {
            (Os::Macos, PathStyle::Posix)
        } else {
            (Os::Linux, PathStyle::Posix)
        };
        let cpus = std::thread::available_parallelism()
            .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX));
        Self {
            info: HostInfo {
                os,
                os_version: read_trimmed(OS_RELEASE_FILE).unwrap_or_else(|| UNKNOWN.to_owned()),
                architecture: std::env::consts::ARCH.to_owned(),
                hostname: read_trimmed(HOST_NAME_FILE)
                    .or_else(|| var("COMPUTERNAME"))
                    .or_else(|| var("HOSTNAME"))
                    .unwrap_or_else(|| DEFAULT_HOST_NAME.to_owned()),
                user: var("USER")
                    .or_else(|| var("USERNAME"))
                    .or_else(|| var("LOGNAME"))
                    .unwrap_or_else(|| UNKNOWN.to_owned()),
                home,
                temp,
                path_style,
                timezone: var("TZ").unwrap_or_else(|| DEFAULT_TIMEZONE.to_owned()),
                locale: var("LC_ALL").or_else(|| var("LANG")),
                cpus,
            },
        }
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

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn read_trimmed(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
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
    fn wall_time_is_after_the_unix_epoch() {
        assert!(SystemClock::new().now() > WallTime::UNIX_EPOCH);
    }

    #[test]
    fn sleep_ends_early_when_cancelled() {
        let clock = SystemClock::new();
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Requested);
        let result = clock.sleep(Duration::from_secs(60), &cancel);
        assert_eq!(result, Err(CancelReason::Requested));
    }

    #[test]
    fn sleep_completes_without_cancellation() {
        let clock = SystemClock::new();
        let result = clock.sleep(Duration::from_millis(1), &CancelToken::new());
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn host_reports_processors_and_paths() {
        let host = SystemHost::detect();
        let info = host.info();
        assert!(info.cpus >= 1);
        assert!(info.temp.is_absolute());
        assert!(!info.architecture.is_empty());
    }

    #[test]
    fn host_reads_the_environment_of_the_process() {
        let host = SystemHost::detect();
        let (name, value) = host
            .env_vars()
            .into_iter()
            .next()
            .expect("the test process has an environment");
        assert_eq!(host.env_var(&name.to_string_lossy()), Some(value));
    }
}
