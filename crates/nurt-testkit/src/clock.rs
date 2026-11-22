// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Virtual clock driven by the test.
//!
//! [`VirtualClock`] implements [`Clock`] without reading the system clock.
//! Time moves only when the test moves it ([`VirtualClock::advance`],
//! [`VirtualClock::set_wall`], [`VirtualClock::jump_wall_back`],
//! [`VirtualClock::jump_wall_forward`], [`VirtualClock::set_local`],
//! [`VirtualClock::suspend`], [`VirtualClock::reboot`]) or when a stage
//! sleeps: [`Clock::sleep`] returns immediately and advances the clock by the
//! requested duration.
//!
//! Local time is computed with the rules of a time zone given either by an
//! IANA name (`Europe/Warsaw`, read from the time zone database of the
//! system) or by a POSIX TZ string (`CET-1CEST,M3.5.0,M10.5.0/3`). POSIX
//! strings need no database, so tests that use them behave the same on every
//! host.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Wall time at which a [`VirtualClock::new`] clock starts:
/// 2026-01-01T00:00:00Z.
pub const DEFAULT_START: WallTime = WallTime::from_unix_nanos(1_767_225_600_000_000_000);

/// Boot identifier of the first boot of a virtual clock.
pub const DEFAULT_BOOT: &str = "boot-1";

/// Failure of the local time helpers of [`VirtualClock`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneError {
    /// The rejected time zone or local time.
    pub input: String,
    /// Description of the problem.
    pub message: String,
}

impl fmt::Display for ZoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid local time input `{}`: {}",
            self.input, self.message
        )
    }
}

impl std::error::Error for ZoneError {}

fn zone_error(input: &str, error: &jiff::Error) -> ZoneError {
    ZoneError {
        input: input.to_owned(),
        message: error.to_string(),
    }
}

/// Time zone of an IANA name or, when no such zone is known, of a POSIX TZ
/// string.
fn time_zone(zone: &str) -> Result<TimeZone, ZoneError> {
    TimeZone::get(zone)
        .or_else(|_| TimeZone::posix(zone))
        .map_err(|error| zone_error(zone, &error))
}

#[derive(Debug, Clone)]
struct ClockState {
    wall: WallTime,
    monotonic: Duration,
    boot: u64,
    since_boot_with_suspend: Duration,
    since_boot_active: Duration,
    sleeps: Vec<Duration>,
}

/// Deterministic clock for tests.
///
/// The clock keeps four readings: wall time, the suspend-inclusive monotonic
/// time of [`Clock::monotonic`] (never decreasing for the lifetime of the
/// clock, also across simulated reboots) and the two readings since boot
/// reported by [`Clock::stamp`].
///
/// ```
/// use std::time::Duration;
///
/// use nurt_contracts::env::Clock;
/// use nurt_contracts::stage::CancelToken;
/// use nurt_testkit::clock::VirtualClock;
///
/// let clock = VirtualClock::new();
/// let start = clock.now();
/// clock.advance(Duration::from_secs(5));
/// assert_eq!(clock.now().saturating_since(start), Duration::from_secs(5));
///
/// // Sleeping completes at once and moves the clock.
/// clock.sleep(Duration::from_secs(60), &CancelToken::new()).unwrap();
/// assert_eq!(clock.monotonic(), Duration::from_secs(65));
/// assert_eq!(clock.sleeps(), vec![Duration::from_secs(60)]);
/// ```
#[derive(Debug)]
pub struct VirtualClock {
    state: Mutex<ClockState>,
}

impl VirtualClock {
    /// Clock starting at [`DEFAULT_START`] with monotonic time zero.
    #[must_use]
    pub fn new() -> Self {
        Self::starting_at(DEFAULT_START)
    }

    /// Clock starting at the given wall time with monotonic time zero.
    #[must_use]
    pub fn starting_at(wall: WallTime) -> Self {
        Self {
            state: Mutex::new(ClockState {
                wall,
                monotonic: Duration::ZERO,
                boot: 1,
                since_boot_with_suspend: Duration::ZERO,
                since_boot_active: Duration::ZERO,
                sleeps: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ClockState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves every reading forwards by `duration`, as running time.
    pub fn advance(&self, duration: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(duration);
        state.monotonic = state.monotonic.saturating_add(duration);
        state.since_boot_with_suspend = state.since_boot_with_suspend.saturating_add(duration);
        state.since_boot_active = state.since_boot_active.saturating_add(duration);
    }

    /// Sets wall time, forwards or backwards, without touching the monotonic
    /// readings (a wall-clock adjustment).
    pub fn set_wall(&self, wall: WallTime) {
        self.lock().wall = wall;
    }

    /// Moves wall time backwards by `duration` without touching the
    /// monotonic readings, as a clock correction of the system does.
    pub fn jump_wall_back(&self, duration: Duration) {
        let mut state = self.lock();
        let delta = i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX);
        state.wall = WallTime(state.wall.0.saturating_sub(delta));
    }

    /// Moves wall time forwards by `duration` without touching the
    /// monotonic readings.
    pub fn jump_wall_forward(&self, duration: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(duration);
    }

    /// Sets wall time to the instant of the local time `civil` in the time
    /// zone `tz`.
    ///
    /// The local time is a date and a time of day without offset, as in the
    /// example below. A local time skipped by a transition moves forwards by
    /// the length of
    /// the gap; a repeated local time takes the earlier instant. The
    /// monotonic readings do not change.
    ///
    /// # Errors
    ///
    /// Fails with [`ZoneError`] when `tz` is neither a known IANA name nor a
    /// valid POSIX TZ string, or `civil` is not a local date and time.
    ///
    /// ```
    /// use nurt_testkit::clock::VirtualClock;
    ///
    /// const CENTRAL_EUROPE: &str = "CET-1CEST,M3.5.0,M10.5.0/3";
    /// let clock = VirtualClock::new();
    /// clock.set_local("2026-03-29T02:30", CENTRAL_EUROPE).unwrap();
    /// assert!(clock.local(CENTRAL_EUROPE).unwrap().starts_with("2026-03-29T03:30:00"));
    /// ```
    pub fn set_local(&self, civil: &str, tz: &str) -> Result<(), ZoneError> {
        let tz = time_zone(tz)?;
        let local: DateTime = civil.parse().map_err(|error| zone_error(civil, &error))?;
        let instant = tz
            .to_timestamp(local)
            .map_err(|error| zone_error(civil, &error))?;
        self.set_wall(WallTime(instant.as_nanosecond()));
        Ok(())
    }

    /// Returns the local time of the current wall time in the time zone `tz`.
    ///
    /// The text holds the date, the time of day and the UTC offset in force.
    ///
    /// # Errors
    ///
    /// Fails with [`ZoneError`] for an unknown zone and for a wall time
    /// outside the range of local times.
    pub fn local(&self, tz: &str) -> Result<String, ZoneError> {
        let tz = time_zone(tz)?;
        let wall = self.lock().wall;
        let instant = Timestamp::from_nanosecond(wall.0)
            .map_err(|error| zone_error(&wall.0.to_string(), &error))?;
        let zoned = instant.to_zoned_datetime(tz);
        Ok(format!("{}{}", zoned.datetime(), zoned.offset()))
    }

    /// Simulates a suspend of `duration`: wall time and the suspend-inclusive
    /// readings move, the running-time reading does not.
    pub fn suspend(&self, duration: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(duration);
        state.monotonic = state.monotonic.saturating_add(duration);
        state.since_boot_with_suspend = state.since_boot_with_suspend.saturating_add(duration);
    }

    /// Simulates a reboot that took `downtime`: wall time moves by
    /// `downtime`, the boot identifier changes (`boot-2`, `boot-3`, ...) and
    /// both readings since boot restart at zero. [`Clock::monotonic`] keeps
    /// increasing by `downtime`, because it is defined for the lifetime of
    /// the clock.
    pub fn reboot(&self, downtime: Duration) {
        let mut state = self.lock();
        state.wall = state.wall.saturating_add(downtime);
        state.monotonic = state.monotonic.saturating_add(downtime);
        state.boot = state.boot.saturating_add(1);
        state.since_boot_with_suspend = Duration::ZERO;
        state.since_boot_active = Duration::ZERO;
    }

    /// Durations requested by [`Clock::sleep`] calls that completed, in call
    /// order.
    #[must_use]
    pub fn sleeps(&self) -> Vec<Duration> {
        self.lock().sleeps.clone()
    }

    /// Current boot identifier.
    #[must_use]
    pub fn boot(&self) -> BootId {
        BootId(format!("boot-{}", self.lock().boot))
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
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
        let state = self.lock();
        ClockStamp {
            wall: state.wall,
            boot: BootId(format!("boot-{}", state.boot)),
            since_boot_with_suspend: state.since_boot_with_suspend,
            since_boot_active: state.since_boot_active,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        self.advance(duration);
        self.lock().sleeps.push(duration);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::{Clock, WallTime};
    use nurt_contracts::stage::{CancelReason, CancelToken};

    use super::{DEFAULT_BOOT, DEFAULT_START, VirtualClock};

    /// Central European rules as a POSIX TZ string: standard time UTC+1,
    /// summer time from the last Sunday of March 02:00 to the last Sunday of
    /// October 03:00.
    const CENTRAL_EUROPE: &str = "CET-1CEST,M3.5.0,M10.5.0/3";

    #[test]
    fn starts_at_the_default_instant() {
        let clock = VirtualClock::new();
        assert_eq!(clock.now(), DEFAULT_START);
        assert_eq!(clock.monotonic(), Duration::ZERO);
        assert_eq!(clock.stamp().boot.0, DEFAULT_BOOT);
    }

    #[test]
    fn advance_moves_all_readings() {
        let clock = VirtualClock::new();
        clock.advance(Duration::from_millis(1_500));
        let stamp = clock.stamp();
        assert_eq!(
            stamp.wall.saturating_since(DEFAULT_START),
            Duration::from_millis(1_500)
        );
        assert_eq!(stamp.since_boot_with_suspend, Duration::from_millis(1_500));
        assert_eq!(stamp.since_boot_active, Duration::from_millis(1_500));
        assert_eq!(clock.monotonic(), Duration::from_millis(1_500));
    }

    #[test]
    fn wall_time_may_move_backwards_while_monotonic_does_not() {
        let clock = VirtualClock::new();
        clock.advance(Duration::from_secs(10));
        clock.set_wall(WallTime::UNIX_EPOCH);
        assert_eq!(clock.now(), WallTime::UNIX_EPOCH);
        assert_eq!(clock.monotonic(), Duration::from_secs(10));
    }

    #[test]
    fn wall_jumps_leave_monotonic_time_alone() {
        let clock = VirtualClock::new();
        clock.jump_wall_back(Duration::from_secs(3_600));
        assert_eq!(
            DEFAULT_START.saturating_since(clock.now()),
            Duration::from_secs(3_600)
        );
        clock.jump_wall_forward(Duration::from_secs(7_200));
        assert_eq!(
            clock.now().saturating_since(DEFAULT_START),
            Duration::from_secs(3_600)
        );
        assert_eq!(clock.monotonic(), Duration::ZERO);
    }

    #[test]
    fn local_time_follows_daylight_saving_transitions() {
        let clock = VirtualClock::new();
        clock
            .set_local("2026-03-29T01:59:00", CENTRAL_EUROPE)
            .unwrap();
        assert!(
            clock
                .local(CENTRAL_EUROPE)
                .unwrap()
                .starts_with("2026-03-29T01:59:00")
        );
        clock.advance(Duration::from_secs(120));
        assert!(
            clock
                .local(CENTRAL_EUROPE)
                .unwrap()
                .starts_with("2026-03-29T03:01:00")
        );
        assert_eq!(clock.monotonic(), Duration::from_secs(120));
    }

    #[test]
    fn local_time_rejects_unknown_zones_and_bad_times() {
        let clock = VirtualClock::new();
        assert!(clock.local("not a zone").is_err());
        let error = clock.set_local("yesterday", CENTRAL_EUROPE).unwrap_err();
        assert_eq!(error.input, "yesterday");
        assert!(error.to_string().contains("yesterday"));
    }

    #[test]
    fn suspend_skips_the_running_time_reading() {
        let clock = VirtualClock::new();
        clock.advance(Duration::from_secs(1));
        clock.suspend(Duration::from_secs(100));
        let stamp = clock.stamp();
        assert_eq!(stamp.since_boot_with_suspend, Duration::from_secs(101));
        assert_eq!(stamp.since_boot_active, Duration::from_secs(1));
        assert_eq!(clock.monotonic(), Duration::from_secs(101));
    }

    #[test]
    fn reboot_restarts_readings_since_boot() {
        let clock = VirtualClock::new();
        clock.advance(Duration::from_secs(30));
        clock.reboot(Duration::from_secs(5));
        let stamp = clock.stamp();
        assert_eq!(stamp.boot.0, "boot-2");
        assert_eq!(clock.boot().0, "boot-2");
        assert_eq!(stamp.since_boot_with_suspend, Duration::ZERO);
        assert_eq!(stamp.since_boot_active, Duration::ZERO);
        assert_eq!(clock.monotonic(), Duration::from_secs(35));
        assert_eq!(
            stamp.wall.saturating_since(DEFAULT_START),
            Duration::from_secs(35)
        );
    }

    #[test]
    fn sleep_advances_and_records() {
        let clock = VirtualClock::new();
        let cancel = CancelToken::new();
        assert_eq!(clock.sleep(Duration::from_secs(3), &cancel), Ok(()));
        assert_eq!(clock.sleep(Duration::from_secs(4), &cancel), Ok(()));
        assert_eq!(clock.monotonic(), Duration::from_secs(7));
        assert_eq!(
            clock.sleeps(),
            vec![Duration::from_secs(3), Duration::from_secs(4)]
        );
    }

    #[test]
    fn sleep_fails_when_cancelled() {
        let clock = VirtualClock::new();
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Timeout);
        assert_eq!(
            clock.sleep(Duration::from_secs(3), &cancel),
            Err(CancelReason::Timeout)
        );
        assert_eq!(clock.monotonic(), Duration::ZERO);
        assert!(clock.sleeps().is_empty());
    }
}
