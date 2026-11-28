// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-service/tests/support/clock.rs
// Responsibility:   Clock whose time moves only when the test or a long sleep moves it.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Clock whose time moves only when the test or a long sleep moves it.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

/// Wall time at the virtual origin: 2026-11-18T11:06:40Z.
pub(crate) const ORIGIN_NANOS: i128 = 1_795_000_000_000_000_000;
const POLL: Duration = Duration::from_millis(2);

/// Virtual clock.
///
/// A sleep of at least `auto_from` moves the time forward at once, which
/// makes restart delays of the supervisor instantaneous. A shorter sleep,
/// such as the heartbeat period, waits until the test calls
/// [`advance`](Self::advance) or the sleep is cancelled.
pub(crate) struct VirtualClock {
    elapsed: Mutex<Duration>,
    auto_from: Duration,
}

impl VirtualClock {
    pub(crate) fn new(auto_from: Duration) -> Self {
        Self {
            elapsed: Mutex::new(Duration::ZERO),
            auto_from,
        }
    }

    /// Moves the time forward.
    pub(crate) fn advance(&self, by: Duration) {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }

    /// Time since the origin.
    pub(crate) fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wall time at the origin.
    pub(crate) fn origin() -> WallTime {
        WallTime::from_unix_nanos(ORIGIN_NANOS)
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        Self::origin().saturating_add(self.elapsed())
    }

    fn monotonic(&self) -> Duration {
        self.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        let elapsed = self.elapsed();
        ClockStamp {
            wall: self.now(),
            boot: BootId(String::new()),
            since_boot_with_suspend: elapsed,
            since_boot_active: elapsed,
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        if duration >= self.auto_from {
            self.advance(duration);
            return Ok(());
        }
        let target = self.elapsed() + duration;
        while self.elapsed() < target {
            if let Some(reason) = cancel.wait_timeout(POLL) {
                return Err(reason);
            }
        }
        Ok(())
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
