// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pass-through journal and test admission.
//!
//! [`NoopJournal`] implements [`JournalStore`] and [`MutationTx`] with the
//! reference semantics of the pass-through journal in the journal
//! specification of the repository: mutations are applied directly to the
//! underlying [`FsWrite`] with a scope of purpose
//! [`ScopePurpose::Operation`], nothing is preserved, and every target is
//! reported with [`PreserveMode::None`] and the irreversible reason
//! [`Irreversible::JournalInactive`]. [`RecordingJournal`] behaves the same
//! and additionally keeps the list of calls in order.
//!
//! [`TestAdmission`] implements [`Admission`] with fixed answers.
//!
//! Misuse of the transaction protocol (preserve after the first change, an
//! undeclared target, a mutation that does not match the declared action)
//! fails with `E_STAGE_FAILED` and rolls the transaction back, so that the
//! defect is visible in the result of the stage under test.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Durability, FileKind, FileMeta, Follow, FsError, FsErrorKind, FsWrite, RenameMode,
    ScopePurpose, TxScope, VolumeId, WriteHandle,
};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, AdmissionRequest, AfterState, Applied, BeforeState, BeginRequest,
    CloneMode, Eligibility, ExemptWrite, Expect, ExternalChange, HistoryQuery, JournalState,
    JournalStore, LimitsOutcome, LimitsRequest, Mutation, MutationTx, OpHandle, OpId, OpKind,
    OperationDetails, OperationRecord, PinOutcome, PinRequest, PreserveMode, Preserved,
    PurgeOutcome, PurgeRequest, RecoveryReport, RedoRequest, RollbackReason, RollbackReport,
    ServiceLiveness, SweepReport, TargetAction, TargetIndex, TargetOutcome, TargetPath, TargetSpec,
    Thresholds, UndoOutcome, UndoRequest, VolumeSpace, VolumeState, VolumeStatus,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};

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
}

impl NoopJournal {
    /// Journal applying mutations to `fs`.
    #[must_use]
    pub fn new(fs: Arc<dyn FsWrite>) -> Self {
        Self {
            fs,
            next_op: AtomicU64::new(1),
            calls: None,
        }
    }

    fn recording(fs: Arc<dyn FsWrite>) -> Self {
        Self {
            calls: Some(Mutex::new(Vec::new())),
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
}

impl std::fmt::Debug for NoopJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoopJournal")
            .field("next_op", &self.next_op.load(Ordering::Relaxed))
            .field("recording", &self.calls.is_some())
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

impl JournalStore for NoopJournal {
    fn recover(&self) -> Result<RecoveryReport> {
        self.record(JournalCall::Recover);
        Ok(RecoveryReport::default())
    }

    fn begin(&self, request: BeginRequest) -> Box<dyn MutationTx + '_> {
        self.record(JournalCall::Begin {
            kind: request.kind.clone(),
        });
        Box::new(NoopTx {
            journal: self,
            request,
            op: None,
            targets: Vec::new(),
            markers: Vec::new(),
            exempt: Vec::new(),
            applied: false,
            rolled_back: false,
        })
    }

    fn rollback_committed(&self, _op: OpId, _reason: RollbackReason) -> Result<UndoOutcome> {
        self.record(JournalCall::Store {
            method: "rollback_committed",
        });
        Err(journal_noop("rollback_committed"))
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

/// [`NoopJournal`] that keeps every call in order.
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::journal::JournalStore;
/// use nurt_testkit::fixtures::begin_request;
/// use nurt_testkit::journal::{JournalCall, RecordingJournal};
/// use nurt_testkit::vfs::VirtualFs;
///
/// let journal = RecordingJournal::new(Arc::new(VirtualFs::default()));
/// let handle = journal.begin(begin_request("write")).commit().unwrap();
/// assert_eq!(handle.op, None);
/// assert_eq!(journal.calls().len(), 2);
/// assert!(matches!(journal.calls()[1], JournalCall::Commit { op: None }));
/// ```
#[derive(Debug)]
pub struct RecordingJournal {
    inner: NoopJournal,
}

impl RecordingJournal {
    /// Recording journal applying mutations to `fs`.
    #[must_use]
    pub fn new(fs: Arc<dyn FsWrite>) -> Self {
        Self {
            inner: NoopJournal::recording(fs),
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

    /// Forgets the calls recorded so far.
    pub fn clear(&self) {
        if let Some(calls) = &self.inner.calls {
            lock(calls).clear();
        }
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
}

struct NoopTx<'a> {
    journal: &'a NoopJournal,
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

    // The pass-through journal is a journal implementation, which the scope
    // constructor is reserved for.
    #[allow(clippy::disallowed_methods)]
    fn scope(&self) -> TxScope {
        TxScope::new_for_journal(self.op.unwrap_or(OpId(0)), ScopePurpose::Operation)
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

    fn fail(&mut self, error: Error) -> Error {
        self.remove_staged();
        if !self.rolled_back {
            self.rolled_back = true;
            self.journal.record(JournalCall::Rollback {
                op: self.op,
                reason: None,
            });
        }
        error
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

    fn declare(&self, spec: &TargetSpec) -> Result<(Target, Option<VolumeId>)> {
        let TargetPath::Fs(path) = &spec.path else {
            let target = Target {
                spec: spec.clone(),
                before: BeforeState::default(),
                after: None,
                changed: false,
                staged: None,
                undeclared: false,
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
        let volume = self.fs().volume(path).ok().map(|info| info.id);
        let target = Target {
            spec: spec.clone(),
            before,
            after: None,
            changed: false,
            staged: None,
            undeclared: false,
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
            handle.apply_meta(meta).map_err(convert)?;
        }
        handle.sync(Durability::Full).map_err(convert)?;
        drop(handle);
        fs.rename(scope, &staged, path, RenameMode::Replace)
            .map_err(convert)
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
                self.write_content(scope, to, &bytes)?;
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
                self.write_content(&scope, path, &bytes)
            }
            Mutation::Remove => {
                let is_dir = self
                    .meta(path)?
                    .is_some_and(|meta| meta.kind == FileKind::Directory);
                if is_dir {
                    fs.remove_tree(&scope, path, 1).map(|_| ()).map_err(convert)
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
        self.perform(index, path, &spec.action, mutation)
            .map_err(|error| self.fail(error))?;
        let after = self.observe(path).map_err(|error| self.fail(error))?;
        self.applied = true;
        if let Some(entry) = self.targets.get_mut(index) {
            entry.changed = true;
            entry.after = Some(after.clone());
        }
        Ok(Applied {
            index: target,
            after,
        })
    }

    fn outcome(&self, index: usize, target: &Target) -> TargetOutcome {
        let reason = if target.undeclared {
            Irreversible::PreviousContentNotKept
        } else {
            Irreversible::JournalInactive
        };
        TargetOutcome {
            index: TargetIndex(u32::try_from(index).unwrap_or(u32::MAX)),
            path: target.spec.path.clone(),
            action: target.spec.action.clone(),
            mode: PreserveMode::None,
            bytes_held: 0,
            before: target.before.clone(),
            after: target.after.clone(),
            irreversible: Some(reason),
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
                mode: PreserveMode::None,
                bytes_held: 0,
                before: target.before.clone(),
                irreversible: Some(Irreversible::JournalInactive),
                notable: Vec::new(),
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
                    self.targets.push(Target {
                        spec: TargetSpec {
                            path: TargetPath::Fs(change.path.clone()),
                            action: TargetAction::ExternalWrite,
                            expect: Expect::Unchecked,
                            size_hint: None,
                        },
                        before: BeforeState::default(),
                        after: None,
                        changed: false,
                        staged: None,
                        undeclared: true,
                    });
                    self.targets.len() - 1
                }
            };
            let after = self
                .observe(&change.path)
                .map_err(|error| self.fail(error))?;
            self.applied = true;
            if let Some(entry) = self.targets.get_mut(index) {
                entry.changed = true;
                entry.after = Some(after.clone());
            }
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
        let targets: Vec<TargetOutcome> = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.changed || target.spec.action != TargetAction::ExternalWrite
            })
            .map(|(index, target)| self.outcome(index, target))
            .collect();
        let mut irreversible: Vec<IrreversibleEntry> = targets
            .iter()
            .filter_map(|outcome| {
                outcome.irreversible.map(|reason| IrreversibleEntry {
                    reason,
                    path: Some(path_text(&outcome.path)),
                    count: None,
                    detail: None,
                })
            })
            .collect();
        irreversible.extend(self.markers.iter().cloned());
        self.journal.record(JournalCall::Commit { op: self.op });
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
        self.remove_staged();
        if !self.rolled_back {
            self.rolled_back = true;
            self.journal.record(JournalCall::Rollback {
                op: self.op,
                reason: Some(reason),
            });
        }
        Ok(RollbackReport {
            op: self.op,
            restored: Vec::new(),
            failed: self
                .targets
                .iter()
                .filter(|target| target.changed)
                .map(|target| target.spec.path.clone())
                .collect(),
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
