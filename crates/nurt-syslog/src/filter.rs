// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::filter
// Responsibility:   Final filtering of log events.
// Dependencies:     nurt_contracts, proptest, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Final filtering of log events.
//!
//! Every reader applies [`EventFilter`] to each event after its native
//! search. The filter gives every parameter of `logs` the meaning of a later
//! `filter` on the output record: `source` and `unit` compare the fields of
//! the same name, `level` admits the levels at or above the threshold,
//! `since` and `until` bound `time` (start inclusive, end exclusive) and
//! `message` searches the message text.

use nurt_contracts::value::Instant;
use regex::Regex;

use crate::event::LogEvent;
use crate::query::{LogQuery, NameMatch};

/// Predicate over events built from a query.
///
/// ```
/// use nurt_contracts::value::Instant;
/// use nurt_syslog::cursor::Cursor;
/// use nurt_syslog::event::{LogEvent, StoreKind};
/// use nurt_syslog::filter::EventFilter;
/// use nurt_syslog::query::{LogQuery, NameMatch};
///
/// let query = LogQuery { source: Some(NameMatch::parse("cron").unwrap()), ..LogQuery::default() };
/// let filter = EventFilter::new(&query);
/// let cursor = Cursor::EventLog { channel: "System".to_owned(), record_id: 1 };
/// let event = LogEvent::new(Instant::from_unix_nanos(0), "cron", 6, "run", StoreKind::EventLog, cursor);
/// assert!(filter.matches(&event));
/// ```
#[derive(Debug, Clone)]
pub struct EventFilter<'q> {
    source: Option<&'q NameMatch>,
    unit: Option<&'q NameMatch>,
    max_priority: u8,
    since: Option<Instant>,
    until: Option<Instant>,
    message: Option<&'q Regex>,
}

impl<'q> EventFilter<'q> {
    /// Filter with all conditions of `query`.
    #[must_use]
    pub fn new(query: &'q LogQuery) -> Self {
        Self {
            source: query.source.as_ref(),
            unit: query.unit.as_ref(),
            max_priority: query.max_priority(),
            since: query.since,
            until: query.until,
            message: query.message.as_ref(),
        }
    }

    /// Replaces the unit condition, for stores that normalize unit names.
    #[must_use]
    pub fn with_unit(mut self, unit: Option<&'q NameMatch>) -> Self {
        self.unit = unit;
        self
    }

    /// Whether the event satisfies every condition.
    #[must_use]
    pub fn matches(&self, event: &LogEvent) -> bool {
        self.admits_priority(event.priority)
            && self.admits_time(event.time)
            && self
                .source
                .is_none_or(|source| source.matches(&event.source))
            && self.unit.is_none_or(|unit| {
                event
                    .unit
                    .as_deref()
                    .is_some_and(|value| unit.matches(value))
            })
            && self
                .message
                .is_none_or(|message| message.is_match(&event.message))
    }

    /// Whether the level threshold admits `priority`.
    #[must_use]
    pub const fn admits_priority(&self, priority: u8) -> bool {
        priority <= self.max_priority
    }

    /// Whether `time` lies in `since..until`.
    #[must_use]
    pub fn admits_time(&self, time: Instant) -> bool {
        self.since.is_none_or(|since| time >= since) && self.until.is_none_or(|until| time < until)
    }

    /// Whether `time` is before `since`, so that older events of a stream
    /// ordered newest first cannot match any more.
    #[must_use]
    pub fn is_before_range(&self, time: Instant) -> bool {
        self.since.is_some_and(|since| time < since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Cursor;
    use crate::event::{StoreKind, max_priority};
    use nurt_contracts::record::LogLevel;
    use nurt_contracts::value::{Record, Value};
    use proptest::prelude::*;

    const SOURCES: [&str; 3] = ["sshd", "cron", "kernel"];
    const UNITS: [&str; 3] = ["ssh.service", "cron.service", "user@1000.service"];
    const MESSAGES: [&str; 3] = ["accepted key", "session opened", "failed password"];

    fn event(
        source: usize,
        unit: Option<usize>,
        priority: u8,
        message: usize,
        time: i64,
    ) -> LogEvent {
        let mut event = LogEvent::new(
            Instant::from_unix_nanos(time),
            SOURCES[source],
            priority,
            MESSAGES[message],
            StoreKind::Journald,
            Cursor::EventLog {
                channel: "c".to_owned(),
                record_id: 0,
            },
        );
        event.unit = unit.map(|index| UNITS[index].to_owned());
        event
    }

    fn text_field<'r>(record: &'r Record, name: &str) -> Option<&'r str> {
        match record.get(name) {
            Some(Value::Text(text)) => Some(text.as_str()),
            _ => None,
        }
    }

    fn level_rank(record: &Record) -> u8 {
        let level = match record.get("level") {
            Some(Value::Enum(text) | Value::Text(text)) => text.parse::<LogLevel>().unwrap(),
            other => panic!("level missing: {other:?}"),
        };
        max_priority(level)
    }

    /// Evaluates the conditions of `query` the way `filter` evaluates them
    /// on the fields of the output record.
    fn record_filter(query: &LogQuery, record: &Record) -> bool {
        let time = match record.get("time") {
            Some(Value::Instant(time)) => *time,
            other => panic!("time missing: {other:?}"),
        };
        let source = text_field(record, "source").unwrap_or_default();
        let message = text_field(record, "message").unwrap_or_default();
        let unit = text_field(record, "unit");
        level_rank(record) <= max_priority(query.level)
            && query.since.is_none_or(|since| time >= since)
            && query.until.is_none_or(|until| time < until)
            && query
                .source
                .as_ref()
                .is_none_or(|name| name.matches(source))
            && query
                .unit
                .as_ref()
                .is_none_or(|name| unit.is_some_and(|unit| name.matches(unit)))
            && query
                .message
                .as_ref()
                .is_none_or(|regex| regex.is_match(message))
    }

    #[test]
    fn empty_query_admits_info_and_rejects_debug() {
        let query = LogQuery::default();
        let filter = EventFilter::new(&query);
        assert!(filter.matches(&event(0, None, 6, 0, 0)));
        assert!(!filter.matches(&event(0, None, 7, 0, 0)));
    }

    #[test]
    fn time_range_includes_start_and_excludes_end() {
        let query = LogQuery {
            since: Some(Instant::from_unix_nanos(10)),
            until: Some(Instant::from_unix_nanos(20)),
            ..LogQuery::default()
        };
        let filter = EventFilter::new(&query);
        assert!(filter.admits_time(Instant::from_unix_nanos(10)));
        assert!(filter.admits_time(Instant::from_unix_nanos(19)));
        assert!(!filter.admits_time(Instant::from_unix_nanos(20)));
        assert!(!filter.admits_time(Instant::from_unix_nanos(9)));
        assert!(filter.is_before_range(Instant::from_unix_nanos(9)));
        assert!(!filter.is_before_range(Instant::from_unix_nanos(10)));
    }

    #[test]
    fn unit_condition_rejects_events_without_unit() {
        let query = LogQuery {
            unit: Some(NameMatch::parse("*").unwrap()),
            ..LogQuery::default()
        };
        let filter = EventFilter::new(&query);
        assert!(!filter.matches(&event(0, None, 6, 0, 0)));
        assert!(filter.matches(&event(0, Some(0), 6, 0, 0)));
    }

    #[test]
    fn replaced_unit_condition_is_used() {
        let query = LogQuery {
            unit: Some(NameMatch::parse("ssh").unwrap()),
            ..LogQuery::default()
        };
        let normalized = NameMatch::parse("ssh.service").unwrap();
        let filter = EventFilter::new(&query).with_unit(Some(&normalized));
        assert!(filter.matches(&event(0, Some(0), 6, 0, 0)));
    }

    #[test]
    fn logs_message_regex_filter_equivalence() {
        let query = LogQuery {
            message: Some(Regex::new("fail|open").unwrap()),
            level: LogLevel::Debug,
            ..LogQuery::default()
        };
        let filter = EventFilter::new(&query);
        for message in 0..MESSAGES.len() {
            let event = event(0, None, 6, message, 0);
            assert_eq!(
                filter.matches(&event),
                record_filter(&query, &event.clone().into_record())
            );
        }
    }

    fn arb_query() -> impl Strategy<Value = LogQuery> {
        (
            proptest::option::of(prop_oneof![
                (0..SOURCES.len()).prop_map(|index| SOURCES[index].to_owned()),
                Just("*d".to_owned()),
                Just("c?on".to_owned()),
            ]),
            proptest::option::of(prop_oneof![
                (0..UNITS.len()).prop_map(|index| UNITS[index].to_owned()),
                Just("*.service".to_owned()),
                Just("user@*".to_owned()),
            ]),
            prop_oneof![
                Just(LogLevel::Critical),
                Just(LogLevel::Error),
                Just(LogLevel::Warning),
                Just(LogLevel::Info),
                Just(LogLevel::Debug),
            ],
            proptest::option::of(0_i64..100),
            proptest::option::of(0_i64..100),
            proptest::option::of(prop_oneof![Just("key"), Just("^session"), Just("fail")]),
        )
            .prop_map(|(source, unit, level, since, until, message)| LogQuery {
                source: source.map(|name| NameMatch::parse(&name).unwrap()),
                unit: unit.map(|name| NameMatch::parse(&name).unwrap()),
                level,
                since: since.map(Instant::from_unix_nanos),
                until: until.map(Instant::from_unix_nanos),
                message: message.map(|pattern| Regex::new(pattern).unwrap()),
                ..LogQuery::default()
            })
    }

    fn arb_event() -> impl Strategy<Value = LogEvent> {
        (
            0..SOURCES.len(),
            proptest::option::of(0..UNITS.len()),
            0_u8..=7,
            0..MESSAGES.len(),
            0_i64..100,
        )
            .prop_map(|(source, unit, priority, message, time)| {
                event(source, unit, priority, message, time)
            })
    }

    proptest! {
        #[test]
        fn filter_is_equivalent_to_conditions_on_the_record(
            query in arb_query(),
            event in arb_event(),
        ) {
            let filter = EventFilter::new(&query);
            prop_assert_eq!(
                filter.matches(&event),
                record_filter(&query, &event.clone().into_record())
            );
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
