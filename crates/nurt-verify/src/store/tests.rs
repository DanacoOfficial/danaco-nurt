// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the history store.

use nurt_contracts::record::{TestResult, TestRun};
use nurt_contracts::value::{Record, Value};

use super::DirStore;
use crate::iface::{EntryKind, StoredEntry};

fn entry(id: &str, tests: &[&str]) -> StoredEntry {
    StoredEntry {
        id: id.to_owned(),
        header: Record::from(TestRun {
            id: id.to_owned(),
            ..TestRun::default()
        }),
        items: tests
            .iter()
            .map(|name| {
                Record::from(TestResult {
                    run: id.to_owned(),
                    id: (*name).to_owned(),
                    name: (*name).to_owned(),
                    ..TestResult::default()
                })
            })
            .collect(),
    }
}

#[test]
fn next_id_follows_the_highest_stored_number() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    assert_eq!(store.next_id(EntryKind::TestRun), "run-1");
    store
        .save(EntryKind::TestRun, &entry("run-1", &[]))
        .unwrap();
    store
        .save(EntryKind::TestRun, &entry("run-7", &[]))
        .unwrap();
    assert_eq!(store.next_id(EntryKind::TestRun), "run-8");
    assert_eq!(store.next_id(EntryKind::Analysis), "an-1");
}

#[test]
fn save_then_load_returns_header_and_items_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    let saved = entry("run-1", &["b", "a"]);
    store.save(EntryKind::TestRun, &saved).unwrap();
    assert_eq!(
        store.load(EntryKind::TestRun, "run-1").unwrap(),
        Some(saved)
    );
}

#[test]
fn load_of_an_unknown_entry_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    assert_eq!(store.load(EntryKind::TestRun, "run-9").unwrap(), None);
}

#[test]
fn save_removes_the_oldest_entries_beyond_the_history_size() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 2);
    for number in 1..=4 {
        store
            .save(EntryKind::TestRun, &entry(&format!("run-{number}"), &[]))
            .unwrap();
    }
    assert_eq!(store.ids(EntryKind::TestRun), ["run-4", "run-3"]);
    assert_eq!(store.load(EntryKind::TestRun, "run-1").unwrap(), None);
}

#[test]
fn history_size_is_kept_per_kind() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 1);
    store
        .save(EntryKind::TestRun, &entry("run-1", &[]))
        .unwrap();
    store
        .save(EntryKind::Analysis, &entry("an-1", &[]))
        .unwrap();
    assert_eq!(store.ids(EntryKind::TestRun), ["run-1"]);
    assert_eq!(store.ids(EntryKind::Analysis), ["an-1"]);
}

#[test]
fn list_returns_newest_headers_first_up_to_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    for number in 1..=3 {
        store
            .save(EntryKind::TestRun, &entry(&format!("run-{number}"), &[]))
            .unwrap();
    }
    let listing = store.list(EntryKind::TestRun, 2).unwrap();
    assert!(listing.skipped.is_empty());
    let ids: Vec<&str> = listing
        .headers
        .iter()
        .filter_map(|header| header.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, ["run-3", "run-2"]);
}

#[test]
fn save_replaces_an_entry_with_the_same_identifier() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    store
        .save(EntryKind::TestRun, &entry("run-1", &["a"]))
        .unwrap();
    store
        .save(EntryKind::TestRun, &entry("run-1", &["b"]))
        .unwrap();
    let loaded = store.load(EntryKind::TestRun, "run-1").unwrap().unwrap();
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(store.ids(EntryKind::TestRun).len(), 1);
}

#[test]
fn next_id_does_not_hand_out_an_identifier_twice_before_its_entry_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    let first = store.next_id(EntryKind::TestRun);
    let second = store.next_id(EntryKind::TestRun);
    assert_eq!((first.as_str(), second.as_str()), ("run-1", "run-2"));
    store
        .save(EntryKind::TestRun, &entry(&first, &["a"]))
        .unwrap();
    store
        .save(EntryKind::TestRun, &entry(&second, &["b"]))
        .unwrap();
    assert_eq!(store.ids(EntryKind::TestRun), ["run-2", "run-1"]);
    assert_eq!(store.next_id(EntryKind::TestRun), "run-3");
    assert!(!dir.path().join("verify/runs/run-1.rsv").exists());
}

#[test]
fn reservations_behind_the_oldest_entry_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    let abandoned = store.next_id(EntryKind::TestRun);
    let saved = store.next_id(EntryKind::TestRun);
    store.save(EntryKind::TestRun, &entry(&saved, &[])).unwrap();
    let reservation = dir.path().join(format!("verify/runs/{abandoned}.rsv"));
    assert!(!reservation.exists());
    assert_eq!(store.next_id(EntryKind::TestRun), "run-3");
}

#[test]
fn load_refuses_identifiers_that_name_other_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    store
        .save(EntryKind::TestRun, &entry("run-1", &["a"]))
        .unwrap();
    let outside = dir.path().join("verify/outside.json");
    std::fs::copy(dir.path().join("verify/runs/run-1.json"), &outside).unwrap();
    let absolute = dir.path().join("verify/outside");
    for id in [
        "../outside",
        "run-",
        "run-x",
        "run-1x",
        "an-1",
        "",
        absolute.to_str().unwrap(),
    ] {
        assert_eq!(store.load(EntryKind::TestRun, id).unwrap(), None, "{id}");
    }
    assert!(store.load(EntryKind::TestRun, "run-1").unwrap().is_some());
}

#[test]
fn pruned_runs_lose_their_reports_and_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 1);
    let reports = dir.path().join("verify/reports");
    std::fs::create_dir_all(reports.join("run-1-coverage")).unwrap();
    for name in [
        "run-1.xml",
        "run-1.json",
        "run-1.coverage",
        "run-1-coverage/lcov.info",
        "run-2.xml",
        "other.txt",
    ] {
        std::fs::write(reports.join(name), "x").unwrap();
    }
    store
        .save(EntryKind::TestRun, &entry("run-1", &[]))
        .unwrap();
    store
        .save(EntryKind::TestRun, &entry("run-2", &[]))
        .unwrap();
    for name in [
        "run-1.xml",
        "run-1.json",
        "run-1.coverage",
        "run-1-coverage",
    ] {
        assert!(!reports.join(name).exists(), "{name}");
    }
    assert!(reports.join("run-2.xml").exists());
    assert!(reports.join("other.txt").exists());
}

#[test]
fn list_leaves_out_a_corrupt_entry_and_reports_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    for number in 1..=3 {
        store
            .save(EntryKind::TestRun, &entry(&format!("run-{number}"), &[]))
            .unwrap();
    }
    let damaged = dir.path().join("verify/runs/run-2.json");
    std::fs::write(&damaged, "{").unwrap();
    let listing = store.list(EntryKind::TestRun, 10).unwrap();
    let ids: Vec<&str> = listing
        .headers
        .iter()
        .filter_map(|header| header.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, ["run-3", "run-1"]);
    assert_eq!(listing.skipped.len(), 1);
    assert_eq!(listing.skipped[0].path, damaged);
    assert!(!listing.skipped[0].error.data().fix.is_empty());
    assert!(store.load(EntryKind::TestRun, "run-2").is_err());
}

#[test]
fn load_reports_a_corrupt_entry_with_a_fix() {
    let dir = tempfile::tempdir().unwrap();
    let store = DirStore::new(dir.path(), 50);
    store
        .save(EntryKind::TestRun, &entry("run-1", &[]))
        .unwrap();
    let path = dir.path().join("verify/runs/run-1.json");
    std::fs::write(path, "{").unwrap();
    let error = store.load(EntryKind::TestRun, "run-1").unwrap_err();
    assert!(!error.data().fix.is_empty());
}
