// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Clock and host of the process that loaded the library.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Boot identifier of Linux.
const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";
/// Kernel release of Linux.
const OS_RELEASE_FILE: &str = "/proc/sys/kernel/osrelease";
/// Host name of Linux.
const HOSTNAME_FILE: &str = "/proc/sys/kernel/hostname";
/// Value of a fact that the host does not report.
const UNKNOWN: &str = "unknown";
/// Host name used when the host reports none.
const LOCALHOST: &str = "localhost";
/// Time rules used when `TZ` is not set.
const UTC: &str = "UTC";

/// Current wall time of the system.
pub(crate) fn now() -> WallTime {
    let nanos = |duration: Duration| i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX);
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(after) => WallTime::from_unix_nanos(nanos(after)),
        Err(before) => WallTime::from_unix_nanos(-nanos(before.duration())),
    }
}

/// Clock of the system; the monotonic reading starts at zero when the clock
/// is created.
#[derive(Debug)]
pub(crate) struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        now()
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        let elapsed = self.monotonic();
        ClockStamp {
            wall: now(),
            boot: BootId(read_trimmed(BOOT_ID_FILE).unwrap_or_else(|| UNKNOWN.to_owned())),
            since_boot_with_suspend: elapsed,
            since_boot_active: elapsed,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.wait_timeout(duration).map_or(Ok(()), Err)
    }
}

/// Host of the process, with the facts read once when it is created.
#[derive(Debug, Clone)]
pub(crate) struct SystemHost {
    info: HostInfo,
}

impl SystemHost {
    /// Reads the facts about the host; a fact that the host does not report
    /// is `unknown` or empty.
    pub(crate) fn detect() -> Self {
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
            os_version: read_trimmed(OS_RELEASE_FILE).unwrap_or_else(|| UNKNOWN.to_owned()),
            architecture: std::env::consts::ARCH.to_owned(),
            hostname: read_trimmed(HOSTNAME_FILE)
                .or_else(|| var("HOSTNAME"))
                .or_else(|| var("COMPUTERNAME"))
                .unwrap_or_else(|| LOCALHOST.to_owned()),
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
            timezone: var("TZ").unwrap_or_else(|| UTC.to_owned()),
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
    use std::time::Duration;

    use nurt_contracts::env::{Clock, Host};
    use nurt_contracts::stage::{CancelReason, CancelToken};

    use super::{SystemClock, SystemHost, now};

    #[test]
    fn clock_reads_wall_and_monotonic_time() {
        let clock = SystemClock::new();
        let before = now();
        let stamp = clock.stamp();
        assert!(stamp.wall.unix_nanos() >= before.unix_nanos());
        assert!(clock.now().unix_nanos() >= stamp.wall.unix_nanos());
        assert!(clock.monotonic() >= stamp.since_boot_active);
        assert!(!stamp.boot.0.is_empty());
    }

    #[test]
    fn sleep_ends_early_on_cancellation() {
        let clock = SystemClock::new();
        assert_eq!(clock.sleep(Duration::ZERO, &CancelToken::new()), Ok(()));
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Requested);
        assert_eq!(
            clock.sleep(Duration::from_secs(60), &cancel),
            Err(CancelReason::Requested)
        );
    }

    #[test]
    fn host_reports_the_process_environment() {
        let host = SystemHost::detect();
        let info = host.info();
        assert_eq!(info.architecture, std::env::consts::ARCH);
        assert!(info.cpus >= 1);
        assert!(!info.hostname.is_empty());
        assert_eq!(host.env_var("PATH"), std::env::var_os("PATH"));
        assert_eq!(host.env_vars().len(), std::env::vars_os().count());
    }
}
