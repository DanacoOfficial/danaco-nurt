// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixed clock of a scenario, shared by the `git` program and the Nurt
//! command.

use std::time::Duration;

use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::stage::{CancelReason, CancelToken};

const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_DAY: i64 = 86_400;
const MINUTES_PER_HOUR: i32 = 60;
const MAX_OFFSET_MINUTES: i32 = 14 * MINUTES_PER_HOUR;
const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// Instant and UTC offset of a scenario clock, written in RFC 3339 form
/// (`2026-01-01T12:00:00+01:00`).
///
/// The offset must be a whole number of hours, because the Nurt side receives
/// it as an IANA time zone of the `Etc` area.
///
/// ```
/// use nurt_git_difftest::time::ScenarioClock;
///
/// let clock = ScenarioClock::parse("2026-01-01T12:00:00+01:00")?;
/// assert_eq!(clock.git_date(), "@1767265200 +0100");
/// assert_eq!(clock.timezone(), "Etc/GMT-1");
/// # Ok::<(), String>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioClock {
    unix_seconds: i64,
    offset_minutes: i32,
}

impl ScenarioClock {
    /// Clock used when a scenario does not set one.
    pub const DEFAULT: &'static str = "2026-01-01T12:00:00+00:00";

    /// Parses an RFC 3339 date and time with seconds and a `Z` or `±hh:mm` offset.
    ///
    /// # Errors
    ///
    /// Fails with a message naming the expected form when the text is not such a
    /// date, when a field is out of range, or when the offset is not a whole
    /// number of hours.
    pub fn parse(text: &str) -> Result<Self, String> {
        let invalid =
            || format!("clock `{text}` is not of the form 2026-01-01T12:00:00+01:00 or ...Z");
        let bytes = text.as_bytes();
        if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' {
            return Err(invalid());
        }
        if !matches!(bytes[10], b'T' | b't') || bytes[16] != b':' {
            return Err(invalid());
        }
        let field = |range: std::ops::Range<usize>| -> Result<i64, String> {
            let part = text.get(range).ok_or_else(invalid)?;
            if !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            part.parse::<i64>().map_err(|_| invalid())
        };
        let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
        let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return Err(format!("clock `{text}` names a day that does not exist"));
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(format!("clock `{text}` has a time of day out of range"));
        }
        let offset_minutes = parse_offset(&text[19..]).ok_or_else(invalid)?;
        if offset_minutes % MINUTES_PER_HOUR != 0 || offset_minutes.abs() > MAX_OFFSET_MINUTES {
            return Err(format!(
                "clock `{text}` has an offset that is not a whole number of hours up to 14"
            ));
        }
        let local = days_from_civil(year, month, day) * SECONDS_PER_DAY
            + hour * 3_600
            + minute * SECONDS_PER_MINUTE
            + second;
        Ok(Self {
            unix_seconds: local - i64::from(offset_minutes) * SECONDS_PER_MINUTE,
            offset_minutes,
        })
    }

    /// Seconds since the Unix epoch.
    #[must_use]
    pub fn unix_seconds(&self) -> i64 {
        self.unix_seconds
    }

    /// Offset from UTC in minutes.
    #[must_use]
    pub fn offset_minutes(&self) -> i32 {
        self.offset_minutes
    }

    /// The same offset, `seconds` later.
    #[must_use]
    pub fn advanced(&self, seconds: i64) -> Self {
        Self {
            unix_seconds: self.unix_seconds + seconds,
            offset_minutes: self.offset_minutes,
        }
    }

    /// Value of `GIT_AUTHOR_DATE` and `GIT_COMMITTER_DATE` in the internal
    /// form of `git` (`@<seconds> ±hhmm`).
    #[must_use]
    pub fn git_date(&self) -> String {
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let minutes = self.offset_minutes.abs();
        format!(
            "@{} {sign}{:02}{:02}",
            self.unix_seconds,
            minutes / MINUTES_PER_HOUR,
            minutes % MINUTES_PER_HOUR
        )
    }

    /// IANA time zone with the offset of the clock: `UTC` or `Etc/GMT∓h`
    /// (the `Etc` area inverts the sign).
    #[must_use]
    pub fn timezone(&self) -> String {
        let hours = self.offset_minutes / MINUTES_PER_HOUR;
        match hours {
            0 => "UTC".to_owned(),
            east if east > 0 => format!("Etc/GMT-{east}"),
            west => format!("Etc/GMT+{}", -west),
        }
    }

    /// Wall time of the clock.
    #[must_use]
    pub fn wall_time(&self) -> WallTime {
        WallTime::from_unix_nanos(i128::from(self.unix_seconds) * NANOSECONDS_PER_SECOND)
    }
}

impl Default for ScenarioClock {
    fn default() -> Self {
        Self {
            unix_seconds: 1_767_268_800,
            offset_minutes: 0,
        }
    }
}

fn parse_offset(text: &str) -> Option<i32> {
    if text == "Z" || text == "z" {
        return Some(0);
    }
    let bytes = text.as_bytes();
    if bytes.len() != 6 || bytes[3] != b':' {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = |part: &str| -> Option<i32> {
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let hours = digits(text.get(1..3)?)?;
    let minutes = digits(text.get(4..6)?)?;
    (minutes < MINUTES_PER_HOUR).then_some(sign * (hours * MINUTES_PER_HOUR + minutes))
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date, after the algorithm
/// `days_from_civil` by Howard Hinnant.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Clock that always shows the scenario instant.
///
/// Monotonic time does not advance and [`Clock::sleep`] returns at once, so
/// every reading inside one Nurt command is identical.
#[derive(Debug, Clone)]
pub struct FixedClock {
    wall: WallTime,
}

impl FixedClock {
    /// Creates a clock fixed at the instant of `clock`.
    #[must_use]
    pub fn new(clock: ScenarioClock) -> Self {
        Self {
            wall: clock.wall_time(),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> WallTime {
        self.wall
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.wall,
            boot: BootId("difftest".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(&self, _duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.reason().map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_and_offsets_to_the_same_instant() {
        let utc = ScenarioClock::parse("2026-01-01T11:00:00Z").unwrap();
        let east = ScenarioClock::parse("2026-01-01T12:00:00+01:00").unwrap();
        assert_eq!(utc.unix_seconds(), east.unix_seconds());
        assert_eq!(east.offset_minutes(), 60);
        assert_eq!(utc.git_date(), "@1767265200 +0000");
    }

    #[test]
    fn formats_negative_offsets_for_git_and_iana() {
        let west = ScenarioClock::parse("2025-06-30T23:59:59-05:00").unwrap();
        assert_eq!(west.git_date(), "@1751345999 -0500");
        assert_eq!(west.timezone(), "Etc/GMT+5");
    }

    #[test]
    fn default_matches_default_text() {
        assert_eq!(
            ScenarioClock::parse(ScenarioClock::DEFAULT).unwrap(),
            ScenarioClock::default()
        );
    }

    #[test]
    fn rejects_malformed_dates() {
        for text in [
            "2026-01-01",
            "2026-13-01T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T12:00:00+01:30",
            "2026-01-01T12:00:00+0100",
            "2026-01-01 12:00:00Z",
            "2026-0a-01T12:00:00Z",
        ] {
            assert!(ScenarioClock::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn accepts_leap_days() {
        let clock = ScenarioClock::parse("2024-02-29T00:00:00Z").unwrap();
        assert_eq!(clock.unix_seconds(), 1_709_164_800);
    }

    #[test]
    fn advancing_keeps_the_offset() {
        let clock = ScenarioClock::parse("2026-01-01T12:00:00+02:00").unwrap();
        let later = clock.advanced(60);
        assert_eq!(later.unix_seconds() - clock.unix_seconds(), 60);
        assert_eq!(later.offset_minutes(), 120);
    }

    #[test]
    fn fixed_clock_reports_the_scenario_instant() {
        let clock = FixedClock::new(ScenarioClock::default());
        assert_eq!(
            clock.now(),
            WallTime::from_unix_nanos(1_767_268_800 * NANOSECONDS_PER_SECOND)
        );
        assert_eq!(clock.stamp().wall, clock.now());
        assert!(
            clock
                .sleep(Duration::from_secs(5), &CancelToken::new())
                .is_ok()
        );
    }
}
