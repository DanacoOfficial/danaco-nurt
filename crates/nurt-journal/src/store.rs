// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal store: construction, shared context and services.
//!
//! [`Journal`] is assembled by [`JournalBuilder`] from the file system, the
//! admission control of the space service, the clock, the volume journal
//! locator and the retention limits. Every module of the crate works on the
//! shared context held by the journal; [`Journal`] implements
//! [`JournalStore`] by delegating to them, and [`EntryCatalog`] for reclaim
//! by the space service.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nurt_contracts::config::OverwriteStrategy;
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, FileKind, FileLock, Follow, FsError, FsResult, FsWrite, ProviderScheme, ProviderWrite,
    ScopePurpose, TxScope, VolumeId, VolumeInfo,
};
use nurt_contracts::journal::{
    Admission, BeginRequest, EffectiveRetention, EntryState, ExpiredEntry, ExpiryReason,
    HistoryQuery, InactiveReason, JournalState, JournalStore, LimitsOutcome, LimitsRequest,
    MutationTx, OpId, OperationDetails, OperationRecord, PinOutcome, PinRequest, PurgeOutcome,
    PurgeRequest, RecoveryReport, RedoRequest, RollbackReason, SessionId, SweepReport, UndoOutcome,
    UndoRequest, VolumeSpace,
};
use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints, NoFailpoints};

use crate::layout::{JournalLayout, StateLayout};
use crate::{catalog, history, pin, recovery, retention, tx, undo};

/// Time to wait for a journal or account lock before the journal is treated
/// as failing for the operation.
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the journal of a volume lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalLocation {
    /// The journal is active in this directory.
    Active(AbsPath),
    /// The journal is inactive on the volume.
    Inactive(InactiveReason),
}

/// Locates the journal directory of each volume.
///
/// Implemented by the volume registry, which chooses the anchor directory,
/// creates the journal directory and records it in the account state.
pub trait VolumeJournals: Send + Sync {
    /// Journal directory of `volume`, or the reason the journal is inactive
    /// there.
    fn locate(&self, volume: &VolumeInfo) -> JournalLocation;

    /// Journal directories of all volumes known to the account with an
    /// active journal.
    fn active(&self) -> Vec<AbsPath>;
}

/// Effective retention limits of each volume.
///
/// Implemented by the retention resolver from the configuration layers.
pub trait RetentionLimits: Send + Sync {
    /// Effective limits of `volume`.
    fn effective(&self, volume: &VolumeInfo) -> EffectiveRetention;

    /// Configuration layers the limits are resolved from, weakest first.
    ///
    /// Expiry passes and `journal.limits` resolve the limits again from these
    /// layers. The default implementation has no layers, so the built-in
    /// limits apply.
    fn layers(&self) -> Vec<nurt_contracts::config::ConfigLayer> {
        Vec::new()
    }

    /// Whether the project configuration level is trusted and may set
    /// retention keys.
    ///
    /// The default implementation does not trust the project level.
    fn project_trusted(&self) -> bool {
        false
    }

    /// User configuration file written by `journal.limits` with
    /// `scope: user`; `None` when the journal has none.
    ///
    /// The default implementation has no file; [`JournalBuilder::user_config`]
    /// sets one for a journal.
    fn user_config(&self) -> Option<AbsPath> {
        None
    }
}

/// Retention limits extended with the user configuration file set by
/// [`JournalBuilder::user_config`].
struct UserConfigLimits {
    limits: Arc<dyn RetentionLimits>,
    path: AbsPath,
}

impl RetentionLimits for UserConfigLimits {
    fn effective(&self, volume: &VolumeInfo) -> EffectiveRetention {
        self.limits.effective(volume)
    }

    fn layers(&self) -> Vec<nurt_contracts::config::ConfigLayer> {
        self.limits.layers()
    }

    fn project_trusted(&self) -> bool {
        self.limits.project_trusted()
    }

    fn user_config(&self) -> Option<AbsPath> {
        Some(self.path.clone())
    }
}

/// Space report of the space service, injected because the journal does not
/// depend on the service implementation.
pub trait SpaceReporter: Send + Sync {
    /// Space report per volume; all known volumes when `volume` is `None`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` for an unknown volume.
    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>>;
}

/// Entry of a volume journal as seen by reclaim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySummary {
    /// Entry number.
    pub op: OpId,
    /// Persistent state.
    pub state: EntryState,
    /// Logical bytes of preserved data still held; zero for data shared with
    /// names outside the journal.
    pub bytes: u64,
    /// Files in the entry directory.
    pub files: u64,
    /// Directories in the entry directory.
    pub directories: u64,
    /// The entry is pinned.
    pub pinned: bool,
    /// Limit the entry is already over (steps or age), when any.
    pub overdue: Option<ExpiryReason>,
}

/// Item of an entry whose preserved data is still held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldItem {
    /// Index of the item in the entry manifest.
    pub index: u32,
    /// File holding the preserved data.
    pub data: AbsPath,
    /// Logical bytes of the data.
    pub bytes: u64,
}

/// Directories of an entry that is being reclaimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimPaths {
    /// Directory of the entry (`ops/<N>`).
    pub entry: AbsPath,
    /// Destination in the reclaim area (`reap/<N>`); must not exist.
    pub reap: AbsPath,
}

/// Access to the journal entries of a volume for reclaim.
///
/// Reclaim of an entry runs under its entry lock, taken with
/// [`EntryCatalog::lock_entry`]; recovery passes over entries whose lock is
/// held by a live process, so the service and command processes never release
/// the same entry twice.
pub trait EntryCatalog: Send + Sync {
    /// Takes the lock of one entry; the entry stays locked until the returned
    /// lock is dropped.
    ///
    /// # Errors
    ///
    /// Fails with `E_IN_USE` when another process holds the lock.
    fn lock_entry(&self, volume: &VolumeId, op: OpId) -> Result<Box<dyn FileLock>>;

    /// Entries of the volume journal that hold data or are being reclaimed,
    /// in any order.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the journal index cannot be read.
    fn entries(&self, volume: &VolumeId) -> Result<Vec<EntrySummary>>;

    /// Items of an entry whose preserved data is still held.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` for an unknown entry and with `E_IO` when
    /// the entry cannot be read.
    fn held_items(&self, volume: &VolumeId, op: OpId) -> Result<Vec<HeldItem>>;

    /// Marks an entry expiring, records the reason for its tombstone and
    /// returns its directories.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` when the entry is gone and with `E_IO`
    /// when its state cannot be written.
    fn begin_reclaim(
        &self,
        volume: &VolumeId,
        op: OpId,
        reason: ExpiryReason,
    ) -> Result<ReclaimPaths>;

    /// Completes the reclaim of an entry moved into the reclaim area: removes
    /// whatever remains of it and writes its tombstone. Idempotent; recovery
    /// uses the same completion.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the remainder cannot be removed or the
    /// tombstone cannot be written.
    fn complete_reclaim(&self, volume: &VolumeId, op: OpId) -> Result<ExpiredEntry>;

    /// Starts a partial release: sets the pending marker of the entry, then
    /// durably marks the items released for space and the entry partial. The
    /// marks are kept beside the entry state, outside the manifest, which
    /// stays unchanged after commit. Data of the items is removed only after
    /// this call returns.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` for an unknown entry and with `E_IO` when
    /// the marker or the marks cannot be written.
    fn mark_released(&self, volume: &VolumeId, op: OpId, items: &[u32]) -> Result<()>;

    /// Ends a partial release after the data of the marked items is removed:
    /// clears the pending marker of the entry.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the marker cannot be removed; recovery then
    /// reconciles the entry.
    fn finish_release(&self, volume: &VolumeId, op: OpId) -> Result<()>;
}

/// Settings of the journal store that are not retention limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    /// Strategy of keeping overwritten files (`journal.overwrite`).
    pub(crate) overwrite: OverwriteStrategy,
    /// Longest wait for a lock.
    pub(crate) lock_timeout: Duration,
    /// Provider schemes whose values are repository references and index
    /// (preserve mode `ref`); every other provider uses `value`.
    pub(crate) reference_schemes: Vec<ProviderScheme>,
}

/// Context shared by all modules of the journal store.
pub(crate) struct JournalCtx {
    /// File system holding the targets and the journal directories.
    pub(crate) fs: Arc<dyn FsWrite>,
    /// Writable providers of path-like resources.
    pub(crate) providers: Vec<Arc<dyn ProviderWrite>>,
    /// Admission control of the space service.
    pub(crate) admission: Arc<dyn Admission>,
    /// Retention limits.
    pub(crate) limits: Arc<dyn RetentionLimits>,
    /// Source of time.
    pub(crate) clock: Arc<dyn Clock>,
    /// Volume journal locator.
    pub(crate) journals: Arc<dyn VolumeJournals>,
    /// Account state directory.
    pub(crate) state: StateLayout,
    /// Fault injection registry; disabled in production builds.
    pub(crate) failpoints: Arc<dyn Failpoints>,
    /// Settings.
    pub(crate) settings: Settings,
    /// Space report of the space service.
    pub(crate) space: Option<Arc<dyn SpaceReporter>>,
    /// Result of the recovery that runs once per process before the first
    /// transaction. A failed recovery degrades preservation of every target
    /// to `none` with the reason `journal_error`; it never fails an operation.
    pub(crate) startup: OnceLock<std::result::Result<RecoveryReport, Error>>,
}

impl JournalCtx {
    /// Layouts of the journal directories of all volumes with an active
    /// journal.
    pub(crate) fn active_journals(&self) -> Vec<JournalLayout> {
        self.journals
            .active()
            .into_iter()
            .map(JournalLayout::new)
            .collect()
    }

    /// Provider serving `scheme`.
    pub(crate) fn provider(&self, scheme: &ProviderScheme) -> Option<&Arc<dyn ProviderWrite>> {
        self.providers
            .iter()
            .find(|provider| &provider.scheme() == scheme)
    }

    /// Passes the fault injection point `point`.
    ///
    /// An injected error is reported exactly as the failure of a system call
    /// at this place; an injected crash stops the caller without cleanup.
    pub(crate) fn hit(&self, point: FailPoint, n: Option<u32>) -> std::result::Result<(), Fault> {
        let n = if point.takes_index() { n } else { None };
        match self.failpoints.hit(point, n) {
            FailAction::Continue => Ok(()),
            FailAction::Error(kind) => Err(Fault::Fs(FsError::new(kind, point.as_str(), None))),
            FailAction::Crash => Err(Fault::Halt),
        }
    }

    /// Takes the account lock that serialises the sequence and the account
    /// index across processes.
    pub(crate) fn lock_account(&self) -> FsResult<Box<dyn FileLock>> {
        self.fs
            .lock_exclusive(&self.state.account_lock()?, self.settings.lock_timeout)
    }
}

/// Failure of a journal step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fault {
    /// A file system call failed, or an error was injected at a fault point.
    Fs(FsError),
    /// A crash was injected; the caller stops without cleanup.
    Halt,
}

impl From<FsError> for Fault {
    fn from(error: FsError) -> Self {
        Self::Fs(error)
    }
}

/// Creates a transaction scope; the journal store is the only creator of
/// scopes.
#[expect(
    clippy::disallowed_methods,
    reason = "the journal store is the component entitled to create scopes"
)]
pub(crate) fn scope(op: OpId, purpose: ScopePurpose) -> TxScope {
    TxScope::new_for_journal(op, purpose)
}

/// Builder of a [`Journal`].
pub struct JournalBuilder {
    ctx: JournalCtx,
}

impl JournalBuilder {
    /// Builder with the required services.
    ///
    /// `state_dir` is the account state directory (`seq`, `ops.jsonl`); it
    /// must exist. Defaults: overwrite strategy `exchange`, lock timeout
    /// [`DEFAULT_LOCK_TIMEOUT`], no providers, no reference schemes, no space
    /// reporter, no user configuration file.
    #[must_use]
    pub fn new(
        fs: Arc<dyn FsWrite>,
        admission: Arc<dyn Admission>,
        clock: Arc<dyn Clock>,
        journals: Arc<dyn VolumeJournals>,
        limits: Arc<dyn RetentionLimits>,
        state_dir: AbsPath,
    ) -> Self {
        let clock = Arc::new(retention::age::EffectiveClock::new(
            clock,
            fs.clone(),
            &state_dir,
        ));
        Self {
            ctx: JournalCtx {
                fs,
                providers: Vec::new(),
                admission,
                limits,
                clock,
                journals,
                state: StateLayout::new(state_dir),
                failpoints: Arc::new(NoFailpoints),
                settings: Settings {
                    overwrite: OverwriteStrategy::default(),
                    lock_timeout: DEFAULT_LOCK_TIMEOUT,
                    reference_schemes: Vec::new(),
                },
                space: None,
                startup: OnceLock::new(),
            },
        }
    }

    /// Adds a writable provider.
    #[must_use]
    pub fn provider(mut self, provider: Arc<dyn ProviderWrite>) -> Self {
        self.ctx.providers.push(provider);
        self
    }

    /// Sets the strategy of keeping overwritten files.
    #[must_use]
    pub fn overwrite(mut self, strategy: OverwriteStrategy) -> Self {
        self.ctx.settings.overwrite = strategy;
        self
    }

    /// Sets the longest wait for a lock.
    #[must_use]
    pub fn lock_timeout(mut self, timeout: Duration) -> Self {
        self.ctx.settings.lock_timeout = timeout;
        self
    }

    /// Names the provider schemes that hold repository references and index;
    /// their targets are preserved with mode `ref`.
    #[must_use]
    pub fn reference_schemes(mut self, schemes: Vec<ProviderScheme>) -> Self {
        self.ctx.settings.reference_schemes = schemes;
        self
    }

    /// Sets the space reporter of the space service.
    #[must_use]
    pub fn space_reporter(mut self, reporter: Arc<dyn SpaceReporter>) -> Self {
        self.ctx.space = Some(reporter);
        self
    }

    /// Sets the user configuration file that `journal.limits` with
    /// `scope: user` writes.
    ///
    /// The file is written through a mutation transaction of kind
    /// `journal.limits` that does not count as a step, so the change can be
    /// undone; missing parent directories are created in the same
    /// transaction. The path is not checked here.
    ///
    /// # Errors
    ///
    /// Setting the path does not fail. Without it, [`JournalStore::limits`]
    /// with `scope: user` and new values fails with `E_UNSUPPORTED`; with it,
    /// the call fails with `E_CONFIG` when the file is not valid TOML or
    /// UTF-8 and with `E_IO` when the file cannot be written.
    #[must_use]
    pub fn user_config(mut self, path: AbsPath) -> Self {
        self.ctx.limits = Arc::new(UserConfigLimits {
            limits: self.ctx.limits.clone(),
            path,
        });
        self
    }

    /// Sets the fault injection registry.
    #[cfg(feature = "failpoints")]
    #[must_use]
    pub fn failpoints(mut self, failpoints: Arc<dyn Failpoints>) -> Self {
        self.ctx.failpoints = failpoints;
        self
    }

    /// Builds the journal.
    ///
    /// No journal file is read or written here; crash recovery runs before
    /// the first transaction or when [`JournalStore::recover`] is called.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the account state directory does not
    /// exist and with `E_IO` when it is not a directory or cannot be
    /// inspected.
    pub fn build(self) -> Result<Journal> {
        let state_dir = self.ctx.state.root();
        let meta = self.ctx.fs.metadata(state_dir, Follow::Follow)?;
        if meta.kind != FileKind::Directory {
            return Err(Error::new(
                ErrorCode::Io,
                format!(
                    "the journal state path {state_dir} is not a directory; remove it or choose another state directory"
                ),
            ));
        }
        Ok(Journal { ctx: self.ctx })
    }
}

/// Operation journal of an account.
///
/// Assembled by [`JournalBuilder`] from injected services: the file system
/// ([`FsWrite`]), admission control of the space service ([`Admission`]), the
/// clock, the volume journal locator ([`VolumeJournals`], implemented by
/// [`JournalDirectories`](crate::volumes::JournalDirectories)), the retention
/// limits ([`RetentionLimits`], implemented by
/// [`ConfiguredLimits`](crate::ConfiguredLimits)) and optionally the space
/// report of the space service ([`SpaceReporter`]) and, with the feature
/// `failpoints`, a fault injection registry.
///
/// Crash recovery runs once per process before the first transaction, undo
/// or redo; when it fails, every target of the process is changed without
/// preservation and carries the irreversible reason `journal_error`. That
/// recovery does not expire entries: the expiry pass reads every entry, so
/// overdue entries expire in [`JournalStore::recover`], called by the space
/// service at start, and in [`JournalStore::sweep`].
pub struct Journal {
    ctx: JournalCtx,
}

impl Journal {
    /// Removes the session limits set by `journal.limits` with
    /// `scope: session` for `session`.
    ///
    /// The host calls it when the session ends; without the call the limits
    /// are removed after the session time to live passes without calls.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the session file cannot be removed.
    pub fn end_session(&self, session: &SessionId) -> Result<()> {
        retention::limits::end_session(&self.ctx, session)
    }

    /// Runs crash recovery once per process; the result is kept for every
    /// later transaction, which degrades to `journal_error` after a failed
    /// recovery. No expiry pass runs here, so the time before the first
    /// transaction depends only on the entries left by an interruption.
    fn ensure_recovered(&self) {
        // A failed recovery is recorded in the context and reported by every
        // transaction as `journal_error`; the operation itself proceeds.
        let _ = recovery::ensure_recovered(&self.ctx);
    }
}

impl JournalStore for Journal {
    /// Completes or reverts interrupted entries, then expires entries over
    /// the retention limits.
    ///
    /// Recovery runs first and reads only entries that are not terminal;
    /// `elapsed_ms` covers recovery alone. The expiry pass runs after it as a
    /// separate step, reads every entry and lists the entries it expired in
    /// `expired`. The first call of the process also serves as the recovery
    /// before the first transaction.
    ///
    /// # Errors
    ///
    /// Fails with the error of recovery (`E_IO`); no entry is then expired.
    /// Fails with the error of the expiry pass (`E_CONFIG`, `E_IN_USE`,
    /// `E_IO`) after a successful recovery; the error then carries the
    /// recovery report in the detail `recovered`.
    fn recover(&self) -> Result<RecoveryReport> {
        let recovered = recovery::recover(&self.ctx);
        // The first pass of the process also serves as the recovery before
        // the first transaction.
        let _ = self.ctx.startup.set(recovered.clone());
        let mut report = recovered?;
        let swept = retention::expiry::sweep(&self.ctx).map_err(|error| {
            error.with_detail(
                "recovered",
                serde_json::to_value(&report).unwrap_or_default(),
            )
        })?;
        report.expired = swept.expired.into_iter().map(|entry| entry.op).collect();
        Ok(report)
    }

    fn begin(&self, request: BeginRequest) -> Box<dyn MutationTx + '_> {
        self.ensure_recovered();
        tx::begin(&self.ctx, request)
    }

    fn rollback_committed(&self, op: OpId, reason: RollbackReason) -> Result<UndoOutcome> {
        self.ensure_recovered();
        undo::rollback_committed(&self.ctx, op, reason)
    }

    fn history(&self, query: &HistoryQuery) -> Result<Vec<OperationRecord>> {
        history::history(&self.ctx, query)
    }

    fn details(&self, op: OpId) -> Result<OperationDetails> {
        history::details(&self.ctx, op)
    }

    fn undo(&self, request: &UndoRequest) -> Result<UndoOutcome> {
        self.ensure_recovered();
        undo::undo(&self.ctx, request)
    }

    fn redo(&self, request: &RedoRequest) -> Result<UndoOutcome> {
        self.ensure_recovered();
        undo::redo(&self.ctx, request)
    }

    fn pin(&self, request: &PinRequest) -> Result<PinOutcome> {
        pin::pin(&self.ctx, request)
    }

    fn purge(&self, request: &PurgeRequest) -> Result<PurgeOutcome> {
        retention::expiry::purge(&self.ctx, request)
    }

    fn status(&self, volume: Option<&AbsPath>) -> Result<Vec<JournalState>> {
        retention::limits::status(&self.ctx, volume)
    }

    fn limits(&self, request: &LimitsRequest) -> Result<LimitsOutcome> {
        retention::limits::limits(&self.ctx, request)
    }

    /// Space report of the injected [`SpaceReporter`].
    ///
    /// # Errors
    ///
    /// Fails with `E_SERVICE_NOT_RUNNING` when no space reporter is
    /// configured, otherwise with the error of the reporter (`E_NOT_FOUND`
    /// for an unknown volume).
    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>> {
        match &self.ctx.space {
            Some(reporter) => reporter.space(volume),
            None => Err(Error::new(
                ErrorCode::ServiceNotRunning,
                "the space service is not connected to the journal; start the space service",
            )),
        }
    }

    fn sweep(&self) -> Result<SweepReport> {
        retention::expiry::sweep(&self.ctx)
    }
}

impl EntryCatalog for Journal {
    fn lock_entry(&self, volume: &VolumeId, op: OpId) -> Result<Box<dyn FileLock>> {
        catalog::lock_entry(&self.ctx, volume, op)
    }

    fn entries(&self, volume: &VolumeId) -> Result<Vec<EntrySummary>> {
        catalog::entries(&self.ctx, volume)
    }

    fn held_items(&self, volume: &VolumeId, op: OpId) -> Result<Vec<HeldItem>> {
        catalog::held_items(&self.ctx, volume, op)
    }

    fn begin_reclaim(
        &self,
        volume: &VolumeId,
        op: OpId,
        reason: ExpiryReason,
    ) -> Result<ReclaimPaths> {
        catalog::begin_reclaim(&self.ctx, volume, op, reason)
    }

    fn complete_reclaim(&self, volume: &VolumeId, op: OpId) -> Result<ExpiredEntry> {
        catalog::complete_reclaim(&self.ctx, volume, op)
    }

    fn mark_released(&self, volume: &VolumeId, op: OpId, items: &[u32]) -> Result<()> {
        catalog::mark_released(&self.ctx, volume, op, items)
    }

    fn finish_release(&self, volume: &VolumeId, op: OpId) -> Result<()> {
        catalog::finish_release(&self.ctx, volume, op)
    }
}

#[cfg(test)]
mod tests;

/// File system and services for tests of the journal store.
///
/// [`DiskFs`](test_support::DiskFs) implements the file system interface on
/// a temporary directory: two volumes (`vol-a`, `vol-b`) with configurable
/// features, a log of every mutating call, injected faults, counters of
/// copied and cloned bytes, simulated special files, refused metadata
/// elements and account identifiers of files.
#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// `path` in the language form of [`AbsPath`] (`/` separators), the form
    /// in which the tests name paths in the log and in fault needles.
    fn language(path: &Path) -> String {
        AbsPath::new(path).map_or_else(
            |_| path.to_string_lossy().into_owned(),
            |path| path.to_language_string(),
        )
    }

    /// On Windows the identity, the link count and the attributes of
    /// `path` from the system back end, which `std::fs::Metadata` does not
    /// report; elsewhere `meta` unchanged.
    #[cfg(windows)]
    fn system_meta(path: &Path, follow: Follow, meta: FileMeta) -> FileMeta {
        let Ok(path) = AbsPath::new(path) else {
            return meta;
        };
        match nurt_fs::system_fs().metadata(&path, follow) {
            Ok(system) => FileMeta {
                links: system.links,
                file_id: system.file_id,
                windows_attributes: system.windows_attributes,
                ..meta
            },
            Err(_) => meta,
        }
    }

    #[cfg(not(windows))]
    fn system_meta(_path: &Path, _follow: Follow, meta: FileMeta) -> FileMeta {
        meta
    }

    /// Runs `op` again after a sharing violation on Windows, where a virus
    /// scanner or an indexer keeps a new file open for a moment, with the
    /// schedule of the system back end.
    fn shared<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        #[cfg(windows)]
        {
            let policy = nurt_fs::windows::retry::RetryPolicy::new(
                nurt_fs::windows::retry::DEFAULT_SHARING_RETRY,
            );
            for delay in policy.delays() {
                match op() {
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                        std::thread::sleep(delay);
                    }
                    other => return other,
                }
            }
        }
        op()
    }

    /// Identity of the file at `path`: the inode number on Unix, the file
    /// index of the system back end on Windows.
    #[cfg(unix)]
    fn path_ino(path: &Path) -> u64 {
        file_ino(&std::fs::metadata(path).unwrap())
    }

    /// Identity of the file at `path`: the inode number on Unix, the file
    /// index of the system back end on Windows.
    #[cfg(not(unix))]
    fn path_ino(path: &Path) -> u64 {
        let path = AbsPath::new(path).unwrap();
        nurt_fs::system_fs()
            .metadata(&path, Follow::Follow)
            .map_or(0, |meta| meta.file_id.low)
    }

    /// Returns the inode number on Unix and zero elsewhere.
    #[cfg(unix)]
    pub(crate) fn file_ino(meta: &std::fs::Metadata) -> u64 {
        std::os::unix::fs::MetadataExt::ino(meta)
    }

    /// Returns the inode number on Unix and zero elsewhere.
    #[cfg(not(unix))]
    pub(crate) fn file_ino(_meta: &std::fs::Metadata) -> u64 {
        0
    }

    #[cfg(unix)]
    fn meta_mode(meta: &std::fs::Metadata) -> u32 {
        std::os::unix::fs::MetadataExt::mode(meta) & 0o7777
    }

    #[cfg(not(unix))]
    fn meta_mode(meta: &std::fs::Metadata) -> u32 {
        if meta.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }

    #[cfg(unix)]
    fn meta_owner(meta: &std::fs::Metadata) -> Option<(u32, u32)> {
        Some((
            std::os::unix::fs::MetadataExt::uid(meta),
            std::os::unix::fs::MetadataExt::gid(meta),
        ))
    }

    #[cfg(not(unix))]
    fn meta_owner(_meta: &std::fs::Metadata) -> Option<(u32, u32)> {
        None
    }

    #[cfg(unix)]
    fn meta_links(meta: &std::fs::Metadata) -> u64 {
        std::os::unix::fs::MetadataExt::nlink(meta)
    }

    #[cfg(not(unix))]
    fn meta_links(_meta: &std::fs::Metadata) -> u64 {
        1
    }

    /// Nanoseconds since the Unix epoch, saturated to the range of `i64` as
    /// the back ends of `nurt-fs` do.
    #[cfg(unix)]
    fn meta_mtime_ns(meta: &std::fs::Metadata) -> i64 {
        std::os::unix::fs::MetadataExt::mtime(meta)
            .saturating_mul(1_000_000_000)
            .saturating_add(std::os::unix::fs::MetadataExt::mtime_nsec(meta))
    }

    #[cfg(all(test, unix))]
    #[test]
    fn modification_time_after_the_year_2262_saturates() {
        let dir = tempfile::tempdir().expect("directory");
        let path = dir.path().join("late");
        let file = std::fs::File::create(&path).expect("file");
        let late = std::time::UNIX_EPOCH + std::time::Duration::from_secs(10_000_000_000);
        if file.set_modified(late).is_err() {
            return;
        }
        let meta = std::fs::metadata(&path).expect("metadata");
        assert_eq!(meta_mtime_ns(&meta), i64::MAX);
    }

    #[cfg(not(unix))]
    fn meta_mtime_ns(meta: &std::fs::Metadata) -> i64 {
        meta.modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
            .unwrap_or(0)
    }

    #[cfg(unix)]
    fn meta_dev(meta: &std::fs::Metadata) -> u64 {
        std::os::unix::fs::MetadataExt::dev(meta)
    }

    #[cfg(not(unix))]
    fn meta_dev(_meta: &std::fs::Metadata) -> u64 {
        0
    }

    /// Sets the permission bits of `path` on Unix and the read-only flag elsewhere.
    #[cfg(unix)]
    pub(crate) fn set_path_mode(path: &Path, mode: u32) -> io::Result<()> {
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))
    }

    /// Sets the permission bits of `path` on Unix and the read-only flag elsewhere.
    #[cfg(not(unix))]
    pub(crate) fn set_path_mode(path: &Path, mode: u32) -> io::Result<()> {
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_readonly(mode & 0o200 == 0);
        std::fs::set_permissions(path, permissions)
    }

    /// Creates a symbolic link at `link` pointing to `target`.
    #[cfg(unix)]
    pub(crate) fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    /// Creates a symbolic link at `link` pointing to `target`.
    #[cfg(not(unix))]
    pub(crate) fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    use nurt_contracts::config::{
        AgeClock, AgeLimit, ByteLimit, ConfigLevel, Oversize, SizeLimit, StepLimit,
    };
    use nurt_contracts::fs::{
        AbsPath, ContentHash, DirEntry, Durability, Etag, ExtendedMeta, FileId, FileKind, FileLock,
        FileMeta, Follow, Fs, FsError, FsErrorKind, FsFeatures, FsResult, FsWrite, Holder,
        LinkKind, MetaChange, MetaTransferReport, PathKey, Provider, ProviderCaps, ProviderChild,
        ProviderKey, ProviderScheme, ProviderSnapshot, ProviderValue, ProviderWrite, ReadHandle,
        RemoveStats, RenameMode, SpaceInfo, TxScope, VolumeId, VolumeInfo, VolumeKind, WriteHandle,
    };
    use nurt_contracts::journal::{
        Admission, AdmissionDecision, AdmissionRequest, EffectiveRetention, Eligibility,
        ServiceLiveness, Sourced, Thresholds, VolumeState, VolumeStatus,
    };
    use nurt_fs::clock::VirtualClock;
    use sha2::{Digest, Sha256};

    use super::{
        JournalBuilder, JournalCtx, JournalLocation, RetentionLimits, Settings, VolumeJournals,
    };
    use crate::layout::{self, StateLayout};

    /// Names of the two test volumes.
    pub(crate) const VOLUMES: [&str; 2] = ["vol-a", "vol-b"];

    /// One injected fault.
    #[derive(Debug, Clone)]
    struct Fault {
        op: &'static str,
        needle: String,
        kind: FsErrorKind,
        persistent: bool,
    }

    #[derive(Default)]
    struct State {
        log: Vec<String>,
        faults: Vec<Fault>,
        copied: u64,
        cloned: u64,
        special: HashSet<PathBuf>,
        refused_meta: Vec<String>,
        sids: HashMap<u64, String>,
        transfer_sid: bool,
        cross_device: Vec<PathBuf>,
        locks: HashSet<PathBuf>,
        writes_after_exchange: Vec<(PathBuf, Vec<u8>)>,
    }

    /// File system on a temporary directory.
    pub(crate) struct DiskFs {
        root: tempfile::TempDir,
        volumes: Mutex<Vec<(PathBuf, VolumeInfo)>>,
        state: Mutex<State>,
        released: Condvar,
    }

    fn io_kind(error: &io::Error) -> FsErrorKind {
        match error.kind() {
            io::ErrorKind::NotFound => FsErrorKind::NotFound,
            io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
            io::ErrorKind::AlreadyExists => FsErrorKind::Exists,
            io::ErrorKind::DirectoryNotEmpty => FsErrorKind::NotEmpty,
            io::ErrorKind::NotADirectory => FsErrorKind::NotADirectory,
            io::ErrorKind::IsADirectory => FsErrorKind::IsADirectory,
            io::ErrorKind::StorageFull => FsErrorKind::NoSpace,
            io::ErrorKind::CrossesDevices => FsErrorKind::CrossDevice,
            _ => FsErrorKind::Io,
        }
    }

    fn fs_error(op: &str, path: &Path, error: &io::Error) -> FsError {
        FsError::new(io_kind(error), op, Some(path))
    }

    fn volume_info(dev: u64, mount: &Path) -> VolumeInfo {
        VolumeInfo {
            id: VolumeId {
                dev,
                uuid: Some(format!("uuid-{dev}")),
                subvolume: 0,
            },
            mount: AbsPath::new(mount).unwrap(),
            fs_type: "testfs".to_owned(),
            kind: VolumeKind::Fixed,
            features: FsFeatures {
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
            },
            rotational: false,
            read_only: false,
        }
    }

    impl DiskFs {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let mut volumes = Vec::new();
            for (dev, name) in (1..).zip(VOLUMES) {
                let mount = root.path().join(name);
                std::fs::create_dir(&mount).unwrap();
                volumes.push((mount.clone(), volume_info(dev, &mount)));
            }
            Self {
                root,
                volumes: Mutex::new(volumes),
                state: Mutex::new(State::default()),
                released: Condvar::new(),
            }
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        /// Absolute path below the temporary root.
        pub(crate) fn path(&self, relative: &str) -> AbsPath {
            AbsPath::new(self.root.path().join(relative)).unwrap()
        }

        /// Mount point of test volume `index`.
        pub(crate) fn mount(&self, index: usize) -> AbsPath {
            self.path(VOLUMES[index])
        }

        /// Description of test volume `index`.
        pub(crate) fn volume_at(&self, index: usize) -> VolumeInfo {
            self.volumes.lock().unwrap()[index].1.clone()
        }

        /// Changes the description of test volume `index`.
        pub(crate) fn configure(&self, index: usize, change: impl FnOnce(&mut VolumeInfo)) {
            change(&mut self.volumes.lock().unwrap()[index].1);
        }

        /// Mutating calls in order, as `"<op> <path>[ <path>]"`.
        pub(crate) fn log(&self) -> Vec<String> {
            self.state().log.clone()
        }

        /// Clears the call log.
        pub(crate) fn clear_log(&self) {
            self.state().log.clear();
        }

        /// Bytes copied by `copy_file`.
        pub(crate) fn copied(&self) -> u64 {
            self.state().copied
        }

        /// Bytes cloned by `clone_file`.
        pub(crate) fn cloned(&self) -> u64 {
            self.state().cloned
        }

        /// Fails the next call `op` on a path containing `needle`.
        pub(crate) fn fail(&self, op: &'static str, needle: &str, kind: FsErrorKind) {
            self.state().faults.push(Fault {
                op,
                needle: needle.to_owned(),
                kind,
                persistent: false,
            });
        }

        /// Fails every call `op` on a path containing `needle`.
        pub(crate) fn fail_always(&self, op: &'static str, needle: &str, kind: FsErrorKind) {
            self.state().faults.push(Fault {
                op,
                needle: needle.to_owned(),
                kind,
                persistent: true,
            });
        }

        /// Removes every injected fault.
        pub(crate) fn clear_faults(&self) {
            self.state().faults.clear();
        }

        /// Reports `path` as a special file.
        pub(crate) fn mark_special(&self, path: &AbsPath) {
            self.state().special.insert(path.as_path().to_path_buf());
        }

        /// Elements every `apply_meta` reports as not transferred.
        pub(crate) fn refuse_meta(&self, elements: &[&str]) {
            self.state().refused_meta = elements.iter().map(|e| (*e).to_owned()).collect();
        }

        /// Assigns an account identifier to the file at `path`.
        pub(crate) fn set_sid(&self, path: &AbsPath, sid: &str) {
            let ino = path_ino(path.as_path());
            self.state().sids.insert(ino, sid.to_owned());
        }

        /// Whether `apply_meta` transfers the account identifier.
        pub(crate) fn transfer_sid(&self, transfer: bool) {
            self.state().transfer_sid = transfer;
        }

        /// Renames of paths below `path` fail with a cross-device error, as
        /// for directories of a lower overlay layer.
        pub(crate) fn lower_layer(&self, path: &AbsPath) {
            self.state().cross_device.push(path.as_path().to_path_buf());
        }

        /// Appends `bytes` to the file at `path` through a handle opened
        /// before the next exchange of names, as another process would.
        pub(crate) fn write_after_exchange(&self, path: &AbsPath, bytes: &[u8]) {
            self.state()
                .writes_after_exchange
                .push((path.as_path().to_path_buf(), bytes.to_vec()));
        }

        fn record(&self, op: &'static str, paths: &[&Path]) -> FsResult<()> {
            let mut state = self.state();
            let mut entry = op.to_owned();
            for path in paths {
                entry.push(' ');
                entry.push_str(&language(path));
            }
            state.log.push(entry);
            let position = state.faults.iter().position(|fault| {
                fault.op == op
                    && paths
                        .iter()
                        .any(|path| language(path).contains(&fault.needle))
            });
            if let Some(position) = position {
                let fault = state.faults[position].clone();
                if !fault.persistent {
                    state.faults.remove(position);
                }
                return Err(FsError::new(fault.kind, op, paths.first().copied()));
            }
            Ok(())
        }

        fn check(&self, op: &'static str, path: &Path) -> FsResult<()> {
            let state = self.state();
            let position = state
                .faults
                .iter()
                .position(|fault| fault.op == op && path.to_string_lossy().contains(&fault.needle));
            drop(state);
            if let Some(position) = position {
                let mut state = self.state();
                let fault = state.faults[position].clone();
                if !fault.persistent {
                    state.faults.remove(position);
                }
                return Err(FsError::new(fault.kind, op, Some(path)));
            }
            Ok(())
        }

        fn volume_of(&self, path: &Path) -> Option<VolumeInfo> {
            self.volumes
                .lock()
                .unwrap()
                .iter()
                .filter(|(mount, _)| path.starts_with(mount))
                .max_by_key(|(mount, _)| mount.as_os_str().len())
                .map(|(_, info)| info.clone())
        }

        fn same_volume_or_cross(&self, from: &Path, to: &Path) -> FsResult<()> {
            let lower = self
                .state()
                .cross_device
                .iter()
                .any(|lower| from.starts_with(lower));
            let differ = self.volume_of(from).map(|v| v.id) != self.volume_of(to).map(|v| v.id);
            if lower || differ {
                return Err(FsError::new(FsErrorKind::CrossDevice, "rename", Some(from)));
            }
            Ok(())
        }

        fn meta_of(&self, path: &Path, follow: Follow) -> FsResult<FileMeta> {
            let meta = match follow {
                Follow::Follow => std::fs::metadata(path),
                Follow::NoFollow => std::fs::symlink_metadata(path),
            }
            .map_err(|error| fs_error("metadata", path, &error))?;
            let kind = if self.state().special.contains(path) {
                FileKind::Special
            } else if meta.file_type().is_symlink() {
                FileKind::Symlink
            } else if meta.is_dir() {
                FileKind::Directory
            } else if meta.is_file() {
                FileKind::File
            } else {
                FileKind::Special
            };
            let meta = FileMeta {
                kind,
                size: meta.len(),
                mode: meta_mode(&meta),
                uid: meta_owner(&meta).map(|(uid, _)| uid),
                gid: meta_owner(&meta).map(|(_, gid)| gid),
                links: meta_links(&meta),
                modified_ns: meta_mtime_ns(&meta),
                accessed_ns: None,
                created_ns: None,
                changed_ns: None,
                file_id: FileId {
                    volume: meta_dev(&meta),
                    low: file_ino(&meta),
                    high: 0,
                },
                windows_attributes: 0,
            };
            Ok(system_meta(path, follow, meta))
        }

        /// Applies writes registered for the file that was at `registered`
        /// before an exchange and is now at `now_at`.
        fn apply_pending_writes(&self, registered: &Path, now_at: &Path) {
            let pending: Vec<Vec<u8>> = {
                let mut state = self.state();
                let (matching, rest) = std::mem::take(&mut state.writes_after_exchange)
                    .into_iter()
                    .partition(|(target, _)| target == registered);
                state.writes_after_exchange = rest;
                matching.into_iter().map(|(_, bytes)| bytes).collect()
            };
            for bytes in pending {
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(now_at)
                    .unwrap();
                file.write_all(&bytes).unwrap();
            }
        }
    }

    struct DiskHandle {
        path: AbsPath,
        file: std::fs::File,
        fs: Arc<DiskFs>,
    }

    impl Write for DiskHandle {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.fs
                .check("write", self.path.as_path())
                .map_err(|error| match error.kind {
                    FsErrorKind::NoSpace => io::Error::from(io::ErrorKind::StorageFull),
                    _ => io::Error::other("injected"),
                })?;
            self.file.write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    impl Seek for DiskHandle {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.file.seek(position)
        }
    }

    impl WriteHandle for DiskHandle {
        fn path(&self) -> &AbsPath {
            &self.path
        }

        fn sync(&mut self, _durability: Durability) -> FsResult<()> {
            self.fs.record("sync_file", &[self.path.as_path()])
        }

        fn set_length(&mut self, length: u64) -> FsResult<()> {
            self.fs.record("set_length", &[self.path.as_path()])?;
            self.file
                .set_len(length)
                .map_err(|error| fs_error("set_length", self.path.as_path(), &error))
        }

        fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
            self.fs.record("apply_meta", &[self.path.as_path()])?;
            set_path_mode(self.path.as_path(), meta.basic.mode)
                .map_err(|error| fs_error("apply_meta", self.path.as_path(), &error))?;
            if let Ok(nanos) = u64::try_from(meta.basic.modified_ns) {
                self.file
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_nanos(nanos))
                    .map_err(|error| fs_error("apply_meta", self.path.as_path(), &error))?;
            }
            let ino = path_ino(self.path.as_path());
            let mut state = self.fs.state();
            if state.transfer_sid
                && let Some(sid) = &meta.account_sid
            {
                state.sids.insert(ino, sid.clone());
            }
            Ok(MetaTransferReport {
                not_transferred: state.refused_meta.clone(),
            })
        }
    }

    struct DiskRead {
        file: std::fs::File,
        meta: FileMeta,
    }

    impl Read for DiskRead {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.file.read(buffer)
        }
    }

    impl Seek for DiskRead {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.file.seek(position)
        }
    }

    impl ReadHandle for DiskRead {
        fn meta(&self) -> FsResult<FileMeta> {
            Ok(self.meta.clone())
        }
    }

    struct DiskLock {
        path: AbsPath,
        fs: Arc<DiskFs>,
    }

    impl FileLock for DiskLock {
        fn path(&self) -> &AbsPath {
            &self.path
        }
    }

    impl Drop for DiskLock {
        fn drop(&mut self) {
            self.fs.state().locks.remove(self.path.as_path());
            self.fs.released.notify_all();
        }
    }

    /// Shared handle to the test file system, implementing the interface.
    #[derive(Clone)]
    pub(crate) struct SharedFs(pub(crate) Arc<DiskFs>);

    impl std::ops::Deref for SharedFs {
        type Target = DiskFs;

        fn deref(&self) -> &DiskFs {
            &self.0
        }
    }

    impl Fs for SharedFs {
        fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
            self.check("metadata", path.as_path())?;
            self.meta_of(path.as_path(), follow)
        }

        fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
            self.check("extended_metadata", path.as_path())?;
            let basic = self.meta_of(path.as_path(), follow)?;
            let link_target = if basic.kind == FileKind::Symlink {
                std::fs::read_link(path.as_path()).ok()
            } else {
                None
            };
            let account_sid = self.state().sids.get(&basic.file_id.low).cloned();
            Ok(ExtendedMeta {
                basic,
                xattrs: Vec::new(),
                acl: None,
                account_sid,
                flags: None,
                streams: Vec::new(),
                link_target,
            })
        }

        fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
            self.check("open_read", path.as_path())?;
            let meta = self.meta_of(path.as_path(), Follow::Follow)?;
            let file = std::fs::File::open(path.as_path())
                .map_err(|error| fs_error("open_read", path.as_path(), &error))?;
            Ok(Box::new(DiskRead { file, meta }))
        }

        fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
            Err(FsError::new(
                FsErrorKind::Unsupported,
                "open_stream_read",
                Some(path.as_path()),
            ))
        }

        fn read_dir<'a>(
            &'a self,
            path: &AbsPath,
        ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
            self.check("read_dir", path.as_path())?;
            let mut names: Vec<OsString> = std::fs::read_dir(path.as_path())
                .map_err(|error| fs_error("read_dir", path.as_path(), &error))?
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            Ok(Box::new(
                names
                    .into_iter()
                    .map(|name| Ok(DirEntry { name, kind: None })),
            ))
        }

        fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
            std::fs::read_link(path.as_path())
                .map_err(|error| fs_error("read_link", path.as_path(), &error))
        }

        fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
            let real = std::fs::canonicalize(path.as_path())
                .map_err(|error| fs_error("real_path", path.as_path(), &error))?;
            AbsPath::new(real)
        }

        fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
            self.check("content_hash", path.as_path())?;
            let bytes = std::fs::read(path.as_path())
                .map_err(|error| fs_error("content_hash", path.as_path(), &error))?;
            Ok(ContentHash(Sha256::digest(&bytes).into()))
        }

        fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
            self.check("volume", path.as_path())?;
            self.volume_of(path.as_path())
                .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "volume", Some(path.as_path())))
        }

        fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
            Ok(SpaceInfo {
                total: 1 << 40,
                available: 1 << 39,
                quota_remaining: None,
                quota_limit: None,
                purgeable_system: None,
            })
        }

        fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
            Ok(PathKey(
                path.as_path().to_string_lossy().as_bytes().to_vec(),
            ))
        }

        fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
            Ok(Vec::new())
        }

        fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
            Ok(true)
        }

        fn etag(&self, path: &AbsPath, follow: Follow) -> FsResult<Etag> {
            self.metadata(path, follow).map(|meta| meta.etag())
        }
    }

    impl FsWrite for SharedFs {
        fn create_temp_in(
            &self,
            scope: &TxScope,
            dir: &AbsPath,
            name_hint: &str,
        ) -> FsResult<Box<dyn WriteHandle>> {
            for attempt in 0_u32.. {
                let path = dir.join_name(&format!("{name_hint}.{attempt}"))?;
                if std::fs::symlink_metadata(path.as_path()).is_ok() {
                    continue;
                }
                return self.create_new(scope, &path);
            }
            unreachable!("unbounded attempts")
        }

        fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
            self.record("create_new", &[path.as_path()])?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path.as_path())
                .map_err(|error| fs_error("create_new", path.as_path(), &error))?;
            Ok(Box::new(DiskHandle {
                path: path.clone(),
                file,
                fs: self.0.clone(),
            }))
        }

        fn open_in_place(
            &self,
            _scope: &TxScope,
            path: &AbsPath,
        ) -> FsResult<Box<dyn WriteHandle>> {
            self.record("open_in_place", &[path.as_path()])?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path.as_path())
                .map_err(|error| fs_error("open_in_place", path.as_path(), &error))?;
            Ok(Box::new(DiskHandle {
                path: path.clone(),
                file,
                fs: self.0.clone(),
            }))
        }

        fn open_stream_write(
            &self,
            _scope: &TxScope,
            path: &AbsPath,
            _stream: &str,
        ) -> FsResult<Box<dyn WriteHandle>> {
            Err(FsError::new(
                FsErrorKind::Unsupported,
                "open_stream_write",
                Some(path.as_path()),
            ))
        }

        fn rename(
            &self,
            _scope: &TxScope,
            from: &AbsPath,
            to: &AbsPath,
            mode: RenameMode,
        ) -> FsResult<()> {
            self.record("rename", &[from.as_path(), to.as_path()])?;
            self.same_volume_or_cross(from.as_path(), to.as_path())?;
            if mode == RenameMode::NoReplace && std::fs::symlink_metadata(to.as_path()).is_ok() {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "rename",
                    Some(to.as_path()),
                ));
            }
            shared(|| std::fs::rename(from.as_path(), to.as_path()))
                .map_err(|error| fs_error("rename", from.as_path(), &error))
        }

        fn exchange(&self, _scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
            self.record("exchange", &[a.as_path(), b.as_path()])?;
            let supported = self
                .volume_of(a.as_path())
                .is_some_and(|v| v.features.exchange);
            if !supported {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    "exchange",
                    Some(a.as_path()),
                ));
            }
            self.same_volume_or_cross(a.as_path(), b.as_path())?;
            let swap = a.as_path().with_extension("swap-test");
            std::fs::rename(a.as_path(), &swap).unwrap();
            std::fs::rename(b.as_path(), a.as_path()).unwrap();
            std::fs::rename(&swap, b.as_path()).unwrap();
            self.apply_pending_writes(b.as_path(), a.as_path());
            self.apply_pending_writes(a.as_path(), b.as_path());
            Ok(())
        }

        fn replace_with_backup(
            &self,
            _scope: &TxScope,
            target: &AbsPath,
            replacement: &AbsPath,
            backup: &AbsPath,
        ) -> FsResult<()> {
            self.record(
                "replace_with_backup",
                &[target.as_path(), replacement.as_path(), backup.as_path()],
            )?;
            let supported = self
                .volume_of(target.as_path())
                .is_some_and(|v| v.features.replace_with_backup);
            if !supported {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    "replace_with_backup",
                    Some(target.as_path()),
                ));
            }
            std::fs::rename(target.as_path(), backup.as_path()).unwrap();
            std::fs::rename(replacement.as_path(), target.as_path()).unwrap();
            Ok(())
        }

        fn hard_link(&self, _scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
            self.record("hard_link", &[existing.as_path(), new.as_path()])?;
            let supported = self
                .volume_of(existing.as_path())
                .is_some_and(|v| v.features.hardlink);
            if !supported {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    "hard_link",
                    Some(existing.as_path()),
                ));
            }
            std::fs::hard_link(existing.as_path(), new.as_path())
                .map_err(|error| fs_error("hard_link", existing.as_path(), &error))
        }

        fn clone_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
            self.record("clone_file", &[src.as_path(), dst.as_path()])?;
            let supported = self
                .volume_of(src.as_path())
                .is_some_and(|v| v.features.reflink);
            if !supported {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    "clone_file",
                    Some(src.as_path()),
                ));
            }
            if std::fs::symlink_metadata(dst.as_path()).is_ok() {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "clone_file",
                    Some(dst.as_path()),
                ));
            }
            let bytes = std::fs::copy(src.as_path(), dst.as_path())
                .map_err(|error| fs_error("clone_file", src.as_path(), &error))?;
            self.state().cloned += bytes;
            Ok(())
        }

        fn copy_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
            self.record("copy_file", &[src.as_path(), dst.as_path()])?;
            if std::fs::symlink_metadata(dst.as_path()).is_ok() {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "copy_file",
                    Some(dst.as_path()),
                ));
            }
            let bytes = std::fs::copy(src.as_path(), dst.as_path())
                .map_err(|error| fs_error("copy_file", src.as_path(), &error))?;
            self.state().copied += bytes;
            Ok(bytes)
        }

        fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            self.record("remove_file", &[path.as_path()])?;
            std::fs::remove_file(path.as_path())
                .map_err(|error| fs_error("remove_file", path.as_path(), &error))
        }

        fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            self.record("remove_dir", &[path.as_path()])?;
            std::fs::remove_dir(path.as_path())
                .map_err(|error| fs_error("remove_dir", path.as_path(), &error))
        }

        fn remove_tree(
            &self,
            _scope: &TxScope,
            path: &AbsPath,
            _parallelism: usize,
        ) -> FsResult<RemoveStats> {
            self.record("remove_tree", &[path.as_path()])?;
            std::fs::remove_dir_all(path.as_path())
                .map_err(|error| fs_error("remove_tree", path.as_path(), &error))?;
            Ok(RemoveStats::default())
        }

        fn create_dir(&self, _scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
            self.record("create_dir", &[path.as_path()])?;
            std::fs::create_dir(path.as_path())
                .map_err(|error| fs_error("create_dir", path.as_path(), &error))?;
            set_path_mode(path.as_path(), mode)
                .map_err(|error| fs_error("create_dir", path.as_path(), &error))
        }

        fn create_link(
            &self,
            _scope: &TxScope,
            target: &Path,
            link: &AbsPath,
            _kind: LinkKind,
        ) -> FsResult<()> {
            self.record("create_link", &[link.as_path()])?;
            symlink(target, link.as_path())
                .map_err(|error| fs_error("create_link", link.as_path(), &error))
        }

        fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
            self.record("set_meta", &[path.as_path()])?;
            if let Some(mode) = change.mode {
                set_path_mode(path.as_path(), mode)
                    .map_err(|error| fs_error("set_meta", path.as_path(), &error))?;
            }
            Ok(())
        }

        fn restore_meta(
            &self,
            _scope: &TxScope,
            path: &AbsPath,
            meta: &ExtendedMeta,
        ) -> FsResult<MetaTransferReport> {
            self.record("restore_meta", &[path.as_path()])?;
            if meta.basic.kind != FileKind::Symlink {
                set_path_mode(path.as_path(), meta.basic.mode)
                    .map_err(|error| fs_error("restore_meta", path.as_path(), &error))?;
            }
            Ok(MetaTransferReport::default())
        }

        fn truncate(&self, _scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
            self.record("truncate", &[path.as_path()])?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(path.as_path())
                .and_then(|file| file.set_len(length))
                .map_err(|error| fs_error("truncate", path.as_path(), &error))
        }

        fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
            self.record("sync_dir", &[dir.as_path()])
        }

        fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
            self.check("lock_exclusive", path.as_path())?;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.as_path())
                .map_err(|error| fs_error("lock_exclusive", path.as_path(), &error))?;
            let deadline = Instant::now() + timeout;
            let mut state = self.state();
            while state.locks.contains(path.as_path()) {
                let now = Instant::now();
                if now >= deadline {
                    return Err(FsError::new(
                        FsErrorKind::InUse,
                        "lock_exclusive",
                        Some(path.as_path()),
                    ));
                }
                state = self
                    .released
                    .wait_timeout(state, deadline - now)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
            state.locks.insert(path.as_path().to_path_buf());
            Ok(Box::new(DiskLock {
                path: path.clone(),
                fs: self.0.clone(),
            }))
        }

        fn mark_hidden(&self, _scope: &TxScope, _path: &AbsPath) -> FsResult<()> {
            Ok(())
        }
    }

    /// Admission with configurable decisions; records every request.
    pub(crate) struct TestAdmission {
        pub(crate) eligibility: Mutex<Eligibility>,
        pub(crate) decision: Mutex<AdmissionDecision>,
        pub(crate) liveness: Mutex<ServiceLiveness>,
        pub(crate) requests: Mutex<Vec<AdmissionRequest>>,
        pub(crate) notified: Mutex<Vec<VolumeId>>,
    }

    impl Default for TestAdmission {
        fn default() -> Self {
            Self {
                eligibility: Mutex::new(Eligibility::Eligible),
                decision: Mutex::new(AdmissionDecision::Keep),
                liveness: Mutex::new(ServiceLiveness::Alive),
                requests: Mutex::new(Vec::new()),
                notified: Mutex::new(Vec::new()),
            }
        }
    }

    impl Admission for TestAdmission {
        fn eligibility(&self, _volume: &VolumeInfo) -> Eligibility {
            *self.eligibility.lock().unwrap()
        }

        fn admit(&self, request: &AdmissionRequest) -> AdmissionDecision {
            self.requests.lock().unwrap().push(request.clone());
            self.decision.lock().unwrap().clone()
        }

        fn after_operation(&self, volume: &VolumeId) {
            self.notified.lock().unwrap().push(volume.clone());
        }

        fn volume_status(&self, _volume: &VolumeId) -> VolumeStatus {
            VolumeStatus {
                state: VolumeState::Normal,
                thresholds: Thresholds {
                    reserve: 0,
                    soft: 0,
                    margin: 0,
                    hysteresis: 0,
                },
            }
        }

        fn liveness(&self) -> ServiceLiveness {
            *self.liveness.lock().unwrap()
        }
    }

    fn sourced<T>(value: T) -> Sourced<T> {
        Sourced {
            value,
            source: ConfigLevel::Builtin,
            raised_to_floor: false,
        }
    }

    /// Default limits: 30 steps, 30 minutes, no size budget.
    pub(crate) fn default_retention() -> EffectiveRetention {
        EffectiveRetention {
            max_ops: sourced(StepLimit::Limited(30)),
            max_age: sourced(AgeLimit::Limited(
                nurt_contracts::config::DurationSetting::from_secs(1800),
            )),
            max_size: sourced(SizeLimit::Unlimited),
            max_size_bytes: None,
            oversize: sourced(Oversize::Skip),
            age_clock: sourced(AgeClock::Wall),
            max_op_size: sourced(ByteLimit::Unlimited),
        }
    }

    /// Retention limits shared by every volume.
    pub(crate) struct TestLimits(pub(crate) Mutex<EffectiveRetention>);

    impl RetentionLimits for TestLimits {
        fn effective(&self, _volume: &VolumeInfo) -> EffectiveRetention {
            self.0.lock().unwrap().clone()
        }
    }

    /// Journal locations keyed by device number.
    pub(crate) struct TestJournals(pub(crate) Mutex<HashMap<u64, JournalLocation>>);

    impl VolumeJournals for TestJournals {
        fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
            self.0
                .lock()
                .unwrap()
                .get(&volume.id.dev)
                .cloned()
                .unwrap_or(JournalLocation::Inactive(
                    nurt_contracts::journal::InactiveReason::NoAnchor,
                ))
        }

        fn active(&self) -> Vec<AbsPath> {
            let mut dirs: Vec<AbsPath> = self
                .0
                .lock()
                .unwrap()
                .values()
                .filter_map(|location| match location {
                    JournalLocation::Active(dir) => Some(dir.clone()),
                    JournalLocation::Inactive(_) => None,
                })
                .collect();
            dirs.sort();
            dirs
        }
    }

    /// Provider of values in memory.
    pub(crate) struct MemProvider {
        pub(crate) scheme: ProviderScheme,
        pub(crate) values: Mutex<HashMap<ProviderKey, ProviderValue>>,
        /// Snapshots passed to `restore`, in order.
        pub(crate) restored: Mutex<Vec<ProviderSnapshot>>,
    }

    impl MemProvider {
        pub(crate) fn new(scheme: &str) -> Self {
            Self {
                scheme: ProviderScheme(scheme.to_owned()),
                values: Mutex::new(HashMap::new()),
                restored: Mutex::new(Vec::new()),
            }
        }
    }

    impl Provider for MemProvider {
        fn scheme(&self) -> ProviderScheme {
            self.scheme.clone()
        }

        fn caps(&self) -> ProviderCaps {
            ProviderCaps {
                writable: true,
                hierarchical: false,
                persistent: true,
            }
        }

        fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn list(&self, _key: &ProviderKey) -> FsResult<Vec<ProviderChild>> {
            Ok(Vec::new())
        }

        fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot> {
            let value = self.values.lock().unwrap().get(key).cloned();
            Ok(ProviderSnapshot {
                key: key.clone(),
                existed: value.is_some(),
                value,
                subtree: None,
            })
        }
    }

    impl ProviderWrite for MemProvider {
        fn set(&self, _scope: &TxScope, key: &ProviderKey, value: &ProviderValue) -> FsResult<()> {
            self.values
                .lock()
                .unwrap()
                .insert(key.clone(), value.clone());
            Ok(())
        }

        fn remove(&self, _scope: &TxScope, key: &ProviderKey) -> FsResult<()> {
            self.values
                .lock()
                .unwrap()
                .remove(key)
                .map(|_| ())
                .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "remove", None))
        }

        fn restore(&self, _scope: &TxScope, snapshot: &ProviderSnapshot) -> FsResult<()> {
            self.restored.lock().unwrap().push(snapshot.clone());
            let mut values = self.values.lock().unwrap();
            match &snapshot.value {
                Some(value) => values.insert(snapshot.key.clone(), value.clone()),
                None => values.remove(&snapshot.key),
            };
            Ok(())
        }

        fn duplicate(
            &self,
            _scope: &TxScope,
            from: &ProviderKey,
            to: &ProviderKey,
        ) -> FsResult<()> {
            let mut values = self.values.lock().unwrap();
            let value = values
                .get(from)
                .cloned()
                .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "copy", None))?;
            values.insert(to.clone(), value);
            Ok(())
        }
    }

    /// Test file system with services: state directory `vol-a/state`, journal
    /// of each volume in `<volume>/.nurt-journal`.
    pub(crate) struct Fixture {
        pub(crate) fs: SharedFs,
        pub(crate) admission: Arc<TestAdmission>,
        pub(crate) limits: Arc<TestLimits>,
        pub(crate) journals: Arc<TestJournals>,
        pub(crate) clock: Arc<VirtualClock>,
        pub(crate) registry: Arc<MemProvider>,
        pub(crate) state_dir: AbsPath,
    }

    // The direct file helpers are methods so that tests read as operations on
    // the fixture; they use the temporary directory, not the fixture state.
    #[allow(clippy::unused_self)]
    impl Fixture {
        pub(crate) fn new() -> Self {
            let fs = SharedFs(Arc::new(DiskFs::new()));
            let state_dir = fs.path("vol-a/state");
            std::fs::create_dir(state_dir.as_path()).unwrap();
            let mut locations = HashMap::new();
            for (index, name) in VOLUMES.iter().enumerate() {
                let dir = fs.path(&format!("{name}/{}", layout::DIR_NAME));
                std::fs::create_dir(dir.as_path()).unwrap();
                locations.insert(fs.volume_at(index).id.dev, JournalLocation::Active(dir));
            }
            Self {
                fs,
                admission: Arc::new(TestAdmission::default()),
                limits: Arc::new(TestLimits(Mutex::new(default_retention()))),
                journals: Arc::new(TestJournals(Mutex::new(locations))),
                clock: Arc::new(VirtualClock::new(VirtualClock::DEFAULT_START)),
                registry: Arc::new(MemProvider::new("registry")),
                state_dir,
            }
        }

        /// Shared context over the fixture services.
        pub(crate) fn ctx(&self) -> JournalCtx {
            JournalCtx {
                fs: Arc::new(self.fs.clone()),
                providers: vec![self.registry.clone()],
                admission: self.admission.clone(),
                limits: self.limits.clone(),
                clock: self.clock.clone(),
                journals: self.journals.clone(),
                state: StateLayout::new(self.state_dir.clone()),
                failpoints: Arc::new(nurt_fs::failpoint::NoFailpoints),
                settings: Settings {
                    overwrite: nurt_contracts::config::OverwriteStrategy::Exchange,
                    lock_timeout: Duration::from_millis(200),
                    reference_schemes: vec![ProviderScheme("gitref".to_owned())],
                },
                space: None,
                startup: OnceLock::new(),
            }
        }

        /// Shared context whose file system is wrapped by the mutation audit;
        /// the state oracle reads the `STATE` file of the entry.
        pub(crate) fn audited_ctx(&self) -> JournalCtx {
            let inner: Arc<dyn nurt_contracts::fs::FsWrite> = Arc::new(self.fs.clone());
            let dirs = self.journals.active();
            let reader = inner.clone();
            let oracle_dirs = dirs.clone();
            let oracle: Arc<dyn nurt_fs::audit::StateOracle> =
                Arc::new(move |op: nurt_contracts::journal::OpId| {
                    oracle_dirs.iter().find_map(|dir| {
                        let entry = layout::JournalLayout::new(dir.clone()).entry(op).ok()?;
                        crate::manifest::read_state(reader.as_ref(), &entry.state().ok()?)
                            .ok()
                            .map(|record| record.state)
                    })
                });
            let audit = nurt_fs::audit::AuditFs::new(inner, oracle);
            for dir in dirs {
                audit.add_internal_root(dir);
            }
            audit.add_internal_root(self.state_dir.clone());
            let mut ctx = self.ctx();
            ctx.fs = Arc::new(audit);
            ctx
        }

        /// Journal over the fixture services.
        pub(crate) fn journal(&self) -> super::Journal {
            JournalBuilder { ctx: self.ctx() }.build().unwrap()
        }

        /// Journal directory of test volume `index`.
        pub(crate) fn journal_dir(&self, index: usize) -> AbsPath {
            self.fs
                .path(&format!("{}/{}", VOLUMES[index], layout::DIR_NAME))
        }

        /// Writes a file directly, outside any transaction.
        pub(crate) fn write(&self, path: &AbsPath, bytes: &[u8]) {
            std::fs::write(path.as_path(), bytes).unwrap();
        }

        /// Reads a file directly.
        pub(crate) fn read(&self, path: &AbsPath) -> Vec<u8> {
            std::fs::read(path.as_path()).unwrap()
        }

        /// Whether an entry exists at `path`.
        pub(crate) fn exists(&self, path: &AbsPath) -> bool {
            std::fs::symlink_metadata(path.as_path()).is_ok()
        }

        /// Sorted names in `dir`; empty for a missing directory.
        pub(crate) fn names(&self, dir: &AbsPath) -> Vec<String> {
            let Ok(entries) = std::fs::read_dir(dir.as_path()) else {
                return Vec::new();
            };
            let mut names: Vec<String> = entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    /// Mutating calls in `log[..end]` that are not durable at `end` under the
    /// minimum POSIX persistence rules: a rename, exchange, hard link or
    /// directory creation is durable after `sync_dir` of every directory it
    /// changes, a metadata change or truncation after a flush of the entry
    /// itself. Each call is listed with the flushes it still needs.
    pub(crate) fn unflushed(log: &[String], end: usize) -> Vec<String> {
        let calls = &log[..end.min(log.len())];
        let mut missing = Vec::new();
        for (position, call) in calls.iter().enumerate() {
            let mut words = call.split(' ');
            let op = words.next().unwrap_or_default();
            let paths: Vec<&Path> = words.map(Path::new).collect();
            let dir_sync = |path: &Path| {
                path.parent()
                    .map(|dir| vec![format!("sync_dir {}", dir.display())])
            };
            let needs: Vec<Vec<String>> = match op {
                "rename" | "exchange" => paths.iter().filter_map(|path| dir_sync(path)).collect(),
                "hard_link" => paths
                    .get(1)
                    .and_then(|path| dir_sync(path))
                    .into_iter()
                    .collect(),
                "create_dir" => paths
                    .first()
                    .and_then(|path| dir_sync(path))
                    .into_iter()
                    .collect(),
                "set_meta" | "restore_meta" | "truncate" => paths
                    .first()
                    .map(|path| {
                        vec![
                            format!("sync_file {}", path.display()),
                            format!("sync_dir {}", path.display()),
                        ]
                    })
                    .into_iter()
                    .collect(),
                _ => continue,
            };
            let later = &calls[position + 1..];
            let lacking: Vec<String> = needs
                .into_iter()
                .filter(|any| !later.iter().any(|entry| any.contains(entry)))
                .map(|any| any.join(" or "))
                .collect();
            if !lacking.is_empty() {
                missing.push(format!("{call}: needs {}", lacking.join(", ")));
            }
        }
        missing
    }
}
