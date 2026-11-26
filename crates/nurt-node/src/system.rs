// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Clock and host of an engine opened by the addon.
//!
//! Both are built from the standard library and the process environment only.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";
const KERNEL_RELEASE_FILE: &str = "/proc/sys/kernel/osrelease";
const HOSTNAME_FILE: &str = "/proc/sys/kernel/hostname";
const TIMEZONE_FILE: &str = "/etc/timezone";
const DEFAULT_TIMEZONE: &str = "UTC";
const UNKNOWN: &str = "unknown";

/// Clock of the operating system.
///
/// The monotonic readings count from the creation of the clock; the time
/// since boot is reported as the same value.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
    boot: BootId,
}

impl SystemClock {
    /// Creates a clock. The boot identifier is the one of the operating
    /// system where it is available, otherwise a value that is unique to the
    /// process.
    #[must_use]
    pub fn new() -> Self {
        let boot = read_trimmed(BOOT_ID_FILE)
            .unwrap_or_else(|| format!("process-{}", wall_nanos().unsigned_abs()));
        Self {
            origin: Instant::now(),
            boot: BootId(boot),
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
        WallTime::from_unix_nanos(wall_nanos())
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        let since_boot = uptime().unwrap_or_else(|| self.monotonic());
        ClockStamp {
            wall: self.now(),
            boot: self.boot.clone(),
            since_boot_with_suspend: since_boot,
            since_boot_active: since_boot,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        match cancel.wait_timeout(duration) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

fn wall_nanos() -> i128 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i128::try_from(elapsed.as_nanos()).unwrap_or(i128::MAX),
        Err(before) => -i128::try_from(before.duration().as_nanos()).unwrap_or(i128::MAX),
    }
}

/// Host described by the process environment.
#[derive(Debug)]
pub struct SystemHost {
    info: HostInfo,
}

impl SystemHost {
    /// Reads the description of the host. Values that the standard library
    /// cannot provide are taken from the environment variables conventional
    /// for the operating system.
    #[must_use]
    pub fn new() -> Self {
        let os = if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        };
        let windows = os == Os::Windows;
        let info = HostInfo {
            os,
            os_version: read_trimmed(KERNEL_RELEASE_FILE).unwrap_or_else(|| UNKNOWN.to_owned()),
            architecture: std::env::consts::ARCH.to_owned(),
            hostname: first_variable(&["HOSTNAME", "COMPUTERNAME"])
                .or_else(|| read_trimmed(HOSTNAME_FILE))
                .unwrap_or_else(|| UNKNOWN.to_owned()),
            user: first_variable(&["USER", "USERNAME", "LOGNAME"])
                .unwrap_or_else(|| UNKNOWN.to_owned()),
            home: first_variable(if windows { &["USERPROFILE"] } else { &["HOME"] })
                .map(PathBuf::from)
                .unwrap_or_default(),
            temp: std::env::temp_dir(),
            path_style: if windows {
                PathStyle::Windows
            } else {
                PathStyle::Posix
            },
            timezone: first_variable(&["TZ"])
                .or_else(|| read_trimmed(TIMEZONE_FILE))
                .unwrap_or_else(|| DEFAULT_TIMEZONE.to_owned()),
            locale: first_variable(&["LC_ALL", "LC_MESSAGES", "LANG"]),
            cpus: thread::available_parallelism()
                .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX)),
        };
        Self { info }
    }
}

impl Default for SystemHost {
    fn default() -> Self {
        Self::new()
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

/// Time since boot of Linux, suspended time included; `None` where the file
/// does not exist. A reading that counts from the creation of the clock would
/// make the ages of journal entries made by other processes of the same boot
/// wrong.
fn uptime() -> Option<Duration> {
    parse_uptime(&read_trimmed("/proc/uptime")?)
}

fn parse_uptime(text: &str) -> Option<Duration> {
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

fn read_trimmed(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn first_variable(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::{Clock, Host};
    use nurt_contracts::stage::{CancelReason, CancelToken};

    use super::{SystemClock, SystemHost, first_variable, read_trimmed, wall_nanos};

    #[test]
    fn unset_variables_and_missing_files_give_nothing() {
        assert_eq!(first_variable(&[]), None);
        assert_eq!(
            first_variable(&["NURT_NODE_UNSET_ONE", "NURT_NODE_UNSET_TWO"]),
            None
        );
        assert_eq!(read_trimmed("/nurt-node-missing/boot_id"), None);
        assert_eq!(read_trimmed("/"), None);
    }

    #[test]
    fn wall_time_is_after_the_epoch_and_does_not_go_back_between_readings() {
        let first = wall_nanos();
        let second = wall_nanos();
        assert!(first > 0);
        assert!(second >= first || first - second < 1_000_000_000);
    }

    #[test]
    fn a_stamp_reports_one_elapsed_time_for_both_monotonic_readings() {
        let clock = SystemClock::default();
        let stamp = clock.stamp();
        assert_eq!(stamp.since_boot_with_suspend, stamp.since_boot_active);
        assert_eq!(stamp.boot, clock.stamp().boot);
        assert!(clock.stamp().since_boot_active >= stamp.since_boot_active);
    }

    #[test]
    fn the_host_default_equals_a_new_host_and_has_an_absolute_temp_directory() {
        let host = SystemHost::default();
        assert_eq!(
            host.info().architecture,
            SystemHost::new().info().architecture
        );
        assert!(host.info().temp.is_absolute());
        assert!(!host.info().hostname.is_empty());
        assert!(!host.info().os_version.is_empty());
    }

    #[test]
    fn monotonic_readings_do_not_decrease() {
        let clock = SystemClock::new();
        let first = clock.monotonic();
        let second = clock.monotonic();
        assert!(second >= first);
    }

    #[test]
    fn stamp_carries_a_boot_identifier() {
        let stamp = SystemClock::new().stamp();
        assert!(!stamp.boot.0.is_empty());
        assert!(stamp.wall.unix_nanos() > 0);
    }

    #[test]
    fn sleep_returns_after_the_duration() {
        let clock = SystemClock::new();
        let result = clock.sleep(Duration::from_millis(5), &CancelToken::new());
        assert!(result.is_ok());
    }

    #[test]
    fn sleep_stops_on_a_cancelled_token() {
        let clock = SystemClock::new();
        let token = CancelToken::new();
        token.cancel(CancelReason::Requested);
        let result = clock.sleep(Duration::from_secs(60), &token);
        assert_eq!(result, Err(CancelReason::Requested));
    }

    #[test]
    fn host_reports_the_platform_of_the_build() {
        let host = SystemHost::new();
        assert_eq!(host.info().architecture, std::env::consts::ARCH);
        assert!(host.info().cpus >= 1);
        assert!(!host.info().timezone.is_empty());
    }

    #[test]
    fn host_reads_the_process_environment() {
        let host = SystemHost::new();
        assert_eq!(host.env_var("NURT_NODE_UNSET_VARIABLE"), None);
        assert_eq!(
            host.env_vars().len(),
            std::env::vars_os().count(),
            "the environment is read without filtering"
        );
    }
}
