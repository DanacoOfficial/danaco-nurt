// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/journal_prior.rs
// Responsibility:   Journal steps shared by the undo, reclaim and scale corpora.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs, nurt_journal
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Journal steps shared by the undo, reclaim and scale corpora.
//!
//! The initial state of these cases contains committed journal entries, so
//! it is built with [`perform_prior`] through a journal without failpoints.
//! Reclaim is driven through `EntryCatalog` with the steps of the space
//! service: a whole entry is locked, marked expiring, renamed into the
//! reclaim area, removed and completed; a partial release marks the largest
//! items released, removes their data and clears the pending marker. The
//! views below read the journal through its public interface for the
//! oracles.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::config::OverwriteStrategy;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, Fs, FsError, FsErrorKind, FsResult, FsWrite, RenameMode,
    ScopePurpose, TxScope, VolumeId,
};
use nurt_contracts::journal::{
    BeginRequest, EntryState, ExpiryReason, FileOp, HistoryQuery, JournalStore, OpId, OpKind,
    OperationRecord, UndoRequest, apply_ops,
};
use nurt_crashfs::crash::CrashFs;
use nurt_crashfs::oracle::{Change, OpOutcome, TreeSnapshot, Violation, check_history};
use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints, SeededFailpoints};
use nurt_journal::layout::{self, BLOBS_DIR, DIR_NAME, JournalLayout, OPS_DIR, REAP_DIR};
use nurt_journal::store::{EntryCatalog, HeldItem, Journal};

use crate::journal::{STATE_DIR, journal, journal_dir};
use crate::support::volume_root;

/// Content written by the later of two operations on the same file.
pub const NEWER: &[u8] = b"content written by a later operation";
/// Strategy of every journal of these corpora.
pub const STRATEGY: OverwriteStrategy = OverwriteStrategy::Exchange;

/// Suffix of the files that hold preserved content in an entry.
const BLOB_SUFFIX: &str = ".blob";
/// Removal workers of a whole-entry release; one keeps the file system calls
/// in a fixed order, so that every run of a seed is the same.
const REMOVAL_WORKERS: usize = 1;
/// Limit of the history listing that covers every entry of a case.
const ALL_ENTRIES: u32 = u32::MAX;

/// Relative path of `path`, written with `/`.
pub fn relative(path: &str) -> PathBuf {
    path.split('/').collect()
}

/// File `path` created or overwritten with `content`.
pub fn write(path: &str, content: &[u8]) -> Change {
    Change::Write {
        path: relative(path),
        content: content.to_vec(),
    }
}

/// File or tree `path` removed.
pub fn remove(path: &str) -> Change {
    Change::Remove {
        path: relative(path),
    }
}

/// `tree` with `changes` applied in order.
pub fn applied(tree: TreeSnapshot, changes: &[Change]) -> TreeSnapshot {
    changes.iter().fold(tree, |tree, change| tree.with(change))
}

/// Bound program recorded by the journal calls of these corpora.
pub fn program() -> BoundDocument {
    BoundDocument::new(
        "catalog",
        "corrections",
        BoundProgram {
            statements: Vec::new(),
        },
    )
}

fn request(command: &str, bulk: bool) -> BeginRequest {
    BeginRequest {
        kind: OpKind::command(command),
        session: None,
        program: program(),
        corrections: Vec::new(),
        bulk,
        counts_as_step: true,
        preserve_budget: None,
    }
}

/// Commits `ops` as one operation of kind `command`; returns its number.
///
/// # Errors
///
/// Fails with the error of the transaction.
pub fn commit(journal: &Journal, command: &str, ops: Vec<FileOp>) -> Result<Option<OpId>, Error> {
    let mut tx = journal.begin(request(command, ops.len() > 1));
    apply_ops(tx.as_mut(), ops)?;
    Ok(tx.commit()?.op)
}

/// Step of an initial state, performed through the journal without faults.
#[derive(Debug)]
pub enum Prior {
    /// One operation of kind `command` made of the listed changes.
    Operation(&'static str, Vec<FileOp>),
    /// An undo.
    Undo(Box<UndoRequest>),
    /// Release for space of the largest item of an entry that still holds
    /// data, as the space service releases items in the hard state.
    Release(OpId),
}

impl Prior {
    /// Whether the step creates a journal entry.
    pub fn creates_entry(&self) -> bool {
        !matches!(self, Self::Release(_))
    }
}

/// Performs `steps` in order with a journal without failpoints.
///
/// # Errors
///
/// Fails with the error of the first step that fails, as an `E_IO` file
/// system error naming the journal error.
pub fn perform_prior(fs: &CrashFs, steps: Vec<Prior>) -> FsResult<()> {
    let journal = view(fs);
    for step in steps {
        let done = match step {
            Prior::Operation(command, ops) => commit(&journal, command, ops).map(|_| ()),
            Prior::Undo(request) => journal.undo(&request).map(|_| ()),
            Prior::Release(op) => release_items(&journal, fs, None, op, 1),
        };
        done.map_err(|error| FsError::new(FsErrorKind::Io, &format!("prior step: {error}"), None))?;
    }
    Ok(())
}

/// Journal on `fs` without failpoints, as a process that only reads it.
pub fn view(fs: &CrashFs) -> Journal {
    journal(Arc::new(fs.clone()), fs, None, STRATEGY)
}

/// Journal on `fs` with the failpoint registry of a run.
pub fn instrumented(fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> Journal {
    let registry: Arc<dyn Failpoints> = Arc::clone(failpoints) as Arc<dyn Failpoints>;
    journal(Arc::new(fs.clone()), fs, Some(registry), STRATEGY)
}

/// Recovers the journal of `fs` in a new journal instance.
///
/// # Errors
///
/// Fails with an `E_IO` file system error naming the recovery error.
pub fn recover(journal: &Journal) -> FsResult<()> {
    journal
        .recover()
        .map(|_| ())
        .map_err(|error| FsError::new(FsErrorKind::Io, &format!("recover: {error}"), None))
}

/// Outcome of a journal call for the oracle: an injected crash stops the
/// call with `E_INTERNAL`; every other error has changed nothing.
pub fn outcome_of<T>(result: &Result<T, Error>, irreversible: impl Fn(&T) -> bool) -> OpOutcome {
    match result {
        Ok(value) => OpOutcome::Committed {
            irreversible: irreversible(value),
        },
        Err(error) if error.code() == ErrorCode::Internal => OpOutcome::Crashed,
        Err(_) => OpOutcome::RolledBack,
    }
}

fn checkpoint(failpoints: Option<&SeededFailpoints>, point: FailPoint) -> Result<(), Error> {
    match failpoints.map_or(FailAction::Continue, |registry| registry.hit(point, None)) {
        FailAction::Continue => Ok(()),
        FailAction::Error(kind) => Err(FsError::new(kind, point.as_str(), None).into()),
        FailAction::Crash => Err(Error::new(
            ErrorCode::Internal,
            format!("crash injected at {}", point.as_str()),
        )),
    }
}

/// Identity of the volume under test.
///
/// # Errors
///
/// Fails when the volume cannot be described.
pub fn volume_id<F: Fs + ?Sized>(fs: &F) -> Result<VolumeId, Error> {
    Ok(fs.volume(&volume_root())?.id)
}

fn reclaim_scope(op: OpId) -> TxScope {
    TxScope::new_for_journal(op, ScopePurpose::Reclaim)
}

/// Releases the whole entry `op` with the steps of the space service.
///
/// # Errors
///
/// Fails with the error of the first step that fails; an injected crash
/// fails with `E_INTERNAL`.
pub fn release_entry(
    journal: &Journal,
    fs: &dyn FsWrite,
    failpoints: Option<&SeededFailpoints>,
    op: OpId,
    reason: ExpiryReason,
) -> Result<(), Error> {
    let volume = volume_id(fs)?;
    let _lock = journal.lock_entry(&volume, op)?;
    let paths = journal.begin_reclaim(&volume, op, reason)?;
    let scope = reclaim_scope(op);
    fs.rename(&scope, &paths.entry, &paths.reap, RenameMode::NoReplace)?;
    sync_parents(fs, &scope, [&paths.entry, &paths.reap])?;
    checkpoint(failpoints, FailPoint::ReclaimMoved)?;
    fs.remove_tree(&scope, &paths.reap, REMOVAL_WORKERS)?;
    journal.complete_reclaim(&volume, op)?;
    Ok(())
}

/// Makes the removal or rename of every path durable by syncing its parent
/// directory once.
fn sync_parents<'p>(
    fs: &dyn FsWrite,
    scope: &TxScope,
    paths: impl IntoIterator<Item = &'p AbsPath>,
) -> Result<(), Error> {
    let mut dirs: Vec<AbsPath> = paths.into_iter().filter_map(AbsPath::parent).collect();
    dirs.sort();
    dirs.dedup();
    for dir in &dirs {
        fs.sync_dir(scope, dir)?;
    }
    Ok(())
}

/// Completes the release of every entry a crash left expiring, as the next
/// pass of the space service does.
///
/// # Errors
///
/// Fails with the error of the first step that fails.
pub fn resume_reclaims(journal: &Journal, fs: &dyn FsWrite) -> Result<(), Error> {
    let volume = volume_id(fs)?;
    for entry in journal.entries(&volume)? {
        if entry.state != EntryState::Expiring {
            continue;
        }
        let layout = JournalLayout::new(journal_dir());
        let source = layout.entry(entry.op)?.dir().clone();
        let reap = layout.reaped(entry.op)?.dir().clone();
        let scope = reclaim_scope(entry.op);
        if fs.metadata(&source, Follow::NoFollow).is_ok() {
            let _lock = journal.lock_entry(&volume, entry.op)?;
            match journal.begin_reclaim(&volume, entry.op, ExpiryReason::Space) {
                Err(error) if error.code() == ErrorCode::JournalNoop => continue,
                other => other?,
            };
            fs.rename(&scope, &source, &reap, RenameMode::NoReplace)?;
            sync_parents(fs, &scope, [&source, &reap])?;
        }
        match journal.complete_reclaim(&volume, entry.op) {
            Ok(_) => continue,
            Err(error) if error.code() == ErrorCode::JournalNoop => {}
            Err(error) => return Err(error),
        }
        if fs.metadata(&reap, Follow::NoFollow).is_ok() {
            let _lock = journal.lock_entry(&volume, entry.op)?;
            fs.remove_tree(&scope, &reap, REMOVAL_WORKERS)?;
        }
    }
    Ok(())
}

/// Releases the largest items of entry `op` until `need` bytes are released,
/// with the steps of the space service: durable marks first, then removal of
/// the data, then the cleared pending marker.
///
/// # Errors
///
/// Fails with the error of the first step that fails; an injected crash
/// fails with `E_INTERNAL`.
pub fn release_items(
    journal: &Journal,
    fs: &dyn FsWrite,
    failpoints: Option<&SeededFailpoints>,
    op: OpId,
    need: u64,
) -> Result<(), Error> {
    let volume = volume_id(fs)?;
    let _lock = journal.lock_entry(&volume, op)?;
    let mut items = journal.held_items(&volume, op)?;
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
    let indices: Vec<u32> = chosen.iter().map(|item| item.index).collect();
    journal.mark_released(&volume, op, &indices)?;
    let scope = reclaim_scope(op);
    for item in &chosen {
        fs.remove_file(&scope, &item.data)?;
    }
    sync_parents(fs, &scope, chosen.iter().map(|item| &item.data))?;
    checkpoint(failpoints, FailPoint::ReclaimPartial)?;
    journal.finish_release(&volume, op)?;
    Ok(())
}

/// Violation for a journal call of an oracle that failed.
pub fn unreadable(error: &Error) -> Violation {
    Violation::Unreadable {
        detail: error.to_string(),
    }
}

/// Tree of the volume without the journal and the account state.
///
/// # Errors
///
/// Fails when the tree cannot be read.
pub fn user_tree(fs: &dyn Fs) -> Result<TreeSnapshot, Violation> {
    Ok(TreeSnapshot::capture(
        fs,
        &volume_root(),
        &[Path::new(DIR_NAME), Path::new(STATE_DIR)],
    )?)
}

/// Tree of the whole volume, the journal included.
///
/// # Errors
///
/// Fails when the tree cannot be read.
pub fn volume_tree(fs: &dyn Fs) -> Result<TreeSnapshot, Violation> {
    Ok(TreeSnapshot::capture(fs, &volume_root(), &[])?)
}

/// Records of `history` by operation number, checked for duplicates.
///
/// # Errors
///
/// Fails when the history cannot be read or lists a number twice.
pub fn records(journal: &Journal) -> Result<BTreeMap<u64, OperationRecord>, Violation> {
    let query = HistoryQuery {
        limit: Some(ALL_ENTRIES),
        ..HistoryQuery::default()
    };
    let listed = journal
        .history(&query)
        .map_err(|error| unreadable(&error))?;
    check_history(&listed)?;
    Ok(listed
        .into_iter()
        .map(|record| (record.id, record))
        .collect())
}

/// Path of entry `op` relative to the volume root.
pub fn entry_path(op: OpId) -> PathBuf {
    Path::new(DIR_NAME)
        .join(OPS_DIR)
        .join(layout::entry_name(op))
}

fn names(fs: &dyn Fs, dir: &AbsPath) -> Result<Vec<String>, Violation> {
    let listing = match fs.read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in listing {
        names.push(entry?.name.to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// Checks that the journal holds the data of every item it lists and
/// nothing else: the reclaim area is empty, no entry is left expiring, every
/// listed entry has the data of each item it counts, and every block in an
/// entry belongs to an item that still holds data.
///
/// # Errors
///
/// Fails with [`Violation::TemporaryLeftover`] for a leftover in the reclaim
/// area, an expiring entry or an orphaned block, and with
/// [`Violation::PreviousContentLost`] for an entry with missing blocks.
pub fn check_blocks(journal: &Journal, fs: &dyn Fs) -> Result<(), Violation> {
    let layout = JournalLayout::new(journal_dir());
    if let Some(name) = names(fs, &layout.reap_dir()?)?.into_iter().next() {
        return Err(Violation::TemporaryLeftover {
            path: Path::new(DIR_NAME).join(REAP_DIR).join(name),
        });
    }
    let volume = volume_id(fs).map_err(|error| unreadable(&error))?;
    let entries = journal
        .entries(&volume)
        .map_err(|error| unreadable(&error))?;
    let mut held: HashSet<AbsPath> = HashSet::new();
    for entry in &entries {
        if entry.state == EntryState::Expiring {
            return Err(Violation::TemporaryLeftover {
                path: entry_path(entry.op),
            });
        }
        let items = journal
            .held_items(&volume, entry.op)
            .map_err(|error| unreadable(&error))?;
        let bytes: u64 = items.iter().map(|item| item.bytes).sum();
        if bytes != entry.bytes {
            return Err(Violation::PreviousContentLost {
                path: entry_path(entry.op),
            });
        }
        held.extend(items.into_iter().map(|item| item.data));
    }
    for name in names(fs, &layout.ops_dir()?)? {
        let Some(op) = layout::parse_entry_name(&name) else {
            continue;
        };
        let blobs = layout.entry(op)?.blobs()?;
        for block in names(fs, &blobs)? {
            if !block.ends_with(BLOB_SUFFIX) {
                continue;
            }
            let path = blobs.join_name(&block)?;
            if fs.metadata(&path, Follow::NoFollow)?.kind == FileKind::Directory {
                continue;
            }
            if !held.contains(&path) {
                return Err(Violation::TemporaryLeftover {
                    path: entry_path(op).join(BLOBS_DIR).join(block),
                });
            }
        }
    }
    Ok(())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
