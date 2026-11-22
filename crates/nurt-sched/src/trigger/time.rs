// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversions between the instants of the contracts and the calendar types.

use std::time::Duration;

use jiff::Timestamp;
use nurt_contracts::env::WallTime;
use nurt_contracts::value::Instant;

/// Instant of a wall-clock reading.
#[must_use]
pub fn instant_from_wall(wall: WallTime) -> Instant {
    Instant::from_unix_nanos(saturate(wall.unix_nanos()))
}

/// Calendar timestamp of an instant.
pub(crate) fn timestamp(instant: Instant) -> Option<Timestamp> {
    Timestamp::from_nanosecond(i128::from(instant.as_unix_nanos())).ok()
}

/// Instant of a calendar timestamp, saturated to the range of [`Instant`].
pub(crate) fn instant(timestamp: Timestamp) -> Instant {
    Instant::from_unix_nanos(saturate(timestamp.as_nanosecond()))
}

/// `instant + duration`, saturated.
#[must_use]
pub fn add(instant: Instant, duration: Duration) -> Instant {
    let nanos = i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX);
    Instant::from_unix_nanos(instant.as_unix_nanos().saturating_add(nanos))
}

/// Time from `earlier` to `later`; zero when `later` is not later.
#[must_use]
pub fn since(later: Instant, earlier: Instant) -> Duration {
    let nanos = later
        .as_unix_nanos()
        .saturating_sub(earlier.as_unix_nanos());
    u64::try_from(nanos).map_or(Duration::ZERO, Duration::from_nanos)
}

/// Contract duration (milliseconds) of a standard duration, saturated.
pub(crate) fn value_duration(duration: Duration) -> nurt_contracts::value::Duration {
    nurt_contracts::value::Duration::from_millis(
        i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
    )
}

/// Standard duration of a contract duration; negative durations are zero.
pub(crate) fn std_duration(duration: nurt_contracts::value::Duration) -> Duration {
    duration.to_std().unwrap_or(Duration::ZERO)
}

fn saturate(nanos: i128) -> i64 {
    i64::try_from(nanos).unwrap_or(if nanos < 0 { i64::MIN } else { i64::MAX })
}
