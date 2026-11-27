// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::track::tests::scale
// Responsibility:   Scan time of a large tree; run explicitly with `--ignored`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Scan time of a large tree; run explicitly with `--ignored`.

use std::time::{Duration, Instant};

use super::support::{DiskFs, Harness};
use crate::track::scan::{Scope, scan};

/// Files of the measured tree.
const FILES: usize = 100_000;

/// Files per directory of the measured tree.
const FILES_PER_DIR: usize = 500;

/// Scan time allowed for the measured tree.
const SCAN_LIMIT: Duration = Duration::from_secs(1);

#[test]
#[ignore = "measures the scan of 100 000 files"]
fn scans_a_hundred_thousand_files_within_a_second() {
    let h = Harness::new();
    for index in 0..FILES {
        let dir = index / FILES_PER_DIR;
        std::fs::create_dir_all(h.path(&format!("d{dir:04}"))).unwrap();
        std::fs::write(h.path(&format!("d{dir:04}/f{index:06}")), b"x").unwrap();
    }
    let roots = [nurt_contracts::fs::AbsPath::new(&h.root).unwrap()];
    let ignore = vec![".git".to_owned()];
    let started = Instant::now();
    let scanned = scan(
        &DiskFs,
        Scope {
            roots: &roots,
            ignore: &ignore,
            hash: false,
            racy: None,
            bound: None,
        },
    );
    let elapsed = started.elapsed();
    eprintln!("scan of {} entries took {elapsed:?}", scanned.entries.len());
    assert!(scanned.entries.len() > FILES);
    assert!(elapsed <= SCAN_LIMIT, "scan took {elapsed:?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
