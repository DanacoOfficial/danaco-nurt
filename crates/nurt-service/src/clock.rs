// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Clock of the operating system for a service run outside an engine.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

#[cfg(target_os = "linux")]
const BOOT_ID_FILE: &str = "/proc/sys/kernel/random/boot_id";

/// Clock that reads the operating system.
///
/// Wall time comes from [`SystemTime`]; monotonic time from [`Instant`],
/// measured from the creation of the clock. The boot identifier is read on
/// Linux and empty elsewhere; the service uses it only for heartbeat records.
#[derive(Debug, Clone)]
pub struct SystemClock {
    origin: Instant,
    boot: BootId,
}

impl SystemClock {
    /// Clock whose monotonic origin is the moment of the call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            boot: BootId(read_boot_id()),
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
            boot: self.boot.clone(),
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

#[cfg(target_os = "linux")]
fn read_boot_id() -> String {
    std::fs::read_to_string(BOOT_ID_FILE)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn read_boot_id() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_returns_cancellation_reason() {
        let clock = SystemClock::new();
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Shutdown);
        assert_eq!(
            clock.sleep(Duration::from_secs(5), &cancel),
            Err(CancelReason::Shutdown)
        );
        assert!(
            clock
                .sleep(Duration::from_millis(1), &CancelToken::new())
                .is_ok()
        );
    }

    #[test]
    fn wall_time_is_after_2020_and_monotonic_grows() {
        let clock = SystemClock::default();
        let year_2020_nanos = 1_577_836_800_i128 * 1_000_000_000;
        assert!(clock.now().unix_nanos() > year_2020_nanos);
        let first = clock.monotonic();
        assert!(clock.stamp().since_boot_active >= first);
    }
}
