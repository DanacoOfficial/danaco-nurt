// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::recovery
// Responsibility:   Crash recovery.
// Dependencies:     nurt_contracts, nurt_fs, nurt_testkit, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Crash recovery.
//!
//! Recovery runs once per process before the first transaction and at start
//! of the space service. Each interrupted operation ends fully applied or
//! fully reverted, and previous content is never discarded.
//!
//! Interrupted entries are found through the markers in `pending/`; the cost
//! grows with the number of non-terminal entries, not with the journal size.
//! An entry is touched only under its lock; a held lock means a live
//! transaction. When the markers cannot be listed, every entry is scanned,
//! reading only state files; that scan completes rollbacks and leaves
//! prepared entries unchanged.
//!
//! Actions: an unfinished entry directory is removed; a prepared entry is
//! rolled back target by target in reverse order and becomes `undone`; a
//! started rollback is completed; a committed entry gets its missing index
//! row and its released items are reconciled. An unreadable manifest or
//! state, a digest mismatch or previous content that cannot be found makes
//! the entry `corrupt` without any other change. A multi-volume entry is
//! completed on every volume when any part is committed. The state
//! `rolling_back` is durable before the first change, so an interrupted
//! recovery is completed by the next one.
//!
//! The previous content of a target is under the target name, in the block
//! of the entry, or under the staged name after an interrupted exchange of
//! names. It is identified by file identity, size and modification time for
//! strategies that keep the original file, by size for clones and copies
//! (whose blocks get their final name only when complete), and by content
//! hash whenever both sides have one. Current content replaced by recovery is
//! moved into the entry, never deleted.

use std::io::{Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, ContentHash, Durability, ExtendedMeta, FileId, FileKind, FileLock, FileMeta, Follow,
    Fs, FsError, FsErrorKind, FsWrite, ProviderSnapshot, RenameMode, ScopePurpose, TxScope,
};
use nurt_contracts::journal::{
    EntryState, OpId, PreserveMode, RecoveryReport, TargetAction, TargetIndex, TargetPath,
    UndoneCause,
};
use nurt_fs::failpoint::FailPoint;
use sha2::{Digest, Sha256};

use crate::blob;
use crate::integrity::{self, ManifestVerdict};
use crate::layout::{self, EntryLayout, JournalLayout};
use crate::manifest::{
    self, BeforeRecord, IndexPart, IndexRow, ItemState, Items, Manifest, ReadError, StateRecord,
    TargetRecord,
};
use crate::store::{self, Fault, JournalCtx};
use crate::undo;

/// Facts about a file system entry, recorded before the change or observed
/// during recovery. Unknown facts are `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// Entry kind.
    pub(crate) kind: Option<FileKind>,
    /// Logical size in bytes.
    pub(crate) size: Option<u64>,
    /// Modification time, nanoseconds since the Unix epoch.
    pub(crate) modified_ns: Option<i64>,
    /// File identity.
    pub(crate) file_id: Option<FileId>,
    /// Content hash.
    pub(crate) hash: Option<ContentHash>,
}

/// Whether observed facts denote the recorded previous state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Likeness {
    Same,
    Different,
    /// Only a content hash can decide.
    Undecided,
}

/// How a holder of content relates to the original file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Identity {
    /// The holder is expected to be the original file itself.
    Original,
    /// The holder is a complete copy of the original content.
    Copy,
}

fn likeness(before: &Facts, now: &Facts, identity: Identity) -> Likeness {
    if let (Some(recorded), Some(observed)) = (before.hash, now.hash) {
        return if recorded == observed {
            Likeness::Same
        } else {
            Likeness::Different
        };
    }
    if let (Some(recorded), Some(observed)) = (before.kind, now.kind)
        && recorded != observed
    {
        return Likeness::Different;
    }
    let is_directory = before.kind == Some(FileKind::Directory);
    let sizes_differ =
        !is_directory && matches!((before.size, now.size), (Some(a), Some(b)) if a != b);
    match identity {
        Identity::Original => original_likeness(before, now, is_directory, sizes_differ),
        Identity::Copy if sizes_differ => Likeness::Different,
        Identity::Copy if is_directory || before.size.is_some() => Likeness::Same,
        Identity::Copy if before.hash.is_some() => Likeness::Undecided,
        Identity::Copy => Likeness::Different,
    }
}

/// Likeness of a holder expected to be the original file: the file identity
/// decides first, then the size and the modification time.
fn original_likeness(
    before: &Facts,
    now: &Facts,
    is_directory: bool,
    sizes_differ: bool,
) -> Likeness {
    match (before.file_id, now.file_id) {
        (Some(recorded), Some(observed)) if recorded != observed => Likeness::Different,
        (Some(_), Some(_)) if is_directory => Likeness::Same,
        (Some(_), Some(_)) => {
            let times_differ = matches!(
                (before.modified_ns, now.modified_ns),
                (Some(a), Some(b)) if a != b
            );
            if sizes_differ || times_differ {
                Likeness::Different
            } else {
                Likeness::Same
            }
        }
        _ if sizes_differ => Likeness::Different,
        _ if before.hash.is_some() => Likeness::Undecided,
        _ => Likeness::Different,
    }
}

/// Likeness of the current target. A target preserved as a copy can be the
/// copy itself, put back by an interrupted rollback: a new file with another
/// identity. When the recorded hash exists, only the content hash can tell it
/// from changed content of the same size.
fn target_likeness(before: &Facts, now: &Facts, identity: Identity) -> Likeness {
    let decided = likeness(before, now, Identity::Original);
    let may_be_the_copy = identity == Identity::Copy
        && before.hash.is_some()
        && now.hash.is_none()
        && before.kind == Some(FileKind::File)
        && now.kind == Some(FileKind::File)
        && !matches!((before.size, now.size), (Some(a), Some(b)) if a != b);
    if decided == Likeness::Different && may_be_the_copy {
        Likeness::Undecided
    } else {
        decided
    }
}

/// Place that holds the previous content of a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holder {
    /// The block of the entry.
    Blob,
    /// The staged name of the target (exchange interrupted before the old
    /// file was moved into the block).
    Staged,
}

/// Action of recovery for one target whose content was preserved by name,
/// clone or copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentAction {
    /// The target holds the previous state.
    Keep,
    /// Move the previous content from the holder back to the target name;
    /// when `displace` is set, the current target is first moved into the
    /// entry.
    Restore {
        /// Place of the previous content.
        from: Holder,
        /// The target name is occupied.
        displace: bool,
    },
    /// The target did not exist before; move the current target into the
    /// entry.
    DisplaceCreated,
    /// Content hashes of the observed entries are needed to decide.
    NeedHashes,
    /// The previous content cannot be located; change nothing.
    Lost,
}

/// Observations of one target, its block and its staged name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetScene {
    /// The target existed before the change.
    pub(crate) existed: bool,
    /// Facts recorded before the change.
    pub(crate) before: Facts,
    /// Current target, when present.
    pub(crate) target: Option<Facts>,
    /// Block of the entry, when present.
    pub(crate) blob: Option<Facts>,
    /// Staged file of the target, when present.
    pub(crate) staged: Option<Facts>,
}

/// Decides how to roll back one target preserved by `mode`.
///
/// Only content-holding strategies are decided here: [`PreserveMode::Moved`],
/// [`PreserveMode::Exchange`], [`PreserveMode::ReplaceBackup`],
/// [`PreserveMode::Hardlink`], [`PreserveMode::Clone`],
/// [`PreserveMode::Copy`] and [`PreserveMode::Created`]. Other strategies
/// yield [`ContentAction::Lost`], so that a caller that routes them here by
/// mistake changes nothing.
pub(crate) fn decide_content(mode: PreserveMode, scene: &TargetScene) -> ContentAction {
    let block_identity = match mode {
        PreserveMode::Moved
        | PreserveMode::Exchange
        | PreserveMode::ReplaceBackup
        | PreserveMode::Hardlink => Identity::Original,
        PreserveMode::Clone | PreserveMode::Copy => Identity::Copy,
        PreserveMode::Created => {
            return if scene.target.is_some() {
                ContentAction::DisplaceCreated
            } else {
                ContentAction::Keep
            };
        }
        _ => return ContentAction::Lost,
    };
    if !scene.existed {
        return if scene.target.is_some() {
            ContentAction::DisplaceCreated
        } else {
            ContentAction::Keep
        };
    }
    let target = scene
        .target
        .map(|facts| target_likeness(&scene.before, &facts, block_identity));
    if target == Some(Likeness::Same) {
        return ContentAction::Keep;
    }
    // A target that may still hold the previous content is decided by its
    // hash first: restoring a holder would displace an untouched target.
    if target == Some(Likeness::Undecided)
        && scene
            .target
            .is_some_and(|facts| facts.kind == Some(FileKind::File) && facts.hash.is_none())
    {
        return ContentAction::NeedHashes;
    }
    let candidates = [
        (Holder::Blob, scene.blob, block_identity),
        (Holder::Staged, scene.staged, Identity::Original),
    ];
    let mut undecided = target == Some(Likeness::Undecided);
    for (holder, facts, identity) in candidates {
        let Some(facts) = facts else { continue };
        match likeness(&scene.before, &facts, identity) {
            Likeness::Same => {
                return ContentAction::Restore {
                    from: holder,
                    displace: scene.target.is_some(),
                };
            }
            Likeness::Undecided => undecided = true,
            Likeness::Different => {}
        }
    }
    if undecided {
        ContentAction::NeedHashes
    } else {
        ContentAction::Lost
    }
}

/// Wait for the lock of an entry: none. A held lock belongs to a live
/// transaction, which recovery leaves alone.
const LIVE_LOCK_WAIT: Duration = Duration::ZERO;
/// Upper bound of worker threads per pass.
const MAX_WORKERS: usize = 8;
/// Workers removing an unfinished entry directory.
const REMOVE_WORKERS: usize = 1;
/// Permission bits of the directory of displaced content.
const DISPLACED_MODE: u32 = 0o700;
/// Permission bits of a journal directory created again by recovery.
const JOURNAL_DIR_MODE: u32 = 0o700;
/// Buffer size for hashing the kept prefix of an appended file.
const HASH_BUFFER: usize = 64 * 1024;
/// Commands whose entries restore items of other entries.
const UNDO_COMMANDS: [&str; 2] = ["undo", "redo"];

/// Full scan of `ops/` made because the marker directory could not be
/// listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FullScan {
    /// Journal directory.
    pub(crate) journal: AbsPath,
    /// Entries whose state was read.
    pub(crate) entries: u64,
    /// Time spent.
    pub(crate) elapsed: Duration,
}

/// Detailed result of a recovery pass; [`recover`] reports its contract
/// part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecoveryOutcome {
    /// Result in the form of the journal interface.
    pub(crate) report: RecoveryReport,
    /// Parts of multi-volume entries completed to `committed`.
    pub(crate) rolled_forward: Vec<OpId>,
    /// Committed entries whose missing index row was appended.
    pub(crate) index_appended: Vec<OpId>,
    /// Entries whose item state file is damaged; their items are treated as
    /// unavailable and the entries are left for salvage.
    pub(crate) items_damaged: Vec<OpId>,
    /// Committed undo entries whose source entries still await their update;
    /// their markers are kept.
    pub(crate) sources_pending: Vec<OpId>,
    /// Entries skipped because a live transaction holds their lock.
    pub(crate) live: Vec<OpId>,
    /// Entries left unchanged because their outcome cannot be decided yet: a
    /// prepared entry without marker, or a multi-volume entry with a part
    /// that cannot be read.
    pub(crate) unresolved: Vec<OpId>,
    /// Entries left non-terminal after an error; retried by the next pass.
    pub(crate) deferred: Vec<(OpId, String)>,
    /// Journal directories with an unknown format version, left untouched.
    pub(crate) foreign: Vec<AbsPath>,
    /// Full scans made.
    pub(crate) full_scans: Vec<FullScan>,
    /// State files read.
    pub(crate) states_read: u64,
}

impl RecoveryOutcome {
    fn merge(&mut self, other: Self) {
        let report = other.report;
        self.report.rolled_back.extend(report.rolled_back);
        self.report
            .completed_rollbacks
            .extend(report.completed_rollbacks);
        self.report
            .completed_reclaims
            .extend(report.completed_reclaims);
        self.report.corrupt.extend(report.corrupt);
        self.report.expired.extend(report.expired);
        self.rolled_forward.extend(other.rolled_forward);
        self.index_appended.extend(other.index_appended);
        self.items_damaged.extend(other.items_damaged);
        self.sources_pending.extend(other.sources_pending);
        self.live.extend(other.live);
        self.unresolved.extend(other.unresolved);
        self.deferred.extend(other.deferred);
        self.foreign.extend(other.foreign);
        self.full_scans.extend(other.full_scans);
        self.states_read += other.states_read;
    }

    fn sort(&mut self) {
        for list in [
            &mut self.report.rolled_back,
            &mut self.report.completed_rollbacks,
            &mut self.report.completed_reclaims,
            &mut self.report.corrupt,
            &mut self.report.expired,
            &mut self.rolled_forward,
            &mut self.index_appended,
            &mut self.items_damaged,
            &mut self.sources_pending,
            &mut self.live,
            &mut self.unresolved,
        ] {
            list.sort_unstable();
            list.dedup();
        }
        self.deferred.sort();
    }

    fn record(&mut self, op: OpId, result: EntryResult) {
        let list = match result {
            EntryResult::Discarded | EntryResult::RolledBack => &mut self.report.rolled_back,
            EntryResult::CompletedRollback => &mut self.report.completed_rollbacks,
            EntryResult::Corrupt => &mut self.report.corrupt,
            EntryResult::RolledForward => &mut self.rolled_forward,
            EntryResult::Settled {
                appended,
                items_damaged,
            } => {
                if items_damaged {
                    self.items_damaged.push(op);
                }
                if !appended {
                    return;
                }
                &mut self.index_appended
            }
            EntryResult::SourcesPending => &mut self.sources_pending,
            EntryResult::Live => &mut self.live,
            EntryResult::Unresolved => &mut self.unresolved,
            EntryResult::MarkerOnly => return,
        };
        list.push(op);
    }
}

/// What recovery did with one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryResult {
    /// An unfinished entry directory was removed.
    Discarded,
    /// A prepared entry was rolled back.
    RolledBack,
    /// A started rollback was completed.
    CompletedRollback,
    /// A part of a multi-volume entry was completed to `committed`.
    RolledForward,
    /// The entry was marked corrupt.
    Corrupt,
    /// A committed or partial entry was checked.
    Settled { appended: bool, items_damaged: bool },
    /// A committed undo entry still awaits the update of its sources.
    SourcesPending,
    /// Only a superfluous marker was removed.
    MarkerOnly,
    /// A live transaction holds the entry.
    Live,
    /// The entry was left unchanged.
    Unresolved,
}

/// Completes or reverts the interrupted entries of all active journals.
///
/// Expiry of overdue entries and completion of reclaims are made by the
/// retention module and are not part of this pass.
///
/// # Errors
///
/// Fails with `E_IO` when a journal directory cannot be read and when the
/// pass was interrupted. It also fails when an entry could not be recovered.
/// Such an entry stays non-terminal, is listed in the details and is retried
/// by the next pass.
pub(crate) fn recover(ctx: &JournalCtx) -> Result<RecoveryReport> {
    let outcome = recover_detailed(ctx)?;
    if outcome.deferred.is_empty() {
        return Ok(outcome.report);
    }
    let ops: Vec<u64> = outcome.deferred.iter().map(|(op, _)| op.0).collect();
    let causes: Vec<&str> = outcome
        .deferred
        .iter()
        .map(|(_, cause)| cause.as_str())
        .collect();
    Err(Error::new(
        ErrorCode::Io,
        format!(
            "{} journal entries could not be recovered; they are retried at the next start",
            ops.len()
        ),
    )
    .with_detail("deferred", serde_json::json!(ops))
    .with_detail("causes", serde_json::json!(causes)))
}

/// Runs recovery once per process and shares the result with every caller.
///
/// Concurrent callers wait for the single pass.
///
/// # Errors
///
/// Returns the error of the first pass. The caller then keeps no previous
/// content (reason `journal_error`) and proceeds with the operation.
pub(crate) fn ensure_recovered(ctx: &JournalCtx) -> Result<()> {
    ctx.startup
        .get_or_init(|| recover(ctx))
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

/// Recovery pass with the detailed outcome.
///
/// # Errors
///
/// Fails as [`recover`]. Entries left after an error do not fail this pass;
/// they are listed in the outcome as deferred.
pub(crate) fn recover_detailed(ctx: &JournalCtx) -> Result<RecoveryOutcome> {
    let started = ctx.clock.monotonic();
    ctx.hit(FailPoint::RecoveryStarted, None)
        .map_err(fault_error)?;
    let journals = ctx.active_journals();
    let mut outcome = RecoveryOutcome::default();
    let mut failure = None;
    for result in parallel_map(&journals, |journal| recover_volume(ctx, journal)) {
        match result {
            Ok(volume) => outcome.merge(volume),
            Err(Fault::Halt) => return Err(fault_error(Fault::Halt)),
            Err(fault) => {
                failure.get_or_insert(fault);
            }
        }
    }
    if let Some(fault) = failure {
        return Err(fault_error(fault));
    }
    outcome.sort();
    outcome
        .report
        .needs_salvage
        .clone_from(&outcome.items_damaged);
    outcome.report.skipped_live.clone_from(&outcome.live);
    outcome.report.unresolved.clone_from(&outcome.unresolved);
    let elapsed = ctx.clock.monotonic().saturating_sub(started);
    outcome.report.elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    Ok(outcome)
}

fn fault_error(fault: Fault) -> Error {
    match fault {
        Fault::Fs(error) => error.into(),
        Fault::Halt => Error::new(
            ErrorCode::Io,
            "journal recovery was interrupted; it resumes at the next start",
        ),
    }
}

/// Applies `work` to every item on up to [`MAX_WORKERS`] threads, keeping
/// the order of items.
fn parallel_map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(MAX_WORKERS)
        .min(items.len());
    if workers <= 1 {
        return items.iter().map(&work).collect();
    }
    let chunk = items.len().div_ceil(workers);
    let work = &work;
    thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(work).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| match handle.join() {
                Ok(results) => results,
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    })
}

fn reader(ctx: &JournalCtx) -> &dyn Fs {
    &*ctx.fs
}

fn recover_volume(
    ctx: &JournalCtx,
    journal: &JournalLayout,
) -> std::result::Result<RecoveryOutcome, Fault> {
    let mut outcome = RecoveryOutcome::default();
    if !version_supported(reader(ctx), journal)? {
        outcome.foreign.push(journal.root().clone());
        return Ok(outcome);
    }
    let states_read = AtomicU64::new(0);
    let (ops, marked) = if let Ok(ops) = layout::pending_entries(reader(ctx), journal) {
        (ops, true)
    } else {
        let started = ctx.clock.monotonic();
        let ops = scan_candidates(ctx, journal, &states_read)?;
        outcome.full_scans.push(FullScan {
            journal: journal.root().clone(),
            entries: states_read.load(Ordering::Relaxed),
            elapsed: ctx.clock.monotonic().saturating_sub(started),
        });
        (ops, false)
    };
    let results = parallel_map(&ops, |&op| {
        let result = recover_entry(ctx, journal, op, marked, &states_read);
        let index = u32::try_from(op.0).unwrap_or(u32::MAX);
        let passed = ctx.hit(FailPoint::RecoveryEntry, Some(index));
        (op, result.and_then(|result| passed.map(|()| result)))
    });
    for (op, result) in results {
        match result {
            Ok(result) => outcome.record(op, result),
            Err(Fault::Halt) => return Err(Fault::Halt),
            Err(Fault::Fs(error)) => outcome.deferred.push((op, error.to_string())),
        }
    }
    outcome.states_read += states_read.load(Ordering::Relaxed);
    Ok(outcome)
}

/// Whether the journal directory has a format this implementation writes;
/// a journal without a version file is taken as current.
fn version_supported(fs: &dyn Fs, journal: &JournalLayout) -> std::result::Result<bool, Fault> {
    match manifest::read_bytes(fs, &journal.version()?) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).trim() == layout::VERSION_TEXT),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

/// Entries to examine when the markers cannot be listed: unfinished entry
/// directories and entries whose state is `rolling_back` or `prepared`. The
/// scan reads one state file per entry, in parallel.
fn scan_candidates(
    ctx: &JournalCtx,
    journal: &JournalLayout,
    states_read: &AtomicU64,
) -> std::result::Result<Vec<OpId>, Fault> {
    let fs = reader(ctx);
    let ops_dir = journal.ops_dir()?;
    let listing = match fs.read_dir(&ops_dir) {
        Ok(listing) => listing,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut unfinished = Vec::new();
    for entry in listing {
        let name = entry?.name;
        let stem = name
            .to_str()
            .and_then(|name| name.strip_suffix(layout::TMP_SUFFIX));
        if let Some(op) = stem.and_then(layout::parse_entry_name) {
            unfinished.push(op);
        }
    }
    let entries = layout::entries(fs, journal)?;
    let states = parallel_map(&entries, |&op| {
        states_read.fetch_add(1, Ordering::Relaxed);
        let state = journal
            .entry(op)
            .ok()
            .and_then(|entry| entry.state().ok())
            .and_then(|path| manifest::read_state(fs, &path).ok());
        (op, state)
    });
    let mut candidates = unfinished;
    candidates.extend(states.into_iter().filter_map(|(op, state)| {
        state
            .filter(|record| matches!(record.state, EntryState::Prepared | EntryState::RollingBack))
            .map(|_| op)
    }));
    candidates.sort_unstable();
    candidates.dedup();
    Ok(candidates)
}

/// Recovers one entry under its lock.
fn recover_entry(
    ctx: &JournalCtx,
    journal: &JournalLayout,
    op: OpId,
    marked: bool,
    states_read: &AtomicU64,
) -> std::result::Result<EntryResult, Fault> {
    let lock_path = journal.entry_lock(op)?;
    let scope = store::scope(op, ScopePurpose::Recovery);
    let Some(lock) = lock_entry(ctx, journal, &scope, &lock_path)? else {
        return Ok(EntryResult::Live);
    };
    let entry = Entry {
        ctx,
        journal,
        op,
        marked,
        layout: journal.entry(op)?,
        scope,
    };
    let result = entry.recover(states_read);
    drop(lock);
    if matches!(
        result,
        Ok(EntryResult::Discarded
            | EntryResult::RolledBack
            | EntryResult::CompletedRollback
            | EntryResult::RolledForward
            | EntryResult::Corrupt
            | EntryResult::Settled { .. }
            | EntryResult::MarkerOnly)
    ) {
        // A lock file left behind is harmless: the next transaction or
        // recovery pass locks it again.
        let _ = ctx.fs.remove_file(&entry.scope, &lock_path);
    }
    result
}

/// Takes the lock of an entry; `None` when a live transaction holds it.
///
/// A crash can leave a journal whose lock directory never became durable
/// while its marker did; the directory is then created again, so that the
/// entry is decided instead of being retried forever.
fn lock_entry(
    ctx: &JournalCtx,
    journal: &JournalLayout,
    scope: &TxScope,
    lock_path: &AbsPath,
) -> std::result::Result<Option<Box<dyn FileLock>>, Fault> {
    let lock = |ctx: &JournalCtx| match ctx.fs.lock_exclusive(lock_path, LIVE_LOCK_WAIT) {
        Ok(lock) => Ok(Some(lock)),
        Err(error) if error.kind == FsErrorKind::InUse => Ok(None),
        Err(error) => Err(error),
    };
    match lock(ctx) {
        Err(error) if error.kind == FsErrorKind::NotFound => {}
        other => return Ok(other?),
    }
    match ctx
        .fs
        .create_dir(scope, &journal.lock_dir()?, JOURNAL_DIR_MODE)
    {
        Ok(()) => ctx.fs.sync_dir(scope, journal.root())?,
        Err(error) if error.kind == FsErrorKind::Exists => {}
        Err(error) => return Err(error.into()),
    }
    Ok(lock(ctx)?)
}

/// One entry being recovered, with its lock held.
struct Entry<'a> {
    ctx: &'a JournalCtx,
    journal: &'a JournalLayout,
    op: OpId,
    /// The entry has a marker in `pending/`.
    marked: bool,
    layout: EntryLayout,
    scope: TxScope,
}

type Step<T> = std::result::Result<T, Fault>;

impl Entry<'_> {
    fn fs(&self) -> &dyn FsWrite {
        &*self.ctx.fs
    }

    fn read(&self) -> &dyn Fs {
        reader(self.ctx)
    }

    fn recover(&self, states_read: &AtomicU64) -> Step<EntryResult> {
        if !exists(self.read(), self.layout.dir())? {
            return self.discard_unfinished();
        }
        states_read.fetch_add(1, Ordering::Relaxed);
        let record = match manifest::read_state(self.read(), &self.layout.state()?) {
            Ok(record) => Some(record),
            Err(ReadError::Format(_)) => None,
            Err(ReadError::Fs(error)) if error.kind == FsErrorKind::NotFound => None,
            Err(ReadError::Fs(error)) => return Err(error.into()),
        };
        let Some(record) = record else {
            return self.corrupt_if_marked();
        };
        match record.state {
            EntryState::Prepared if self.marked => self.prepared(),
            EntryState::Prepared => Ok(EntryResult::Unresolved),
            EntryState::RollingBack => {
                let cause = record.cause.unwrap_or(UndoneCause::Crash);
                match self.manifest()? {
                    Some((manifest, _)) => self.roll_back(&manifest, cause, true),
                    None => self.mark_corrupt(),
                }
            }
            EntryState::Committed | EntryState::Partial if self.marked => self.settle(record.state),
            _ => self.finish(EntryResult::MarkerOnly),
        }
    }

    /// Removes the directory of an entry that never became prepared.
    fn discard_unfinished(&self) -> Step<EntryResult> {
        let unfinished = self.journal.entry_tmp(self.op)?;
        let discarded = exists(self.read(), unfinished.dir())?;
        if discarded {
            self.fs()
                .remove_tree(&self.scope, unfinished.dir(), REMOVE_WORKERS)?;
            self.fs().sync_dir(&self.scope, &self.journal.ops_dir()?)?;
        }
        self.finish(if discarded {
            EntryResult::Discarded
        } else {
            EntryResult::MarkerOnly
        })
    }

    /// Removes the marker of the entry, when it has one, and returns
    /// `result`.
    fn finish(&self, result: EntryResult) -> Step<EntryResult> {
        if self.marked {
            match self
                .fs()
                .remove_file(&self.scope, &self.journal.pending(self.op)?)
            {
                Ok(()) => {}
                Err(error) if error.kind == FsErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            self.fs()
                .sync_dir(&self.scope, &self.journal.pending_dir()?)?;
        }
        Ok(result)
    }

    fn corrupt_if_marked(&self) -> Step<EntryResult> {
        if self.marked {
            self.mark_corrupt()
        } else {
            Ok(EntryResult::Unresolved)
        }
    }

    fn mark_corrupt(&self) -> Step<EntryResult> {
        self.write_state(StateRecord::new(EntryState::Corrupt))?;
        self.finish(EntryResult::Corrupt)
    }

    fn write_state(&self, record: StateRecord) -> Step<()> {
        manifest::write_state(
            self.fs(),
            &self.scope,
            self.layout.dir(),
            &self.layout.state()?,
            record,
        )?;
        Ok(())
    }

    /// Verified manifest of this part with its digest; `None` when it is
    /// missing, unreadable or fails its digest.
    fn manifest(&self) -> Step<Option<(Manifest, ContentHash)>> {
        load_manifest(self.read(), &self.layout)
    }

    fn prepared(&self) -> Step<EntryResult> {
        let Some((manifest, _)) = self.manifest()? else {
            return self.mark_corrupt();
        };
        if manifest.parts.len() > 1 {
            match self.part_decision(&manifest) {
                PartDecision::Forward => return self.roll_forward(&manifest),
                PartDecision::Unknown => return Ok(EntryResult::Unresolved),
                PartDecision::Back => {}
            }
        }
        self.roll_back(&manifest, UndoneCause::Crash, false)
    }

    /// Restores every target, in reverse order, after deciding all of them;
    /// when the previous content of any target cannot be located, nothing is
    /// changed and the entry becomes corrupt.
    fn roll_back(
        &self,
        manifest: &Manifest,
        cause: UndoneCause,
        started: bool,
    ) -> Step<EntryResult> {
        let mut plan: Vec<Action> = Vec::new();
        let mut vacated: Vec<AbsPath> = Vec::new();
        for record in manifest.targets.iter().rev() {
            match self.plan_target(record, &vacated)? {
                Some(steps) => {
                    vacated.extend(steps.iter().filter_map(Action::vacates).cloned());
                    plan.extend(steps);
                }
                None if Self::superseded(manifest, record) => {}
                None => return self.mark_corrupt(),
            }
        }
        if !started {
            self.write_state(StateRecord::rolling_back(cause))?;
        }
        for action in &plan {
            self.execute(action)?;
        }
        for record in &manifest.targets {
            self.remove_staged(record)?;
        }
        if is_undo_entry(manifest) {
            self.release_item_marks(manifest)?;
        }
        self.write_state(StateRecord::undone(cause))?;
        self.finish(if started {
            EntryResult::CompletedRollback
        } else {
            EntryResult::RolledBack
        })
    }

    /// Whether `record` repeats the path of an earlier target. The facts it
    /// recorded describe the content before the earlier target changed the
    /// path, so they cannot locate its previous content once the earlier
    /// target applied; the earlier target restores that content.
    fn superseded(manifest: &Manifest, record: &TargetRecord) -> bool {
        matches!(record.path, TargetPath::Fs(_))
            && manifest
                .targets
                .iter()
                .any(|other| other.index < record.index && other.path == record.path)
    }

    /// Actions that restore one target. `vacated` lists the names that the
    /// actions planned for later targets of the entry move away before the
    /// actions of this target run.
    fn plan_target(&self, record: &TargetRecord, vacated: &[AbsPath]) -> Step<Option<Vec<Action>>> {
        let before = &record.before;
        match (&record.path, before.store) {
            (
                TargetPath::Fs(path),
                PreserveMode::Moved
                | PreserveMode::Exchange
                | PreserveMode::ReplaceBackup
                | PreserveMode::Hardlink
                | PreserveMode::Clone
                | PreserveMode::Copy
                | PreserveMode::Created,
            ) => self.plan_content(record, path),
            (TargetPath::Fs(path), PreserveMode::Meta) => self.plan_meta(before, path),
            (TargetPath::Fs(path), PreserveMode::Append) => self.plan_append(record, path),
            (TargetPath::Fs(path), PreserveMode::Rename) => self.plan_rename(record, path, vacated),
            (TargetPath::Provider(_), PreserveMode::Value | PreserveMode::Ref) => {
                self.plan_provider(before)
            }
            (_, PreserveMode::None | PreserveMode::External) => Ok(Some(Vec::new())),
            _ => Ok(None),
        }
    }

    fn blob_path(&self, recorded: Option<&str>) -> Option<AbsPath> {
        recorded.and_then(|recorded| self.layout.resolve(recorded).ok())
    }

    /// Block of the previous content of a record that keeps it in the
    /// manifest, written when missing, so that the restoring actions work on
    /// a file like for any other copy. The block stays in the entry, which is
    /// removed as a whole when it expires.
    fn materialize_inline(&self, record: &TargetRecord, bytes: &[u8]) -> Step<AbsPath> {
        let blob = self.layout.blob(layout::BlobId::new(
            record.index,
            layout::BlobRole::Previous,
        ))?;
        if observe(self.read(), &blob)?.is_none() {
            manifest::write_atomic(self.fs(), &self.scope, &self.layout.tmp()?, &blob, bytes)?;
        }
        Ok(blob)
    }

    fn plan_content(&self, record: &TargetRecord, target: &AbsPath) -> Step<Option<Vec<Action>>> {
        let fs = self.read();
        let before = &record.before;
        let blob = match &before.inline {
            Some(bytes) => Some(self.materialize_inline(record, bytes)?),
            None => self.blob_path(before.blob.as_deref()),
        };
        let staged = staged_path(target, self.op, record.index);
        let mut scene = TargetScene {
            existed: before.exists,
            before: before_facts(before),
            target: observe(fs, target)?,
            blob: observe_optional(fs, blob.as_ref())?,
            staged: observe_optional(fs, staged.as_ref())?,
        };
        let mut decision = decide_content(before.store, &scene);
        if decision == ContentAction::NeedHashes {
            scene.target = with_hash(fs, Some(target), scene.target)?;
            scene.blob = with_hash(fs, blob.as_ref(), scene.blob)?;
            scene.staged = with_hash(fs, staged.as_ref(), scene.staged)?;
            decision = decide_content(before.store, &scene);
        }
        let copies = matches!(before.store, PreserveMode::Clone | PreserveMode::Copy);
        let action = match decision {
            ContentAction::Keep => return Ok(Some(Vec::new())),
            ContentAction::Lost | ContentAction::NeedHashes => return Ok(None),
            ContentAction::DisplaceCreated => Action::Displace {
                index: record.index,
                target: target.clone(),
            },
            ContentAction::Restore { from, displace } => {
                let source = match from {
                    Holder::Blob => blob,
                    Holder::Staged => staged,
                };
                let Some(source) = source else {
                    return Ok(None);
                };
                let meta = if copies {
                    self.blob_path(before.meta.as_deref())
                        .and_then(|path| manifest::read_meta(fs, &path).ok())
                        .or_else(|| before.plain_meta())
                        .map(Box::new)
                } else {
                    None
                };
                Action::Restore {
                    index: record.index,
                    from: source,
                    target: target.clone(),
                    displace,
                    in_place: record.in_place && copies,
                    meta,
                }
            }
        };
        Ok(Some(vec![action]))
    }

    fn plan_meta(&self, before: &BeforeRecord, target: &AbsPath) -> Step<Option<Vec<Action>>> {
        let Some(meta) = self.blob_path(before.meta.as_deref()) else {
            return Ok(None);
        };
        if observe(self.read(), target)?.is_none() {
            return Ok(None);
        }
        match manifest::read_meta(self.read(), &meta) {
            Ok(meta) => Ok(Some(vec![Action::RestoreMeta {
                target: target.clone(),
                meta: Box::new(meta),
            }])),
            Err(ReadError::Fs(error)) if error.kind != FsErrorKind::NotFound => Err(error.into()),
            Err(_) => Ok(None),
        }
    }

    fn plan_append(&self, record: &TargetRecord, target: &AbsPath) -> Step<Option<Vec<Action>>> {
        let before = &record.before;
        let (Some(current), Some(length)) = (observe(self.read(), target)?, before.size) else {
            return Ok(None);
        };
        let identity_changed = matches!(
            (before.file_id, current.file_id),
            (Some(recorded), Some(observed)) if recorded != observed
        );
        let size = current.size.unwrap_or_default();
        if identity_changed || size < length {
            return Ok(None);
        }
        if size == length {
            return Ok(Some(Vec::new()));
        }
        if let Some(expected) = before.hash
            && prefix_hash(self.read(), target, length)? != expected
        {
            return Ok(None);
        }
        Ok(Some(vec![Action::Truncate {
            index: record.index,
            target: target.clone(),
            length,
        }]))
    }

    /// Actions that undo a rename: the renamed entry returns to the source
    /// name, then the replaced destination returns from its block.
    ///
    /// Every state a crash can leave is decided: neither step made, the
    /// destination displaced, the rename made, and the destination already
    /// back. Content that another action of the entry is about to move away
    /// (`vacated`) does not hold the source name.
    fn plan_rename(
        &self,
        record: &TargetRecord,
        from: &AbsPath,
        vacated: &[AbsPath],
    ) -> Step<Option<Vec<Action>>> {
        let TargetAction::Rename { to, .. } = &record.action else {
            return Ok(None);
        };
        let fs = self.read();
        let before = before_facts(&record.before);
        let is_source = |facts: &Option<Facts>| {
            facts.is_some_and(|facts| {
                likeness(&before, &facts, Identity::Original) == Likeness::Same
            })
        };
        let at_source = observe(fs, from)?;
        let at_destination = observe(fs, to)?;
        let mut actions = Vec::new();
        let mut destination_free = at_destination.is_none();
        if !is_source(&at_source) {
            let name_taken = at_source.is_some() && !vacated.contains(from);
            if name_taken || !is_source(&at_destination) {
                return Ok(None);
            }
            actions.push(Action::Restore {
                index: record.index,
                from: to.clone(),
                target: from.clone(),
                displace: false,
                in_place: false,
                meta: None,
            });
            destination_free = true;
        }
        if let Some(replaced) = &record.replaced
            && let Some(blob) = self.blob_path(replaced.blob.as_deref())
            && let Some(kept) = observe(fs, &blob)?
        {
            let recorded = before_facts(replaced);
            if likeness(&recorded, &kept, Identity::Original) != Likeness::Same {
                return Ok(None);
            }
            let restored = !destination_free
                && at_destination.is_some_and(|facts| {
                    likeness(&recorded, &facts, Identity::Original) == Likeness::Same
                });
            if !restored {
                actions.push(Action::Restore {
                    index: record.index,
                    from: blob,
                    target: to.clone(),
                    displace: !destination_free,
                    in_place: false,
                    meta: None,
                });
            }
        }
        Ok(Some(actions))
    }

    fn plan_provider(&self, before: &BeforeRecord) -> Step<Option<Vec<Action>>> {
        let Some(blob) = self.blob_path(before.blob.as_deref()) else {
            return Ok(None);
        };
        match manifest::read_bytes(self.read(), &blob) {
            Ok(bytes) => Ok(serde_json::from_slice::<ProviderSnapshot>(&bytes)
                .ok()
                .map(|snapshot| vec![Action::Provider(Box::new(snapshot))])),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(Some(Vec::new())),
            Err(error) => Err(error.into()),
        }
    }

    fn execute(&self, action: &Action) -> Step<()> {
        let fs = self.fs();
        match action {
            Action::Restore {
                index,
                from,
                target,
                displace,
                in_place,
                meta,
            } => {
                if *in_place {
                    if *displace {
                        self.displace_copy(*index, target)?;
                    }
                    write_in_place(fs, &self.scope, from, target)?;
                } else {
                    if *displace {
                        self.displace(*index, target)?;
                    }
                    move_back(fs, &self.scope, from, target)?;
                }
                if let Some(meta) = meta {
                    fs.restore_meta(&self.scope, target, meta)?;
                    blob::sync_entry(fs, &self.scope, target)?;
                }
                Ok(())
            }
            Action::Displace { index, target } => self.displace(*index, target),
            Action::RestoreMeta { target, meta } => {
                fs.restore_meta(&self.scope, target, meta)?;
                blob::sync_entry(fs, &self.scope, target)?;
                Ok(())
            }
            Action::Truncate {
                index,
                target,
                length,
            } => {
                self.displace_copy(*index, target)?;
                fs.truncate(&self.scope, target, *length)?;
                blob::sync_entry(fs, &self.scope, target)?;
                Ok(())
            }
            Action::Provider(snapshot) => {
                let provider = self.ctx.provider(&snapshot.key.scheme).ok_or_else(|| {
                    FsError::new(FsErrorKind::Unsupported, "provider_restore", None)
                })?;
                provider.restore(&self.scope, snapshot)?;
                Ok(())
            }
        }
    }

    /// Free name for displaced content of target `index`.
    fn displaced_name(&self, index: TargetIndex) -> Step<AbsPath> {
        let dir = self.layout.displaced_dir()?;
        match self.fs().create_dir(&self.scope, &dir, DISPLACED_MODE) {
            Ok(()) => self.fs().sync_dir(&self.scope, self.layout.dir())?,
            Err(error) if error.kind == FsErrorKind::Exists => {}
            Err(error) => return Err(error.into()),
        }
        let first = self.layout.displaced(index)?;
        if observe(self.read(), &first)?.is_none() {
            return Ok(first);
        }
        let mut attempt: u64 = 1;
        loop {
            let candidate = dir.join_name(&format!("{}.{attempt}", index.0))?;
            if observe(self.read(), &candidate)?.is_none() {
                return Ok(candidate);
            }
            attempt += 1;
        }
    }

    /// Moves the current target into the entry.
    fn displace(&self, index: TargetIndex, target: &AbsPath) -> Step<()> {
        let destination = self.displaced_name(index)?;
        match self
            .fs()
            .rename(&self.scope, target, &destination, RenameMode::NoReplace)
        {
            Ok(()) => {}
            Err(error) if error.kind == FsErrorKind::CrossDevice => {
                blob::copy_entry(self.fs(), &self.scope, target, &destination)?;
                blob::sync_tree(self.fs(), &self.scope, &destination)?;
                blob::remove_entry(self.fs(), &self.scope, target)?;
            }
            Err(error) => return Err(error.into()),
        }
        blob::sync_parents(self.fs(), &self.scope, &[target, &destination])?;
        Ok(())
    }

    /// Copies the current target into the entry, keeping the target itself
    /// (files written in place keep their other hard links).
    fn displace_copy(&self, index: TargetIndex, target: &AbsPath) -> Step<()> {
        let destination = self.displaced_name(index)?;
        self.fs().copy_file(&self.scope, target, &destination)?;
        blob::sync_entry(self.fs(), &self.scope, &destination)?;
        blob::sync_parents(self.fs(), &self.scope, &[&destination])?;
        Ok(())
    }

    /// Removes the staged file of a target; a staged file that still is the
    /// previous file is moved into the entry instead.
    fn remove_staged(&self, record: &TargetRecord) -> Step<()> {
        let TargetPath::Fs(target) = &record.path else {
            return Ok(());
        };
        let Some(staged) = staged_path(target, self.op, record.index) else {
            return Ok(());
        };
        let Some(found) = observe(self.read(), &staged)? else {
            return Ok(());
        };
        let before = before_facts(&record.before);
        if record.before.exists
            && before.file_id.is_some()
            && likeness(&before, &found, Identity::Original) == Likeness::Same
        {
            return self.displace(record.index, &staged);
        }
        if found.kind == Some(FileKind::Directory) {
            self.fs()
                .remove_tree(&self.scope, &staged, REMOVE_WORKERS)?;
        } else {
            self.fs().remove_file(&self.scope, &staged)?;
        }
        sync_parent(self.fs(), &self.scope, &staged)
    }

    /// Clears item marks that a rolled-back undo entry set on its sources.
    fn release_item_marks(&self, manifest: &Manifest) -> Step<()> {
        let mut sources: Vec<OpId> = manifest.undo_of.clone();
        sources.extend(manifest.redo_of);
        sources.extend(manifest.items.iter().map(|item| item.op));
        sources.sort_unstable();
        sources.dedup();
        for source in sources {
            let entry = self.journal.entry(source)?;
            let mut items = match manifest::read_items(self.read(), &entry.items()?) {
                Ok(items) => items,
                Err(ReadError::Fs(error)) if error.kind != FsErrorKind::NotFound => {
                    return Err(error.into());
                }
                Err(_) => continue,
            };
            let mut changed = false;
            for item in &mut items.items {
                if item.undone_by == Some(self.op) {
                    item.undone_by = None;
                    changed = true;
                }
            }
            if changed {
                write_items(self.fs(), &self.scope, &entry, &items)?;
            }
        }
        Ok(())
    }

    /// Decision for a part of a multi-volume entry from the states of the
    /// other parts.
    fn part_decision(&self, manifest: &Manifest) -> PartDecision {
        let mut unknown = false;
        for part in &manifest.parts {
            if &part.dir == self.journal.root() {
                continue;
            }
            match part_state(self.read(), &JournalLayout::new(part.dir.clone()), self.op) {
                PartState::Committed => return PartDecision::Forward,
                PartState::Unknown => unknown = true,
                PartState::NotCommitted => {}
            }
        }
        if unknown {
            PartDecision::Unknown
        } else {
            PartDecision::Back
        }
    }

    /// Completes a part whose operation is committed on another volume.
    fn roll_forward(&self, manifest: &Manifest) -> Step<EntryResult> {
        let is_final = manifest.targets.is_empty()
            || manifest.targets.iter().any(|target| target.after.is_some());
        if !is_final {
            return self.mark_corrupt();
        }
        self.write_state(StateRecord::new(EntryState::Committed))?;
        if self.made_by_another_account(manifest) {
            return Ok(EntryResult::Unresolved);
        }
        self.ensure_index_row(manifest)?;
        self.finish(EntryResult::RolledForward)
    }

    /// Whether another account than the one of this process made the entry.
    /// The row of such an entry belongs to the index of that account, whose
    /// recovery appends it; the marker stays until then.
    fn made_by_another_account(&self, manifest: &Manifest) -> bool {
        manifest
            .account
            .as_deref()
            .is_some_and(|recorded| recorded != self.ctx.state.account_id())
    }

    /// Checks a committed or partial entry with a marker: reconciles item
    /// states with the preserved data, appends a missing index row and
    /// removes the marker.
    fn settle(&self, state: EntryState) -> Step<EntryResult> {
        let Some((manifest, _)) = self.manifest()? else {
            return self.mark_corrupt();
        };
        let items_damaged = !self.reconcile_items(&manifest, state)?;
        // The source update is idempotent; when it fails the marker stays and
        // the next pass repeats it.
        if is_undo_entry(&manifest) && undo::apply_source_updates(self.ctx, &manifest).is_err() {
            return Ok(EntryResult::SourcesPending);
        }
        if self.made_by_another_account(&manifest) {
            return Ok(EntryResult::Unresolved);
        }
        let appended = self.ensure_index_row(&manifest)?;
        self.finish(EntryResult::Settled {
            appended,
            items_damaged,
        })
    }

    /// Finishes an interrupted release of items: data of released items is
    /// removed, and items whose data is gone are marked released. Returns
    /// `false` when the item state file is damaged; its items are then
    /// treated as unavailable and nothing is changed.
    fn reconcile_items(&self, manifest: &Manifest, state: EntryState) -> Step<bool> {
        let mut items = match manifest::read_items(self.read(), &self.layout.items()?) {
            Ok(items) => items,
            Err(ReadError::Fs(error)) => return Err(error.into()),
            Err(ReadError::Format(_)) => return Ok(false),
        };
        let reclaim = store::scope(self.op, ScopePurpose::Reclaim);
        let mut changed = false;
        for record in &manifest.targets {
            if !holds_data(record.before.store) {
                continue;
            }
            let Some(blob) = self.blob_path(record.before.blob.as_deref()) else {
                continue;
            };
            let kept = observe(self.read(), &blob)?;
            let position = items
                .items
                .iter()
                .position(|item| item.index == record.index);
            let item = position.and_then(|position| items.items.get(position));
            let released = item.is_some_and(|item| item.released);
            let undone = item.is_some_and(|item| item.undone_by.is_some());
            match kept {
                Some(found) if released => {
                    remove_entry(self.fs(), &reclaim, &blob, found.kind)?;
                }
                None if !released && !undone => {
                    match position.and_then(|position| items.items.get_mut(position)) {
                        Some(item) => item.released = true,
                        None => items.items.push(ItemState {
                            index: record.index,
                            released: true,
                            undone_by: None,
                        }),
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            write_items(self.fs(), &reclaim, &self.layout, &items)?;
        }
        let any_released = items.items.iter().any(|item| item.released);
        if any_released && state == EntryState::Committed {
            self.write_state(StateRecord::new(EntryState::Partial))?;
        }
        Ok(true)
    }

    /// Appends the index row of a committed operation when it is missing.
    /// Returns whether a row was appended.
    fn ensure_index_row(&self, manifest: &Manifest) -> Step<bool> {
        let index = self.ctx.state.index()?;
        if index_has_row(self.read(), &index, self.op)? {
            return Ok(false);
        }
        let row = self.index_row(manifest)?;
        let _account = self.ctx.lock_account()?;
        if index_has_row(self.read(), &index, self.op)? {
            return Ok(false);
        }
        let maintenance = store::scope(self.op, ScopePurpose::Maintenance);
        manifest::append_index_row(self.fs(), &maintenance, &index, &row).map_err(read_fault)?;
        Ok(true)
    }

    fn index_row(&self, manifest: &Manifest) -> Step<IndexRow> {
        let mut parts = Vec::with_capacity(manifest.parts.len());
        for part in &manifest.parts {
            let entry = JournalLayout::new(part.dir.clone()).entry(self.op)?;
            let Some((_, digest)) = load_manifest(self.read(), &entry)? else {
                return Err(FsError::new(
                    FsErrorKind::Io,
                    "verify_manifest",
                    Some(entry.dir().as_path()),
                )
                .into());
            };
            parts.push(IndexPart {
                vol: part.vol.clone(),
                dir: part.dir.clone(),
                manifest_sha256: digest,
            });
        }
        parts.sort_by(|left, right| left.vol.cmp(&right.vol));
        Ok(IndexRow {
            op: self.op,
            time: manifest.time,
            kind: manifest.kind.clone(),
            session: manifest.session.clone(),
            bulk: manifest.bulk,
            counts_as_step: manifest.counts_as_step,
            parts,
        })
    }
}

/// One change made by a rollback.
#[derive(Debug, Clone, PartialEq)]
enum Action {
    /// Moves (or, for files written in place, copies) previous content back
    /// to the target; the current target is first moved or copied into the
    /// entry when `displace` is set.
    Restore {
        index: TargetIndex,
        from: AbsPath,
        target: AbsPath,
        displace: bool,
        in_place: bool,
        meta: Option<Box<ExtendedMeta>>,
    },
    /// Moves a target created by the operation into the entry.
    Displace { index: TargetIndex, target: AbsPath },
    /// Restores recorded metadata.
    RestoreMeta {
        target: AbsPath,
        meta: Box<ExtendedMeta>,
    },
    /// Copies an appended file into the entry and cuts it to its previous
    /// length.
    Truncate {
        index: TargetIndex,
        target: AbsPath,
        length: u64,
    },
    /// Restores a provider value.
    Provider(Box<ProviderSnapshot>),
}

impl Action {
    /// Name whose entry the action moves away, when it does.
    fn vacates(&self) -> Option<&AbsPath> {
        match self {
            Self::Restore {
                from,
                in_place: false,
                ..
            } => Some(from),
            Self::Displace { target, .. } => Some(target),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartDecision {
    Forward,
    Back,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartState {
    Committed,
    NotCommitted,
    Unknown,
}

/// Whether the part of operation `op` in `journal` was committed. Entries
/// undone after their commit (the state file says so; files of an explicit
/// undo written before that word existed name the cause `undo`) were
/// committed before; entries undone by a rollback were not.
fn part_state(fs: &dyn Fs, journal: &JournalLayout, op: OpId) -> PartState {
    let Ok(entry) = journal.entry(op) else {
        return PartState::Unknown;
    };
    let Ok(state_path) = entry.state() else {
        return PartState::Unknown;
    };
    match manifest::read_state(fs, &state_path) {
        Ok(record) => match (record.state, record.cause) {
            (
                EntryState::Committed
                | EntryState::Partial
                | EntryState::Expiring
                | EntryState::Expired,
                _,
            )
            | (EntryState::Undone, Some(UndoneCause::Undo)) => PartState::Committed,
            (EntryState::Undone, _) if record.after_commit => PartState::Committed,
            (EntryState::Prepared | EntryState::RollingBack | EntryState::Undone, _) => {
                PartState::NotCommitted
            }
            _ => PartState::Unknown,
        },
        Err(ReadError::Fs(error)) if error.kind == FsErrorKind::NotFound => {
            let reaped = journal
                .reaped(op)
                .is_ok_and(|reaped| exists(fs, reaped.dir()).unwrap_or(false));
            let tomb = journal
                .tomb(op)
                .is_ok_and(|tomb| exists(fs, &tomb).unwrap_or(false));
            if reaped || tomb {
                PartState::Committed
            } else if exists(fs, journal.root()).unwrap_or(false) {
                PartState::NotCommitted
            } else {
                PartState::Unknown
            }
        }
        Err(_) => PartState::Unknown,
    }
}

/// Reads and verifies the manifest of an entry part.
fn load_manifest(
    fs: &dyn Fs,
    entry: &EntryLayout,
) -> std::result::Result<Option<(Manifest, ContentHash)>, Fault> {
    let bytes = match manifest::read_bytes(fs, &entry.manifest()?) {
        Ok(bytes) => bytes,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let ManifestVerdict::Intact { digest, .. } = integrity::verify_manifest(&bytes) else {
        return Ok(None);
    };
    Ok(Manifest::decode(&bytes)
        .ok()
        .map(|manifest| (manifest, digest)))
}

fn is_undo_entry(manifest: &Manifest) -> bool {
    UNDO_COMMANDS.contains(&manifest.kind.command.as_str())
}

/// Strategies whose items keep data in the entry.
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

fn read_fault(error: ReadError) -> Fault {
    match error {
        ReadError::Fs(error) => error.into(),
        ReadError::Format(error) => {
            FsError::new(FsErrorKind::Io, &format!("journal_format: {error}"), None).into()
        }
    }
}

fn write_items(
    fs: &dyn FsWrite,
    scope: &TxScope,
    entry: &EntryLayout,
    items: &Items,
) -> std::result::Result<(), Fault> {
    manifest::write_items(fs, scope, entry.dir(), &entry.items()?, items).map_err(read_fault)
}

/// Whether the account index has a row for `op`; the tail is checked first.
fn index_has_row(fs: &dyn Fs, index: &AbsPath, op: OpId) -> std::result::Result<bool, Fault> {
    if manifest::last_index_row(fs, index)
        .map_err(read_fault)?
        .is_some_and(|row| row.op == op)
    {
        return Ok(true);
    }
    Ok(manifest::read_index(fs, index)
        .map_err(read_fault)?
        .iter()
        .any(|row| row.op == op))
}

fn exists(fs: &dyn Fs, path: &AbsPath) -> std::result::Result<bool, Fault> {
    Ok(observe(fs, path)?.is_some())
}

fn facts_of(meta: &FileMeta) -> Facts {
    Facts {
        kind: Some(meta.kind),
        size: Some(meta.size),
        modified_ns: Some(meta.modified_ns),
        file_id: Some(meta.file_id),
        hash: None,
    }
}

fn before_facts(before: &BeforeRecord) -> Facts {
    Facts {
        kind: before.kind,
        size: before.size,
        modified_ns: before.modified_ns,
        file_id: before.file_id,
        hash: before.hash,
    }
}

/// Facts of the entry at `path` itself (links are not followed); `None`
/// when it does not exist.
fn observe(fs: &dyn Fs, path: &AbsPath) -> std::result::Result<Option<Facts>, Fault> {
    match fs.metadata(path, Follow::NoFollow) {
        Ok(meta) => Ok(Some(facts_of(&meta))),
        Err(error)
            if matches!(
                error.kind,
                FsErrorKind::NotFound | FsErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn observe_optional(
    fs: &dyn Fs,
    path: Option<&AbsPath>,
) -> std::result::Result<Option<Facts>, Fault> {
    path.map_or(Ok(None), |path| observe(fs, path))
}

/// Adds the content hash to the facts of a regular file.
fn with_hash(
    fs: &dyn Fs,
    path: Option<&AbsPath>,
    facts: Option<Facts>,
) -> std::result::Result<Option<Facts>, Fault> {
    match (path, facts) {
        (Some(path), Some(facts)) if facts.kind == Some(FileKind::File) && facts.hash.is_none() => {
            Ok(Some(Facts {
                hash: Some(fs.content_hash(path)?),
                ..facts
            }))
        }
        (_, facts) => Ok(facts),
    }
}

/// Staged name of target `index` next to the target.
fn staged_path(target: &AbsPath, op: OpId, index: TargetIndex) -> Option<AbsPath> {
    target
        .parent()
        .and_then(|dir| dir.join_name(&layout::staged_name(op, index)).ok())
}

/// SHA-256 of the first `length` bytes of a file.
fn prefix_hash(
    fs: &dyn Fs,
    path: &AbsPath,
    length: u64,
) -> std::result::Result<ContentHash, Fault> {
    let io_error = || FsError::new(FsErrorKind::Io, "read", Some(path.as_path()));
    let mut handle = fs.open_read(path)?;
    let mut hasher = Sha256::new();
    let mut remaining = length;
    let mut buffer = vec![0_u8; HASH_BUFFER];
    while remaining > 0 {
        let want = usize::try_from(remaining).map_or(HASH_BUFFER, |left| left.min(HASH_BUFFER));
        let read = handle
            .read(buffer.get_mut(..want).unwrap_or_default())
            .map_err(|_| io_error())?;
        if read == 0 {
            return Err(io_error().into());
        }
        hasher.update(buffer.get(..read).unwrap_or_default());
        remaining -= u64::try_from(read).unwrap_or(remaining);
    }
    Ok(ContentHash(hasher.finalize().into()))
}

/// Moves preserved content back; content preserved across layers or volumes
/// is copied. The result is durable when the function returns: a rename
/// changes the directories of both names, and both are flushed.
fn move_back(
    fs: &dyn FsWrite,
    scope: &TxScope,
    from: &AbsPath,
    to: &AbsPath,
) -> std::result::Result<(), Fault> {
    match fs.rename(scope, from, to, RenameMode::NoReplace) {
        Ok(()) => blob::sync_parents(fs, scope, &[from, to])?,
        Err(error) if error.kind == FsErrorKind::CrossDevice => {
            fs.copy_file(scope, from, to)?;
            blob::sync_entry(fs, scope, to)?;
            blob::sync_parents(fs, scope, &[to])?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Writes the content of `from` into the existing file `target`, so that
/// every hard link of the target sees it.
fn write_in_place(
    fs: &dyn FsWrite,
    scope: &TxScope,
    from: &AbsPath,
    target: &AbsPath,
) -> std::result::Result<(), Fault> {
    let io_error = || FsError::new(FsErrorKind::Io, "write_in_place", Some(target.as_path()));
    let mut source = fs.open_read(from)?;
    let mut handle = fs.open_in_place(scope, target)?;
    handle.set_length(0)?;
    handle.seek(SeekFrom::Start(0)).map_err(|_| io_error())?;
    std::io::copy(&mut source, &mut handle).map_err(|_| io_error())?;
    handle.sync(Durability::Full)?;
    Ok(())
}

fn remove_entry(
    fs: &dyn FsWrite,
    scope: &TxScope,
    path: &AbsPath,
    kind: Option<FileKind>,
) -> std::result::Result<(), Fault> {
    if kind == Some(FileKind::Directory) {
        fs.remove_tree(scope, path, REMOVE_WORKERS)?;
    } else {
        fs.remove_file(scope, path)?;
    }
    sync_parent(fs, scope, path)
}

fn sync_parent(
    fs: &dyn FsWrite,
    scope: &TxScope,
    path: &AbsPath,
) -> std::result::Result<(), Fault> {
    if let Some(parent) = path.parent() {
        fs.sync_dir(scope, &parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_ID: FileId = FileId {
        volume: 1,
        low: 100,
        high: 0,
    };
    const NEW_ID: FileId = FileId {
        volume: 1,
        low: 200,
        high: 0,
    };
    const COPY_ID: FileId = FileId {
        volume: 1,
        low: 300,
        high: 0,
    };

    /// Previous file: 10 bytes, written at t = 1000, identity 100.
    fn old_file() -> Facts {
        Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            modified_ns: Some(1_000),
            file_id: Some(OLD_ID),
            hash: None,
        }
    }

    /// New content written by the interrupted operation.
    fn new_file() -> Facts {
        Facts {
            size: Some(12),
            modified_ns: Some(2_000),
            file_id: Some(NEW_ID),
            ..old_file()
        }
    }

    /// Complete copy of the previous file under a new identity.
    fn copy_of_old() -> Facts {
        Facts {
            modified_ns: Some(3_000),
            file_id: Some(COPY_ID),
            ..old_file()
        }
    }

    fn scene(target: Option<Facts>, blob: Option<Facts>, staged: Option<Facts>) -> TargetScene {
        TargetScene {
            existed: true,
            before: old_file(),
            target,
            blob,
            staged,
        }
    }

    #[test]
    fn untouched_target_is_kept_for_every_strategy() {
        for mode in [
            PreserveMode::Moved,
            PreserveMode::Exchange,
            PreserveMode::ReplaceBackup,
            PreserveMode::Hardlink,
            PreserveMode::Clone,
            PreserveMode::Copy,
        ] {
            let action = decide_content(mode, &scene(Some(old_file()), None, None));
            assert_eq!(action, ContentAction::Keep, "{mode:?}");
        }
    }

    #[test]
    fn moved_target_is_moved_back_without_displacement() {
        let action = decide_content(PreserveMode::Moved, &scene(None, Some(old_file()), None));
        assert_eq!(
            action,
            ContentAction::Restore {
                from: Holder::Blob,
                displace: false
            }
        );
    }

    #[test]
    fn exchange_before_move_to_block_restores_from_staged_name() {
        let action = decide_content(
            PreserveMode::Exchange,
            &scene(Some(new_file()), None, Some(old_file())),
        );
        assert_eq!(
            action,
            ContentAction::Restore {
                from: Holder::Staged,
                displace: true
            }
        );
    }

    #[test]
    fn exchange_staged_new_content_is_not_mistaken_for_old() {
        let action = decide_content(
            PreserveMode::Exchange,
            &scene(Some(old_file()), None, Some(new_file())),
        );
        assert_eq!(action, ContentAction::Keep);
    }

    #[test]
    fn exchange_after_move_to_block_restores_from_block() {
        let action = decide_content(
            PreserveMode::Exchange,
            &scene(Some(new_file()), Some(old_file()), None),
        );
        assert_eq!(
            action,
            ContentAction::Restore {
                from: Holder::Blob,
                displace: true
            }
        );
    }

    #[test]
    fn hard_link_block_shares_identity_with_untouched_target() {
        let action = decide_content(
            PreserveMode::Hardlink,
            &scene(Some(old_file()), Some(old_file()), None),
        );
        assert_eq!(action, ContentAction::Keep);
    }

    #[test]
    fn replace_backup_with_missing_target_name_restores_backup() {
        let action = decide_content(
            PreserveMode::ReplaceBackup,
            &scene(None, Some(old_file()), Some(new_file())),
        );
        assert_eq!(
            action,
            ContentAction::Restore {
                from: Holder::Blob,
                displace: false
            }
        );
    }

    #[test]
    fn copy_block_restores_after_switch() {
        for mode in [PreserveMode::Clone, PreserveMode::Copy] {
            let action = decide_content(mode, &scene(Some(new_file()), Some(copy_of_old()), None));
            assert_eq!(
                action,
                ContentAction::Restore {
                    from: Holder::Blob,
                    displace: true
                }
            );
        }
    }

    #[test]
    fn in_place_write_is_detected_by_modification_time() {
        let written_in_place = Facts {
            modified_ns: Some(2_000),
            ..old_file()
        };
        let action = decide_content(
            PreserveMode::Copy,
            &scene(Some(written_in_place), Some(copy_of_old()), None),
        );
        assert_eq!(
            action,
            ContentAction::Restore {
                from: Holder::Blob,
                displace: true
            }
        );
    }

    #[test]
    fn copy_block_of_wrong_size_is_not_used() {
        let truncated = Facts {
            size: Some(4),
            ..copy_of_old()
        };
        let action = decide_content(
            PreserveMode::Copy,
            &scene(Some(new_file()), Some(truncated), None),
        );
        assert_eq!(action, ContentAction::Lost);
    }

    #[test]
    fn changed_target_without_previous_content_is_lost() {
        let action = decide_content(PreserveMode::Moved, &scene(Some(new_file()), None, None));
        assert_eq!(action, ContentAction::Lost);
    }

    #[test]
    fn target_with_the_recorded_identity_and_another_modification_time_is_not_kept() {
        let touched = Facts {
            modified_ns: Some(3_000),
            ..old_file()
        };
        let action = decide_content(
            PreserveMode::Exchange,
            &scene(Some(touched), None, Some(new_file())),
        );
        assert_eq!(action, ContentAction::Lost);
        let untouched = decide_content(
            PreserveMode::Exchange,
            &scene(Some(old_file()), None, Some(new_file())),
        );
        assert_eq!(untouched, ContentAction::Keep);
    }

    #[test]
    fn created_target_is_displaced_and_absent_one_kept() {
        let created = TargetScene {
            existed: false,
            target: Some(new_file()),
            ..TargetScene::default()
        };
        assert_eq!(
            decide_content(PreserveMode::Created, &created),
            ContentAction::DisplaceCreated
        );
        assert_eq!(
            decide_content(PreserveMode::Copy, &created),
            ContentAction::DisplaceCreated
        );
        let absent = TargetScene::default();
        assert_eq!(
            decide_content(PreserveMode::Created, &absent),
            ContentAction::Keep
        );
        assert_eq!(
            decide_content(PreserveMode::Exchange, &absent),
            ContentAction::Keep
        );
    }

    #[test]
    fn known_hash_decides_when_identity_is_missing() {
        let hash = ContentHash([7; 32]);
        let before = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            hash: Some(hash),
            ..Facts::default()
        };
        let unknown_target = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            ..Facts::default()
        };
        let mut scene = TargetScene {
            existed: true,
            before,
            target: Some(unknown_target),
            blob: Some(unknown_target),
            staged: None,
        };
        assert_eq!(
            decide_content(PreserveMode::Exchange, &scene),
            ContentAction::NeedHashes
        );
        scene.target = Some(Facts {
            hash: Some(ContentHash([8; 32])),
            ..unknown_target
        });
        scene.blob = Some(Facts {
            hash: Some(hash),
            ..unknown_target
        });
        assert_eq!(
            decide_content(PreserveMode::Exchange, &scene),
            ContentAction::Restore {
                from: Holder::Blob,
                displace: true
            }
        );
        scene.target = Some(Facts {
            hash: Some(hash),
            ..unknown_target
        });
        assert_eq!(
            decide_content(PreserveMode::Exchange, &scene),
            ContentAction::Keep
        );
    }

    #[test]
    fn untouched_target_without_identity_is_hashed_before_a_copy_block_is_used() {
        let hash = ContentHash([7; 32]);
        let before = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            hash: Some(hash),
            ..Facts::default()
        };
        let same_size = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            modified_ns: Some(1_000),
            file_id: Some(OLD_ID),
            hash: None,
        };
        let mut scene = TargetScene {
            existed: true,
            before,
            target: Some(same_size),
            blob: Some(Facts {
                file_id: Some(COPY_ID),
                ..same_size
            }),
            staged: None,
        };
        assert_eq!(
            decide_content(PreserveMode::Copy, &scene),
            ContentAction::NeedHashes
        );
        scene.target = Some(Facts {
            hash: Some(hash),
            ..same_size
        });
        assert_eq!(
            decide_content(PreserveMode::Copy, &scene),
            ContentAction::Keep
        );
    }

    #[test]
    fn directory_identity_ignores_modification_time() {
        let before = Facts {
            kind: Some(FileKind::Directory),
            file_id: Some(OLD_ID),
            modified_ns: Some(1),
            ..Facts::default()
        };
        let moved = Facts {
            modified_ns: Some(5),
            ..before
        };
        let scene = TargetScene {
            existed: true,
            before,
            target: None,
            blob: Some(moved),
            staged: None,
        };
        assert_eq!(
            decide_content(PreserveMode::Moved, &scene),
            ContentAction::Restore {
                from: Holder::Blob,
                displace: false
            }
        );
    }

    #[test]
    fn kind_change_is_a_difference() {
        let link = Facts {
            kind: Some(FileKind::Symlink),
            ..old_file()
        };
        let action = decide_content(
            PreserveMode::Moved,
            &scene(Some(link), Some(old_file()), None),
        );
        assert_eq!(
            action,
            ContentAction::Restore {
                from: Holder::Blob,
                displace: true
            }
        );
    }

    #[test]
    fn strategies_without_content_are_not_decided_here() {
        for mode in [
            PreserveMode::None,
            PreserveMode::Meta,
            PreserveMode::Append,
            PreserveMode::Rename,
            PreserveMode::Value,
            PreserveMode::Ref,
            PreserveMode::External,
        ] {
            assert_eq!(
                decide_content(mode, &scene(Some(new_file()), Some(old_file()), None)),
                ContentAction::Lost
            );
        }
    }

    #[test]
    fn equal_hashes_are_the_same_and_different_hashes_are_different() {
        let recorded = Facts {
            hash: Some(ContentHash([1; 32])),
            ..old_file()
        };
        let same = Facts {
            hash: Some(ContentHash([1; 32])),
            file_id: Some(COPY_ID),
            ..old_file()
        };
        let other = Facts {
            hash: Some(ContentHash([2; 32])),
            ..old_file()
        };
        for identity in [Identity::Original, Identity::Copy] {
            assert_eq!(likeness(&recorded, &same, identity), Likeness::Same);
            assert_eq!(likeness(&recorded, &other, identity), Likeness::Different);
        }
    }

    #[test]
    fn another_kind_is_different_for_both_identities() {
        let directory = Facts {
            kind: Some(FileKind::Directory),
            ..old_file()
        };
        for identity in [Identity::Original, Identity::Copy] {
            assert_eq!(
                likeness(&old_file(), &directory, identity),
                Likeness::Different
            );
        }
    }

    #[test]
    fn target_restored_from_a_copy_is_decided_by_its_hash() {
        let hash = ContentHash([7; 32]);
        let before = Facts {
            hash: Some(hash),
            ..old_file()
        };
        let restored = Facts {
            file_id: Some(COPY_ID),
            modified_ns: Some(5),
            ..old_file()
        };
        let scene = TargetScene {
            existed: true,
            before,
            target: Some(restored),
            ..TargetScene::default()
        };
        for mode in [PreserveMode::Copy, PreserveMode::Clone] {
            assert_eq!(decide_content(mode, &scene), ContentAction::NeedHashes);
            let same = TargetScene {
                target: Some(Facts {
                    hash: Some(hash),
                    ..restored
                }),
                ..scene
            };
            assert_eq!(decide_content(mode, &same), ContentAction::Keep);
            let changed = TargetScene {
                target: Some(Facts {
                    hash: Some(ContentHash([8; 32])),
                    ..restored
                }),
                ..scene
            };
            assert_eq!(decide_content(mode, &changed), ContentAction::Lost);
        }
        assert_eq!(
            decide_content(PreserveMode::Moved, &scene),
            ContentAction::Lost
        );
    }

    #[test]
    fn copy_identity_is_decided_by_size_and_then_by_hash() {
        let sized = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            ..Facts::default()
        };
        let longer = Facts {
            size: Some(11),
            ..sized
        };
        assert_eq!(likeness(&sized, &sized, Identity::Copy), Likeness::Same);
        assert_eq!(
            likeness(&sized, &longer, Identity::Copy),
            Likeness::Different
        );
        let hashed = Facts {
            kind: Some(FileKind::File),
            hash: Some(ContentHash([3; 32])),
            ..Facts::default()
        };
        let unhashed = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            ..Facts::default()
        };
        assert_eq!(
            likeness(&hashed, &unhashed, Identity::Copy),
            Likeness::Undecided
        );
        let nothing = Facts {
            kind: Some(FileKind::File),
            ..Facts::default()
        };
        assert_eq!(
            likeness(&nothing, &unhashed, Identity::Copy),
            Likeness::Different
        );
    }

    #[test]
    fn original_identity_without_file_identity_is_decided_by_size_and_hash() {
        let sized = Facts {
            kind: Some(FileKind::File),
            size: Some(10),
            ..Facts::default()
        };
        let longer = Facts {
            size: Some(11),
            ..sized
        };
        assert_eq!(
            likeness(&sized, &longer, Identity::Original),
            Likeness::Different
        );
        assert_eq!(
            likeness(&sized, &sized, Identity::Original),
            Likeness::Different
        );
        let hashed = Facts {
            hash: Some(ContentHash([4; 32])),
            ..sized
        };
        assert_eq!(
            likeness(&hashed, &sized, Identity::Original),
            Likeness::Undecided
        );
    }

    #[test]
    fn original_identity_with_a_missing_modification_time_is_the_same_file() {
        let recorded = Facts {
            modified_ns: None,
            ..old_file()
        };
        assert_eq!(
            likeness(&recorded, &old_file(), Identity::Original),
            Likeness::Same
        );
        let other_file = Facts {
            file_id: Some(NEW_ID),
            ..old_file()
        };
        assert_eq!(
            likeness(&old_file(), &other_file, Identity::Original),
            Likeness::Different
        );
    }
}

#[cfg(test)]
mod pass_tests {
    use std::collections::HashMap;
    use std::io::{Seek, Write};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use nurt_contracts::config::OverwriteStrategy;
    use nurt_contracts::env::{BootId, ClockStamp, WallTime};
    use nurt_contracts::fs::{
        AbsPath, FileKind, Follow, Fs, FsErrorKind, FsFeatures, FsWrite, MetaChange, ProviderKey,
        ProviderScheme, ProviderSnapshot, ProviderValue, RenameMode, ScopePurpose,
    };
    use nurt_contracts::journal::{
        EntryState, OpId, OpKind, PreserveMode, TargetAction, TargetIndex, TargetPath, UndoneCause,
    };
    use nurt_fs::clock::VirtualClock;
    use nurt_fs::failpoint::{
        FailAction, FailPlan, FailPoint, FailRule, NoFailpoints, SeededFailpoints,
    };
    use nurt_fs::mem::{MemFault, MemFs, VolumeSpec};

    use super::{ensure_recovered, recover, recover_detailed};
    use crate::layout::{BlobId, BlobRole, JournalLayout, StateLayout};
    use crate::manifest::{
        self, AfterRecord, BeforeRecord, FORMAT_VERSION, IndexPart, IndexRow, Integrity, ItemState,
        Items, Manifest, PartRef, StateRecord, TargetRecord, TargetRole,
    };
    use crate::store::test_support::{
        MemProvider, TestAdmission, TestJournals, TestLimits, default_retention,
    };
    use crate::store::{self, JournalCtx, JournalLocation, Settings};

    const VOLUMES: [&str; 2] = ["/a", "/b"];
    const OLD: &[u8] = b"previous content";
    const NEW: &[u8] = b"new content of the operation";

    fn abs(text: &str) -> AbsPath {
        AbsPath::for_test(text)
    }

    fn features() -> FsFeatures {
        FsFeatures {
            exchange: true,
            rename_noreplace: true,
            replace_with_backup: true,
            hardlink: true,
            reflink: true,
            xattr: true,
            acl: true,
            alternate_streams: false,
            durable_metadata: true,
            case_sensitive: true,
            decomposes_names: false,
        }
    }

    /// Two volumes `/a` and `/b`, each with a journal in `.nurt-journal`;
    /// the account state lives in `/a/state`.
    struct World {
        fs: Arc<MemFs>,
        ctx: JournalCtx,
        journals: Vec<JournalLayout>,
        registry: Arc<MemProvider>,
    }

    impl World {
        fn new() -> Self {
            let mut builder = MemFs::builder();
            for mount in VOLUMES {
                builder = builder.volume(VolumeSpec::posix(abs(mount)).with_features(features()));
            }
            let fs = Arc::new(builder.build().unwrap());
            let mut locations = HashMap::new();
            let mut journals = Vec::new();
            for mount in VOLUMES {
                let journal = JournalLayout::new(abs(&format!("{mount}/.nurt-journal")));
                for dir in journal.structure().unwrap() {
                    fs.seed_dir(&dir).unwrap();
                }
                let dev = fs.volume(&abs(mount)).unwrap().id.dev;
                locations.insert(dev, JournalLocation::Active(journal.root().clone()));
                journals.push(journal);
            }
            fs.seed_dir(&abs("/a/state")).unwrap();
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
            Self {
                fs,
                ctx,
                journals,
                registry,
            }
        }

        fn journal(&self) -> &JournalLayout {
            &self.journals[0]
        }

        fn scope(op: u64) -> nurt_contracts::fs::TxScope {
            store::scope(OpId(op), ScopePurpose::Operation)
        }

        fn read(&self, path: &str) -> Option<Vec<u8>> {
            manifest::read_bytes(&*self.fs, &abs(path)).ok()
        }

        fn exists(&self, path: &AbsPath) -> bool {
            self.fs.metadata(path, Follow::NoFollow).is_ok()
        }

        fn blob(&self, journal: usize, op: u64, index: u32) -> AbsPath {
            self.journals[journal]
                .entry(OpId(op))
                .unwrap()
                .blob(BlobId::new(TargetIndex(index), BlobRole::Previous))
                .unwrap()
        }

        fn staged(path: &str, op: u64, index: u32) -> AbsPath {
            let target = abs(path);
            target
                .parent()
                .unwrap()
                .join_name(&crate::layout::staged_name(OpId(op), TargetIndex(index)))
                .unwrap()
        }

        /// State before the change of the file at `path`, kept by `store` in
        /// the block of target `index`.
        fn before(&self, path: &str, store: PreserveMode, index: u32) -> BeforeRecord {
            let meta = self.fs.metadata(&abs(path), Follow::NoFollow).unwrap();
            let id = BlobId::new(TargetIndex(index), BlobRole::Previous);
            BeforeRecord {
                exists: true,
                kind: Some(meta.kind),
                size: Some(meta.size),
                mode: Some(meta.mode),
                uid: meta.uid,
                gid: meta.gid,
                modified_ns: Some(meta.modified_ns),
                hash: None,
                links: Some(meta.links),
                file_id: Some(meta.file_id),
                store,
                blob: Some(id.content_name()),
                meta: Some(id.meta_name()),
                inline: None,
            }
        }

        fn part(&self, journal: usize) -> PartRef {
            PartRef {
                vol: self.fs.volume(&abs(VOLUMES[journal])).unwrap().id,
                dir: self.journals[journal].root().clone(),
            }
        }

        /// Writes the manifest, state and optional marker of an entry and
        /// returns the manifest digest.
        fn write_entry(
            &self,
            journal: usize,
            mut entry: Manifest,
            state: StateRecord,
            marker: bool,
        ) -> nurt_contracts::fs::ContentHash {
            let layout = &self.journals[journal];
            let digest = entry.seal().unwrap();
            let dir = layout.entry(entry.op).unwrap();
            self.fs
                .seed_file(&dir.manifest().unwrap(), &entry.encode().unwrap())
                .unwrap();
            self.fs
                .seed_file(&dir.state().unwrap(), &state.encode())
                .unwrap();
            let _ = self.fs.seed_dir(&dir.blobs().unwrap());
            if marker {
                self.fs
                    .seed_file(&layout.pending(entry.op).unwrap(), b"")
                    .unwrap();
            }
            digest
        }

        fn state(&self, journal: usize, op: u64) -> StateRecord {
            let path = self.journals[journal]
                .entry(OpId(op))
                .unwrap()
                .state()
                .unwrap();
            manifest::read_state(&*self.fs, &path).unwrap()
        }

        fn marked(&self, journal: usize, op: u64) -> bool {
            self.exists(&self.journals[journal].pending(OpId(op)).unwrap())
        }

        fn displaced(&self, op: u64, index: u32) -> Option<Vec<u8>> {
            let path = self
                .journal()
                .entry(OpId(op))
                .unwrap()
                .displaced(TargetIndex(index))
                .unwrap();
            manifest::read_bytes(&*self.fs, &path).ok()
        }
    }

    fn entry_of(op: u64, targets: Vec<TargetRecord>, parts: Vec<PartRef>) -> Manifest {
        let wall = WallTime(1_700_000_000_000_000_000);
        Manifest {
            v: FORMAT_VERSION,
            op: OpId(op),
            uuid: uuid::Uuid::from_u128(u128::from(op)),
            time: wall,
            time_basis: ClockStamp {
                wall,
                boot: BootId("boot".to_owned()),
                since_boot_with_suspend: Duration::from_secs(60),
                since_boot_active: Duration::from_secs(60),
            },
            kind: OpKind::command("write"),
            session: None,
            account: None,
            ast_hash: manifest::hash_bytes(b"program"),
            ast: serde_json::json!({}),
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            targets,
            parts,
            bytes_held: 0,
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
            expires: None,
            undo_of: Vec::new(),
            redo_of: None,
            scope: None,
            items: Vec::new(),
            integrity: Integrity::default(),
        }
    }

    /// State before the change of an entry that did not exist, with the
    /// planned preserve mode `store` and no preserved data.
    fn absent(store: PreserveMode) -> BeforeRecord {
        BeforeRecord {
            exists: false,
            kind: None,
            size: None,
            mode: None,
            uid: None,
            gid: None,
            modified_ns: None,
            hash: None,
            links: None,
            file_id: None,
            store,
            blob: None,
            meta: None,
            inline: None,
        }
    }

    fn target(index: u32, path: &str, action: TargetAction, before: BeforeRecord) -> TargetRecord {
        TargetRecord {
            index: TargetIndex(index),
            path: TargetPath::Fs(abs(path)),
            action,
            role: TargetRole::Whole,
            before,
            replaced: None,
            after: None,
            in_place: false,
            bytes_held: 0,
            irreversible: None,
            notable: Vec::new(),
        }
    }

    fn prepared() -> StateRecord {
        StateRecord::new(EntryState::Prepared)
    }

    /// Prepared single-target entry 1 overwriting `/a/f`, kept by `store`.
    fn overwrite_entry(world: &World, store: PreserveMode) {
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        let before = world.before("/a/f", store, 0);
        let parts = vec![world.part(0)];
        let entry = entry_of(
            1,
            vec![target(0, "/a/f", TargetAction::Modify, before)],
            parts,
        );
        world.write_entry(0, entry, prepared(), true);
        world
            .fs
            .seed_file(&World::staged("/a/f", 1, 0), NEW)
            .unwrap();
    }

    fn assert_rolled_back(world: &World) {
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
        assert_eq!(world.state(0, 1), StateRecord::undone(UndoneCause::Crash));
        assert!(!world.marked(0, 1));
        assert!(!world.exists(&World::staged("/a/f", 1, 0)));
    }

    #[test]
    fn unfinished_entry_directory_is_removed_and_targets_untouched() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        let unfinished = world.journal().entry_tmp(OpId(1)).unwrap();
        world
            .fs
            .seed_file(&unfinished.manifest().unwrap(), b"{}")
            .unwrap();
        world
            .fs
            .seed_file(&world.journal().pending(OpId(1)).unwrap(), b"")
            .unwrap();
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.rolled_back, vec![OpId(1)]);
        assert!(!world.exists(unfinished.dir()));
        assert!(!world.marked(0, 1));
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
    }

    #[test]
    fn prepared_entry_without_preserved_content_becomes_undone_by_crash() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.rolled_back, vec![OpId(1)]);
        assert_rolled_back(&world);
    }

    #[test]
    fn moved_file_and_tree_are_moved_back_by_rename() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        for n in 0..10_000 {
            world
                .fs
                .seed_file(&abs(&format!("/a/tree/d{}/f{n}", n % 100)), b"x")
                .unwrap();
        }
        let targets = vec![
            target(
                0,
                "/a/f",
                TargetAction::Delete,
                world.before("/a/f", PreserveMode::Moved, 0),
            ),
            target(
                1,
                "/a/tree",
                TargetAction::Delete,
                world.before("/a/tree", PreserveMode::Moved, 1),
            ),
        ];
        world.write_entry(
            0,
            entry_of(1, targets, vec![world.part(0)]),
            prepared(),
            true,
        );
        let scope = World::scope(1);
        world
            .fs
            .rename(
                &scope,
                &abs("/a/f"),
                &world.blob(0, 1, 0),
                RenameMode::NoReplace,
            )
            .unwrap();
        world
            .fs
            .rename(
                &scope,
                &abs("/a/tree"),
                &world.blob(0, 1, 1),
                RenameMode::NoReplace,
            )
            .unwrap();
        world.fs.reset_counters();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
        assert!(world.exists(&abs("/a/tree/d7/f9907")));
        let counters = world.fs.counters();
        assert_eq!(counters.bytes_copied, 0);
        assert!(counters.renames <= 6, "{counters:?}");
    }

    #[test]
    fn exchange_interrupted_before_move_to_block_restores_from_staged_name() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        world
            .fs
            .exchange(&World::scope(1), &abs("/a/f"), &World::staged("/a/f", 1, 0))
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
        assert_eq!(world.displaced(1, 0).as_deref(), Some(NEW));
    }

    #[test]
    fn repeated_target_is_restored_by_the_first_one() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        let first = world.before("/a/f", PreserveMode::Exchange, 0);
        let second = world.before("/a/f", PreserveMode::Exchange, 1);
        let entry = entry_of(
            1,
            vec![
                target(0, "/a/f", TargetAction::Modify, first),
                target(1, "/a/f", TargetAction::Modify, second),
            ],
            vec![world.part(0)],
        );
        world.write_entry(0, entry, prepared(), true);
        let scope = World::scope(1);
        let staged = World::staged("/a/f", 1, 0);
        world.fs.seed_file(&staged, NEW).unwrap();
        world.fs.exchange(&scope, &abs("/a/f"), &staged).unwrap();
        world
            .fs
            .rename(&scope, &staged, &world.blob(0, 1, 0), RenameMode::NoReplace)
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
    }

    #[test]
    fn exchange_after_move_to_block_restores_from_block() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let scope = World::scope(1);
        let staged = World::staged("/a/f", 1, 0);
        world.fs.exchange(&scope, &abs("/a/f"), &staged).unwrap();
        world
            .fs
            .rename(&scope, &staged, &world.blob(0, 1, 0), RenameMode::NoReplace)
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
        assert_eq!(world.displaced(1, 0).as_deref(), Some(NEW));
    }

    #[test]
    fn hard_link_fallback_is_restored_after_switch() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Hardlink);
        let scope = World::scope(1);
        world
            .fs
            .hard_link(&scope, &abs("/a/f"), &world.blob(0, 1, 0))
            .unwrap();
        world
            .fs
            .rename(
                &scope,
                &World::staged("/a/f", 1, 0),
                &abs("/a/f"),
                RenameMode::Replace,
            )
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
    }

    #[test]
    fn backup_name_is_restored_after_complete_and_partial_replacement() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::ReplaceBackup);
        world
            .fs
            .replace_with_backup(
                &World::scope(1),
                &abs("/a/f"),
                &World::staged("/a/f", 1, 0),
                &world.blob(0, 1, 0),
            )
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);

        let partial = World::new();
        overwrite_entry(&partial, PreserveMode::ReplaceBackup);
        partial
            .fs
            .rename(
                &World::scope(1),
                &abs("/a/f"),
                &partial.blob(0, 1, 0),
                RenameMode::NoReplace,
            )
            .unwrap();
        recover(&partial.ctx).unwrap();
        assert_rolled_back(&partial);
    }

    #[test]
    fn copy_block_is_restored_after_switch_and_new_content_kept_in_entry() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Copy);
        let scope = World::scope(1);
        world
            .fs
            .copy_file(&scope, &abs("/a/f"), &world.blob(0, 1, 0))
            .unwrap();
        world
            .fs
            .rename(
                &scope,
                &World::staged("/a/f", 1, 0),
                &abs("/a/f"),
                RenameMode::Replace,
            )
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
        assert_eq!(world.displaced(1, 0).as_deref(), Some(NEW));
    }

    #[test]
    fn target_equal_to_recorded_hash_is_kept() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        let mut before = world.before("/a/f", PreserveMode::Copy, 0);
        before.file_id = None;
        before.modified_ns = None;
        before.hash = Some(manifest::hash_bytes(OLD));
        world.write_entry(
            0,
            entry_of(
                1,
                vec![target(0, "/a/f", TargetAction::Modify, before)],
                vec![world.part(0)],
            ),
            prepared(),
            true,
        );
        world
            .fs
            .copy_file(&World::scope(1), &abs("/a/f"), &world.blob(0, 1, 0))
            .unwrap();
        recover(&world.ctx).unwrap();
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
        assert_eq!(world.displaced(1, 0), None);
        assert!(world.exists(&world.blob(0, 1, 0)));
    }

    #[test]
    fn staged_files_are_removed() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Copy);
        assert!(world.exists(&World::staged("/a/f", 1, 0)));
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
    }

    #[test]
    fn started_rollback_is_completed_after_each_restored_target() {
        for restored in 0..=2_u32 {
            let world = World::new();
            world.fs.seed_file(&abs("/a/f0"), b"zero").unwrap();
            world.fs.seed_file(&abs("/a/f1"), b"one").unwrap();
            let targets = (0..2)
                .map(|n| {
                    let path = format!("/a/f{n}");
                    target(
                        n,
                        &path,
                        TargetAction::Delete,
                        world.before(&path, PreserveMode::Moved, n),
                    )
                })
                .collect();
            world.write_entry(
                0,
                entry_of(1, targets, vec![world.part(0)]),
                StateRecord::rolling_back(UndoneCause::RolledBack),
                true,
            );
            let scope = World::scope(1);
            for n in 0..2 {
                let path = abs(&format!("/a/f{n}"));
                world
                    .fs
                    .rename(&scope, &path, &world.blob(0, 1, n), RenameMode::NoReplace)
                    .unwrap();
            }
            for n in (2 - restored)..2 {
                let path = abs(&format!("/a/f{n}"));
                world
                    .fs
                    .rename(&scope, &world.blob(0, 1, n), &path, RenameMode::NoReplace)
                    .unwrap();
            }
            let report = recover(&world.ctx).unwrap();
            assert_eq!(report.completed_rollbacks, vec![OpId(1)], "{restored}");
            assert_eq!(world.read("/a/f0").as_deref(), Some(&b"zero"[..]));
            assert_eq!(world.read("/a/f1").as_deref(), Some(&b"one"[..]));
            assert_eq!(
                world.state(0, 1),
                StateRecord::undone(UndoneCause::RolledBack)
            );
        }
    }

    /// Committed entry 1 deleting `/a/f0` and `/a/f1` by moving them into the
    /// entry, with a marker.
    fn committed_deletion(world: &World) -> nurt_contracts::fs::ContentHash {
        world.fs.seed_file(&abs("/a/f0"), b"zero").unwrap();
        world.fs.seed_file(&abs("/a/f1"), b"one").unwrap();
        let targets = (0..2)
            .map(|n| {
                let path = format!("/a/f{n}");
                let mut record = target(
                    n,
                    &path,
                    TargetAction::Delete,
                    world.before(&path, PreserveMode::Moved, n),
                );
                record.after = Some(AfterRecord::default());
                record
            })
            .collect();
        let digest = world.write_entry(
            0,
            entry_of(1, targets, vec![world.part(0)]),
            StateRecord::new(EntryState::Committed),
            true,
        );
        let scope = World::scope(1);
        for n in 0..2 {
            let path = abs(&format!("/a/f{n}"));
            world
                .fs
                .rename(&scope, &path, &world.blob(0, 1, n), RenameMode::NoReplace)
                .unwrap();
        }
        digest
    }

    #[test]
    fn released_items_are_reconciled_after_interrupted_release() {
        let world = World::new();
        committed_deletion(&world);
        let entry = world.journal().entry(OpId(1)).unwrap();
        let items = Items {
            items: vec![ItemState {
                index: TargetIndex(0),
                released: true,
                undone_by: None,
            }],
        };
        world
            .fs
            .seed_file(&entry.items().unwrap(), &items.encode().unwrap())
            .unwrap();
        world
            .fs
            .remove_file(&World::scope(1), &world.blob(0, 1, 1))
            .unwrap();
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert!(!world.exists(&world.blob(0, 1, 0)));
        let items = manifest::read_items(&*world.fs, &entry.items().unwrap()).unwrap();
        assert!(items.items.iter().all(|item| item.released));
        assert_eq!(items.items.len(), 2);
        assert_eq!(world.state(0, 1).state, EntryState::Partial);
        assert_eq!(outcome.index_appended, vec![OpId(1)]);
        assert!(!world.marked(0, 1));
    }

    #[test]
    fn damaged_item_state_is_left_for_salvage() {
        let world = World::new();
        committed_deletion(&world);
        let entry = world.journal().entry(OpId(1)).unwrap();
        world
            .fs
            .seed_file(&entry.items().unwrap(), b"{\"items\":[]}\ncrc32 00000000\n")
            .unwrap();
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.items_damaged, vec![OpId(1)]);
        assert!(world.exists(&world.blob(0, 1, 0)));
        assert!(world.exists(&world.blob(0, 1, 1)));
        assert_eq!(world.state(0, 1).state, EntryState::Committed);
    }

    /// Entry 1 moving `/a/f` and `/b/g` into the journals of their volumes.
    fn two_volume_entry(world: &World, states: [StateRecord; 2]) {
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        world.fs.seed_file(&abs("/b/g"), OLD).unwrap();
        let parts = vec![world.part(0), world.part(1)];
        for (journal, path) in ["/a/f", "/b/g"].into_iter().enumerate() {
            let mut record = target(
                0,
                path,
                TargetAction::Delete,
                world.before(path, PreserveMode::Moved, 0),
            );
            record.after = Some(AfterRecord::default());
            world.write_entry(
                journal,
                entry_of(1, vec![record], parts.clone()),
                states[journal],
                true,
            );
            world
                .fs
                .rename(
                    &World::scope(1),
                    &abs(path),
                    &world.blob(journal, 1, 0),
                    RenameMode::NoReplace,
                )
                .unwrap();
        }
    }

    #[test]
    fn multi_volume_entry_is_completed_when_any_part_is_committed() {
        let world = World::new();
        two_volume_entry(
            &world,
            [prepared(), StateRecord::new(EntryState::Committed)],
        );
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(world.state(0, 1).state, EntryState::Committed);
        assert_eq!(world.state(1, 1).state, EntryState::Committed);
        assert_eq!(outcome.rolled_forward, vec![OpId(1)]);
        let rows = manifest::read_index(&*world.fs, &world.ctx.state.index().unwrap()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].parts.len(), 2);
        assert!(!world.exists(&abs("/a/f")));
        assert!(!world.exists(&abs("/b/g")));
    }

    #[test]
    fn multi_volume_entry_is_rolled_back_when_no_part_is_committed() {
        let world = World::new();
        two_volume_entry(&world, [prepared(), prepared()]);
        recover(&world.ctx).unwrap();
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
        assert_eq!(world.read("/b/g").as_deref(), Some(OLD));
        assert_eq!(world.state(0, 1), StateRecord::undone(UndoneCause::Crash));
        assert_eq!(world.state(1, 1), StateRecord::undone(UndoneCause::Crash));
    }

    #[test]
    fn interrupted_undo_entry_is_rolled_back_and_source_marks_cleared() {
        let world = World::new();
        committed_deletion(&world);
        let source = world.journal().entry(OpId(1)).unwrap();
        world
            .fs
            .remove_file(&World::scope(1), &world.journal().pending(OpId(1)).unwrap())
            .unwrap();
        let marks = Items {
            items: vec![ItemState {
                index: TargetIndex(0),
                released: false,
                undone_by: Some(OpId(2)),
            }],
        };
        world
            .fs
            .seed_file(&source.items().unwrap(), &marks.encode().unwrap())
            .unwrap();
        world
            .fs
            .seed_file(&abs("/a/f0"), b"restored by undo")
            .unwrap();
        let mut undo = entry_of(2, Vec::new(), vec![world.part(0)]);
        undo.kind = OpKind::undo();
        undo.undo_of = vec![OpId(1)];
        let created = absent(PreserveMode::Created);
        undo.targets = vec![target(0, "/a/f0", TargetAction::Create, created)];
        world.write_entry(0, undo, prepared(), true);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.rolled_back, vec![OpId(2)]);
        assert!(!world.exists(&abs("/a/f0")));
        assert_eq!(world.state(0, 1).state, EntryState::Committed);
        let marks = manifest::read_items(&*world.fs, &source.items().unwrap()).unwrap();
        assert_eq!(marks.items[0].undone_by, None);
        assert_eq!(
            world.displaced(2, 0).as_deref(),
            Some(&b"restored by undo"[..])
        );
    }

    #[test]
    fn missing_index_row_is_appended_once_and_the_fork_is_reported() {
        let world = World::new();
        let index = world.ctx.state.index().unwrap();
        let first = committed_deletion(&world);
        let part = world.part(0);
        let row = |op: u64, digest| IndexRow {
            op: OpId(op),
            time: WallTime(1),
            kind: OpKind::command("write"),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: vec![IndexPart {
                vol: part.vol.clone(),
                dir: part.dir.clone(),
                manifest_sha256: digest,
            }],
        };
        let maintenance = store::scope(OpId(1), ScopePurpose::Maintenance);
        manifest::append_index_row(&*world.fs, &maintenance, &index, &row(1, first)).unwrap();
        let mut unindexed = entry_of(2, Vec::new(), vec![part.clone()]);
        unindexed.integrity.chain = Some(first);
        let unindexed_digest =
            world.write_entry(0, unindexed, StateRecord::new(EntryState::Committed), true);
        let mut later = entry_of(3, Vec::new(), vec![part.clone()]);
        later.integrity.chain = Some(first);
        let later_digest =
            world.write_entry(0, later, StateRecord::new(EntryState::Committed), false);
        manifest::append_index_row(&*world.fs, &maintenance, &index, &row(3, later_digest))
            .unwrap();

        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.index_appended, vec![OpId(2)]);
        world
            .fs
            .seed_file(&world.journal().pending(OpId(2)).unwrap(), b"")
            .unwrap();
        let again = recover_detailed(&world.ctx).unwrap();
        assert!(again.index_appended.is_empty());
        let rows = manifest::read_index(&*world.fs, &index).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.op.0).collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        assert_eq!(rows[2].digest(), Some(unindexed_digest));
    }

    #[test]
    fn part_state_tells_a_rollback_after_the_commit_from_one_before_it() {
        use super::{PartState, part_state};

        let world = World::new();
        let part = world.part(0);
        let cases = [
            (
                1,
                StateRecord::undone(UndoneCause::ValidateFailed),
                PartState::NotCommitted,
            ),
            (
                2,
                StateRecord::undone_after_commit(UndoneCause::ValidateFailed),
                PartState::Committed,
            ),
            (
                3,
                StateRecord::undone(UndoneCause::RolledBack),
                PartState::NotCommitted,
            ),
            (
                4,
                StateRecord::undone_after_commit(UndoneCause::RolledBack),
                PartState::Committed,
            ),
            (
                5,
                StateRecord::undone(UndoneCause::VerifyFailed),
                PartState::NotCommitted,
            ),
            (
                6,
                StateRecord::undone(UndoneCause::Undo),
                PartState::Committed,
            ),
        ];
        for (op, state, expected) in cases {
            let entry = entry_of(op, Vec::new(), vec![part.clone()]);
            world.write_entry(0, entry, state, false);
            assert_eq!(
                part_state(&*world.fs, world.journal(), OpId(op)),
                expected,
                "{state:?}"
            );
        }
    }

    #[test]
    fn index_row_of_an_entry_of_another_account_is_left_to_that_account() {
        let world = World::new();
        let index = world.ctx.state.index().unwrap();
        let part = world.part(0);
        let mut foreign = entry_of(1, Vec::new(), vec![part.clone()]);
        foreign.account = Some(nurt_testkit::paths::abs("/home/other/.local/state/nurt"));
        world.write_entry(0, foreign, StateRecord::new(EntryState::Committed), true);
        let mut own = entry_of(2, Vec::new(), vec![part]);
        own.account = Some(world.ctx.state.account_id());
        world.write_entry(0, own, StateRecord::new(EntryState::Committed), true);

        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.index_appended, vec![OpId(2)]);
        assert_eq!(outcome.unresolved, vec![OpId(1)]);
        assert!(world.marked(0, 1));
        assert!(!world.marked(0, 2));
        let rows = manifest::read_index(&*world.fs, &index).unwrap();
        assert_eq!(rows.iter().map(|row| row.op.0).collect::<Vec<_>>(), [2]);
    }

    fn tamper_manifest(world: &World, op: u64) {
        let path = world.journal().entry(OpId(op)).unwrap().manifest().unwrap();
        let mut bytes = manifest::read_bytes(&*world.fs, &path).unwrap();
        let text = String::from_utf8(bytes.clone())
            .unwrap()
            .replace("/a/f", "/a/x");
        bytes = text.into_bytes();
        let mut handle = world.fs.open_in_place(&World::scope(op), &path).unwrap();
        handle.set_length(0).unwrap();
        handle.write_all(&bytes).unwrap();
    }

    #[test]
    fn committed_entry_with_mismatching_digest_becomes_corrupt_with_data_kept() {
        let world = World::new();
        committed_deletion(&world);
        tamper_manifest(&world, 1);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.state(0, 1).state, EntryState::Corrupt);
        assert!(world.exists(&world.blob(0, 1, 0)));
        assert!(world.exists(&world.blob(0, 1, 1)));
    }

    #[test]
    fn prepared_entry_with_mismatching_digest_is_kept_as_found() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let scope = World::scope(1);
        let staged = World::staged("/a/f", 1, 0);
        world.fs.exchange(&scope, &abs("/a/f"), &staged).unwrap();
        world
            .fs
            .rename(&scope, &staged, &world.blob(0, 1, 0), RenameMode::NoReplace)
            .unwrap();
        tamper_manifest(&world, 1);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.read("/a/f").as_deref(), Some(NEW));
        assert_eq!(
            manifest::read_bytes(&*world.fs, &world.blob(0, 1, 0)).unwrap(),
            OLD
        );
    }

    #[test]
    fn garbage_state_marks_entry_corrupt() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let state = world.journal().entry(OpId(1)).unwrap().state().unwrap();
        let mut handle = world.fs.open_in_place(&World::scope(1), &state).unwrap();
        handle.set_length(0).unwrap();
        handle.write_all(b"\xffwhat").unwrap();
        drop(handle);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
    }

    #[test]
    fn previous_content_that_cannot_be_found_marks_entry_corrupt_without_changes() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        world
            .fs
            .remove_file(&World::scope(1), &abs("/a/f"))
            .unwrap();
        world.fs.seed_file(&abs("/a/f"), NEW).unwrap();
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.read("/a/f").as_deref(), Some(NEW));
        assert!(world.exists(&World::staged("/a/f", 1, 0)));
    }

    #[test]
    fn entry_locked_by_live_transaction_is_untouched() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let lock = world
            .fs
            .lock_exclusive(
                &world.journal().entry_lock(OpId(1)).unwrap(),
                Duration::ZERO,
            )
            .unwrap();
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.live, vec![OpId(1)]);
        assert_eq!(world.state(0, 1).state, EntryState::Prepared);
        assert!(world.marked(0, 1));
        assert!(world.exists(&World::staged("/a/f", 1, 0)));
        drop(lock);
        recover(&world.ctx).unwrap();
        assert_rolled_back(&world);
    }

    /// Two prepared entries: 1 overwrites `/a/f` by exchange, 2 deletes
    /// `/a/g` by move.
    fn two_entries(world: &World) {
        overwrite_entry(world, PreserveMode::Exchange);
        let scope = World::scope(1);
        let staged = World::staged("/a/f", 1, 0);
        world.fs.exchange(&scope, &abs("/a/f"), &staged).unwrap();
        world
            .fs
            .rename(&scope, &staged, &world.blob(0, 1, 0), RenameMode::NoReplace)
            .unwrap();
        world.fs.seed_file(&abs("/a/g"), b"g").unwrap();
        let before = world.before("/a/g", PreserveMode::Moved, 0);
        world.write_entry(
            0,
            entry_of(
                2,
                vec![target(0, "/a/g", TargetAction::Delete, before)],
                vec![world.part(0)],
            ),
            prepared(),
            true,
        );
        world
            .fs
            .rename(
                &World::scope(2),
                &abs("/a/g"),
                &world.blob(0, 2, 0),
                RenameMode::NoReplace,
            )
            .unwrap();
    }

    #[test]
    fn interrupted_recovery_reaches_the_same_final_state() {
        let reference = World::new();
        two_entries(&reference);
        recover(&reference.ctx).unwrap();
        let expected = reference.fs.snapshot(&abs("/a")).unwrap();

        let crash_rules = [
            (FailPoint::RecoveryStarted, None),
            (FailPoint::RecoveryEntry, Some(1)),
            (FailPoint::RecoveryEntry, Some(2)),
        ];
        for (point, n) in crash_rules {
            let mut world = World::new();
            two_entries(&world);
            let rule = FailRule {
                point,
                n,
                occurrence: 0,
                action: FailAction::Crash,
            };
            world.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(7, rule)));
            assert!(recover(&world.ctx).is_err(), "{point:?}");
            world.ctx.failpoints = Arc::new(NoFailpoints);
            recover(&world.ctx).unwrap();
            let found = world.fs.snapshot(&abs("/a")).unwrap();
            assert!(
                expected.diff(&found).is_empty(),
                "{point:?}: {:?}",
                expected.diff(&found)
            );
        }

        let world = World::new();
        two_entries(&world);
        world
            .fs
            .inject(MemFault::once(
                world.blob(0, 2, 0),
                "rename",
                FsErrorKind::Io,
            ))
            .unwrap();
        let error = recover(&world.ctx).unwrap_err();
        assert_eq!(error.details["deferred"], serde_json::json!([2]));
        assert_eq!(
            world.state(0, 2),
            StateRecord::rolling_back(UndoneCause::Crash)
        );
        recover(&world.ctx).unwrap();
        let found = world.fs.snapshot(&abs("/a")).unwrap();
        assert!(
            expected.diff(&found).is_empty(),
            "{:?}",
            expected.diff(&found)
        );
    }

    #[test]
    fn first_transaction_waits_for_one_recovery_pass() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| ensure_recovered(&world.ctx).unwrap());
            }
        });
        assert_rolled_back(&world);
        world.fs.seed_file(&abs("/a/g"), b"g").unwrap();
        let before = world.before("/a/g", PreserveMode::Moved, 0);
        world.write_entry(
            0,
            entry_of(
                2,
                vec![target(0, "/a/g", TargetAction::Delete, before)],
                vec![world.part(0)],
            ),
            prepared(),
            true,
        );
        ensure_recovered(&world.ctx).unwrap();
        assert!(world.marked(0, 2));
        assert_eq!(world.ctx.startup.get().map(Result::is_ok), Some(true));
    }

    #[test]
    fn unreadable_journal_fails_the_gate_for_every_caller() {
        let world = World::new();
        world
            .fs
            .deny_access(&world.journal().pending_dir().unwrap())
            .unwrap();
        world
            .fs
            .deny_access(&world.journal().ops_dir().unwrap())
            .unwrap();
        assert!(ensure_recovered(&world.ctx).is_err());
        world.fs.clear_faults();
        assert!(ensure_recovered(&world.ctx).is_err());
    }

    #[test]
    fn committed_entries_without_markers_are_not_read() {
        let world = World::new();
        for op in 10..100_010 {
            world
                .fs
                .seed_dir(world.journal().entry(OpId(op)).unwrap().dir())
                .unwrap();
        }
        overwrite_entry(&world, PreserveMode::Exchange);
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.states_read, 1);
        assert!(outcome.report.corrupt.is_empty());
        assert!(!world.exists(&world.journal().entry(OpId(500)).unwrap().state().unwrap()));
        assert_rolled_back(&world);
    }

    #[test]
    fn unreadable_markers_trigger_a_full_scan_that_completes_rollbacks_only() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/g"), b"g").unwrap();
        let before = world.before("/a/g", PreserveMode::Moved, 0);
        world.write_entry(
            0,
            entry_of(
                1,
                vec![target(0, "/a/g", TargetAction::Delete, before)],
                vec![world.part(0)],
            ),
            StateRecord::rolling_back(UndoneCause::RolledBack),
            false,
        );
        world
            .fs
            .rename(
                &World::scope(1),
                &abs("/a/g"),
                &world.blob(0, 1, 0),
                RenameMode::NoReplace,
            )
            .unwrap();
        world.fs.seed_file(&abs("/a/h"), b"h").unwrap();
        let before = world.before("/a/h", PreserveMode::Moved, 0);
        world.write_entry(
            0,
            entry_of(
                2,
                vec![target(0, "/a/h", TargetAction::Delete, before)],
                vec![world.part(0)],
            ),
            prepared(),
            false,
        );
        let unfinished = world.journal().entry_tmp(OpId(3)).unwrap();
        world
            .fs
            .seed_file(&unfinished.manifest().unwrap(), b"{}")
            .unwrap();
        world
            .fs
            .deny_access(&world.journal().pending_dir().unwrap())
            .unwrap();
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.full_scans.len(), 1);
        assert_eq!(outcome.full_scans[0].entries, 2);
        assert_eq!(outcome.report.completed_rollbacks, vec![OpId(1)]);
        assert_eq!(outcome.report.rolled_back, vec![OpId(3)]);
        assert_eq!(outcome.unresolved, vec![OpId(2)]);
        assert_eq!(world.read("/a/g").as_deref(), Some(&b"g"[..]));
        assert_eq!(world.state(0, 2).state, EntryState::Prepared);
    }

    #[test]
    fn journal_with_unknown_format_version_is_untouched() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        world
            .fs
            .seed_file(&world.journal().version().unwrap(), b"nurt-journal 9\n")
            .unwrap();
        let outcome = recover_detailed(&world.ctx).unwrap();
        assert_eq!(outcome.foreign, vec![world.journal().root().clone()]);
        assert_eq!(world.state(0, 1).state, EntryState::Prepared);
    }

    #[test]
    fn metadata_append_and_rename_targets_are_rolled_back() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/m"), b"m").unwrap();
        world.fs.seed_file(&abs("/a/log"), b"abc").unwrap();
        world.fs.seed_file(&abs("/a/src"), b"source").unwrap();
        world.fs.seed_file(&abs("/a/dst"), b"destination").unwrap();
        let entry = world.journal().entry(OpId(1)).unwrap();
        let _ = world.fs.seed_dir(&entry.blobs().unwrap());
        let meta = world
            .fs
            .extended_metadata(&abs("/a/m"), Follow::NoFollow)
            .unwrap();
        let meta_id = BlobId::new(TargetIndex(0), BlobRole::Previous);
        world
            .fs
            .seed_file(
                &entry.blob_meta(meta_id).unwrap(),
                &serde_json::to_vec(&meta).unwrap(),
            )
            .unwrap();
        let mut appended = world.before("/a/log", PreserveMode::Append, 1);
        appended.hash = Some(manifest::hash_bytes(b"abc"));
        let mut renamed = target(
            2,
            "/a/src",
            TargetAction::Rename {
                to: abs("/a/dst"),
                replace: true,
            },
            world.before("/a/src", PreserveMode::Rename, 2),
        );
        let mut replaced = world.before("/a/dst", PreserveMode::Moved, 2);
        replaced.blob = Some(BlobId::new(TargetIndex(2), BlobRole::Destination).content_name());
        renamed.replaced = Some(replaced);
        let targets = vec![
            target(
                0,
                "/a/m",
                TargetAction::Attributes,
                world.before("/a/m", PreserveMode::Meta, 0),
            ),
            target(1, "/a/log", TargetAction::Append, appended),
            renamed,
        ];
        world.write_entry(
            0,
            entry_of(1, targets, vec![world.part(0)]),
            prepared(),
            true,
        );
        let scope = World::scope(1);
        let change = MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        };
        world.fs.set_meta(&scope, &abs("/a/m"), &change).unwrap();
        let mut log = world.fs.open_in_place(&scope, &abs("/a/log")).unwrap();
        log.seek(std::io::SeekFrom::End(0)).unwrap();
        log.write_all(b"def").unwrap();
        drop(log);
        let destination_blob = entry
            .blob(BlobId::new(TargetIndex(2), BlobRole::Destination))
            .unwrap();
        world
            .fs
            .rename(
                &scope,
                &abs("/a/dst"),
                &destination_blob,
                RenameMode::NoReplace,
            )
            .unwrap();
        world
            .fs
            .rename(
                &scope,
                &abs("/a/src"),
                &abs("/a/dst"),
                RenameMode::NoReplace,
            )
            .unwrap();

        recover(&world.ctx).unwrap();
        let mode = world
            .fs
            .metadata(&abs("/a/m"), Follow::NoFollow)
            .unwrap()
            .mode;
        assert_eq!(mode & 0o777, meta.basic.mode & 0o777);
        assert_eq!(world.read("/a/log").as_deref(), Some(&b"abc"[..]));
        assert_eq!(world.displaced(1, 1).as_deref(), Some(&b"abcdef"[..]));
        assert_eq!(world.read("/a/src").as_deref(), Some(&b"source"[..]));
        assert_eq!(world.read("/a/dst").as_deref(), Some(&b"destination"[..]));
        assert_eq!(world.state(0, 1), StateRecord::undone(UndoneCause::Crash));
    }

    #[test]
    fn provider_value_and_created_target_are_rolled_back() {
        let world = World::new();
        let key = ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["HKCU".to_owned(), "Software".to_owned()],
            value: Some("Setting".to_owned()),
        };
        let snapshot = ProviderSnapshot {
            key: key.clone(),
            existed: false,
            value: None,
            subtree: None,
        };
        let entry = world.journal().entry(OpId(1)).unwrap();
        let _ = world.fs.seed_dir(&entry.blobs().unwrap());
        let blob_id = BlobId::new(TargetIndex(0), BlobRole::Previous);
        world
            .fs
            .seed_file(
                &entry.blob(blob_id).unwrap(),
                &serde_json::to_vec(&snapshot).unwrap(),
            )
            .unwrap();
        let mut value = target(
            0,
            "/a/unused",
            TargetAction::Modify,
            BeforeRecord {
                blob: Some(blob_id.content_name()),
                ..absent(PreserveMode::Value)
            },
        );
        value.path = TargetPath::Provider(key.clone());
        let created = absent(PreserveMode::Created);
        let targets = vec![value, target(1, "/a/new", TargetAction::Create, created)];
        world.write_entry(
            0,
            entry_of(1, targets, vec![world.part(0)]),
            prepared(),
            true,
        );
        world
            .registry
            .values
            .lock()
            .unwrap()
            .insert(key.clone(), ProviderValue::Text("changed".to_owned()));
        world.fs.seed_file(&abs("/a/new"), b"created").unwrap();
        recover(&world.ctx).unwrap();
        assert!(world.registry.values.lock().unwrap().get(&key).is_none());
        assert!(!world.exists(&abs("/a/new")));
        assert_eq!(world.displaced(1, 1).as_deref(), Some(&b"created"[..]));
    }

    #[test]
    fn superfluous_marker_of_terminal_entry_is_removed() {
        let world = World::new();
        world.write_entry(
            0,
            entry_of(1, Vec::new(), vec![world.part(0)]),
            StateRecord::undone(UndoneCause::Undo),
            true,
        );
        recover(&world.ctx).unwrap();
        assert!(!world.marked(0, 1));
        assert_eq!(world.state(0, 1), StateRecord::undone(UndoneCause::Undo));
        let kind = world
            .fs
            .metadata(
                world.journal().entry(OpId(1)).unwrap().dir(),
                Follow::NoFollow,
            )
            .unwrap()
            .kind;
        assert_eq!(kind, FileKind::Directory);
    }

    #[test]
    fn prepared_entry_without_a_manifest_becomes_corrupt_without_changes() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let manifest_path = world.journal().entry(OpId(1)).unwrap().manifest().unwrap();
        world
            .fs
            .remove_file(&World::scope(1), &manifest_path)
            .unwrap();
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.state(0, 1).state, EntryState::Corrupt);
        assert!(!world.marked(0, 1));
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
        assert!(world.exists(&World::staged("/a/f", 1, 0)));
    }

    #[test]
    fn prepared_entry_with_a_truncated_manifest_becomes_corrupt_without_changes() {
        let world = World::new();
        overwrite_entry(&world, PreserveMode::Exchange);
        let manifest_path = world.journal().entry(OpId(1)).unwrap().manifest().unwrap();
        let mut handle = world
            .fs
            .open_in_place(&World::scope(1), &manifest_path)
            .unwrap();
        handle.set_length(0).unwrap();
        handle.write_all(b"{\"v\":").unwrap();
        drop(handle);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.state(0, 1).state, EntryState::Corrupt);
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
        assert!(world.exists(&World::staged("/a/f", 1, 0)));
    }

    #[test]
    fn started_rollback_with_a_damaged_manifest_becomes_corrupt() {
        let world = World::new();
        world.fs.seed_file(&abs("/a/f"), OLD).unwrap();
        let before = world.before("/a/f", PreserveMode::Exchange, 0);
        world.write_entry(
            0,
            entry_of(
                1,
                vec![target(0, "/a/f", TargetAction::Modify, before)],
                vec![world.part(0)],
            ),
            StateRecord::rolling_back(UndoneCause::Crash),
            true,
        );
        tamper_manifest(&world, 1);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert!(report.completed_rollbacks.is_empty());
        assert_eq!(world.state(0, 1).state, EntryState::Corrupt);
        assert_eq!(world.read("/a/f").as_deref(), Some(OLD));
    }

    /// Prepared entry 1 that appended to `/a/log`, which held `abc`.
    fn append_entry(world: &World) {
        world.fs.seed_file(&abs("/a/log"), b"abc").unwrap();
        let before = world.before("/a/log", PreserveMode::Append, 0);
        world.write_entry(
            0,
            entry_of(
                1,
                vec![target(0, "/a/log", TargetAction::Append, before)],
                vec![world.part(0)],
            ),
            prepared(),
            true,
        );
    }

    #[test]
    fn append_rollback_of_an_unchanged_file_changes_nothing() {
        let world = World::new();
        append_entry(&world);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.rolled_back, vec![OpId(1)]);
        assert_eq!(world.read("/a/log").as_deref(), Some(&b"abc"[..]));
        assert_eq!(world.displaced(1, 0), None);
        assert_eq!(world.state(0, 1), StateRecord::undone(UndoneCause::Crash));
    }

    #[test]
    fn append_rollback_of_a_shortened_file_marks_the_entry_corrupt() {
        let world = World::new();
        append_entry(&world);
        let mut handle = world
            .fs
            .open_in_place(&World::scope(1), &abs("/a/log"))
            .unwrap();
        handle.set_length(2).unwrap();
        drop(handle);
        let report = recover(&world.ctx).unwrap();
        assert_eq!(report.corrupt, vec![OpId(1)]);
        assert_eq!(world.read("/a/log").as_deref(), Some(&b"ab"[..]));
        assert_eq!(world.state(0, 1).state, EntryState::Corrupt);
    }

    #[test]
    fn repeated_recovery_changes_nothing() {
        let world = World::new();
        two_entries(&world);
        recover(&world.ctx).unwrap();
        let first = world.fs.snapshot(&abs("/a")).unwrap();
        let again = recover(&world.ctx).unwrap();
        assert!(again.rolled_back.is_empty(), "{again:?}");
        assert!(again.corrupt.is_empty(), "{again:?}");
        let second = world.fs.snapshot(&abs("/a")).unwrap();
        assert!(first.diff(&second).is_empty(), "{:?}", first.diff(&second));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
