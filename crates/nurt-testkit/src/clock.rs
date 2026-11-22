// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Virtual clock driven by the test.
//!
//! [`VirtualClock`] implements [`Clock`] without reading the system clock.
//! Time moves only when the test moves it ([`VirtualClock::advance`],
//! [`VirtualClock::set_wall`], [`VirtualClock::suspend`],
//! [`VirtualClock::reboot`]) or when a stage sleeps: [`Clock::sleep`] returns
//! immediately and advances the clock by the requested duration.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Wall time at which a [`VirtualClock::new`] clock starts:
/// 2026-01-01T00:00:00Z.
pub const DEFAULT_START: WallTime = WallTime::from_unix_nanos(1_767_225_600_000_000_000);

/// Boot identifier of the first boot of a virtual clock.
pub const DEFAULT_BOOT: &str = "boot-1";

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
