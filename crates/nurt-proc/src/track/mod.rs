// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of file changes made by programs and preservation of previous
//! content.
//!
//! Before a run the watched directories are scanned and the previous content
//! of their files is declared to the journal as `external_write` targets,
//! within the time budget and limits of the `run.preimage*` keys. After the
//! run a second scan, or the paths reported by file system events, reveals
//! the changes; they are reported with `MutationTx::observe_external`.
//! Changes outside the watched directories are not detected.
//!
//! A scan compares kind, size, modification time, file identity and, when
//! both scans have one, the content hash. A file system stores modification
//! times with a limited granularity, so a file rewritten with the same size
//! right after the first scan can keep its stored time. Such a file is racy:
//! its modification time is less than `nurt_contracts::fs::RACY_WINDOW` older
//! than the start of the first scan. Racy files are hashed in the first scan;
//! the second scan, or the refresh by events, hashes a file only when the first
//! scan hashed it and its size, modification time and identity did not change,
//! because a hash on one side decides nothing. A file larger than
//! `run.preimage_copy_max` (64 MiB when that key is unlimited) is not hashed
//! for this reason and is compared by metadata only, so a same-size rewrite of
//! such a file within the timestamp granularity stays undetected; the hash is
//! not interrupted by the time budget, which is why its size is bounded.
//! Older files are hashed only on request (`scan_hash`).

/// Largest file hashed because it is racy when `run.preimage_copy_max` does
/// not bound the copies.
const RACY_MAX_BYTES_DEFAULT: u64 = 64 << 20;

/// Largest file hashed because it is racy: the largest file whose content is
/// kept, or [`RACY_MAX_BYTES_DEFAULT`] when no such limit is set.
fn racy_limit(copy_max: u64) -> u64 {
    if copy_max == 0 || copy_max == u64::MAX {
        RACY_MAX_BYTES_DEFAULT
    } else {
        copy_max
    }
}

mod diff;
mod journal;
mod preimage;
mod prepared;
mod scan;
mod watch_events;

#[cfg(test)]
#[cfg(unix)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use nurt_contracts::config::PreimageMode;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Fs};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::ExecCtx;

use self::preimage::{Candidate, Limits};
use self::prepared::{Detection, Prepared, State};
use self::scan::{
    Bound, Kind, Racy, ScanLimits, Scope, Snapshot, Stop, has_name, racy_cutoff, scan,
};
use self::watch_events::EventWatch;
use crate::iface::{ChangeTracker, PreparedScan, TrackMode, TrackSpec};

/// Largest number of entries that automatic detection scans.
const AUTO_MAX_ENTRIES: usize = 200_000;

/// Time budget of one scan of automatic detection.
const AUTO_BUDGET: Duration = Duration::from_secs(3);

/// Time a scan after the program may use beyond the time limit of the run,
/// so that the changes of a program ended by the limit are still recorded.
const POST_SCAN_MIN: Duration = Duration::from_secs(5);

/// Change tracker of `run` and of the tool runs of test and analysis
/// commands.
///
/// Automatic detection is bounded: it does not scan the file system root,
/// does not enter directories on other volumes, and gives up when the scope
/// holds more than [`AUTO_MAX_ENTRIES`] entries or a scan takes longer than
/// [`AUTO_BUDGET`]; the run then proceeds without detection and the report
/// carries a `detection_scope` note that says why. Every mode ends its scans
/// at the time limit of the run and at cancellation.
#[derive(Debug, Clone, Copy)]
struct Tracker {
    automatic: ScanLimits,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            automatic: ScanLimits {
                max_entries: AUTO_MAX_ENTRIES,
                budget: Some(AUTO_BUDGET),
                one_volume: true,
            },
        }
    }
}

impl ChangeTracker for Tracker {
    fn prepare(
        &self,
        ctx: &ExecCtx,
        spec: &TrackSpec,
        tx: &mut dyn MutationTx,
    ) -> Result<Box<dyn PreparedScan>> {
        Ok(Box::new(self.capture(ctx, spec, tx)?))
    }
}

impl Tracker {
    /// Captures the watched scope before a run and keeps previous content.
    fn capture(
        &self,
        ctx: &ExecCtx,
        spec: &TrackSpec,
        tx: &mut dyn MutationTx,
    ) -> Result<Prepared> {
        let started = ctx.clock().monotonic();
        let deadline = spec
            .timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));
        let watch = absolute(&spec.watch)?;
        let exact = spec.exact.as_deref().map(absolute).transpose()?;
        let reported: Vec<PathBuf> = watch
            .iter()
            .map(|path| path.as_path().to_path_buf())
            .collect();
        let detection = match spec.mode {
            TrackMode::None if exact.is_none() => Detection::None,
            TrackMode::None | TrackMode::Scan => Detection::Scan,
            TrackMode::Auto | TrackMode::Events => Detection::Events,
        };
        if detection == Detection::None {
            return Ok(Prepared {
                state: State::inactive(reported),
                watch: None,
            });
        }
        let fs = ctx.require_fs()?;
        let run = &ctx.config().run;
        let roots = roots(spec.mode, &watch, exact.as_deref());
        let mut ignore = run.track_ignore.clone();
        ignore.extend(spec.ignore.iter().cloned());
        ignore.sort();
        ignore.dedup();
        let automatic = spec.mode == TrackMode::Auto;
        if automatic && let Some(root) = roots.iter().find(|root| root.as_path().parent().is_none())
        {
            return Ok(skipped(
                reported,
                format!(
                    "automatic change detection skipped: {} is the file system root, which \
                     holds too many entries to scan; name the directories the program changes \
                     in `watch` or use `track_changes: scan`",
                    root.as_path().display()
                ),
            ));
        }
        let scan_limits = if automatic {
            self.automatic
        } else {
            ScanLimits::UNLIMITED
        };
        let racy_cutoff_ns = racy_cutoff(SystemTime::now());
        let racy_max_bytes = racy_limit(run.preimage_copy_max.bytes());
        let bound = Bound::new(scan_limits, deadline, ctx.cancel().clone());
        let scope = Scope {
            roots: &roots,
            ignore: &ignore,
            hash: spec.scan_hash,
            racy: Some(Racy {
                cutoff_ns: racy_cutoff_ns,
                max_bytes: racy_max_bytes,
                previous: None,
            }),
            bound: Some(&bound),
        };
        let scanned = scan(fs, scope);
        match scanned.stop {
            None => {}
            Some(Stop::Cancelled) => return Err(cancelled(ctx)),
            Some(Stop::TimeLimit) => return Err(timed_out("before the program started")),
            Some(Stop::Budget) => {
                return Ok(skipped(
                    reported,
                    format!(
                        "automatic change detection skipped: scanning the watched directories \
                         took longer than {} s; name the directories the program changes in \
                         `watch` or use `track_changes: scan`",
                        scan_limits.budget.unwrap_or_default().as_secs()
                    ),
                ));
            }
            Some(Stop::Entries) => {
                return Ok(skipped(
                    reported,
                    format!(
                        "automatic change detection skipped: the watched directories hold more \
                         than {} entries; name the directories the program changes in `watch` \
                         or use `track_changes: scan`",
                        scan_limits.max_entries
                    ),
                ));
            }
        }
        let preimage_enabled =
            spec.preimage && run.preimage == PreimageMode::Auto && exact.is_none();
        let mut explicit_paths = absolute(&spec.preserve)?;
        explicit_paths.extend(exact.iter().flatten().cloned());
        let listing = Bound::new(ScanLimits::UNLIMITED, deadline, ctx.cancel().clone());
        let explicit = files_under(fs, &explicit_paths, &ignore, &listing);
        match listing.stopped() {
            Some(Stop::Cancelled) => return Err(cancelled(ctx)),
            Some(_) => return Err(timed_out("while the files to keep were listed")),
            None => {}
        }
        let watched = if preimage_enabled {
            watched_files(&scanned.entries, scope, &run.preimage_exclude)
        } else {
            Vec::new()
        };
        let limits = Limits {
            budget: run.preimage_budget.as_std(),
            max_entries: u64::from(run.preimage_max_entries),
            copy_max: run.preimage_copy_max.bytes(),
        };
        let preimage = preimage::keep(ctx, fs, tx, &explicit, &watched, limits, started)?;
        let watcher = if detection == Detection::Events {
            EventWatch::start(&roots)
        } else {
            None
        };
        let state = State {
            detection,
            watch: reported,
            ignore,
            exclude: run.preimage_exclude.clone(),
            preimage_enabled,
            scan_hash: spec.scan_hash,
            racy_cutoff_ns: Some(racy_cutoff_ns),
            racy_max_bytes,
            snapshot: scanned.entries,
            denied: scanned.denied,
            preimage,
            op: tx.op().map(|op| op.0),
            roots,
            limits: scan_limits,
            deadline,
            foreign: scanned.foreign,
            skipped: None,
        };
        Ok(Prepared {
            state,
            watch: watcher,
        })
    }
}

/// Change tracker used by the run host.
pub(crate) fn tracker() -> Arc<dyn ChangeTracker> {
    Arc::new(Tracker::default())
}

/// Preparation of a run without detection, with the reason in a
/// `detection_scope` note.
fn skipped(watch: Vec<PathBuf>, reason: String) -> Prepared {
    let mut state = State::inactive(watch);
    state.skipped = Some(reason);
    Prepared { state, watch: None }
}

/// Error of a scan that stopped because the execution was cancelled.
fn cancelled(ctx: &ExecCtx) -> Error {
    ctx.cancel().check().err().unwrap_or_else(|| {
        Error::new(
            ErrorCode::Cancelled,
            "the execution was cancelled while the changes were scanned",
        )
    })
}

/// Error of a scan that stopped at the time limit of the run.
fn timed_out(phase: &str) -> Error {
    Error::new(
        ErrorCode::Timeout,
        format!(
            "the time limit of the run ended while the watched directories were scanned {phase}"
        ),
    )
    .with_fix("raise `timeout`, name fewer directories in `watch` or use `track_changes: none`")
}

fn absolute(paths: &[PathBuf]) -> Result<Vec<AbsPath>> {
    paths
        .iter()
        .map(|path| {
            AbsPath::new(path).map_err(|_| {
                Error::new(
                    ErrorCode::Arg,
                    format!(
                        "path `{}` is not absolute; give a full path",
                        path.display()
                    ),
                )
            })
        })
        .collect()
}

/// Scanned roots: the watched directories and the exact files outside them,
/// without roots nested in other roots. Without detection only the exact
/// files are compared.
fn roots(mode: TrackMode, watch: &[AbsPath], exact: Option<&[AbsPath]>) -> Vec<AbsPath> {
    let mut candidates: Vec<AbsPath> = match mode {
        TrackMode::None => Vec::new(),
        _ => watch.to_vec(),
    };
    candidates.extend(exact.into_iter().flatten().cloned());
    candidates.sort();
    candidates.dedup();
    let mut roots: Vec<AbsPath> = Vec::new();
    for candidate in candidates {
        if !roots.iter().any(|root| candidate.starts_with(root)) {
            roots.push(candidate);
        }
    }
    roots
}

/// Regular files at or below `paths`; the listing ends early when `bound`
/// stops it, which the caller reads from the bound.
fn files_under(fs: &dyn Fs, paths: &[AbsPath], ignore: &[String], bound: &Bound) -> Vec<Candidate> {
    let mut files: Vec<Candidate> = paths
        .iter()
        .flat_map(|path| {
            let roots = [path.clone()];
            scan(
                fs,
                Scope {
                    roots: &roots,
                    ignore,
                    hash: false,
                    racy: None,
                    bound: Some(bound),
                },
            )
            .entries
        })
        .filter(|(_, state)| state.kind == Kind::File)
        .map(|(path, state)| (path, state.size))
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Regular files of the scan whose content may be kept.
fn watched_files(snapshot: &Snapshot, scope: Scope<'_>, exclude: &[String]) -> Vec<Candidate> {
    snapshot
        .iter()
        .filter(|(_, state)| state.kind == Kind::File)
        .filter(|(path, _)| {
            scope.root_of(path).is_some_and(|root| {
                path.strip_prefix(root)
                    .is_ok_and(|relative| !has_name(relative, exclude))
            })
        })
        .map(|(path, state)| (path.clone(), state.size))
        .collect()
}
