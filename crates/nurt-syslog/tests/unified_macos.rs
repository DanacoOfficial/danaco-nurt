// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Reads of the unified log of the running system and of log archives on
//! macOS.
//!
//! Every test checks its precondition and passes without assertions when it
//! is not met: reading the local log store and collecting an archive require
//! root, and the comparison with `log show` requires the `log` program.

#[cfg(target_os = "macos")]
mod system {
    use std::path::Path;
    use std::process::Command;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::record::LogLevel;
    use nurt_syslog::event::{LogEvent, StoreKind};
    use nurt_syslog::limits::LogLimits;
    use nurt_syslog::query::{LogQuery, LogStore, NameMatch};
    use nurt_syslog::unified::backend::UnifiedBackend;
    use nurt_syslog::unified::store::UnifiedStore;
    use nurt_syslog::unified::{ArchiveBackend, OsLogBackend};
    use regex::Regex;

    fn is_root() -> bool {
        Command::new("id")
            .arg("-u")
            .output()
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "0")
    }

    fn read<B: UnifiedBackend>(store: &UnifiedStore<B>, query: &LogQuery) -> Option<Vec<LogEvent>> {
        match store.query(query) {
            Err(error) if error.code() == ErrorCode::Privilege => None,
            result => Some(result.unwrap().map(Result::unwrap).collect()),
        }
    }

    fn assert_newest_first(events: &[LogEvent]) {
        for pair in events.windows(2) {
            assert!(pair[0].time >= pair[1].time, "{pair:?}");
        }
        assert!(
            events
                .iter()
                .all(|event| event.store == StoreKind::UnifiedLog)
        );
    }

    #[test]
    #[ignore = "requires the macOS unified log and root"]
    fn unified_local_store_pages_newest_first() {
        if !is_root() {
            return;
        }
        let store = UnifiedStore::new(OsLogBackend::local(), LogLimits::default());
        let first = LogQuery {
            level: LogLevel::Debug,
            limit: 50,
            ..LogQuery::default()
        };
        let Some(events) = read(&store, &first) else {
            return;
        };
        assert_newest_first(&events);
        let mut stream = store.query(&first).unwrap();
        let page: Vec<LogEvent> = stream.by_ref().map(Result::unwrap).collect();
        let Some(cursor) = stream.next_cursor().cloned() else {
            return;
        };
        let next = LogQuery {
            cursor: Some(cursor.clone()),
            ..first.clone()
        };
        let following = read(&store, &next).unwrap();
        assert_newest_first(&following);
        let last = page.last().unwrap();
        assert!(following.iter().all(|event| event.time <= last.time));
        assert!(following.iter().all(|event| event.cursor != cursor));
    }

    #[test]
    #[ignore = "requires the macOS unified log, root and the logger program"]
    fn unified_local_store_finds_a_message_written_by_logger() {
        if !is_root() {
            return;
        }
        let marker = format!("nurt-syslog test message {}", std::process::id());
        let written = Command::new("logger").arg(&marker).status();
        if !written.is_ok_and(|status| status.success()) {
            return;
        }
        let store = UnifiedStore::new(OsLogBackend::local(), LogLimits::default());
        let query = LogQuery {
            source: Some(NameMatch::parse("logger").unwrap()),
            level: LogLevel::Debug,
            message: Some(Regex::new(&regex::escape(&marker)).unwrap()),
            limit: 5,
            ..LogQuery::default()
        };
        let Some(events) = read(&store, &query) else {
            return;
        };
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source, "logger");

        let shown = Command::new("log")
            .args(["show", "--style", "ndjson", "--last", "5m", "--predicate"])
            .arg("process == \"logger\"")
            .output();
        if let Ok(output) = shown {
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(
                text.contains(&marker),
                "log show did not return the message"
            );
        }
    }

    #[test]
    #[ignore = "requires the macOS unified log, root and the log program"]
    fn unified_archive_backends_agree_on_a_collected_archive() {
        if !is_root() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("recent.logarchive");
        let collected = Command::new("log")
            .args(["collect", "--last", "2m", "--output"])
            .arg(&archive)
            .status();
        if !collected.is_ok_and(|status| status.success()) || !Path::new(&archive).is_dir() {
            return;
        }
        let query = LogQuery {
            level: LogLevel::Error,
            limit: 200,
            ..LogQuery::default()
        };
        let framework = UnifiedStore::new(OsLogBackend::archive(&archive), LogLimits::default());
        let parsed = UnifiedStore::new(ArchiveBackend::new(&archive), LogLimits::default());
        let from_framework = read(&framework, &query).unwrap();
        let from_parser = read(&parsed, &query).unwrap();
        assert_newest_first(&from_framework);
        assert_newest_first(&from_parser);
        let key = |event: &LogEvent| (event.time, event.priority, event.source.clone());
        let parsed_keys: Vec<_> = from_parser.iter().map(key).collect();
        let shared = from_framework
            .iter()
            .filter(|event| parsed_keys.contains(&key(event)))
            .count();
        assert!(shared * 2 >= from_framework.len().min(from_parser.len()));
    }

    #[test]
    fn unified_archive_backends_reject_a_missing_directory() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.logarchive");
        let parsed = UnifiedStore::new(ArchiveBackend::new(&missing), LogLimits::default());
        let error = parsed.query(&LogQuery::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let framework = UnifiedStore::new(OsLogBackend::archive(&missing), LogLimits::default());
        let error = framework.query(&LogQuery::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}
