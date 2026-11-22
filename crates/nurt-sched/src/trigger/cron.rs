// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Five-field cron expressions.
//!
//! Fields are minute (0–59), hour (0–23), day of month (1–31), month (1–12
//! or `jan`–`dec`) and day of week (0–7 or `sun`–`sat`, where 0 and 7 are
//! Sunday). A field holds `*`, numbers, names, ranges `a-b`, lists `a,b` and
//! steps: `*/n` from the lower bound, `a-b/n` within the range and `a/n` from
//! `a` to the end of the range. Names are not case sensitive. The shortcuts
//! `@yearly`, `@annually`, `@monthly`, `@weekly`, `@daily` and `@hourly` are
//! accepted; `@reboot` and the extensions `L`, `W`, `#` and `?` are not.
//!
//! When neither the day of month nor the day of week starts with `*`, a day
//! matches when either field matches (Vixie cron); otherwise both must match.
//!
//! Croner parses the fields; the search for the next run works on the wall
//! clock of the time zone of the schedule: a time skipped by a change to
//! daylight saving time runs at the same distance after the change, and a
//! time repeated by the change back runs once, at its first occurrence.

use croner::parser::{CronParser, Seconds, Year};
use jiff::civil::{Date, DateTime};
use jiff::tz::{AmbiguousOffset, TimeZone};
use nurt_contracts::error::Error;
use nurt_contracts::value::Instant;

use super::time;
use crate::errors::invalid;

const MAX_LENGTH: usize = 200;
const FIELDS: usize = 5;
const SEARCH_DAYS: i32 = 5 * 366;
const MONTH_FIELD: usize = 3;
const WEEKDAY_FIELD: usize = 4;
const DAY_FIELD: usize = 2;

const EXAMPLES: &str = "for example `*/10 * * * *` (every ten minutes) or `0 3 * * mon` \
                        (Mondays at 03:00)";

struct FieldRule {
    name: &'static str,
    min: u16,
    max: u16,
    names: &'static [&'static str],
}

const MONTH_NAMES: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAY_NAMES: &[&str] = &["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

const RULES: [FieldRule; FIELDS] = [
    FieldRule {
        name: "minute",
        min: 0,
        max: 59,
        names: &[],
    },
    FieldRule {
        name: "hour",
        min: 0,
        max: 23,
        names: &[],
    },
    FieldRule {
        name: "day of month",
        min: 1,
        max: 31,
        names: &[],
    },
    FieldRule {
        name: "month",
        min: 1,
        max: 12,
        names: MONTH_NAMES,
    },
    FieldRule {
        name: "day of week",
        min: 0,
        max: 7,
        names: WEEKDAY_NAMES,
    },
];

const SHORTCUTS: &[(&str, &str)] = &[
    ("@yearly", "0 0 1 1 *"),
    ("@annually", "0 0 1 1 *"),
    ("@monthly", "0 0 1 * *"),
    ("@weekly", "0 0 * * 0"),
    ("@daily", "0 0 * * *"),
    ("@hourly", "0 * * * *"),
];

/// A parsed cron expression.
///
/// ```
/// use nurt_sched::trigger::cron::CronSpec;
///
/// let spec = CronSpec::parse("*/15 9-17 * jan,JUN mon-fri")?;
/// assert_eq!(spec.expression(), "*/15 9-17 * jan,JUN mon-fri");
/// assert!(CronSpec::parse("61 * * * *").is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    expression: String,
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    any_day: bool,
}

impl CronSpec {
    /// Parses an expression or a shortcut.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` with the field at fault and an example of a
    /// valid expression.
    pub fn parse(expression: &str) -> Result<Self, Error> {
        let trimmed = expression.trim();
        if trimmed.chars().count() > MAX_LENGTH {
            return Err(invalid(format!(
                "a cron expression has at most {MAX_LENGTH} characters"
            )));
        }
        let fields_text = expand_shortcut(trimmed)?;
        let fields: Vec<&str> = fields_text.split_whitespace().collect();
        if fields.len() != FIELDS {
            return Err(invalid(format!(
                "a cron expression has five fields (minute, hour, day of month, month, day of \
                 week), not {}; {EXAMPLES}",
                fields.len()
            ))
            .with_found(trimmed));
        }
        for (field, rule) in fields.iter().zip(RULES.iter()) {
            check_field(field, rule).map_err(|error| error.with_found(trimmed))?;
        }
        let parsed = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .sloppy_ranges(true)
            .build()
            .parse(&fields_text)
            .map_err(|error| invalid(format!("invalid cron expression: {error}; {EXAMPLES}")))?;
        let pattern = &parsed.pattern;
        let starts_with_star = |index: usize| fields.get(index).is_some_and(|f| f.starts_with('*'));
        Ok(Self {
            expression: trimmed.to_owned(),
            minutes: bits(&pattern.minutes.values()),
            hours: narrow(bits(&pattern.hours.values())),
            days: narrow(bits(&pattern.days.values())),
            months: narrow(bits(&pattern.months.values())),
            weekdays: narrow(bits(&pattern.days_of_week.values())),
            any_day: starts_with_star(DAY_FIELD) || starts_with_star(WEEKDAY_FIELD),
        })
    }

    /// The expression as written, without surrounding white space.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// First run strictly after `after` in `zone`, or `None` when the
    /// expression has no run within five years.
    #[must_use]
    pub fn next_after(&self, zone: &TimeZone, after: Instant) -> Option<Instant> {
        let start = time::timestamp(after)?.to_zoned_datetime(zone.clone()).date();
        let mut date = start;
        for _ in 0..=SEARCH_DAYS {
            if self.date_matches(date)
                && let Some(found) = self.first_on(zone, date, after)
            {
                return Some(found);
            }
            date = date.tomorrow().ok()?;
        }
        None
    }

    fn date_matches(&self, date: Date) -> bool {
        let month_ok = self.months & (1 << date.month()) != 0;
        let day_ok = self.days & (1 << date.day()) != 0;
        let weekday = date.weekday().to_sunday_zero_offset();
        let weekday_ok = self.weekdays & (1 << weekday) != 0;
        let day_matches = if self.any_day {
            day_ok && weekday_ok
        } else {
            day_ok || weekday_ok
        };
        month_ok && day_matches
    }

    fn first_on(&self, zone: &TimeZone, date: Date, after: Instant) -> Option<Instant> {
        for hour in set_bits(u64::from(self.hours)) {
            for minute in set_bits(self.minutes) {
                let civil = DateTime::new(
                    date.year(),
                    date.month(),
                    date.day(),
                    i8::try_from(hour).ok()?,
                    i8::try_from(minute).ok()?,
                    0,
                    0,
                )
                .ok()?;
                let Some(candidate) = resolve(zone, civil) else {
                    continue;
                };
                if candidate.as_unix_nanos() > after.as_unix_nanos() {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

/// Instant of a wall-clock time in `zone`: a skipped time is moved forward by
/// the length of the gap, a repeated time resolves to its first occurrence.
fn resolve(zone: &TimeZone, civil: DateTime) -> Option<Instant> {
    let ambiguous = zone.to_ambiguous_zoned_datetime(civil);
    let zoned = match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } | AmbiguousOffset::Gap { .. } => {
            ambiguous.compatible().ok()?
        }
        AmbiguousOffset::Fold { .. } => ambiguous.earlier().ok()?,
    };
    Some(time::instant(zoned.timestamp()))
}

fn expand_shortcut(text: &str) -> Result<String, Error> {
    if !text.starts_with('@') {
        return Ok(text.to_owned());
    }
    SHORTCUTS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|(_, fields)| (*fields).to_owned())
        .ok_or_else(|| {
            invalid(format!(
                "unknown cron shortcut `{text}`; use one of @yearly, @monthly, @weekly, @daily, \
                 @hourly"
            ))
            .with_found(text)
        })
}

fn check_field(field: &str, rule: &FieldRule) -> Result<(), Error> {
    for part in field.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, Some(step)),
            None => (part, None),
        };
        if let Some(step) = step {
            match step.parse::<u16>() {
                Ok(0) | Err(_) => {
                    return Err(invalid(format!(
                        "the step `{step}` of the {} is not a positive number",
                        rule.name
                    )));
                }
                Ok(_) => {}
            }
        }
        if range == "*" {
            continue;
        }
        let (low, high) = match range.split_once('-') {
            Some((low, high)) => (value(low, rule)?, Some(value(high, rule)?)),
            None => (value(range, rule)?, None),
        };
        if let Some(high) = high
            && high < low
            && !(rule.names == WEEKDAY_NAMES && high == 0)
        {
            return Err(invalid(format!(
                "the range `{range}` of the {} decreases; write the lower bound first",
                rule.name
            )));
        }
    }
    Ok(())
}

fn value(text: &str, rule: &FieldRule) -> Result<u16, Error> {
    let lower = text.to_ascii_lowercase();
    if let Some(position) = rule.names.iter().position(|name| *name == lower) {
        let offset = u16::from(rule.min == 1);
        return u16::try_from(position).map(|index| index + offset).map_err(|_| {
            invalid(format!("the {} `{text}` is not valid", rule.name))
        });
    }
    let number: u16 = text.parse().map_err(|_| {
        let names = if rule.names.is_empty() {
            String::new()
        } else {
            format!(" or a name such as `{}`", rule.names.first().copied().unwrap_or_default())
        };
        invalid(format!(
            "the {} `{text}` is not valid; use a number from {} to {}{names}",
            rule.name, rule.min, rule.max
        ))
    })?;
    if number < rule.min || number > rule.max {
        return Err(invalid(format!(
            "the {} {number} is outside {}-{}",
            rule.name, rule.min, rule.max
        )));
    }
    Ok(number)
}

fn bits(values: &[u16]) -> u64 {
    values
        .iter()
        .filter(|value| **value < 64)
        .fold(0, |acc, value| acc | (1 << value))
}

fn narrow<T: TryFrom<u64> + Default>(bits: u64) -> T {
    T::try_from(bits).unwrap_or_default()
}

fn set_bits(bits: u64) -> impl Iterator<Item = u32> {
    (0..64).filter(move |bit| bits & (1 << bit) != 0)
}
