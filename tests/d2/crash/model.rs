// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/model.rs
// Responsibility:   Persistence model of `CrashFs`.
// Dependencies:     nurt_contracts, nurt_crashfs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Persistence model of `CrashFs`.

use std::collections::BTreeSet;
use std::io::Write;

use nurt_contracts::fs::{Durability, FsErrorKind, FsWrite, RenameMode};
use nurt_crashfs::crash::{CrashFs, CrashProfile};
use nurt_crashfs::oracle::TreeSnapshot;

use crate::support::{
    SEEDS, at, content, durable_file, exists, new_fs, overwrite, scope, volume_root,
};

const OLD: &[u8] = b"previous content";
const NEW: &[u8] = b"replacement data";

/// Outcomes of `scenario` over the seeds `0..SEEDS` on `profile`.
fn outcomes<T: Ord>(profile: CrashProfile, scenario: impl Fn(&CrashFs) -> T) -> BTreeSet<T> {
    (0..SEEDS)
        .map(|seed| scenario(&new_fs(profile, seed)))
        .collect()
}

#[test]
fn crashfs_model_loses_unsynced_data() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        durable_file(fs, &at("f"), OLD);
        overwrite(fs, &at("f"), NEW).unwrap();
        fs.crash();
        content(fs, &at("f")).unwrap()
    });
    assert!(
        seen.contains(OLD),
        "an unflushed write survived every crash"
    );
    for observed in &seen {
        let new_prefix = observed.iter().zip(NEW).take_while(|(a, b)| a == b).count();
        assert_eq!(
            observed[new_prefix..],
            OLD[new_prefix..],
            "content is neither the old data nor a torn new write"
        );
    }
}

#[test]
fn crashfs_model_keeps_flushed_data() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        durable_file(fs, &at("f"), OLD);
        let mut file = fs.open_in_place(&scope(), &at("f")).unwrap();
        file.write_all(NEW).unwrap();
        file.sync(Durability::Data).unwrap();
        fs.crash();
        content(fs, &at("f")).unwrap()
    });
    assert_eq!(seen, BTreeSet::from([NEW.to_vec()]));
}

#[test]
fn crashfs_model_loses_directory_entry_without_directory_sync() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        let mut file = fs.create_new(&scope(), &at("f")).unwrap();
        file.write_all(OLD).unwrap();
        file.sync(Durability::Full).unwrap();
        fs.crash();
        exists(fs, &at("f"))
    });
    assert_eq!(seen, BTreeSet::from([false, true]));
}

#[test]
fn crashfs_model_keeps_entry_after_directory_sync() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        durable_file(fs, &at("f"), OLD);
        fs.crash();
        content(fs, &at("f"))
    });
    assert_eq!(seen, BTreeSet::from([Some(OLD.to_vec())]));
}

#[test]
fn crashfs_model_rename_is_atomic() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        durable_file(fs, &at("a"), OLD);
        durable_file(fs, &at("b"), NEW);
        fs.rename(&scope(), &at("a"), &at("b"), RenameMode::Replace)
            .unwrap();
        fs.crash();
        (content(fs, &at("a")), content(fs, &at("b")))
    });
    let before = (Some(OLD.to_vec()), Some(NEW.to_vec()));
    let after = (None, Some(OLD.to_vec()));
    assert_eq!(seen, BTreeSet::from([before, after]));
}

#[test]
fn crashfs_model_rename_between_directories_needs_both_flushes() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        let dir = at("d");
        fs.create_dir(&scope(), &dir, 0o755).unwrap();
        fs.sync_dir(&scope(), &volume_root()).unwrap();
        durable_file(fs, &at("a"), OLD);
        let target = dir.join_name("a").unwrap();
        fs.rename(&scope(), &at("a"), &target, RenameMode::NoReplace)
            .unwrap();
        fs.sync_dir(&scope(), &dir).unwrap();
        fs.crash();
        (exists(fs, &at("a")), exists(fs, &target))
    });
    assert_eq!(seen, BTreeSet::from([(false, true), (true, false)]));
}

#[test]
fn crashfs_model_hard_link_may_keep_both_names() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        durable_file(fs, &at("a"), OLD);
        fs.hard_link(&scope(), &at("a"), &at("b")).unwrap();
        fs.remove_file(&scope(), &at("a")).unwrap();
        fs.crash();
        (exists(fs, &at("a")), exists(fs, &at("b")))
    });
    assert!(
        seen.contains(&(true, true)),
        "a hard link never left both names: {seen:?}"
    );
}

#[test]
fn crashfs_model_tears_writes_to_a_prefix() {
    const LENGTH: usize = 4096;
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        durable_file(fs, &at("f"), b"");
        overwrite(fs, &at("f"), &[0xA5; LENGTH]).unwrap();
        fs.crash();
        content(fs, &at("f")).unwrap().len()
    });
    assert!(seen.contains(&0));
    assert!(seen.contains(&LENGTH));
    assert!(
        seen.iter().any(|length| *length > 0 && *length < LENGTH),
        "no torn write in {seen:?}"
    );
}

#[test]
fn crashfs_model_replays_from_seed() {
    let run = |seed| {
        let fs = new_fs(CrashProfile::PosixWorstCase, seed);
        durable_file(&fs, &at("a"), OLD);
        overwrite(&fs, &at("a"), NEW).unwrap();
        fs.create_dir(&scope(), &at("d"), 0o700).unwrap();
        durable_file(&fs, &at("d").join_name("x").unwrap(), NEW);
        fs.rename(&scope(), &at("a"), &at("b"), RenameMode::Replace)
            .unwrap();
        let summary = fs.crash();
        (
            summary,
            TreeSnapshot::capture(&fs, &volume_root(), &[]).unwrap(),
        )
    };
    assert_eq!(run(11), run(11));
    let images: BTreeSet<String> = (0..SEEDS)
        .map(|seed| format!("{:?}", run(seed).1))
        .collect();
    assert!(images.len() > 1, "every seed produced the same image");
}

#[test]
fn crashfs_model_stops_at_armed_call() {
    let fs = new_fs(CrashProfile::PosixWorstCase, 3);
    durable_file(&fs, &at("a"), OLD);
    let next = fs.stats().calls + 1;
    fs.arm_crash_at_call(next);
    let err = fs.create_dir(&scope(), &at("d"), 0o755).unwrap_err();
    assert_eq!(err.kind, FsErrorKind::Io);
    assert!(fs.is_halted());
    assert_eq!(
        fs.remove_file(&scope(), &at("a")).unwrap_err().kind,
        FsErrorKind::Io
    );
    fs.crash();
    assert!(!fs.is_halted());
    assert!(!exists(&fs, &at("d")));
    assert_eq!(content(&fs, &at("a")).unwrap(), OLD);
}

#[test]
fn crashfs_model_invalidates_handles_after_crash() {
    let fs = new_fs(CrashProfile::PosixWorstCase, 5);
    durable_file(&fs, &at("a"), OLD);
    let mut file = fs.open_in_place(&scope(), &at("a")).unwrap();
    fs.crash();
    assert!(file.write_all(NEW).is_err());
    assert_eq!(
        file.sync(Durability::Full).unwrap_err().kind,
        FsErrorKind::Io
    );
}

#[test]
fn crashfs_model_btrfs_never_tears_writes() {
    let seen = outcomes(CrashProfile::Btrfs, |fs| {
        durable_file(fs, &at("f"), b"");
        overwrite(fs, &at("f"), &[1; 2048]).unwrap();
        fs.crash();
        content(fs, &at("f")).unwrap().len()
    });
    assert_eq!(seen, BTreeSet::from([0, 2048]));
}

#[test]
fn crashfs_model_journaled_profiles_keep_metadata_order() {
    for profile in [
        CrashProfile::Ext4Ordered,
        CrashProfile::Xfs,
        CrashProfile::Ntfs,
        CrashProfile::Apfs,
    ] {
        let seen = outcomes(profile, |fs| {
            fs.create_dir(&scope(), &at("1"), 0o755).unwrap();
            fs.create_dir(&scope(), &at("2"), 0o755).unwrap();
            fs.create_dir(&scope(), &at("3"), 0o755).unwrap();
            fs.crash();
            ["1", "2", "3"].map(|name| exists(fs, &at(name)))
        });
        for state in &seen {
            let survived = state.iter().take_while(|present| **present).count();
            assert!(
                state[survived..].iter().all(|present| !present),
                "{profile:?}: {state:?}"
            );
        }
        assert!(seen.len() > 1, "{profile:?}");
    }
}

#[test]
fn crashfs_model_file_flush_commits_metadata_on_journaled_profiles() {
    let seen = outcomes(CrashProfile::Ext4Ordered, |fs| {
        let mut file = fs.create_new(&scope(), &at("f")).unwrap();
        file.write_all(OLD).unwrap();
        file.sync(Durability::Data).unwrap();
        fs.crash();
        content(fs, &at("f"))
    });
    assert_eq!(seen, BTreeSet::from([Some(OLD.to_vec())]));
}

#[test]
fn crashfs_model_apfs_requires_full_flush_for_data() {
    let data_only = outcomes(CrashProfile::Apfs, |fs| {
        durable_file(fs, &at("f"), OLD);
        let mut file = fs.open_in_place(&scope(), &at("f")).unwrap();
        file.write_all(NEW).unwrap();
        file.sync(Durability::Data).unwrap();
        fs.crash();
        content(fs, &at("f")).unwrap() == NEW
    });
    assert!(data_only.contains(&false));
    let full = outcomes(CrashProfile::Apfs, |fs| {
        durable_file(fs, &at("f"), OLD);
        let mut file = fs.open_in_place(&scope(), &at("f")).unwrap();
        file.write_all(NEW).unwrap();
        file.sync(Durability::Full).unwrap();
        fs.crash();
        content(fs, &at("f")).unwrap() == NEW
    });
    assert_eq!(full, BTreeSet::from([true]));
}

#[test]
fn crashfs_model_counts_copied_bytes_but_not_renames() {
    let fs = new_fs(CrashProfile::PosixWorstCase, 1);
    durable_file(&fs, &at("big"), &[7; 10_000]);
    fs.reset_stats();
    fs.rename(&scope(), &at("big"), &at("moved"), RenameMode::NoReplace)
        .unwrap();
    assert_eq!(fs.stats().bytes_copied, 0);
    assert_eq!(fs.stats().renames, 1);
    assert_eq!(
        fs.copy_file(&scope(), &at("moved"), &at("copy")).unwrap(),
        10_000
    );
    fs.clone_file(&scope(), &at("moved"), &at("clone")).unwrap();
    let stats = fs.stats();
    assert_eq!(stats.bytes_copied, 10_000);
    assert_eq!(stats.clones, 1);
    assert_eq!(content(&fs, &at("clone")).unwrap(), vec![7; 10_000]);
}

#[test]
fn crashfs_model_sync_all_makes_everything_durable() {
    let seen = outcomes(CrashProfile::PosixWorstCase, |fs| {
        fs.create_dir(&scope(), &at("d"), 0o755).unwrap();
        let mut file = fs
            .create_new(&scope(), &at("d").join_name("f").unwrap())
            .unwrap();
        file.write_all(NEW).unwrap();
        fs.sync_all();
        assert_eq!(fs.pending_count(), 0);
        fs.crash();
        content(fs, &at("d").join_name("f").unwrap())
    });
    assert_eq!(seen, BTreeSet::from([Some(NEW.to_vec())]));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
