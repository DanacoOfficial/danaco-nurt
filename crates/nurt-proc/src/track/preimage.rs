// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Keeping the previous content of files before a run: declared targets of
//! the action `external_write`, preserved by the journal by clone or copy
//! within a time budget and size limits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, Fs};
use nurt_contracts::journal::{
    Expect, MutationTx, PreserveMode, TargetAction, TargetPath, TargetSpec,
};
use nurt_contracts::report::Irreversible;
use nurt_contracts::stage::ExecCtx;
use serde::{Deserialize, Serialize};

/// Targets declared in one call of `preserve`; the budget is checked between
/// calls.
const PRESERVE_BATCH: usize = 64;

/// Why a file in the watched directories has no kept content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Shortfall {
    /// The time budget ran out.
    Budget,
    /// The number of files or the amount of copied bytes reached its limit.
    Limit,
}

impl Shortfall {
    /// Flag of the change record.
    pub(super) fn flag(self) -> &'static str {
        match self {
            Self::Budget => "preimage_budget",
            Self::Limit => "preimage_limit",
        }
    }
}

/// Previous content kept for one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Kept {
    /// Index of the declared target.
    pub(super) index: u32,
    /// How the journal kept the content.
    pub(super) mode: PreserveMode,
    /// Bytes held by the journal.
    pub(super) bytes: u64,
    /// Content hash before the run, `sha256:…`.
    pub(super) hash_before: Option<String>,
    /// Reason when the journal could not keep the content.
    pub(super) irreversible: Option<Irreversible>,
}

/// Result of keeping previous content.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Preimage {
    /// Kept files.
    pub(super) kept: BTreeMap<PathBuf, Kept>,
    /// Why the watched files were kept only in part.
    pub(super) shortfall: Option<Shortfall>,
    /// Files skipped because they could not be read.
    pub(super) unreadable: BTreeSet<PathBuf>,
}

/// Limits of keeping the watched files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    /// Time budget of the preparation, from its start.
    pub(super) budget: Duration,
    /// Largest number of kept watched files.
    pub(super) max_entries: u64,
    /// Largest amount of copied bytes when the volume does not clone.
    pub(super) copy_max: u64,
}

/// Candidate file and its size.
pub(super) type Candidate = (PathBuf, u64);

struct Keeper<'a> {
    ctx: &'a ExecCtx,
    fs: &'a dyn Fs,
    tx: &'a mut dyn MutationTx,
    preimage: Preimage,
    copied: u64,
    cloning: bool,
}

impl Keeper<'_> {
    /// Declares the readable files of `batch` and records the outcome.
    fn declare(&mut self, batch: &[Candidate]) -> Result<()> {
        let mut readable = Vec::with_capacity(batch.len());
        for (path, size) in batch {
            match AbsPath::new(path) {
                Ok(abs) if self.fs.open_read(&abs).is_ok() => readable.push((abs, *size)),
                _ => {
                    self.preimage.unreadable.insert(path.clone());
                }
            }
        }
        if readable.is_empty() {
            return Ok(());
        }
        let specs: Vec<TargetSpec> = readable
            .iter()
            .map(|(path, size)| TargetSpec {
                path: TargetPath::Fs(path.clone()),
                action: TargetAction::ExternalWrite,
                expect: Expect::Unchecked,
                size_hint: Some(*size),
            })
            .collect();
        let preserved = self.tx.preserve(&specs)?;
        for ((path, size), result) in readable.iter().zip(preserved) {
            match result.mode {
                PreserveMode::Clone => self.cloning = true,
                PreserveMode::Copy => self.copied += size,
                _ => {}
            }
            self.preimage.kept.insert(
                path.as_path().to_path_buf(),
                Kept {
                    index: result.index.0,
                    mode: result.mode,
                    bytes: result.bytes_held,
                    hash_before: result.before.hash.map(|hash| hash.to_string()),
                    irreversible: result.irreversible,
                },
            );
        }
        Ok(())
    }

    fn over_budget(&self, started: Duration, limits: Limits) -> bool {
        self.ctx.clock().monotonic().saturating_sub(started) >= limits.budget
    }

    /// Keeps watched files in order until the entry limit or the budget stops
    /// it. A file too large to copy is skipped, not the files after it; it is
    /// kept afterwards when the volume turns out to clone.
    fn watched(&mut self, files: &[Candidate], limits: Limits, started: Duration) -> Result<()> {
        let mut batch: Vec<Candidate> = Vec::new();
        let mut batch_bytes = 0;
        let mut accepted: u64 = 0;
        let mut oversize: Vec<Candidate> = Vec::new();
        for (path, size) in files {
            if self.over_budget(started, limits) {
                self.declare(&batch)?;
                self.preimage.shortfall = Some(Shortfall::Budget);
                return Ok(());
            }
            if accepted >= limits.max_entries {
                self.preimage.shortfall = Some(Shortfall::Limit);
                return self.declare(&batch);
            }
            if !self.cloning && self.copied + batch_bytes + size > limits.copy_max {
                // The first declared batch tells whether the volume clones;
                // cloned files do not count against the copy limit.
                self.declare(&batch)?;
                batch.clear();
                batch_bytes = 0;
            }
            if !self.cloning && self.copied + size > limits.copy_max {
                oversize.push((path.clone(), *size));
                continue;
            }
            accepted += 1;
            batch_bytes += size;
            batch.push((path.clone(), *size));
            if batch.len() == PRESERVE_BATCH {
                self.declare(&batch)?;
                batch.clear();
                batch_bytes = 0;
            }
        }
        self.declare(&batch)?;
        self.keep_oversize(&oversize, accepted, limits, started)
    }

    /// Keeps the files skipped for their size when the volume clones, within
    /// the entry limit and the budget; otherwise they count as a shortfall.
    fn keep_oversize(
        &mut self,
        oversize: &[Candidate],
        accepted: u64,
        limits: Limits,
        started: Duration,
    ) -> Result<()> {
        if oversize.is_empty() {
            return Ok(());
        }
        if !self.cloning {
            self.preimage.shortfall = Some(Shortfall::Limit);
            return Ok(());
        }
        let mut room = limits.max_entries.saturating_sub(accepted);
        for batch in oversize.chunks(PRESERVE_BATCH) {
            if self.over_budget(started, limits) {
                self.preimage.shortfall = Some(Shortfall::Budget);
                return Ok(());
            }
            let take = usize::try_from(room).unwrap_or(usize::MAX).min(batch.len());
            self.declare(&batch[..take])?;
            room = room.saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
            if take < batch.len() {
                self.preimage.shortfall = Some(Shortfall::Limit);
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Declares `explicit` files without limits, then the `watched` files within
/// `limits`, measured from `started` on the clock of `ctx`.
///
/// # Errors
///
/// The errors of `MutationTx::preserve` for a target (`E_NOT_FOUND`,
/// `E_PERMISSION`, `E_IN_USE`, `E_IO`, `E_CANCELLED`); the transaction is then
/// rolled back.
pub(super) fn keep(
    ctx: &ExecCtx,
    fs: &dyn Fs,
    tx: &mut dyn MutationTx,
    explicit: &[Candidate],
    watched: &[Candidate],
    limits: Limits,
    started: Duration,
) -> Result<Preimage> {
    let mut keeper = Keeper {
        ctx,
        fs,
        tx,
        preimage: Preimage::default(),
        copied: 0,
        cloning: false,
    };
    for batch in explicit.chunks(PRESERVE_BATCH) {
        keeper.declare(batch)?;
    }
    let remaining: Vec<Candidate> = watched
        .iter()
        .filter(|(path, _)| !keeper.preimage.kept.contains_key(path))
        .cloned()
        .collect();
    keeper.watched(&remaining, limits, started)?;
    Ok(keeper.preimage)
}

impl Preimage {
    /// Kept content of `path`, when the journal kept it.
    pub(super) fn kept(&self, path: &Path) -> Option<&Kept> {
        self.kept.get(path).filter(|kept| kept.mode.is_preserved())
    }
}
