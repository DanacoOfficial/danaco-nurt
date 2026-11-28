// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/oracle.rs
// Responsibility:   Oracle checks on hand-made trees.
// Dependencies:     nurt_contracts, nurt_crashfs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Oracle checks on hand-made trees.

use std::path::{Path, PathBuf};

use nurt_contracts::fs::FsWrite;
use nurt_crashfs::crash::CrashProfile;
use nurt_crashfs::oracle::{
    Change, Expected, Matched, OpOutcome, TEMP_PREFIX, TreeSnapshot, Violation, check_complete,
    check_explicit, check_idempotent, check_no_leftovers, check_preserved,
};

use crate::support::{at, durable_file, new_fs, scope, volume_root};

fn target() -> PathBuf {
    PathBuf::from("t")
}

fn before() -> TreeSnapshot {
    TreeSnapshot::default().with(&Change::Write {
        path: target(),
        content: b"old".to_vec(),
    })
}

fn after() -> TreeSnapshot {
    before().with(&Change::Write {
        path: target(),
        content: b"new".to_vec(),
    })
}

#[test]
fn oracle_accepts_complete_states_only() {
    assert_eq!(
        check_complete(&before(), &before(), &after(), Expected::Either),
        Ok(Matched::Before)
    );
    assert_eq!(
        check_complete(&after(), &before(), &after(), Expected::After),
        Ok(Matched::After)
    );
    let mixed = after().with(&Change::CreateDir {
        path: PathBuf::from("extra"),
        mode: 0o700,
    });
    assert!(matches!(
        check_complete(&mixed, &before(), &after(), Expected::Either),
        Err(Violation::IntermediateState { .. })
    ));
    assert_eq!(
        check_complete(&before(), &before(), &after(), Expected::After),
        Err(Violation::WrongState {
            expected: Expected::After,
            found: Matched::Before
        })
    );
}

#[test]
fn oracle_finds_previous_content_anywhere_on_the_volume() {
    let moved = after().with(&Change::Write {
        path: Path::new("journal").join("blob"),
        content: b"old".to_vec(),
    });
    assert_eq!(check_preserved(&before(), &[target()], &moved), Ok(()));
    assert_eq!(
        check_preserved(&before(), &[target()], &after()),
        Err(Violation::PreviousContentLost { path: target() })
    );
}

#[test]
fn oracle_models_rename_and_removal() {
    let tree = TreeSnapshot::default()
        .with(&Change::CreateDir {
            path: PathBuf::from("a"),
            mode: 0o755,
        })
        .with(&Change::Write {
            path: Path::new("a").join("f"),
            content: b"x".to_vec(),
        });
    let renamed = tree.with(&Change::Rename {
        from: PathBuf::from("a"),
        to: PathBuf::from("b"),
    });
    assert!(renamed.get(&Path::new("b").join("f")).is_some());
    assert!(renamed.get(Path::new("a")).is_none());
    let removed = renamed.with(&Change::Remove {
        path: PathBuf::from("b"),
    });
    assert!(removed.entries().is_empty());
    let mode = tree.with(&Change::SetMode {
        path: PathBuf::from("a"),
        mode: 0o700,
    });
    assert_eq!(mode.differences(&tree), vec![PathBuf::from("a")]);
}

#[test]
fn oracle_matches_the_model_with_a_captured_tree() {
    let fs = new_fs(CrashProfile::PosixWorstCase, 1);
    durable_file(&fs, &at("t"), b"old");
    let captured = TreeSnapshot::capture(&fs, &volume_root(), &[]).unwrap();
    assert_eq!(captured.differences(&before()), Vec::<PathBuf>::new());
    fs.create_dir(&scope(), &at("skip"), 0o755).unwrap();
    let filtered = TreeSnapshot::capture(&fs, &volume_root(), &[Path::new("skip")]).unwrap();
    assert_eq!(filtered, captured);
}

#[test]
fn oracle_reports_leftovers_silent_errors_and_repeated_recovery_changes() {
    let leftover = before().with(&Change::Write {
        path: PathBuf::from(format!("{TEMP_PREFIX}1")),
        content: Vec::new(),
    });
    assert!(matches!(
        check_no_leftovers(&leftover),
        Err(Violation::TemporaryLeftover { .. })
    ));
    assert_eq!(check_no_leftovers(&before()), Ok(()));
    assert_eq!(
        check_explicit(OpOutcome::FailedSilently),
        Err(Violation::SilentError)
    );
    assert_eq!(check_explicit(OpOutcome::RolledBack), Ok(()));
    assert!(matches!(
        check_idempotent(&before(), &after()),
        Err(Violation::NotIdempotent { .. })
    ));
    assert!(!Violation::SilentError.to_string().is_empty());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
