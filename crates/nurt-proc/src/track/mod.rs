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

use nurt_contracts::config::PreimageMode;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Fs};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::ExecCtx;

use self::preimage::{Candidate, Limits};
use self::prepared::{Detection, Prepared, State};
use self::scan::{Kind, Scope, Snapshot, has_name, scan};
use self::watch_events::EventWatch;
use crate::iface::{ChangeTracker, PreparedScan, TrackMode, TrackSpec};

/// Change tracker of `run` and of the tool runs of test and analysis
/// commands.
#[derive(Debug, Clone, Copy, Default)]
struct Tracker;

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
        let scope = Scope {
            roots: &roots,
            ignore: &ignore,
            hash: spec.scan_hash,
        };
        let scanned = scan(fs, scope);
        let preimage_enabled =
            spec.preimage && run.preimage == PreimageMode::Auto && exact.is_none();
        let mut explicit_paths = absolute(&spec.preserve)?;
        explicit_paths.extend(exact.iter().flatten().cloned());
        let explicit = files_under(fs, &explicit_paths, &ignore);
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
            snapshot: scanned.entries,
            denied: scanned.denied,
            preimage,
            op: tx.op().map(|op| op.0),
            roots,
        };
        Ok(Prepared {
            state,
            watch: watcher,
        })
    }
}

/// Change tracker used by the run host.
pub(crate) fn tracker() -> Arc<dyn ChangeTracker> {
    Arc::new(Tracker)
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

/// Regular files at or below `paths`.
fn files_under(fs: &dyn Fs, paths: &[AbsPath], ignore: &[String]) -> Vec<Candidate> {
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
