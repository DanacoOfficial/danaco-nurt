// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syslog/fuzz/fuzz_targets/journal_file.rs
// Responsibility:   Reads arbitrary bytes as a journal file.
// Dependencies:     libfuzzer_sys, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reads arbitrary bytes as a journal file.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nurt_contracts::record::LogLevel;
use nurt_syslog::journald::query_bytes;
use nurt_syslog::limits::LogLimits;
use nurt_syslog::query::LogQuery;

const LIMIT: u32 = 1_000;

fuzz_target!(|data: &[u8]| {
    let query = LogQuery {
        level: LogLevel::Debug,
        limit: LIMIT,
        ..LogQuery::default()
    };
    if let Ok(stream) = query_bytes(data.to_vec(), &query, &LogLimits::default()) {
        stream.for_each(drop);
    }
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
