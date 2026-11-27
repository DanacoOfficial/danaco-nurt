// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Error injection for file system calls.
//!
//! [`InjectFs`] wraps any [`FsWrite`] implementation, [`CrashFs`] or a
//! platform back end, and fails selected calls with the error an operating
//! system reports (`EIO`, `EACCES`, `EXDEV`, `EBUSY`, sharing violations,
//! `ESTALE`, `ENOTSUP`, `ENOSPC`, quota exhaustion, `EINTR`), shortens reads
//! and writes, or delays calls through a caller-supplied clock. Faults follow
//! an [`InjectPlan`]; every injected fault is recorded.
//!
//! [`CrashFs`]: crate::crash::CrashFs

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, ExtendedMeta, FileLock, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, SpecialNode, TxScope, VolumeId,
    VolumeInfo, WriteHandle,
};
use nurt_fs::failpoint::SplitMix64;
use serde::{Deserialize, Serialize};

/// File system call that can receive a fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Call {
    /// [`FsWrite::create_temp_in`].
    CreateTempIn,
    /// [`FsWrite::create_new`].
    CreateNew,
    /// [`FsWrite::open_in_place`].
    OpenInPlace,
    /// [`FsWrite::rename`].
    Rename,
    /// [`FsWrite::exchange`].
    Exchange,
    /// [`FsWrite::replace_with_backup`].
    ReplaceWithBackup,
    /// [`FsWrite::hard_link`].
    HardLink,
    /// [`FsWrite::clone_file`].
    CloneFile,
    /// [`FsWrite::copy_file`].
    CopyFile,
    /// [`FsWrite::remove_file`].
    RemoveFile,
    /// [`FsWrite::remove_dir`].
    RemoveDir,
    /// [`FsWrite::remove_tree`].
    RemoveTree,
    /// [`FsWrite::create_dir`].
    CreateDir,
    /// [`FsWrite::create_link`].
    CreateLink,
    /// [`FsWrite::set_meta`].
    SetMeta,
    /// [`FsWrite::restore_meta`].
    RestoreMeta,
    /// [`FsWrite::truncate`].
    Truncate,
    /// [`FsWrite::sync_dir`].
    SyncDir,
    /// [`FsWrite::lock_exclusive`].
    LockExclusive,
    /// [`FsWrite::mark_hidden`].
    MarkHidden,
    /// [`Fs::open_read`] and reads through the returned handle.
    Read,
    /// Writes through a [`WriteHandle`].
    Write,
    /// [`WriteHandle::sync`].
    Sync,
    /// [`WriteHandle::set_length`].
    SetLength,
    /// [`WriteHandle::apply_meta`].
    ApplyMeta,
}

impl Call {
    /// Every call.
    pub const ALL: [Call; 25] = [
        Self::CreateTempIn,
        Self::CreateNew,
        Self::OpenInPlace,
        Self::Rename,
        Self::Exchange,
        Self::ReplaceWithBackup,
        Self::HardLink,
        Self::CloneFile,
        Self::CopyFile,
        Self::RemoveFile,
        Self::RemoveDir,
        Self::RemoveTree,
        Self::CreateDir,
        Self::CreateLink,
        Self::SetMeta,
        Self::RestoreMeta,
        Self::Truncate,
        Self::SyncDir,
        Self::LockExclusive,
        Self::MarkHidden,
        Self::Read,
        Self::Write,
        Self::Sync,
        Self::SetLength,
        Self::ApplyMeta,
    ];

    /// Operation name used in injected errors.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateTempIn => "create_temp_in",
            Self::CreateNew => "create_new",
            Self::OpenInPlace => "open_in_place",
            Self::Rename => "rename",
            Self::Exchange => "exchange",
            Self::ReplaceWithBackup => "replace_with_backup",
            Self::HardLink => "hard_link",
            Self::CloneFile => "clone_file",
            Self::CopyFile => "copy_file",
            Self::RemoveFile => "remove_file",
            Self::RemoveDir => "remove_dir",
            Self::RemoveTree => "remove_tree",
            Self::CreateDir => "create_dir",
            Self::CreateLink => "create_link",
            Self::SetMeta => "set_meta",
            Self::RestoreMeta => "restore_meta",
            Self::Truncate => "truncate",
            Self::SyncDir => "sync_dir",
            Self::LockExclusive => "lock_exclusive",
            Self::MarkHidden => "mark_hidden",
            Self::Read => "read",
            Self::Write => "write",
            Self::Sync => "sync",
            Self::SetLength => "set_length",
            Self::ApplyMeta => "apply_meta",
        }
    }
}

/// Error kinds injected by the error corpus, with the system errors they
/// stand for.
pub const INJECTED_KINDS: [FsErrorKind; 9] = [
    FsErrorKind::Io,
    FsErrorKind::PermissionDenied,
    FsErrorKind::CrossDevice,
    FsErrorKind::InUse,
    FsErrorKind::Stale,
    FsErrorKind::Unsupported,
    FsErrorKind::NoSpace,
    FsErrorKind::QuotaExceeded,
    FsErrorKind::Interrupted,
];

/// Raw system error code reported with an injected error of `kind`.
#[must_use]
pub fn os_code(kind: FsErrorKind) -> Option<i32> {
    if cfg!(windows) {
        match kind {
            FsErrorKind::Io => Some(1117),
            FsErrorKind::PermissionDenied => Some(5),
            FsErrorKind::CrossDevice => Some(17),
            FsErrorKind::InUse => Some(32),
            FsErrorKind::Unsupported => Some(50),
            FsErrorKind::NoSpace => Some(112),
            FsErrorKind::QuotaExceeded => Some(1295),
            FsErrorKind::Interrupted => Some(995),
            _ => None,
        }
    } else {
        match kind {
            FsErrorKind::Io => Some(5),
            FsErrorKind::PermissionDenied => Some(13),
            FsErrorKind::CrossDevice => Some(18),
            FsErrorKind::InUse => Some(16),
            FsErrorKind::Stale => Some(if cfg!(target_os = "macos") { 70 } else { 116 }),
            FsErrorKind::Unsupported => Some(if cfg!(target_os = "macos") { 45 } else { 95 }),
            FsErrorKind::NoSpace => Some(28),
            FsErrorKind::QuotaExceeded => Some(if cfg!(target_os = "macos") { 69 } else { 122 }),
            FsErrorKind::Interrupted => Some(4),
            _ => None,
        }
    }
}

/// Fault applied to a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fault", rename_all = "snake_case")]
pub enum Fault {
    /// The call fails with the error and has no effect.
    Error {
        /// Error kind.
        kind: FsErrorKind,
    },
    /// A write through a handle accepts at most `max` bytes.
    ShortWrite {
        /// Bytes accepted.
        max: usize,
    },
    /// A read through a handle returns at most `max` bytes.
    ShortRead {
        /// Bytes returned.
        max: usize,
    },
    /// The call waits for `millis` milliseconds of the injected clock.
    Delay {
        /// Delay in milliseconds.
        millis: u64,
    },
}

/// Rule selecting the calls that receive a fault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectRule {
    /// Call kind; `None` matches every call.
    pub call: Option<Call>,
    /// Only calls on paths below this one match.
    pub below: Option<PathBuf>,
    /// Zero-based number of the matching call that receives the fault;
    /// `None` applies it to every matching call.
    pub occurrence: Option<u32>,
    /// Fault applied.
    pub fault: Fault,
}

impl InjectRule {
    /// Rule failing occurrence `occurrence` of `call` with `kind`.
    #[must_use]
    pub fn error_at(call: Call, occurrence: u32, kind: FsErrorKind) -> Self {
        Self {
            call: Some(call),
            below: None,
            occurrence: Some(occurrence),
            fault: Fault::Error { kind },
        }
    }
}

/// Plan of injected faults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectPlan {
    /// Seed of the random errors.
    pub seed: u64,
    /// Rules checked in order; the first rule that applies decides.
    pub rules: Vec<InjectRule>,
    /// Probability of a random error per call without a rule, per million.
    pub error_per_million: u32,
    /// Error kinds drawn for random errors.
    pub error_kinds: Vec<FsErrorKind>,
}

/// One injected fault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Injected {
    /// Zero-based position of the call among all calls seen.
    pub seq: u64,
    /// Call kind.
    pub call: Call,
    /// Path of the call.
    pub path: PathBuf,
    /// Fault applied.
    pub fault: Fault,
}

/// Sink of injected delays, typically advancing a virtual clock.
pub type DelaySink = Arc<dyn Fn(Duration) + Send + Sync>;

struct InjectState {
    plan: InjectPlan,
    rng: SplitMix64,
    matched: HashMap<usize, u32>,
    calls: HashMap<Call, u64>,
    seen: u64,
    injected: Vec<Injected>,
}

struct Shared {
    state: Mutex<InjectState>,
    delay: Option<DelaySink>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, InjectState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn decide(&self, call: Call, path: &Path) -> Option<Fault> {
        let mut state = self.lock();
        state.seen += 1;
        *state.calls.entry(call).or_insert(0) += 1;
        let fault = state
            .rule_fault(call, path)
            .or_else(|| state.random_fault());
        if let Some(fault) = fault {
            let seq = state.seen - 1;
            state.injected.push(Injected {
                seq,
                call,
                path: path.to_path_buf(),
                fault,
            });
        }
        fault
    }

    /// Applies error and delay faults; returns the size limit of a short
    /// transfer, if any.
    fn gate(&self, call: Call, path: &Path) -> FsResult<Option<usize>> {
        match self.decide(call, path) {
            Some(Fault::Error { kind }) => {
                let mut err = FsError::new(kind, call.as_str(), Some(path));
                err.os_code = os_code(kind);
                Err(err)
            }
            Some(Fault::Delay { millis }) => {
                if let Some(delay) = &self.delay {
                    delay(Duration::from_millis(millis));
                }
                Ok(None)
            }
            Some(Fault::ShortWrite { max } | Fault::ShortRead { max }) => Ok(Some(max)),
            None => Ok(None),
        }
    }
}

impl InjectState {
    fn rule_fault(&mut self, call: Call, path: &Path) -> Option<Fault> {
        for (index, rule) in self.plan.rules.iter().enumerate() {
            let applies = rule.call.is_none_or(|wanted| wanted == call)
                && rule
                    .below
                    .as_ref()
                    .is_none_or(|below| path.starts_with(below));
            if !applies {
                continue;
            }
            let count = self.matched.entry(index).or_insert(0);
            let current = *count;
            *count += 1;
            if rule.occurrence.is_none_or(|wanted| wanted == current) {
                return Some(rule.fault);
            }
        }
        None
    }

    fn random_fault(&mut self) -> Option<Fault> {
        if self.plan.error_kinds.is_empty() || !self.rng.chance(self.plan.error_per_million) {
            return None;
        }
        let len = self.plan.error_kinds.len() as u64;
        let index = usize::try_from(self.rng.below(len)).unwrap_or(0);
        self.plan
            .error_kinds
            .get(index)
            .map(|kind| Fault::Error { kind: *kind })
    }
}

/// File system wrapper that injects faults into calls of `F`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::{AbsPath, FsErrorKind, FsWrite, ScopePurpose, TxScope};
/// use nurt_contracts::journal::OpId;
/// use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
/// use nurt_crashfs::inject::{Call, InjectFs, InjectPlan, InjectRule};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let root = AbsPath::new(if cfg!(windows) { "C:\\vol" } else { "/vol" })?;
/// let inner = CrashFs::new(CrashProfile::PosixWorstCase, 1, vec![VolumeSpec::new(root.clone())])?;
/// let plan = InjectPlan {
///     rules: vec![InjectRule::error_at(Call::CreateDir, 0, FsErrorKind::NoSpace)],
///     ..InjectPlan::default()
/// };
/// let fs = InjectFs::new(inner, plan);
/// let scope = TxScope::new_for_journal(OpId(1), ScopePurpose::Maintenance);
/// let err = fs.create_dir(&scope, &root.join_name("d")?, 0o755).unwrap_err();
/// assert_eq!(err.kind, FsErrorKind::NoSpace);
/// assert!(fs.create_dir(&scope, &root.join_name("d")?, 0o755).is_ok());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct InjectFs<F> {
    inner: F,
    shared: Arc<Shared>,
}

impl<F: FsWrite> InjectFs<F> {
    /// Wraps `inner` with faults planned by `plan`.
    pub fn new(inner: F, plan: InjectPlan) -> Self {
        Self::with_delay(inner, plan, None)
    }

    /// Wraps `inner`; delays are passed to `delay`.
    pub fn with_delay(inner: F, plan: InjectPlan, delay: Option<DelaySink>) -> Self {
        let rng = SplitMix64::new(plan.seed);
        Self {
            inner,
            shared: Arc::new(Shared {
                state: Mutex::new(InjectState {
                    plan,
                    rng,
                    matched: HashMap::new(),
                    calls: HashMap::new(),
                    seen: 0,
                    injected: Vec::new(),
                }),
                delay,
            }),
        }
    }

    /// Wrapped file system.
    pub fn inner(&self) -> &F {
        &self.inner
    }

    /// Faults injected so far.
    pub fn injected(&self) -> Vec<Injected> {
        self.shared.lock().injected.clone()
    }

    /// Number of calls of `call` seen so far, faulted or not.
    pub fn calls(&self, call: Call) -> u64 {
        self.shared.lock().calls.get(&call).copied().unwrap_or(0)
    }

    fn gate(&self, call: Call, path: &AbsPath) -> FsResult<Option<usize>> {
        self.shared.gate(call, path.as_path())
    }

    fn wrap(&self, handle: Box<dyn WriteHandle>) -> Box<dyn WriteHandle> {
        Box::new(InjectWriter {
            inner: handle,
            shared: Arc::clone(&self.shared),
        })
    }
}

impl<F: FsWrite> Fs for InjectFs<F> {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        self.inner.metadata(path, follow)
    }

    fn special_node(&self, path: &AbsPath) -> FsResult<Option<SpecialNode>> {
        self.inner.special_node(path)
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        self.inner.extended_metadata(path, follow)
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let limit = self.gate(Call::Read, path)?;
        let inner = self.inner.open_read(path)?;
        Ok(match limit {
            Some(max) => Box::new(ShortReader { inner, max }),
            None => inner,
        })
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
        self.gate(Call::Read, path)?;
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

impl<F: FsWrite> FsWrite for InjectFs<F> {
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.gate(Call::CreateTempIn, dir)?;
        self.inner
            .create_temp_in(scope, dir, name_hint)
            .map(|handle| self.wrap(handle))
    }

    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.gate(Call::CreateNew, path)?;
        self.inner
            .create_new(scope, path)
            .map(|handle| self.wrap(handle))
    }

    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.gate(Call::OpenInPlace, path)?;
        self.inner
            .open_in_place(scope, path)
            .map(|handle| self.wrap(handle))
    }

    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.inner
            .open_stream_write(scope, path, stream)
            .map(|handle| self.wrap(handle))
    }

    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        self.gate(Call::Rename, from)?;
        self.inner.rename(scope, from, to, mode)
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        self.gate(Call::Exchange, a)?;
        self.inner.exchange(scope, a, b)
    }

    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        self.gate(Call::ReplaceWithBackup, target)?;
        self.inner
            .replace_with_backup(scope, target, replacement, backup)
    }

    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        self.gate(Call::HardLink, existing)?;
        self.inner.hard_link(scope, existing, new)
    }

    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        self.gate(Call::CloneFile, src)?;
        self.inner.clone_file(scope, src, dst)
    }

    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        self.gate(Call::CopyFile, src)?;
        self.inner.copy_file(scope, src, dst)
    }

    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.gate(Call::RemoveFile, path)?;
        self.inner.remove_file(scope, path)
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.gate(Call::RemoveDir, path)?;
        self.inner.remove_dir(scope, path)
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        self.gate(Call::RemoveTree, path)?;
        self.inner.remove_tree(scope, path, parallelism)
    }

    fn create_node(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        node: SpecialNode,
        mode: u32,
    ) -> FsResult<()> {
        self.inner.create_node(scope, path, node, mode)
    }

    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        self.gate(Call::CreateDir, path)?;
        self.inner.create_dir(scope, path, mode)
    }

    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        self.gate(Call::CreateLink, link)?;
        self.inner.create_link(scope, target, link, kind)
    }

    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        self.gate(Call::SetMeta, path)?;
        self.inner.set_meta(scope, path, change)
    }

    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        self.gate(Call::RestoreMeta, path)?;
        self.inner.restore_meta(scope, path, meta)
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        self.gate(Call::Truncate, path)?;
        self.inner.truncate(scope, path, length)
    }

    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        self.gate(Call::SyncDir, dir)?;
        self.inner.sync_dir(scope, dir)
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        self.gate(Call::LockExclusive, path)?;
        self.inner.lock_exclusive(path, timeout)
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.gate(Call::MarkHidden, path)?;
        self.inner.mark_hidden(scope, path)
    }
}

fn io_error(err: FsError) -> io::Error {
    let kind = match err.kind {
        FsErrorKind::NoSpace | FsErrorKind::QuotaExceeded => io::ErrorKind::StorageFull,
        FsErrorKind::Interrupted => io::ErrorKind::Interrupted,
        FsErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, err)
}

/// Write handle that receives faults of [`Call::Write`], [`Call::Sync`],
/// [`Call::SetLength`] and [`Call::ApplyMeta`].
struct InjectWriter {
    inner: Box<dyn WriteHandle>,
    shared: Arc<Shared>,
}

impl Write for InjectWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let limit = self
            .shared
            .gate(Call::Write, self.inner.path().as_path())
            .map_err(io_error)?;
        let accepted = limit.map_or(buf.len(), |max| max.clamp(1, buf.len().max(1)));
        self.inner.write(buf.get(..accepted).unwrap_or(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for InjectWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl WriteHandle for InjectWriter {
    fn path(&self) -> &AbsPath {
        self.inner.path()
    }

    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        self.shared.gate(Call::Sync, self.inner.path().as_path())?;
        self.inner.sync(durability)
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        self.shared
            .gate(Call::SetLength, self.inner.path().as_path())?;
        self.inner.set_length(length)
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        self.shared
            .gate(Call::ApplyMeta, self.inner.path().as_path())?;
        self.inner.apply_meta(meta)
    }
}

/// Read handle returning at most `max` bytes per read; `max` of zero is
/// treated as one so that readers never see a false end of file.
struct ShortReader {
    inner: Box<dyn ReadHandle>,
    max: usize,
}

impl Read for ShortReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let limit = self.max.max(1).min(buf.len());
        let window = buf.get_mut(..limit).unwrap_or_default();
        self.inner.read(window)
    }
}

impl Seek for ShortReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl ReadHandle for ShortReader {
    fn meta(&self) -> FsResult<FileMeta> {
        self.inner.meta()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nurt_contracts::fs::{ScopePurpose, TxScope};
    use nurt_contracts::journal::OpId;

    use super::*;
    use crate::crash::{CrashFs, CrashProfile, VolumeSpec};

    fn root() -> AbsPath {
        AbsPath::new(if cfg!(windows) { "C:\\vol" } else { "/vol" }).unwrap()
    }

    #[allow(clippy::disallowed_methods)] // the test builds a transaction scope as the journal does
    fn scope() -> TxScope {
        TxScope::new_for_journal(OpId(1), ScopePurpose::Maintenance)
    }

    fn inner() -> CrashFs {
        CrashFs::new(
            CrashProfile::PosixWorstCase,
            3,
            vec![VolumeSpec::new(root())],
        )
        .unwrap()
    }

    fn at(name: &str) -> AbsPath {
        root().join_name(name).unwrap()
    }

    fn rule(call: Call, fault: Fault) -> InjectRule {
        InjectRule {
            call: Some(call),
            below: None,
            occurrence: None,
            fault,
        }
    }

    fn plan(rules: Vec<InjectRule>) -> InjectPlan {
        InjectPlan {
            rules,
            ..InjectPlan::default()
        }
    }

    #[test]
    fn every_call_has_a_distinct_name() {
        let names: HashSet<&str> = Call::ALL.iter().map(|call| call.as_str()).collect();
        assert_eq!(names.len(), Call::ALL.len());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_error_numbers_are_reported() {
        assert_eq!(os_code(FsErrorKind::Stale), Some(116));
        assert_eq!(os_code(FsErrorKind::Unsupported), Some(95));
        assert_eq!(os_code(FsErrorKind::QuotaExceeded), Some(122));
        assert_eq!(os_code(FsErrorKind::NoSpace), Some(28));
        assert_eq!(os_code(FsErrorKind::NotFound), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_error_numbers_differ_from_linux() {
        assert_eq!(os_code(FsErrorKind::Stale), Some(70));
        assert_eq!(os_code(FsErrorKind::Unsupported), Some(45));
        assert_eq!(os_code(FsErrorKind::QuotaExceeded), Some(69));
        assert_eq!(os_code(FsErrorKind::NoSpace), Some(28));
    }

    #[test]
    fn an_error_rule_fails_only_the_chosen_occurrence() {
        let fs = InjectFs::new(
            inner(),
            plan(vec![InjectRule::error_at(
                Call::CreateDir,
                1,
                FsErrorKind::PermissionDenied,
            )]),
        );
        fs.create_dir(&scope(), &at("a"), 0o755).unwrap();
        let err = fs.create_dir(&scope(), &at("b"), 0o755).unwrap_err();
        assert_eq!(err.kind, FsErrorKind::PermissionDenied);
        assert_eq!(err.os_code, os_code(FsErrorKind::PermissionDenied));
        fs.create_dir(&scope(), &at("c"), 0o755).unwrap();
        assert!(fs.inner().metadata(&at("b"), Follow::NoFollow).is_err());
        assert_eq!(fs.calls(Call::CreateDir), 3);
        let injected = fs.injected();
        assert_eq!(injected.len(), 1);
        assert_eq!(injected[0].seq, 1);
        assert_eq!(injected[0].call, Call::CreateDir);
    }

    #[test]
    fn a_rule_applies_only_below_its_path() {
        let mut limited = rule(
            Call::CreateDir,
            Fault::Error {
                kind: FsErrorKind::Io,
            },
        );
        limited.below = Some(at("x").as_path().to_path_buf());
        let fs = InjectFs::new(inner(), plan(vec![limited]));
        fs.create_dir(&scope(), &at("y"), 0o755).unwrap();
        let err = fs.create_dir(&scope(), &at("x"), 0o755).unwrap_err();
        assert_eq!(err.kind, FsErrorKind::Io);
    }

    #[test]
    fn short_writes_are_completed_by_write_all() {
        let fs = InjectFs::new(
            inner(),
            plan(vec![rule(Call::Write, Fault::ShortWrite { max: 2 })]),
        );
        let mut handle = fs.create_new(&scope(), &at("f")).unwrap();
        handle.write_all(b"hello").unwrap();
        drop(handle);
        let mut stored = Vec::new();
        fs.open_read(&at("f"))
            .unwrap()
            .read_to_end(&mut stored)
            .unwrap();
        assert_eq!(stored, b"hello");
        assert_eq!(fs.injected().len(), 3);
    }

    #[test]
    fn short_reads_return_at_most_the_limit() {
        let fs = InjectFs::new(
            inner(),
            plan(vec![rule(Call::Read, Fault::ShortRead { max: 3 })]),
        );
        let mut handle = fs.create_new(&scope(), &at("f")).unwrap();
        handle.write_all(b"abcdef").unwrap();
        drop(handle);
        let mut reader = fs.open_read(&at("f")).unwrap();
        let mut buffer = [0_u8; 10];
        assert_eq!(reader.read(&mut buffer).unwrap(), 3);
        assert_eq!(&buffer[..3], b"abc");
        assert!(reader.meta().is_ok());
    }

    #[test]
    fn delays_are_passed_to_the_sink() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        let sink: DelaySink = Arc::new(move |delay| {
            sink_seen.lock().unwrap().push(delay);
        });
        let fs = InjectFs::with_delay(
            inner(),
            plan(vec![rule(Call::CreateDir, Fault::Delay { millis: 5 })]),
            Some(sink),
        );
        fs.create_dir(&scope(), &at("a"), 0o755).unwrap();
        assert_eq!(*seen.lock().unwrap(), [Duration::from_millis(5)]);
    }

    #[test]
    fn random_errors_follow_the_probability_and_the_kinds() {
        let always = InjectPlan {
            seed: 9,
            rules: Vec::new(),
            error_per_million: 1_000_000,
            error_kinds: vec![FsErrorKind::Io],
        };
        let fs = InjectFs::new(inner(), always);
        let err = fs.create_dir(&scope(), &at("a"), 0o755).unwrap_err();
        assert_eq!(err.kind, FsErrorKind::Io);
        let without_kinds = InjectPlan {
            error_per_million: 1_000_000,
            ..InjectPlan::default()
        };
        let fs = InjectFs::new(inner(), without_kinds);
        fs.create_dir(&scope(), &at("a"), 0o755).unwrap();
        assert!(fs.injected().is_empty());
    }

    #[test]
    fn a_failed_write_reports_the_error_through_the_io_interface() {
        let fs = InjectFs::new(
            inner(),
            plan(vec![rule(
                Call::Write,
                Fault::Error {
                    kind: FsErrorKind::NoSpace,
                },
            )]),
        );
        let mut handle = fs.create_new(&scope(), &at("f")).unwrap();
        let err = handle.write(b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
    }
}
