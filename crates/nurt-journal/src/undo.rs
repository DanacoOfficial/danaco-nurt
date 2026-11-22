// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Undo, redo and the rollback of committed entries.
//!
//! An undo runs in three phases. Selection turns the request into a plan:
//! the items to restore, newest entry first and, within an entry, in reverse
//! order of declaration. The check compares every item of the whole plan
//! with the journal before anything is written; a difference changes
//! nothing, consumes no operation number and, depending on `on_conflict`,
//! fails, aborts, extends the plan by later entries or is overridden.
//! Execution is an ordinary journaled transaction of kind `undo` (or `redo`):
//! the current state of every target is preserved in the new entry first, so
//! the undo itself can be undone, then the items are restored.
//!
//! # Ranges and undo entries
//!
//! Undo and redo entries are ordinary entries and appear inside ranges
//! (`undo to:`, `undo file:`). An item of such an entry reverses the item it
//! names as its source. Within a range, an item is undone only when the chain
//! of sources that lies inside the range has even length: an undo whose
//! source is in the range cancels it and is skipped together with it, while a
//! redo of such an undo restores the effect and is undone. The same rule
//! decides whether a later entry still changes an item.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, FsError, FsErrorKind, LinkKind, MetaChange, ProviderSnapshot,
    ScopePurpose,
};
use nurt_contracts::journal::{
    BeginRequest, CloneMode, Conflict, ConflictKind, EntryState, Expect, Mutation, MutationTx,
    OnConflict, OpId, OpKind, PreserveMode, Preserved, RedoRequest, RollbackReason, RollbackReport,
    SessionId, Since, SkippedEntry, TargetAction, TargetIndex, TargetOutcome, TargetPath,
    TargetSpec, UndoOutcome, UndoRequest, UndoScope, UndoSelector, UndoneCause,
};
use nurt_contracts::report::IrreversibleEntry;
use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints};

use crate::blob;
use crate::conflict::{self, Expected, ItemKey, TargetProbe};
use crate::history::{
    EntryItem, EntrySource, ItemSource, JournalEntry, RecordIndex, StoredPart, record_index,
    stored_parts,
};
use crate::layout::EntryLayout;
use crate::manifest::{
    self, BeforeRecord, ItemRef, ItemState, Manifest, StateRecord, TargetRecord, TargetRole,
};
use crate::store::{self, JournalCtx};
use crate::tx::{self, Link};

/// Whether undo and redo entries count as steps (`journal.count_undo_as_op`).
const COUNT_UNDO_AS_OP: bool = true;

/// Permission bits of a restored directory whose bits were not recorded.
const DEFAULT_DIR_MODE: u32 = 0o755;

/// One item to restore.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RestoreItem {
    /// Entry and index of the item being reversed.
    pub(crate) source: ItemSource,
    /// The item as recorded by its entry.
    pub(crate) target: TargetOutcome,
    /// Role of the record for renames across volumes.
    pub(crate) role: TargetRole,
    /// State the address where the entry is now (the destination of a
    /// rename, otherwise the target) must have immediately before the
    /// restore; checked atomically with the change.
    pub(crate) expect: Expect,
}

/// Transaction parameters of an undo or redo entry.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RestorePlan {
    /// `undo` or `redo`.
    pub(crate) kind: OpKind,
    /// Session of the caller; for a rollback, the session of the source.
    pub(crate) session: Option<SessionId>,
    /// Bound program of the call; `None` for a rollback after a failed
    /// check, which records the program of the source entry.
    pub(crate) program: Option<BoundDocument>,
    /// The entry counts towards the step limit (`journal.count_undo_as_op`).
    pub(crate) counts_as_step: bool,
    /// Scope recorded in the entry.
    pub(crate) scope: UndoScope,
    /// Entries undone, newest first; empty for a redo.
    pub(crate) undo_of: Vec<OpId>,
    /// Undo entry redone.
    pub(crate) redo_of: Option<OpId>,
    /// Items in restore order; recorded as the `items` of the entry.
    pub(crate) items: Vec<RestoreItem>,
    /// Cause recorded on source entries that become undone.
    pub(crate) cause: UndoneCause,
}

/// Result of a committed restoring transaction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Executed {
    /// New entry.
    pub(crate) entry: OpId,
    /// Outcome of every restored item, in plan order.
    pub(crate) restored: Vec<TargetOutcome>,
    /// Irreversible markers of the new entry (current state not preserved).
    pub(crate) irreversible: Vec<IrreversibleEntry>,
}

/// Transaction that restores the items of a plan.
pub(crate) trait RestoreTx {
    /// Assigns the number, writes the prepared entry and preserves the
    /// current state of every target of the plan before any change. Lack of
    /// space never fails: the target is restored without preservation and
    /// marked irreversible.
    fn preserve(&mut self) -> Result<()>;

    /// Restores item `n` of the plan from the data of its source entry,
    /// after checking its expected state.
    fn restore(&mut self, n: usize) -> Result<TargetOutcome>;

    /// Makes the new entry durable (`committed`), then records its effect on
    /// the source entries of the plan with [`update_sources`] while the entry
    /// is still pending, and only then removes it from the pending entries.
    /// After a crash in between, recovery repeats the source updates with
    /// [`apply_source_updates`]. A failure before the entry is durable rolls
    /// the transaction back.
    fn commit(self: Box<Self>) -> Result<Executed>;

    /// Restores the targets changed so far in reverse order.
    fn rollback(self: Box<Self>) -> Result<RollbackReport>;
}

/// Journal services used by undo and redo.
pub(crate) trait UndoBackend: EntrySource + TargetProbe {
    /// Opens the restoring transaction of a plan.
    fn begin_restore<'a>(&'a self, plan: &'a RestorePlan) -> Box<dyn RestoreTx + 'a>;

    /// Records in the item state file of `op` that its items were undone by
    /// the entry `by`.
    fn mark_items_undone(&self, op: OpId, items: &[TargetIndex], by: OpId) -> Result<()>;

    /// Changes the persistent state of an entry.
    fn set_state(&self, op: OpId, state: EntryState, cause: Option<UndoneCause>) -> Result<()>;

    /// Fault injection registry.
    fn failpoints(&self) -> &dyn Failpoints;

    /// Whether undo and redo entries count as steps.
    fn count_undo_as_op(&self) -> bool;
}

/// Undoes the entries of the journal of the account selected by the request.
///
/// Fails as [`undo_with`].
pub(crate) fn undo(ctx: &JournalCtx, request: &UndoRequest) -> Result<UndoOutcome> {
    undo_with(ctx, request)
}

/// Redoes an undo entry of the journal of the account.
///
/// Fails as [`undo_with`].
pub(crate) fn redo(ctx: &JournalCtx, request: &RedoRequest) -> Result<UndoOutcome> {
    redo_with(ctx, request)
}

/// Undoes a committed entry of the journal of the account because a check
/// after its commit failed.
///
/// Fails as [`rollback_committed_with`].
pub(crate) fn rollback_committed(
    ctx: &JournalCtx,
    op: OpId,
    reason: RollbackReason,
) -> Result<UndoOutcome> {
    rollback_committed_with(ctx, op, reason)
}

/// Undoes the entries selected by the request.
///
/// Fails with `E_JOURNAL_NOOP`, `E_JOURNAL_EXPIRED`, `E_JOURNAL_CORRUPT`,
/// `E_JOURNAL_PARTIAL` or `E_JOURNAL_CONFLICT` before anything is written,
/// and with the error of the restoring transaction (`E_STALE`,
/// `E_ROLLBACK_FAILED`, a file system error) after it was rolled back.
pub(crate) fn undo_with<S>(store: &S, request: &UndoRequest) -> Result<UndoOutcome>
where
    S: UndoBackend + ?Sized,
{
    let session = request.session.as_ref();
    let view = View::load(store, &stop_for(&request.selector, session))?;
    let mut plan = select(store, &view, request)?;
    let forced = settle(store, &view, &mut plan, request.on_conflict)?;
    if plan.items.is_empty() {
        return Err(conflict::nothing_error(
            "nothing left to undo in the selection; list operations with history",
        ));
    }
    let restore = RestorePlan {
        kind: OpKind::undo(),
        session: request.session.clone(),
        program: Some(request.program.clone()),
        counts_as_step: store.count_undo_as_op(),
        scope: plan.scope,
        undo_of: plan.sources(),
        redo_of: None,
        items: restore_items(store, &plan, &forced)?,
        cause: UndoneCause::Undo,
    };
    let executed = execute(store, &restore)?;
    Ok(outcome(executed, plan, forced))
}

/// Redoes an undo entry: undoes it, with the same check as `undo` and
/// `on_conflict: report`.
///
/// Fails as [`undo_with`].
pub(crate) fn redo_with<S>(store: &S, request: &RedoRequest) -> Result<UndoOutcome>
where
    S: UndoBackend + ?Sized,
{
    let session = request.session.as_ref();
    let stop = match request.undo_entry {
        Some(op) => Stop::AtOrBelow(op),
        None => Stop::RedoCandidate(session),
    };
    let view = View::load(store, &stop)?;
    let (mut plan, undo_entry) = select_redo(&view, request.undo_entry, session)?;
    let forced = settle(store, &view, &mut plan, OnConflict::Report)?;
    if plan.items.is_empty() {
        return Err(conflict::noop_error(undo_entry, None).with_problem("nothing_restorable"));
    }
    let restore = RestorePlan {
        kind: OpKind::redo(),
        session: request.session.clone(),
        program: Some(request.program.clone()),
        counts_as_step: store.count_undo_as_op(),
        scope: UndoScope::Op,
        undo_of: Vec::new(),
        redo_of: Some(undo_entry),
        items: restore_items(store, &plan, &forced)?,
        cause: UndoneCause::Undo,
    };
    let executed = execute(store, &restore)?;
    Ok(outcome(executed, plan, forced))
}

/// Undoes a committed entry because a check after its commit failed.
///
/// Differences found by the check do not stop the rollback; they are
/// reported in `forced`, and the current state is preserved in the new
/// entry. The source entry becomes undone with the cause of the reason.
///
/// Fails with `E_JOURNAL_NOOP` when the entry does not exist, is not
/// committed or has nothing to restore, and with `E_ROLLBACK_FAILED` listing
/// the items when the restore fails.
pub(crate) fn rollback_committed_with<S>(
    store: &S,
    op: OpId,
    reason: RollbackReason,
) -> Result<UndoOutcome>
where
    S: UndoBackend + ?Sized,
{
    let view = View::load(store, &Stop::AtOrBelow(op))?;
    let entry = match view.get(op) {
        Some(entry) if entry.state == EntryState::Committed => entry,
        found => return Err(conflict::noop_error(op, found.map(|entry| entry.state))),
    };
    let mut plan = Plan::new(UndoScope::Op);
    plan.add_entry(entry, |_| true);
    if plan.items.is_empty() {
        return Err(conflict::noop_error(op, Some(entry.state)).with_problem("nothing_restorable"));
    }
    let forced = check_plan(store, &view, &plan)?;
    let restore = RestorePlan {
        kind: OpKind::undo(),
        session: entry.session.clone(),
        program: None,
        counts_as_step: store.count_undo_as_op(),
        scope: UndoScope::Op,
        undo_of: vec![op],
        redo_of: None,
        items: restore_items(store, &plan, &forced)?,
        cause: cause_of(reason),
    };
    let executed = execute(store, &restore).map_err(|error| {
        if error.code == ErrorCode::RollbackFailed || error.effect_applied {
            error
        } else {
            let paths: Vec<TargetPath> = plan
                .items
                .iter()
                .map(|planned| planned.item.target.path.clone())
                .collect();
            conflict::rollback_failed_error(&paths).with_cause(error)
        }
    })?;
    Ok(outcome(executed, plan, forced))
}

fn cause_of(reason: RollbackReason) -> UndoneCause {
    match reason {
        RollbackReason::VerifyFailed => UndoneCause::VerifyFailed,
        RollbackReason::ValidateFailed => UndoneCause::ValidateFailed,
        _ => UndoneCause::RolledBack,
    }
}

/// Entries loaded for one call, newest first.
struct View {
    entries: Vec<JournalEntry>,
    index: HashMap<OpId, usize>,
    /// Positions of the items by target index, for entries whose item at
    /// position `n` does not have target index `n` (entries with several
    /// parts); the first item of an index is kept.
    positions: HashMap<OpId, HashMap<TargetIndex, usize>>,
}

/// Last entry a call needs, scanning from the newest.
enum Stop<'a> {
    /// Every entry.
    Never,
    /// Down to the first entry with a number at or below this one.
    AtOrBelow(OpId),
    /// Down to the default candidate of `undo`.
    UndoCandidate(Option<&'a SessionId>),
    /// Down to the default candidate of `redo`.
    RedoCandidate(Option<&'a SessionId>),
}

impl Stop<'_> {
    fn reached(&self, entry: &JournalEntry) -> bool {
        match self {
            Self::Never => false,
            Self::AtOrBelow(op) => entry.op <= *op,
            Self::UndoCandidate(session) => is_undo_candidate(entry, *session),
            Self::RedoCandidate(session) => is_redo_candidate(entry, *session),
        }
    }
}

fn is_undo_candidate(entry: &JournalEntry, session: Option<&SessionId>) -> bool {
    entry.is_available() && !entry.is_undo() && entry.in_session(session)
}

fn is_redo_candidate(entry: &JournalEntry, session: Option<&SessionId>) -> bool {
    entry.is_available() && entry.is_undo() && entry.in_session(session)
}

fn stop_for<'a>(selector: &UndoSelector, session: Option<&'a SessionId>) -> Stop<'a> {
    match selector {
        UndoSelector::Last => Stop::UndoCandidate(session),
        UndoSelector::Op(op)
        | UndoSelector::Paths { op, .. }
        | UndoSelector::To { after: op, .. }
        | UndoSelector::File {
            since: Some(Since::Op(op)),
            ..
        } => Stop::AtOrBelow(*op),
        UndoSelector::File { .. } => Stop::Never,
    }
}

impl View {
    fn load<S>(store: &S, stop: &Stop<'_>) -> Result<Self>
    where
        S: EntrySource + ?Sized,
    {
        let mut entries = Vec::new();
        for op in store.entry_ids()? {
            let Some(entry) = store.load(op)? else {
                continue;
            };
            let reached = stop.reached(&entry);
            entries.push(entry);
            if reached {
                break;
            }
        }
        Ok(Self::from_entries(entries))
    }

    fn from_entries(entries: Vec<JournalEntry>) -> Self {
        let index = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.op, position))
            .collect();
        let positions = entries
            .iter()
            .filter(|entry| !is_positional(entry))
            .map(|entry| {
                let mut positions = HashMap::with_capacity(entry.items.len());
                for (position, item) in entry.items.iter().enumerate() {
                    positions.entry(item.target.index).or_insert(position);
                }
                (entry.op, positions)
            })
            .collect();
        Self {
            entries,
            index,
            positions,
        }
    }

    fn get(&self, op: OpId) -> Option<&JournalEntry> {
        self.index.get(&op).map(|&position| &self.entries[position])
    }

    /// Item of a loaded entry with the given target index.
    fn item<'v>(&'v self, entry: &'v JournalEntry, index: TargetIndex) -> Option<&'v EntryItem> {
        let position = match self.positions.get(&entry.op) {
            Some(positions) => *positions.get(&index)?,
            None => usize::try_from(index.0).ok()?,
        };
        entry.items.get(position)
    }

    fn newest_first(&self) -> impl Iterator<Item = &JournalEntry> {
        self.entries.iter()
    }
}

/// Whether every item of the entry is stored at the position of its target
/// index.
fn is_positional(entry: &JournalEntry) -> bool {
    entry.items.iter().enumerate().all(|(position, item)| {
        usize::try_from(item.target.index.0).is_ok_and(|index| index == position)
    })
}

/// Entries whose effect a range covers.
#[derive(Debug, Clone, Copy)]
enum Bound {
    All,
    After(OpId),
    Since(WallTime),
}

impl Bound {
    fn of(since: Option<&Since>) -> Self {
        match since {
            None => Self::All,
            Some(Since::Op(op)) => Self::After(*op),
            Some(Since::Time(time)) => Self::Since(*time),
        }
    }

    fn contains(self, entry: &JournalEntry) -> bool {
        match self {
            Self::All => true,
            Self::After(op) => entry.op > op,
            Self::Since(time) => entry.time >= time,
        }
    }
}

/// Whether the change of an item is still in effect with respect to the
/// range: the chain of its sources inside the range has even length.
fn net_applied(view: &View, item: &EntryItem, bound: Bound) -> bool {
    let mut applied = true;
    let mut next = item.source;
    while let Some(source) = next {
        let Some(entry) = view.get(source.op).filter(|entry| bound.contains(entry)) else {
            break;
        };
        applied = !applied;
        next = view
            .item(entry, source.index)
            .and_then(|found| found.source);
    }
    applied
}

/// Item of the plan.
#[derive(Debug, Clone)]
struct Planned {
    op: OpId,
    item: EntryItem,
}

/// Selection of an undo or redo.
#[derive(Debug)]
struct Plan {
    scope: UndoScope,
    /// Items in restore order.
    items: Vec<Planned>,
    /// Entries considered, including those that contributed no item.
    entries: BTreeSet<OpId>,
    skipped: Vec<SkippedEntry>,
    /// Items released for space.
    released: Vec<TargetPath>,
    /// Entries with items released for space.
    released_in: Vec<OpId>,
    /// Items whose previous state was never kept.
    unpreserved: Vec<TargetPath>,
    /// Ready programs offered with a conflict.
    fixes: Vec<String>,
}

impl Plan {
    fn new(scope: UndoScope) -> Self {
        Self {
            scope,
            items: Vec::new(),
            entries: BTreeSet::new(),
            skipped: Vec::new(),
            released: Vec::new(),
            released_in: Vec::new(),
            unpreserved: Vec::new(),
            fixes: Vec::new(),
        }
    }

    /// Adds the items of an entry selected by `keep` that are still in
    /// effect, in reverse order of declaration.
    fn add_entry(&mut self, entry: &JournalEntry, keep: impl Fn(&EntryItem) -> bool) {
        self.entries.insert(entry.op);
        let mut released = false;
        for item in entry.items.iter().rev() {
            if !item.is_in_effect() || !keep(item) {
                continue;
            }
            if item.is_restorable() {
                self.items.push(Planned {
                    op: entry.op,
                    item: item.clone(),
                });
            } else if item.released {
                self.released.push(item.target.path.clone());
                released = true;
            } else {
                self.unpreserved.push(item.target.path.clone());
            }
        }
        if released {
            self.released_in.push(entry.op);
        }
    }

    /// Restores the order newest entry first after entries were added.
    fn sort(&mut self) {
        self.items
            .sort_by_key(|planned| std::cmp::Reverse(planned.op));
    }

    /// Entries with items in the plan, newest first.
    fn sources(&self) -> Vec<OpId> {
        let mut sources: Vec<OpId> = self.items.iter().map(|planned| planned.op).collect();
        sources.dedup();
        sources
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty() && self.released.is_empty()
    }
}

fn entry_gate(view: &View, op: OpId) -> Result<&JournalEntry> {
    let entry = view.get(op).ok_or_else(|| conflict::noop_error(op, None))?;
    match entry.state {
        EntryState::Committed | EntryState::Partial => Ok(entry),
        EntryState::Expiring | EntryState::Expired => Err(conflict::expired_error(&[op])),
        EntryState::Corrupt => Err(conflict::corrupt_error(op)),
        state => Err(conflict::noop_error(op, Some(state))),
    }
}

fn select<S>(store: &S, view: &View, request: &UndoRequest) -> Result<Plan>
where
    S: TargetProbe + ?Sized,
{
    let session = request.session.as_ref();
    let mut plan = match &request.selector {
        UndoSelector::Last => select_last(view, session)?,
        UndoSelector::Op(op) => select_op(view, *op)?,
        UndoSelector::To {
            after,
            all_sessions,
        } => select_to(view, *after, *all_sessions, session)?,
        UndoSelector::File { path, since } => select_file(store, view, path, since.as_ref())?,
        UndoSelector::Paths { op, paths } => select_paths(store, view, *op, paths)?,
    };
    plan.fixes = fixes(&request.selector, &plan);
    Ok(plan)
}

fn select_last(view: &View, session: Option<&SessionId>) -> Result<Plan> {
    let entry = view
        .newest_first()
        .find(|entry| is_undo_candidate(entry, session))
        .ok_or_else(|| {
            conflict::nothing_error("nothing to undo in this session; list operations with history")
        })?;
    select_op(view, entry.op)
}

fn select_op(view: &View, op: OpId) -> Result<Plan> {
    let entry = entry_gate(view, op)?;
    let mut plan = Plan::new(UndoScope::Op);
    plan.add_entry(entry, |_| true);
    if plan.is_empty() {
        return Err(conflict::noop_error(op, Some(entry.state)).with_problem("nothing_restorable"));
    }
    Ok(plan)
}

fn select_to(
    view: &View,
    after: OpId,
    all_sessions: bool,
    session: Option<&SessionId>,
) -> Result<Plan> {
    let bound = Bound::After(after);
    let mut plan = Plan::new(UndoScope::To);
    let mut expired = Vec::new();
    for entry in view.newest_first().filter(|entry| bound.contains(entry)) {
        if !all_sessions && !entry.in_session(session) {
            if entry.is_available() {
                plan.skipped.push(SkippedEntry {
                    op: entry.op,
                    session: entry.session.clone(),
                });
            }
            continue;
        }
        match entry.state {
            EntryState::Committed | EntryState::Partial => {
                plan.add_entry(entry, |item| net_applied(view, item, bound));
            }
            EntryState::Expiring | EntryState::Expired => expired.push(entry.op),
            EntryState::Corrupt => return Err(conflict::corrupt_error(entry.op)),
            _ => {}
        }
    }
    if !expired.is_empty() {
        return Err(conflict::expired_error(&expired));
    }
    if plan.is_empty() {
        return Err(conflict::nothing_error(&format!(
            "nothing to undo after operation {}; list operations with history",
            after.0
        )));
    }
    Ok(plan)
}

fn select_file<S>(store: &S, view: &View, path: &AbsPath, since: Option<&Since>) -> Result<Plan>
where
    S: TargetProbe + ?Sized,
{
    let key = store.item_key(&TargetPath::Fs(path.clone()))?;
    let bound = Bound::of(since);
    let mut plan = Plan::new(UndoScope::File);
    let mut expired = Vec::new();
    for entry in view.newest_first().filter(|entry| bound.contains(entry)) {
        let touching = touching_items(store, entry, &key)?;
        match entry.state {
            EntryState::Committed | EntryState::Partial => {
                plan.add_entry(entry, |item| {
                    touching.contains(&item.target.index) && net_applied(view, item, bound)
                });
            }
            EntryState::Expiring | EntryState::Expired if since.is_some() => {
                expired.push(entry.op);
            }
            EntryState::Corrupt if since.is_some() || !touching.is_empty() => {
                return Err(conflict::corrupt_error(entry.op));
            }
            _ => {}
        }
    }
    if !expired.is_empty() {
        return Err(conflict::expired_error(&expired));
    }
    if plan.is_empty() {
        return Err(conflict::nothing_error(&format!(
            "no available operation changed {}; list its operations with history path: {}",
            path.to_language_string(),
            quoted(&path.to_language_string())
        )));
    }
    Ok(plan)
}

/// Target indexes of the items of an entry that address the key.
fn touching_items<S>(store: &S, entry: &JournalEntry, key: &ItemKey) -> Result<HashSet<TargetIndex>>
where
    S: TargetProbe + ?Sized,
{
    let mut touching = HashSet::new();
    for item in &entry.items {
        for address in conflict::addresses(&item.target, item.role) {
            if &store.item_key(&address)? == key {
                touching.insert(item.target.index);
            }
        }
    }
    Ok(touching)
}

fn select_paths<S>(store: &S, view: &View, op: OpId, paths: &[TargetPath]) -> Result<Plan>
where
    S: TargetProbe + ?Sized,
{
    let entry = entry_gate(view, op)?;
    let mut wanted = Vec::with_capacity(paths.len());
    for path in paths {
        wanted.push(store.item_key(path)?);
    }
    let wanted_keys: HashSet<&ItemKey> = wanted.iter().collect();
    let mut matched = HashSet::new();
    let mut found = HashSet::new();
    for item in &entry.items {
        for address in conflict::addresses(&item.target, item.role) {
            let key = store.item_key(&address)?;
            if wanted_keys.contains(&key) {
                matched.insert(item.target.index);
                found.insert(key);
            }
        }
    }
    let missing: Vec<&TargetPath> = paths
        .iter()
        .zip(&wanted)
        .filter(|(_, key)| !found.contains(*key))
        .map(|(path, _)| path)
        .collect();
    if !missing.is_empty() {
        let mut error = conflict::nothing_error(&format!(
            "operation {op} did not change {count} of the paths; list its items with history {op} details: true",
            op = op.0,
            count = missing.len()
        ))
        .with_problem("path_not_in_operation");
        for path in missing {
            error = error.with_sample(conflict::display_path(path));
        }
        return Err(error);
    }
    let mut plan = Plan::new(UndoScope::Paths);
    plan.add_entry(entry, |item| matched.contains(&item.target.index));
    if plan.is_empty() {
        return Err(
            conflict::noop_error(op, Some(EntryState::Undone)).with_problem("already_undone")
        );
    }
    Ok(plan)
}

fn select_redo(
    view: &View,
    undo_entry: Option<OpId>,
    session: Option<&SessionId>,
) -> Result<(Plan, OpId)> {
    let entry = match undo_entry {
        Some(op) => {
            let entry = entry_gate(view, op)?;
            if !entry.is_undo() {
                return Err(conflict::nothing_error(&format!(
                    "operation {} is not an undo; redo applies undo operations only",
                    op.0
                ))
                .with_problem("not_an_undo")
                .with_detail("op", serde_json::Value::from(op.0)));
            }
            entry
        }
        None => view
            .newest_first()
            .find(|entry| is_redo_candidate(entry, session))
            .ok_or_else(|| {
                conflict::nothing_error(
                    "no undo to redo in this session; list operations with history",
                )
            })?,
    };
    let mut plan = Plan::new(UndoScope::Op);
    plan.add_entry(entry, |_| true);
    Ok((plan, entry.op))
}

/// Applies the conflict policy; returns the conflicts overridden by `force`.
fn settle<S>(store: &S, view: &View, plan: &mut Plan, mode: OnConflict) -> Result<Vec<Conflict>>
where
    S: TargetProbe + ?Sized,
{
    let accepts_partial = matches!(mode, OnConflict::Partial | OnConflict::Force);
    loop {
        if !plan.released_in.is_empty() && !accepts_partial {
            let available: Vec<TargetPath> = plan
                .items
                .iter()
                .map(|p| p.item.target.path.clone())
                .collect();
            return Err(conflict::partial_error(
                &plan.released_in,
                &available,
                &plan.released,
            ));
        }
        let conflicts = check_plan(store, view, plan)?;
        if conflicts.is_empty() {
            return Ok(conflicts);
        }
        match mode {
            OnConflict::Force => return Ok(conflicts),
            OnConflict::Abort => return Err(conflict::aborted_error(conflicts.len())),
            OnConflict::IncludeLater if plan.scope != UndoScope::To => {
                if !include_later(view, plan, &conflicts)? {
                    return Err(conflict::conflict_error(&conflicts, &plan.fixes));
                }
            }
            _ => return Err(conflict::conflict_error(&conflicts, &plan.fixes)),
        }
    }
}

/// Adds the later entries named by the conflicts to the plan; returns false
/// when no entry was added.
fn include_later(view: &View, plan: &mut Plan, conflicts: &[Conflict]) -> Result<bool> {
    let later: BTreeSet<OpId> = conflicts
        .iter()
        .filter_map(|found| match found.kind {
            ConflictKind::LaterEntry { op } if !plan.entries.contains(&op) => Some(op),
            _ => None,
        })
        .collect();
    let Some(oldest) = plan.entries.first().copied() else {
        return Ok(false);
    };
    let bound = Bound::After(oldest);
    let mut expired = Vec::new();
    for &op in later.iter().rev() {
        let Some(entry) = view.get(op) else {
            continue;
        };
        match entry.state {
            EntryState::Committed | EntryState::Partial => {
                plan.add_entry(entry, |item| net_applied(view, item, bound));
            }
            EntryState::Corrupt => return Err(conflict::corrupt_error(op)),
            _ => expired.push(op),
        }
    }
    if !expired.is_empty() {
        return Err(conflict::expired_error(&expired));
    }
    plan.sort();
    Ok(!later.is_empty())
}

/// Compares every item of the plan with the journal.
///
/// The current state of an address is compared once, with the newest item
/// of the plan that changed it; older items of the same address are restored
/// on top of the newer ones. Every item is checked for later entries outside
/// the plan and for changed preserved content; the preserved content of each
/// entry is located with one read of the entry.
fn check_plan<S>(store: &S, view: &View, plan: &Plan) -> Result<Vec<Conflict>>
where
    S: TargetProbe + ?Sized,
{
    let later = later_changes(store, view, plan)?;
    let hashes = conflict::preserved_hashes(
        store,
        plan.items
            .iter()
            .map(|planned| (planned.op, &planned.item.target)),
    )?;
    let mut seen = HashSet::new();
    let mut conflicts = Vec::new();
    for planned in &plan.items {
        for (path, expected) in conflict::expected_after(&planned.item.target, planned.item.role) {
            let key = store.item_key(&path)?;
            if let Some(&op) = later.get(&key).filter(|&&op| op > planned.op) {
                conflicts.push(Conflict {
                    op: planned.op,
                    path,
                    kind: ConflictKind::LaterEntry { op },
                    expected: expected.hash(),
                    actual: None,
                });
                seen.insert(key);
                continue;
            }
            if seen.insert(key) {
                conflicts.extend(conflict::check_address(
                    store, planned.op, &path, &expected,
                )?);
            }
        }
        conflicts.extend(conflict::check_blob(
            &hashes,
            planned.op,
            &planned.item.target,
        )?);
    }
    Ok(conflicts)
}

/// Newest entry outside the plan, newer than its oldest entry, whose change
/// of an address is still in effect, per address key.
fn later_changes<S>(store: &S, view: &View, plan: &Plan) -> Result<HashMap<ItemKey, OpId>>
where
    S: TargetProbe + ?Sized,
{
    let mut later = HashMap::new();
    let Some(oldest) = plan.items.iter().map(|planned| planned.op).min() else {
        return Ok(later);
    };
    let bound = Bound::After(oldest);
    let candidates = view.newest_first().filter(|entry| {
        entry.op > oldest
            && !plan.entries.contains(&entry.op)
            && (entry.is_available() || entry.is_expired() || entry.state == EntryState::Corrupt)
    });
    for entry in candidates {
        let in_effect = entry
            .items
            .iter()
            .filter(|item| item.is_in_effect() && net_applied(view, item, bound));
        for item in in_effect {
            for address in conflict::addresses(&item.target, item.role) {
                later.entry(store.item_key(&address)?).or_insert(entry.op);
            }
        }
    }
    Ok(later)
}

/// Builds the restore items with the state each address must have
/// immediately before its restore.
fn restore_items<S>(store: &S, plan: &Plan, forced: &[Conflict]) -> Result<Vec<RestoreItem>>
where
    S: TargetProbe + ?Sized,
{
    let mut overridden: HashMap<(OpId, &TargetPath), &Conflict> = HashMap::new();
    for found in forced {
        overridden.entry((found.op, &found.path)).or_insert(found);
    }
    let mut restored: HashMap<ItemKey, Expect> = HashMap::new();
    let mut items = Vec::with_capacity(plan.items.len());
    for planned in &plan.items {
        let target = &planned.item.target;
        let role = planned.item.role;
        let primary = conflict::primary(target, role);
        let key = store.item_key(&primary)?;
        let expect = match restored.get(&key) {
            Some(expect) => expect.clone(),
            None => {
                let forced_here = overridden.get(&(planned.op, &primary)).copied();
                current_expect(planned, &primary, forced_here)
            }
        };
        for (address, state) in state_after_restore(target, role) {
            restored.insert(store.item_key(&address)?, state);
        }
        items.push(RestoreItem {
            source: ItemSource {
                op: planned.op,
                index: target.index,
            },
            target: target.clone(),
            role,
            expect,
        });
    }
    Ok(items)
}

/// Expected state of the newest item of an address: the state after the
/// change, or the state found by the check when the difference `overridden`
/// of the address is forced.
fn current_expect(
    planned: &Planned,
    primary: &TargetPath,
    overridden: Option<&Conflict>,
) -> Expect {
    match overridden.map(|found| (&found.kind, found.actual)) {
        Some((ConflictKind::Deleted, _)) => Expect::Absent,
        Some((ConflictKind::BlobModified, _)) | None => {
            conflict::expected_after(&planned.item.target, planned.item.role)
                .into_iter()
                .find(|(path, _)| path == primary)
                .map_or(Expect::Unchecked, |(_, expected)| expected.to_expect())
        }
        Some((_, Some(hash))) => Expect::Hash(hash),
        Some((_, None)) => Expect::Unchecked,
    }
}

/// State of every address of a target after its restore.
fn state_after_restore(target: &TargetOutcome, role: TargetRole) -> Vec<(TargetPath, Expect)> {
    let before = Expected::before(&target.before).to_expect();
    match &target.action {
        TargetAction::Rename { to, replace } if role == TargetRole::Whole => {
            let destination = if *replace {
                Expect::Unchecked
            } else {
                Expect::Absent
            };
            vec![
                (target.path.clone(), before),
                (TargetPath::Fs(to.clone()), destination),
            ]
        }
        _ => vec![(target.path.clone(), before)],
    }
}

/// Stops a restoring transaction at an injected fault.
enum Halt {
    /// An error as a system call would return it; the transaction is rolled
    /// back.
    Fault(Error),
    /// A simulated crash; the transaction is abandoned without cleanup and
    /// left to recovery.
    Crash(Error),
}

fn hit(
    failpoints: &dyn Failpoints,
    point: FailPoint,
    n: Option<u32>,
) -> std::result::Result<(), Halt> {
    match failpoints.hit(point, n) {
        FailAction::Continue => Ok(()),
        FailAction::Error(kind) => {
            Err(Halt::Fault(FsError::new(kind, point.as_str(), None).into()))
        }
        FailAction::Crash => Err(Halt::Crash(Error::new(
            ErrorCode::Io,
            format!("execution stopped at {}", point.as_str()),
        ))),
    }
}

/// Runs the restoring transaction of a plan and records its effect on the
/// source entries.
///
/// The source updates are made by [`RestoreTx::commit`] after the new entry
/// is durably committed and before it leaves the set of pending entries.
fn execute<S>(store: &S, plan: &RestorePlan) -> Result<Executed>
where
    S: UndoBackend + ?Sized,
{
    let mut tx = store.begin_restore(plan);
    if let Err(error) = tx.preserve() {
        return Err(abandon(tx, error));
    }
    if let Err(halt) = hit(store.failpoints(), FailPoint::UndoChecked, None) {
        return Err(stop(tx, halt));
    }
    for n in 0..plan.items.len() {
        if let Err(error) = tx.restore(n) {
            return Err(abandon(tx, error));
        }
        let index = u32::try_from(n).unwrap_or(u32::MAX);
        if let Err(halt) = hit(store.failpoints(), FailPoint::UndoRestored, Some(index)) {
            return Err(stop(tx, halt));
        }
    }
    tx.commit()
}

fn stop(tx: Box<dyn RestoreTx + '_>, halt: Halt) -> Error {
    match halt {
        Halt::Fault(error) => abandon(tx, error),
        Halt::Crash(error) => error,
    }
}

/// Rolls the transaction back and returns the error that caused it, or
/// `E_ROLLBACK_FAILED` when targets could not be restored.
fn abandon(tx: Box<dyn RestoreTx + '_>, error: Error) -> Error {
    match tx.rollback() {
        Ok(report) if report.failed.is_empty() => error,
        Ok(report) => conflict::rollback_failed_error(&report.failed).with_cause(error),
        Err(failure) => failure.with_cause(error),
    }
}

/// Completes the source updates of a committed undo or redo entry from its
/// manifest; used by recovery for an entry that is committed and still
/// pending.
///
/// The cause recorded on sources that become undone is
/// [`UndoneCause::Undo`]; the manifest does not record the reason of a
/// rollback after a failed check.
///
/// Fails with the error of the state or item state file that cannot be
/// written; a repeated call completes the update.
pub(crate) fn apply_source_updates<S>(store: &S, manifest: &Manifest) -> Result<()>
where
    S: UndoBackend + ?Sized,
{
    let sources: Vec<ItemSource> = manifest
        .items
        .iter()
        .map(|item| ItemSource {
            op: item.op,
            index: item.index,
        })
        .collect();
    let scope = manifest.scope.unwrap_or(UndoScope::Op);
    update_sources(store, manifest.op, scope, &sources, UndoneCause::Undo)
}

/// Records on the source entries that the entry `by` restored their items.
///
/// An entry with no restorable item left becomes undone; otherwise it becomes
/// partial and the restored items are marked in its item state file, which
/// is also done for every `file` and `paths` scope. Idempotent: entries
/// already undone are left alone and markers are set, not toggled.
fn update_sources<S>(
    store: &S,
    by: OpId,
    scope: UndoScope,
    sources: &[ItemSource],
    cause: UndoneCause,
) -> Result<()>
where
    S: UndoBackend + ?Sized,
{
    let mut by_source: BTreeMap<OpId, Vec<TargetIndex>> = BTreeMap::new();
    for source in sources {
        by_source.entry(source.op).or_default().push(source.index);
    }
    let per_item = matches!(scope, UndoScope::File | UndoScope::Paths);
    for (op, indexes) in by_source {
        let Some(entry) = store.load(op)? else {
            continue;
        };
        if entry.state == EntryState::Undone {
            continue;
        }
        let undone: HashSet<TargetIndex> = indexes.iter().copied().collect();
        let remaining = entry
            .items
            .iter()
            .any(|item| item.is_restorable() && !undone.contains(&item.target.index));
        if per_item || remaining {
            store.mark_items_undone(op, &indexes, by)?;
        }
        if remaining {
            store.set_state(op, EntryState::Partial, None)?;
        } else {
            store.set_state(op, EntryState::Undone, Some(cause))?;
        }
    }
    Ok(())
}

fn outcome(executed: Executed, plan: Plan, forced: Vec<Conflict>) -> UndoOutcome {
    let sources = plan.sources();
    let mut unavailable = plan.released;
    unavailable.extend(plan.unpreserved);
    UndoOutcome {
        entry: executed.entry,
        scope: plan.scope,
        sources,
        restored: executed.restored,
        unavailable,
        skipped: plan.skipped,
        forced,
        irreversible: executed.irreversible,
    }
}

/// Ready programs that resolve a conflict of the selection.
fn fixes(selector: &UndoSelector, plan: &Plan) -> Vec<String> {
    const FORCE: &str = "on_conflict: force";
    const LATER: &str = "on_conflict: include_later";
    match selector {
        UndoSelector::Last | UndoSelector::Op(_) => plan
            .entries
            .first()
            .map(|op| {
                vec![
                    format!("undo {} {LATER}", op.0),
                    format!("undo {} {FORCE}", op.0),
                ]
            })
            .unwrap_or_default(),
        UndoSelector::To {
            after,
            all_sessions,
        } => {
            let sessions = if *all_sessions {
                " all_sessions: true"
            } else {
                ""
            };
            vec![format!("undo to: {}{sessions} {FORCE}", after.0)]
        }
        UndoSelector::File { path, since } => {
            let file = quoted(&path.to_language_string());
            match since {
                None => vec![format!("undo file: {file} {FORCE}")],
                Some(Since::Op(op)) => vec![format!("undo file: {file} since: {} {FORCE}", op.0)],
                Some(Since::Time(_)) => Vec::new(),
            }
        }
        UndoSelector::Paths { op, paths } => {
            let Some(list) = fs_path_list(paths) else {
                return Vec::new();
            };
            vec![
                format!("undo {} paths: [{list}] {LATER}", op.0),
                format!("undo {} paths: [{list}] {FORCE}", op.0),
            ]
        }
    }
}

fn fs_path_list(paths: &[TargetPath]) -> Option<String> {
    let mut quoted_paths = Vec::with_capacity(paths.len());
    for path in paths {
        match path {
            TargetPath::Fs(path) => quoted_paths.push(quoted(&path.to_language_string())),
            TargetPath::Provider(_) => return None,
        }
    }
    Some(quoted_paths.join(", "))
}

/// String literal of the language.
fn quoted(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Undo services of the journal store: restoring transactions are linked
/// transactions of the store, and source updates write the state and item
/// state files of every part of the source entry.
impl UndoBackend for JournalCtx {
    fn begin_restore<'a>(&'a self, plan: &'a RestorePlan) -> Box<dyn RestoreTx + 'a> {
        Box::new(CtxRestore {
            ctx: self,
            plan,
            tx: None,
            steps: Vec::new(),
            spans: Vec::new(),
            preserved: Vec::new(),
            failed: false,
        })
    }

    fn mark_items_undone(&self, op: OpId, items: &[TargetIndex], by: OpId) -> Result<()> {
        let fs = self.fs.as_ref();
        let scope = store::scope(op, ScopePurpose::Maintenance);
        let (parts, _) = stored_parts(self, op)?;
        for part in parts {
            let mut states = part.items;
            let mut changed = false;
            let targets: HashSet<TargetIndex> = part
                .manifest
                .targets
                .iter()
                .map(|record| record.index)
                .collect();
            let mut positions: HashMap<TargetIndex, usize> =
                HashMap::with_capacity(states.items.len());
            for (position, state) in states.items.iter().enumerate() {
                positions.entry(state.index).or_insert(position);
            }
            for &index in items.iter().filter(|index| targets.contains(index)) {
                match positions.get(&index).copied() {
                    Some(position) => {
                        if let Some(state) = states.items.get_mut(position)
                            && state.undone_by != Some(by)
                        {
                            state.undone_by = Some(by);
                            changed = true;
                        }
                    }
                    None => {
                        positions.insert(index, states.items.len());
                        states.items.push(ItemState {
                            index,
                            released: false,
                            undone_by: Some(by),
                        });
                        changed = true;
                    }
                }
            }
            if changed {
                states.items.sort_by_key(|state| state.index);
                manifest::write_items(
                    fs,
                    &scope,
                    &part.entry.tmp()?,
                    &part.entry.items()?,
                    &states,
                )?;
            }
        }
        Ok(())
    }

    fn set_state(&self, op: OpId, state: EntryState, cause: Option<UndoneCause>) -> Result<()> {
        let fs = self.fs.as_ref();
        let scope = store::scope(op, ScopePurpose::Maintenance);
        let record = match (state, cause) {
            (EntryState::Undone, Some(cause)) => StateRecord::undone(cause),
            _ => StateRecord::new(state),
        };
        let (parts, _) = stored_parts(self, op)?;
        for part in parts.iter().filter(|part| part.state != record) {
            manifest::write_state(fs, &scope, &part.entry.tmp()?, &part.entry.state()?, record)?;
        }
        Ok(())
    }

    fn failpoints(&self) -> &dyn Failpoints {
        self.failpoints.as_ref()
    }

    fn count_undo_as_op(&self) -> bool {
        COUNT_UNDO_AS_OP
    }
}

/// Restoring transaction on the journal store: a linked transaction whose
/// targets are the current addresses of the items, changed by the mutations
/// that bring back the recorded previous state. Its current state is
/// preserved like that of any operation, so the undo can itself be undone.
struct CtxRestore<'a> {
    ctx: &'a JournalCtx,
    plan: &'a RestorePlan,
    tx: Option<Box<dyn MutationTx + 'a>>,
    steps: Vec<Step>,
    /// Range of `steps` that restores each plan item.
    spans: Vec<std::ops::Range<usize>>,
    preserved: Vec<Preserved>,
    /// A call failed and the journal already rolled the transaction back.
    failed: bool,
}

/// Declaration and mutation that restore one item.
struct Step {
    spec: TargetSpec,
    mutation: Mutation,
    /// Preserved directory whose entries are copied into the directory the
    /// mutation recreates.
    tree: Option<AbsPath>,
}

fn undeclared() -> Error {
    Error::new(
        ErrorCode::Internal,
        "the undo transaction was used out of order",
    )
}

impl<'a> RestoreTx for CtxRestore<'a> {
    fn preserve(&mut self) -> Result<()> {
        let mut sources: HashMap<OpId, Vec<StoredPart>> = HashMap::new();
        for item in &self.plan.items {
            if let std::collections::hash_map::Entry::Vacant(slot) = sources.entry(item.source.op) {
                slot.insert(stored_parts(self.ctx, item.source.op)?.0);
            }
        }
        let records: HashMap<OpId, RecordIndex<'_>> = sources
            .iter()
            .map(|(op, parts)| (*op, record_index(parts)))
            .collect();
        let mut steps = Vec::with_capacity(self.plan.items.len());
        let mut spans = Vec::with_capacity(self.plan.items.len());
        for item in &self.plan.items {
            let found = records
                .get(&item.source.op)
                .and_then(|records| records.get(&item.source.index))
                .copied();
            let first = steps.len();
            steps.extend(restore_steps(self.ctx, found, item)?);
            spans.push(first..steps.len());
        }
        let program = match &self.plan.program {
            Some(program) => program.clone(),
            None => source_program(&sources, &self.plan.undo_of)?,
        };
        let request = BeginRequest {
            kind: self.plan.kind.clone(),
            session: self.plan.session.clone(),
            program,
            corrections: Vec::new(),
            bulk: self.plan.items.len() > 1,
            counts_as_step: self.plan.counts_as_step,
            preserve_budget: None,
        };
        let mut tx: Box<dyn MutationTx + 'a> = Box::new(tx::begin_linked(
            self.ctx,
            request,
            link_of(self.plan, &spans),
        ));
        let specs: Vec<TargetSpec> = steps.iter().map(|step| step.spec.clone()).collect();
        match tx.preserve(&specs) {
            Ok(preserved) => {
                self.preserved = preserved;
                self.steps = steps;
                self.spans = spans;
                self.tx = Some(tx);
                Ok(())
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    fn restore(&mut self, n: usize) -> Result<TargetOutcome> {
        let span = self.spans.get(n).cloned().ok_or_else(undeclared)?;
        let mut outcome = None;
        for at in span {
            outcome = Some(self.apply_step(at)?);
        }
        outcome.ok_or_else(undeclared)
    }

    fn commit(self: Box<Self>) -> Result<Executed> {
        let tx = self.tx.ok_or_else(undeclared)?;
        let handle = tx.commit()?;
        let entry = handle.op.ok_or_else(undeclared)?;
        Ok(Executed {
            entry,
            restored: handle.targets,
            irreversible: handle.irreversible,
        })
    }

    fn rollback(self: Box<Self>) -> Result<RollbackReport> {
        let CtxRestore { tx, failed, .. } = *self;
        match tx {
            Some(tx) if !failed => tx.rollback(RollbackReason::TargetFailed),
            tx => Ok(RollbackReport {
                op: tx.and_then(|tx| tx.op()),
                restored: Vec::new(),
                failed: Vec::new(),
            }),
        }
    }
}

impl CtxRestore<'_> {
    /// Applies the mutation of step `at` and copies the entries of a
    /// recreated directory.
    fn apply_step(&mut self, at: usize) -> Result<TargetOutcome> {
        let (Some(tx), Some(step), Some(preserved)) =
            (self.tx.as_mut(), self.steps.get(at), self.preserved.get(at))
        else {
            return Err(undeclared());
        };
        let applied = match tx.apply(preserved.index, step.mutation.clone()) {
            Ok(done) => done,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        if let (Some(tree), TargetPath::Fs(dest)) = (&step.tree, &step.spec.path) {
            let op = tx.op().ok_or_else(undeclared)?;
            copy_children(self.ctx, op, tree, dest)?;
        }
        Ok(TargetOutcome {
            index: preserved.index,
            path: step.spec.path.clone(),
            action: step.spec.action.clone(),
            mode: preserved.mode,
            bytes_held: preserved.bytes_held,
            before: preserved.before.clone(),
            after: Some(applied.after),
            irreversible: preserved.irreversible,
            notable: preserved.notable.clone(),
        })
    }
}

/// Relation of the new entry to its sources; the source updates run inside
/// the commit, before the pending marker of the entry is removed. The entry
/// names the item it reverses once per target, so that target `n` of the
/// entry reverses `items[n]`.
fn link_of(plan: &RestorePlan, spans: &[std::ops::Range<usize>]) -> Link {
    let sources: Vec<ItemSource> = plan.items.iter().map(|item| item.source).collect();
    let (scope, cause) = (plan.scope, plan.cause);
    Link {
        undo_of: plan.undo_of.clone(),
        redo_of: plan.redo_of,
        scope: Some(plan.scope),
        items: plan
            .items
            .iter()
            .zip(spans)
            .flat_map(|(item, span)| {
                span.clone().map(move |_| ItemRef {
                    op: item.source.op,
                    index: item.source.index,
                    path: item.target.path.clone(),
                })
            })
            .collect(),
        after_commit: Some(Box::new(move |ctx: &JournalCtx, op: OpId| {
            update_sources(ctx, op, scope, &sources, cause)
        })),
    }
}

/// Bound program recorded by the newest source entry, for a rollback that
/// has no program of its own.
fn source_program(
    sources: &HashMap<OpId, Vec<StoredPart>>,
    undo_of: &[OpId],
) -> Result<BoundDocument> {
    let op = undo_of.first().copied().ok_or_else(undeclared)?;
    let part = sources
        .get(&op)
        .and_then(|parts| parts.first())
        .ok_or_else(|| conflict::noop_error(op, None))?;
    serde_json::from_value(part.manifest.ast.clone())
        .map_err(|_| conflict::corrupt_error(op).with_problem("damaged_program"))
}

fn unsupported(target: &TargetOutcome, what: &str) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!(
            "undo cannot restore {} ({what}); nothing changed, restore it from the data listed by history details",
            conflict::display_path(&target.path)
        ),
    )
}

/// Declarations and mutations that bring the address of an item back to the
/// state recorded before its change, from the record `found` of the item in
/// its source entry. One step restores an item, except a rename over an
/// existing entry, which takes two.
fn restore_steps(
    ctx: &JournalCtx,
    found: Option<(&EntryLayout, &TargetRecord)>,
    item: &RestoreItem,
) -> Result<Vec<Step>> {
    let target = &item.target;
    let op = item.source.op;
    if item.role != TargetRole::Whole {
        return Err(unsupported(target, "rename across volumes"));
    }
    let (entry, record) =
        found.ok_or_else(|| conflict::corrupt_error(op).with_problem("missing_item"))?;
    if let TargetAction::Rename { to, .. } = &target.action {
        let TargetPath::Fs(from) = &target.path else {
            return Err(unsupported(target, "rename of a provider key"));
        };
        let kept = record
            .replaced
            .as_ref()
            .filter(|replaced| replaced.exists && replaced.store != PreserveMode::None);
        return match kept {
            Some(replaced) => rename_over_steps(item, entry, replaced, (from, to)).map(Vec::from),
            None => Ok(vec![rename_step(to, from, false, item.expect.clone())]),
        };
    }
    restore_step(ctx, (entry, record), item).map(|step| vec![step])
}

/// Step that renames `from` to `to`; the entry at `to` may exist when
/// `replace` is set and is then preserved by the transaction.
fn rename_step(from: &AbsPath, to: &AbsPath, replace: bool, expect: Expect) -> Step {
    Step {
        spec: TargetSpec {
            path: TargetPath::Fs(from.clone()),
            action: TargetAction::Rename {
                to: to.clone(),
                replace,
            },
            expect,
            size_hint: None,
        },
        mutation: Mutation::Rename,
        tree: None,
    }
}

/// Steps that undo a rename over a kept destination: the renamed entry
/// returns to its source name by a rename that keeps its identity, and the
/// block that holds the entry the rename displaced is renamed to the
/// destination name. The second rename declares the destination as
/// replaced; the transaction finds it vacated by the first one and keeps
/// nothing for it. Both are targets of the restoring transaction, so a
/// failure or a crash is rolled back by the journal like any other change.
fn rename_over_steps(
    item: &RestoreItem,
    entry: &EntryLayout,
    replaced: &BeforeRecord,
    (from, to): (&AbsPath, &AbsPath),
) -> Result<[Step; 2]> {
    let name = replaced.blob.as_deref().ok_or_else(|| {
        conflict::corrupt_error(item.source.op).with_problem("missing_preserved_content")
    })?;
    let kept = entry.resolve(name)?;
    Ok([
        rename_step(to, from, false, item.expect.clone()),
        rename_step(&kept, to, true, Expect::Unchecked),
    ])
}

/// Step that brings back the address of one item other than a rename.
fn restore_step(
    ctx: &JournalCtx,
    (entry, record): (&EntryLayout, &TargetRecord),
    item: &RestoreItem,
) -> Result<Step> {
    let target = &item.target;
    let op = item.source.op;
    let step = |path: TargetPath, action: TargetAction, mutation: Mutation| Step {
        spec: TargetSpec {
            path,
            action,
            expect: item.expect.clone(),
            size_hint: None,
        },
        mutation,
        tree: None,
    };
    let removal = if matches!(target.path, TargetPath::Provider(_)) {
        Mutation::ProviderRemove
    } else {
        Mutation::Remove
    };
    if !record.before.exists || target.mode == PreserveMode::Created {
        return Ok(step(target.path.clone(), TargetAction::Delete, removal));
    }
    match target.mode {
        PreserveMode::Append => Ok(step(
            target.path.clone(),
            TargetAction::Truncate,
            Mutation::SetLength(record.before.size.unwrap_or(0)),
        )),
        PreserveMode::Meta => {
            let change = meta_change(ctx, target, &record.before)?;
            Ok(step(
                target.path.clone(),
                TargetAction::Attributes,
                Mutation::SetMeta(change),
            ))
        }
        PreserveMode::Value | PreserveMode::Ref => {
            let bytes = manifest::read_bytes(ctx.fs.as_ref(), &preserved_path(entry, record, op)?)?;
            let snapshot: ProviderSnapshot = serde_json::from_slice(&bytes)
                .map_err(|_| conflict::corrupt_error(op).with_problem("damaged_preserved_value"))?;
            Ok(match snapshot.value {
                Some(value) => step(
                    target.path.clone(),
                    TargetAction::Modify,
                    Mutation::ProviderSet(value),
                ),
                None => step(target.path.clone(), TargetAction::Delete, removal),
            })
        }
        PreserveMode::Moved
        | PreserveMode::Exchange
        | PreserveMode::ReplaceBackup
        | PreserveMode::Hardlink
        | PreserveMode::Clone
        | PreserveMode::Copy => {
            let shared = target.mode == PreserveMode::Moved
                && record.before.kind == Some(FileKind::File)
                && record.before.links.unwrap_or(1) > 1;
            let data = preserved_path(entry, record, op)?;
            if let (true, Expect::Absent, TargetPath::Fs(path)) =
                (shared, &item.expect, &target.path)
            {
                return Ok(rename_step(&data, path, false, Expect::Unchecked));
            }
            content_step(
                ctx,
                Content {
                    path: target.path.clone(),
                    expect: item.expect.clone(),
                    data,
                    shared,
                },
                &record.before,
            )
        }
        _ => Err(unsupported(target, "nothing was preserved")),
    }
}

/// Where preserved or live content is put back and how it is checked.
struct Content {
    path: TargetPath,
    expect: Expect,
    /// File or directory the content is taken from.
    data: AbsPath,
    /// The data is a name of a file that has other hard links; the restored
    /// name becomes a hard link of it instead of an independent copy.
    shared: bool,
}

/// Step that brings back content of the kind `before`: a file by copying (or
/// cloning) it into place, or by linking it when it is shared, a link by
/// recreating it, a directory by recreating it and copying its entries.
fn content_step(ctx: &JournalCtx, content: Content, before: &BeforeRecord) -> Result<Step> {
    let Content {
        path,
        expect,
        data,
        shared,
    } = content;
    let step = |action: TargetAction, mutation: Mutation, tree: Option<AbsPath>| Step {
        spec: TargetSpec {
            path: path.clone(),
            action,
            expect: expect.clone(),
            size_hint: None,
        },
        mutation,
        tree,
    };
    Ok(match before.kind {
        Some(FileKind::Directory) => step(
            TargetAction::CreateTree,
            Mutation::CreateDir {
                mode: before.mode.unwrap_or(DEFAULT_DIR_MODE),
            },
            Some(data),
        ),
        Some(kind @ (FileKind::Symlink | FileKind::Junction)) => step(
            TargetAction::Link,
            Mutation::CreateLink {
                target: ctx.fs.read_link(&data)?,
                kind: if kind == FileKind::Junction {
                    LinkKind::Junction
                } else {
                    LinkKind::SymlinkFile
                },
            },
            None,
        ),
        _ if shared => step(
            TargetAction::Link,
            Mutation::CreateHardLink { existing: data },
            None,
        ),
        _ => step(
            TargetAction::Modify,
            Mutation::CopyFrom {
                source: data,
                clone: CloneMode::Auto,
            },
            None,
        ),
    })
}

/// File of the preserved data of a record.
fn preserved_path(entry: &EntryLayout, record: &TargetRecord, op: OpId) -> Result<AbsPath> {
    let name = record
        .before
        .blob
        .as_deref()
        .ok_or_else(|| conflict::corrupt_error(op).with_problem("missing_preserved_content"))?;
    Ok(entry.resolve(name)?)
}

/// Metadata change back to the recorded permission bits and modification
/// time; the account and group of the file are set only when they differ.
fn meta_change(
    ctx: &JournalCtx,
    target: &TargetOutcome,
    before: &BeforeRecord,
) -> Result<MetaChange> {
    let TargetPath::Fs(path) = &target.path else {
        return Err(unsupported(target, "metadata of a provider key"));
    };
    let current = ctx.fs.metadata(path, Follow::NoFollow)?;
    Ok(MetaChange {
        mode: before.mode,
        uid: before.uid.filter(|uid| current.uid != Some(*uid)),
        gid: before.gid.filter(|gid| current.gid != Some(*gid)),
        modified_ns: before.modified_ns,
        ..MetaChange::default()
    })
}

/// Copies the entries of a preserved directory into the recreated directory.
fn copy_children(ctx: &JournalCtx, op: OpId, from: &AbsPath, to: &AbsPath) -> Result<()> {
    let fs = ctx.fs.as_ref();
    let scope = store::scope(op, ScopePurpose::Operation);
    let mut names = Vec::new();
    for child in fs.read_dir(from)? {
        names.push(child?.name);
    }
    for name in names {
        let name = name
            .to_str()
            .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath, "copy", Some(from.as_path())))?;
        blob::copy_entry(fs, &scope, &from.join_name(name)?, &to.join_name(name)?)?;
    }
    blob::sync_tree(fs, &scope, to)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod testkit {
    //! In-memory journal of test builds: a world of files and provider
    //! values, entries with their preserved content, and a restoring
    //! transaction with the semantics of the journal protocol.

    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, PoisonError};
    use std::time::Duration;

    use nurt_contracts::bound::{BoundDocument, BoundProgram};
    use nurt_contracts::env::WallTime;
    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::fs::{
        AbsPath, ContentHash, DirEntry, ExtendedMeta, FileId, FileKind, FileMeta, Follow, Fs,
        FsError, FsErrorKind, FsResult, Holder, PathKey, ProviderKey, ProviderScheme, ReadHandle,
        SpaceInfo, VolumeId, VolumeInfo,
    };
    use nurt_contracts::journal::{
        AfterState, BeforeState, EntryState, Expect, OpId, OpKind, PreserveMode, RollbackReport,
        SessionId, TargetAction, TargetIndex, TargetOutcome, TargetPath, UndoneCause,
    };
    use nurt_contracts::report::{Irreversible, IrreversibleEntry};
    use nurt_fs::failpoint::{FailPoint, Failpoints, NoFailpoints};
    use sha2::{Digest, Sha256};

    use super::{Executed, RestorePlan, RestoreTx, UndoBackend};
    use crate::conflict::{self, ItemKey, Observed, TargetProbe};
    use crate::history::{EntryItem, EntrySource, ItemSource, JournalEntry};
    use crate::manifest::TargetRole;
    use crate::pin::{PinBackend, PinSet};

    const START_NS: i128 = 1_700_000_000_000_000_000;
    const SECOND_NS: i128 = 1_000_000_000;
    const FILE_MODE: u32 = 0o644;
    const DIR_MODE: u32 = 0o755;

    pub(crate) fn abs(path: &str) -> AbsPath {
        AbsPath::new(path).unwrap()
    }

    pub(crate) fn fs_path(path: &str) -> TargetPath {
        TargetPath::Fs(abs(path))
    }

    pub(crate) fn value_path(name: &str) -> TargetPath {
        TargetPath::Provider(ProviderKey {
            scheme: ProviderScheme("env".to_owned()),
            segments: vec![name.to_owned()],
            value: None,
        })
    }

    pub(crate) fn hash_of(bytes: &[u8]) -> ContentHash {
        ContentHash(Sha256::digest(bytes).into())
    }

    pub(crate) fn volume(dev: u64) -> VolumeId {
        VolumeId {
            dev,
            uuid: None,
            subvolume: 0,
        }
    }

    pub(crate) fn program() -> BoundDocument {
        BoundDocument::new(
            "test",
            "test",
            BoundProgram {
                statements: Vec::new(),
            },
        )
    }

    pub(crate) fn session(name: &str) -> SessionId {
        SessionId(name.to_owned())
    }

    /// Entry of the world.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Node {
        File { content: Vec<u8>, mode: u32 },
        Dir { mode: u32 },
    }

    impl Node {
        pub(crate) fn file(content: &[u8]) -> Self {
            Self::File {
                content: content.to_vec(),
                mode: FILE_MODE,
            }
        }

        fn kind(&self) -> FileKind {
            match self {
                Self::File { .. } => FileKind::File,
                Self::Dir { .. } => FileKind::Directory,
            }
        }

        fn hash(&self) -> Option<ContentHash> {
            match self {
                Self::File { content, .. } => Some(hash_of(content)),
                Self::Dir { .. } => None,
            }
        }

        fn size(&self) -> Option<u64> {
            match self {
                Self::File { content, .. } => Some(content.len() as u64),
                Self::Dir { .. } => None,
            }
        }

        fn mode(&self) -> u32 {
            match self {
                Self::File { mode, .. } | Self::Dir { mode } => *mode,
            }
        }

        fn set_mode(&mut self, new: u32) {
            match self {
                Self::File { mode, .. } | Self::Dir { mode } => *mode = new,
            }
        }
    }

    fn before_of(path: &TargetPath, node: Option<&Node>) -> BeforeState {
        let Some(node) = node else {
            return BeforeState::default();
        };
        let fs = matches!(path, TargetPath::Fs(_));
        BeforeState {
            exists: true,
            kind: fs.then(|| node.kind()),
            size: if fs { node.size() } else { None },
            hash: node.hash(),
            mode: fs.then(|| node.mode()),
        }
    }

    fn after_of(path: &TargetPath, node: Option<&Node>) -> AfterState {
        let before = before_of(path, node);
        AfterState {
            exists: before.exists,
            kind: before.kind,
            size: before.size,
            hash: before.hash,
            mode: before.mode,
            meta: None,
        }
    }

    /// Complete state of the test journal and its world.
    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct State {
        world: BTreeMap<TargetPath, Node>,
        entries: BTreeMap<OpId, JournalEntry>,
        blobs: BTreeMap<(OpId, u32), Node>,
        seq: u64,
        now: i128,
        pins: BTreeMap<VolumeId, Vec<u8>>,
        step_limit: Option<u64>,
        last_manifest: Option<ContentHash>,
        last_plan_counts_as_step: Option<bool>,
        fail_restore_at: Option<usize>,
        fail_rollback: bool,
    }

    /// Journal of an account kept in memory.
    pub(crate) struct TestJournal {
        state: Mutex<State>,
        failpoints: Box<dyn Failpoints>,
    }

    impl TestJournal {
        pub(crate) fn new() -> Self {
            Self::with_failpoints(Box::new(NoFailpoints))
        }

        pub(crate) fn with_failpoints(failpoints: Box<dyn Failpoints>) -> Self {
            Self {
                state: Mutex::new(State {
                    world: BTreeMap::new(),
                    entries: BTreeMap::new(),
                    blobs: BTreeMap::new(),
                    seq: 0,
                    now: START_NS,
                    pins: BTreeMap::new(),
                    step_limit: None,
                    last_manifest: None,
                    last_plan_counts_as_step: None,
                    fail_restore_at: None,
                    fail_rollback: false,
                }),
                failpoints,
            }
        }

        fn lock(&self) -> MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub(crate) fn snapshot(&self) -> State {
            self.lock().clone()
        }

        pub(crate) fn world(&self) -> BTreeMap<TargetPath, Node> {
            self.lock().world.clone()
        }

        pub(crate) fn put(&self, path: &str, content: &[u8]) {
            self.lock().world.insert(fs_path(path), Node::file(content));
        }

        pub(crate) fn put_node(&self, path: TargetPath, node: Node) {
            self.lock().world.insert(path, node);
        }

        pub(crate) fn put_dir(&self, path: &str) {
            self.lock()
                .world
                .insert(fs_path(path), Node::Dir { mode: DIR_MODE });
        }

        pub(crate) fn remove(&self, path: &str) {
            self.lock().world.remove(&fs_path(path));
        }

        pub(crate) fn get(&self, path: &str) -> Option<Node> {
            self.lock().world.get(&fs_path(path)).cloned()
        }

        pub(crate) fn entry(&self, op: OpId) -> JournalEntry {
            self.lock().entries[&op].clone()
        }

        pub(crate) fn seq(&self) -> u64 {
            self.lock().seq
        }

        pub(crate) fn now_time(&self) -> WallTime {
            WallTime::from_unix_nanos(self.lock().now)
        }

        pub(crate) fn advance(&self, duration: Duration) {
            self.lock().now += i128::try_from(duration.as_nanos()).unwrap();
        }

        pub(crate) fn set_state(&self, op: OpId, state: EntryState) {
            self.lock().entries.get_mut(&op).unwrap().state = state;
        }

        /// Replaces the entry by its tombstone.
        pub(crate) fn expire(&self, op: OpId) {
            let mut state = self.lock();
            state.blobs.retain(|(owner, _), _| *owner != op);
            let entry = state.entries.get_mut(&op).unwrap();
            entry.items.clear();
            entry.state = EntryState::Expired;
        }

        pub(crate) fn release_item(&self, op: OpId, index: u32) {
            let mut state = self.lock();
            state.blobs.remove(&(op, index));
            let entry = state.entries.get_mut(&op).unwrap();
            entry.items[usize::try_from(index).unwrap()].released = true;
            entry.state = EntryState::Partial;
        }

        pub(crate) fn mark_undone(&self, op: OpId, index: u32, by: OpId) {
            let mut state = self.lock();
            let entry = state.entries.get_mut(&op).unwrap();
            entry.items[usize::try_from(index).unwrap()].undone_by = Some(by);
            entry.state = EntryState::Partial;
        }

        pub(crate) fn tamper_blob(&self, op: OpId, index: u32, content: &[u8]) {
            self.lock().blobs.insert((op, index), Node::file(content));
        }

        pub(crate) fn fail_restore_at(&self, n: usize) {
            self.lock().fail_restore_at = Some(n);
        }

        pub(crate) fn fail_rollback(&self) {
            self.lock().fail_rollback = true;
        }

        pub(crate) fn last_plan_counts_as_step(&self) -> Option<bool> {
            self.lock().last_plan_counts_as_step
        }

        pub(crate) fn pins_of(&self, volume: &VolumeId) -> PinSet {
            self.read_pins(volume).unwrap()
        }

        pub(crate) fn set_step_limit(&self, limit: u64) {
            self.lock().step_limit = Some(limit);
        }

        /// Expires available entries over the step limit unless pinned and
        /// returns them.
        pub(crate) fn sweep(&self) -> Vec<OpId> {
            let pins = self.pins().unwrap();
            let (limit, seq, now, ops) = {
                let state = self.lock();
                let ops: Vec<OpId> = state
                    .entries
                    .values()
                    .filter(|entry| entry.is_available())
                    .map(|entry| entry.op)
                    .collect();
                (
                    state.step_limit,
                    state.seq,
                    WallTime::from_unix_nanos(state.now),
                    ops,
                )
            };
            let Some(limit) = limit else {
                return Vec::new();
            };
            let mut expired = Vec::new();
            for op in ops {
                if seq - op.0 >= limit && !pins.is_pinned(op, now) {
                    self.expire(op);
                    expired.push(op);
                }
            }
            expired
        }

        pub(crate) fn with_fs<R>(&self, run: impl FnOnce(&dyn Fs) -> R) -> R {
            let state = self.lock();
            run(&WorldFs {
                world: &state.world,
            })
        }

        pub(crate) fn op(&self, session: Option<&str>) -> OpBuilder<'_> {
            OpBuilder {
                journal: self,
                session: session.map(self::session),
                changes: Vec::new(),
                volumes: vec![volume(1)],
                external: false,
            }
        }

        fn record(&self, builder: OpBuilder<'_>) -> OpId {
            let mut state = self.lock();
            state.seq += 1;
            let op = OpId(state.seq);
            let mut items = Vec::new();
            let mut irreversible = Vec::new();
            for (position, change) in builder.changes.into_iter().enumerate() {
                let index = u32::try_from(position).unwrap();
                let target = state.apply(op, index, change);
                if let Some(reason) = target.irreversible {
                    irreversible.push(IrreversibleEntry::new(reason));
                }
                items.push(EntryItem {
                    target,
                    role: TargetRole::Whole,
                    source: None,
                    undone_by: None,
                    released: false,
                });
            }
            if builder.external {
                irreversible.push(IrreversibleEntry::new(Irreversible::ExternalEffect));
            }
            let entry = state.entry_for(
                op,
                OpKind::command("test"),
                builder.session,
                builder.volumes,
                items,
                irreversible,
            );
            state.entries.insert(op, entry);
            state.now += SECOND_NS;
            op
        }
    }

    impl State {
        fn entry_for(
            &mut self,
            op: OpId,
            kind: OpKind,
            session: Option<SessionId>,
            volumes: Vec<VolumeId>,
            items: Vec<EntryItem>,
            irreversible: Vec<IrreversibleEntry>,
        ) -> JournalEntry {
            let manifest_hash = hash_of(format!("manifest {}", op.0).as_bytes());
            let chain = self.last_manifest.replace(manifest_hash);
            let bytes = items
                .iter()
                .filter_map(|item| item.target.after.as_ref().and_then(|after| after.size))
                .sum();
            JournalEntry {
                op,
                time: WallTime::from_unix_nanos(self.now),
                kind,
                session,
                state: EntryState::Committed,
                bulk: items.len() > 1,
                volumes,
                files: items.len() as u64,
                bytes,
                items,
                irreversible,
                undo_of: Vec::new(),
                redo_of: None,
                scope: None,
                ast_hash: hash_of(format!("program {}", op.0).as_bytes()),
                manifest_hash,
                chain,
            }
        }

        #[allow(
            clippy::too_many_lines,
            reason = "one arm per kind of change of the test world"
        )]
        fn apply(&mut self, op: OpId, index: u32, change: Change) -> TargetOutcome {
            let (path, action, mode, before, after) = match change {
                Change::Write(path, content, mode) => {
                    let previous = self.world.insert(path.clone(), Node::file(&content));
                    let mode = if previous.is_some() {
                        mode
                    } else {
                        PreserveMode::Created
                    };
                    (path, TargetAction::Modify, mode, previous, None)
                }
                Change::Create(path, node) => {
                    let previous = self.world.insert(path.clone(), node);
                    (
                        path,
                        TargetAction::Create,
                        PreserveMode::Created,
                        previous,
                        None,
                    )
                }
                Change::Delete(path) => {
                    let previous = self.world.remove(&path);
                    (
                        path,
                        TargetAction::Delete,
                        PreserveMode::Moved,
                        previous,
                        None,
                    )
                }
                Change::Rename(from, to) => {
                    let node = self.world.remove(&TargetPath::Fs(from.clone()));
                    let after = node.clone();
                    if let Some(node) = node.clone() {
                        self.world.insert(TargetPath::Fs(to.clone()), node);
                    }
                    let action = TargetAction::Rename { to, replace: false };
                    (
                        TargetPath::Fs(from),
                        action,
                        PreserveMode::Rename,
                        node,
                        Some(after),
                    )
                }
                Change::Chmod(path, new) => {
                    let previous = self.world.get(&path).cloned();
                    if let Some(node) = self.world.get_mut(&path) {
                        node.set_mode(new);
                    }
                    (
                        path,
                        TargetAction::Attributes,
                        PreserveMode::Meta,
                        previous,
                        None,
                    )
                }
                Change::Append(path, bytes) => {
                    let previous = self.world.get(&path).cloned();
                    if let Some(Node::File { content, .. }) = self.world.get_mut(&path) {
                        content.extend_from_slice(&bytes);
                    }
                    (
                        path,
                        TargetAction::Append,
                        PreserveMode::Append,
                        previous,
                        None,
                    )
                }
                Change::SetValue(path, bytes) => {
                    let previous = self.world.insert(path.clone(), Node::file(&bytes));
                    (
                        path,
                        TargetAction::Modify,
                        PreserveMode::Value,
                        previous,
                        None,
                    )
                }
            };
            let after_node = match after {
                Some(after) => after,
                None => self.world.get(&path).cloned(),
            };
            let keeps_blob = !matches!(
                mode,
                PreserveMode::Created
                    | PreserveMode::Rename
                    | PreserveMode::Append
                    | PreserveMode::None
            );
            if keeps_blob && let Some(previous) = &before {
                self.blobs.insert((op, index), previous.clone());
            }
            let irreversible = (mode == PreserveMode::None).then_some(Irreversible::NoSpaceForCopy);
            TargetOutcome {
                index: TargetIndex(index),
                after: Some(after_of(&path, after_node.as_ref())),
                before: before_of(&path, before.as_ref()),
                path,
                action,
                mode,
                bytes_held: 0,
                irreversible,
                notable: Vec::new(),
            }
        }
    }

    enum Change {
        Write(TargetPath, Vec<u8>, PreserveMode),
        Create(TargetPath, Node),
        Delete(TargetPath),
        Rename(AbsPath, AbsPath),
        Chmod(TargetPath, u32),
        Append(TargetPath, Vec<u8>),
        SetValue(TargetPath, Vec<u8>),
    }

    /// Operation recorded in the test journal.
    pub(crate) struct OpBuilder<'a> {
        journal: &'a TestJournal,
        session: Option<SessionId>,
        changes: Vec<Change>,
        volumes: Vec<VolumeId>,
        external: bool,
    }

    impl OpBuilder<'_> {
        pub(crate) fn write(self, path: &str, content: &[u8]) -> Self {
            self.write_with(path, content, PreserveMode::Exchange)
        }

        pub(crate) fn write_with(mut self, path: &str, content: &[u8], mode: PreserveMode) -> Self {
            self.changes
                .push(Change::Write(fs_path(path), content.to_vec(), mode));
            self
        }

        pub(crate) fn create(mut self, path: &str, content: &[u8]) -> Self {
            self.changes
                .push(Change::Create(fs_path(path), Node::file(content)));
            self
        }

        pub(crate) fn create_dir(mut self, path: &str) -> Self {
            self.changes
                .push(Change::Create(fs_path(path), Node::Dir { mode: DIR_MODE }));
            self
        }

        pub(crate) fn delete(mut self, path: &str) -> Self {
            self.changes.push(Change::Delete(fs_path(path)));
            self
        }

        pub(crate) fn rename(mut self, from: &str, to: &str) -> Self {
            self.changes.push(Change::Rename(abs(from), abs(to)));
            self
        }

        pub(crate) fn chmod(mut self, path: &str, mode: u32) -> Self {
            self.changes.push(Change::Chmod(fs_path(path), mode));
            self
        }

        pub(crate) fn append(mut self, path: &str, bytes: &[u8]) -> Self {
            self.changes
                .push(Change::Append(fs_path(path), bytes.to_vec()));
            self
        }

        pub(crate) fn set_value(mut self, name: &str, bytes: &[u8]) -> Self {
            self.changes
                .push(Change::SetValue(value_path(name), bytes.to_vec()));
            self
        }

        pub(crate) fn external_only(mut self) -> Self {
            self.external = true;
            self
        }

        pub(crate) fn on_volumes(mut self, devs: &[u64]) -> Self {
            self.volumes = devs.iter().map(|&dev| volume(dev)).collect();
            self
        }

        pub(crate) fn commit(self) -> OpId {
            self.journal.record(self)
        }
    }

    /// Read-only file system view of the world.
    struct WorldFs<'a> {
        world: &'a BTreeMap<TargetPath, Node>,
    }

    fn unsupported(op: &str, path: &AbsPath) -> FsError {
        FsError::new(FsErrorKind::Unsupported, op, Some(path.as_path()))
    }

    impl WorldFs<'_> {
        fn node(&self, op: &str, path: &AbsPath) -> FsResult<&Node> {
            if path.as_path().starts_with("/denied") {
                return Err(FsError::new(
                    FsErrorKind::PermissionDenied,
                    op,
                    Some(path.as_path()),
                ));
            }
            self.world
                .get(&TargetPath::Fs(path.clone()))
                .ok_or_else(|| FsError::new(FsErrorKind::NotFound, op, Some(path.as_path())))
        }
    }

    impl Fs for WorldFs<'_> {
        fn metadata(&self, path: &AbsPath, _follow: Follow) -> FsResult<FileMeta> {
            let node = self.node("stat", path)?;
            Ok(FileMeta {
                kind: node.kind(),
                size: node.size().unwrap_or(0),
                mode: node.mode(),
                uid: None,
                gid: None,
                links: 1,
                modified_ns: 0,
                accessed_ns: None,
                created_ns: None,
                changed_ns: None,
                file_id: FileId {
                    volume: 1,
                    low: 0,
                    high: 0,
                },
                windows_attributes: 0,
            })
        }

        fn extended_metadata(&self, path: &AbsPath, _follow: Follow) -> FsResult<ExtendedMeta> {
            Err(unsupported("extended_metadata", path))
        }

        fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
            Err(unsupported("open", path))
        }

        fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
            Err(unsupported("open_stream", path))
        }

        fn read_dir<'a>(
            &'a self,
            path: &AbsPath,
        ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
            Err(unsupported("read_dir", path))
        }

        fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
            Err(unsupported("read_link", path))
        }

        fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
            Ok(path.clone())
        }

        fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
            self.node("hash", path)?.hash().ok_or_else(|| {
                FsError::new(FsErrorKind::IsADirectory, "hash", Some(path.as_path()))
            })
        }

        fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
            Err(unsupported("volume", path))
        }

        fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
            Err(FsError::new(FsErrorKind::Unsupported, "space", None))
        }

        fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
            Ok(PathKey(
                path.as_path().as_os_str().as_encoded_bytes().to_vec(),
            ))
        }

        fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
            Ok(Vec::new())
        }

        fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
            Ok(true)
        }
    }

    impl EntrySource for TestJournal {
        fn entry_ids(&self) -> Result<Vec<OpId>> {
            Ok(self.lock().entries.keys().rev().copied().collect())
        }

        fn load(&self, op: OpId) -> Result<Option<JournalEntry>> {
            Ok(self.lock().entries.get(&op).cloned())
        }

        fn pins(&self) -> Result<PinSet> {
            let files: Vec<Vec<u8>> = self.lock().pins.values().cloned().collect();
            let mut merged = PinSet::default();
            for file in files {
                merged.merge(&PinSet::parse(&file)?);
            }
            Ok(merged)
        }

        fn now(&self) -> WallTime {
            self.now_time()
        }
    }

    impl TargetProbe for TestJournal {
        fn observe(&self, path: &TargetPath, with_hash: bool) -> Result<Observed> {
            let state = self.lock();
            match path {
                TargetPath::Fs(path) => conflict::observe_fs(
                    &WorldFs {
                        world: &state.world,
                    },
                    path,
                    with_hash,
                ),
                TargetPath::Provider(_) => {
                    Ok(state
                        .world
                        .get(path)
                        .map_or_else(Observed::default, |node| Observed {
                            exists: true,
                            hash: node.hash(),
                            ..Observed::default()
                        }))
                }
            }
        }

        fn item_key(&self, path: &TargetPath) -> Result<ItemKey> {
            let state = self.lock();
            conflict::fs_item_key(
                &WorldFs {
                    world: &state.world,
                },
                path,
            )
        }

        fn blob_hashes(
            &self,
            op: OpId,
            indexes: &[TargetIndex],
        ) -> Result<HashMap<TargetIndex, ContentHash>> {
            let state = self.lock();
            Ok(indexes
                .iter()
                .filter_map(|&index| {
                    let hash = state.blobs.get(&(op, index.0)).and_then(Node::hash)?;
                    Some((index, hash))
                })
                .collect())
        }
    }

    impl UndoBackend for TestJournal {
        fn begin_restore<'a>(&'a self, plan: &'a RestorePlan) -> Box<dyn RestoreTx + 'a> {
            self.lock().last_plan_counts_as_step = Some(plan.counts_as_step);
            Box::new(TestTx {
                journal: self,
                plan,
                op: None,
                done: Vec::new(),
                taken: Vec::new(),
                finished: false,
            })
        }

        fn mark_items_undone(&self, op: OpId, items: &[TargetIndex], by: OpId) -> Result<()> {
            let items: HashSet<TargetIndex> = items.iter().copied().collect();
            let mut state = self.lock();
            let entry = state.entries.get_mut(&op).unwrap();
            for item in &mut entry.items {
                if items.contains(&item.target.index) {
                    item.undone_by = Some(by);
                }
            }
            Ok(())
        }

        fn set_state(
            &self,
            op: OpId,
            state: EntryState,
            _cause: Option<UndoneCause>,
        ) -> Result<()> {
            self.lock().entries.get_mut(&op).unwrap().state = state;
            Ok(())
        }

        fn failpoints(&self) -> &dyn Failpoints {
            &*self.failpoints
        }

        fn count_undo_as_op(&self) -> bool {
            true
        }
    }

    impl PinBackend for TestJournal {
        fn read_pins(&self, volume: &VolumeId) -> Result<PinSet> {
            match self.lock().pins.get(volume) {
                Some(bytes) => PinSet::parse(bytes),
                None => Ok(PinSet::default()),
            }
        }

        fn write_pins(&self, volume: &VolumeId, pins: &PinSet) -> Result<()> {
            let bytes = pins.to_bytes()?;
            self.lock().pins.insert(volume.clone(), bytes);
            Ok(())
        }

        fn pin_max_age(&self) -> Duration {
            crate::pin::DEFAULT_PIN_MAX_AGE
        }

        fn expire_over_limits(&self) -> Result<Vec<OpId>> {
            Ok(self.sweep())
        }
    }

    /// Restoring transaction of the test journal: the current state of every
    /// address is kept as the preserved content of the new entry, restores
    /// check the expected state and move the preserved content of the source
    /// into place, and an unfinished transaction is rolled back when dropped,
    /// as recovery does, giving the moved content back to its source.
    struct TestTx<'a> {
        journal: &'a TestJournal,
        plan: &'a RestorePlan,
        op: Option<OpId>,
        /// Restored items: address changed, previous node, new item.
        done: Vec<(TargetPath, TargetPath, Option<Node>, TargetOutcome)>,
        /// Preserved content moved out of source entries.
        taken: Vec<((OpId, u32), Node)>,
        finished: bool,
    }

    fn stale(path: &TargetPath) -> Error {
        Error::new(ErrorCode::Stale, "the target changed during the undo")
            .with_detail("path", serde_json::to_value(path).unwrap())
    }

    fn expect_holds(expect: &Expect, node: Option<&Node>) -> bool {
        match expect {
            Expect::Absent => node.is_none(),
            Expect::Hash(hash) => node.and_then(Node::hash) == Some(*hash),
            _ => true,
        }
    }

    impl TestTx<'_> {
        fn undo_changes(&mut self) -> Vec<TargetPath> {
            let mut state = self.journal.lock();
            let mut restored = Vec::new();
            while let Some((origin, landing, previous, _)) = self.done.pop() {
                let moved = state.world.remove(&landing);
                if landing != origin
                    && let Some(node) = moved
                {
                    state.world.insert(origin.clone(), node);
                }
                match previous {
                    Some(node) => state.world.insert(origin.clone(), node),
                    None if landing == origin => None,
                    None => state.world.remove(&origin),
                };
                restored.push(origin);
            }
            if let Some(op) = self.op {
                state.blobs.retain(|(owner, _), _| *owner != op);
            }
            for (source, node) in self.taken.drain(..) {
                state.blobs.insert(source, node);
            }
            self.finished = true;
            restored
        }
    }

    impl RestoreTx for TestTx<'_> {
        fn preserve(&mut self) -> Result<()> {
            let mut state = self.journal.lock();
            state.seq += 1;
            let op = OpId(state.seq);
            for (n, item) in self.plan.items.iter().enumerate() {
                let address = conflict::primary(&item.target, item.role);
                if let Some(node) = state.world.get(&address).cloned() {
                    state.blobs.insert((op, u32::try_from(n).unwrap()), node);
                }
            }
            self.op = Some(op);
            Ok(())
        }

        fn restore(&mut self, n: usize) -> Result<TargetOutcome> {
            let mut state = self.journal.lock();
            if state.fail_restore_at == Some(n) {
                return Err(Error::new(ErrorCode::Io, "restore failed"));
            }
            let item = &self.plan.items[n];
            let target = &item.target;
            let origin = conflict::primary(target, item.role);
            let current = state.world.get(&origin).cloned();
            if !expect_holds(&item.expect, current.as_ref()) {
                return Err(stale(&origin));
            }
            let landing = if let TargetAction::Rename { .. } = &target.action {
                if let Some(node) = state.world.remove(&origin) {
                    state.world.insert(target.path.clone(), node);
                }
                target.path.clone()
            } else {
                let source = (item.source.op, item.source.index.0);
                match target.mode {
                    PreserveMode::Created => {
                        state.world.remove(&origin);
                    }
                    PreserveMode::Meta => {
                        let mode = target.before.mode.unwrap();
                        state.world.get_mut(&origin).unwrap().set_mode(mode);
                    }
                    PreserveMode::Append => {
                        let size = usize::try_from(target.before.size.unwrap()).unwrap();
                        if let Some(Node::File { content, .. }) = state.world.get_mut(&origin) {
                            content.truncate(size);
                        }
                    }
                    _ => match state.blobs.remove(&source) {
                        Some(node) => {
                            self.taken.push((source, node.clone()));
                            state.world.insert(origin.clone(), node);
                        }
                        None if !target.before.exists => {
                            state.world.remove(&origin);
                        }
                        None => {
                            return Err(Error::new(ErrorCode::JournalCorrupt, "blob missing"));
                        }
                    },
                }
                origin.clone()
            };
            let after = state.world.get(&landing).cloned();
            let index = TargetIndex(u32::try_from(n).unwrap());
            let outcome = match &target.action {
                TargetAction::Rename { .. } => TargetOutcome {
                    index,
                    path: origin.clone(),
                    action: TargetAction::Rename {
                        to: match &landing {
                            TargetPath::Fs(path) => path.clone(),
                            TargetPath::Provider(_) => unreachable!(),
                        },
                        replace: false,
                    },
                    mode: PreserveMode::Rename,
                    bytes_held: 0,
                    before: before_of(&origin, current.as_ref()),
                    after: Some(after_of(&landing, after.as_ref())),
                    irreversible: None,
                    notable: Vec::new(),
                },
                _ => TargetOutcome {
                    index,
                    path: origin.clone(),
                    action: TargetAction::Modify,
                    mode: match (&current, &after) {
                        (None, Some(_)) => PreserveMode::Created,
                        (Some(_), None) => PreserveMode::Moved,
                        _ => PreserveMode::Exchange,
                    },
                    bytes_held: 0,
                    before: before_of(&origin, current.as_ref()),
                    after: Some(after_of(&landing, after.as_ref())),
                    irreversible: None,
                    notable: Vec::new(),
                },
            };
            self.done.push((origin, landing, current, outcome.clone()));
            Ok(outcome)
        }

        fn commit(mut self: Box<Self>) -> Result<Executed> {
            let op = self.op.unwrap();
            let restored: Vec<TargetOutcome> =
                self.done.iter().map(|done| done.3.clone()).collect();
            let items = restored
                .iter()
                .zip(&self.plan.items)
                .map(|(target, item)| EntryItem {
                    target: target.clone(),
                    role: TargetRole::Whole,
                    source: Some(item.source),
                    undone_by: None,
                    released: false,
                })
                .collect();
            let mut state = self.journal.lock();
            let mut entry = state.entry_for(
                op,
                self.plan.kind.clone(),
                self.plan.session.clone(),
                vec![volume(1)],
                items,
                Vec::new(),
            );
            entry.undo_of.clone_from(&self.plan.undo_of);
            entry.redo_of = self.plan.redo_of;
            entry.scope = Some(self.plan.scope);
            state.entries.insert(op, entry);
            state.now += SECOND_NS;
            drop(state);
            self.finished = true;
            self.taken.clear();
            let sources: Vec<ItemSource> = self.plan.items.iter().map(|item| item.source).collect();
            super::update_sources(self.journal, op, self.plan.scope, &sources, self.plan.cause)
                .map_err(|error| error.with_effect_applied(true))?;
            Ok(Executed {
                entry: op,
                restored,
                irreversible: Vec::new(),
            })
        }

        fn rollback(mut self: Box<Self>) -> Result<RollbackReport> {
            let failing = self.journal.lock().fail_rollback;
            let paths: Vec<TargetPath> = self.done.iter().map(|done| done.0.clone()).collect();
            if failing {
                self.finished = true;
                return Ok(RollbackReport {
                    op: self.op,
                    restored: Vec::new(),
                    failed: paths,
                });
            }
            let restored = self.undo_changes();
            Ok(RollbackReport {
                op: self.op,
                restored,
                failed: Vec::new(),
            })
        }
    }

    impl Drop for TestTx<'_> {
        fn drop(&mut self) {
            if !self.finished {
                self.undo_changes();
            }
        }
    }

    /// Registry that fails one point once.
    pub(crate) struct FailOnce {
        point: FailPoint,
        n: Option<u32>,
        action: nurt_fs::failpoint::FailAction,
        fired: Mutex<bool>,
    }

    impl FailOnce {
        pub(crate) fn new(
            point: FailPoint,
            n: Option<u32>,
            action: nurt_fs::failpoint::FailAction,
        ) -> Self {
            Self {
                point,
                n,
                action,
                fired: Mutex::new(false),
            }
        }
    }

    impl Failpoints for FailOnce {
        fn hit(&self, point: FailPoint, n: Option<u32>) -> nurt_fs::failpoint::FailAction {
            let mut fired = self.fired.lock().unwrap_or_else(PoisonError::into_inner);
            if !*fired && point == self.point && n == self.n {
                *fired = true;
                return self.action;
            }
            nurt_fs::failpoint::FailAction::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nurt_contracts::env::{BootId, ClockStamp, WallTime};
    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::fs::FileKind;
    use nurt_contracts::journal::{
        EntryState, HistoryQuery, OnConflict, OpId, OpKind, PinRequest, PreserveMode, RedoRequest,
        RollbackReason, Since, TargetAction, TargetIndex, UndoOutcome, UndoRequest, UndoScope,
        UndoSelector,
    };
    use nurt_fs::failpoint::{FailAction, FailPoint};

    use super::testkit::{
        FailOnce, Node, TestJournal, abs, fs_path, hash_of, program, value_path, volume,
    };
    use super::{
        apply_source_updates, redo_with as redo, rollback_committed_with as rollback_committed,
        undo_with as undo,
    };
    use crate::history::{self, ItemSource, JournalEntry};
    use crate::manifest::{
        AfterRecord, BeforeRecord, FORMAT_VERSION, Integrity, ItemRef, ItemState, Items, Manifest,
        PartRef, StateRecord, TargetRecord, TargetRole,
    };
    use crate::pin;

    /// Number of targets of the bulk tests of the regular run.
    const BULK: usize = 10_000;
    /// Number of targets of the full-scale bulk run.
    const FULL_SCALE: usize = 1_000_000;

    fn request(
        selector: UndoSelector,
        on_conflict: OnConflict,
        session: Option<&str>,
    ) -> UndoRequest {
        UndoRequest {
            selector,
            on_conflict,
            session: session.map(super::testkit::session),
            program: program(),
        }
    }

    fn undo_op(journal: &TestJournal, op: u64, on_conflict: OnConflict) -> Result<UndoOutcome> {
        undo(
            journal,
            &request(UndoSelector::Op(OpId(op)), on_conflict, None),
        )
    }

    fn redo_entry(
        journal: &TestJournal,
        undo_entry: Option<u64>,
        session: Option<&str>,
    ) -> Result<UndoOutcome> {
        redo(
            journal,
            &RedoRequest {
                undo_entry: undo_entry.map(OpId),
                session: session.map(super::testkit::session),
                program: program(),
            },
        )
    }

    fn code<T: std::fmt::Debug>(result: Result<T>) -> ErrorCode {
        result.unwrap_err().code
    }

    fn conflict_kinds(error: &Error) -> Vec<String> {
        error.details["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|found| found["kind"]["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "compared with the optional node found at an address"
    )]
    fn file(content: &[u8]) -> Option<Node> {
        Some(Node::file(content))
    }

    #[test]
    fn undo_last_selects_newest_entry_of_the_session() {
        let journal = TestJournal::new();
        journal.op(Some("a")).create("/home/user/a", b"a").commit();
        journal.op(Some("b")).create("/home/user/b", b"b").commit();
        let outcome = undo(
            &journal,
            &request(UndoSelector::Last, OnConflict::Report, Some("a")),
        )
        .unwrap();
        assert_eq!(outcome.sources, [OpId(1)]);
        assert_eq!(outcome.scope, UndoScope::Op);
        assert_eq!(journal.get("/home/user/a"), None);
        assert_eq!(journal.get("/home/user/b"), file(b"b"));
    }

    #[test]
    fn undo_last_without_session_selects_newest_entry_of_the_account() {
        let journal = TestJournal::new();
        journal.op(Some("a")).create("/home/user/a", b"a").commit();
        journal.op(Some("b")).create("/home/user/b", b"b").commit();
        let outcome = undo(
            &journal,
            &request(UndoSelector::Last, OnConflict::Report, None),
        )
        .unwrap();
        assert_eq!(outcome.sources, [OpId(2)]);
    }

    #[test]
    fn undo_last_skips_undo_entries_and_accepts_redo_entries() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        let last = request(UndoSelector::Last, OnConflict::Report, None);
        assert_eq!(undo(&journal, &last).unwrap().sources, [OpId(2)]);
        assert_eq!(undo(&journal, &last).unwrap().sources, [OpId(1)]);
        let redone = redo_entry(&journal, None, None).unwrap();
        assert_eq!(journal.get("/home/user/a"), file(b"a"));
        assert_eq!(undo(&journal, &last).unwrap().sources, [redone.entry]);
        assert_eq!(journal.get("/home/user/a"), None);
    }

    #[test]
    fn undo_last_without_candidate_reports_noop() {
        let journal = TestJournal::new();
        let last = request(UndoSelector::Last, OnConflict::Report, None);
        assert_eq!(code(undo(&journal, &last)), ErrorCode::JournalNoop);
        journal.op(Some("a")).create("/home/user/a", b"a").commit();
        let other = request(UndoSelector::Last, OnConflict::Report, Some("x"));
        assert_eq!(code(undo(&journal, &other)), ErrorCode::JournalNoop);
    }

    /// Undoes `op`, compares the world with `before`, redoes and compares the
    /// world with the state after the operation.
    fn roundtrip(
        journal: &TestJournal,
        op: OpId,
        before: &std::collections::BTreeMap<nurt_contracts::journal::TargetPath, Node>,
    ) {
        let after = journal.world();
        let outcome = undo_op(journal, op.0, OnConflict::Report).unwrap();
        assert_eq!(&journal.world(), before);
        assert_eq!(journal.entry(op).state, EntryState::Undone);
        redo_entry(journal, Some(outcome.entry.0), None).unwrap();
        assert_eq!(journal.world(), after);
        assert_eq!(journal.entry(outcome.entry).state, EntryState::Undone);
    }

    #[test]
    fn undo_op_roundtrip_restores_overwritten_file_for_every_overwrite_mode() {
        let modes = [
            PreserveMode::Exchange,
            PreserveMode::ReplaceBackup,
            PreserveMode::Hardlink,
            PreserveMode::Clone,
            PreserveMode::Copy,
        ];
        for mode in modes {
            let journal = TestJournal::new();
            journal.put("/home/user/a", b"old");
            let before = journal.world();
            let op = journal
                .op(None)
                .write_with("/home/user/a", b"new", mode)
                .commit();
            roundtrip(&journal, op, &before);
        }
    }

    #[test]
    fn undo_op_roundtrip_restores_deleted_file_and_directory() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"a");
        journal.put_dir("/home/user/d");
        let before = journal.world();
        let op = journal
            .op(None)
            .delete("/home/user/a")
            .delete("/home/user/d")
            .commit();
        roundtrip(&journal, op, &before);
    }

    #[test]
    fn undo_op_roundtrip_removes_created_entries() {
        let journal = TestJournal::new();
        let before = journal.world();
        let op = journal
            .op(None)
            .create("/home/user/a", b"a")
            .create_dir("/home/user/d")
            .commit();
        roundtrip(&journal, op, &before);
    }

    #[test]
    fn undo_op_roundtrip_restores_metadata_append_value_and_rename() {
        let journal = TestJournal::new();
        journal.put("/home/user/m", b"m");
        journal.put("/home/user/log", b"abc");
        journal.put("/home/user/r", b"r");
        journal.put_node(value_path("PATH"), Node::file(b"/usr/bin"));
        let before = journal.world();
        let op = journal
            .op(None)
            .chmod("/home/user/m", 0o600)
            .append("/home/user/log", b"def")
            .set_value("PATH", b"/opt/bin")
            .rename("/home/user/r", "/home/user/s")
            .commit();
        assert_eq!(journal.get("/home/user/s"), file(b"r"));
        roundtrip(&journal, op, &before);
        let prior = journal.world();
        let created = journal.op(None).set_value("HOME", b"/home/user").commit();
        let before_value = journal.entry(created).items[0].target.before.exists;
        assert!(!before_value);
        undo_op(&journal, created.0, OnConflict::Report).unwrap();
        assert_eq!(journal.world(), prior);
    }

    #[test]
    fn undo_states_report_expired_undone_corrupt_unknown_and_transient_entries() {
        let journal = TestJournal::new();
        for name in ["a", "b", "c", "d"] {
            journal
                .op(None)
                .create(&format!("/home/user/{name}"), b"x")
                .commit();
        }
        journal.expire(OpId(1));
        journal.set_state(OpId(2), EntryState::Undone);
        journal.set_state(OpId(3), EntryState::Corrupt);
        journal.set_state(OpId(4), EntryState::Prepared);
        let expired = undo_op(&journal, 1, OnConflict::Report).unwrap_err();
        assert_eq!(expired.code, ErrorCode::JournalExpired);
        assert_eq!(expired.details["expired"], serde_json::json!([1]));
        assert_eq!(
            code(undo_op(&journal, 2, OnConflict::Report)),
            ErrorCode::JournalNoop
        );
        assert_eq!(
            code(undo_op(&journal, 3, OnConflict::Report)),
            ErrorCode::JournalCorrupt
        );
        assert_eq!(
            code(undo_op(&journal, 4, OnConflict::Report)),
            ErrorCode::JournalNoop
        );
        assert_eq!(
            code(undo_op(&journal, 9, OnConflict::Report)),
            ErrorCode::JournalNoop
        );
    }

    #[test]
    fn undo_of_external_effect_only_reports_noop() {
        let journal = TestJournal::new();
        journal.op(None).external_only().commit();
        let error = undo_op(&journal, 1, OnConflict::Report).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalNoop);
        assert_eq!(error.problem.as_deref(), Some("nothing_restorable"));
    }

    #[test]
    fn conflict_kinds_report_each_difference() {
        let later = TestJournal::new();
        later.put("/home/user/a", b"0");
        later.op(None).write("/home/user/a", b"1").commit();
        later.op(None).write("/home/user/a", b"2").commit();
        let error = undo_op(&later, 1, OnConflict::Report).unwrap_err();
        assert_eq!(conflict_kinds(&error), ["later_entry"]);
        assert_eq!(error.details["conflicts"][0]["kind"]["op"], 2);

        let modified = TestJournal::new();
        modified.op(None).create("/home/user/a", b"x").commit();
        modified.put("/home/user/a", b"y");
        let error = undo_op(&modified, 1, OnConflict::Report).unwrap_err();
        assert_eq!(conflict_kinds(&error), ["modified"]);
        assert_eq!(
            error.details["conflicts"][0]["actual"],
            hash_of(b"y").to_string()
        );

        let deleted = TestJournal::new();
        deleted.op(None).create("/home/user/a", b"x").commit();
        deleted.remove("/home/user/a");
        assert_eq!(
            conflict_kinds(&undo_op(&deleted, 1, OnConflict::Report).unwrap_err()),
            ["deleted"]
        );

        let created = TestJournal::new();
        created.put("/home/user/a", b"x");
        created.op(None).delete("/home/user/a").commit();
        created.put("/home/user/a", b"new");
        assert_eq!(
            conflict_kinds(&undo_op(&created, 1, OnConflict::Report).unwrap_err()),
            ["created"]
        );

        let replaced = TestJournal::new();
        replaced.op(None).create("/home/user/a", b"x").commit();
        replaced.remove("/home/user/a");
        replaced.put_dir("/home/user/a");
        assert_eq!(
            conflict_kinds(&undo_op(&replaced, 1, OnConflict::Report).unwrap_err()),
            ["type_changed"]
        );

        let blob = TestJournal::new();
        blob.put("/home/user/a", b"old");
        blob.op(None).write("/home/user/a", b"new").commit();
        blob.tamper_blob(OpId(1), 0, b"written through an open descriptor");
        assert_eq!(
            conflict_kinds(&undo_op(&blob, 1, OnConflict::Report).unwrap_err()),
            ["blob_modified"]
        );
    }

    #[test]
    fn conflict_writes_nothing() {
        type Scenario = (UndoSelector, fn(&TestJournal));
        let scenarios: Vec<Scenario> = vec![
            (UndoSelector::Op(OpId(1)), |j| {
                j.put("/home/user/a", b"other");
            }),
            (UndoSelector::Op(OpId(1)), |j| {
                j.op(None).write("/home/user/a", b"later").commit();
            }),
            (UndoSelector::Last, |j| j.remove("/home/user/b")),
            (
                UndoSelector::To {
                    after: OpId(0),
                    all_sessions: false,
                },
                |j| j.put("/home/user/b", b"x"),
            ),
            (
                UndoSelector::File {
                    path: abs("/home/user/a"),
                    since: None,
                },
                |j| j.put("/home/user/a", b"x"),
            ),
            (
                UndoSelector::Paths {
                    op: OpId(1),
                    paths: vec![fs_path("/home/user/a")],
                },
                |j| j.tamper_blob(OpId(1), 0, b"changed"),
            ),
        ];
        for (selector, disturb) in scenarios {
            let journal = TestJournal::new();
            journal.put("/home/user/a", b"0");
            journal
                .op(None)
                .write("/home/user/a", b"1")
                .create("/home/user/b", b"b")
                .commit();
            disturb(&journal);
            let snapshot = journal.snapshot();
            let error = undo(
                &journal,
                &request(selector.clone(), OnConflict::Report, None),
            )
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::JournalConflict, "{selector:?}");
            assert_eq!(journal.snapshot(), snapshot, "{selector:?}");
        }
    }

    #[test]
    fn on_conflict_report_lists_conflicts_and_ready_programs() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"x").commit();
        journal.put("/home/user/a", b"y");
        let error = undo_op(&journal, 1, OnConflict::Report).unwrap_err();
        assert!(error.fix.contains(&"undo 1 on_conflict: force".to_owned()));
        assert!(
            error
                .fix
                .contains(&"undo 1 on_conflict: include_later".to_owned())
        );
        let file = undo(
            &journal,
            &request(
                UndoSelector::File {
                    path: abs("/home/user/a"),
                    since: Some(Since::Op(OpId(0))),
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap_err();
        assert_eq!(
            file.fix,
            ["undo file: \"/home/user/a\" since: 0 on_conflict: force"]
        );
    }

    #[test]
    fn on_conflict_force_keeps_current_content_in_the_undo_entry() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"x").commit();
        journal.put("/home/user/a", b"y");
        let outcome = undo_op(&journal, 1, OnConflict::Force).unwrap();
        assert_eq!(outcome.forced.len(), 1);
        assert_eq!(journal.get("/home/user/a"), None);
        redo_entry(&journal, Some(outcome.entry.0), None).unwrap();
        assert_eq!(journal.get("/home/user/a"), file(b"y"));
    }

    #[test]
    fn on_conflict_include_later_undoes_later_entries_transitively_newest_first() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"0");
        journal.op(None).write("/home/user/a", b"1").commit();
        journal
            .op(None)
            .write("/home/user/a", b"2")
            .create("/home/user/b", b"b")
            .commit();
        journal.op(None).write("/home/user/b", b"b2").commit();
        journal.op(None).create("/home/user/c", b"c").commit();
        let outcome = undo_op(&journal, 1, OnConflict::IncludeLater).unwrap();
        assert_eq!(outcome.sources, [OpId(3), OpId(2), OpId(1)]);
        assert_eq!(journal.get("/home/user/a"), file(b"0"));
        assert_eq!(journal.get("/home/user/b"), None);
        assert_eq!(journal.get("/home/user/c"), file(b"c"));
    }

    #[test]
    fn on_conflict_include_later_with_expiring_later_entry_changes_nothing() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"0");
        journal.op(None).write("/home/user/a", b"1").commit();
        journal.op(None).write("/home/user/a", b"2").commit();
        journal.set_state(OpId(2), EntryState::Expiring);
        let snapshot = journal.snapshot();
        let error = undo_op(&journal, 1, OnConflict::IncludeLater).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalExpired);
        assert_eq!(error.details["expired"], serde_json::json!([2]));
        assert_eq!(journal.snapshot(), snapshot);
    }

    #[test]
    fn on_conflict_abort_reports_only_the_abort() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"x").commit();
        journal.op(None).create("/home/user/b", b"x").commit();
        journal.put("/home/user/a", b"y");
        let error = undo_op(&journal, 1, OnConflict::Abort).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalConflict);
        assert_eq!(error.problem.as_deref(), Some("aborted"));
        assert!(!error.details.contains_key("conflicts"));
        undo_op(&journal, 2, OnConflict::Abort).unwrap();
        assert_eq!(journal.get("/home/user/b"), None);
    }

    #[test]
    fn undo_partial_entry_follows_on_conflict() {
        let setup = || {
            let journal = TestJournal::new();
            journal
                .op(None)
                .create("/home/user/a", b"a")
                .create("/home/user/b", b"b")
                .commit();
            journal.release_item(OpId(1), 1);
            journal
        };
        for mode in [
            OnConflict::Report,
            OnConflict::Abort,
            OnConflict::IncludeLater,
        ] {
            let journal = setup();
            let snapshot = journal.snapshot();
            let error = undo_op(&journal, 1, mode).unwrap_err();
            assert_eq!(error.code, ErrorCode::JournalPartial);
            assert_eq!(
                error.details["unavailable"].as_array().map(Vec::len),
                Some(1)
            );
            assert_eq!(journal.snapshot(), snapshot);
        }
        for mode in [OnConflict::Partial, OnConflict::Force] {
            let journal = setup();
            let outcome = undo_op(&journal, 1, mode).unwrap();
            assert_eq!(outcome.unavailable, [fs_path("/home/user/b")]);
            assert_eq!(journal.get("/home/user/a"), None);
            assert_eq!(journal.get("/home/user/b"), file(b"b"));
            assert_eq!(journal.entry(OpId(1)).state, EntryState::Undone);
        }
    }

    #[test]
    fn undo_to_undoes_the_range_newest_first_in_one_entry() {
        let journal = TestJournal::new();
        for name in ["a", "b", "c"] {
            journal
                .op(None)
                .create(&format!("/home/user/{name}"), b"x")
                .commit();
        }
        let outcome = undo(
            &journal,
            &request(
                UndoSelector::To {
                    after: OpId(1),
                    all_sessions: false,
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap();
        assert_eq!(outcome.sources, [OpId(3), OpId(2)]);
        let entry = journal.entry(outcome.entry);
        assert_eq!(entry.undo_of, [OpId(3), OpId(2)]);
        assert_eq!(entry.scope, Some(UndoScope::To));
        assert_eq!(journal.get("/home/user/a"), file(b"x"));
        assert_eq!(
            (journal.get("/home/user/b"), journal.get("/home/user/c")),
            (None, None)
        );
        assert_eq!(journal.entry(OpId(2)).state, EntryState::Undone);
    }

    #[test]
    fn undo_to_with_conflict_in_the_middle_changes_nothing() {
        let journal = TestJournal::new();
        for name in ["a", "b", "c"] {
            journal
                .op(None)
                .create(&format!("/home/user/{name}"), b"x")
                .commit();
        }
        journal.put("/home/user/b", b"edited");
        let snapshot = journal.snapshot();
        let to = request(
            UndoSelector::To {
                after: OpId(1),
                all_sessions: false,
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(code(undo(&journal, &to)), ErrorCode::JournalConflict);
        assert_eq!(journal.snapshot(), snapshot);
        let forced = request(
            UndoSelector::To {
                after: OpId(1),
                all_sessions: false,
            },
            OnConflict::IncludeLater,
            None,
        );
        assert_eq!(code(undo(&journal, &forced)), ErrorCode::JournalConflict);
    }

    #[test]
    fn undo_to_skips_other_sessions_unless_all_sessions() {
        let setup = || {
            let journal = TestJournal::new();
            journal.op(Some("a")).create("/home/user/a", b"a").commit();
            journal.op(Some("b")).create("/home/user/b", b"b").commit();
            journal.op(Some("a")).create("/home/user/c", b"c").commit();
            journal
        };
        let journal = setup();
        let own = request(
            UndoSelector::To {
                after: OpId(1),
                all_sessions: false,
            },
            OnConflict::Report,
            Some("a"),
        );
        let outcome = undo(&journal, &own).unwrap();
        assert_eq!(
            outcome.skipped.iter().map(|s| s.op).collect::<Vec<_>>(),
            [OpId(2)]
        );
        assert_eq!(journal.get("/home/user/b"), file(b"b"));
        assert_eq!(journal.get("/home/user/c"), None);
        let journal = setup();
        let all = request(
            UndoSelector::To {
                after: OpId(1),
                all_sessions: true,
            },
            OnConflict::Report,
            Some("a"),
        );
        let outcome = undo(&journal, &all).unwrap();
        assert!(outcome.skipped.is_empty());
        assert_eq!(journal.get("/home/user/b"), None);
    }

    #[test]
    fn undo_to_with_expired_entry_in_the_range_reports_expired() {
        let journal = TestJournal::new();
        for name in ["a", "b", "c"] {
            journal
                .op(None)
                .create(&format!("/home/user/{name}"), b"x")
                .commit();
        }
        journal.expire(OpId(2));
        let snapshot = journal.snapshot();
        let error = undo(
            &journal,
            &request(
                UndoSelector::To {
                    after: OpId(1),
                    all_sessions: false,
                },
                OnConflict::Force,
                None,
            ),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalExpired);
        assert_eq!(error.details["expired"], serde_json::json!([2]));
        assert_eq!(journal.snapshot(), snapshot);
    }

    #[test]
    fn undo_to_skips_undo_pairs_and_undoes_redo_inside_the_range() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        undo_op(&journal, 2, OnConflict::Report).unwrap();
        let to = request(
            UndoSelector::To {
                after: OpId(1),
                all_sessions: false,
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(code(undo(&journal, &to)), ErrorCode::JournalNoop);
        redo_entry(&journal, Some(3), None).unwrap();
        assert_eq!(journal.get("/home/user/b"), file(b"b"));
        let outcome = undo(&journal, &to).unwrap();
        assert_eq!(outcome.sources, [OpId(4)]);
        assert_eq!(journal.get("/home/user/b"), None);
        assert_eq!(journal.get("/home/user/a"), file(b"a"));
    }

    fn file_history() -> TestJournal {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"1")
            .create("/home/user/c", b"c")
            .commit();
        journal.op(None).write("/home/user/a", b"2").commit();
        journal
            .op(None)
            .write("/home/user/a", b"3")
            .create("/home/user/d", b"d")
            .commit();
        journal
    }

    #[test]
    fn undo_file_restores_state_before_oldest_entry_and_leaves_other_targets() {
        let journal = file_history();
        let outcome = undo(
            &journal,
            &request(
                UndoSelector::File {
                    path: abs("/home/user/a"),
                    since: None,
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap();
        assert_eq!(outcome.scope, UndoScope::File);
        assert_eq!(outcome.sources, [OpId(3), OpId(2), OpId(1)]);
        assert_eq!(journal.get("/home/user/a"), None);
        assert_eq!(journal.get("/home/user/c"), file(b"c"));
        assert_eq!(journal.get("/home/user/d"), file(b"d"));
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Partial);
        assert_eq!(journal.entry(OpId(2)).state, EntryState::Undone);
        assert_eq!(journal.entry(OpId(3)).state, EntryState::Partial);
        assert_eq!(
            journal.entry(OpId(1)).items[0].undone_by,
            Some(outcome.entry)
        );
    }

    #[test]
    fn undo_file_since_operation_or_instant() {
        let journal = file_history();
        let since_op = request(
            UndoSelector::File {
                path: abs("/home/user/a"),
                since: Some(Since::Op(OpId(1))),
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(
            undo(&journal, &since_op).unwrap().sources,
            [OpId(3), OpId(2)]
        );
        assert_eq!(journal.get("/home/user/a"), file(b"1"));
        let journal = file_history();
        let instant = journal.entry(OpId(3)).time;
        let since_time = request(
            UndoSelector::File {
                path: abs("/home/user/a"),
                since: Some(Since::Time(instant)),
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(undo(&journal, &since_time).unwrap().sources, [OpId(3)]);
        assert_eq!(journal.get("/home/user/a"), file(b"2"));
        journal.expire(OpId(1));
        let expired = request(
            UndoSelector::File {
                path: abs("/home/user/a"),
                since: Some(Since::Op(OpId(0))),
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(code(undo(&journal, &expired)), ErrorCode::JournalExpired);
        let untouched = request(
            UndoSelector::File {
                path: abs("/home/user/none"),
                since: None,
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(code(undo(&journal, &untouched)), ErrorCode::JournalNoop);
    }

    #[test]
    fn undo_paths_undoes_selected_items_until_the_entry_is_undone() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .create("/home/user/c", b"c")
            .commit();
        let first = undo(
            &journal,
            &request(
                UndoSelector::Paths {
                    op: OpId(1),
                    paths: vec![fs_path("/home/user/a")],
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap();
        assert_eq!(first.scope, UndoScope::Paths);
        assert_eq!(journal.get("/home/user/a"), None);
        assert_eq!(journal.get("/home/user/b"), file(b"b"));
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Partial);
        assert_eq!(journal.entry(OpId(1)).items[0].undone_by, Some(first.entry));
        let again = request(
            UndoSelector::Paths {
                op: OpId(1),
                paths: vec![fs_path("/home/user/a")],
            },
            OnConflict::Report,
            None,
        );
        assert_eq!(code(undo(&journal, &again)), ErrorCode::JournalNoop);
        let rest = vec![fs_path("/home/user/b"), fs_path("/home/user/c")];
        undo(
            &journal,
            &request(
                UndoSelector::Paths {
                    op: OpId(1),
                    paths: rest,
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap();
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Undone);
        let whole = undo_op(&journal, 1, OnConflict::Report).unwrap_err();
        assert_eq!(whole.code, ErrorCode::JournalNoop);
    }

    #[test]
    fn undo_paths_checks_only_selected_items() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        journal.put("/home/user/b", b"edited");
        undo(
            &journal,
            &request(
                UndoSelector::Paths {
                    op: OpId(1),
                    paths: vec![fs_path("/home/user/a")],
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap();
        let unknown = request(
            UndoSelector::Paths {
                op: OpId(1),
                paths: vec![fs_path("/home/user/z")],
            },
            OnConflict::Report,
            None,
        );
        let error = undo(&journal, &unknown).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalNoop);
        assert_eq!(error.problem.as_deref(), Some("path_not_in_operation"));
        assert_eq!(error.sample, ["/home/user/z"]);
    }

    #[test]
    fn undo_entry_manifest_records_sources_scope_and_step() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        let seq = journal.seq();
        let outcome = undo_op(&journal, 1, OnConflict::Report).unwrap();
        assert_eq!(journal.seq(), seq + 1);
        assert_eq!(journal.last_plan_counts_as_step(), Some(true));
        let entry = journal.entry(outcome.entry);
        assert_eq!(entry.kind, OpKind::undo());
        assert_eq!(entry.undo_of, [OpId(1)]);
        assert_eq!(entry.scope, Some(UndoScope::Op));
        let sources: Vec<_> = entry.items.iter().map(|item| item.source).collect();
        assert_eq!(
            sources,
            [
                Some(ItemSource {
                    op: OpId(1),
                    index: TargetIndex(1)
                }),
                Some(ItemSource {
                    op: OpId(1),
                    index: TargetIndex(0)
                }),
            ]
        );
        assert_eq!(outcome.restored.len(), 2);
    }

    #[test]
    fn redo_selects_latest_undo_of_the_session() {
        let journal = TestJournal::new();
        journal.op(Some("a")).create("/home/user/a", b"a").commit();
        journal.op(Some("b")).create("/home/user/b", b"b").commit();
        undo(
            &journal,
            &request(UndoSelector::Last, OnConflict::Report, Some("a")),
        )
        .unwrap();
        undo(
            &journal,
            &request(UndoSelector::Last, OnConflict::Report, Some("b")),
        )
        .unwrap();
        let outcome = redo_entry(&journal, None, Some("a")).unwrap();
        assert_eq!(outcome.sources, [OpId(3)]);
        assert_eq!(journal.entry(outcome.entry).redo_of, Some(OpId(3)));
        assert_eq!(journal.get("/home/user/a"), file(b"a"));
        assert_eq!(journal.get("/home/user/b"), None);
        assert_eq!(
            code(redo_entry(&journal, None, Some("c"))),
            ErrorCode::JournalNoop
        );
    }

    #[test]
    fn redo_of_file_scope_restores_only_its_items() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/c", b"c")
            .commit();
        let undone = undo(
            &journal,
            &request(
                UndoSelector::File {
                    path: abs("/home/user/a"),
                    since: None,
                },
                OnConflict::Report,
                None,
            ),
        )
        .unwrap();
        journal.remove("/home/user/c");
        redo_entry(&journal, Some(undone.entry.0), None).unwrap();
        assert_eq!(journal.get("/home/user/a"), file(b"a"));
        assert_eq!(journal.get("/home/user/c"), None);
        assert_eq!(journal.entry(undone.entry).state, EntryState::Undone);
    }

    #[test]
    fn redo_is_invalidated_by_a_later_operation_and_requires_an_undo_entry() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        undo_op(&journal, 1, OnConflict::Report).unwrap();
        journal.op(None).create("/home/user/a", b"new").commit();
        let snapshot = journal.snapshot();
        let error = redo_entry(&journal, Some(2), None).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalConflict);
        assert_eq!(journal.snapshot(), snapshot);
        let not_undo = redo_entry(&journal, Some(3), None).unwrap_err();
        assert_eq!(not_undo.problem.as_deref(), Some("not_an_undo"));
    }

    #[test]
    fn undo_crash_at_each_point_leaves_the_state_before_the_undo() {
        let points = [
            (FailPoint::UndoChecked, None),
            (FailPoint::UndoRestored, Some(0)),
            (FailPoint::UndoRestored, Some(1)),
        ];
        for (point, n) in points {
            let journal =
                TestJournal::with_failpoints(Box::new(FailOnce::new(point, n, FailAction::Crash)));
            journal.put("/home/user/a", b"0");
            journal
                .op(None)
                .write("/home/user/a", b"1")
                .create("/home/user/b", b"b")
                .commit();
            let world = journal.world();
            assert_eq!(
                code(undo_op(&journal, 1, OnConflict::Report)),
                ErrorCode::Io
            );
            assert_eq!(journal.world(), world, "{point:?} {n:?}");
            assert_eq!(journal.entry(OpId(1)).state, EntryState::Committed);
            undo_op(&journal, 1, OnConflict::Report).unwrap();
            assert_eq!(journal.get("/home/user/a"), file(b"0"));
        }
    }

    #[test]
    fn undo_fault_rolls_back_and_reports_the_error() {
        let fault = FailAction::Error(nurt_contracts::fs::FsErrorKind::PermissionDenied);
        let journal = TestJournal::with_failpoints(Box::new(FailOnce::new(
            FailPoint::UndoRestored,
            Some(0),
            fault,
        )));
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        let world = journal.world();
        assert_eq!(
            code(undo_op(&journal, 1, OnConflict::Report)),
            ErrorCode::Permission
        );
        assert_eq!(journal.world(), world);
        journal.fail_restore_at(1);
        journal.fail_rollback();
        assert_eq!(
            code(undo_op(&journal, 1, OnConflict::Report)),
            ErrorCode::RollbackFailed
        );
    }

    /// Records one bulk operation of `count` targets and checks that undo
    /// restores the world byte for byte; returns the durations of the
    /// operation and of the undo.
    fn bulk(count: usize, kind: &str) -> (Duration, Duration) {
        let journal = TestJournal::new();
        for n in 0..count {
            journal.put(&format!("/home/user/f{n}"), format!("{n}").as_bytes());
        }
        let before = journal.world();
        let started = Instant::now();
        let mut builder = journal.op(None);
        for n in 0..count {
            let path = format!("/home/user/f{n}");
            builder = match kind {
                "remove" => builder.delete(&path),
                "move" => builder.rename(&path, &format!("/home/user/g{n}")),
                "replace" => builder.write(&path, b"replaced"),
                _ => builder.create(&format!("/home/user/x/e{n}"), b"extracted"),
            };
        }
        let op = builder.commit();
        let operation = started.elapsed();
        let started = Instant::now();
        undo_op(&journal, op.0, OnConflict::Report).unwrap();
        let restore = started.elapsed();
        assert_eq!(journal.world(), before, "{kind}");
        (operation, restore)
    }

    #[test]
    fn undo_mass_operations_restore_every_target() {
        for kind in ["remove", "move", "replace", "extract"] {
            bulk(BULK, kind);
        }
    }

    #[test]
    #[ignore = "full-scale run"]
    fn undo_mass_full_scale_within_three_times_the_operation() {
        for kind in ["remove", "move", "replace", "extract"] {
            let (operation, restore) = bulk(FULL_SCALE, kind);
            assert!(
                restore <= operation * 3,
                "{kind}: {restore:?} > 3 x {operation:?}"
            );
        }
    }

    #[test]
    fn undo_multi_volume_entry_restores_every_part() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/media/disk/b", b"b")
            .on_volumes(&[1, 2])
            .commit();
        let outcome = undo_op(&journal, 1, OnConflict::Report).unwrap();
        assert_eq!(outcome.restored.len(), 2);
        assert_eq!(
            (journal.get("/home/user/a"), journal.get("/media/disk/b")),
            (None, None)
        );
    }

    #[test]
    fn history_no_expiry_fields() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        let pinned = pin::pin_with(
            &journal,
            &PinRequest {
                ids: vec![OpId(1)],
                release: false,
                until: None,
            },
        )
        .unwrap();
        let outcome = undo_op(&journal, 2, OnConflict::Report).unwrap();
        let records = history::history_with(&journal, &HistoryQuery::default()).unwrap();
        let details = history::details_with(&journal, OpId(1)).unwrap();
        let texts = [
            serde_json::to_string(&records).unwrap(),
            serde_json::to_string(&details).unwrap(),
            serde_json::to_string(&outcome).unwrap(),
            serde_json::to_string(&pinned).unwrap(),
        ];
        for text in texts {
            for forbidden in [
                "deadline",
                "after_seconds",
                "after_ops",
                "expires",
                "until",
                "remaining",
            ] {
                assert!(!text.contains(forbidden), "{forbidden} in {text}");
            }
        }
    }

    #[test]
    fn rollback_committed_restores_and_marks_the_source() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"0");
        journal.op(None).write("/home/user/a", b"1").commit();
        let outcome = rollback_committed(&journal, OpId(1), RollbackReason::VerifyFailed).unwrap();
        assert_eq!(outcome.sources, [OpId(1)]);
        assert_eq!(journal.get("/home/user/a"), file(b"0"));
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Undone);
        assert_eq!(journal.entry(outcome.entry).undo_of, [OpId(1)]);
        let last = request(UndoSelector::Last, OnConflict::Report, None);
        assert_eq!(code(undo(&journal, &last)), ErrorCode::JournalNoop);
        assert_eq!(
            code(rollback_committed(
                &journal,
                OpId(1),
                RollbackReason::ValidateFailed
            )),
            ErrorCode::JournalNoop
        );
        assert_eq!(
            code(rollback_committed(
                &journal,
                OpId(9),
                RollbackReason::Cancelled
            )),
            ErrorCode::JournalNoop
        );
    }

    #[test]
    fn rollback_committed_reports_items_not_restored() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.fail_restore_at(0);
        let error =
            rollback_committed(&journal, OpId(1), RollbackReason::TargetFailed).unwrap_err();
        assert_eq!(error.code, ErrorCode::RollbackFailed);
        assert_eq!(error.details["failed"].as_array().map(Vec::len), Some(1));
        journal.op(None).external_only().commit();
        assert_eq!(
            code(rollback_committed(
                &journal,
                OpId(2),
                RollbackReason::VerifyFailed
            )),
            ErrorCode::JournalNoop
        );
    }

    #[test]
    fn concurrent_undo_of_one_entry_restores_once() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        let results: Vec<Result<UndoOutcome>> = std::thread::scope(|scope| {
            let first = scope.spawn(|| undo_op(&journal, 1, OnConflict::Report));
            let second = scope.spawn(|| undo_op(&journal, 1, OnConflict::Report));
            vec![first.join().unwrap(), second.join().unwrap()]
        });
        let succeeded = results.iter().filter(|result| result.is_ok()).count();
        assert_eq!(succeeded, 1);
        for result in results.iter().filter_map(|result| result.as_ref().err()) {
            assert!(
                matches!(
                    result.code,
                    ErrorCode::JournalNoop | ErrorCode::Stale | ErrorCode::JournalConflict
                ),
                "{result:?}"
            );
        }
        assert_eq!(
            (journal.get("/home/user/a"), journal.get("/home/user/b")),
            (None, None)
        );
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Undone);
    }

    fn manifest(op: u64, targets: Vec<TargetRecord>) -> Manifest {
        let time = WallTime::from_unix_nanos(5);
        Manifest {
            v: FORMAT_VERSION,
            op: OpId(op),
            uuid: uuid::Uuid::nil(),
            time,
            time_basis: ClockStamp {
                wall: time,
                boot: BootId("boot".to_owned()),
                since_boot_with_suspend: Duration::ZERO,
                since_boot_active: Duration::ZERO,
            },
            kind: OpKind::undo(),
            session: None,
            ast_hash: hash_of(b"program"),
            ast: serde_json::Value::Null,
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            targets,
            parts: vec![
                PartRef {
                    vol: volume(1),
                    dir: abs("/home/user/.nurt-journal"),
                },
                PartRef {
                    vol: volume(2),
                    dir: abs("/media/disk/.nurt-journal"),
                },
            ],
            bytes_held: 0,
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
            expires: None,
            undo_of: vec![OpId(3)],
            redo_of: None,
            scope: Some(UndoScope::Paths),
            items: Vec::new(),
            integrity: Integrity::default(),
        }
    }

    fn record(index: u32, path: &str) -> TargetRecord {
        TargetRecord {
            index: TargetIndex(index),
            path: fs_path(path),
            action: TargetAction::Modify,
            role: TargetRole::Whole,
            before: BeforeRecord {
                exists: true,
                kind: Some(FileKind::File),
                size: Some(1),
                mode: Some(0o644),
                uid: None,
                gid: None,
                modified_ns: None,
                hash: Some(hash_of(b"x")),
                links: Some(1),
                file_id: None,
                store: PreserveMode::Exchange,
                blob: None,
                meta: None,
            },
            replaced: None,
            after: Some(AfterRecord {
                exists: true,
                kind: Some(FileKind::File),
                size: Some(4),
                mode: Some(0o644),
                hash: Some(hash_of(b"yyyy")),
                modified_ns: None,
            }),
            in_place: false,
            bytes_held: 1,
            irreversible: None,
            notable: Vec::new(),
        }
    }

    #[test]
    fn entry_from_parts_joins_manifests_states_and_item_states() {
        let mut first = manifest(7, vec![record(0, "/home/user/a")]);
        first.items = vec![
            ItemRef {
                op: OpId(3),
                index: TargetIndex(4),
                path: fs_path("/home/user/a"),
            },
            ItemRef {
                op: OpId(3),
                index: TargetIndex(5),
                path: fs_path("/media/disk/b"),
            },
        ];
        first.integrity.manifest_sha256 = Some(hash_of(b"manifest"));
        let mut second = first.clone();
        second.targets = vec![record(1, "/media/disk/b")];
        let released = Items {
            items: vec![ItemState {
                index: TargetIndex(1),
                released: true,
                undone_by: None,
            }],
        };
        let parts = vec![
            (
                first,
                StateRecord::new(EntryState::Committed),
                Items { items: Vec::new() },
            ),
            (second, StateRecord::new(EntryState::Partial), released),
        ];
        let entry = JournalEntry::from_parts(&parts).unwrap();
        assert_eq!(entry.state, EntryState::Partial);
        assert_eq!(entry.files, 2);
        assert_eq!(entry.bytes, 8);
        assert_eq!(entry.volumes, [volume(1), volume(2)]);
        assert_eq!(entry.manifest_hash, hash_of(b"manifest"));
        assert_eq!(
            entry.items[1].source,
            Some(ItemSource {
                op: OpId(3),
                index: TargetIndex(5)
            })
        );
        assert!(entry.items[1].released && !entry.items[0].released);
        assert!(JournalEntry::from_parts(&[]).is_none());
    }

    #[test]
    fn view_finds_items_of_parts_stored_out_of_index_order() {
        let first = manifest(7, vec![record(1, "/home/user/b")]);
        let mut second = first.clone();
        second.targets = vec![record(0, "/home/user/a")];
        let committed = || StateRecord::new(EntryState::Committed);
        let parts = vec![
            (first, committed(), Items { items: Vec::new() }),
            (second, committed(), Items { items: Vec::new() }),
        ];
        let entry = JournalEntry::from_parts(&parts).unwrap();
        let view = super::View::from_entries(vec![entry.clone()]);
        let path_of = |index: u32| {
            view.item(&entry, TargetIndex(index))
                .map(|item| item.target.path.clone())
        };
        assert_eq!(path_of(0), Some(fs_path("/home/user/a")));
        assert_eq!(path_of(1), Some(fs_path("/home/user/b")));
        assert_eq!(path_of(2), None);
    }

    #[test]
    fn apply_source_updates_completes_an_interrupted_update_idempotently() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        let mut undo_entry = manifest(7, vec![record(0, "/home/user/a")]);
        undo_entry.items = vec![ItemRef {
            op: OpId(1),
            index: TargetIndex(0),
            path: fs_path("/home/user/a"),
        }];
        apply_source_updates(&journal, &undo_entry).unwrap();
        let once = journal.entry(OpId(1));
        apply_source_updates(&journal, &undo_entry).unwrap();
        assert_eq!(journal.entry(OpId(1)), once);
        assert_eq!(once.state, EntryState::Partial);
        assert_eq!(once.items[0].undone_by, Some(OpId(7)));
        assert_eq!(once.items[1].undone_by, None);
        undo_entry.items.push(ItemRef {
            op: OpId(1),
            index: TargetIndex(1),
            path: fs_path("/home/user/b"),
        });
        undo_entry.scope = None;
        apply_source_updates(&journal, &undo_entry).unwrap();
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Undone);
    }
}

#[cfg(test)]
mod journal_ctx_tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use nurt_contracts::config::OverwriteStrategy;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::fs::{
        AbsPath, ContentHash, DirEntry, ExtendedMeta, FileLock, FileMeta, Follow, Fs, FsFeatures,
        FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport, PathKey, ProviderKey,
        ProviderScheme, ProviderValue, ReadHandle, RemoveStats, RenameMode, ScopePurpose,
        SpaceInfo, TxScope, VolumeId, VolumeInfo, WriteHandle,
    };
    use nurt_contracts::journal::{
        BeginRequest, EntryState, Expect, ExpiryReason, FileOp, HistoryQuery, Mutation, OnConflict,
        OpId, OpKind, OpState, PinRequest, RedoRequest, RollbackReason, TargetAction, TargetPath,
        TargetSpec, UndoRequest, UndoScope, UndoSelector, UndoneCause, apply_ops,
    };
    use nurt_fs::clock::VirtualClock;
    use nurt_fs::failpoint::NoFailpoints;
    use nurt_fs::mem::{MemFs, VolumeSpec};

    use super::testkit::program;
    use super::{apply_source_updates, redo, rollback_committed, undo};
    use crate::history::{details, history};
    use crate::layout::{JournalLayout, StateLayout};
    use crate::manifest;
    use crate::pin::{PinSet, pin};
    use crate::retention::ctx::CtxStore;
    use crate::retention::tomb::{self, Tombstone};
    use crate::store::test_support::{
        MemProvider, TestAdmission, TestJournals, TestLimits, default_retention,
    };
    use crate::store::{self, JournalCtx, JournalLocation, Settings};

    /// Mount point of the single test volume.
    const MOUNT: &str = "/a";
    /// Journal directory of the test volume.
    const JOURNAL: &str = "/a/.nurt-journal";

    fn abs(text: &str) -> AbsPath {
        AbsPath::new(text).unwrap()
    }

    fn fs_path(text: &str) -> TargetPath {
        TargetPath::Fs(abs(text))
    }

    /// One volume `/a` with its journal and the account state in `/a/state`.
    struct World {
        fs: Arc<MemFs>,
        ctx: JournalCtx,
        registry: Arc<MemProvider>,
    }

    impl World {
        fn new() -> Self {
            let features = FsFeatures {
                exchange: true,
                rename_noreplace: true,
                replace_with_backup: false,
                hardlink: true,
                reflink: false,
                xattr: true,
                acl: false,
                alternate_streams: false,
                durable_metadata: true,
                case_sensitive: true,
                decomposes_names: false,
            };
            let fs = Arc::new(
                MemFs::builder()
                    .volume(VolumeSpec::posix(abs(MOUNT)).with_features(features))
                    .build()
                    .unwrap(),
            );
            let journal = JournalLayout::new(abs(JOURNAL));
            for dir in journal.structure().unwrap() {
                fs.seed_dir(&dir).unwrap();
            }
            fs.seed_dir(&abs("/a/state")).unwrap();
            let dev = fs.volume(&abs(MOUNT)).unwrap().id.dev;
            let locations = HashMap::from([(dev, JournalLocation::Active(abs(JOURNAL)))]);
            let registry = Arc::new(MemProvider::new("registry"));
            let ctx = JournalCtx {
                fs: fs.clone(),
                providers: vec![registry.clone()],
                admission: Arc::new(TestAdmission::default()),
                limits: Arc::new(TestLimits(Mutex::new(default_retention()))),
                clock: Arc::new(VirtualClock::new(VirtualClock::DEFAULT_START)),
                journals: Arc::new(TestJournals(Mutex::new(locations))),
                state: StateLayout::new(abs("/a/state")),
                failpoints: Arc::new(NoFailpoints),
                settings: Settings {
                    overwrite: OverwriteStrategy::Exchange,
                    lock_timeout: Duration::from_millis(50),
                    reference_schemes: Vec::new(),
                },
                space: None,
                startup: OnceLock::new(),
            };
            Self { fs, ctx, registry }
        }

        fn read(&self, path: &str) -> Option<Vec<u8>> {
            manifest::read_bytes(&*self.fs, &abs(path)).ok()
        }

        /// Overwrites a file outside the journal, as another program would.
        fn overwrite(&self, path: &str, content: &[u8]) {
            let scope = store::scope(OpId(0), ScopePurpose::Operation);
            manifest::write_atomic(&*self.fs, &scope, &abs(MOUNT), &abs(path), content).unwrap();
        }

        /// Runs one journaled operation.
        fn run(&self, ops: Vec<FileOp>) -> OpId {
            let mut tx = crate::tx::begin(&self.ctx, begin("write"));
            apply_ops(tx.as_mut(), ops).unwrap();
            tx.commit().unwrap().op.unwrap()
        }

        fn state(&self, op: OpId) -> manifest::StateRecord {
            let entry = JournalLayout::new(abs(JOURNAL)).entry(op).unwrap();
            manifest::read_state(&*self.fs, &entry.state().unwrap()).unwrap()
        }

        fn manifest(&self, op: OpId) -> manifest::Manifest {
            let entry = JournalLayout::new(abs(JOURNAL)).entry(op).unwrap();
            manifest::read_manifest(&*self.fs, &entry.manifest().unwrap()).unwrap()
        }

        fn mode(&self, path: &str) -> u32 {
            self.fs.metadata(&abs(path), Follow::NoFollow).unwrap().mode
        }

        /// The same world with the journal store reading through a
        /// [`ReadCounter`].
        fn with_read_counter() -> (Self, Arc<ReadCounter>) {
            let mut world = Self::new();
            let counter = Arc::new(ReadCounter {
                inner: world.fs.clone(),
                reads: Mutex::default(),
            });
            world.ctx.fs = counter.clone();
            (world, counter)
        }
    }

    /// File system that forwards every call to a [`MemFs`] and counts the
    /// files opened for reading, per path.
    struct ReadCounter {
        inner: Arc<MemFs>,
        reads: Mutex<HashMap<AbsPath, usize>>,
    }

    impl ReadCounter {
        fn reads_of(&self, path: &AbsPath) -> usize {
            self.reads
                .lock()
                .unwrap()
                .get(path)
                .copied()
                .unwrap_or_default()
        }

        fn reset(&self) {
            self.reads.lock().unwrap().clear();
        }
    }

    impl Fs for ReadCounter {
        fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
            self.inner.metadata(path, follow)
        }

        fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
            self.inner.extended_metadata(path, follow)
        }

        fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
            *self.reads.lock().unwrap().entry(path.clone()).or_default() += 1;
            self.inner.open_read(path)
        }

        fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
            self.inner.open_stream_read(path, stream)
        }

        fn read_dir<'a>(
            &'a self,
            path: &AbsPath,
        ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
            self.inner.read_dir(path)
        }

        fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
            self.inner.read_link(path)
        }

        fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
            self.inner.real_path(path)
        }

        fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
            self.inner.content_hash(path)
        }

        fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
            self.inner.volume(path)
        }

        fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
            self.inner.space(volume)
        }

        fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
            self.inner.path_key(path)
        }

        fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
            self.inner.holders(path)
        }

        fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
            self.inner.can_write_dir(dir)
        }
    }

    impl FsWrite for ReadCounter {
        fn create_temp_in(
            &self,
            scope: &TxScope,
            dir: &AbsPath,
            name_hint: &str,
        ) -> FsResult<Box<dyn WriteHandle>> {
            self.inner.create_temp_in(scope, dir, name_hint)
        }

        fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
            self.inner.create_new(scope, path)
        }

        fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
            self.inner.open_in_place(scope, path)
        }

        fn open_stream_write(
            &self,
            scope: &TxScope,
            path: &AbsPath,
            stream: &str,
        ) -> FsResult<Box<dyn WriteHandle>> {
            self.inner.open_stream_write(scope, path, stream)
        }

        fn rename(
            &self,
            scope: &TxScope,
            from: &AbsPath,
            to: &AbsPath,
            mode: RenameMode,
        ) -> FsResult<()> {
            self.inner.rename(scope, from, to, mode)
        }

        fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
            self.inner.exchange(scope, a, b)
        }

        fn replace_with_backup(
            &self,
            scope: &TxScope,
            target: &AbsPath,
            replacement: &AbsPath,
            backup: &AbsPath,
        ) -> FsResult<()> {
            self.inner
                .replace_with_backup(scope, target, replacement, backup)
        }

        fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
            self.inner.hard_link(scope, existing, new)
        }

        fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
            self.inner.clone_file(scope, src, dst)
        }

        fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
            self.inner.copy_file(scope, src, dst)
        }

        fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            self.inner.remove_file(scope, path)
        }

        fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            self.inner.remove_dir(scope, path)
        }

        fn remove_tree(
            &self,
            scope: &TxScope,
            path: &AbsPath,
            parallelism: usize,
        ) -> FsResult<RemoveStats> {
            self.inner.remove_tree(scope, path, parallelism)
        }

        fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
            self.inner.create_dir(scope, path, mode)
        }

        fn create_link(
            &self,
            scope: &TxScope,
            target: &Path,
            link: &AbsPath,
            kind: LinkKind,
        ) -> FsResult<()> {
            self.inner.create_link(scope, target, link, kind)
        }

        fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
            self.inner.set_meta(scope, path, change)
        }

        fn restore_meta(
            &self,
            scope: &TxScope,
            path: &AbsPath,
            meta: &ExtendedMeta,
        ) -> FsResult<MetaTransferReport> {
            self.inner.restore_meta(scope, path, meta)
        }

        fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
            self.inner.truncate(scope, path, length)
        }

        fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
            self.inner.sync_dir(scope, dir)
        }

        fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
            self.inner.lock_exclusive(path, timeout)
        }

        fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            self.inner.mark_hidden(scope, path)
        }
    }

    /// Numbers of files removed by the two bulk operations whose undo is
    /// compared.
    const SMALL_BULK: usize = 4;
    const LARGE_BULK: usize = 64;
    /// Upper bound of the reads of the source manifest by one undo.
    const MAX_MANIFEST_READS: usize = 8;

    /// Removes `count` files in one operation, undoes it and returns how many
    /// times the undo opened the manifest of the removed entry.
    fn manifest_reads_of_bulk_undo(count: usize) -> usize {
        let (world, counter) = World::with_read_counter();
        let paths: Vec<AbsPath> = (0..count).map(|n| abs(&format!("/a/f{n}"))).collect();
        for path in &paths {
            world.fs.seed_file(path, b"kept").unwrap();
        }
        let op = world.run(paths.iter().cloned().map(FileOp::remove).collect());
        let source = JournalLayout::new(abs(JOURNAL))
            .entry(op)
            .unwrap()
            .manifest()
            .unwrap();
        counter.reset();
        let outcome = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        assert_eq!(outcome.restored.len(), count);
        for path in &paths {
            assert_eq!(manifest::read_bytes(&*world.fs, path).unwrap(), b"kept");
        }
        counter.reads_of(&source)
    }

    #[test]
    fn bulk_undo_reads_the_source_manifest_a_constant_number_of_times() {
        let small = manifest_reads_of_bulk_undo(SMALL_BULK);
        let large = manifest_reads_of_bulk_undo(LARGE_BULK);
        assert_eq!(small, large);
        assert!(
            (1..=MAX_MANIFEST_READS).contains(&large),
            "{large} reads of the source manifest"
        );
    }

    /// Creates `link` as a hard link of `existing` in one operation.
    fn link_operation(world: &World, existing: &str, link: &str) -> OpId {
        let mut tx = crate::tx::begin(&world.ctx, begin("link"));
        let spec = TargetSpec {
            path: fs_path(link),
            action: TargetAction::Link,
            expect: Expect::Absent,
            size_hint: None,
        };
        let preserved = tx.preserve(&[spec]).unwrap();
        tx.apply(
            preserved[0].index,
            Mutation::CreateHardLink {
                existing: abs(existing),
            },
        )
        .unwrap();
        tx.commit().unwrap().op.unwrap()
    }

    fn meta(world: &World, path: &str) -> nurt_contracts::fs::FileMeta {
        world.fs.metadata(&abs(path), Follow::NoFollow).unwrap()
    }

    #[test]
    fn undo_of_a_rename_over_a_file_restores_both_names_and_redo_repeats_it() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/src"), b"source").unwrap();
        world.fs.seed_file(&abs("/a/dst"), b"displaced").unwrap();
        let source_meta = meta(&world, "/a/src");
        let op = world.run(vec![FileOp::rename(abs("/a/src"), abs("/a/dst"), true)]);
        assert_eq!(world.read("/a/src"), None);
        assert_eq!(world.read("/a/dst").as_deref(), Some(&b"source"[..]));
        let undone = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        assert_eq!(world.read("/a/src").as_deref(), Some(&b"source"[..]));
        assert_eq!(world.read("/a/dst").as_deref(), Some(&b"displaced"[..]));
        assert_eq!(meta(&world, "/a/src").mode, source_meta.mode);
        assert_eq!(meta(&world, "/a/src").links, 1);
        assert_eq!(world.state(op).state, EntryState::Undone);
        redo(&world.ctx, &redo_latest()).unwrap();
        assert_eq!(world.read("/a/src"), None);
        assert_eq!(world.read("/a/dst").as_deref(), Some(&b"source"[..]));
        assert_eq!(world.state(undone.entry).state, EntryState::Undone);
        undo(&world.ctx, &select(UndoSelector::Last)).unwrap();
        assert_eq!(world.read("/a/src").as_deref(), Some(&b"source"[..]));
        assert_eq!(world.read("/a/dst").as_deref(), Some(&b"displaced"[..]));
    }

    #[test]
    fn undo_of_a_rename_over_a_file_reports_a_changed_destination_and_force_restores_both() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/src"), b"source").unwrap();
        world.fs.seed_file(&abs("/a/dst"), b"displaced").unwrap();
        let op = world.run(vec![FileOp::rename(abs("/a/src"), abs("/a/dst"), true)]);
        world.overwrite("/a/dst", b"edited elsewhere");
        let error = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalConflict);
        assert_eq!(world.read("/a/src"), None);
        assert_eq!(
            world.read("/a/dst").as_deref(),
            Some(&b"edited elsewhere"[..])
        );
        let forced = UndoRequest {
            on_conflict: OnConflict::Force,
            ..select(UndoSelector::Op(op))
        };
        undo(&world.ctx, &forced).unwrap();
        assert_eq!(
            world.read("/a/src").as_deref(),
            Some(&b"edited elsewhere"[..])
        );
        assert_eq!(world.read("/a/dst").as_deref(), Some(&b"displaced"[..]));
    }

    #[test]
    fn undo_of_a_rename_over_a_file_is_one_entry_of_two_targets() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/src"), b"source").unwrap();
        world.fs.seed_file(&abs("/a/dst"), b"displaced").unwrap();
        let op = world.run(vec![FileOp::rename(abs("/a/src"), abs("/a/dst"), true)]);
        let outcome = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        let entry = world.manifest(outcome.entry);
        assert_eq!(entry.undo_of, [op]);
        assert_eq!(entry.items.len(), 1);
        assert_eq!(entry.targets.len(), 2);
    }

    #[test]
    fn redo_of_a_removed_hard_link_recreates_a_link_to_the_same_file() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/original"), b"shared").unwrap();
        let op = link_operation(&world, "/a/original", "/a/link");
        assert_eq!(meta(&world, "/a/link").links, 2);
        undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        assert_eq!(world.read("/a/link"), None);
        redo(&world.ctx, &redo_latest()).unwrap();
        let (link, original) = (meta(&world, "/a/link"), meta(&world, "/a/original"));
        assert_eq!(link.file_id, original.file_id);
        assert!(link.links >= 2, "{} links", link.links);
        assert_eq!(world.read("/a/link").as_deref(), Some(&b"shared"[..]));
    }

    fn begin(command: &str) -> BeginRequest {
        BeginRequest {
            kind: OpKind::command(command),
            session: None,
            program: program(),
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            preserve_budget: None,
        }
    }

    fn select(selector: UndoSelector) -> UndoRequest {
        UndoRequest {
            selector,
            on_conflict: OnConflict::Report,
            session: None,
            program: program(),
        }
    }

    fn redo_latest() -> RedoRequest {
        RedoRequest {
            undo_entry: None,
            session: None,
            program: program(),
        }
    }

    fn registry_key() -> ProviderKey {
        ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["HKCU".to_owned(), "Environment".to_owned()],
            value: Some("PATH".to_owned()),
        }
    }

    /// Overwrites `/a/f`, removes `/a/g` and creates `/a/h` in one operation.
    fn mixed_operation(world: &World) -> OpId {
        world.fs.seed_file(&abs("/a/f"), b"old").unwrap();
        world.fs.seed_file(&abs("/a/g"), b"gone").unwrap();
        world.run(vec![
            FileOp::write(abs("/a/f"), b"new".to_vec()),
            FileOp::remove(abs("/a/g")),
            FileOp::create(abs("/a/h"), b"made".to_vec()),
        ])
    }

    #[test]
    fn undo_redo_on_the_journal_store_reapplies_the_undone_operation() {
        let world = World::new();
        let op = mixed_operation(&world);
        let outcome = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        let redone = redo(&world.ctx, &redo_latest()).unwrap();
        assert_eq!(redone.sources, [outcome.entry]);
        assert_eq!(world.read("/a/f").as_deref(), Some(&b"new"[..]));
        assert_eq!(world.read("/a/g"), None);
        assert_eq!(world.read("/a/h").as_deref(), Some(&b"made"[..]));
        assert_eq!(world.state(outcome.entry).state, EntryState::Undone);
        assert_eq!(world.manifest(redone.entry).redo_of, Some(outcome.entry));
    }

    #[test]
    fn undo_on_the_journal_store_restores_files_and_records_the_undo_entry() {
        let world = World::new();
        let op = mixed_operation(&world);
        let outcome = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        assert_eq!(outcome.sources, [op]);
        assert_eq!(outcome.restored.len(), 3);
        assert_eq!(world.read("/a/f").as_deref(), Some(&b"old"[..]));
        assert_eq!(world.read("/a/g").as_deref(), Some(&b"gone"[..]));
        assert_eq!(world.read("/a/h"), None);
        assert_eq!(world.state(op).state, EntryState::Undone);
        let undo_entry = world.manifest(outcome.entry);
        assert_eq!(undo_entry.kind, OpKind::undo());
        assert_eq!(undo_entry.undo_of, [op]);
        assert_eq!(undo_entry.scope, Some(UndoScope::Op));
        assert_eq!(undo_entry.items.len(), 3);
        let records = history(&world.ctx, &HistoryQuery::default()).unwrap();
        assert_eq!(
            records.iter().map(|r| r.id).collect::<Vec<_>>(),
            [outcome.entry.0, op.0]
        );
        assert_eq!(records[0].undo_of, [op.0]);
        assert_eq!(records[1].state, OpState::Undone);
    }

    #[test]
    fn undo_on_the_journal_store_reports_a_conflict_and_writes_nothing() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), b"old").unwrap();
        let op = world.run(vec![FileOp::write(abs("/a/f"), b"new".to_vec())]);
        world.overwrite("/a/f", b"edited elsewhere");
        let before = world.fs.snapshot(&abs(MOUNT)).unwrap();
        let error = undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalConflict);
        let after = world.fs.snapshot(&abs(MOUNT)).unwrap();
        assert!(before.diff(&after).is_empty(), "{:?}", before.diff(&after));
        let forced = undo(
            &world.ctx,
            &UndoRequest {
                on_conflict: OnConflict::Force,
                ..select(UndoSelector::Op(op))
            },
        )
        .unwrap();
        assert_eq!(forced.forced.len(), 1);
        assert_eq!(world.read("/a/f").as_deref(), Some(&b"old"[..]));
    }

    #[test]
    fn undo_paths_on_the_journal_store_marks_items_for_details() {
        let world = World::new();
        let op = world.run(vec![
            FileOp::create(abs("/a/x"), b"x".to_vec()),
            FileOp::create(abs("/a/y"), b"y".to_vec()),
        ]);
        let first = undo(
            &world.ctx,
            &select(UndoSelector::Paths {
                op,
                paths: vec![fs_path("/a/x")],
            }),
        )
        .unwrap();
        assert_eq!(first.scope, UndoScope::Paths);
        assert_eq!(world.read("/a/x"), None);
        assert_eq!(world.read("/a/y").as_deref(), Some(&b"y"[..]));
        let found = details(&world.ctx, op).unwrap();
        assert_eq!(found.record.state, OpState::Partial);
        assert_eq!(
            found
                .items
                .iter()
                .map(|item| (item.path.clone(), item.undone))
                .collect::<Vec<_>>(),
            [(fs_path("/a/x"), true), (fs_path("/a/y"), false)]
        );
        let undo_manifest = world.manifest(first.entry);
        apply_source_updates(&world.ctx, &undo_manifest).unwrap();
        assert_eq!(details(&world.ctx, op).unwrap(), found);
        undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        assert_eq!(world.read("/a/y"), None);
        assert_eq!(world.state(op).state, EntryState::Undone);
    }

    #[test]
    fn undo_on_the_journal_store_restores_a_tree_metadata_and_a_value() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/d/x"), b"x").unwrap();
        world.fs.seed_file(&abs("/a/d/sub/y"), b"y").unwrap();
        world.fs.seed_file(&abs("/a/m"), b"m").unwrap();
        let mode = world.mode("/a/m");
        world
            .registry
            .values
            .lock()
            .unwrap()
            .insert(registry_key(), ProviderValue::Text("old".to_owned()));
        let value = FileOp {
            target: TargetSpec {
                path: TargetPath::Provider(registry_key()),
                action: TargetAction::Modify,
                expect: Expect::Unchecked,
                size_hint: None,
            },
            mutation: Mutation::ProviderSet(ProviderValue::Text("new".to_owned())),
        };
        let chmod = MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        };
        let op = world.run(vec![
            FileOp::remove(abs("/a/d")),
            FileOp::set_meta(abs("/a/m"), chmod),
            value,
        ]);
        assert_eq!(world.read("/a/d/x"), None);
        assert_eq!(world.mode("/a/m"), 0o600);
        undo(&world.ctx, &select(UndoSelector::Op(op))).unwrap();
        assert_eq!(world.read("/a/d/x").as_deref(), Some(&b"x"[..]));
        assert_eq!(world.read("/a/d/sub/y").as_deref(), Some(&b"y"[..]));
        assert_eq!(world.mode("/a/m"), mode);
        assert_eq!(
            world.registry.values.lock().unwrap().get(&registry_key()),
            Some(&ProviderValue::Text("old".to_owned()))
        );
    }

    #[test]
    fn undo_rollback_on_the_journal_store_records_the_failed_check() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), b"old").unwrap();
        let op = world.run(vec![FileOp::write(abs("/a/f"), b"new".to_vec())]);
        let outcome = rollback_committed(&world.ctx, op, RollbackReason::VerifyFailed).unwrap();
        assert_eq!(world.read("/a/f").as_deref(), Some(&b"old"[..]));
        assert_eq!(
            world.state(op),
            manifest::StateRecord::undone(UndoneCause::VerifyFailed)
        );
        assert_eq!(world.manifest(outcome.entry).undo_of, [op]);
        assert_eq!(
            undo(&world.ctx, &select(UndoSelector::Last))
                .unwrap_err()
                .code,
            ErrorCode::JournalNoop
        );
    }

    #[test]
    fn pin_on_the_journal_store_keeps_pins_outside_the_manifest() {
        let world = World::new();
        let op = world.run(vec![FileOp::create(abs("/a/x"), b"x".to_vec())]);
        let sealed = history(&world.ctx, &HistoryQuery::default()).unwrap()[0]
            .manifest_sha256
            .clone();
        let request = PinRequest {
            ids: vec![op, OpId(9)],
            release: false,
            until: None,
        };
        let outcome = pin(&world.ctx, &request).unwrap();
        assert_eq!(outcome.changed, [op]);
        assert_eq!(outcome.not_available, [OpId(9)]);
        let file = manifest::read_bytes(
            &*world.fs,
            &JournalLayout::new(abs(JOURNAL)).pins().unwrap(),
        )
        .unwrap();
        let now = nurt_contracts::env::Clock::now(&*world.ctx.clock);
        assert!(PinSet::parse(&file).unwrap().is_pinned(op, now));
        let record = history(&world.ctx, &HistoryQuery::default()).unwrap()[0].clone();
        assert!(record.pinned);
        assert_eq!(record.manifest_sha256, sealed);
        let released = pin(
            &world.ctx,
            &PinRequest {
                release: true,
                ..request
            },
        )
        .unwrap();
        assert_eq!(released.changed, [op]);
        assert!(!history(&world.ctx, &HistoryQuery::default()).unwrap()[0].pinned);
    }

    #[test]
    fn history_on_the_journal_store_lists_tombstones_as_expired() {
        let world = World::new();
        let op = world.run(vec![FileOp::create(abs("/a/x"), b"x".to_vec())]);
        let tombstone = Tombstone {
            op: OpId(op.0 + 1),
            time: nurt_contracts::env::Clock::now(&*world.ctx.clock),
            kind: OpKind::command("remove"),
            files: 3,
            bytes: 10,
            irreversible: Vec::new(),
            state: EntryState::Expired,
            reason: ExpiryReason::Ops,
        };
        tomb::write(&CtxStore(&world.ctx), &abs(MOUNT), &tombstone).unwrap();
        let records = history(&world.ctx, &HistoryQuery::default()).unwrap();
        assert_eq!(
            records.iter().map(|r| r.id).collect::<Vec<_>>(),
            [op.0 + 1, op.0]
        );
        assert_eq!(records[0].state, OpState::Expired);
        assert_eq!(records[0].files, 3);
        let error = undo(&world.ctx, &select(UndoSelector::Op(OpId(op.0 + 1)))).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalExpired);
    }
}

#[cfg(test)]
mod copy_children_tests {
    use nurt_contracts::journal::OpId;

    use super::copy_children;
    use crate::store::test_support::{Fixture, unflushed};

    #[test]
    fn restored_directory_content_is_durable() {
        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        let from = fixture.fs.path("vol-a/preserved");
        let to = fixture.fs.path("vol-a/restored");
        std::fs::create_dir_all(from.as_path().join("sub")).unwrap();
        std::fs::write(from.as_path().join("x"), b"x").unwrap();
        std::fs::write(from.as_path().join("sub/y"), b"y").unwrap();
        std::fs::create_dir_all(to.as_path()).unwrap();
        fixture.fs.clear_log();
        copy_children(&ctx, OpId(1), &from, &to).unwrap();
        let log = fixture.fs.log();
        let pending = unflushed(&log, log.len());
        assert!(pending.is_empty(), "{pending:#?}\n{log:#?}");
        assert_eq!(std::fs::read(to.as_path().join("sub/y")).unwrap(), b"y");
    }
}
