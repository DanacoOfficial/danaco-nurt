// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-testkit/tests/fs_contract.rs
// Responsibility:   The conformance checks of the file system interface on the virtual file.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The conformance checks of the file system interface on the virtual file
//! system, with and without optional volume features.

// Test assertions fail through unwrap, expect and panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::fs::FsFeatures;
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::fs_contract::{CHECKS, CheckOutcome, FsContract};
use nurt_testkit::vfs::{VirtualFs, VolumeConfig, test_path};

fn file_system(config: VolumeConfig) -> VirtualFs {
    let fs = VirtualFs::with_root(Arc::new(VirtualClock::new()), config);
    fs.create_dir_all(&test_path("/contract").unwrap()).unwrap();
    fs
}

#[test]
fn virtual_file_system_meets_the_contract() {
    let fs = file_system(VolumeConfig::default());
    let scope = FsContract::scope();
    let report = FsContract::new(&fs, test_path("/contract").unwrap(), &scope).run();
    assert!(report.is_success(), "{report}");
    assert_eq!(report.checks.len(), CHECKS.len());
    assert_eq!(report.skipped().count(), 0, "{report}");
}

#[test]
fn checks_of_missing_features_are_skipped() {
    let config = VolumeConfig {
        features: FsFeatures {
            hardlink: false,
            rename_noreplace: false,
            ..VolumeConfig::default_features()
        },
        ..VolumeConfig::default()
    };
    let fs = file_system(config);
    let scope = FsContract::scope();
    let report = FsContract::new(&fs, test_path("/contract").unwrap(), &scope).run();
    assert!(report.is_success(), "{report}");
    assert_eq!(report.skipped().collect::<Vec<_>>(), ["hard_links"]);
    assert!(
        report
            .to_string()
            .starts_with(&format!("{} passed, 1 skipped, 0 failed", CHECKS.len() - 1))
    );
}

#[test]
fn single_checks_run_by_name() {
    let fs = file_system(VolumeConfig::default());
    let scope = FsContract::scope();
    let contract = FsContract::new(&fs, test_path("/contract").unwrap(), &scope);
    assert_eq!(contract.run_check("rename"), Some(CheckOutcome::Passed));
    assert_eq!(contract.run_check("unknown"), None);
    let again = contract.run_check("rename").unwrap();
    assert!(
        matches!(&again, CheckOutcome::Failed { message } if message.contains("create_dir")),
        "{again:?}"
    );
}

#[test]
fn deviations_are_reported_as_failures() {
    let fs = file_system(VolumeConfig::default());
    let scope = FsContract::scope();
    let report = FsContract::new(&fs, test_path("/absent").unwrap(), &scope).run();
    assert!(!report.is_success());
    assert_eq!(report.failures().count(), CHECKS.len());
    assert!(report.to_string().contains("files_round_trip: create_dir"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
