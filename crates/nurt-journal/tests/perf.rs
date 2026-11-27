// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-journal/tests/perf.rs
// Responsibility:   Wall-clock limits of the journal. The binary is named `perf` so that the.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Wall-clock limits of the journal. The binary is named `perf` so that the
//! filter `not binary(perf)` of `cargo nextest` skips it on loaded machines.

use std::time::Duration;

use nurt_journal::testing::time_recovery_among_empty_entries;

/// Number of empty entry directories next to the entry to recover.
const EMPTY_ENTRIES: u64 = 1_000_000;

/// Time within which the recovery must finish.
const RECOVERY_LIMIT: Duration = Duration::from_secs(5);

#[test]
fn recovery_with_a_million_entries_takes_less_than_five_seconds() {
    let elapsed = time_recovery_among_empty_entries(EMPTY_ENTRIES);
    assert!(elapsed < RECOVERY_LIMIT, "the recovery took {elapsed:?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
