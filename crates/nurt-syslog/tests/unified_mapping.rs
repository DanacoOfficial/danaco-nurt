// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syslog/tests/unified_mapping.rs
// Responsibility:   Levels, predicates and the mapping of unified log entries and log archive.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Levels, predicates and the mapping of unified log entries and log archive
//! records to log events.

use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;
use nurt_syslog::cursor::Cursor;
use nurt_syslog::event::StoreKind;
use nurt_syslog::unified::archive::{
    ArchiveBuffer, ArchiveKind, ArchiveRecord, boot_id, image_name, lost_count,
};
use nurt_syslog::unified::backend::{BackendItem, EntryRequest, Requirement};
use nurt_syslog::unified::entry::{
    UnifiedEntry, digest, instant_from_unix_nanos_f64, instant_from_unix_seconds, unix_seconds,
};
use nurt_syslog::unified::level::{
    EntryLevel, TYPE_DEFAULT, TYPE_ERROR, TYPE_FAULT, TYPE_INFO, message_types,
};
use nurt_syslog::unified::predicate::Predicate;
use serde_json::Value;

/// Recorded `OSLogEntryLog` properties, newest first; see the README of the
/// fixture directory.
const ENTRIES: &str = include_str!("fixtures/unified/entries.json");

/// 2025-11-21T10:15:30.25Z, the time of the first recorded entry, in
/// nanoseconds.
const FIRST_NANOS: i64 = 1_763_720_130_250_000_000;

/// Spacing of doubles near current times in nanoseconds since the epoch:
/// 2^60 to 2^61 nanoseconds have a spacing of 256, rounded up.
const DOUBLE_NANOS_RESOLUTION: i64 = 512;

fn text(value: &Value, name: &str) -> String {
    value[name].as_str().unwrap().to_owned()
}

fn entry_of(value: &Value) -> UnifiedEntry {
    let activity = value["activityIdentifier"].as_u64().unwrap();
    UnifiedEntry {
        time: instant_from_unix_seconds(value["date"].as_f64().unwrap()).unwrap(),
        level: EntryLevel::from_raw(value["level"].as_i64().unwrap()),
        message: text(value, "composedMessage"),
        process: text(value, "process"),
        pid: value["processIdentifier"].as_u64(),
        tid: value["threadIdentifier"].as_u64(),
        subsystem: Some(text(value, "subsystem")),
        category: Some(text(value, "category")),
        sender: Some(text(value, "sender")),
        activity_id: (activity != 0).then_some(activity),
        format_string: Some(text(value, "formatString")),
        ..UnifiedEntry::default()
    }
}

fn recorded() -> Vec<UnifiedEntry> {
    let values: Vec<Value> = serde_json::from_str(ENTRIES).unwrap();
    values.iter().map(entry_of).collect()
}

fn at(nanos: i64) -> Instant {
    Instant::from_unix_nanos(nanos)
}

fn record(kind: ArchiveKind, nanos: f64, message: &str) -> ArchiveRecord {
    ArchiveRecord {
        kind: Some(kind),
        time: nanos,
        subsystem: "com.example.store".to_owned(),
        category: "database".to_owned(),
        process: "/usr/libexec/exampled".to_owned(),
        pid: 411,
        thread_id: 9001,
        euid: 501,
        library: "/System/Library/PrivateFrameworks/ExampleStore.framework/ExampleStore".to_owned(),
        library_uuid: "5D1C8A0E-2F43-4B6A-9C1D-0E2F3A4B5C6D".to_owned(),
        process_uuid: "0A1B2C3D-4E5F-4061-8273-94A5B6C7D8E9".to_owned(),
        boot_uuid: "8E4C2A3B-1F2D-4C5E-9A8B-7C6D5E4F3A2B".to_owned(),
        activity_id: 0,
        message: message.to_owned(),
        raw_message: String::new(),
    }
}

fn log_entry(item: BackendItem) -> UnifiedEntry {
    match item {
        BackendItem::Log(entry) => *entry,
        other => panic!("expected a log message, found {other:?}"),
    }
}

#[test]
fn unified_level_mapping() {
    let cases = [
        (0, EntryLevel::Undefined, 5, LogLevel::Info),
        (1, EntryLevel::Debug, 7, LogLevel::Debug),
        (2, EntryLevel::Info, 6, LogLevel::Info),
        (3, EntryLevel::Notice, 5, LogLevel::Info),
        (4, EntryLevel::Error, 3, LogLevel::Error),
        (5, EntryLevel::Fault, 2, LogLevel::Critical),
        (6, EntryLevel::Undefined, 5, LogLevel::Info),
        (-1, EntryLevel::Undefined, 5, LogLevel::Info),
    ];
    for (raw, level, priority, mapped) in cases {
        assert_eq!(EntryLevel::from_raw(raw), level, "raw level {raw}");
        assert_eq!(level.priority(), priority, "{level:?}");
        let entry = UnifiedEntry {
            level,
            ..UnifiedEntry::default()
        };
        assert_eq!(entry.to_event().level(), mapped, "{level:?}");
    }
    assert_eq!(EntryLevel::default(), EntryLevel::Undefined);
}

#[test]
fn unified_message_types_follow_the_threshold() {
    assert_eq!(message_types(LogLevel::Critical), Some(&[TYPE_FAULT][..]));
    assert_eq!(
        message_types(LogLevel::Error),
        Some(&[TYPE_ERROR, TYPE_FAULT][..])
    );
    assert_eq!(
        message_types(LogLevel::Warning),
        Some(&[TYPE_ERROR, TYPE_FAULT][..])
    );
    assert_eq!(
        message_types(LogLevel::Info),
        Some(&[TYPE_DEFAULT, TYPE_INFO, TYPE_ERROR, TYPE_FAULT][..])
    );
    assert_eq!(message_types(LogLevel::Debug), None);
}

#[test]
fn unified_predicate_build() {
    let all = Predicate::new(Some("com.example.store"), Some("exampled"), LogLevel::Info).unwrap();
    assert_eq!(
        all.format(),
        "subsystem == %@ AND process == %@ AND messageType IN {0, 1, 16, 17}"
    );
    assert_eq!(all.arguments(), ["com.example.store", "exampled"]);
    assert!(all.has_level());

    let process = Predicate::new(None, Some("exampled"), LogLevel::Debug).unwrap();
    assert_eq!(process.format(), "process == %@");
    assert_eq!(process.arguments(), ["exampled"]);
    assert!(!process.has_level());
    assert_eq!(process.without_level(), Some(process.clone()));

    let critical = Predicate::new(None, None, LogLevel::Critical).unwrap();
    assert_eq!(critical.format(), "messageType IN {17}");
    assert!(critical.arguments().is_empty());
    assert_eq!(critical.without_level(), None);

    let reduced = all.without_level().unwrap();
    assert_eq!(reduced.format(), "subsystem == %@ AND process == %@");
    assert_eq!(reduced.arguments(), all.arguments());

    assert_eq!(Predicate::new(None, None, LogLevel::Debug), None);
}

#[test]
fn unified_predicate_quotes_nothing_into_the_format() {
    let name = "com.example' OR 1 == 1";
    let predicate = Predicate::new(Some(name), None, LogLevel::Debug).unwrap();
    assert_eq!(predicate.format(), "subsystem == %@");
    assert_eq!(predicate.arguments(), [name]);
}

#[test]
fn unified_predicate_matches_like_the_store() {
    let entries = recorded();
    let store = Predicate::new(Some("com.example.store"), None, LogLevel::Debug).unwrap();
    let matched: Vec<&str> = entries
        .iter()
        .filter(|entry| store.matches(entry))
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(matched, ["database file is damaged", "started"]);

    let errors = Predicate::new(None, Some("exampled"), LogLevel::Error).unwrap();
    let matched: Vec<&str> = entries
        .iter()
        .filter(|entry| errors.matches(entry))
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(matched, ["database file is damaged", "connection refused"]);

    let info = Predicate::new(None, None, LogLevel::Info).unwrap();
    assert_eq!(
        entries.iter().filter(|entry| info.matches(entry)).count(),
        5
    );

    let unnamed = UnifiedEntry {
        subsystem: None,
        ..entries[0].clone()
    };
    assert!(!store.matches(&unnamed));
}

#[test]
fn unified_fault_entry_maps_every_field() {
    let entries = recorded();
    let event = entries[0].to_event();
    assert_eq!(event.time, at(FIRST_NANOS));
    assert_eq!(event.source, "exampled");
    assert_eq!(event.priority, 2);
    assert_eq!(event.level(), LogLevel::Critical);
    assert_eq!(event.message, "database file is damaged");
    assert_eq!(event.unit.as_deref(), Some("com.example.store"));
    assert_eq!(event.category.as_deref(), Some("database"));
    assert_eq!((event.pid, event.tid), (Some(411), Some(9001)));
    assert_eq!(event.store, StoreKind::UnifiedLog);
    assert_eq!(event.id, None);
    assert_eq!(event.host, None);
    assert_eq!(
        event.fields,
        [
            ("sender", "ExampleStore"),
            ("activityIdentifier", "12"),
            ("formatString", "database file is %{public}s"),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
    assert!(!event.flags.unformatted);
    assert_eq!(
        event.cursor.to_string(),
        format!(
            "v1:m:{FIRST_NANOS}:411:9001:{:016x}",
            digest("database file is damaged")
        )
    );
}

#[test]
fn unified_entries_without_optional_values_map_to_log_events() {
    let entries = recorded();
    let started = entries[2].to_event();
    assert_eq!(started.time, at(1_763_720_128_000_001_000));
    assert_eq!(started.category, None);
    assert_eq!(started.level(), LogLevel::Info);
    assert!(
        !started
            .fields
            .iter()
            .any(|(name, _)| name == "activityIdentifier")
    );

    let helper = entries[3].to_event();
    assert_eq!(helper.unit, None);
    assert_eq!(helper.message, "cache hit <private>");
    assert_eq!(helper.priority, 6);
    assert_eq!(entries[4].to_event().level(), LogLevel::Debug);
    assert_eq!(entries[5].to_event().priority, 5);
}

#[test]
fn unified_entry_without_identifiers_has_zero_in_its_cursor() {
    let entry = UnifiedEntry {
        time: at(7),
        message: "a".to_owned(),
        ..UnifiedEntry::default()
    };
    let cursor = entry.cursor();
    assert_eq!(
        cursor,
        Cursor::Unified {
            nanos: 7,
            pid: 0,
            tid: 0,
            digest: digest("a"),
        }
    );
    assert!(entry.is_at(&cursor));
    let other = UnifiedEntry {
        message: "b".to_owned(),
        ..entry.clone()
    };
    assert!(!other.is_at(&cursor));
}

#[test]
fn unified_entry_maps_archive_only_fields() {
    let entry = UnifiedEntry {
        process: "exampled".to_owned(),
        user: Some("501".to_owned()),
        boot_id: Some("8e4c2a3b1f2d4c5e9a8b7c6d5e4f3a2b".to_owned()),
        sender: Some(String::new()),
        process_uuid: Some("0A1B".to_owned()),
        sender_uuid: Some("5D1C".to_owned()),
        unformatted: true,
        ..UnifiedEntry::default()
    };
    let event = entry.to_event();
    assert_eq!(event.user.as_deref(), Some("501"));
    assert_eq!(
        event.boot_id.as_deref(),
        Some("8e4c2a3b1f2d4c5e9a8b7c6d5e4f3a2b")
    );
    assert!(event.flags.unformatted);
    assert_eq!(
        event.fields,
        [("processUUID", "0A1B"), ("senderUUID", "5D1C")]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
}

#[test]
fn unified_times_convert_without_loss_above_microseconds() {
    assert_eq!(
        instant_from_unix_seconds(1_763_720_130.25),
        Some(at(FIRST_NANOS))
    );
    assert_eq!(instant_from_unix_seconds(-1.5), Some(at(-1_500_000_000)));
    assert_eq!(instant_from_unix_seconds(f64::INFINITY), None);
    assert_eq!(instant_from_unix_seconds(1.0e300), None);
    assert_eq!(instant_from_unix_nanos_f64(2.6), Some(at(3)));
    assert_eq!(instant_from_unix_nanos_f64(-1.0e19), None);
    assert_eq!(instant_from_unix_nanos_f64(f64::NAN), None);
    assert_eq!(unix_seconds(at(FIRST_NANOS)), 1_763_720_130.25);
    assert_eq!(unix_seconds(at(-500_000_000)), -0.5);
}

#[test]
fn unified_privilege_requirement_of_the_local_store() {
    assert_eq!(Requirement::for_local_store(false), Requirement::Root);
    assert_eq!(
        Requirement::for_local_store(true),
        Requirement::FullDiskAccess
    );
    assert_eq!(Requirement::Root.as_str(), "root");
    assert_eq!(Requirement::FullDiskAccess.as_str(), "full_disk_access");
}

#[test]
fn unified_archive_record_maps_to_entry() {
    let entry = log_entry(
        record(
            ArchiveKind::Log(EntryLevel::Fault),
            1.763_720_130_25e18,
            "database file is damaged",
        )
        .into_item(),
    );
    assert_eq!(entry.process, "exampled");
    assert_eq!(entry.sender.as_deref(), Some("ExampleStore"));
    assert_eq!(entry.level, EntryLevel::Fault);
    assert_eq!(entry.pid, Some(411));
    assert_eq!(entry.tid, Some(9001));
    assert_eq!(entry.user.as_deref(), Some("501"));
    assert_eq!(
        entry.boot_id.as_deref(),
        Some("8e4c2a3b1f2d4c5e9a8b7c6d5e4f3a2b")
    );
    assert_eq!(entry.subsystem.as_deref(), Some("com.example.store"));
    assert_eq!(entry.category.as_deref(), Some("database"));
    assert_eq!(entry.activity_id, None);
    assert_eq!(entry.format_string, None);
    assert_eq!(
        entry.process_uuid.as_deref(),
        Some("0A1B2C3D-4E5F-4061-8273-94A5B6C7D8E9")
    );
    assert!(!entry.unformatted);
    assert!((entry.time.as_unix_nanos() - FIRST_NANOS).abs() < DOUBLE_NANOS_RESOLUTION);
}

#[test]
fn unified_archive_record_without_message_is_unformatted() {
    let mut raw = record(ArchiveKind::Log(EntryLevel::Notice), 1.0e18, "");
    raw.raw_message = "connection %s".to_owned();
    raw.subsystem = String::new();
    raw.category = String::new();
    raw.library = String::new();
    raw.library_uuid = String::new();
    raw.process_uuid = String::new();
    raw.boot_uuid = "not a uuid".to_owned();
    raw.activity_id = 42;
    let entry = log_entry(raw.into_item());
    assert!(entry.unformatted);
    assert_eq!(entry.message, "connection %s");
    assert_eq!(entry.subsystem, None);
    assert_eq!(entry.category, None);
    assert_eq!(entry.sender, None);
    assert_eq!(entry.sender_uuid, None);
    assert_eq!(entry.process_uuid, None);
    assert_eq!(entry.boot_id, None);
    assert_eq!(entry.activity_id, Some(42));
    assert!(entry.to_event().flags.unformatted);

    let empty = log_entry(record(ArchiveKind::Log(EntryLevel::Info), 1.0e18, "").into_item());
    assert!(!empty.unformatted);
    assert_eq!(empty.message, "");
}

#[test]
fn unified_archive_records_other_than_messages() {
    let loss = record(ArchiveKind::Loss, 1.0e18, "lost 12 unreliable messages");
    assert_eq!(loss.into_item(), BackendItem::Lost(12));
    let other = record(ArchiveKind::Other, 1.0e18, "activity");
    assert_eq!(other.into_item(), BackendItem::Other);
    let unknown = ArchiveRecord::default();
    assert_eq!(unknown.into_item(), BackendItem::Other);
    let outside = record(ArchiveKind::Log(EntryLevel::Info), f64::NAN, "m");
    assert_eq!(outside.into_item(), BackendItem::Other);
}

#[test]
fn unified_archive_helpers() {
    assert_eq!(lost_count("lost 12 unreliable messages"), 12);
    assert_eq!(lost_count("7"), 7);
    assert_eq!(lost_count("lost 0 messages"), 1);
    assert_eq!(lost_count("messages lost"), 1);
    assert_eq!(lost_count("lost 99999999999999999999999 messages"), 1);
    assert_eq!(image_name("/usr/sbin/exampled"), "exampled");
    assert_eq!(image_name("kernel"), "kernel");
    assert_eq!(image_name("/usr/lib/"), "");
    assert_eq!(
        boot_id("8e4c2a3b1f2d4c5e9a8b7c6d5e4f3a2b").as_deref(),
        Some("8e4c2a3b1f2d4c5e9a8b7c6d5e4f3a2b")
    );
    assert_eq!(boot_id("8E4C2A3B-1F2D-4C5E-9A8B-7C6D5E4F3A2"), None);
    assert_eq!(boot_id("ZE4C2A3B-1F2D-4C5E-9A8B-7C6D5E4F3A2B"), None);
}

#[test]
fn unified_archive_buffer_returns_matching_entries_newest_first() {
    let request = EntryRequest {
        predicate: Predicate::new(None, Some("exampled"), LogLevel::Error),
        until: Some(at(5_000)),
    };
    let mut buffer = ArchiveBuffer::new(&request);
    buffer.push(record(
        ArchiveKind::Log(EntryLevel::Error),
        1_000.0,
        "oldest",
    ));
    buffer.push(record(
        ArchiveKind::Log(EntryLevel::Fault),
        4_000.0,
        "newest",
    ));
    buffer.push(record(
        ArchiveKind::Log(EntryLevel::Error),
        5_000.0,
        "at until",
    ));
    buffer.push(record(ArchiveKind::Log(EntryLevel::Info), 3_000.0, "info"));
    buffer.push(record(ArchiveKind::Loss, 2_500.0, "lost 2 messages"));
    buffer.push(record(ArchiveKind::Log(EntryLevel::Error), 2_000.0, "b"));
    buffer.push(record(ArchiveKind::Log(EntryLevel::Error), 2_000.0, "a"));
    buffer.push(record(ArchiveKind::Loss, 1_500.0, "lost 3 messages"));
    buffer.push(record(ArchiveKind::Other, 3_500.0, "activity"));
    let mut helper = record(ArchiveKind::Log(EntryLevel::Fault), 3_000.0, "helper");
    helper.process = "/usr/libexec/examplehelper".to_owned();
    buffer.push(helper);

    let items: Vec<BackendItem> = buffer
        .into_items()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(items[0], BackendItem::Lost(5));
    let messages: Vec<String> = items
        .into_iter()
        .skip(1)
        .map(|item| log_entry(item).message)
        .collect();
    let mut same_time = ["a", "b"];
    same_time.sort_by_key(|message| std::cmp::Reverse(digest(message)));
    assert_eq!(
        messages,
        ["newest", same_time[0], same_time[1], "oldest"].map(str::to_owned)
    );
}

#[test]
fn unified_archive_buffer_without_conditions_keeps_every_message() {
    let mut buffer = ArchiveBuffer::new(&EntryRequest::default());
    buffer.push(record(ArchiveKind::Log(EntryLevel::Debug), 1.0, "debug"));
    buffer.push(record(
        ArchiveKind::Log(EntryLevel::Undefined),
        2.0,
        "undefined",
    ));
    let items = buffer.into_items();
    assert_eq!(items.len(), 2);
    assert!(ArchiveBuffer::default().into_items().is_empty());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
