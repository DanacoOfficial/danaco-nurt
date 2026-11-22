// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Keeping previous content before a run: clone, copy, budget and limits.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::config::{Config, DurationSetting, PreimageMode, SizeSetting};
use nurt_contracts::journal::PreserveMode;
use nurt_contracts::record::Preimage;
use nurt_contracts::report::Irreversible;

use super::support::{Harness, RecordingTx};
use crate::iface::{TrackMode, TrackSpec};

/// Number of files in the budget test; more than one declaration batch.
const MANY_FILES: usize = 100;

/// Files declared in one batch before the budget is checked again.
const BATCH: u64 = 64;

fn config(change: impl FnOnce(&mut Config)) -> Config {
    let mut config = Config::default();
    change(&mut config);
    config
}

#[test]
fn copied_content_is_restored_after_undo() {
    let h = Harness::new();
    let binary: String = (0u8..=255).map(char::from).collect();
    h.write("data/blob.bin", &binary);
    h.write("data/text.txt", "line one\nline two\n");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write("data/blob.bin", "overwritten");
        std::fs::remove_file(h.path("data/text.txt")).unwrap();
    });
    assert_eq!(outcome.preimage, Preimage::Copy);
    assert_eq!(outcome.preimage_entries, 2);
    assert_eq!(
        outcome.preimage_bytes,
        (binary.len() + "line one\nline two\n".len()) as u64
    );
    let blob = h.change(&outcome, "data/blob.bin");
    assert_eq!(blob.store.as_deref(), Some("copy"));
    assert!(blob.hash_before.is_some());
    assert!(blob.hash_after.is_some());
    tx.undo();
    assert_eq!(h.read("data/blob.bin"), binary);
    assert_eq!(h.read("data/text.txt"), "line one\nline two\n");
}

#[test]
fn cloned_content_is_reported_as_clone_and_restored() {
    let h = Harness::with_config(config(|config| {
        config.run.preimage_copy_max = SizeSetting::from_bytes(4);
    }));
    for name in ["a", "b", "c", "d"] {
        h.write(name, "0123");
    }
    let mut tx = RecordingTx::with_mode(PreserveMode::Clone);
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| h.write("d", "changed"));
    assert_eq!(outcome.preimage, Preimage::Clone);
    assert_eq!(outcome.preimage_entries, 4);
    assert_eq!(h.change(&outcome, "d").store.as_deref(), Some("clone"));
    tx.undo();
    assert_eq!(h.read("d"), "0123");
}

#[test]
fn budget_stops_keeping_and_marks_the_rest() {
    let h = Harness::with_config(config(|config| {
        config.run.preimage_budget = DurationSetting::from_millis(100);
    }));
    for index in 0..MANY_FILES {
        h.write(&format!("f{index:03}"), "content");
    }
    let mut tx = RecordingTx::new();
    tx.clock = Some(Arc::clone(&h.clock));
    tx.delay = Duration::from_millis(30);
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write("f000", "first changed");
        h.write("f099", "last changed");
    });
    assert_eq!(outcome.preimage, Preimage::Partial);
    assert_eq!(outcome.preimage_entries, BATCH);
    let first = h.change(&outcome, "f000");
    assert_eq!(first.store.as_deref(), Some("copy"));
    assert_eq!(first.irreversible, None);
    let last = h.change(&outcome, "f099");
    assert_eq!(last.store.as_deref(), Some("none"));
    assert_eq!(last.irreversible, Some(Irreversible::PreviousContentNotKept));
    assert_eq!(last.flags, Some(vec!["preimage_budget".to_owned()]));
}

#[test]
fn entry_limit_marks_files_without_copy() {
    let h = Harness::with_config(config(|config| {
        config.run.preimage_max_entries = 3;
    }));
    for name in ["a", "b", "c", "d", "e"] {
        h.write(name, "x");
    }
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write("a", "changed");
        h.write("e", "changed");
    });
    assert_eq!(tx.declared.len(), 3);
    assert_eq!(outcome.preimage, Preimage::Partial);
    assert_eq!(h.change(&outcome, "a").irreversible, None);
    assert_eq!(
        h.change(&outcome, "e").flags,
        Some(vec!["preimage_limit".to_owned()])
    );
}

#[test]
fn copy_limit_marks_files_without_copy() {
    let h = Harness::with_config(config(|config| {
        config.run.preimage_copy_max = SizeSetting::from_bytes(10);
    }));
    for name in ["a", "b", "c", "d"] {
        h.write(name, "1234");
    }
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| h.write("c", "changed"));
    assert_eq!(tx.declared.len(), 2);
    assert_eq!(outcome.preimage_bytes, 8);
    assert_eq!(
        h.change(&outcome, "c").flags,
        Some(vec!["preimage_limit".to_owned()])
    );
}

#[test]
fn excluded_names_are_not_kept() {
    let h = Harness::new();
    h.write("node_modules/pkg/index.js", "module.exports = 1");
    h.write("src/main.js", "main");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| {
        h.write("node_modules/pkg/index.js", "module.exports = 2");
    });
    assert_eq!(tx.declared.len(), 1);
    let change = h.change(&outcome, "node_modules/pkg/index.js");
    assert_eq!(change.store.as_deref(), Some("none"));
    assert_eq!(change.flags, Some(vec!["preimage_excluded".to_owned()]));
    assert_eq!(outcome.preimage, Preimage::Copy);
}

#[test]
fn explicit_paths_are_kept_regardless_of_limits() {
    let h = Harness::with_config(config(|config| {
        config.run.preimage_max_entries = 0;
    }));
    h.write("important/state.json", "{\"v\":1}");
    h.write("other.txt", "other");
    let spec = TrackSpec {
        preserve: vec![h.path("important")],
        ..h.spec(TrackMode::Scan)
    };
    let mut tx = RecordingTx::new();
    let outcome = h.run(&spec, &mut tx, |h| {
        h.write("important/state.json", "{\"v\":2}");
        h.write("other.txt", "changed");
    });
    assert_eq!(
        h.change(&outcome, "important/state.json").store.as_deref(),
        Some("copy")
    );
    assert_eq!(
        h.change(&outcome, "other.txt").flags,
        Some(vec!["preimage_limit".to_owned()])
    );
    tx.undo();
    assert_eq!(h.read("important/state.json"), "{\"v\":1}");
}

#[test]
fn disabled_preimage_marks_changes_without_flag() {
    let h = Harness::with_config(config(|config| {
        config.run.preimage = PreimageMode::Off;
    }));
    h.write("a.txt", "alpha");
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::Scan), &mut tx, |h| h.write("a.txt", "changed"));
    assert_eq!(tx.preserve_calls, 0);
    assert_eq!(outcome.preimage, Preimage::None);
    let change = h.change(&outcome, "a.txt");
    assert_eq!(change.irreversible, Some(Irreversible::PreviousContentNotKept));
    assert_eq!(change.flags, None);
    assert_eq!(tx.declared_indexes(), vec![None]);
}
