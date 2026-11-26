// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Expiry passes and removal order.
//!
//! An entry expires when any limit is exceeded:
//!
//! * steps: the distance of entry `n`, the number of counted committed
//!   entries with a number greater than `n`, reaches `journal.max_ops`;
//! * age: the age of the entry reaches `journal.max_age`;
//! * size: the logical bytes held on a volume exceed its budget, or the
//!   bytes held on all volumes exceed `journal.max_size_total`.
//!
//! Entries over the step or age limit are removed first, oldest first; then
//! entries over a size budget, oldest first. An entry is a unit: a bulk
//! operation and an operation spanning several volumes expire as a whole.
//! Pinned entries and entries kept above the budget with `oversize = keep`
//! are not removed for size. The plan depends only on its input, so the
//! order is deterministic.
//!
//! An entry is reclaimed in three steps: `ops/<N>` is renamed to `reap/<N>`
//! with the state `expiring` and the reason, the tombstone is written, and
//! `reap/<N>` is removed. Each step is idempotent, so an interrupted reclaim
//! is completed by [`complete_reap`].

use std::collections::{BTreeMap, BTreeSet};

use nurt_contracts::config::{AgeLimit, ByteLimit, Oversize, Resolved, StepLimit, resolve};
use nurt_contracts::env::ClockStamp;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    EffectiveRetention, EntryState, ExpiredEntry, ExpiryReason, JournalNotable, OpId, PurgeOutcome,
    PurgeRequest, PurgeSelector, SweepReport,
};
use nurt_fs::failpoint::FailPoint;

use super::age::{AGE_STATE_FILE, AgeEvaluator, AgeState};
use super::ctx::CtxStore;
use super::limits::runtime_limits;
use super::port::{Area, EntryFacts, RetentionStore, StepKind, StepMark, VolumeFacts};
use super::resolve::{ResolutionInput, RetentionResolver, RuntimeLimits};
use super::tomb::{self, Tombstone};
use crate::store::JournalCtx;

/// Whether retention may expire an entry in this state.
pub(crate) fn expirable(state: EntryState) -> bool {
    matches!(
        state,
        EntryState::Committed | EntryState::Partial | EntryState::Undone
    )
}

/// Whether an entry in this state holds bytes counted against the budget.
fn holds_space(state: EntryState) -> bool {
    !matches!(state, EntryState::Expiring | EntryState::Expired)
}

/// Distances of entries in steps.
struct Distances {
    counted: Vec<OpId>,
}

impl Distances {
    fn new(steps: &[StepMark], count_undo_as_op: bool) -> Self {
        let mut counted: Vec<OpId> = steps
            .iter()
            .filter(|mark| match mark.kind {
                StepKind::Operation => true,
                StepKind::Undo => count_undo_as_op,
                StepKind::NotCounted => false,
            })
            .map(|mark| mark.op)
            .collect();
        counted.sort_unstable();
        counted.dedup();
        Self { counted }
    }

    /// Counted entries with a number greater than `op`.
    fn after(&self, op: OpId) -> u64 {
        let newer = self.counted.len() - self.counted.partition_point(|mark| *mark <= op);
        u64::try_from(newer).unwrap_or(u64::MAX)
    }
}

/// Everything an expiry plan depends on.
pub(crate) struct ExpiryInput<'a> {
    /// Entry parts of every volume.
    pub(crate) entries: &'a [EntryFacts],
    /// Committed entries of the account with their step class.
    pub(crate) steps: &'a [StepMark],
    /// Effective limits per volume.
    pub(crate) limits: &'a BTreeMap<AbsPath, EffectiveRetention>,
    /// Whether undo and redo entries count as steps.
    pub(crate) count_undo_as_op: bool,
    /// Budget summed over all volumes, when set.
    pub(crate) max_size_total: Option<u64>,
    /// Entry ages at the moment of the plan.
    pub(crate) ages: AgeEvaluator<'a>,
}

/// Parts of every entry, by number.
type Groups<'a> = BTreeMap<OpId, Vec<&'a EntryFacts>>;

fn grouped<'a>(parts: impl Iterator<Item = &'a EntryFacts>) -> Groups<'a> {
    let mut groups: Groups<'a> = BTreeMap::new();
    for part in parts {
        groups.entry(part.op).or_default().push(part);
    }
    groups
}

/// Parts of the entries of the journal. Entries rolled back before their
/// commit are not entries: limits neither expire them nor count their bytes.
fn groups(entries: &[EntryFacts]) -> Groups<'_> {
    grouped(entries.iter().filter(|part| !part.rolled_back))
}

/// Whether limits may expire the entry: every part is expirable and none
/// is pinned.
fn removable(parts: &[&EntryFacts]) -> bool {
    parts
        .iter()
        .all(|part| expirable(part.state) && !part.pinned)
}

/// Entries to expire, in removal order.
pub(crate) fn plan(input: &ExpiryInput<'_>) -> Vec<ExpiredEntry> {
    let groups = groups(input.entries);
    let mut plan = Vec::new();
    let mut taken = BTreeSet::new();
    let distances = Distances::new(input.steps, input.count_undo_as_op);
    for (op, parts) in &groups {
        if !removable(parts) {
            continue;
        }
        let reason = parts
            .iter()
            .find_map(|part| overdue(part, &distances, input));
        if let Some(reason) = reason {
            plan.push(ExpiredEntry { op: *op, reason });
            taken.insert(*op);
        }
    }
    let mut held = held_bytes(&groups, &taken);
    for (volume, limits) in input.limits {
        if let Some(budget) = limits.max_size_bytes {
            let keep_oversize = limits.oversize.value == Oversize::Keep;
            let over =
                |held: &BTreeMap<AbsPath, u64>| held.get(volume).copied().unwrap_or(0) > budget;
            let victim = |part: &EntryFacts| {
                &part.volume == volume && !(keep_oversize && part.bytes_held > budget)
            };
            evict(&groups, &mut held, &mut taken, &mut plan, over, victim);
        }
    }
    if let Some(total) = input.max_size_total {
        let over = |held: &BTreeMap<AbsPath, u64>| held.values().sum::<u64>() > total;
        evict(&groups, &mut held, &mut taken, &mut plan, over, |_| true);
    }
    plan
}

fn overdue(
    part: &EntryFacts,
    distances: &Distances,
    input: &ExpiryInput<'_>,
) -> Option<ExpiryReason> {
    let limits = input.limits.get(&part.volume)?;
    if let StepLimit::Limited(max) = limits.max_ops.value
        && distances.after(part.op) >= u64::from(max)
    {
        return Some(ExpiryReason::Ops);
    }
    if let AgeLimit::Limited(max) = limits.max_age.value
        && input.ages.age(limits.age_clock.value, &part.basis) >= max.as_std()
    {
        return Some(ExpiryReason::Time);
    }
    None
}

fn held_bytes(groups: &Groups<'_>, taken: &BTreeSet<OpId>) -> BTreeMap<AbsPath, u64> {
    let mut held: BTreeMap<AbsPath, u64> = BTreeMap::new();
    for (op, parts) in groups {
        if taken.contains(op) {
            continue;
        }
        for part in parts.iter().filter(|part| holds_space(part.state)) {
            let sum = held.entry(part.volume.clone()).or_default();
            *sum = sum.saturating_add(part.bytes_held);
        }
    }
    held
}

/// Expires the oldest removable entries with a part accepted by `victim`
/// while `over` holds.
fn evict(
    groups: &Groups<'_>,
    held: &mut BTreeMap<AbsPath, u64>,
    taken: &mut BTreeSet<OpId>,
    plan: &mut Vec<ExpiredEntry>,
    over: impl Fn(&BTreeMap<AbsPath, u64>) -> bool,
    victim: impl Fn(&EntryFacts) -> bool,
) {
    for (op, parts) in groups {
        if !over(held) {
            return;
        }
        if taken.contains(op) || !removable(parts) || !parts.iter().any(|part| victim(part)) {
            continue;
        }
        for part in parts {
            if let Some(sum) = held.get_mut(&part.volume) {
                *sum = sum.saturating_sub(part.bytes_held);
            }
        }
        taken.insert(*op);
        plan.push(ExpiredEntry {
            op: *op,
            reason: ExpiryReason::Size,
        });
    }
}

/// Configuration, clock and entries read once for a retention call.
pub(crate) struct Snapshot {
    /// Merged configuration.
    pub(crate) resolved: Resolved,
    /// Limits of the live sessions.
    pub(crate) runtime: Option<RuntimeLimits>,
    /// Clock readings of the call.
    pub(crate) now: ClockStamp,
    /// Persisted age state.
    pub(crate) age: AgeState,
    /// Volumes of the journal.
    pub(crate) volumes: Vec<VolumeFacts>,
    /// Entry parts of every volume.
    pub(crate) entries: Vec<EntryFacts>,
    /// Committed entries of the account.
    pub(crate) steps: Vec<StepMark>,
}

impl Snapshot {
    /// Reads the state of the store.
    ///
    /// # Errors
    ///
    /// Returns `E_CONFIG` for an invalid configuration and `E_IO` when the
    /// store cannot be read.
    pub(crate) fn load(store: &dyn RetentionStore) -> Result<Self> {
        let resolved = resolve_config(store)?;
        let now = store.clock();
        let runtime = runtime_limits(store, &resolved, &now)?;
        let age = match store.read_file(Area::State, AGE_STATE_FILE)? {
            Some(content) => AgeState::decode(&content).unwrap_or_default(),
            None => AgeState::default(),
        };
        let volumes = store.volumes()?;
        let mut entries = Vec::new();
        for volume in &volumes {
            entries.extend(store.entries(&volume.mount)?);
        }
        Ok(Self {
            resolved,
            runtime,
            now,
            age,
            volumes,
            entries,
            steps: store.steps()?,
        })
    }

    /// Resolver of the snapshot configuration.
    pub(crate) fn resolver(&self) -> RetentionResolver<'_> {
        RetentionResolver::new(&self.resolved, self.runtime.as_ref())
    }

    /// Effective limits of every volume.
    ///
    /// A budget given as a share of free space is computed from the free
    /// bytes of the volume plus the bytes held by the journal there (`F + J`),
    /// so that the journal does not shrink its own budget by growing.
    pub(crate) fn limits(&self) -> BTreeMap<AbsPath, EffectiveRetention> {
        let resolver = self.resolver();
        self.volumes
            .iter()
            .map(|volume| {
                let key = volume.mount.to_language_string();
                let held = self
                    .entries
                    .iter()
                    .filter(|part| {
                        part.volume == volume.mount && !part.rolled_back && holds_space(part.state)
                    })
                    .fold(0_u64, |sum, part| sum.saturating_add(part.bytes_held));
                let input = ResolutionInput {
                    mount: &key,
                    capacity: volume.capacity,
                    free_with_journal: volume.free.saturating_add(held),
                };
                (volume.mount.clone(), resolver.resolve(&input))
            })
            .collect()
    }

    /// Expiry plan under `limits`.
    pub(crate) fn plan(&self, limits: &BTreeMap<AbsPath, EffectiveRetention>) -> Vec<ExpiredEntry> {
        let journal = &self.resolved.config.journal;
        plan(&ExpiryInput {
            entries: &self.entries,
            steps: &self.steps,
            limits,
            count_undo_as_op: journal.count_undo_as_op,
            max_size_total: match journal.max_size_total {
                ByteLimit::Bytes(size) => Some(size.bytes()),
                ByteLimit::Unlimited => None,
            },
            ages: AgeEvaluator::new(&self.age, &self.now),
        })
    }

    /// Expires the planned entries and records the clock observation.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when an entry cannot be reclaimed.
    pub(crate) fn execute(
        &mut self,
        store: &dyn RetentionStore,
        plan: &[ExpiredEntry],
    ) -> Result<u64> {
        let history_keep = self.resolved.config.journal.history_keep;
        discard_rolled_back(store, &self.entries, history_keep)?;
        let bytes = execute(store, &self.entries, plan, history_keep)?;
        self.age.observe(&self.now);
        store.write_file(Area::State, AGE_STATE_FILE, &self.age.encode()?)?;
        Ok(bytes)
    }
}

/// Merges the configuration layers of the store.
///
/// # Errors
///
/// Returns `E_CONFIG` naming the key, level and file of an invalid value.
pub(crate) fn resolve_config(store: &dyn RetentionStore) -> Result<Resolved> {
    resolve(&store.config_layers(), store.project_trusted())
        .map_err(|error| Error::new(ErrorCode::Config, error.to_string()))
}

/// Volumes selected by a `volume` parameter: all volumes, or the volume
/// whose mount point is the longest prefix of the path.
///
/// # Errors
///
/// Returns `E_NOT_FOUND` when no volume contains the path.
pub(crate) fn select_volumes<'a>(
    volumes: &'a [VolumeFacts],
    path: Option<&AbsPath>,
) -> Result<Vec<&'a VolumeFacts>> {
    let Some(path) = path else {
        return Ok(volumes.iter().collect());
    };
    volumes
        .iter()
        .filter(|volume| path.starts_with(&volume.mount))
        .max_by_key(|volume| volume.mount.as_path().components().count())
        .map(|volume| vec![volume])
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NotFound,
                format!("no journal volume contains {path}; omit volume to select all volumes"),
            )
        })
}

/// Reclaims the planned entries: every part is moved to `reap/`, then
/// tombstones are written and the data is removed. Returns the logical
/// bytes released.
fn execute(
    store: &dyn RetentionStore,
    entries: &[EntryFacts],
    plan: &[ExpiredEntry],
    history_keep: u32,
) -> Result<u64> {
    let groups = grouped(entries.iter());
    let mut bytes: u64 = 0;
    let mut touched = BTreeSet::new();
    for expired in plan {
        let parts = groups
            .get(&expired.op)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut locks = Vec::with_capacity(parts.len());
        for part in parts {
            locks.push(store.lock_entry(&part.volume, part.op)?);
            store.begin_reap(&part.volume, part.op, expired.reason)?;
            store.hit(FailPoint::ReclaimMoved)?;
        }
        for part in parts {
            finish(store, part, expired.reason)?;
            bytes = bytes.saturating_add(part.bytes_held);
            touched.insert(&part.volume);
        }
        drop(locks);
    }
    for volume in touched {
        tomb::prune(store, volume, history_keep)?;
    }
    Ok(bytes)
}

/// Removes what entries rolled back before their commit left behind. They
/// are not entries, so no tombstone is written and nothing is reported as
/// expired.
fn discard_rolled_back(
    store: &dyn RetentionStore,
    entries: &[EntryFacts],
    history_keep: u32,
) -> Result<()> {
    let plan: Vec<ExpiredEntry> = grouped(entries.iter())
        .into_iter()
        .filter(|(_, parts)| parts.iter().all(|part| part.rolled_back))
        .map(|(op, _)| ExpiredEntry {
            op,
            reason: ExpiryReason::Space,
        })
        .collect();
    if plan.is_empty() {
        return Ok(());
    }
    execute(store, entries, &plan, history_keep)?;
    Ok(())
}

fn finish(store: &dyn RetentionStore, part: &EntryFacts, reason: ExpiryReason) -> Result<()> {
    if !part.rolled_back {
        tomb::write(store, &part.volume, &Tombstone::of(part, reason))?;
    }
    store.remove_reaped(&part.volume, part.op)
}

/// Expires entries over any limit (steps, age, size).
///
/// # Errors
///
/// Returns `E_CONFIG` for an invalid configuration and `E_IO` when the
/// store cannot be read or an entry cannot be reclaimed.
pub(crate) fn sweep_store(store: &dyn RetentionStore) -> Result<SweepReport> {
    let mut snapshot = Snapshot::load(store)?;
    let limits = snapshot.limits();
    let expired = snapshot.plan(&limits);
    let bytes = snapshot.execute(store, &expired)?;
    Ok(SweepReport { expired, bytes })
}

/// Releases the selected entries immediately, pinned entries included.
///
/// An entry spanning several volumes is released on all of them. The
/// release is recorded in the operation index; it creates no entry and
/// consumes no step.
///
/// # Errors
///
/// Returns `E_NOT_FOUND` when no volume contains the requested path and
/// `E_IO` when an entry cannot be reclaimed.
pub(crate) fn purge_store(
    store: &dyn RetentionStore,
    request: &PurgeRequest,
) -> Result<PurgeOutcome> {
    let mut snapshot = Snapshot::load(store)?;
    let selected: BTreeSet<AbsPath> = select_volumes(&snapshot.volumes, request.volume.as_ref())?
        .into_iter()
        .map(|volume| volume.mount.clone())
        .collect();
    let chosen = |op: OpId| match &request.selector {
        PurgeSelector::All => true,
        PurgeSelector::UpTo(last) => op <= *last,
        PurgeSelector::Ids(ids) => ids.contains(&op),
    };
    let released: Vec<OpId> = snapshot
        .entries
        .iter()
        .filter(|part| {
            selected.contains(&part.volume)
                && !part.rolled_back
                && expirable(part.state)
                && chosen(part.op)
        })
        .map(|part| part.op)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let plan: Vec<ExpiredEntry> = released
        .iter()
        .map(|op| ExpiredEntry {
            op: *op,
            reason: ExpiryReason::Purge,
        })
        .collect();
    let bytes = snapshot.execute(store, &plan)?;
    store.audit(&serde_json::json!({
        "event": "journal.purge",
        "time": snapshot.now.wall,
        "released": released,
        "bytes": bytes,
    }))?;
    Ok(PurgeOutcome { released, bytes })
}

/// Completes the reclaim of an entry moved to `reap/<N>`: writes its
/// tombstones and removes its data on every volume, each part under the lock
/// of the entry. Idempotent: an entry
/// whose reclaim is complete yields the reason of its tombstone.
///
/// A part moved without a recorded reason is reported as reclaimed for
/// space, the only reclaim that does not go through retention.
///
/// # Errors
///
/// Returns `E_JOURNAL_NOOP` when the entry is neither being reclaimed nor
/// expired, and `E_IO` when it cannot be completed.
pub(crate) fn complete_reap_store(store: &dyn RetentionStore, op: OpId) -> Result<ExpiredEntry> {
    let volumes = store.volumes()?;
    let history_keep = resolve_config(store)?.config.journal.history_keep;
    let mut reason = None;
    for volume in &volumes {
        for part in store.entries(&volume.mount)? {
            if part.op == op && part.state == EntryState::Expiring {
                let _lock = store.lock_entry(&volume.mount, op)?;
                let part_reason = part.reason.unwrap_or(ExpiryReason::Space);
                finish(store, &part, part_reason)?;
                tomb::prune(store, &volume.mount, history_keep)?;
                reason = Some(part_reason);
            }
        }
    }
    if reason.is_none() {
        for volume in &volumes {
            if let Some(tombstone) = tomb::read(store, &volume.mount, op)? {
                reason = Some(tombstone.reason);
                break;
            }
        }
    }
    reason
        .map(|reason| ExpiredEntry { op, reason })
        .ok_or_else(|| {
            Error::new(
                ErrorCode::JournalNoop,
                format!(
                    "entry {} is not being reclaimed; list entries with history",
                    op.0
                ),
            )
        })
}

/// Expires the entries of the journal store over any limit (steps, age,
/// size); the expiry pass of `JournalStore::sweep` and the step of
/// `JournalStore::recover` that follows recovery. The recovery before the
/// first transaction of a process does not run it.
///
/// # Errors
///
/// Returns `E_CONFIG` for an invalid configuration, `E_IN_USE` or `E_IO`
/// when an entry lock cannot be taken, and `E_IO` when a journal directory
/// cannot be read or an entry cannot be reclaimed.
pub(crate) fn sweep(ctx: &JournalCtx) -> Result<SweepReport> {
    sweep_store(&CtxStore(ctx))
}

/// Completes the reclaim of an entry of the journal store moved to
/// `reap/<N>`, under the lock of the entry; idempotent. Used by recovery and
/// by reclaim for space.
///
/// # Errors
///
/// Returns `E_JOURNAL_NOOP` when the entry is neither being reclaimed nor
/// expired, `E_IN_USE` when another process holds the entry lock and `E_IO`
/// when the reclaim cannot be completed.
pub(crate) fn complete_reap(ctx: &JournalCtx, op: OpId) -> Result<ExpiredEntry> {
    complete_reap_store(&CtxStore(ctx), op)
}

/// Releases entries of the journal store immediately (`journal.purge`).
///
/// # Errors
///
/// Returns `E_NOT_FOUND` when no journal volume contains the requested path
/// and `E_IO` when an entry cannot be reclaimed or the audit record cannot be
/// written.
pub(crate) fn purge(ctx: &JournalCtx, request: &PurgeRequest) -> Result<PurgeOutcome> {
    purge_store(&CtxStore(ctx), request)
}

/// Decision of the size budget for new journal data on one volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BudgetAdmission {
    /// The data fits after the listed entries expire, oldest first.
    Fits {
        /// Entries to expire for size before the data is kept.
        victims: Vec<OpId>,
    },
    /// The operation exceeds the budget or the single-operation limit and
    /// `oversize = skip`: it runs without preservation, marked
    /// `too_large_for_journal`, and no entry expires.
    TooLarge {
        /// Logical size of the operation.
        size: u64,
        /// Limit it exceeds.
        limit: u64,
    },
}

/// Checks new journal data of an operation against the size limits of a
/// volume.
///
/// `entries` are the entry parts of the volume, `extra` the bytes the
/// journal will additionally hold and `op_size` the logical size of the
/// operation. The data fits when `bytes_held + extra` stays within the
/// budget after the oldest removable entries expire. With `oversize =
/// keep` an operation larger than the budget is kept and every other
/// removable entry expires first.
pub(crate) fn budget_admission(
    entries: &[EntryFacts],
    extra: u64,
    op_size: u64,
    limits: &EffectiveRetention,
) -> BudgetAdmission {
    if let ByteLimit::Bytes(limit) = limits.max_op_size.value
        && op_size > limit.bytes()
    {
        return BudgetAdmission::TooLarge {
            size: op_size,
            limit: limit.bytes(),
        };
    }
    let Some(budget) = limits.max_size_bytes else {
        return BudgetAdmission::Fits {
            victims: Vec::new(),
        };
    };
    if op_size > budget && limits.oversize.value == Oversize::Skip {
        return BudgetAdmission::TooLarge {
            size: op_size,
            limit: budget,
        };
    }
    let groups = groups(entries);
    let mut held: u64 = entries
        .iter()
        .filter(|part| !part.rolled_back && holds_space(part.state))
        .map(|part| part.bytes_held)
        .sum();
    let mut victims = Vec::new();
    for (op, parts) in &groups {
        if held.saturating_add(extra) <= budget {
            break;
        }
        if removable(parts) {
            held = held.saturating_sub(parts.iter().map(|part| part.bytes_held).sum());
            victims.push(*op);
        }
    }
    BudgetAdmission::Fits { victims }
}

/// Note `journal_oversize` shown by a preview when the operation exceeds
/// the size budget or the single-operation limit; the smaller exceeded
/// limit is reported.
pub(crate) fn oversize_note(op_size: u64, limits: &EffectiveRetention) -> Option<JournalNotable> {
    let single = match limits.max_op_size.value {
        ByteLimit::Bytes(limit) => Some(limit.bytes()),
        ByteLimit::Unlimited => None,
    };
    [limits.max_size_bytes, single]
        .into_iter()
        .flatten()
        .filter(|limit| op_size > *limit)
        .min()
        .map(|limit| JournalNotable::JournalOversize {
            size: op_size,
            limit,
        })
}

/// Reserves room in the size budget of the volume mounted at `mount` for an
/// operation that will keep `extra` bytes of previous content, expiring the
/// oldest removable entries first.
///
/// The entries of the volume are read only when a size budget is set and the
/// operation does not already exceed a limit, so a journal without a budget
/// pays nothing at admission. Entries are expired in the same way as by
/// [`sweep`], with the reason `size`.
///
/// # Errors
///
/// Returns `E_CONFIG` for an invalid configuration, `E_IN_USE` or `E_IO` when
/// an entry lock cannot be taken, and `E_IO` when the entries cannot be read
/// or reclaimed.
pub(crate) fn admit_size(
    ctx: &JournalCtx,
    mount: &AbsPath,
    extra: u64,
    op_size: u64,
    limits: &EffectiveRetention,
) -> Result<BudgetAdmission> {
    let limits_only = budget_admission(&[], extra, op_size, limits);
    if limits.max_size_bytes.is_none() || matches!(limits_only, BudgetAdmission::TooLarge { .. }) {
        return Ok(limits_only);
    }
    let store = CtxStore(ctx);
    let entries = store.entries(mount)?;
    let admission = budget_admission(&entries, extra, op_size, limits);
    if let BudgetAdmission::Fits { victims } = &admission
        && !victims.is_empty()
    {
        let plan: Vec<ExpiredEntry> = victims
            .iter()
            .map(|op| ExpiredEntry {
                op: *op,
                reason: ExpiryReason::Size,
            })
            .collect();
        let history_keep = resolve_config(&store)?.config.journal.history_keep;
        execute(&store, &entries, &plan, history_keep)?;
    }
    Ok(admission)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::{ConfigLayer, ConfigLevel};
    use serde_json::{Value, json};

    use super::super::testing::{GIB, MIB, MemStore, entry, mount};
    use super::*;

    /// Store with the volume `/home` of 500 GiB, 200 GiB free.
    fn store() -> MemStore {
        let store = MemStore::new();
        store.volume("/home", 500 * GIB, 200 * GIB);
        store
    }

    fn sized(op: u64, volume: &str, bytes: u64) -> EntryFacts {
        let mut facts = entry(op, volume);
        facts.bytes_held = bytes;
        facts.bytes_physical = bytes;
        facts
    }

    fn expired(report: &SweepReport) -> Vec<(u64, ExpiryReason)> {
        report
            .expired
            .iter()
            .map(|entry| (entry.op.0, entry.reason))
            .collect()
    }

    fn effective(journal: Value) -> EffectiveRetention {
        let values = json!({ "journal": journal });
        let layer = ConfigLayer::new(
            ConfigLevel::User,
            "user.toml",
            values.as_object().cloned().unwrap_or_default(),
        );
        let resolved = resolve(&[layer], false).unwrap();
        RetentionResolver::new(&resolved, None).resolve(&ResolutionInput {
            mount: "/home",
            capacity: 500 * GIB,
            free_with_journal: 200 * GIB,
        })
    }

    #[test]
    fn expires_the_oldest_entry_at_the_thirty_first_operation() {
        let store = store();
        store.commit("/home", 1..=31);
        store.set_clock(40);
        let report = sweep_store(&store).unwrap();
        assert_eq!(expired(&report), [(1, ExpiryReason::Ops)]);
        assert_eq!(report.bytes, MIB);
        assert_eq!(store.live("/home"), (2..=31).collect::<Vec<_>>());
        let tombstone = tomb::read(&store, &mount("/home"), OpId(1))
            .unwrap()
            .unwrap();
        assert_eq!(tombstone.reason, ExpiryReason::Ops);
        assert_eq!(tombstone.state, EntryState::Expired);
        assert!(store.locks_taken() >= 1);
        assert!(
            store
                .read_file(Area::State, AGE_STATE_FILE)
                .unwrap()
                .is_some()
        );
        assert!(sweep_store(&store).unwrap().expired.is_empty());
    }

    #[test]
    fn bulk_and_external_operations_count_one_step_each() {
        let store = store();
        let mut bulk = entry(1, "/home");
        bulk.files = 1000;
        store.add(bulk, StepKind::Operation);
        let mut external = entry(2, "/home");
        external.bytes_held = 0;
        store.add(external, StepKind::Operation);
        store.commit("/home", 3..=30);
        assert!(sweep_store(&store).unwrap().expired.is_empty());
        store.commit("/home", [31]);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Ops)]
        );
    }

    #[test]
    fn undo_counts_as_a_step_unless_disabled() {
        let build = || {
            let store = store();
            store.commit("/home", 1..=29);
            let mut undo = entry(30, "/home");
            undo.kind = nurt_contracts::journal::OpKind::undo();
            store.add(undo, StepKind::Undo);
            store.commit("/home", [31]);
            store
        };
        let counted = build();
        assert_eq!(
            expired(&sweep_store(&counted).unwrap()),
            [(1, ExpiryReason::Ops)]
        );

        let uncounted = build();
        uncounted.layer(
            ConfigLevel::User,
            json!({ "journal": { "count_undo_as_op": false } }),
        );
        assert!(sweep_store(&uncounted).unwrap().expired.is_empty());
    }

    #[test]
    fn entries_that_count_no_step_do_not_age_others() {
        let store = store();
        store.commit("/home", 1..=29);
        store.add(entry(30, "/home"), StepKind::NotCounted);
        store.commit("/home", [32]);
        assert!(sweep_store(&store).unwrap().expired.is_empty());
        store.commit("/home", [33]);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Ops)]
        );
    }

    #[test]
    fn expires_entries_that_reach_the_age_limit() {
        let store = store();
        store.commit("/home", [1, 2]);
        store.set_clock(1801);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Time)]
        );
        assert_eq!(store.live("/home"), [2]);
    }

    #[test]
    fn size_budget_expires_the_oldest_entries_first() {
        let store = store();
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "20gib" } }),
        );
        for op in 1..=3 {
            store.add(sized(op, "/home", 8 * GIB), StepKind::Operation);
        }
        let report = sweep_store(&store).unwrap();
        assert_eq!(expired(&report), [(1, ExpiryReason::Size)]);
        assert_eq!(report.bytes, 8 * GIB);
    }

    #[test]
    fn oversize_keep_protects_an_entry_above_the_budget() {
        let keep = store();
        keep.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "20gib", "oversize": "keep" } }),
        );
        keep.add(sized(1, "/home", 5 * GIB), StepKind::Operation);
        keep.add(sized(2, "/home", 25 * GIB), StepKind::Operation);
        assert_eq!(
            expired(&sweep_store(&keep).unwrap()),
            [(1, ExpiryReason::Size)]
        );
        assert_eq!(keep.live("/home"), [2]);

        let skip = store();
        skip.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "20gib" } }),
        );
        skip.add(sized(1, "/home", 5 * GIB), StepKind::Operation);
        skip.add(sized(2, "/home", 25 * GIB), StepKind::Operation);
        assert_eq!(
            expired(&sweep_store(&skip).unwrap()),
            [(1, ExpiryReason::Size), (2, ExpiryReason::Size)]
        );
    }

    #[test]
    fn volumes_have_independent_budgets() {
        let store = store();
        store.volume("/mnt/data", 500 * GIB, 200 * GIB);
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "10gib" } }),
        );
        store.add(sized(1, "/home", 6 * GIB), StepKind::Operation);
        store.add(sized(2, "/mnt/data", 6 * GIB), StepKind::Operation);
        store.add(sized(3, "/home", 6 * GIB), StepKind::Operation);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Size)]
        );
        assert_eq!(store.live("/mnt/data"), [2]);
    }

    #[test]
    fn total_budget_expires_the_oldest_entry_of_any_volume() {
        let store = store();
        store.volume("/mnt/data", 500 * GIB, 200 * GIB);
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size_total": "10gib" } }),
        );
        store.add(sized(1, "/mnt/data", 6 * GIB), StepKind::Operation);
        store.add(sized(2, "/home", 6 * GIB), StepKind::Operation);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Size)]
        );
        assert_eq!(store.live("/home"), [2]);
    }

    #[test]
    fn overdue_entries_expire_before_the_size_budget_in_a_stable_order() {
        let store = store();
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_ops": 2, "max_size": "2gib" } }),
        );
        store.add(sized(1, "/home", GIB), StepKind::Operation);
        store.add(sized(2, "/home", GIB), StepKind::Operation);
        store.add(sized(3, "/home", GIB + GIB / 2), StepKind::Operation);
        let snapshot = Snapshot::load(&store).unwrap();
        let limits = snapshot.limits();
        let first = snapshot.plan(&limits);
        assert_eq!(first, snapshot.plan(&limits));
        assert_eq!(
            first,
            [
                ExpiredEntry {
                    op: OpId(1),
                    reason: ExpiryReason::Ops
                },
                ExpiredEntry {
                    op: OpId(2),
                    reason: ExpiryReason::Size
                },
            ]
        );
    }

    #[test]
    fn pinned_entries_survive_steps_age_and_size() {
        let store = store();
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "1mib" } }),
        );
        let mut pinned = entry(1, "/home");
        pinned.pinned = true;
        store.add(pinned, StepKind::Operation);
        store.commit("/home", 2..=32);
        store.set_clock(7200);
        let report = sweep_store(&store).unwrap();
        assert!(report.expired.iter().all(|entry| entry.op != OpId(1)));
        assert_eq!(store.live("/home"), [1]);
    }

    #[test]
    fn entry_on_several_volumes_expires_as_a_whole() {
        let store = store();
        store.volume("/mnt/data", 500 * GIB, 200 * GIB);
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "4gib" } }),
        );
        store.add(sized(1, "/home", 5 * GIB), StepKind::Operation);
        store.add(sized(1, "/mnt/data", MIB), StepKind::Operation);
        store.add(sized(2, "/mnt/data", MIB), StepKind::Operation);
        let report = sweep_store(&store).unwrap();
        assert_eq!(expired(&report), [(1, ExpiryReason::Size)]);
        assert_eq!(report.bytes, 5 * GIB + MIB);
        assert_eq!(store.live("/mnt/data"), [2]);
        for volume in ["/home", "/mnt/data"] {
            assert!(
                tomb::read(&store, &mount(volume), OpId(1))
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn purge_releases_all_entries_of_a_volume() {
        let store = store();
        store.volume("/mnt/data", 500 * GIB, 200 * GIB);
        store.commit("/home", [1, 3]);
        store.commit("/mnt/data", [2]);
        let mut pinned = entry(4, "/home");
        pinned.pinned = true;
        store.add(pinned, StepKind::Operation);
        let outcome = purge_store(
            &store,
            &PurgeRequest {
                volume: Some(mount("/home/user/project")),
                selector: PurgeSelector::All,
            },
        )
        .unwrap();
        assert_eq!(outcome.released, [OpId(1), OpId(3), OpId(4)]);
        assert_eq!(outcome.bytes, 3 * MIB);
        assert!(store.live("/home").is_empty());
        assert_eq!(store.live("/mnt/data"), [2]);
        let tombstone = tomb::read(&store, &mount("/home"), OpId(4))
            .unwrap()
            .unwrap();
        assert_eq!(tombstone.reason, ExpiryReason::Purge);
        let audit = store.audit_records();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0]["event"], "journal.purge");
        assert_eq!(audit[0]["released"], json!([1, 3, 4]));
    }

    #[test]
    fn purge_up_to_includes_the_given_entry() {
        let store = store();
        store.commit("/home", 1..=5);
        let outcome = purge_store(
            &store,
            &PurgeRequest {
                volume: None,
                selector: PurgeSelector::UpTo(OpId(3)),
            },
        )
        .unwrap();
        assert_eq!(outcome.released, [OpId(1), OpId(2), OpId(3)]);
        assert_eq!(store.live("/home"), [4, 5]);
    }

    #[test]
    fn purge_by_id_keeps_older_entries() {
        let store = store();
        store.commit("/home", 1..=5);
        let outcome = purge_store(
            &store,
            &PurgeRequest {
                volume: None,
                selector: PurgeSelector::Ids(vec![OpId(4), OpId(9)]),
            },
        )
        .unwrap();
        assert_eq!(outcome.released, [OpId(4)]);
        assert_eq!(store.live("/home"), [1, 2, 3, 5]);
        assert!(store.steps().unwrap().len() == 5);
    }

    #[test]
    fn purge_of_an_unknown_volume_is_not_found() {
        let store = store();
        let error = purge_store(
            &store,
            &PurgeRequest {
                volume: Some(mount("/srv")),
                selector: PurgeSelector::All,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn interrupted_reclaim_is_completed_once() {
        let store = store();
        store.commit("/home", [1, 2]);
        store.set_clock(1801);
        store.fail_at(FailPoint::ReclaimMoved);
        assert!(sweep_store(&store).is_err());
        assert_eq!(store.state("/home", 1), Some(EntryState::Expiring));

        let first = complete_reap_store(&store, OpId(1)).unwrap();
        assert_eq!(
            first,
            ExpiredEntry {
                op: OpId(1),
                reason: ExpiryReason::Time
            }
        );
        assert_eq!(store.state("/home", 1), None);
        assert_eq!(complete_reap_store(&store, OpId(1)).unwrap(), first);
        let error = complete_reap_store(&store, OpId(2)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::JournalNoop);
    }

    #[test]
    fn reclaim_without_a_recorded_reason_is_reported_as_space() {
        let store = store();
        let mut moved = entry(7, "/home");
        moved.state = EntryState::Expiring;
        store.add(moved, StepKind::Operation);
        assert_eq!(
            complete_reap_store(&store, OpId(7)).unwrap().reason,
            ExpiryReason::Space
        );
    }

    #[test]
    fn budget_admission_lists_the_oldest_entries_to_expire() {
        let limits = effective(json!({ "max_size": "4gib" }));
        let mut pinned = sized(1, "/home", GIB);
        pinned.pinned = true;
        let entries = [
            pinned,
            sized(2, "/home", GIB),
            sized(3, "/home", GIB),
            sized(4, "/home", GIB),
        ];
        assert_eq!(
            budget_admission(&entries, 2 * GIB, 2 * GIB, &limits),
            BudgetAdmission::Fits {
                victims: vec![OpId(2), OpId(3)]
            }
        );
        assert_eq!(
            budget_admission(&entries, 0, MIB, &limits),
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
    }

    #[test]
    fn budget_admission_skips_an_operation_larger_than_the_budget() {
        let entries = [sized(1, "/home", GIB)];
        let skip = effective(json!({ "max_size": "4gib" }));
        assert_eq!(
            budget_admission(&entries, 5 * GIB, 5 * GIB, &skip),
            BudgetAdmission::TooLarge {
                size: 5 * GIB,
                limit: 4 * GIB
            }
        );
        let keep = effective(json!({ "max_size": "4gib", "oversize": "keep" }));
        assert_eq!(
            budget_admission(&entries, 5 * GIB, 5 * GIB, &keep),
            BudgetAdmission::Fits {
                victims: vec![OpId(1)]
            }
        );
        let single = effective(json!({ "max_op_size": "1gib" }));
        assert_eq!(
            budget_admission(&entries, 2 * GIB, 2 * GIB, &single),
            BudgetAdmission::TooLarge {
                size: 2 * GIB,
                limit: GIB
            }
        );
        let unlimited = effective(json!({}));
        assert_eq!(
            budget_admission(&entries, 900 * GIB, 900 * GIB, &unlimited),
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
    }

    #[test]
    fn oversize_note_reports_the_smaller_exceeded_limit() {
        let limits = effective(json!({ "max_size": "4gib", "max_op_size": "2gib" }));
        assert_eq!(
            oversize_note(3 * GIB, &limits),
            Some(JournalNotable::JournalOversize {
                size: 3 * GIB,
                limit: 2 * GIB
            })
        );
        assert_eq!(oversize_note(GIB, &limits), None);
        assert_eq!(oversize_note(GIB, &effective(json!({}))), None);
    }

    #[test]
    fn step_distance_counts_only_newer_counted_entries() {
        let marks = [
            StepMark {
                op: OpId(1),
                kind: StepKind::Operation,
            },
            StepMark {
                op: OpId(2),
                kind: StepKind::Operation,
            },
            StepMark {
                op: OpId(3),
                kind: StepKind::NotCounted,
            },
            StepMark {
                op: OpId(4),
                kind: StepKind::Undo,
            },
            StepMark {
                op: OpId(4),
                kind: StepKind::Undo,
            },
        ];
        let counted = Distances::new(&marks, true);
        assert_eq!(counted.after(OpId(0)), 3);
        assert_eq!(counted.after(OpId(1)), 2);
        assert_eq!(counted.after(OpId(3)), 1);
        assert_eq!(counted.after(OpId(4)), 0);
        assert_eq!(counted.after(OpId(u64::MAX)), 0);
        let uncounted = Distances::new(&marks, false);
        assert_eq!(uncounted.after(OpId(0)), 2);
        assert_eq!(Distances::new(&[], true).after(OpId(0)), 0);
    }

    #[test]
    fn sweep_of_an_empty_journal_expires_nothing() {
        let store = store();
        let report = sweep_store(&store).unwrap();
        assert!(report.expired.is_empty());
        assert_eq!(report.bytes, 0);
        let empty = MemStore::new();
        assert!(sweep_store(&empty).unwrap().expired.is_empty());
    }

    #[test]
    fn entry_expires_exactly_when_its_age_reaches_the_limit() {
        let store = store();
        store.commit("/home", [1]);
        store.set_clock(1800);
        assert!(sweep_store(&store).unwrap().expired.is_empty());
        store.set_clock(1801);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Time)]
        );
    }

    #[test]
    fn size_budget_is_exceeded_only_by_a_byte_above_it() {
        let store = store();
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "20gib" } }),
        );
        store.add(sized(1, "/home", 10 * GIB), StepKind::Operation);
        store.add(sized(2, "/home", 10 * GIB), StepKind::Operation);
        assert!(sweep_store(&store).unwrap().expired.is_empty());
        store.add(sized(3, "/home", 1), StepKind::Operation);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Size)]
        );
        assert_eq!(store.live("/home"), [2, 3]);
    }

    #[test]
    fn total_budget_is_exceeded_only_by_a_byte_above_it() {
        let store = store();
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size_total": "10gib" } }),
        );
        store.add(sized(1, "/home", 5 * GIB), StepKind::Operation);
        store.add(sized(2, "/home", 5 * GIB), StepKind::Operation);
        assert!(sweep_store(&store).unwrap().expired.is_empty());
        store.add(sized(3, "/home", 1), StepKind::Operation);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Size)]
        );
    }

    #[test]
    fn unlimited_steps_and_age_never_expire_entries() {
        let store = store();
        store.layer(
            ConfigLevel::User,
            json!({ "journal": { "max_ops": "unlimited", "max_age": "unlimited" } }),
        );
        store.commit("/home", 1..=100);
        store.set_clock(10 * 365 * 24 * 3600);
        assert!(sweep_store(&store).unwrap().expired.is_empty());
        assert_eq!(store.live("/home").len(), 100);
    }

    #[test]
    fn entries_in_transient_or_corrupt_states_are_never_expired_by_limits() {
        let store = store();
        let mut prepared = entry(1, "/home");
        prepared.state = EntryState::Prepared;
        store.add(prepared, StepKind::Operation);
        let mut corrupt = entry(2, "/home");
        corrupt.state = EntryState::Corrupt;
        store.add(corrupt, StepKind::Operation);
        store.commit("/home", 3..=33);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(3, ExpiryReason::Ops)]
        );
        let mut live = vec![1, 2];
        live.extend(4..=33);
        assert_eq!(store.live("/home"), live);
    }

    #[test]
    fn undone_entries_expire_like_available_ones() {
        let store = store();
        let mut undone = entry(1, "/home");
        undone.state = EntryState::Undone;
        store.add(undone, StepKind::Operation);
        store.commit("/home", 2..=31);
        assert_eq!(
            expired(&sweep_store(&store).unwrap()),
            [(1, ExpiryReason::Ops)]
        );
    }

    #[test]
    fn entries_rolled_back_before_commit_leave_no_tombstone_and_no_report() {
        let store = store();
        let mut rolled_back = entry(1, "/home");
        rolled_back.state = EntryState::Undone;
        rolled_back.rolled_back = true;
        store.add(rolled_back, StepKind::Operation);
        let report = sweep_store(&store).unwrap();
        assert!(report.expired.is_empty());
        assert_eq!(report.bytes, 0);
        assert!(store.live("/home").is_empty());
        assert!(tomb::list(&store, &mount("/home")).unwrap().is_empty());
    }

    #[test]
    fn purge_of_nothing_selected_releases_nothing_but_is_audited() {
        let store = store();
        store.commit("/home", 1..=3);
        for selector in [
            PurgeSelector::UpTo(OpId(0)),
            PurgeSelector::Ids(Vec::new()),
        ] {
            let outcome = purge_store(
                &store,
                &PurgeRequest {
                    volume: None,
                    selector,
                },
            )
            .unwrap();
            assert!(outcome.released.is_empty());
            assert_eq!(outcome.bytes, 0);
        }
        assert_eq!(store.live("/home"), [1, 2, 3]);
        assert_eq!(store.audit_records().len(), 2);
    }

    #[test]
    fn select_volumes_takes_the_longest_mount_that_contains_the_path() {
        let facts = |path: &str| VolumeFacts {
            mount: mount(path),
            capacity: GIB,
            free: GIB,
            inactive: None,
        };
        let volumes = [facts("/"), facts("/mnt/data"), facts("/mnt/data/archive")];
        let selected = |path: &str| {
            select_volumes(&volumes, Some(&mount(path)))
                .map(|found| found.iter().map(|v| v.mount.clone()).collect::<Vec<_>>())
        };
        assert_eq!(
            selected("/mnt/data/archive/2026").unwrap(),
            [mount("/mnt/data/archive")]
        );
        assert_eq!(selected("/mnt/data").unwrap(), [mount("/mnt/data")]);
        assert_eq!(selected("/mnt/data2/x").unwrap(), [mount("/")]);
        assert_eq!(select_volumes(&volumes, None).unwrap().len(), 3);
        let without_root = &volumes[1..];
        let error = select_volumes(without_root, Some(&mount("/srv"))).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(select_volumes(&[], None).unwrap().is_empty());
    }

    #[test]
    fn admission_at_the_budget_expires_nothing_and_one_byte_more_expires_the_oldest() {
        let limits = effective(json!({ "max_size": "4gib" }));
        let entries = [sized(1, "/home", GIB), sized(2, "/home", GIB)];
        assert_eq!(
            budget_admission(&entries, 2 * GIB, 2 * GIB, &limits),
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
        assert_eq!(
            budget_admission(&entries, 2 * GIB + 1, 2 * GIB + 1, &limits),
            BudgetAdmission::Fits {
                victims: vec![OpId(1)]
            }
        );
        assert_eq!(
            budget_admission(&[], 4 * GIB, 4 * GIB, &limits),
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
    }

    #[test]
    fn single_operation_limit_is_exceeded_only_by_a_byte_above_it() {
        let limits = effective(json!({ "max_op_size": "1gib" }));
        assert_eq!(
            budget_admission(&[], GIB, GIB, &limits),
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
        assert_eq!(
            budget_admission(&[], GIB + 1, GIB + 1, &limits),
            BudgetAdmission::TooLarge {
                size: GIB + 1,
                limit: GIB
            }
        );
        assert_eq!(oversize_note(GIB, &limits), None);
        assert_eq!(
            oversize_note(GIB + 1, &limits),
            Some(JournalNotable::JournalOversize {
                size: GIB + 1,
                limit: GIB
            })
        );
    }

    #[test]
    fn rolled_back_entries_do_not_count_against_the_admission_budget() {
        let limits = effective(json!({ "max_size": "4gib" }));
        let mut rolled_back = sized(1, "/home", 4 * GIB);
        rolled_back.rolled_back = true;
        let entries = [rolled_back, sized(2, "/home", GIB)];
        assert_eq!(
            budget_admission(&entries, 2 * GIB, 2 * GIB, &limits),
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
    }

    #[test]
    fn invalid_configuration_is_reported() {
        let store = store();
        store.layer(ConfigLevel::User, json!({ "journal": { "max_ops": 0 } }));
        assert_eq!(sweep_store(&store).unwrap_err().code(), ErrorCode::Config);
    }
}
