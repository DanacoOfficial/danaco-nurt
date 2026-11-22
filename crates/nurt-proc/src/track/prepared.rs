// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! State captured before a run and its comparison after the run.

use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{MutationTx, PreserveMode};
use nurt_contracts::record::{Preimage as KeptContent, Tracking};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;

use super::diff;
use super::journal::Recorder;
use super::preimage::Preimage;
use super::scan::{Scope, Snapshot, has_name, refresh, scan};
use super::watch_events::EventWatch;
use crate::iface::{PreparedScan, TrackOutcome};

/// Flag of a changed file below a name of `run.preimage_exclude`.
const FLAG_EXCLUDED: &str = "preimage_excluded";

/// How changes are detected after the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Detection {
    /// Not detected.
    None,
    /// By a second scan.
    Scan,
    /// By file system events, with a second scan when events were lost.
    Events,
}

/// State captured before a run. The job thread that prepares a run also
/// finishes it, so the state never leaves the process.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct State {
    /// Detection mechanism.
    pub(super) detection: Detection,
    /// Scanned directories and files.
    pub(super) roots: Vec<AbsPath>,
    /// Watched directories as reported.
    pub(super) watch: Vec<PathBuf>,
    /// Names excluded from detection.
    pub(super) ignore: Vec<String>,
    /// Names excluded from keeping previous content.
    pub(super) exclude: Vec<String>,
    /// Previous content of watched files was kept within the limits.
    pub(super) preimage_enabled: bool,
    /// Content hashes are compared.
    pub(super) scan_hash: bool,
    /// State before the run.
    pub(super) snapshot: Snapshot,
    /// Directories that could not be listed before the run.
    pub(super) denied: u64,
    /// Kept previous content.
    pub(super) preimage: Preimage,
    /// Journal operation of the transaction that kept the content.
    pub(super) op: Option<u64>,
}

impl State {
    /// State of a run without detection.
    pub(super) fn inactive(watch: Vec<PathBuf>) -> Self {
        Self {
            detection: Detection::None,
            roots: Vec::new(),
            watch,
            ignore: Vec::new(),
            exclude: Vec::new(),
            preimage_enabled: false,
            scan_hash: false,
            snapshot: Snapshot::new(),
            denied: 0,
            preimage: Preimage::default(),
            op: None,
        }
    }

    fn scope(&self) -> Scope<'_> {
        Scope {
            roots: &self.roots,
            ignore: &self.ignore,
            hash: self.scan_hash,
        }
    }

    /// Flag of a changed file of `path` without kept content.
    fn not_kept(&self, path: &Path) -> Option<&'static str> {
        let excluded = self.scope().root_of(path).is_some_and(|root| {
            path.strip_prefix(root)
                .is_ok_and(|relative| has_name(relative, &self.exclude))
        });
        if excluded {
            return Some(FLAG_EXCLUDED);
        }
        if !self.preimage_enabled || self.preimage.unreadable.contains(path) {
            return None;
        }
        self.preimage.shortfall.map(|shortfall| shortfall.flag())
    }

    /// Kind, number of files and bytes of the kept content.
    fn kept_summary(&self) -> (KeptContent, u64, u64) {
        let kept: Vec<_> = self
            .preimage
            .kept
            .values()
            .filter(|kept| kept.mode.is_preserved())
            .collect();
        let entries = u64::try_from(kept.len()).unwrap_or(u64::MAX);
        let bytes = kept.iter().map(|kept| kept.bytes).sum();
        let partial = self.preimage.shortfall.is_some()
            || !self.preimage.unreadable.is_empty()
            || kept.len() < self.preimage.kept.len();
        let summary = if kept.is_empty() {
            KeptContent::None
        } else if partial {
            KeptContent::Partial
        } else if kept.iter().all(|kept| kept.mode == PreserveMode::Clone) {
            KeptContent::Clone
        } else {
            KeptContent::Copy
        };
        (summary, entries, bytes)
    }

    fn notes(&self, tracking: Tracking, ignored: u64, denied: u64) -> Vec<NotableEntry> {
        let mut notes = Vec::new();
        if tracking != Tracking::None {
            notes.push(NotableEntry::new(Notable::DetectionScope));
        }
        if ignored > 0 {
            notes.push(NotableEntry {
                count: Some(ignored),
                ..NotableEntry::new(Notable::Ignored)
            });
        }
        if denied > 0 {
            notes.push(NotableEntry {
                count: Some(denied),
                ..NotableEntry::new(Notable::PermissionDeniedSkipped)
            });
        }
        notes
    }
}

/// State before a run, with the event watcher when one runs.
#[derive(Debug)]
pub(super) struct Prepared {
    /// Captured state.
    pub(super) state: State,
    /// Event watcher; absent when changes are found by scanning.
    pub(super) watch: Option<EventWatch>,
}

impl PreparedScan for Prepared {
    fn finish(self: Box<Self>, ctx: &ExecCtx, tx: &mut dyn MutationTx) -> Result<TrackOutcome> {
        let Self { state, watch } = *self;
        let (summary, preimage_entries, preimage_bytes) = state.kept_summary();
        let mut outcome = TrackOutcome {
            watch: state.watch.clone(),
            preimage: summary,
            preimage_entries,
            preimage_bytes,
            ..TrackOutcome::default()
        };
        if state.detection == Detection::None {
            return Ok(outcome);
        }
        let fs = ctx.require_fs()?;
        let scope = state.scope();
        let (after, tracking, ignored, denied) = match watch.and_then(EventWatch::finish) {
            Some(touched) => {
                let mut after = state.snapshot.clone();
                let ignored = refresh(fs, scope, &mut after, &touched);
                (after, Tracking::Events, ignored, state.denied)
            }
            None => {
                let scanned = scan(fs, scope);
                let denied = state.denied.max(scanned.denied);
                (scanned.entries, Tracking::Scan, 0, denied)
            }
        };
        let detected = diff::compare(&state.snapshot, &after);
        let not_kept = |path: &Path| state.not_kept(path);
        let recorder = Recorder {
            preimage: &state.preimage,
            declared: state.op.is_some() && tx.op().map(|op| op.0) == state.op,
            not_kept: &not_kept,
        };
        outcome.changes = recorder.record(&detected, tx)?;
        outcome.tracking = tracking;
        outcome.notes = state.notes(tracking, ignored, denied);
        Ok(outcome)
    }
}
