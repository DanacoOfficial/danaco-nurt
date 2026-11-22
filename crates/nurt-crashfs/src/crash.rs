// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File system model that simulates power loss.
//!
//! [`CrashFs`] implements [`Fs`] and [`FsWrite`] in memory and keeps two
//! images of every volume: the live image seen by callers and the durable
//! image that survives a crash. Every mutation changes the live image at once
//! and is queued as a pending operation. Flushes move pending operations into
//! the durable image following the persistence rules of a [`CrashProfile`]:
//! file content and size become durable through [`WriteHandle::sync`],
//! directory entries through [`FsWrite::sync_dir`] of every directory they
//! change. [`CrashFs::crash`] builds the image after power loss from the
//! durable image and a seeded selection of pending operations, including torn
//! writes, and continues from it as a restarted system.
//!
//! Results depend only on the seed and on the sequence of calls.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, ExtendedMeta, FileId, FileKind, FileLock, FileMeta,
    Follow, Fs, FsError, FsErrorKind, FsFeatures, FsResult, FsWrite, Holder, LinkKind, MetaChange,
    MetaTransferReport, PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, TxScope, VolumeId,
    VolumeInfo, VolumeKind, WriteHandle, Xattr,
};
use nurt_fs::failpoint::{SeededFailpoints, SplitMix64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type NodeId = u64;

const ROOT_NODE: NodeId = 1;
const FIRST_DEVICE: u64 = 0x0800;
const PER_MILLION: u64 = 1_000_000;
/// Probability that a kept data write is torn, per million.
const TORN_PER_MILLION: u32 = 250_000;
/// Start of the model clock: 2026-01-01T00:00:00Z.
const CLOCK_START_NS: i64 = 1_767_225_600_000_000_000;
/// Model clock step per mutating call.
const CLOCK_TICK_NS: i64 = 1_000_000;
/// Symbolic link depth at which resolution fails with `Loop`, as `MAXSYMLINKS`
/// on Linux.
const MAX_LINK_DEPTH: u32 = 40;
const DEFAULT_FILE_MODE: u32 = 0o644;
const DEFAULT_DIR_MODE: u32 = 0o755;
const LINK_MODE: u32 = 0o777;
const DEFAULT_UID: u32 = 1000;
const DEFAULT_GID: u32 = 1000;
const DEFAULT_CAPACITY_BYTES: u64 = 1 << 30;
const DEFAULT_CAPACITY_INODES: u64 = 1 << 20;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x2000;
const SEED_STRIDE: u64 = 0x9E37_79B9_7F4A_7C15;

/// Persistence semantics applied when building the image after a crash.
///
/// Profiles other than [`CrashProfile::PosixWorstCase`] approximate the
/// documented behaviour of a file system and serve to compare the model with
/// samples taken on that file system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashProfile {
    /// Only what POSIX guarantees: any subset of unflushed operations
    /// survives, writes may be torn at any byte, and a file flush makes no
    /// directory entry durable.
    PosixWorstCase,
    /// The ext4 file system with `data=ordered`: metadata is journaled in
    /// order, and file data written before a journal commit is flushed with
    /// it.
    Ext4Ordered,
    /// XFS: metadata is journaled in order, file data is written back
    /// independently.
    Xfs,
    /// Btrfs: every operation is persisted in order by copy-on-write
    /// transactions; no torn writes.
    Btrfs,
    /// NTFS: metadata is journaled in order, file data is not.
    Ntfs,
    /// APFS: metadata is persisted in order; only a full flush
    /// (`F_FULLFSYNC`) makes file data durable.
    Apfs,
}

impl CrashProfile {
    /// Every profile.
    pub const ALL: [CrashProfile; 6] = [
        Self::PosixWorstCase,
        Self::Ext4Ordered,
        Self::Xfs,
        Self::Btrfs,
        Self::Ntfs,
        Self::Apfs,
    ];

    /// Stable snake-case name, equal to the serialized form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PosixWorstCase => "posix_worst_case",
            Self::Ext4Ordered => "ext4_ordered",
            Self::Xfs => "xfs",
            Self::Btrfs => "btrfs",
            Self::Ntfs => "ntfs",
            Self::Apfs => "apfs",
        }
    }

    fn rules(self) -> ProfileRules {
        let journaled = ProfileRules {
            order: Order::MetadataPrefix,
            data_with_commit: false,
            torn_sector: Some(SECTOR),
            flush_commits_metadata: true,
            data_sync_durable: true,
        };
        match self {
            Self::PosixWorstCase => ProfileRules {
                order: Order::Unordered,
                data_with_commit: false,
                torn_sector: Some(1),
                flush_commits_metadata: false,
                data_sync_durable: true,
            },
            Self::Ext4Ordered => ProfileRules {
                data_with_commit: true,
                ..journaled
            },
            Self::Xfs | Self::Ntfs => journaled,
            Self::Btrfs => ProfileRules {
                order: Order::Global,
                torn_sector: None,
                ..journaled
            },
            Self::Apfs => ProfileRules {
                data_sync_durable: false,
                ..journaled
            },
        }
    }
}

/// Sector size used for torn writes on journaled profiles.
const SECTOR: u64 = 512;

#[derive(Clone, Copy, Debug)]
enum Order {
    Unordered,
    MetadataPrefix,
    Global,
}

#[derive(Clone, Copy, Debug)]
struct ProfileRules {
    order: Order,
    data_with_commit: bool,
    torn_sector: Option<u64>,
    flush_commits_metadata: bool,
    data_sync_durable: bool,
}

/// Description of one simulated volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeSpec {
    /// Mount point; the volume root directory.
    pub mount: AbsPath,
    /// File system type name reported by [`Fs::volume`].
    pub fs_type: String,
    /// Volume class.
    pub kind: VolumeKind,
    /// Capabilities; disabled capabilities fail with
    /// [`FsErrorKind::Unsupported`].
    pub features: FsFeatures,
    /// Capacity in bytes of file content.
    pub capacity_bytes: u64,
    /// Capacity in entries, the root directory included.
    pub capacity_inodes: u64,
    /// Account quota in bytes; exceeding it fails with
    /// [`FsErrorKind::QuotaExceeded`].
    pub quota_bytes: Option<u64>,
}

impl VolumeSpec {
    /// Local fixed volume with every capability except replacement with a
    /// backup name and alternate streams, 1 GiB and 2^20 entries.
    pub fn new(mount: AbsPath) -> Self {
        Self {
            mount,
            fs_type: "crashfs".to_owned(),
            kind: VolumeKind::Fixed,
            features: FsFeatures {
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
            },
            capacity_bytes: DEFAULT_CAPACITY_BYTES,
            capacity_inodes: DEFAULT_CAPACITY_INODES,
            quota_bytes: None,
        }
    }

    /// Sets the capacity in bytes.
    #[must_use]
    pub fn with_capacity(mut self, bytes: u64, inodes: u64) -> Self {
        self.capacity_bytes = bytes;
        self.capacity_inodes = inodes;
        self
    }

    /// Sets the account quota in bytes.
    #[must_use]
    pub fn with_quota(mut self, bytes: u64) -> Self {
        self.quota_bytes = Some(bytes);
        self
    }

    /// Replaces the capabilities.
    #[must_use]
    pub fn with_features(mut self, features: FsFeatures) -> Self {
        self.features = features;
        self
    }
}

/// Counters of file system work, kept across crashes until reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashStats {
    /// Mutating calls, flushes and handle writes; each is a crash point.
    pub calls: u64,
    /// Renames, including replacement with a backup name.
    pub renames: u64,
    /// Name exchanges.
    pub exchanges: u64,
    /// Bytes written through handles.
    pub bytes_written: u64,
    /// Bytes copied by [`FsWrite::copy_file`].
    pub bytes_copied: u64,
    /// Clones made by [`FsWrite::clone_file`].
    pub clones: u64,
    /// Entries removed.
    pub removals: u64,
    /// File flushes.
    pub file_syncs: u64,
    /// Directory flushes.
    pub dir_syncs: u64,
    /// Simulated crashes.
    pub crashes: u64,
}

/// Outcome of [`CrashFs::crash`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashSummary {
    /// Seed of the selection of pending operations.
    pub seed: u64,
    /// Pending operations at the crash.
    pub pending: u64,
    /// Pending operations that reached the image.
    pub applied: u64,
    /// Writes that reached the image only in part.
    pub torn: u64,
}

/// Simulated file system kept in memory. A simulated crash discards
/// unflushed operations.
///
/// Mutations take a [`TxScope`] as the trait requires and do not inspect it.
/// After the call armed by [`CrashFs::arm_crash_at_call`] is reached, or once
/// the attached failpoint registry has crashed, every call fails with
/// [`FsErrorKind::Io`] until [`CrashFs::crash`] restarts the system.
///
/// # Examples
///
/// ```
/// use std::io::Write;
///
/// use nurt_contracts::fs::{AbsPath, Durability, Fs, FsWrite, ScopePurpose, TxScope};
/// use nurt_contracts::journal::OpId;
/// use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let root = AbsPath::new(if cfg!(windows) { "C:\\vol" } else { "/vol" })?;
/// let fs = CrashFs::new(CrashProfile::PosixWorstCase, 7, vec![VolumeSpec::new(root.clone())])?;
/// let scope = TxScope::new_for_journal(OpId(1), ScopePurpose::Maintenance);
/// let mut file = fs.create_new(&scope, &root.join_name("a")?)?;
/// file.write_all(b"data")?;
/// file.sync(Durability::Data)?;
/// fs.crash();
/// let entry = fs.metadata(&root.join_name("a")?, nurt_contracts::fs::Follow::NoFollow);
/// assert!(entry.is_err(), "the directory entry was not flushed");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct CrashFs {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug)]
struct State {
    profile: CrashProfile,
    seed: u64,
    volumes: Vec<Volume>,
    next_ino: NodeId,
    clock_ns: i64,
    stats: CrashStats,
    crash_at: Option<u64>,
    halted: bool,
    generation: u64,
    temp_counter: u64,
    locks: HashSet<PathBuf>,
    failpoints: Option<Arc<SeededFailpoints>>,
}

#[derive(Debug)]
struct Volume {
    spec: VolumeSpec,
    dev: u64,
    live: Image,
    durable: Image,
    pending: Vec<Pending>,
    inits: HashMap<NodeId, Node>,
    next_seq: u64,
}

#[derive(Clone, Debug)]
struct Image {
    nodes: HashMap<NodeId, Node>,
    used_bytes: u64,
    used_inodes: u64,
}

#[derive(Clone, Debug)]
struct Node {
    body: Body,
    meta: Meta,
    nlink: u64,
}

#[derive(Clone, Debug)]
enum Body {
    File(Arc<Vec<u8>>),
    Dir(BTreeMap<OsString, NodeId>),
    Link(PathBuf, LinkKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Meta {
    mode: u32,
    uid: u32,
    gid: u32,
    modified_ns: i64,
    accessed_ns: i64,
    created_ns: i64,
    changed_ns: i64,
    xattrs: Vec<Xattr>,
    acl: Option<Vec<u8>>,
    flags: Option<u64>,
    windows_attributes: u32,
}

impl Meta {
    fn new(mode: u32, now: i64) -> Self {
        Self {
            mode,
            uid: DEFAULT_UID,
            gid: DEFAULT_GID,
            modified_ns: now,
            accessed_ns: now,
            created_ns: now,
            changed_ns: now,
            xattrs: Vec::new(),
            acl: None,
            flags: None,
            windows_attributes: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct Pending {
    seq: u64,
    op: Op,
    unsynced_dirs: Vec<NodeId>,
}

#[derive(Clone, Debug)]
struct Entry {
    dir: NodeId,
    name: OsString,
}

#[derive(Clone, Debug)]
enum Op {
    Write {
        id: NodeId,
        offset: u64,
        data: Vec<u8>,
        time: i64,
    },
    SetLen {
        id: NodeId,
        length: u64,
        time: i64,
    },
    Content {
        id: NodeId,
        data: Arc<Vec<u8>>,
    },
    Meta {
        id: NodeId,
        meta: Box<Meta>,
    },
    Link {
        at: Entry,
        id: NodeId,
    },
    Unlink {
        at: Entry,
        id: NodeId,
    },
    Rename {
        from: Entry,
        to: Entry,
        id: NodeId,
    },
    Exchange {
        a: Entry,
        a_id: NodeId,
        b: Entry,
        b_id: NodeId,
    },
    ReplaceWithBackup {
        target: Entry,
        target_id: NodeId,
        replacement: Entry,
        replacement_id: NodeId,
        backup: Entry,
    },
}

impl Op {
    fn is_data(&self) -> bool {
        matches!(
            self,
            Self::Write { .. } | Self::SetLen { .. } | Self::Content { .. }
        )
    }

    fn data_node(&self) -> Option<NodeId> {
        match self {
            Self::Write { id, .. } | Self::SetLen { id, .. } | Self::Content { id, .. } => {
                Some(*id)
            }
            _ => None,
        }
    }

    fn dirs(&self) -> Vec<NodeId> {
        let mut dirs = match self {
            Self::Link { at, .. } | Self::Unlink { at, .. } => vec![at.dir],
            Self::Rename { from, to, .. } => vec![from.dir, to.dir],
            Self::Exchange { a, b, .. } => vec![a.dir, b.dir],
            Self::ReplaceWithBackup {
                target,
                replacement,
                backup,
                ..
            } => vec![target.dir, replacement.dir, backup.dir],
            _ => Vec::new(),
        };
        dirs.sort_unstable();
        dirs.dedup();
        dirs
    }
}

impl Image {
    fn new(now: i64) -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT_NODE,
            Node {
                body: Body::Dir(BTreeMap::new()),
                meta: Meta::new(DEFAULT_DIR_MODE, now),
                nlink: 1,
            },
        );
        Self {
            nodes,
            used_bytes: 0,
            used_inodes: 1,
        }
    }

    fn ensure(&mut self, id: NodeId, inits: &HashMap<NodeId, Node>) -> bool {
        if self.nodes.contains_key(&id) {
            return true;
        }
        match inits.get(&id) {
            Some(init) => {
                let mut node = init.clone();
                node.nlink = 0;
                self.nodes.insert(id, node);
                true
            }
            None => false,
        }
    }

    fn entry(&self, at: &Entry) -> Option<NodeId> {
        match self.nodes.get(&at.dir).map(|node| &node.body) {
            Some(Body::Dir(entries)) => entries.get(&at.name).copied(),
            _ => None,
        }
    }

    fn node_size(node: &Node) -> u64 {
        match &node.body {
            Body::File(data) => data.len() as u64,
            _ => 0,
        }
    }

    fn adjust_links(&mut self, id: NodeId, increment: bool) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let size = Self::node_size(node);
        if increment {
            node.nlink += 1;
            if node.nlink == 1 {
                self.used_bytes += size;
                self.used_inodes += 1;
            }
        } else if node.nlink > 0 {
            node.nlink -= 1;
            if node.nlink == 0 {
                self.used_bytes = self.used_bytes.saturating_sub(size);
                self.used_inodes = self.used_inodes.saturating_sub(1);
            }
        }
    }

    fn set_entry(&mut self, at: &Entry, id: Option<NodeId>, inits: &HashMap<NodeId, Node>) {
        if !self.ensure(at.dir, inits) {
            return;
        }
        if let Some(id) = id
            && !self.ensure(id, inits)
        {
            return;
        }
        let Some(Body::Dir(entries)) = self.nodes.get_mut(&at.dir).map(|node| &mut node.body)
        else {
            return;
        };
        let old = match id {
            Some(id) => entries.insert(at.name.clone(), id),
            None => entries.remove(&at.name),
        };
        if old == id {
            return;
        }
        if let Some(old) = old {
            self.adjust_links(old, false);
        }
        if let Some(id) = id {
            self.adjust_links(id, true);
        }
    }

    fn set_data(&mut self, id: NodeId, change: impl FnOnce(&mut Vec<u8>)) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let Body::File(data) = &mut node.body else {
            return;
        };
        let before = data.len() as u64;
        change(Arc::make_mut(data));
        let after = data.len() as u64;
        if node.nlink > 0 {
            self.used_bytes = (self.used_bytes + after).saturating_sub(before);
        }
    }

    fn apply(&mut self, op: &Op, inits: &HashMap<NodeId, Node>, torn: Option<usize>) {
        match op {
            Op::Write {
                id,
                offset,
                data,
                time,
            } => {
                if !self.ensure(*id, inits) {
                    return;
                }
                let bytes = torn.map_or(data.as_slice(), |prefix| &data[..prefix.min(data.len())]);
                self.set_data(*id, |content| write_at(content, *offset, bytes));
                self.touch(*id, *time);
            }
            Op::SetLen { id, length, time } => {
                if self.ensure(*id, inits) {
                    let length = usize::try_from(*length).unwrap_or(usize::MAX);
                    self.set_data(*id, |content| content.resize(length, 0));
                    self.touch(*id, *time);
                }
            }
            Op::Content { id, data } => {
                if self.ensure(*id, inits) {
                    self.set_data(*id, |content| content.clone_from(data));
                }
            }
            Op::Meta { id, meta } => {
                if self.ensure(*id, inits)
                    && let Some(node) = self.nodes.get_mut(id)
                {
                    node.meta = (**meta).clone();
                }
            }
            _ => self.apply_entries(op, inits),
        }
    }

    fn apply_entries(&mut self, op: &Op, inits: &HashMap<NodeId, Node>) {
        match op {
            Op::Link { at, id } => self.set_entry(at, Some(*id), inits),
            Op::Unlink { at, id } => {
                if self.entry(at) == Some(*id) {
                    self.set_entry(at, None, inits);
                }
            }
            Op::Rename { from, to, id } => self.move_entry(from, to, *id, inits),
            Op::Exchange { a, a_id, b, b_id } => {
                self.set_entry(a, Some(*b_id), inits);
                self.set_entry(b, Some(*a_id), inits);
            }
            Op::ReplaceWithBackup {
                target,
                target_id,
                replacement,
                replacement_id,
                backup,
            } => {
                self.move_entry(target, backup, *target_id, inits);
                self.move_entry(replacement, target, *replacement_id, inits);
            }
            _ => {}
        }
    }

    /// A persisted rename sets the destination even when the creation of the
    /// source never reached the image; the source name is removed only while
    /// it still denotes the renamed entry.
    fn move_entry(&mut self, from: &Entry, to: &Entry, id: NodeId, inits: &HashMap<NodeId, Node>) {
        self.set_entry(to, Some(id), inits);
        if self.entry(from) == Some(id) {
            self.set_entry(from, None, inits);
        }
    }

    fn touch(&mut self, id: NodeId, time: i64) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.meta.modified_ns = time;
            node.meta.changed_ns = time;
        }
    }

    fn collect_garbage(&mut self) {
        let mut reachable = HashSet::new();
        let mut stack = vec![ROOT_NODE];
        while let Some(id) = stack.pop() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some(Body::Dir(entries)) = self.nodes.get(&id).map(|node| &node.body) {
                stack.extend(entries.values().copied());
            }
        }
        self.nodes.retain(|id, _| reachable.contains(id));
        let mut links: HashMap<NodeId, u64> = HashMap::new();
        links.insert(ROOT_NODE, 1);
        for node in self.nodes.values() {
            if let Body::Dir(entries) = &node.body {
                for child in entries.values() {
                    *links.entry(*child).or_insert(0) += 1;
                }
            }
        }
        let mut bytes = 0;
        for (id, node) in &mut self.nodes {
            node.nlink = links.get(id).copied().unwrap_or(0);
            bytes += Self::node_size(node);
        }
        self.used_bytes = bytes;
        self.used_inodes = self.nodes.len() as u64;
    }
}

fn write_at(content: &mut Vec<u8>, offset: u64, bytes: &[u8]) {
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    let end = start.saturating_add(bytes.len());
    if content.len() < end {
        content.resize(end, 0);
    }
    if let Some(slot) = content.get_mut(start..end) {
        slot.copy_from_slice(bytes);
    }
}

fn error(kind: FsErrorKind, op: &str, path: &AbsPath) -> FsError {
    FsError::new(kind, op, Some(path.as_path()))
}

fn io_error(err: FsError) -> io::Error {
    let kind = match err.kind {
        FsErrorKind::NoSpace | FsErrorKind::QuotaExceeded => io::ErrorKind::StorageFull,
        FsErrorKind::Interrupted => io::ErrorKind::Interrupted,
        FsErrorKind::NotFound => io::ErrorKind::NotFound,
        FsErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, err)
}

/// Location of an entry in the live image.
#[derive(Clone, Debug)]
struct Located {
    vol: usize,
    id: NodeId,
    parent: Option<Entry>,
    path: AbsPath,
}

/// Parent directory and name for an entry to create or replace.
#[derive(Clone, Debug)]
struct Slot {
    vol: usize,
    at: Entry,
    existing: Option<NodeId>,
}

impl State {
    fn now(&mut self) -> i64 {
        self.clock_ns += CLOCK_TICK_NS;
        self.clock_ns
    }

    fn halted_error(op: &str, path: &AbsPath) -> FsError {
        error(FsErrorKind::Io, op, path)
    }

    fn is_stopped(&mut self) -> bool {
        if !self.halted
            && let Some(failpoints) = &self.failpoints
            && failpoints.is_crashed()
        {
            self.halted = true;
        }
        self.halted
    }

    /// Counts a crash point and fails when the system is stopped.
    fn enter(&mut self, op: &str, path: &AbsPath) -> FsResult<()> {
        if self.is_stopped() {
            return Err(Self::halted_error(op, path));
        }
        self.stats.calls += 1;
        if self.crash_at.is_some_and(|at| self.stats.calls >= at) {
            self.halt();
            return Err(Self::halted_error(op, path));
        }
        Ok(())
    }

    fn halt(&mut self) {
        self.halted = true;
        if let Some(failpoints) = &self.failpoints {
            failpoints.mark_crashed();
        }
    }

    fn check_read(&mut self, op: &str, path: &AbsPath) -> FsResult<()> {
        if self.is_stopped() {
            return Err(Self::halted_error(op, path));
        }
        Ok(())
    }

    fn volume_of(&self, path: &AbsPath) -> Option<usize> {
        self.volumes
            .iter()
            .enumerate()
            .filter(|(_, volume)| path.starts_with(&volume.spec.mount))
            .max_by_key(|(_, volume)| volume.spec.mount.as_path().components().count())
            .map(|(index, _)| index)
    }

    fn names(mount: &AbsPath, path: &AbsPath) -> Vec<OsString> {
        path.as_path()
            .strip_prefix(mount.as_path())
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

    fn resolve(&self, path: &AbsPath, follow: Follow, op: &str) -> FsResult<Located> {
        self.resolve_depth(path, follow, op, 0)
    }

    fn resolve_depth(
        &self,
        path: &AbsPath,
        follow: Follow,
        op: &str,
        depth: u32,
    ) -> FsResult<Located> {
        if depth > MAX_LINK_DEPTH {
            return Err(error(FsErrorKind::Loop, op, path));
        }
        let vol = self
            .volume_of(path)
            .ok_or_else(|| error(FsErrorKind::NotFound, op, path))?;
        let volume = &self.volumes[vol];
        let names = Self::names(&volume.spec.mount, path);
        let mut current = ROOT_NODE;
        let mut parent = None;
        let mut prefix = volume.spec.mount.clone();
        for (index, name) in names.iter().enumerate() {
            let entries = match volume.live.nodes.get(&current).map(|node| &node.body) {
                Some(Body::Dir(entries)) => entries,
                _ => return Err(error(FsErrorKind::NotADirectory, op, path)),
            };
            let child = *entries
                .get(name)
                .ok_or_else(|| error(FsErrorKind::NotFound, op, path))?;
            let last = index + 1 == names.len();
            if let Some(Body::Link(target, _)) =
                volume.live.nodes.get(&child).map(|node| &node.body)
                && (!last || follow == Follow::Follow)
            {
                let rest = names.get(index + 1..).unwrap_or_default();
                let next = Self::link_path(&prefix, target, rest, op, path)?;
                return self.resolve_depth(&next, follow, op, depth + 1);
            }
            parent = Some(Entry {
                dir: current,
                name: name.clone(),
            });
            prefix = Self::join(&prefix, name, op, path)?;
            current = child;
        }
        Ok(Located {
            vol,
            id: current,
            parent,
            path: path.clone(),
        })
    }

    fn join(base: &AbsPath, name: &OsStr, op: &str, path: &AbsPath) -> FsResult<AbsPath> {
        AbsPath::new(base.as_path().join(name))
            .map_err(|_| error(FsErrorKind::InvalidPath, op, path))
    }

    fn link_path(
        dir: &AbsPath,
        target: &Path,
        rest: &[OsString],
        op: &str,
        path: &AbsPath,
    ) -> FsResult<AbsPath> {
        let mut next = if target.is_absolute() {
            target.to_path_buf()
        } else {
            dir.as_path().join(target)
        };
        for name in rest {
            next.push(name);
        }
        AbsPath::new(next).map_err(|_| error(FsErrorKind::InvalidPath, op, path))
    }

    fn node(&self, located: &Located) -> FsResult<&Node> {
        self.volumes[located.vol]
            .live
            .nodes
            .get(&located.id)
            .ok_or_else(|| error(FsErrorKind::NotFound, "lookup", &located.path))
    }

    /// Parent directory (links followed) and name of `path`.
    fn slot(&self, path: &AbsPath, op: &str) -> FsResult<Slot> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(error(FsErrorKind::InvalidPath, op, path));
        };
        let dir = self.resolve(&parent, Follow::Follow, op)?;
        if self.volume_of(path) != Some(dir.vol) {
            return Err(error(FsErrorKind::InvalidPath, op, path));
        }
        let entries = match &self.node(&dir)?.body {
            Body::Dir(entries) => entries,
            _ => return Err(error(FsErrorKind::NotADirectory, op, path)),
        };
        Ok(Slot {
            vol: dir.vol,
            existing: entries.get(name).copied(),
            at: Entry {
                dir: dir.id,
                name: name.to_os_string(),
            },
        })
    }

    fn existing_slot(&self, path: &AbsPath, op: &str) -> FsResult<(Slot, NodeId)> {
        let slot = self.slot(path, op)?;
        let id = slot
            .existing
            .ok_or_else(|| error(FsErrorKind::NotFound, op, path))?;
        Ok((slot, id))
    }

    fn file_meta(&self, vol: usize, id: NodeId, node: &Node) -> FileMeta {
        let kind = match &node.body {
            Body::File(_) => FileKind::File,
            Body::Dir(_) => FileKind::Directory,
            Body::Link(_, LinkKind::Junction) => FileKind::Junction,
            Body::Link(..) => FileKind::Symlink,
        };
        let size = match &node.body {
            Body::File(data) => data.len() as u64,
            Body::Link(target, _) => target.as_os_str().len() as u64,
            Body::Dir(_) => 0,
        };
        FileMeta {
            kind,
            size,
            mode: node.meta.mode,
            uid: Some(node.meta.uid),
            gid: Some(node.meta.gid),
            links: node.nlink,
            modified_ns: node.meta.modified_ns,
            accessed_ns: Some(node.meta.accessed_ns),
            created_ns: Some(node.meta.created_ns),
            changed_ns: Some(node.meta.changed_ns),
            file_id: FileId {
                volume: self.volumes[vol].dev,
                low: id,
                high: 0,
            },
            windows_attributes: node.meta.windows_attributes,
        }
    }

    fn check_space(
        &self,
        vol: usize,
        bytes: u64,
        inodes: u64,
        op: &str,
        path: &AbsPath,
    ) -> FsResult<()> {
        let volume = &self.volumes[vol];
        let live = &volume.live;
        if live.used_inodes.saturating_add(inodes) > volume.spec.capacity_inodes
            || live.used_bytes.saturating_add(bytes) > volume.spec.capacity_bytes
        {
            return Err(error(FsErrorKind::NoSpace, op, path));
        }
        if volume
            .spec
            .quota_bytes
            .is_some_and(|quota| live.used_bytes.saturating_add(bytes) > quota)
        {
            return Err(error(FsErrorKind::QuotaExceeded, op, path));
        }
        Ok(())
    }

    fn push(&mut self, vol: usize, op: Op) {
        let volume = &mut self.volumes[vol];
        volume.live.apply(&op, &volume.inits, None);
        let seq = volume.next_seq;
        volume.next_seq += 1;
        let unsynced_dirs = op.dirs();
        volume.pending.push(Pending {
            seq,
            op,
            unsynced_dirs,
        });
    }

    fn new_node(&mut self, vol: usize, body: Body, mode: u32) -> NodeId {
        let now = self.now();
        let id = self.next_ino;
        self.next_ino += 1;
        let node = Node {
            body,
            meta: Meta::new(mode, now),
            nlink: 0,
        };
        let volume = &mut self.volumes[vol];
        volume.live.nodes.insert(id, node.clone());
        volume.inits.insert(id, node);
        id
    }

    fn create_file(&mut self, slot: &Slot, path: &AbsPath, op: &str) -> FsResult<NodeId> {
        if slot.existing.is_some() {
            return Err(error(FsErrorKind::Exists, op, path));
        }
        self.check_space(slot.vol, 0, 1, op, path)?;
        let id = self.new_node(
            slot.vol,
            Body::File(Arc::new(Vec::new())),
            DEFAULT_FILE_MODE,
        );
        self.push(
            slot.vol,
            Op::Link {
                at: slot.at.clone(),
                id,
            },
        );
        Ok(id)
    }

    fn make_durable(&mut self, vol: usize, selected: impl Fn(&Pending) -> bool) {
        let volume = &mut self.volumes[vol];
        let (durable, rest): (Vec<Pending>, Vec<Pending>) = volume
            .pending
            .drain(..)
            .partition(|pending| selected(pending));
        for pending in &durable {
            volume.durable.apply(&pending.op, &volume.inits, None);
        }
        volume.pending = rest;
    }

    fn commit_metadata(&mut self, vol: usize) {
        let rules = self.profile.rules();
        let everything = rules.data_with_commit || matches!(rules.order, Order::Global);
        self.make_durable(vol, |pending| everything || !pending.op.is_data());
    }

    fn sync_file(&mut self, vol: usize, id: NodeId, durability: Durability) {
        self.stats.file_syncs += 1;
        let rules = self.profile.rules();
        if durability == Durability::Data && !rules.data_sync_durable {
            return;
        }
        if rules.flush_commits_metadata {
            self.commit_metadata(vol);
        }
        let full = durability == Durability::Full;
        self.make_durable(vol, |pending| match &pending.op {
            Op::Meta { id: target, .. } => full && *target == id,
            op => op.data_node() == Some(id),
        });
    }

    fn sync_dir(&mut self, vol: usize, dir: NodeId) {
        self.stats.dir_syncs += 1;
        if self.profile.rules().flush_commits_metadata {
            self.commit_metadata(vol);
            return;
        }
        for pending in &mut self.volumes[vol].pending {
            pending.unsynced_dirs.retain(|unsynced| *unsynced != dir);
        }
        self.make_durable(vol, |pending| match &pending.op {
            Op::Meta { id, .. } => *id == dir,
            op => !op.is_data() && pending.unsynced_dirs.is_empty(),
        });
    }

    fn crash(&mut self) -> CrashSummary {
        let seed = self.seed ^ (self.stats.crashes + 1).wrapping_mul(SEED_STRIDE);
        let mut rng = SplitMix64::new(seed);
        let rules = self.profile.rules();
        let mut summary = CrashSummary {
            seed,
            pending: 0,
            applied: 0,
            torn: 0,
        };
        for volume in &mut self.volumes {
            let mut image = volume.durable.clone();
            let selection = select(&volume.pending, rules, &mut rng);
            summary.pending += volume.pending.len() as u64;
            for (pending, choice) in volume.pending.iter().zip(selection) {
                if let Choice::Apply(torn) = choice {
                    summary.applied += 1;
                    summary.torn += u64::from(torn.is_some());
                    image.apply(&pending.op, &volume.inits, torn);
                }
            }
            image.collect_garbage();
            volume.live = image.clone();
            volume.durable = image;
            volume.pending.clear();
            volume.inits.clear();
        }
        self.stats.crashes += 1;
        self.generation += 1;
        self.halted = false;
        self.crash_at = None;
        self.locks.clear();
        summary
    }
}

#[derive(Clone, Copy, Debug)]
enum Choice {
    Skip,
    Apply(Option<usize>),
}

fn torn_length(op: &Op, sector: Option<u64>, rng: &mut SplitMix64) -> Option<usize> {
    let (Op::Write { data, .. }, Some(sector)) = (op, sector) else {
        return None;
    };
    if data.is_empty() || !rng.chance(TORN_PER_MILLION) {
        return None;
    }
    let prefix = rng.below(data.len() as u64);
    usize::try_from(prefix - prefix % sector).ok()
}

fn select(pending: &[Pending], rules: ProfileRules, rng: &mut SplitMix64) -> Vec<Choice> {
    let keep = u32::try_from(rng.below(PER_MILLION + 1)).unwrap_or(u32::MAX);
    match rules.order {
        Order::Unordered => pending
            .iter()
            .map(|item| {
                if rng.chance(keep) {
                    Choice::Apply(torn_length(&item.op, rules.torn_sector, rng))
                } else {
                    Choice::Skip
                }
            })
            .collect(),
        Order::Global => {
            let cut = rng.below(pending.len() as u64 + 1);
            (0..pending.len() as u64)
                .map(|index| {
                    if index < cut {
                        Choice::Apply(None)
                    } else {
                        Choice::Skip
                    }
                })
                .collect()
        }
        Order::MetadataPrefix => select_prefix(pending, rules, keep, rng),
    }
}

/// Metadata survives as a prefix of its journal; file data survives
/// independently, except that data written before the last committed
/// metadata operation is durable when the profile flushes it with commits.
fn select_prefix(
    pending: &[Pending],
    rules: ProfileRules,
    keep: u32,
    rng: &mut SplitMix64,
) -> Vec<Choice> {
    let metadata = pending.iter().filter(|item| !item.op.is_data()).count() as u64;
    let cut = rng.below(metadata + 1);
    let boundary = pending
        .iter()
        .filter(|item| !item.op.is_data())
        .nth(usize::try_from(cut).unwrap_or(usize::MAX))
        .map_or(u64::MAX, |item| item.seq);
    let mut seen = 0;
    pending
        .iter()
        .map(|item| {
            if !item.op.is_data() {
                seen += 1;
                return if seen <= cut {
                    Choice::Apply(None)
                } else {
                    Choice::Skip
                };
            }
            if rules.data_with_commit && item.seq < boundary {
                return Choice::Apply(None);
            }
            if rng.chance(keep) {
                Choice::Apply(torn_length(&item.op, rules.torn_sector, rng))
            } else {
                Choice::Skip
            }
        })
        .collect()
}

impl CrashFs {
    /// Creates a file system from volume descriptions. Every volume starts
    /// with an empty root directory, already durable.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when no volume is given or two
    /// volumes share a mount point.
    pub fn new(profile: CrashProfile, seed: u64, volumes: Vec<VolumeSpec>) -> FsResult<Self> {
        let mut mounts = HashSet::new();
        for volume in &volumes {
            if !mounts.insert(volume.mount.clone()) {
                return Err(error(FsErrorKind::InvalidPath, "crashfs", &volume.mount));
            }
        }
        if volumes.is_empty() {
            return Err(FsError::new(FsErrorKind::InvalidPath, "crashfs", None));
        }
        let volumes = volumes
            .into_iter()
            .zip(FIRST_DEVICE..)
            .map(|(spec, dev)| {
                let image = Image::new(CLOCK_START_NS);
                Volume {
                    spec,
                    dev,
                    live: image.clone(),
                    durable: image,
                    pending: Vec::new(),
                    inits: HashMap::new(),
                    next_seq: 0,
                }
            })
            .collect();
        let state = State {
            profile,
            seed,
            volumes,
            next_ino: ROOT_NODE + 1,
            clock_ns: CLOCK_START_NS,
            stats: CrashStats::default(),
            crash_at: None,
            halted: false,
            generation: 0,
            temp_counter: 0,
            locks: HashSet::new(),
            failpoints: None,
        };
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.shared.lock()
    }

    /// Persistence profile.
    pub fn profile(&self) -> CrashProfile {
        self.state().profile
    }

    /// Links the system to a failpoint registry: a crash decided by the
    /// registry stops the system, and a crash triggered by
    /// [`CrashFs::arm_crash_at_call`] is reported to the registry.
    pub fn attach_failpoints(&self, failpoints: Arc<SeededFailpoints>) {
        self.state().failpoints = Some(failpoints);
    }

    /// Removes the link to the failpoint registry.
    pub fn detach_failpoints(&self) {
        self.state().failpoints = None;
    }

    /// Stops the system when call number `call` (counted from one, see
    /// [`CrashStats::calls`]) begins; that call has no effect.
    pub fn arm_crash_at_call(&self, call: u64) {
        self.state().crash_at = Some(call);
    }

    /// Stops the system now, as a crash decided by the caller.
    pub fn halt(&self) {
        self.state().halt();
    }

    /// Whether the system is stopped and waits for [`CrashFs::crash`].
    pub fn is_halted(&self) -> bool {
        self.state().is_stopped()
    }

    /// Simulates power loss: builds the image from durable state and a
    /// seeded selection of pending operations, discards open handles and
    /// locks, and restarts the system on that image.
    pub fn crash(&self) -> CrashSummary {
        self.state().crash()
    }

    /// Makes every pending operation durable, as `sync(2)` does.
    pub fn sync_all(&self) {
        let mut state = self.state();
        for vol in 0..state.volumes.len() {
            state.make_durable(vol, |_| true);
        }
    }

    /// Number of operations not yet durable.
    pub fn pending_count(&self) -> usize {
        self.state()
            .volumes
            .iter()
            .map(|volume| volume.pending.len())
            .sum()
    }

    /// Counters since creation or the last [`CrashFs::reset_stats`].
    pub fn stats(&self) -> CrashStats {
        self.state().stats
    }

    /// Resets the counters; the call counter used by
    /// [`CrashFs::arm_crash_at_call`] restarts at zero.
    pub fn reset_stats(&self) {
        self.state().stats = CrashStats::default();
    }

    fn handle(
        &self,
        vol: usize,
        id: NodeId,
        path: AbsPath,
        generation: u64,
    ) -> Box<dyn WriteHandle> {
        Box::new(CrashWriter {
            shared: Arc::clone(&self.shared),
            vol,
            id,
            path,
            position: 0,
            generation,
        })
    }

    fn remove_entry(state: &mut State, path: &AbsPath, op: &str, directory: bool) -> FsResult<()> {
        let (slot, id) = state.existing_slot(path, op)?;
        let node = state.volumes[slot.vol]
            .live
            .nodes
            .get(&id)
            .ok_or_else(|| error(FsErrorKind::NotFound, op, path))?;
        match (&node.body, directory) {
            (Body::Dir(_), false) => return Err(error(FsErrorKind::IsADirectory, op, path)),
            (Body::Dir(entries), true) if !entries.is_empty() => {
                return Err(error(FsErrorKind::NotEmpty, op, path));
            }
            (Body::File(_) | Body::Link(..), true) => {
                return Err(error(FsErrorKind::NotADirectory, op, path));
            }
            _ => {}
        }
        if path == &state.volumes[slot.vol].spec.mount {
            return Err(error(FsErrorKind::InvalidPath, op, path));
        }
        state.stats.removals += 1;
        state.push(slot.vol, Op::Unlink { at: slot.at, id });
        Ok(())
    }

    fn check_rename_target(
        state: &State,
        from: &Located,
        to: &Slot,
        mode: RenameMode,
        path: &AbsPath,
    ) -> FsResult<()> {
        let Some(existing) = to.existing else {
            return Ok(());
        };
        if mode == RenameMode::NoReplace {
            return Err(error(FsErrorKind::Exists, "rename", path));
        }
        let nodes = &state.volumes[to.vol].live.nodes;
        let source_dir = matches!(
            nodes.get(&from.id).map(|node| &node.body),
            Some(Body::Dir(_))
        );
        match nodes.get(&existing).map(|node| &node.body) {
            Some(Body::Dir(_)) if !source_dir => {
                Err(error(FsErrorKind::IsADirectory, "rename", path))
            }
            Some(Body::Dir(entries)) if !entries.is_empty() => {
                Err(error(FsErrorKind::NotEmpty, "rename", path))
            }
            Some(Body::File(_) | Body::Link(..)) if source_dir => {
                Err(error(FsErrorKind::NotADirectory, "rename", path))
            }
            _ => Ok(()),
        }
    }

    /// Fails when `inner` lies inside the directory `outer`.
    fn check_not_inside(outer: &AbsPath, inner: &AbsPath, op: &str) -> FsResult<()> {
        if inner != outer && inner.starts_with(outer) {
            return Err(error(FsErrorKind::InvalidPath, op, inner));
        }
        Ok(())
    }

    fn source(state: &State, path: &AbsPath, op: &str) -> FsResult<(Located, Entry)> {
        let located = state.resolve(path, Follow::NoFollow, op)?;
        let parent = located
            .parent
            .clone()
            .ok_or_else(|| error(FsErrorKind::InvalidPath, op, path))?;
        Ok((located, parent))
    }

    fn file_content(
        state: &State,
        path: &AbsPath,
        op: &str,
    ) -> FsResult<(Located, Arc<Vec<u8>>, Meta)> {
        let located = state.resolve(path, Follow::Follow, op)?;
        let node = state.node(&located)?;
        match &node.body {
            Body::File(data) => Ok((located.clone(), Arc::clone(data), node.meta.clone())),
            Body::Dir(_) => Err(error(FsErrorKind::IsADirectory, op, path)),
            Body::Link(..) => Err(error(FsErrorKind::InvalidPath, op, path)),
        }
    }

    fn duplicate(&self, src: &AbsPath, dst: &AbsPath, clone: bool) -> FsResult<u64> {
        let op = if clone { "clone_file" } else { "copy_file" };
        let mut state = self.state();
        state.enter(op, dst)?;
        let (source, data, meta) = Self::file_content(&state, src, op)?;
        let slot = state.slot(dst, op)?;
        if clone {
            if !state.volumes[slot.vol].spec.features.reflink {
                return Err(error(FsErrorKind::Unsupported, op, dst));
            }
            if source.vol != slot.vol {
                return Err(error(FsErrorKind::CrossDevice, op, dst));
            }
        }
        let size = data.len() as u64;
        state.check_space(slot.vol, size, 0, op, dst)?;
        let id = state.create_file(&slot, dst, op)?;
        let now = state.now();
        let mode = meta.mode;
        let mut copied = Meta::new(mode, now);
        copied.xattrs = meta.xattrs;
        state.push(slot.vol, Op::Content { id, data });
        state.push(
            slot.vol,
            Op::Meta {
                id,
                meta: Box::new(copied),
            },
        );
        if clone {
            state.stats.clones += 1;
        } else {
            state.stats.bytes_copied += size;
        }
        Ok(size)
    }

    fn restored_meta(
        spec: &VolumeSpec,
        current: &Meta,
        meta: &ExtendedMeta,
    ) -> (Meta, MetaTransferReport) {
        let mut report = MetaTransferReport::default();
        let mut next = current.clone();
        next.mode = meta.basic.mode;
        next.uid = meta.basic.uid.unwrap_or(current.uid);
        next.gid = meta.basic.gid.unwrap_or(current.gid);
        next.modified_ns = meta.basic.modified_ns;
        next.accessed_ns = meta.basic.accessed_ns.unwrap_or(current.accessed_ns);
        next.created_ns = meta.basic.created_ns.unwrap_or(current.created_ns);
        next.windows_attributes = meta.basic.windows_attributes;
        next.flags = meta.flags;
        if spec.features.xattr {
            next.xattrs.clone_from(&meta.xattrs);
        } else {
            report.not_transferred.extend(
                meta.xattrs
                    .iter()
                    .map(|xattr| format!("xattr:{}", xattr.name.to_string_lossy())),
            );
        }
        if spec.features.acl {
            next.acl.clone_from(&meta.acl);
        } else if meta.acl.is_some() {
            report.not_transferred.push("acl".to_owned());
        }
        if meta.account_sid.is_some() {
            report.not_transferred.push("account".to_owned());
        }
        report.not_transferred.extend(
            meta.streams
                .iter()
                .map(|stream| format!("stream:{}", stream.name)),
        );
        (next, report)
    }

    fn changed_meta(
        spec: &VolumeSpec,
        current: &Meta,
        change: &MetaChange,
        now: i64,
        path: &AbsPath,
    ) -> FsResult<Meta> {
        let wants_xattrs = !change.set_xattrs.is_empty() || !change.remove_xattrs.is_empty();
        if (wants_xattrs && !spec.features.xattr) || (change.acl.is_some() && !spec.features.acl) {
            return Err(error(FsErrorKind::Unsupported, "set_meta", path));
        }
        let mut next = current.clone();
        next.mode = change.mode.unwrap_or(next.mode);
        next.uid = change.uid.unwrap_or(next.uid);
        next.gid = change.gid.unwrap_or(next.gid);
        next.modified_ns = change.modified_ns.unwrap_or(next.modified_ns);
        next.accessed_ns = change.accessed_ns.unwrap_or(next.accessed_ns);
        next.created_ns = change.created_ns.unwrap_or(next.created_ns);
        next.xattrs
            .retain(|xattr| !change.remove_xattrs.contains(&xattr.name));
        for xattr in &change.set_xattrs {
            next.xattrs.retain(|existing| existing.name != xattr.name);
            next.xattrs.push(xattr.clone());
        }
        next.xattrs.sort_by(|a, b| a.name.cmp(&b.name));
        if change.acl.is_some() {
            next.acl.clone_from(&change.acl);
        }
        next.windows_attributes = (next.windows_attributes | change.set_windows_attributes)
            & !change.clear_windows_attributes;
        next.changed_ns = now;
        Ok(next)
    }

    fn update_meta(
        &self,
        path: &AbsPath,
        op: &str,
        follow: Follow,
        change: impl FnOnce(&VolumeSpec, &Meta, i64) -> FsResult<(Meta, MetaTransferReport)>,
    ) -> FsResult<MetaTransferReport> {
        let mut state = self.state();
        state.enter(op, path)?;
        let located = state.resolve(path, follow, op)?;
        let current = state.node(&located)?.meta.clone();
        let now = state.now();
        let (meta, report) = change(&state.volumes[located.vol].spec, &current, now)?;
        state.push(
            located.vol,
            Op::Meta {
                id: located.id,
                meta: Box::new(meta),
            },
        );
        Ok(report)
    }

    fn remove_tree_entries(state: &State, root: &Located) -> Vec<(Entry, NodeId, bool, u64)> {
        let nodes = &state.volumes[root.vol].live.nodes;
        let mut order = Vec::new();
        let mut stack = vec![(root.parent.clone(), root.id, false)];
        while let Some((entry, id, expanded)) = stack.pop() {
            let Some(node) = nodes.get(&id) else {
                continue;
            };
            match (&node.body, expanded) {
                (Body::Dir(entries), false) => {
                    stack.push((entry, id, true));
                    for (name, child) in entries.iter().rev() {
                        stack.push((
                            Some(Entry {
                                dir: id,
                                name: name.clone(),
                            }),
                            *child,
                            false,
                        ));
                    }
                }
                (body, _) => {
                    if let Some(entry) = entry {
                        let is_dir = matches!(body, Body::Dir(_));
                        order.push((entry, id, is_dir, Image::node_size(node)));
                    }
                }
            }
        }
        order
    }
}

impl Fs for CrashFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let mut state = self.state();
        state.check_read("metadata", path)?;
        let located = state.resolve(path, follow, "metadata")?;
        let node = state.node(&located)?;
        Ok(state.file_meta(located.vol, located.id, node))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        let mut state = self.state();
        state.check_read("extended_metadata", path)?;
        let located = state.resolve(path, follow, "extended_metadata")?;
        let node = state.node(&located)?;
        let link_target = match &node.body {
            Body::Link(target, _) => Some(target.clone()),
            _ => None,
        };
        Ok(ExtendedMeta {
            basic: state.file_meta(located.vol, located.id, node),
            xattrs: node.meta.xattrs.clone(),
            acl: node.meta.acl.clone(),
            account_sid: None,
            flags: node.meta.flags,
            streams: Vec::new(),
            link_target,
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let mut state = self.state();
        state.check_read("open_read", path)?;
        let (located, data, _) = Self::file_content(&state, path, "open_read")?;
        let node = state.node(&located)?;
        let meta = state.file_meta(located.vol, located.id, node);
        Ok(Box::new(CrashReader {
            data,
            position: 0,
            meta,
        }))
    }

    fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        Err(error(FsErrorKind::Unsupported, "open_stream_read", path))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let mut state = self.state();
        state.check_read("read_dir", path)?;
        let located = state.resolve(path, Follow::Follow, "read_dir")?;
        let nodes = &state.volumes[located.vol].live.nodes;
        let Some(Body::Dir(entries)) = nodes.get(&located.id).map(|node| &node.body) else {
            return Err(error(FsErrorKind::NotADirectory, "read_dir", path));
        };
        let listed: Vec<FsResult<DirEntry>> = entries
            .iter()
            .map(|(name, child)| {
                let kind = nodes.get(child).map(|node| match &node.body {
                    Body::File(_) => FileKind::File,
                    Body::Dir(_) => FileKind::Directory,
                    Body::Link(_, LinkKind::Junction) => FileKind::Junction,
                    Body::Link(..) => FileKind::Symlink,
                });
                Ok(DirEntry {
                    name: name.clone(),
                    kind,
                })
            })
            .collect();
        Ok(Box::new(listed.into_iter()))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        let mut state = self.state();
        state.check_read("read_link", path)?;
        let located = state.resolve(path, Follow::NoFollow, "read_link")?;
        match &state.node(&located)?.body {
            Body::Link(target, _) => Ok(target.clone()),
            _ => Err(error(FsErrorKind::InvalidPath, "read_link", path)),
        }
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let mut state = self.state();
        state.check_read("real_path", path)?;
        let mut current = path.clone();
        for _ in 0..=MAX_LINK_DEPTH {
            let located = state.resolve(&current, Follow::NoFollow, "real_path")?;
            let Body::Link(target, _) = &state.node(&located)?.body else {
                return Ok(current);
            };
            let dir = current
                .parent()
                .ok_or_else(|| error(FsErrorKind::InvalidPath, "real_path", path))?;
            current = State::link_path(&dir, target, &[], "real_path", path)?;
        }
        Err(error(FsErrorKind::Loop, "real_path", path))
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let mut state = self.state();
        state.check_read("content_hash", path)?;
        let (_, data, _) = Self::file_content(&state, path, "content_hash")?;
        Ok(ContentHash(Sha256::digest(data.as_slice()).into()))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        let state = self.state();
        let vol = state
            .volume_of(path)
            .ok_or_else(|| error(FsErrorKind::NotFound, "volume", path))?;
        let volume = &state.volumes[vol];
        Ok(VolumeInfo {
            id: VolumeId {
                dev: volume.dev,
                uuid: None,
                subvolume: 0,
            },
            mount: volume.spec.mount.clone(),
            fs_type: volume.spec.fs_type.clone(),
            kind: volume.spec.kind,
            features: volume.spec.features,
            rotational: false,
            read_only: false,
        })
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        let state = self.state();
        let found = state
            .volumes
            .iter()
            .find(|candidate| candidate.dev == volume.dev)
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "space", None))?;
        let used = found.live.used_bytes;
        let total = found
            .spec
            .quota_bytes
            .map_or(found.spec.capacity_bytes, |quota| {
                quota.min(found.spec.capacity_bytes)
            });
        Ok(SpaceInfo {
            total,
            available: total.saturating_sub(used),
            quota_remaining: found
                .spec
                .quota_bytes
                .map(|quota| quota.saturating_sub(used)),
            purgeable_system: None,
        })
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        self.volume(path)?;
        Ok(PathKey(
            path.as_path().as_os_str().as_encoded_bytes().to_vec(),
        ))
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        self.metadata(path, Follow::NoFollow)?;
        Ok(Vec::new())
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        let meta = self.metadata(dir, Follow::Follow)?;
        if meta.kind != FileKind::Directory {
            return Err(error(FsErrorKind::NotADirectory, "can_write_dir", dir));
        }
        Ok(true)
    }
}

impl FsWrite for CrashFs {
    fn create_temp_in(
        &self,
        _scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let mut state = self.state();
        state.enter("create_temp_in", dir)?;
        let located = state.resolve(dir, Follow::Follow, "create_temp_in")?;
        let entries = match &state.node(&located)?.body {
            Body::Dir(entries) => entries.clone(),
            _ => return Err(error(FsErrorKind::NotADirectory, "create_temp_in", dir)),
        };
        let name = loop {
            state.temp_counter += 1;
            let candidate = OsString::from(format!("{name_hint}{:08x}", state.temp_counter));
            if !entries.contains_key(&candidate) {
                break candidate;
            }
        };
        let path = State::join(dir, &name, "create_temp_in", dir)?;
        let slot = Slot {
            vol: located.vol,
            at: Entry {
                dir: located.id,
                name,
            },
            existing: None,
        };
        let id = state.create_file(&slot, &path, "create_temp_in")?;
        let generation = state.generation;
        Ok(self.handle(located.vol, id, path, generation))
    }

    fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let mut state = self.state();
        state.enter("create_new", path)?;
        let slot = state.slot(path, "create_new")?;
        let id = state.create_file(&slot, path, "create_new")?;
        let generation = state.generation;
        Ok(self.handle(slot.vol, id, path.clone(), generation))
    }

    fn open_in_place(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let mut state = self.state();
        state.enter("open_in_place", path)?;
        let (located, _, _) = Self::file_content(&state, path, "open_in_place")?;
        let generation = state.generation;
        Ok(self.handle(located.vol, located.id, path.clone(), generation))
    }

    fn open_stream_write(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        Err(error(FsErrorKind::Unsupported, "open_stream_write", path))
    }

    fn rename(
        &self,
        _scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        let mut state = self.state();
        state.enter("rename", from)?;
        let (source, from_entry) = Self::source(&state, from, "rename")?;
        let target = state.slot(to, "rename")?;
        if source.vol != target.vol {
            return Err(error(FsErrorKind::CrossDevice, "rename", to));
        }
        if mode == RenameMode::NoReplace
            && !state.volumes[target.vol].spec.features.rename_noreplace
        {
            return Err(error(FsErrorKind::Unsupported, "rename", to));
        }
        if target.existing == Some(source.id) {
            return Ok(());
        }
        Self::check_not_inside(from, to, "rename")?;
        Self::check_rename_target(&state, &source, &target, mode, to)?;
        state.stats.renames += 1;
        state.push(
            source.vol,
            Op::Rename {
                from: from_entry,
                to: target.at,
                id: source.id,
            },
        );
        Ok(())
    }

    fn exchange(&self, _scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.enter("exchange", a)?;
        let (first, a_entry) = Self::source(&state, a, "exchange")?;
        let (second, b_entry) = Self::source(&state, b, "exchange")?;
        if !state.volumes[first.vol].spec.features.exchange {
            return Err(error(FsErrorKind::Unsupported, "exchange", a));
        }
        if first.vol != second.vol {
            return Err(error(FsErrorKind::CrossDevice, "exchange", b));
        }
        Self::check_not_inside(a, b, "exchange")?;
        Self::check_not_inside(b, a, "exchange")?;
        state.stats.exchanges += 1;
        state.push(
            first.vol,
            Op::Exchange {
                a: a_entry,
                a_id: first.id,
                b: b_entry,
                b_id: second.id,
            },
        );
        Ok(())
    }

    fn replace_with_backup(
        &self,
        _scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        let mut state = self.state();
        state.enter("replace_with_backup", target)?;
        let (old, target_entry) = Self::source(&state, target, "replace_with_backup")?;
        if !state.volumes[old.vol].spec.features.replace_with_backup {
            return Err(error(
                FsErrorKind::Unsupported,
                "replace_with_backup",
                target,
            ));
        }
        let (new, replacement_entry) = Self::source(&state, replacement, "replace_with_backup")?;
        let backup_slot = state.slot(backup, "replace_with_backup")?;
        if old.vol != new.vol || old.vol != backup_slot.vol {
            return Err(error(
                FsErrorKind::CrossDevice,
                "replace_with_backup",
                replacement,
            ));
        }
        let nodes = &state.volumes[old.vol].live.nodes;
        let is_dir =
            |id: NodeId| matches!(nodes.get(&id).map(|node| &node.body), Some(Body::Dir(_)));
        if is_dir(old.id) || is_dir(new.id) || backup_slot.existing.is_some_and(is_dir) {
            return Err(error(
                FsErrorKind::IsADirectory,
                "replace_with_backup",
                target,
            ));
        }
        state.stats.renames += 2;
        state.push(
            old.vol,
            Op::ReplaceWithBackup {
                target: target_entry,
                target_id: old.id,
                replacement: replacement_entry,
                replacement_id: new.id,
                backup: backup_slot.at,
            },
        );
        Ok(())
    }

    fn hard_link(&self, _scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.enter("hard_link", new)?;
        let source = state.resolve(existing, Follow::NoFollow, "hard_link")?;
        if !state.volumes[source.vol].spec.features.hardlink {
            return Err(error(FsErrorKind::Unsupported, "hard_link", new));
        }
        if matches!(state.node(&source)?.body, Body::Dir(_)) {
            return Err(error(FsErrorKind::PermissionDenied, "hard_link", existing));
        }
        let slot = state.slot(new, "hard_link")?;
        if slot.vol != source.vol {
            return Err(error(FsErrorKind::CrossDevice, "hard_link", new));
        }
        if slot.existing.is_some() {
            return Err(error(FsErrorKind::Exists, "hard_link", new));
        }
        state.push(
            slot.vol,
            Op::Link {
                at: slot.at,
                id: source.id,
            },
        );
        Ok(())
    }

    fn clone_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        self.duplicate(src, dst, true).map(|_| ())
    }

    fn copy_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        self.duplicate(src, dst, false)
    }

    fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.enter("remove_file", path)?;
        Self::remove_entry(&mut state, path, "remove_file", false)
    }

    fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.enter("remove_dir", path)?;
        Self::remove_entry(&mut state, path, "remove_dir", true)
    }

    fn remove_tree(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _parallelism: usize,
    ) -> FsResult<RemoveStats> {
        let mut state = self.state();
        let root = state.resolve(path, Follow::NoFollow, "remove_tree")?;
        if root.parent.is_none() {
            return Err(error(FsErrorKind::InvalidPath, "remove_tree", path));
        }
        let mut stats = RemoveStats::default();
        for (entry, id, is_dir, bytes) in Self::remove_tree_entries(&state, &root) {
            state.enter("remove_tree", path)?;
            state.stats.removals += 1;
            state.push(root.vol, Op::Unlink { at: entry, id });
            if is_dir {
                stats.directories += 1;
            } else {
                stats.files += 1;
                stats.bytes += bytes;
            }
        }
        Ok(stats)
    }

    fn create_dir(&self, _scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        let mut state = self.state();
        state.enter("create_dir", path)?;
        let slot = state.slot(path, "create_dir")?;
        if slot.existing.is_some() {
            return Err(error(FsErrorKind::Exists, "create_dir", path));
        }
        state.check_space(slot.vol, 0, 1, "create_dir", path)?;
        let id = state.new_node(slot.vol, Body::Dir(BTreeMap::new()), mode);
        state.push(slot.vol, Op::Link { at: slot.at, id });
        Ok(())
    }

    fn create_link(
        &self,
        _scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        let mut state = self.state();
        state.enter("create_link", link)?;
        let slot = state.slot(link, "create_link")?;
        if slot.existing.is_some() {
            return Err(error(FsErrorKind::Exists, "create_link", link));
        }
        state.check_space(slot.vol, 0, 1, "create_link", link)?;
        let id = state.new_node(slot.vol, Body::Link(target.to_path_buf(), kind), LINK_MODE);
        state.push(slot.vol, Op::Link { at: slot.at, id });
        Ok(())
    }

    fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        self.update_meta(path, "set_meta", Follow::NoFollow, |spec, current, now| {
            Self::changed_meta(spec, current, change, now, path)
                .map(|meta| (meta, MetaTransferReport::default()))
        })
        .map(|_| ())
    }

    fn restore_meta(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        self.update_meta(
            path,
            "restore_meta",
            Follow::NoFollow,
            |spec, current, _| Ok(Self::restored_meta(spec, current, meta)),
        )
    }

    fn truncate(&self, _scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        let mut state = self.state();
        state.enter("truncate", path)?;
        let (located, data, _) = Self::file_content(&state, path, "truncate")?;
        let grow = length.saturating_sub(data.len() as u64);
        state.check_space(located.vol, grow, 0, "truncate", path)?;
        let time = state.now();
        state.push(
            located.vol,
            Op::SetLen {
                id: located.id,
                length,
                time,
            },
        );
        Ok(())
    }

    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.enter("sync_dir", dir)?;
        let located = state.resolve(dir, Follow::Follow, "sync_dir")?;
        if !matches!(state.node(&located)?.body, Body::Dir(_)) {
            return Err(error(FsErrorKind::NotADirectory, "sync_dir", dir));
        }
        state.sync_dir(located.vol, located.id);
        Ok(())
    }

    fn lock_exclusive(&self, path: &AbsPath, _timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        let mut state = self.state();
        state.enter("lock_exclusive", path)?;
        let slot = state.slot(path, "lock_exclusive")?;
        if slot.existing.is_none() {
            state.create_file(&slot, path, "lock_exclusive")?;
        }
        if !state.locks.insert(path.as_path().to_path_buf()) {
            return Err(error(FsErrorKind::InUse, "lock_exclusive", path));
        }
        Ok(Box::new(CrashLock {
            shared: Arc::clone(&self.shared),
            path: path.clone(),
            generation: state.generation,
        }))
    }

    fn mark_hidden(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let hidden = MetaChange {
            set_windows_attributes: FILE_ATTRIBUTE_HIDDEN
                | FILE_ATTRIBUTE_SYSTEM
                | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED,
            ..MetaChange::default()
        };
        self.update_meta(path, "mark_hidden", Follow::Follow, |spec, current, now| {
            Self::changed_meta(spec, current, &hidden, now, path)
                .map(|meta| (meta, MetaTransferReport::default()))
        })
        .map(|_| ())
    }
}

/// Write handle of [`CrashFs`]; fails after the system crashed.
#[derive(Debug)]
struct CrashWriter {
    shared: Arc<Shared>,
    vol: usize,
    id: NodeId,
    path: AbsPath,
    position: u64,
    generation: u64,
}

impl CrashWriter {
    fn enter<'a>(&'a self, op: &str) -> FsResult<MutexGuard<'a, State>> {
        let mut state = self.shared.lock();
        if state.generation != self.generation {
            return Err(error(FsErrorKind::Io, op, &self.path));
        }
        state.enter(op, &self.path)?;
        Ok(state)
    }

    fn size(state: &State, vol: usize, id: NodeId) -> u64 {
        state.volumes[vol]
            .live
            .nodes
            .get(&id)
            .map_or(0, Image::node_size)
    }

    fn write_bytes(&mut self, buf: &[u8]) -> FsResult<usize> {
        let mut state = self.enter("write")?;
        let size = Self::size(&state, self.vol, self.id);
        let end = self.position.saturating_add(buf.len() as u64);
        state.check_space(self.vol, end.saturating_sub(size), 0, "write", &self.path)?;
        let time = state.now();
        state.stats.bytes_written += buf.len() as u64;
        state.push(
            self.vol,
            Op::Write {
                id: self.id,
                offset: self.position,
                data: buf.to_vec(),
                time,
            },
        );
        drop(state);
        self.position = end;
        Ok(buf.len())
    }
}

impl Write for CrashWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.write_bytes(buf).map_err(io_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for CrashWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let size = {
            let state = self.shared.lock();
            Self::size(&state, self.vol, self.id)
        };
        self.position = seek_position(self.position, size, pos)?;
        Ok(self.position)
    }
}

fn seek_position(current: u64, size: u64, pos: SeekFrom) -> io::Result<u64> {
    let target = match pos {
        SeekFrom::Start(offset) => Some(offset),
        SeekFrom::End(delta) => size.checked_add_signed(delta),
        SeekFrom::Current(delta) => current.checked_add_signed(delta),
    };
    target.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "seek before the start of the file",
        )
    })
}

impl WriteHandle for CrashWriter {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        let mut state = self.enter("sync")?;
        state.sync_file(self.vol, self.id, durability);
        Ok(())
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        let mut state = self.enter("set_length")?;
        let size = Self::size(&state, self.vol, self.id);
        state.check_space(
            self.vol,
            length.saturating_sub(size),
            0,
            "set_length",
            &self.path,
        )?;
        let time = state.now();
        state.push(
            self.vol,
            Op::SetLen {
                id: self.id,
                length,
                time,
            },
        );
        Ok(())
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        let mut state = self.enter("apply_meta")?;
        let current = state.volumes[self.vol]
            .live
            .nodes
            .get(&self.id)
            .map(|node| node.meta.clone())
            .ok_or_else(|| error(FsErrorKind::NotFound, "apply_meta", &self.path))?;
        let (next, report) = CrashFs::restored_meta(&state.volumes[self.vol].spec, &current, meta);
        state.push(
            self.vol,
            Op::Meta {
                id: self.id,
                meta: Box::new(next),
            },
        );
        Ok(report)
    }
}

/// Read handle over the content at the time of opening.
#[derive(Debug)]
struct CrashReader {
    data: Arc<Vec<u8>>,
    position: u64,
    meta: FileMeta,
}

impl Read for CrashReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(self.position).unwrap_or(usize::MAX);
        let available = self.data.get(start..).unwrap_or_default();
        let count = available.len().min(buf.len());
        if let (Some(target), Some(source)) = (buf.get_mut(..count), available.get(..count)) {
            target.copy_from_slice(source);
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for CrashReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.data.len() as u64, pos)?;
        Ok(self.position)
    }
}

impl ReadHandle for CrashReader {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(self.meta.clone())
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        Some(self.data.as_slice())
    }
}

/// Advisory lock of [`CrashFs`]; released on drop or by a crash.
#[derive(Debug)]
struct CrashLock {
    shared: Arc<Shared>,
    path: AbsPath,
    generation: u64,
}

impl FileLock for CrashLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

impl Drop for CrashLock {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        if state.generation == self.generation {
            state.locks.remove(self.path.as_path());
        }
    }
}
