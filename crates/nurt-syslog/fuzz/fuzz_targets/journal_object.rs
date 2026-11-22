// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Overwrites the objects of valid journal files and reads the result.
//!
//! The input selects one of two files written by the test writer (regular,
//! or compact with the keyed hash and zstd), the query, and the position
//! after the header from which the remaining input bytes replace the file
//! content. The header stays valid, so the reader reaches the objects.

#![no_main]

#[path = "../../tests/support/journal_writer.rs"]
mod journal_writer;

use std::sync::OnceLock;

use journal_writer::{Compression, HEADER_SIZE, Options, Writer, sample_entries};
use libfuzzer_sys::fuzz_target;
use nurt_contracts::record::LogLevel;
use nurt_syslog::journald::query_bytes;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::{LogQuery, NameMatch};

const START: u64 = 1_600_000_000_000_000;
const ENTRIES: u64 = 8;
const LIMIT: u32 = 1_000;
const FILE_BIT: u8 = 1;
const UNIT_BIT: u8 = 2;

fn bases() -> &'static [Vec<u8>; 2] {
    static BASES: OnceLock<[Vec<u8>; 2]> = OnceLock::new();
    BASES.get_or_init(|| {
        let entries = sample_entries(ENTRIES, START);
        let compact = Options {
            compact: true,
            keyed_hash: true,
            compression: Compression::Zstd,
            compress_threshold: 8,
            ..Options::default()
        };
        [
            Writer::build(Options::default(), &entries),
            Writer::build(compact, &entries),
        ]
    })
}

fuzz_target!(|data: &[u8]| {
    let [selector, high, low, rest @ ..] = data else {
        return;
    };
    let base = &bases()[usize::from(selector & FILE_BIT)];
    let Ok(header) = usize::try_from(HEADER_SIZE) else {
        return;
    };
    let span = base.len().saturating_sub(header).max(1);
    let start = header + usize::from(u16::from_be_bytes([*high, *low])) % span;
    let mut file = base.clone();
    if let Some(tail) = file.get_mut(start..) {
        for (slot, byte) in tail.iter_mut().zip(rest) {
            *slot = *byte;
        }
    }
    let mut query = LogQuery {
        level: LogLevel::Debug,
        limit: LIMIT,
        ..LogQuery::default()
    };
    if selector & UNIT_BIT != 0 {
        query.unit = NameMatch::parse("ssh").ok();
    }
    if let Ok(stream) = query_bytes(file, &query, &LogLimits::default()) {
        stream.for_each(drop);
    }
});
