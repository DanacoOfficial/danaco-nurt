// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Queries of the journal store over files produced by the test writer.

#[path = "support/journal_writer.rs"]
mod journal_writer;

use std::fs;
use std::str::FromStr;

use journal_writer::{
    Compression, Entry, FLAG_COMPACT, FLAG_KEYED_HASH, MACHINE_ID, Options, State, Writer,
    sample_entries, system_root,
};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::LogLevel;
use nurt_contracts::stage::{CancelReason, CancelToken};
use nurt_contracts::value::Instant;
use nurt_syslog::cursor::Cursor;
use nurt_syslog::event::{LogEvent, StoreKind};
use nurt_syslog::journald::{JournaldStore, query_bytes};
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogNotice, LogQuery, LogStore, NameMatch, NoticeCode};
use regex::Regex;

/// Reception time of the first sample entry, in microseconds.
const START: u64 = 1_700_000_000_000_000;
const NANOS_PER_MICRO: u64 = 1_000;
const STEP: u64 = 1_000;
const LARGE_LIMIT: u32 = 10_000;
const HEADER_INCOMPATIBLE: std::ops::Range<usize> = 12..16;
const DATA_PAYLOAD_REGULAR: usize = 64;

type Summary = (String, String, u8, Option<String>, Vec<(String, String)>);

fn query(level: LogLevel) -> LogQuery {
    LogQuery {
        level,
        limit: LARGE_LIMIT,
        ..LogQuery::default()
    }
}

fn unit(name: &str) -> LogQuery {
    LogQuery {
        unit: Some(NameMatch::parse(name).unwrap()),
        ..query(LogLevel::Debug)
    }
}

fn source(name: &str) -> LogQuery {
    LogQuery {
        source: Some(NameMatch::parse(name).unwrap()),
        ..query(LogLevel::Debug)
    }
}

fn at(micros: u64) -> Instant {
    Instant::from_unix_nanos(i64::try_from(micros * NANOS_PER_MICRO).unwrap())
}

fn read_with(
    bytes: Vec<u8>,
    query: &LogQuery,
    limits: &LogLimits,
) -> (Vec<LogEvent>, Vec<LogNotice>) {
    let mut stream = query_bytes(bytes, query, limits).unwrap();
    let events = stream.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    (events, stream.notices())
}

fn read(bytes: Vec<u8>, query: &LogQuery) -> (Vec<LogEvent>, Vec<LogNotice>) {
    read_with(bytes, query, &LogLimits::default())
}

fn read_store(store: &JournaldStore, query: &LogQuery) -> (Vec<LogEvent>, Vec<LogNotice>) {
    let mut stream = store.query(query).unwrap();
    let events = stream.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    (events, stream.notices())
}

fn store_error(store: &JournaldStore, query: &LogQuery) -> Error {
    match store.query(query) {
        Ok(_) => panic!("the query was expected to fail"),
        Err(error) => error,
    }
}

fn messages(events: &[LogEvent]) -> Vec<String> {
    events.iter().map(|event| event.message.clone()).collect()
}

fn expected(range: std::ops::Range<u64>) -> Vec<String> {
    range.rev().map(|index| format!("event {index}")).collect()
}

fn summary(events: &[LogEvent]) -> Vec<Summary> {
    events
        .iter()
        .map(|event| {
            (
                event.message.clone(),
                event.source.clone(),
                event.priority,
                event.unit.clone(),
                event.fields.clone(),
            )
        })
        .collect()
}

fn notice(notices: &[LogNotice], code: NoticeCode) -> &LogNotice {
    notices
        .iter()
        .find(|notice| notice.code == code)
        .unwrap_or_else(|| panic!("no notice {code:?} in {notices:?}"))
}

fn has(notices: &[LogNotice], code: NoticeCode) -> bool {
    notices.iter().any(|notice| notice.code == code)
}

fn sample(count: u64) -> Vec<u8> {
    Writer::build(Options::default(), &sample_entries(count, START))
}

fn long_unit() -> String {
    format!("app-{}.service", "a".repeat(300))
}

/// Entries whose messages and one unit name are long enough to be stored
/// compressed.
fn compressible_entries(count: u64) -> Vec<Entry> {
    let long_unit = long_unit();
    (0..count)
        .map(|index| {
            let unit = if index.is_multiple_of(3) {
                long_unit.as_str()
            } else {
                "cron.service"
            };
            let message = format!("event {index} {}", "lorem ipsum ".repeat(20));
            let priority = (index % 8).to_string();
            Entry::new(
                START + index * STEP,
                &[
                    ("MESSAGE", message.as_str()),
                    ("PRIORITY", priority.as_str()),
                    ("SYSLOG_IDENTIFIER", "app"),
                    ("_SYSTEMD_UNIT", unit),
                ],
            )
        })
        .collect()
}

fn compressible(options: Options) -> Vec<u8> {
    let options = Options {
        compress_threshold: 64,
        ..options
    };
    Writer::build(options, &compressible_entries(24))
}

fn incompatible_flags(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[HEADER_INCOMPATIBLE].try_into().unwrap())
}

fn machine_name() -> String {
    MACHINE_ID
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn logs_level_maps_priority() {
    let bytes = sample(16);
    let (events, _) = read(bytes.clone(), &query(LogLevel::Error));
    assert_eq!(
        messages(&events),
        [
            "event 11", "event 10", "event 9", "event 8", "event 3", "event 2", "event 1",
            "event 0"
        ]
    );
    assert!(events.iter().all(|event| event.priority <= 3));
    let counts: Vec<usize> = [
        LogLevel::Critical,
        LogLevel::Error,
        LogLevel::Warning,
        LogLevel::Info,
        LogLevel::Debug,
    ]
    .into_iter()
    .map(|level| read(bytes.clone(), &query(level)).0.len())
    .collect();
    assert_eq!(counts, [6, 8, 10, 14, 16]);
    assert!(
        events
            .iter()
            .all(|event| event.store == StoreKind::Journald)
    );
}

#[test]
fn logs_unit_hash_lookup_no_decompress() {
    let options = Options {
        compression: Compression::Zstd,
        compress_threshold: 64,
        ..Options::default()
    };
    let cron_message = format!("cron {}", "payload ".repeat(30));
    let mut writer = Writer::new(options);
    writer.add(&Entry::new(
        START,
        &[("MESSAGE", "ssh one"), ("_SYSTEMD_UNIT", "ssh.service")],
    ));
    writer.add(&Entry::new(
        START + STEP,
        &[
            ("MESSAGE", cron_message.as_str()),
            ("_SYSTEMD_UNIT", "cron.service"),
        ],
    ));
    writer.add(&Entry::new(
        START + 2 * STEP,
        &[("MESSAGE", "ssh two"), ("_SYSTEMD_UNIT", "ssh.service")],
    ));
    let damaged = usize::try_from(
        writer
            .data_offset(format!("MESSAGE={cron_message}").as_bytes())
            .unwrap(),
    )
    .unwrap();
    let mut bytes = writer.finish();
    bytes[damaged + DATA_PAYLOAD_REGULAR..damaged + DATA_PAYLOAD_REGULAR + 4].fill(0);

    let (events, notices) = read(bytes.clone(), &unit("ssh"));
    assert_eq!(messages(&events), ["ssh two", "ssh one"]);
    assert!(notices.is_empty(), "{notices:?}");

    let (events, notices) = read(bytes, &query(LogLevel::Debug));
    assert_eq!(messages(&events), ["ssh two"]);
    assert_eq!(
        notice(&notices, NoticeCode::FileSkipped).detail.as_deref(),
        Some("corrupt")
    );
}

#[test]
fn logs_since_until_entry_array() {
    let options = Options {
        first_array_capacity: 1,
        ..Options::default()
    };
    let bytes = Writer::build(options, &sample_entries(20, START));
    let range = LogQuery {
        since: Some(at(START + 5 * STEP)),
        until: Some(at(START + 12 * STEP)),
        ..query(LogLevel::Debug)
    };
    assert_eq!(messages(&read(bytes.clone(), &range).0), expected(5..12));

    let between = LogQuery {
        since: Some(at(START + 4 * STEP + 500)),
        until: Some(at(START + 4 * STEP + 600)),
        ..query(LogLevel::Debug)
    };
    assert!(read(bytes.clone(), &between).0.is_empty());

    let unit_range = LogQuery {
        since: Some(at(START + 5 * STEP)),
        until: Some(at(START + 12 * STEP)),
        ..unit("ssh")
    };
    assert_eq!(
        messages(&read(bytes.clone(), &unit_range).0),
        ["event 10", "event 8", "event 6"]
    );

    let after = LogQuery {
        since: Some(at(START + 100 * STEP)),
        ..query(LogLevel::Debug)
    };
    assert!(read(bytes.clone(), &after).0.is_empty());
    let before = LogQuery {
        until: Some(at(START)),
        ..query(LogLevel::Debug)
    };
    assert!(read(bytes, &before).0.is_empty());
}

#[test]
fn logs_newest_first_limit() {
    let query = LogQuery {
        limit: 3,
        ..query(LogLevel::Debug)
    };
    let mut stream = query_bytes(sample(20), &query, &LogLimits::default()).unwrap();
    let events: Vec<LogEvent> = stream.by_ref().map(Result::unwrap).collect();
    assert_eq!(messages(&events), expected(17..20));
    assert_eq!(
        stream.next_cursor().map(ToString::to_string),
        Some(events[2].cursor.to_string())
    );
    assert_eq!(
        notice(&stream.notices(), NoticeCode::LimitReached).count,
        Some(3)
    );
}

#[test]
fn logs_cursor_roundtrip() {
    let bytes = sample(20);
    let mut pages = Vec::new();
    let mut cursor: Option<Cursor> = None;
    loop {
        let page = LogQuery {
            limit: 7,
            cursor: cursor.clone(),
            ..query(LogLevel::Debug)
        };
        let mut stream = query_bytes(bytes.clone(), &page, &LogLimits::default()).unwrap();
        pages.extend(stream.by_ref().map(|event| event.unwrap().message));
        let next = stream.next_cursor().map(ToString::to_string);
        assert!(pages.len() <= 20);
        match next {
            Some(text) => cursor = Some(Cursor::from_str(&text).unwrap()),
            None => break,
        }
    }
    assert_eq!(pages, expected(0..20));
}

#[test]
fn logs_cursor_after_rotation_approximate() {
    let entries = sample_entries(10, START);
    let first_page = LogQuery {
        limit: 5,
        ..query(LogLevel::Debug)
    };
    let mut stream = query_bytes(
        Writer::build(Options::default(), &entries),
        &first_page,
        &LogLimits::default(),
    )
    .unwrap();
    stream.by_ref().for_each(drop);
    let cursor = stream.next_cursor().cloned().unwrap();
    drop(stream);

    let root = tempfile::tempdir().unwrap();
    let rotated = Options {
        file_id: [0x44; 16],
        seqnum_id: [0x55; 16],
        ..Options::default()
    };
    system_root(
        root.path(),
        &[("system.journal", Writer::build(rotated, &entries))],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let resumed = LogQuery {
        cursor: Some(cursor),
        ..query(LogLevel::Debug)
    };
    let (events, notices) = read_store(&store, &resumed);
    assert_eq!(messages(&events), expected(0..5));
    assert!(has(&notices, NoticeCode::CursorApproximate));
}

#[test]
fn logs_compact_mode() {
    let reference = compressible(Options::default());
    let compact = compressible(Options {
        compact: true,
        ..Options::default()
    });
    assert_ne!(incompatible_flags(&compact) & FLAG_COMPACT, 0);
    for query in [
        query(LogLevel::Debug),
        unit(&long_unit()),
        query(LogLevel::Warning),
    ] {
        let expected = summary(&read(reference.clone(), &query).0);
        assert!(!expected.is_empty());
        assert_eq!(summary(&read(compact.clone(), &query).0), expected);
    }
}

#[test]
fn logs_keyed_hash() {
    let reference = compressible(Options::default());
    let keyed = compressible(Options {
        keyed_hash: true,
        ..Options::default()
    });
    assert_ne!(incompatible_flags(&keyed) & FLAG_KEYED_HASH, 0);
    for query in [
        unit("cron"),
        unit(&long_unit()),
        source("app"),
        query(LogLevel::Error),
    ] {
        let expected = summary(&read(reference.clone(), &query).0);
        assert!(!expected.is_empty());
        assert_eq!(summary(&read(keyed.clone(), &query).0), expected);
    }
}

#[test]
fn logs_xz_lz4_zstd() {
    let plain = compressible(Options::default());
    let queries = [
        query(LogLevel::Debug),
        unit(&long_unit()),
        query(LogLevel::Error),
    ];
    let expected: Vec<Vec<Summary>> = queries
        .iter()
        .map(|query| summary(&read(plain.clone(), query).0))
        .collect();
    for compression in [Compression::Xz, Compression::Lz4, Compression::Zstd] {
        for (compact, keyed_hash) in [(false, false), (true, true)] {
            let bytes = compressible(Options {
                compression,
                compact,
                keyed_hash,
                ..Options::default()
            });
            assert_ne!(incompatible_flags(&bytes) & compression.flag(), 0);
            assert!(
                bytes.len() < plain.len(),
                "{compression:?} did not shrink the file"
            );
            for (query, expected) in queries.iter().zip(&expected) {
                let (events, notices) = read(bytes.clone(), query);
                assert!(notices.is_empty(), "{compression:?}: {notices:?}");
                assert_eq!(&summary(&events), expected, "{compression:?}");
            }
        }
    }
}

#[test]
fn logs_online_file_snapshot() {
    let entries = sample_entries(8, START);
    let mut writer = Writer::new(Options {
        state: State::Online,
        ..Options::default()
    });
    for entry in &entries[..5] {
        writer.add(entry);
    }
    writer.freeze();
    for entry in &entries[5..] {
        writer.add(entry);
    }
    let bytes = writer.finish();
    let (events, notices) = read(bytes.clone(), &query(LogLevel::Debug));
    assert_eq!(messages(&events), expected(0..5));
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(
        messages(&read(bytes.clone(), &unit("ssh")).0),
        ["event 4", "event 2", "event 0"]
    );
    assert_eq!(
        messages(&read(bytes, &source("cron")).0),
        ["event 3", "event 1"]
    );
}

#[test]
fn every_header_state_is_read() {
    for state in [State::Offline, State::Online, State::Archived] {
        let bytes = Writer::build(
            Options {
                state,
                ..Options::default()
            },
            &sample_entries(3, START),
        );
        assert_eq!(
            messages(&read(bytes, &query(LogLevel::Debug)).0),
            expected(0..3)
        );
    }
}

#[test]
fn logs_truncated_file_skipped() {
    let bytes = sample(12);
    let full = messages(&read(bytes.clone(), &query(LogLevel::Debug)).0);
    let (events, notices) = read(
        bytes[..bytes.len() * 2 / 3].to_vec(),
        &query(LogLevel::Debug),
    );
    assert!(events.len() < full.len());
    assert!(
        messages(&events)
            .iter()
            .all(|message| full.contains(message))
    );
    assert_eq!(
        notice(&notices, NoticeCode::FileSkipped).detail.as_deref(),
        Some("truncated")
    );

    let root = tempfile::tempdir().unwrap();
    let other = Writer::build(
        Options {
            file_id: [0x91; 16],
            seqnum_id: [0x92; 16],
            ..Options::default()
        },
        &sample_entries(4, START),
    );
    system_root(
        root.path(),
        &[
            ("system.journal", bytes),
            ("system@cut.journal", other[..100].to_vec()),
        ],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let (events, notices) = read_store(&store, &query(LogLevel::Debug));
    assert_eq!(events.len(), 12);
    let skipped = notice(&notices, NoticeCode::FileSkipped);
    assert_eq!(skipped.detail.as_deref(), Some("truncated"));
    assert!(
        skipped
            .path
            .as_deref()
            .unwrap()
            .ends_with("system@cut.journal")
    );
}

#[test]
fn logs_unknown_incompatible_flag_skipped() {
    let future = Writer::build(
        Options {
            extra_incompatible: 1 << 5,
            file_id: [0x71; 16],
            ..Options::default()
        },
        &sample_entries(3, START),
    );
    let (events, notices) = read(future.clone(), &query(LogLevel::Debug));
    assert!(events.is_empty());
    assert_eq!(
        notice(&notices, NoticeCode::FileSkipped).detail.as_deref(),
        Some("unsupported_format")
    );

    let root = tempfile::tempdir().unwrap();
    system_root(
        root.path(),
        &[
            ("system.journal", sample(4)),
            ("system@future.journal", future),
        ],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let (events, notices) = read_store(&store, &query(LogLevel::Debug));
    assert_eq!(messages(&events), expected(0..4));
    assert_eq!(
        notice(&notices, NoticeCode::FileSkipped).detail.as_deref(),
        Some("unsupported_format")
    );
}

#[test]
fn logs_all_corrupt_e_log_format() {
    let future = Writer::build(
        Options {
            extra_incompatible: 1 << 6,
            ..Options::default()
        },
        &sample_entries(2, START),
    );
    let root = tempfile::tempdir().unwrap();
    system_root(
        root.path(),
        &[
            (
                "system.journal",
                b"bytes that are not a journal file".to_vec(),
            ),
            ("user-1000.journal", future),
        ],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    assert_eq!(
        store_error(&store, &query(LogLevel::Debug)).code(),
        ErrorCode::Format
    );
}

#[cfg(unix)]
#[test]
fn logs_permission_partial_notable() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let user = Writer::build(
        Options {
            file_id: [0x66; 16],
            seqnum_id: [0x77; 16],
            ..Options::default()
        },
        &sample_entries(2, START + 100 * STEP),
    );
    let directory = system_root(
        root.path(),
        &[("system.journal", sample(4)), ("user-1000.journal", user)],
    );
    let locked = directory.join("user-1000.journal");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&locked).is_ok() {
        return;
    }
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let (events, notices) = read_store(&store, &query(LogLevel::Debug));
    assert_eq!(messages(&events), expected(0..4));
    let denied = notice(&notices, NoticeCode::PermissionDeniedSkipped);
    assert_eq!(denied.count, Some(1));
    assert!(denied.to_notable().is_some());

    fs::set_permissions(
        directory.join("system.journal"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert_eq!(
        store_error(&store, &query(LogLevel::Debug)).code(),
        ErrorCode::Permission
    );
}

#[test]
fn logs_message_regex_filter_equivalence() {
    let bytes = sample(30);
    let pattern = Regex::new("event [12][0-9]$").unwrap();
    let all = read(bytes.clone(), &query(LogLevel::Debug)).0;
    let filtered = read(
        bytes,
        &LogQuery {
            message: Some(pattern.clone()),
            ..query(LogLevel::Debug)
        },
    )
    .0;
    let expected: Vec<String> = all
        .iter()
        .filter(|event| pattern.is_match(&event.message))
        .map(|event| event.message.clone())
        .collect();
    assert_eq!(filtered.len(), 20);
    assert_eq!(messages(&filtered), expected);
}

#[test]
fn logs_field_limits_truncated_flag() {
    let entry = Entry::new(
        START,
        &[
            ("MESSAGE", "a message longer than the limit"),
            ("SYSLOG_IDENTIFIER", "app"),
        ],
    )
    .with_bytes("BLOB", &[b'z'; 300]);
    let bytes = Writer::build(Options::default(), &[entry]);

    let (events, _) = read(bytes.clone(), &query(LogLevel::Debug));
    assert!(!events[0].flags.truncated);

    let short_message = LogLimits {
        max_message_bytes: 9,
        ..LogLimits::default()
    };
    let (events, _) = read_with(bytes.clone(), &query(LogLevel::Debug), &short_message);
    assert_eq!(events[0].message, "a message");
    assert!(events[0].flags.truncated);

    let short_data = LogLimits {
        max_data_bytes: 64,
        ..LogLimits::default()
    };
    let (events, _) = read_with(bytes, &query(LogLevel::Debug), &short_data);
    let blob = events[0]
        .fields
        .iter()
        .find(|(name, _)| name == "BLOB")
        .unwrap();
    assert_eq!(blob.1.len(), 64 - "BLOB=".len());
    assert!(events[0].flags.truncated);
}

#[test]
fn binary_fields_are_decoded_lossily() {
    let entry =
        Entry::new(START, &[("SYSLOG_IDENTIFIER", "app")]).with_bytes("MESSAGE", b"bad \xff byte");
    let (events, _) = read(
        Writer::build(Options::default(), &[entry]),
        &query(LogLevel::Debug),
    );
    assert_eq!(events[0].message, "bad \u{fffd} byte");
    assert!(events[0].flags.lossy);
}

#[test]
fn logs_cancel_between_events() {
    let token = CancelToken::new();
    let query = query(LogLevel::Debug);
    let mut stream = query_bytes(sample(10), &query, &LogLimits::default())
        .unwrap()
        .with_cancel(token.clone());
    assert!(stream.next().unwrap().is_ok());
    token.cancel(CancelReason::Requested);
    let error = stream.next().unwrap().unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert!(stream.next().is_none());
}

#[test]
fn unknown_names_list_similar_names() {
    let (events, notices) = read(sample(6), &unit("sshd"));
    assert!(events.is_empty());
    assert_eq!(
        notice(&notices, NoticeCode::UnknownName).detail.as_deref(),
        Some("ssh.service")
    );

    let (events, notices) = read(sample(6), &source("sshx"));
    assert!(events.is_empty());
    assert_eq!(
        notice(&notices, NoticeCode::UnknownName).detail.as_deref(),
        Some("sshd")
    );

    let (_, notices) = read(sample(6), &unit("zzz"));
    assert_eq!(notice(&notices, NoticeCode::UnknownName).detail, None);
}

#[test]
fn glob_patterns_select_units_and_sources() {
    let bytes = sample(6);
    assert_eq!(
        messages(&read(bytes.clone(), &unit("ssh*")).0),
        ["event 4", "event 2", "event 0"]
    );
    assert_eq!(
        messages(&read(bytes.clone(), &source("cr?n")).0),
        ["event 5", "event 3", "event 1"]
    );
    let both = LogQuery {
        source: Some(NameMatch::parse("cron").unwrap()),
        ..unit("cron.service")
    };
    assert_eq!(
        messages(&read(bytes, &both).0),
        ["event 5", "event 3", "event 1"]
    );
}

#[test]
fn units_of_manager_and_user_messages_are_found() {
    let entries = [
        Entry::new(
            START,
            &[
                ("MESSAGE", "Started cron"),
                ("_PID", "1"),
                ("_SYSTEMD_UNIT", "init.scope"),
                ("UNIT", "cron.service"),
            ],
        ),
        Entry::new(
            START + STEP,
            &[
                ("MESSAGE", "user unit"),
                ("_SYSTEMD_UNIT", "user@1000.service"),
                ("_SYSTEMD_USER_UNIT", "music.service"),
            ],
        ),
    ];
    let bytes = Writer::build(Options::default(), &entries);
    let (events, _) = read(bytes.clone(), &unit("cron"));
    assert_eq!(messages(&events), ["Started cron"]);
    assert_eq!(events[0].unit.as_deref(), Some("cron.service"));
    assert_eq!(
        messages(&read(bytes, &unit("music.service")).0),
        ["user unit"]
    );
}

#[test]
fn source_falls_back_to_executable_and_unknown() {
    let entries = [
        Entry::new(
            START,
            &[("MESSAGE", "from exe"), ("_EXE", "/usr/libexec/helper")],
        ),
        Entry::new(START + STEP, &[("MESSAGE", "anonymous")]),
        Entry::new(
            START + 2 * STEP,
            &[("MESSAGE", "kernel line"), ("_TRANSPORT", "kernel")],
        ),
    ];
    let bytes = Writer::build(Options::default(), &entries);
    assert_eq!(
        messages(&read(bytes.clone(), &source("helper")).0),
        ["from exe"]
    );
    assert_eq!(
        messages(&read(bytes.clone(), &source("help*")).0),
        ["from exe"]
    );
    assert_eq!(
        messages(&read(bytes.clone(), &source("unknown")).0),
        ["anonymous"]
    );
    assert_eq!(messages(&read(bytes, &source("kernel")).0), ["kernel line"]);
}

#[test]
fn copies_of_one_file_are_read_once() {
    let root = tempfile::tempdir().unwrap();
    let bytes = sample(5);
    system_root(
        root.path(),
        &[
            ("system.journal", bytes.clone()),
            ("system@copy.journal", bytes),
        ],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    assert_eq!(
        messages(&read_store(&store, &query(LogLevel::Debug)).0),
        expected(0..5)
    );
}

#[test]
fn merges_rotated_and_parallel_files_newest_first() {
    let entries = sample_entries(12, START);
    let archived = Writer::build(
        Options {
            state: State::Archived,
            file_id: [0x81; 16],
            ..Options::default()
        },
        &entries[..6],
    );
    let active = Writer::build(
        Options {
            file_id: [0x82; 16],
            first_seqnum: 7,
            ..Options::default()
        },
        &entries[6..],
    );
    let user_entries: Vec<Entry> = (0..3)
        .map(|index| {
            let message = format!("user {index}");
            Entry::new(
                START + 500 + index * 4 * STEP,
                &[("MESSAGE", message.as_str())],
            )
        })
        .collect();
    let user = Writer::build(
        Options {
            file_id: [0x83; 16],
            seqnum_id: [0x84; 16],
            ..Options::default()
        },
        &user_entries,
    );
    let root = tempfile::tempdir().unwrap();
    system_root(
        root.path(),
        &[
            ("system@0001.journal", archived),
            ("system.journal", active),
            ("user-1000.journal", user),
        ],
    );
    let single = JournaldStore::new(
        root.path(),
        LogLimits {
            max_open_files: 1,
            ..LogLimits::default()
        },
    );
    let mut sequential = messages(&read_store(&single, &query(LogLevel::Debug)).0);
    sequential.sort();
    assert_eq!(sequential.len(), 15);
    for max_open_files in [3, 64] {
        let limits = LogLimits {
            max_open_files,
            ..LogLimits::default()
        };
        let store = JournaldStore::new(root.path(), limits);
        let (events, _) = read_store(&store, &query(LogLevel::Debug));
        assert_eq!(events.len(), 15);
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0].time.as_unix_nanos() >= pair[1].time.as_unix_nanos())
        );
        assert_eq!(events[0].message, "event 11");
        assert_eq!(events[14].message, "event 0");
    }
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let recent = LogQuery {
        since: Some(at(START + 7 * STEP)),
        ..query(LogLevel::Debug)
    };
    assert_eq!(
        messages(&read_store(&store, &recent).0),
        [
            "event 11", "event 10", "event 9", "user 2", "event 8", "event 7"
        ]
    );
}

#[test]
fn missing_journal_is_not_found() {
    let root = tempfile::tempdir().unwrap();
    let store = JournaldStore::new(root.path(), LogLimits::default());
    assert_eq!(
        store_error(&store, &query(LogLevel::Debug)).code(),
        ErrorCode::NotFound
    );
}

#[test]
fn files_of_other_machines_are_ignored() {
    let foreign = Writer::build(
        Options {
            machine_id: [0x99; 16],
            ..Options::default()
        },
        &sample_entries(3, START),
    );
    let root = tempfile::tempdir().unwrap();
    let directory = system_root(root.path(), &[("system.journal", foreign)]);
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let (events, notices) = read_store(&store, &query(LogLevel::Debug));
    assert!(events.is_empty());
    assert!(notices.is_empty(), "{notices:?}");

    let files =
        JournaldStore::from_files(vec![directory.join("system.journal")], LogLimits::default());
    assert_eq!(
        messages(&read_store(&files, &query(LogLevel::Debug)).0),
        expected(0..3)
    );
}

#[test]
fn without_machine_id_every_machine_directory_is_read() {
    let root = tempfile::tempdir().unwrap();
    system_root(root.path(), &[("system.journal", sample(2))]);
    fs::remove_file(root.path().join("etc/machine-id")).unwrap();
    let volatile = root.path().join("run/log/journal").join(machine_name());
    fs::create_dir_all(&volatile).unwrap();
    let runtime = Writer::build(
        Options {
            file_id: [0x31; 16],
            seqnum_id: [0x32; 16],
            ..Options::default()
        },
        &sample_entries(1, START + 10 * STEP),
    );
    fs::write(volatile.join("system.journal"), runtime).unwrap();
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let (events, _) = read_store(&store, &query(LogLevel::Debug));
    assert_eq!(messages(&events), ["event 0", "event 1", "event 0"]);
}

#[test]
fn damaged_entry_keeps_newer_events() {
    let entries = sample_entries(6, START);
    let mut writer = Writer::new(Options::default());
    for entry in &entries[..3] {
        writer.add(entry);
    }
    let damaged = usize::try_from(writer.last_entry_offset()).unwrap();
    for entry in &entries[3..] {
        writer.add(entry);
    }
    let mut bytes = writer.finish();
    bytes[damaged] = 0x7f;
    let (events, notices) = read(bytes, &query(LogLevel::Debug));
    assert_eq!(messages(&events), expected(3..6));
    assert_eq!(
        notice(&notices, NoticeCode::FileSkipped).detail.as_deref(),
        Some("corrupt")
    );
}

#[test]
fn cursor_of_another_store_is_rejected() {
    let query = LogQuery {
        cursor: Some(Cursor::from_str("v1:w:System:5").unwrap()),
        ..query(LogLevel::Debug)
    };
    let error = query_bytes(sample(1), &query, &LogLimits::default())
        .err()
        .unwrap();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn event_fields_follow_the_mapping() {
    let entry = Entry::new(
        START,
        &[
            ("MESSAGE", "accepted"),
            ("PRIORITY", "5"),
            ("SYSLOG_IDENTIFIER", "sshd"),
            ("_SYSTEMD_UNIT", "ssh.service"),
            ("_PID", "812"),
            ("TID", "813"),
            ("_UID", "0"),
            ("_HOSTNAME", "server"),
            ("MESSAGE_ID", "0123456789abcdef0123456789abcdef"),
        ],
    );
    let (events, _) = read(
        Writer::build(Options::default(), &[entry]),
        &query(LogLevel::Debug),
    );
    let event = &events[0];
    assert_eq!(event.time.as_unix_nanos(), at(START).as_unix_nanos());
    assert_eq!(event.source, "sshd");
    assert_eq!(event.priority, 5);
    assert_eq!(event.pid, Some(812));
    assert_eq!(event.tid, Some(813));
    assert_eq!(event.host.as_deref(), Some("server"));
    assert_eq!(
        event.id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        event.boot_id.as_deref(),
        Some("33333333333333333333333333333333")
    );
    assert!(event.cursor.to_string().starts_with("v1:j:"));
    let names: Vec<&str> = event.fields.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "PRIORITY",
            "SYSLOG_IDENTIFIER",
            "_SYSTEMD_UNIT",
            "TID",
            "_UID"
        ]
    );
}

#[test]
fn field_dropped_for_its_size_marks_the_event_truncated() {
    let long = "x".repeat(1_000);
    let entry = Entry::new(
        START,
        &[("MESSAGE", long.as_str()), ("SYSLOG_IDENTIFIER", "app")],
    );
    let bytes = Writer::build(
        Options {
            compression: Compression::Lz4,
            compress_threshold: 64,
            ..Options::default()
        },
        &[entry],
    );

    let (events, _) = read(bytes.clone(), &query(LogLevel::Debug));
    assert_eq!(events[0].message, long);
    assert!(!events[0].flags.truncated);

    let short_data = LogLimits {
        max_data_bytes: 64,
        ..LogLimits::default()
    };
    let (events, _) = read_with(bytes, &query(LogLevel::Debug), &short_data);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message, "");
    assert_eq!(events[0].source, "app");
    assert!(events[0].flags.truncated);
}

#[test]
fn bounds_between_microseconds_keep_the_microsecond_of_the_entry() {
    let bytes = sample(8);
    let after = |micros: u64| {
        Instant::from_unix_nanos(at(micros).as_unix_nanos() + 1)
    };
    let since = LogQuery {
        since: Some(after(START + 3 * STEP)),
        ..query(LogLevel::Debug)
    };
    assert_eq!(messages(&read(bytes.clone(), &since).0), expected(4..8));
    let until_after = LogQuery {
        until: Some(after(START + 3 * STEP)),
        ..query(LogLevel::Debug)
    };
    assert_eq!(
        messages(&read(bytes.clone(), &until_after).0),
        expected(0..4)
    );
    let until_exact = LogQuery {
        until: Some(at(START + 3 * STEP)),
        ..query(LogLevel::Debug)
    };
    assert_eq!(messages(&read(bytes, &until_exact).0), expected(0..3));
}

#[test]
fn cursor_continues_into_the_archived_file_of_the_same_sequence_space() {
    let entries = sample_entries(12, START);
    let archived = Writer::build(
        Options {
            state: State::Archived,
            file_id: [0x81; 16],
            ..Options::default()
        },
        &entries[..6],
    );
    let active = Writer::build(
        Options {
            file_id: [0x82; 16],
            first_seqnum: 7,
            ..Options::default()
        },
        &entries[6..],
    );
    let root = tempfile::tempdir().unwrap();
    system_root(
        root.path(),
        &[("system@0001.journal", archived), ("system.journal", active)],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let first = LogQuery {
        limit: 8,
        ..query(LogLevel::Debug)
    };
    let mut stream = store.query(&first).unwrap();
    let page: Vec<LogEvent> = stream.by_ref().map(Result::unwrap).collect();
    assert_eq!(messages(&page), expected(4..12));
    let cursor = stream.next_cursor().cloned().unwrap();
    drop(stream);

    let second = LogQuery {
        cursor: Some(cursor),
        ..query(LogLevel::Debug)
    };
    let (events, notices) = read_store(&store, &second);
    assert_eq!(messages(&events), expected(0..4));
    assert!(!has(&notices, NoticeCode::CursorApproximate));
}

#[test]
fn files_renamed_after_an_unclean_shutdown_are_read() {
    let dirty = Writer::build(
        Options {
            file_id: [0x41; 16],
            seqnum_id: [0x42; 16],
            ..Options::default()
        },
        &sample_entries(2, START + 100 * STEP),
    );
    let root = tempfile::tempdir().unwrap();
    system_root(
        root.path(),
        &[
            ("system.journal", sample(3)),
            ("user-1000@0005.journal~", dirty),
            ("notes.txt", b"ignored".to_vec()),
        ],
    );
    let store = JournaldStore::new(root.path(), LogLimits::default());
    let (events, notices) = read_store(&store, &query(LogLevel::Debug));
    assert_eq!(
        messages(&events),
        ["event 1", "event 0", "event 2", "event 1", "event 0"]
    );
    assert!(notices.is_empty(), "{notices:?}");
}
