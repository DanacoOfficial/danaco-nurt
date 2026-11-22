// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Virtual clock: wall time, boot identity and both boot clocks move only when
//! a test moves them.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Wall time at which every virtual clock starts: 2026-01-01T00:00:00Z.
pub(crate) const START: WallTime = WallTime::from_unix_nanos(1_767_225_600 * NANOS_PER_SECOND);

const NANOS_PER_SECOND: i128 = 1_000_000_000;

#[derive(Debug, Clone)]
struct State {
    wall: WallTime,
    boot: u32,
    since_boot_with_suspend: Duration,
    since_boot_active: Duration,
}

/// Clock under full control of a test.
///
/// Running time advances every clock; suspension advances the wall clock and
/// the boot clock that counts suspension; a wall clock step changes the wall
/// clock only; a reboot starts a new boot with both boot clocks at zero.
#[derive(Debug)]
pub(crate) struct VirtualClock {
    state: Mutex<State>,
}

impl VirtualClock {
    /// Creates a clock at [`START`] in the first boot.
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                wall: START,
                boot: 1,
                since_boot_with_suspend: Duration::ZERO,
                since_boot_active: Duration::ZERO,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lets the computer run for `duration`.
    pub(crate) fn advance(&self, duration: Duration) {
        let mut state = self.state();
        state.wall = state.wall.saturating_add(duration);
        state.since_boot_with_suspend += duration;
        state.since_boot_active += duration;
    }

    /// Suspends the computer for `duration`.
    pub(crate) fn suspend(&self, duration: Duration) {
        let mut state = self.state();
        state.wall = state.wall.saturating_add(duration);
        state.since_boot_with_suspend += duration;
    }

    /// Sets the wall clock forward by `duration`, as a time synchronisation
    /// would.
    pub(crate) fn step_wall_forward(&self, duration: Duration) {
        let mut state = self.state();
        state.wall = state.wall.saturating_add(duration);
    }

    /// Sets the wall clock back by `duration`.
    pub(crate) fn step_wall_back(&self, duration: Duration) {
        let mut state = self.state();
        let nanos = i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX);
        state.wall = WallTime::from_unix_nanos(state.wall.unix_nanos().saturating_sub(nanos));
    }

    /// Powers the computer off for `off` and starts a new boot.
    pub(crate) fn reboot(&self, off: Duration) {
        let mut state = self.state();
        state.wall = state.wall.saturating_add(off);
        state.boot += 1;
        state.since_boot_with_suspend = Duration::ZERO;
        state.since_boot_active = Duration::ZERO;
    }

    /// Identity of the current boot.
    pub(crate) fn boot_id(&self) -> BootId {
        BootId(format!("boot-{}", self.state().boot))
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        self.state().wall
    }

    fn monotonic(&self) -> Duration {
        self.state().since_boot_active
    }

    fn stamp(&self) -> ClockStamp {
        let state = self.state().clone();
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
        Ok(())
    }
}
