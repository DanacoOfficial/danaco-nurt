// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Queries of the unified log store over a backend of recorded entries.

use std::sync::Mutex;

use nurt_contracts::error::{ErrorCode, ErrorData};
use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;
use nurt_syslog::cursor::Cursor;
use nurt_syslog::event::LogEvent;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogNotice, LogQuery, LogStore, NameMatch, NoticeCode};
use nurt_syslog::unified::backend::{
    BackendError, BackendItem, EntryRequest, Requirement, UnifiedBackend,
};
use nurt_syslog::unified::entry::UnifiedEntry;
use nurt_syslog::unified::level::EntryLevel;
use nurt_syslog::unified::predicate::Predicate;
use nurt_syslog::unified::store::UnifiedStore;
use regex::Regex;

/// Seconds of the recorded entries: 2025-11-21T10:00:00Z.
const BASE_SECONDS: i64 = 1_763_719_200;

const NANOS_PER_SECOND: i64 = 1_000_000_000;

type Items = Vec<Result<BackendItem, BackendError>>;

/// Backend over recorded items, newest first.
///
/// The mock returns every item regardless of the request, so that the
/// filters of the store are exercised, and records each request.
#[derive(Default)]
struct MockBackend {
    items: Items,
    refused: Option<BackendError>,
    requests: Mutex<Vec<EntryRequest>>,
}

impl UnifiedBackend for MockBackend {
    type Items<'a> = std::vec::IntoIter<Result<BackendItem, BackendError>>;

    fn items<'a>(&'a self, request: &EntryRequest) -> Result<Self::Items<'a>, BackendError> {
        self.requests.lock().unwrap().push(request.clone());
        match &self.refused {
            Some(error) => Err(error.clone()),
            None => Ok(self.items.clone().into_iter()),
        }
    }
}

fn at(second: i64) -> Instant {
    Instant::from_unix_nanos((BASE_SECONDS + second) * NANOS_PER_SECOND)
}

fn entry(
    second: i64,
    level: EntryLevel,
    process: &str,
    subsystem: &str,
    message: &str,
) -> UnifiedEntry {
    UnifiedEntry {
        time: at(second),
        level,
        message: message.to_owned(),
        process: process.to_owned(),
        pid: Some(411),
        tid: Some(9001),
        subsystem: Some(subsystem.to_owned()),
        ..UnifiedEntry::default()
    }
}

fn log(entry: UnifiedEntry) -> Result<BackendItem, BackendError> {
    Ok(BackendItem::Log(Box::new(entry)))
}

/// Six messages of two processes, newest first, one second apart, with an
/// activity and a loss record between them.
fn recorded() -> Items {
    vec![
        log(entry(
            60,
            EntryLevel::Fault,
            "exampled",
            "com.example.store",
            "database file is damaged",
        )),
        log(entry(
            50,
            EntryLevel::Error,
            "exampled",
            "com.example.network",
            "connection refused",
        )),
        Ok(BackendItem::Other),
        log(entry(
            40,
            EntryLevel::Notice,
            "exampled",
            "com.example.store",
            "started",
        )),
        Ok(BackendItem::Lost(3)),
        log(entry(
            30,
            EntryLevel::Info,
            "examplehelper",
            "com.example.helper",
            "cache hit",
        )),
        log(entry(
            20,
            EntryLevel::Debug,
            "examplehelper",
            "com.example.helper",
            "state 3",
        )),
        log(entry(
            10,
            EntryLevel::Error,
            "examplehelper",
            "com.example.helper",
            "lookup failed",
        )),
    ]
}

fn store(items: Items) -> UnifiedStore<MockBackend> {
    UnifiedStore::new(
        MockBackend {
            items,
            ..MockBackend::default()
        },
        LogLimits::default(),
    )
}

fn refused(error: BackendError) -> UnifiedStore<MockBackend> {
    UnifiedStore::new(
        MockBackend {
            refused: Some(error),
            ..MockBackend::default()
        },
        LogLimits::default(),
    )
}

fn name(text: &str) -> Option<NameMatch> {
    Some(NameMatch::parse(text).unwrap())
}

fn run(
    store: &UnifiedStore<MockBackend>,
    query: &LogQuery,
) -> (Vec<LogEvent>, Vec<LogNotice>, Option<Cursor>) {
    let mut stream = store.query(query).unwrap();
    let events = stream.by_ref().map(Result::unwrap).collect();
    (events, stream.notices(), stream.next_cursor().cloned())
}

fn messages(events: &[LogEvent]) -> Vec<&str> {
    events.iter().map(|event| event.message.as_str()).collect()
}

fn query_error(store: &UnifiedStore<MockBackend>, query: &LogQuery) -> ErrorData {
    store.query(query).unwrap_err().into_data()
}

fn notice(notices: &[LogNotice], code: NoticeCode) -> Option<&LogNotice> {
    notices.iter().find(|notice| notice.code == code)
}

fn last_request(store: &UnifiedStore<MockBackend>) -> EntryRequest {
    store_backend_requests(store).pop().unwrap()
}

fn store_backend_requests(store: &UnifiedStore<MockBackend>) -> Vec<EntryRequest> {
    store.backend().requests.lock().unwrap().clone()
}

#[test]
fn unified_query_returns_messages_newest_first() {
    let store = store(recorded());
    let (events, notices, cursor) = run(&store, &LogQuery::default());
    assert_eq!(
        messages(&events),
        [
            "database file is damaged",
            "connection refused",
            "started",
            "cache hit",
            "lookup failed"
        ]
    );
    assert_eq!(cursor, None);
    assert!(notice(&notices, NoticeCode::LimitReached).is_none());
    let request = last_request(&store);
    assert_eq!(request.until, None);
    assert_eq!(
        request.predicate,
        Predicate::new(None, None, LogLevel::Info)
    );
}

#[test]
fn unified_events_lost_notable() {
    let mut items = recorded();
    items.push(Ok(BackendItem::Lost(2)));
    let store = store(items);
    let query = LogQuery {
        level: LogLevel::Debug,
        ..LogQuery::default()
    };
    let (events, notices, _) = run(&store, &query);
    assert_eq!(events.len(), 6);
    let lost = notice(&notices, NoticeCode::EventsLost).unwrap();
    assert_eq!(lost.count, Some(5));
    assert!(lost.to_notable().is_none());
}

#[test]
fn unified_privilege_error_fix() {
    let error = query_error(
        &refused(BackendError::AccessDenied(Requirement::Root)),
        &LogQuery::default(),
    );
    assert_eq!(error.code, ErrorCode::Privilege);
    assert_eq!(error.details["required"], "root");
    assert!(error.message.contains("sudo"), "{}", error.message);

    let error = query_error(
        &refused(BackendError::AccessDenied(Requirement::FullDiskAccess)),
        &LogQuery::default(),
    );
    assert_eq!(error.code, ErrorCode::Privilege);
    assert_eq!(error.details["required"], "full_disk_access");
    assert!(
        error.message.contains("Full Disk Access"),
        "{}",
        error.message
    );
}

#[test]
fn unified_backend_failures_map_to_error_codes() {
    let unavailable = query_error(
        &refused(BackendError::Unavailable("no log store".to_owned())),
        &LogQuery::default(),
    );
    assert_eq!(unavailable.code, ErrorCode::NotFound);
    assert!(unavailable.message.contains("no log store"));

    let failed = query_error(
        &refused(BackendError::Failed("enumeration failed".to_owned())),
        &LogQuery::default(),
    );
    assert_eq!(failed.code, ErrorCode::StageFailed);

    let mut items = recorded();
    items.insert(1, Err(BackendError::Failed("entry unreadable".to_owned())));
    let store = store(items);
    let query = LogQuery::default();
    let mut stream = store.query(&query).unwrap();
    assert!(stream.next().unwrap().is_ok());
    let error = stream.next().unwrap().unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(stream.next().is_none());
}

#[test]
fn unified_since_stops_at_the_first_older_message() {
    let mut items = recorded();
    items.insert(
        6,
        log(entry(
            55,
            EntryLevel::Error,
            "exampled",
            "com.example.store",
            "out of order",
        )),
    );
    let store = store(items);
    let query = LogQuery {
        since: Some(at(35)),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(
        messages(&events),
        ["database file is damaged", "connection refused", "started"]
    );
}

#[test]
fn unified_until_is_exclusive_and_sent_to_the_backend() {
    let store = store(recorded());
    let query = LogQuery {
        until: Some(at(50)),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(messages(&events), ["started", "cache hit", "lookup failed"]);
    assert_eq!(last_request(&store).until, Some(at(50)));
}

#[test]
fn unified_predicate_carries_literal_names_only() {
    let store = store(recorded());
    let query = LogQuery {
        unit: name("com.example.store"),
        source: name("exampled"),
        level: LogLevel::Error,
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(messages(&events), ["database file is damaged"]);
    assert_eq!(
        last_request(&store).predicate,
        Predicate::new(Some("com.example.store"), Some("exampled"), LogLevel::Error)
    );

    let pattern = LogQuery {
        unit: name("com.example.*"),
        source: name("example*"),
        level: LogLevel::Debug,
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &pattern);
    assert_eq!(events.len(), 6);
    assert_eq!(last_request(&store).predicate, None);
}

#[test]
fn unified_final_filter_applies_to_every_parameter() {
    let store = store(recorded());
    let unit = LogQuery {
        unit: name("com.example.h*"),
        level: LogLevel::Debug,
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &unit);
    assert_eq!(messages(&events), ["cache hit", "state 3", "lookup failed"]);

    let message = LogQuery {
        message: Some(Regex::new("fail|refused").unwrap()),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &message);
    assert_eq!(messages(&events), ["connection refused", "lookup failed"]);

    let warning = LogQuery {
        level: LogLevel::Warning,
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &warning);
    assert_eq!(
        messages(&events),
        [
            "database file is damaged",
            "connection refused",
            "lookup failed"
        ]
    );

    let critical = LogQuery {
        level: LogLevel::Critical,
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &critical);
    assert_eq!(messages(&events), ["database file is damaged"]);
}

#[test]
fn logs_limit_reached_notable() {
    let store = store(recorded());
    let limited = LogQuery {
        limit: 2,
        ..LogQuery::default()
    };
    let (events, notices, cursor) = run(&store, &limited);
    assert_eq!(
        messages(&events),
        ["database file is damaged", "connection refused"]
    );
    assert_eq!(
        notice(&notices, NoticeCode::LimitReached).unwrap().count,
        Some(2)
    );
    assert_eq!(cursor.as_ref(), Some(&events[1].cursor));
}

#[test]
fn unified_cursor_continues_after_its_entry() {
    let store = store(recorded());
    let first = LogQuery {
        limit: 2,
        ..LogQuery::default()
    };
    let (_, _, cursor) = run(&store, &first);
    let next = LogQuery {
        cursor: cursor.clone(),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &next);
    assert_eq!(messages(&events), ["started", "cache hit", "lookup failed"]);
    let Some(Cursor::Unified { nanos, .. }) = cursor else {
        panic!("expected a unified log cursor");
    };
    assert_eq!(
        last_request(&store).until,
        Some(Instant::from_unix_nanos(nanos + 1))
    );

    let bounded = LogQuery {
        until: Some(at(45)),
        cursor: Some(events[0].cursor.clone()),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &bounded);
    assert_eq!(messages(&events), ["cache hit", "lookup failed"]);
    assert_eq!(
        last_request(&store).until,
        Some(Instant::from_unix_nanos(at(40).as_unix_nanos() + 1))
    );
}

#[test]
fn unified_cursor_skips_entries_of_its_time_up_to_its_entry() {
    let items = vec![
        log(entry(
            30,
            EntryLevel::Error,
            "exampled",
            "com.example.store",
            "first",
        )),
        log(entry(
            30,
            EntryLevel::Error,
            "exampled",
            "com.example.store",
            "second",
        )),
        log(entry(
            30,
            EntryLevel::Error,
            "exampled",
            "com.example.store",
            "third",
        )),
        log(entry(
            20,
            EntryLevel::Error,
            "exampled",
            "com.example.store",
            "older",
        )),
    ];
    let store = store(items);
    let second = entry(
        30,
        EntryLevel::Error,
        "exampled",
        "com.example.store",
        "second",
    )
    .cursor();
    let query = LogQuery {
        cursor: Some(second),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(messages(&events), ["third", "older"]);

    let missing = entry(
        30,
        EntryLevel::Error,
        "exampled",
        "com.example.store",
        "removed",
    )
    .cursor();
    let query = LogQuery {
        cursor: Some(missing),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(messages(&events), ["older"]);

    let newer = entry(
        40,
        EntryLevel::Error,
        "exampled",
        "com.example.store",
        "gone",
    )
    .cursor();
    let query = LogQuery {
        cursor: Some(newer),
        ..LogQuery::default()
    };
    let (events, _, _) = run(&store, &query);
    assert_eq!(messages(&events), ["first", "second", "third", "older"]);
}

#[test]
fn unified_foreign_cursor_is_rejected() {
    let store = store(recorded());
    let query = LogQuery {
        cursor: Some(Cursor::EventLog {
            channel: "System".to_owned(),
            record_id: 4,
        }),
        ..LogQuery::default()
    };
    let error = query_error(&store, &query);
    assert_eq!(error.code, ErrorCode::InvalidValue);
    assert_eq!(error.found.as_deref(), Some("v1:w:System:4"));
    assert!(store_backend_requests(&store).is_empty());
}

#[test]
fn unified_limits_cut_long_messages() {
    let long = "x".repeat(64);
    let store = UnifiedStore::new(
        MockBackend {
            items: vec![log(entry(
                1,
                EntryLevel::Error,
                "exampled",
                "com.example.store",
                &long,
            ))],
            ..MockBackend::default()
        },
        LogLimits {
            max_message_bytes: 16,
            ..LogLimits::default()
        },
    );
    let (events, _, _) = run(&store, &LogQuery::default());
    assert!(events[0].message.len() <= 16);
    assert!(events[0].flags.truncated);
}
