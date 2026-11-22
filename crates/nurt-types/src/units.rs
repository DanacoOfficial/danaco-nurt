// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text forms of sizes, durations, instants and ranges.
//!
//! Every unit has a canonical form, which parses back to the same value, and
//! a short display form used in reports, which may round.

use jiff::civil;
use jiff::tz::TimeZone;
use nurt_contracts::value::{Duration, Instant, Range, Size, ValueError};

const KB: u64 = 1_000;
const KIB: u64 = 1_024;

/// Size units with their number of bytes, as accepted by [`parse_size`].
const SIZE_UNITS: [(&str, u64); 9] = [
    ("b", 1),
    ("kb", KB),
    ("mb", KB.pow(2)),
    ("gb", KB.pow(3)),
    ("tb", KB.pow(4)),
    ("kib", KIB),
    ("mib", KIB.pow(2)),
    ("gib", KIB.pow(3)),
    ("tib", KIB.pow(4)),
];

/// Size units other than bytes, largest first, for the canonical form.
const SIZE_UNITS_BY_FACTOR: [(&str, u64); 8] = [
    ("tib", KIB.pow(4)),
    ("tb", KB.pow(4)),
    ("gib", KIB.pow(3)),
    ("gb", KB.pow(3)),
    ("mib", KIB.pow(2)),
    ("mb", KB.pow(2)),
    ("kib", KIB),
    ("kb", KB),
];

/// Binary units of the short form, smallest first.
const DISPLAY_SIZE_UNITS: [&str; 6] = ["kib", "mib", "gib", "tib", "pib", "eib"];

/// Tenths of a unit from which the short form moves to the next unit.
const TENTHS_PER_NEXT_UNIT: u128 = 10 * KIB as u128;

/// Fraction digits accepted in a size; more cannot change the byte count
/// of any size that fits 64 bits.
const MAX_FRACTION_DIGITS: u32 = 18;

const MS_PER_SECOND: u64 = 1_000;
const MS_PER_MINUTE: u64 = 60 * MS_PER_SECOND;
const MS_PER_HOUR: u64 = 60 * MS_PER_MINUTE;
const MS_PER_DAY: u64 = 24 * MS_PER_HOUR;
const MS_PER_WEEK: u64 = 7 * MS_PER_DAY;

/// Duration units with their milliseconds, as accepted by [`parse_duration`].
const DURATION_UNITS: [(&str, u64); 6] = [
    ("w", MS_PER_WEEK),
    ("d", MS_PER_DAY),
    ("h", MS_PER_HOUR),
    ("m", MS_PER_MINUTE),
    ("s", MS_PER_SECOND),
    ("ms", 1),
];

/// Units of the canonical and short forms of a duration, largest first.
const CANONICAL_DURATION_UNITS: [(&str, u64); 5] = [
    ("d", MS_PER_DAY),
    ("h", MS_PER_HOUR),
    ("m", MS_PER_MINUTE),
    ("s", MS_PER_SECOND),
    ("ms", 1),
];

const TENTHS_PER_MINUTE: u64 = 600;
const MS_PER_TENTH: u64 = 100;

/// Length of the date part `YYYY-MM-DD` of an instant.
const DATE_LENGTH: usize = 10;
const NANOS_PER_SECOND: i64 = 1_000_000_000;

const SIZE_FORMAT: &str = "expected a number and a unit, for example 10mb or 1.5gib";
const DURATION_FORMAT: &str = "expected numbers with units, for example 250ms, 30s or 1h30m";
const INSTANT_FORMAT: &str =
    "expected RFC 3339 date and time such as 2026-11-21T10:00:00Z, or a date such as 2026-11-21";
const RANGE_FORMAT: &str = "expected an integer or a range such as 1..5, 40.. or ..9";

/// Parses a size: a number with an optional fraction and a unit.
///
/// Units are `b`, the decimal `kb mb gb tb` (powers of 1000) and the binary
/// `kib mib gib tib` (powers of 1024), in any letter case; digits may be
/// grouped with `_`. A fraction is rounded to the nearest byte.
///
/// # Errors
///
/// Returns a [`ValueError`] when the text is not a size or the size does
/// not fit 64 bits.
///
/// # Examples
///
/// ```rust
/// use nurt_types::parse_size;
///
/// assert_eq!(parse_size("1.5kib")?.bytes(), 1_536);
/// assert_eq!(parse_size("10MB")?.bytes(), 10_000_000);
/// assert!(parse_size("10").is_err());
/// # Ok::<(), nurt_contracts::value::ValueError>(())
/// ```
pub fn parse_size(text: &str) -> Result<Size, ValueError> {
    let invalid = |reason: &str| ValueError::new("size", text, reason);
    let split = text
        .find(|character: char| character.is_ascii_alphabetic())
        .ok_or_else(|| invalid(SIZE_FORMAT))?;
    let (number, unit) = text.split_at(split);
    let unit = unit.to_ascii_lowercase();
    let factor = SIZE_UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|&(_, factor)| u128::from(factor))
        .ok_or_else(|| invalid("known units are b, kb, mb, gb, tb, kib, mib, gib, tib"))?;
    let (whole, fraction) = match number.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let whole = parse_digits(whole).ok_or_else(|| invalid(SIZE_FORMAT))?;
    let mut bytes = whole.checked_mul(factor);
    if let Some(fraction) = fraction {
        let (value, width) = parse_fraction(fraction).ok_or_else(|| invalid(SIZE_FORMAT))?;
        let scale = 10_u128.pow(width);
        let rounded = (value * factor + scale / 2) / scale;
        bytes = bytes.and_then(|bytes| bytes.checked_add(rounded));
    }
    bytes
        .and_then(|bytes| u64::try_from(bytes).ok())
        .map(Size::from_bytes)
        .ok_or_else(|| invalid("the size exceeds the largest size of 64 bits"))
}

/// Canonical form of a size: the largest unit that divides it exactly.
///
/// The result parses back to the same size with [`parse_size`].
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::Size;
/// use nurt_types::format_size;
///
/// assert_eq!(format_size(Size::from_bytes(1_048_576)), "1mib");
/// assert_eq!(format_size(Size::from_bytes(3_000)), "3kb");
/// assert_eq!(format_size(Size::from_bytes(1_200)), "1200b");
/// ```
#[must_use]
pub fn format_size(size: Size) -> String {
    let bytes = size.bytes();
    SIZE_UNITS_BY_FACTOR
        .iter()
        .find(|(_, factor)| bytes != 0 && bytes.is_multiple_of(*factor))
        .map_or_else(
            || format!("{bytes}b"),
            |(name, factor)| format!("{}{name}", bytes / factor),
        )
}

/// Short form of a size in reports: bytes below one kibibyte, otherwise
/// one fraction digit of the largest binary unit (powers of 1024) that keeps
/// the number below 1024.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::Size;
/// use nurt_types::display_size;
///
/// assert_eq!(display_size(Size::from_bytes(12)), "12b");
/// assert_eq!(display_size(Size::from_bytes(42_189)), "41.2kib");
/// assert_eq!(display_size(Size::from_bytes(8_192)), "8.0kib");
/// ```
#[must_use]
pub fn display_size(size: Size) -> String {
    let bytes = u128::from(size.bytes());
    let mut factor = u128::from(KIB);
    if bytes < factor {
        return format!("{bytes}b");
    }
    let mut chosen = ("kib", 0);
    for (index, unit) in DISPLAY_SIZE_UNITS.into_iter().enumerate() {
        let tenths = (bytes * 10 + factor / 2) / factor;
        chosen = (unit, tenths);
        if tenths < TENTHS_PER_NEXT_UNIT || index + 1 == DISPLAY_SIZE_UNITS.len() {
            break;
        }
        factor *= u128::from(KIB);
    }
    let (unit, tenths) = chosen;
    format!("{}.{}{unit}", tenths / 10, tenths % 10)
}

/// Parses a duration: one or more integers, each followed by a unit.
///
/// Units are `ms`, `s`, `m`, `h`, `d` and `w`; each may appear once, in any
/// order; a leading `-` makes the duration negative.
///
/// # Errors
///
/// Returns a [`ValueError`] when the text is not a duration or it does not
/// fit 64 bits of milliseconds.
///
/// # Examples
///
/// ```rust
/// use nurt_types::parse_duration;
///
/// assert_eq!(parse_duration("1h30m")?.as_millis(), 5_400_000);
/// assert_eq!(parse_duration("250ms")?.as_millis(), 250);
/// assert!(parse_duration("1h1h").is_err());
/// # Ok::<(), nurt_contracts::value::ValueError>(())
/// ```
pub fn parse_duration(text: &str) -> Result<Duration, ValueError> {
    let invalid = |reason: &str| ValueError::new("duration", text, reason);
    let (negative, mut rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if rest.is_empty() {
        return Err(invalid(DURATION_FORMAT));
    }
    let mut seen = [false; DURATION_UNITS.len()];
    let mut total: u128 = 0;
    while !rest.is_empty() {
        let (count, unit, next) =
            split_duration_part(rest).ok_or_else(|| invalid(DURATION_FORMAT))?;
        let index = DURATION_UNITS
            .iter()
            .position(|(name, _)| *name == unit)
            .ok_or_else(|| invalid("known units are ms, s, m, h, d, w"))?;
        if seen.get(index).copied().unwrap_or(true) {
            return Err(invalid("a unit is repeated"));
        }
        if let Some(flag) = seen.get_mut(index) {
            *flag = true;
        }
        let factor = DURATION_UNITS.get(index).map_or(1, |&(_, factor)| factor);
        total = count
            .checked_mul(u128::from(factor))
            .and_then(|millis| total.checked_add(millis))
            .ok_or_else(|| invalid("the duration exceeds the range of 64-bit milliseconds"))?;
        rest = next;
    }
    let magnitude = i128::try_from(total).unwrap_or(i128::MAX);
    let millis = if negative { -magnitude } else { magnitude };
    i64::try_from(millis)
        .map(Duration::from_millis)
        .map_err(|_| invalid("the duration exceeds the range of 64-bit milliseconds"))
}

/// Splits `30m…` into the count, the unit and the rest of the text.
fn split_duration_part(text: &str) -> Option<(u128, &str, &str)> {
    let digits_end =
        text.find(|character: char| !(character.is_ascii_digit() || character == '_'))?;
    let (number, after) = text.split_at(digits_end);
    let count = parse_digits(number)?;
    let unit_end = after
        .find(|character: char| !character.is_ascii_alphabetic())
        .unwrap_or(after.len());
    let (unit, rest) = after.split_at(unit_end);
    (!unit.is_empty()).then_some((count, unit, rest))
}

/// Canonical form of a duration: days, hours, minutes, seconds and
/// milliseconds, each present when not zero; zero is `0s`.
///
/// The result parses back to the same duration with [`parse_duration`].
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::Duration;
/// use nurt_types::format_duration;
///
/// assert_eq!(format_duration(Duration::from_millis(5_400_000)), "1h30m");
/// assert_eq!(format_duration(Duration::from_millis(-1_250)), "-1s250ms");
/// assert_eq!(format_duration(Duration::ZERO), "0s");
/// ```
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    let mut rest = millis.unsigned_abs();
    if rest == 0 {
        return "0s".to_owned();
    }
    let mut text = String::new();
    if millis < 0 {
        text.push('-');
    }
    for (name, factor) in CANONICAL_DURATION_UNITS {
        let count = rest / factor;
        if count > 0 {
            text.push_str(&count.to_string());
            text.push_str(name);
            rest %= factor;
        }
    }
    text
}

/// Short form of a duration in reports.
///
/// Below one second: milliseconds (`250ms`); below one minute: seconds with
/// one fraction digit when needed (`1.2s`, `3s`); otherwise the two largest
/// units of the duration rounded to seconds (`1h30m`, `2d4h`, `5m`).
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::Duration;
/// use nurt_types::display_duration;
///
/// assert_eq!(display_duration(Duration::from_millis(1_240)), "1.2s");
/// assert_eq!(display_duration(Duration::from_millis(5_412_000)), "1h30m");
/// ```
#[must_use]
pub fn display_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    let sign = if millis < 0 { "-" } else { "" };
    let magnitude = millis.unsigned_abs();
    if magnitude < MS_PER_SECOND {
        return format!("{sign}{magnitude}ms");
    }
    let tenths = (magnitude + MS_PER_TENTH / 2) / MS_PER_TENTH;
    if tenths < TENTHS_PER_MINUTE {
        return if tenths.is_multiple_of(10) {
            format!("{sign}{}s", tenths / 10)
        } else {
            format!("{sign}{}.{}s", tenths / 10, tenths % 10)
        };
    }
    let seconds = magnitude.saturating_add(MS_PER_SECOND / 2) / MS_PER_SECOND * MS_PER_SECOND;
    let parts: Vec<(u64, &str)> = CANONICAL_DURATION_UNITS
        .iter()
        .filter(|(_, factor)| *factor >= MS_PER_SECOND)
        .scan(seconds, |rest, &(name, factor)| {
            let count = *rest / factor;
            *rest %= factor;
            Some((count, name))
        })
        .collect();
    let first = parts.iter().position(|(count, _)| *count > 0).unwrap_or(0);
    let mut text = sign.to_owned();
    for (count, name) in parts.iter().skip(first).take(2) {
        if *count > 0 {
            text.push_str(&count.to_string());
            text.push_str(name);
        }
    }
    text
}

/// Parses an instant.
///
/// Accepts RFC 3339 with an offset (`2026-11-21T10:00:00Z`,
/// `2026-11-21T11:00:00+01:00`), a date and time without an offset
/// (`2026-11-21T10:00:00`) and a date alone (`2026-11-21`, the start of the
/// day). Texts without an offset are read in the local time rules `tz`.
///
/// # Errors
///
/// Returns a [`ValueError`] when the text is not an instant or it lies
/// outside the years 1677 to 2262.
///
/// # Examples
///
/// ```rust
/// use jiff::tz::{self, TimeZone};
/// use nurt_types::parse_instant;
///
/// let utc = TimeZone::UTC;
/// assert_eq!(parse_instant("2026-11-21", &utc)?.to_string(), "2026-11-21T00:00:00Z");
/// let warsaw_winter = TimeZone::fixed(tz::offset(1));
/// assert_eq!(
///     parse_instant("2026-11-21T10:00:00", &warsaw_winter)?.to_string(),
///     "2026-11-21T09:00:00Z"
/// );
/// # Ok::<(), nurt_contracts::value::ValueError>(())
/// ```
pub fn parse_instant(text: &str, tz: &TimeZone) -> Result<Instant, ValueError> {
    if let Ok(instant) = text.parse::<Instant>() {
        return Ok(instant);
    }
    parse_local_instant(text, tz).ok_or_else(|| ValueError::new("instant", text, INSTANT_FORMAT))
}

fn parse_local_instant(text: &str, tz: &TimeZone) -> Option<Instant> {
    let time_part = text.get(DATE_LENGTH..)?;
    if time_part.contains(['+', '-', 'Z', 'z']) {
        return None;
    }
    let local: civil::DateTime = if time_part.is_empty() {
        text.parse::<civil::Date>()
            .ok()?
            .to_datetime(civil::Time::midnight())
    } else {
        text.parse().ok()?
    };
    let timestamp = tz.to_timestamp(local).ok()?;
    i64::try_from(timestamp.as_nanosecond())
        .ok()
        .map(Instant::from_unix_nanos)
}

/// Short form of an instant in reports: RFC 3339 in UTC to the second.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::Instant;
/// use nurt_types::display_instant;
///
/// let instant: Instant = "2026-11-21T10:00:00.750Z".parse()?;
/// assert_eq!(display_instant(instant), "2026-11-21T10:00:00Z");
/// # Ok::<(), nurt_contracts::value::ValueError>(())
/// ```
#[must_use]
pub fn display_instant(instant: Instant) -> String {
    let nanos = instant.as_unix_nanos();
    Instant::from_unix_nanos(nanos - nanos.rem_euclid(NANOS_PER_SECOND)).to_string()
}

/// Parses a range: `5`, `1..5`, `40..`, `..9` or `..`, bounds inclusive.
///
/// The canonical form is the `Display` form of [`Range`].
///
/// # Errors
///
/// Returns a [`ValueError`] when the text is not a range or the start is
/// greater than the end.
///
/// # Examples
///
/// ```rust
/// use nurt_types::parse_range;
///
/// let range = parse_range("-3..7")?;
/// assert_eq!((range.start(), range.end()), (Some(-3), Some(7)));
/// assert_eq!(parse_range("40..")?.to_string(), "40..");
/// assert!(parse_range("5..1").is_err());
/// # Ok::<(), nurt_contracts::value::ValueError>(())
/// ```
pub fn parse_range(text: &str) -> Result<Range, ValueError> {
    let invalid = || ValueError::new("range", text, RANGE_FORMAT);
    let bound = |part: &str| -> Result<Option<i64>, ValueError> {
        if part.is_empty() {
            Ok(None)
        } else {
            parse_integer(part).map(Some).ok_or_else(invalid)
        }
    };
    match text.split_once("..") {
        None => parse_integer(text).map(Range::single).ok_or_else(invalid),
        Some((start, end)) => Range::new(bound(start)?, bound(end)?),
    }
}

/// Parses `-?[0-9][0-9_]*` to a 64-bit integer.
fn parse_integer(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text),
    };
    let magnitude = i128::try_from(parse_digits(digits)?).ok()?;
    i64::try_from(if negative { -magnitude } else { magnitude }).ok()
}

/// Parses `[0-9][0-9_]*`; `None` on other text or overflow.
fn parse_digits(text: &str) -> Option<u128> {
    if !text.starts_with(|character: char| character.is_ascii_digit()) {
        return None;
    }
    text.bytes()
        .filter(|&byte| byte != b'_')
        .try_fold(0_u128, |value, byte| {
            byte.is_ascii_digit()
                .then_some(())
                .and_then(|()| value.checked_mul(10))
                .and_then(|value| value.checked_add(u128::from(byte - b'0')))
        })
}

/// Parses the digits after the decimal point; returns the value and the
/// number of digits.
fn parse_fraction(text: &str) -> Option<(u128, u32)> {
    let width = u32::try_from(text.len()).ok()?;
    if width == 0 || width > MAX_FRACTION_DIGITS || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    parse_digits(text).map(|value| (value, width))
}
