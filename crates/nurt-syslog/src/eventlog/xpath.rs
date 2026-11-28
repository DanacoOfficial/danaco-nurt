// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::eventlog::xpath
// Responsibility:   XPath queries of the Windows event log.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! XPath queries of the Windows event log.
//!
//! The event log accepts a subset of XPath 1.0 that selects events by the
//! elements and attributes of their `System` element. The query only narrows
//! the events read from a channel: every event is checked again against the
//! parameters of `logs`, so a condition that cannot be expressed exactly is
//! widened or left out, never narrowed.

use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;

use super::level;

const NANOS_PER_MILLI: i64 = 1_000_000;

/// Conditions of a query of one channel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XPathFilter<'a> {
    /// Lowest level of the events.
    pub level: LogLevel,
    /// Earliest creation time, inclusive.
    pub since: Option<Instant>,
    /// Latest creation time, exclusive.
    pub until: Option<Instant>,
    /// Exact name of the provider.
    pub provider: Option<&'a str>,
    /// Only events with a smaller `EventRecordID`.
    pub before_record: Option<u64>,
}

impl XPathFilter<'_> {
    /// The XPath expression for `EvtQuery`.
    ///
    /// Times are written with millisecond precision: `since` is rounded down
    /// and `until` up, so that the query never excludes an event the
    /// parameters accept. A provider name that contains both kinds of quotes
    /// cannot be written as an XPath 1.0 literal and is left out of the query.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::record::LogLevel;
    /// use nurt_syslog::eventlog::xpath::XPathFilter;
    /// let filter = XPathFilter {
    ///     level: LogLevel::Error,
    ///     provider: Some("Service Control Manager"),
    ///     ..XPathFilter::default()
    /// };
    /// assert_eq!(
    ///     filter.to_xpath(),
    ///     "*[System[(Level=1 or Level=2) and Provider[@Name='Service Control Manager']]]",
    /// );
    /// assert_eq!(XPathFilter { level: LogLevel::Debug, ..XPathFilter::default() }.to_xpath(), "*");
    /// ```
    #[must_use]
    pub fn to_xpath(&self) -> String {
        let mut terms = Vec::new();
        if let Some(condition) = level::xpath_condition(self.level) {
            terms.push(condition);
        }
        if let Some(condition) = self.time_condition() {
            terms.push(condition);
        }
        if let Some(literal) = self.provider.and_then(literal) {
            terms.push(format!("Provider[@Name={literal}]"));
        }
        if let Some(record) = self.before_record {
            terms.push(format!("EventRecordID<{record}"));
        }
        if terms.is_empty() {
            "*".to_owned()
        } else {
            format!("*[System[{}]]", terms.join(" and "))
        }
    }

    fn time_condition(&self) -> Option<String> {
        let since = self
            .since
            .map(|since| format!("@SystemTime>='{}'", system_time(floor_millis(since))));
        let until = self
            .until
            .map(|until| format!("@SystemTime<'{}'", system_time(ceil_millis(until))));
        match (since, until) {
            (None, None) => None,
            (Some(one), None) | (None, Some(one)) => Some(format!("TimeCreated[{one}]")),
            (Some(since), Some(until)) => Some(format!("TimeCreated[{since} and {until}]")),
        }
    }
}

/// XPath 1.0 string literal for the text, or `None` when the text contains
/// both an apostrophe and a quotation mark.
///
/// XPath 1.0 has no escape sequences in literals; a literal is delimited by
/// the quote character it does not contain.
///
/// # Examples
///
/// ```
/// use nurt_syslog::eventlog::xpath::literal;
/// assert_eq!(literal("Disk").as_deref(), Some("'Disk'"));
/// assert_eq!(literal("Bob's app").as_deref(), Some("\"Bob's app\""));
/// assert_eq!(literal("a'b\"c"), None);
/// ```
#[must_use]
pub fn literal(text: &str) -> Option<String> {
    if !text.contains('\'') {
        Some(format!("'{text}'"))
    } else if !text.contains('"') {
        Some(format!("\"{text}\""))
    } else {
        None
    }
}

fn floor_millis(instant: Instant) -> Instant {
    let nanos = instant.as_unix_nanos();
    nanos
        .checked_sub(nanos.rem_euclid(NANOS_PER_MILLI))
        .map_or(instant, Instant::from_unix_nanos)
}

fn ceil_millis(instant: Instant) -> Instant {
    let floor = floor_millis(instant);
    if floor == instant {
        return instant;
    }
    floor
        .as_unix_nanos()
        .checked_add(NANOS_PER_MILLI)
        .map_or(floor, Instant::from_unix_nanos)
}

/// `TimeCreated/@SystemTime` form of a whole number of milliseconds:
/// `2026-11-21T10:00:00.000Z`.
fn system_time(instant: Instant) -> String {
    let text = instant.to_string();
    match text.strip_suffix('Z') {
        Some(seconds) if !seconds.contains('.') => format!("{seconds}.000Z"),
        _ => text,
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
