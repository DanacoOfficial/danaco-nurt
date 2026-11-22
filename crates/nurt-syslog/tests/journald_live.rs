// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Queries of the journal of the running system.

use std::path::Path;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::LogLevel;
use nurt_syslog::event::StoreKind;
use nurt_syslog::journald::JournaldStore;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogQuery, LogStore};

const SYSTEMD_RUNTIME: &str = "/run/systemd/system";

#[test]
#[ignore = "requires a systemd journal"]
fn reads_the_journal_of_the_running_system() {
    if !Path::new(SYSTEMD_RUNTIME).is_dir() {
        return;
    }
    let store = JournaldStore::local(LogLimits::default());
    let query = LogQuery {
        level: LogLevel::Debug,
        limit: 50,
        ..LogQuery::default()
    };
    let stream = match store.query(&query) {
        Ok(stream) => stream,
        Err(error) => {
            assert!(
                matches!(error.code(), ErrorCode::Permission | ErrorCode::NotFound),
                "{error:?}"
            );
            return;
        }
    };
    let events: Vec<_> = stream.map(Result::unwrap).collect();
    assert!(events.len() <= 50);
    for event in &events {
        assert_eq!(event.store, StoreKind::Journald);
        assert!(!event.source.is_empty());
        assert!(event.boot_id.is_some());
    }
}
