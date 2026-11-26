// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Queries of the event log store over an event source of recorded events.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;
use nurt_syslog::cursor::Cursor;
use nurt_syslog::event::LogEvent;
use nurt_syslog::eventlog::source::{EventSource, MessagePart, SourceError};
use nurt_syslog::eventlog::store::EventLogStore;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogNotice, LogQuery, LogStore, NameMatch, NoticeCode};
use regex::Regex;

/// Start of the recorded events: 2025-11-21T10:00:00Z.
const BASE_SECONDS: i64 = 1_763_719_200;

/// Recorded event: its XML and the record identifier the mock filters by.
#[derive(Debug, Clone)]
struct Recorded {
    record_id: u64,
    xml: String,
    provider: String,
}

/// Channel and XPath of every query the mock received.
type QueryLog = Arc<Mutex<Vec<(String, String)>>>;

/// Event source over recorded channels, newest event first.
///
/// The mock evaluates only the conditions on `EventRecordID`; every other
/// condition of the XPath query is left to the final filter of the store.
#[derive(Default)]
struct MockSource {
    channels: BTreeMap<String, Vec<Recorded>>,
    denied: Vec<String>,
    failure: Option<SourceError>,
    messages: BTreeMap<String, String>,
    tasks: BTreeMap<String, String>,
    queries: QueryLog,
}

impl MockSource {
    fn channel(mut self, name: &str, events: Vec<Recorded>) -> Self {
        self.channels.insert(name.to_owned(), events);
        self
    }
}

fn record_condition(xpath: &str, marker: &str) -> Option<u64> {
    let start = xpath.find(marker)? + marker.len();
    let digits: String = xpath[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

impl EventSource for MockSource {
    type Event = Recorded;
    type Events<'a> = std::vec::IntoIter<Result<Recorded, SourceError>>;

    fn channels(&self) -> Result<Vec<String>, SourceError> {
        Ok(self.channels.keys().cloned().collect())
    }

    fn query<'a>(&'a self, channel: &str, xpath: &str) -> Result<Self::Events<'a>, SourceError> {
        self.queries
            .lock()
            .unwrap()
            .push((channel.to_owned(), xpath.to_owned()));
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        if self
            .denied
            .iter()
            .any(|name| name.eq_ignore_ascii_case(channel))
        {
            return Err(SourceError::AccessDenied);
        }
        let events = self
            .channels
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(channel))
            .map(|(_, events)| events)
            .ok_or(SourceError::ChannelNotFound)?;
        let exact = record_condition(xpath, "EventRecordID=");
        let before = record_condition(xpath, "EventRecordID<");
        let selected: Vec<Result<Recorded, SourceError>> = events
            .iter()
            .filter(|event| exact.is_none_or(|id| event.record_id == id))
            .filter(|event| before.is_none_or(|id| event.record_id < id))
            .cloned()
            .map(Ok)
            .collect();
        Ok(selected.into_iter())
    }

    fn render_xml(&self, event: &Recorded) -> Result<String, SourceError> {
        Ok(event.xml.clone())
    }

    fn format(&self, event: &Recorded, provider: &str, part: MessagePart) -> Option<String> {
        assert_eq!(provider, event.provider);
        match part {
            MessagePart::Event => self.messages.get(provider).cloned(),
            MessagePart::Task => self.tasks.get(provider).cloned(),
        }
    }
}

/// Event of `channel` created `seconds` after [`BASE_SECONDS`].
fn event(channel: &str, record_id: u64, seconds: i64, level: u8, provider: &str) -> Recorded {
    let time = Instant::from_unix_seconds(BASE_SECONDS + seconds).unwrap();
    let xml = format!(
        "<Event><System><Provider Name='{provider}'/><EventID>100</EventID><Level>{level}</Level>\
         <TimeCreated SystemTime='{time}'/><EventRecordID>{record_id}</EventRecordID>\
         <Channel>{channel}</Channel><Computer>host.example.com</Computer></System>\
         <EventData><Data Name='step'>{record_id}</Data></EventData></Event>"
    );
    Recorded {
        record_id,
        xml,
        provider: provider.to_owned(),
    }
}

/// `System` with records 30, 20, 10 at 50, 30, 10 s and `Application` with
/// records 7, 6 at 40, 20 s.
fn two_channels() -> MockSource {
    MockSource::default()
        .channel(
            "System",
            vec![
                event("System", 30, 50, 4, "Example-A"),
                event("System", 20, 30, 2, "Example-A"),
                event("System", 10, 10, 4, "Example-B"),
            ],
        )
        .channel(
            "Application",
            vec![
                event("Application", 7, 40, 3, "Example-B"),
                event("Application", 6, 20, 4, "Example-A"),
            ],
        )
}

fn open_store(source: MockSource) -> (EventLogStore<MockSource>, QueryLog) {
    let queries = Arc::clone(&source.queries);
    (EventLogStore::new(source, LogLimits::default()), queries)
}

fn run(
    store: &EventLogStore<MockSource>,
    query: &LogQuery,
) -> (Vec<LogEvent>, Vec<LogNotice>, Option<Cursor>) {
    let mut stream = store.query(query).unwrap();
    let events = stream.by_ref().map(Result::unwrap).collect();
    let cursor = stream.next_cursor().cloned();
    (events, stream.notices(), cursor)
}

fn query_error(store: &EventLogStore<MockSource>, query: &LogQuery) -> Error {
    store.query(query).expect_err("query fails")
}

fn records(events: &[LogEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| event.cursor.to_string())
        .collect()
}

fn name(text: &str) -> Option<NameMatch> {
    Some(NameMatch::parse(text).unwrap())
}

fn notice(notices: &[LogNotice], code: NoticeCode) -> Option<&LogNotice> {
    notices.iter().find(|notice| notice.code == code)
}

fn taken(queries: &QueryLog) -> Vec<(String, String)> {
    std::mem::take(&mut *queries.lock().unwrap())
}

#[test]
fn eventlog_reverse_order_merge_channels() {
    let (store, queries) = open_store(two_channels());
    let (events, _, _) = run(&store, &LogQuery::default());
    assert_eq!(
        records(&events),
        [
            "v1:w:System:30",
            "v1:w:Application:7",
            "v1:w:System:20",
            "v1:w:Application:6",
            "v1:w:System:10"
        ]
    );
    let channels: Vec<String> = taken(&queries)
        .into_iter()
        .map(|(channel, _)| channel)
        .collect();
    assert_eq!(channels, ["System", "Application"]);
}

#[test]
fn eventlog_query_sends_level_and_provider_to_the_source() {
    let (store, queries) = open_store(two_channels());
    let query = LogQuery {
        source: name("Example-A"),
        level: LogLevel::Warning,
        ..LogQuery::default()
    };
    drop(run(&store, &query));
    let expected = "*[System[(Level=1 or Level=2 or Level=3) and Provider[@Name='Example-A']]]";
    for (_, xpath) in taken(&queries) {
        assert_eq!(xpath, expected);
    }
    let pattern = LogQuery {
        source: name("Example-*"),
        level: LogLevel::Debug,
        ..LogQuery::default()
    };
    assert_eq!(run(&store, &pattern).0.len(), 5);
    for (_, xpath) in taken(&queries) {
        assert_eq!(xpath, "*");
    }
}

#[test]
fn eventlog_final_filter_applies_to_every_parameter() {
    let (store, _) = open_store(two_channels());
    let exact = LogQuery {
        unit: name("System"),
        source: name("Example-A"),
        level: LogLevel::Error,
        ..LogQuery::default()
    };
    assert_eq!(records(&run(&store, &exact).0), ["v1:w:System:20"]);
    let warnings = LogQuery {
        level: LogLevel::Warning,
        ..LogQuery::default()
    };
    assert_eq!(
        records(&run(&store, &warnings).0),
        ["v1:w:Application:7", "v1:w:System:20"]
    );
    let window = LogQuery {
        since: Some(Instant::from_unix_seconds(BASE_SECONDS + 20).unwrap()),
        until: Some(Instant::from_unix_seconds(BASE_SECONDS + 40).unwrap()),
        ..LogQuery::default()
    };
    assert_eq!(
        records(&run(&store, &window).0),
        ["v1:w:System:20", "v1:w:Application:6"]
    );
    let message = LogQuery {
        message: Some(Regex::new("step=1\\d").unwrap()),
        ..LogQuery::default()
    };
    assert_eq!(records(&run(&store, &message).0), ["v1:w:System:10"]);
}

#[test]
fn eventlog_unformatted_raw_message() {
    let mut source = two_channels();
    source.messages.insert(
        "Example-A".to_owned(),
        "Example-A finished step.".to_owned(),
    );
    source
        .tasks
        .insert("Example-A".to_owned(), "Maintenance".to_owned());
    let (store, _) = open_store(source);
    let (events, notices, _) = run(&store, &LogQuery::default());
    let formatted = &events[0];
    assert_eq!(formatted.message, "Example-A finished step.");
    assert_eq!(formatted.category.as_deref(), Some("Maintenance"));
    assert!(!formatted.flags.unformatted);
    let raw = &events[1];
    assert_eq!(raw.source, "Example-B");
    assert_eq!(raw.message, "step=7");
    assert!(raw.flags.unformatted);
    let unformatted = notice(&notices, NoticeCode::Unformatted).unwrap();
    assert_eq!(unformatted.count, Some(2));
}

#[test]
fn eventlog_unknown_channel_candidates() {
    let (store, _) = open_store(two_channels());
    let error = query_error(
        &store,
        &LogQuery {
            unit: name("Sytem"),
            ..LogQuery::default()
        },
    );
    assert_eq!(error.code, ErrorCode::UnknownValue);
    assert_eq!(error.found.as_deref(), Some("Sytem"));
    assert_eq!(error.expected.as_ref().unwrap()[0], "System");
    let pattern = query_error(
        &store,
        &LogQuery {
            unit: name("Microsoft-*"),
            ..LogQuery::default()
        },
    );
    assert_eq!(pattern.code, ErrorCode::UnknownValue);
}

#[test]
fn eventlog_unit_pattern_ignores_letter_case() {
    let source = two_channels().channel(
        "Microsoft-Windows-Example/Operational",
        vec![event(
            "Microsoft-Windows-Example/Operational",
            3,
            60,
            4,
            "Example-C",
        )],
    );
    let (store, _) = open_store(source);
    let query = LogQuery {
        unit: name("microsoft-*"),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(
        records(&events),
        ["v1:w:Microsoft-Windows-Example/Operational:3"]
    );
    let named = LogQuery {
        unit: name("system"),
        ..LogQuery::default()
    };
    assert_eq!(run(&store, &named).0.len(), 3);
}

#[test]
fn eventlog_security_privilege() {
    let mut source =
        two_channels().channel("Security", vec![event("Security", 1, 1, 0, "Example-A")]);
    source.denied.push("Security".to_owned());
    let (store, _) = open_store(source);
    let error = query_error(
        &store,
        &LogQuery {
            unit: name("Security"),
            ..LogQuery::default()
        },
    );
    assert_eq!(error.code, ErrorCode::Privilege);
    assert_eq!(error.details["required"], "administrator");
    assert!(error.message.contains("elevated prompt"));
}

#[test]
fn eventlog_default_channel_denied_is_skipped() {
    let mut source = two_channels();
    source.denied.push("Application".to_owned());
    let (store, _) = open_store(source);
    let (events, notices, _) = run(&store, &LogQuery::default());
    assert_eq!(events.len(), 3);
    let denied = notice(&notices, NoticeCode::PermissionDeniedSkipped).unwrap();
    assert_eq!(denied.path.as_deref(), Some("Application"));
    assert!(denied.to_notable().is_some());

    let mut closed = two_channels();
    closed.denied = vec!["System".to_owned(), "Application".to_owned()];
    let (closed_store, _) = open_store(closed);
    let error = query_error(&closed_store, &LogQuery::default());
    assert_eq!(error.code, ErrorCode::Privilege);
}

#[test]
fn eventlog_cursor_record_id() {
    let (store, queries) = open_store(two_channels());
    let first = LogQuery {
        limit: 2,
        ..LogQuery::default()
    };
    let (page, _, cursor) = run(&store, &first);
    assert_eq!(records(&page), ["v1:w:System:30", "v1:w:Application:7"]);
    let cursor = cursor.unwrap();
    assert_eq!(cursor.to_string(), "v1:w:Application:7");
    taken(&queries);

    let second = LogQuery {
        limit: 2,
        cursor: Some(cursor),
        ..LogQuery::default()
    };
    let (page, notices, cursor) = run(&store, &second);
    assert_eq!(records(&page), ["v1:w:System:20", "v1:w:Application:6"]);
    assert!(notice(&notices, NoticeCode::CursorApproximate).is_none());
    let sent = taken(&queries);
    assert_eq!(
        sent[0],
        (
            "Application".to_owned(),
            "*[System[EventRecordID=7]]".to_owned()
        )
    );
    assert_eq!(sent[1].0, "System");
    assert!(sent[1].1.contains("@SystemTime<'2025-11-21T10:00:40.001Z'"));
    assert_eq!(sent[2].0, "Application");
    assert!(sent[2].1.ends_with("EventRecordID<7]]"));

    let third = LogQuery {
        limit: 2,
        cursor,
        ..LogQuery::default()
    };
    let (page, notices, cursor) = run(&store, &third);
    assert_eq!(records(&page), ["v1:w:System:10"]);
    assert_eq!(cursor, None);
    assert!(notice(&notices, NoticeCode::LimitReached).is_none());
}

#[test]
fn eventlog_cursor_of_removed_record_is_approximate() {
    let (store, _) = open_store(two_channels());
    let query = LogQuery {
        cursor: Some(Cursor::EventLog {
            channel: "System".to_owned(),
            record_id: 25,
        }),
        ..LogQuery::default()
    };
    let (events, notices, _) = run(&store, &query);
    let approximate = notice(&notices, NoticeCode::CursorApproximate).unwrap();
    assert_eq!(approximate.path.as_deref(), Some("System"));
    assert_eq!(
        records(&events),
        [
            "v1:w:Application:7",
            "v1:w:System:20",
            "v1:w:Application:6",
            "v1:w:System:10"
        ]
    );
}

#[test]
fn eventlog_foreign_cursor_is_rejected() {
    let (store, _) = open_store(two_channels());
    let query = LogQuery {
        cursor: Some(Cursor::Unified {
            nanos: 1,
            pid: 2,
            tid: 3,
            digest: 4,
        }),
        ..LogQuery::default()
    };
    assert_eq!(query_error(&store, &query).code, ErrorCode::InvalidValue);
}

#[test]
fn logs_limit_reached_notable() {
    let (store, _) = open_store(two_channels());
    let limited = LogQuery {
        limit: 2,
        ..LogQuery::default()
    };
    let (events, notices, cursor) = run(&store, &limited);
    assert_eq!(events.len(), 2);
    assert_eq!(
        notice(&notices, NoticeCode::LimitReached).unwrap().count,
        Some(2)
    );
    assert!(cursor.is_some());
    let (events, notices, cursor) = run(&store, &LogQuery::default());
    assert_eq!(events.len(), 5);
    assert!(notice(&notices, NoticeCode::LimitReached).is_none());
    assert_eq!(cursor, None);
}

#[test]
fn eventlog_invalid_event_is_skipped_with_notice() {
    let mut broken = event("System", 15, 20, 4, "Example-A");
    broken.xml = "<Event><System>".to_owned();
    let source = MockSource::default().channel(
        "System",
        vec![event("System", 16, 30, 4, "Example-A"), broken],
    );
    let (store, _) = open_store(source);
    let query = LogQuery {
        unit: name("System"),
        ..LogQuery::default()
    };
    let (events, notices, _) = run(&store, &query);
    assert_eq!(records(&events), ["v1:w:System:16"]);
    let skipped = notice(&notices, NoticeCode::FileSkipped).unwrap();
    assert_eq!(skipped.detail.as_deref(), Some("corrupt"));
    assert_eq!(skipped.path.as_deref(), Some("System"));
}

#[test]
fn eventlog_source_failures_map_to_error_codes() {
    let cases = [
        (SourceError::ServiceUnavailable, ErrorCode::NotFound),
        (SourceError::InvalidQuery, ErrorCode::StageFailed),
        (SourceError::ChannelNotFound, ErrorCode::UnknownValue),
        (
            SourceError::System {
                code: 1_450,
                message: "insufficient system resources".to_owned(),
            },
            ErrorCode::StageFailed,
        ),
    ];
    for (failure, code) in cases {
        let mut source = two_channels();
        source.failure = Some(failure);
        let (store, _) = open_store(source);
        let query = LogQuery {
            unit: name("System"),
            ..LogQuery::default()
        };
        assert_eq!(query_error(&store, &query).code, code);
    }
    let mut invalid = two_channels();
    invalid.failure = Some(SourceError::InvalidQuery);
    let (store, _) = open_store(invalid);
    let error = query_error(&store, &LogQuery::default());
    assert_eq!(error.details["xpath"], "*[System[(Level<5 or Level>5)]]");
}

#[test]
fn eventlog_default_channel_missing_is_skipped() {
    let source =
        MockSource::default().channel("System", vec![event("System", 1, 1, 4, "Example-A")]);
    let (store, _) = open_store(source);
    let (events, _, _) = run(&store, &LogQuery::default());
    assert_eq!(records(&events), ["v1:w:System:1"]);
}

#[test]
fn eventlog_records_convert_to_log_event_records() {
    let (store, _) = open_store(two_channels());
    let (events, _, _) = run(&store, &LogQuery::default());
    let record = events[0].to_record();
    assert_eq!(record.level, LogLevel::Info);
    assert_eq!(record.unit.as_deref(), Some("System"));
    assert_eq!(record.cursor.as_deref(), Some("v1:w:System:30"));
}

#[test]
fn eventlog_cursor_channel_is_compared_without_regard_to_letter_case() {
    let (store, queries) = open_store(two_channels());
    let query = LogQuery {
        cursor: Some(Cursor::EventLog {
            channel: "system".to_owned(),
            record_id: 20,
        }),
        ..LogQuery::default()
    };
    let (events, notices, _) = run(&store, &query);
    assert_eq!(records(&events), ["v1:w:Application:6", "v1:w:System:10"]);
    assert!(notice(&notices, NoticeCode::CursorApproximate).is_none());
    let sent = taken(&queries);
    let system: Vec<&String> = sent
        .iter()
        .filter(|(channel, xpath)| channel == "System" && xpath.contains("EventRecordID<20"))
        .map(|(_, xpath)| xpath)
        .collect();
    assert_eq!(system.len(), 1);
}

#[test]
fn eventlog_pattern_unit_skips_denied_channels_with_a_notice() {
    let mut source =
        two_channels().channel("Security", vec![event("Security", 1, 1, 0, "Example-A")]);
    source.denied.push("Security".to_owned());
    let (store, _) = open_store(source);
    let query = LogQuery {
        unit: name("*"),
        ..LogQuery::default()
    };
    let (events, notices, _) = run(&store, &query);
    assert_eq!(events.len(), 5);
    let denied = notice(&notices, NoticeCode::PermissionDeniedSkipped).unwrap();
    assert_eq!(denied.path.as_deref(), Some("Security"));
    assert_eq!(denied.count, Some(1));
}

#[test]
fn eventlog_events_of_equal_time_in_two_channels_keep_a_stable_order() {
    let source = MockSource::default()
        .channel("System", vec![event("System", 2, 10, 4, "Example-A")])
        .channel(
            "Application",
            vec![event("Application", 9, 10, 4, "Example-A")],
        );
    let (store, _) = open_store(source);
    let (first, _, _) = run(&store, &LogQuery::default());
    let (second, _, _) = run(&store, &LogQuery::default());
    assert_eq!(records(&first), records(&second));
    assert_eq!(first.len(), 2);
}

#[test]
fn eventlog_pattern_unit_reads_only_the_matching_channels() {
    let (store, _) = open_store(two_channels());
    let query = LogQuery {
        unit: name("App*"),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(
        records(&events),
        ["v1:w:Application:7", "v1:w:Application:6"]
    );
}

#[test]
fn eventlog_limits_cut_long_messages_of_formatted_events() {
    let mut source = two_channels();
    source
        .messages
        .insert("Example-A".to_owned(), "m".repeat(200));
    let limits = LogLimits {
        max_message_bytes: 16,
        ..LogLimits::default()
    };
    let store = EventLogStore::new(source, limits);
    let (events, _, _) = run(&store, &LogQuery::default());
    let formatted = events
        .iter()
        .find(|event| event.source == "Example-A")
        .unwrap();
    assert_eq!(formatted.message.len(), 16);
    assert!(formatted.flags.truncated);
}
