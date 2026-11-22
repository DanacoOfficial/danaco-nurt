// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Triggers of schedules: cron expressions, intervals and single times.
//!
//! A trigger is saved in a definition as a [`TriggerDef`] in normalized form
//! and compiled into a [`Trigger`] that computes run times. Every computed run
//! time is strictly later than the instant it is computed from.

pub mod cron;
pub mod time;
mod zone;

use std::time::Duration;

use jiff::tz::TimeZone;
use nurt_contracts::error::Error;
use nurt_contracts::record::TriggerKind;
use nurt_contracts::value::Instant;
use serde::{Deserialize, Serialize};

pub use self::cron::CronSpec;
pub use self::zone::LOCAL;
use crate::errors::invalid;
use crate::settings::ScheduleSettings;

const SECOND: u64 = 1;
const MINUTE: u64 = 60;
const HOUR: u64 = 3_600;
const DAY: u64 = 86_400;
const MILLIS_PER_SECOND: u128 = 1_000;

/// Trigger as saved in a schedule definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDef {
    /// Kind of trigger.
    pub kind: TriggerKind,
    /// Cron expression, interval (`10m`, `1h30m`) or point in time (RFC 3339,
    /// UTC).
    pub expression: String,
    /// IANA name of the time zone of the cron expression and of the times
    /// shown to the user.
    pub timezone: String,
}

impl TriggerDef {
    /// A cron trigger; `timezone` may be `local`, which is replaced by
    /// `host_zone`.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` for an invalid expression, an unknown time
    /// zone (with similar names as candidates) or an expression without a run
    /// within five years after `now`.
    pub fn cron(expression: &str, timezone: &str, host_zone: &str, now: Instant) -> Result<Self, Error> {
        let spec = CronSpec::parse(expression)?;
        let (name, zone) = zone::resolve(timezone, host_zone)?;
        if spec.next_after(&zone, now).is_none() {
            return Err(invalid(format!(
                "the cron expression `{}` has no run within five years; check the day and month \
                 fields",
                spec.expression()
            ))
            .with_found(spec.expression()));
        }
        Ok(Self {
            kind: TriggerKind::Cron,
            expression: spec.expression().to_owned(),
            timezone: name,
        })
    }

    /// An interval trigger.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` when the interval lies outside
    /// [`ScheduleSettings::min_interval`] and
    /// [`ScheduleSettings::max_interval`], or for an unknown time zone.
    pub fn every(
        interval: Duration,
        timezone: &str,
        host_zone: &str,
        settings: &ScheduleSettings,
    ) -> Result<Self, Error> {
        if interval < settings.min_interval {
            let shortest = format_duration(settings.min_interval);
            return Err(invalid(format!(
                "the shortest interval is {shortest}; use `every: {shortest}` or longer"
            ))
            .with_found(format_duration(interval))
            .with_fix(format!("every: {shortest}")));
        }
        if interval > settings.max_interval {
            let longest = format_duration(settings.max_interval);
            return Err(invalid(format!(
                "the longest interval is {longest}; use a cron expression for rarer runs"
            ))
            .with_found(format_duration(interval)));
        }
        let (name, _) = zone::resolve(timezone, host_zone)?;
        Ok(Self {
            kind: TriggerKind::Every,
            expression: format_duration(interval),
            timezone: name,
        })
    }

    /// A trigger at a single point in time.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` when `at` is not later than `now`, or for an
    /// unknown time zone.
    pub fn at(at: Instant, timezone: &str, host_zone: &str, now: Instant) -> Result<Self, Error> {
        if at.as_unix_nanos() <= now.as_unix_nanos() {
            return Err(invalid(format!(
                "`at: {at}` is a time that has already passed; give a later time such as \
                 `at: from_now(10m)`"
            ))
            .with_found(at.to_string()));
        }
        let (name, _) = zone::resolve(timezone, host_zone)?;
        Ok(Self {
            kind: TriggerKind::At,
            expression: at.to_string(),
            timezone: name,
        })
    }

    /// Normalized form shown in the `trigger` of a schedule:
    /// `cron */10 * * * *`, `every 10m`, `at 2026-11-23T03:00:00+01:00`.
    #[must_use]
    pub fn normalized(&self) -> String {
        match self.kind {
            TriggerKind::At => {
                let local = self
                    .expression
                    .parse::<Instant>()
                    .ok()
                    .and_then(|at| format_local(at, &self.timezone));
                format!("at {}", local.unwrap_or_else(|| self.expression.clone()))
            }
            kind => format!("{} {}", kind.as_str(), self.expression),
        }
    }
}

/// A compiled trigger.
#[derive(Debug, Clone)]
pub enum Trigger {
    /// Cron expression in a time zone.
    Cron {
        /// Expression.
        spec: CronSpec,
        /// Time zone of the expression.
        zone: TimeZone,
    },
    /// Fixed interval.
    Every {
        /// Interval.
        interval: Duration,
    },
    /// Single point in time.
    At {
        /// The time.
        at: Instant,
    },
}

impl Trigger {
    /// Compiles a saved trigger.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` when the saved form is not valid.
    pub fn compile(def: &TriggerDef) -> Result<Self, Error> {
        match def.kind {
            TriggerKind::Every => parse_duration(&def.expression)
                .map(|interval| Self::Every { interval })
                .ok_or_else(|| invalid(format!("invalid interval `{}`", def.expression))),
            TriggerKind::At => def
                .expression
                .parse::<Instant>()
                .map(|at| Self::At { at })
                .map_err(|_| invalid(format!("invalid time `{}`", def.expression))),
            _ => Ok(Self::Cron {
                spec: CronSpec::parse(&def.expression)?,
                zone: zone::get(&def.timezone)?,
            }),
        }
    }

    /// First run time strictly after `after`; `None` when there is none.
    #[must_use]
    pub fn next_after(&self, after: Instant) -> Option<Instant> {
        match self {
            Self::Cron { spec, zone } => spec.next_after(zone, after),
            Self::Every { interval } => Some(time::add(after, *interval)),
            Self::At { at } => (at.as_unix_nanos() > after.as_unix_nanos()).then_some(*at),
        }
    }

    /// First run time of a schedule created at `created` and first seen by
    /// the service at `now`.
    ///
    /// An interval counts from the creation; when that run time has passed,
    /// from `now`.
    #[must_use]
    pub fn first_run(&self, created: Instant, now: Instant) -> Option<Instant> {
        match self {
            Self::Every { interval } => {
                let first = time::add(created, *interval);
                if first.as_unix_nanos() > now.as_unix_nanos() {
                    Some(first)
                } else {
                    Some(time::add(now, *interval))
                }
            }
            Self::At { at } => Some(*at),
            Self::Cron { .. } => self.next_after(later(created, now)),
        }
    }

    /// Run time that follows the run time `due`, handled at `now`.
    ///
    /// An interval adds to the previous run time while that lies in the
    /// future, and counts from `now` after a pause in service, so missed runs
    /// are not made up one by one.
    #[must_use]
    pub fn following(&self, due: Instant, now: Instant) -> Option<Instant> {
        match self {
            Self::Every { interval } => {
                let next = time::add(due, *interval);
                if next.as_unix_nanos() > now.as_unix_nanos() {
                    Some(next)
                } else {
                    Some(time::add(now, *interval))
                }
            }
            Self::At { .. } => None,
            Self::Cron { .. } => self.next_after(later(due, now)),
        }
    }

    /// Returns `true` for a trigger with a single run.
    #[must_use]
    pub fn is_single(&self) -> bool {
        matches!(self, Self::At { .. })
    }
}

fn later(a: Instant, b: Instant) -> Instant {
    if a.as_unix_nanos() >= b.as_unix_nanos() { a } else { b }
}

/// Text form of a duration in the units `d`, `h`, `m`, `s` and `ms`, for
/// example `1h30m`.
///
/// ```
/// use std::time::Duration;
/// use nurt_sched::trigger::format_duration;
///
/// assert_eq!(format_duration(Duration::from_secs(5400)), "1h30m");
/// assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
/// ```
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let mut seconds = duration.as_secs();
    let millis = duration.as_millis() % MILLIS_PER_SECOND;
    let mut text = String::new();
    for (unit, size) in [("d", DAY), ("h", HOUR), ("m", MINUTE), ("s", SECOND)] {
        let count = seconds / size;
        if count > 0 {
            text.push_str(&format!("{count}{unit}"));
            seconds %= size;
        }
    }
    if millis > 0 || text.is_empty() {
        text.push_str(&format!("{millis}ms"));
    }
    text
}

fn parse_duration(text: &str) -> Option<Duration> {
    let mut total = Duration::ZERO;
    let mut rest = text;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        let (number, tail) = rest.split_at(digits);
        let count: u64 = number.parse().ok()?;
        let unit_end = tail.find(|c: char| c.is_ascii_digit()).unwrap_or(tail.len());
        let (unit, tail) = tail.split_at(unit_end);
        let part = match unit {
            "d" => Duration::from_secs(count.checked_mul(DAY)?),
            "h" => Duration::from_secs(count.checked_mul(HOUR)?),
            "m" => Duration::from_secs(count.checked_mul(MINUTE)?),
            "s" => Duration::from_secs(count),
            "ms" => Duration::from_millis(count),
            _ => return None,
        };
        total = total.checked_add(part)?;
        rest = tail;
    }
    Some(total)
}

/// RFC 3339 form of `instant` with the offset of the time zone `zone`.
#[must_use]
pub fn format_local(instant: Instant, zone: &str) -> Option<String> {
    let zone = TimeZone::get(zone).ok()?;
    let zoned = time::timestamp(instant)?.to_zoned_datetime(zone);
    Some(zoned.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
}
