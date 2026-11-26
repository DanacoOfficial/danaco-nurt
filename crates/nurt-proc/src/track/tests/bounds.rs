// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Bounds of change detection: entry cap, time budget, time limit of the run,
//! file system root, other volumes and cancellation.

use std::path::Path;
use std::time::{Duration, Instant};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::Tracking;
use nurt_contracts::report::{Notable, NotableEntry, Severity};
use nurt_contracts::stage::{CancelReason, CancelToken};

use super::support::{DiskFs, Harness, RecordingTx};
use crate::iface::{PreparedScan, TrackMode, TrackOutcome};
use crate::track::Tracker;
use crate::track::scan::{Bound, ScanLimits, Scope, Stop, scan};

/// Directories of the synthetic tree.
const DIRECTORIES: usize = 40;
/// Files in each directory of the synthetic tree.
const FILES: usize = 25;
/// Entries of the synthetic tree below its root: directories and files.
const TREE_ENTRIES: usize = DIRECTORIES + DIRECTORIES * FILES;

fn tree(h: &Harness) {
    for directory in 0..DIRECTORIES {
        for file in 0..FILES {
            h.write(&format!("d{directory:02}/f{file:02}.txt"), "x");
        }
    }
}

fn bounded_scan(
    root: &Path,
    limits: ScanLimits,
    deadline: Option<Instant>,
    cancel: &CancelToken,
) -> (crate::track::scan::Scanned, Option<Stop>) {
    let roots = [AbsPath::new(root).unwrap()];
    let ignore: Vec<String> = Vec::new();
    let bound = Bound::new(limits, deadline, cancel.clone());
    let scanned = scan(
        &DiskFs,
        Scope {
            roots: &roots,
            ignore: &ignore,
            hash: false,
            bound: Some(&bound),
        },
    );
    let stop = bound.stopped();
    (scanned, stop)
}

fn limits(max_entries: usize) -> ScanLimits {
    ScanLimits {
        max_entries,
        budget: None,
        one_volume: true,
    }
}

fn tracker(max_entries: usize) -> Tracker {
    Tracker {
        automatic: limits(max_entries),
    }
}

fn scope_notes(outcome: &TrackOutcome) -> Vec<&NotableEntry> {
    outcome
        .notes
        .iter()
        .filter(|note| note.code == Notable::DetectionScope)
        .collect()
}

#[test]
fn a_scan_without_a_bound_lists_the_whole_tree() {
    let h = Harness::new();
    tree(&h);
    let (scanned, stop) = bounded_scan(&h.root, ScanLimits::UNLIMITED, None, &CancelToken::new());
    assert_eq!(stop, None);
    assert_eq!(scanned.entries.len(), TREE_ENTRIES + 1);
    assert_eq!(scanned.stop, None);
}

#[test]
fn a_scan_stops_at_the_entry_cap_of_a_large_tree() {
    let h = Harness::new();
    tree(&h);
    let cap = 100;
    let (scanned, stop) = bounded_scan(&h.root, limits(cap), None, &CancelToken::new());
    assert_eq!(stop, Some(Stop::Entries));
    assert!(
        scanned.entries.len() < TREE_ENTRIES / 2,
        "{} entries",
        scanned.entries.len()
    );
}

#[test]
fn a_scan_with_an_exhausted_budget_stops_at_once() {
    let h = Harness::new();
    tree(&h);
    let spent = ScanLimits {
        budget: Some(Duration::ZERO),
        ..limits(usize::MAX)
    };
    let (scanned, stop) = bounded_scan(&h.root, spent, None, &CancelToken::new());
    assert_eq!(stop, Some(Stop::Budget));
    assert!(scanned.entries.len() < TREE_ENTRIES / 2);
}

#[test]
fn a_scan_stops_at_the_time_limit_of_the_run() {
    let h = Harness::new();
    tree(&h);
    let (scanned, stop) = bounded_scan(
        &h.root,
        ScanLimits::UNLIMITED,
        Some(Instant::now()),
        &CancelToken::new(),
    );
    assert_eq!(stop, Some(Stop::TimeLimit));
    assert!(scanned.entries.len() < TREE_ENTRIES / 2);
}

#[test]
fn a_scan_stops_when_the_execution_is_cancelled() {
    let h = Harness::new();
    tree(&h);
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Requested);
    let (scanned, stop) = bounded_scan(&h.root, ScanLimits::UNLIMITED, None, &cancel);
    assert_eq!(stop, Some(Stop::Cancelled));
    assert!(scanned.entries.len() < TREE_ENTRIES / 2);
    assert_eq!(scanned.stop, Some(Stop::Cancelled));
}

#[test]
fn the_first_reason_to_stop_wins() {
    let h = Harness::new();
    tree(&h);
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Requested);
    let (_, stop) = bounded_scan(&h.root, limits(1), Some(Instant::now()), &cancel);
    assert!(matches!(stop, Some(Stop::Cancelled | Stop::Entries)));
}

#[cfg(target_os = "linux")]
#[test]
fn a_scan_does_not_enter_another_volume() {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    let sys = Path::new("/sys/fs");
    let cgroup = Path::new("/sys/fs/cgroup");
    let (Ok(parent), Ok(child)) = (fs::metadata(sys), fs::metadata(cgroup)) else {
        return;
    };
    if parent.dev() == child.dev() {
        return;
    }
    let (scanned, _) = bounded_scan(sys, limits(usize::MAX), None, &CancelToken::new());
    assert!(scanned.foreign >= 1);
    assert!(scanned.entries.contains_key(cgroup));
    assert!(
        !scanned
            .entries
            .keys()
            .any(|path| path.starts_with(cgroup) && path != cgroup)
    );
    let unbounded = bounded_scan(sys, ScanLimits::UNLIMITED, None, &CancelToken::new()).0;
    assert!(unbounded.entries.len() >= scanned.entries.len());
}

#[test]
fn automatic_detection_of_a_large_scope_is_skipped_with_a_note() {
    let h = Harness::new();
    tree(&h);
    let mut tx = RecordingTx::new();
    let prepared = tracker(100)
        .capture(&h.ctx, &h.spec(TrackMode::Auto), &mut tx)
        .unwrap();
    assert!(prepared.watch.is_none());
    h.write("d00/new.txt", "created by the program");
    let outcome = Box::new(prepared).finish(&h.ctx, &mut tx).unwrap();
    assert_eq!(outcome.tracking, Tracking::None);
    assert!(outcome.changes.is_empty());
    assert!(tx.observed.is_empty() && tx.declared.is_empty());
    let notes = scope_notes(&outcome);
    assert_eq!(notes.len(), 1);
    let detail = notes[0].detail.as_deref().unwrap();
    assert!(detail.contains("more than 100 entries"), "{detail}");
    assert!(detail.contains("`watch`"), "{detail}");
}

#[test]
fn a_scope_within_the_cap_is_tracked_and_the_note_names_the_directories() {
    let h = Harness::new();
    h.write("a.txt", "a");
    let mut tx = RecordingTx::new();
    let prepared = tracker(100)
        .capture(&h.ctx, &h.spec(TrackMode::Auto), &mut tx)
        .unwrap();
    h.write("b.txt", "b");
    let outcome = Box::new(prepared).finish(&h.ctx, &mut tx).unwrap();
    assert_eq!(h.actions(&outcome).len(), 1);
    let notes = scope_notes(&outcome);
    assert_eq!(notes.len(), 1);
    let detail = notes[0].detail.as_deref().unwrap();
    assert!(detail.contains(&h.root.display().to_string()), "{detail}");
    assert!(detail.contains("elsewhere are not detected"), "{detail}");
}

#[test]
fn a_run_without_detection_has_no_scope_note() {
    let h = Harness::new();
    tree(&h);
    let mut tx = RecordingTx::new();
    let outcome = h.run(&h.spec(TrackMode::None), &mut tx, |h| {
        h.write("d00/new.txt", "x");
    });
    assert!(outcome.notes.is_empty());
    assert_eq!(outcome.tracking, Tracking::None);
}

#[test]
fn explicit_scanning_has_no_entry_cap() {
    let h = Harness::new();
    tree(&h);
    let mut tx = RecordingTx::new();
    let spec = h.spec(TrackMode::Scan);
    let prepared = tracker(10).capture(&h.ctx, &spec, &mut tx).unwrap();
    h.write("d00/new.txt", "x");
    let outcome = Box::new(prepared).finish(&h.ctx, &mut tx).unwrap();
    assert_eq!(outcome.tracking, Tracking::Scan);
    assert_eq!(h.actions(&outcome).len(), 1);
}

#[test]
fn the_file_system_root_is_not_scanned_by_automatic_detection() {
    let h = Harness::new();
    let mut tx = RecordingTx::new();
    let started = Instant::now();
    let spec = crate::iface::TrackSpec {
        watch: vec![Path::new("/").to_path_buf()],
        ..h.spec(TrackMode::Auto)
    };
    let prepared = tracker(usize::MAX).capture(&h.ctx, &spec, &mut tx).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(prepared.watch.is_none());
    assert!(prepared.state.snapshot.is_empty());
    let outcome = Box::new(prepared).finish(&h.ctx, &mut tx).unwrap();
    assert!(outcome.changes.is_empty());
    let notes = scope_notes(&outcome);
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("file system root")
    );
}

#[test]
fn the_time_limit_of_the_run_ends_the_scan_before_the_program() {
    let h = Harness::new();
    tree(&h);
    for mode in [TrackMode::Auto, TrackMode::Scan, TrackMode::Events] {
        let spec = crate::iface::TrackSpec {
            timeout: Some(Duration::ZERO),
            ..h.spec(mode)
        };
        let mut tx = RecordingTx::new();
        let error = Tracker::default()
            .capture(&h.ctx, &spec, &mut tx)
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::Timeout, "{mode:?}");
    }
}

#[test]
fn a_time_limit_that_has_not_ended_does_not_disturb_the_scan() {
    let h = Harness::new();
    h.write("a.txt", "a");
    let spec = crate::iface::TrackSpec {
        timeout: Some(Duration::from_secs(3600)),
        ..h.spec(TrackMode::Scan)
    };
    let mut tx = RecordingTx::new();
    let outcome = h.run(&spec, &mut tx, |h| h.write("b.txt", "b"));
    assert_eq!(h.actions(&outcome).len(), 1);
}

#[test]
fn cancellation_ends_the_scan_before_the_program() {
    let h = Harness::new();
    tree(&h);
    for mode in [TrackMode::Auto, TrackMode::Scan, TrackMode::Events] {
        let (ctx, _) = super::support::context(nurt_contracts::config::Config::default());
        ctx.cancel().cancel(CancelReason::Requested);
        let mut tx = RecordingTx::new();
        let error = Tracker::default()
            .capture(&ctx, &h.spec(mode), &mut tx)
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::Cancelled, "{mode:?}");
    }
}

#[test]
fn cancellation_after_the_program_reports_that_changes_are_unknown() {
    let h = Harness::new();
    h.write("a.txt", "a");
    let mut tx = RecordingTx::new();
    let prepared = h.prepare(&h.spec(TrackMode::Scan), &mut tx);
    h.write("b.txt", "b");
    h.ctx.cancel().cancel(CancelReason::Requested);
    let outcome = prepared.finish(&h.ctx, &mut tx).unwrap();
    assert!(outcome.changes.is_empty());
    assert_eq!(outcome.tracking, Tracking::None);
    let notes = scope_notes(&outcome);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].severity, Severity::Warning);
    let detail = notes[0].detail.as_deref().unwrap();
    assert!(
        detail.contains("not detected") && detail.contains("cancelled"),
        "{detail}"
    );
}

#[test]
fn the_scan_after_the_program_may_outlast_a_time_limit_that_the_program_used_up() {
    let h = Harness::new();
    h.write("a.txt", "a");
    let mut tx = RecordingTx::new();
    let mut prepared = tracker(100)
        .capture(&h.ctx, &h.spec(TrackMode::Scan), &mut tx)
        .unwrap();
    prepared.state.deadline = Some(Instant::now());
    h.write("b.txt", "b");
    let outcome = Box::new(prepared).finish(&h.ctx, &mut tx).unwrap();
    assert_eq!(h.actions(&outcome).len(), 1);
    assert_eq!(outcome.tracking, Tracking::Scan);
}

/// File system that panics when it lists one directory.
struct PanickingFs {
    directory: std::path::PathBuf,
}

impl nurt_contracts::fs::Fs for PanickingFs {
    fn metadata(
        &self,
        path: &AbsPath,
        follow: nurt_contracts::fs::Follow,
    ) -> nurt_contracts::fs::FsResult<nurt_contracts::fs::FileMeta> {
        DiskFs.metadata(path, follow)
    }

    fn extended_metadata(
        &self,
        path: &AbsPath,
        follow: nurt_contracts::fs::Follow,
    ) -> nurt_contracts::fs::FsResult<nurt_contracts::fs::ExtendedMeta> {
        DiskFs.extended_metadata(path, follow)
    }

    fn open_read(
        &self,
        path: &AbsPath,
    ) -> nurt_contracts::fs::FsResult<Box<dyn nurt_contracts::fs::ReadHandle>> {
        DiskFs.open_read(path)
    }

    fn open_stream_read(
        &self,
        path: &AbsPath,
        stream: &str,
    ) -> nurt_contracts::fs::FsResult<Box<dyn nurt_contracts::fs::ReadHandle>> {
        DiskFs.open_stream_read(path, stream)
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> nurt_contracts::fs::FsResult<
        Box<
            dyn Iterator<Item = nurt_contracts::fs::FsResult<nurt_contracts::fs::DirEntry>>
                + Send
                + 'a,
        >,
    > {
        assert!(path.as_path() != self.directory, "listing failed");
        DiskFs.read_dir(path)
    }

    fn read_link(&self, path: &AbsPath) -> nurt_contracts::fs::FsResult<std::path::PathBuf> {
        DiskFs.read_link(path)
    }

    fn real_path(&self, path: &AbsPath) -> nurt_contracts::fs::FsResult<AbsPath> {
        DiskFs.real_path(path)
    }

    fn content_hash(
        &self,
        path: &AbsPath,
    ) -> nurt_contracts::fs::FsResult<nurt_contracts::fs::ContentHash> {
        DiskFs.content_hash(path)
    }

    fn volume(
        &self,
        path: &AbsPath,
    ) -> nurt_contracts::fs::FsResult<nurt_contracts::fs::VolumeInfo> {
        DiskFs.volume(path)
    }

    fn space(
        &self,
        volume: &nurt_contracts::fs::VolumeId,
    ) -> nurt_contracts::fs::FsResult<nurt_contracts::fs::SpaceInfo> {
        DiskFs.space(volume)
    }

    fn path_key(
        &self,
        path: &AbsPath,
    ) -> nurt_contracts::fs::FsResult<nurt_contracts::fs::PathKey> {
        DiskFs.path_key(path)
    }

    fn holders(
        &self,
        path: &AbsPath,
    ) -> nurt_contracts::fs::FsResult<Vec<nurt_contracts::fs::Holder>> {
        DiskFs.holders(path)
    }

    fn can_write_dir(&self, dir: &AbsPath) -> nurt_contracts::fs::FsResult<bool> {
        DiskFs.can_write_dir(dir)
    }
}

#[test]
fn a_listing_that_panics_ends_the_scan_instead_of_blocking_the_other_threads() {
    let h = Harness::new();
    tree(&h);
    let fs = PanickingFs {
        directory: h.root.join("d07"),
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let root = h.root.clone();
    std::thread::spawn(move || {
        let roots = [AbsPath::new(&root).unwrap()];
        let ignore: Vec<String> = Vec::new();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan(
                &fs,
                Scope {
                    roots: &roots,
                    ignore: &ignore,
                    hash: false,
                    bound: None,
                },
            )
        }));
        let _ = sender.send(outcome.is_err());
    });
    let panicked = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("the scan did not end after a panic in a listing");
    assert!(panicked);
}

#[test]
fn the_listing_of_files_to_keep_ends_when_the_execution_is_cancelled() {
    let h = Harness::new();
    tree(&h);
    let roots = [AbsPath::new(&h.root).unwrap()];
    let cancel = CancelToken::new();
    let unbounded = Bound::new(ScanLimits::UNLIMITED, None, cancel.clone());
    let all = crate::track::files_under(&DiskFs, &roots, &[], &unbounded);
    assert_eq!(all.len(), DIRECTORIES * FILES);
    assert_eq!(unbounded.stopped(), None);
    cancel.cancel(CancelReason::Requested);
    let bound = Bound::new(ScanLimits::UNLIMITED, None, cancel);
    let listed = crate::track::files_under(&DiskFs, &roots, &[], &bound);
    assert!(listed.len() < all.len());
    assert_eq!(bound.stopped(), Some(Stop::Cancelled));
}
