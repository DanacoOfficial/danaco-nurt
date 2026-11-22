// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Entries of a volume journal as seen by reclaim for space.
//!
//! These functions back [`EntryCatalog`](crate::store::EntryCatalog) of the
//! journal store. The caller takes the entry lock with [`lock_entry`] and
//! holds it across every other call for the same entry; no other function of
//! this module takes the lock.
//!
//! A whole entry is reclaimed in four steps: [`begin_reclaim`] records the
//! reason in the entry directory and writes the tombstone, the caller renames
//! `ops/<N>` to `reap/<N>` and removes it, and [`complete_reclaim`] removes
//! whatever remains and prunes the tombstones. The tombstone is written first
//! because the manifest is gone once the caller has removed the entry.
//!
//! A partial release sets the pending marker and marks the items released in
//! `ITEMS` before the caller removes their data ([`mark_released`]), then
//! clears the marker ([`finish_release`]); recovery reconciles marks and data
//! of an entry whose marker remains.

use std::collections::BTreeMap;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, FileKind, FileLock, Follow, Fs, FsErrorKind, FsResult, ScopePurpose, VolumeId,
};
use nurt_contracts::journal::{
    EntryState, ExpiredEntry, ExpiryReason, OpId, PreserveMode, TargetIndex,
};

use crate::layout::{self, EntryLayout, JournalLayout};
use crate::manifest::{self, ItemState, Items, Manifest, ReadError, StateRecord};
use crate::pin;
use crate::retention::ctx::{CtxStore, EXPIRY_FILE, reason_text};
use crate::retention::expiry::{self, Snapshot};
use crate::retention::port::RetentionStore;
use crate::retention::tomb::{self, Tombstone};
use crate::store::{EntrySummary, HeldItem, JournalCtx, ReclaimPaths, scope};

/// Permission bits of journal directories created on demand.
const DIR_MODE: u32 = 0o700;

/// Files of every entry directory besides preserved data: the manifest and
/// the state file.
const ENTRY_FILES: u64 = 2;

/// Directories of every entry directory: the entry directory itself.
const ENTRY_DIRECTORIES: u64 = 1;

/// Active journal of one volume.
struct VolumeJournal {
    layout: JournalLayout,
    mount: AbsPath,
}

/// Active journal of `volume`.
fn journal_of(ctx: &JournalCtx, volume: &VolumeId) -> Result<VolumeJournal> {
    for layout in ctx.active_journals() {
        let Ok(info) = ctx.fs.volume(layout.root()) else {
            continue;
        };
        if &info.id == volume {
            return Ok(VolumeJournal {
                layout,
                mount: info.mount,
            });
        }
    }
    Err(Error::new(
        ErrorCode::NotFound,
        "the volume has no active journal; list the journal volumes with journal.status",
    ))
}

fn exists(fs: &dyn Fs, path: &AbsPath) -> FsResult<bool> {
    match fs.metadata(path, Follow::NoFollow) {
        Ok(_) => Ok(true),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn ensure_dir(ctx: &JournalCtx, op: OpId, dir: &AbsPath) -> Result<()> {
    match ctx
        .fs
        .create_dir(&scope(op, ScopePurpose::Reclaim), dir, DIR_MODE)
    {
        Ok(()) => Ok(()),
        Err(error) if error.kind == FsErrorKind::Exists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn unknown_entry(op: OpId, mount: &AbsPath) -> Error {
    Error::new(
        ErrorCode::JournalNoop,
        format!(
            "entry {} does not exist on {mount}; list the entries with history",
            op.0
        ),
    )
}

/// Strategies whose items keep preserved data in the entry.
fn holds_data(mode: PreserveMode) -> bool {
    matches!(
        mode,
        PreserveMode::Moved
            | PreserveMode::Exchange
            | PreserveMode::ReplaceBackup
            | PreserveMode::Hardlink
            | PreserveMode::Clone
            | PreserveMode::Copy
    )
}

/// Whether item `index` no longer holds data: released for space or undone
/// individually.
fn unavailable(items: &Items, index: TargetIndex) -> bool {
    items
        .items
        .iter()
        .any(|item| item.index == index && (item.released || item.undone_by.is_some()))
}

/// Manifest of an existing entry directory.
fn entry_manifest(ctx: &JournalCtx, journal: &VolumeJournal, op: OpId) -> Result<Manifest> {
    let entry = journal.layout.entry(op)?;
    match manifest::read_manifest(ctx.fs.as_ref(), &entry.manifest()?) {
        Ok(manifest) => Ok(manifest),
        Err(ReadError::Fs(error)) if error.kind == FsErrorKind::NotFound => {
            Err(unknown_entry(op, &journal.mount))
        }
        Err(error) => Err(error.into()),
    }
}

/// Takes the lock of one entry.
///
/// # Errors
///
/// Fails with `E_IN_USE` when another holder keeps the lock beyond the lock
/// timeout, `E_NOT_FOUND` for a volume without an active journal and `E_IO`
/// when the lock directory cannot be created.
pub(crate) fn lock_entry(
    ctx: &JournalCtx,
    volume: &VolumeId,
    op: OpId,
) -> Result<Box<dyn FileLock>> {
    let journal = journal_of(ctx, volume)?;
    ensure_dir(ctx, op, &journal.layout.lock_dir()?)?;
    Ok(ctx
        .fs
        .lock_exclusive(&journal.layout.entry_lock(op)?, ctx.settings.lock_timeout)?)
}

/// Entries over the step or age limit, with the limit, as planned by the
/// next expiry pass. Pinned entries are never planned and so never overdue.
fn overdue(ctx: &JournalCtx) -> Result<BTreeMap<OpId, ExpiryReason>> {
    let store = CtxStore(ctx);
    let snapshot = Snapshot::load(&store)?;
    let limits = snapshot.limits();
    Ok(snapshot
        .plan(&limits)
        .into_iter()
        .filter(|expired| matches!(expired.reason, ExpiryReason::Ops | ExpiryReason::Time))
        .map(|expired| (expired.op, expired.reason))
        .collect())
}

/// Numbers of the entries in the reclaim area, in ascending order.
fn reaped(fs: &dyn Fs, journal: &JournalLayout) -> FsResult<Vec<OpId>> {
    let listing = match fs.read_dir(&journal.reap_dir()?) {
        Ok(listing) => listing,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut ops = Vec::new();
    for entry in listing {
        if let Some(op) = entry?.name.to_str().and_then(layout::parse_entry_name) {
            ops.push(op);
        }
    }
    ops.sort_unstable();
    Ok(ops)
}

/// Summary of one entry directory. A missing or unreadable manifest yields
/// an entry without held bytes.
fn summary(
    ctx: &JournalCtx,
    entry: &EntryLayout,
    op: OpId,
    state: EntryState,
) -> Result<EntrySummary> {
    let fs = ctx.fs.as_ref();
    let mut summary = EntrySummary {
        op,
        state,
        bytes: 0,
        files: ENTRY_FILES,
        directories: ENTRY_DIRECTORIES,
        pinned: false,
        overdue: None,
    };
    let manifest = match manifest::read_manifest(fs, &entry.manifest()?) {
        Ok(manifest) => manifest,
        Err(ReadError::Fs(error)) if error.kind != FsErrorKind::NotFound => {
            return Err(error.into());
        }
        Err(_) => return Ok(summary),
    };
    let items = manifest::read_items(fs, &entry.items()?)?;
    let mut blobs = false;
    for record in &manifest.targets {
        if record.before.meta.is_some() {
            summary.files += 1;
        }
        if !holds_data(record.before.store) || record.before.blob.is_none() {
            continue;
        }
        blobs = true;
        if record.before.kind == Some(FileKind::Directory) {
            summary.directories += 1;
        } else {
            summary.files += 1;
        }
        if !unavailable(&items, record.index) {
            summary.bytes = summary.bytes.saturating_add(record.bytes_held);
        }
    }
    if blobs {
        summary.directories += 1;
    }
    Ok(summary)
}

/// Entries of the volume journal that hold data or are being reclaimed:
/// every entry in `ops/` except expired ones, and every entry in `reap/` as
/// expiring.
///
/// File and directory counts are taken from the manifest; a preserved
/// directory tree counts as one directory.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` for a volume without an active journal,
/// `E_CONFIG` for an invalid retention configuration and `E_IO` when the
/// journal cannot be read.
pub(crate) fn entries(ctx: &JournalCtx, volume: &VolumeId) -> Result<Vec<EntrySummary>> {
    let journal = journal_of(ctx, volume)?;
    let fs = ctx.fs.as_ref();
    let overdue = overdue(ctx)?;
    let pins = pin::read_pin_file(ctx, &journal.layout)?;
    let now = ctx.clock.now();
    let mut summaries = Vec::new();
    for op in layout::entries(fs, &journal.layout)? {
        let entry = journal.layout.entry(op)?;
        let state = match manifest::read_state(fs, &entry.state()?) {
            Ok(record) => record.state,
            Err(ReadError::Format(_)) => EntryState::Corrupt,
            Err(ReadError::Fs(error)) if error.kind == FsErrorKind::NotFound => continue,
            Err(ReadError::Fs(error)) => return Err(error.into()),
        };
        if state == EntryState::Expired {
            continue;
        }
        let mut entry_summary = summary(ctx, &entry, op, state)?;
        entry_summary.pinned = pins.is_pinned(op, now);
        entry_summary.overdue = overdue.get(&op).copied();
        summaries.push(entry_summary);
    }
    for op in reaped(fs, &journal.layout)? {
        let entry = journal.layout.reaped(op)?;
        summaries.push(summary(ctx, &entry, op, EntryState::Expiring)?);
    }
    Ok(summaries)
}

/// Items of an entry whose preserved data is still held, one per data file.
/// Preserved directory trees are released only with the whole entry.
///
/// # Errors
///
/// Fails with `E_JOURNAL_NOOP` for an unknown entry, `E_JOURNAL_CORRUPT`
/// when the manifest or the item state file is damaged and `E_IO` when the
/// entry cannot be read.
pub(crate) fn held_items(ctx: &JournalCtx, volume: &VolumeId, op: OpId) -> Result<Vec<HeldItem>> {
    let journal = journal_of(ctx, volume)?;
    let entry = journal.layout.entry(op)?;
    let manifest = entry_manifest(ctx, &journal, op)?;
    let fs = ctx.fs.as_ref();
    let items = manifest::read_items(fs, &entry.items()?)?;
    let mut held = Vec::new();
    for record in &manifest.targets {
        if !holds_data(record.before.store)
            || record.before.kind == Some(FileKind::Directory)
            || unavailable(&items, record.index)
        {
            continue;
        }
        let Some(blob) = record.before.blob.as_deref() else {
            continue;
        };
        let data = entry.resolve(blob)?;
        if exists(fs, &data)? {
            held.push(HeldItem {
                index: record.index.0,
                data,
                bytes: record.bytes_held,
            });
        }
    }
    Ok(held)
}

/// Tombstone of an entry from its manifest.
fn tombstone(manifest: &Manifest, reason: ExpiryReason) -> Tombstone {
    let mut irreversible: Vec<_> = manifest
        .irreversible
        .iter()
        .map(|entry| entry.reason)
        .chain(
            manifest
                .targets
                .iter()
                .filter_map(|target| target.irreversible),
        )
        .collect();
    irreversible.dedup();
    Tombstone {
        op: manifest.op,
        time: manifest.time,
        kind: manifest.kind.clone(),
        files: u64::try_from(manifest.targets.len()).unwrap_or(u64::MAX),
        bytes: manifest.bytes_held,
        irreversible,
        state: EntryState::Expired,
        reason,
    }
}

/// Records why the entry expires and writes its tombstone; returns the
/// entry directory and its destination in the reclaim area, which exists
/// after this call.
///
/// # Errors
///
/// Fails with `E_JOURNAL_NOOP` when the entry is gone, `E_JOURNAL_CORRUPT`
/// when its manifest is damaged and `E_IO` when the reason or the tombstone
/// cannot be written.
pub(crate) fn begin_reclaim(
    ctx: &JournalCtx,
    volume: &VolumeId,
    op: OpId,
    reason: ExpiryReason,
) -> Result<ReclaimPaths> {
    let journal = journal_of(ctx, volume)?;
    let entry = journal.layout.entry(op)?;
    let reaped = journal.layout.reaped(op)?;
    let fs = ctx.fs.as_ref();
    if !exists(fs, entry.dir())? {
        return Err(unknown_entry(op, &journal.mount));
    }
    let manifest = entry_manifest(ctx, &journal, op)?;
    let scope = scope(op, ScopePurpose::Reclaim);
    manifest::write_atomic(
        fs,
        &scope,
        entry.dir(),
        &entry.dir().join_name(EXPIRY_FILE)?,
        reason_text(reason).as_bytes(),
    )?;
    tomb::write(
        &CtxStore(ctx),
        &journal.mount,
        &tombstone(&manifest, reason),
    )?;
    ensure_dir(ctx, op, &journal.layout.reap_dir()?)?;
    Ok(ReclaimPaths {
        entry: entry.dir().clone(),
        reap: reaped.dir().clone(),
    })
}

/// Completes the reclaim of an entry: removes whatever remains of it in the
/// reclaim area, completes its parts on other volumes and prunes the
/// tombstones of the volume. Idempotent.
///
/// # Errors
///
/// Fails with `E_JOURNAL_NOOP` when the entry was never being reclaimed,
/// `E_CONFIG` for an invalid configuration and `E_IO` when the remainder
/// cannot be removed or the tombstones cannot be pruned.
pub(crate) fn complete_reclaim(
    ctx: &JournalCtx,
    volume: &VolumeId,
    op: OpId,
) -> Result<ExpiredEntry> {
    let journal = journal_of(ctx, volume)?;
    let store = CtxStore(ctx);
    let completed = expiry::complete_reap(ctx, op);
    store.remove_reaped(&journal.mount, op)?;
    let expired = completed?;
    let keep = expiry::resolve_config(&store)?.config.journal.history_keep;
    tomb::prune(&store, &journal.mount, keep)?;
    Ok(expired)
}

/// Sets the pending marker of the entry, marks `items` released in `ITEMS`
/// and a committed entry partial, each step durable before the next.
///
/// # Errors
///
/// Fails with `E_JOURNAL_NOOP` for an unknown entry, `E_JOURNAL_CORRUPT`
/// when the item state file is damaged and `E_IO` when the marker, the
/// marks or the state cannot be written.
pub(crate) fn mark_released(
    ctx: &JournalCtx,
    volume: &VolumeId,
    op: OpId,
    items: &[u32],
) -> Result<()> {
    let journal = journal_of(ctx, volume)?;
    let entry = journal.layout.entry(op)?;
    let fs = ctx.fs.as_ref();
    if !exists(fs, entry.dir())? {
        return Err(unknown_entry(op, &journal.mount));
    }
    let scope = scope(op, ScopePurpose::Reclaim);
    let pending_dir = journal.layout.pending_dir()?;
    ensure_dir(ctx, op, &pending_dir)?;
    match fs.create_new(&scope, &journal.layout.pending(op)?) {
        Ok(handle) => drop(handle),
        Err(error) if error.kind == FsErrorKind::Exists => {}
        Err(error) => return Err(error.into()),
    }
    fs.sync_dir(&scope, &pending_dir)?;
    let path = entry.items()?;
    let mut marks = manifest::read_items(fs, &path)?;
    for &index in items {
        let index = TargetIndex(index);
        match marks.items.iter_mut().find(|item| item.index == index) {
            Some(item) => item.released = true,
            None => marks.items.push(ItemState {
                index,
                released: true,
                undone_by: None,
            }),
        }
    }
    manifest::write_items(fs, &scope, entry.dir(), &path, &marks)?;
    let state = manifest::read_state(fs, &entry.state()?)?;
    if state.state == EntryState::Committed {
        manifest::write_state(
            fs,
            &scope,
            entry.dir(),
            &entry.state()?,
            StateRecord::new(EntryState::Partial),
        )?;
    }
    Ok(())
}

/// Clears the pending marker of the entry after a partial release.
///
/// # Errors
///
/// Fails with `E_IO` when the marker cannot be removed.
pub(crate) fn finish_release(ctx: &JournalCtx, volume: &VolumeId, op: OpId) -> Result<()> {
    let journal = journal_of(ctx, volume)?;
    let scope = scope(op, ScopePurpose::Reclaim);
    match ctx.fs.remove_file(&scope, &journal.layout.pending(op)?) {
        Ok(()) => {}
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    Ok(ctx.fs.sync_dir(&scope, &journal.layout.pending_dir()?)?)
}
