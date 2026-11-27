// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal entries as seen by `history`, `undo`, `redo` and `journal.pin`.
//!
//! [`JournalEntry`] joins the immutable manifest of an entry with its mutable
//! state (`STATE`, the item state file `ITEMS`) or, for an expired entry, with
//! its tombstone. Records built here carry no expiry time and nothing from
//! which one can be derived; retention state is reported only by
//! `journal.status`.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, ContentHash, Follow, FsErrorKind, VolumeId};
use nurt_contracts::journal::{
    EntryState, HistoryQuery, ItemDetail, OpId, OpKind, OpState, OperationDetails, OperationRecord,
    SessionId, TargetAction, TargetIndex, TargetOutcome, TargetPath, UndoScope,
};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::value::{Instant, Size};
use sha2::{Digest, Sha256};

use crate::conflict::{self, ItemKey, TargetProbe};
use crate::layout::{self, EntryLayout};
use crate::manifest::{
    self, AfterRecord, ItemState, Items, Manifest, StateRecord, TargetRecord, TargetRole,
};
use crate::pin::{self, PinSet};
use crate::retention::ctx::{Commits, CtxStore};
use crate::retention::tomb;
use crate::store::JournalCtx;

/// Number of entries listed when the query sets no limit.
pub(crate) const DEFAULT_LIMIT: u32 = 20;

/// Manifest hash of an entry whose manifest is not sealed yet. Such entries
/// are in a transient state and are never listed.
const UNSEALED: ContentHash = ContentHash([0; 32]);

/// Item restored by an item of an undo or redo entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ItemSource {
    /// Entry that recorded the restored item.
    pub(crate) op: OpId,
    /// Index of the restored item in that entry.
    pub(crate) index: TargetIndex,
}

/// One item of an entry together with its state from the item state file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EntryItem {
    /// Target as recorded in the manifest.
    pub(crate) target: TargetOutcome,
    /// Role of the record for renames across volumes.
    pub(crate) role: TargetRole,
    /// Item restored by this item; set for every item of undo and redo
    /// entries.
    pub(crate) source: Option<ItemSource>,
    /// Undo entry that undid this item individually.
    pub(crate) undone_by: Option<OpId>,
    /// The preserved content was released to free disk space.
    pub(crate) released: bool,
}

impl EntryItem {
    /// Whether the previous state of the item is kept and not yet restored.
    pub(crate) fn is_restorable(&self) -> bool {
        !self.released
            && self.undone_by.is_none()
            && self.target.mode.is_preserved()
            && self.target.after.is_some()
    }

    /// Whether the change recorded by the item is still in effect: the item
    /// was not undone individually. A released item still changed its target.
    pub(crate) fn is_in_effect(&self) -> bool {
        self.undone_by.is_none()
    }
}

/// An entry of the journal, or the tombstone of an expired entry.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JournalEntry {
    /// Number.
    pub(crate) op: OpId,
    /// Commit time.
    pub(crate) time: WallTime,
    /// Kind.
    pub(crate) kind: OpKind,
    /// Session of the request that created the entry.
    pub(crate) session: Option<SessionId>,
    /// Account that made the operation; `None` for entries written before the
    /// account was recorded.
    pub(crate) account: Option<String>,
    /// Persistent state.
    pub(crate) state: EntryState,
    /// Bulk operation.
    pub(crate) bulk: bool,
    /// Volumes holding a part of the entry.
    pub(crate) volumes: Vec<VolumeId>,
    /// Items in declaration order; empty for tombstones and for entries
    /// whose manifest cannot be read.
    pub(crate) items: Vec<EntryItem>,
    /// Number of items, also known for tombstones.
    pub(crate) files: u64,
    /// Logical bytes changed, also known for tombstones.
    pub(crate) bytes: u64,
    /// Irreversible markers of the operation.
    pub(crate) irreversible: Vec<IrreversibleEntry>,
    /// Entries undone by this entry.
    pub(crate) undo_of: Vec<OpId>,
    /// Undo entry redone by this entry.
    pub(crate) redo_of: Option<OpId>,
    /// Scope of an undo entry.
    pub(crate) scope: Option<UndoScope>,
    /// Hash of the canonical bound program.
    pub(crate) ast_hash: ContentHash,
    /// Hash of the manifest.
    pub(crate) manifest_hash: ContentHash,
    /// Manifest hash of the previous entry of the account.
    pub(crate) chain: Option<ContentHash>,
}

impl JournalEntry {
    /// Whether the entry can be undone, completely or for its remaining
    /// items.
    pub(crate) fn is_available(&self) -> bool {
        matches!(self.state, EntryState::Committed | EntryState::Partial)
    }

    /// Whether the entry no longer holds its preserved data.
    pub(crate) fn is_expired(&self) -> bool {
        matches!(self.state, EntryState::Expiring | EntryState::Expired)
    }

    /// Whether the entry records an undo.
    pub(crate) fn is_undo(&self) -> bool {
        self.kind == OpKind::undo()
    }

    /// Whether another account than `account` made the entry. The journal
    /// directory of a volume is shared by all accounts; entries that record no
    /// account belong to every account.
    pub(crate) fn is_foreign_to(&self, account: &str) -> bool {
        self.account
            .as_deref()
            .is_some_and(|recorded| recorded != account)
    }

    /// Whether the entry belongs to the session; without a session every
    /// entry of the account belongs to the caller.
    pub(crate) fn in_session(&self, session: Option<&SessionId>) -> bool {
        session.is_none_or(|session| self.session.as_ref() == Some(session))
    }

    fn has_deletion(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.target.action, TargetAction::Delete))
    }
}

impl JournalEntry {
    /// Entry built from the parts read from its volumes: the manifest, the
    /// state file and the item state file of every part, ordered by volume.
    ///
    /// Item `n` of an undo or redo entry restores `items[n]` of its manifest.
    /// When the states of the parts disagree (an update interrupted by a
    /// crash) the most advanced state is taken; recovery makes the parts
    /// agree. Returns `None` without parts.
    pub(crate) fn from_parts(parts: &[(Manifest, StateRecord, Items)]) -> Option<Self> {
        let (first, _, _) = parts.first()?;
        let mut items = Vec::new();
        let mut irreversible: Vec<IrreversibleEntry> = Vec::new();
        for (manifest, _, states) in parts {
            let states: HashMap<TargetIndex, &ItemState> = states
                .items
                .iter()
                .map(|state| (state.index, state))
                .collect();
            for marker in &manifest.irreversible {
                if !irreversible.contains(marker) {
                    irreversible.push(marker.clone());
                }
            }
            for record in &manifest.targets {
                if let Some(reason) = record.irreversible {
                    irreversible.push(IrreversibleEntry {
                        path: Some(conflict::display_path(&record.path)),
                        raw_path: conflict::raw_path(&record.path),
                        ..IrreversibleEntry::new(reason)
                    });
                }
                let state = states.get(&record.index);
                items.push(EntryItem {
                    target: target_outcome(record),
                    role: record.role,
                    source: usize::try_from(record.index.0)
                        .ok()
                        .and_then(|position| manifest.items.get(position))
                        .map(|item| ItemSource {
                            op: item.op,
                            index: item.index,
                        }),
                    undone_by: state.and_then(|state| state.undone_by),
                    released: state.is_some_and(|state| state.released),
                });
            }
        }
        let mut indexes: Vec<TargetIndex> = items.iter().map(|item| item.target.index).collect();
        indexes.sort_unstable();
        indexes.dedup();
        let bytes = items.iter().map(|item| changed_bytes(&item.target)).sum();
        Some(Self {
            op: first.op,
            time: first.time,
            kind: first.kind.clone(),
            session: first.session.clone(),
            account: first.account.clone(),
            state: merged_state(parts.iter().map(|(_, state, _)| state.state)),
            bulk: first.bulk,
            volumes: first.parts.iter().map(|part| part.vol.clone()).collect(),
            files: indexes.len() as u64,
            bytes,
            items,
            irreversible,
            undo_of: first.undo_of.clone(),
            redo_of: first.redo_of,
            scope: first.scope,
            ast_hash: first.ast_hash,
            manifest_hash: first.integrity.manifest_sha256.unwrap_or(UNSEALED),
            chain: first.integrity.chain,
        })
    }
}

fn target_outcome(record: &TargetRecord) -> TargetOutcome {
    TargetOutcome {
        index: record.index,
        path: record.path.clone(),
        action: record.action.clone(),
        mode: record.before.store,
        bytes_held: record.bytes_held,
        before: record.before.to_state(),
        after: record.after.as_ref().map(AfterRecord::to_state),
        irreversible: record.irreversible,
        notable: record.notable.clone(),
    }
}

/// Logical bytes changed by a target: the size of the removed entry for a
/// deletion, otherwise the size of the new state.
fn changed_bytes(target: &TargetOutcome) -> u64 {
    match &target.after {
        Some(after) if after.exists => after.size.unwrap_or(0),
        _ => target.before.size.unwrap_or(0),
    }
}

/// State of an entry from the states of its parts: the part that advanced
/// furthest decides.
fn merged_state(states: impl Iterator<Item = EntryState>) -> EntryState {
    fn rank(state: EntryState) -> u8 {
        match state {
            EntryState::Committed => 0,
            EntryState::Partial => 1,
            EntryState::Undone => 2,
            EntryState::Expiring => 3,
            EntryState::Expired => 4,
            EntryState::Prepared => 5,
            EntryState::RollingBack => 6,
            _ => 7,
        }
    }
    states
        .max_by_key(|state| rank(*state))
        .unwrap_or(EntryState::Corrupt)
}

/// Read access to the entries of the journal of an account.
pub(crate) trait EntrySource {
    /// Numbers of all entries and tombstones, newest first.
    fn entry_ids(&self) -> Result<Vec<OpId>>;

    /// Entry or tombstone with the number; `None` when the number is unknown.
    fn load(&self, op: OpId) -> Result<Option<JournalEntry>>;

    /// Pins of every volume, merged.
    fn pins(&self) -> Result<PinSet>;

    /// Current wall time.
    fn now(&self) -> WallTime;
}

/// One part of an entry as stored in the journal directory of its volume.
pub(crate) struct StoredPart {
    /// Journal directory of the volume.
    pub(crate) root: AbsPath,
    /// Directory of the entry.
    pub(crate) entry: EntryLayout,
    /// Manifest.
    pub(crate) manifest: Manifest,
    /// State file.
    pub(crate) state: StateRecord,
    /// Item state file.
    pub(crate) items: Items,
}

/// Parts of an entry found in the active journals, ordered as the manifest
/// lists its parts, and whether a part exists whose files cannot be read.
///
/// Fails with the file system error when an entry directory cannot be
/// inspected.
pub(crate) fn stored_parts(ctx: &JournalCtx, op: OpId) -> Result<(Vec<StoredPart>, bool)> {
    let fs = ctx.fs.as_ref();
    let mut parts = Vec::new();
    let mut damaged = false;
    for journal in ctx.active_journals() {
        let entry = journal.entry(op)?;
        match fs.metadata(entry.dir(), Follow::NoFollow) {
            Ok(_) => {}
            Err(error) if error.kind == FsErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        let read = manifest::read_manifest(fs, &entry.manifest()?).and_then(|found| {
            let state = manifest::read_state(fs, &entry.state()?)?;
            let items = manifest::read_items(fs, &entry.items()?)?;
            Ok((found, state, items))
        });
        match read {
            Ok((manifest, state, items)) => parts.push(StoredPart {
                root: journal.root().clone(),
                entry,
                manifest,
                state,
                items,
            }),
            Err(_) => damaged = true,
        }
    }
    let order: Vec<AbsPath> = parts
        .first()
        .map(|part| part.manifest.parts.iter().map(|p| p.dir.clone()).collect())
        .unwrap_or_default();
    parts.sort_by_key(|part| order.iter().position(|dir| dir == &part.root));
    Ok((parts, damaged))
}

/// Records of the targets of an entry by target index, with the directory
/// of their part.
pub(crate) type RecordIndex<'a> = HashMap<TargetIndex, (&'a EntryLayout, &'a TargetRecord)>;

/// Index of the records among the parts of an entry. Of several records of
/// one target (a rename across volumes) the last one that keeps preserved
/// data is taken, or the last one when none does.
pub(crate) fn record_index(parts: &[StoredPart]) -> RecordIndex<'_> {
    let mut index = RecordIndex::new();
    for part in parts {
        for record in &part.manifest.targets {
            match index.entry(record.index) {
                Entry::Occupied(mut slot) => {
                    if record.before.blob.is_some() || slot.get().1.before.blob.is_none() {
                        slot.insert((&part.entry, record));
                    }
                }
                Entry::Vacant(slot) => {
                    slot.insert((&part.entry, record));
                }
            }
        }
    }
    index
}

/// Preserved content of one target.
pub(crate) enum PreservedData {
    /// File holding the content.
    File(AbsPath),
    /// Content kept in the manifest, represented by its hash.
    Inline(ContentHash),
}

/// Preserved content of the targets `indexes` of entry `op`: the files that
/// hold it and the hashes of content kept in the manifest. Targets for which
/// the entry keeps no content are left out. The entry is read once.
///
/// Fails when the entry cannot be read or names a file outside its
/// directory.
pub(crate) fn preserved_files(
    ctx: &JournalCtx,
    op: OpId,
    indexes: &[TargetIndex],
) -> Result<Vec<(TargetIndex, PreservedData)>> {
    let (parts, _) = stored_parts(ctx, op)?;
    let records = record_index(&parts);
    let mut files = Vec::with_capacity(indexes.len());
    for index in indexes {
        let Some((entry, record)) = records.get(index) else {
            continue;
        };
        if let Some(name) = &record.before.blob {
            files.push((*index, PreservedData::File(entry.resolve(name)?)));
        } else if let Some(bytes) = &record.before.inline {
            let hash = ContentHash(Sha256::digest(bytes).into());
            files.push((*index, PreservedData::Inline(hash)));
        }
    }
    Ok(files)
}

/// Entries of the journal directories of all active volumes, their
/// tombstones and their pin files.
///
/// An entry whose manifest cannot be read is listed as corrupt from its row
/// in the account index; a damaged pin file counts as empty.
impl EntrySource for JournalCtx {
    fn entry_ids(&self) -> Result<Vec<OpId>> {
        let fs = self.fs.as_ref();
        let store = CtxStore(self);
        let mut ids = BTreeSet::new();
        for journal in self.active_journals() {
            ids.extend(layout::entries(fs, &journal)?);
            let mount = fs.volume(journal.root())?.mount;
            ids.extend(tomb::list(&store, &mount)?);
        }
        Ok(ids.into_iter().rev().collect())
    }

    fn load(&self, op: OpId) -> Result<Option<JournalEntry>> {
        let (parts, damaged) = stored_parts(self, op)?;
        if !damaged && !parts.is_empty() {
            let commits = Commits::new(self);
            if parts
                .iter()
                .all(|part| commits.rolled_back(op, &part.state))
            {
                return Ok(None);
            }
        }
        if parts.is_empty() {
            return if damaged {
                corrupt_from_index(self, op)
            } else {
                tombstone_entry(self, op)
            };
        }
        let joined: Vec<(Manifest, StateRecord, Items)> = parts
            .into_iter()
            .map(|part| (part.manifest, part.state, part.items))
            .collect();
        let account = self.state.account_id();
        Ok(JournalEntry::from_parts(&joined)
            .filter(|entry| !entry.is_foreign_to(&account))
            .map(|mut entry| {
                if damaged {
                    entry.state = EntryState::Corrupt;
                }
                entry
            }))
    }

    fn pins(&self) -> Result<PinSet> {
        let mut merged = PinSet::default();
        for journal in self.active_journals() {
            merged.merge(&pin::read_pin_file(self, &journal)?);
        }
        Ok(merged)
    }

    fn now(&self) -> WallTime {
        self.clock.now()
    }
}

/// Entry built from the tombstone of an expired entry.
fn tombstone_entry(ctx: &JournalCtx, op: OpId) -> Result<Option<JournalEntry>> {
    let store = CtxStore(ctx);
    for journal in ctx.active_journals() {
        let mount = ctx.fs.volume(journal.root())?.mount;
        if let Some(found) = tomb::read(&store, &mount, op)? {
            if found
                .account
                .as_deref()
                .is_some_and(|recorded| recorded != ctx.state.account_id())
            {
                continue;
            }
            return Ok(Some(JournalEntry {
                op,
                time: found.time,
                kind: found.kind,
                session: None,
                account: found.account,
                state: found.state,
                bulk: found.files > 1,
                volumes: Vec::new(),
                items: Vec::new(),
                files: found.files,
                bytes: found.bytes,
                irreversible: found
                    .irreversible
                    .into_iter()
                    .map(IrreversibleEntry::new)
                    .collect(),
                undo_of: Vec::new(),
                redo_of: None,
                scope: None,
                ast_hash: UNSEALED,
                manifest_hash: UNSEALED,
                chain: None,
            }));
        }
    }
    Ok(None)
}

/// Corrupt entry built from its row in the account index; `None` when the
/// index cannot be read or has no row for the entry.
fn corrupt_from_index(ctx: &JournalCtx, op: OpId) -> Result<Option<JournalEntry>> {
    let Ok(rows) = manifest::read_index(ctx.fs.as_ref(), &ctx.state.index()?) else {
        return Ok(None);
    };
    Ok(rows.into_iter().find(|row| row.op == op).map(|row| {
        let manifest_hash = row.digest().unwrap_or(UNSEALED);
        JournalEntry {
            op,
            time: row.time,
            kind: row.kind,
            session: row.session,
            account: None,
            state: EntryState::Corrupt,
            bulk: row.bulk,
            volumes: row.parts.into_iter().map(|part| part.vol).collect(),
            items: Vec::new(),
            files: 0,
            bytes: 0,
            irreversible: Vec::new(),
            undo_of: Vec::new(),
            redo_of: None,
            scope: None,
            ast_hash: UNSEALED,
            manifest_hash,
            chain: None,
        }
    }))
}

/// Lists the entries of the journal of the account, newest first.
///
/// Fails as [`history_with`].
pub(crate) fn history(ctx: &JournalCtx, query: &HistoryQuery) -> Result<Vec<OperationRecord>> {
    history_with(ctx, query)
}

/// Details of one entry of the journal of the account.
///
/// Fails as [`details_with`].
pub(crate) fn details(ctx: &JournalCtx, op: OpId) -> Result<OperationDetails> {
    details_with(ctx, op)
}

/// Lists entries, newest first, filtered by the query.
///
/// Entries in transient states and entries rolled back before their commit
/// are not listed; expiring and corrupt entries are listed as expired. Without a limit at most [`DEFAULT_LIMIT`] entries
/// are returned.
///
/// Fails with the error of the entry source.
pub(crate) fn history_with<S>(store: &S, query: &HistoryQuery) -> Result<Vec<OperationRecord>>
where
    S: EntrySource + TargetProbe + ?Sized,
{
    let limit = usize::try_from(query.limit.unwrap_or(DEFAULT_LIMIT)).unwrap_or(usize::MAX);
    let path_key = match &query.path {
        Some(path) => Some(store.item_key(&TargetPath::Fs(path.clone()))?),
        None => None,
    };
    let ids = match query.id {
        Some(id) => vec![id],
        None => store.entry_ids()?,
    };
    let pins = store.pins()?;
    let now = store.now();
    let mut records = Vec::new();
    for op in ids {
        if records.len() >= limit || query.since.is_some_and(|since| op <= since) {
            break;
        }
        if query.before.is_some_and(|before| op >= before) {
            continue;
        }
        let Some(entry) = store.load(op)? else {
            continue;
        };
        let Some(state) = OpState::from_entry(entry.state) else {
            continue;
        };
        if matches_filters(store, &entry, state, query, path_key.as_ref()) {
            records.push(record(&entry, state, pins.is_pinned(op, now)));
        }
    }
    Ok(records)
}

/// Details of one entry with its items.
///
/// Fails with `E_JOURNAL_NOOP` for an unknown entry or an entry in a
/// transient state, and with `E_JOURNAL_CORRUPT` for an entry that failed
/// its integrity check.
pub(crate) fn details_with<S>(store: &S, op: OpId) -> Result<OperationDetails>
where
    S: EntrySource + ?Sized,
{
    let entry = store
        .load(op)?
        .ok_or_else(|| conflict::noop_error(op, None))?;
    if entry.state == EntryState::Corrupt {
        return Err(conflict::corrupt_error(op));
    }
    let state = OpState::from_entry(entry.state)
        .ok_or_else(|| conflict::noop_error(op, Some(entry.state)))?;
    let pinned = store.pins()?.is_pinned(op, store.now());
    let items = entry.items.iter().map(item_detail).collect();
    Ok(OperationDetails {
        record: record(&entry, state, pinned),
        items,
    })
}

/// Record of `history` for an entry in a listed state.
pub(crate) fn record(entry: &JournalEntry, state: OpState, pinned: bool) -> OperationRecord {
    OperationRecord {
        id: entry.op.0,
        time: Instant::saturating_from_wall(entry.time),
        kind: entry.kind.command.clone(),
        action: entry.kind.action.clone(),
        summary: summary(entry),
        state,
        files: entry.files,
        bytes: Size::from_bytes(entry.bytes),
        irreversible: entry.irreversible.clone(),
        bulk: entry.bulk,
        session: entry.session.as_ref().map(|session| session.0.clone()),
        undo_of: entry.undo_of.iter().map(|op| op.0).collect(),
        redo_of: entry.redo_of.map(|op| op.0),
        undo_scope: entry.scope,
        ast_hash: entry.ast_hash.to_string(),
        manifest_sha256: entry.manifest_hash.to_string(),
        chain: entry.chain.map(|chain| chain.to_string()),
        pinned,
        items: None,
    }
}

fn matches_filters<S>(
    store: &S,
    entry: &JournalEntry,
    state: OpState,
    query: &HistoryQuery,
    path_key: Option<&ItemKey>,
) -> bool
where
    S: TargetProbe + ?Sized,
{
    if query.state.is_some_and(|wanted| wanted != state) {
        return false;
    }
    if query.deleted_only && !entry.has_deletion() {
        return false;
    }
    path_key.is_none_or(|key| touches(store, entry, key))
}

/// Whether an item of the entry addresses the key. Items whose key cannot be
/// computed (for example on a volume that is no longer mounted) do not match.
fn touches<S>(store: &S, entry: &JournalEntry, key: &ItemKey) -> bool
where
    S: TargetProbe + ?Sized,
{
    entry.items.iter().any(|item| {
        conflict::addresses(&item.target, item.role)
            .iter()
            .any(|path| store.item_key(path).is_ok_and(|found| &found == key))
    })
}

fn item_detail(item: &EntryItem) -> ItemDetail {
    ItemDetail {
        path: item.target.path.clone(),
        action: item.target.action.clone(),
        mode: item.target.mode,
        before_hash: item.target.before.hash,
        after_hash: item.target.after.as_ref().and_then(|after| after.hash),
        undone: item.undone_by.is_some(),
        released: item.released,
    }
}

/// One-line English summary of the effect of an entry.
fn summary(entry: &JournalEntry) -> String {
    let restored = count_phrase(entry.files, "item", "items");
    if entry.is_undo() {
        return format!("undo of {}: {restored} restored", op_list(&entry.undo_of));
    }
    if let Some(undo) = entry.redo_of {
        return format!("redo of {undo}: {restored} restored", undo = undo.0);
    }
    if entry.items.is_empty() {
        return match (entry.files, entry.irreversible.is_empty()) {
            (0, false) => "external effect only".to_owned(),
            (0, true) => "no changes".to_owned(),
            (files, _) => count_phrase(files, "item", "items"),
        };
    }
    action_counts(&entry.items)
}

fn action_counts(items: &[EntryItem]) -> String {
    const LABELS: [&str; 10] = [
        "created",
        "modified",
        "appended",
        "deleted",
        "replaced",
        "renamed",
        "attributes changed",
        "linked",
        "truncated",
        "written externally",
    ];
    let mut counts = [0_u64; LABELS.len()];
    for item in items {
        let slot = match item.target.action {
            TargetAction::Create | TargetAction::CreateTree => 0,
            TargetAction::Append => 2,
            TargetAction::Delete => 3,
            TargetAction::Replace => 4,
            TargetAction::Rename { .. } => 5,
            TargetAction::Attributes => 6,
            TargetAction::Link => 7,
            TargetAction::Truncate => 8,
            TargetAction::ExternalWrite => 9,
            _ => 1,
        };
        counts[slot] += 1;
    }
    let mut text = String::new();
    for (count, label) in counts.iter().zip(LABELS) {
        if *count > 0 {
            if !text.is_empty() {
                text.push_str(", ");
            }
            let _ = write!(text, "{count} {label}");
        }
    }
    text
}

fn count_phrase(count: u64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

fn op_list(ops: &[OpId]) -> String {
    let mut text = String::new();
    for (position, op) in ops.iter().enumerate() {
        if position > 0 {
            text.push_str(", ");
        }
        let _ = write!(text, "{}", op.0);
    }
    text
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::journal::{EntryState, HistoryQuery, OpId, OpState, PinRequest};

    use super::{DEFAULT_LIMIT, details_with as details, history_with as history};
    use crate::pin::pin_with as pin;
    use crate::undo::testkit::{TestJournal, abs, fs_path};

    fn query() -> HistoryQuery {
        HistoryQuery::default()
    }

    #[test]
    fn history_lists_newest_first_with_default_limit() {
        let journal = TestJournal::new();
        for n in 0..25 {
            journal
                .op(None)
                .create(&format!("/home/user/f{n}"), b"x")
                .commit();
        }
        let records = history(&journal, &query()).unwrap();
        assert_eq!(records.len(), usize::try_from(DEFAULT_LIMIT).unwrap());
        assert_eq!(records[0].id, 25);
        assert_eq!(records[19].id, 6);
    }

    #[test]
    fn history_applies_id_before_since_and_limit() {
        let journal = TestJournal::new();
        for n in 0..6 {
            journal
                .op(None)
                .create(&format!("/home/user/f{n}"), b"x")
                .commit();
        }
        let single = history(
            &journal,
            &HistoryQuery {
                id: Some(OpId(3)),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(
            single.iter().map(|r| OpId(r.id)).collect::<Vec<_>>(),
            [OpId(3)]
        );
        let range = history(
            &journal,
            &HistoryQuery {
                before: Some(OpId(5)),
                since: Some(OpId(2)),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(
            range.iter().map(|r| OpId(r.id)).collect::<Vec<_>>(),
            [OpId(4), OpId(3)]
        );
        let limited = history(
            &journal,
            &HistoryQuery {
                limit: Some(2),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(limited.len(), 2);
        let unknown = history(
            &journal,
            &HistoryQuery {
                id: Some(OpId(99)),
                ..query()
            },
        )
        .unwrap();
        assert!(unknown.is_empty());
    }

    #[test]
    fn history_filters_by_state_path_and_deletions() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"a");
        journal.op(None).write("/home/user/a", b"a2").commit();
        journal.op(None).delete("/home/user/a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        journal.expire(OpId(1));
        let expired = history(
            &journal,
            &HistoryQuery {
                state: Some(OpState::Expired),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(
            expired.iter().map(|r| OpId(r.id)).collect::<Vec<_>>(),
            [OpId(1)]
        );
        let by_path = history(
            &journal,
            &HistoryQuery {
                path: Some(abs("/home/user/a")),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(
            by_path.iter().map(|r| OpId(r.id)).collect::<Vec<_>>(),
            [OpId(2)]
        );
        let deleted = history(
            &journal,
            &HistoryQuery {
                deleted_only: true,
                ..query()
            },
        )
        .unwrap();
        assert_eq!(
            deleted.iter().map(|r| OpId(r.id)).collect::<Vec<_>>(),
            [OpId(2)]
        );
    }

    #[test]
    fn history_hides_transient_entries_and_shows_corrupt_as_expired() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        journal.op(None).create("/home/user/c", b"c").commit();
        journal.set_state(OpId(2), EntryState::Prepared);
        journal.set_state(OpId(3), EntryState::Corrupt);
        let records = history(&journal, &query()).unwrap();
        assert_eq!(
            records.iter().map(|r| OpId(r.id)).collect::<Vec<_>>(),
            [OpId(3), OpId(1)]
        );
        assert_eq!(records[0].state, OpState::Expired);
    }

    #[test]
    fn history_shows_tombstones_as_expired_with_metrics() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        journal.expire(OpId(1));
        let records = history(&journal, &query()).unwrap();
        assert_eq!(records[0].state, OpState::Expired);
        assert_eq!(records[0].files, 2);
        assert_eq!(records[0].summary, "2 items");
    }

    #[test]
    fn history_marks_pinned_entries() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        pin(
            &journal,
            &PinRequest {
                ids: vec![OpId(1)],
                release: false,
                until: None,
            },
        )
        .unwrap();
        let records = history(&journal, &query()).unwrap();
        assert!(!records[0].pinned);
        assert!(records[1].pinned);
        journal.advance(Duration::from_hours(25));
        let later = history(&journal, &query()).unwrap();
        assert!(!later[1].pinned);
    }

    #[test]
    fn history_summarizes_effects() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"a");
        journal
            .op(None)
            .write("/home/user/a", b"a2")
            .create("/home/user/b", b"b")
            .create("/home/user/c", b"c")
            .commit();
        journal.op(None).external_only().commit();
        let records = history(&journal, &query()).unwrap();
        assert_eq!(records[0].summary, "external effect only");
        assert_eq!(records[1].summary, "2 created, 1 modified");
    }

    #[test]
    fn details_reports_items_with_undone_and_released_markers() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .create("/home/user/b", b"b")
            .commit();
        journal.mark_undone(OpId(1), 0, OpId(7));
        journal.release_item(OpId(1), 1);
        let found = details(&journal, OpId(1)).unwrap();
        assert_eq!(found.record.state, OpState::Partial);
        assert_eq!(found.items.len(), 2);
        assert!(found.items[0].undone && !found.items[0].released);
        assert!(found.items[1].released && !found.items[1].undone);
        assert_eq!(found.items[0].path, fs_path("/home/user/a"));
    }

    #[test]
    fn details_rejects_unknown_transient_and_corrupt_entries() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        assert_eq!(
            details(&journal, OpId(9)).unwrap_err().code,
            ErrorCode::JournalNoop
        );
        journal.set_state(OpId(1), EntryState::RollingBack);
        assert_eq!(
            details(&journal, OpId(1)).unwrap_err().code,
            ErrorCode::JournalNoop
        );
        journal.set_state(OpId(2), EntryState::Corrupt);
        assert_eq!(
            details(&journal, OpId(2)).unwrap_err().code,
            ErrorCode::JournalCorrupt
        );
    }

    fn part(state: EntryState) -> (Manifest, StateRecord, Items) {
        (
            crate::manifest::tests::sample(),
            StateRecord::new(state),
            Items::default(),
        )
    }

    use super::JournalEntry;
    use crate::manifest::{ItemState, Items, Manifest, StateRecord};

    #[test]
    fn an_entry_without_parts_does_not_exist() {
        assert!(JournalEntry::from_parts(&[]).is_none());
    }

    #[test]
    fn an_entry_with_a_committed_and_a_prepared_part_is_in_progress_and_stays_unlisted() {
        for parts in [
            [part(EntryState::Committed), part(EntryState::Prepared)],
            [part(EntryState::Prepared), part(EntryState::Committed)],
        ] {
            let entry = JournalEntry::from_parts(&parts).unwrap();
            assert_eq!(entry.state, EntryState::Prepared);
            assert!(!entry.is_available());
            assert!(!entry.is_expired());
        }
    }

    #[test]
    fn an_entry_takes_the_most_advanced_state_of_its_parts() {
        let table = [
            (
                EntryState::Committed,
                EntryState::Partial,
                EntryState::Partial,
            ),
            (EntryState::Partial, EntryState::Undone, EntryState::Undone),
            (
                EntryState::Undone,
                EntryState::Expiring,
                EntryState::Expiring,
            ),
            (
                EntryState::Expiring,
                EntryState::Expired,
                EntryState::Expired,
            ),
            (
                EntryState::Committed,
                EntryState::Corrupt,
                EntryState::Corrupt,
            ),
            (
                EntryState::Committed,
                EntryState::Committed,
                EntryState::Committed,
            ),
        ];
        for (first, second, merged) in table {
            for parts in [[part(first), part(second)], [part(second), part(first)]] {
                let entry = JournalEntry::from_parts(&parts).unwrap();
                assert_eq!(entry.state, merged, "{first:?} {second:?}");
            }
        }
        let single = JournalEntry::from_parts(&[part(EntryState::Committed)]).unwrap();
        assert!(single.is_available());
        assert!(!single.is_expired());
        let expired = JournalEntry::from_parts(&[part(EntryState::Expiring)]).unwrap();
        assert!(!expired.is_available());
        assert!(expired.is_expired());
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // one scenario with many checks
    fn an_entry_reports_the_metrics_hash_and_items_of_its_manifest() {
        use nurt_contracts::journal::{SessionId, TargetIndex};
        let mut manifest = crate::manifest::tests::sample();
        let digest = manifest.seal().unwrap();
        let states = Items {
            items: vec![ItemState {
                index: TargetIndex(0),
                released: true,
                undone_by: Some(OpId(40)),
            }],
        };
        let entry = JournalEntry::from_parts(&[(
            manifest,
            StateRecord::new(EntryState::Committed),
            states,
        )])
        .unwrap();
        assert_eq!(entry.op, OpId(18));
        assert_eq!((entry.files, entry.bytes), (1, 1131));
        assert_eq!(entry.manifest_hash, digest);
        assert_eq!(entry.volumes.len(), 1);
        assert_eq!(entry.items.len(), 1);
        assert!(entry.items[0].released);
        assert_eq!(entry.items[0].undone_by, Some(OpId(40)));
        assert!(!entry.items[0].is_restorable());
        assert!(!entry.items[0].is_in_effect());
        assert!(entry.in_session(None));
        assert!(entry.in_session(Some(&SessionId("s-1".to_owned()))));
        assert!(!entry.in_session(Some(&SessionId("other".to_owned()))));
    }

    #[test]
    fn an_entry_reports_irreversible_targets_of_its_manifest() {
        use nurt_contracts::report::Irreversible;

        let mut manifest = crate::manifest::tests::sample();
        manifest.targets[0].irreversible = Some(Irreversible::JournalError);
        let entry = JournalEntry::from_parts(&[(
            manifest,
            StateRecord::new(EntryState::Committed),
            Items { items: Vec::new() },
        )])
        .unwrap();
        assert_eq!(entry.irreversible.len(), 1);
        assert_eq!(entry.irreversible[0].reason, Irreversible::JournalError);
        assert_eq!(
            entry.irreversible[0].path.as_deref(),
            Some("/srv/app/src/a.go")
        );
    }

    #[test]
    fn an_unsealed_manifest_has_the_zero_hash_and_untouched_items_are_restorable() {
        let entry = JournalEntry::from_parts(&[part(EntryState::Committed)]).unwrap();
        assert_eq!(entry.manifest_hash, super::UNSEALED);
        assert!(entry.items[0].is_restorable());
        assert!(entry.items[0].is_in_effect());
        assert!(!entry.is_undo());
    }

    fn ids(records: &[nurt_contracts::journal::OperationRecord]) -> Vec<u64> {
        records.iter().map(|record| record.id).collect()
    }

    #[test]
    fn history_bounds_exclude_the_named_entry_and_a_zero_limit_lists_nothing() {
        let journal = TestJournal::new();
        for n in 0..5 {
            journal
                .op(None)
                .create(&format!("/home/user/f{n}"), b"x")
                .commit();
        }
        let with = |query: HistoryQuery| ids(&history(&journal, &query).unwrap());
        assert_eq!(
            with(HistoryQuery {
                before: Some(OpId(3)),
                ..query()
            }),
            [2, 1]
        );
        assert!(
            with(HistoryQuery {
                before: Some(OpId(1)),
                ..query()
            })
            .is_empty()
        );
        assert!(
            with(HistoryQuery {
                since: Some(OpId(5)),
                ..query()
            })
            .is_empty()
        );
        assert_eq!(
            with(HistoryQuery {
                since: Some(OpId(0)),
                ..query()
            }),
            [5, 4, 3, 2, 1]
        );
        assert_eq!(
            with(HistoryQuery {
                limit: Some(1),
                ..query()
            }),
            [5]
        );
        assert!(
            with(HistoryQuery {
                limit: Some(0),
                ..query()
            })
            .is_empty()
        );
        assert_eq!(
            with(HistoryQuery {
                limit: Some(u32::MAX),
                ..query()
            })
            .len(),
            5
        );
    }

    #[test]
    fn history_of_an_empty_journal_is_empty() {
        let journal = TestJournal::new();
        assert!(history(&journal, &query()).unwrap().is_empty());
        let by_path = HistoryQuery {
            path: Some(abs("/home/user/none")),
            ..query()
        };
        assert!(history(&journal, &by_path).unwrap().is_empty());
    }

    #[test]
    fn details_of_an_expired_entry_list_no_items() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.expire(OpId(1));
        let found = details(&journal, OpId(1)).unwrap();
        assert_eq!(found.record.state, OpState::Expired);
        assert_eq!(found.record.files, 1);
        assert_eq!(found.record.summary, "1 item");
        assert!(found.items.is_empty());
    }

    #[test]
    fn history_state_filter_separates_available_and_expired_entries() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        journal.expire(OpId(1));
        let listed = |state: OpState| {
            ids(&history(
                &journal,
                &HistoryQuery {
                    state: Some(state),
                    ..query()
                },
            )
            .unwrap())
        };
        assert_eq!(listed(OpState::Available), [2]);
        assert_eq!(listed(OpState::Expired), [1]);
        assert!(listed(OpState::Partial).is_empty());
    }
}
