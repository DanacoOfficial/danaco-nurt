// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test doubles: a file system over a real directory, a virtual clock, a
//! host and a journal transaction that keeps byte copies in memory and can
//! undo the recorded changes.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileId, FileKind, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, Holder, PathKey, ReadHandle, SpaceInfo, VolumeId, VolumeInfo,
};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, ExternalChangeKind, Mutation,
    MutationTx, OpHandle, OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport,
    TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::{ChangeAction, IrreversibleEntry};
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx};
use sha2::{Digest, Sha256};

use crate::iface::{PreparedScan, TrackMode, TrackOutcome, TrackSpec};
use crate::track::tracker;

/// Operation number given by the recording transaction.
pub(super) const TEST_OP: u64 = 7;

/// SHA-256 of `bytes`.
pub(super) fn digest(bytes: &[u8]) -> ContentHash {
    let mut hash = [0; 32];
    hash.copy_from_slice(Sha256::digest(bytes).as_slice());
    ContentHash(hash)
}

fn fs_error(error: &io::Error, op: &str, path: &Path) -> FsError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => FsErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        _ => FsErrorKind::Io,
    };
    FsError::new(kind, op, Some(path))
}

fn unsupported(op: &str) -> FsError {
    FsError::new(FsErrorKind::Unsupported, op, None)
}

fn file_meta(meta: &fs::Metadata) -> FileMeta {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Directory
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Special
    };
    FileMeta {
        kind,
        size: meta.len(),
        mode: meta.mode() & 0o7777,
        uid: Some(meta.uid()),
        gid: Some(meta.gid()),
        links: meta.nlink(),
        modified_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
        accessed_ns: None,
        created_ns: None,
        changed_ns: None,
        file_id: FileId {
            volume: meta.dev(),
            low: meta.ino(),
            high: 0,
        },
        windows_attributes: 0,
    }
}

/// Open file of [`DiskFs`].
struct DiskRead(fs::File);

impl Read for DiskRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for DiskRead {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

impl ReadHandle for DiskRead {
    fn meta(&self) -> FsResult<FileMeta> {
        self.0
            .metadata()
            .map(|meta| file_meta(&meta))
            .map_err(|error| fs_error(&error, "meta", Path::new("")))
    }
}

/// Read-only file system over the real directory tree.
pub(super) struct DiskFs;

impl Fs for DiskFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let meta = match follow {
            Follow::NoFollow => fs::symlink_metadata(path.as_path()),
            Follow::Follow => fs::metadata(path.as_path()),
        };
        meta.map(|meta| file_meta(&meta))
            .map_err(|error| fs_error(&error, "metadata", path.as_path()))
    }

    fn extended_metadata(&self, _path: &AbsPath, _follow: Follow) -> FsResult<ExtendedMeta> {
        Err(unsupported("extended_metadata"))
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let file =
            fs::File::open(path.as_path()).map_err(|error| fs_error(&error, "open", path.as_path()))?;
        Ok(Box::new(DiskRead(file)))
    }

    fn open_stream_read(&self, _path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        Err(unsupported("open_stream_read"))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let dir = path.as_path().to_path_buf();
        let entries =
            fs::read_dir(&dir).map_err(|error| fs_error(&error, "read_dir", path.as_path()))?;
        Ok(Box::new(entries.map(move |entry| {
            entry
                .map(|entry| DirEntry {
                    name: entry.file_name(),
                    kind: None,
                })
                .map_err(|error| fs_error(&error, "read_dir", &dir))
        })))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        fs::read_link(path.as_path()).map_err(|error| fs_error(&error, "read_link", path.as_path()))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let real = fs::canonicalize(path.as_path())
            .map_err(|error| fs_error(&error, "real_path", path.as_path()))?;
        AbsPath::new(real)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let bytes =
            fs::read(path.as_path()).map_err(|error| fs_error(&error, "read", path.as_path()))?;
        Ok(digest(&bytes))
    }

    fn volume(&self, _path: &AbsPath) -> FsResult<VolumeInfo> {
        Err(unsupported("volume"))
    }

    fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
        Err(unsupported("space"))
    }

    fn path_key(&self, _path: &AbsPath) -> FsResult<PathKey> {
        Err(unsupported("path_key"))
    }

    fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
        Ok(Vec::new())
    }

    fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
        Ok(true)
    }
}

/// Clock whose time moves only when advanced.
pub(super) struct VirtualClock {
    monotonic: Mutex<Duration>,
}

impl VirtualClock {
    fn new() -> Self {
        Self {
            monotonic: Mutex::new(Duration::ZERO),
        }
    }

    /// Moves the time forward.
    pub(super) fn advance(&self, by: Duration) {
        *self.monotonic.lock().unwrap() += by;
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(i128::try_from(self.monotonic().as_nanos()).unwrap())
    }

    fn monotonic(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> std::result::Result<(), CancelReason> {
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        self.advance(duration);
        Ok(())
    }
}

struct TestHost(HostInfo);

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Context with the disk file system, a virtual clock and `config`.
pub(super) fn context(config: Config) -> (ExecCtx, Arc<VirtualClock>) {
    let clock = Arc::new(VirtualClock::new());
    let host = TestHost(HostInfo {
        os: Os::Linux,
        os_version: "6.8.0".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "test".to_owned(),
        user: "user".to_owned(),
        home: PathBuf::from("/home/user"),
        temp: PathBuf::from("/tmp"),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 2,
    });
    let ctx = ExecCtx::builder(clock.clone(), Arc::new(host))
        .fs(Arc::new(DiskFs))
        .config(Arc::new(config))
        .build();
    (ctx, clock)
}

/// Journal transaction that keeps declared files as byte copies.
pub(super) struct RecordingTx {
    /// Strategy reported for kept files.
    pub(super) mode: PreserveMode,
    /// Clock advanced by `delay` for every kept file.
    pub(super) clock: Option<Arc<VirtualClock>>,
    /// Simulated time of keeping one file.
    pub(super) delay: Duration,
    /// Operation number of the transaction.
    pub(super) op_number: u64,
    op: Option<OpId>,
    /// Declared paths and their content, by target index.
    pub(super) declared: Vec<(PathBuf, Vec<u8>)>,
    /// Number of calls of `preserve`.
    pub(super) preserve_calls: usize,
    /// Reported changes.
    pub(super) observed: Vec<ExternalChange>,
}

impl RecordingTx {
    /// Transaction keeping copies.
    pub(super) fn new() -> Self {
        Self {
            mode: PreserveMode::Copy,
            clock: None,
            delay: Duration::ZERO,
            op_number: TEST_OP,
            op: None,
            declared: Vec::new(),
            preserve_calls: 0,
            observed: Vec::new(),
        }
    }

    /// Transaction that reports `mode` for kept files.
    pub(super) fn with_mode(mode: PreserveMode) -> Self {
        Self {
            mode,
            ..Self::new()
        }
    }

    /// Transaction of another operation, for a finish in a new transaction.
    pub(super) fn other_operation() -> Self {
        Self {
            op_number: TEST_OP + 1,
            op: Some(OpId(TEST_OP + 1)),
            ..Self::new()
        }
    }

    /// Reverts the reported changes as the journal does on undo: created
    /// entries are removed and declared targets get their kept content back.
    pub(super) fn undo(&self) {
        for change in self.observed.iter().rev() {
            let path = change.path.as_path();
            match (change.kind, change.declared) {
                (ExternalChangeKind::Created, _) => remove(path),
                (_, Some(TargetIndex(index))) => {
                    let (_, content) = &self.declared[index as usize];
                    remove(path);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, content).unwrap();
                }
                _ => {}
            }
        }
    }

    /// Declared target indexes of the reported changes.
    pub(super) fn declared_indexes(&self) -> Vec<Option<TargetIndex>> {
        self.observed.iter().map(|change| change.declared).collect()
    }
}

fn remove(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path).unwrap(),
        Ok(_) => fs::remove_file(path).unwrap(),
        Err(_) => {}
    }
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        self.op
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        self.preserve_calls += 1;
        self.op.get_or_insert(OpId(self.op_number));
        let mut preserved = Vec::new();
        for target in targets {
            let TargetPath::Fs(path) = &target.path else {
                return Err(Error::new(ErrorCode::StageFailed, "unexpected target"));
            };
            let content = fs::read(path.as_path())
                .map_err(|_| Error::new(ErrorCode::NotFound, "target is missing"))?;
            let index = u32::try_from(self.declared.len()).unwrap();
            if let Some(clock) = &self.clock {
                clock.advance(self.delay);
            }
            preserved.push(Preserved {
                index: TargetIndex(index),
                volume: None,
                mode: self.mode,
                bytes_held: content.len() as u64,
                before: BeforeState {
                    exists: true,
                    kind: Some(FileKind::File),
                    size: Some(content.len() as u64),
                    hash: Some(digest(&content)),
                    mode: None,
                },
                irreversible: None,
                notable: Vec::new(),
            });
            self.declared.push((path.as_path().to_path_buf(), content));
        }
        Ok(preserved)
    }

    fn stage(
        &mut self,
        _target: TargetIndex,
    ) -> Result<Box<dyn nurt_contracts::fs::WriteHandle>> {
        Err(Error::new(ErrorCode::StageFailed, "not used by change detection"))
    }

    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        Err(Error::new(ErrorCode::StageFailed, "not used by change detection"))
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.observed.extend(changes.iter().cloned());
        Ok(changes
            .iter()
            .map(|change| {
                let content = fs::read(change.path.as_path()).ok();
                Applied {
                    index: change.declared.unwrap_or(TargetIndex(u32::MAX)),
                    after: AfterState {
                        exists: change.kind != ExternalChangeKind::Deleted,
                        kind: None,
                        size: content.as_ref().map(|bytes| bytes.len() as u64),
                        hash: content.as_deref().map(digest),
                        mode: None,
                    },
                }
            })
            .collect())
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Ok(OpHandle {
            op: if self.observed.is_empty() { None } else { self.op },
            kind: OpKind::command("run"),
            bulk: true,
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Ok(RollbackReport {
            op: self.op,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

/// Temporary watched directory with a context.
pub(super) struct Harness {
    _dir: tempfile::TempDir,
    /// Watched directory.
    pub(super) root: PathBuf,
    /// Clock of the context.
    pub(super) clock: Arc<VirtualClock>,
    /// Context.
    pub(super) ctx: ExecCtx,
}

impl Harness {
    /// Harness with the default configuration.
    pub(super) fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// Harness with `config`.
    pub(super) fn with_config(config: Config) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("work");
        fs::create_dir(&root).unwrap();
        let (ctx, clock) = context(config);
        Self {
            _dir: dir,
            root,
            clock,
            ctx,
        }
    }

    /// Absolute path of `relative`.
    pub(super) fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Writes a file, creating its directory.
    pub(super) fn write(&self, relative: &str, text: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Content of a file.
    pub(super) fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path(relative)).unwrap()
    }

    /// Sets the permission bits of an entry.
    pub(super) fn chmod(&self, relative: &str, mode: u32) {
        fs::set_permissions(self.path(relative), fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Detection of the watched directory in `mode`.
    pub(super) fn spec(&self, mode: TrackMode) -> TrackSpec {
        TrackSpec {
            mode,
            watch: vec![self.root.clone()],
            preimage: true,
            ..TrackSpec::default()
        }
    }

    /// Prepares, applies `change` and finishes.
    pub(super) fn run(
        &self,
        spec: &TrackSpec,
        tx: &mut RecordingTx,
        change: impl FnOnce(&Self),
    ) -> TrackOutcome {
        let prepared = self.prepare(spec, tx);
        change(self);
        prepared.finish(&self.ctx, tx).unwrap()
    }

    /// Prepares a run.
    pub(super) fn prepare(&self, spec: &TrackSpec, tx: &mut RecordingTx) -> Box<dyn PreparedScan> {
        tracker().prepare(&self.ctx, spec, tx).unwrap()
    }

    /// Path of a change record relative to the watched directory.
    pub(super) fn relative(&self, path: &str) -> String {
        let root = format!("{}/", self.root.display());
        path.strip_prefix(&root).unwrap_or(path).to_owned()
    }

    /// Relative paths and actions of the change records.
    pub(super) fn actions(&self, outcome: &TrackOutcome) -> Vec<(String, ChangeAction)> {
        outcome
            .changes
            .iter()
            .map(|change| (self.relative(change.path.as_str()), change.action))
            .collect()
    }

    /// Change record of `relative`.
    pub(super) fn change<'a>(&self, outcome: &'a TrackOutcome, relative: &str) -> &'a Change {
        outcome
            .changes
            .iter()
            .find(|change| self.relative(change.path.as_str()) == relative)
            .unwrap_or_else(|| panic!("no change of {relative}"))
    }
}
