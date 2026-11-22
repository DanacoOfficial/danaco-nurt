// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Reads of the event log of the running Windows system.
//!
//! The tests read existing events of the channel `System` and do not write
//! to the event log. The comparison with `Get-WinEvent` runs only when
//! PowerShell is available.

#[cfg(windows)]
mod system {
    use std::process::Command;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::record::LogLevel;
    use nurt_syslog::cursor::Cursor;
    use nurt_syslog::event::{LogEvent, StoreKind};
    use nurt_syslog::eventlog::WevtSource;
    use nurt_syslog::eventlog::store::EventLogStore;
    use nurt_syslog::limits::LogLimits;
    use nurt_syslog::query::{LogQuery, LogStore, NameMatch};

    fn system(limit: u32) -> LogQuery {
        LogQuery {
            unit: Some(NameMatch::parse("System").unwrap()),
            level: LogLevel::Debug,
            limit,
            ..LogQuery::default()
        }
    }

    fn record_ids(events: &[LogEvent]) -> Vec<u64> {
        events
            .iter()
            .map(|event| match &event.cursor {
                Cursor::EventLog { record_id, .. } => *record_id,
                other => panic!("unexpected cursor {other}"),
            })
            .collect()
    }

    #[test]
    #[ignore = "requires the Windows event log"]
    fn eventlog_system_channel_pages_newest_first() {
        let store = EventLogStore::new(WevtSource::new(), LogLimits::default());
        let first = system(20);
        let mut stream = store.query(&first).unwrap();
        let page: Vec<LogEvent> = stream.by_ref().map(Result::unwrap).collect();
        for pair in page.windows(2) {
            assert!(pair[0].time >= pair[1].time);
        }
        assert!(page.iter().all(|event| event.store == StoreKind::EventLog));
        assert!(
            page.iter()
                .all(|event| event.unit.as_deref() == Some("System"))
        );
        let Some(cursor) = stream.next_cursor().cloned() else {
            return;
        };
        let next = LogQuery {
            cursor: Some(cursor),
            ..system(20)
        };
        let following: Vec<LogEvent> = store.query(&next).unwrap().map(Result::unwrap).collect();
        let last = *record_ids(&page).last().unwrap();
        assert!(record_ids(&following).iter().all(|id| *id < last));
    }

    #[test]
    #[ignore = "requires the Windows event log"]
    fn eventlog_unknown_channel_lists_candidates_of_the_system() {
        let store = EventLogStore::new(WevtSource::new(), LogLimits::default());
        let query = LogQuery {
            unit: Some(NameMatch::parse("Systen").unwrap()),
            ..LogQuery::default()
        };
        let error = store.query(&query).unwrap_err().into_data();
        assert_eq!(error.code, ErrorCode::UnknownValue);
        let expected = error.expected.unwrap();
        assert!(expected.as_array().is_some_and(|names| !names.is_empty()));
    }

    #[test]
    #[ignore = "requires the Windows event log and PowerShell"]
    fn eventlog_matches_get_winevent() {
        let store = EventLogStore::new(WevtSource::new(), LogLimits::default());
        let query = system(10);
        let ours: Vec<LogEvent> = store.query(&query).unwrap().map(Result::unwrap).collect();
        let script = "Get-WinEvent -LogName System -MaxEvents 30 | ForEach-Object { $_.RecordId }";
        let Ok(output) = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
        else {
            return;
        };
        let theirs: Vec<u64> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        let ours = record_ids(&ours);
        let start = theirs
            .iter()
            .position(|id| Some(id) == ours.first())
            .unwrap();
        let end = (start + ours.len()).min(theirs.len());
        assert_eq!(&theirs[start..end], &ours[..end - start]);
    }
}
