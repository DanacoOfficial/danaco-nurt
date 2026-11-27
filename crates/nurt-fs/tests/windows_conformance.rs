// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-fs/tests/windows_conformance.rs
// Responsibility:   Conformance suite of the file system interface on the Windows back end.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conformance suite of the file system interface on the Windows back end.
//!
//! The suite runs on NTFS in the temporary directory of the account. The run
//! on `ReFS` needs a directory named by `NURT_TEST_REFS_DIR` and is ignored by
//! default; it also verifies cross-volume renames towards the temporary
//! directory.

#![cfg(all(windows, feature = "conformance"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod common;

use nurt_contracts::fs::Fs;
use nurt_fs::conformance::{self, Options, Outcome};

use common::{fixture, fixture_in, refs_dir};

/// Checks that NTFS supports; the suite skips a check only when the volume
/// declares the capability missing, so a skip here is a wrong declaration.
const NTFS_CHECKS: [&str; 6] = [
    "fs-conf-replace-with-backup",
    "fs-conf-hard-link",
    "fs-conf-streams",
    "fs-conf-meta-round-trip",
    "fs-conf-lock-exclusive",
    "fs-conf-rename-noreplace",
];

#[test]
fn conforms_on_the_temporary_ntfs_volume() {
    let fx = fixture();
    let report = conformance::run(&fx.fs, &fx.root, &fx.scope);
    assert!(report.passed(), "{report}");
    for id in NTFS_CHECKS {
        assert_eq!(report.outcome(id), Some(&Outcome::Passed), "{id}\n{report}");
    }
}

#[test]
#[ignore = "requires NURT_TEST_REFS_DIR naming a directory on a ReFS volume"]
fn conforms_on_a_refs_volume() {
    let refs = fixture_in(&refs_dir());
    let volume = refs.fs.volume(&refs.root).unwrap();
    assert_eq!(volume.fs_type, "refs", "{volume:?}");
    let other = fixture();
    assert!(!refs.fs.same_volume(&refs.root, &other.root).unwrap());
    let options = Options {
        other_volume: Some(other.root.clone()),
    };
    let report = conformance::run_with(&refs.fs, &refs.root, &refs.scope, &options);
    assert!(report.passed(), "{report}");
    for id in [
        "fs-conf-clone-file",
        "fs-conf-replace-with-backup",
        "fs-conf-cross-device",
    ] {
        assert_eq!(report.outcome(id), Some(&Outcome::Passed), "{id}\n{report}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
