// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-contracts::env
// Responsibility:   Clock and host interfaces.
// Dependencies:     schemars, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Clock and host interfaces.
//!
//! Stage implementations never read the system clock, the process
//! environment or facts about the host directly. They use the [`Clock`] and
//! [`Host`] carried by the execution context
//! ([`ExecCtx`](crate::stage::ExecCtx)). Production implementations read the
//! operating system; tests use virtual implementations that make time,
//! environment variables and host facts deterministic.
//!
//! # Clock sources
//!
//! | Reading | Linux | macOS | Windows |
//! |---|---|---|---|
//! | wall time | `CLOCK_REALTIME` | `clock_gettime(CLOCK_REALTIME)` | `GetSystemTimePreciseAsFileTime` |
//! | monotonic, including suspend | `CLOCK_BOOTTIME` | `mach_continuous_time` | `QueryInterruptTimePrecise` |
//! | monotonic, running time only | `CLOCK_MONOTONIC` | `mach_absolute_time` | `QueryUnbiasedInterruptTimePrecise` |
//!
//! [`Clock::monotonic`] is the suspend-inclusive reading. Timeouts, deadlines
//! and waiting use it; wall time is used only for values shown to the user and
//! for records.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, Serializer};

use crate::stage::{CancelReason, CancelToken};

const NANOS_PER_SEC: i128 = 1_000_000_000;

/// Wall-clock instant: nanoseconds since the Unix epoch, UTC.
///
/// ```
/// use nurt_contracts::env::WallTime;
/// use std::time::Duration;
///
/// let earlier = WallTime::from_unix_nanos(1_000);
/// let later = WallTime::from_unix_nanos(3_500);
/// assert_eq!(later.saturating_since(earlier), Duration::from_nanos(2_500));
/// assert_eq!(earlier.saturating_since(later), Duration::ZERO);
/// ```
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct WallTime(pub i128);

impl WallTime {
    /// The Unix epoch.
    pub const UNIX_EPOCH: Self = Self(0);

    /// Creates an instant from nanoseconds since the Unix epoch.
    #[must_use]
    pub const fn from_unix_nanos(nanos: i128) -> Self {
        Self(nanos)
    }

    /// Nanoseconds since the Unix epoch.
    #[must_use]
    pub const fn unix_nanos(self) -> i128 {
        self.0
    }

    /// Saturating difference `self - earlier`; zero when `earlier` is later.
    #[must_use]
    pub fn saturating_since(self, earlier: Self) -> Duration {
        let delta = self.0.saturating_sub(earlier.0);
        if delta <= 0 {
            return Duration::ZERO;
        }
        let secs = delta / NANOS_PER_SEC;
        let nanos = delta % NANOS_PER_SEC;
        match (u64::try_from(secs), u32::try_from(nanos)) {
            (Ok(secs), Ok(nanos)) => Duration::new(secs, nanos),
            _ => Duration::MAX,
        }
    }

    /// The instant `duration` after `self`, saturating at the largest
    /// representable instant.
    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        let nanos = i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX);
        Self(self.0.saturating_add(nanos))
    }
}

/// Identifier of the current boot of the computer.
///
/// Linux: `/proc/sys/kernel/random/boot_id`; macOS: `kern.bootsessionuuid`;
/// Windows: boot count combined with boot time. Monotonic readings are
/// comparable only within one boot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct BootId(pub String);

/// Readings of all clocks taken at one moment.
///
/// Used by the journal to evaluate the age of entries across suspend,
/// hibernation, reboots and wall-clock changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClockStamp {
    /// Wall time.
    pub wall: WallTime,
    /// Boot identifier.
    pub boot: BootId,
    /// Monotonic time since boot, including time spent suspended.
    pub since_boot_with_suspend: Duration,
    /// Monotonic time since boot, running time only.
    pub since_boot_active: Duration,
}

/// Source of time.
///
/// Implementations must guarantee that [`monotonic`](Clock::monotonic) never
/// decreases. A virtual clock used in tests may move wall time forwards and
/// backwards, simulate suspend and reboot, and complete [`sleep`](Clock::sleep)
/// immediately by advancing its own time.
pub trait Clock: Send + Sync {
    /// Current wall time.
    fn now(&self) -> WallTime;

    /// Suspend-inclusive monotonic time since an arbitrary origin fixed for
    /// the lifetime of the clock.
    fn monotonic(&self) -> Duration;

    /// Readings of all clocks at one moment.
    fn stamp(&self) -> ClockStamp;

    /// Blocks for `duration` or until `cancel` is cancelled, whichever is first.
    ///
    /// # Errors
    ///
    /// Fails with the cancellation reason when the token was cancelled before
    /// or during the wait.
    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason>;
}

/// Operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Os {
    /// The Linux operating system.
    Linux,
    /// The Apple macOS operating system.
    Macos,
    /// The Microsoft Windows operating system.
    Windows,
}

/// Native notation of file system paths on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PathStyle {
    /// Paths start at `/` and use `/` as the separator.
    Posix,
    /// Paths start with a drive or a UNC prefix and use `\` as the native
    /// separator; the language also accepts `/`.
    Windows,
}

/// Serializes a path as text, replacing invalid Unicode, instead of failing
/// the serialization of the whole structure.
fn serialize_lossy<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

/// Stable facts about the host, fixed for the lifetime of an engine.
///
/// Values that change while the program runs (free memory, uptime, load) are
/// not part of this structure; commands that report them read the operating
/// system through their own interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HostInfo {
    /// Operating system family.
    pub os: Os,
    /// Operating system version, for example `"6.8.0"` or `"10.0.26100"`.
    pub os_version: String,
    /// Processor architecture, for example `"x86_64"` or `"aarch64"`.
    pub architecture: String,
    /// Host name.
    pub hostname: String,
    /// Name of the account running the engine.
    pub user: String,
    /// Absolute home directory of that account, for example `/home/user` or
    /// `C:/Users/user`. A name that is not valid Unicode is serialized
    /// lossily.
    #[serde(serialize_with = "serialize_lossy")]
    pub home: PathBuf,
    /// Absolute directory for temporary files; serialized like `home`.
    #[serde(serialize_with = "serialize_lossy")]
    pub temp: PathBuf,
    /// Native path notation.
    pub path_style: PathStyle,
    /// Local time rules as an IANA tz database name, for example
    /// `"Europe/Warsaw"`; `"UTC"` when the host does not define them.
    pub timezone: String,
    /// Locale identifier, for example `"en_US.UTF-8"`, when the host defines
    /// one.
    pub locale: Option<String>,
    /// Number of logical processors available to the engine.
    pub cpus: u32,
}

/// Facts about the host and the environment of the engine process.
///
/// Stage implementations read environment variables only through this
/// interface, never through `std::env`. An embedding application can
/// therefore give each execution its own environment, and tests substitute
/// it.
pub trait Host: Send + Sync {
    /// Stable facts about the host.
    fn info(&self) -> &HostInfo;

    /// Value of one environment variable of the engine process.
    ///
    /// Names are compared as the host compares them: case-sensitively on
    /// Linux and macOS, case-insensitively on Windows.
    fn env_var(&self, name: &str) -> Option<OsString>;

    /// All environment variables of the engine process, in the order the
    /// host reports them.
    fn env_vars(&self) -> Vec<(OsString, OsString)>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturating_since_clamps_negative_and_huge_differences() {
        let early = WallTime::from_unix_nanos(i128::MIN);
        let late = WallTime::from_unix_nanos(i128::MAX);
        assert_eq!(late.saturating_since(early), Duration::MAX);
        assert_eq!(early.saturating_since(late), Duration::ZERO);
        let later = WallTime::from_unix_nanos(1_999_999_999);
        assert_eq!(
            later.saturating_since(WallTime::UNIX_EPOCH),
            Duration::new(1, 999_999_999)
        );
        assert_eq!(WallTime::UNIX_EPOCH.saturating_since(later), Duration::ZERO);
    }

    #[test]
    fn saturating_add_stops_at_the_largest_instant() {
        let near = WallTime::from_unix_nanos(i128::MAX - 1);
        let moved = near.saturating_add(Duration::from_secs(1));
        assert_eq!(moved.unix_nanos(), i128::MAX);
        let half = WallTime::UNIX_EPOCH.saturating_add(Duration::from_millis(1500));
        assert_eq!(half.unix_nanos(), 1_500_000_000);
        let longest = WallTime::UNIX_EPOCH.saturating_add(Duration::MAX);
        assert_eq!(
            longest.unix_nanos(),
            i128::from(u64::MAX) * NANOS_PER_SEC + 999_999_999
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
