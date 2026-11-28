// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::clock
// Responsibility:   System and virtual clocks, boot identity and process liveness.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! System and virtual clocks, boot identity and process liveness.
//!
//! [`SystemClock`] reads the clocks of the operating system listed in the
//! contract of [`Clock`]. [`VirtualClock`] is a deterministic clock driven by
//! tests: it moves wall time in both directions, simulates suspend and
//! reboots, and completes [`Clock::sleep`] immediately by advancing itself.

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Nanoseconds in one second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Length of one Windows time unit (`FILETIME`, interrupt time) in nanoseconds.
const NANOS_PER_FILETIME_UNIT: u64 = 100;

/// `FILETIME` value of the Unix epoch (100-nanosecond units since 1601-01-01).
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// Clock of the operating system.
///
/// Readings come from the sources listed in the contract of [`Clock`]. The
/// boot identifier is read once per process and shared by all instances.
///
/// | System | Boot identifier |
/// |---|---|
/// | Linux | `/proc/sys/kernel/random/boot_id` |
/// | macOS | `kern.bootsessionuuid` |
/// | Windows | boot counter of the system (`BootId` in the prefetch parameters of the registry); boot time in seconds when the counter is unreadable |
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::Clock;
/// use nurt_fs::clock::SystemClock;
///
/// let clock = SystemClock::new();
/// let first = clock.stamp();
/// let second = clock.stamp();
/// assert!(second.since_boot_with_suspend >= first.since_boot_with_suspend);
/// assert_eq!(first.boot, second.boot);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl SystemClock {
    /// Creates the system clock.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        sys::wall()
    }

    fn monotonic(&self) -> Duration {
        sys::since_boot_with_suspend()
    }

    fn stamp(&self) -> ClockStamp {
        // The running-time clock is read first so that the suspend-inclusive
        // reading, taken later, is never the smaller one.
        let since_boot_active = sys::since_boot_active();
        ClockStamp {
            since_boot_with_suspend: sys::since_boot_with_suspend(),
            since_boot_active,
            wall: sys::wall(),
            boot: boot_id(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        match cancel.wait_timeout(duration) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

fn boot_id() -> BootId {
    static BOOT: OnceLock<BootId> = OnceLock::new();
    BOOT.get_or_init(|| BootId(sys::read_boot_id())).clone()
}

/// Deterministic clock controlled by tests.
///
/// The clock starts at a given wall time in boot 1 with both monotonic
/// readings at zero. Time moves only through the methods below:
///
/// | Method | Wall time | With suspend | Active | Boot |
/// |---|---|---|---|---|
/// | [`advance`](Self::advance) | `+d` | `+d` | `+d` | same |
/// | [`suspend`](Self::suspend) | `+d` | `+d` | unchanged | same |
/// | [`set_wall`](Self::set_wall) | set | unchanged | unchanged | same |
/// | [`reboot`](Self::reboot) | `+downtime` | zero | zero | next |
///
/// [`Clock::monotonic`] grows by every advance, suspend and downtime and never
/// decreases, also across simulated reboots. [`Clock::sleep`] advances the
/// clock by the requested duration and returns immediately.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use nurt_contracts::env::{Clock, WallTime};
/// use nurt_fs::clock::VirtualClock;
///
/// let clock = VirtualClock::new(WallTime::from_unix_nanos(0));
/// clock.advance(Duration::from_secs(10));
/// clock.suspend(Duration::from_secs(5));
/// let stamp = clock.stamp();
/// assert_eq!(stamp.since_boot_with_suspend, Duration::from_secs(15));
/// assert_eq!(stamp.since_boot_active, Duration::from_secs(10));
///
/// clock.reboot(Duration::from_secs(60));
/// let after = clock.stamp();
/// assert_ne!(after.boot, stamp.boot);
/// assert_eq!(after.since_boot_with_suspend, Duration::ZERO);
/// assert_eq!(clock.monotonic(), Duration::from_secs(75));
/// ```
#[derive(Debug)]
pub struct VirtualClock {
    state: Mutex<VirtualState>,
}

#[derive(Debug, Clone, Copy)]
struct VirtualState {
    wall: WallTime,
    boot: u64,
    with_suspend: Duration,
    active: Duration,
    monotonic: Duration,
}

impl VirtualClock {
    /// Wall time at which [`VirtualClock::default`] starts:
    /// 2024-01-01T00:00:00Z.
    pub const DEFAULT_START: WallTime = WallTime::from_unix_nanos(1_704_067_200_000_000_000);

    /// Creates a clock at `start` in boot 1.
    #[must_use]
    pub fn new(start: WallTime) -> Self {
        Self {
            state: Mutex::new(VirtualState {
                wall: start,
                boot: 1,
                with_suspend: Duration::ZERO,
                active: Duration::ZERO,
                monotonic: Duration::ZERO,
            }),
        }
    }

    /// Lets `duration` of running time pass.
    pub fn advance(&self, duration: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(duration);
        state.with_suspend = state.with_suspend.saturating_add(duration);
        state.active = state.active.saturating_add(duration);
        state.monotonic = state.monotonic.saturating_add(duration);
    }

    /// Simulates a suspend of `duration`: wall time and the suspend-inclusive
    /// clock move, the active clock does not.
    pub fn suspend(&self, duration: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(duration);
        state.with_suspend = state.with_suspend.saturating_add(duration);
        state.monotonic = state.monotonic.saturating_add(duration);
    }

    /// Sets the wall clock, forwards or backwards, without moving the
    /// monotonic clocks.
    pub fn set_wall(&self, wall: WallTime) {
        self.lock().wall = wall;
    }

    /// Simulates a reboot after `downtime` of power-off: the boot identifier
    /// changes and both since-boot readings restart at zero.
    pub fn reboot(&self, downtime: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(downtime);
        state.boot = state.boot.saturating_add(1);
        state.with_suspend = Duration::ZERO;
        state.active = Duration::ZERO;
        state.monotonic = state.monotonic.saturating_add(downtime);
    }

    fn lock(&self) -> MutexGuard<'_, VirtualState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new(Self::DEFAULT_START)
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        self.lock().wall
    }

    fn monotonic(&self) -> Duration {
        self.lock().monotonic
    }

    fn stamp(&self) -> ClockStamp {
        let state = *self.lock();
        ClockStamp {
            wall: state.wall,
            boot: BootId(format!("virtual-{}", state.boot)),
            since_boot_with_suspend: state.with_suspend,
            since_boot_active: state.active,
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

/// Whether a process with the given id exists.
///
/// A process that exists but cannot be inspected by the account counts as
/// alive. Linux and macOS send the null signal (`kill(pid, 0)`): success or
/// `EPERM` means alive, `ESRCH` means gone. Windows opens the process with
/// `PROCESS_QUERY_LIMITED_INFORMATION` and reads its exit code; a process that
/// exited with code 259 (`STILL_ACTIVE`) is indistinguishable from a running
/// one and is reported alive. Id 0 is never a live process.
///
/// # Examples
///
/// ```
/// use nurt_fs::clock::process_alive;
///
/// assert!(process_alive(std::process::id()));
/// assert!(!process_alive(0));
/// ```
#[must_use]
pub fn process_alive(pid: u32) -> bool {
    pid != 0 && sys::process_alive(pid)
}

/// Converts a Windows `FILETIME` value (100-nanosecond units since
/// 1601-01-01 UTC) to wall time.
#[must_use]
pub fn filetime_to_wall(filetime: u64) -> WallTime {
    let units = i128::from(filetime) - i128::from(FILETIME_UNIX_EPOCH);
    WallTime::from_unix_nanos(units * i128::from(NANOS_PER_FILETIME_UNIT))
}

/// Converts wall time to a Windows `FILETIME` value, saturating at the
/// representable range.
#[must_use]
pub fn wall_to_filetime(wall: WallTime) -> u64 {
    let units = wall
        .unix_nanos()
        .div_euclid(i128::from(NANOS_PER_FILETIME_UNIT))
        + i128::from(FILETIME_UNIX_EPOCH);
    u64::try_from(units.max(0)).unwrap_or(u64::MAX)
}

/// Converts a count of 100-nanosecond units (Windows interrupt time) to a
/// duration.
#[must_use]
pub fn filetime_units_to_duration(units: u64) -> Duration {
    let per_second = NANOS_PER_SECOND / NANOS_PER_FILETIME_UNIT;
    let secs = units / per_second;
    let rest = units % per_second;
    let nanos = u32::try_from(rest * NANOS_PER_FILETIME_UNIT).unwrap_or(0);
    Duration::new(secs, nanos)
}

/// Converts a `timespec` reading to a duration, mapping negative readings to
/// zero.
#[cfg(unix)]
fn timespec_to_duration(secs: i64, nanos: i64) -> Duration {
    let secs = u64::try_from(secs).unwrap_or(0);
    let nanos = u32::try_from(nanos).unwrap_or(0);
    Duration::new(secs, 0).saturating_add(Duration::from_nanos(u64::from(nanos)))
}

/// Converts a `timespec` reading of the real-time clock to wall time.
#[cfg(unix)]
fn timespec_to_wall(secs: i64, nanos: i64) -> WallTime {
    WallTime::from_unix_nanos(i128::from(secs) * i128::from(NANOS_PER_SECOND) + i128::from(nanos))
}

#[cfg(unix)]
fn unix_process_alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        Err(errno) => errno == rustix::io::Errno::PERM,
    }
}

// `time_t` and `c_long` are 32 bits wide on some Unix targets.
#[allow(
    clippy::useless_conversion,
    reason = "the field types differ between targets"
)]
#[cfg(unix)]
fn read_clock(id: libc::clockid_t) -> (i64, i64) {
    let mut spec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `spec` is a valid, writable `timespec`; `clock_gettime` writes
    // only into it and the clock ids used here are supported by the system.
    let status = unsafe { libc::clock_gettime(id, &raw mut spec) };
    if status != 0 {
        return (0, 0);
    }
    (i64::from(spec.tv_sec), i64::from(spec.tv_nsec))
}

#[cfg(target_os = "linux")]
mod sys {
    use std::time::Duration;

    use nurt_contracts::env::WallTime;

    const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";

    pub(super) fn wall() -> WallTime {
        let (secs, nanos) = super::read_clock(libc::CLOCK_REALTIME);
        super::timespec_to_wall(secs, nanos)
    }

    pub(super) fn since_boot_with_suspend() -> Duration {
        let (secs, nanos) = super::read_clock(libc::CLOCK_BOOTTIME);
        super::timespec_to_duration(secs, nanos)
    }

    pub(super) fn since_boot_active() -> Duration {
        let (secs, nanos) = super::read_clock(libc::CLOCK_MONOTONIC);
        super::timespec_to_duration(secs, nanos)
    }

    pub(super) fn read_boot_id() -> String {
        std::fs::read_to_string(BOOT_ID_FILE)
            .map(|text| text.trim().to_owned())
            .unwrap_or_default()
    }

    pub(super) fn process_alive(pid: u32) -> bool {
        super::unix_process_alive(pid)
    }
}

/// `mach_continuous_time` and `mach_absolute_time` are read through
/// `CLOCK_MONOTONIC_RAW` and `CLOCK_UPTIME_RAW`, which the system defines as
/// the same clocks already converted to nanoseconds.
#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::CStr;
    use std::time::Duration;

    use nurt_contracts::env::WallTime;

    const BOOT_SESSION: &CStr = c"kern.bootsessionuuid";
    const UUID_TEXT_CAPACITY: usize = 64;

    pub(super) fn wall() -> WallTime {
        let (secs, nanos) = super::read_clock(libc::CLOCK_REALTIME);
        super::timespec_to_wall(secs, nanos)
    }

    pub(super) fn since_boot_with_suspend() -> Duration {
        let (secs, nanos) = super::read_clock(libc::CLOCK_MONOTONIC_RAW);
        super::timespec_to_duration(secs, nanos)
    }

    pub(super) fn since_boot_active() -> Duration {
        let (secs, nanos) = super::read_clock(libc::CLOCK_UPTIME_RAW);
        super::timespec_to_duration(secs, nanos)
    }

    pub(super) fn read_boot_id() -> String {
        let mut buffer = [0_u8; UUID_TEXT_CAPACITY];
        let mut length = buffer.len();
        // SAFETY: the name is a NUL-terminated string; `buffer` is writable for
        // `length` bytes and `length` is updated by the call to the number of
        // bytes written; no new value is set.
        let status = unsafe {
            libc::sysctlbyname(
                BOOT_SESSION.as_ptr(),
                buffer.as_mut_ptr().cast(),
                &raw mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return String::new();
        }
        let bytes = buffer.get(..length).unwrap_or_default();
        let text = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
        String::from_utf8_lossy(text).into_owned()
    }

    pub(super) fn process_alive(pid: u32) -> bool {
        super::unix_process_alive(pid)
    }
}

#[cfg(windows)]
mod sys {
    use std::time::Duration;

    use nurt_contracts::env::WallTime;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_SUCCESS, FILETIME, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::System::WindowsProgramming::{
        QueryInterruptTimePrecise, QueryUnbiasedInterruptTimePrecise,
    };

    const PREFETCH_KEY: &str =
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters";
    const BOOT_VALUE: &str = "BootId";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn filetime_now() -> u64 {
        let mut time = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        // SAFETY: `time` is a valid, writable `FILETIME`.
        unsafe { GetSystemTimePreciseAsFileTime(&raw mut time) };
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    }

    pub(super) fn wall() -> WallTime {
        super::filetime_to_wall(filetime_now())
    }

    pub(super) fn since_boot_with_suspend() -> Duration {
        let mut units = 0_u64;
        // SAFETY: `units` is a valid, writable `u64`.
        unsafe { QueryInterruptTimePrecise(&raw mut units) };
        super::filetime_units_to_duration(units)
    }

    pub(super) fn since_boot_active() -> Duration {
        let mut units = 0_u64;
        // SAFETY: `units` is a valid, writable `u64`.
        unsafe { QueryUnbiasedInterruptTimePrecise(&raw mut units) };
        super::filetime_units_to_duration(units)
    }

    pub(super) fn read_boot_id() -> String {
        let key = wide(PREFETCH_KEY);
        let value = wide(BOOT_VALUE);
        let mut data = 0_u32;
        let mut size = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(0);
        // SAFETY: key and value names are NUL-terminated UTF-16 strings that
        // outlive the call; `data` is writable for `size` bytes and the type
        // filter restricts the value to a 32-bit number.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut data).cast(),
                &raw mut size,
            )
        };
        if status == ERROR_SUCCESS {
            return format!("boot-{data}");
        }
        let boot = super::filetime_to_wall(filetime_now()).unix_nanos()
            - i128::try_from(since_boot_with_suspend().as_nanos()).unwrap_or(0);
        format!(
            "boot-time-{}",
            boot.div_euclid(i128::from(super::NANOS_PER_SECOND))
        )
    }

    pub(super) fn process_alive(pid: u32) -> bool {
        // SAFETY: plain call with an access mask and a process id; the
        // returned handle is checked before use.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // SAFETY: reads the error code of the calling thread.
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        }
        let mut code = 0_u32;
        // SAFETY: `handle` is a valid process handle opened above and `code`
        // is a valid, writable `u32`.
        let queried = unsafe { GetExitCodeProcess(handle, &raw mut code) };
        // SAFETY: `handle` was opened above and is closed exactly once.
        unsafe { CloseHandle(handle) };
        queried == 0 || i64::from(code) == i64::from(STILL_ACTIVE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_readings_are_monotonic_and_boot_is_stable() {
        let clock = SystemClock::new();
        let first = clock.stamp();
        let second = clock.stamp();
        assert!(second.since_boot_with_suspend >= first.since_boot_with_suspend);
        assert!(second.since_boot_active >= first.since_boot_active);
        assert!(first.since_boot_with_suspend >= first.since_boot_active);
        assert!(first.since_boot_with_suspend > Duration::ZERO);
        assert!(first.wall > WallTime::UNIX_EPOCH);
        assert_eq!(first.boot, second.boot);
        assert!(!first.boot.0.is_empty());
        assert!(clock.monotonic() >= second.since_boot_with_suspend);
        assert!(clock.now() >= first.wall);
    }

    #[test]
    fn system_clock_sleep_waits_and_reports_cancellation() {
        let clock = SystemClock::new();
        let token = CancelToken::new();
        let before = clock.monotonic();
        assert_eq!(clock.sleep(Duration::from_millis(5), &token), Ok(()));
        assert!(clock.monotonic() >= before + Duration::from_millis(5));
        token.cancel(CancelReason::Requested);
        assert_eq!(
            clock.sleep(Duration::from_secs(10), &token),
            Err(CancelReason::Requested)
        );
    }

    #[test]
    fn virtual_clock_wall_jumps_leave_monotonic_clocks_unchanged() {
        let clock = VirtualClock::default();
        clock.advance(Duration::from_secs(30));
        let before = clock.stamp();
        clock.set_wall(WallTime::from_unix_nanos(0));
        let back = clock.stamp();
        assert_eq!(back.wall, WallTime::UNIX_EPOCH);
        assert_eq!(back.since_boot_with_suspend, before.since_boot_with_suspend);
        assert_eq!(back.since_boot_active, before.since_boot_active);
        let ahead = before.wall.saturating_add(Duration::from_hours(24));
        clock.set_wall(ahead);
        assert_eq!(clock.now(), ahead);
        assert_eq!(clock.monotonic(), Duration::from_secs(30));
    }

    #[test]
    fn virtual_clock_suspend_stops_active_time_only() {
        let clock = VirtualClock::default();
        clock.advance(Duration::from_secs(1));
        clock.suspend(Duration::from_hours(2));
        let stamp = clock.stamp();
        assert_eq!(stamp.since_boot_active, Duration::from_secs(1));
        assert_eq!(stamp.since_boot_with_suspend, Duration::from_secs(7_201));
        assert_eq!(
            stamp.wall,
            VirtualClock::DEFAULT_START.saturating_add(Duration::from_secs(7_201))
        );
    }

    #[test]
    fn virtual_clock_reboot_changes_boot_and_resets_since_boot_clocks() {
        let clock = VirtualClock::default();
        clock.advance(Duration::from_secs(100));
        let before = clock.stamp();
        clock.reboot(Duration::from_mins(40));
        let after = clock.stamp();
        assert_ne!(after.boot, before.boot);
        assert_eq!(after.since_boot_with_suspend, Duration::ZERO);
        assert_eq!(after.since_boot_active, Duration::ZERO);
        assert_eq!(
            after.wall.saturating_since(before.wall),
            Duration::from_mins(40)
        );
        assert_eq!(clock.monotonic(), Duration::from_secs(2_500));
    }

    #[test]
    fn virtual_clock_sleep_advances_unless_cancelled() {
        let clock = VirtualClock::default();
        let token = CancelToken::new();
        assert_eq!(clock.sleep(Duration::from_secs(3), &token), Ok(()));
        assert_eq!(clock.monotonic(), Duration::from_secs(3));
        token.cancel(CancelReason::Timeout);
        assert_eq!(
            clock.sleep(Duration::from_secs(3), &token),
            Err(CancelReason::Timeout)
        );
        assert_eq!(clock.monotonic(), Duration::from_secs(3));
    }

    #[test]
    fn filetime_conversions_round_trip_and_saturate() {
        assert_eq!(filetime_to_wall(FILETIME_UNIX_EPOCH), WallTime::UNIX_EPOCH);
        assert_eq!(wall_to_filetime(WallTime::UNIX_EPOCH), FILETIME_UNIX_EPOCH);
        let wall = WallTime::from_unix_nanos(1_704_067_200_123_456_700);
        assert_eq!(filetime_to_wall(wall_to_filetime(wall)), wall);
        assert_eq!(
            filetime_to_wall(0).unix_nanos(),
            -11_644_473_600_000_000_000
        );
        assert_eq!(wall_to_filetime(WallTime::from_unix_nanos(i128::MIN)), 0);
        assert_eq!(
            wall_to_filetime(WallTime::from_unix_nanos(i128::MAX)),
            u64::MAX
        );
        assert_eq!(
            filetime_units_to_duration(12_345_678),
            Duration::new(1, 234_567_800)
        );
        assert_eq!(
            filetime_units_to_duration(u64::MAX),
            Duration::new(1_844_674_407_370, 955_161_500)
        );
    }

    #[cfg(unix)]
    #[test]
    fn timespec_conversions_clamp_negative_readings() {
        assert_eq!(timespec_to_duration(-1, 5), Duration::from_nanos(5));
        assert_eq!(timespec_to_duration(2, -1), Duration::from_secs(2));
        assert_eq!(timespec_to_wall(-1, 0).unix_nanos(), -1_000_000_000);
    }

    #[test]
    fn process_alive_distinguishes_running_and_exited_processes() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(0));
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit"][..]
            } else {
                &[][..]
            })
            .spawn()
            .expect("spawn child");
        let pid = child.id();
        child.wait().expect("wait for child");
        assert!(!process_alive(pid));
    }

    #[cfg(unix)]
    #[test]
    fn process_alive_rejects_ids_outside_the_pid_range() {
        assert!(!process_alive(u32::MAX));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
