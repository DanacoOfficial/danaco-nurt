// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Random entries written by the test writer and read back equal the result
//! of a direct filter over the same entries.

#[path = "support/journal_writer.rs"]
mod journal_writer;

use journal_writer::{Compression, Entry, Options, Writer};
use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;
use nurt_syslog::event::max_priority;
use nurt_syslog::journald::query_bytes;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogQuery, NameMatch};
use proptest::prelude::*;

const START: u64 = 1_650_000_000_000_000;
const UNITS: [&str; 3] = ["a.service", "b.service", "c.service"];
const LEVELS: [LogLevel; 5] = [
    LogLevel::Critical,
    LogLevel::Error,
    LogLevel::Warning,
    LogLevel::Info,
    LogLevel::Debug,
];
const COMPRESSIONS: [Compression; 4] = [
    Compression::None,
    Compression::Xz,
    Compression::Lz4,
    Compression::Zstd,
];

#[derive(Debug, Clone)]
struct Spec {
    priority: u8,
    unit: usize,
    gap: u64,
    text: String,
}

fn spec() -> impl Strategy<Value = Spec> {
    (0_u8..8, 0_usize..UNITS.len(), 1_u64..5_000, "[a-z ]{0,40}").prop_map(
        |(priority, unit, gap, text)| Spec {
            priority,
            unit,
            gap,
            text,
        },
    )
}

/// Entry with its time and message, as the model sees it.
struct Written {
    realtime: u64,
    message: String,
    priority: u8,
    unit: usize,
}

fn write(specs: &[Spec]) -> (Vec<Entry>, Vec<Written>) {
    let mut realtime = START;
    let mut entries = Vec::new();
    let mut written = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        realtime += spec.gap;
        let message = format!("{index} {}", spec.text);
        let priority = spec.priority.to_string();
        entries.push(Entry::new(
            realtime,
            &[
                ("MESSAGE", message.as_str()),
                ("PRIORITY", priority.as_str()),
                ("_SYSTEMD_UNIT", UNITS[spec.unit]),
                ("SYSLOG_IDENTIFIER", "app"),
            ],
        ));
        written.push(Written {
            realtime,
            message,
            priority: spec.priority,
            unit: spec.unit,
        });
    }
    (entries, written)
}

fn instant(micros: u64) -> Instant {
    Instant::from_unix_nanos(i64::try_from(micros * 1_000).unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn reader_matches_a_direct_filter(
        specs in proptest::collection::vec(spec(), 0..40),
        compact in any::<bool>(),
        keyed_hash in any::<bool>(),
        compression in 0_usize..COMPRESSIONS.len(),
        first_array_capacity in 1_u64..6,
        level in 0_usize..LEVELS.len(),
        unit in proptest::option::of(0_usize..UNITS.len()),
        since in proptest::option::of(0_u64..100_000),
        until in proptest::option::of(0_u64..100_000),
        limit in 1_u32..50,
    ) {
        let (entries, written) = write(&specs);
        let options = Options {
            compact,
            keyed_hash,
            compression: COMPRESSIONS[compression],
            compress_threshold: 16,
            first_array_capacity,
            ..Options::default()
        };
        let bytes = Writer::build(options, &entries);
        let query = LogQuery {
            level: LEVELS[level],
            unit: unit.map(|unit| NameMatch::parse(UNITS[unit]).unwrap()),
            since: since.map(|since| instant(START + since)),
            until: until.map(|until| instant(START + until)),
            limit,
            ..LogQuery::default()
        };
        let threshold = max_priority(LEVELS[level]);
        let expected: Vec<&str> = written
            .iter()
            .rev()
            .filter(|entry| entry.priority <= threshold)
            .filter(|entry| unit.is_none_or(|unit| unit == entry.unit))
            .filter(|entry| since.is_none_or(|since| entry.realtime >= START + since))
            .filter(|entry| until.is_none_or(|until| entry.realtime < START + until))
            .map(|entry| entry.message.as_str())
            .take(usize::try_from(limit).unwrap())
            .collect();
        let mut stream = query_bytes(bytes, &query, &LogLimits::default()).unwrap();
        let actual: Vec<String> = stream.by_ref().map(|event| event.unwrap().message).collect();
        prop_assert_eq!(actual, expected);
    }
}
