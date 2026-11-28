// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::cim::datetime
// Responsibility:   Values of the CIM type `datetime`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Values of the CIM type `datetime`.
//!
//! WMI writes a point in time as `yyyymmddHHMMSS.mmmmmmsUUU`, where
//! `mmmmmm` are microseconds, `s` is `+` or `-` and `UUU` is the offset from
//! UTC in minutes. An interval uses the same length with the layout
//! `ddddddddHHMMSS.mmmmmm:000`. Fields that the provider does not know are
//! filled with `*`; such values are not converted.

use nurt_contracts::value::{Duration, Instant};

/// Length of every `datetime` value.
const LENGTH: usize = 25;
/// Byte index of the `.` before the microseconds.
const DOT: usize = 14;
/// Byte index of the sign of the offset, or `:` for an interval.
const SIGN: usize = 21;
/// Offset field of an interval.
const INTERVAL_SUFFIX: &str = "000";

const MILLIS_PER_SECOND: i64 = 1_000;
const MICROS_PER_MILLI: i64 = 1_000;
const SECONDS_PER_MINUTE: i64 = 60;
const MINUTES_PER_HOUR: i64 = 60;
const HOURS_PER_DAY: i64 = 24;

/// A converted `datetime` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CimTime {
    /// A point in time, converted to UTC.
    Instant(Instant),
    /// An interval.
    Interval(Duration),
}

/// Parses a `datetime` value; `None` when the text does not have the CIM
/// layout or has unknown fields.
pub(crate) fn parse(text: &str) -> Option<CimTime> {
    let bytes = text.as_bytes();
    if bytes.len() != LENGTH || bytes[DOT] != b'.' {
        return None;
    }
    let digits_ok = bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| index == DOT || index == SIGN || byte.is_ascii_digit());
    if !digits_ok {
        return None;
    }
    match bytes[SIGN] {
        b'+' | b'-' => parse_instant(text).map(CimTime::Instant),
        b':' if &text[SIGN + 1..] == INTERVAL_SUFFIX => parse_interval(text).map(CimTime::Interval),
        _ => None,
    }
}

fn number(text: &str, start: usize, end: usize) -> Option<i64> {
    text.get(start..end)?.parse().ok()
}

fn parse_instant(text: &str) -> Option<Instant> {
    let offset_minutes = number(text, SIGN + 1, LENGTH)?;
    let rfc3339 = format!(
        "{}-{}-{}T{}:{}:{}.{}{}{:02}:{:02}",
        &text[0..4],
        &text[4..6],
        &text[6..8],
        &text[8..10],
        &text[10..12],
        &text[12..14],
        &text[DOT + 1..SIGN],
        &text[SIGN..=SIGN],
        offset_minutes / MINUTES_PER_HOUR,
        offset_minutes % MINUTES_PER_HOUR,
    );
    rfc3339.parse().ok()
}

fn parse_interval(text: &str) -> Option<Duration> {
    let days = number(text, 0, 8)?;
    let hours = number(text, 8, 10)?;
    let minutes = number(text, 10, 12)?;
    let seconds = number(text, 12, 14)?;
    let micros = number(text, DOT + 1, SIGN)?;
    let total_hours = days.checked_mul(HOURS_PER_DAY)?.checked_add(hours)?;
    let total_minutes = total_hours
        .checked_mul(MINUTES_PER_HOUR)?
        .checked_add(minutes)?;
    let total_seconds = total_minutes
        .checked_mul(SECONDS_PER_MINUTE)?
        .checked_add(seconds)?;
    let millis = total_seconds
        .checked_mul(MILLIS_PER_SECOND)?
        .checked_add(micros / MICROS_PER_MILLI)?;
    Some(Duration::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(text: &str) -> CimTime {
        CimTime::Instant(text.parse().unwrap())
    }

    #[test]
    fn cim_datetime_parse_offset_and_interval() {
        assert_eq!(
            parse("20261121103000.250000+060"),
            Some(instant("2026-11-21T09:30:00.250Z"))
        );
        assert_eq!(
            parse("20261121103000.000000-330"),
            Some(instant("2026-11-21T16:00:00Z"))
        );
        assert_eq!(
            parse("20260101000000.000001+000"),
            Some(instant("2026-01-01T00:00:00.000001Z"))
        );
        assert_eq!(
            parse("00000001020304.005000:000"),
            Some(CimTime::Interval(Duration::from_millis(
                ((24 + 2) * 60 + 3) * 60 * 1000 + 4 * 1000 + 5
            )))
        );
        assert_eq!(
            parse("00000000000000.000000:000"),
            Some(CimTime::Interval(Duration::ZERO))
        );
    }

    #[test]
    fn cim_datetime_rejects_other_layouts() {
        let rejected = [
            "",
            "2026-11-21T10:00:00Z",
            "20261121103000.250000+06",
            "20261121103000,250000+060",
            "2026112110300*.******+060",
            "00000001020304.005000:001",
            "20261121103000.250000*060",
            "20261321103000.000000+000",
            "20261121103000.000000+0ä",
        ];
        for text in rejected {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
