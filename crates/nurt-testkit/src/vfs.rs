// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-memory file system with the reference semantics of test implementations.
//!
//! [`VirtualFs`] implements [`Fs`] and [`FsWrite`] as specified for test
//! implementations in the journal specification of the repository:
//!
//! * one error kind per failure condition (a missing entry or parent is
//!   [`FsErrorKind::NotFound`], a file used as a directory component is
//!   [`FsErrorKind::NotADirectory`], and so on);
//! * [`Fs::read_dir`] returns entries in byte order of their names;
//! * every content change sets the modification time to the wall time of the
//!   injected [`Clock`] and changes nothing else;
//! * file identities are assigned from 1 upwards per volume (the root
//!   directory of a volume has identity 1); a rename keeps the identity;
//! * the entity tag is `(size, modified_ns, file_id)` and
//!   [`Fs::content_hash`] is SHA-256 of the content;
//! * faults are injected per path and operation ([`Fault`]) and apply to the
//!   next matching call only, unless made persistent.
//!
//! Every successful [`FsWrite`] call is recorded with the operation of its
//! [`TxScope`] ([`VirtualFs::mutations`]), so that tests can assert that all
//! changes went through a transaction. The setup methods
//! ([`VirtualFs::write_file`], [`VirtualFs::create_dir_all`], ...) prepare
//! fixtures without a scope and are not recorded.
//!
//! Names on volumes configured as case-insensitive are compared after
//! Unicode lower-case mapping. Names on volumes with
//! [`FsFeatures::decomposes_names`] are stored and compared in Unicode
//! normalization form D, as on HFS+. Names that are not valid UTF-8 are
//! stored and compared byte for byte on every volume.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::env::Clock;
use nurt_contracts::fs::{
    AbsPath, AlternateStream, ContentHash, DirEntry, ExtendedMeta, FileId, FileKind, FileLock,
    FileMeta, Follow, Fs, FsError, FsErrorKind, FsFeatures, FsResult, FsWrite, Holder, LinkKind,
    MetaChange, MetaTransferReport, PathKey, ReadHandle, RemoveStats, RenameMode, ScopePurpose,
    SpaceInfo, TxScope, VolumeId, VolumeInfo, VolumeKind, WriteHandle, Xattr,
};
use nurt_contracts::journal::OpId;
use unicode_normalization::UnicodeNormalization;

use crate::hash::sha256;

/// Largest number of symbolic links followed while resolving one path;
/// one more yields [`FsErrorKind::Loop`].
pub const MAX_LINK_HOPS: u32 = 40;

/// Capacity reported by [`Fs::space`] for volumes without a configured
/// capacity: 1 TiB.
pub const DEFAULT_CAPACITY: u64 = 1 << 40;

/// User id of the default account.
pub const DEFAULT_UID: u32 = 1000;

/// Group id of the default account.
pub const DEFAULT_GID: u32 = 1000;

/// Permission bits of directories created by the setup methods.
pub const DEFAULT_DIR_MODE: u32 = 0o755;

/// Permission bits of files created by [`FsWrite::create_new`] and the setup
/// methods.
pub const DEFAULT_FILE_MODE: u32 = 0o644;

/// Permission bits of files created by [`FsWrite::create_temp_in`].
pub const TEMP_FILE_MODE: u32 = 0o600;

/// Permission bits of symbolic links and junctions.
pub const LINK_MODE: u32 = 0o777;

const MODE_OWNER_WRITE: u32 = 0o200;
const MODE_GROUP_WRITE: u32 = 0o020;
const MODE_OTHER_WRITE: u32 = 0o002;
const SUPERUSER_UID: u32 = 0;

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x2000;

/// Path of the virtual root volume in the native form of the host.
fn native_root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    }
}

/// Absolute path from a POSIX-style text, in the native form of the host.
///
/// On Windows the drive `C:` is prepended, so that tests written with
/// `/data/file.txt` address the same entry on every system.
///
/// Fails with [`FsErrorKind::InvalidPath`] when `text` does not start with
/// `/` or escapes the root with `..`.
///
/// ```
/// use nurt_testkit::vfs::test_path;
///
/// let path = test_path("/data/report.txt").unwrap();
/// assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("report.txt"));
/// assert!(test_path("relative/path").is_err());
/// ```
pub fn test_path(text: &str) -> FsResult<AbsPath> {
    if !text.starts_with('/') {
        return Err(FsError::new(
            FsErrorKind::InvalidPath,
            "test_path",
            Some(Path::new(text)),
        ));
    }
    if cfg!(windows) {
        AbsPath::new(format!("C:{text}"))
    } else {
        AbsPath::new(text)
    }
}

/// Settings of one virtual volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeConfig {
    /// File system type name reported by [`Fs::volume`].
    pub fs_type: String,
    /// Volume class.
    pub kind: VolumeKind,
    /// Capabilities; they decide which operations fail with
    /// [`FsErrorKind::Unsupported`] and how names are compared.
    pub features: FsFeatures,
    /// Rotational medium.
    pub rotational: bool,
    /// Read-only volume: every mutation fails with [`FsErrorKind::ReadOnly`].
    pub read_only: bool,
    /// Capacity in bytes; writes beyond it fail with
    /// [`FsErrorKind::NoSpace`]. `None` reports [`DEFAULT_CAPACITY`] and never
    /// runs out of space.
    pub capacity: Option<u64>,
    /// File system UUID.
    pub uuid: Option<String>,
}

impl VolumeConfig {
    /// Capabilities of a default volume: case-sensitive names, exchange,
    /// rename without replacement, hard links, extended attributes and
    /// durable metadata; no clones, ACLs, alternate streams or replacement
    /// with backup.
    #[must_use]
    pub fn default_features() -> FsFeatures {
        FsFeatures {
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
        }
    }
}

impl Default for VolumeConfig {
    fn default() -> Self {
        Self {
            fs_type: "vfs".to_owned(),
            kind: VolumeKind::Fixed,
            features: Self::default_features(),
            rotational: false,
            read_only: false,
            capacity: None,
            uuid: None,
        }
    }
}

/// Effect of an injected fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultEffect {
    /// The matching call fails with this kind and these holders.
    Error {
        /// Error kind.
        kind: FsErrorKind,
        /// Processes reported in [`FsError::holders`].
        holders: Vec<Holder>,
    },
    /// Handles opened by the matching `open_read` or `open_stream_read` call
    /// return no more than `max` bytes per read and expose no mapped content.
    ShortReads {
        /// Largest number of bytes per read call; at least one.
        max: usize,
    },
}

/// Fault injected into [`VirtualFs`] for one path.
///
/// A fault matches a call when the path given to the call equals the fault
/// path (lexically) and the operation name equals the fault operation (the
/// method name, for example `rename` or `open_read`), or the fault applies
/// to every operation. A fault applies to the next matching call only unless
/// made [`persistent`](Fault::persistent).
///
/// ```
/// use nurt_contracts::fs::{Follow, Fs, FsErrorKind};
/// use nurt_testkit::vfs::{Fault, VirtualFs, test_path};
///
/// let fs = VirtualFs::default();
/// let path = test_path("/data/a.txt").unwrap();
/// fs.write_file(&path, b"one").unwrap();
/// fs.inject(Fault::error(&path, "metadata", FsErrorKind::PermissionDenied));
///
/// let first = fs.metadata(&path, Follow::Follow).unwrap_err();
/// assert_eq!(first.kind, FsErrorKind::PermissionDenied);
/// assert!(fs.metadata(&path, Follow::Follow).is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// Path the fault applies to.
    pub path: AbsPath,
    /// Operation name; `None` matches every operation.
    pub op: Option<String>,
    /// Effect.
    pub effect: FaultEffect,
    /// The fault stays after it matched.
    pub persistent: bool,
}

impl Fault {
    /// Error of `kind` for the next call of `op` on `path`.
    #[must_use]
    pub fn error(path: &AbsPath, op: &str, kind: FsErrorKind) -> Self {
        Self {
            path: path.clone(),
            op: Some(op.to_owned()),
            effect: FaultEffect::Error {
                kind,
                holders: Vec::new(),
            },
            persistent: false,
        }
    }

    /// Error of `kind` for the next call of any operation on `path`.
    #[must_use]
    pub fn any_op(path: &AbsPath, kind: FsErrorKind) -> Self {
        Self {
            op: None,
            ..Self::error(path, "", kind)
        }
    }

    /// Sharing violation ([`FsErrorKind::InUse`]) for the next call of `op`
    /// on `path`, reporting `holders`.
    #[must_use]
    pub fn in_use(path: &AbsPath, op: &str, holders: Vec<Holder>) -> Self {
        Self {
            effect: FaultEffect::Error {
                kind: FsErrorKind::InUse,
                holders,
            },
            ..Self::error(path, op, FsErrorKind::InUse)
        }
    }

    /// Short reads of no more than `max` bytes (at least one) for the next file
    /// opened for reading at `path`.
    #[must_use]
    pub fn short_reads(path: &AbsPath, max: usize) -> Self {
        Self {
            path: path.clone(),
            op: Some("open_read".to_owned()),
            effect: FaultEffect::ShortReads { max: max.max(1) },
            persistent: false,
        }
    }

    /// The same fault, applying to every matching call.
    #[must_use]
    pub fn persistent(mut self) -> Self {
        self.persistent = true;
        self
    }

    fn matches(&self, op: &str, path: &Path) -> bool {
        self.path.as_path() == path && self.op.as_deref().is_none_or(|known| known == op)
    }
}

/// One successful mutation recorded by [`VirtualFs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationRecord {
    /// Name of the [`FsWrite`] method.
    pub op: String,
    /// Paths given to the call, in parameter order.
    pub paths: Vec<AbsPath>,
    /// Operation of the scope the call was made with.
    pub scope_op: OpId,
    /// Purpose of the scope.
    pub purpose: ScopePurpose,
}

#[derive(Debug, Clone)]
struct Inode {
    kind: FileKind,
    content: Vec<u8>,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
    modified_ns: i64,
    accessed_ns: i64,
    created_ns: i64,
    changed_ns: i64,
    windows_attributes: u32,
    xattrs: Vec<Xattr>,
    acl: Option<Vec<u8>>,
    flags: Option<u64>,
    link_target: Option<PathBuf>,
    streams: BTreeMap<String, Vec<u8>>,
}

impl Inode {
    fn new(kind: FileKind, mode: u32, account: Account, now: i64) -> Self {
        Self {
            kind,
            content: Vec::new(),
            mode,
            uid: account.uid,
            gid: account.gid,
            links: 1,
            modified_ns: now,
            accessed_ns: now,
            created_ns: now,
            changed_ns: now,
            windows_attributes: 0,
            xattrs: Vec::new(),
            acl: None,
            flags: None,
            link_target: None,
            streams: BTreeMap::new(),
        }
    }

    fn is_dir(&self) -> bool {
        self.kind == FileKind::Directory
    }

    fn is_link(&self) -> bool {
        matches!(self.kind, FileKind::Symlink | FileKind::Junction)
    }

    fn size(&self) -> u64 {
        match self.kind {
            FileKind::File => to_u64(self.content.len()),
            FileKind::Symlink | FileKind::Junction => self
                .link_target
                .as_ref()
                .map_or(0, |target| to_u64(target.as_os_str().len())),
            _ => 0,
        }
    }

    fn stored_bytes(&self) -> u64 {
        let streams: u64 = self.streams.values().map(|data| to_u64(data.len())).sum();
        to_u64(self.content.len()).saturating_add(streams)
    }
}

#[derive(Debug, Clone)]
struct Name {
    stored: OsString,
    inode: u64,
}

type Key = Vec<OsString>;

#[derive(Debug, Clone)]
struct VolumeSettings {
    id: VolumeId,
    fs_type: String,
    kind: VolumeKind,
    features: FsFeatures,
    rotational: bool,
    read_only: bool,
}

#[derive(Debug)]
struct Volume {
    mount: PathBuf,
    info: VolumeSettings,
    capacity: Option<u64>,
    names: BTreeMap<Key, Name>,
    inodes: BTreeMap<u64, Inode>,
    next_id: u64,
}

impl Volume {
    fn fold(&self, name: &OsStr) -> OsString {
        let features = self.info.features;
        if features.case_sensitive && !features.decomposes_names {
            return name.to_owned();
        }
        let Some(text) = name.to_str() else {
            return name.to_owned();
        };
        let text: String = if features.decomposes_names {
            text.nfd().collect()
        } else {
            text.to_owned()
        };
        if features.case_sensitive {
            OsString::from(text)
        } else {
            OsString::from(text.to_lowercase())
        }
    }

    fn stored(&self, name: &OsStr) -> OsString {
        match name.to_str() {
            Some(text) if self.info.features.decomposes_names => {
                OsString::from(text.nfd().collect::<String>())
            }
            _ => name.to_owned(),
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn lookup(&self, key: &[OsString]) -> Option<(&Name, &Inode)> {
        let name = self.names.get(key)?;
        let inode = self.inodes.get(&name.inode)?;
        Some((name, inode))
    }

    fn inode_at(&self, key: &[OsString]) -> Option<&Inode> {
        self.lookup(key).map(|(_, inode)| inode)
    }

    fn inode_mut_at(&mut self, key: &[OsString]) -> Option<&mut Inode> {
        let id = self.names.get(key)?.inode;
        self.inodes.get_mut(&id)
    }

    fn subtree(&self, key: &[OsString]) -> Vec<Key> {
        self.names
            .range(key.to_vec()..)
            .take_while(|(candidate, _)| candidate.starts_with(key))
            .map(|(candidate, _)| candidate.clone())
            .collect()
    }

    fn children(&self, key: &[OsString]) -> Vec<(OsString, FileKind)> {
        let depth = key.len() + 1;
        let mut out: Vec<(OsString, FileKind)> = self
            .names
            .range(key.to_vec()..)
            .take_while(|(candidate, _)| candidate.starts_with(key))
            .filter(|(candidate, _)| candidate.len() == depth)
            .filter_map(|(_, name)| {
                self.inodes
                    .get(&name.inode)
                    .map(|inode| (name.stored.clone(), inode.kind))
            })
            .collect();
        out.sort_by(|left, right| left.0.as_encoded_bytes().cmp(right.0.as_encoded_bytes()));
        out
    }

    fn used(&self) -> u64 {
        self.inodes.values().map(Inode::stored_bytes).sum()
    }

    fn ensure_room(&self, grow: u64, op: &str, path: &Path) -> FsResult<()> {
        match self.capacity {
            Some(capacity) if self.used().saturating_add(grow) > capacity => {
                Err(FsError::new(FsErrorKind::NoSpace, op, Some(path)))
            }
            _ => Ok(()),
        }
    }

    fn unlink(&mut self, key: &[OsString]) {
        if let Some(name) = self.names.remove(key) {
            let drop_inode = match self.inodes.get_mut(&name.inode) {
                Some(inode) => {
                    inode.links = inode.links.saturating_sub(1);
                    inode.links == 0
                }
                None => false,
            };
            if drop_inode {
                self.inodes.remove(&name.inode);
            }
        }
    }

    fn move_subtree(&mut self, from: &[OsString], to: &[OsString], stored: OsString) {
        let stored = self.stored(&stored);
        let keys = self.subtree(from);
        let mut moved = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(name) = self.names.remove(&key) {
                let mut target = to.to_vec();
                target.extend_from_slice(&key[from.len()..]);
                moved.push((target, name));
            }
        }
        for (key, mut name) in moved {
            if key.len() == to.len() {
                name.stored.clone_from(&stored);
            }
            self.names.insert(key, name);
        }
    }

    fn file_id(&self, inode: u64) -> FileId {
        FileId {
            volume: self.info.id.dev,
            low: inode,
            high: 0,
        }
    }

    fn meta(&self, id: u64, inode: &Inode) -> FileMeta {
        FileMeta {
            kind: inode.kind,
            size: inode.size(),
            mode: inode.mode,
            uid: Some(inode.uid),
            gid: Some(inode.gid),
            links: inode.links,
            modified_ns: inode.modified_ns,
            accessed_ns: Some(inode.accessed_ns),
            created_ns: Some(inode.created_ns),
            changed_ns: Some(inode.changed_ns),
            file_id: self.file_id(id),
            windows_attributes: inode.windows_attributes,
        }
    }

    fn extended(&self, id: u64, inode: &Inode) -> ExtendedMeta {
        ExtendedMeta {
            basic: self.meta(id, inode),
            xattrs: inode.xattrs.clone(),
            acl: inode.acl.clone(),
            account_sid: None,
            flags: inode.flags,
            streams: inode
                .streams
                .iter()
                .map(|(name, data)| AlternateStream {
                    name: name.clone(),
                    size: to_u64(data.len()),
                })
                .collect(),
            link_target: inode.link_target.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Account {
    uid: u32,
    gid: u32,
}

#[derive(Debug)]
struct State {
    volumes: Vec<Volume>,
    faults: Vec<Fault>,
    holders: Vec<(AbsPath, Vec<Holder>)>,
    account: Account,
    locks: Vec<AbsPath>,
    log: Vec<MutationRecord>,
    temp_counter: u64,
}

/// Location of an existing or resolved entry.
#[derive(Debug, Clone)]
struct Resolved {
    vol: usize,
    key: Key,
    real: PathBuf,
    exists: bool,
}

/// Location of a name inside an existing, writable directory; the last
/// component is not followed.
#[derive(Debug, Clone)]
struct Slot {
    vol: usize,
    key: Key,
    name: OsString,
}

enum Part {
    Root(PathBuf),
    Parent,
    Name(OsString),
}

fn parts(path: &Path) -> Vec<Part> {
    let mut root = PathBuf::new();
    let mut rest = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => root.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => rest.push(Part::Parent),
            Component::Normal(name) => rest.push(Part::Name(name.to_owned())),
        }
    }
    let mut out = Vec::with_capacity(rest.len() + 1);
    if !root.as_os_str().is_empty() {
        out.push(Part::Root(root));
    }
    out.extend(rest);
    out
}

fn to_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn fs_error(kind: FsErrorKind, op: &str, path: &Path) -> FsError {
    FsError::new(kind, op, Some(path))
}

impl State {
    fn volume_of(&self, path: &Path) -> Option<usize> {
        self.volumes
            .iter()
            .enumerate()
            .filter(|(_, volume)| path.starts_with(&volume.mount))
            .max_by_key(|(_, volume)| volume.mount.components().count())
            .map(|(index, _)| index)
    }

    fn split(&self, path: &Path) -> Option<(usize, Key)> {
        let vol = self.volume_of(path)?;
        let volume = &self.volumes[vol];
        let rest = path.strip_prefix(&volume.mount).ok()?;
        let key = rest
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(volume.fold(name)),
                _ => None,
            })
            .collect();
        Some((vol, key))
    }

    fn check_fault(&mut self, op: &str, path: &Path) -> FsResult<()> {
        let Some(index) = self.faults.iter().position(|fault| {
            fault.matches(op, path) && matches!(fault.effect, FaultEffect::Error { .. })
        }) else {
            return Ok(());
        };
        let fault = if self.faults[index].persistent {
            self.faults[index].clone()
        } else {
            self.faults.remove(index)
        };
        match fault.effect {
            FaultEffect::Error { kind, holders } => {
                let mut error = fs_error(kind, op, path);
                error.holders = holders;
                Err(error)
            }
            FaultEffect::ShortReads { .. } => Ok(()),
        }
    }

    fn take_short_reads(&mut self, op: &str, path: &Path) -> Option<usize> {
        let index = self.faults.iter().position(|fault| {
            fault.matches(op, path) && matches!(fault.effect, FaultEffect::ShortReads { .. })
        })?;
        let fault = if self.faults[index].persistent {
            self.faults[index].clone()
        } else {
            self.faults.remove(index)
        };
        match fault.effect {
            FaultEffect::ShortReads { max } => Some(max),
            FaultEffect::Error { .. } => None,
        }
    }

    fn resolve(&self, path: &Path, follow_last: bool, op: &str) -> FsResult<Resolved> {
        let not_found = || fs_error(FsErrorKind::NotFound, op, path);
        let mut queue: VecDeque<Part> = parts(path).into();
        let mut current = PathBuf::new();
        let mut hops = 0_u32;
        while let Some(part) = queue.pop_front() {
            match part {
                Part::Root(root) => current = root,
                Part::Parent => {
                    if current.parent().is_some() {
                        current.pop();
                    }
                }
                Part::Name(name) => {
                    let last = queue.is_empty();
                    let (dir_vol, dir_key) = self.split(&current).ok_or_else(not_found)?;
                    let dir = self.volumes[dir_vol]
                        .inode_at(&dir_key)
                        .ok_or_else(not_found)?;
                    if !dir.is_dir() {
                        return Err(fs_error(FsErrorKind::NotADirectory, op, path));
                    }
                    let candidate = current.join(&name);
                    let (vol, key) = self.split(&candidate).ok_or_else(not_found)?;
                    let Some((entry, inode)) = self.volumes[vol].lookup(&key) else {
                        if last {
                            return Ok(Resolved {
                                vol,
                                key,
                                real: candidate,
                                exists: false,
                            });
                        }
                        return Err(not_found());
                    };
                    if inode.is_link() && (!last || follow_last) {
                        hops += 1;
                        if hops > MAX_LINK_HOPS {
                            return Err(fs_error(FsErrorKind::Loop, op, path));
                        }
                        let target = inode.link_target.clone().unwrap_or_default();
                        for part in parts(&target).into_iter().rev() {
                            queue.push_front(part);
                        }
                    } else {
                        // The root of a mounted volume stores no name; the
                        // mount point keeps the spelling of the path.
                        if entry.stored.is_empty() {
                            current.push(&name);
                        } else {
                            current.push(&entry.stored);
                        }
                        if last {
                            return Ok(Resolved {
                                vol,
                                key,
                                real: current,
                                exists: true,
                            });
                        }
                    }
                }
            }
        }
        let (vol, key) = self.split(&current).ok_or_else(not_found)?;
        let exists = self.volumes[vol].inode_at(&key).is_some();
        Ok(Resolved {
            vol,
            key,
            real: current,
            exists,
        })
    }

    fn existing(&self, path: &Path, follow: bool, op: &str) -> FsResult<Resolved> {
        let resolved = self.resolve(path, follow, op)?;
        if resolved.exists {
            Ok(resolved)
        } else {
            Err(fs_error(FsErrorKind::NotFound, op, path))
        }
    }

    fn inode(&self, at: &Resolved) -> Option<(u64, &Inode)> {
        let volume = &self.volumes[at.vol];
        let name = volume.names.get(&at.key)?;
        volume
            .inodes
            .get(&name.inode)
            .map(|inode| (name.inode, inode))
    }

    fn can_write(&self, vol: usize, inode: &Inode) -> bool {
        if self.volumes[vol].info.read_only {
            return false;
        }
        let account = self.account;
        if account.uid == SUPERUSER_UID {
            return true;
        }
        let bit = if inode.uid == account.uid {
            MODE_OWNER_WRITE
        } else if inode.gid == account.gid {
            MODE_GROUP_WRITE
        } else {
            MODE_OTHER_WRITE
        };
        inode.mode & bit != 0
    }

    fn writable_volume(&self, vol: usize, op: &str, path: &Path) -> FsResult<()> {
        if self.volumes[vol].info.read_only {
            Err(fs_error(FsErrorKind::ReadOnly, op, path))
        } else {
            Ok(())
        }
    }

    fn slot(&self, path: &AbsPath, op: &str) -> FsResult<Slot> {
        let native = path.as_path();
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(fs_error(FsErrorKind::InUse, op, native));
        };
        let dir = self.existing(parent.as_path(), true, op)?;
        let Some((_, dir_inode)) = self.inode(&dir) else {
            return Err(fs_error(FsErrorKind::NotFound, op, native));
        };
        if !dir_inode.is_dir() {
            return Err(fs_error(FsErrorKind::NotADirectory, op, native));
        }
        let (vol, key) = self
            .split(&dir.real.join(name))
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, native))?;
        if key.is_empty() {
            return Err(fs_error(FsErrorKind::InUse, op, native));
        }
        self.writable_volume(vol, op, native)?;
        if !self.can_write(dir.vol, dir_inode) {
            return Err(fs_error(FsErrorKind::PermissionDenied, op, native));
        }
        Ok(Slot {
            vol,
            key,
            name: name.to_owned(),
        })
    }

    fn slot_inode(&self, slot: &Slot) -> Option<(u64, &Inode)> {
        let volume = &self.volumes[slot.vol];
        let name = volume.names.get(&slot.key)?;
        volume
            .inodes
            .get(&name.inode)
            .map(|inode| (name.inode, inode))
    }

    fn insert(&mut self, slot: &Slot, inode: Inode) -> u64 {
        let volume = &mut self.volumes[slot.vol];
        let id = volume.alloc_id();
        volume.inodes.insert(id, inode);
        let stored = volume.stored(&slot.name);
        volume
            .names
            .insert(slot.key.clone(), Name { stored, inode: id });
        id
    }

    fn record(&mut self, scope: &TxScope, op: &str, paths: &[&AbsPath]) {
        self.log.push(MutationRecord {
            op: op.to_owned(),
            paths: paths.iter().map(|path| (*path).clone()).collect(),
            scope_op: scope.op(),
            purpose: scope.purpose(),
        });
    }
}

struct Shared {
    state: Mutex<State>,
    clock: Arc<dyn Clock>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> i64 {
        let raw = self.clock.now().unix_nanos();
        i64::try_from(raw).unwrap_or(if raw < 0 { i64::MIN } else { i64::MAX })
    }
}

/// In-memory file system for tests.
///
/// The file system starts with one volume mounted at the root of the host
/// notation (`/`, or `C:\` on Windows) with [`VolumeConfig::default`];
/// further volumes are added with [`VirtualFs::add_volume`]. The default
/// account has user id [`DEFAULT_UID`] and group id [`DEFAULT_GID`] and owns
/// the root directory.
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::fs::{Follow, Fs, FileKind};
/// use nurt_testkit::clock::VirtualClock;
/// use nurt_testkit::vfs::{VirtualFs, test_path};
///
/// let fs = VirtualFs::new(Arc::new(VirtualClock::new()));
/// let file = test_path("/project/src/main.rs").unwrap();
/// fs.write_file(&file, b"fn main() {}\n").unwrap();
///
/// let meta = fs.metadata(&file, Follow::Follow).unwrap();
/// assert_eq!(meta.kind, FileKind::File);
/// assert_eq!(meta.size, 13);
/// let names: Vec<_> = fs
///     .read_dir(&test_path("/project").unwrap())
///     .unwrap()
///     .map(|entry| entry.unwrap().name)
///     .collect();
/// assert_eq!(names, ["src"]);
/// ```
pub struct VirtualFs {
    shared: Arc<Shared>,
}

impl VirtualFs {
    /// File system with a default root volume and the given clock.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_root(clock, VolumeConfig::default())
    }

    /// File system whose root volume uses `config`.
    #[must_use]
    pub fn with_root(clock: Arc<dyn Clock>, config: VolumeConfig) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                volumes: Vec::new(),
                faults: Vec::new(),
                holders: Vec::new(),
                account: Account {
                    uid: DEFAULT_UID,
                    gid: DEFAULT_GID,
                },
                locks: Vec::new(),
                log: Vec::new(),
                temp_counter: 0,
            }),
            clock,
        });
        let fs = Self { shared };
        let now = fs.shared.now();
        let root = native_root();
        fs.lock().volumes.push(new_volume(
            root,
            1,
            config,
            now,
            Account {
                uid: DEFAULT_UID,
                gid: DEFAULT_GID,
            },
        ));
        fs
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.lock()
    }

    /// Mounts a new, empty volume at `mount` and returns its identity.
    ///
    /// Missing directories up to the mount point are created on the
    /// enclosing volume, so that listings of the parent show the mount point.
    /// Volume device numbers are assigned from 1 upwards.
    ///
    /// Fails with [`FsErrorKind::Exists`] when a volume is already mounted at
    /// `mount`, and as [`VirtualFs::create_dir_all`] for the mount point.
    pub fn add_volume(&self, mount: &AbsPath, config: VolumeConfig) -> FsResult<VolumeId> {
        if self
            .lock()
            .volumes
            .iter()
            .any(|volume| volume.mount == mount.as_path())
        {
            return Err(fs_error(FsErrorKind::Exists, "add_volume", mount.as_path()));
        }
        self.create_dir_all(mount)?;
        let now = self.shared.now();
        let mut state = self.lock();
        let dev = to_u64(state.volumes.len()).saturating_add(1);
        let account = state.account;
        let volume = new_volume(mount.as_path().to_path_buf(), dev, config, now, account);
        let id = volume.info.id.clone();
        state.volumes.push(volume);
        Ok(id)
    }

    /// Root of the default volume.
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] only when the native root is
    /// not absolute, which does not happen on supported systems.
    pub fn root(&self) -> FsResult<AbsPath> {
        AbsPath::new(native_root())
    }

    /// Sets the account used for new entries and for
    /// [`Fs::can_write_dir`]; user id 0 may write everywhere except on
    /// read-only volumes.
    pub fn set_account(&self, uid: u32, gid: u32) {
        self.lock().account = Account { uid, gid };
    }

    /// Injects a fault.
    pub fn inject(&self, fault: Fault) {
        self.lock().faults.push(fault);
    }

    /// Removes every injected fault.
    pub fn clear_faults(&self) {
        self.lock().faults.clear();
    }

    /// Sets the processes reported by [`Fs::holders`] for `path`.
    pub fn set_holders(&self, path: &AbsPath, holders: Vec<Holder>) {
        let mut state = self.lock();
        state.holders.retain(|(known, _)| known != path);
        state.holders.push((path.clone(), holders));
    }

    /// Successful [`FsWrite`] calls, in call order.
    #[must_use]
    pub fn mutations(&self) -> Vec<MutationRecord> {
        self.lock().log.clone()
    }

    /// Creates a directory and its missing parents with
    /// [`DEFAULT_DIR_MODE`], without a scope and without recording.
    ///
    /// Fails with [`FsErrorKind::NotADirectory`] when a component is not a
    /// directory, and with [`FsErrorKind::Loop`] for a cycle of links.
    pub fn create_dir_all(&self, path: &AbsPath) -> FsResult<()> {
        let now = self.shared.now();
        let mut state = self.lock();
        let mut chain = Vec::new();
        let mut cursor = Some(path.clone());
        while let Some(current) = cursor {
            let resolved = match state.resolve(current.as_path(), true, "create_dir_all") {
                Ok(resolved) => Some(resolved),
                Err(error) if error.kind == FsErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if let Some(resolved) = resolved.filter(|resolved| resolved.exists) {
                let is_dir = state
                    .inode(&resolved)
                    .is_some_and(|(_, inode)| inode.is_dir());
                if !is_dir {
                    return Err(fs_error(
                        FsErrorKind::NotADirectory,
                        "create_dir_all",
                        path.as_path(),
                    ));
                }
                break;
            }
            cursor = current.parent();
            chain.push(current);
        }
        let account = state.account;
        for dir in chain.into_iter().rev() {
            let resolved = state.resolve(dir.as_path(), true, "create_dir_all")?;
            let name = dir.file_name().map(OsStr::to_owned).unwrap_or_default();
            let slot = Slot {
                vol: resolved.vol,
                key: resolved.key,
                name,
            };
            state.insert(
                &slot,
                Inode::new(FileKind::Directory, DEFAULT_DIR_MODE, account, now),
            );
        }
        Ok(())
    }

    /// Writes a file with missing parents, without a scope and without
    /// recording. An existing file keeps its identity and metadata; its
    /// content is replaced and its modification time set to the current
    /// wall time.
    ///
    /// Fails with [`FsErrorKind::IsADirectory`] when `path` is a directory,
    /// and as [`VirtualFs::create_dir_all`] for the parents.
    pub fn write_file(&self, path: &AbsPath, content: &[u8]) -> FsResult<()> {
        if let Some(parent) = path.parent() {
            self.create_dir_all(&parent)?;
        }
        let now = self.shared.now();
        let mut state = self.lock();
        let resolved = state.resolve(path.as_path(), true, "write_file")?;
        let account = state.account;
        let volume = &mut state.volumes[resolved.vol];
        if let Some(inode) = volume.inode_mut_at(&resolved.key) {
            if inode.kind != FileKind::File {
                return Err(fs_error(
                    FsErrorKind::IsADirectory,
                    "write_file",
                    path.as_path(),
                ));
            }
            inode.content = content.to_vec();
            inode.modified_ns = now;
            return Ok(());
        }
        let mut inode = Inode::new(FileKind::File, DEFAULT_FILE_MODE, account, now);
        inode.content = content.to_vec();
        let name = resolved
            .real
            .file_name()
            .map(OsStr::to_owned)
            .unwrap_or_default();
        state.insert(
            &Slot {
                vol: resolved.vol,
                key: resolved.key,
                name,
            },
            inode,
        );
        Ok(())
    }

    /// Creates a symbolic link with missing parents, without a scope and
    /// without recording.
    ///
    /// Fails with [`FsErrorKind::Exists`] when `link` exists, and as
    /// [`VirtualFs::create_dir_all`] for the parents.
    pub fn symlink(&self, target: impl Into<PathBuf>, link: &AbsPath) -> FsResult<()> {
        if let Some(parent) = link.parent() {
            self.create_dir_all(&parent)?;
        }
        let now = self.shared.now();
        let mut state = self.lock();
        let resolved = state.resolve(link.as_path(), false, "symlink")?;
        if resolved.exists {
            return Err(fs_error(FsErrorKind::Exists, "symlink", link.as_path()));
        }
        let mut inode = Inode::new(FileKind::Symlink, LINK_MODE, state.account, now);
        inode.link_target = Some(target.into());
        let name = link.file_name().map(OsStr::to_owned).unwrap_or_default();
        state.insert(
            &Slot {
                vol: resolved.vol,
                key: resolved.key,
                name,
            },
            inode,
        );
        Ok(())
    }

    /// Content of a file, following links.
    ///
    /// Fails with [`FsErrorKind::NotFound`] and with
    /// [`FsErrorKind::IsADirectory`] for a directory.
    pub fn read_file(&self, path: &AbsPath) -> FsResult<Vec<u8>> {
        let state = self.lock();
        let resolved = state.existing(path.as_path(), true, "read_file")?;
        match state.inode(&resolved) {
            Some((_, inode)) if inode.kind == FileKind::File => Ok(inode.content.clone()),
            Some(_) => Err(fs_error(
                FsErrorKind::IsADirectory,
                "read_file",
                path.as_path(),
            )),
            None => Err(fs_error(FsErrorKind::NotFound, "read_file", path.as_path())),
        }
    }

    /// Whether an entry exists at `path`, without following a link in the
    /// last component.
    #[must_use]
    pub fn exists(&self, path: &AbsPath) -> bool {
        self.lock()
            .resolve(path.as_path(), false, "exists")
            .is_ok_and(|resolved| resolved.exists)
    }

    /// Sets the permission bits of an entry, without a scope and without
    /// recording.
    ///
    /// Fails with [`FsErrorKind::NotFound`].
    pub fn set_mode(&self, path: &AbsPath, mode: u32) -> FsResult<()> {
        self.setup_inode(path, "set_mode", |inode| inode.mode = mode)
    }

    /// Sets the user and group ids of an entry, without a scope and without
    /// recording.
    ///
    /// Fails with [`FsErrorKind::NotFound`].
    pub fn set_ids(&self, path: &AbsPath, uid: u32, gid: u32) -> FsResult<()> {
        self.setup_inode(path, "set_ids", |inode| {
            inode.uid = uid;
            inode.gid = gid;
        })
    }

    /// Sets the modification time of an entry in nanoseconds since the Unix
    /// epoch, without a scope and without recording.
    ///
    /// Fails with [`FsErrorKind::NotFound`].
    pub fn set_modified(&self, path: &AbsPath, modified_ns: i64) -> FsResult<()> {
        self.setup_inode(path, "set_modified", |inode| {
            inode.modified_ns = modified_ns;
        })
    }

    /// Removes an entry, a directory with everything below it, without a
    /// scope and without recording; a link in the last component is removed
    /// itself.
    ///
    /// Fails with [`FsErrorKind::NotFound`], and with [`FsErrorKind::InUse`]
    /// for the root of a volume.
    pub fn remove_entry(&self, path: &AbsPath) -> FsResult<()> {
        let mut state = self.lock();
        let resolved = state.existing(path.as_path(), false, "remove_entry")?;
        if resolved.key.is_empty() {
            return Err(fs_error(FsErrorKind::InUse, "remove_entry", path.as_path()));
        }
        let volume = &mut state.volumes[resolved.vol];
        for key in volume.subtree(&resolved.key).into_iter().rev() {
            volume.unlink(&key);
        }
        Ok(())
    }

    /// Creates a special file (a device, socket or named pipe; the kind is
    /// not distinguished) with missing parents, without a scope and without
    /// recording.
    ///
    /// Fails with [`FsErrorKind::Exists`] when `path` exists, and as
    /// [`VirtualFs::create_dir_all`] for the parents.
    pub fn create_special(&self, path: &AbsPath) -> FsResult<()> {
        if let Some(parent) = path.parent() {
            self.create_dir_all(&parent)?;
        }
        let now = self.shared.now();
        let mut state = self.lock();
        let resolved = state.resolve(path.as_path(), false, "create_special")?;
        if resolved.exists {
            return Err(fs_error(
                FsErrorKind::Exists,
                "create_special",
                path.as_path(),
            ));
        }
        let inode = Inode::new(FileKind::Special, DEFAULT_FILE_MODE, state.account, now);
        let name = path.file_name().map(OsStr::to_owned).unwrap_or_default();
        state.insert(
            &Slot {
                vol: resolved.vol,
                key: resolved.key,
                name,
            },
            inode,
        );
        Ok(())
    }

    fn setup_inode(
        &self,
        path: &AbsPath,
        op: &str,
        change: impl FnOnce(&mut Inode),
    ) -> FsResult<()> {
        let mut state = self.lock();
        let resolved = state.existing(path.as_path(), false, op)?;
        let inode = state.volumes[resolved.vol]
            .inode_mut_at(&resolved.key)
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, path.as_path()))?;
        change(inode);
        Ok(())
    }

    fn open_reader(
        &self,
        path: &AbsPath,
        stream: Option<&str>,
        op: &str,
    ) -> FsResult<Box<dyn ReadHandle>> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault(op, native)?;
        let max_chunk = state.take_short_reads("open_read", native);
        let resolved = state.existing(native, true, op)?;
        let volume = &state.volumes[resolved.vol];
        if stream.is_some() && !volume.info.features.alternate_streams {
            return Err(fs_error(FsErrorKind::Unsupported, op, native));
        }
        let Some((id, inode)) = state.inode(&resolved) else {
            return Err(fs_error(FsErrorKind::NotFound, op, native));
        };
        if inode.is_dir() {
            return Err(fs_error(FsErrorKind::IsADirectory, op, native));
        }
        let data = match stream {
            Some(name) => inode
                .streams
                .get(name)
                .cloned()
                .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, native))?,
            None => inode.content.clone(),
        };
        Ok(Box::new(VfsReader {
            data,
            position: 0,
            meta: volume.meta(id, inode),
            max_chunk,
        }))
    }

    fn open_writer(
        &self,
        scope: &TxScope,
        op: &str,
        path: &AbsPath,
        stream: Option<String>,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault(op, native)?;
        let resolved = state.existing(native, true, op)?;
        state.writable_volume(resolved.vol, op, native)?;
        if stream.is_some() && !state.volumes[resolved.vol].info.features.alternate_streams {
            return Err(fs_error(FsErrorKind::Unsupported, op, native));
        }
        let Some((id, inode)) = state.inode(&resolved) else {
            return Err(fs_error(FsErrorKind::NotFound, op, native));
        };
        if inode.is_dir() {
            return Err(fs_error(FsErrorKind::IsADirectory, op, native));
        }
        if !state.can_write(resolved.vol, inode) {
            return Err(fs_error(FsErrorKind::PermissionDenied, op, native));
        }
        if let Some(name) = &stream
            && let Some(inode) = state.volumes[resolved.vol].inodes.get_mut(&id)
        {
            inode.streams.entry(name.clone()).or_default();
        }
        state.record(scope, op, &[path]);
        Ok(Box::new(VfsWriter {
            shared: Arc::clone(&self.shared),
            vol: resolved.vol,
            inode: id,
            stream,
            path: path.clone(),
            position: 0,
        }))
    }

    fn create_file(
        &self,
        scope: &TxScope,
        op: &str,
        path: &AbsPath,
        mode: u32,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, path.as_path())?;
        let slot = state.slot(path, op)?;
        if state.slot_inode(&slot).is_some() {
            return Err(fs_error(FsErrorKind::Exists, op, path.as_path()));
        }
        let inode = Inode::new(FileKind::File, mode, state.account, now);
        let id = state.insert(&slot, inode);
        state.record(scope, op, &[path]);
        Ok(Box::new(VfsWriter {
            shared: Arc::clone(&self.shared),
            vol: slot.vol,
            inode: id,
            stream: None,
            path: path.clone(),
            position: 0,
        }))
    }

    fn change_inode(
        &self,
        scope: &TxScope,
        op: &str,
        path: &AbsPath,
        follow: bool,
        change: impl FnOnce(&mut Volume, u64, i64) -> FsResult<()>,
    ) -> FsResult<()> {
        let native = path.as_path();
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, native)?;
        let resolved = state.existing(native, follow, op)?;
        state.writable_volume(resolved.vol, op, native)?;
        let id = state
            .inode(&resolved)
            .map(|(id, _)| id)
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, native))?;
        change(&mut state.volumes[resolved.vol], id, now)?;
        state.record(scope, op, &[path]);
        Ok(())
    }
}

fn new_volume(
    mount: PathBuf,
    dev: u64,
    config: VolumeConfig,
    now: i64,
    account: Account,
) -> Volume {
    let info = VolumeSettings {
        id: VolumeId {
            dev,
            uuid: config.uuid,
            subvolume: 0,
        },
        fs_type: config.fs_type,
        kind: config.kind,
        features: config.features,
        rotational: config.rotational,
        read_only: config.read_only,
    };
    let mut volume = Volume {
        mount,
        info,
        capacity: config.capacity,
        names: BTreeMap::new(),
        inodes: BTreeMap::new(),
        next_id: 1,
    };
    let id = volume.alloc_id();
    volume.inodes.insert(
        id,
        Inode::new(FileKind::Directory, DEFAULT_DIR_MODE, account, now),
    );
    volume.names.insert(
        Vec::new(),
        Name {
            stored: OsString::new(),
            inode: id,
        },
    );
    volume
}

impl Default for VirtualFs {
    /// File system with a default root volume and a new
    /// [`VirtualClock`](crate::clock::VirtualClock).
    fn default() -> Self {
        Self::new(Arc::new(crate::clock::VirtualClock::new()))
    }
}

impl fmt::Debug for VirtualFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        f.debug_struct("VirtualFs")
            .field("volumes", &state.volumes.len())
            .field("faults", &state.faults.len())
            .field("mutations", &state.log.len())
            .finish_non_exhaustive()
    }
}

impl Fs for VirtualFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("metadata", native)?;
        let resolved = state.existing(native, follow == Follow::Follow, "metadata")?;
        let (id, inode) = state
            .inode(&resolved)
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, "metadata", native))?;
        Ok(state.volumes[resolved.vol].meta(id, inode))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("extended_metadata", native)?;
        let resolved = state.existing(native, follow == Follow::Follow, "extended_metadata")?;
        let (id, inode) = state
            .inode(&resolved)
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, "extended_metadata", native))?;
        Ok(state.volumes[resolved.vol].extended(id, inode))
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        self.open_reader(path, None, "open_read")
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        self.open_reader(path, Some(stream), "open_stream_read")
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("read_dir", native)?;
        let resolved = state.existing(native, true, "read_dir")?;
        let is_dir = state
            .inode(&resolved)
            .is_some_and(|(_, inode)| inode.is_dir());
        if !is_dir {
            return Err(fs_error(FsErrorKind::NotADirectory, "read_dir", native));
        }
        let entries: Vec<FsResult<DirEntry>> = state.volumes[resolved.vol]
            .children(&resolved.key)
            .into_iter()
            .map(|(name, kind)| {
                Ok(DirEntry {
                    name,
                    kind: Some(kind),
                })
            })
            .collect();
        Ok(Box::new(entries.into_iter()))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("read_link", native)?;
        let resolved = state.existing(native, false, "read_link")?;
        match state.inode(&resolved) {
            Some((_, inode)) if inode.is_link() => {
                Ok(inode.link_target.clone().unwrap_or_default())
            }
            Some(_) => Err(fs_error(FsErrorKind::InvalidPath, "read_link", native)),
            None => Err(fs_error(FsErrorKind::NotFound, "read_link", native)),
        }
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("real_path", native)?;
        let resolved = state.existing(native, true, "real_path")?;
        AbsPath::new(resolved.real)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("content_hash", native)?;
        let resolved = state.existing(native, true, "content_hash")?;
        match state.inode(&resolved) {
            Some((_, inode)) if inode.is_dir() => {
                Err(fs_error(FsErrorKind::IsADirectory, "content_hash", native))
            }
            Some((_, inode)) => Ok(sha256(&inode.content)),
            None => Err(fs_error(FsErrorKind::NotFound, "content_hash", native)),
        }
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("volume", native)?;
        let vol = state
            .volume_of(native)
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, "volume", native))?;
        let volume = &state.volumes[vol];
        let settings = volume.info.clone();
        Ok(VolumeInfo {
            id: settings.id,
            mount: AbsPath::new(volume.mount.clone())?,
            fs_type: settings.fs_type,
            kind: settings.kind,
            features: settings.features,
            rotational: settings.rotational,
            read_only: settings.read_only,
        })
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        let state = self.lock();
        let found = state
            .volumes
            .iter()
            .find(|candidate| &candidate.info.id == volume)
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "space", None))?;
        let total = found.capacity.unwrap_or(DEFAULT_CAPACITY);
        Ok(SpaceInfo {
            total,
            available: total.saturating_sub(found.used()),
            quota_remaining: None,
            quota_limit: None,
            purgeable_system: None,
        })
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("path_key", native)?;
        let (vol, key) = state
            .split(native)
            .ok_or_else(|| fs_error(FsErrorKind::NotFound, "path_key", native))?;
        let mut bytes = state.volumes[vol].info.id.dev.to_be_bytes().to_vec();
        for component in key {
            bytes.push(b'/');
            bytes.extend_from_slice(component.as_encoded_bytes());
        }
        Ok(PathKey(bytes))
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        let native = path.as_path();
        let mut state = self.lock();
        state.check_fault("holders", native)?;
        state.existing(native, false, "holders")?;
        Ok(state
            .holders
            .iter()
            .find(|(known, _)| known == path)
            .map(|(_, holders)| holders.clone())
            .unwrap_or_default())
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        let native = dir.as_path();
        let mut state = self.lock();
        state.check_fault("can_write_dir", native)?;
        let resolved = state.existing(native, true, "can_write_dir")?;
        match state.inode(&resolved) {
            Some((_, inode)) if inode.is_dir() => Ok(state.can_write(resolved.vol, inode)),
            Some(_) => Err(fs_error(
                FsErrorKind::NotADirectory,
                "can_write_dir",
                native,
            )),
            None => Err(fs_error(FsErrorKind::NotFound, "can_write_dir", native)),
        }
    }
}

impl FsWrite for VirtualFs {
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let path = {
            let mut state = self.lock();
            state.check_fault("create_temp_in", dir.as_path())?;
            loop {
                state.temp_counter = state.temp_counter.saturating_add(1);
                let candidate =
                    dir.join_name(&format!(".{name_hint}.{}.tmp", state.temp_counter))?;
                let taken = state
                    .resolve(candidate.as_path(), false, "create_temp_in")
                    .is_ok_and(|resolved| resolved.exists);
                if !taken {
                    break candidate;
                }
            }
        };
        self.create_file(scope, "create_temp_in", &path, TEMP_FILE_MODE)
    }

    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.create_file(scope, "create_new", path, DEFAULT_FILE_MODE)
    }

    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.open_writer(scope, "open_in_place", path, None)
    }

    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.open_writer(scope, "open_stream_write", path, Some(stream.to_owned()))
    }

    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        let op = "rename";
        let mut state = self.lock();
        state.check_fault(op, from.as_path())?;
        state.check_fault(op, to.as_path())?;
        let source = state.slot(from, op)?;
        let dest = state.slot(to, op)?;
        let Some((_, source_inode)) = state.slot_inode(&source) else {
            return Err(fs_error(FsErrorKind::NotFound, op, from.as_path()));
        };
        let source_is_dir = source_inode.is_dir();
        if source.vol != dest.vol {
            return Err(fs_error(FsErrorKind::CrossDevice, op, to.as_path()));
        }
        if source.key == dest.key {
            if source.name != dest.name {
                state.volumes[source.vol].move_subtree(&source.key, &dest.key, dest.name.clone());
            }
            state.record(scope, op, &[from, to]);
            return Ok(());
        }
        if dest.key.starts_with(&source.key) {
            return Err(fs_error(FsErrorKind::InvalidPath, op, to.as_path()));
        }
        if let Some((_, existing)) = state.slot_inode(&dest) {
            if mode == RenameMode::NoReplace {
                return Err(fs_error(FsErrorKind::Exists, op, to.as_path()));
            }
            match (source_is_dir, existing.is_dir()) {
                (true, false) => {
                    return Err(fs_error(FsErrorKind::NotADirectory, op, to.as_path()));
                }
                (false, true) => return Err(fs_error(FsErrorKind::IsADirectory, op, to.as_path())),
                (true, true) if !state.volumes[dest.vol].children(&dest.key).is_empty() => {
                    return Err(fs_error(FsErrorKind::NotEmpty, op, to.as_path()));
                }
                _ => {}
            }
            state.volumes[dest.vol].unlink(&dest.key);
        }
        state.volumes[source.vol].move_subtree(&source.key, &dest.key, dest.name.clone());
        state.record(scope, op, &[from, to]);
        Ok(())
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        let op = "exchange";
        let mut state = self.lock();
        state.check_fault(op, a.as_path())?;
        state.check_fault(op, b.as_path())?;
        let first = state.slot(a, op)?;
        let second = state.slot(b, op)?;
        if !state.volumes[first.vol].info.features.exchange {
            return Err(fs_error(FsErrorKind::Unsupported, op, a.as_path()));
        }
        if state.slot_inode(&first).is_none() {
            return Err(fs_error(FsErrorKind::NotFound, op, a.as_path()));
        }
        if state.slot_inode(&second).is_none() {
            return Err(fs_error(FsErrorKind::NotFound, op, b.as_path()));
        }
        if first.vol != second.vol {
            return Err(fs_error(FsErrorKind::CrossDevice, op, b.as_path()));
        }
        if first.key.starts_with(&second.key) || second.key.starts_with(&first.key) {
            return Err(fs_error(FsErrorKind::InvalidPath, op, b.as_path()));
        }
        let volume = &mut state.volumes[first.vol];
        // NUL never occurs in a file name, so the parking key cannot collide
        // with an entry.
        let parked: Key = vec![OsString::from("\u{0}exchange")];
        volume.move_subtree(&first.key, &parked, first.name.clone());
        volume.move_subtree(&second.key, &first.key, first.name.clone());
        volume.move_subtree(&parked, &second.key, second.name.clone());
        state.record(scope, op, &[a, b]);
        Ok(())
    }

    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        let op = "replace_with_backup";
        let mut state = self.lock();
        for path in [target, replacement, backup] {
            state.check_fault(op, path.as_path())?;
        }
        let target_slot = state.slot(target, op)?;
        let replacement_slot = state.slot(replacement, op)?;
        let backup_slot = state.slot(backup, op)?;
        if !state.volumes[target_slot.vol]
            .info
            .features
            .replace_with_backup
        {
            return Err(fs_error(FsErrorKind::Unsupported, op, target.as_path()));
        }
        for (slot, path) in [(&target_slot, target), (&replacement_slot, replacement)] {
            match state.slot_inode(slot) {
                None => return Err(fs_error(FsErrorKind::NotFound, op, path.as_path())),
                Some((_, inode)) if inode.is_dir() => {
                    return Err(fs_error(FsErrorKind::IsADirectory, op, path.as_path()));
                }
                Some(_) => {}
            }
        }
        if replacement_slot.vol != target_slot.vol || backup_slot.vol != target_slot.vol {
            return Err(fs_error(FsErrorKind::CrossDevice, op, target.as_path()));
        }
        let volume = &mut state.volumes[target_slot.vol];
        volume.unlink(&backup_slot.key);
        volume.move_subtree(&target_slot.key, &backup_slot.key, backup_slot.name.clone());
        volume.move_subtree(
            &replacement_slot.key,
            &target_slot.key,
            target_slot.name.clone(),
        );
        state.record(scope, op, &[target, replacement, backup]);
        Ok(())
    }

    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        let op = "hard_link";
        let mut state = self.lock();
        state.check_fault(op, existing.as_path())?;
        state.check_fault(op, new.as_path())?;
        let source = state.existing(existing.as_path(), false, op)?;
        let dest = state.slot(new, op)?;
        if !state.volumes[source.vol].info.features.hardlink {
            return Err(fs_error(FsErrorKind::Unsupported, op, existing.as_path()));
        }
        let Some((id, inode)) = state.inode(&source) else {
            return Err(fs_error(FsErrorKind::NotFound, op, existing.as_path()));
        };
        if inode.is_dir() {
            return Err(fs_error(
                FsErrorKind::PermissionDenied,
                op,
                existing.as_path(),
            ));
        }
        if state.slot_inode(&dest).is_some() {
            return Err(fs_error(FsErrorKind::Exists, op, new.as_path()));
        }
        if source.vol != dest.vol {
            return Err(fs_error(FsErrorKind::CrossDevice, op, new.as_path()));
        }
        let volume = &mut state.volumes[dest.vol];
        if let Some(inode) = volume.inodes.get_mut(&id) {
            inode.links = inode.links.saturating_add(1);
        }
        volume.names.insert(
            dest.key.clone(),
            Name {
                stored: dest.name.clone(),
                inode: id,
            },
        );
        state.record(scope, op, &[existing, new]);
        Ok(())
    }

    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        let op = "clone_file";
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, src.as_path())?;
        state.check_fault(op, dst.as_path())?;
        let source = state.existing(src.as_path(), true, op)?;
        let dest = state.slot(dst, op)?;
        if !state.volumes[source.vol].info.features.reflink {
            return Err(fs_error(FsErrorKind::Unsupported, op, src.as_path()));
        }
        let copy = copy_source(&state, &source, op, src)?;
        if state.slot_inode(&dest).is_some() {
            return Err(fs_error(FsErrorKind::Exists, op, dst.as_path()));
        }
        if source.vol != dest.vol {
            return Err(fs_error(FsErrorKind::CrossDevice, op, dst.as_path()));
        }
        let mut inode = Inode::new(FileKind::File, copy.mode, state.account, now);
        inode.content = copy.content;
        inode.xattrs = copy.xattrs;
        state.insert(&dest, inode);
        state.record(scope, op, &[src, dst]);
        Ok(())
    }

    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        let op = "copy_file";
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, src.as_path())?;
        state.check_fault(op, dst.as_path())?;
        let source = state.existing(src.as_path(), true, op)?;
        let dest = state.slot(dst, op)?;
        let copy = copy_source(&state, &source, op, src)?;
        if state.slot_inode(&dest).is_some() {
            return Err(fs_error(FsErrorKind::Exists, op, dst.as_path()));
        }
        let streams = if state.volumes[dest.vol].info.features.alternate_streams {
            copy.streams
        } else {
            BTreeMap::new()
        };
        let mut inode = Inode::new(FileKind::File, copy.mode, state.account, now);
        inode.content = copy.content;
        inode.streams = streams;
        let size = inode.stored_bytes();
        state.volumes[dest.vol].ensure_room(size, op, dst.as_path())?;
        let copied = to_u64(inode.content.len());
        state.insert(&dest, inode);
        state.record(scope, op, &[src, dst]);
        Ok(copied)
    }

    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let op = "remove_file";
        let mut state = self.lock();
        state.check_fault(op, path.as_path())?;
        let slot = state.slot(path, op)?;
        match state.slot_inode(&slot) {
            None => return Err(fs_error(FsErrorKind::NotFound, op, path.as_path())),
            Some((_, inode)) if inode.is_dir() => {
                return Err(fs_error(FsErrorKind::IsADirectory, op, path.as_path()));
            }
            Some(_) => {}
        }
        state.volumes[slot.vol].unlink(&slot.key);
        state.record(scope, op, &[path]);
        Ok(())
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let op = "remove_dir";
        let mut state = self.lock();
        state.check_fault(op, path.as_path())?;
        let slot = state.slot(path, op)?;
        match state.slot_inode(&slot) {
            None => return Err(fs_error(FsErrorKind::NotFound, op, path.as_path())),
            Some((_, inode)) if !inode.is_dir() => {
                return Err(fs_error(FsErrorKind::NotADirectory, op, path.as_path()));
            }
            Some(_) => {}
        }
        if !state.volumes[slot.vol].children(&slot.key).is_empty() {
            return Err(fs_error(FsErrorKind::NotEmpty, op, path.as_path()));
        }
        state.volumes[slot.vol].unlink(&slot.key);
        state.record(scope, op, &[path]);
        Ok(())
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        _parallelism: usize,
    ) -> FsResult<RemoveStats> {
        let op = "remove_tree";
        let mut state = self.lock();
        state.check_fault(op, path.as_path())?;
        let slot = state.slot(path, op)?;
        if state.slot_inode(&slot).is_none() {
            return Err(fs_error(FsErrorKind::NotFound, op, path.as_path()));
        }
        let volume = &mut state.volumes[slot.vol];
        let mut stats = RemoveStats::default();
        let keys = volume.subtree(&slot.key);
        for key in keys.iter().rev() {
            if let Some(inode) = volume.inode_at(key) {
                if inode.is_dir() {
                    stats.directories += 1;
                } else {
                    stats.files += 1;
                    stats.bytes = stats.bytes.saturating_add(inode.size());
                }
            }
            volume.unlink(key);
        }
        state.record(scope, op, &[path]);
        Ok(stats)
    }

    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        let op = "create_dir";
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, path.as_path())?;
        let slot = state.slot(path, op)?;
        if state.slot_inode(&slot).is_some() {
            return Err(fs_error(FsErrorKind::Exists, op, path.as_path()));
        }
        let inode = Inode::new(FileKind::Directory, mode, state.account, now);
        state.insert(&slot, inode);
        state.record(scope, op, &[path]);
        Ok(())
    }

    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        let op = "create_link";
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, link.as_path())?;
        let slot = state.slot(link, op)?;
        if state.slot_inode(&slot).is_some() {
            return Err(fs_error(FsErrorKind::Exists, op, link.as_path()));
        }
        let file_kind = match kind {
            LinkKind::Junction => FileKind::Junction,
            LinkKind::SymlinkFile | LinkKind::SymlinkDirectory => FileKind::Symlink,
        };
        let mut inode = Inode::new(file_kind, LINK_MODE, state.account, now);
        inode.link_target = Some(target.to_path_buf());
        state.insert(&slot, inode);
        state.record(scope, op, &[link]);
        Ok(())
    }

    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        let op = "set_meta";
        let native = path.as_path().to_path_buf();
        self.change_inode(scope, op, path, true, |volume, id, _now| {
            let features = volume.info.features;
            if !features.xattr
                && (!change.set_xattrs.is_empty() || !change.remove_xattrs.is_empty())
            {
                return Err(fs_error(FsErrorKind::Unsupported, op, &native));
            }
            if !features.acl && change.acl.is_some() {
                return Err(fs_error(FsErrorKind::Unsupported, op, &native));
            }
            let inode = volume
                .inodes
                .get_mut(&id)
                .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, &native))?;
            apply_change(inode, change);
            Ok(())
        })
    }

    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        let op = "restore_meta";
        let native = path.as_path().to_path_buf();
        let mut report = MetaTransferReport::default();
        self.change_inode(scope, op, path, false, |volume, id, _now| {
            let features = volume.info.features;
            let inode = volume
                .inodes
                .get_mut(&id)
                .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, &native))?;
            report = restore_into(inode, meta, features);
            Ok(())
        })?;
        Ok(report)
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        let op = "truncate";
        let native = path.as_path().to_path_buf();
        self.change_inode(scope, op, path, true, |volume, id, now| {
            let current = volume
                .inodes
                .get(&id)
                .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, &native))?;
            if current.is_dir() {
                return Err(fs_error(FsErrorKind::IsADirectory, op, &native));
            }
            let grow = length.saturating_sub(to_u64(current.content.len()));
            volume.ensure_room(grow, op, &native)?;
            let new_size =
                usize::try_from(length).map_err(|_| fs_error(FsErrorKind::NoSpace, op, &native))?;
            if let Some(inode) = volume.inodes.get_mut(&id) {
                inode.content.resize(new_size, 0);
                inode.modified_ns = now;
            }
            Ok(())
        })
    }

    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        let native = dir.as_path();
        let mut state = self.lock();
        state.check_fault("sync_dir", native)?;
        let resolved = state.existing(native, true, "sync_dir")?;
        match state.inode(&resolved) {
            Some((_, inode)) if inode.is_dir() => Ok(()),
            _ => Err(fs_error(FsErrorKind::NotADirectory, "sync_dir", native)),
        }
    }

    fn lock_exclusive(&self, path: &AbsPath, _timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        let op = "lock_exclusive";
        let now = self.shared.now();
        let mut state = self.lock();
        state.check_fault(op, path.as_path())?;
        if state.locks.contains(path) {
            return Err(fs_error(FsErrorKind::InUse, op, path.as_path()));
        }
        let slot = state.slot(path, op)?;
        match state.slot_inode(&slot) {
            Some((_, inode)) if inode.is_dir() => {
                return Err(fs_error(FsErrorKind::IsADirectory, op, path.as_path()));
            }
            Some(_) => {}
            None => {
                let inode = Inode::new(FileKind::File, DEFAULT_FILE_MODE, state.account, now);
                state.insert(&slot, inode);
            }
        }
        state.locks.push(path.clone());
        Ok(Box::new(VfsLock {
            shared: Arc::clone(&self.shared),
            path: path.clone(),
        }))
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let op = "mark_hidden";
        let native = path.as_path().to_path_buf();
        self.change_inode(scope, op, path, false, |volume, id, _now| {
            let inode = volume
                .inodes
                .get_mut(&id)
                .ok_or_else(|| fs_error(FsErrorKind::NotFound, op, &native))?;
            inode.windows_attributes |=
                FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;
            Ok(())
        })
    }
}

struct CopySource {
    content: Vec<u8>,
    mode: u32,
    xattrs: Vec<Xattr>,
    streams: BTreeMap<String, Vec<u8>>,
}

fn copy_source(state: &State, source: &Resolved, op: &str, path: &AbsPath) -> FsResult<CopySource> {
    match state.inode(source) {
        None => Err(fs_error(FsErrorKind::NotFound, op, path.as_path())),
        Some((_, inode)) if inode.kind != FileKind::File => {
            Err(fs_error(FsErrorKind::IsADirectory, op, path.as_path()))
        }
        Some((_, inode)) => Ok(CopySource {
            content: inode.content.clone(),
            mode: inode.mode,
            xattrs: inode.xattrs.clone(),
            streams: inode.streams.clone(),
        }),
    }
}

fn apply_change(inode: &mut Inode, change: &MetaChange) {
    if let Some(mode) = change.mode {
        inode.mode = mode;
    }
    if let Some(uid) = change.uid {
        inode.uid = uid;
    }
    if let Some(gid) = change.gid {
        inode.gid = gid;
    }
    if let Some(modified) = change.modified_ns {
        inode.modified_ns = modified;
    }
    if let Some(accessed) = change.accessed_ns {
        inode.accessed_ns = accessed;
    }
    if let Some(created) = change.created_ns {
        inode.created_ns = created;
    }
    for xattr in &change.set_xattrs {
        inode.xattrs.retain(|known| known.name != xattr.name);
        inode.xattrs.push(xattr.clone());
    }
    inode
        .xattrs
        .retain(|known| !change.remove_xattrs.contains(&known.name));
    inode
        .xattrs
        .sort_by(|left, right| left.name.cmp(&right.name));
    if let Some(acl) = &change.acl {
        inode.acl = Some(acl.clone());
    }
    inode.windows_attributes |= change.set_windows_attributes;
    inode.windows_attributes &= !change.clear_windows_attributes;
}

fn restore_into(
    inode: &mut Inode,
    meta: &ExtendedMeta,
    features: FsFeatures,
) -> MetaTransferReport {
    let mut report = MetaTransferReport::default();
    let basic = &meta.basic;
    inode.mode = basic.mode;
    if let Some(uid) = basic.uid {
        inode.uid = uid;
    }
    if let Some(gid) = basic.gid {
        inode.gid = gid;
    }
    inode.modified_ns = basic.modified_ns;
    if let Some(accessed) = basic.accessed_ns {
        inode.accessed_ns = accessed;
    }
    if let Some(created) = basic.created_ns {
        inode.created_ns = created;
    }
    inode.windows_attributes = basic.windows_attributes;
    inode.flags = meta.flags;
    if features.xattr {
        inode.xattrs.clone_from(&meta.xattrs);
    } else {
        report.not_transferred.extend(
            meta.xattrs
                .iter()
                .map(|xattr| format!("xattr:{}", xattr.name.to_string_lossy())),
        );
    }
    match (&meta.acl, features.acl) {
        (Some(acl), true) => inode.acl = Some(acl.clone()),
        (Some(_), false) => report.not_transferred.push("acl".to_owned()),
        (None, _) => inode.acl = None,
    }
    if meta.account_sid.is_some() {
        report.not_transferred.push("account".to_owned());
    }
    report
}

struct VfsReader {
    data: Vec<u8>,
    position: u64,
    meta: FileMeta,
    max_chunk: Option<usize>,
}

impl Read for VfsReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(self.position)
            .unwrap_or(usize::MAX)
            .min(self.data.len());
        let remaining = &self.data[start..];
        let limit = self.max_chunk.unwrap_or(usize::MAX);
        let count = buf.len().min(remaining.len()).min(limit);
        buf[..count].copy_from_slice(&remaining[..count]);
        self.position = self.position.saturating_add(to_u64(count));
        Ok(count)
    }
}

impl Seek for VfsReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.position = seek_target(self.position, to_u64(self.data.len()), position)?;
        Ok(self.position)
    }
}

impl ReadHandle for VfsReader {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(self.meta.clone())
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self.max_chunk {
            Some(_) => None,
            None => Some(&self.data),
        }
    }
}

fn seek_target(current: u64, end: u64, position: SeekFrom) -> io::Result<u64> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "seek before the start of the file",
        )
    };
    match position {
        SeekFrom::Start(offset) => Ok(offset),
        SeekFrom::End(delta) => end.checked_add_signed(delta).ok_or_else(invalid),
        SeekFrom::Current(delta) => current.checked_add_signed(delta).ok_or_else(invalid),
    }
}

struct VfsWriter {
    shared: Arc<Shared>,
    vol: usize,
    inode: u64,
    stream: Option<String>,
    path: AbsPath,
    position: u64,
}

impl VfsWriter {
    fn with_data<T>(
        &self,
        change: impl FnOnce(&mut Vec<u8>, &Volume) -> io::Result<T>,
    ) -> io::Result<T> {
        let now = self.shared.now();
        let mut state = self.shared.lock();
        let volume = state
            .volumes
            .get_mut(self.vol)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let mut inode = volume
            .inodes
            .remove(&self.inode)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let result = {
            let data = match &self.stream {
                Some(name) => inode.streams.entry(name.clone()).or_default(),
                None => &mut inode.content,
            };
            change(data, volume)
        };
        if result.is_ok() {
            inode.modified_ns = now;
        }
        volume.inodes.insert(self.inode, inode);
        result
    }
}

fn no_space() -> io::Error {
    io::Error::new(
        io::ErrorKind::StorageFull,
        "no space left on the virtual volume",
    )
}

impl Write for VfsWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let start = usize::try_from(self.position).map_err(|_| no_space())?;
        let written = self.with_data(|data, volume| {
            let end = start.saturating_add(buf.len());
            let grow = to_u64(end.saturating_sub(data.len()));
            if let Some(capacity) = volume.capacity
                && volume
                    .used()
                    .saturating_add(to_u64(data.len()))
                    .saturating_add(grow)
                    > capacity
            {
                return Err(no_space());
            }
            if data.len() < end {
                data.resize(end, 0);
            }
            data[start..end].copy_from_slice(buf);
            Ok(buf.len())
        })?;
        self.position = self.position.saturating_add(to_u64(written));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for VfsWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let end = self.with_data(|data, _| Ok(to_u64(data.len())))?;
        self.position = seek_target(self.position, end, position)?;
        Ok(self.position)
    }
}

impl WriteHandle for VfsWriter {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, _durability: nurt_contracts::fs::Durability) -> FsResult<()> {
        self.with_data(|_, _| Ok(()))
            .map_err(|_| fs_error(FsErrorKind::Io, "sync", self.path.as_path()))
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        let path = self.path.as_path().to_path_buf();
        let new_size = usize::try_from(length)
            .map_err(|_| fs_error(FsErrorKind::NoSpace, "set_length", &path))?;
        self.with_data(|data, volume| {
            let grow = to_u64(new_size.saturating_sub(data.len()));
            if let Some(capacity) = volume.capacity
                && volume
                    .used()
                    .saturating_add(to_u64(data.len()))
                    .saturating_add(grow)
                    > capacity
            {
                return Err(no_space());
            }
            data.resize(new_size, 0);
            Ok(())
        })
        .map_err(|error| {
            let kind = if error.kind() == io::ErrorKind::StorageFull {
                FsErrorKind::NoSpace
            } else {
                FsErrorKind::Io
            };
            fs_error(kind, "set_length", &path)
        })
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        let mut state = self.shared.lock();
        let volume = state
            .volumes
            .get_mut(self.vol)
            .ok_or_else(|| fs_error(FsErrorKind::Io, "apply_meta", self.path.as_path()))?;
        let features = volume.info.features;
        let inode = volume
            .inodes
            .get_mut(&self.inode)
            .ok_or_else(|| fs_error(FsErrorKind::Io, "apply_meta", self.path.as_path()))?;
        let modified = inode.modified_ns;
        let report = restore_into(inode, meta, features);
        inode.modified_ns = modified;
        Ok(report)
    }
}

struct VfsLock {
    shared: Arc<Shared>,
    path: AbsPath,
}

impl FileLock for VfsLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

impl Drop for VfsLock {
    fn drop(&mut self) {
        self.shared.lock().locks.retain(|held| held != &self.path);
    }
}
