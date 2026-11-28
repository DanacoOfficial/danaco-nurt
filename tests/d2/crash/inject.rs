// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/inject.rs
// Responsibility:   Error injection over `CrashFs`.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Error injection over `CrashFs`.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nurt_contracts::fs::{Durability, Fs, FsErrorKind, FsWrite, RenameMode};
use nurt_crashfs::crash::{CrashFs, CrashProfile};
use nurt_crashfs::inject::{
    Call, Fault, INJECTED_KINDS, InjectFs, InjectPlan, InjectRule, os_code,
};
use nurt_crashfs::oracle::OpOutcome;
use nurt_crashfs::points::Phase;
use nurt_fs::failpoint::NoFailpoints;

use crate::reference;
use crate::support::{at, content, durable_file, exists, new_fs, scope};

fn prepared() -> CrashFs {
    let fs = new_fs(CrashProfile::PosixWorstCase, 1);
    reference::setup(&fs).unwrap();
    fs.sync_all();
    fs
}

fn calls_of_reference_run() -> Vec<(Call, u64)> {
    let fs = InjectFs::new(prepared(), InjectPlan::default());
    let outcome = reference::replace(&fs, &NoFailpoints, true);
    assert_eq!(
        outcome,
        OpOutcome::Committed {
            irreversible: false
        }
    );
    Call::ALL
        .into_iter()
        .map(|call| (call, fs.calls(call)))
        .filter(|(_, count)| *count > 0)
        .collect()
}

#[test]
fn inject_errors_fail_the_call_without_effect() {
    for kind in INJECTED_KINDS {
        let plan = InjectPlan {
            rules: vec![InjectRule::error_at(Call::CreateDir, 0, kind)],
            ..InjectPlan::default()
        };
        let fs = InjectFs::new(new_fs(CrashProfile::PosixWorstCase, 1), plan);
        let err = fs.create_dir(&scope(), &at("d"), 0o755).unwrap_err();
        assert_eq!(err.kind, kind);
        assert_eq!(err.os_code, os_code(kind));
        assert!(!exists(&fs, &at("d")));
        assert_eq!(fs.injected().len(), 1);
    }
}

#[test]
fn inject_errors_every_kind_at_every_step_of_the_reference_protocol() {
    let mut runs = 0;
    for (call, count) in calls_of_reference_run() {
        for occurrence in 0..count {
            for kind in INJECTED_KINDS {
                let occurrence = u32::try_from(occurrence).unwrap();
                let plan = InjectPlan {
                    rules: vec![InjectRule::error_at(call, occurrence, kind)],
                    ..InjectPlan::default()
                };
                let fs = InjectFs::new(prepared(), plan);
                let outcome = reference::replace(&fs, &NoFailpoints, true);
                assert!(
                    matches!(outcome, OpOutcome::RolledBack | OpOutcome::Committed { .. }),
                    "{call:?}#{occurrence} {kind:?}: {outcome:?}"
                );
                reference::check_state(fs.inner(), Phase::AfterRecovery, outcome).unwrap_or_else(
                    |violation| panic!("{call:?}#{occurrence} {kind:?}: {violation}"),
                );
                runs += 1;
            }
        }
    }
    assert!(runs >= 10 * INJECTED_KINDS.len());
}

#[test]
fn inject_errors_short_transfers_keep_content() {
    let base = new_fs(CrashProfile::PosixWorstCase, 1);
    durable_file(&base, &at("f"), b"0123456789");
    let plan = InjectPlan {
        rules: vec![
            InjectRule {
                call: Some(Call::Write),
                below: None,
                occurrence: None,
                fault: Fault::ShortWrite { max: 3 },
            },
            InjectRule {
                call: Some(Call::Read),
                below: None,
                occurrence: None,
                fault: Fault::ShortRead { max: 2 },
            },
        ],
        ..InjectPlan::default()
    };
    let fs = InjectFs::new(base, plan);
    let mut file = fs.create_new(&scope(), &at("g")).unwrap();
    file.write_all(b"abcdefghij").unwrap();
    file.sync(Durability::Data).unwrap();
    let mut handle = fs.open_read(&at("f")).unwrap();
    let mut buffer = [0_u8; 8];
    assert_eq!(handle.read(&mut buffer).unwrap(), 2);
    assert_eq!(content(&fs, &at("g")).unwrap(), b"abcdefghij");
    assert!(fs.injected().len() >= 4);
}

#[test]
fn inject_errors_delays_reach_the_clock() {
    let elapsed = Arc::new(AtomicU64::new(0));
    let sink = Arc::clone(&elapsed);
    let plan = InjectPlan {
        rules: vec![InjectRule {
            call: Some(Call::SyncDir),
            below: None,
            occurrence: None,
            fault: Fault::Delay { millis: 250 },
        }],
        ..InjectPlan::default()
    };
    let fs = InjectFs::with_delay(
        new_fs(CrashProfile::PosixWorstCase, 1),
        plan,
        Some(Arc::new(move |delay: std::time::Duration| {
            sink.fetch_add(u64::try_from(delay.as_millis()).unwrap(), Ordering::SeqCst);
        })),
    );
    durable_file(fs.inner(), &at("x"), b"x");
    fs.sync_dir(&scope(), &crate::support::volume_root())
        .unwrap();
    fs.sync_dir(&scope(), &crate::support::volume_root())
        .unwrap();
    assert_eq!(elapsed.load(Ordering::SeqCst), 500);
}

#[test]
fn inject_errors_overlay_lower_layer_rename_is_cross_device() {
    let base = new_fs(CrashProfile::PosixWorstCase, 1);
    durable_file(&base, &at("lower"), b"from the image");
    let plan = InjectPlan {
        rules: vec![InjectRule {
            call: Some(Call::Rename),
            below: Some(at("lower").as_path().to_path_buf()),
            occurrence: None,
            fault: Fault::Error {
                kind: FsErrorKind::CrossDevice,
            },
        }],
        ..InjectPlan::default()
    };
    let fs = InjectFs::new(base, plan);
    let err = fs
        .rename(&scope(), &at("lower"), &at("moved"), RenameMode::NoReplace)
        .unwrap_err();
    assert_eq!(err.kind, FsErrorKind::CrossDevice);
    assert_eq!(
        fs.copy_file(&scope(), &at("lower"), &at("copy")).unwrap(),
        14
    );
}

#[test]
fn inject_errors_random_plan_is_reproducible() {
    let run = || {
        let plan = InjectPlan {
            seed: 77,
            rules: Vec::new(),
            error_per_million: 300_000,
            error_kinds: INJECTED_KINDS.to_vec(),
        };
        let fs = InjectFs::new(prepared(), plan);
        let outcome = reference::replace(&fs, &NoFailpoints, true);
        (outcome, fs.injected())
    };
    let first = run();
    assert!(
        !first.1.is_empty(),
        "the random plan injected no fault, so the comparison proves nothing"
    );
    assert_eq!(first, run());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
