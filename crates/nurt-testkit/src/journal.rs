// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pass-through and restoring journals and test admission.
//!
//! [`NoopJournal`] implements [`JournalStore`] and [`MutationTx`] with the
//! reference semantics of the pass-through journal in the journal
//! specification of the repository: mutations are applied directly to the
//! underlying [`FsWrite`] with a scope of purpose
//! [`ScopePurpose::Operation`], nothing is preserved, and every target is
//! reported with [`PreserveMode::None`] and the irreversible reason
//! [`Irreversible::JournalInactive`]. [`RecordingJournal`] behaves the same,
//! keeps the list of calls in order and a register of transactions
//! ([`TxRecord`]).
//!
//! [`RecordingJournal::keeping`] creates a recording journal that keeps an
//! in-memory copy of the previous state of every file system target. It
//! restores that state when the transaction is rolled back, when a call of
//! the transaction fails and when a committed operation is rolled back with
//! [`JournalStore::rollback_committed`]. Restoring chosen targets can be made
//! to fail ([`RecordingJournal::fail_rollback_of`]), which yields
//! `E_ROLLBACK_FAILED` with the targets not restored.
//!
//! | Situation | Pass-through | Keeping |
//! |---|---|---|
//! | mode of an existing target | [`PreserveMode::None`] | [`PreserveMode::Copy`] (also `meta`, `rename`, `append` by action) |
//! | mode of an absent target | [`PreserveMode::None`] | [`PreserveMode::Created`] |
//! | rollback | lists changed targets as failed | restores them in reverse order |
//! | failed call | rolls back, returns the error | restores; `E_ROLLBACK_FAILED` when incomplete |
//! | [`JournalStore::rollback_committed`] | `E_JOURNAL_NOOP` | restores the committed operation |
//!
//! [`TestAdmission`] implements [`Admission`] with fixed answers.
//!
//! Misuse of the transaction protocol (preserve after the first change, an
//! undeclared target, a mutation that does not match the declared action)
//! fails with `E_STAGE_FAILED` and rolls the transaction back, so that the
//! defect is visible in the result of the stage under test.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FileKind, FileMeta, Follow, FsError, FsErrorKind, FsResult,
    FsWrite, LinkKind, MetaChange, RenameMode, ScopePurpose, TxScope, VolumeId, WriteHandle,
};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, AdmissionRequest, AfterState, Applied, BeforeState, BeginRequest,
    CloneMode, Eligibility, ExemptWrite, Expect, ExternalChange, HistoryQuery, JournalState,
    JournalStore, LimitsOutcome, LimitsRequest, Mutation, MutationTx, OpHandle, OpId, OpKind,
    OperationDetails, OperationRecord, PinOutcome, PinRequest, PreserveMode, Preserved,
    PurgeOutcome, PurgeRequest, RecoveryReport, RedoRequest, RollbackReason, RollbackReport,
    ServiceLiveness, SweepReport, TargetAction, TargetIndex, TargetOutcome, TargetPath, TargetSpec,
    Thresholds, UndoOutcome, UndoRequest, UndoScope, VolumeSpace, VolumeState, VolumeStatus,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};

/// Parallelism passed to [`FsWrite::remove_tree`] by the journals of this
/// module.
const REMOVE_PARALLELISM: usize = 1;

/// Permission bits of a mode, including the set-id and sticky bits.
const PERMISSION_BITS: u32 = 0o7777;

/// Converts a file system error into the language error with the code given
/// by [`FsError::code_id`]; the kind and the path become error details.
///
/// ```
/// use nurt_contracts::error::ErrorCode;
/// use nurt_contracts::fs::{FsError, FsErrorKind};
/// use nurt_testkit::journal::fs_error_to_error;
///
/// let error = fs_error_to_error(&FsError::new(FsErrorKind::NotEmpty, "remove_dir", None));
/// assert_eq!(error.code(), ErrorCode::Io);
/// ```
#[must_use]
pub fn fs_error_to_error(error: &FsError) -> Error {
    let code = error.code_id().parse().unwrap_or(ErrorCode::Io);
    let mut out = Error::new(code, error.to_string())
        .with_detail("kind", serde_json::Value::from(error.kind.as_str()));
    if let Some(path) = &error.path {
        out = out.with_detail(
            "path",
            serde_json::Value::from(path.to_string_lossy().into_owned()),
        );
    }
    out
}

fn io_error_to_error(error: &io::Error, path: &AbsPath) -> Error {
    let kind = if error.kind() == io::ErrorKind::StorageFull {
        FsErrorKind::NoSpace
    } else {
        FsErrorKind::Io
    };
    fs_error_to_error(&FsError::new(kind, "write", Some(path.as_path())))
}

fn convert(error: FsError) -> Error {
    fs_error_to_error(&error)
}

/// One call made to a [`RecordingJournal`] or to one of its transactions.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum JournalCall {
    /// [`JournalStore::recover`].
    Recover,
    /// [`JournalStore::begin`].
    Begin {
        /// Kind of the operation.
        kind: OpKind,
    },
    /// [`MutationTx::preserve`].
    Preserve {
        /// Operation number after the call.
        op: Option<OpId>,
        /// Declared targets.
        targets: Vec<TargetPath>,
    },
    /// [`MutationTx::stage`].
    Stage {
        /// Operation number.
        op: Option<OpId>,
        /// Target.
        index: TargetIndex,
    },
    /// [`MutationTx::apply`].
    Apply {
        /// Operation number.
        op: Option<OpId>,
        /// Target.
        index: TargetIndex,
        /// Snake-case name of the mutation (`write_bytes`, `remove`, ...).
        mutation: &'static str,
        /// The call succeeded.
        ok: bool,
    },
    /// [`MutationTx::observe_external`].
    ObserveExternal {
        /// Operation number.
        op: Option<OpId>,
        /// Reported entries.
        paths: Vec<AbsPath>,
    },
    /// [`MutationTx::mark_irreversible`].
    MarkIrreversible {
        /// Operation number.
        op: Option<OpId>,
        /// Reason of the marker.
        reason: Irreversible,
    },
    /// [`MutationTx::record_exempt`].
    RecordExempt {
        /// Operation number.
        op: Option<OpId>,
    },
    /// [`MutationTx::commit`].
    Commit {
        /// Operation number of the handle.
        op: Option<OpId>,
    },
    /// [`MutationTx::rollback`], also when caused by a failed call.
    Rollback {
        /// Operation number.
        op: Option<OpId>,
        /// Reason; `None` for the rollback after a failed call.
        reason: Option<RollbackReason>,
    },
    /// Any other method of [`JournalStore`], by name (`history`, `undo`,
    /// `status`, ...).
    Store {
        /// Method name.
        method: &'static str,
    },
}

/// State of a transaction in the register of a journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TxState {
    /// Neither committed nor rolled back; also a transaction dropped
    /// without either.
    Open,
    /// Committed.
    Committed,
    /// Rolled back before commit (explicitly or after a failed call), or
    /// rolled back after commit by [`JournalStore::rollback_committed`].
    RolledBack,
}

/// One transaction of a journal, for assertions: what was preserved,
/// applied, committed and rolled back.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TxRecord {
    /// Kind of the operation.
    pub kind: OpKind,
    /// Operation number; `None` while none is assigned.
    pub op: Option<OpId>,
    /// Targets declared by [`MutationTx::preserve`] and by
    /// [`MutationTx::observe_external`] without an index, in declaration
    /// order.
    pub preserved: Vec<TargetPath>,
    /// Targets changed by a successful [`MutationTx::apply`] or reported by
    /// [`MutationTx::observe_external`], in order of their first change.
    pub applied: Vec<TargetPath>,
    /// State.
    pub state: TxState,
    /// Reason of the rollback; `None` for a rollback after a failed call and
    /// for transactions not rolled back.
    pub reason: Option<RollbackReason>,
    /// Targets restored by the rollback.
    pub restored: Vec<TargetPath>,
    /// Changed targets the rollback did not restore.
    pub not_restored: Vec<TargetPath>,
}

impl TxRecord {
    fn open(kind: OpKind) -> Self {
        Self {
            kind,
            op: None,
            preserved: Vec::new(),
            applied: Vec::new(),
            state: TxState::Open,
            reason: None,
            restored: Vec::new(),
            not_restored: Vec::new(),
        }
    }
}

/// Previous state of one entry, kept by a keeping journal.
#[derive(Debug, Clone)]
enum PreImage {
    Absent,
    File {
        content: Vec<u8>,
        meta: Box<ExtendedMeta>,
    },
    Directory {
        meta: Box<ExtendedMeta>,
        children: Vec<(OsString, PreImage)>,
    },
    Link {
        target: PathBuf,
        kind: LinkKind,
    },
    Special,
}

fn child_path(path: &AbsPath, name: &OsString) -> FsResult<AbsPath> {
    AbsPath::new(path.as_path().join(name))
}

impl PreImage {
    fn capture(fs: &dyn FsWrite, path: &AbsPath) -> FsResult<Self> {
        let meta = match fs.extended_metadata(path, Follow::NoFollow) {
            Ok(meta) => meta,
            Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Self::Absent),
            Err(error) => return Err(error),
        };
        match meta.basic.kind {
            FileKind::File => {
                let mut handle = fs.open_read(path)?;
                let mut content = Vec::new();
                handle
                    .read_to_end(&mut content)
                    .map_err(|_| FsError::new(FsErrorKind::Io, "read", Some(path.as_path())))?;
                Ok(Self::File {
                    content,
                    meta: Box::new(meta),
                })
            }
            FileKind::Directory => {
                let mut children = Vec::new();
                for entry in fs.read_dir(path)? {
                    let entry = entry?;
                    let child = Self::capture(fs, &child_path(path, &entry.name)?)?;
                    children.push((entry.name, child));
                }
                Ok(Self::Directory {
                    meta: Box::new(meta),
                    children,
                })
            }
            FileKind::Symlink | FileKind::Junction => {
                let target = fs.read_link(path)?;
                let kind = if meta.basic.kind == FileKind::Junction {
                    LinkKind::Junction
                } else if fs
                    .metadata(path, Follow::Follow)
                    .is_ok_and(|resolved| resolved.kind == FileKind::Directory)
                {
                    LinkKind::SymlinkDirectory
                } else {
                    LinkKind::SymlinkFile
                };
                Ok(Self::Link { target, kind })
            }
            _ => Ok(Self::Special),
        }
    }

    fn restorable(&self) -> bool {
        match self {
            Self::Special => false,
            Self::Directory { children, .. } => {
                children.iter().all(|(_, child)| child.restorable())
            }
            _ => true,
        }
    }

    fn bytes(&self) -> u64 {
        match self {
            Self::File { content, .. } => u64::try_from(content.len()).unwrap_or(u64::MAX),
            Self::Directory { children, .. } => children
                .iter()
                .map(|(_, child)| child.bytes())
                .fold(0, u64::saturating_add),
            _ => 0,
        }
    }

    fn restore(&self, fs: &dyn FsWrite, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        match fs.metadata(path, Follow::NoFollow) {
            Err(error) if error.kind == FsErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(meta) if meta.kind == FileKind::Directory => {
                fs.remove_tree(scope, path, REMOVE_PARALLELISM)?;
            }
            Ok(_) => fs.remove_file(scope, path)?,
        }
        self.create(fs, scope, path)
    }

    fn create(&self, fs: &dyn FsWrite, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        match self {
            Self::Absent => Ok(()),
            Self::File { content, meta } => {
                let mut handle = fs.create_new(scope, path)?;
                handle
                    .write_all(content)
                    .map_err(|_| FsError::new(FsErrorKind::Io, "write", Some(path.as_path())))?;
                handle.sync(Durability::Full)?;
                drop(handle);
                fs.restore_meta(scope, path, meta).map(|_| ())
            }
            Self::Directory { meta, children } => {
                fs.create_dir(scope, path, meta.basic.mode)?;
                for (name, child) in children {
                    child.create(fs, scope, &child_path(path, name)?)?;
                }
                fs.restore_meta(scope, path, meta).map(|_| ())
            }
            Self::Link { target, kind } => fs.create_link(scope, target, path, *kind),
            Self::Special => Err(FsError::new(
                FsErrorKind::Unsupported,
                "restore",
                Some(path.as_path()),
            )),
        }
    }
}

/// Previous state of one changed target of a committed operation.
#[derive(Debug, Clone)]
struct KeptTarget {
    outcome: TargetOutcome,
    images: Vec<(AbsPath, PreImage)>,
}

#[derive(Debug, Default)]
struct Register {
    transactions: Vec<TxRecord>,
    kept: BTreeMap<OpId, Vec<KeptTarget>>,
    failing: Vec<TargetPath>,
}

/// Pass-through journal over a mutating file system.
///
/// Operation numbers are assigned from 1 upwards per instance, at the first
/// [`MutationTx::preserve`] or, for transactions consisting of irreversible
/// markers only, at commit.
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::journal::{
///     JournalStore, Mutation, PreserveMode, TargetAction, TargetIndex, TargetPath, TargetSpec,
/// };
/// use nurt_testkit::journal::NoopJournal;
/// use nurt_testkit::vfs::{VirtualFs, test_path};
/// use nurt_testkit::fixtures::begin_request;
///
/// let fs = Arc::new(VirtualFs::default());
/// let path = test_path("/data/a.txt").unwrap();
/// fs.write_file(&path, b"old").unwrap();
///
/// let journal = NoopJournal::new(fs.clone());
/// let mut tx = journal.begin(begin_request("write"));
/// let preserved = tx
///     .preserve(&[TargetSpec {
///         path: TargetPath::Fs(path.clone()),
///         action: TargetAction::Modify,
///         expect: Default::default(),
///         size_hint: None,
///     }])
///     .unwrap();
/// assert_eq!(preserved[0].mode, PreserveMode::None);
/// tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec())).unwrap();
/// let handle = tx.commit().unwrap();
/// assert_eq!(handle.op.map(|op| op.0), Some(1));
/// assert_eq!(fs.read_file(&path).unwrap(), b"new");
/// ```
pub struct NoopJournal {
    fs: Arc<dyn FsWrite>,
    next_op: AtomicU64,
    calls: Option<Mutex<Vec<JournalCall>>>,
    keep: bool,
    register: Mutex<Register>,
}

impl NoopJournal {
    /// Journal applying mutations to `fs`.
    #[must_use]
    pub fn new(fs: Arc<dyn FsWrite>) -> Self {
        Self {
            fs,
            next_op: AtomicU64::new(1),
            calls: None,
            keep: false,
            register: Mutex::new(Register::default()),
        }
    }

    fn recording(fs: Arc<dyn FsWrite>, keep: bool) -> Self {
        Self {
            calls: Some(Mutex::new(Vec::new())),
            keep,
            ..Self::new(fs)
        }
    }

    fn record(&self, call: JournalCall) {
        if let Some(calls) = &self.calls {
            lock(calls).push(call);
        }
    }

    fn allocate(&self) -> OpId {
        OpId(self.next_op.fetch_add(1, Ordering::Relaxed))
    }

    fn register(&self) -> MutexGuard<'_, Register> {
        lock(&self.register)
    }

    fn update(&self, tx: usize, change: impl FnOnce(&mut TxRecord)) {
        if let Some(record) = self.register().transactions.get_mut(tx) {
            change(record);
        }
    }

    // A journal implementation is the only holder of scopes, which the
    // constructor is reserved for.
    #[allow(clippy::disallowed_methods)]
    fn scope_for(op: Option<OpId>) -> TxScope {
        TxScope::new_for_journal(op.unwrap_or(OpId(0)), ScopePurpose::Operation)
    }

    /// Restores one target from its previous state; `false` when the target
    /// was not kept, restoring it is set to fail, or a file system call
    /// failed.
    fn restore_target(
        &self,
        op: Option<OpId>,
        path: &TargetPath,
        mode: PreserveMode,
        images: &[(AbsPath, PreImage)],
    ) -> bool {
        if !mode.is_preserved() || self.register().failing.contains(path) {
            return false;
        }
        let scope = Self::scope_for(op);
        images
            .iter()
            .rev()
            .all(|(entry, image)| image.restore(self.fs.as_ref(), &scope, entry).is_ok())
    }
}

impl std::fmt::Debug for NoopJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoopJournal")
            .field("next_op", &self.next_op.load(Ordering::Relaxed))
            .field("recording", &self.calls.is_some())
            .field("keep", &self.keep)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn journal_noop(what: &str) -> Error {
    Error::new(
        ErrorCode::JournalNoop,
        format!("the pass-through journal keeps no entries: {what} has nothing to act on"),
    )
}

fn rollback_failed(cause: Option<ErrorCode>, failed: &[TargetPath]) -> Error {
    let mut error = Error::new(
        ErrorCode::RollbackFailed,
        format!(
            "{} target(s) could not be restored; inspect them before retrying",
            failed.len()
        ),
    )
    .with_detail(
        "failed",
        serde_json::Value::from(failed.iter().map(path_text).collect::<Vec<_>>()),
    );
    if let Some(code) = cause {
        error = error.with_detail("cause", serde_json::Value::from(code.to_string()));
    }
    error
}

impl JournalStore for NoopJournal {
    fn recover(&self) -> Result<RecoveryReport> {
        self.record(JournalCall::Recover);
        Ok(RecoveryReport::default())
    }

    fn begin(&self, request: BeginRequest) -> Box<dyn MutationTx + '_> {
        self.record(JournalCall::Begin {
            kind: request.kind.clone(),
        });
        let tx = {
            let mut register = self.register();
            register
                .transactions
                .push(TxRecord::open(request.kind.clone()));
            register.transactions.len() - 1
        };
        Box::new(NoopTx {
            journal: self,
            tx,
            request,
            op: None,
            targets: Vec::new(),
            markers: Vec::new(),
            exempt: Vec::new(),
            applied: false,
            rolled_back: false,
        })
    }

    fn rollback_committed(&self, op: OpId, reason: RollbackReason) -> Result<UndoOutcome> {
        self.record(JournalCall::Store {
            method: "rollback_committed",
        });
        if !self.keep {
            return Err(journal_noop("rollback_committed"));
        }
        let Some(kept) = self.register().kept.remove(&op) else {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("operation {} has no committed entry to roll back", op.0),
            ));
        };
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        for target in kept.iter().rev() {
            let outcome = &target.outcome;
            if self.restore_target(Some(op), &outcome.path, outcome.mode, &target.images) {
                restored.push(outcome.clone());
            } else {
                failed.push(outcome.path.clone());
            }
        }
        restored.reverse();
        failed.reverse();
        let restored_paths: Vec<TargetPath> = restored.iter().map(|t| t.path.clone()).collect();
        {
            let mut register = self.register();
            if let Some(record) = register
                .transactions
                .iter_mut()
                .find(|record| record.op == Some(op))
            {
                record.state = TxState::RolledBack;
                record.reason = Some(reason);
                record.restored = restored_paths;
                record.not_restored.clone_from(&failed);
            }
        }
        if !failed.is_empty() {
            return Err(rollback_failed(None, &failed));
        }
        Ok(UndoOutcome {
            entry: self.allocate(),
            scope: UndoScope::Op,
            sources: vec![op],
            restored,
            unavailable: Vec::new(),
            skipped: Vec::new(),
            forced: Vec::new(),
            irreversible: Vec::new(),
        })
    }

    fn history(&self, _query: &HistoryQuery) -> Result<Vec<OperationRecord>> {
        self.record(JournalCall::Store { method: "history" });
        Ok(Vec::new())
    }

    fn details(&self, _op: OpId) -> Result<OperationDetails> {
        self.record(JournalCall::Store { method: "details" });
        Err(journal_noop("details"))
    }

    fn undo(&self, _request: &UndoRequest) -> Result<UndoOutcome> {
        self.record(JournalCall::Store { method: "undo" });
        Err(journal_noop("undo"))
    }

    fn redo(&self, _request: &RedoRequest) -> Result<UndoOutcome> {
        self.record(JournalCall::Store { method: "redo" });
        Err(journal_noop("redo"))
    }

    fn pin(&self, request: &PinRequest) -> Result<PinOutcome> {
        self.record(JournalCall::Store { method: "pin" });
        Ok(PinOutcome {
            not_available: request.ids.clone(),
            ..PinOutcome::default()
        })
    }

    fn purge(&self, _request: &PurgeRequest) -> Result<PurgeOutcome> {
        self.record(JournalCall::Store { method: "purge" });
        Ok(PurgeOutcome {
            released: Vec::new(),
            bytes: 0,
        })
    }

    fn status(&self, _volume: Option<&AbsPath>) -> Result<Vec<JournalState>> {
        self.record(JournalCall::Store { method: "status" });
        Ok(Vec::new())
    }

    fn limits(&self, _request: &LimitsRequest) -> Result<LimitsOutcome> {
        self.record(JournalCall::Store { method: "limits" });
        Ok(LimitsOutcome {
            effective: Vec::new(),
            expiring: Vec::new(),
            expiring_bytes: 0,
            applied: false,
        })
    }

    fn space(&self, _volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>> {
        self.record(JournalCall::Store { method: "space" });
        Ok(Vec::new())
    }

    fn sweep(&self) -> Result<SweepReport> {
        self.record(JournalCall::Store { method: "sweep" });
        Ok(SweepReport::default())
    }
}

/// [`NoopJournal`] that keeps every call in order and a register of
/// transactions; [`RecordingJournal::keeping`] also restores previous
/// states.
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::journal::JournalStore;
/// use nurt_testkit::fixtures::begin_request;
/// use nurt_testkit::journal::{JournalCall, RecordingJournal, TxState};
/// use nurt_testkit::vfs::VirtualFs;
///
/// let journal = RecordingJournal::new(Arc::new(VirtualFs::default()));
/// let handle = journal.begin(begin_request("write")).commit().unwrap();
/// assert_eq!(handle.op, None);
/// assert_eq!(journal.calls().len(), 2);
/// assert!(matches!(journal.calls()[1], JournalCall::Commit { op: None }));
/// assert_eq!(journal.transactions()[0].state, TxState::Committed);
/// ```
#[derive(Debug)]
pub struct RecordingJournal {
    inner: NoopJournal,
}

impl RecordingJournal {
    /// Recording pass-through journal applying mutations to `fs`.
    #[must_use]
    pub fn new(fs: Arc<dyn FsWrite>) -> Self {
        Self {
            inner: NoopJournal::recording(fs, false),
        }
    }

    /// Recording journal applying mutations to `fs` that keeps an in-memory
    /// copy of the previous state of every file system target and restores
    /// it on rollback and on [`JournalStore::rollback_committed`].
    ///
    /// Files, directory trees and links are kept with their metadata;
    /// special files are not kept and get [`PreserveMode::None`] with
    /// [`Irreversible::PreviousContentNotKept`].
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use nurt_contracts::journal::{FileOp, JournalStore, PreserveMode, RollbackReason, apply_ops};
    /// use nurt_testkit::fixtures::begin_request;
    /// use nurt_testkit::journal::RecordingJournal;
    /// use nurt_testkit::vfs::{VirtualFs, test_path};
    ///
    /// let fs = Arc::new(VirtualFs::default());
    /// let path = test_path("/data/a.txt").unwrap();
    /// fs.write_file(&path, b"old").unwrap();
    ///
    /// let journal = RecordingJournal::keeping(fs.clone());
    /// let mut tx = journal.begin(begin_request("write"));
    /// let ops = vec![FileOp::write(path.clone(), b"new".to_vec())];
    /// let done = apply_ops(tx.as_mut(), ops).unwrap();
    /// assert_eq!(done[0].preserved.mode, PreserveMode::Copy);
    /// let report = tx.rollback(RollbackReason::VerifyFailed).unwrap();
    /// assert!(report.failed.is_empty());
    /// assert_eq!(fs.read_file(&path).unwrap(), b"old");
    /// ```
    #[must_use]
    pub fn keeping(fs: Arc<dyn FsWrite>) -> Self {
        Self {
            inner: NoopJournal::recording(fs, true),
        }
    }

    /// Calls made so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<JournalCall> {
        self.inner
            .calls
            .as_ref()
            .map(|calls| lock(calls).clone())
            .unwrap_or_default()
    }

    /// Numbers of the operations committed so far, in commit order.
    #[must_use]
    pub fn committed(&self) -> Vec<OpId> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Commit { op } => op,
                _ => None,
            })
            .collect()
    }

    /// Transactions begun so far, in order of [`JournalStore::begin`].
    #[must_use]
    pub fn transactions(&self) -> Vec<TxRecord> {
        self.inner.register().transactions.clone()
    }

    /// Makes every later attempt to restore `path` fail, in rollbacks and in
    /// [`JournalStore::rollback_committed`]; the target is then listed in
    /// [`RollbackReport::failed`] and failed calls report
    /// `E_ROLLBACK_FAILED`.
    pub fn fail_rollback_of(&self, path: TargetPath) {
        let mut register = self.inner.register();
        if !register.failing.contains(&path) {
            register.failing.push(path);
        }
    }

    /// Forgets the calls and transactions recorded so far. Kept previous
    /// states of committed operations and rollback failures stay.
    pub fn clear(&self) {
        if let Some(calls) = &self.inner.calls {
            lock(calls).clear();
        }
        self.inner.register().transactions.clear();
    }
}

impl JournalStore for RecordingJournal {
    fn recover(&self) -> Result<RecoveryReport> {
        self.inner.recover()
    }

    fn begin(&self, request: BeginRequest) -> Box<dyn MutationTx + '_> {
        self.inner.begin(request)
    }

    fn rollback_committed(&self, op: OpId, reason: RollbackReason) -> Result<UndoOutcome> {
        self.inner.rollback_committed(op, reason)
    }

    fn history(&self, query: &HistoryQuery) -> Result<Vec<OperationRecord>> {
        self.inner.history(query)
    }

    fn details(&self, op: OpId) -> Result<OperationDetails> {
        self.inner.details(op)
    }

    fn undo(&self, request: &UndoRequest) -> Result<UndoOutcome> {
        self.inner.undo(request)
    }

    fn redo(&self, request: &RedoRequest) -> Result<UndoOutcome> {
        self.inner.redo(request)
    }

    fn pin(&self, request: &PinRequest) -> Result<PinOutcome> {
        self.inner.pin(request)
    }

    fn purge(&self, request: &PurgeRequest) -> Result<PurgeOutcome> {
        self.inner.purge(request)
    }

    fn status(&self, volume: Option<&AbsPath>) -> Result<Vec<JournalState>> {
        self.inner.status(volume)
    }

    fn limits(&self, request: &LimitsRequest) -> Result<LimitsOutcome> {
        self.inner.limits(request)
    }

    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>> {
        self.inner.space(volume)
    }

    fn sweep(&self) -> Result<SweepReport> {
        self.inner.sweep()
    }
}

#[derive(Debug)]
struct Target {
    spec: TargetSpec,
    before: BeforeState,
    after: Option<AfterState>,
    changed: bool,
    staged: Option<AbsPath>,
    undeclared: bool,
    mode: PreserveMode,
    reason: Option<Irreversible>,
    images: Vec<(AbsPath, PreImage)>,
}

impl Target {
    fn bytes_held(&self) -> u64 {
        if self.mode.is_preserved() {
            self.images
                .iter()
                .map(|(_, image)| image.bytes())
                .fold(0, u64::saturating_add)
        } else {
            0
        }
    }
}

struct NoopTx<'a> {
    journal: &'a NoopJournal,
    tx: usize,
    request: BeginRequest,
    op: Option<OpId>,
    targets: Vec<Target>,
    markers: Vec<IrreversibleEntry>,
    exempt: Vec<ExemptWrite>,
    applied: bool,
    rolled_back: bool,
}

fn mutation_name(mutation: &Mutation) -> &'static str {
    match mutation {
        Mutation::CommitStaged => "commit_staged",
        Mutation::WriteBytes(_) => "write_bytes",
        Mutation::AppendBytes(_) => "append_bytes",
        Mutation::WriteAt { .. } => "write_at",
        Mutation::SetLength(_) => "set_length",
        Mutation::CopyFrom { .. } => "copy_from",
        Mutation::Remove => "remove",
        Mutation::Rename => "rename",
        Mutation::CreateDir { .. } => "create_dir",
        Mutation::CreateLink { .. } => "create_link",
        Mutation::CreateHardLink { .. } => "create_hard_link",
        Mutation::SetMeta(_) => "set_meta",
        Mutation::ProviderSet(_) => "provider_set",
        Mutation::ProviderRemove => "provider_remove",
        _ => "other",
    }
}

/// Whether `mutation` is allowed for a target declared with `action`, by the
/// table of common operations of the journal specification.
fn allowed(action: &TargetAction, mutation: &Mutation) -> bool {
    let writes_content = matches!(
        mutation,
        Mutation::WriteBytes(_) | Mutation::CommitStaged | Mutation::CopyFrom { .. }
    );
    let creates_entry = matches!(
        mutation,
        Mutation::CreateDir { .. } | Mutation::CreateLink { .. } | Mutation::CreateHardLink { .. }
    );
    match action {
        TargetAction::Create => writes_content || creates_entry,
        TargetAction::CreateTree => matches!(mutation, Mutation::CreateDir { .. }),
        TargetAction::Modify => {
            writes_content
                || matches!(
                    mutation,
                    Mutation::WriteAt { .. } | Mutation::SetLength(_) | Mutation::ProviderSet(_)
                )
        }
        TargetAction::Append => matches!(mutation, Mutation::AppendBytes(_)),
        TargetAction::Delete => matches!(mutation, Mutation::Remove | Mutation::ProviderRemove),
        TargetAction::Replace => {
            writes_content || creates_entry || matches!(mutation, Mutation::Remove)
        }
        TargetAction::Rename { .. } => matches!(mutation, Mutation::Rename),
        TargetAction::Attributes => matches!(mutation, Mutation::SetMeta(_)),
        TargetAction::Link => matches!(
            mutation,
            Mutation::CreateLink { .. } | Mutation::CreateHardLink { .. }
        ),
        TargetAction::Truncate => matches!(mutation, Mutation::SetLength(_)),
        _ => false,
    }
}

fn stages_content(action: &TargetAction) -> bool {
    matches!(
        action,
        TargetAction::Create | TargetAction::Modify | TargetAction::Replace
    )
}

/// Preserve mode of a target kept by a keeping journal.
fn kept_mode(action: &TargetAction, images: &[(AbsPath, PreImage)]) -> PreserveMode {
    if images.is_empty() || !images.iter().all(|(_, image)| image.restorable()) {
        return PreserveMode::None;
    }
    match (action, images.first()) {
        (_, Some((_, PreImage::Absent))) => PreserveMode::Created,
        (TargetAction::Attributes, _) => PreserveMode::Meta,
        (TargetAction::Rename { .. }, _) => PreserveMode::Rename,
        (TargetAction::Append, _) => PreserveMode::Append,
        _ => PreserveMode::Copy,
    }
}

fn stale(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Stale,
        format!("{path} changed since it was read; read it again"),
    )
    .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

fn no_provider(key: &TargetPath) -> Error {
    let scheme = match key {
        TargetPath::Provider(key) => key.scheme.0.clone(),
        TargetPath::Fs(_) => String::new(),
    };
    Error::new(
        ErrorCode::Unsupported,
        format!("the pass-through journal has no provider for scheme `{scheme}`"),
    )
}

impl NoopTx<'_> {
    fn fs(&self) -> &dyn FsWrite {
        self.journal.fs.as_ref()
    }

    fn scope(&self) -> TxScope {
        NoopJournal::scope_for(self.op)
    }

    fn ensure_open(&self) -> Result<()> {
        if self.rolled_back {
            Err(Error::new(
                ErrorCode::RolledBack,
                "the transaction was rolled back after an earlier failure",
            ))
        } else {
            Ok(())
        }
    }

    fn remove_staged(&mut self) {
        let scope = self.scope();
        let staged: Vec<AbsPath> = self
            .targets
            .iter_mut()
            .filter_map(|target| target.staged.take())
            .collect();
        for path in staged {
            let _ignored = self.fs().remove_file(&scope, &path);
        }
    }

    /// Restores the changed targets in reverse order; returns the restored
    /// and the not restored targets in declaration order.
    fn restore_changed(&self) -> (Vec<TargetPath>, Vec<TargetPath>) {
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        for target in self.targets.iter().rev().filter(|target| target.changed) {
            let path = target.spec.path.clone();
            if self
                .journal
                .restore_target(self.op, &path, target.mode, &target.images)
            {
                restored.push(path);
            } else {
                failed.push(path);
            }
        }
        restored.reverse();
        failed.reverse();
        (restored, failed)
    }

    /// Rolls the transaction back once; returns the restored and the not
    /// restored targets.
    fn roll_back(&mut self, reason: Option<RollbackReason>) -> (Vec<TargetPath>, Vec<TargetPath>) {
        self.remove_staged();
        if self.rolled_back {
            let register = self.journal.register();
            return register
                .transactions
                .get(self.tx)
                .map(|record| (record.restored.clone(), record.not_restored.clone()))
                .unwrap_or_default();
        }
        self.rolled_back = true;
        let (restored, failed) = self.restore_changed();
        self.journal.record(JournalCall::Rollback {
            op: self.op,
            reason,
        });
        let op = self.op;
        let (kept_restored, kept_failed) = (restored.clone(), failed.clone());
        self.journal.update(self.tx, |record| {
            record.op = op;
            record.state = TxState::RolledBack;
            record.reason = reason;
            record.restored = kept_restored;
            record.not_restored = kept_failed;
        });
        (restored, failed)
    }

    fn fail(&mut self, error: Error) -> Error {
        if self.rolled_back {
            self.remove_staged();
            return error;
        }
        let (_, failed) = self.roll_back(None);
        if self.journal.keep && !failed.is_empty() {
            rollback_failed(Some(error.code()), &failed)
        } else {
            error
        }
    }

    fn misuse(&mut self, what: &str) -> Error {
        self.fail(Error::new(
            ErrorCode::StageFailed,
            format!("journal protocol misuse: {what}"),
        ))
    }

    fn meta(&self, path: &AbsPath) -> Result<Option<FileMeta>> {
        match self.fs().metadata(path, Follow::NoFollow) {
            Ok(meta) => Ok(Some(meta)),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(None),
            Err(error) => Err(convert(error)),
        }
    }

    fn observe(&self, path: &AbsPath) -> Result<AfterState> {
        let Some(meta) = self.meta(path)? else {
            return Ok(AfterState::default());
        };
        let hash = if meta.kind == FileKind::File {
            Some(self.fs().content_hash(path).map_err(convert)?)
        } else {
            None
        };
        Ok(AfterState {
            exists: true,
            kind: Some(meta.kind),
            size: Some(meta.size),
            hash,
            mode: Some(meta.mode),
            meta: Some(meta),
        })
    }

    fn check_expect(&self, path: &AbsPath, expect: &Expect) -> Result<()> {
        let matches = match expect {
            Expect::Unchecked => true,
            Expect::Absent => self.meta(path)?.is_none(),
            Expect::Etag(tag) => match self.fs().etag(path, Follow::NoFollow) {
                Ok(current) => current.matches(tag),
                Err(error) if error.kind == FsErrorKind::NotFound => false,
                Err(error) => return Err(convert(error)),
            },
            Expect::Hash(hash) => match self.fs().content_hash(path) {
                Ok(current) => &current == hash,
                Err(error) if error.kind == FsErrorKind::NotFound => false,
                Err(error) => return Err(convert(error)),
            },
        };
        if matches { Ok(()) } else { Err(stale(path)) }
    }

    fn not_kept(&self) -> Irreversible {
        if self.journal.keep {
            Irreversible::PreviousContentNotKept
        } else {
            Irreversible::JournalInactive
        }
    }

    fn declare(&self, spec: &TargetSpec) -> Result<(Target, Option<VolumeId>)> {
        let TargetPath::Fs(path) = &spec.path else {
            let target = Target {
                spec: spec.clone(),
                before: BeforeState::default(),
                after: None,
                changed: false,
                staged: None,
                undeclared: false,
                mode: PreserveMode::None,
                reason: Some(self.not_kept()),
                images: Vec::new(),
            };
            return Ok((target, None));
        };
        let meta = self.meta(path)?;
        let exists = meta.is_some();
        match &spec.action {
            TargetAction::Create | TargetAction::CreateTree | TargetAction::Link if exists => {
                return Err(fs_error_to_error(&FsError::new(
                    FsErrorKind::Exists,
                    "preserve",
                    Some(path.as_path()),
                )));
            }
            TargetAction::Delete
            | TargetAction::Replace
            | TargetAction::Rename { .. }
            | TargetAction::Attributes
            | TargetAction::Truncate
                if !exists =>
            {
                return Err(fs_error_to_error(&FsError::new(
                    FsErrorKind::NotFound,
                    "preserve",
                    Some(path.as_path()),
                )));
            }
            _ => {}
        }
        self.check_expect(path, &spec.expect)?;
        let observed = self.observe(path)?;
        let before = BeforeState {
            exists: observed.exists,
            kind: observed.kind,
            size: observed.size,
            hash: observed.hash,
            mode: observed.mode,
        };
        let mut images = Vec::new();
        if self.journal.keep {
            images.push((
                path.clone(),
                PreImage::capture(self.fs(), path).map_err(convert)?,
            ));
            if let TargetAction::Rename { to, .. } = &spec.action {
                images.push((
                    to.clone(),
                    PreImage::capture(self.fs(), to).map_err(convert)?,
                ));
            }
        }
        let mode = kept_mode(&spec.action, &images);
        let reason = if mode.is_preserved() {
            None
        } else {
            Some(self.not_kept())
        };
        let volume = self.fs().volume(path).ok().map(|info| info.id);
        let target = Target {
            spec: spec.clone(),
            before,
            after: None,
            changed: false,
            staged: None,
            undeclared: false,
            mode,
            reason,
            images,
        };
        Ok((target, volume))
    }

    fn write_content(&self, scope: &TxScope, path: &AbsPath, bytes: &[u8]) -> Result<()> {
        let fs = self.fs();
        let parent = path.parent().ok_or_else(|| {
            fs_error_to_error(&FsError::new(
                FsErrorKind::InvalidPath,
                "write",
                Some(path.as_path()),
            ))
        })?;
        let hint = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let previous = match fs.extended_metadata(path, Follow::NoFollow) {
            Ok(meta) if meta.basic.kind == FileKind::File => Some(meta),
            Ok(_) => None,
            Err(error) if error.kind == FsErrorKind::NotFound => None,
            Err(error) => return Err(convert(error)),
        };
        let mut handle = fs.create_temp_in(scope, &parent, &hint).map_err(convert)?;
        let staged = handle.path().clone();
        handle
            .write_all(bytes)
            .map_err(|error| io_error_to_error(&error, path))?;
        if let Some(meta) = &previous {
            // The new content is dated by its write; the rest of the
            // metadata of the previous file is transferred.
            let written = fs.metadata(&staged, Follow::NoFollow).map_err(convert)?;
            let mut wanted = meta.clone();
            wanted.basic.modified_ns = written.modified_ns;
            wanted.basic.accessed_ns = written.accessed_ns;
            handle.apply_meta(&wanted).map_err(convert)?;
        }
        handle.sync(Durability::Full).map_err(convert)?;
        drop(handle);
        fs.rename(scope, &staged, path, RenameMode::Replace)
            .map_err(convert)
    }

    /// Gives content copied or moved from a file the times of that file, as
    /// the journal does for a copy and for a move across volumes; the
    /// permission bits follow when the content went to a new file.
    fn take_over(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        origin: &FileMeta,
        with_mode: bool,
    ) -> Result<()> {
        let change = MetaChange {
            mode: with_mode.then_some(origin.mode & PERMISSION_BITS),
            modified_ns: Some(origin.modified_ns),
            accessed_ns: origin.accessed_ns,
            ..MetaChange::default()
        };
        self.fs().set_meta(scope, path, &change).map_err(convert)
    }

    fn read_all(&self, path: &AbsPath) -> Result<Vec<u8>> {
        let mut handle = self.fs().open_read(path).map_err(convert)?;
        let mut bytes = Vec::new();
        handle
            .read_to_end(&mut bytes)
            .map_err(|error| io_error_to_error(&error, path))?;
        Ok(bytes)
    }

    fn write_in_place(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        at: SeekFrom,
        bytes: &[u8],
    ) -> Result<()> {
        let fs = self.fs();
        let mut handle: Box<dyn WriteHandle> = match fs.open_in_place(scope, path) {
            Ok(handle) => handle,
            Err(error) if error.kind == FsErrorKind::NotFound => {
                fs.create_new(scope, path).map_err(convert)?
            }
            Err(error) => return Err(convert(error)),
        };
        handle
            .seek(at)
            .and_then(|_| handle.write_all(bytes))
            .map_err(|error| io_error_to_error(&error, path))?;
        handle.sync(Durability::Full).map_err(convert)
    }

    fn rename(&self, scope: &TxScope, from: &AbsPath, to: &AbsPath, replace: bool) -> Result<()> {
        let fs = self.fs();
        let mode = if replace {
            RenameMode::Replace
        } else {
            RenameMode::NoReplace
        };
        match fs.rename(scope, from, to, mode) {
            Err(error) if error.kind == FsErrorKind::CrossDevice => {
                let is_file = self
                    .meta(from)?
                    .is_some_and(|meta| meta.kind == FileKind::File);
                if !is_file {
                    return Err(convert(error));
                }
                if !replace && self.meta(to)?.is_some() {
                    return Err(fs_error_to_error(&FsError::new(
                        FsErrorKind::Exists,
                        "rename",
                        Some(to.as_path()),
                    )));
                }
                let bytes = self.read_all(from)?;
                let origin = fs.metadata(from, Follow::NoFollow).map_err(convert)?;
                self.write_content(scope, to, &bytes)?;
                self.take_over(scope, to, &origin, true)?;
                fs.remove_file(scope, from).map_err(convert)
            }
            other => other.map_err(convert),
        }
    }

    fn perform(
        &mut self,
        index: usize,
        path: &AbsPath,
        action: &TargetAction,
        mutation: &Mutation,
    ) -> Result<()> {
        let scope = self.scope();
        let journal = self.journal;
        let fs = journal.fs.as_ref();
        match mutation {
            Mutation::CommitStaged => {
                let Some(staged) = self.targets[index].staged.take() else {
                    return Err(self.misuse("commit_staged without a staged file"));
                };
                if let Ok(previous) = fs.extended_metadata(path, Follow::NoFollow)
                    && previous.basic.kind == FileKind::File
                {
                    let mut handle = fs.open_in_place(&scope, &staged).map_err(convert)?;
                    handle.apply_meta(&previous).map_err(convert)?;
                }
                fs.rename(&scope, &staged, path, RenameMode::Replace)
                    .map_err(convert)
            }
            Mutation::WriteBytes(bytes) => self.write_content(&scope, path, bytes),
            Mutation::AppendBytes(bytes) => {
                self.write_in_place(&scope, path, SeekFrom::End(0), bytes)
            }
            Mutation::WriteAt { offset, bytes } => {
                self.write_in_place(&scope, path, SeekFrom::Start(*offset), bytes)
            }
            Mutation::SetLength(length) => fs.truncate(&scope, path, *length).map_err(convert),
            Mutation::CopyFrom { source, clone } => {
                if *clone == CloneMode::Always {
                    let reflink = fs.volume(source).map_err(convert)?.features.reflink;
                    if !reflink {
                        return Err(fs_error_to_error(&FsError::new(
                            FsErrorKind::Unsupported,
                            "clone_file",
                            Some(source.as_path()),
                        )));
                    }
                }
                let bytes = self.read_all(source)?;
                let origin = fs.metadata(source, Follow::NoFollow).map_err(convert)?;
                let replaced = self
                    .meta(path)?
                    .is_some_and(|meta| meta.kind == FileKind::File);
                self.write_content(&scope, path, &bytes)?;
                self.take_over(&scope, path, &origin, !replaced)
            }
            Mutation::Remove => {
                let is_dir = self
                    .meta(path)?
                    .is_some_and(|meta| meta.kind == FileKind::Directory);
                if is_dir {
                    fs.remove_tree(&scope, path, REMOVE_PARALLELISM)
                        .map(|_| ())
                        .map_err(convert)
                } else {
                    fs.remove_file(&scope, path).map_err(convert)
                }
            }
            Mutation::Rename => match action {
                TargetAction::Rename { to, replace } => self.rename(&scope, path, to, *replace),
                _ => Err(self.misuse("rename for a target not declared as rename")),
            },
            Mutation::CreateDir { mode } => fs.create_dir(&scope, path, *mode).map_err(convert),
            Mutation::CreateLink { target, kind } => {
                fs.create_link(&scope, target, path, *kind).map_err(convert)
            }
            Mutation::CreateHardLink { existing } => {
                fs.hard_link(&scope, existing, path).map_err(convert)
            }
            Mutation::SetMeta(change) => fs.set_meta(&scope, path, change).map_err(convert),
            _ => Err(self.misuse("mutation not supported for file system targets")),
        }
    }

    fn mark_changed(&mut self, index: usize, after: AfterState) {
        let Some(entry) = self.targets.get_mut(index) else {
            return;
        };
        let first = !entry.changed;
        entry.changed = true;
        entry.after = Some(after);
        if first {
            let path = entry.spec.path.clone();
            let op = self.op;
            self.journal.update(self.tx, |record| {
                record.op = op;
                record.applied.push(path);
            });
        }
    }

    fn apply_inner(&mut self, target: TargetIndex, mutation: &Mutation) -> Result<Applied> {
        let index = usize::try_from(target.0).unwrap_or(usize::MAX);
        let Some(declared) = self.targets.get(index) else {
            return Err(self.misuse("apply to an undeclared target"));
        };
        if declared.undeclared || !allowed(&declared.spec.action, mutation) {
            return Err(self.misuse("mutation does not match the declared action"));
        }
        let spec = declared.spec.clone();
        let first_change = !declared.changed;
        let TargetPath::Fs(path) = &spec.path else {
            return Err(self.fail(no_provider(&spec.path)));
        };
        if first_change {
            self.check_expect(path, &spec.expect)
                .map_err(|error| self.fail(error))?;
        }
        if let Err(error) = self.perform(index, path, &spec.action, mutation) {
            // The failed call may have changed the target partly; it is
            // restored together with the targets changed before it.
            if let Some(entry) = self.targets.get_mut(index) {
                entry.changed = true;
            }
            return Err(self.fail(error));
        }
        let after = self.observe(path).map_err(|error| self.fail(error))?;
        self.applied = true;
        self.mark_changed(index, after.clone());
        Ok(Applied {
            index: target,
            after,
        })
    }

    fn outcome(&self, index: usize, target: &Target) -> TargetOutcome {
        TargetOutcome {
            index: TargetIndex(u32::try_from(index).unwrap_or(u32::MAX)),
            path: target.spec.path.clone(),
            action: target.spec.action.clone(),
            mode: target.mode,
            bytes_held: target.bytes_held(),
            before: target.before.clone(),
            after: target.after.clone(),
            irreversible: target.reason,
            notable: Vec::new(),
        }
    }
}

fn path_text(path: &TargetPath) -> String {
    match path {
        TargetPath::Fs(path) => path.to_language_string(),
        TargetPath::Provider(key) => {
            let mut text = format!("{}:{}", key.scheme.0, key.segments.join("/"));
            if let Some(value) = &key.value {
                text.push('@');
                text.push_str(value);
            }
            text
        }
    }
}

impl MutationTx for NoopTx<'_> {
    fn op(&self) -> Option<OpId> {
        self.op
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        self.ensure_open()?;
        if self.applied {
            return Err(self.misuse("preserve after the first change"));
        }
        if self.op.is_none() {
            self.op = Some(self.journal.allocate());
        }
        self.journal.record(JournalCall::Preserve {
            op: self.op,
            targets: targets.iter().map(|spec| spec.path.clone()).collect(),
        });
        let mut out = Vec::with_capacity(targets.len());
        for spec in targets {
            let (target, volume) = self.declare(spec).map_err(|error| self.fail(error))?;
            let index = TargetIndex(u32::try_from(self.targets.len()).unwrap_or(u32::MAX));
            out.push(Preserved {
                index,
                volume,
                mode: target.mode,
                bytes_held: target.bytes_held(),
                before: target.before.clone(),
                irreversible: target.reason,
                notable: Vec::new(),
            });
            let op = self.op;
            let path = spec.path.clone();
            self.journal.update(self.tx, |record| {
                record.op = op;
                record.preserved.push(path);
            });
            self.targets.push(target);
        }
        Ok(out)
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        self.ensure_open()?;
        self.journal.record(JournalCall::Stage {
            op: self.op,
            index: target,
        });
        let index = usize::try_from(target.0).unwrap_or(usize::MAX);
        let Some(declared) = self.targets.get(index) else {
            return Err(self.misuse("stage for an undeclared target"));
        };
        if !stages_content(&declared.spec.action) {
            return Err(self.misuse("stage for a target that does not write content"));
        }
        let TargetPath::Fs(path) = declared.spec.path.clone() else {
            return Err(self.misuse("stage for a provider target"));
        };
        let Some(parent) = path.parent() else {
            return Err(self.misuse("stage for a root directory"));
        };
        let hint = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let scope = self.scope();
        let handle = self
            .fs()
            .create_temp_in(&scope, &parent, &hint)
            .map_err(|error| self.fail(convert(error)))?;
        let staged = handle.path().clone();
        if let Some(previous) = self
            .targets
            .get_mut(index)
            .and_then(|entry| entry.staged.replace(staged))
        {
            let _ignored = self.fs().remove_file(&scope, &previous);
        }
        Ok(handle)
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.ensure_open()?;
        let result = self.apply_inner(target, &mutation);
        self.journal.record(JournalCall::Apply {
            op: self.op,
            index: target,
            mutation: mutation_name(&mutation),
            ok: result.is_ok(),
        });
        result
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.ensure_open()?;
        self.journal.record(JournalCall::ObserveExternal {
            op: self.op,
            paths: changes.iter().map(|change| change.path.clone()).collect(),
        });
        let mut out = Vec::with_capacity(changes.len());
        for change in changes {
            let index = match change.declared {
                Some(declared) => {
                    let index = usize::try_from(declared.0).unwrap_or(usize::MAX);
                    let external = self
                        .targets
                        .get(index)
                        .is_some_and(|target| target.spec.action == TargetAction::ExternalWrite);
                    if !external {
                        return Err(self
                            .misuse("external change of a target not declared as external_write"));
                    }
                    index
                }
                None => {
                    if self.op.is_none() {
                        self.op = Some(self.journal.allocate());
                    }
                    let path = TargetPath::Fs(change.path.clone());
                    let op = self.op;
                    let declared = path.clone();
                    self.journal.update(self.tx, |record| {
                        record.op = op;
                        record.preserved.push(declared);
                    });
                    self.targets.push(Target {
                        spec: TargetSpec {
                            path,
                            action: TargetAction::ExternalWrite,
                            expect: Expect::Unchecked,
                            size_hint: None,
                        },
                        before: BeforeState::default(),
                        after: None,
                        changed: false,
                        staged: None,
                        undeclared: true,
                        mode: PreserveMode::None,
                        reason: Some(Irreversible::PreviousContentNotKept),
                        images: Vec::new(),
                    });
                    self.targets.len() - 1
                }
            };
            let after = self
                .observe(&change.path)
                .map_err(|error| self.fail(error))?;
            self.applied = true;
            self.mark_changed(index, after.clone());
            out.push(Applied {
                index: TargetIndex(u32::try_from(index).unwrap_or(u32::MAX)),
                after,
            });
        }
        Ok(out)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        if self.rolled_back {
            return;
        }
        self.journal.record(JournalCall::MarkIrreversible {
            op: self.op,
            reason: marker.reason,
        });
        self.markers.push(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        if self.rolled_back {
            return;
        }
        self.journal
            .record(JournalCall::RecordExempt { op: self.op });
        self.exempt.push(write);
    }

    fn commit(mut self: Box<Self>) -> Result<OpHandle> {
        self.ensure_open()?;
        self.remove_staged();
        if self.op.is_none() && !self.markers.is_empty() {
            self.op = Some(self.journal.allocate());
        }
        let shown: Vec<(usize, &Target)> = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.changed || target.spec.action != TargetAction::ExternalWrite
            })
            .collect();
        let targets: Vec<TargetOutcome> = shown
            .iter()
            .map(|(index, target)| self.outcome(*index, target))
            .collect();
        let mut irreversible: Vec<IrreversibleEntry> = targets
            .iter()
            .filter_map(|outcome| {
                outcome.irreversible.map(|reason| IrreversibleEntry {
                    reason,
                    path: Some(path_text(&outcome.path)),
                    raw_path: None,
                    count: None,
                    detail: None,
                })
            })
            .collect();
        irreversible.extend(self.markers.iter().cloned());
        if self.journal.keep
            && let Some(op) = self.op
        {
            let kept: Vec<KeptTarget> = shown
                .iter()
                .filter(|(_, target)| target.changed)
                .map(|(index, target)| KeptTarget {
                    outcome: self.outcome(*index, target),
                    images: target.images.clone(),
                })
                .collect();
            self.journal.register().kept.insert(op, kept);
        }
        self.journal.record(JournalCall::Commit { op: self.op });
        let op = self.op;
        self.journal.update(self.tx, |record| {
            record.op = op;
            record.state = TxState::Committed;
        });
        Ok(OpHandle {
            op: self.op,
            kind: self.request.kind.clone(),
            bulk: self.request.bulk,
            targets,
            irreversible,
            notable: Vec::new(),
            exempt: std::mem::take(&mut self.exempt),
        })
    }

    fn rollback(mut self: Box<Self>, reason: RollbackReason) -> Result<RollbackReport> {
        let (restored, failed) = self.roll_back(Some(reason));
        Ok(RollbackReport {
            op: self.op,
            restored,
            failed,
        })
    }
}

impl Drop for NoopTx<'_> {
    fn drop(&mut self) {
        self.remove_staged();
    }
}

/// Admission with fixed answers.
///
/// By default every volume is eligible, every request is kept, every volume
/// is in state [`VolumeState::Normal`] with zero thresholds, and the space
/// service is alive. A configured answer applies to every call. Requests and
/// notifications are recorded.
///
/// ```
/// use nurt_contracts::fs::VolumeId;
/// use nurt_contracts::journal::{Admission, AdmissionDecision, AdmissionRequest};
/// use nurt_contracts::report::Irreversible;
/// use nurt_testkit::journal::TestAdmission;
///
/// let admission = TestAdmission::default().with_decision(AdmissionDecision::Skip {
///     reason: Irreversible::NoSpaceForCopy,
/// });
/// let request = AdmissionRequest {
///     volume: VolumeId { dev: 1, uuid: None, subvolume: 0 },
///     extra: 10,
///     op_size: 10,
///     bytes_held: 0,
///     size_budget: None,
///     oversize_keep: false,
///     op_size_limit: None,
/// };
/// assert!(matches!(admission.admit(&request), AdmissionDecision::Skip { .. }));
/// assert_eq!(admission.requests().len(), 1);
/// ```
#[derive(Debug)]
pub struct TestAdmission {
    eligibility: Eligibility,
    decision: AdmissionDecision,
    status: VolumeStatus,
    liveness: ServiceLiveness,
    requests: Mutex<Vec<AdmissionRequest>>,
    notified: Mutex<Vec<VolumeId>>,
}

impl TestAdmission {
    /// Answers every eligibility query with `eligibility`.
    #[must_use]
    pub fn with_eligibility(mut self, eligibility: Eligibility) -> Self {
        self.eligibility = eligibility;
        self
    }

    /// Answers every request with `decision`.
    #[must_use]
    pub fn with_decision(mut self, decision: AdmissionDecision) -> Self {
        self.decision = decision;
        self
    }

    /// Reports `status` for every volume.
    #[must_use]
    pub fn with_status(mut self, status: VolumeStatus) -> Self {
        self.status = status;
        self
    }

    /// Reports `liveness` for the space service.
    #[must_use]
    pub fn with_liveness(mut self, liveness: ServiceLiveness) -> Self {
        self.liveness = liveness;
        self
    }

    /// Requests received by [`Admission::admit`], in order.
    #[must_use]
    pub fn requests(&self) -> Vec<AdmissionRequest> {
        lock(&self.requests).clone()
    }

    /// Volumes passed to [`Admission::after_operation`], in order.
    #[must_use]
    pub fn notified(&self) -> Vec<VolumeId> {
        lock(&self.notified).clone()
    }
}

impl Default for TestAdmission {
    fn default() -> Self {
        Self {
            eligibility: Eligibility::Eligible,
            decision: AdmissionDecision::Keep,
            status: VolumeStatus {
                state: VolumeState::Normal,
                thresholds: Thresholds {
                    reserve: 0,
                    soft: 0,
                    margin: 0,
                    hysteresis: 0,
                },
            },
            liveness: ServiceLiveness::Alive,
            requests: Mutex::new(Vec::new()),
            notified: Mutex::new(Vec::new()),
        }
    }
}

impl Admission for TestAdmission {
    fn eligibility(&self, _volume: &nurt_contracts::fs::VolumeInfo) -> Eligibility {
        self.eligibility
    }

    fn admit(&self, request: &AdmissionRequest) -> AdmissionDecision {
        lock(&self.requests).push(request.clone());
        self.decision.clone()
    }

    fn after_operation(&self, volume: &VolumeId) {
        lock(&self.notified).push(volume.clone());
    }

    fn volume_status(&self, _volume: &VolumeId) -> VolumeStatus {
        self.status
    }

    fn liveness(&self) -> ServiceLiveness {
        self.liveness
    }
}
