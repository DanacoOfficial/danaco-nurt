// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Deterministic in-memory file system implementing the mutating interface.
//!
//! [`MemFs`] implements [`Fs`] and [`FsWrite`] with the reference semantics
//! required of test implementations:
//!
//! * Entries are inodes. A name refers to an inode; hard links share one
//!   inode, and open handles refer to the inode, not to the name, so a handle
//!   opened before a rename or an exchange keeps writing the same content
//!   under its new name.
//! * File identities are assigned from 1 upwards per volume (the volume root
//!   is 1); a rename keeps the identity. Timestamps come from the injected
//!   [`Clock`]; a content change sets the modification and status change
//!   times to the current wall time.
//! * [`Fs::read_dir`] returns entries in byte order of their stored names.
//! * Names are compared by [`key::name_key`] with the emulated system and the
//!   case sensitivity of the volume.
//! * Space is accounted per volume in materialised bytes: a file consists of
//!   materialised bytes followed by a zero-filled tail, so
//!   [`WriteHandle::set_length`] creates sparse files. Clones share content
//!   until one side is modified; hard links share the inode. A write that
//!   does not fit fails with [`FsErrorKind::NoSpace`] or, when the volume has
//!   a quota, [`FsErrorKind::QuotaExceeded`].
//! * Overlay volumes emulate a read-only lower layer: entries marked with
//!   [`MemFs::seed_lower`] are not charged, renaming a lower directory fails
//!   with [`FsErrorKind::CrossDevice`], and modifying a lower file first copies
//!   it to the upper layer, which charges its size.
//! * The file system is always durable: [`WriteHandle::sync`] and
//!   [`FsWrite::sync_dir`] only count calls. Loss of unsynchronised writes is
//!   modelled by wrappers over [`FsWrite`].
//!
//! Permission bits decide [`Fs::can_write_dir`] only. Changing the account
//! of an entry requires the configured account `0`.
//!
//! # Examples
//!
//! ```
//! use std::io::Read;
//! use nurt_contracts::fs::{AbsPath, Fs};
//! use nurt_fs::mem::MemFs;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let fs = MemFs::new()?;
//! let path = AbsPath::new("/data/note.txt")?;
//! fs.seed_file(&path, b"hello")?;
//! let mut text = String::new();
//! fs.open_read(&path)?.read_to_string(&mut text)?;
//! assert_eq!(text, "hello");
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use nurt_contracts::env::{Clock, Os};
use nurt_contracts::fs::{
    AbsPath, AlternateStream, ContentHash, DirEntry, Durability, ExtendedMeta, FileId, FileKind,
    FileLock, FileMeta, Follow, Fs, FsError, FsErrorKind, FsFeatures, FsResult, FsWrite, Holder,
    LinkKind, MetaChange, MetaTransferReport, PathKey, ReadHandle, RemoveStats, RenameMode,
    SpaceInfo, TxScope, VolumeId, VolumeInfo, VolumeKind, WriteHandle, Xattr,
};
use sha2::{Digest, Sha256};

use crate::clock::VirtualClock;
use crate::key;

/// Capacity of a volume unless configured otherwise: 1 TiB.
pub const DEFAULT_CAPACITY: u64 = 1 << 40;

/// Account (user and group id) of a file system unless configured otherwise.
pub const DEFAULT_ACCOUNT: u32 = 1000;

const ROOT_ACCOUNT: u32 = 0;
const DEFAULT_FILE_MODE: u32 = 0o644;
const DEFAULT_DIR_MODE: u32 = 0o755;
const LINK_MODE: u32 = 0o777;
const PERMISSION_BITS: u32 = 0o7777;
const WRITE_AND_SEARCH: u32 = 0o3;
const OWNER_SHIFT: u32 = 6;
const GROUP_SHIFT: u32 = 3;
const MAX_LINK_HOPS: usize = 40;
const HASH_CHUNK: usize = 64 * 1024;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x2000;
const MACOS_UF_HIDDEN: u64 = 0x8000;
const MEMFS_TYPE: &str = "memfs";
const OVERLAY_TYPE: &str = "overlay";

const POSIX_FEATURES: FsFeatures = FsFeatures {
    exchange: true,
    rename_noreplace: true,
    replace_with_backup: false,
    hardlink: true,
    reflink: true,
    xattr: true,
    acl: true,
    alternate_streams: false,
    durable_metadata: true,
    case_sensitive: true,
    decomposes_names: false,
};

const WINDOWS_FEATURES: FsFeatures = FsFeatures {
    exchange: false,
    rename_noreplace: true,
    replace_with_backup: true,
    hardlink: true,
    reflink: false,
    xattr: false,
    acl: true,
    alternate_streams: true,
    durable_metadata: true,
    case_sensitive: false,
    decomposes_names: false,
};

const OVERLAY_FEATURES: FsFeatures = FsFeatures {
    reflink: false,
    ..POSIX_FEATURES
};

/// Configuration of one volume of a [`MemFs`].
///
/// | Preset | Kind | Features |
/// |---|---|---|
/// | [`posix`](Self::posix) | fixed | exchange, no-replace rename, hard links, clones, extended attributes, ACLs, case-sensitive |
/// | [`windows`](Self::windows) | fixed | no-replace rename, replace with backup, hard links, ACLs, alternate streams, case-insensitive |
/// | [`overlay`](Self::overlay) | overlay | as `posix`, without clones |
///
/// Every preset has [`DEFAULT_CAPACITY`], no quota, a non-rotational medium
/// and is writable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeSpec {
    mount: AbsPath,
    fs_type: String,
    kind: VolumeKind,
    features: FsFeatures,
    rotational: bool,
    read_only: bool,
    capacity: u64,
    quota: Option<u64>,
    uuid: Option<String>,
}

impl VolumeSpec {
    fn preset(mount: AbsPath, fs_type: &str, kind: VolumeKind, features: FsFeatures) -> Self {
        Self {
            mount,
            fs_type: fs_type.to_owned(),
            kind,
            features,
            rotational: false,
            read_only: false,
            capacity: DEFAULT_CAPACITY,
            quota: None,
            uuid: None,
        }
    }

    /// Volume with the features of a POSIX file system.
    #[must_use]
    pub fn posix(mount: AbsPath) -> Self {
        Self::preset(mount, MEMFS_TYPE, VolumeKind::Fixed, POSIX_FEATURES)
    }

    /// Volume with the features of NTFS.
    #[must_use]
    pub fn windows(mount: AbsPath) -> Self {
        Self::preset(mount, MEMFS_TYPE, VolumeKind::Fixed, WINDOWS_FEATURES)
    }

    /// Overlay volume (container root) with an emulated lower layer.
    #[must_use]
    pub fn overlay(mount: AbsPath) -> Self {
        Self::preset(mount, OVERLAY_TYPE, VolumeKind::Overlay, OVERLAY_FEATURES)
    }

    /// Replaces the feature set.
    #[must_use]
    pub fn with_features(mut self, features: FsFeatures) -> Self {
        self.features = features;
        self
    }

    /// Sets the capacity in bytes.
    #[must_use]
    pub fn with_capacity(mut self, bytes: u64) -> Self {
        self.capacity = bytes;
        self
    }

    /// Sets the quota of the account in bytes.
    #[must_use]
    pub fn with_quota(mut self, bytes: u64) -> Self {
        self.quota = Some(bytes);
        self
    }

    /// Sets the volume class.
    #[must_use]
    pub fn with_kind(mut self, kind: VolumeKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets the file system type name reported by [`Fs::volume`].
    #[must_use]
    pub fn with_fs_type(mut self, fs_type: &str) -> Self {
        fs_type.clone_into(&mut self.fs_type);
        self
    }

    /// Sets the volume UUID reported in [`VolumeId::uuid`].
    #[must_use]
    pub fn with_uuid(mut self, uuid: &str) -> Self {
        self.uuid = Some(uuid.to_owned());
        self
    }

    /// Marks the medium as rotational.
    #[must_use]
    pub fn with_rotational(mut self, rotational: bool) -> Self {
        self.rotational = rotational;
        self
    }

    /// Mounts the volume read-only; every mutation fails with
    /// [`FsErrorKind::ReadOnly`].
    #[must_use]
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

/// Builder of a [`MemFs`].
///
/// Defaults: emulated system Linux, account [`DEFAULT_ACCOUNT`], a
/// [`VirtualClock`] at [`VirtualClock::DEFAULT_START`], and one
/// [`VolumeSpec::posix`] volume at the root of the host (`/`, or `C:\` on
/// Windows) when no volume is added.
pub struct MemFsBuilder {
    os: Os,
    clock: Option<Arc<dyn Clock>>,
    uid: u32,
    gid: u32,
    volumes: Vec<VolumeSpec>,
}

impl fmt::Debug for MemFsBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemFsBuilder")
            .field("os", &self.os)
            .field("uid", &self.uid)
            .field("gid", &self.gid)
            .field("volumes", &self.volumes)
            .finish_non_exhaustive()
    }
}

impl MemFsBuilder {
    /// Emulated operating system: selects the name key ([`key::name_key`]),
    /// junction support and the metadata elements that can be set.
    #[must_use]
    pub fn os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    /// Clock for timestamps.
    #[must_use]
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Account that owns new entries and whose permissions
    /// [`Fs::can_write_dir`] evaluates.
    #[must_use]
    pub fn account(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    /// Adds a volume. Volumes may be nested; a path belongs to the volume
    /// with the longest matching mount point, and mount points are created
    /// as directories in the enclosing volume.
    #[must_use]
    pub fn volume(mut self, spec: VolumeSpec) -> Self {
        self.volumes.push(spec);
        self
    }

    /// Builds the file system.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when two volumes share a mount
    /// point, and with [`FsErrorKind::NoSpace`] when a mount point does not
    /// fit in its enclosing volume.
    pub fn build(self) -> FsResult<MemFs> {
        let clock = self
            .clock
            .unwrap_or_else(|| Arc::new(VirtualClock::default()));
        let specs = if self.volumes.is_empty() {
            vec![VolumeSpec::posix(host_root()?)]
        } else {
            self.volumes
        };
        let mut state = State {
            os: self.os,
            uid: self.uid,
            gid: self.gid,
            clock,
            volumes: Vec::with_capacity(specs.len()),
            temp_counter: 0,
            faults: Vec::new(),
            holders: HashMap::new(),
            read_limits: HashMap::new(),
            counters: MemCounters::default(),
        };
        let mut mounts = HashSet::new();
        for spec in specs {
            if !mounts.insert(spec.mount.clone()) {
                return Err(FsError::new(
                    FsErrorKind::InvalidPath,
                    "mem_volume",
                    Some(spec.mount.as_path()),
                ));
            }
            state.add_volume(spec);
        }
        state.create_mount_points()?;
        Ok(MemFs {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                locks: Mutex::new(HashSet::new()),
                released: Condvar::new(),
            }),
        })
    }
}

fn host_root() -> FsResult<AbsPath> {
    AbsPath::new(if cfg!(windows) { r"C:\" } else { "/" })
}

/// Counters of operations, for assertions on the cost of an operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemCounters {
    /// Bytes written through write handles.
    pub bytes_written: u64,
    /// Bytes copied by [`FsWrite::copy_file`].
    pub bytes_copied: u64,
    /// Successful [`FsWrite::rename`] calls.
    pub renames: u64,
    /// Successful [`FsWrite::exchange`] calls.
    pub exchanges: u64,
    /// Successful [`FsWrite::replace_with_backup`] calls.
    pub replacements: u64,
    /// [`WriteHandle::sync`] calls.
    pub file_syncs: u64,
    /// [`FsWrite::sync_dir`] calls.
    pub dir_syncs: u64,
    /// Successful metadata changes ([`FsWrite::set_meta`],
    /// [`FsWrite::restore_meta`], [`WriteHandle::apply_meta`],
    /// [`FsWrite::mark_hidden`]).
    pub metadata_changes: u64,
    /// Lower-layer files copied to the upper layer of an overlay volume.
    pub copy_ups: u64,
}

/// Fault injected into a [`MemFs`].
///
/// The fault applies to calls of the operation `op` (the method name of
/// [`Fs`] or [`FsWrite`], for example `"rename"`), or of every operation when
/// `op` is `None`, on `path` as compared by the volume's name key. A fault of
/// kind [`FsErrorKind::InUse`] carries the holders configured with
/// [`MemFs::set_holders`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemFault {
    /// Path the fault applies to.
    pub path: AbsPath,
    /// Operation the fault applies to; `None` for every operation.
    pub op: Option<String>,
    /// Error kind reported.
    pub kind: FsErrorKind,
    /// Whether the fault applies to every matching call instead of only the
    /// next one.
    pub persistent: bool,
}

impl MemFault {
    /// Fault of the next call of `op` on `path`.
    #[must_use]
    pub fn once(path: AbsPath, op: &str, kind: FsErrorKind) -> Self {
        Self {
            path,
            op: Some(op.to_owned()),
            kind,
            persistent: false,
        }
    }

    /// Fault of every call of `op` on `path`, or of every operation when `op`
    /// is `None`.
    #[must_use]
    pub fn persistent(path: AbsPath, op: Option<&str>, kind: FsErrorKind) -> Self {
        Self {
            path,
            op: op.map(str::to_owned),
            kind,
            persistent: true,
        }
    }
}

/// State of one entry in a [`TreeSnapshot`].
///
/// File identity, access time and status change time are not part of the
/// snapshot: restoring an entry may legitimately change them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySnapshot {
    /// Entry kind.
    pub kind: FileKind,
    /// Logical size in bytes.
    pub size: u64,
    /// Permission bits.
    pub mode: u32,
    /// User id of the entry.
    pub uid: u32,
    /// Group id of the entry.
    pub gid: u32,
    /// Number of hard links.
    pub links: u64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub modified_ns: i64,
    /// Birth time, nanoseconds since the Unix epoch.
    pub created_ns: i64,
    /// Windows attribute bits.
    pub windows_attributes: u32,
    /// Extended attributes in name order.
    pub xattrs: Vec<Xattr>,
    /// ACL in platform form.
    pub acl: Option<Vec<u8>>,
    /// Security identifier of the account.
    pub account_sid: Option<String>,
    /// File flags.
    pub flags: Option<u64>,
    /// Content digest of a file.
    pub content: Option<ContentHash>,
    /// Alternate streams with their content digests, in name order.
    pub streams: Vec<(String, ContentHash)>,
    /// Target of a link.
    pub link_target: Option<PathBuf>,
}

/// Content digests and metadata of every entry of a tree, keyed by the path
/// relative to the tree root with `/` separators (the root itself is `""`).
///
/// Taken by [`MemFs::snapshot`] to compare the state before an operation with
/// the state after undoing it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeSnapshot {
    /// Entries by relative path.
    pub entries: BTreeMap<String, EntrySnapshot>,
}

impl TreeSnapshot {
    /// Relative paths whose entries differ between `self` and `other`,
    /// including entries present in only one of them, in byte order.
    #[must_use]
    pub fn diff(&self, other: &TreeSnapshot) -> Vec<String> {
        let paths: BTreeSet<&String> = self.entries.keys().chain(other.entries.keys()).collect();
        paths
            .into_iter()
            .filter(|path| self.entries.get(*path) != other.entries.get(*path))
            .cloned()
            .collect()
    }
}

/// Deterministic in-memory file system.
///
/// Clones share the same file system. See the [module documentation](self)
/// for the semantics.
#[derive(Clone)]
pub struct MemFs {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    locks: Mutex<HashSet<Vec<u8>>>,
    released: Condvar,
}

impl fmt::Debug for MemFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemFs").finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MemFs {
    /// File system with the default configuration of [`MemFsBuilder`].
    ///
    /// # Errors
    ///
    /// Fails as [`MemFsBuilder::build`].
    pub fn new() -> FsResult<Self> {
        Self::builder().build()
    }

    /// Builder with the default configuration.
    #[must_use]
    pub fn builder() -> MemFsBuilder {
        MemFsBuilder {
            os: Os::Linux,
            clock: None,
            uid: DEFAULT_ACCOUNT,
            gid: DEFAULT_ACCOUNT,
            volumes: Vec::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.shared.state)
    }

    fn run<T>(
        &self,
        op: &'static str,
        path: &AbsPath,
        body: impl FnOnce(&mut State, At<'_>) -> FsResult<T>,
    ) -> FsResult<T> {
        let mut state = self.state();
        state.check_fault(op, path.as_path())?;
        body(&mut state, At::new(op, path.as_path()))
    }

    /// Counters accumulated since creation or the last
    /// [`reset_counters`](Self::reset_counters).
    #[must_use]
    pub fn counters(&self) -> MemCounters {
        self.state().counters
    }

    /// Sets all counters to zero.
    pub fn reset_counters(&self) {
        self.state().counters = MemCounters::default();
    }

    /// Creates a file with `content`, creating missing parent directories.
    /// Setup bypasses [`FsWrite`] and needs no transaction scope.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Exists`] when the entry exists, with
    /// [`FsErrorKind::NotADirectory`] when a parent is not a directory, with
    /// [`FsErrorKind::NotFound`] outside every volume and with
    /// [`FsErrorKind::NoSpace`] when the content does not fit.
    pub fn seed_file(&self, path: &AbsPath, content: &[u8]) -> FsResult<()> {
        let mut state = self.state();
        let at = At::new("seed_file", path.as_path());
        let (vol, parent, name) = state.seed_parent(path, at)?;
        state.charge(vol, to_u64(content.len()), at)?;
        let blob = Blob {
            data: content.to_vec(),
            len: to_u64(content.len()),
        };
        let body = Body::File(Arc::new(blob));
        state.create_child(vol, parent, &name, body, DEFAULT_FILE_MODE, at)?;
        Ok(())
    }

    /// Creates a directory and its missing parents; an existing directory is
    /// kept.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotADirectory`] when an entry on the path is
    /// not a directory and with [`FsErrorKind::NotFound`] outside every
    /// volume.
    pub fn seed_dir(&self, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        let at = At::new("seed_dir", path.as_path());
        let vol = state.volume_index(path.as_path(), at)?;
        let names = relative_names(state.volumes[vol].info.mount.as_path(), path.as_path());
        state.create_chain(vol, &names, at).map(|_| ())
    }

    /// Creates a symbolic link to `target`, creating missing parent
    /// directories.
    ///
    /// # Errors
    ///
    /// Fails as [`seed_file`](Self::seed_file).
    pub fn seed_symlink(&self, path: &AbsPath, target: &Path) -> FsResult<()> {
        let mut state = self.state();
        let at = At::new("seed_symlink", path.as_path());
        let (vol, parent, name) = state.seed_parent(path, at)?;
        let body = Body::Link {
            target: target.to_path_buf(),
            kind: LinkKind::SymlinkFile,
        };
        state.create_child(vol, parent, &name, body, LINK_MODE, at)?;
        Ok(())
    }

    /// Marks an entry and everything below it as part of the lower layer of
    /// an overlay volume. Marked files stop being charged to the volume.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] for a missing entry and with
    /// [`FsErrorKind::Unsupported`] on a volume that is not an overlay.
    pub fn seed_lower(&self, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        let at = At::new("seed_lower", path.as_path());
        let loc = state.locate(path.as_path(), false, at)?;
        let ino = loc.existing(at)?;
        if state.volumes[loc.vol].info.kind != VolumeKind::Overlay {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        state.mark_lower(loc.vol, ino);
        Ok(())
    }

    /// Simulates another program filling or freeing the volume of `path` so
    /// that exactly `available` bytes remain free.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] outside every volume.
    pub fn set_available(&self, path: &AbsPath, available: u64) -> FsResult<()> {
        let mut state = self.state();
        let at = At::new("set_available", path.as_path());
        let vol = state.volume_index(path.as_path(), at)?;
        let volume = &mut state.volumes[vol];
        volume.external = volume
            .capacity
            .saturating_sub(volume.used)
            .saturating_sub(available);
        Ok(())
    }

    /// Sets the processes reported by [`Fs::holders`] for `path` and attached
    /// to [`FsErrorKind::InUse`] faults on it.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] outside every volume.
    pub fn set_holders(&self, path: &AbsPath, holders: Vec<Holder>) -> FsResult<()> {
        let mut state = self.state();
        let key = state.fault_key(path.as_path(), At::new("set_holders", path.as_path()))?;
        state.holders.insert(key, holders);
        Ok(())
    }

    /// Injects a fault.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] when the path lies outside every
    /// volume.
    pub fn inject(&self, fault: MemFault) -> FsResult<()> {
        let mut state = self.state();
        let key = state.fault_key(
            fault.path.as_path(),
            At::new("inject", fault.path.as_path()),
        )?;
        state.faults.push(FaultRule {
            key,
            op: fault.op,
            kind: fault.kind,
            persistent: fault.persistent,
        });
        Ok(())
    }

    /// Refuses every operation on `path` with
    /// [`FsErrorKind::PermissionDenied`]; [`Fs::can_write_dir`] reports
    /// `false` for it.
    ///
    /// # Errors
    ///
    /// Fails as [`inject`](Self::inject).
    pub fn deny_access(&self, path: &AbsPath) -> FsResult<()> {
        self.inject(MemFault::persistent(
            path.clone(),
            None,
            FsErrorKind::PermissionDenied,
        ))
    }

    /// Removes every injected fault.
    pub fn clear_faults(&self) {
        self.state().faults.clear();
    }

    /// Limits every read call of handles opened on `path` afterwards to no
    /// more than `max_bytes` bytes (short reads).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] outside every volume.
    pub fn limit_reads(&self, path: &AbsPath, max_bytes: usize) -> FsResult<()> {
        let mut state = self.state();
        let key = state.fault_key(path.as_path(), At::new("limit_reads", path.as_path()))?;
        state.read_limits.insert(key, max_bytes.max(1));
        Ok(())
    }

    /// Snapshot of the tree at `root`, without following links.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] when `root` does not exist.
    pub fn snapshot(&self, root: &AbsPath) -> FsResult<TreeSnapshot> {
        let state = self.state();
        let at = At::new("snapshot", root.as_path());
        let loc = state.locate(root.as_path(), false, at)?;
        let ino = loc.existing(at)?;
        state.snapshot(loc.vol, ino, at)
    }

    fn open_reader(
        &self,
        op: &'static str,
        path: &AbsPath,
        stream: Option<&str>,
    ) -> FsResult<Box<dyn ReadHandle>> {
        let (vol, ino, limit) = self.run(op, path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            let ino = loc.existing(at)?;
            state.require_file(loc.vol, ino, at)?;
            if let Some(name) = stream {
                state.require_streams(loc.vol, at)?;
                if !state
                    .node(loc.vol, ino, at)?
                    .meta
                    .streams
                    .contains_key(name)
                {
                    return Err(at.err(FsErrorKind::NotFound));
                }
            }
            state.open(loc.vol, ino);
            let limit = state.read_limit(path.as_path());
            Ok((loc.vol, ino, limit))
        })?;
        Ok(Box::new(MemReader {
            shared: Arc::clone(&self.shared),
            vol,
            ino,
            stream: stream.map(str::to_owned),
            position: 0,
            limit,
        }))
    }

    fn writer(
        &self,
        vol: usize,
        ino: u64,
        stream: Option<&str>,
        path: AbsPath,
    ) -> Box<dyn WriteHandle> {
        Box::new(MemWriter {
            shared: Arc::clone(&self.shared),
            vol,
            ino,
            stream: stream.map(str::to_owned),
            position: 0,
            path,
        })
    }
}

impl Fs for MemFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        self.run("metadata", path, |state, at| {
            let loc = state.locate(path.as_path(), follow == Follow::Follow, at)?;
            state.file_meta(loc.vol, loc.existing(at)?, at)
        })
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        self.run("extended_metadata", path, |state, at| {
            let loc = state.locate(path.as_path(), follow == Follow::Follow, at)?;
            state.extended_meta(loc.vol, loc.existing(at)?, at)
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        self.open_reader("open_read", path, None)
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        self.open_reader("open_stream_read", path, Some(stream))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let entries = self.run("read_dir", path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            state.list(loc.vol, loc.existing(at)?, at)
        })?;
        Ok(Box::new(entries.into_iter().map(Ok)))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        self.run("read_link", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            match &state.node(loc.vol, loc.existing(at)?, at)?.body {
                Body::Link { target, .. } => Ok(target.clone()),
                Body::File(_) | Body::Dir(_) => Err(at.err(FsErrorKind::InvalidPath)),
            }
        })
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        self.run("real_path", path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            loc.existing(at)?;
            AbsPath::new(loc.real)
        })
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        self.run("content_hash", path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            let ino = loc.existing(at)?;
            state.require_file(loc.vol, ino, at)?;
            let blob = state
                .blob(loc.vol, ino, None)
                .ok_or_else(|| at.err(FsErrorKind::Io))?;
            Ok(blob.digest())
        })
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        self.run("volume", path, |state, at| {
            let mut probe = Some(path.clone());
            while let Some(current) = probe {
                if let Ok(loc) = state.locate(current.as_path(), true, at)
                    && loc.ino.is_some()
                {
                    return Ok(state.volumes[loc.vol].info.clone());
                }
                probe = current.parent();
            }
            let vol = state.volume_index(path.as_path(), at)?;
            Ok(state.volumes[vol].info.clone())
        })
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        let state = self.state();
        state
            .volumes
            .iter()
            .find(|candidate| candidate.info.id == *volume)
            .map(Volume::space)
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "space", None))
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        self.run("path_key", path, |state, at| {
            state.fault_key(path.as_path(), at)
        })
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        self.run("holders", path, |state, at| {
            state.locate(path.as_path(), false, at)?.existing(at)?;
            Ok(state.holders_of(path.as_path()))
        })
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        let state = self.state();
        let at = At::new("can_write_dir", dir.as_path());
        let loc = state.locate(dir.as_path(), true, at)?;
        let ino = loc.existing(at)?;
        let node = state.node(loc.vol, ino, at)?;
        if !matches!(node.body, Body::Dir(_)) {
            return Err(at.err(FsErrorKind::NotADirectory));
        }
        if state.volumes[loc.vol].info.read_only || state.denied(dir.as_path()) {
            return Ok(false);
        }
        Ok(state.may_write(node))
    }
}

impl FsWrite for MemFs {
    fn create_temp_in(
        &self,
        _scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let (vol, ino, path) = self.run("create_temp_in", dir, |state, at| {
            if !valid_name(name_hint) {
                return Err(at.err(FsErrorKind::InvalidPath));
            }
            let loc = state.locate(dir.as_path(), true, at)?;
            let parent = loc.existing(at)?;
            state.writable(loc.vol, at)?;
            loop {
                state.temp_counter += 1;
                let name = format!("{name_hint}.{}", state.temp_counter);
                let key = state.key_for(loc.vol, OsStr::new(&name));
                let exists = state.entries(loc.vol, parent, at)?.contains_key(&key);
                if !exists {
                    let body = Body::File(Arc::default());
                    let ino = state.create_child(
                        loc.vol,
                        parent,
                        OsStr::new(&name),
                        body,
                        DEFAULT_FILE_MODE,
                        at,
                    )?;
                    state.open(loc.vol, ino);
                    let path = AbsPath::new(loc.real.join(&name))?;
                    return Ok((loc.vol, ino, path));
                }
            }
        })?;
        Ok(self.writer(vol, ino, None, path))
    }

    fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let (vol, ino) = self.run("create_new", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let ino = state.create_at(&loc, Body::File(Arc::default()), DEFAULT_FILE_MODE, at)?;
            state.open(loc.vol, ino);
            Ok((loc.vol, ino))
        })?;
        Ok(self.writer(vol, ino, None, path.clone()))
    }

    fn open_in_place(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let (vol, ino) = self.run("open_in_place", path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            let ino = loc.existing(at)?;
            state.require_file(loc.vol, ino, at)?;
            state.writable(loc.vol, at)?;
            state.copy_up(loc.vol, ino, at)?;
            state.open(loc.vol, ino);
            Ok((loc.vol, ino))
        })?;
        Ok(self.writer(vol, ino, None, path.clone()))
    }

    fn open_stream_write(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let (vol, ino) = self.run("open_stream_write", path, |state, at| {
            state.require_streams(state.volume_index(path.as_path(), at)?, at)?;
            if !valid_name(stream) || stream.contains(':') {
                return Err(at.err(FsErrorKind::InvalidPath));
            }
            let loc = state.locate(path.as_path(), true, at)?;
            let ino = loc.existing(at)?;
            state.require_file(loc.vol, ino, at)?;
            state.writable(loc.vol, at)?;
            state.copy_up(loc.vol, ino, at)?;
            state
                .node_mut(loc.vol, ino, at)?
                .meta
                .streams
                .entry(stream.to_owned())
                .or_default();
            state.open(loc.vol, ino);
            Ok((loc.vol, ino))
        })?;
        Ok(self.writer(vol, ino, Some(stream), path.clone()))
    }

    fn rename(
        &self,
        _scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        self.run("rename", from, |state, at| {
            state.check_fault("rename", to.as_path())?;
            state.rename(from, to, mode, at)
        })
    }

    fn exchange(&self, _scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        self.run("exchange", a, |state, at| {
            state.check_fault("exchange", b.as_path())?;
            state.exchange(a, b, at)
        })
    }

    fn replace_with_backup(
        &self,
        _scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        self.run("replace_with_backup", target, |state, at| {
            state.check_fault("replace_with_backup", replacement.as_path())?;
            state.check_fault("replace_with_backup", backup.as_path())?;
            state.replace_with_backup(target, replacement, backup, at)
        })
    }

    fn hard_link(&self, _scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        self.run("hard_link", existing, |state, at| {
            state.check_fault("hard_link", new.as_path())?;
            state.hard_link(existing, new, at)
        })
    }

    fn clone_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        self.run("clone_file", src, |state, at| {
            state.check_fault("clone_file", dst.as_path())?;
            state.clone_file(src, dst, at)
        })
    }

    fn copy_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        self.run("copy_file", src, |state, at| {
            state.check_fault("copy_file", dst.as_path())?;
            state.copy_file(src, dst, at)
        })
    }

    fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.run("remove_file", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let ino = loc.existing(at)?;
            if matches!(state.node(loc.vol, ino, at)?.body, Body::Dir(_)) {
                return Err(at.err(FsErrorKind::IsADirectory));
            }
            state.writable(loc.vol, at)?;
            state.unlink(&loc, at)
        })
    }

    fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.run("remove_dir", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let ino = loc.existing(at)?;
            if !state.entries(loc.vol, ino, at)?.is_empty() {
                return Err(at.err(FsErrorKind::NotEmpty));
            }
            state.writable(loc.vol, at)?;
            state.require_unmounted(&loc, at)?;
            state.unlink(&loc, at)
        })
    }

    fn remove_tree(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _parallelism: usize,
    ) -> FsResult<RemoveStats> {
        self.run("remove_tree", path, |state, at| state.remove_tree(path, at))
    }

    fn create_dir(&self, _scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        self.run("create_dir", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let body = Body::Dir(BTreeMap::new());
            state
                .create_at(&loc, body, mode & PERMISSION_BITS, at)
                .map(|_| ())
        })
    }

    fn create_link(
        &self,
        _scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        self.run("create_link", link, |state, at| {
            if kind == LinkKind::Junction {
                if state.os != Os::Windows {
                    return Err(at.err(FsErrorKind::Unsupported));
                }
                if !target.is_absolute() {
                    return Err(at.err(FsErrorKind::InvalidPath));
                }
            }
            let loc = state.locate(link.as_path(), false, at)?;
            let body = Body::Link {
                target: target.to_path_buf(),
                kind,
            };
            state.create_at(&loc, body, LINK_MODE, at).map(|_| ())
        })
    }

    fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        self.run("set_meta", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let ino = loc.existing(at)?;
            state.writable(loc.vol, at)?;
            state.set_meta(loc.vol, ino, change, at)
        })
    }

    fn restore_meta(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        self.run("restore_meta", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let ino = loc.existing(at)?;
            state.writable(loc.vol, at)?;
            state.apply_extended(loc.vol, ino, meta, at)
        })
    }

    fn truncate(&self, _scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        self.run("truncate", path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            let ino = loc.existing(at)?;
            state.require_file(loc.vol, ino, at)?;
            state.writable(loc.vol, at)?;
            state.copy_up(loc.vol, ino, at)?;
            state.edit(
                Target {
                    vol: loc.vol,
                    ino,
                    stream: None,
                },
                &Edit::SetLength(length),
                at,
            )
        })
    }

    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        self.run("sync_dir", dir, |state, at| {
            let loc = state.locate(dir.as_path(), true, at)?;
            state.entries(loc.vol, loc.existing(at)?, at)?;
            state.counters.dir_syncs += 1;
            Ok(())
        })
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        let key = self.run("lock_exclusive", path, |state, at| {
            let loc = state.locate(path.as_path(), true, at)?;
            match loc.ino {
                Some(ino) => state.require_file(loc.vol, ino, at)?,
                None => {
                    state.create_at(&loc, Body::File(Arc::default()), DEFAULT_FILE_MODE, at)?;
                }
            }
            state.fault_key(path.as_path(), at)
        })?;
        let deadline = Instant::now().checked_add(timeout);
        let mut held = lock(&self.shared.locks);
        while held.contains(&key.0) {
            let remaining = deadline.map_or(Duration::MAX, |deadline| {
                deadline.saturating_duration_since(Instant::now())
            });
            if remaining.is_zero() {
                return Err(FsError::new(
                    FsErrorKind::InUse,
                    "lock_exclusive",
                    Some(path.as_path()),
                ));
            }
            held = self
                .shared
                .released
                .wait_timeout(held, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        held.insert(key.0.clone());
        Ok(Box::new(MemLock {
            shared: Arc::clone(&self.shared),
            key: key.0,
            path: path.clone(),
        }))
    }

    fn mark_hidden(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.run("mark_hidden", path, |state, at| {
            let loc = state.locate(path.as_path(), false, at)?;
            let ino = loc.existing(at)?;
            state.writable(loc.vol, at)?;
            state.copy_up(loc.vol, ino, at)?;
            let os = state.os;
            let node = state.node_mut(loc.vol, ino, at)?;
            match os {
                Os::Windows => {
                    node.meta.windows_attributes |= FILE_ATTRIBUTE_HIDDEN
                        | FILE_ATTRIBUTE_SYSTEM
                        | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;
                }
                Os::Macos => {
                    node.meta.flags = Some(node.meta.flags.unwrap_or(0) | MACOS_UF_HIDDEN);
                }
                _ => {}
            }
            state.counters.metadata_changes += 1;
            Ok(())
        })
    }
}

#[derive(Clone, Copy)]
struct At<'a> {
    op: &'static str,
    path: &'a Path,
}

impl<'a> At<'a> {
    fn new(op: &'static str, path: &'a Path) -> Self {
        Self { op, path }
    }

    fn err(self, kind: FsErrorKind) -> FsError {
        FsError::new(kind, self.op, Some(self.path))
    }

    fn on(self, path: &'a Path) -> Self {
        Self { op: self.op, path }
    }
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn valid_name(name: &str) -> bool {
    !(name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']))
}

fn relative_names(mount: &Path, path: &Path) -> Vec<OsString> {
    path.strip_prefix(mount)
        .map(|rest| {
            rest.components()
                .filter_map(|component| match component {
                    Component::Normal(name) => Some(name.to_os_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn clamp_nanos(nanos: i128) -> i64 {
    i64::try_from(nanos).unwrap_or(if nanos < 0 { i64::MIN } else { i64::MAX })
}

#[derive(Debug, Clone, Default)]
struct Blob {
    data: Vec<u8>,
    len: u64,
}

impl Blob {
    fn allocated(&self) -> u64 {
        to_u64(self.data.len())
    }

    fn read_at(&self, position: u64, buf: &mut [u8], limit: Option<usize>) -> usize {
        if position >= self.len {
            return 0;
        }
        let available = usize::try_from(self.len - position).unwrap_or(usize::MAX);
        let count = buf.len().min(available).min(limit.unwrap_or(usize::MAX));
        let start = usize::try_from(position).unwrap_or(usize::MAX);
        for (offset, byte) in buf.iter_mut().take(count).enumerate() {
            *byte = start
                .checked_add(offset)
                .and_then(|index| self.data.get(index))
                .copied()
                .unwrap_or(0);
        }
        count
    }

    fn digest(&self) -> ContentHash {
        static ZEROS: [u8; HASH_CHUNK] = [0; HASH_CHUNK];
        let mut hasher = Sha256::new();
        hasher.update(&self.data);
        let zeros = &ZEROS;
        let mut tail = self.len.saturating_sub(self.allocated());
        while tail > 0 {
            let step = usize::try_from(tail).unwrap_or(HASH_CHUNK).min(HASH_CHUNK);
            hasher.update(zeros.get(..step).unwrap_or_default());
            tail -= to_u64(step);
        }
        ContentHash(hasher.finalize().into())
    }
}

type Content = Arc<Blob>;

#[derive(Debug, Clone)]
struct Slot {
    name: OsString,
    ino: u64,
}

#[derive(Debug, Clone)]
enum Body {
    File(Content),
    Dir(BTreeMap<Vec<u8>, Slot>),
    Link { target: PathBuf, kind: LinkKind },
}

#[derive(Debug, Clone)]
struct Meta {
    mode: u32,
    uid: u32,
    gid: u32,
    modified: i64,
    accessed: i64,
    changed: i64,
    created: i64,
    xattrs: BTreeMap<OsString, Vec<u8>>,
    acl: Option<Vec<u8>>,
    account_sid: Option<String>,
    flags: Option<u64>,
    windows_attributes: u32,
    streams: BTreeMap<String, Content>,
}

#[derive(Debug, Clone)]
struct Inode {
    body: Body,
    links: u64,
    open: u64,
    lower: bool,
    meta: Meta,
}

impl Inode {
    fn kind(&self) -> FileKind {
        match &self.body {
            Body::File(_) => FileKind::File,
            Body::Dir(_) => FileKind::Directory,
            Body::Link {
                kind: LinkKind::Junction,
                ..
            } => FileKind::Junction,
            Body::Link { .. } => FileKind::Symlink,
        }
    }

    fn size(&self) -> u64 {
        match &self.body {
            Body::File(content) => content.len,
            Body::Dir(_) => 0,
            Body::Link { target, .. } => to_u64(target.as_os_str().len()),
        }
    }

    fn blobs(&self) -> impl Iterator<Item = &Content> {
        let content = match &self.body {
            Body::File(content) => Some(content),
            Body::Dir(_) | Body::Link { .. } => None,
        };
        content.into_iter().chain(self.meta.streams.values())
    }

    /// Bytes charged to this inode that no other inode shares.
    fn exclusive_bytes(&self) -> u64 {
        self.blobs()
            .filter(|blob| Arc::strong_count(blob) == 1)
            .map(|blob| blob.allocated())
            .sum()
    }

    fn allocated(&self) -> u64 {
        self.blobs().map(|blob| blob.allocated()).sum()
    }
}

struct Volume {
    info: VolumeInfo,
    capacity: u64,
    quota: Option<u64>,
    used: u64,
    external: u64,
    next_ino: u64,
    root: u64,
    inodes: HashMap<u64, Inode>,
}

impl Volume {
    fn space(&self) -> SpaceInfo {
        let free = self
            .capacity
            .saturating_sub(self.used.saturating_add(self.external));
        let quota_remaining = self.quota.map(|quota| quota.saturating_sub(self.used));
        SpaceInfo {
            total: self
                .quota
                .map_or(self.capacity, |quota| quota.min(self.capacity)),
            available: quota_remaining.map_or(free, |remaining| remaining.min(free)),
            quota_remaining,
            quota_limit: self.quota,
            purgeable_system: None,
        }
    }
}

#[derive(Debug, Clone)]
struct FaultRule {
    key: PathKey,
    op: Option<String>,
    kind: FsErrorKind,
    persistent: bool,
}

struct State {
    os: Os,
    uid: u32,
    gid: u32,
    clock: Arc<dyn Clock>,
    volumes: Vec<Volume>,
    temp_counter: u64,
    faults: Vec<FaultRule>,
    holders: HashMap<PathKey, Vec<Holder>>,
    read_limits: HashMap<PathKey, usize>,
    counters: MemCounters,
}

/// Result of resolving a path: the entry, or the place where it would be
/// created.
struct Loc {
    vol: usize,
    parent: Option<u64>,
    key: Vec<u8>,
    name: OsString,
    ino: Option<u64>,
    real: PathBuf,
}

impl Loc {
    fn existing(&self, at: At<'_>) -> FsResult<u64> {
        self.ino.ok_or_else(|| at.err(FsErrorKind::NotFound))
    }

    fn parent(&self, at: At<'_>) -> FsResult<u64> {
        self.parent.ok_or_else(|| at.err(FsErrorKind::InUse))
    }
}

enum Walk {
    Found(Loc),
    Redirect(PathBuf),
}

enum Step {
    Into(u64, OsString),
    Redirect(PathBuf),
}

#[derive(Clone, Copy)]
struct Target<'a> {
    vol: usize,
    ino: u64,
    stream: Option<&'a str>,
}

enum Edit<'a> {
    Write { offset: u64, bytes: &'a [u8] },
    SetLength(u64),
}

impl Edit<'_> {
    fn data_len_after(&self, data_len: usize, at: At<'_>) -> FsResult<usize> {
        match self {
            Self::Write { offset, bytes } => usize::try_from(*offset)
                .ok()
                .and_then(|start| start.checked_add(bytes.len()))
                .map(|end| end.max(data_len))
                .ok_or_else(|| at.err(FsErrorKind::NoSpace)),
            Self::SetLength(length) => {
                Ok(usize::try_from(*length).map_or(data_len, |length| length.min(data_len)))
            }
        }
    }

    fn apply(&self, blob: &mut Blob) {
        match self {
            Self::Write { offset, bytes } => {
                let start = usize::try_from(*offset).unwrap_or(usize::MAX);
                let end = start.saturating_add(bytes.len());
                if blob.data.len() < end {
                    blob.data.resize(end, 0);
                }
                if let Some(range) = blob.data.get_mut(start..end) {
                    range.copy_from_slice(bytes);
                }
                blob.len = blob.len.max(to_u64(end));
            }
            Self::SetLength(length) => {
                let keep = usize::try_from(*length).unwrap_or(usize::MAX);
                blob.data.truncate(keep);
                blob.len = *length;
            }
        }
    }
}

impl State {
    fn now(&self) -> i64 {
        clamp_nanos(self.clock.now().unix_nanos())
    }

    fn add_volume(&mut self, spec: VolumeSpec) {
        let dev = to_u64(self.volumes.len()) + 1;
        let now = self.now();
        let mut inodes = HashMap::new();
        inodes.insert(
            1,
            Inode {
                body: Body::Dir(BTreeMap::new()),
                links: 1,
                open: 0,
                lower: false,
                meta: self.new_meta(DEFAULT_DIR_MODE, now),
            },
        );
        self.volumes.push(Volume {
            info: VolumeInfo {
                id: VolumeId {
                    dev,
                    uuid: spec.uuid,
                    subvolume: 0,
                },
                mount: spec.mount,
                fs_type: spec.fs_type,
                kind: spec.kind,
                features: spec.features,
                rotational: spec.rotational,
                read_only: spec.read_only,
            },
            capacity: spec.capacity,
            quota: spec.quota,
            used: 0,
            external: 0,
            next_ino: 2,
            root: 1,
            inodes,
        });
    }

    fn create_mount_points(&mut self) -> FsResult<()> {
        for index in 0..self.volumes.len() {
            let mount = self.volumes[index].info.mount.clone();
            let enclosing = mount
                .parent()
                .and_then(|parent| self.volume_of(parent.as_path()));
            if let Some(outer) = enclosing {
                let names =
                    relative_names(self.volumes[outer].info.mount.as_path(), mount.as_path());
                self.create_chain(outer, &names, At::new("mem_volume", mount.as_path()))?;
            }
        }
        Ok(())
    }

    fn new_meta(&self, mode: u32, now: i64) -> Meta {
        Meta {
            mode,
            uid: self.uid,
            gid: self.gid,
            modified: now,
            accessed: now,
            changed: now,
            created: now,
            xattrs: BTreeMap::new(),
            acl: None,
            account_sid: None,
            flags: None,
            windows_attributes: 0,
            streams: BTreeMap::new(),
        }
    }

    fn volume_of(&self, path: &Path) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        for (index, volume) in self.volumes.iter().enumerate() {
            let mount = volume.info.mount.as_path();
            if path.starts_with(mount) {
                let depth = mount.components().count();
                if best.is_none_or(|(_, deepest)| depth > deepest) {
                    best = Some((index, depth));
                }
            }
        }
        best.map(|(index, _)| index)
    }

    fn volume_index(&self, path: &Path, at: At<'_>) -> FsResult<usize> {
        self.volume_of(path)
            .ok_or_else(|| at.err(FsErrorKind::NotFound))
    }

    fn features(&self, vol: usize) -> FsFeatures {
        self.volumes[vol].info.features
    }

    fn key_for(&self, vol: usize, name: &OsStr) -> Vec<u8> {
        key::name_key(self.os, self.features(vol).case_sensitive, name)
    }

    fn fault_key(&self, path: &Path, at: At<'_>) -> FsResult<PathKey> {
        let vol = self.volume_index(path, at)?;
        Ok(key::path_key(
            self.os,
            self.features(vol).case_sensitive,
            path,
        ))
    }

    fn check_fault(&mut self, op: &'static str, path: &Path) -> FsResult<()> {
        if self.faults.is_empty() {
            return Ok(());
        }
        let Ok(key) = self.fault_key(path, At::new(op, path)) else {
            return Ok(());
        };
        let matching = self
            .faults
            .iter()
            .position(|rule| rule.key == key && rule.op.as_deref().is_none_or(|name| name == op));
        let Some(index) = matching else {
            return Ok(());
        };
        let rule = if self.faults[index].persistent {
            self.faults[index].clone()
        } else {
            self.faults.remove(index)
        };
        let mut error = FsError::new(rule.kind, op, Some(path));
        if rule.kind == FsErrorKind::InUse {
            error.holders = self.holders.get(&key).cloned().unwrap_or_default();
        }
        Err(error)
    }

    fn denied(&self, path: &Path) -> bool {
        let Ok(key) = self.fault_key(path, At::new("can_write_dir", path)) else {
            return false;
        };
        self.faults.iter().any(|rule| {
            rule.key == key && rule.kind == FsErrorKind::PermissionDenied && rule.op.is_none()
        })
    }

    fn holders_of(&self, path: &Path) -> Vec<Holder> {
        self.fault_key(path, At::new("holders", path))
            .ok()
            .and_then(|key| self.holders.get(&key).cloned())
            .unwrap_or_default()
    }

    fn read_limit(&self, path: &Path) -> Option<usize> {
        self.fault_key(path, At::new("open_read", path))
            .ok()
            .and_then(|key| self.read_limits.get(&key).copied())
    }

    fn may_write(&self, node: &Inode) -> bool {
        if self.uid == ROOT_ACCOUNT {
            return true;
        }
        let shift = if node.meta.uid == self.uid {
            OWNER_SHIFT
        } else if node.meta.gid == self.gid {
            GROUP_SHIFT
        } else {
            0
        };
        (node.meta.mode >> shift) & WRITE_AND_SEARCH == WRITE_AND_SEARCH
    }

    fn inode(&self, vol: usize, ino: u64) -> Option<&Inode> {
        self.volumes.get(vol)?.inodes.get(&ino)
    }

    fn node(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<&Inode> {
        self.inode(vol, ino).ok_or_else(|| at.err(FsErrorKind::Io))
    }

    fn node_mut(&mut self, vol: usize, ino: u64, at: At<'_>) -> FsResult<&mut Inode> {
        self.volumes
            .get_mut(vol)
            .and_then(|volume| volume.inodes.get_mut(&ino))
            .ok_or_else(|| at.err(FsErrorKind::Io))
    }

    fn entries(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<&BTreeMap<Vec<u8>, Slot>> {
        match &self.node(vol, ino, at)?.body {
            Body::Dir(entries) => Ok(entries),
            Body::File(_) | Body::Link { .. } => Err(at.err(FsErrorKind::NotADirectory)),
        }
    }

    fn entries_mut(
        &mut self,
        vol: usize,
        ino: u64,
        at: At<'_>,
    ) -> FsResult<&mut BTreeMap<Vec<u8>, Slot>> {
        match &mut self.node_mut(vol, ino, at)?.body {
            Body::Dir(entries) => Ok(entries),
            Body::File(_) | Body::Link { .. } => Err(at.err(FsErrorKind::NotADirectory)),
        }
    }

    fn blob(&self, vol: usize, ino: u64, stream: Option<&str>) -> Option<&Content> {
        let node = self.inode(vol, ino)?;
        match stream {
            Some(name) => node.meta.streams.get(name),
            None => match &node.body {
                Body::File(content) => Some(content),
                Body::Dir(_) | Body::Link { .. } => None,
            },
        }
    }

    fn require_file(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<()> {
        match self.node(vol, ino, at)?.body {
            Body::File(_) => Ok(()),
            Body::Dir(_) => Err(at.err(FsErrorKind::IsADirectory)),
            Body::Link { .. } => Err(at.err(FsErrorKind::InvalidPath)),
        }
    }

    fn require_streams(&self, vol: usize, at: At<'_>) -> FsResult<()> {
        if self.features(vol).alternate_streams {
            Ok(())
        } else {
            Err(at.err(FsErrorKind::Unsupported))
        }
    }

    fn writable(&self, vol: usize, at: At<'_>) -> FsResult<()> {
        if self.volumes[vol].info.read_only {
            Err(at.err(FsErrorKind::ReadOnly))
        } else {
            Ok(())
        }
    }

    fn locate(&self, path: &Path, follow: bool, at: At<'_>) -> FsResult<Loc> {
        let mut current = path.to_path_buf();
        for _ in 0..=MAX_LINK_HOPS {
            match self.walk(&current, follow, at)? {
                Walk::Found(loc) => return Ok(loc),
                Walk::Redirect(next) => current = next,
            }
        }
        Err(at.err(FsErrorKind::Loop))
    }

    fn walk(&self, path: &Path, follow: bool, at: At<'_>) -> FsResult<Walk> {
        let vol = self.volume_index(path, at)?;
        let volume = &self.volumes[vol];
        let names = relative_names(volume.info.mount.as_path(), path);
        let mut real = volume.info.mount.as_path().to_path_buf();
        let mut dir = volume.root;
        let Some((last, parents)) = names.split_last() else {
            return Ok(Walk::Found(Loc {
                vol,
                parent: None,
                key: Vec::new(),
                name: OsString::new(),
                ino: Some(dir),
                real,
            }));
        };
        for (index, name) in parents.iter().enumerate() {
            let rest = names.get(index + 1..).unwrap_or_default();
            match self.step(vol, dir, name, &real, rest, at)? {
                Step::Into(child, stored) => {
                    real.push(stored);
                    dir = child;
                }
                Step::Redirect(next) => return Ok(Walk::Redirect(next)),
            }
        }
        self.finish(vol, dir, last, &real, follow, at)
    }

    fn step(
        &self,
        vol: usize,
        dir: u64,
        name: &OsStr,
        real: &Path,
        rest: &[OsString],
        at: At<'_>,
    ) -> FsResult<Step> {
        let key = self.key_for(vol, name);
        let slot = self
            .entries(vol, dir, at)?
            .get(&key)
            .ok_or_else(|| at.err(FsErrorKind::NotFound))?;
        if let Body::Link { target, .. } = &self.node(vol, slot.ino, at)?.body {
            return Ok(Step::Redirect(redirect(real, target, rest, at)?));
        }
        Ok(Step::Into(slot.ino, slot.name.clone()))
    }

    fn finish(
        &self,
        vol: usize,
        dir: u64,
        name: &OsStr,
        real: &Path,
        follow: bool,
        at: At<'_>,
    ) -> FsResult<Walk> {
        let key = self.key_for(vol, name);
        let Some(slot) = self.entries(vol, dir, at)?.get(&key) else {
            return Ok(Walk::Found(Loc {
                vol,
                parent: Some(dir),
                key,
                name: name.to_os_string(),
                ino: None,
                real: real.join(name),
            }));
        };
        if follow && let Body::Link { target, .. } = &self.node(vol, slot.ino, at)?.body {
            return Ok(Walk::Redirect(redirect(real, target, &[], at)?));
        }
        Ok(Walk::Found(Loc {
            vol,
            parent: Some(dir),
            key,
            name: slot.name.clone(),
            ino: Some(slot.ino),
            real: real.join(&slot.name),
        }))
    }

    fn alloc(&mut self, vol: usize, body: Body, mode: u32) -> u64 {
        let now = self.now();
        let meta = self.new_meta(mode, now);
        let volume = &mut self.volumes[vol];
        let ino = volume.next_ino;
        volume.next_ino += 1;
        volume.inodes.insert(
            ino,
            Inode {
                body,
                links: 1,
                open: 0,
                lower: false,
                meta,
            },
        );
        ino
    }

    fn touch_dir(&mut self, vol: usize, dir: u64) {
        let now = self.now();
        if let Some(node) = self.volumes[vol].inodes.get_mut(&dir) {
            node.meta.modified = now;
            node.meta.changed = now;
        }
    }

    fn insert_entry(
        &mut self,
        vol: usize,
        parent: u64,
        key: Vec<u8>,
        name: OsString,
        ino: u64,
        at: At<'_>,
    ) -> FsResult<()> {
        self.entries_mut(vol, parent, at)?
            .insert(key, Slot { name, ino });
        self.touch_dir(vol, parent);
        Ok(())
    }

    fn remove_entry(&mut self, vol: usize, parent: u64, key: &[u8], at: At<'_>) -> FsResult<Slot> {
        let slot = self
            .entries_mut(vol, parent, at)?
            .remove(key)
            .ok_or_else(|| at.err(FsErrorKind::NotFound))?;
        self.touch_dir(vol, parent);
        Ok(slot)
    }

    fn create_child(
        &mut self,
        vol: usize,
        parent: u64,
        name: &OsStr,
        body: Body,
        mode: u32,
        at: At<'_>,
    ) -> FsResult<u64> {
        let key = self.key_for(vol, name);
        if self.entries(vol, parent, at)?.contains_key(&key) {
            return Err(at.err(FsErrorKind::Exists));
        }
        let ino = self.alloc(vol, body, mode);
        self.insert_entry(vol, parent, key, name.to_os_string(), ino, at)?;
        Ok(ino)
    }

    fn create_at(&mut self, loc: &Loc, body: Body, mode: u32, at: At<'_>) -> FsResult<u64> {
        if loc.ino.is_some() {
            return Err(at.err(FsErrorKind::Exists));
        }
        let parent = loc.parent(at)?;
        self.writable(loc.vol, at)?;
        let ino = self.alloc(loc.vol, body, mode);
        self.insert_entry(loc.vol, parent, loc.key.clone(), loc.name.clone(), ino, at)?;
        Ok(ino)
    }

    fn create_chain(&mut self, vol: usize, names: &[OsString], at: At<'_>) -> FsResult<u64> {
        let mut dir = self.volumes[vol].root;
        for name in names {
            let key = self.key_for(vol, name);
            let existing = self.entries(vol, dir, at)?.get(&key).map(|slot| slot.ino);
            dir = if let Some(ino) = existing {
                self.entries(vol, ino, at)?;
                ino
            } else {
                let body = Body::Dir(BTreeMap::new());
                self.create_child(vol, dir, name, body, DEFAULT_DIR_MODE, at)?
            };
        }
        Ok(dir)
    }

    fn seed_parent(&mut self, path: &AbsPath, at: At<'_>) -> FsResult<(usize, u64, OsString)> {
        let vol = self.volume_index(path.as_path(), at)?;
        let names = relative_names(self.volumes[vol].info.mount.as_path(), path.as_path());
        let (name, parents) = names
            .split_last()
            .ok_or_else(|| at.err(FsErrorKind::Exists))?;
        let parent = self.create_chain(vol, parents, at)?;
        Ok((vol, parent, name.clone()))
    }

    fn mark_lower(&mut self, vol: usize, ino: u64) {
        let mut pending = vec![ino];
        while let Some(current) = pending.pop() {
            let Some(node) = self.volumes[vol].inodes.get(&current) else {
                continue;
            };
            if let Body::Dir(entries) = &node.body {
                pending.extend(entries.values().map(|slot| slot.ino));
            }
            let freed = if node.lower {
                0
            } else {
                node.exclusive_bytes()
            };
            let volume = &mut self.volumes[vol];
            volume.used = volume.used.saturating_sub(freed);
            if let Some(node) = volume.inodes.get_mut(&current) {
                node.lower = true;
            }
        }
    }

    fn charge(&mut self, vol: usize, bytes: u64, at: At<'_>) -> FsResult<()> {
        if bytes == 0 {
            return Ok(());
        }
        let volume = &mut self.volumes[vol];
        let free = volume
            .capacity
            .saturating_sub(volume.used.saturating_add(volume.external));
        if bytes > free {
            return Err(at.err(FsErrorKind::NoSpace));
        }
        if let Some(quota) = volume.quota
            && volume.used.saturating_add(bytes) > quota
        {
            return Err(at.err(FsErrorKind::QuotaExceeded));
        }
        volume.used += bytes;
        Ok(())
    }

    fn release(&mut self, vol: usize, bytes: u64) {
        let volume = &mut self.volumes[vol];
        volume.used = volume.used.saturating_sub(bytes);
    }

    fn copy_up(&mut self, vol: usize, ino: u64, at: At<'_>) -> FsResult<()> {
        let node = self.node(vol, ino, at)?;
        if !node.lower || !matches!(node.body, Body::File(_)) {
            return Ok(());
        }
        let bytes = node.allocated();
        self.charge(vol, bytes, at)?;
        self.node_mut(vol, ino, at)?.lower = false;
        self.counters.copy_ups += 1;
        Ok(())
    }

    fn open(&mut self, vol: usize, ino: u64) {
        if let Some(node) = self.volumes[vol].inodes.get_mut(&ino) {
            node.open += 1;
        }
    }

    fn close(&mut self, vol: usize, ino: u64) {
        if let Some(node) = self.volumes[vol].inodes.get_mut(&ino) {
            node.open = node.open.saturating_sub(1);
        }
        self.release_if_unused(vol, ino);
    }

    fn drop_link(&mut self, vol: usize, ino: u64) {
        let now = self.now();
        if let Some(node) = self.volumes[vol].inodes.get_mut(&ino) {
            node.links = node.links.saturating_sub(1);
            node.meta.changed = now;
        }
        self.release_if_unused(vol, ino);
    }

    fn release_if_unused(&mut self, vol: usize, ino: u64) {
        let volume = &mut self.volumes[vol];
        let unused = volume
            .inodes
            .get(&ino)
            .is_some_and(|node| node.links == 0 && node.open == 0);
        if !unused {
            return;
        }
        if let Some(node) = volume.inodes.remove(&ino) {
            let freed = if node.lower {
                0
            } else {
                node.exclusive_bytes()
            };
            volume.used = volume.used.saturating_sub(freed);
        }
    }

    fn edit(&mut self, target: Target<'_>, edit: &Edit<'_>, at: At<'_>) -> FsResult<()> {
        let content = self
            .blob(target.vol, target.ino, target.stream)
            .ok_or_else(|| at.err(FsErrorKind::NotFound))?;
        let data_len = content.data.len();
        let shared = Arc::strong_count(content) > 1;
        let new_len = edit.data_len_after(data_len, at)?;
        let charge = if shared {
            new_len
        } else {
            new_len.saturating_sub(data_len)
        };
        self.charge(target.vol, to_u64(charge), at)?;
        if !shared {
            self.release(target.vol, to_u64(data_len.saturating_sub(new_len)));
        }
        let now = self.now();
        let node = self.node_mut(target.vol, target.ino, at)?;
        let content = match target.stream {
            Some(name) => node.meta.streams.get_mut(name),
            None => match &mut node.body {
                Body::File(content) => Some(content),
                Body::Dir(_) | Body::Link { .. } => None,
            },
        }
        .ok_or_else(|| at.err(FsErrorKind::NotFound))?;
        edit.apply(Arc::make_mut(content));
        node.meta.modified = now;
        node.meta.changed = now;
        Ok(())
    }

    fn file_meta(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<FileMeta> {
        let node = self.node(vol, ino, at)?;
        let unix = self.os != Os::Windows;
        Ok(FileMeta {
            kind: node.kind(),
            size: node.size(),
            mode: node.meta.mode,
            uid: unix.then_some(node.meta.uid),
            gid: unix.then_some(node.meta.gid),
            links: node.links,
            modified_ns: node.meta.modified,
            accessed_ns: Some(node.meta.accessed),
            created_ns: Some(node.meta.created),
            changed_ns: Some(node.meta.changed),
            file_id: FileId {
                volume: self.volumes[vol].info.id.dev,
                low: ino,
                high: 0,
            },
            windows_attributes: node.meta.windows_attributes,
        })
    }

    fn extended_meta(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<ExtendedMeta> {
        let basic = self.file_meta(vol, ino, at)?;
        let node = self.node(vol, ino, at)?;
        Ok(ExtendedMeta {
            basic,
            xattrs: node
                .meta
                .xattrs
                .iter()
                .map(|(name, value)| Xattr {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
            acl: node.meta.acl.clone(),
            account_sid: node.meta.account_sid.clone(),
            flags: node.meta.flags,
            streams: node
                .meta
                .streams
                .iter()
                .map(|(name, content)| AlternateStream {
                    name: name.clone(),
                    size: content.len,
                })
                .collect(),
            link_target: match &node.body {
                Body::Link { target, .. } => Some(target.clone()),
                Body::File(_) | Body::Dir(_) => None,
            },
        })
    }

    fn list(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<Vec<DirEntry>> {
        let mut out: Vec<DirEntry> = self
            .entries(vol, ino, at)?
            .values()
            .map(|slot| DirEntry {
                name: slot.name.clone(),
                kind: self.inode(vol, slot.ino).map(Inode::kind),
            })
            .collect();
        out.sort_by(|a, b| a.name.as_encoded_bytes().cmp(b.name.as_encoded_bytes()));
        Ok(out)
    }

    fn require_unmounted(&self, loc: &Loc, at: At<'_>) -> FsResult<()> {
        let mounted = self
            .volumes
            .iter()
            .any(|volume| volume.info.mount.as_path() == loc.real);
        if mounted || loc.parent.is_none() {
            Err(at.err(FsErrorKind::InUse))
        } else {
            Ok(())
        }
    }

    fn unlink(&mut self, loc: &Loc, at: At<'_>) -> FsResult<()> {
        let parent = loc.parent(at)?;
        let slot = self.remove_entry(loc.vol, parent, &loc.key, at)?;
        self.drop_link(loc.vol, slot.ino);
        Ok(())
    }

    fn is_dir(&self, vol: usize, ino: u64) -> bool {
        self.inode(vol, ino)
            .is_some_and(|node| matches!(node.body, Body::Dir(_)))
    }

    fn move_guard(&mut self, vol: usize, ino: u64, at: At<'_>) -> FsResult<()> {
        let node = self.node(vol, ino, at)?;
        if node.lower && matches!(node.body, Body::Dir(_)) {
            return Err(at.err(FsErrorKind::CrossDevice));
        }
        self.copy_up(vol, ino, at)
    }

    fn rename(
        &mut self,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
        at: At<'_>,
    ) -> FsResult<()> {
        let src = self.locate(from.as_path(), false, at)?;
        let src_ino = src.existing(at)?;
        self.require_unmounted(&src, at)?;
        let dst = self.locate(to.as_path(), false, at.on(to.as_path()))?;
        let dst_parent = dst.parent(at.on(to.as_path()))?;
        if dst.vol != src.vol {
            return Err(at.err(FsErrorKind::CrossDevice));
        }
        self.writable(src.vol, at)?;
        if mode == RenameMode::NoReplace && !self.features(src.vol).rename_noreplace {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        if self.is_dir(src.vol, src_ino) && dst.real.starts_with(&src.real) && dst.real != src.real
        {
            return Err(at.err(FsErrorKind::InvalidPath));
        }
        if let Some(dst_ino) = dst.ino {
            if dst_ino == src_ino {
                let spelling = to.file_name().unwrap_or_default();
                return self.respell(&src, &dst, spelling, at);
            }
            if mode == RenameMode::NoReplace {
                return Err(at.on(to.as_path()).err(FsErrorKind::Exists));
            }
            self.check_replace(src.vol, src_ino, dst_ino, at.on(to.as_path()))?;
        }
        self.move_guard(src.vol, src_ino, at)?;
        if dst.ino.is_some() {
            self.require_unmounted(&dst, at.on(to.as_path()))?;
            self.unlink(&dst, at.on(to.as_path()))?;
        }
        let slot = self.remove_entry(src.vol, src.parent(at)?, &src.key, at)?;
        self.insert_entry(src.vol, dst_parent, dst.key, dst.name, slot.ino, at)?;
        self.counters.renames += 1;
        Ok(())
    }

    fn respell(&mut self, src: &Loc, dst: &Loc, spelling: &OsStr, at: At<'_>) -> FsResult<()> {
        if src.parent == dst.parent && src.key == dst.key {
            let parent = src.parent(at)?;
            if let Some(slot) = self.entries_mut(src.vol, parent, at)?.get_mut(&src.key) {
                spelling.clone_into(&mut slot.name);
            }
        }
        self.counters.renames += 1;
        Ok(())
    }

    fn check_replace(&self, vol: usize, src: u64, dst: u64, at: At<'_>) -> FsResult<()> {
        match (self.is_dir(vol, src), self.is_dir(vol, dst)) {
            (true, false) => Err(at.err(FsErrorKind::NotADirectory)),
            (false, true) => Err(at.err(FsErrorKind::IsADirectory)),
            (true, true) if !self.entries(vol, dst, at)?.is_empty() => {
                Err(at.err(FsErrorKind::NotEmpty))
            }
            _ => Ok(()),
        }
    }

    fn exchange(&mut self, a: &AbsPath, b: &AbsPath, at: At<'_>) -> FsResult<()> {
        let first = self.locate(a.as_path(), false, at)?;
        let first_ino = first.existing(at)?;
        let second = self.locate(b.as_path(), false, at.on(b.as_path()))?;
        let second_ino = second.existing(at.on(b.as_path()))?;
        if first.vol != second.vol {
            return Err(at.err(FsErrorKind::CrossDevice));
        }
        if !self.features(first.vol).exchange {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        self.writable(first.vol, at)?;
        if first_ino == second_ino {
            return Ok(());
        }
        if first.real.starts_with(&second.real) || second.real.starts_with(&first.real) {
            return Err(at.err(FsErrorKind::InvalidPath));
        }
        self.require_unmounted(&first, at)?;
        self.require_unmounted(&second, at.on(b.as_path()))?;
        self.move_guard(first.vol, first_ino, at)?;
        self.move_guard(first.vol, second_ino, at.on(b.as_path()))?;
        self.set_slot(first.vol, first.parent(at)?, &first.key, second_ino, at)?;
        self.set_slot(first.vol, second.parent(at)?, &second.key, first_ino, at)?;
        self.counters.exchanges += 1;
        Ok(())
    }

    fn set_slot(
        &mut self,
        vol: usize,
        parent: u64,
        key: &[u8],
        ino: u64,
        at: At<'_>,
    ) -> FsResult<()> {
        let slot = self
            .entries_mut(vol, parent, at)?
            .get_mut(key)
            .ok_or_else(|| at.err(FsErrorKind::NotFound))?;
        slot.ino = ino;
        self.touch_dir(vol, parent);
        Ok(())
    }

    fn existing_file(&self, path: &AbsPath, at: At<'_>) -> FsResult<Loc> {
        let at = at.on(path.as_path());
        let loc = self.locate(path.as_path(), false, at)?;
        let ino = loc.existing(at)?;
        if self.is_dir(loc.vol, ino) {
            return Err(at.err(FsErrorKind::IsADirectory));
        }
        Ok(loc)
    }

    /// Replaces `target` like `ReplaceFileW`: the replacement takes the
    /// target name and inherits its birth time, Windows attributes, ACL,
    /// account and the alternate streams it lacks; the previous target keeps
    /// its inode under the backup name, replacing an existing backup.
    fn replace_with_backup(
        &mut self,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
        at: At<'_>,
    ) -> FsResult<()> {
        let old = self.existing_file(target, at)?;
        if !self.features(old.vol).replace_with_backup {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        let new = self.existing_file(replacement, at)?;
        let spare = self.locate(backup.as_path(), false, at.on(backup.as_path()))?;
        if new.vol != old.vol || spare.vol != old.vol {
            return Err(at.err(FsErrorKind::CrossDevice));
        }
        self.writable(old.vol, at)?;
        let (old_ino, new_ino) = (old.existing(at)?, new.existing(at)?);
        if spare.ino.is_some() {
            self.existing_file(backup, at)?;
            self.unlink(&spare, at.on(backup.as_path()))?;
        }
        self.inherit(old.vol, old_ino, new_ino, at)?;
        let backup_loc = self.locate(backup.as_path(), false, at.on(backup.as_path()))?;
        self.remove_entry(new.vol, new.parent(at)?, &new.key, at)?;
        self.set_slot(old.vol, old.parent(at)?, &old.key, new_ino, at)?;
        let backup_parent = backup_loc.parent(at)?;
        self.insert_entry(
            old.vol,
            backup_parent,
            backup_loc.key,
            backup_loc.name,
            old_ino,
            at,
        )?;
        self.counters.replacements += 1;
        Ok(())
    }

    fn inherit(&mut self, vol: usize, from: u64, to: u64, at: At<'_>) -> FsResult<()> {
        let source = self.node(vol, from, at)?.meta.clone();
        let node = self.node_mut(vol, to, at)?;
        node.meta.created = source.created;
        node.meta.windows_attributes = source.windows_attributes;
        node.meta.acl = source.acl;
        node.meta.account_sid = source.account_sid;
        for (name, content) in source.streams {
            node.meta.streams.entry(name).or_insert(content);
        }
        Ok(())
    }

    fn hard_link(&mut self, existing: &AbsPath, new: &AbsPath, at: At<'_>) -> FsResult<()> {
        let src = self.locate(existing.as_path(), false, at)?;
        let ino = src.existing(at)?;
        if !self.features(src.vol).hardlink {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        if self.is_dir(src.vol, ino) {
            return Err(at.err(FsErrorKind::PermissionDenied));
        }
        let dst = self.locate(new.as_path(), false, at.on(new.as_path()))?;
        if dst.vol != src.vol {
            return Err(at.err(FsErrorKind::CrossDevice));
        }
        if dst.ino.is_some() {
            return Err(at.on(new.as_path()).err(FsErrorKind::Exists));
        }
        self.writable(src.vol, at)?;
        self.copy_up(src.vol, ino, at)?;
        let parent = dst.parent(at)?;
        self.insert_entry(dst.vol, parent, dst.key, dst.name, ino, at)?;
        let now = self.now();
        let node = self.node_mut(src.vol, ino, at)?;
        node.links += 1;
        node.meta.changed = now;
        Ok(())
    }

    fn new_file_at(&self, path: &AbsPath, at: At<'_>) -> FsResult<Loc> {
        let loc = self.locate(path.as_path(), false, at.on(path.as_path()))?;
        if loc.ino.is_some() {
            return Err(at.on(path.as_path()).err(FsErrorKind::Exists));
        }
        self.writable(loc.vol, at)?;
        Ok(loc)
    }

    fn clone_file(&mut self, src: &AbsPath, dst: &AbsPath, at: At<'_>) -> FsResult<()> {
        let source = self.locate(src.as_path(), true, at)?;
        let ino = source.existing(at)?;
        if !self.features(source.vol).reflink {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        self.require_file(source.vol, ino, at)?;
        let target = self.new_file_at(dst, at)?;
        let node = self.node(source.vol, ino, at)?;
        if target.vol != source.vol || node.lower {
            return Err(at.err(FsErrorKind::CrossDevice));
        }
        let (body, mode) = (node.body.clone(), node.meta.mode);
        self.create_at(&target, body, mode, at)?;
        Ok(())
    }

    fn copy_file(&mut self, src: &AbsPath, dst: &AbsPath, at: At<'_>) -> FsResult<u64> {
        let source = self.locate(src.as_path(), true, at)?;
        let ino = source.existing(at)?;
        self.require_file(source.vol, ino, at)?;
        let target = self.new_file_at(dst, at)?;
        let node = self.node(source.vol, ino, at)?;
        let mode = node.meta.mode;
        let content = self
            .blob(source.vol, ino, None)
            .map(|blob| Blob::clone(blob))
            .unwrap_or_default();
        let keep_streams = self.features(target.vol).alternate_streams;
        let streams: BTreeMap<String, Content> = if keep_streams {
            node.meta
                .streams
                .iter()
                .map(|(name, blob)| (name.clone(), Arc::new(Blob::clone(blob))))
                .collect()
        } else {
            BTreeMap::new()
        };
        let charged =
            content.allocated() + streams.values().map(|blob| blob.allocated()).sum::<u64>();
        let copied = content.len + streams.values().map(|blob| blob.len).sum::<u64>();
        self.charge(target.vol, charged, at.on(dst.as_path()))?;
        let created = self.create_at(&target, Body::File(Arc::new(content)), mode, at)?;
        self.node_mut(target.vol, created, at)?.meta.streams = streams;
        self.counters.bytes_copied += copied;
        Ok(copied)
    }

    fn remove_tree(&mut self, path: &AbsPath, at: At<'_>) -> FsResult<RemoveStats> {
        let loc = self.locate(path.as_path(), false, at)?;
        let ino = loc.existing(at)?;
        self.writable(loc.vol, at)?;
        self.require_unmounted(&loc, at)?;
        let nested = self.volumes.iter().any(|volume| {
            let mount = volume.info.mount.as_path();
            mount != loc.real && mount.starts_with(&loc.real)
        });
        if nested {
            return Err(at.err(FsErrorKind::InUse));
        }
        let mut stats = RemoveStats::default();
        let mut order = Vec::new();
        let mut pending = vec![ino];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = self.entries(loc.vol, dir, at) else {
                continue;
            };
            for (key, slot) in entries {
                order.push((dir, key.clone(), slot.ino));
                if self.is_dir(loc.vol, slot.ino) {
                    pending.push(slot.ino);
                }
            }
        }
        for (parent, key, child) in order.into_iter().rev() {
            self.count_removed(loc.vol, child, &mut stats);
            self.remove_entry(loc.vol, parent, &key, at)?;
            self.drop_link(loc.vol, child);
        }
        self.count_removed(loc.vol, ino, &mut stats);
        self.unlink(&loc, at)?;
        Ok(stats)
    }

    fn count_removed(&self, vol: usize, ino: u64, stats: &mut RemoveStats) {
        if let Some(node) = self.inode(vol, ino) {
            if matches!(node.body, Body::Dir(_)) {
                stats.directories += 1;
            } else {
                stats.files += 1;
                stats.bytes += node.size();
            }
        }
    }

    fn may_change_account(&self, current: u32, requested: Option<u32>, own: u32) -> bool {
        requested.is_none_or(|value| {
            value == current || self.uid == ROOT_ACCOUNT || (own != u32::MAX && value == own)
        })
    }

    fn validate_change(
        &self,
        vol: usize,
        node: &Inode,
        change: &MetaChange,
        at: At<'_>,
    ) -> FsResult<()> {
        let features = self.features(vol);
        let unsupported = (!features.xattr
            && (!change.set_xattrs.is_empty() || !change.remove_xattrs.is_empty()))
            || (!features.acl && change.acl.is_some())
            || (self.os == Os::Linux && change.created_ns.is_some())
            || (self.os != Os::Windows
                && (change.set_windows_attributes | change.clear_windows_attributes) != 0);
        if unsupported {
            return Err(at.err(FsErrorKind::Unsupported));
        }
        let allowed = self.may_change_account(node.meta.uid, change.uid, u32::MAX)
            && self.may_change_account(node.meta.gid, change.gid, self.gid);
        if allowed {
            Ok(())
        } else {
            Err(at.err(FsErrorKind::Privilege))
        }
    }

    fn set_meta(&mut self, vol: usize, ino: u64, change: &MetaChange, at: At<'_>) -> FsResult<()> {
        self.validate_change(vol, self.node(vol, ino, at)?, change, at)?;
        self.copy_up(vol, ino, at)?;
        let now = self.now();
        let node = self.node_mut(vol, ino, at)?;
        let meta = &mut node.meta;
        if let Some(mode) = change.mode {
            meta.mode = mode & PERMISSION_BITS;
        }
        meta.uid = change.uid.unwrap_or(meta.uid);
        meta.gid = change.gid.unwrap_or(meta.gid);
        meta.modified = change.modified_ns.unwrap_or(meta.modified);
        meta.accessed = change.accessed_ns.unwrap_or(meta.accessed);
        meta.created = change.created_ns.unwrap_or(meta.created);
        for xattr in &change.set_xattrs {
            meta.xattrs.insert(xattr.name.clone(), xattr.value.clone());
        }
        for name in &change.remove_xattrs {
            meta.xattrs.remove(name);
        }
        if let Some(acl) = &change.acl {
            meta.acl = Some(acl.clone());
        }
        meta.windows_attributes = (meta.windows_attributes | change.set_windows_attributes)
            & !change.clear_windows_attributes;
        meta.changed = now;
        self.counters.metadata_changes += 1;
        Ok(())
    }

    /// Applies complete metadata and lists the elements that could not be
    /// applied: `account` and `group` without the privilege to change them,
    /// `created` on Linux, `xattr:<name>` and `acl` on volumes without them,
    /// `account_sid` and `windows_attributes` outside Windows, and
    /// `stream:<name>` for streams whose content is missing.
    fn apply_extended(
        &mut self,
        vol: usize,
        ino: u64,
        meta: &ExtendedMeta,
        at: At<'_>,
    ) -> FsResult<MetaTransferReport> {
        self.copy_up(vol, ino, at)?;
        let report = self.untransferable(vol, ino, meta, at)?;
        let skipped: HashSet<&str> = report.not_transferred.iter().map(String::as_str).collect();
        let features = self.features(vol);
        let os = self.os;
        let now = self.now();
        let node = self.node_mut(vol, ino, at)?;
        let target = &mut node.meta;
        target.mode = meta.basic.mode & PERMISSION_BITS;
        if !skipped.contains("account") {
            target.uid = meta.basic.uid.unwrap_or(target.uid);
        }
        if !skipped.contains("group") {
            target.gid = meta.basic.gid.unwrap_or(target.gid);
        }
        target.modified = meta.basic.modified_ns;
        target.accessed = meta.basic.accessed_ns.unwrap_or(target.accessed);
        if os != Os::Linux {
            target.created = meta.basic.created_ns.unwrap_or(target.created);
        }
        if features.xattr {
            target.xattrs = meta
                .xattrs
                .iter()
                .map(|xattr| (xattr.name.clone(), xattr.value.clone()))
                .collect();
        }
        if features.acl {
            target.acl.clone_from(&meta.acl);
        }
        if os == Os::Windows {
            target.account_sid.clone_from(&meta.account_sid);
            target.windows_attributes = meta.basic.windows_attributes;
        }
        target.flags = meta.flags;
        if features.alternate_streams {
            target
                .streams
                .retain(|name, _| meta.streams.iter().any(|stream| stream.name == *name));
        }
        target.changed = now;
        self.counters.metadata_changes += 1;
        Ok(report)
    }

    fn untransferable(
        &self,
        vol: usize,
        ino: u64,
        meta: &ExtendedMeta,
        at: At<'_>,
    ) -> FsResult<MetaTransferReport> {
        let node = self.node(vol, ino, at)?;
        let features = self.features(vol);
        let mut missing = Vec::new();
        if !self.may_change_account(node.meta.uid, meta.basic.uid, u32::MAX) {
            missing.push("account".to_owned());
        }
        if !self.may_change_account(node.meta.gid, meta.basic.gid, self.gid) {
            missing.push("group".to_owned());
        }
        if self.os == Os::Linux
            && meta
                .basic
                .created_ns
                .is_some_and(|time| time != node.meta.created)
        {
            missing.push("created".to_owned());
        }
        if !features.xattr {
            missing.extend(
                meta.xattrs
                    .iter()
                    .map(|xattr| format!("xattr:{}", xattr.name.to_string_lossy())),
            );
        }
        if !features.acl && meta.acl.is_some() {
            missing.push("acl".to_owned());
        }
        if self.os != Os::Windows {
            if meta.account_sid.is_some() {
                missing.push("account_sid".to_owned());
            }
            if meta.basic.windows_attributes != 0 {
                missing.push("windows_attributes".to_owned());
            }
        }
        missing.extend(
            meta.streams
                .iter()
                .filter(|stream| {
                    !features.alternate_streams || !node.meta.streams.contains_key(&stream.name)
                })
                .map(|stream| format!("stream:{}", stream.name)),
        );
        Ok(MetaTransferReport {
            not_transferred: missing,
        })
    }

    fn snapshot(&self, vol: usize, ino: u64, at: At<'_>) -> FsResult<TreeSnapshot> {
        let mut out = TreeSnapshot::default();
        let mut pending = vec![(String::new(), ino)];
        while let Some((path, current)) = pending.pop() {
            let node = self.node(vol, current, at)?;
            if let Body::Dir(entries) = &node.body {
                for slot in entries.values() {
                    let name = slot.name.to_string_lossy();
                    let child = if path.is_empty() {
                        name.into_owned()
                    } else {
                        format!("{path}/{name}")
                    };
                    pending.push((child, slot.ino));
                }
            }
            out.entries.insert(path, entry_snapshot(node));
        }
        Ok(out)
    }
}

fn entry_snapshot(node: &Inode) -> EntrySnapshot {
    EntrySnapshot {
        kind: node.kind(),
        size: node.size(),
        mode: node.meta.mode,
        uid: node.meta.uid,
        gid: node.meta.gid,
        links: node.links,
        modified_ns: node.meta.modified,
        created_ns: node.meta.created,
        windows_attributes: node.meta.windows_attributes,
        xattrs: node
            .meta
            .xattrs
            .iter()
            .map(|(name, value)| Xattr {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        acl: node.meta.acl.clone(),
        account_sid: node.meta.account_sid.clone(),
        flags: node.meta.flags,
        content: match &node.body {
            Body::File(content) => Some(content.digest()),
            Body::Dir(_) | Body::Link { .. } => None,
        },
        streams: node
            .meta
            .streams
            .iter()
            .map(|(name, content)| (name.clone(), content.digest()))
            .collect(),
        link_target: match &node.body {
            Body::Link { target, .. } => Some(target.clone()),
            Body::File(_) | Body::Dir(_) => None,
        },
    }
}

fn redirect(base: &Path, target: &Path, rest: &[OsString], at: At<'_>) -> FsResult<PathBuf> {
    let mut next = if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    };
    for name in rest {
        next.push(name);
    }
    AbsPath::new(next)
        .map(|path| path.as_path().to_path_buf())
        .map_err(|_| at.err(FsErrorKind::NotFound))
}

fn to_io(error: FsError) -> io::Error {
    let kind = match error.kind {
        FsErrorKind::NoSpace => io::ErrorKind::StorageFull,
        FsErrorKind::QuotaExceeded => io::ErrorKind::QuotaExceeded,
        FsErrorKind::ReadOnly => io::ErrorKind::ReadOnlyFilesystem,
        FsErrorKind::NotFound => io::ErrorKind::NotFound,
        FsErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

fn seek_position(current: u64, len: u64, target: SeekFrom) -> io::Result<u64> {
    let (base, offset) = match target {
        SeekFrom::Start(position) => return Ok(position),
        SeekFrom::End(offset) => (len, offset),
        SeekFrom::Current(offset) => (current, offset),
    };
    base.checked_add_signed(offset).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "seek before the start of the file",
        )
    })
}

struct MemReader {
    shared: Arc<Shared>,
    vol: usize,
    ino: u64,
    stream: Option<String>,
    position: u64,
    limit: Option<usize>,
}

impl MemReader {
    fn len(&self) -> u64 {
        lock(&self.shared.state)
            .blob(self.vol, self.ino, self.stream.as_deref())
            .map_or(0, |blob| blob.len)
    }
}

impl Read for MemReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let state = lock(&self.shared.state);
        let count = state
            .blob(self.vol, self.ino, self.stream.as_deref())
            .map_or(0, |blob| blob.read_at(self.position, buf, self.limit));
        self.position += to_u64(count);
        Ok(count)
    }
}

impl Seek for MemReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.len(), target)?;
        Ok(self.position)
    }
}

impl ReadHandle for MemReader {
    fn meta(&self) -> FsResult<FileMeta> {
        let state = lock(&self.shared.state);
        state.file_meta(self.vol, self.ino, At::new("meta", Path::new("")))
    }
}

impl Drop for MemReader {
    fn drop(&mut self) {
        lock(&self.shared.state).close(self.vol, self.ino);
    }
}

struct MemWriter {
    shared: Arc<Shared>,
    vol: usize,
    ino: u64,
    stream: Option<String>,
    position: u64,
    path: AbsPath,
}

impl MemWriter {
    fn target(&self) -> Target<'_> {
        Target {
            vol: self.vol,
            ino: self.ino,
            stream: self.stream.as_deref(),
        }
    }

    fn at(&self, op: &'static str) -> At<'_> {
        At::new(op, self.path.as_path())
    }
}

impl Write for MemWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = lock(&self.shared.state);
        let at = self.at("write");
        state.writable(self.vol, at).map_err(to_io)?;
        let edit = Edit::Write {
            offset: self.position,
            bytes: buf,
        };
        state.edit(self.target(), &edit, at).map_err(to_io)?;
        state.counters.bytes_written += to_u64(buf.len());
        self.position += to_u64(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for MemWriter {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let len = lock(&self.shared.state)
            .blob(self.vol, self.ino, self.stream.as_deref())
            .map_or(0, |blob| blob.len);
        self.position = seek_position(self.position, len, target)?;
        Ok(self.position)
    }
}

impl WriteHandle for MemWriter {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, _durability: Durability) -> FsResult<()> {
        lock(&self.shared.state).counters.file_syncs += 1;
        Ok(())
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        let mut state = lock(&self.shared.state);
        let at = self.at("set_length");
        state.writable(self.vol, at)?;
        state.edit(self.target(), &Edit::SetLength(length), at)
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        let mut state = lock(&self.shared.state);
        let at = self.at("apply_meta");
        state.writable(self.vol, at)?;
        state.apply_extended(self.vol, self.ino, meta, at)
    }
}

impl Drop for MemWriter {
    fn drop(&mut self) {
        lock(&self.shared.state).close(self.vol, self.ino);
    }
}

struct MemLock {
    shared: Arc<Shared>,
    key: Vec<u8>,
    path: AbsPath,
}

impl FileLock for MemLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

impl Drop for MemLock {
    fn drop(&mut self) {
        lock(&self.shared.locks).remove(&self.key);
        self.shared.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use nurt_contracts::fs::ScopePurpose;
    use nurt_contracts::journal::OpId;

    use super::*;
    use crate::conformance::{self, Options, Outcome, Report};

    // The journal is the only producer of scopes in production; tests of the
    // file system layer act as the journal.
    #[allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]
    fn scope() -> TxScope {
        TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
    }

    fn p(text: &str) -> AbsPath {
        AbsPath::new(text).expect("absolute test path")
    }

    fn read(fs: &MemFs, path: &str) -> Vec<u8> {
        let mut out = Vec::new();
        fs.open_read(&p(path))
            .expect("open")
            .read_to_end(&mut out)
            .expect("read");
        out
    }

    fn write(fs: &MemFs, path: &str, content: &[u8]) {
        let mut handle = fs.create_new(&scope(), &p(path)).expect("create");
        handle.write_all(content).expect("write");
    }

    fn kind<T>(result: FsResult<T>) -> FsErrorKind {
        match result {
            Ok(_) => panic!("operation must fail"),
            Err(error) => error.kind,
        }
    }

    fn skipped(report: &Report) -> Vec<&'static str> {
        report
            .results
            .iter()
            .filter(|result| matches!(result.outcome, Outcome::Skipped(_)))
            .map(|result| result.id)
            .collect()
    }

    fn conform(fs: &MemFs, other: Option<&str>) -> Report {
        fs.seed_dir(&p("/conformance")).expect("root");
        let options = Options {
            other_volume: other.map(p),
        };
        let report = conformance::run_with(fs, &p("/conformance"), &scope(), &options);
        assert!(report.passed(), "{report}");
        report
    }

    #[test]
    fn posix_case_sensitive_volume_passes_conformance() {
        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(p("/")))
            .volume(VolumeSpec::posix(p("/other")))
            .build()
            .expect("build");
        let report = conform(&fs, Some("/other"));
        assert_eq!(
            skipped(&report),
            ["fs-conf-replace-with-backup", "fs-conf-streams"]
        );
    }

    #[test]
    fn windows_case_insensitive_volume_passes_conformance() {
        let fs = MemFs::builder()
            .os(Os::Windows)
            .volume(VolumeSpec::windows(p("/")))
            .volume(VolumeSpec::windows(p("/other")))
            .build()
            .expect("build");
        let report = conform(&fs, Some("/other"));
        assert_eq!(skipped(&report), ["fs-conf-exchange", "fs-conf-clone-file"]);
    }

    #[test]
    fn volume_without_exchange_passes_conformance() {
        let features = FsFeatures {
            exchange: false,
            rename_noreplace: false,
            hardlink: false,
            reflink: false,
            xattr: false,
            acl: false,
            ..POSIX_FEATURES
        };
        let fs = MemFs::builder()
            .os(Os::Macos)
            .volume(VolumeSpec::posix(p("/")).with_features(features))
            .build()
            .expect("build");
        let report = conform(&fs, None);
        assert_eq!(
            skipped(&report),
            [
                "fs-conf-rename-noreplace",
                "fs-conf-exchange",
                "fs-conf-replace-with-backup",
                "fs-conf-hard-link",
                "fs-conf-clone-file",
                "fs-conf-cross-device",
                "fs-conf-streams",
            ]
        );
    }

    #[test]
    fn overlay_volume_passes_conformance() {
        let fs = MemFs::builder()
            .volume(VolumeSpec::overlay(p("/")))
            .build()
            .expect("build");
        conform(&fs, None);
    }

    #[test]
    fn write_through_handle_opened_before_exchange_changes_the_block_name() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/w/target"), b"old").expect("seed");
        fs.seed_file(&p("/w/block"), b"new").expect("seed");
        let mut foreign = fs.open_in_place(&scope(), &p("/w/target")).expect("open");
        fs.exchange(&scope(), &p("/w/target"), &p("/w/block"))
            .expect("exchange");
        assert_eq!(read(&fs, "/w/target"), b"new");
        foreign.write_all(b"OLD").expect("write");
        assert_eq!(read(&fs, "/w/block"), b"OLD");
        assert_eq!(read(&fs, "/w/target"), b"new");
    }

    #[test]
    fn open_in_place_with_two_links_updates_both_names() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/w/a"), b"before").expect("seed");
        fs.hard_link(&scope(), &p("/w/a"), &p("/w/b"))
            .expect("link");
        assert_eq!(
            fs.metadata(&p("/w/a"), Follow::NoFollow)
                .expect("meta")
                .links,
            2
        );
        let mut handle = fs.open_in_place(&scope(), &p("/w/b")).expect("open");
        handle.set_length(0).expect("truncate");
        handle.write_all(b"after").expect("write");
        drop(handle);
        assert_eq!(read(&fs, "/w/a"), b"after");
        assert_eq!(read(&fs, "/w/b"), b"after");
        fs.remove_file(&scope(), &p("/w/a")).expect("remove");
        assert_eq!(
            fs.metadata(&p("/w/b"), Follow::NoFollow)
                .expect("meta")
                .links,
            1
        );
    }

    fn small_volume(capacity: u64) -> MemFs {
        MemFs::builder()
            .volume(VolumeSpec::posix(p("/")).with_capacity(capacity))
            .build()
            .expect("build")
    }

    fn available(fs: &MemFs) -> u64 {
        let id = fs.volume(&p("/")).expect("volume").id;
        fs.space(&id).expect("space").available
    }

    #[test]
    fn writes_beyond_capacity_fail_with_no_space() {
        let fs = small_volume(100);
        write(&fs, "/a", &[1; 60]);
        assert_eq!(available(&fs), 40);
        let mut handle = fs.create_new(&scope(), &p("/b")).expect("create");
        let error = handle.write_all(&[2; 50]).expect_err("no space");
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        drop(handle);
        assert_eq!(available(&fs), 40);
        fs.clone_file(&scope(), &p("/a"), &p("/c")).expect("clone");
        assert_eq!(available(&fs), 40);
        fs.remove_file(&scope(), &p("/a")).expect("remove");
        assert_eq!(available(&fs), 40);
        fs.remove_file(&scope(), &p("/c")).expect("remove");
        assert_eq!(available(&fs), 100);
        assert_eq!(
            kind(fs.copy_file(&scope(), &p("/b"), &p("/b"))),
            FsErrorKind::Exists
        );
    }

    #[test]
    fn modifying_a_clone_charges_its_own_copy() {
        let fs = small_volume(100);
        write(&fs, "/a", &[1; 40]);
        fs.clone_file(&scope(), &p("/a"), &p("/c")).expect("clone");
        let mut handle = fs.open_in_place(&scope(), &p("/c")).expect("open");
        handle.write_all(b"x").expect("write");
        drop(handle);
        assert_eq!(available(&fs), 20);
        assert_eq!(
            kind(fs.copy_file(&scope(), &p("/a"), &p("/d"))),
            FsErrorKind::NoSpace
        );
        fs.truncate(&scope(), &p("/c"), 10).expect("truncate");
        assert_eq!(available(&fs), 50);
    }

    #[test]
    fn quota_and_simulated_foreign_usage_limit_writes() {
        let fs = MemFs::builder()
            .volume(
                VolumeSpec::posix(p("/"))
                    .with_capacity(1_000)
                    .with_quota(50),
            )
            .build()
            .expect("build");
        let id = fs.volume(&p("/")).expect("volume").id;
        write(&fs, "/a", &[0; 30]);
        let space = fs.space(&id).expect("space");
        assert_eq!(space.total, 50);
        assert_eq!(space.quota_remaining, Some(20));
        assert_eq!(space.quota_limit, Some(50));
        assert_eq!(space.available, 20);
        let mut handle = fs.create_new(&scope(), &p("/b")).expect("create");
        let error = handle.write_all(&[0; 30]).expect_err("quota");
        assert_eq!(error.kind(), io::ErrorKind::QuotaExceeded);
        fs.set_available(&p("/"), 5).expect("fill");
        let error = handle.write_all(&[0; 10]).expect_err("full");
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert_eq!(fs.space(&id).expect("space").available, 5);
    }

    #[test]
    fn reports_no_quota_limit_without_quota() {
        let fs = small_volume(1_000);
        let id = fs.volume(&p("/")).expect("volume").id;
        let space = fs.space(&id).expect("space");
        assert_eq!(space.quota_remaining, None);
        assert_eq!(space.quota_limit, None);
    }

    #[test]
    fn sparse_files_charge_only_written_bytes() {
        let fs = small_volume(1_000);
        let mut handle = fs.create_new(&scope(), &p("/sparse")).expect("create");
        handle.set_length(1 << 30).expect("grow");
        handle.write_all(b"head").expect("write");
        drop(handle);
        assert_eq!(available(&fs), 996);
        let meta = fs.metadata(&p("/sparse"), Follow::NoFollow).expect("meta");
        assert_eq!(meta.size, 1 << 30);
        let mut reader = fs.open_read(&p("/sparse")).expect("open");
        reader.seek(SeekFrom::End(-2)).expect("seek");
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail).expect("read");
        assert_eq!(tail, [0, 0]);
    }

    fn overlay() -> MemFs {
        let fs = MemFs::builder()
            .volume(VolumeSpec::overlay(p("/")).with_capacity(100))
            .build()
            .expect("build");
        fs.seed_file(&p("/image/dir/file"), &[7; 30]).expect("seed");
        fs.seed_lower(&p("/image")).expect("lower");
        fs
    }

    #[test]
    fn overlay_lower_directory_rename_fails_cross_device_and_files_copy_up() {
        let fs = overlay();
        assert_eq!(available(&fs), 100);
        assert_eq!(
            kind(fs.rename(
                &scope(),
                &p("/image/dir"),
                &p("/moved"),
                RenameMode::Replace
            )),
            FsErrorKind::CrossDevice
        );
        fs.rename(
            &scope(),
            &p("/image/dir/file"),
            &p("/image/file"),
            RenameMode::Replace,
        )
        .expect("rename of a lower file");
        assert_eq!(fs.counters().copy_ups, 1);
        assert_eq!(available(&fs), 70);
        fs.create_dir(&scope(), &p("/upper"), 0o755).expect("dir");
        fs.rename(&scope(), &p("/upper"), &p("/renamed"), RenameMode::Replace)
            .expect("rename of an upper directory");
    }

    #[test]
    fn overlay_copy_up_fails_without_space() {
        let fs = overlay();
        fs.set_available(&p("/"), 10).expect("fill");
        assert_eq!(
            kind(fs.open_in_place(&scope(), &p("/image/dir/file"))),
            FsErrorKind::NoSpace
        );
        assert_eq!(
            kind(fs.clone_file(&scope(), &p("/image/dir/file"), &p("/c"))),
            FsErrorKind::Unsupported
        );
        assert_eq!(kind(fs.seed_lower(&p("/missing"))), FsErrorKind::NotFound);
        assert_eq!(
            kind(MemFs::new().expect("build").seed_lower(&p("/"))),
            FsErrorKind::Unsupported
        );
    }

    #[test]
    fn renaming_a_million_file_tree_is_one_rename_without_copies() {
        let fs = MemFs::new().expect("build");
        let started = Instant::now();
        for dir in 0..1_000 {
            fs.seed_dir(&p(&format!("/big/d{dir:03}"))).expect("dir");
            for file in 0..1_000 {
                fs.seed_file(&p(&format!("/big/d{dir:03}/f{file:03}")), b"")
                    .expect("file");
            }
        }
        let built = started.elapsed();
        assert!(built < Duration::from_secs(120), "tree built in {built:?}");
        fs.reset_counters();
        fs.seed_dir(&p("/journal")).expect("journal");
        fs.rename(
            &scope(),
            &p("/big"),
            &p("/journal/big"),
            RenameMode::NoReplace,
        )
        .expect("rename");
        let counters = fs.counters();
        assert_eq!(counters.renames, 1);
        assert_eq!(counters.bytes_copied, 0);
        assert_eq!(counters.bytes_written, 0);
        let stats = fs
            .remove_tree(&scope(), &p("/journal/big"), 4)
            .expect("remove");
        assert_eq!(stats.files, 1_000_000);
        assert_eq!(stats.directories, 1_001);
    }

    fn scripted_run() -> (TreeSnapshot, Vec<FileId>, Vec<OsString>, MemCounters) {
        let fs = MemFs::new().expect("build");
        fs.seed_dir(&p("/run")).expect("dir");
        for name in ["zeta", "alpha", "mid"] {
            write(&fs, &format!("/run/{name}"), name.as_bytes());
        }
        let temp = fs
            .create_temp_in(&scope(), &p("/run"), "stage")
            .expect("temp")
            .path()
            .clone();
        fs.rename(&scope(), &temp, &p("/run/beta"), RenameMode::Replace)
            .expect("rename");
        fs.exchange(&scope(), &p("/run/alpha"), &p("/run/beta"))
            .expect("exchange");
        let names: Vec<OsString> = fs
            .read_dir(&p("/run"))
            .expect("list")
            .map(|entry| entry.expect("entry").name)
            .collect();
        let ids = names
            .iter()
            .map(|name| {
                let path = p("/run").join_name(&name.to_string_lossy()).expect("path");
                fs.metadata(&path, Follow::NoFollow).expect("meta").file_id
            })
            .collect();
        (
            fs.snapshot(&p("/run")).expect("snapshot"),
            ids,
            names,
            fs.counters(),
        )
    }

    #[test]
    fn identical_sequences_produce_identical_state() {
        let first = scripted_run();
        let second = scripted_run();
        assert_eq!(first, second);
        assert_eq!(first.2, ["alpha", "beta", "mid", "zeta"]);
        assert_eq!(first.0.diff(&second.0), Vec::<String>::new());
    }

    #[test]
    fn file_identities_and_times_follow_the_injected_clock() {
        let clock = Arc::new(VirtualClock::default());
        let fs = MemFs::builder()
            .clock(clock.clone())
            .build()
            .expect("build");
        write(&fs, "/a", b"1");
        clock.advance(Duration::from_secs(5));
        let mut handle = fs.open_in_place(&scope(), &p("/a")).expect("open");
        handle.write_all(b"2").expect("write");
        drop(handle);
        let meta = fs.metadata(&p("/a"), Follow::NoFollow).expect("meta");
        assert_eq!(meta.file_id.low, 2);
        assert_eq!(meta.file_id.volume, 1);
        assert_eq!(
            i128::from(meta.modified_ns),
            VirtualClock::DEFAULT_START.unix_nanos() + 5_000_000_000
        );
        assert_eq!(
            i128::from(meta.created_ns.expect("birth")),
            VirtualClock::DEFAULT_START.unix_nanos()
        );
        let root = fs.metadata(&p("/"), Follow::NoFollow).expect("root");
        assert_eq!(root.file_id.low, 1);
    }

    #[test]
    fn faults_apply_once_or_persistently_with_holders() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/f"), b"x").expect("seed");
        let holder = Holder {
            pid: 42,
            name: "editor".to_owned(),
        };
        fs.set_holders(&p("/f"), vec![holder.clone()])
            .expect("holders");
        assert_eq!(
            fs.holders(&p("/f")).expect("holders"),
            std::slice::from_ref(&holder)
        );
        fs.inject(MemFault::once(p("/f"), "remove_file", FsErrorKind::InUse))
            .expect("inject");
        let error = fs.remove_file(&scope(), &p("/f")).expect_err("in use");
        assert_eq!(error.kind, FsErrorKind::InUse);
        assert_eq!(error.holders, [holder]);
        fs.inject(MemFault::persistent(
            p("/f"),
            Some("open_read"),
            FsErrorKind::Io,
        ))
        .expect("inject");
        assert_eq!(kind(fs.open_read(&p("/f"))), FsErrorKind::Io);
        assert_eq!(kind(fs.open_read(&p("/f"))), FsErrorKind::Io);
        fs.clear_faults();
        fs.remove_file(&scope(), &p("/f")).expect("remove");
    }

    #[test]
    fn denied_paths_refuse_access_and_are_not_writable() {
        let fs = MemFs::new().expect("build");
        fs.seed_dir(&p("/locked")).expect("dir");
        fs.deny_access(&p("/locked")).expect("deny");
        assert!(!fs.can_write_dir(&p("/locked")).expect("query"));
        assert_eq!(
            kind(fs.metadata(&p("/locked"), Follow::NoFollow)),
            FsErrorKind::PermissionDenied
        );
        fs.seed_dir(&p("/open")).expect("dir");
        assert!(fs.can_write_dir(&p("/open")).expect("query"));
    }

    #[test]
    fn can_write_dir_follows_mode_bits_of_the_account() {
        let fs = MemFs::builder().account(1000, 100).build().expect("build");
        fs.seed_dir(&p("/d")).expect("dir");
        let set = |mode, uid, gid| MetaChange {
            mode: Some(mode),
            uid,
            gid,
            ..MetaChange::default()
        };
        fs.set_meta(&scope(), &p("/d"), &set(0o555, None, None))
            .expect("chmod");
        assert!(!fs.can_write_dir(&p("/d")).expect("query"));
        assert_eq!(
            kind(fs.set_meta(&scope(), &p("/d"), &set(0o777, Some(7), None))),
            FsErrorKind::Privilege
        );
        let root = MemFs::builder().account(0, 0).build().expect("build");
        root.seed_dir(&p("/d")).expect("dir");
        root.set_meta(&scope(), &p("/d"), &set(0o570, Some(7), Some(100)))
            .expect("chown");
        assert!(root.can_write_dir(&p("/d")).expect("query"));
        let group = MemFs::builder().account(5, 100).build().expect("build");
        group.seed_dir(&p("/d")).expect("dir");
        group
            .set_meta(&scope(), &p("/d"), &set(0o730, None, None))
            .expect("chmod");
        assert!(group.can_write_dir(&p("/d")).expect("query"));
    }

    #[test]
    fn short_reads_return_at_most_the_configured_bytes() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/f"), b"abcdef").expect("seed");
        fs.limit_reads(&p("/f"), 2).expect("limit");
        let mut handle = fs.open_read(&p("/f")).expect("open");
        let mut buf = [0_u8; 6];
        assert_eq!(handle.read(&mut buf).expect("read"), 2);
        assert_eq!(handle.meta().expect("meta").size, 6);
        assert_eq!(read(&fs, "/f"), b"abcdef");
    }

    #[test]
    fn read_only_volume_refuses_every_mutation() {
        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(p("/")))
            .volume(VolumeSpec::posix(p("/ro")).with_read_only(true))
            .build()
            .expect("build");
        assert_eq!(
            kind(fs.create_dir(&scope(), &p("/ro/d"), 0o755)),
            FsErrorKind::ReadOnly
        );
        assert_eq!(
            kind(fs.lock_exclusive(&p("/ro/lock"), Duration::ZERO)),
            FsErrorKind::ReadOnly
        );
        assert!(!fs.can_write_dir(&p("/ro")).expect("query"));
        assert_eq!(
            kind(fs.remove_dir(&scope(), &p("/ro"))),
            FsErrorKind::ReadOnly
        );
        fs.create_dir(&scope(), &p("/rw"), 0o755)
            .expect("writable root");
    }

    #[test]
    fn nested_volume_mount_points_are_busy() {
        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(p("/")))
            .volume(VolumeSpec::posix(p("/mnt/data")))
            .build()
            .expect("build");
        let outer = fs.volume(&p("/mnt")).expect("outer").id;
        let inner = fs.volume(&p("/mnt/data/x/y")).expect("inner").id;
        assert_ne!(outer, inner);
        assert_eq!(
            kind(fs.remove_tree(&scope(), &p("/mnt"), 1)),
            FsErrorKind::InUse
        );
        assert_eq!(
            kind(fs.rename(&scope(), &p("/mnt/data"), &p("/x"), RenameMode::Replace)),
            FsErrorKind::InUse
        );
        assert_eq!(
            kind(fs.space(&VolumeId {
                dev: 99,
                uuid: None,
                subvolume: 0
            })),
            FsErrorKind::NotFound
        );
        let duplicate = MemFs::builder()
            .volume(VolumeSpec::posix(p("/")))
            .volume(VolumeSpec::posix(p("/")))
            .build();
        assert_eq!(kind(duplicate), FsErrorKind::InvalidPath);
    }

    #[test]
    fn case_insensitive_names_keep_the_stored_spelling() {
        let fs = MemFs::builder()
            .os(Os::Windows)
            .volume(VolumeSpec::windows(p("/")))
            .build()
            .expect("build");
        write(&fs, "/Readme.TXT", b"x");
        assert_eq!(read(&fs, "/README.txt"), b"x");
        assert_eq!(
            kind(fs.create_new(&scope(), &p("/readme.txt"))),
            FsErrorKind::Exists
        );
        fs.rename(
            &scope(),
            &p("/readme.txt"),
            &p("/README.TXT"),
            RenameMode::NoReplace,
        )
        .expect("respell");
        let names: Vec<OsString> = fs
            .read_dir(&p("/"))
            .expect("list")
            .map(|entry| entry.expect("entry").name)
            .collect();
        assert_eq!(names, ["README.TXT"]);
        assert_eq!(
            fs.real_path(&p("/readme.txt")).expect("real"),
            p("/README.TXT")
        );
        let meta = fs
            .metadata(&p("/README.TXT"), Follow::NoFollow)
            .expect("meta");
        assert_eq!(meta.uid, None);
    }

    #[test]
    fn symbolic_link_cycles_fail_with_loop() {
        let fs = MemFs::new().expect("build");
        fs.seed_symlink(&p("/a"), Path::new("/b")).expect("link");
        fs.seed_symlink(&p("/b"), Path::new("a")).expect("link");
        assert_eq!(
            kind(fs.metadata(&p("/a"), Follow::Follow)),
            FsErrorKind::Loop
        );
        assert_eq!(
            fs.metadata(&p("/a"), Follow::NoFollow).expect("link").kind,
            FileKind::Symlink
        );
        fs.seed_symlink(&p("/up"), Path::new("../.."))
            .expect("link");
        assert_eq!(
            kind(fs.metadata(&p("/up/x"), Follow::Follow)),
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn rename_rejects_moves_into_own_subtree_and_kind_mismatches() {
        let fs = MemFs::new().expect("build");
        fs.seed_dir(&p("/d/sub")).expect("dir");
        fs.seed_file(&p("/f"), b"x").expect("file");
        fs.seed_file(&p("/d/sub/g"), b"y").expect("file");
        let s = scope();
        assert_eq!(
            kind(fs.rename(&s, &p("/d"), &p("/d/sub/inner"), RenameMode::Replace)),
            FsErrorKind::InvalidPath
        );
        assert_eq!(
            kind(fs.rename(&s, &p("/d"), &p("/f"), RenameMode::Replace)),
            FsErrorKind::NotADirectory
        );
        assert_eq!(
            kind(fs.rename(&s, &p("/f"), &p("/d"), RenameMode::Replace)),
            FsErrorKind::IsADirectory
        );
        fs.seed_dir(&p("/e")).expect("dir");
        assert_eq!(
            kind(fs.rename(&s, &p("/e"), &p("/d"), RenameMode::Replace)),
            FsErrorKind::NotEmpty
        );
        fs.rename(&s, &p("/d/sub"), &p("/e"), RenameMode::Replace)
            .expect("replace empty directory");
        assert_eq!(read(&fs, "/e/g"), b"y");
        fs.hard_link(&s, &p("/f"), &p("/f2")).expect("link");
        fs.rename(&s, &p("/f"), &p("/f2"), RenameMode::Replace)
            .expect("rename onto own hard link");
        assert_eq!(
            kind(fs.exchange(&s, &p("/e"), &p("/e/g"))),
            FsErrorKind::InvalidPath
        );
        assert_eq!(
            kind(fs.hard_link(&s, &p("/e"), &p("/e3"))),
            FsErrorKind::PermissionDenied
        );
    }

    #[test]
    fn replace_with_backup_transfers_windows_metadata() {
        let fs = MemFs::builder()
            .os(Os::Windows)
            .volume(VolumeSpec::windows(p("/")))
            .build()
            .expect("build");
        let s = scope();
        write(&fs, "/target", b"old");
        let mut stream = fs
            .open_stream_write(&s, &p("/target"), "Zone")
            .expect("stream");
        stream.write_all(b"3").expect("write");
        drop(stream);
        fs.set_meta(
            &s,
            &p("/target"),
            &MetaChange {
                created_ns: Some(7),
                acl: Some(b"D:P".to_vec()),
                set_windows_attributes: FILE_ATTRIBUTE_HIDDEN,
                ..MetaChange::default()
            },
        )
        .expect("meta");
        write(&fs, "/new", b"new");
        write(&fs, "/backup", b"stale");
        fs.replace_with_backup(&s, &p("/target"), &p("/new"), &p("/backup"))
            .expect("replace");
        let meta = fs
            .extended_metadata(&p("/target"), Follow::NoFollow)
            .expect("meta");
        assert_eq!(meta.basic.created_ns, Some(7));
        assert_eq!(meta.acl.as_deref(), Some(&b"D:P"[..]));
        assert_eq!(meta.basic.windows_attributes, FILE_ATTRIBUTE_HIDDEN);
        assert_eq!(meta.streams.len(), 1);
        assert_eq!(read(&fs, "/backup"), b"old");
        fs.mark_hidden(&s, &p("/backup")).expect("hide");
        let hidden = fs.metadata(&p("/backup"), Follow::NoFollow).expect("meta");
        assert_ne!(hidden.windows_attributes & FILE_ATTRIBUTE_SYSTEM, 0);
        fs.create_link(&s, Path::new("/"), &p("/junction"), LinkKind::Junction)
            .expect("junction");
        assert_eq!(
            kind(fs.create_link(&s, Path::new("rel"), &p("/j2"), LinkKind::Junction)),
            FsErrorKind::InvalidPath
        );
    }

    #[test]
    fn restore_meta_reports_elements_the_system_cannot_set() {
        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(p("/")).with_features(FsFeatures {
                xattr: false,
                acl: false,
                ..POSIX_FEATURES
            }))
            .build()
            .expect("build");
        fs.seed_file(&p("/f"), b"x").expect("seed");
        let mut wanted = fs
            .extended_metadata(&p("/f"), Follow::NoFollow)
            .expect("meta");
        wanted.basic.uid = Some(0);
        wanted.basic.created_ns = Some(1);
        wanted.basic.windows_attributes = FILE_ATTRIBUTE_HIDDEN;
        wanted.xattrs = vec![Xattr {
            name: "user.a".into(),
            value: Vec::new(),
        }];
        wanted.acl = Some(Vec::new());
        wanted.account_sid = Some("S-1-5-18".to_owned());
        wanted.streams = vec![AlternateStream {
            name: "s".to_owned(),
            size: 1,
        }];
        let report = fs
            .restore_meta(&scope(), &p("/f"), &wanted)
            .expect("restore");
        assert_eq!(
            report.not_transferred,
            [
                "account",
                "created",
                "xattr:user.a",
                "acl",
                "account_sid",
                "windows_attributes",
                "stream:s"
            ]
        );
        assert_eq!(
            kind(fs.set_meta(
                &scope(),
                &p("/f"),
                &MetaChange {
                    created_ns: Some(1),
                    ..MetaChange::default()
                }
            )),
            FsErrorKind::Unsupported
        );
    }

    #[test]
    fn snapshot_detects_content_and_metadata_changes() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/t/a"), b"1").expect("seed");
        fs.seed_symlink(&p("/t/l"), Path::new("a")).expect("link");
        let before = fs.snapshot(&p("/t")).expect("snapshot");
        assert_eq!(before.entries.len(), 3);
        fs.set_meta(
            &scope(),
            &p("/t/a"),
            &MetaChange {
                mode: Some(0o600),
                ..MetaChange::default()
            },
        )
        .expect("chmod");
        write(&fs, "/t/b", b"2");
        let after = fs.snapshot(&p("/t")).expect("snapshot");
        assert_eq!(before.diff(&after), ["a", "b"]);
    }

    #[test]
    fn seek_positions_handles_relative_to_start_current_and_end() {
        let fs = MemFs::new().expect("build");
        let mut handle = fs.create_new(&scope(), &p("/f")).expect("create");
        handle.write_all(b"0123456789").expect("write");
        assert_eq!(handle.seek(SeekFrom::Current(-4)).expect("seek"), 6);
        handle.write_all(b"X").expect("write");
        assert_eq!(handle.seek(SeekFrom::End(0)).expect("seek"), 10);
        assert!(handle.seek(SeekFrom::Current(-11)).is_err());
        handle.flush().expect("flush");
        drop(handle);
        let mut reader = fs.open_read(&p("/f")).expect("open");
        reader.seek(SeekFrom::Start(6)).expect("seek");
        let mut rest = String::new();
        reader.read_to_string(&mut rest).expect("read");
        assert_eq!(rest, "X789");
        assert!(reader.as_bytes().is_none());
    }

    #[test]
    fn lock_is_released_when_dropped_and_waiters_proceed() {
        let fs = MemFs::new().expect("build");
        let held = fs
            .lock_exclusive(&p("/lock"), Duration::ZERO)
            .expect("lock");
        let other = fs.clone();
        let waiter = std::thread::spawn(move || {
            other
                .lock_exclusive(&p("/lock"), Duration::from_secs(10))
                .map(|lock| lock.path().clone())
        });
        std::thread::sleep(Duration::from_millis(20));
        drop(held);
        assert_eq!(waiter.join().expect("join").expect("lock"), p("/lock"));
        fs.seed_dir(&p("/dir")).expect("dir");
        assert_eq!(
            kind(fs.lock_exclusive(&p("/dir"), Duration::ZERO)),
            FsErrorKind::IsADirectory
        );
    }

    #[test]
    fn handles_keep_removed_files_alive_until_closed() {
        let fs = small_volume(100);
        write(&fs, "/f", &[1; 10]);
        let reader = fs.open_read(&p("/f")).expect("open");
        fs.remove_file(&scope(), &p("/f")).expect("remove");
        assert_eq!(available(&fs), 90);
        assert_eq!(reader.meta().expect("meta").links, 0);
        drop(reader);
        assert_eq!(available(&fs), 100);
    }

    #[test]
    fn stream_operations_require_the_feature_and_valid_names() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/f"), b"x").expect("seed");
        assert_eq!(
            kind(fs.open_stream_write(&scope(), &p("/f"), "s")),
            FsErrorKind::Unsupported
        );
        assert_eq!(
            kind(fs.open_stream_read(&p("/f"), "s")),
            FsErrorKind::Unsupported
        );
        let windows = MemFs::builder()
            .os(Os::Windows)
            .volume(VolumeSpec::windows(p("/")))
            .build()
            .expect("build");
        windows.seed_file(&p("/f"), b"x").expect("seed");
        assert_eq!(
            kind(windows.open_stream_write(&scope(), &p("/f"), "a:b")),
            FsErrorKind::InvalidPath
        );
        assert_eq!(
            kind(windows.create_temp_in(&scope(), &p("/"), "a/b")),
            FsErrorKind::InvalidPath
        );
    }

    #[test]
    fn macos_hidden_marker_sets_the_hidden_flag() {
        let fs = MemFs::builder().os(Os::Macos).build().expect("build");
        fs.seed_dir(&p("/j")).expect("dir");
        fs.mark_hidden(&scope(), &p("/j")).expect("hide");
        let meta = fs
            .extended_metadata(&p("/j"), Follow::NoFollow)
            .expect("meta");
        assert_eq!(meta.flags, Some(MACOS_UF_HIDDEN));
        assert_eq!(fs.counters().metadata_changes, 1);
    }

    #[test]
    fn remove_tree_of_a_link_removes_the_link_and_keeps_its_target() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/outside/kept"), b"keep").expect("seed");
        fs.seed_symlink(&p("/link"), Path::new("/outside"))
            .expect("link");
        let stats = fs.remove_tree(&scope(), &p("/link"), 1).expect("remove");
        assert_eq!((stats.files, stats.directories), (1, 0));
        assert_eq!(read(&fs, "/outside/kept"), b"keep");
        assert_eq!(
            kind(fs.metadata(&p("/link"), Follow::NoFollow)),
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn remove_tree_does_not_follow_a_link_to_its_own_ancestor() {
        let fs = MemFs::new().expect("build");
        fs.seed_dir(&p("/tree/sub")).expect("seed");
        fs.seed_symlink(&p("/tree/sub/up"), Path::new("/tree"))
            .expect("link");
        let stats = fs.remove_tree(&scope(), &p("/tree"), 1).expect("remove");
        assert_eq!((stats.files, stats.directories), (1, 2));
        assert_eq!(
            kind(fs.metadata(&p("/tree"), Follow::NoFollow)),
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn rename_moves_a_link_and_leaves_its_target() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/target"), b"data").expect("seed");
        fs.seed_symlink(&p("/link"), Path::new("target"))
            .expect("link");
        fs.rename(&scope(), &p("/link"), &p("/moved"), RenameMode::NoReplace)
            .expect("rename");
        assert_eq!(
            fs.read_link(&p("/moved")).expect("target"),
            PathBuf::from("target")
        );
        assert_eq!(
            kind(fs.metadata(&p("/link"), Follow::NoFollow)),
            FsErrorKind::NotFound
        );
        assert_eq!(read(&fs, "/target"), b"data");
    }

    #[test]
    fn copy_of_a_link_source_copies_the_content_of_its_target() {
        let fs = MemFs::new().expect("build");
        fs.seed_file(&p("/source"), b"payload").expect("seed");
        fs.seed_symlink(&p("/link"), Path::new("/source"))
            .expect("link");
        let copied = fs
            .copy_file(&scope(), &p("/link"), &p("/copy"))
            .expect("copy");
        assert_eq!(copied, 7);
        assert_eq!(read(&fs, "/copy"), b"payload");
        assert_eq!(
            fs.metadata(&p("/copy"), Follow::NoFollow)
                .expect("meta")
                .kind,
            FileKind::File
        );
    }

    #[test]
    fn lock_with_the_longest_timeout_is_taken_when_the_file_is_free() {
        let fs = MemFs::new().expect("build");
        let lock = fs.lock_exclusive(&p("/lock"), Duration::MAX).expect("lock");
        assert_eq!(lock.path(), &p("/lock"));
    }

    #[cfg(unix)]
    #[test]
    fn names_that_are_not_valid_utf8_stay_distinct_and_sorted_by_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let fs = MemFs::new().expect("build");
        let named = |name: &[u8]| {
            AbsPath::new(Path::new("/d").join(OsStr::from_bytes(name))).expect("path")
        };
        let (first, second) = (named(b"a\xff"), named(b"a\xfe"));
        fs.seed_file(&first, b"1").expect("seed");
        fs.seed_file(&second, b"2").expect("seed");
        let names: Vec<Vec<u8>> = fs
            .read_dir(&p("/d"))
            .expect("listing")
            .map(|entry| entry.expect("entry").name.as_bytes().to_vec())
            .collect();
        assert_eq!(names, [b"a\xfe".to_vec(), b"a\xff".to_vec()]);
        let mut content = Vec::new();
        fs.open_read(&first)
            .expect("open")
            .read_to_end(&mut content)
            .expect("read");
        assert_eq!(content, b"1");
    }

    #[test]
    fn volume_presets_and_options_are_reported() {
        let spec = VolumeSpec::posix(p("/"))
            .with_kind(VolumeKind::Removable)
            .with_fs_type("ext4")
            .with_uuid("0000-1111")
            .with_rotational(true);
        let fs = MemFs::builder().volume(spec).build().expect("build");
        let info = fs.volume(&p("/missing/deeper")).expect("volume");
        assert_eq!(info.kind, VolumeKind::Removable);
        assert_eq!(info.fs_type, "ext4");
        assert_eq!(info.id.uuid.as_deref(), Some("0000-1111"));
        assert!(info.rotational);
        assert!(format!("{fs:?}").starts_with("MemFs"));
        assert!(format!("{:?}", MemFs::builder()).starts_with("MemFsBuilder"));
    }
}
