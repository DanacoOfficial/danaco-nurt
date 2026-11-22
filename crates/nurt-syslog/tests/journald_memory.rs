// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Peak memory of a query over large journals.
//!
//! The peak resident set size is reset through `/proc/self/clear_refs` after
//! the files are written, so the measurement covers the query only. Each
//! test runs alone in this test binary.

#[cfg(target_os = "linux")]
#[path = "support/journal_writer.rs"]
mod journal_writer;

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, PoisonError};

    use super::journal_writer::{Entry, Options, Writer, system_root};
    use nurt_contracts::record::LogLevel;
    use nurt_syslog::journald::JournaldStore;
    use nurt_syslog::limits::LogLimits;
    use nurt_syslog::query::{LogQuery, LogStore};

    const START: u64 = 1_690_000_000_000_000;
    const KIBIBYTE: u64 = 1_024;
    const MEBIBYTE: u64 = 1_024 * KIBIBYTE;
    const RESET_PEAK: &str = "5";

    /// Keeps the measurements of this binary from overlapping.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn peak_bytes() -> Option<u64> {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kib * KIBIBYTE)
    }

    fn reset_peak() -> bool {
        fs::write("/proc/self/clear_refs", RESET_PEAK).is_ok()
    }

    /// Writes `files` journal files of `entries` entries each, with distinct
    /// messages of about 400 bytes.
    fn write_journals(root: &Path, files: u64, entries: u64) {
        let directory = system_root(root, &[]);
        let filler = "0123456789".repeat(40);
        for file in 0..files {
            let options = Options {
                file_id: [u8::try_from(file % 200).unwrap() + 1; 16],
                first_seqnum: file * entries + 1,
                data_buckets: 4_096,
                ..Options::default()
            };
            let mut writer = Writer::new(options);
            for index in 0..entries {
                let number = file * entries + index;
                let message = format!("{number} {filler}");
                writer.add(&Entry::new(
                    START + number * 1_000,
                    &[
                        ("MESSAGE", message.as_str()),
                        ("PRIORITY", "6"),
                        ("SYSLOG_IDENTIFIER", "load"),
                    ],
                ));
            }
            let name = if file + 1 == files {
                "system.journal".to_owned()
            } else {
                format!("system@{file:04}.journal")
            };
            fs::write(directory.join(name), writer.finish()).unwrap();
        }
    }

    /// Reads every event and returns their number and the growth of the peak
    /// resident set size, or `None` when the system does not report it.
    fn measure(root: &Path, limits: LogLimits) -> Option<(u64, u64)> {
        if !reset_peak() {
            return None;
        }
        let before = peak_bytes()?;
        let store = JournaldStore::new(root, limits);
        let query = LogQuery {
            level: LogLevel::Debug,
            limit: u32::MAX,
            ..LogQuery::default()
        };
        let mut count = 0;
        for event in store.query(&query).unwrap() {
            event.unwrap();
            count += 1;
        }
        Some((count, peak_bytes()?.saturating_sub(before)))
    }

    #[test]
    fn logs_memory_bound() {
        const ENTRIES: u64 = 20_000;
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        write_journals(root.path(), 1, ENTRIES);
        let limits = LogLimits {
            read_cache_bytes: MEBIBYTE,
            ..LogLimits::default()
        };
        let Some((count, growth)) = measure(root.path(), limits) else {
            return;
        };
        assert_eq!(count, ENTRIES);
        assert!(growth < 8 * MEBIBYTE, "peak grew by {growth} bytes");
    }

    #[test]
    #[ignore = "performance test: writes 1 GiB of journal files"]
    fn logs_memory_bound_one_gibibyte() {
        const FILES: u64 = 16;
        const ENTRIES: u64 = 130_000;
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        write_journals(root.path(), FILES, ENTRIES);
        let Some((count, growth)) = measure(root.path(), LogLimits::default()) else {
            return;
        };
        assert_eq!(count, FILES * ENTRIES);
        assert!(growth < 96 * MEBIBYTE, "peak grew by {growth} bytes");
    }
}
