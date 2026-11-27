// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mutation transaction implementation.
//!
//! Protocol of one operation:
//!
//! 1. The number is assigned at the first `preserve` (or at commit for a
//!    transaction of markers only), under the account lock.
//! 2. Every declared target is inspected and a preserve mode is chosen:
//!    volume eligibility, liveness of the space service, admission with the
//!    bytes the journal would additionally hold, and the overwrite plan.
//! 3. For every volume part: the entry lock is taken, the marker
//!    `pending/<N>` is made durable, the entry is written in `ops/<N>.tmp/`
//!    (manifest and `STATE = prepared`), flushed and renamed to `ops/<N>/`.
//! 4. The previous content of every target that is cloned or copied, and the
//!    previous metadata, are preserved before `preserve` returns. Content
//!    kept by moving or exchanging names is preserved atomically with its
//!    change. In a `preserve` call with many targets the content of small
//!    files is kept in the manifest, which is then written again durably.
//! 5. New content is written to `.nurt-tmp-<op>-<n>` in the directory of the
//!    target, receives the previous metadata, is flushed, the expected state
//!    is checked and the file is switched in. In a transaction with many
//!    targets the flush of the new content and of the directories is deferred
//!    to one `sync_paths` call that precedes every other kind of change and the
//!    commit; see [`Unsynced`] for why this is safe.
//! 6. At commit, under the account lock: the final manifests of all parts
//!    are written, then `STATE = committed` of every part, then the index
//!    row; then the markers are removed and the entry locks released.
//! 7. A failure in steps 4 and 5 writes `STATE = rolling_back`, restores the
//!    changed targets in reverse order, removes staged files and writes
//!    `STATE = undone (rolled_back)`; no new entry is created.
//!
//! A failure of the journal itself never fails the operation: the affected
//! targets are changed without preservation and carry the irreversible
//! reason `journal_error`.

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Seek, SeekFrom};

use nurt_contracts::config::{AgeLimit, ByteLimit, Oversize, StepLimit};
use nurt_contracts::env::ClockStamp;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, ContentHash, Durability, ExtendedMeta, FileKind, FileLock, Follow, FsError,
    FsErrorKind, FsResult, ProviderSnapshot, RenameMode, ScopePurpose, TxScope, VolumeInfo,
    WriteHandle,
};
use nurt_contracts::journal::{
    AdmissionDecision, AdmissionRequest, Applied, BeginRequest, CloneMode, EffectiveRetention,
    Eligibility, ExemptWrite, Expect, ExternalChange, JournalNotable, Mutation, MutationTx,
    OpHandle, OpId, PreserveMode, Preserved, RollbackReason, RollbackReport, ServiceLiveness,
    TargetAction, TargetIndex, TargetOutcome, TargetPath, TargetSpec, UndoScope, UndoneCause,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_fs::failpoint::FailPoint;
use sha2::{Digest, Sha256};

use crate::blob::{self, HeldBytes};
use crate::conflict;
use crate::layout::{self, BlobId, BlobRole, EntryLayout, JournalLayout};
use crate::manifest::{
    self, AfterRecord, BeforeRecord, Expires, IndexPart, IndexRow, Integrity, ItemRef, Manifest,
    PartRef, StateRecord, TargetRecord, TargetRole,
};
use crate::overwrite::{self, Exchange, Plan, Switch};
use crate::retention::expiry::{self, BudgetAdmission};
use crate::seq;
use crate::store::{self, Fault, JournalCtx, JournalLocation};

/// Permission bits of directories created inside a journal.
const JOURNAL_DIR_MODE: u32 = 0o700;

/// Attempts of one operation to take a number that another process has
/// already claimed in a shared journal before the journal is given up.
const MAX_NUMBER_RACES: u32 = 8;

/// Workers removing the directory of an entry that could not be written.
const REMOVE_WORKERS: usize = 1;

/// Number of targets of a transaction from which its changes are made
/// durable together: the flushes of the new content and of the directories
/// run once before the commit instead of once per target.
const BATCH_MIN_TARGETS: usize = 64;

/// Number of targets of one `preserve` call from which the previous content of
/// small files is kept in the manifest instead of in a block per file.
const INLINE_MIN_TARGETS: usize = 64;

/// Largest previous content kept in the manifest.
const INLINE_MAX_BYTES: u64 = 4096;

/// Write permission bit of the account of a file; a file without it cannot be
/// reopened for writing to flush it.
const ACCOUNT_WRITE: u32 = 0o200;

/// File flags that the kernel sets on its own (`FS_EXTENT_FL`) and that
/// restoring a file does not need to bring back.
const KERNEL_FILE_FLAGS: u64 = 0x0008_0000;

/// Opens a transaction of a user operation.
pub(crate) fn begin(ctx: &JournalCtx, request: BeginRequest) -> Box<dyn MutationTx + '_> {
    Box::new(Tx::new(ctx, request, Link::default()))
}

/// Opens a transaction that records an undo or redo.
pub(crate) fn begin_linked(ctx: &JournalCtx, request: BeginRequest, link: Link) -> Tx<'_> {
    Tx::new(ctx, request, link)
}

/// Relation of an undo or redo entry to its source entries.
#[derive(Default)]
pub(crate) struct Link {
    /// Entries undone.
    pub(crate) undo_of: Vec<OpId>,
    /// Undo entry redone.
    pub(crate) redo_of: Option<OpId>,
    /// Scope of the undo.
    pub(crate) scope: Option<UndoScope>,
    /// Items restored.
    pub(crate) items: Vec<ItemRef>,
    /// Metadata of the state restored by each target of the transaction, in
    /// the order of the targets; shorter than the targets when unknown.
    pub(crate) recorded: Vec<RecordedMeta>,
    /// Update of the source entries, run after the entry is durably committed
    /// and indexed and before its marker is removed; recovery repeats it
    /// idempotently when the marker remains.
    pub(crate) after_commit: Option<AfterCommit>,
}

/// Metadata recorded with the previous state of a target that an undo or
/// redo brings back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecordedMeta {
    /// Block with the extended metadata kept with the previous content.
    pub(crate) blob: Option<AbsPath>,
    /// Metadata rebuilt from the record of a previous state kept inline.
    pub(crate) inline: Option<Box<ExtendedMeta>>,
    /// Modification time of the previous state, nanoseconds since the Unix
    /// epoch.
    pub(crate) modified_ns: Option<i64>,
}

/// Update of source entries after an undo entry is committed.
pub(crate) type AfterCommit = Box<dyn FnOnce(&JournalCtx, OpId) -> Result<()> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Open,
    Applying,
    Failed,
    Halted,
    Done,
}

/// One volume part of the entry.
struct Part {
    volume: VolumeInfo,
    layout: JournalLayout,
    entry: EntryLayout,
    lock: Option<Box<dyn FileLock>>,
    prepared: bool,
    broken: bool,
    digest: Option<ContentHash>,
}

/// Existing destination of a rename.
struct Dest {
    path: AbsPath,
    part: Option<usize>,
    record: TargetRecord,
    cross: bool,
    moved: bool,
    created: bool,
}

/// One declared target.
struct Target {
    spec: TargetSpec,
    record: TargetRecord,
    path: Option<AbsPath>,
    part: Option<usize>,
    old_meta: Option<ExtendedMeta>,
    plan: Option<Plan>,
    snapshot: Option<ProviderSnapshot>,
    staged: Option<AbsPath>,
    prefix: Option<Sha256>,
    dest: Option<Dest>,
    changed: bool,
    created: bool,
    copied_on_move: bool,
    observed: bool,
    /// The previous content is kept in the manifest.
    inline: bool,
}

impl Target {
    fn index(&self) -> TargetIndex {
        self.record.index
    }

    fn mode(&self) -> PreserveMode {
        self.record.before.store
    }

    fn degrade(&mut self, reason: Irreversible) {
        self.record.before.store = PreserveMode::None;
        self.record.before.blob = None;
        self.record.before.inline = None;
        self.inline = false;
        self.record.irreversible = Some(reason);
        self.record.bytes_held = 0;
    }

    fn fs_path(&self) -> Result<&AbsPath> {
        self.path
            .as_ref()
            .ok_or_else(|| misuse("the mutation needs a file system target"))
    }

    /// Irreversible reason of the target, or else of the existing
    /// destination it replaces.
    fn irreversible(&self) -> Option<Irreversible> {
        self.record
            .irreversible
            .or_else(|| self.dest.as_ref().and_then(|dest| dest.record.irreversible))
    }
}

/// Content of a write.
enum Content {
    Bytes(Vec<u8>),
    Staged,
    Copy { source: AbsPath, clone: CloneMode },
    At { offset: u64, bytes: Vec<u8> },
    Length(u64),
}

/// Mutation transaction.
pub(crate) struct Tx<'a> {
    ctx: &'a JournalCtx,
    request: BeginRequest,
    link: Link,
    phase: Phase,
    op: Option<OpId>,
    unavailable: Option<Irreversible>,
    parts: Vec<Part>,
    failed_volumes: Vec<(nurt_contracts::fs::VolumeId, Irreversible)>,
    targets: Vec<Target>,
    irreversible: Vec<IrreversibleEntry>,
    notable: Vec<JournalNotable>,
    exempt: Vec<ExemptWrite>,
    uuid: uuid::Uuid,
    stamp: ClockStamp,
    ast: serde_json::Value,
    ast_hash: ContentHash,
    external_since: Option<std::time::Duration>,
    /// Volumes for which entries were expired to make room for this
    /// operation.
    budgeted: Vec<nurt_contracts::fs::VolumeId>,
    /// Volumes of the directories of the targets seen so far.
    volumes: HashMap<AbsPath, VolumeInfo>,
    /// The targets declared by the running `preserve` call keep small
    /// previous content in the manifest.
    inline_batch: bool,
    /// Targets of the running `preserve` call that keep their previous
    /// content in the manifest and wait for one admission, with the bytes
    /// they add.
    inline_admission: Vec<(usize, u64)>,
    /// New content and directory changes not yet flushed.
    unsynced: Unsynced,
    /// Whether `preserve` is allowed after the first change.
    incremental: bool,
}

/// Changes of a transaction whose flush is deferred to one flush of all of
/// them before the commit.
///
/// A deferred change is safe because the previous state of its target is
/// durable before the change is made (the blob, the inline content or the
/// recorded file identity) and the entry is `prepared`: a crash before the
/// flush rolls the entry back, and recovery restores every target whose
/// state is not the recorded previous state, whatever part of the new content
/// reached the disk. Nothing is committed before the flush returned.
#[derive(Default)]
struct Unsynced {
    files: Vec<AbsPath>,
    dirs: Vec<AbsPath>,
    known_dirs: HashSet<AbsPath>,
}

impl Unsynced {
    fn file(&mut self, path: AbsPath) {
        self.files.push(path);
    }

    fn dir(&mut self, path: AbsPath) {
        if self.known_dirs.insert(path.clone()) {
            self.dirs.push(path);
        }
    }

    fn is_empty(&self) -> bool {
        self.files.is_empty() && self.dirs.is_empty()
    }
}

fn misuse(problem: &str) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!("journal protocol violated: {problem}"),
    )
    .with_problem(problem)
}

/// Failure of the journal while it moves a target it had agreed to keep: the
/// operation stops before anything is removed, so no data is lost.
fn journal_kept_nothing(index: TargetIndex, error: FsError) -> Error {
    target_error(index, error)
        .with_problem("the journal could not keep the previous content; nothing was removed")
}

fn rolled_back() -> Error {
    Error::new(
        ErrorCode::RolledBack,
        "the transaction was rolled back; start a new operation",
    )
}

fn halted() -> Error {
    Error::new(
        ErrorCode::Internal,
        "the transaction stopped at an injected crash",
    )
    .with_problem("injected_crash")
}

fn stale(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Stale,
        format!("{path} changed after it was read; read it again"),
    )
    .with_detail("path", serde_json::Value::from(path.to_string()))
}

fn target_error(index: TargetIndex, error: FsError) -> Error {
    Error::from(error).with_detail("target", serde_json::Value::from(index.0))
}

fn record_path(path: &TargetPath) -> String {
    match path {
        TargetPath::Fs(path) => path.to_string(),
        TargetPath::Provider(key) => {
            let mut text = format!("{}:{}", key.scheme.0, key.segments.join("/"));
            if let Some(value) = &key.value {
                text.push('/');
                text.push_str(value);
            }
            text
        }
    }
}

fn ast_hash(ast: &serde_json::Value) -> ContentHash {
    let stripped = nurt_contracts::ast::strip_for_hash(ast);
    let canonical = serde_json_canonicalizer::to_vec(&stripped).unwrap_or_default();
    manifest::hash_bytes(&canonical)
}

/// Whether the metadata of a file is fully described by the record of its
/// state before the change: no extended attributes, ACL, streams or
/// attributes beyond the ones the kernel sets itself. Only such a file keeps
/// its previous content in the manifest.
fn plain_meta(meta: &ExtendedMeta) -> bool {
    meta.xattrs.is_empty()
        && meta.acl.is_none()
        && meta.account_sid.is_none()
        && meta.streams.is_empty()
        && meta.link_target.is_none()
        && meta
            .flags
            .is_none_or(|flags| flags & !KERNEL_FILE_FLAGS == 0)
}

fn writes_content(action: &TargetAction) -> bool {
    matches!(
        action,
        TargetAction::Create
            | TargetAction::Modify
            | TargetAction::Truncate
            | TargetAction::Replace
            | TargetAction::Append
    )
}

fn must_exist(action: &TargetAction) -> bool {
    matches!(
        action,
        TargetAction::Delete
            | TargetAction::Attributes
            | TargetAction::Truncate
            | TargetAction::Replace
            | TargetAction::Rename { .. }
    )
}

fn must_be_absent(action: &TargetAction) -> bool {
    matches!(
        action,
        TargetAction::Create | TargetAction::CreateTree | TargetAction::Link
    )
}

/// Change applied to a provider target.
enum ProviderChange {
    Set(nurt_contracts::fs::ProviderValue),
    Remove,
    Copy(nurt_contracts::fs::ProviderKey),
    Restore(nurt_contracts::fs::ProviderSnapshot),
}

/// Whether a mutation of a provider target matches the declared action.
fn provider_accepts(action: &TargetAction, mutation: &Mutation) -> bool {
    match mutation {
        Mutation::ProviderSet(_) => matches!(action, TargetAction::Modify | TargetAction::Create),
        Mutation::ProviderRemove => matches!(action, TargetAction::Delete),
        Mutation::ProviderCopy(_) => matches!(action, TargetAction::Create),
        Mutation::ProviderRestore(_) => matches!(action, TargetAction::Modify),
        _ => false,
    }
}

fn accepts(action: &TargetAction, mutation: &Mutation, provider: bool) -> bool {
    if provider {
        return provider_accepts(action, mutation);
    }
    match mutation {
        Mutation::CommitStaged | Mutation::WriteBytes(_) | Mutation::CopyFrom { .. } => matches!(
            action,
            TargetAction::Create | TargetAction::Modify | TargetAction::Replace
        ),
        Mutation::AppendBytes(_) => matches!(action, TargetAction::Append),
        Mutation::WriteAt { .. } | Mutation::SetLength(_) => {
            matches!(action, TargetAction::Modify | TargetAction::Truncate)
        }
        Mutation::Remove => matches!(action, TargetAction::Delete | TargetAction::Replace),
        Mutation::Rename => matches!(action, TargetAction::Rename { .. }),
        Mutation::CreateDir { .. } => matches!(
            action,
            TargetAction::Create | TargetAction::CreateTree | TargetAction::Replace
        ),
        Mutation::CreateLink { .. } | Mutation::CreateHardLink { .. } => matches!(
            action,
            TargetAction::Create | TargetAction::Link | TargetAction::Replace
        ),
        Mutation::SetMeta(_) => matches!(action, TargetAction::Attributes),
        _ => false,
    }
}

fn after_record(
    ctx: &JournalCtx,
    path: &AbsPath,
    hash: Option<ContentHash>,
) -> FsResult<AfterRecord> {
    match ctx.fs.metadata(path, Follow::NoFollow) {
        Ok(meta) => Ok(AfterRecord {
            exists: true,
            kind: Some(meta.kind),
            size: Some(meta.size),
            mode: Some(meta.mode),
            hash,
            modified_ns: Some(meta.modified_ns),
        }),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(AfterRecord::default()),
        Err(error) => Err(error),
    }
}

impl<'a> Tx<'a> {
    fn new(ctx: &'a JournalCtx, request: BeginRequest, link: Link) -> Self {
        let ast = serde_json::to_value(&request.program).unwrap_or(serde_json::Value::Null);
        let unavailable = match ctx.startup.get() {
            Some(Err(_)) => Some(Irreversible::JournalError),
            _ => None,
        };
        Self {
            ctx,
            ast_hash: ast_hash(&ast),
            ast,
            request,
            link,
            phase: Phase::Open,
            op: None,
            unavailable,
            parts: Vec::new(),
            failed_volumes: Vec::new(),
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
            uuid: uuid::Uuid::new_v4(),
            stamp: ctx.clock.stamp(),
            external_since: None,
            budgeted: Vec::new(),
            volumes: HashMap::new(),
            inline_batch: false,
            inline_admission: Vec::new(),
            unsynced: Unsynced::default(),
            incremental: false,
        }
    }

    fn scope(&self) -> TxScope {
        store::scope(self.op.unwrap_or(OpId(0)), ScopePurpose::Operation)
    }

    fn journal_scope(&self) -> TxScope {
        store::scope(self.op.unwrap_or(OpId(0)), ScopePurpose::Maintenance)
    }

    fn usable(&self) -> Result<()> {
        match self.phase {
            Phase::Open | Phase::Applying => Ok(()),
            Phase::Halted => Err(halted()),
            Phase::Failed | Phase::Done => Err(rolled_back()),
        }
    }

    fn halt(&mut self) -> Error {
        self.phase = Phase::Halted;
        halted()
    }

    fn fail(&mut self, error: Error) -> Error {
        if self.phase == Phase::Halted {
            return error;
        }
        let outcome = self.restore_all(UndoneCause::RolledBack);
        self.phase = Phase::Failed;
        match outcome {
            Err(_) => self.halt(),
            Ok(report) if !report.failed.is_empty() => rollback_failed(&report.failed, error),
            Ok(_) => error,
        }
    }

    /// Assigns the operation number; `None` when the journal is unavailable
    /// for this operation.
    fn number(&mut self) -> std::result::Result<Option<OpId>, Fault> {
        if self.op.is_some() || self.unavailable.is_some() {
            return Ok(self.op);
        }
        let lock = match self.ctx.lock_account() {
            Ok(lock) => lock,
            Err(_) => {
                self.unavailable = Some(Irreversible::JournalError);
                return Ok(None);
            }
        };
        match self.allocate_unused() {
            Ok(op) => self.op = Some(op),
            Err(Fault::Halt) => return Err(Fault::Halt),
            Err(Fault::Fs(_)) => self.unavailable = Some(Irreversible::JournalError),
        }
        drop(lock);
        Ok(self.op)
    }

    /// Assigns a number that no active journal names yet.
    ///
    /// A journal directory can be shared by accounts that keep separate
    /// sequences. A number already named there is dropped and the sequence is
    /// rebuilt from the journals, so that an entry of another account is never
    /// overwritten or shadowed.
    fn allocate_unused(&self) -> std::result::Result<OpId, Fault> {
        let op = seq::allocate(self.ctx)?;
        if !self.named_in_journals(op) {
            return Ok(op);
        }
        let scope = store::scope(op, ScopePurpose::Maintenance);
        self.ctx.fs.remove_file(&scope, &self.ctx.state.seq()?)?;
        seq::allocate(self.ctx)
    }

    /// Whether an entry, a temporary entry, a marker, a reclaimed entry or a
    /// tombstone of `op` exists in an active journal.
    fn named_in_journals(&self, op: OpId) -> bool {
        self.ctx
            .active_journals()
            .iter()
            .any(|journal| self.named_in(journal, op))
    }

    /// Whether an entry, a temporary entry, a marker, a reclaimed entry or a
    /// tombstone of `op` exists in `journal`.
    fn named_in(&self, journal: &JournalLayout, op: OpId) -> bool {
        let fs = self.ctx.fs.as_ref();
        let names = [
            journal.entry(op).map(|entry| entry.dir().clone()),
            journal.entry_tmp(op).map(|entry| entry.dir().clone()),
            journal.reaped(op).map(|entry| entry.dir().clone()),
            journal.pending(op),
            journal.tomb(op),
        ];
        names
            .into_iter()
            .flatten()
            .any(|name| fs.metadata(&name, Follow::NoFollow).is_ok())
    }

    /// Replaces the number of an operation that has no open part yet with a
    /// number no active journal names; `None` when the account lock cannot be
    /// taken.
    fn renumber(&mut self) -> std::result::Result<Option<OpId>, Fault> {
        let Ok(lock) = self.ctx.lock_account() else {
            return Ok(None);
        };
        let op = self.allocate_unused()?;
        drop(lock);
        self.op = Some(op);
        Ok(Some(op))
    }

    /// Part of `volume`, opened on first use; `Err` names the reason why the
    /// journal cannot be used there.
    fn part_for(&mut self, volume: &VolumeInfo) -> std::result::Result<usize, Located> {
        if let Some(position) = self.parts.iter().position(|p| p.volume.id == volume.id) {
            return if self.parts[position].broken {
                Err(Located::Skip(Irreversible::JournalError))
            } else {
                Ok(position)
            };
        }
        if let Some((_, reason)) = self.failed_volumes.iter().find(|(id, _)| *id == volume.id) {
            return Err(Located::Skip(*reason));
        }
        let skip = |tx: &mut Self, reason: Irreversible| {
            tx.failed_volumes.push((volume.id.clone(), reason));
            Err(Located::Skip(reason))
        };
        if let Eligibility::Inactive(reason) = self.ctx.admission.eligibility(volume) {
            return skip(self, reason.irreversible());
        }
        if self.ctx.admission.liveness() == ServiceLiveness::Down {
            return skip(self, Irreversible::JournalInactive);
        }
        let dir = match self.ctx.journals.locate(volume) {
            JournalLocation::Active(dir) => dir,
            JournalLocation::Inactive(reason) => return skip(self, reason.irreversible()),
        };
        let Some(mut op) = self.op else {
            let reason = self.unavailable.unwrap_or(Irreversible::JournalError);
            return skip(self, reason);
        };
        // The number was checked against the journals known to the account.
        // A volume met for the first time registers its journal only now, so
        // a number another account already used there is replaced while no
        // part is open.
        //
        // Another process can still take the number between that check and
        // the creation of the marker, because accounts with separate state
        // directories share the journal of a volume and the marker is the
        // only exclusive claim. The loser takes a new number and tries again.
        let mut lost_races = 0;
        loop {
            if self.parts.is_empty() && self.named_in(&JournalLayout::new(dir.clone()), op) {
                match self.renumber() {
                    Ok(Some(renumbered)) => op = renumbered,
                    Ok(None) | Err(Fault::Fs(_)) => return skip(self, Irreversible::JournalError),
                    Err(Fault::Halt) => return Err(Located::Halt),
                }
            }
            match self.open_part(op, volume.clone(), dir.clone()) {
                Ok(part) => {
                    self.parts.push(part);
                    return Ok(self.parts.len() - 1);
                }
                Err(Fault::Halt) => return Err(Located::Halt),
                Err(Fault::Fs(error))
                    if error.kind == FsErrorKind::Exists
                        && self.parts.is_empty()
                        && lost_races < MAX_NUMBER_RACES =>
                {
                    lost_races += 1;
                    match self.renumber() {
                        Ok(Some(renumbered)) => op = renumbered,
                        Ok(None) | Err(Fault::Fs(_)) => {
                            return skip(self, Irreversible::JournalError);
                        }
                        Err(Fault::Halt) => return Err(Located::Halt),
                    }
                }
                Err(Fault::Fs(_)) => return skip(self, Irreversible::JournalError),
            }
        }
    }

    fn open_part(
        &self,
        op: OpId,
        volume: VolumeInfo,
        dir: AbsPath,
    ) -> std::result::Result<Part, Fault> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.journal_scope();
        let layout = JournalLayout::new(dir);
        check_version(ctx, &scope, &layout)?;
        let mut created = false;
        for path in layout.structure()? {
            match fs.create_dir(&scope, &path, JOURNAL_DIR_MODE) {
                Ok(()) => created = true,
                Err(error) if error.kind == FsErrorKind::Exists => {}
                Err(error) => return Err(error.into()),
            }
        }
        // The marker directory must be durable before the marker: a marker
        // in a directory lost by a crash leaves its entry unrecovered.
        if created {
            fs.sync_dir(&scope, layout.root())?;
        }
        let lock = fs.lock_exclusive(&layout.entry_lock(op)?, ctx.settings.lock_timeout)?;
        // A holder of the lock that finished meanwhile has left its entry but
        // no marker, so the marker alone does not show that the number is gone.
        if self.named_in(&layout, op) {
            return Err(Fault::Fs(FsError::new(
                FsErrorKind::Exists,
                "create_new",
                None,
            )));
        }
        drop(fs.create_new(&scope, &layout.pending(op)?)?);
        fs.sync_dir(&scope, &layout.pending_dir()?)?;
        Ok(Part {
            entry: layout.entry(op)?,
            volume,
            layout,
            lock: Some(lock),
            prepared: false,
            broken: false,
            digest: None,
        })
    }

    fn manifest(&self, part: usize, final_form: bool, chain: Option<ContentHash>) -> Manifest {
        let records = self
            .targets
            .iter()
            .flat_map(|target| {
                let own = (target.part == Some(part)).then_some(&target.record);
                let dest = target
                    .dest
                    .as_ref()
                    .filter(|dest| dest.cross && dest.part == Some(part))
                    .map(|dest| &dest.record);
                own.into_iter().chain(dest)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut parts: Vec<PartRef> = self
            .parts
            .iter()
            .map(|p| PartRef {
                vol: p.volume.id.clone(),
                dir: p.layout.root().clone(),
            })
            .collect();
        parts.sort_by(|a, b| a.vol.cmp(&b.vol));
        let expires = final_form.then(|| self.expires(&self.parts[part].volume));
        Manifest {
            v: if records.len() >= manifest::PACKED_MIN_TARGETS {
                manifest::PACKED_FORMAT_VERSION
            } else {
                manifest::FORMAT_VERSION
            },
            op: self.op.unwrap_or(OpId(0)),
            uuid: self.uuid,
            time: self.stamp.wall,
            time_basis: self.stamp.clone(),
            kind: self.request.kind.clone(),
            session: self.request.session.clone(),
            account: Some(self.ctx.state.account_id()),
            ast_hash: self.ast_hash,
            ast: self.ast.clone(),
            corrections: self.request.corrections.clone(),
            bulk: self.request.bulk,
            counts_as_step: self.request.counts_as_step,
            bytes_held: records.iter().map(|r| r.bytes_held).sum(),
            targets: records,
            parts,
            irreversible: self.irreversible.clone(),
            notable: self.notable.clone(),
            exempt: self.exempt.clone(),
            expires,
            undo_of: self.link.undo_of.clone(),
            redo_of: self.link.redo_of,
            scope: self.link.scope,
            items: self.link.items.clone(),
            integrity: Integrity {
                chain,
                ..Integrity::default()
            },
        }
    }

    fn expires(&self, volume: &VolumeInfo) -> Expires {
        let limits = self.ctx.limits.effective(volume);
        let after_seconds = match limits.max_age.value {
            AgeLimit::Limited(age) => Some(age.as_std().as_secs()),
            AgeLimit::Unlimited => None,
        };
        Expires {
            after_ops: match limits.max_ops.value {
                StepLimit::Limited(steps) => Some(steps),
                StepLimit::Unlimited => None,
            },
            after_seconds,
            deadline: after_seconds.map(|secs| {
                self.stamp
                    .wall
                    .saturating_add(std::time::Duration::from_secs(secs))
            }),
        }
    }

    /// Writes or rewrites the prepared manifest of `part`.
    fn write_prepared(&mut self, part: usize) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.journal_scope();
        let mut manifest = self.manifest(part, false, None);
        manifest
            .seal()
            .map_err(|_| FsError::new(FsErrorKind::Io, "seal", None))?;
        let bytes = manifest
            .encode()
            .map_err(|_| FsError::new(FsErrorKind::Io, "encode", None))?;
        let op = manifest.op;
        let target = &self.parts[part];
        if target.prepared {
            let entry = target.entry.clone();
            manifest::write_atomic(fs, &scope, entry.dir(), &entry.manifest()?, &bytes)?;
            ctx.hit(FailPoint::ManifestSynced, None)?;
            return Ok(());
        }
        let tmp = target.layout.entry_tmp(op)?;
        let ops_dir = target.layout.ops_dir()?;
        let entry_dir = target.entry.dir().clone();
        // A directory that exists already is not this entry's: it is left as
        // found. A directory created here is removed when it cannot be
        // completed, because no marker leads recovery to it afterwards.
        fs.create_dir(&scope, tmp.dir(), JOURNAL_DIR_MODE)?;
        let filled = fill_entry_tmp(ctx, &scope, &tmp, &bytes, (&entry_dir, &ops_dir));
        if matches!(filled, Err(Fault::Fs(_))) {
            let _ = fs.remove_tree(&scope, tmp.dir(), REMOVE_WORKERS);
        }
        filled?;
        self.parts[part].prepared = true;
        Ok(())
    }

    /// Writes the prepared manifest of every part touched by the targets in
    /// `range`; a part that cannot be written degrades its new targets.
    fn prepare_parts(&mut self, range: std::ops::Range<usize>) -> std::result::Result<(), Fault> {
        let mut touched: Vec<usize> = self.targets[range.clone()]
            .iter()
            .flat_map(|t| [t.part, t.dest.as_ref().and_then(|d| d.part)])
            .flatten()
            .collect();
        if !touched.is_empty() {
            touched.extend(0..self.parts.len());
        }
        touched.sort_unstable();
        touched.dedup();
        for part in touched {
            if self.parts[part].broken {
                continue;
            }
            match self.write_prepared(part) {
                Ok(()) => {}
                Err(Fault::Halt) => return Err(Fault::Halt),
                Err(Fault::Fs(_)) => self.break_part(part, range.clone()),
            }
        }
        Ok(())
    }

    fn break_part(&mut self, part: usize, range: std::ops::Range<usize>) {
        self.parts[part].broken = true;
        for target in &mut self.targets[range] {
            if target.part == Some(part) && target.mode().is_preserved() {
                target.degrade(Irreversible::JournalError);
            }
            if let Some(dest) = target.dest.as_mut().filter(|d| d.part == Some(part)) {
                dest.record.before.store = PreserveMode::None;
                dest.record.irreversible = Some(Irreversible::JournalError);
            }
        }
    }

    fn check_expect(&self, target: &Target) -> Result<()> {
        let Some(path) = &target.path else {
            return Ok(());
        };
        let current = match self.ctx.fs.metadata(path, Follow::NoFollow) {
            Ok(meta) => Some(meta),
            Err(error) if error.kind == FsErrorKind::NotFound => None,
            Err(error) => return Err(target_error(target.index(), error)),
        };
        let matches = match (&target.spec.expect, &current) {
            (Expect::Unchecked, _) => true,
            (Expect::Absent, current) => current.is_none(),
            (Expect::Etag(_) | Expect::Hash(_), None) => false,
            (Expect::Etag(expected), Some(meta)) => expected.matches(&meta.etag()),
            (Expect::Hash(expected), Some(_)) => {
                let hash = self
                    .ctx
                    .fs
                    .content_hash(path)
                    .map_err(|error| target_error(target.index(), error))?;
                hash == *expected
            }
        };
        if matches { Ok(()) } else { Err(stale(path)) }
    }
}

/// Fills the new directory `tmp` of an entry with its manifest and its
/// prepared state, flushes it and renames it to the name of the entry.
fn fill_entry_tmp(
    ctx: &JournalCtx,
    scope: &TxScope,
    tmp: &EntryLayout,
    manifest_bytes: &[u8],
    (entry_dir, ops_dir): (&AbsPath, &AbsPath),
) -> std::result::Result<(), Fault> {
    let fs = ctx.fs.as_ref();
    for dir in [tmp.blobs()?, tmp.tmp()?] {
        fs.create_dir(scope, &dir, JOURNAL_DIR_MODE)?;
    }
    let mut handle = fs.create_new(scope, &tmp.manifest()?)?;
    manifest::write_all(handle.as_mut(), manifest_bytes)?;
    ctx.hit(FailPoint::ManifestWritten, None)?;
    handle.sync(Durability::Full)?;
    drop(handle);
    let mut state = fs.create_new(scope, &tmp.state()?)?;
    manifest::write_all(
        state.as_mut(),
        &StateRecord::new(nurt_contracts::journal::EntryState::Prepared).encode(),
    )?;
    state.sync(Durability::Full)?;
    drop(state);
    fs.sync_dir(scope, tmp.dir())?;
    ctx.hit(FailPoint::ManifestSynced, None)?;
    fs.rename(scope, tmp.dir(), entry_dir, RenameMode::NoReplace)?;
    ctx.hit(FailPoint::PreparedRenamed, None)?;
    fs.sync_dir(scope, ops_dir)?;
    ctx.hit(FailPoint::PreparedDurable, None)?;
    Ok(())
}

/// Outcome of locating the journal part of a target.
enum Located {
    Skip(Irreversible),
    Halt,
}

fn rollback_failed(failed: &[TargetPath], cause: Error) -> Error {
    let paths: Vec<String> = failed.iter().map(record_path).collect();
    Error::new(
        ErrorCode::RollbackFailed,
        format!(
            "{} target(s) could not be restored; recovery completes the rollback",
            paths.len()
        ),
    )
    .with_detail(
        "failed",
        serde_json::to_value(failed).unwrap_or(serde_json::Value::Null),
    )
    .with_cause(cause)
}

fn check_version(
    ctx: &JournalCtx,
    scope: &TxScope,
    layout: &JournalLayout,
) -> std::result::Result<(), Fault> {
    let path = layout.version()?;
    match manifest::read_bytes(ctx.fs.as_ref(), &path) {
        Ok(bytes) if bytes.trim_ascii() == layout::VERSION_TEXT.as_bytes() => Ok(()),
        Ok(_) => Err(Fault::Fs(FsError::new(
            FsErrorKind::Unsupported,
            "journal_version",
            Some(path.as_path()),
        ))),
        Err(error) if error.kind == FsErrorKind::NotFound => {
            let text = format!("{}\n", layout::VERSION_TEXT);
            manifest::write_atomic(
                ctx.fs.as_ref(),
                scope,
                layout.root(),
                &path,
                text.as_bytes(),
            )?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

impl Tx<'_> {
    /// Inspects one declaration and builds its target.
    fn inspect(&self, index: TargetIndex, spec: &TargetSpec) -> Result<Target> {
        let fs = self.ctx.fs.as_ref();
        let mut path = None;
        let mut before = BeforeRecord {
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
            store: PreserveMode::None,
            blob: None,
            meta: None,
            inline: None,
        };
        let mut snapshot = None;
        match &spec.path {
            TargetPath::Fs(declared) => {
                let mut effective = declared.clone();
                let mut meta = match fs.metadata(declared, Follow::NoFollow) {
                    Ok(meta) => Some(meta),
                    Err(error) if error.kind == FsErrorKind::NotFound => None,
                    Err(error) => return Err(target_error(index, error)),
                };
                let follows = writes_content(&spec.action)
                    || matches!(spec.action, TargetAction::ExternalWrite);
                if follows && meta.as_ref().is_some_and(|m| m.kind == FileKind::Symlink) {
                    effective = fs
                        .real_path(declared)
                        .map_err(|error| target_error(index, error))?;
                    meta = Some(
                        fs.metadata(&effective, Follow::NoFollow)
                            .map_err(|error| target_error(index, error))?,
                    );
                }
                if must_exist(&spec.action) && meta.is_none() {
                    return Err(target_error(
                        index,
                        FsError::new(FsErrorKind::NotFound, "preserve", Some(declared.as_path())),
                    ));
                }
                if must_be_absent(&spec.action) && meta.is_some() {
                    return Err(target_error(
                        index,
                        FsError::new(FsErrorKind::Exists, "preserve", Some(declared.as_path())),
                    ));
                }
                if let Some(meta) = &meta {
                    let etag_hash = match &spec.expect {
                        Expect::Etag(etag) if etag.matches(&meta.etag()) => etag.hash,
                        Expect::Hash(hash) => Some(*hash),
                        _ => None,
                    };
                    before = BeforeRecord {
                        exists: true,
                        kind: Some(meta.kind),
                        size: Some(meta.size),
                        mode: Some(meta.mode),
                        uid: meta.uid,
                        gid: meta.gid,
                        modified_ns: Some(meta.modified_ns),
                        hash: etag_hash,
                        links: Some(meta.links),
                        file_id: Some(meta.file_id),
                        ..before
                    };
                }
                path = Some(effective);
            }
            TargetPath::Provider(key) => {
                let provider = self.ctx.provider(&key.scheme).ok_or_else(|| {
                    Error::new(
                        ErrorCode::Unsupported,
                        format!("no writable provider for `{}:`", key.scheme.0),
                    )
                })?;
                let taken = provider
                    .snapshot(key)
                    .map_err(|error| target_error(index, error))?;
                let named = record_path(&spec.path);
                let named = std::path::Path::new(&named);
                if must_be_absent(&spec.action) && taken.existed {
                    return Err(target_error(
                        index,
                        FsError::new(FsErrorKind::Exists, "preserve", Some(named)),
                    ));
                }
                if must_exist(&spec.action) && !taken.existed {
                    return Err(target_error(
                        index,
                        FsError::new(FsErrorKind::NotFound, "preserve", Some(named)),
                    ));
                }
                before.exists = taken.existed;
                snapshot = Some(taken);
            }
        }
        let mut target = Target {
            spec: spec.clone(),
            record: TargetRecord {
                index,
                path: match &path {
                    Some(path) => TargetPath::Fs(path.clone()),
                    None => spec.path.clone(),
                },
                action: spec.action.clone(),
                role: TargetRole::Whole,
                before,
                replaced: None,
                after: None,
                in_place: false,
                bytes_held: 0,
                irreversible: None,
                notable: Vec::new(),
            },
            path,
            part: None,
            old_meta: None,
            plan: None,
            snapshot,
            staged: None,
            prefix: None,
            dest: None,
            changed: matches!(spec.action, TargetAction::ExternalWrite),
            created: false,
            copied_on_move: false,
            observed: false,
            inline: false,
        };
        if let TargetAction::Rename { to, replace } = &spec.action {
            let existing = match fs.metadata(to, Follow::NoFollow) {
                Ok(meta) => Some(meta),
                Err(error) if error.kind == FsErrorKind::NotFound => None,
                Err(error) => return Err(target_error(index, error)),
            };
            if existing.is_some() && !replace {
                return Err(target_error(
                    index,
                    FsError::new(FsErrorKind::Exists, "rename", Some(to.as_path())),
                ));
            }
            let mut record = target.record.clone();
            record.path = TargetPath::Fs(to.clone());
            record.role = TargetRole::Destination;
            record.before = BeforeRecord {
                exists: existing.is_some(),
                kind: existing.as_ref().map(|m| m.kind),
                size: existing.as_ref().map(|m| m.size),
                mode: existing.as_ref().map(|m| m.mode),
                uid: existing.as_ref().and_then(|m| m.uid),
                gid: existing.as_ref().and_then(|m| m.gid),
                modified_ns: existing.as_ref().map(|m| m.modified_ns),
                hash: None,
                links: existing.as_ref().map(|m| m.links),
                file_id: existing.as_ref().map(|m| m.file_id),
                store: PreserveMode::None,
                blob: None,
                meta: None,
                inline: None,
            };
            target.dest = Some(Dest {
                path: to.clone(),
                part: None,
                record,
                cross: false,
                moved: false,
                created: false,
            });
        }
        if target.path.is_some() {
            self.check_expect(&target)?;
        }
        Ok(target)
    }

    /// Chooses the preserve mode of a file system target and asks admission
    /// for the bytes the journal would additionally hold.
    fn plan_target(&mut self, position: usize, op_size: u64) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let Some(path) = self.targets[position].path.clone() else {
            return Ok(());
        };
        let volume = match self.volume_of(position, &path) {
            Ok(volume) => volume,
            Err(_) => {
                self.targets[position].degrade(Irreversible::JournalError);
                return Ok(());
            }
        };
        let action = self.targets[position].spec.action.clone();
        if let TargetAction::Rename { to, .. } = &action {
            self.plan_rename(position, &volume, to, op_size)?;
            return Ok(());
        }
        let part = match self.part_for(&volume) {
            Ok(part) => part,
            Err(Located::Halt) => return Err(Fault::Halt),
            Err(Located::Skip(reason)) => {
                self.targets[position].degrade(reason);
                return Ok(());
            }
        };
        self.targets[position].part = Some(part);
        let target = &self.targets[position];
        let before = target.record.before.clone();
        if before.kind == Some(FileKind::Special) {
            self.targets[position].degrade(Irreversible::SpecialFile);
            return Ok(());
        }
        let (mode, extra) = match (&action, before.exists) {
            (TargetAction::Attributes, _) => (PreserveMode::Meta, 0),
            (TargetAction::Append, true) => (PreserveMode::Append, 0),
            (TargetAction::Delete | TargetAction::Replace, _) => {
                (PreserveMode::Moved, before.size.unwrap_or(0))
            }
            (TargetAction::ExternalWrite, true) if before.kind == Some(FileKind::File) => {
                if self.external_budget_spent() {
                    self.targets[position].degrade(Irreversible::PreviousContentNotKept);
                    return Ok(());
                }
                let mode = if volume.features.reflink {
                    PreserveMode::Clone
                } else {
                    PreserveMode::Copy
                };
                (mode, before.size.unwrap_or(0))
            }
            (TargetAction::ExternalWrite, true) => {
                self.targets[position].degrade(Irreversible::PreviousContentNotKept);
                return Ok(());
            }
            (TargetAction::Modify | TargetAction::Truncate, true) => {
                if self.keeps_inline(position, &path) {
                    self.targets[position].plan = Some(Plan {
                        exchange: Vec::new(),
                        clone: false,
                        in_place: false,
                    });
                    self.targets[position].inline = true;
                    (PreserveMode::Copy, before.size.unwrap_or(0))
                } else {
                    let plan = overwrite::plan(
                        ctx.settings.overwrite,
                        &volume.features,
                        before.links.unwrap_or(1),
                    );
                    let mode = plan.planned_mode();
                    self.targets[position].record.in_place = plan.in_place;
                    self.targets[position].plan = Some(plan);
                    (mode, before.size.unwrap_or(0))
                }
            }
            (_, _) => (PreserveMode::Created, 0),
        };
        let index = self.targets[position].index();
        let id = BlobId::new(index, BlobRole::Previous);
        let inline = self.targets[position].inline;
        let record = &mut self.targets[position].record;
        record.before.store = mode;
        if !inline
            && matches!(
                mode,
                PreserveMode::Moved
                    | PreserveMode::Exchange
                    | PreserveMode::ReplaceBackup
                    | PreserveMode::Hardlink
                    | PreserveMode::Clone
                    | PreserveMode::Copy
            )
        {
            record.before.blob = Some(id.content_name());
        }
        if !inline
            && matches!(
                mode,
                PreserveMode::Moved
                    | PreserveMode::Meta
                    | PreserveMode::Exchange
                    | PreserveMode::ReplaceBackup
                    | PreserveMode::Hardlink
                    | PreserveMode::Clone
                    | PreserveMode::Copy
            )
        {
            record.before.meta = Some(id.meta_name());
        }
        if inline {
            self.inline_admission.push((position, extra));
        } else if extra > 0 || mode.is_preserved() && record.before.blob.is_some() {
            self.admit(&[position], &volume, extra, op_size);
        }
        Ok(())
    }

    /// Volume of a target. Entries of one directory share their volume, so the
    /// description of a volume, which reads the mount table, is made once per
    /// directory instead of once per target; directories are described
    /// individually because a directory can be a mount point.
    fn volume_of(&mut self, position: usize, path: &AbsPath) -> FsResult<VolumeInfo> {
        let ctx = self.ctx;
        let is_directory = self.targets[position].record.before.kind == Some(FileKind::Directory);
        let Some(parent) = path.parent().filter(|_| !is_directory) else {
            return ctx.fs.volume(path);
        };
        if let Some(volume) = self.volumes.get(&parent) {
            return Ok(volume.clone());
        }
        let volume = ctx.fs.volume(path)?;
        self.volumes.insert(parent, volume.clone());
        Ok(volume)
    }

    /// Whether the previous content of the target is kept in the manifest: a
    /// small regular file with one name and plain metadata, in a `preserve`
    /// call with many targets. The metadata read for the decision is kept for
    /// the metadata transfer to the new file.
    fn keeps_inline(&mut self, position: usize, path: &AbsPath) -> bool {
        let target = &self.targets[position];
        let before = &target.record.before;
        let candidate = self.inline_batch
            && target.spec.action == TargetAction::Modify
            && before.kind == Some(FileKind::File)
            && before.links.unwrap_or(1) <= 1
            && before.size.is_some_and(|size| size <= INLINE_MAX_BYTES);
        if !candidate {
            return false;
        }
        match self.ctx.fs.extended_metadata(path, Follow::NoFollow) {
            Ok(meta) if plain_meta(&meta) => {
                self.targets[position].old_meta = Some(meta);
                true
            }
            _ => false,
        }
    }

    /// Asks admission once for the inline targets of the running `preserve`
    /// call of each volume, with the bytes they add together.
    fn admit_inline(&mut self, op_size: u64) {
        let waiting = std::mem::take(&mut self.inline_admission);
        let mut groups: Vec<(usize, Vec<usize>, u64)> = Vec::new();
        for (position, extra) in waiting {
            let Some(part) = self.targets[position].part else {
                continue;
            };
            match groups.iter_mut().find(|group| group.0 == part) {
                Some(group) => {
                    group.1.push(position);
                    group.2 = group.2.saturating_add(extra);
                }
                None => groups.push((part, vec![position], extra)),
            }
        }
        for (part, positions, extra) in groups {
            let volume = self.parts[part].volume.clone();
            self.admit(&positions, &volume, extra, op_size);
        }
    }

    fn external_budget_spent(&mut self) -> bool {
        let Some(budget) = self.request.preserve_budget else {
            return false;
        };
        let now = self.ctx.clock.monotonic();
        let since = *self.external_since.get_or_insert(now);
        now.saturating_sub(since) > budget
    }

    fn admit(&mut self, positions: &[usize], volume: &VolumeInfo, extra: u64, op_size: u64) {
        let limits = self.ctx.limits.effective(volume);
        let op_size_limit = match limits.max_op_size.value {
            ByteLimit::Bytes(size) => Some(size.bytes()),
            ByteLimit::Unlimited => None,
        };
        let request = AdmissionRequest {
            volume: volume.id.clone(),
            extra,
            op_size,
            bytes_held: 0,
            size_budget: limits.max_size_bytes,
            oversize_keep: limits.oversize.value == Oversize::Keep,
            op_size_limit,
        };
        let skipped = match self.ctx.admission.admit(&request) {
            AdmissionDecision::Skip { reason } => Some(reason),
            _ => self.admit_size(volume, extra, op_size, &limits),
        };
        if let Some(reason) = skipped {
            if reason == Irreversible::TooLargeForJournal {
                self.note_oversize(op_size, &limits);
            }
            for &position in positions {
                self.targets[position].degrade(reason);
            }
        }
    }

    /// Checks the operation against the size limits of the volume.
    ///
    /// The oldest removable entries are expired for the whole operation once
    /// per volume and transaction; later targets of the volume only check the
    /// limits. A failure to expire entries never fails the operation: the
    /// next expiry pass enforces the budget.
    fn admit_size(
        &mut self,
        volume: &VolumeInfo,
        extra: u64,
        op_size: u64,
        limits: &EffectiveRetention,
    ) -> Option<Irreversible> {
        let admission = if self.budgeted.contains(&volume.id) {
            Ok(expiry::budget_admission(&[], extra, op_size, limits))
        } else {
            self.budgeted.push(volume.id.clone());
            expiry::admit_size(self.ctx, &volume.mount, op_size, op_size, limits)
        };
        matches!(admission, Ok(BudgetAdmission::TooLarge { .. }))
            .then_some(Irreversible::TooLargeForJournal)
    }

    fn note_oversize(&mut self, op_size: u64, limits: &EffectiveRetention) {
        let notable =
            expiry::oversize_note(op_size, limits).unwrap_or(JournalNotable::JournalOversize {
                size: op_size,
                limit: limits.max_size_bytes.unwrap_or(0),
            });
        if !self.notable.contains(&notable) {
            self.notable.push(notable);
        }
    }

    fn plan_rename(
        &mut self,
        position: usize,
        volume: &VolumeInfo,
        to: &AbsPath,
        op_size: u64,
    ) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let dest_volume = to
            .parent()
            .map_or_else(|| ctx.fs.volume(to), |parent| ctx.fs.volume(&parent))
            .ok();
        let cross = dest_volume.as_ref().is_none_or(|v| v.id != volume.id);
        let part = self.part_for(volume);
        let index = self.targets[position].index();
        match part {
            Ok(part) => {
                self.targets[position].part = Some(part);
                let record = &mut self.targets[position].record;
                if cross {
                    let id = BlobId::new(index, BlobRole::Previous);
                    record.role = TargetRole::Source;
                    record.before.store = PreserveMode::Moved;
                    record.before.blob = Some(id.content_name());
                    record.before.meta = Some(id.meta_name());
                } else {
                    record.before.store = PreserveMode::Rename;
                }
            }
            Err(Located::Halt) => return Err(Fault::Halt),
            Err(Located::Skip(reason)) => self.targets[position].degrade(reason),
        }
        let dest_part = match (&dest_volume, cross) {
            (Some(dest), true) => self.part_for(dest),
            (_, false) => part,
            (None, true) => Err(Located::Skip(Irreversible::JournalError)),
        };
        let Some(dest) = self.targets[position].dest.as_mut() else {
            return Ok(());
        };
        dest.cross = cross;
        match dest_part {
            Ok(part) => {
                dest.part = Some(part);
                let id = BlobId::new(index, BlobRole::Destination);
                if dest.record.before.exists {
                    dest.record.before.store = PreserveMode::Moved;
                    dest.record.before.blob = Some(id.content_name());
                    dest.record.before.meta = Some(id.meta_name());
                } else {
                    dest.record.before.store = PreserveMode::Created;
                }
            }
            Err(Located::Halt) => return Err(Fault::Halt),
            Err(Located::Skip(reason)) => {
                dest.record.before.store = PreserveMode::None;
                dest.record.irreversible = Some(reason);
            }
        }
        let dest_size = dest.record.before.size.unwrap_or(0);
        let dest_exists = dest.record.before.exists && dest.record.before.store.is_preserved();
        if let Some(dest_volume) = dest_volume.filter(|_| dest_exists) {
            let limits = ctx.limits.effective(&dest_volume);
            let request = AdmissionRequest {
                volume: dest_volume.id.clone(),
                extra: dest_size,
                op_size,
                bytes_held: 0,
                size_budget: limits.max_size_bytes,
                oversize_keep: limits.oversize.value == Oversize::Keep,
                op_size_limit: match limits.max_op_size.value {
                    ByteLimit::Bytes(size) => Some(size.bytes()),
                    ByteLimit::Unlimited => None,
                },
            };
            if let AdmissionDecision::Skip { reason } = ctx.admission.admit(&request)
                && let Some(dest) = self.targets[position].dest.as_mut()
            {
                dest.record.before.store = PreserveMode::None;
                dest.record.before.blob = None;
                dest.record.irreversible = Some(reason);
            }
        }
        if cross && self.targets[position].mode() == PreserveMode::Moved {
            let size = self.targets[position].record.before.size.unwrap_or(0);
            self.admit(&[position], volume, size, op_size);
        }
        // The prepared manifest names the block of the replaced destination,
        // so that recovery can put it back after a crash at any later step.
        if !cross {
            let replaced = self.targets[position]
                .dest
                .as_ref()
                .filter(|dest| dest.record.before.exists)
                .map(|dest| dest.record.before.clone());
            self.targets[position].record.replaced = replaced;
        }
        Ok(())
    }

    /// Preserves the previous content and metadata of the targets in `range`
    /// (step 4); content moved or exchanged is preserved at apply.
    fn preserve_content(
        &mut self,
        range: std::ops::Range<usize>,
    ) -> std::result::Result<bool, Fault> {
        let mut degraded = false;
        for position in range {
            let index = self.targets[position].index();
            self.ctx.hit(FailPoint::PreserveBefore, Some(index.0))?;
            match self.preserve_one(position) {
                Ok(()) => {}
                Err(Fault::Halt) => return Err(Fault::Halt),
                Err(Fault::Fs(error)) => {
                    degraded = true;
                    let reason = if error.kind == FsErrorKind::NoSpace {
                        Irreversible::NoSpaceForCopy
                    } else {
                        Irreversible::JournalError
                    };
                    self.targets[position].degrade(reason);
                    self.targets[position].record.before.meta = None;
                }
            }
            self.ctx.hit(FailPoint::PreserveDurable, Some(index.0))?;
        }
        Ok(degraded)
    }

    fn preserve_one(&mut self, position: usize) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.journal_scope();
        let target = &self.targets[position];
        let index = target.index();
        if let Some(snapshot) = target.snapshot.clone() {
            if let Some(part) = target.part {
                let entry = self.parts[part].entry.clone();
                let id = BlobId::new(index, BlobRole::Previous);
                let bytes = serde_json::to_vec(&snapshot)
                    .map_err(|_| FsError::new(FsErrorKind::Io, "encode_snapshot", None))?;
                manifest::write_atomic(fs, &scope, &entry.tmp()?, &entry.blob(id)?, &bytes)?;
                ctx.hit(FailPoint::PreserveMoved, Some(index.0))?;
                self.targets[position].record.before.blob = Some(id.content_name());
            }
            return Ok(());
        }
        let (Some(path), Some(part)) = (target.path.clone(), target.part) else {
            return Ok(());
        };
        if target.inline {
            // The content is read into the manifest. A previous content that
            // changed since the target was inspected fails the expected state
            // check of the change, so it is recorded as read.
            let mut bytes = Vec::new();
            fs.open_read(&path)?
                .take(INLINE_MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| FsError::new(FsErrorKind::Io, "read", Some(path.as_path())))?;
            if bytes.len() as u64 > INLINE_MAX_BYTES {
                return Err(
                    FsError::new(FsErrorKind::Io, "inline_size", Some(path.as_path())).into(),
                );
            }
            ctx.hit(FailPoint::PreserveMoved, Some(index.0))?;
            let record = &mut self.targets[position].record;
            record.bytes_held = bytes.len() as u64;
            record.before.inline = Some(bytes);
            return Ok(());
        }
        let entry = self.parts[part].entry.clone();
        let mode = target.mode();
        let id = BlobId::new(index, BlobRole::Previous);
        if target.record.before.meta.is_some() {
            let meta = fs.extended_metadata(&path, Follow::NoFollow)?;
            blob::write_meta(fs, &scope, &entry.tmp()?, &entry.blob_meta(id)?, &meta)?;
            self.targets[position].old_meta = Some(meta);
        }
        if let Some(dest) = &self.targets[position].dest
            && dest.record.before.store == PreserveMode::Moved
            && let Some(dest_part) = dest.part
        {
            let dest_entry = self.parts[dest_part].entry.clone();
            let dest_id = BlobId::new(index, BlobRole::Destination);
            let meta = fs.extended_metadata(&dest.path, Follow::NoFollow)?;
            blob::write_meta(
                fs,
                &scope,
                &dest_entry.tmp()?,
                &dest_entry.blob_meta(dest_id)?,
                &meta,
            )?;
        }
        let copies_first = match (&self.targets[position].plan, mode) {
            (Some(plan), _) => plan.copies_first(),
            (None, PreserveMode::Clone | PreserveMode::Copy) => true,
            _ => false,
        };
        if copies_first && mode.is_preserved() {
            let allow_clone = mode == PreserveMode::Clone;
            let used = blob::preserve_copy(
                fs,
                &scope,
                &path,
                &entry.tmp()?,
                &entry.blob(id)?,
                allow_clone,
            )?;
            ctx.hit(FailPoint::PreserveMoved, Some(index.0))?;
            let size = self.targets[position].record.before.size.unwrap_or(0);
            let record = &mut self.targets[position].record;
            record.before.store = used;
            record.bytes_held = size;
        }
        if mode == PreserveMode::Append {
            let before = &self.targets[position].record.before;
            if before.hash.is_none() {
                let mut hasher = Sha256::new();
                let mut reader = fs.open_read(&path)?;
                let mut buffer = vec![0_u8; blob::COPY_BUFFER];
                loop {
                    let read = std::io::Read::read(&mut reader, &mut buffer)
                        .map_err(|_| FsError::new(FsErrorKind::Io, "read", Some(path.as_path())))?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                }
                self.targets[position].record.before.hash =
                    Some(ContentHash(hasher.clone().finalize().into()));
                self.targets[position].prefix = Some(hasher);
            }
        }
        Ok(())
    }
}

impl Tx<'_> {
    fn do_preserve(&mut self, specs: &[TargetSpec]) -> Result<Vec<Preserved>> {
        self.inline_batch = specs.len() >= INLINE_MIN_TARGETS;
        let start = self.targets.len();
        let mut inspected = Vec::with_capacity(specs.len());
        for (offset, spec) in specs.iter().enumerate() {
            let index = TargetIndex(u32::try_from(start + offset).unwrap_or(u32::MAX));
            inspected.push(self.inspect(index, spec)?);
        }
        if self.number().is_err() {
            return Err(self.halt());
        }
        self.targets.extend(inspected);
        let range = start..self.targets.len();
        let op_size: u64 = self
            .targets
            .iter()
            .filter(|t| t.record.before.exists)
            .map(|t| t.record.before.size.unwrap_or(0))
            .sum();
        for position in range.clone() {
            let planned = if let Some(reason) = self.unavailable.filter(|_| self.op.is_none()) {
                self.targets[position].degrade(reason);
                Ok(())
            } else if self.targets[position].path.is_some() {
                self.plan_target(position, op_size)
            } else {
                self.plan_provider(position)
            };
            if planned.is_err() {
                return Err(self.halt());
            }
        }
        self.admit_inline(op_size);
        // Targets that keep their content in the manifest write no file into the
        // entry, so the entry is first written with the content in it.
        let manifest_waits = range.clone().all(|position| self.targets[position].inline);
        let steps = self
            .ctx
            .hit(FailPoint::PlanDecided, None)
            .and_then(|()| {
                if manifest_waits {
                    Ok(())
                } else {
                    self.prepare_parts(range.clone())
                }
            })
            .and_then(|()| self.preserve_content(range.clone()))
            .and_then(|degraded| {
                // The inline content is part of the manifest, so the manifest
                // is written again before the first change: a crash after a
                // change finds the previous content in the durable manifest.
                let inline = range
                    .clone()
                    .any(|position| self.targets[position].record.before.inline.is_some());
                if degraded || inline {
                    self.prepare_parts(range.clone())
                } else {
                    Ok(())
                }
            });
        match steps {
            Ok(()) => {}
            Err(Fault::Halt) => return Err(self.halt()),
            Err(Fault::Fs(error)) => {
                for target in &mut self.targets[range.clone()] {
                    if target.mode().is_preserved() && target.part.is_some() {
                        target.degrade(Irreversible::JournalError);
                    }
                }
                let _ = error;
            }
        }
        Ok(self.targets[range]
            .iter()
            .map(|target| Preserved {
                index: target.index(),
                volume: target.part.map(|part| self.parts[part].volume.id.clone()),
                mode: target.mode(),
                bytes_held: target.record.bytes_held,
                before: target.record.before.to_state(),
                irreversible: target.irreversible(),
                notable: target.record.notable.clone(),
            })
            .collect())
    }

    fn plan_provider(&mut self, position: usize) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let TargetPath::Provider(key) = self.targets[position].spec.path.clone() else {
            return Ok(());
        };
        let volume = ctx
            .fs
            .volume(ctx.state.root())
            .map_err(|_| Located::Skip(Irreversible::JournalError));
        let part = match volume {
            Ok(volume) => self.part_for(&volume),
            Err(skip) => Err(skip),
        };
        match part {
            Ok(part) => {
                let reference = ctx.settings.reference_schemes.contains(&key.scheme);
                let target = &mut self.targets[position];
                target.part = Some(part);
                target.record.before.store = if reference {
                    PreserveMode::Ref
                } else {
                    PreserveMode::Value
                };
                Ok(())
            }
            Err(Located::Halt) => Err(Fault::Halt),
            Err(Located::Skip(reason)) => {
                self.targets[position].degrade(reason);
                Ok(())
            }
        }
    }

    fn position(&self, index: TargetIndex) -> Result<usize> {
        let position = usize::try_from(index.0).unwrap_or(usize::MAX);
        if position < self.targets.len() {
            Ok(position)
        } else {
            Err(misuse("the target was not declared"))
        }
    }

    fn staged_path(&self, position: usize) -> Result<AbsPath> {
        let target = &self.targets[position];
        let path = target.fs_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| misuse("a root cannot be written"))?;
        parent
            .join_name(&layout::staged_name(
                self.op.unwrap_or(OpId(0)),
                target.index(),
            ))
            .map_err(|error| target_error(target.index(), error))
    }

    fn do_stage(&mut self, index: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        let position = self.position(index)?;
        if !writes_content(&self.targets[position].spec.action)
            || self.targets[position].spec.action == TargetAction::Append
        {
            return Err(misuse("the target was not declared for new content"));
        }
        self.phase = Phase::Applying;
        let staged = self.staged_path(position)?;
        let _ = blob::discard(self.ctx.fs.as_ref(), &self.scope(), &staged);
        let handle = self
            .ctx
            .fs
            .create_new(&self.scope(), &staged)
            .map_err(|error| target_error(index, error))?;
        self.targets[position].staged = Some(staged);
        Ok(handle)
    }

    fn do_apply(&mut self, index: TargetIndex, mutation: Mutation) -> Result<Applied> {
        let position = self.position(index)?;
        let provider = self.targets[position].snapshot.is_some();
        if !accepts(&self.targets[position].spec.action, &mutation, provider) {
            return Err(misuse("the mutation does not match the declared action"));
        }
        if !matches!(
            mutation,
            Mutation::WriteBytes(_)
                | Mutation::CommitStaged
                | Mutation::CopyFrom { .. }
                | Mutation::WriteAt { .. }
                | Mutation::SetLength(_)
        ) {
            // Other changes order their own flushes against the names that
            // deferred changes may still use.
            self.flush_unsynced()?;
        }
        self.phase = Phase::Applying;
        match mutation {
            Mutation::WriteBytes(bytes) => self.write(position, Content::Bytes(bytes)),
            Mutation::CommitStaged => self.write(position, Content::Staged),
            Mutation::CopyFrom { source, clone } => {
                self.write(position, Content::Copy { source, clone })
            }
            Mutation::WriteAt { offset, bytes } => {
                self.write(position, Content::At { offset, bytes })
            }
            Mutation::SetLength(length) => self.write(position, Content::Length(length)),
            Mutation::AppendBytes(bytes) => self.append(position, &bytes),
            Mutation::Remove => self.remove(position),
            Mutation::Rename => self.rename(position),
            Mutation::CreateDir { mode } => {
                self.create(position, |fs, scope, path| fs.create_dir(scope, path, mode))
            }
            Mutation::CreateLink { target, kind } => self.create(position, |fs, scope, path| {
                fs.create_link(scope, &target, path, kind)
            }),
            Mutation::CreateHardLink { existing } => self.create(position, |fs, scope, path| {
                fs.hard_link(scope, &existing, path)
            }),
            Mutation::SetMeta(change) => {
                let target = &self.targets[position];
                self.check_expect(target)?;
                let path = target.fs_path()?.clone();
                let fs = self.ctx.fs.as_ref();
                fs.set_meta(&self.scope(), &path, &change)
                    .map_err(|error| target_error(index, error))?;
                self.targets[position].changed = true;
                blob::sync_entry(fs, &self.scope(), &path)
                    .map_err(|error| target_error(index, error))?;
                self.finish(position, None)
            }
            Mutation::ProviderSet(value) => self.provider(position, ProviderChange::Set(value)),
            Mutation::ProviderRemove => self.provider(position, ProviderChange::Remove),
            Mutation::ProviderCopy(from) => self.provider(position, ProviderChange::Copy(from)),
            Mutation::ProviderRestore(snapshot) => {
                self.provider(position, ProviderChange::Restore(snapshot))
            }
            _ => Err(misuse("the mutation is not supported")),
        }
    }

    fn finish(&mut self, position: usize, hash: Option<ContentHash>) -> Result<Applied> {
        let index = self.targets[position].index();
        let after = match self.targets[position].path.clone() {
            Some(path) => {
                after_record(self.ctx, &path, hash).map_err(|error| target_error(index, error))?
            }
            None => AfterRecord {
                exists: true,
                ..AfterRecord::default()
            },
        };
        let state = after.to_state();
        self.targets[position].record.after = Some(after);
        Ok(Applied {
            index,
            after: state,
        })
    }

    fn provider(&mut self, position: usize, change: ProviderChange) -> Result<Applied> {
        let index = self.targets[position].index();
        let TargetPath::Provider(key) = self.targets[position].spec.path.clone() else {
            return Err(misuse("the target is not a provider key"));
        };
        let provider = self
            .ctx
            .provider(&key.scheme)
            .ok_or_else(|| misuse("the provider disappeared"))?
            .clone();
        let scope = self.scope();
        let result = match &change {
            ProviderChange::Set(value) => provider.set(&scope, &key, value),
            ProviderChange::Remove => provider.remove(&scope, &key),
            ProviderChange::Copy(from) => provider.duplicate(&scope, from, &key),
            ProviderChange::Restore(snapshot) => provider.restore(&scope, snapshot),
        };
        result.map_err(|error| target_error(index, error))?;
        self.targets[position].changed = true;
        let after = AfterRecord {
            exists: match &change {
                ProviderChange::Remove => false,
                ProviderChange::Restore(snapshot) => snapshot.existed,
                ProviderChange::Set(_) | ProviderChange::Copy(_) => true,
            },
            ..AfterRecord::default()
        };
        let state = after.to_state();
        self.targets[position].record.after = Some(after);
        Ok(Applied {
            index,
            after: state,
        })
    }

    fn create(
        &mut self,
        position: usize,
        make: impl FnOnce(&dyn nurt_contracts::fs::FsWrite, &TxScope, &AbsPath) -> FsResult<()>,
    ) -> Result<Applied> {
        let target = &self.targets[position];
        let index = target.index();
        let path = target.fs_path()?.clone();
        if !(target.spec.action == TargetAction::Replace && target.changed) {
            self.check_expect(target)?;
        }
        let fs = self.ctx.fs.as_ref();
        make(fs, &self.scope(), &path).map_err(|error| target_error(index, error))?;
        let target = &mut self.targets[position];
        target.created = true;
        target.changed = true;
        blob::sync_parents(fs, &self.scope(), &[&path])
            .map_err(|error| target_error(index, error))?;
        self.finish(position, None)
    }
}

impl Tx<'_> {
    /// Writes new content (step 5).
    #[expect(
        clippy::cognitive_complexity,
        reason = "one linear protocol of stage, flush, check and switch"
    )]
    fn write(&mut self, position: usize, content: Content) -> Result<Applied> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.scope();
        let index = self.targets[position].index();
        let path = self.targets[position].fs_path()?.clone();
        let replacing = self.targets[position].spec.action == TargetAction::Replace;
        let existed = self.targets[position].record.before.exists && !replacing;
        let staged = self.staged_path(position)?;
        let as_target = |error: FsError| target_error(index, error);
        let mut hasher = Sha256::new();
        let mut hashed = false;
        let mut cloned = false;
        let copied = match &content {
            Content::Copy { source, .. } => fs.extended_metadata(source, Follow::NoFollow).ok(),
            _ => None,
        };
        let mut handle = match &content {
            Content::Staged => {
                if self.targets[position].staged.as_ref() != Some(&staged) {
                    return Err(misuse("no staged file was opened for the target"));
                }
                fs.open_in_place(&scope, &staged).map_err(as_target)?
            }
            Content::Copy { source, clone } if *clone != CloneMode::Never => {
                let _ = blob::discard(fs, &scope, &staged);
                match fs.clone_file(&scope, source, &staged) {
                    Ok(()) => {
                        cloned = true;
                        fs.open_in_place(&scope, &staged).map_err(as_target)?
                    }
                    Err(error)
                        if error.kind == FsErrorKind::Unsupported && *clone == CloneMode::Auto =>
                    {
                        fs.create_new(&scope, &staged).map_err(as_target)?
                    }
                    Err(error) => return Err(as_target(error)),
                }
            }
            _ => {
                let _ = blob::discard(fs, &scope, &staged);
                fs.create_new(&scope, &staged).map_err(as_target)?
            }
        };
        self.targets[position].staged = Some(staged.clone());
        match content {
            Content::Bytes(bytes) => {
                hasher.update(&bytes);
                hashed = true;
                manifest::write_all(handle.as_mut(), &bytes).map_err(as_target)?;
            }
            Content::Staged => {}
            Content::Copy { source, .. } => {
                if !cloned {
                    blob::stream_copy(fs, &source, handle.as_mut(), Some(&mut hasher))
                        .map_err(as_target)?;
                    hashed = true;
                }
            }
            Content::At { offset, bytes } => {
                blob::stream_copy(fs, &path, handle.as_mut(), None).map_err(as_target)?;
                handle
                    .seek(SeekFrom::Start(offset))
                    .map_err(|_| as_target(FsError::new(FsErrorKind::Io, "seek", None)))?;
                manifest::write_all(handle.as_mut(), &bytes).map_err(as_target)?;
            }
            Content::Length(length) => {
                if existed {
                    blob::stream_copy(fs, &path, handle.as_mut(), None).map_err(as_target)?;
                }
                handle.set_length(length).map_err(as_target)?;
            }
        }
        self.step(FailPoint::StageWritten, index)?;
        let previous = self.targets[position].old_meta.as_ref().filter(|_| existed);
        let mut writable = true;
        let complete = match self
            .metadata_for(index, previous, &staged, copied.as_ref())
            .map_err(as_target)?
        {
            Some(meta) => {
                writable = meta.basic.mode & ACCOUNT_WRITE != 0;
                handle.apply_meta(&meta).map_err(as_target)?.is_complete()
            }
            None => true,
        };
        // In a transaction with many targets the flush of the new content and
        // of the directories is left to one flush before the commit; a file
        // that cannot be reopened for writing is flushed through its handle.
        let deferred = writable && self.batched();
        if !deferred {
            handle.sync(Durability::Full).map_err(as_target)?;
        }
        drop(handle);
        self.step(FailPoint::StageSynced, index)?;
        let hash = if hashed {
            ContentHash(hasher.finalize().into())
        } else {
            fs.content_hash(&staged).map_err(as_target)?
        };
        if !(replacing && self.targets[position].changed) {
            self.check_expect(&self.targets[position])?;
        }
        if existed {
            self.switch_existing(position, &path, &staged, complete, deferred)?;
        } else {
            match fs.rename(&scope, &staged, &path, RenameMode::NoReplace) {
                Ok(()) => {}
                Err(error) if error.kind == FsErrorKind::Exists => return Err(stale(&path)),
                Err(error) => return Err(as_target(error)),
            }
            self.targets[position].staged = None;
            self.targets[position].created = true;
            if !replacing {
                self.targets[position].record.before.store = match self.targets[position].mode() {
                    PreserveMode::None => PreserveMode::None,
                    _ => PreserveMode::Created,
                };
            }
        }
        self.targets[position].changed = true;
        self.step(FailPoint::ApplySwitched, index)?;
        if deferred {
            self.unsynced.file(path.clone());
            if let Some(parent) = path.parent() {
                self.unsynced.dir(parent);
            }
        } else if let Some(parent) = path.parent() {
            fs.sync_dir(&scope, &parent).map_err(as_target)?;
        }
        self.finish(position, Some(hash))
    }

    /// Whether the transaction has so many targets that the flushes of its
    /// changes are made together.
    fn batched(&self) -> bool {
        self.targets.len() >= BATCH_MIN_TARGETS
    }

    /// Flushes the new content and the directories changed since the last
    /// flush, in one call, so that they are durable before the entry is
    /// committed.
    fn flush_unsynced(&mut self) -> Result<()> {
        if self.unsynced.is_empty() {
            return Ok(());
        }
        let unsynced = std::mem::take(&mut self.unsynced);
        self.ctx
            .fs
            .sync_paths(&self.scope(), &unsynced.files, &unsynced.dirs)
            .map_err(Error::from)
    }

    /// Metadata the new content of a target receives before it is switched
    /// in.
    ///
    /// An undo or redo applies the metadata recorded with the state it brings
    /// back, also to a file that does not exist any more, so that the file is
    /// never visible with other permissions. Any other change transfers the
    /// metadata of the previous file `previous`, except the modification
    /// time, which is the time of the write. The content of a copy takes the
    /// times of the file it was copied from, `copied`; a new file takes all
    /// its metadata, so that a copy keeps times, permissions and attributes.
    fn metadata_for(
        &self,
        index: TargetIndex,
        previous: Option<&ExtendedMeta>,
        written: &AbsPath,
        copied: Option<&ExtendedMeta>,
    ) -> FsResult<Option<ExtendedMeta>> {
        let fs = self.ctx.fs.as_ref();
        let recorded = usize::try_from(index.0)
            .ok()
            .and_then(|position| self.link.recorded.get(position));
        let kept = recorded
            .and_then(|recorded| recorded.inline.as_deref().cloned())
            .or_else(|| {
                recorded
                    .and_then(|recorded| recorded.blob.as_ref())
                    .and_then(|blob| manifest::read_meta(fs, blob).ok())
            });
        if kept.is_some() {
            return Ok(kept);
        }
        let Some(previous) = previous else {
            return Ok(copied.cloned());
        };
        let current = match copied {
            Some(source) => source.basic.clone(),
            None => fs.metadata(written, Follow::NoFollow)?,
        };
        let mut meta = nurt_fs::transfer::for_content_change(previous, &current);
        if let Some(modified_ns) = recorded.and_then(|recorded| recorded.modified_ns) {
            meta.basic.modified_ns = modified_ns;
        }
        Ok(Some(meta))
    }

    fn step(&mut self, point: FailPoint, index: TargetIndex) -> Result<()> {
        match self.ctx.hit(point, Some(index.0)) {
            Ok(()) => Ok(()),
            Err(Fault::Halt) => Err(self.halt()),
            Err(Fault::Fs(error)) => Err(target_error(index, error)),
        }
    }

    /// Switches new content over an existing file, keeping the previous
    /// file in the journal when the plan allows it.
    fn switch_existing(
        &mut self,
        position: usize,
        path: &AbsPath,
        staged: &AbsPath,
        complete: bool,
        deferred: bool,
    ) -> Result<()> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.scope();
        let index = self.targets[position].index();
        let target = &self.targets[position];
        let blob_path = match (target.part, &target.record.before.blob) {
            (Some(part), Some(_)) => self.parts[part]
                .entry
                .blob(BlobId::new(index, BlobRole::Previous))
                .ok(),
            _ => None,
        };
        let variants = match (&target.plan, complete, &blob_path) {
            (Some(plan), true, Some(_)) => plan.exchange.clone(),
            _ => Vec::new(),
        };
        if let Some(blob_path) = &blob_path {
            for variant in variants {
                match overwrite::switch(fs, &scope, variant, path, staged, blob_path) {
                    Ok(()) => {
                        if variant == Exchange::Backup && !self.security_kept(position, path) {
                            if overwrite::revert(fs, &scope, path, staged, blob_path).is_ok() {
                                // The staged file cannot carry the account, ACL
                                // or streams of the target, so the new content
                                // goes into the existing file after the copy.
                                self.targets[position].record.in_place = true;
                                break;
                            }
                            self.targets[position].degrade(Irreversible::JournalError);
                            self.targets[position].staged = None;
                            return Ok(());
                        }
                        let size = self.targets[position].record.before.size.unwrap_or(0);
                        let record = &mut self.targets[position].record;
                        record.before.store = variant.mode();
                        record.bytes_held = size;
                        record.in_place = false;
                        self.targets[position].staged = None;
                        // `switch` flushes the link of the hard-link variant
                        // itself, before the target changes.
                        if variant != Exchange::Link
                            && let Ok(blobs) = self.parts[self.targets[position].part.unwrap_or(0)]
                                .entry
                                .blobs()
                        {
                            if deferred {
                                self.unsynced.dir(blobs);
                            } else if fs.sync_dir(&scope, &blobs).is_err() {
                                self.targets[position].record.irreversible =
                                    Some(Irreversible::JournalError);
                            }
                        }
                        return Ok(());
                    }
                    Err(Switch::Before(_)) => {}
                    Err(Switch::Exchanged(_)) => {
                        if fs.exchange(&scope, staged, path).is_err() {
                            self.targets[position].degrade(Irreversible::JournalError);
                            self.targets[position].staged = None;
                            return Ok(());
                        }
                    }
                }
            }
            if fs.metadata(blob_path, Follow::NoFollow).is_err() {
                let tmp = self.parts[self.targets[position].part.unwrap_or(0)]
                    .entry
                    .tmp();
                let plan_clone = self.targets[position]
                    .plan
                    .as_ref()
                    .is_some_and(|p| p.clone);
                let copied = tmp.and_then(|tmp| {
                    blob::preserve_copy(fs, &scope, path, &tmp, blob_path, plan_clone)
                });
                match copied {
                    Ok(mode) => {
                        let size = self.targets[position].record.before.size.unwrap_or(0);
                        let record = &mut self.targets[position].record;
                        record.before.store = mode;
                        record.bytes_held = size;
                    }
                    Err(error) => {
                        self.targets[position].degrade(if error.kind == FsErrorKind::NoSpace {
                            Irreversible::NoSpaceForCopy
                        } else {
                            Irreversible::JournalError
                        })
                    }
                }
            }
        }
        let in_place = self.targets[position].record.in_place || !complete;
        if in_place {
            let mut handle = fs
                .open_in_place(&scope, path)
                .map_err(|error| target_error(index, error))?;
            handle
                .set_length(0)
                .and_then(|()| blob::stream_copy(fs, staged, handle.as_mut(), None).map(|_| ()))
                .and_then(|()| handle.sync(Durability::Full))
                .map_err(|error| target_error(index, error))?;
            drop(handle);
            fs.remove_file(&scope, staged)
                .map_err(|error| target_error(index, error))?;
            self.targets[position].record.in_place = true;
            if self.targets[position].record.before.links.unwrap_or(1) > 1 {
                let notable = JournalNotable::HardlinkSetPreserved {
                    path: self.targets[position].record.path.clone(),
                };
                self.targets[position].record.notable.push(notable);
            }
        } else {
            fs.rename(&scope, staged, path, RenameMode::Replace)
                .map_err(|error| target_error(index, error))?;
        }
        self.targets[position].staged = None;
        Ok(())
    }

    fn security_kept(&self, position: usize, path: &AbsPath) -> bool {
        match (
            &self.targets[position].old_meta,
            self.ctx.fs.extended_metadata(path, Follow::NoFollow),
        ) {
            (Some(before), Ok(after)) => overwrite::security_kept(before, &after),
            _ => false,
        }
    }

    fn append(&mut self, position: usize, bytes: &[u8]) -> Result<Applied> {
        let fs = self.ctx.fs.as_ref();
        let scope = self.scope();
        let index = self.targets[position].index();
        let path = self.targets[position].fs_path()?.clone();
        self.check_expect(&self.targets[position])?;
        let as_target = |error: FsError| target_error(index, error);
        let mut handle = if self.targets[position].record.before.exists {
            fs.open_in_place(&scope, &path).map_err(as_target)?
        } else {
            fs.create_new(&scope, &path).map_err(as_target)?
        };
        self.targets[position].changed = true;
        self.targets[position].created = !self.targets[position].record.before.exists;
        handle
            .seek(SeekFrom::End(0))
            .map_err(|_| as_target(FsError::new(FsErrorKind::Io, "seek", None)))?;
        manifest::write_all(handle.as_mut(), bytes).map_err(as_target)?;
        handle.sync(Durability::Full).map_err(as_target)?;
        drop(handle);
        if self.targets[position].created {
            blob::sync_parents(fs, &scope, &[&path]).map_err(as_target)?;
        }
        self.step(FailPoint::ApplySwitched, index)?;
        let hash = match self.targets[position].prefix.clone() {
            Some(mut prefix) => {
                prefix.update(bytes);
                ContentHash(prefix.finalize().into())
            }
            None if !self.targets[position].record.before.exists => manifest::hash_bytes(bytes),
            None => fs.content_hash(&path).map_err(as_target)?,
        };
        self.finish(position, Some(hash))
    }

    fn remove(&mut self, position: usize) -> Result<Applied> {
        let index = self.targets[position].index();
        let path = self.targets[position].fs_path()?.clone();
        self.check_expect(&self.targets[position])?;
        self.move_into_journal(position, &path)?;
        self.step(FailPoint::ApplySwitched, index)?;
        if let Some(parent) = path.parent() {
            self.ctx
                .fs
                .sync_dir(&self.scope(), &parent)
                .map_err(|error| target_error(index, error))?;
        }
        self.finish(position, None)
    }

    /// Moves the entry at `path` into the journal as the previous content of
    /// the target, by rename or, across layers, by copy. A target the journal
    /// was not asked to keep, or cannot hold for lack of space, is removed
    /// without preservation; any other failure of the journal fails the
    /// operation with the entry still in place.
    fn move_into_journal(&mut self, position: usize, path: &AbsPath) -> Result<()> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.scope();
        let index = self.targets[position].index();
        let entry = self.targets[position]
            .part
            .map(|part| self.parts[part].entry.clone());
        let id = BlobId::new(index, BlobRole::Previous);
        if self.targets[position].mode() == PreserveMode::Moved
            && let Some(entry) = entry
        {
            let blob_path = entry.blob(id).map_err(|error| target_error(index, error))?;
            match fs.rename(&scope, path, &blob_path, RenameMode::NoReplace) {
                Ok(()) => {
                    self.targets[position].changed = true;
                    self.account_moved(position);
                    // The directory of the target is flushed by the caller.
                    if blob::sync_parents(fs, &self.journal_scope(), &[&blob_path]).is_err() {
                        self.targets[position].record.irreversible =
                            Some(Irreversible::JournalError);
                    }
                    return Ok(());
                }
                Err(error) if error.kind == FsErrorKind::CrossDevice => {
                    let copied = entry.tmp().and_then(|tmp| {
                        let temp = tmp.join_name(&format!("{id}.blob"))?;
                        blob::copy_entry(fs, &scope, path, &temp)?;
                        blob::sync_tree(fs, &scope, &temp)?;
                        fs.rename(&scope, &temp, &blob_path, RenameMode::NoReplace)?;
                        blob::sync_parents(fs, &scope, &[&temp, &blob_path])
                    });
                    match copied {
                        Ok(()) => {
                            let size = self.targets[position].record.before.size.unwrap_or(0);
                            let target = &mut self.targets[position];
                            target.record.before.store = PreserveMode::Copy;
                            target.record.bytes_held = size;
                            target.copied_on_move = true;
                            target.record.notable.push(JournalNotable::CrossDeviceCopy {
                                path: target.record.path.clone(),
                            });
                        }
                        Err(error) => {
                            let _ = entry.tmp().and_then(|tmp| {
                                blob::remove_entry(
                                    fs,
                                    &scope,
                                    &tmp.join_name(&format!("{id}.blob"))?,
                                )
                            });
                            if error.kind != FsErrorKind::NoSpace {
                                return Err(journal_kept_nothing(index, error));
                            }
                            self.targets[position].degrade(Irreversible::NoSpaceForCopy);
                        }
                    }
                }
                Err(error) => return Err(journal_kept_nothing(index, error)),
            }
        } else if self.targets[position].mode() == PreserveMode::Moved {
            return Err(journal_kept_nothing(
                index,
                FsError::new(FsErrorKind::Io, "journal", None),
            ));
        }
        blob::remove_entry(fs, &scope, path).map_err(|error| target_error(index, error))?;
        self.targets[position].changed = true;
        Ok(())
    }

    fn account_moved(&mut self, position: usize) {
        let target = &mut self.targets[position];
        let before = &target.record.before;
        let shared = before.kind == Some(FileKind::File) && before.links.unwrap_or(1) > 1;
        let mut held = HeldBytes::default();
        target.record.bytes_held = held.count(
            PreserveMode::Moved,
            before.size.unwrap_or(0),
            before.file_id,
            shared,
        );
        if shared {
            target.record.notable.push(JournalNotable::SharedData {
                path: target.record.path.clone(),
            });
        }
    }

    fn rename(&mut self, position: usize) -> Result<Applied> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.scope();
        let index = self.targets[position].index();
        let from = self.targets[position].fs_path()?.clone();
        self.check_expect(&self.targets[position])?;
        let Some(dest) = self.targets[position].dest.as_ref() else {
            return Err(misuse("the rename has no destination"));
        };
        let to = dest.path.clone();
        let cross = dest.cross;
        let as_target = |error: FsError| target_error(index, error);
        self.displace_destination(position)?;
        if cross {
            blob::copy_entry(fs, &scope, &from, &to).map_err(as_target)?;
            if let Some(dest) = self.targets[position].dest.as_mut() {
                dest.created = true;
            }
            blob::sync_tree(fs, &scope, &to).map_err(as_target)?;
            if let Some(dest) = self.targets[position].dest.as_mut() {
                dest.created = true;
                dest.record.after = Some(AfterRecord {
                    exists: true,
                    ..AfterRecord::default()
                });
            }
            self.move_into_journal(position, &from)?;
        } else {
            fs.rename(&scope, &from, &to, RenameMode::NoReplace)
                .map_err(as_target)?;
            self.targets[position].changed = true;
        }
        self.step(FailPoint::ApplySwitched, index)?;
        for dir in [from.parent(), to.parent()].into_iter().flatten() {
            fs.sync_dir(&scope, &dir).map_err(as_target)?;
        }
        let after = after_record(ctx, &to, None).map_err(as_target)?;
        let state = after.to_state();
        self.targets[position].record.after = Some(after);
        Ok(Applied {
            index,
            after: state,
        })
    }

    /// Whether an earlier target of the transaction moved the entry at `path`
    /// away after the destination was declared, so that nothing is left to
    /// displace.
    fn vacated(&self, position: usize, path: &AbsPath) -> bool {
        let moved_away = self.targets[..position]
            .iter()
            .any(|earlier| earlier.changed && earlier.path.as_ref() == Some(path));
        moved_away
            && matches!(
                self.ctx.fs.metadata(path, Follow::NoFollow),
                Err(error) if error.kind == FsErrorKind::NotFound
            )
    }

    /// Drops the preservation planned for a destination that was vacated.
    fn forget_destination(&mut self, position: usize) {
        if let Some(dest) = self.targets[position].dest.as_mut() {
            dest.record.before.exists = false;
            dest.record.before.store = PreserveMode::Created;
            dest.record.before.blob = None;
            dest.record.before.meta = None;
            dest.record.irreversible = None;
        }
        self.targets[position].record.replaced = None;
    }

    fn displace_destination(&mut self, position: usize) -> Result<()> {
        let fs = self.ctx.fs.as_ref();
        let scope = self.scope();
        let index = self.targets[position].index();
        let Some(dest) = self.targets[position].dest.as_ref() else {
            return Ok(());
        };
        if !dest.record.before.exists {
            return Ok(());
        }
        let path = dest.path.clone();
        if self.vacated(position, &path) {
            self.forget_destination(position);
            return Ok(());
        }
        let blob_path = dest
            .part
            .filter(|_| dest.record.before.store == PreserveMode::Moved)
            .and_then(|part| {
                self.parts[part]
                    .entry
                    .blob(BlobId::new(index, BlobRole::Destination))
                    .ok()
            });
        // A destination that the journal should keep is never removed: when
        // it cannot be moved into the entry, the operation fails and is
        // rolled back with the destination in place. Only a destination on a
        // volume without an active journal is replaced without preservation.
        let moved = match &blob_path {
            Some(blob_path) => {
                fs.rename(&scope, &path, blob_path, RenameMode::NoReplace)
                    .map_err(|error| target_error(index, error))?;
                true
            }
            None => false,
        };
        // Both directories of the move are flushed before the destination
        // name is reused; otherwise a crash can persist the rename over the
        // destination without the move and lose the previous destination.
        let durable = match &blob_path {
            Some(blob_path) => {
                blob::sync_parents(fs, &self.journal_scope(), &[&path, blob_path]).is_ok()
            }
            None => {
                blob::remove_entry(fs, &scope, &path)
                    .map_err(|error| target_error(index, error))?;
                true
            }
        };
        if let Some(dest) = self.targets[position].dest.as_mut() {
            if moved {
                dest.moved = true;
                dest.record.bytes_held = dest.record.before.size.unwrap_or(0);
                if !durable {
                    dest.record.irreversible = Some(Irreversible::JournalError);
                }
            } else {
                dest.record.before.store = PreserveMode::None;
                dest.record.before.blob = None;
                dest.record
                    .irreversible
                    .get_or_insert(Irreversible::JournalError);
            }
            if !dest.cross {
                let mut replaced = dest.record.before.clone();
                if !moved {
                    replaced.store = PreserveMode::None;
                }
                self.targets[position].record.replaced = Some(replaced);
            }
        }
        Ok(())
    }
}

/// Restoration of targets (step 7).
impl Tx<'_> {
    fn restore_all(&mut self, cause: UndoneCause) -> std::result::Result<RollbackReport, Fault> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        self.unsynced = Unsynced::default();
        let journal_scope = self.journal_scope();
        for part in 0..self.parts.len() {
            if self.parts[part].prepared && !self.parts[part].broken {
                let entry = self.parts[part].entry.clone();
                let written = entry.state().and_then(|state| {
                    manifest::write_state(
                        fs,
                        &journal_scope,
                        entry.dir(),
                        &state,
                        StateRecord::rolling_back(cause),
                    )
                });
                if written.is_err() {
                    self.parts[part].broken = true;
                }
            }
        }
        if let Err(Fault::Halt) = ctx.hit(FailPoint::RollbackStarted, None) {
            return Err(Fault::Halt);
        }
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        for position in (0..self.targets.len()).rev() {
            let target = &self.targets[position];
            let dest_changed = target.dest.as_ref().is_some_and(|d| d.moved || d.created);
            if !target.changed && !dest_changed {
                continue;
            }
            // A restored target is durable before the entry becomes undone and
            // loses its marker; otherwise the rollback stays incomplete and
            // recovery repeats it.
            match self
                .restore(position)
                .and_then(|()| self.flush_restored(position))
            {
                Ok(()) => restored.push(self.targets[position].spec.path.clone()),
                Err(_) => failed.push(self.targets[position].spec.path.clone()),
            }
            self.targets[position].changed = false;
            if let Err(Fault::Halt) = ctx.hit(
                FailPoint::RollbackRestored,
                Some(self.targets[position].index().0),
            ) {
                return Err(Fault::Halt);
            }
        }
        let scope = self.scope();
        for target in &mut self.targets {
            if let Some(staged) = target.staged.take() {
                let _ = blob::discard(fs, &scope, &staged);
            }
        }
        if failed.is_empty() {
            for part in 0..self.parts.len() {
                self.close_part(part, Some(StateRecord::undone(cause)));
            }
        } else {
            for part in &mut self.parts {
                part.lock = None;
            }
        }
        Ok(RollbackReport {
            op: self.op,
            restored,
            failed,
        })
    }

    /// Writes the terminal state of a part (when given), removes its marker
    /// and releases its lock.
    fn close_part(&mut self, part: usize, state: Option<StateRecord>) {
        let fs = self.ctx.fs.as_ref();
        let scope = self.journal_scope();
        let target = &self.parts[part];
        let op = self.op.unwrap_or(OpId(0));
        let mut terminal = true;
        if let Some(state) = state
            && target.prepared
        {
            let entry = target.entry.clone();
            terminal = entry
                .state()
                .and_then(|path| manifest::write_state(fs, &scope, entry.dir(), &path, state))
                .is_ok();
        }
        let layout = target.layout.clone();
        if terminal {
            let removed = layout
                .pending(op)
                .and_then(|marker| blob::discard(fs, &scope, &marker))
                .and_then(|()| fs.sync_dir(&scope, &layout.pending_dir()?));
            if removed.is_ok() {
                self.parts[part].lock = None;
                if let Ok(lock) = layout.entry_lock(op) {
                    let _ = blob::discard(fs, &scope, &lock);
                }
            }
        }
        self.parts[part].lock = None;
    }

    fn restore(&mut self, position: usize) -> FsResult<()> {
        let fs = self.ctx.fs.as_ref();
        let scope = self.scope();
        let unsupported = || FsError::new(FsErrorKind::Unsupported, "restore", None);
        let target = &self.targets[position];
        let index = target.index();
        let blob_path = |role: BlobRole, part: Option<usize>| -> FsResult<AbsPath> {
            let part = part.ok_or_else(unsupported)?;
            self.parts[part].entry.blob(BlobId::new(index, role))
        };
        if let Some(dest) = &target.dest {
            if dest.created {
                blob::remove_entry(fs, &scope, &dest.path)?;
            }
            if target.changed && !dest.cross {
                let from = target.fs_path().map_err(|_| unsupported())?;
                fs.rename(&scope, &dest.path, from, RenameMode::NoReplace)?;
                // Durable before the destination name receives its block.
                blob::sync_parents(fs, &scope, &[&dest.path, from])?;
            }
            if dest.moved {
                let blob = blob_path(BlobRole::Destination, dest.part)?;
                fs.rename(&scope, &blob, &dest.path, RenameMode::NoReplace)?;
            }
            if !(target.changed && dest.cross) {
                return Ok(());
            }
        }
        if let Some(snapshot) = &target.snapshot {
            let TargetPath::Provider(key) = &target.spec.path else {
                return Err(unsupported());
            };
            let provider = self.ctx.provider(&key.scheme).ok_or_else(unsupported)?;
            if target.mode() == PreserveMode::None {
                return Err(unsupported());
            }
            return provider.restore(&scope, snapshot);
        }
        let path = target.path.clone().ok_or_else(unsupported)?;
        match target.mode() {
            PreserveMode::Exchange | PreserveMode::ReplaceBackup | PreserveMode::Hardlink => {
                let blob = blob_path(BlobRole::Previous, target.part)?;
                fs.rename(&scope, &blob, &path, RenameMode::Replace)
            }
            PreserveMode::Copy if target.record.before.inline.is_some() => {
                self.restore_inline(position, &path)
            }
            PreserveMode::Clone | PreserveMode::Copy if target.copied_on_move => {
                let blob = blob_path(BlobRole::Previous, target.part)?;
                blob::copy_entry(fs, &scope, &blob, &path)?;
                blob::sync_tree(fs, &scope, &path)
            }
            PreserveMode::Clone | PreserveMode::Copy => {
                let blob = blob_path(BlobRole::Previous, target.part)?;
                if target.record.in_place {
                    let mut handle = fs.open_in_place(&scope, &path)?;
                    handle.set_length(0)?;
                    blob::stream_copy(fs, &blob, handle.as_mut(), None)?;
                    handle.sync(Durability::Full)
                } else {
                    fs.rename(&scope, &blob, &path, RenameMode::Replace)?;
                    match &target.old_meta {
                        Some(meta) => fs.restore_meta(&scope, &path, meta).map(|_| ()),
                        None => Ok(()),
                    }
                }
            }
            PreserveMode::Moved => {
                if target.created {
                    blob::remove_entry(fs, &scope, &path)?;
                }
                let blob = blob_path(BlobRole::Previous, target.part)?;
                match fs.rename(&scope, &blob, &path, RenameMode::NoReplace) {
                    Err(error) if error.kind == FsErrorKind::CrossDevice => {
                        blob::copy_entry(fs, &scope, &blob, &path)?;
                        blob::sync_tree(fs, &scope, &path)
                    }
                    other => other,
                }
            }
            PreserveMode::Created => blob::remove_entry(fs, &scope, &path),
            PreserveMode::Meta => {
                let meta = target.old_meta.as_ref().ok_or_else(unsupported)?;
                fs.restore_meta(&scope, &path, meta).map(|_| ())
            }
            PreserveMode::Append if target.created => blob::remove_entry(fs, &scope, &path),
            PreserveMode::Append => {
                fs.truncate(&scope, &path, target.record.before.size.unwrap_or(0))
            }
            _ => Err(unsupported()),
        }
    }

    /// Writes the previous content kept in the manifest back over the target:
    /// a new file with the previous metadata is flushed and renamed over it.
    fn restore_inline(&self, position: usize, path: &AbsPath) -> FsResult<()> {
        let fs = self.ctx.fs.as_ref();
        let scope = self.scope();
        let target = &self.targets[position];
        let bytes = target.record.before.inline.as_deref().ok_or_else(|| {
            FsError::new(FsErrorKind::Unsupported, "restore", Some(path.as_path()))
        })?;
        let staged = self
            .staged_path(position)
            .map_err(|_| FsError::new(FsErrorKind::InvalidPath, "restore", Some(path.as_path())))?;
        let _ = blob::discard(fs, &scope, &staged);
        let mut handle = fs.create_new(&scope, &staged)?;
        manifest::write_all(handle.as_mut(), bytes)?;
        if let Some(meta) = &target.old_meta {
            handle.apply_meta(meta)?;
        }
        handle.sync(Durability::Full)?;
        drop(handle);
        fs.rename(&scope, &staged, path, RenameMode::Replace)
    }

    /// Flushes what [`Self::restore`] changed for one target: the directories
    /// of the target, of its destination and of the blocks moved back, and
    /// the target itself when its metadata or length was restored.
    fn flush_restored(&self, position: usize) -> FsResult<()> {
        let fs = self.ctx.fs.as_ref();
        let scope = self.scope();
        let target = &self.targets[position];
        let Some(path) = &target.path else {
            return Ok(());
        };
        let index = target.index();
        let mut changed = vec![path.clone()];
        if let Some(dest) = &target.dest {
            changed.push(dest.path.clone());
            if dest.moved
                && let Some(part) = dest.part
            {
                changed.push(
                    self.parts[part]
                        .entry
                        .blob(BlobId::new(index, BlobRole::Destination))?,
                );
            }
        }
        let mode = target.mode();
        if blob_holding(mode)
            && let Some(part) = target.part
        {
            changed.push(
                self.parts[part]
                    .entry
                    .blob(BlobId::new(index, BlobRole::Previous))?,
            );
        }
        let refs: Vec<&AbsPath> = changed.iter().collect();
        blob::sync_parents(fs, &scope, &refs)?;
        let rewritten = matches!(mode, PreserveMode::Meta | PreserveMode::Append)
            || (matches!(mode, PreserveMode::Clone | PreserveMode::Copy)
                && !target.record.in_place
                && !target.copied_on_move);
        if rewritten && !target.created {
            blob::sync_entry(fs, &scope, path)?;
        }
        Ok(())
    }
}

/// Modes whose previous content lives in a block of the entry.
fn blob_holding(mode: PreserveMode) -> bool {
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

/// Commit (step 6).
impl Tx<'_> {
    fn do_commit(&mut self) -> Result<OpHandle> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        if let Err(error) = self.flush_unsynced() {
            return Err(self.fail(error));
        }
        let dropped: Vec<usize> = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, t)| t.spec.action == TargetAction::ExternalWrite && !t.observed)
            .map(|(position, _)| position)
            .collect();
        for position in &dropped {
            if let (Some(part), Some(_)) = (
                self.targets[*position].part,
                &self.targets[*position].record.before.blob,
            ) {
                let index = self.targets[*position].index();
                if let Ok(blob) = self.parts[part]
                    .entry
                    .blob(BlobId::new(index, BlobRole::Previous))
                {
                    let _ = blob::remove_entry(fs, &self.journal_scope(), &blob);
                }
            }
        }
        self.targets = std::mem::take(&mut self.targets)
            .into_iter()
            .enumerate()
            .filter(|(position, _)| !dropped.contains(position))
            .map(|(_, target)| target)
            .collect();
        let has_markers = !self.irreversible.is_empty() || !self.exempt.is_empty();
        if self.targets.is_empty() && !has_markers {
            self.release_parts();
            return Ok(self.handle(None));
        }
        if self.op.is_none() && self.unavailable.is_none() {
            if self.number().is_err() {
                return Err(self.halt());
            }
            if self.op.is_some() && self.parts.is_empty() {
                let volume = fs.volume(ctx.state.root()).ok();
                if let Some(volume) = volume {
                    match self.part_for(&volume) {
                        Ok(part) => {
                            if let Err(Fault::Halt) = self.write_prepared(part) {
                                return Err(self.halt());
                            }
                        }
                        Err(Located::Halt) => return Err(self.halt()),
                        Err(Located::Skip(_)) => {}
                    }
                }
            }
        }
        if let Err(Fault::Halt) = ctx.hit(FailPoint::ApplyDone, None) {
            return Err(self.halt());
        }
        let Some(op) = self.op else {
            return Ok(self.handle(None));
        };
        match self.commit_parts(op) {
            Ok(()) => {}
            Err(Fault::Halt) => return Err(self.halt()),
            Err(Fault::Fs(_)) => {}
        }
        Ok(self.handle(Some(op)))
    }

    fn commit_parts(&mut self, op: OpId) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.journal_scope();
        let lock = ctx.lock_account().ok();
        let index_path = ctx.state.index()?;
        let chain = if lock.is_some() {
            manifest::last_index_row(fs, &index_path)
                .ok()
                .flatten()
                .and_then(|row| row.digest())
        } else {
            None
        };
        let mut order: Vec<usize> = (0..self.parts.len()).collect();
        order.sort_by(|a, b| self.parts[*a].volume.id.cmp(&self.parts[*b].volume.id));
        for &part in &order {
            if self.parts[part].broken || !self.parts[part].prepared || lock.is_none() {
                self.abandon(part);
                continue;
            }
            let mut manifest = self.manifest(part, true, chain);
            let entry = self.parts[part].entry.clone();
            let written = manifest
                .seal()
                .map_err(|_| FsError::new(FsErrorKind::Io, "seal", None))
                .and_then(|digest| {
                    let bytes = manifest
                        .encode()
                        .map_err(|_| FsError::new(FsErrorKind::Io, "encode", None))?;
                    manifest::write_atomic(fs, &scope, entry.dir(), &entry.manifest()?, &bytes)?;
                    Ok(digest)
                });
            match written {
                Ok(digest) => self.parts[part].digest = Some(digest),
                Err(_) => self.abandon(part),
            }
        }
        for &part in &order {
            if self.parts[part].digest.is_none() {
                continue;
            }
            if let Err(fault) = self.write_committed(part) {
                if fault == Fault::Halt {
                    return Err(Fault::Halt);
                }
                self.abandon(part);
            }
        }
        let parts: Vec<IndexPart> = order
            .iter()
            .filter_map(|&part| {
                let p = &self.parts[part];
                p.digest.map(|digest| IndexPart {
                    vol: p.volume.id.clone(),
                    dir: p.layout.root().clone(),
                    manifest_sha256: digest,
                })
            })
            .collect();
        let row = IndexRow {
            op,
            time: ctx.clock.now(),
            kind: self.request.kind.clone(),
            session: self.request.session.clone(),
            bulk: self.request.bulk,
            counts_as_step: self.request.counts_as_step,
            parts,
        };
        let indexed =
            lock.is_some() && manifest::append_index_row(fs, &scope, &index_path, &row).is_ok();
        drop(lock);
        let linked = match self.link.after_commit.take() {
            Some(update) if indexed => update(ctx, op).is_ok(),
            Some(_) => false,
            None => true,
        };
        for &part in &order {
            if self.parts[part].digest.is_some() {
                if indexed && linked {
                    self.close_part(part, None);
                } else {
                    self.parts[part].lock = None;
                }
                ctx.admission.after_operation(&self.parts[part].volume.id);
            }
        }
        Ok(())
    }

    fn write_committed(&mut self, part: usize) -> std::result::Result<(), Fault> {
        let ctx = self.ctx;
        let fs = ctx.fs.as_ref();
        let scope = self.journal_scope();
        let entry = self.parts[part].entry.clone();
        let state = entry.state()?;
        // The temporary file lives next to `STATE`: the rename then changes
        // one directory, and one flush of it makes the state durable.
        let mut handle = fs.create_temp_in(&scope, entry.dir(), ".state")?;
        let temp = handle.path().clone();
        manifest::write_all(
            handle.as_mut(),
            &StateRecord::new(nurt_contracts::journal::EntryState::Committed).encode(),
        )?;
        handle.sync(Durability::Full)?;
        drop(handle);
        ctx.hit(FailPoint::CommitWritten, None)?;
        fs.rename(&scope, &temp, &state, RenameMode::Replace)?;
        ctx.hit(FailPoint::CommitRenamed, None)?;
        fs.sync_dir(&scope, entry.dir())?;
        ctx.hit(FailPoint::CommitDurable, None)?;
        Ok(())
    }

    /// Gives up the journal of a part whose commit failed after its changes
    /// were applied: the marker is removed so that recovery does not revert
    /// the operation, and its targets report `journal_error`.
    fn abandon(&mut self, part: usize) {
        self.parts[part].digest = None;
        self.close_part(part, None);
        for target in &mut self.targets {
            if target.part == Some(part) && target.record.irreversible.is_none() {
                target.degrade(Irreversible::JournalError);
            }
        }
    }

    fn release_parts(&mut self) {
        for part in 0..self.parts.len() {
            if self.parts[part].prepared {
                self.close_part(part, Some(StateRecord::undone(UndoneCause::RolledBack)));
            } else {
                self.close_part(part, None);
            }
        }
    }

    fn handle(&self, op: Option<OpId>) -> OpHandle {
        let mut irreversible = Vec::new();
        let mut notable = Vec::new();
        let mut outcomes = Vec::new();
        for target in &self.targets {
            let record = &target.record;
            if let Some(reason) = record.irreversible {
                irreversible.push(IrreversibleEntry {
                    path: Some(record_path(&target.spec.path)),
                    raw_path: conflict::raw_path(&target.spec.path),
                    ..IrreversibleEntry::new(reason)
                });
            }
            if let Some(dest) = &target.dest
                && let Some(reason) = dest.record.irreversible
            {
                irreversible.push(IrreversibleEntry {
                    path: Some(dest.path.to_string()),
                    raw_path: dest.path.raw_bytes(),
                    ..IrreversibleEntry::new(reason)
                });
            }
            notable.extend(record.notable.iter().cloned());
            outcomes.push(TargetOutcome {
                index: record.index,
                path: target.spec.path.clone(),
                action: record.action.clone(),
                mode: record.before.store,
                bytes_held: record.bytes_held
                    + target.dest.as_ref().map_or(0, |d| d.record.bytes_held),
                before: record.before.to_state(),
                after: record.after.as_ref().map(AfterRecord::to_state),
                irreversible: target.irreversible(),
                notable: record.notable.clone(),
            });
        }
        irreversible.extend(self.irreversible.iter().cloned());
        notable.extend(self.notable.iter().cloned());
        OpHandle {
            op,
            kind: self.request.kind.clone(),
            bulk: self.request.bulk,
            targets: outcomes,
            irreversible,
            notable,
            exempt: self.exempt.clone(),
        }
    }
}

impl Tx<'_> {
    fn do_observe(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        let mut applied = Vec::new();
        for change in changes {
            let declared = change.declared.or_else(|| {
                self.targets
                    .iter()
                    .find(|t| {
                        t.spec.action == TargetAction::ExternalWrite
                            && t.path.as_ref() == Some(&change.path)
                    })
                    .map(Target::index)
            });
            let Some(index) = declared else {
                self.irreversible.push(IrreversibleEntry {
                    path: Some(change.path.to_string()),
                    raw_path: change.path.raw_bytes(),
                    ..IrreversibleEntry::new(Irreversible::PreviousContentNotKept)
                });
                continue;
            };
            let position = self.position(index)?;
            if self.targets[position].spec.action != TargetAction::ExternalWrite {
                return Err(misuse(
                    "the change names a target not declared as external write",
                ));
            }
            let hash = match change.kind {
                nurt_contracts::journal::ExternalChangeKind::Deleted => None,
                _ => Some(
                    self.ctx
                        .fs
                        .content_hash(&change.path)
                        .map_err(|error| target_error(index, error))?,
                ),
            };
            self.targets[position].observed = true;
            applied.push(self.finish(position, hash)?);
        }
        Ok(applied)
    }
}

impl MutationTx for Tx<'_> {
    fn op(&self) -> Option<OpId> {
        self.op
    }

    fn allow_incremental(&mut self) {
        self.incremental = true;
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        self.usable()?;
        if self.phase == Phase::Applying && !self.incremental {
            let error = misuse("preserve was called after the first change");
            return Err(self.fail(error));
        }
        match self.do_preserve(targets) {
            Ok(preserved) => Ok(preserved),
            Err(error) if self.phase == Phase::Halted => Err(error),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        self.usable()?;
        match self.do_stage(target) {
            Ok(handle) => Ok(handle),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.usable()?;
        match self.do_apply(target, mutation) {
            Ok(applied) => Ok(applied),
            Err(error) if self.phase == Phase::Halted => Err(error),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.usable()?;
        match self.do_observe(changes) {
            Ok(applied) => Ok(applied),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.exempt.push(write);
    }

    fn commit(mut self: Box<Self>) -> Result<OpHandle> {
        self.usable()?;
        let result = self.do_commit();
        if self.phase != Phase::Halted {
            self.phase = Phase::Done;
        }
        result
    }

    fn rollback(mut self: Box<Self>, reason: RollbackReason) -> Result<RollbackReport> {
        match self.phase {
            Phase::Halted => return Err(halted()),
            Phase::Failed | Phase::Done => {
                self.phase = Phase::Done;
                return Ok(RollbackReport {
                    op: self.op,
                    restored: Vec::new(),
                    failed: Vec::new(),
                });
            }
            Phase::Open | Phase::Applying => {}
        }
        let cause = match reason {
            RollbackReason::VerifyFailed => UndoneCause::VerifyFailed,
            RollbackReason::ValidateFailed => UndoneCause::ValidateFailed,
            _ => UndoneCause::RolledBack,
        };
        let outcome = self.restore_all(cause);
        match outcome {
            Ok(report) => {
                self.phase = Phase::Done;
                Ok(report)
            }
            Err(_) => Err(self.halt()),
        }
    }
}

impl Drop for Tx<'_> {
    fn drop(&mut self) {
        if matches!(self.phase, Phase::Open | Phase::Applying) {
            let _ = self.restore_all(UndoneCause::RolledBack);
            self.phase = Phase::Done;
        }
    }
}

impl std::fmt::Debug for Tx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tx")
            .field("op", &self.op)
            .field("phase", &self.phase)
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use nurt_contracts::bound::{BoundDocument, BoundProgram};
    use nurt_contracts::config::{OverwriteStrategy, SizeSetting};
    use nurt_contracts::fs::{Fs, MetaChange, ProviderKey, ProviderScheme, ProviderValue};
    use nurt_contracts::journal::{EntryState, ExternalChangeKind, InactiveReason, OpKind};
    use nurt_fs::failpoint::{FailAction, FailPlan, FailRule, NoFailpoints, SeededFailpoints};

    use super::*;
    use crate::store::test_support::{Fixture, unflushed};

    fn request(command: &str) -> BeginRequest {
        BeginRequest {
            kind: OpKind::command(command),
            session: None,
            program: BoundDocument::new(
                "cat-1",
                "corr-1",
                BoundProgram {
                    statements: Vec::new(),
                },
            ),
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            preserve_budget: None,
        }
    }

    fn spec(path: &AbsPath, action: TargetAction) -> TargetSpec {
        TargetSpec {
            path: TargetPath::Fs(path.clone()),
            action,
            expect: Expect::Unchecked,
            size_hint: None,
        }
    }

    /// Fixture with a shared context.
    struct Env {
        fixture: Fixture,
        ctx: JournalCtx,
    }

    impl Env {
        fn new() -> Self {
            let fixture = Fixture::new();
            let ctx = fixture.ctx();
            Self { fixture, ctx }
        }

        fn audited() -> Self {
            let fixture = Fixture::new();
            let ctx = fixture.audited_ctx();
            Self { fixture, ctx }
        }

        /// Writes `bytes` to `<volume>/<name>` outside any transaction.
        fn file(&self, relative: &str, bytes: &[u8]) -> AbsPath {
            let path = self.fixture.fs.path(relative);
            self.fixture.write(&path, bytes);
            path
        }

        fn layout(&self, volume: usize) -> JournalLayout {
            JournalLayout::new(self.fixture.journal_dir(volume))
        }

        fn state(&self, volume: usize, op: u64) -> StateRecord {
            let entry = self.layout(volume).entry(OpId(op)).unwrap();
            manifest::read_state(&self.fixture.fs, &entry.state().unwrap()).unwrap()
        }

        fn manifest(&self, volume: usize, op: u64) -> Manifest {
            let entry = self.layout(volume).entry(OpId(op)).unwrap();
            manifest::read_manifest(&self.fixture.fs, &entry.manifest().unwrap()).unwrap()
        }

        fn blob(&self, volume: usize, op: u64, index: u32, role: BlobRole) -> AbsPath {
            let entry = self.layout(volume).entry(OpId(op)).unwrap();
            entry.blob(BlobId::new(TargetIndex(index), role)).unwrap()
        }

        fn pending(&self, volume: usize) -> Vec<OpId> {
            layout::pending_entries(&self.fixture.fs, &self.layout(volume)).unwrap()
        }

        fn index(&self) -> Vec<IndexRow> {
            manifest::read_index(&self.fixture.fs, &self.ctx.state.index().unwrap()).unwrap()
        }

        fn read(&self, path: &AbsPath) -> Vec<u8> {
            self.fixture.read(path)
        }

        /// Replaces the content of `path` in one committed operation.
        fn overwrite(&self, path: &AbsPath, bytes: &[u8]) -> OpHandle {
            let mut tx = begin(&self.ctx, request("write"));
            tx.preserve(&[spec(path, TargetAction::Modify)]).unwrap();
            tx.apply(TargetIndex(0), Mutation::WriteBytes(bytes.to_vec()))
                .unwrap();
            tx.commit().unwrap()
        }
    }

    fn position(log: &[String], predicate: impl Fn(&str) -> bool) -> usize {
        log.iter()
            .position(|entry| predicate(entry))
            .unwrap_or_else(|| panic!("no matching entry in {log:#?}"))
    }

    #[test]
    fn overwrite_of_a_large_file_exchanges_names_without_copying() {
        let env = Env::new();
        let path = env.fixture.fs.path("vol-a/big.bin");
        std::fs::File::create(path.as_path())
            .unwrap()
            .set_len(1 << 30)
            .unwrap();
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.op, Some(OpId(1)));
        let outcome = &handle.targets[0];
        assert_eq!(outcome.mode, PreserveMode::Exchange);
        assert_eq!(outcome.bytes_held, 1 << 30);
        assert_eq!(env.fixture.fs.copied(), 0);
        assert_eq!(env.read(&path), b"new");
        let blob = env.blob(0, 1, 0, BlobRole::Previous);
        assert_eq!(std::fs::metadata(blob.as_path()).unwrap().len(), 1 << 30);
        assert_eq!(env.state(0, 1).state, EntryState::Committed);
        let manifest = env.manifest(0, 1);
        assert!(matches!(
            crate::integrity::verify_manifest(&manifest.encode().unwrap()),
            crate::integrity::ManifestVerdict::Intact { .. }
        ));
        assert_eq!(manifest.integrity.chain, None);
        assert!(manifest.targets[0].after.as_ref().unwrap().hash.is_some());
        assert!(env.pending(0).is_empty());
        assert_eq!(env.index().len(), 1);
        assert!(!handle.irreversible.iter().any(|entry| entry.path.is_some()));
    }

    /// Creates `count` small files `vol-a/bulk/f<N>.txt`.
    fn bulk_files(env: &Env, count: usize) -> Vec<AbsPath> {
        std::fs::create_dir_all(env.fixture.fs.path("vol-a/bulk").as_path()).unwrap();
        (0..count)
            .map(|index| {
                let path = env.file(
                    &format!("vol-a/bulk/f{index}.txt"),
                    format!("old {index}\n").as_bytes(),
                );
                #[cfg(unix)]
                {
                    let mut permissions = std::fs::metadata(path.as_path()).unwrap().permissions();
                    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o640);
                    std::fs::set_permissions(path.as_path(), permissions).unwrap();
                }
                path
            })
            .collect()
    }

    #[cfg(unix)]
    fn mode_of(path: &AbsPath) -> u32 {
        std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(path.as_path()).unwrap().permissions(),
        ) & 0o777
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // one scenario with many checks
    fn bulk_overwrite_keeps_small_previous_content_in_a_packed_manifest() {
        let env = Env::new();
        let count = manifest::PACKED_MIN_TARGETS + 4;
        let paths = bulk_files(&env, count);
        let specs: Vec<TargetSpec> = paths
            .iter()
            .map(|path| spec(path, TargetAction::Modify))
            .collect();
        let mut tx = begin(&env.ctx, request("replace"));
        let preserved = tx.preserve(&specs).unwrap();
        assert!(preserved.iter().all(|kept| kept.mode == PreserveMode::Copy));
        for (index, kept) in preserved.iter().enumerate() {
            tx.apply(
                kept.index,
                Mutation::WriteBytes(format!("new {index}\n").into_bytes()),
            )
            .unwrap();
        }
        let handle = tx.commit().unwrap();
        let op = handle.op.unwrap();
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(env.read(path), format!("new {index}\n").as_bytes());
            #[cfg(unix)]
            assert_eq!(mode_of(path), 0o640);
        }
        let entry = env.layout(0).entry(op).unwrap();
        assert_eq!(
            std::fs::read_dir(entry.blobs().unwrap().as_path())
                .unwrap()
                .count(),
            0,
            "no block is written for inline content"
        );
        let bytes = manifest::read_bytes(&env.fixture.fs, &entry.manifest().unwrap()).unwrap();
        assert!(matches!(
            crate::integrity::verify_manifest(&bytes),
            crate::integrity::ManifestVerdict::Intact { .. }
        ));
        let manifest = env.manifest(0, op.0);
        assert_eq!(manifest.v, manifest::PACKED_FORMAT_VERSION);
        assert_eq!(manifest.targets.len(), count);
        for (index, record) in manifest.targets.iter().enumerate() {
            assert_eq!(
                record.before.inline.as_deref(),
                Some(format!("old {index}\n").as_bytes())
            );
            assert!(record.before.blob.is_none() && record.before.meta.is_none());
            assert_eq!(record.before.store, PreserveMode::Copy);
        }
        assert_eq!(env.state(0, op.0).state, EntryState::Committed);
    }

    #[test]
    fn rollback_of_a_bulk_overwrite_restores_inline_content_and_permissions() {
        let env = Env::new();
        let paths = bulk_files(&env, BATCH_MIN_TARGETS + 6);
        let specs: Vec<TargetSpec> = paths
            .iter()
            .map(|path| spec(path, TargetAction::Modify))
            .collect();
        let mut tx = begin(&env.ctx, request("replace"));
        let preserved = tx.preserve(&specs).unwrap();
        for kept in &preserved {
            tx.apply(kept.index, Mutation::WriteBytes(b"replaced\n".to_vec()))
                .unwrap();
        }
        assert_eq!(env.read(&paths[0]), b"replaced\n");
        tx.rollback(RollbackReason::ValidateFailed).unwrap();
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(env.read(path), format!("old {index}\n").as_bytes());
            #[cfg(unix)]
            assert_eq!(mode_of(path), 0o640);
        }
        assert_eq!(env.state(0, 1).state, EntryState::Undone);
        assert!(env.pending(0).is_empty());
    }

    #[test]
    fn failed_flush_before_the_commit_rolls_the_bulk_operation_back() {
        let env = Env::new();
        let paths = bulk_files(&env, BATCH_MIN_TARGETS + 2);
        let specs: Vec<TargetSpec> = paths
            .iter()
            .map(|path| spec(path, TargetAction::Modify))
            .collect();
        let mut tx = begin(&env.ctx, request("replace"));
        let preserved = tx.preserve(&specs).unwrap();
        for kept in &preserved {
            tx.apply(kept.index, Mutation::WriteBytes(b"replaced\n".to_vec()))
                .unwrap();
        }
        // The directory flush of the deferred changes is the first flush of the
        // directory of the targets: no change was made durable before the commit.
        env.fixture
            .fs
            .fail("sync_dir", "vol-a/bulk", FsErrorKind::Io);
        tx.commit().unwrap_err();
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(env.read(path), format!("old {index}\n").as_bytes());
        }
        assert_eq!(env.state(0, 1).state, EntryState::Undone);
        assert!(env.pending(0).is_empty());
        assert!(env.index().is_empty());
    }

    #[test]
    fn small_operations_keep_the_block_per_file_layout() {
        let env = Env::new();
        let paths = bulk_files(&env, INLINE_MIN_TARGETS - 1);
        let specs: Vec<TargetSpec> = paths
            .iter()
            .map(|path| spec(path, TargetAction::Modify))
            .collect();
        let mut tx = begin(&env.ctx, request("replace"));
        let preserved = tx.preserve(&specs).unwrap();
        assert!(
            preserved
                .iter()
                .all(|kept| kept.mode == PreserveMode::Exchange)
        );
        for kept in &preserved {
            tx.apply(kept.index, Mutation::WriteBytes(b"new\n".to_vec()))
                .unwrap();
        }
        let op = tx.commit().unwrap().op.unwrap();
        let manifest = env.manifest(0, op.0);
        assert_eq!(manifest.v, manifest::FORMAT_VERSION);
        assert!(
            manifest
                .targets
                .iter()
                .all(|record| record.before.inline.is_none() && record.before.blob.is_some())
        );
    }

    #[test]
    fn consecutive_entries_are_chained_through_the_index() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"one");
        env.overwrite(&path, b"two");
        env.overwrite(&path, b"three");
        let rows = env.index();
        assert_eq!(rows.len(), 2);
        assert_eq!(env.manifest(0, 2).integrity.chain, rows[0].digest());
        assert_eq!(
            rows[1].digest(),
            env.manifest(0, 2).integrity.manifest_sha256
        );
    }

    #[test]
    fn exchange_unsupported_falls_back_to_a_hard_link() {
        let env = Env::new();
        env.fixture
            .fs
            .configure(0, |volume| volume.features.exchange = false);
        let path = env.file("vol-a/a.txt", b"old content");
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.targets[0].mode, PreserveMode::Hardlink);
        assert_eq!(handle.targets[0].bytes_held, 11);
        assert_eq!(
            env.read(&env.blob(0, 1, 0, BlobRole::Previous)),
            b"old content"
        );
        assert_eq!(env.fixture.fs.copied(), 0);
    }

    #[test]
    fn windows_style_volume_replaces_with_a_backup_name() {
        let env = Env::new();
        env.fixture.fs.configure(0, |volume| {
            volume.features.exchange = false;
            volume.features.hardlink = false;
            volume.features.replace_with_backup = true;
        });
        let path = env.file("vol-a/a.txt", b"old");
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.targets[0].mode, PreserveMode::ReplaceBackup);
        assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"old");
    }

    #[test]
    fn incomplete_metadata_transfer_skips_the_exchange_and_writes_in_place() {
        for reflink in [false, true] {
            let env = Env::new();
            env.fixture
                .fs
                .configure(0, |volume| volume.features.reflink = reflink);
            env.fixture
                .fs
                .refuse_meta(&["account", "xattr:security.selinux"]);
            let path = env.file("vol-a/a.txt", b"old");
            let inode = std::fs::metadata(path.as_path()).unwrap();
            let handle = env.overwrite(&path, b"new");
            let expected = if reflink {
                PreserveMode::Clone
            } else {
                PreserveMode::Copy
            };
            assert_eq!(handle.targets[0].mode, expected);
            assert_eq!(env.read(&path), b"new");
            assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"old");
            use crate::store::test_support::file_ino;
            assert_eq!(
                file_ino(&std::fs::metadata(path.as_path()).unwrap()),
                file_ino(&inode)
            );
            assert!(env.manifest(0, 1).targets[0].in_place);
        }
    }

    #[test]
    fn changed_account_after_replacement_reverts_to_a_copy() {
        let env = Env::new();
        env.fixture.fs.configure(0, |volume| {
            volume.features.exchange = false;
            volume.features.hardlink = false;
            volume.features.replace_with_backup = true;
        });
        let path = env.file("vol-a/a.txt", b"old");
        env.fixture.fs.set_sid(&path, "S-1-5-21-1001");
        env.fixture.fs.transfer_sid(false);
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.targets[0].mode, PreserveMode::Copy);
        assert_eq!(env.read(&path), b"new");
        assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"old");
        let account = env
            .fixture
            .fs
            .extended_metadata(&path, Follow::NoFollow)
            .unwrap()
            .account_sid;
        assert_eq!(account.as_deref(), Some("S-1-5-21-1001"));
    }

    #[test]
    fn clone_or_copy_strategy_clones_or_copies_before_the_change() {
        for (reflink, mode) in [(true, PreserveMode::Clone), (false, PreserveMode::Copy)] {
            let mut env = Env::new();
            env.ctx.settings.overwrite = OverwriteStrategy::CloneOrCopy;
            env.fixture
                .fs
                .configure(0, |volume| volume.features.reflink = reflink);
            let path = env.file("vol-a/a.txt", b"old!");
            let handle = env.overwrite(&path, b"new");
            assert_eq!(handle.targets[0].mode, mode);
            assert_eq!(handle.targets[0].bytes_held, 4);
            let copied = if reflink { 0 } else { 4 };
            assert_eq!(env.fixture.fs.copied(), copied);
            assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"old!");
            assert_eq!(env.read(&path), b"new");
        }
    }

    #[test]
    fn lack_of_space_for_a_copy_overwrites_without_preservation() {
        let env = Env::new();
        env.fixture.fs.configure(0, |volume| {
            volume.features.exchange = false;
            volume.features.hardlink = false;
        });
        let path = env.file("vol-a/a.txt", b"old");
        env.fixture
            .fs
            .fail_always("copy_file", "a.txt", FsErrorKind::NoSpace);
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.targets[0].mode, PreserveMode::None);
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::NoSpaceForCopy)
        );
        assert_eq!(handle.irreversible[0].reason, Irreversible::NoSpaceForCopy);
        assert_eq!(env.read(&path), b"new");
    }

    #[test]
    fn file_with_several_links_is_written_in_place() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"old");
        let other = env.fixture.fs.path("vol-a/other.txt");
        std::fs::hard_link(path.as_path(), other.as_path()).unwrap();
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.targets[0].mode, PreserveMode::Copy);
        assert_eq!(env.read(&other), b"new");
        assert!(matches!(
            handle.notable.as_slice(),
            [JournalNotable::HardlinkSetPreserved { .. }]
        ));
        env.fixture
            .fs
            .fail_always("copy_file", "a.txt", FsErrorKind::NoSpace);
        let handle = env.overwrite(&path, b"newer");
        assert_eq!(handle.targets[0].mode, PreserveMode::None);
        assert_eq!(env.read(&other), b"newer");
    }

    #[test]
    fn writes_through_a_handle_opened_before_the_exchange_change_the_blob() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"old");
        let mut etag = env.fixture.fs.etag(&path, Follow::NoFollow).unwrap();
        etag.hash = Some(manifest::hash_bytes(b"old"));
        env.fixture.fs.write_after_exchange(&path, b"+late");
        let mut tx = begin(&env.ctx, request("write"));
        let mut target = spec(&path, TargetAction::Modify);
        target.expect = Expect::Etag(etag);
        let preserved = tx.preserve(&[target]).unwrap();
        assert_eq!(preserved[0].before.hash, Some(manifest::hash_bytes(b"old")));
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            env.read(&env.blob(0, 1, 0, BlobRole::Previous)),
            b"old+late"
        );
        assert_eq!(
            env.manifest(0, 1).targets[0].before.hash,
            Some(manifest::hash_bytes(b"old"))
        );
    }

    #[test]
    fn append_records_length_and_prefix_hash_and_rolls_back_by_truncation() {
        let env = Env::new();
        let path = env.file("vol-a/log.txt", b"abc");
        let mut tx = begin(&env.ctx, request("write"));
        let preserved = tx.preserve(&[spec(&path, TargetAction::Append)]).unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Append);
        assert_eq!(preserved[0].before.size, Some(3));
        assert_eq!(preserved[0].before.hash, Some(manifest::hash_bytes(b"abc")));
        let applied = tx
            .apply(TargetIndex(0), Mutation::AppendBytes(b"def".to_vec()))
            .unwrap();
        assert_eq!(applied.after.hash, Some(manifest::hash_bytes(b"abcdef")));
        tx.rollback(RollbackReason::Cancelled).unwrap();
        assert_eq!(env.read(&path), b"abc");
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::RolledBack)
        );
    }

    #[test]
    fn prepend_is_recorded_as_an_overwrite() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"body");
        let handle = env.overwrite(&path, b"head body");
        assert_eq!(handle.targets[0].mode, PreserveMode::Exchange);
        assert_eq!(env.read(&path), b"head body");
    }

    #[test]
    fn deletion_moves_a_file_or_a_whole_tree_with_one_rename() {
        let env = Env::new();
        let file = env.file("vol-a/a.txt", b"data");
        let tree = env.fixture.fs.path("vol-a/tree");
        for dir in 0..10 {
            let sub = tree.as_path().join(format!("d{dir}"));
            std::fs::create_dir_all(&sub).unwrap();
            for name in 0..100 {
                std::fs::write(sub.join(format!("f{name}")), b"x").unwrap();
            }
        }
        env.fixture.fs.clear_log();
        let mut tx = begin(&env.ctx, request("remove"));
        tx.preserve(&[
            spec(&file, TargetAction::Delete),
            spec(&tree, TargetAction::Delete),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        tx.apply(TargetIndex(1), Mutation::Remove).unwrap();
        let handle = tx.commit().unwrap();
        assert!(handle.targets.iter().all(|t| t.mode == PreserveMode::Moved));
        assert_eq!(handle.targets[0].bytes_held, 4);
        assert!(!env.fixture.exists(&file) && !env.fixture.exists(&tree));
        let tree_renames = env
            .fixture
            .fs
            .log()
            .iter()
            .filter(|entry| entry.starts_with("rename ") && entry.contains("/tree"))
            .count();
        assert_eq!(tree_renames, 1);
        assert_eq!(env.fixture.fs.copied(), 0);
        let blobs = env.layout(0).entry(OpId(1)).unwrap().blobs().unwrap();
        assert!(
            env.fixture
                .names(&blobs)
                .iter()
                .all(|name| !name.ends_with(".tree"))
        );
        assert_eq!(
            env.fixture
                .names(&env.blob(0, 1, 1, BlobRole::Previous))
                .len(),
            10
        );
    }

    #[test]
    fn deleting_a_file_with_other_names_holds_no_bytes() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"shared");
        std::fs::hard_link(path.as_path(), env.fixture.fs.path("vol-a/b.txt").as_path()).unwrap();
        let mut tx = begin(&env.ctx, request("remove"));
        tx.preserve(&[spec(&path, TargetAction::Delete)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(handle.targets[0].bytes_held, 0);
        assert!(matches!(
            handle.notable.as_slice(),
            [JournalNotable::SharedData { .. }]
        ));
    }

    #[test]
    fn creation_metadata_rename_and_truncation_use_their_modes() {
        let env = Env::new();
        let created = env.fixture.fs.path("vol-a/new.txt");
        let dir = env.fixture.fs.path("vol-a/newdir");
        let meta = env.file("vol-a/meta.txt", b"m");
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        let truncated = env.file("vol-a/trunc.txt", b"123456");
        let mut tx = begin(&env.ctx, request("mixed"));
        let mut create = spec(&created, TargetAction::Create);
        create.expect = Expect::Absent;
        let preserved = tx
            .preserve(&[
                create,
                spec(&dir, TargetAction::Create),
                spec(&meta, TargetAction::Attributes),
                spec(
                    &from,
                    TargetAction::Rename {
                        to: to.clone(),
                        replace: true,
                    },
                ),
                spec(&truncated, TargetAction::Truncate),
            ])
            .unwrap();
        let modes: Vec<PreserveMode> = preserved.iter().map(|p| p.mode).collect();
        assert_eq!(
            modes,
            [
                PreserveMode::Created,
                PreserveMode::Created,
                PreserveMode::Meta,
                PreserveMode::Rename,
                PreserveMode::Exchange
            ]
        );
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"c".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(1), Mutation::CreateDir { mode: 0o755 })
            .unwrap();
        let change = MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        };
        tx.apply(TargetIndex(2), Mutation::SetMeta(change)).unwrap();
        tx.apply(TargetIndex(3), Mutation::Rename).unwrap();
        tx.apply(TargetIndex(4), Mutation::SetLength(2)).unwrap();
        tx.commit().unwrap();
        assert_eq!(env.read(&created), b"c");
        assert!(env.fixture.exists(&dir));
        assert_eq!(env.read(&to), b"from");
        assert_eq!(env.read(&env.blob(0, 1, 3, BlobRole::Destination)), b"to");
        assert_eq!(env.read(&truncated), b"12");
        let manifest = env.manifest(0, 1);
        let renamed = manifest.target(TargetIndex(3), TargetRole::Whole).unwrap();
        assert_eq!(
            renamed.replaced.as_ref().unwrap().store,
            PreserveMode::Moved
        );
        let meta_blob = env.layout(0).entry(OpId(1)).unwrap();
        let meta_blob = meta_blob
            .blob_meta(BlobId::new(TargetIndex(2), BlobRole::Previous))
            .unwrap();
        assert!(manifest::read_meta(&env.fixture.fs, &meta_blob).is_ok());
    }

    #[test]
    fn rollback_restores_created_renamed_and_attribute_targets() {
        let env = Env::new();
        let created = env.fixture.fs.path("vol-a/new.txt");
        let meta = env.file("vol-a/meta.txt", b"m");
        crate::store::test_support::set_path_mode(meta.as_path(), 0o644).unwrap();
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        let mut tx = begin(&env.ctx, request("mixed"));
        tx.preserve(&[
            spec(&created, TargetAction::Create),
            spec(&meta, TargetAction::Attributes),
            spec(
                &from,
                TargetAction::Rename {
                    to: to.clone(),
                    replace: true,
                },
            ),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"c".to_vec()))
            .unwrap();
        let change = MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        };
        tx.apply(TargetIndex(1), Mutation::SetMeta(change)).unwrap();
        tx.apply(TargetIndex(2), Mutation::Rename).unwrap();
        let report = tx.rollback(RollbackReason::ValidateFailed).unwrap();
        assert_eq!(report.restored.len(), 3);
        assert!(!env.fixture.exists(&created));
        assert_eq!(env.read(&from), b"from");
        assert_eq!(env.read(&to), b"to");
        let mode = env
            .fixture
            .fs
            .metadata(&meta, Follow::NoFollow)
            .unwrap()
            .mode;
        assert_eq!(mode, 0o644);
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::ValidateFailed)
        );
    }

    #[test]
    fn provider_values_are_snapshotted_and_restored() {
        let env = Env::new();
        let key = |scheme: &str| ProviderKey {
            scheme: ProviderScheme(scheme.to_owned()),
            segments: vec!["HKCU".to_owned(), "Software".to_owned()],
            value: Some("Theme".to_owned()),
        };
        env.fixture
            .registry
            .values
            .lock()
            .unwrap()
            .insert(key("registry"), ProviderValue::Text("dark".to_owned()));
        let refs = Arc::new(crate::store::test_support::MemProvider::new("gitref"));
        let mut ctx = env.fixture.ctx();
        ctx.providers.push(refs.clone());
        let target = |scheme: &str| TargetSpec {
            path: TargetPath::Provider(key(scheme)),
            action: TargetAction::Modify,
            expect: Expect::Unchecked,
            size_hint: None,
        };
        let mut tx = begin(&ctx, request("registry.set"));
        let preserved = tx
            .preserve(&[target("registry"), target("gitref")])
            .unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Value);
        assert_eq!(preserved[1].mode, PreserveMode::Ref);
        let value = ProviderValue::Text("light".to_owned());
        tx.apply(TargetIndex(0), Mutation::ProviderSet(value.clone()))
            .unwrap();
        tx.apply(TargetIndex(1), Mutation::ProviderSet(value.clone()))
            .unwrap();
        assert!(env.fixture.exists(&env.blob(0, 1, 0, BlobRole::Previous)));
        tx.rollback(RollbackReason::Cancelled).unwrap();
        let values = env.fixture.registry.values.lock().unwrap();
        assert_eq!(
            values.get(&key("registry")),
            Some(&ProviderValue::Text("dark".to_owned()))
        );
        assert!(refs.values.lock().unwrap().is_empty());
    }

    #[test]
    fn external_writes_are_copied_before_and_observed_after() {
        let env = Env::new();
        let db = env.file("vol-a/db.sqlite", b"old db");
        let untouched = env.file("vol-a/untouched", b"same");
        let mut tx = begin(&env.ctx, request("sql.exec"));
        let preserved = tx
            .preserve(&[
                spec(&db, TargetAction::ExternalWrite),
                spec(&untouched, TargetAction::ExternalWrite),
            ])
            .unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Copy);
        env.fixture.write(&db, b"new db");
        let scanned = env.file("vol-a/scanned", b"made by a program");
        let applied = tx
            .observe_external(&[
                ExternalChange {
                    path: db.clone(),
                    kind: ExternalChangeKind::Modified,
                    declared: Some(TargetIndex(0)),
                },
                ExternalChange {
                    path: scanned,
                    kind: ExternalChangeKind::Created,
                    declared: None,
                },
            ])
            .unwrap();
        assert_eq!(applied[0].after.hash, Some(manifest::hash_bytes(b"new db")));
        let handle = tx.commit().unwrap();
        assert_eq!(handle.targets.len(), 1);
        assert_eq!(
            handle.irreversible[0].reason,
            Irreversible::PreviousContentNotKept
        );
        assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"old db");
        assert!(!env.fixture.exists(&env.blob(0, 1, 1, BlobRole::Previous)));
    }

    #[test]
    fn operation_with_markers_only_gets_an_entry_at_commit() {
        let env = Env::new();
        let mut tx = begin(&env.ctx, request("http.request"));
        assert_eq!(tx.op(), None);
        tx.mark_irreversible(IrreversibleEntry::new(Irreversible::Network));
        tx.record_exempt(ExemptWrite::ObjectStore {
            repo: env.fixture.fs.path("vol-a"),
        });
        let handle = tx.commit().unwrap();
        assert_eq!(handle.op, Some(OpId(1)));
        assert_eq!(handle.irreversible[0].reason, Irreversible::Network);
        assert_eq!(env.state(0, 1).state, EntryState::Committed);
        assert_eq!(env.manifest(0, 1).irreversible.len(), 1);
        assert_eq!(env.index()[0].parts.len(), 1);
        let empty = begin(&env.ctx, request("noop")).commit().unwrap();
        assert_eq!(empty.op, None);
    }

    #[test]
    fn special_files_are_never_preserved() {
        let env = Env::new();
        let path = env.file("vol-a/fifo", b"");
        env.fixture.fs.mark_special(&path);
        let mut tx = begin(&env.ctx, request("write"));
        let preserved = tx.preserve(&[spec(&path, TargetAction::Modify)]).unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::None);
        assert_eq!(preserved[0].irreversible, Some(Irreversible::SpecialFile));
    }

    #[test]
    fn operation_on_two_volumes_has_one_number_and_commits_after_all_changes() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let b = env.file("vol-b/b.txt", b"b");
        env.fixture.fs.clear_log();
        let mut tx = begin(&env.ctx, request("replace"));
        tx.preserve(&[
            spec(&a, TargetAction::Modify),
            spec(&b, TargetAction::Modify),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(1), Mutation::WriteBytes(b"B".to_vec()))
            .unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(handle.op, Some(OpId(1)));
        for volume in 0..2 {
            let manifest = env.manifest(volume, 1);
            assert_eq!(manifest.parts.len(), 2);
            assert_eq!(manifest.targets.len(), 1);
            assert_eq!(env.state(volume, 1).state, EntryState::Committed);
        }
        assert_eq!(env.index()[0].parts.len(), 2);
        let log = env.fixture.fs.log();
        let last_change = log
            .iter()
            .rposition(|entry| entry.starts_with("exchange "))
            .unwrap();
        let first_commit = position(&log, |e| e.starts_with("rename ") && e.ends_with("/STATE"));
        assert!(last_change < first_commit);
        let manifests_final = log
            .iter()
            .rposition(|e| e.starts_with("rename ") && e.ends_with("/op.json"))
            .unwrap();
        assert!(manifests_final < first_commit);
    }

    #[test]
    fn overlay_lower_layer_is_preserved_by_copy() {
        let env = Env::new();
        let dir = env.fixture.fs.path("vol-a/lower");
        std::fs::create_dir(dir.as_path()).unwrap();
        std::fs::write(dir.as_path().join("f"), b"lower").unwrap();
        env.fixture.fs.lower_layer(&dir);
        let mut tx = begin(&env.ctx, request("remove"));
        tx.preserve(&[spec(&dir, TargetAction::Delete)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(handle.targets[0].mode, PreserveMode::Copy);
        assert!(matches!(
            handle.notable.as_slice(),
            [JournalNotable::CrossDeviceCopy { .. }]
        ));
        assert!(!env.fixture.exists(&dir));
        let blob = env.blob(0, 1, 0, BlobRole::Previous);
        assert_eq!(std::fs::read(blob.as_path().join("f")).unwrap(), b"lower");

        std::fs::create_dir(dir.as_path()).unwrap();
        std::fs::write(dir.as_path().join("f"), b"lower").unwrap();
        env.fixture.fs.fail("copy_file", "/f", FsErrorKind::NoSpace);
        let mut tx = begin(&env.ctx, request("remove"));
        tx.preserve(&[spec(&dir, TargetAction::Delete)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::NoSpaceForCopy)
        );
        assert!(!env.fixture.exists(&dir));
    }

    #[test]
    fn move_across_volumes_records_source_and_destination_in_their_journals() {
        let env = Env::new();
        let from = env.file("vol-a/x.txt", b"moved");
        let to = env.fixture.fs.path("vol-b/y.txt");
        let mut tx = begin(&env.ctx, request("move"));
        tx.preserve(&[spec(
            &from,
            TargetAction::Rename {
                to: to.clone(),
                replace: false,
            },
        )])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::Rename).unwrap();
        tx.commit().unwrap();
        assert_eq!(env.read(&to), b"moved");
        assert!(!env.fixture.exists(&from));
        let source = env.manifest(0, 1);
        let source = source.target(TargetIndex(0), TargetRole::Source).unwrap();
        assert_eq!(source.before.store, PreserveMode::Moved);
        let destination = env.manifest(1, 1);
        let destination = destination
            .target(TargetIndex(0), TargetRole::Destination)
            .unwrap();
        assert_eq!(destination.before.store, PreserveMode::Created);
        assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"moved");
    }

    #[test]
    fn stale_target_rolls_back_without_a_new_entry() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let b = env.file("vol-a/b.txt", b"b");
        let mut tx = begin(&env.ctx, request("write"));
        let mut stale_spec = spec(&b, TargetAction::Modify);
        stale_spec.expect = Expect::Etag(env.fixture.fs.etag(&b, Follow::NoFollow).unwrap());
        tx.preserve(&[spec(&a, TargetAction::Modify), stale_spec])
            .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        env.fixture.write(&b, b"changed by someone");
        let error = tx
            .apply(TargetIndex(1), Mutation::WriteBytes(b"B".to_vec()))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(env.read(&a), b"a");
        assert_eq!(env.read(&b), b"changed by someone");
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::RolledBack)
        );
        assert!(env.pending(0).is_empty());
        assert!(env.index().is_empty());
        let next = tx.apply(TargetIndex(0), Mutation::WriteBytes(Vec::new()));
        assert_eq!(next.unwrap_err().code(), ErrorCode::RolledBack);
        assert_eq!(tx.commit().unwrap_err().code(), ErrorCode::RolledBack);
        assert_eq!(env.overwrite(&a, b"x").op, Some(OpId(2)));
    }

    #[test]
    fn failure_of_a_target_restores_the_targets_changed_before() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let gone = env.file("vol-a/gone.txt", b"gone");
        let b = env.file("vol-a/b.txt", b"b");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[
            spec(&a, TargetAction::Modify),
            spec(&gone, TargetAction::Delete),
            spec(&b, TargetAction::Modify),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(1), Mutation::Remove).unwrap();
        env.fixture
            .fs
            .fail("create_new", ".nurt-tmp-1-2", FsErrorKind::PermissionDenied);
        let error = tx
            .apply(TargetIndex(2), Mutation::WriteBytes(b"B".to_vec()))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Permission);
        assert_eq!(env.read(&a), b"a");
        assert_eq!(env.read(&gone), b"gone");
        assert_eq!(env.read(&b), b"b");
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::RolledBack)
        );
    }

    #[test]
    fn dropping_an_uncommitted_transaction_rolls_it_back() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        drop(tx);
        assert_eq!(env.read(&a), b"a");
        assert_eq!(env.state(0, 1).state, EntryState::Undone);
        assert!(env.pending(0).is_empty());
    }

    #[test]
    fn incomplete_rollback_leaves_the_entry_rolling_back() {
        let env = Env::new();
        let gone = env.file("vol-a/gone.txt", b"gone");
        let b = env.file("vol-a/b.txt", b"b");
        let mut tx = begin(&env.ctx, request("remove"));
        tx.preserve(&[
            spec(&gone, TargetAction::Delete),
            spec(&b, TargetAction::Modify),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        env.fixture
            .fs
            .fail_always("rename", "0000000000000000.blob", FsErrorKind::Io);
        env.fixture
            .fs
            .fail("create_new", ".nurt-tmp-1-1", FsErrorKind::Io);
        let error = tx
            .apply(TargetIndex(1), Mutation::WriteBytes(b"B".to_vec()))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::RollbackFailed);
        assert_eq!(
            env.state(0, 1),
            StateRecord::rolling_back(UndoneCause::RolledBack)
        );
        assert_eq!(env.pending(0), vec![OpId(1)]);
        assert_eq!(env.read(&env.blob(0, 1, 0, BlobRole::Previous)), b"gone");
    }

    #[test]
    fn protocol_misuse_fails_and_rolls_back() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        let error = tx.preserve(&[spec(&a, TargetAction::Delete)]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::StageFailed);
        assert_eq!(env.read(&a), b"a");
        assert_eq!(
            tx.stage(TargetIndex(0)).err().map(|e| e.code()),
            Some(ErrorCode::RolledBack)
        );
        for mutation in [Mutation::Remove, Mutation::AppendBytes(Vec::new())] {
            let mut tx = begin(&env.ctx, request("write"));
            tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
            assert_eq!(
                tx.apply(TargetIndex(0), mutation).unwrap_err().code(),
                ErrorCode::StageFailed
            );
        }
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        let error = tx.apply(TargetIndex(9), Mutation::Remove).unwrap_err();
        assert_eq!(error.code(), ErrorCode::StageFailed);
        let missing = env.fixture.fs.path("vol-a/missing");
        let mut tx = begin(&env.ctx, request("remove"));
        let error = tx
            .preserve(&[spec(&missing, TargetAction::Delete)])
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let mut tx = begin(&env.ctx, request("write"));
        let error = tx.preserve(&[spec(&a, TargetAction::Create)]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
    }

    #[test]
    fn abandoned_transaction_before_preserve_consumes_no_number() {
        let env = Env::new();
        let tx = begin(&env.ctx, request("write"));
        assert_eq!(tx.op(), None);
        drop(tx);
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        assert_eq!(tx.op(), Some(OpId(1)));
    }

    #[test]
    fn number_named_in_a_shared_journal_is_never_assigned() {
        let env = Env::new();
        let layout = env.layout(0);
        let used = [
            layout.entry(OpId(1)).unwrap(),
            layout.entry_tmp(OpId(2)).unwrap(),
        ];
        for entry in &used {
            std::fs::create_dir_all(entry.dir().as_path()).unwrap();
        }
        env.fixture
            .write(&env.ctx.state.seq().unwrap(), &seq::encode(OpId(0)));
        let path = env.file("vol-a/a.txt", b"old");
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.op, Some(OpId(3)));
        assert_eq!(handle.targets[0].mode, PreserveMode::Exchange);
        assert!(handle.irreversible.is_empty(), "{handle:?}");
        assert_eq!(env.read(&env.blob(0, 3, 0, BlobRole::Previous)), b"old");
        assert!(used.iter().all(|entry| env.fixture.exists(entry.dir())));
    }

    /// Journals that become known to the account on the first `locate`, as
    /// with the volume registry of a new account.
    struct FirstUse {
        inner: Arc<crate::store::test_support::TestJournals>,
        known: std::sync::Mutex<Vec<AbsPath>>,
    }

    impl crate::store::VolumeJournals for FirstUse {
        fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
            let location = self.inner.locate(volume);
            if let JournalLocation::Active(dir) = &location {
                let mut known = self.known.lock().unwrap();
                if !known.contains(dir) {
                    known.push(dir.clone());
                    known.sort();
                }
            }
            location
        }

        fn active(&self) -> Vec<AbsPath> {
            self.known.lock().unwrap().clone()
        }
    }

    #[test]
    fn number_used_by_another_account_on_a_volume_met_first_is_replaced() {
        let mut env = Env::new();
        env.ctx.journals = Arc::new(FirstUse {
            inner: env.fixture.journals.clone(),
            known: std::sync::Mutex::new(Vec::new()),
        });
        let used = env.layout(0).entry(OpId(1)).unwrap();
        std::fs::create_dir_all(used.dir().as_path()).unwrap();
        let path = env.file("vol-a/a.txt", b"old");
        let handle = env.overwrite(&path, b"new");
        assert_eq!(handle.op, Some(OpId(2)));
        assert_eq!(handle.targets[0].mode, PreserveMode::Exchange);
        assert!(handle.irreversible.is_empty(), "{handle:?}");
        assert_eq!(env.read(&env.blob(0, 2, 0, BlobRole::Previous)), b"old");
        assert!(env.fixture.exists(used.dir()));
    }

    #[test]
    fn failed_preparation_removes_its_temporary_entry_directory() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"old");
        env.fixture
            .fs
            .fail("create_new", "op.json", FsErrorKind::NoSpace);
        let handle = env.overwrite(&path, b"new");
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::JournalError)
        );
        assert_eq!(env.read(&path), b"new");
        let tmp = env.layout(0).entry_tmp(OpId(1)).unwrap();
        assert!(!env.fixture.exists(tmp.dir()));
        assert!(env.pending(0).is_empty());
    }

    #[test]
    fn overwrite_keeps_the_mode_and_gives_the_new_content_the_time_of_the_write() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"old");
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        std::fs::File::options()
            .write(true)
            .open(path.as_path())
            .unwrap()
            .set_modified(old)
            .unwrap();
        // Windows keeps of the permission bits only the read-only flag.
        let mode = if cfg!(windows) { 0o444 } else { 0o600 };
        crate::store::test_support::set_path_mode(path.as_path(), mode).unwrap();
        env.overwrite(&path, b"new");
        let written = env.fixture.fs.metadata(&path, Follow::NoFollow).unwrap();
        assert_eq!(env.read(&path), b"new");
        assert_eq!(written.mode & 0o777, mode);
        assert!(written.modified_ns > 1_000_000_000_000, "{written:?}");
    }

    #[test]
    fn a_copy_takes_the_times_of_its_source() {
        let env = Env::new();
        let source = env.file("vol-a/source.txt", b"copied");
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        std::fs::File::options()
            .write(true)
            .open(source.as_path())
            .unwrap()
            .set_modified(old)
            .unwrap();
        let target = env.fixture.fs.path("vol-a/copy.txt");
        let mut tx = begin(&env.ctx, request("copy"));
        tx.preserve(&[spec(&target, TargetAction::Create)]).unwrap();
        tx.apply(
            TargetIndex(0),
            Mutation::CopyFrom {
                source,
                clone: CloneMode::Never,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        let copied = env.fixture.fs.metadata(&target, Follow::NoFollow).unwrap();
        assert_eq!(env.read(&target), b"copied");
        assert_eq!(copied.modified_ns, 1_000_000_000_000);
    }

    #[test]
    fn write_at_and_set_length_change_the_content_in_place_of_the_range() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"abcdef");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&path, TargetAction::Modify)]).unwrap();
        tx.apply(
            TargetIndex(0),
            Mutation::WriteAt {
                offset: 2,
                bytes: b"XY".to_vec(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(env.read(&path), b"abXYef");
        let grown = env.file("vol-a/b.txt", b"abc");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&grown, TargetAction::Truncate)])
            .unwrap();
        tx.apply(TargetIndex(0), Mutation::SetLength(5)).unwrap();
        tx.commit().unwrap();
        assert_eq!(env.read(&grown), b"abc\0\0");
    }

    #[test]
    fn append_to_a_missing_file_creates_it_and_rollback_removes_it() {
        let env = Env::new();
        let path = env.fixture.fs.path("vol-a/new.log");
        let mut tx = begin(&env.ctx, request("write"));
        let preserved = tx.preserve(&[spec(&path, TargetAction::Append)]).unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Created);
        tx.apply(TargetIndex(0), Mutation::AppendBytes(b"x".to_vec()))
            .unwrap();
        assert_eq!(env.read(&path), b"x");
        tx.rollback(RollbackReason::Cancelled).unwrap();
        assert!(!env.fixture.exists(&path));
    }

    #[test]
    fn replacing_a_file_by_a_directory_is_rolled_back_to_the_file() {
        let env = Env::new();
        let path = env.file("vol-a/x", b"file");
        let mut tx = begin(&env.ctx, request("replace"));
        let preserved = tx.preserve(&[spec(&path, TargetAction::Replace)]).unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Moved);
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        tx.apply(TargetIndex(0), Mutation::CreateDir { mode: 0o755 })
            .unwrap();
        assert!(std::fs::metadata(path.as_path()).unwrap().is_dir());
        tx.rollback(RollbackReason::Cancelled).unwrap();
        assert_eq!(env.read(&path), b"file");
    }

    #[test]
    fn expected_state_that_differs_fails_the_declaration_as_stale() {
        let env = Env::new();
        let path = env.file("vol-a/a.txt", b"a");
        let mut wrong_hash = spec(&path, TargetAction::Modify);
        wrong_hash.expect = Expect::Hash(manifest::hash_bytes(b"other"));
        let mut tx = begin(&env.ctx, request("write"));
        assert_eq!(
            tx.preserve(&[wrong_hash]).unwrap_err().code(),
            ErrorCode::Stale
        );
        let mut not_absent = spec(&path, TargetAction::Modify);
        not_absent.expect = Expect::Absent;
        let mut tx = begin(&env.ctx, request("write"));
        assert_eq!(
            tx.preserve(&[not_absent]).unwrap_err().code(),
            ErrorCode::Stale
        );
        let mut right_hash = spec(&path, TargetAction::Modify);
        right_hash.expect = Expect::Hash(manifest::hash_bytes(b"a"));
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[right_hash]).unwrap();
        assert_eq!(env.read(&path), b"a");
    }

    #[test]
    fn provider_value_removal_is_restored_by_rollback() {
        let env = Env::new();
        let key = ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["HKCU".to_owned(), "Software".to_owned()],
            value: Some("Theme".to_owned()),
        };
        env.fixture
            .registry
            .values
            .lock()
            .unwrap()
            .insert(key.clone(), ProviderValue::Text("dark".to_owned()));
        let target = TargetSpec {
            path: TargetPath::Provider(key.clone()),
            action: TargetAction::Delete,
            expect: Expect::Unchecked,
            size_hint: None,
        };
        let mut tx = begin(&env.ctx, request("registry.remove"));
        let preserved = tx.preserve(&[target]).unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Value);
        tx.apply(TargetIndex(0), Mutation::ProviderRemove).unwrap();
        assert!(env.fixture.registry.values.lock().unwrap().is_empty());
        tx.rollback(RollbackReason::Cancelled).unwrap();
        assert_eq!(
            env.fixture.registry.values.lock().unwrap().get(&key),
            Some(&ProviderValue::Text("dark".to_owned()))
        );
    }

    fn theme_key(name: &str) -> ProviderKey {
        ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["HKCU".to_owned(), "Software".to_owned()],
            value: Some(name.to_owned()),
        }
    }

    fn provider_target(key: ProviderKey, action: TargetAction, expect: Expect) -> TargetSpec {
        TargetSpec {
            path: TargetPath::Provider(key),
            action,
            expect,
            size_hint: None,
        }
    }

    #[test]
    fn provider_copy_creates_the_destination_and_rollback_removes_it() {
        let env = Env::new();
        let dark = ProviderValue::Text("dark".to_owned());
        env.fixture
            .registry
            .values
            .lock()
            .unwrap()
            .insert(theme_key("Theme"), dark.clone());
        let target = provider_target(theme_key("Copy"), TargetAction::Create, Expect::Absent);
        let mut tx = begin(&env.ctx, request("registry.copy"));
        tx.preserve(&[target]).unwrap();
        tx.apply(TargetIndex(0), Mutation::ProviderCopy(theme_key("Theme")))
            .unwrap();
        assert_eq!(
            env.fixture
                .registry
                .values
                .lock()
                .unwrap()
                .get(&theme_key("Copy")),
            Some(&dark)
        );
        tx.rollback(RollbackReason::Cancelled).unwrap();
        let values = env.fixture.registry.values.lock().unwrap();
        assert_eq!(values.get(&theme_key("Copy")), None);
        assert_eq!(values.get(&theme_key("Theme")), Some(&dark));
    }

    #[test]
    fn provider_copy_is_accepted_only_for_a_created_target() {
        let env = Env::new();
        env.fixture
            .registry
            .values
            .lock()
            .unwrap()
            .insert(theme_key("Theme"), ProviderValue::Text("dark".to_owned()));
        let target = provider_target(theme_key("Copy"), TargetAction::Modify, Expect::Unchecked);
        let mut tx = begin(&env.ctx, request("registry.copy"));
        tx.preserve(&[target]).unwrap();
        let error = tx
            .apply(TargetIndex(0), Mutation::ProviderCopy(theme_key("Theme")))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::StageFailed);
    }

    #[test]
    fn an_existing_provider_target_cannot_be_declared_as_created() {
        let env = Env::new();
        env.fixture
            .registry
            .values
            .lock()
            .unwrap()
            .insert(theme_key("Copy"), ProviderValue::Text("old".to_owned()));
        let target = provider_target(theme_key("Copy"), TargetAction::Create, Expect::Absent);
        let mut tx = begin(&env.ctx, request("registry.copy"));
        let error = tx.preserve(&[target]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
    }

    #[test]
    fn a_missing_provider_target_cannot_be_declared_as_deleted() {
        let env = Env::new();
        let target = provider_target(theme_key("Gone"), TargetAction::Delete, Expect::Unchecked);
        let mut tx = begin(&env.ctx, request("registry.remove"));
        let error = tx.preserve(&[target]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn provider_restore_hands_the_snapshot_to_the_provider() {
        let env = Env::new();
        let old = ProviderValue::Text("old".to_owned());
        env.fixture
            .registry
            .values
            .lock()
            .unwrap()
            .insert(theme_key("Theme"), ProviderValue::Text("new".to_owned()));
        let snapshot = ProviderSnapshot {
            key: theme_key("Theme"),
            existed: true,
            value: Some(old.clone()),
            subtree: Some(b"subtree".to_vec()),
        };
        let target = provider_target(theme_key("Theme"), TargetAction::Modify, Expect::Unchecked);
        let mut tx = begin(&env.ctx, request("registry.undo"));
        tx.preserve(&[target]).unwrap();
        tx.apply(TargetIndex(0), Mutation::ProviderRestore(snapshot.clone()))
            .unwrap();
        assert_eq!(
            env.fixture.registry.restored.lock().unwrap().first(),
            Some(&snapshot)
        );
        assert_eq!(
            env.fixture
                .registry
                .values
                .lock()
                .unwrap()
                .get(&theme_key("Theme")),
            Some(&old)
        );
    }

    #[test]
    fn file_mutation_on_a_provider_target_is_a_protocol_violation() {
        let env = Env::new();
        let key = ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["HKCU".to_owned()],
            value: Some("Theme".to_owned()),
        };
        let target = TargetSpec {
            path: TargetPath::Provider(key),
            action: TargetAction::Modify,
            expect: Expect::Unchecked,
            size_hint: None,
        };
        let mut tx = begin(&env.ctx, request("registry.set"));
        tx.preserve(&[target]).unwrap();
        let error = tx
            .apply(TargetIndex(0), Mutation::WriteBytes(Vec::new()))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::StageFailed);
    }

    #[test]
    fn symbolic_link_is_created_and_recorded() {
        let env = Env::new();
        let link = env.fixture.fs.path("vol-a/link");
        let mut tx = begin(&env.ctx, request("link"));
        tx.preserve(&[spec(&link, TargetAction::Link)]).unwrap();
        tx.apply(
            TargetIndex(0),
            Mutation::CreateLink {
                target: std::path::PathBuf::from("a.txt"),
                kind: nurt_contracts::fs::LinkKind::SymlinkFile,
            },
        )
        .unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(handle.targets[0].mode, PreserveMode::Created);
        assert_eq!(
            std::fs::read_link(link.as_path()).unwrap(),
            std::path::PathBuf::from("a.txt")
        );
    }

    #[test]
    fn staged_content_is_switched_in_by_commit_staged() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        let mut handle = tx.stage(TargetIndex(0)).unwrap();
        std::io::Write::write_all(&mut handle, b"streamed").unwrap();
        drop(handle);
        let applied = tx.apply(TargetIndex(0), Mutation::CommitStaged).unwrap();
        assert_eq!(applied.after.hash, Some(manifest::hash_bytes(b"streamed")));
        let source = env.file("vol-a/source.txt", b"copied");
        let target = env.fixture.fs.path("vol-a/copy.txt");
        tx.commit().unwrap();
        assert_eq!(env.read(&a), b"streamed");
        let mut tx = begin(&env.ctx, request("copy"));
        tx.preserve(&[spec(&target, TargetAction::Create)]).unwrap();
        let applied = tx
            .apply(
                TargetIndex(0),
                Mutation::CopyFrom {
                    source,
                    clone: CloneMode::Auto,
                },
            )
            .unwrap();
        assert_eq!(applied.after.hash, Some(manifest::hash_bytes(b"copied")));
        tx.apply(
            TargetIndex(0),
            Mutation::WriteAt {
                offset: 0,
                bytes: Vec::new(),
            },
        )
        .unwrap_err();
    }

    #[test]
    fn protocol_steps_happen_in_order() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        env.fixture.fs.clear_log();
        env.overwrite(&a, b"A");
        let log = env.fixture.fs.log();
        let steps = [
            position(&log, |e| e.starts_with("rename ") && e.ends_with("/seq")),
            position(&log, |e| {
                e.starts_with("create_new ") && e.ends_with("pending/000000000001")
            }),
            position(&log, |e| {
                e.starts_with("create_new ") && e.ends_with(".tmp/op.json")
            }),
            position(&log, |e| {
                e.starts_with("sync_file ") && e.ends_with(".tmp/op.json")
            }),
            position(&log, |e| {
                e.starts_with("rename ") && e.ends_with("ops/000000000001")
            }),
            position(&log, |e| e.starts_with("sync_dir ") && e.ends_with("/ops")),
            position(&log, |e| e.starts_with("exchange ")),
            position(&log, |e| e.starts_with("rename ") && e.ends_with("/STATE")),
            position(&log, |e| e.ends_with("ops.jsonl")),
            position(&log, |e| {
                e.starts_with("remove_file ") && e.ends_with("pending/000000000001")
            }),
        ];
        assert!(
            steps.windows(2).all(|pair| pair[0] < pair[1]),
            "{steps:?} {log:#?}"
        );
    }

    #[test]
    fn no_target_changes_outside_a_prepared_entry() {
        let env = Env::audited();
        let a = env.file("vol-a/a.txt", b"a");
        let gone = env.file("vol-a/gone.txt", b"gone");
        let created = env.fixture.fs.path("vol-a/new.txt");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[
            spec(&a, TargetAction::Modify),
            spec(&gone, TargetAction::Delete),
            spec(&created, TargetAction::Create),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(1), Mutation::Remove).unwrap();
        tx.apply(TargetIndex(2), Mutation::WriteBytes(b"n".to_vec()))
            .unwrap();
        tx.commit().unwrap();
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"B".to_vec()))
            .unwrap();
        tx.rollback(RollbackReason::TargetFailed).unwrap();
        assert_eq!(env.read(&a), b"A");
    }

    #[test]
    fn inactive_volumes_map_to_irreversible_reasons() {
        let table = [
            (InactiveReason::ServiceDown, Irreversible::JournalInactive),
            (InactiveReason::NetworkVolume, Irreversible::NetworkVolume),
            (
                InactiveReason::RemovableVolume,
                Irreversible::RemovableVolume,
            ),
            (InactiveReason::MemoryVolume, Irreversible::JournalInactive),
            (InactiveReason::BelowReserve, Irreversible::NoSpaceForCopy),
            (InactiveReason::NoAnchor, Irreversible::JournalInactive),
            (InactiveReason::Disabled, Irreversible::JournalInactive),
            (InactiveReason::NotDurable, Irreversible::JournalInactive),
        ];
        for (reason, expected) in table {
            assert_eq!(reason.irreversible(), expected, "{reason:?}");
            let env = Env::new();
            *env.fixture.admission.eligibility.lock().unwrap() = Eligibility::Inactive(reason);
            let a = env.file("vol-a/a.txt", b"a");
            let handle = env.overwrite(&a, b"A");
            assert_eq!(handle.targets[0].irreversible, Some(expected));
            assert_eq!(env.read(&a), b"A");
        }
        let env = Env::new();
        *env.fixture.admission.liveness.lock().unwrap() = ServiceLiveness::Down;
        let a = env.file("vol-a/a.txt", b"a");
        assert_eq!(
            env.overwrite(&a, b"A").targets[0].irreversible,
            Some(Irreversible::JournalInactive)
        );
        for reason in [
            Irreversible::NoSpaceForCopy,
            Irreversible::JournalInactive,
            Irreversible::NetworkVolume,
        ] {
            let env = Env::new();
            *env.fixture.admission.decision.lock().unwrap() = AdmissionDecision::Skip { reason };
            let a = env.file("vol-a/a.txt", b"a");
            assert_eq!(
                env.overwrite(&a, b"A").targets[0].irreversible,
                Some(reason)
            );
        }
    }

    #[test]
    fn oversized_operation_is_not_preserved_and_reports_the_limit() {
        let env = Env::new();
        env.fixture.limits.0.lock().unwrap().max_op_size.value =
            ByteLimit::Bytes(SizeSetting::from_bytes(1));
        *env.fixture.admission.decision.lock().unwrap() = AdmissionDecision::Skip {
            reason: Irreversible::TooLargeForJournal,
        };
        let a = env.file("vol-a/a.txt", b"abc");
        let handle = env.overwrite(&a, b"A");
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::TooLargeForJournal)
        );
        assert_eq!(
            handle.notable,
            vec![JournalNotable::JournalOversize { size: 3, limit: 1 }]
        );
        let request = &env.fixture.admission.requests.lock().unwrap()[0];
        assert_eq!(request.op_size_limit, Some(1));
        assert_eq!(request.extra, 3);
    }

    #[test]
    fn journal_space_and_lock_failures_never_fail_the_operation() {
        let env = Env::new();
        env.fixture
            .fs
            .fail_always("create_new", "pending", FsErrorKind::NoSpace);
        let a = env.file("vol-a/a.txt", b"a");
        let handle = env.overwrite(&a, b"A");
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::JournalError)
        );
        assert_eq!(env.read(&a), b"A");
        env.fixture.fs.clear_faults();

        let env = Env::new();
        let held = env.ctx.lock_account().unwrap();
        let a = env.file("vol-a/a.txt", b"a");
        let handle = env.overwrite(&a, b"A");
        drop(held);
        assert_eq!(handle.op, None);
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::JournalError)
        );
        assert_eq!(env.read(&a), b"A");

        let env = Env::new();
        env.ctx
            .startup
            .set(Err(Error::new(ErrorCode::Io, "recovery failed")))
            .unwrap();
        let a = env.file("vol-a/a.txt", b"a");
        let handle = env.overwrite(&a, b"A");
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::JournalError)
        );

        let env = Env::new();
        let version = env.layout(0).version().unwrap();
        env.fixture.write(&version, b"nurt-journal 9\n");
        let a = env.file("vol-a/a.txt", b"a");
        assert_eq!(
            env.overwrite(&a, b"A").targets[0].irreversible,
            Some(Irreversible::JournalError)
        );
    }

    #[test]
    fn results_carry_no_expiry() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("write"));
        let preserved = tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        let handle = tx.commit().unwrap();
        let text = format!(
            "{}{}",
            serde_json::to_string(&handle).unwrap(),
            serde_json::to_string(&preserved).unwrap()
        );
        for word in ["expires", "deadline", "after_seconds", "after_ops"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
        assert!(env.manifest(0, 1).expires.is_some());
    }

    #[test]
    fn marker_exists_from_before_the_entry_until_after_the_terminal_state() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        assert_eq!(env.pending(0), vec![OpId(1)]);
        tx.commit().unwrap();
        assert!(env.pending(0).is_empty());
        let log = env.fixture.fs.log();
        let marker = position(&log, |e| {
            e.starts_with("create_new ") && e.contains("pending/")
        });
        let entry = position(&log, |e| {
            e.starts_with("create_dir ") && e.ends_with("ops/000000000001.tmp")
        });
        assert!(marker < entry);
        let removed = position(&log, |e| {
            e.starts_with("remove_file ") && e.contains("pending/")
        });
        let indexed = position(&log, |e| e.ends_with("ops.jsonl"));
        assert!(indexed < removed);
    }

    fn scenario(env: &Env) -> std::result::Result<OpHandle, Error> {
        let a = env.fixture.fs.path("vol-a/a.txt");
        let gone = env.fixture.fs.path("vol-a/gone.txt");
        env.fixture.write(&a, b"old");
        env.fixture.write(&gone, b"gone");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[
            spec(&a, TargetAction::Modify),
            spec(&gone, TargetAction::Delete),
        ])?;
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))?;
        tx.apply(TargetIndex(1), Mutation::Remove)?;
        tx.commit()
    }

    const PROTOCOL_POINTS: [FailPoint; 22] = [
        FailPoint::SeqBefore,
        FailPoint::SeqWritten,
        FailPoint::PlanDecided,
        FailPoint::ManifestWritten,
        FailPoint::ManifestSynced,
        FailPoint::PreparedRenamed,
        FailPoint::PreparedDurable,
        FailPoint::PreserveBefore,
        FailPoint::PreserveMoved,
        FailPoint::PreserveDurable,
        FailPoint::StageWritten,
        FailPoint::StageSynced,
        FailPoint::ApplySwitched,
        FailPoint::ApplyDone,
        FailPoint::CommitWritten,
        FailPoint::CommitRenamed,
        FailPoint::CommitDurable,
        FailPoint::RollbackStarted,
        FailPoint::RollbackRestored,
        FailPoint::SeqBefore,
        FailPoint::SeqBefore,
        FailPoint::SeqBefore,
    ];

    #[test]
    fn every_injected_error_ends_rolled_back_or_irreversible() {
        let kinds = [
            FsErrorKind::Io,
            FsErrorKind::PermissionDenied,
            FsErrorKind::CrossDevice,
            FsErrorKind::InUse,
            FsErrorKind::Stale,
            FsErrorKind::Unsupported,
            FsErrorKind::NoSpace,
        ];
        for point in &PROTOCOL_POINTS[..19] {
            for kind in kinds {
                let mut env = Env::new();
                env.ctx.settings.overwrite = OverwriteStrategy::CloneOrCopy;
                env.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(
                    0,
                    FailRule {
                        point: *point,
                        n: None,
                        occurrence: 0,
                        action: FailAction::Error(kind),
                    },
                )));
                let a = env.fixture.fs.path("vol-a/a.txt");
                let gone = env.fixture.fs.path("vol-a/gone.txt");
                match scenario(&env) {
                    Ok(handle) => {
                        assert_eq!(env.read(&a), b"new", "{point:?} {kind:?}");
                        assert!(!env.fixture.exists(&gone));
                        for target in &handle.targets {
                            assert!(
                                target.mode.is_preserved() || target.irreversible.is_some(),
                                "silent loss at {point:?} {kind:?}"
                            );
                        }
                    }
                    Err(error) => {
                        assert_ne!(error.code(), ErrorCode::Internal);
                        assert_eq!(env.read(&a), b"old", "{point:?} {kind:?}");
                        assert_eq!(env.read(&gone), b"gone");
                        assert_ne!(
                            (error.code(), *point),
                            (ErrorCode::NoSpace, FailPoint::ManifestWritten)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn injected_errors_during_rollback_still_restore_targets() {
        let mut env = Env::new();
        let rules = [
            FailPoint::StageWritten,
            FailPoint::RollbackStarted,
            FailPoint::RollbackRestored,
        ];
        env.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan {
            seed: 0,
            rules: rules
                .iter()
                .map(|point| FailRule {
                    point: *point,
                    n: None,
                    occurrence: 0,
                    action: FailAction::Error(FsErrorKind::Io),
                })
                .collect(),
            random: nurt_fs::failpoint::RandomFaults::default(),
        }));
        let a = env.fixture.fs.path("vol-a/a.txt");
        assert!(scenario(&env).is_err());
        assert_eq!(env.read(&a), b"old");
    }

    #[test]
    fn injected_crash_stops_without_cleanup() {
        let mut env = Env::new();
        env.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(
            0,
            FailRule {
                point: FailPoint::StageSynced,
                n: None,
                occurrence: 0,
                action: FailAction::Crash,
            },
        )));
        let error = scenario(&env).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(env.state(0, 1).state, EntryState::Prepared);
        assert_eq!(env.pending(0), vec![OpId(1)]);
        assert!(
            env.fixture
                .exists(&env.fixture.fs.path("vol-a/.nurt-tmp-1-0"))
        );
    }

    #[test]
    fn every_protocol_point_is_reached() {
        let mut hit = BTreeSet::new();
        for stage_error in [false, true] {
            let mut env = Env::new();
            env.fixture
                .fs
                .configure(0, |volume| volume.features.exchange = !stage_error);
            env.ctx.settings.overwrite = if stage_error {
                OverwriteStrategy::CloneOrCopy
            } else {
                OverwriteStrategy::Exchange
            };
            let plan = if stage_error {
                FailPlan::at(
                    0,
                    FailRule {
                        point: FailPoint::ApplySwitched,
                        n: Some(1),
                        occurrence: 0,
                        action: FailAction::Error(FsErrorKind::Io),
                    },
                )
            } else {
                FailPlan::observe(0)
            };
            let failpoints = Arc::new(SeededFailpoints::new(plan));
            env.ctx.failpoints = failpoints.clone();
            let _ = scenario(&env);
            hit.extend(failpoints.record().hits.iter().map(|record| record.point));
        }
        for point in &PROTOCOL_POINTS[..19] {
            assert!(hit.contains(point), "{point:?} not reached");
        }
    }

    #[test]
    fn linked_entry_records_its_sources_and_runs_the_update_before_releasing() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"a");
        let updated = Arc::new(std::sync::Mutex::new(None));
        let seen = updated.clone();
        let link = Link {
            undo_of: vec![OpId(7)],
            redo_of: None,
            scope: Some(UndoScope::Op),
            items: vec![ItemRef {
                op: OpId(7),
                index: TargetIndex(0),
                path: TargetPath::Fs(a.clone()),
            }],
            recorded: Vec::new(),
            after_commit: Some(Box::new(move |ctx: &JournalCtx, op| {
                let layout = JournalLayout::new(ctx.journals.active()[0].clone());
                let pending = layout::pending_entries(ctx.fs.as_ref(), &layout).unwrap();
                *seen.lock().unwrap() = Some((op, pending));
                Ok(())
            })),
        };
        let mut tx = begin_linked(&env.ctx, request("undo"), link);
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"b".to_vec()))
            .unwrap();
        Box::new(tx).commit().unwrap();
        assert_eq!(*updated.lock().unwrap(), Some((OpId(1), vec![OpId(1)])));
        let manifest = env.manifest(0, 1);
        assert_eq!(manifest.undo_of, vec![OpId(7)]);
        assert_eq!(manifest.scope, Some(UndoScope::Op));
        assert!(env.pending(0).is_empty());
        let debug = format!(
            "{:?}",
            begin_linked(&env.ctx, request("redo"), Link::default())
        );
        assert!(debug.starts_with("Tx"));
    }

    #[test]
    fn journal_store_begins_transactions() {
        use nurt_contracts::journal::JournalStore as _;

        let fixture = Fixture::new();
        let journal = fixture.journal();
        let a = fixture.fs.path("vol-a/a.txt");
        fixture.write(&a, b"a");
        let mut tx = journal.begin(request("write"));
        tx.preserve(&[spec(&a, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        assert_eq!(tx.commit().unwrap().op, Some(OpId(1)));
        assert_eq!(
            journal.space(None).unwrap_err().code,
            ErrorCode::ServiceNotRunning
        );
    }

    /// Position of the last rename that replaces the state file of an entry.
    fn last_state_rename(log: &[String]) -> usize {
        log.iter()
            .rposition(|entry| entry.starts_with("rename ") && entry.ends_with("/STATE"))
            .unwrap_or_else(|| panic!("no state rename in {log:#?}"))
    }

    /// Asserts that every change logged before the last state rename is
    /// durable when that rename is made, and every change of the log at its
    /// end.
    fn assert_durable_before_state(label: &str, log: &[String]) {
        let early = unflushed(log, last_state_rename(log));
        assert!(early.is_empty(), "{label}: {early:#?}\n{log:#?}");
        let late = unflushed(log, log.len());
        assert!(late.is_empty(), "{label}: {late:#?}\n{log:#?}");
    }

    fn rename_over(from: &AbsPath, to: &AbsPath) -> TargetSpec {
        spec(
            from,
            TargetAction::Rename {
                to: to.clone(),
                replace: true,
            },
        )
    }

    #[test]
    fn every_change_is_durable_before_the_committed_state() {
        let overwrites = [
            ("exchange", OverwriteStrategy::Exchange, true, false),
            ("hard link", OverwriteStrategy::Exchange, false, false),
            ("clone", OverwriteStrategy::CloneOrCopy, true, true),
            ("copy", OverwriteStrategy::CloneOrCopy, true, false),
        ];
        for (label, strategy, exchange, reflink) in overwrites {
            let mut env = Env::new();
            env.ctx.settings.overwrite = strategy;
            env.fixture.fs.configure(0, |volume| {
                volume.features.exchange = exchange;
                volume.features.reflink = reflink;
            });
            let path = env.file("vol-a/a.txt", b"old");
            let handle = env.overwrite(&path, b"new");
            assert!(handle.irreversible.is_empty(), "{label}: {handle:?}");
            assert_durable_before_state(label, &env.fixture.fs.log());
        }
        let env = Env::new();
        let gone = env.file("vol-a/gone.txt", b"gone");
        let tree = env.fixture.fs.path("vol-a/tree");
        std::fs::create_dir_all(tree.as_path().join("sub")).unwrap();
        std::fs::write(tree.as_path().join("sub/x"), b"x").unwrap();
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        let made = env.fixture.fs.path("vol-a/made");
        let log = env.file("vol-a/log.txt", b"line\n");
        let meta = env.file("vol-a/meta.txt", b"m");
        let mut tx = begin(&env.ctx, request("bulk"));
        tx.preserve(&[
            spec(&gone, TargetAction::Delete),
            spec(&tree, TargetAction::Delete),
            rename_over(&from, &to),
            spec(&made, TargetAction::Create),
            spec(&log, TargetAction::Append),
            spec(&meta, TargetAction::Attributes),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        tx.apply(TargetIndex(1), Mutation::Remove).unwrap();
        tx.apply(TargetIndex(2), Mutation::Rename).unwrap();
        tx.apply(TargetIndex(3), Mutation::CreateDir { mode: 0o755 })
            .unwrap();
        tx.apply(TargetIndex(4), Mutation::AppendBytes(b"more\n".to_vec()))
            .unwrap();
        let change = MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        };
        tx.apply(TargetIndex(5), Mutation::SetMeta(change)).unwrap();
        let handle = tx.commit().unwrap();
        assert!(handle.irreversible.is_empty(), "{handle:?}");
        assert_durable_before_state("bulk", &env.fixture.fs.log());
    }

    #[test]
    fn overwrite_flushes_by_strategy() {
        for (strategy, expected) in [
            (OverwriteStrategy::Exchange, 19),
            (OverwriteStrategy::CloneOrCopy, 21),
        ] {
            let mut env = Env::new();
            env.ctx.settings.overwrite = strategy;
            let path = env.file("vol-a/a.txt", b"old");
            env.overwrite(&path, b"first");
            env.fixture.fs.clear_log();
            env.overwrite(&path, b"second");
            let log = env.fixture.fs.log();
            let flushes = log
                .iter()
                .filter(|entry| entry.starts_with("sync_"))
                .count();
            assert_eq!(flushes, expected, "{strategy:?}: {log:#?}");
        }
    }

    #[test]
    fn restored_targets_are_durable_before_the_rollback_ends() {
        let env = Env::new();
        let created = env.fixture.fs.path("vol-a/new.txt");
        let meta = env.file("vol-a/meta.txt", b"m");
        let gone = env.file("vol-a/gone.txt", b"gone");
        let changed = env.file("vol-a/a.txt", b"a");
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        let mut tx = begin(&env.ctx, request("mixed"));
        tx.preserve(&[
            spec(&created, TargetAction::Create),
            spec(&meta, TargetAction::Attributes),
            spec(&gone, TargetAction::Delete),
            spec(&changed, TargetAction::Modify),
            rename_over(&from, &to),
        ])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"c".to_vec()))
            .unwrap();
        let change = MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        };
        tx.apply(TargetIndex(1), Mutation::SetMeta(change)).unwrap();
        tx.apply(TargetIndex(2), Mutation::Remove).unwrap();
        tx.apply(TargetIndex(3), Mutation::WriteBytes(b"A".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(4), Mutation::Rename).unwrap();
        env.fixture.fs.clear_log();
        let report = tx.rollback(RollbackReason::Cancelled).unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
        assert!(!env.fixture.exists(&created));
        assert_eq!(env.read(&gone), b"gone");
        assert_eq!(env.read(&changed), b"a");
        assert_eq!(env.read(&from), b"from");
        assert_eq!(env.read(&to), b"to");
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::RolledBack)
        );
        assert_durable_before_state("rollback", &env.fixture.fs.log());
    }

    #[test]
    fn recovery_recreates_a_lost_lock_directory() {
        let env = Env::new();
        let layout = env.layout(0);
        let marker = layout.pending(OpId(1)).unwrap();
        std::fs::create_dir_all(marker.as_path().parent().unwrap()).unwrap();
        env.fixture.write(&marker, b"");
        assert!(!env.fixture.exists(&layout.lock_dir().unwrap()));
        let report = crate::recovery::recover(&env.ctx).unwrap();
        assert!(report.rolled_back.is_empty(), "{report:?}");
        assert!(env.pending(0).is_empty());
        assert!(env.fixture.exists(&layout.lock_dir().unwrap()));
    }

    #[test]
    fn recovery_after_a_crash_before_commit_is_durable_and_idempotent() {
        let mut env = Env::new();
        env.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(
            0,
            FailRule {
                point: FailPoint::ApplyDone,
                n: None,
                occurrence: 0,
                action: FailAction::Crash,
            },
        )));
        let path = env.file("vol-a/a.txt", b"old");
        let mut tx = begin(&env.ctx, request("write"));
        tx.preserve(&[spec(&path, TargetAction::Modify)]).unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
            .unwrap();
        let error = tx.commit().unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(error.problem.as_deref(), Some("injected_crash"));
        env.ctx.failpoints = Arc::new(NoFailpoints);
        env.fixture.fs.clear_log();
        let report = crate::recovery::recover(&env.ctx).unwrap();
        assert_eq!(report.rolled_back, vec![OpId(1)]);
        assert_eq!(env.read(&path), b"old");
        let log = env.fixture.fs.log();
        let lacking = unflushed(&log, log.len());
        assert!(lacking.is_empty(), "{lacking:#?}\n{log:#?}");
        env.fixture.fs.clear_log();
        let again = crate::recovery::recover(&env.ctx).unwrap();
        assert!(again.rolled_back.is_empty(), "{again:?}");
        assert_eq!(env.read(&path), b"old");
        let log = env.fixture.fs.log();
        assert!(
            log.iter()
                .all(|entry| !entry.starts_with("rename ") && !entry.starts_with("remove")),
            "{log:#?}"
        );
    }

    #[test]
    fn crash_after_a_rename_over_a_destination_restores_both_names() {
        let mut env = Env::new();
        env.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(
            0,
            FailRule {
                point: FailPoint::ApplySwitched,
                n: None,
                occurrence: 0,
                action: FailAction::Crash,
            },
        )));
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        let mut tx = begin(&env.ctx, request("move"));
        tx.preserve(&[rename_over(&from, &to)]).unwrap();
        let replaced = env.manifest(0, 1).targets[0].replaced.clone();
        assert!(replaced.is_some_and(|before| before.blob.is_some()));
        let error = tx.apply(TargetIndex(0), Mutation::Rename).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
        drop(tx);
        env.ctx.failpoints = Arc::new(NoFailpoints);
        let report = crate::recovery::recover(&env.ctx).unwrap();
        assert_eq!(report.rolled_back, vec![OpId(1)]);
        assert_eq!(env.read(&from), b"from");
        assert_eq!(env.read(&to), b"to");
    }

    #[test]
    fn lost_race_for_a_number_takes_a_new_number_instead_of_giving_up_the_journal() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"old");
        env.fixture
            .fs
            .fail("create_new", "pending", FsErrorKind::Exists);
        let handle = env.overwrite(&a, b"new");
        assert_eq!(handle.op, Some(OpId(2)));
        assert!(handle.irreversible.is_empty(), "{handle:?}");
        assert_eq!(handle.targets[0].mode, PreserveMode::Exchange);
        assert_eq!(env.read(&env.blob(0, 2, 0, BlobRole::Previous)), b"old");
        assert_eq!(env.read(&a), b"new");
    }

    #[test]
    fn persistent_conflict_over_the_marker_gives_up_after_a_bounded_number_of_attempts() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"old");
        env.fixture
            .fs
            .fail_always("create_new", "pending", FsErrorKind::Exists);
        let handle = env.overwrite(&a, b"new");
        assert_eq!(
            handle.targets[0].irreversible,
            Some(Irreversible::JournalError)
        );
        assert_eq!(env.read(&a), b"new");
    }

    #[test]
    fn two_state_directories_sharing_a_journal_never_share_a_number() {
        let env = Env::new();
        let other_state = env.fixture.fs.path("vol-a/state-other");
        std::fs::create_dir(other_state.as_path()).unwrap();
        let mut other = env.fixture.ctx();
        other.state = layout::StateLayout::new(other_state);
        let own = env.file("vol-a/own.txt", b"o");
        let foreign = env.file("vol-a/foreign.txt", b"f");
        let rounds = 12;
        let run = |ctx: &JournalCtx, path: &AbsPath| -> Vec<OpHandle> {
            (0..rounds)
                .map(|round| {
                    let mut tx = begin(ctx, request("write"));
                    tx.preserve(&[spec(path, TargetAction::Modify)]).unwrap();
                    tx.apply(
                        TargetIndex(0),
                        Mutation::WriteBytes(format!("{round}").into_bytes()),
                    )
                    .unwrap();
                    tx.commit().unwrap()
                })
                .collect()
        };
        let (mine, theirs) = std::thread::scope(|scope| {
            let first = scope.spawn(|| run(&env.ctx, &own));
            let second = scope.spawn(|| run(&other, &foreign));
            (first.join().unwrap(), second.join().unwrap())
        });
        let mut ops = Vec::new();
        for handle in mine.iter().chain(&theirs) {
            assert!(handle.irreversible.is_empty(), "{handle:?}");
            ops.push(handle.op.unwrap());
        }
        let total = ops.len();
        ops.sort();
        ops.dedup();
        assert_eq!(ops.len(), total);
    }

    #[test]
    fn failed_move_of_a_removed_file_into_the_journal_keeps_the_file() {
        let env = Env::new();
        let a = env.file("vol-a/a.txt", b"precious");
        env.fixture.fs.fail("rename", ".blob", FsErrorKind::Io);
        let mut tx = begin(&env.ctx, request("remove"));
        tx.preserve(&[spec(&a, TargetAction::Delete)]).unwrap();
        let error = tx.apply(TargetIndex(0), Mutation::Remove).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
        assert_eq!(env.read(&a), b"precious");
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::RolledBack)
        );
    }

    #[test]
    fn removal_the_journal_declined_up_front_proceeds_unpreserved() {
        let env = Env::new();
        *env.fixture.admission.eligibility.lock().unwrap() =
            Eligibility::Inactive(InactiveReason::NoAnchor);
        let a = env.file("vol-a/a.txt", b"a");
        let mut tx = begin(&env.ctx, request("remove"));
        let preserved = tx.preserve(&[spec(&a, TargetAction::Delete)]).unwrap();
        assert_eq!(
            preserved[0].irreversible,
            Some(Irreversible::JournalInactive)
        );
        tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
        tx.commit().unwrap();
        assert!(!env.fixture.exists(&a));
    }

    #[test]
    fn failed_move_of_the_destination_keeps_it_and_rolls_back() {
        let env = Env::new();
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        env.fixture.fs.fail("rename", ".blob", FsErrorKind::Io);
        let mut tx = begin(&env.ctx, request("move"));
        tx.preserve(&[rename_over(&from, &to)]).unwrap();
        let error = tx.apply(TargetIndex(0), Mutation::Rename).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
        assert_eq!(env.read(&from), b"from");
        assert_eq!(env.read(&to), b"to");
        assert_eq!(
            env.state(0, 1),
            StateRecord::undone(UndoneCause::RolledBack)
        );

        let env = Env::new();
        *env.fixture.admission.eligibility.lock().unwrap() =
            Eligibility::Inactive(InactiveReason::NoAnchor);
        let from = env.file("vol-a/from.txt", b"from");
        let to = env.file("vol-a/to.txt", b"to");
        let mut tx = begin(&env.ctx, request("move"));
        let preserved = tx.preserve(&[rename_over(&from, &to)]).unwrap();
        assert_eq!(
            preserved[0].irreversible,
            Some(Irreversible::JournalInactive)
        );
        tx.apply(TargetIndex(0), Mutation::Rename).unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(env.read(&to), b"from");
        let destination = to.to_string();
        assert!(
            handle
                .irreversible
                .iter()
                .any(|entry| entry.path.as_deref() == Some(destination.as_str())),
            "{handle:?}"
        );
    }
}
