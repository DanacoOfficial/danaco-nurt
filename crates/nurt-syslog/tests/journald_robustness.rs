// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

//! Damaged journal files: every defect ends in events and notices, never in
//! a panic or an endless loop.

#[path = "support/journal_writer.rs"]
mod journal_writer;

use journal_writer::{Compression, Entry, HEADER_SIZE, Options, Writer, sample_entries};
use nurt_contracts::record::LogLevel;
use nurt_contracts::value::Instant;
use nurt_syslog::journald::query_bytes;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogNotice, LogQuery, NameMatch, NoticeCode};
use proptest::prelude::*;
use proptest::sample::Index;

const START: u64 = 1_600_000_000_000_000;
const STEP: u64 = 1_000;
const DATA_NEXT_HASH: usize = 24;
const ARRAY_NEXT: usize = 16;
const TRUNCATION_STEP: usize = 8;

fn all_levels() -> LogQuery {
    LogQuery {
        level: LogLevel::Debug,
        limit: 1_000,
        ..LogQuery::default()
    }
}

fn queries() -> Vec<LogQuery> {
    vec![
        all_levels(),
        LogQuery {
            unit: Some(NameMatch::parse("ssh").unwrap()),
            level: LogLevel::Error,
            ..all_levels()
        },
        LogQuery {
            source: Some(NameMatch::parse("c*").unwrap()),
            since: Some(Instant::from_unix_nanos(
                i64::try_from((START + 2 * STEP) * 1_000).unwrap(),
            )),
            ..all_levels()
        },
    ]
}

/// Number of events read and the notices; a query refused before reading
/// counts as no events.
fn consume(bytes: Vec<u8>, query: &LogQuery) -> (usize, Vec<LogNotice>) {
    match query_bytes(bytes, query, &LogLimits::default()) {
        Ok(mut stream) => {
            let count = stream.by_ref().filter(Result::is_ok).count();
            (count, stream.notices())
        }
        Err(_) => (0, Vec::new()),
    }
}

fn entries() -> Vec<Entry> {
    let mut entries = sample_entries(6, START);
    let long = format!("long {}", "abcdefgh".repeat(40));
    entries.push(Entry::new(
        START + 6 * STEP,
        &[
            ("MESSAGE", long.as_str()),
            ("_SYSTEMD_UNIT", "ssh.service"),
            ("PRIORITY", "2"),
        ],
    ));
    entries
}

fn variants() -> Vec<Vec<u8>> {
    let base = Options {
        compress_threshold: 64,
        ..Options::default()
    };
    [
        base.clone(),
        Options {
            compact: true,
            keyed_hash: true,
            compression: Compression::Zstd,
            ..base.clone()
        },
        Options {
            compression: Compression::Lz4,
            first_array_capacity: 1,
            ..base.clone()
        },
        Options {
            compression: Compression::Xz,
            data_buckets: 2,
            field_buckets: 1,
            ..base
        },
    ]
    .into_iter()
    .map(|options| Writer::build(options, &entries()))
    .collect()
}

#[test]
fn every_truncation_is_survived() {
    let queries = queries();
    for bytes in variants() {
        let header = usize::try_from(HEADER_SIZE).unwrap();
        let cuts = (0..header + 64).chain((header + 64..bytes.len()).step_by(TRUNCATION_STEP));
        for len in cuts {
            for query in &queries {
                let (_, notices) = consume(bytes[..len].to_vec(), query);
                assert!(
                    notices
                        .iter()
                        .any(|notice| notice.code == NoticeCode::FileSkipped),
                    "cut at {len} without a notice"
                );
            }
        }
        assert_eq!(consume(bytes, &all_levels()).0, 7);
    }
}

#[test]
fn every_single_bit_flip_is_survived() {
    let query = all_levels();
    for bytes in variants() {
        for position in 0..bytes.len() {
            let mut copy = bytes.clone();
            copy[position] ^= 1 << (position % 8);
            consume(copy, &query);
        }
    }
}

#[test]
fn hash_chain_loop_is_reported_corrupt() {
    let mut writer = Writer::new(Options {
        data_buckets: 1,
        ..Options::default()
    });
    for entry in sample_entries(4, START) {
        writer.add(&entry);
    }
    let first = writer.data_offset(b"MESSAGE=event 0").unwrap();
    let later = usize::try_from(writer.data_offset(b"MESSAGE=event 3").unwrap()).unwrap();
    let mut bytes = writer.finish();
    bytes[later + DATA_NEXT_HASH..later + DATA_NEXT_HASH + 8].copy_from_slice(&first.to_le_bytes());
    let query = LogQuery {
        unit: Some(NameMatch::parse("missing").unwrap()),
        ..all_levels()
    };
    let (count, notices) = consume(bytes, &query);
    assert_eq!(count, 0);
    let skipped = notices
        .iter()
        .find(|notice| notice.code == NoticeCode::FileSkipped)
        .unwrap();
    assert_eq!(skipped.detail.as_deref(), Some("corrupt"));
}

#[test]
fn entry_array_loop_is_reported_corrupt() {
    let mut writer = Writer::new(Options {
        first_array_capacity: 1,
        ..Options::default()
    });
    for entry in sample_entries(6, START) {
        writer.add(&entry);
    }
    let first = writer.first_entry_array();
    let mut bytes = writer.finish();
    let at = usize::try_from(first).unwrap() + ARRAY_NEXT;
    let second =
        usize::try_from(u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())).unwrap();
    bytes[second + ARRAY_NEXT..second + ARRAY_NEXT + 8].copy_from_slice(&first.to_le_bytes());
    let (count, notices) = consume(bytes, &all_levels());
    assert_eq!(count, 0);
    let skipped = notices
        .iter()
        .find(|notice| notice.code == NoticeCode::FileSkipped)
        .unwrap();
    assert_eq!(skipped.detail.as_deref(), Some("corrupt"));
}

#[test]
fn header_pointing_outside_the_file_is_rejected() {
    let base = Writer::build(Options::default(), &sample_entries(2, START));
    for (offset, value) in [
        (104_usize, u64::MAX - 7),
        (120, 3),
        (136, 9),
        (176, u64::MAX),
        (96, u64::MAX),
    ] {
        let mut bytes = base.clone();
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        let (count, notices) = consume(bytes, &all_levels());
        assert!(count <= 2);
        assert!(!notices.is_empty(), "field at {offset}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        consume(data, &all_levels());
    }

    #[test]
    fn arbitrary_objects_after_a_valid_header_never_panic(
        data in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let base = Writer::build(Options::default(), &sample_entries(1, START));
        let mut bytes = base[..usize::try_from(HEADER_SIZE).unwrap()].to_vec();
        bytes.extend(data);
        for query in queries() {
            consume(bytes.clone(), &query);
        }
    }

    #[test]
    fn mutated_files_never_panic(
        variant in 0_usize..4,
        edits in proptest::collection::vec((any::<Index>(), any::<u8>()), 1..24),
    ) {
        let mut bytes = variants().swap_remove(variant);
        for (index, value) in edits {
            let position = index.index(bytes.len());
            bytes[position] = value;
        }
        for query in queries() {
            consume(bytes.clone(), &query);
        }
    }
}
