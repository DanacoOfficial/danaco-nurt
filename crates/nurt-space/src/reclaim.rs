// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reclaim of journal entries.
//!
//! An entry is released in three steps: it is marked expiring, its directory
//! is renamed into the reclaim area in one operation, and the moved tree is
//! removed bottom-up without following links. The journal then writes the
//! tombstone. A crash at any step leaves an entry that recovery completes from
//! the reclaim area.
//!
//! Order of release: entries over a step or age limit first, oldest first;
//! then, for space, in decreasing order of the cost-model score, and for the
//! size budget, oldest first. Ties go to the lower number. Pinned entries are
//! released for space like any other; they are kept only from the size
//! budget. In the hard state an entry larger than the remaining need is
//! released partially, largest items first, and becomes partial.

use std::cmp::Ordering;
use std::num::NonZeroUsize;
use std::time::Duration;

use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, Result};
use nurt_contracts::fs::{
    AbsPath, FsError, FsErrorKind, FsWrite, RenameMode, ScopePurpose, TxScope, VolumeInfo,
};
use nurt_contracts::journal::{EntryState, ExpiryReason, OpId, VolumeState};
use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints};
use rayon::prelude::*;

use nurt_journal::store::{EntryCatalog, EntrySummary, HeldItem};

use crate::admission::CostModel;
use crate::measure::{self, Measurement};

/// Upper bound of removal workers on solid-state media.
const SSD_WORKERS: usize = 8;

/// Removal workers on rotational media.
const HDD_WORKERS: usize = 2;

/// Operation number of scopes that belong to no journal entry (calibration).
/// Entry numbers start at 1.
const MAINTENANCE_OP: OpId = OpId(0);

/// Scope of a reclaim of entry `op`.
#[allow(
    clippy::disallowed_methods,
    reason = "the space service releases journal entries and needs a reclaim scope"
)]
pub(crate) fn reclaim_scope(op: OpId) -> TxScope {
    TxScope::new_for_journal(op, ScopePurpose::Reclaim)
}

/// Scope of journal maintenance that belongs to no entry.
#[allow(
    clippy::disallowed_methods,
    reason = "calibration writes in the journal directory and needs a maintenance scope"
)]
pub(crate) fn maintenance_scope() -> TxScope {
    TxScope::new_for_journal(MAINTENANCE_OP, ScopePurpose::Maintenance)
}

/// Removal workers for a volume: `min(8, cores)` on solid-state media, 2 on
/// rotational media, every core in the emergency state.
pub(crate) fn parallelism(rotational: bool, state: VolumeState, cores: NonZeroUsize) -> usize {
    let cores = cores.get();
    if state == VolumeState::Emergency {
        cores
    } else if rotational {
        HDD_WORKERS.min(cores)
    } else {
        SSD_WORKERS.min(cores)
    }
}

/// Whether an entry holds data that reclaim may release.
pub(crate) fn releasable(entry: &EntrySummary) -> bool {
    matches!(
        entry.state,
        EntryState::Committed | EntryState::Partial | EntryState::Undone
    )
}

fn overdue_first(a: &EntrySummary, b: &EntrySummary) -> Ordering {
    b.overdue.is_some().cmp(&a.overdue.is_some()).then_with(|| {
        if a.overdue.is_some() {
            a.op.cmp(&b.op)
        } else {
            Ordering::Equal
        }
    })
}

/// Release order for space: overdue entries oldest first, then the others by
/// decreasing score, ties to the lower number.
pub(crate) fn space_order<'a>(
    entries: &'a [EntrySummary],
    model: &CostModel,
) -> Vec<&'a EntrySummary> {
    let mut order: Vec<_> = entries.iter().filter(|entry| releasable(entry)).collect();
    order.sort_by(|a, b| {
        overdue_first(a, b)
            .then_with(|| model.score(b).total_cmp(&model.score(a)))
            .then_with(|| a.op.cmp(&b.op))
    });
    order
}

/// Release order for the size budget: overdue entries oldest first, then the
/// other unpinned entries oldest first.
pub(crate) fn budget_order(entries: &[EntrySummary]) -> Vec<&EntrySummary> {
    let mut order: Vec<_> = entries
        .iter()
        .filter(|entry| releasable(entry) && (entry.overdue.is_some() || !entry.pinned))
        .collect();
    order.sort_by(|a, b| overdue_first(a, b).then_with(|| a.op.cmp(&b.op)));
    order
}

/// Entries released by one reclaim pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Released {
    /// Entries released completely.
    pub(crate) whole: Vec<OpId>,
    /// Entries released in part.
    pub(crate) partial: Vec<OpId>,
    /// Logical bytes released.
    pub(crate) bytes: u64,
}

impl Released {
    pub(crate) fn contains(&self, op: OpId) -> bool {
        self.whole.contains(&op) || self.partial.contains(&op)
    }

    pub(crate) fn absorb(&mut self, other: Self) {
        self.whole.extend(other.whole);
        self.partial.extend(other.partial);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }
}

/// Target of a space reclaim pass.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpaceGoal {
    /// Free bytes to reach.
    pub(crate) target: u64,
    /// Monotonic time at which the pass stops.
    pub(crate) deadline: Duration,
    /// Release entries in part when they exceed the remaining need.
    pub(crate) partial: bool,
}

/// Reclaim on one volume.
pub(crate) struct Reclaimer<'a> {
    pub(crate) fs: &'a dyn FsWrite,
    pub(crate) clock: &'a dyn Clock,
    pub(crate) failpoints: &'a dyn Failpoints,
    pub(crate) catalog: &'a dyn EntryCatalog,
    pub(crate) volume: &'a VolumeInfo,
    pub(crate) quota_aware: bool,
    pub(crate) workers: usize,
    pub(crate) poll: Duration,
    pub(crate) settle: Duration,
}

fn checkpoint(failpoints: &dyn Failpoints, point: FailPoint) -> Result<()> {
    match failpoints.hit(point, None) {
        FailAction::Continue => Ok(()),
        FailAction::Error(kind) => Err(FsError::new(kind, point.as_str(), None).into()),
        FailAction::Crash => {
            Err(FsError::new(FsErrorKind::Interrupted, point.as_str(), None).into())
        }
    }
}

impl Reclaimer<'_> {
    fn measure(&self) -> Result<Measurement> {
        measure::measure(self.fs, &self.volume.id, self.quota_aware).map_err(Error::from)
    }

    fn expired(&self, deadline: Duration) -> bool {
        self.clock.monotonic() >= deadline
    }

    /// Whether removing `entry` is estimated to end after `deadline`; such
    /// entries are passed over in favour of cheaper ones, because a removal
    /// once started runs to its end.
    fn overruns(&self, entry: &EntrySummary, model: &CostModel, deadline: Duration) -> bool {
        let estimate = Duration::try_from_secs_f64(model.seconds(entry)).unwrap_or(Duration::MAX);
        self.clock.monotonic().saturating_add(estimate) > deadline
    }

    /// Releases a whole entry under its entry lock: expiring, renamed into the
    /// reclaim area, removed, tombstone written.
    ///
    /// # Errors
    ///
    /// Fails with the error of the first step that fails; an entry already
    /// renamed is completed by recovery.
    pub(crate) fn release_entry(&self, entry: &EntrySummary, reason: ExpiryReason) -> Result<()> {
        let volume = &self.volume.id;
        let _lock = self.catalog.lock_entry(volume, entry.op)?;
        let paths = self.catalog.begin_reclaim(volume, entry.op, reason)?;
        let scope = reclaim_scope(entry.op);
        self.fs
            .rename(&scope, &paths.entry, &paths.reap, RenameMode::NoReplace)?;
        self.sync_parents(&scope, [&paths.entry, &paths.reap])?;
        checkpoint(self.failpoints, FailPoint::ReclaimMoved)?;
        self.fs.remove_tree(&scope, &paths.reap, self.workers)?;
        self.catalog.complete_reclaim(volume, entry.op)?;
        Ok(())
    }

    /// Releases the largest items of an entry until `need` bytes are
    /// released; returns the bytes released.
    ///
    /// The release runs under the entry lock. Items are durably marked
    /// released before their data is removed, so that a crash never leaves an
    /// item that claims data it no longer has; the pending marker of the
    /// entry tells recovery to reconcile marks and data.
    ///
    /// # Errors
    ///
    /// Fails when the items cannot be listed or marked, or when data of a
    /// marked item cannot be removed.
    pub(crate) fn release_items(&self, entry: &EntrySummary, need: u64) -> Result<u64> {
        let volume = &self.volume.id;
        let _lock = self.catalog.lock_entry(volume, entry.op)?;
        let mut items = self.catalog.held_items(volume, entry.op)?;
        items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.index.cmp(&b.index)));
        let mut chosen: Vec<HeldItem> = Vec::new();
        let mut bytes = 0_u64;
        for item in items {
            if bytes >= need {
                break;
            }
            bytes = bytes.saturating_add(item.bytes);
            chosen.push(item);
        }
        if chosen.is_empty() {
            return Ok(0);
        }
        let indices: Vec<u32> = chosen.iter().map(|item| item.index).collect();
        self.catalog.mark_released(volume, entry.op, &indices)?;
        self.remove_items(entry.op, &chosen)?;
        self.sync_parents(
            &reclaim_scope(entry.op),
            chosen.iter().map(|item| &item.data),
        )?;
        checkpoint(self.failpoints, FailPoint::ReclaimPartial)?;
        self.catalog.finish_release(volume, entry.op)?;
        Ok(bytes)
    }

    /// Makes the removal or rename of every path durable by syncing its parent
    /// directory, once per directory. The pending marker is cleared only after
    /// this, so that recovery still sees the release as unfinished when a
    /// removal is lost in a crash.
    fn sync_parents<'p>(
        &self,
        scope: &TxScope,
        paths: impl IntoIterator<Item = &'p AbsPath>,
    ) -> Result<()> {
        let mut dirs: Vec<AbsPath> = paths.into_iter().filter_map(AbsPath::parent).collect();
        dirs.sort();
        dirs.dedup();
        dirs.iter()
            .try_for_each(|dir| self.fs.sync_dir(scope, dir))
            .map_err(Error::from)
    }

    fn remove_items(&self, op: OpId, items: &[HeldItem]) -> Result<()> {
        let scope = reclaim_scope(op);
        let remove = |item: &HeldItem| self.fs.remove_file(&scope, &item.data);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.workers.max(1))
            .build();
        let outcome = match pool {
            Ok(pool) => pool.install(|| items.par_iter().try_for_each(remove)),
            Err(_) => items.iter().try_for_each(remove),
        };
        outcome.map_err(Error::from)
    }

    /// Releases whole entries in budget order until `need` logical bytes are
    /// released or `deadline` passes. Entries that fail to release, or whose
    /// estimated removal ends after the deadline, are skipped.
    pub(crate) fn for_budget(
        &self,
        entries: &[EntrySummary],
        model: &CostModel,
        need: u64,
        deadline: Duration,
    ) -> Released {
        let mut released = Released::default();
        for entry in budget_order(entries) {
            if released.bytes >= need || self.expired(deadline) {
                break;
            }
            if self.overruns(entry, model, deadline) {
                continue;
            }
            let reason = entry.overdue.unwrap_or(ExpiryReason::Size);
            if self.release_entry(entry, reason).is_ok() {
                released.whole.push(entry.op);
                released.bytes = released.bytes.saturating_add(entry.bytes);
            }
        }
        released
    }

    fn release_one(
        &self,
        entry: &EntrySummary,
        need: u64,
        goal: &SpaceGoal,
        released: &mut Released,
    ) {
        if goal.partial && entry.overdue.is_none() && entry.bytes > need {
            if let Ok(bytes) = self.release_items(entry, need)
                && bytes > 0
            {
                released.partial.push(entry.op);
                released.bytes = released.bytes.saturating_add(bytes);
            }
            return;
        }
        let reason = entry.overdue.unwrap_or(ExpiryReason::Space);
        if self.release_entry(entry, reason).is_ok() {
            released.whole.push(entry.op);
            released.bytes = released.bytes.saturating_add(entry.bytes);
        }
    }

    fn settle_until(&self, target: u64, deadline: Duration) -> Result<Measurement> {
        let window = self
            .settle
            .min(deadline.saturating_sub(self.clock.monotonic()));
        measure::settle(
            self.fs,
            self.clock,
            &self.volume.id,
            self.quota_aware,
            target,
            window,
            self.poll,
        )
        .map_err(Error::from)
    }

    /// Releases entries in space order until free space reaches the goal,
    /// judged by measured free space, or the deadline passes.
    ///
    /// Releasing stops once the released bytes would cover the need, and the
    /// pass waits up to the settle window for the file system to show them;
    /// only when they do not appear does it release more.
    ///
    /// # Errors
    ///
    /// Fails when free space cannot be measured.
    pub(crate) fn for_space(
        &self,
        entries: &[EntrySummary],
        model: &CostModel,
        goal: &SpaceGoal,
        skip: &Released,
    ) -> Result<(Released, Measurement)> {
        let mut released = Released::default();
        let mut reading = self.measure()?;
        let mut expected = reading.free;
        for entry in space_order(entries, model) {
            if reading.free >= goal.target || self.expired(goal.deadline) {
                break;
            }
            if skip.contains(entry.op) || self.overruns(entry, model, goal.deadline) {
                continue;
            }
            if expected >= goal.target {
                reading = self.settle_until(goal.target, goal.deadline)?;
                if reading.free >= goal.target || self.expired(goal.deadline) {
                    break;
                }
                expected = reading.free;
            }
            let before = released.bytes;
            self.release_one(entry, goal.target - expected, goal, &mut released);
            expected = expected.saturating_add(released.bytes - before);
            reading = self.measure()?;
        }
        if reading.free < goal.target && expected >= goal.target && !self.expired(goal.deadline) {
            reading = self.settle_until(goal.target, goal.deadline)?;
        }
        Ok((released, reading))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Journal catalog over a test volume.

    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::fs::{AbsPath, FileLock, VolumeId};
    use nurt_contracts::journal::{EntryState, ExpiredEntry, ExpiryReason, OpId};

    use nurt_journal::store::{EntryCatalog, EntrySummary, HeldItem, ReclaimPaths};

    use crate::measure::testing::{TestVolume, path};

    #[derive(Debug, Clone)]
    struct Stored {
        summary: EntrySummary,
        items: Vec<u64>,
        reason: Option<ExpiryReason>,
    }

    /// Entries laid out as `vol/j/ops/<n>/blobs/<i>` on a [`TestVolume`].
    #[derive(Debug, Default)]
    pub(crate) struct TestCatalog {
        entries: Mutex<BTreeMap<OpId, Stored>>,
        released: Mutex<BTreeMap<OpId, Vec<u32>>>,
        tombs: Mutex<Vec<(OpId, ExpiryReason)>>,
        fail_listing: Mutex<bool>,
        locks: Mutex<Vec<OpId>>,
        pending: Mutex<Vec<OpId>>,
    }

    /// Entry lock that only names its file.
    struct TestLock(AbsPath);

    impl FileLock for TestLock {
        fn path(&self) -> &AbsPath {
            &self.0
        }
    }

    pub(crate) fn entry_dir(op: OpId) -> AbsPath {
        path(&format!("vol/j/ops/{}", op.0))
    }

    pub(crate) fn reap_dir(op: OpId) -> AbsPath {
        path(&format!("vol/j/reap/{}", op.0))
    }

    pub(crate) fn blob(op: OpId, index: u32) -> AbsPath {
        path(&format!("vol/j/ops/{}/blobs/{index}", op.0))
    }

    impl TestCatalog {
        /// Adds a committed entry holding one blob per item size.
        pub(crate) fn add(&self, volume: &TestVolume, op: u64, items: &[u64]) -> OpId {
            let op = OpId(op);
            for (index, size) in (0_u32..).zip(items) {
                volume.put_file(&blob(op, index), *size);
            }
            let summary = EntrySummary {
                op,
                state: EntryState::Committed,
                bytes: items.iter().sum(),
                files: items.len() as u64,
                directories: 2,
                pinned: false,
                overdue: None,
            };
            self.entries.lock().expect("entries").insert(
                op,
                Stored {
                    summary,
                    items: items.to_vec(),
                    reason: None,
                },
            );
            op
        }

        pub(crate) fn update(&self, op: OpId, change: impl FnOnce(&mut EntrySummary)) {
            if let Some(stored) = self.entries.lock().expect("entries").get_mut(&op) {
                change(&mut stored.summary);
            }
        }

        pub(crate) fn fail_listing(&self) {
            *self.fail_listing.lock().expect("flag") = true;
        }

        pub(crate) fn tombs(&self) -> Vec<(OpId, ExpiryReason)> {
            self.tombs.lock().expect("tombs").clone()
        }

        pub(crate) fn released_items(&self, op: OpId) -> Vec<u32> {
            self.released
                .lock()
                .expect("released")
                .get(&op)
                .cloned()
                .unwrap_or_default()
        }

        /// Entries locked so far, in order.
        pub(crate) fn locks(&self) -> Vec<OpId> {
            self.locks.lock().expect("locks").clone()
        }

        /// Entries with a pending partial release.
        pub(crate) fn pending(&self) -> Vec<OpId> {
            self.pending.lock().expect("pending").clone()
        }

        pub(crate) fn state(&self, op: OpId) -> Option<EntryState> {
            self.entries
                .lock()
                .expect("entries")
                .get(&op)
                .map(|stored| stored.summary.state)
        }
    }

    fn unknown(op: OpId) -> Error {
        Error::new(
            ErrorCode::JournalNoop,
            format!("entry {} does not exist", op.0),
        )
    }

    impl EntryCatalog for TestCatalog {
        fn lock_entry(&self, _volume: &VolumeId, op: OpId) -> Result<Box<dyn FileLock>> {
            self.locks.lock().expect("locks").push(op);
            Ok(Box::new(TestLock(path(&format!(
                "vol/j/lock/{}.lock",
                op.0
            )))))
        }

        fn finish_release(&self, _volume: &VolumeId, op: OpId) -> Result<()> {
            self.pending
                .lock()
                .expect("pending")
                .retain(|pending| *pending != op);
            Ok(())
        }

        fn entries(&self, _volume: &VolumeId) -> Result<Vec<EntrySummary>> {
            if *self.fail_listing.lock().expect("flag") {
                return Err(Error::new(ErrorCode::Io, "journal index cannot be read"));
            }
            Ok(self
                .entries
                .lock()
                .expect("entries")
                .values()
                .filter(|stored| stored.summary.state != EntryState::Expired)
                .map(|stored| stored.summary.clone())
                .collect())
        }

        fn held_items(&self, _volume: &VolumeId, op: OpId) -> Result<Vec<HeldItem>> {
            let entries = self.entries.lock().expect("entries");
            let stored = entries.get(&op).ok_or_else(|| unknown(op))?;
            let released = self.released_items(op);
            Ok((0_u32..)
                .zip(&stored.items)
                .filter(|(index, _)| !released.contains(index))
                .map(|(index, bytes)| HeldItem {
                    index,
                    data: blob(op, index),
                    bytes: *bytes,
                })
                .collect())
        }

        fn begin_reclaim(
            &self,
            _volume: &VolumeId,
            op: OpId,
            reason: ExpiryReason,
        ) -> Result<ReclaimPaths> {
            let mut entries = self.entries.lock().expect("entries");
            let stored = entries.get_mut(&op).ok_or_else(|| unknown(op))?;
            stored.summary.state = EntryState::Expiring;
            stored.reason = Some(reason);
            Ok(ReclaimPaths {
                entry: entry_dir(op),
                reap: reap_dir(op),
            })
        }

        fn complete_reclaim(
            &self,
            _volume: &VolumeId,
            op: OpId,
        ) -> Result<nurt_contracts::journal::ExpiredEntry> {
            let mut entries = self.entries.lock().expect("entries");
            let stored = entries.get_mut(&op).ok_or_else(|| unknown(op))?;
            stored.summary.state = EntryState::Expired;
            let reason = stored.reason.unwrap_or(ExpiryReason::Space);
            self.tombs.lock().expect("tombs").push((op, reason));
            Ok(ExpiredEntry { op, reason })
        }

        fn mark_released(&self, _volume: &VolumeId, op: OpId, items: &[u32]) -> Result<()> {
            let mut entries = self.entries.lock().expect("entries");
            let stored = entries.get_mut(&op).ok_or_else(|| unknown(op))?;
            stored.summary.state = EntryState::Partial;
            self.pending.lock().expect("pending").push(op);
            let removed: u64 = items
                .iter()
                .filter_map(|index| stored.items.get(*index as usize))
                .sum();
            stored.summary.bytes = stored.summary.bytes.saturating_sub(removed);
            self.released
                .lock()
                .expect("released")
                .entry(op)
                .or_default()
                .extend_from_slice(items);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::time::Duration;

    use nurt_contracts::fs::FsErrorKind;
    use nurt_contracts::journal::{EntryState, ExpiryReason, OpId, VolumeState};
    use nurt_fs::failpoint::{FailAction, FailPoint, NoFailpoints};

    use super::testing::{TestCatalog, blob, entry_dir, reap_dir};
    use super::*;
    use crate::measure::testing::{OneFault, TestVolume, path, volume_info};

    fn summary(op: u64, bytes: u64, files: u64) -> EntrySummary {
        EntrySummary {
            op: OpId(op),
            state: EntryState::Committed,
            bytes,
            files,
            directories: 1,
            pinned: false,
            overdue: None,
        }
    }

    fn ops(order: &[&EntrySummary]) -> Vec<u64> {
        order.iter().map(|entry| entry.op.0).collect()
    }

    fn model() -> CostModel {
        CostModel {
            per_file: 1e-3,
            per_dir: 1e-3,
            bytes_per_second: 1e6,
        }
    }

    fn reclaimer<'a>(
        volume: &'a TestVolume,
        catalog: &'a TestCatalog,
        failpoints: &'a dyn Failpoints,
        info: &'a VolumeInfo,
    ) -> Reclaimer<'a> {
        Reclaimer {
            fs: volume,
            clock: volume.clock.as_ref(),
            failpoints,
            catalog,
            volume: info,
            quota_aware: true,
            workers: 4,
            poll: Duration::from_millis(50),
            settle: Duration::from_millis(1_000),
        }
    }

    #[test]
    fn reclaim_order_overdue_entries_come_first_oldest_first() {
        let mut entries = vec![
            summary(1, 10, 1),
            summary(2, 1_000_000, 1),
            summary(3, 10, 1),
            summary(4, 10, 1),
        ];
        entries[2].overdue = Some(ExpiryReason::Time);
        entries[3].overdue = Some(ExpiryReason::Ops);
        assert_eq!(ops(&space_order(&entries, &model())), vec![3, 4, 2, 1]);
        assert_eq!(ops(&budget_order(&entries)), vec![3, 4, 1, 2]);
    }

    #[test]
    fn reclaim_order_space_prefers_higher_score_and_lower_number_on_ties() {
        let entries = vec![
            summary(1, 1_000, 1_000),
            summary(2, 5_000_000, 1),
            summary(3, 5_000_000, 1),
            summary(4, 2_000_000, 1),
        ];
        assert_eq!(ops(&space_order(&entries, &model())), vec![2, 3, 4, 1]);
    }

    #[test]
    fn reclaim_order_pinned_entries_are_released_for_space_only() {
        let mut entries = vec![summary(1, 10, 1), summary(2, 10, 1)];
        entries[0].pinned = true;
        assert_eq!(ops(&space_order(&entries, &model())), vec![1, 2]);
        assert_eq!(ops(&budget_order(&entries)), vec![2]);
        entries[0].overdue = Some(ExpiryReason::Time);
        assert_eq!(ops(&budget_order(&entries)), vec![1, 2]);
    }

    #[test]
    fn reclaim_order_skips_entries_without_releasable_data() {
        let mut entries = vec![
            summary(1, 10, 1),
            summary(2, 10, 1),
            summary(3, 10, 1),
            summary(4, 10, 1),
        ];
        entries[0].state = EntryState::Corrupt;
        entries[1].state = EntryState::Prepared;
        entries[2].state = EntryState::Expiring;
        entries[3].state = EntryState::Partial;
        assert_eq!(ops(&space_order(&entries, &model())), vec![4]);
        assert_eq!(ops(&budget_order(&entries)), vec![4]);
    }

    #[test]
    fn reclaim_parallelism_depends_on_medium_and_state() {
        let cores = |n| NonZeroUsize::new(n).expect("cores");
        assert_eq!(parallelism(false, VolumeState::Soft, cores(16)), 8);
        assert_eq!(parallelism(false, VolumeState::Hard, cores(4)), 4);
        assert_eq!(parallelism(true, VolumeState::Hard, cores(16)), 2);
        assert_eq!(parallelism(true, VolumeState::Soft, cores(1)), 1);
        assert_eq!(parallelism(false, VolumeState::Emergency, cores(16)), 16);
        assert_eq!(parallelism(true, VolumeState::Emergency, cores(12)), 12);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn release_entry_moves_to_reclaim_area_removes_and_writes_tombstone() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        let op = catalog.add(&volume, 5, &[100, 200]);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entry = summary(5, 300, 2);
        reclaimer
            .release_entry(&entry, ExpiryReason::Space)
            .expect("release");
        assert!(!volume.exists(&entry_dir(op)));
        assert!(!volume.exists(&reap_dir(op)));
        assert_eq!(catalog.tombs(), vec![(op, ExpiryReason::Space)]);
        assert_eq!(catalog.locks(), vec![op]);
        let log = volume.log();
        assert!(log[0].starts_with("rename") && log[0].contains("/reap/5"));
        assert!(log[1].starts_with("sync_dir") && log[2].starts_with("sync_dir"));
        assert!(log[3].starts_with("remove_tree") && log[3].ends_with("/reap/5"));
        assert_eq!(volume.tree_parallelism(), vec![4]);
    }

    #[test]
    fn crash_after_move_leaves_entry_expiring_in_reclaim_area() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        let op = catalog.add(&volume, 5, &[100]);
        let info = volume_info(false);
        let crash = OneFault {
            point: FailPoint::ReclaimMoved,
            action: FailAction::Crash,
        };
        let reclaimer = reclaimer(&volume, &catalog, &crash, &info);
        assert!(
            reclaimer
                .release_entry(&summary(5, 100, 1), ExpiryReason::Space)
                .is_err()
        );
        assert!(volume.exists(&reap_dir(op)));
        assert_eq!(catalog.state(op), Some(EntryState::Expiring));
        assert!(catalog.tombs().is_empty());
    }

    #[test]
    fn reclaim_partial_hard_releases_largest_items_first() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        let op = catalog.add(&volume, 3, &[100, 900, 50, 900, 400]);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entry = summary(3, 2_350, 5);
        let bytes = reclaimer.release_items(&entry, 1_000).expect("partial");
        assert_eq!(bytes, 1_800);
        assert_eq!(catalog.released_items(op), vec![1, 3]);
        assert_eq!(catalog.state(op), Some(EntryState::Partial));
        assert!(!volume.exists(&blob(op, 1)));
        assert!(!volume.exists(&blob(op, 3)));
        assert!(volume.exists(&blob(op, 4)));
        assert!(volume.exists(&entry_dir(op)));
        assert!(catalog.tombs().is_empty());
        assert_eq!(catalog.locks(), vec![op]);
        assert!(catalog.pending().is_empty());
    }

    #[test]
    fn partial_release_syncs_block_directories_before_clearing_the_marker() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        let op = catalog.add(&volume, 3, &[100, 900]);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        reclaimer
            .release_items(&summary(3, 1_000, 2), 500)
            .expect("partial");
        let log = volume.log();
        let removed = log
            .iter()
            .position(|line| line.starts_with("remove_file"))
            .expect("remove_file");
        let synced = log
            .iter()
            .position(|line| {
                *line
                    == format!(
                        "sync_dir {}",
                        blob(op, 1).as_path().parent().expect("parent").display()
                    )
            })
            .expect("sync_dir of the block directory");
        assert!(removed < synced);
        assert!(catalog.pending().is_empty());
    }

    #[test]
    fn partial_release_reports_injected_fault_after_removal() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        let op = catalog.add(&volume, 3, &[100, 900]);
        let info = volume_info(false);
        let fault = OneFault {
            point: FailPoint::ReclaimPartial,
            action: FailAction::Error(FsErrorKind::Io),
        };
        let reclaimer = reclaimer(&volume, &catalog, &fault, &info);
        assert!(reclaimer.release_items(&summary(3, 1_000, 2), 500).is_err());
        assert_eq!(catalog.released_items(op), vec![1]);
        assert!(!volume.exists(&blob(op, 1)));
        assert_eq!(catalog.pending(), vec![op]);
    }

    #[test]
    fn partial_release_of_entry_without_items_releases_nothing() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        catalog.add(&volume, 3, &[]);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        assert_eq!(
            reclaimer
                .release_items(&summary(3, 0, 0), 10)
                .expect("partial"),
            0
        );
    }

    #[test]
    fn budget_pass_releases_oldest_until_need_is_met() {
        let volume = TestVolume::new(100_000);
        let catalog = TestCatalog::default();
        for op in 1..=4 {
            catalog.add(&volume, op, &[1_000]);
        }
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entries = catalog.entries(&info.id).expect("entries");
        let released = reclaimer.for_budget(&entries, &model(), 1_500, Duration::from_secs(1));
        assert_eq!(released.whole, vec![OpId(1), OpId(2)]);
        assert_eq!(released.bytes, 2_000);
        assert_eq!(
            catalog.tombs(),
            vec![(OpId(1), ExpiryReason::Size), (OpId(2), ExpiryReason::Size)]
        );
    }

    #[test]
    fn reclaim_effect_measured_by_growth_of_free_space() {
        let volume = TestVolume::new(10_000);
        volume.set_release_delay(Duration::from_millis(400));
        let catalog = TestCatalog::default();
        catalog.add(&volume, 1, &[3_000]);
        catalog.add(&volume, 2, &[3_000]);
        volume.set_foreign(3_000);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entries = catalog.entries(&info.id).expect("entries");
        let goal = SpaceGoal {
            target: 3_500,
            deadline: Duration::from_secs(10),
            partial: false,
        };
        let (released, reading) = reclaimer
            .for_space(&entries, &model(), &goal, &Released::default())
            .expect("space");
        assert_eq!(released.whole, vec![OpId(1)]);
        assert_eq!(reading.free, 4_000);
        assert!(volume.clock.monotonic() >= Duration::from_millis(400));
    }

    #[test]
    fn space_pass_releases_more_when_growth_does_not_appear() {
        let volume = TestVolume::new(10_000);
        volume.set_release_delay(Duration::from_secs(30));
        let catalog = TestCatalog::default();
        catalog.add(&volume, 1, &[3_000]);
        catalog.add(&volume, 2, &[2_000]);
        volume.set_foreign(4_000);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entries = catalog.entries(&info.id).expect("entries");
        let goal = SpaceGoal {
            target: 2_000,
            deadline: Duration::from_secs(5),
            partial: false,
        };
        let (released, reading) = reclaimer
            .for_space(&entries, &model(), &goal, &Released::default())
            .expect("space");
        assert_eq!(released.whole, vec![OpId(1), OpId(2)]);
        assert_eq!(reading.free, 1_000);
        assert!(volume.clock.monotonic() >= Duration::from_secs(1));
    }

    #[test]
    fn space_pass_uses_partial_release_when_allowed() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        let op = catalog.add(&volume, 1, &[1_000, 4_000, 2_000]);
        volume.set_foreign(2_500);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entries = catalog.entries(&info.id).expect("entries");
        let goal = SpaceGoal {
            target: 4_000,
            deadline: Duration::from_secs(5),
            partial: true,
        };
        let (released, reading) = reclaimer
            .for_space(&entries, &model(), &goal, &Released::default())
            .expect("space");
        assert_eq!(released.partial, vec![op]);
        assert!(released.whole.is_empty());
        assert_eq!(reading.free, 4_500);
        assert_eq!(catalog.released_items(op), vec![1]);
    }

    #[test]
    fn space_pass_stops_at_deadline() {
        let volume = TestVolume::new(10_000);
        volume.set_per_file(Duration::from_millis(800));
        let catalog = TestCatalog::default();
        for op in 1..=5 {
            catalog.add(&volume, op, &[1_000]);
        }
        volume.set_foreign(5_000);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let entries = catalog.entries(&info.id).expect("entries");
        let goal = SpaceGoal {
            target: 9_000,
            deadline: Duration::from_millis(1_500),
            partial: false,
        };
        let (released, _) = reclaimer
            .for_space(&entries, &model(), &goal, &Released::default())
            .expect("space");
        assert_eq!(released.whole.len(), 2);
    }

    #[test]
    fn space_pass_skips_entries_released_earlier_and_failed_entries() {
        let volume = TestVolume::new(10_000);
        let catalog = TestCatalog::default();
        catalog.add(&volume, 1, &[1_000]);
        catalog.add(&volume, 2, &[1_000]);
        volume.set_foreign(7_000);
        let info = volume_info(false);
        let reclaimer = reclaimer(&volume, &catalog, &NoFailpoints, &info);
        let mut entries = catalog.entries(&info.id).expect("entries");
        entries.push(summary(9, 50_000, 1));
        let skip = Released {
            whole: vec![OpId(1)],
            ..Released::default()
        };
        let goal = SpaceGoal {
            target: 10_000,
            deadline: Duration::from_secs(5),
            partial: false,
        };
        let (released, _) = reclaimer
            .for_space(&entries, &model(), &goal, &skip)
            .expect("space");
        assert_eq!(released.whole, vec![OpId(2)]);
        assert!(volume.exists(&path("vol/j/ops/1/blobs/0")));
    }
}
