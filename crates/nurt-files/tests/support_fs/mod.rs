// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Read-only file systems for the integration tests of the walker.
//!
//! [`DiskFs`] reads a real directory tree through `std::fs`. [`MemFs`] is an
//! in-memory tree with injectable failures, used for cases that a real file
//! system cannot produce on every platform or under every account: refused
//! access, names that are not valid Unicode, entries whose kind the listing
//! does not report.

// Every integration test binary uses a different subset of the helpers; the
// module is private to each binary, so its public items are reachable only
// there; helpers report setup failures by panicking.
#![allow(dead_code, unreachable_pub, clippy::expect_used)]

use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileId, FileKind, FileMeta, Follow, Fs, FsError,
    FsErrorKind, FsFeatures, FsResult, Holder, PathKey, ReadHandle, SpaceInfo, VolumeId,
    VolumeInfo, VolumeKind,
};

/// Longest chain of links followed before resolution fails with
/// [`FsErrorKind::Loop`], as on Linux.
const MAX_LINK_HOPS: u32 = 40;

/// Volume number reported by [`MemFs`].
const MEM_VOLUME: u64 = 1;

fn error(kind: FsErrorKind, op: &str, path: &Path) -> FsError {
    FsError::new(kind, op, Some(path))
}

fn unsupported(op: &str, path: &Path) -> FsError {
    error(FsErrorKind::Unsupported, op, path)
}

fn volume_info(mount: AbsPath, dev: u64) -> VolumeInfo {
    VolumeInfo {
        id: VolumeId {
            dev,
            uuid: None,
            subvolume: 0,
        },
        mount,
        fs_type: "test".to_owned(),
        kind: VolumeKind::Other,
        features: FsFeatures {
            case_sensitive: true,
            ..FsFeatures::default()
        },
        rotational: false,
        read_only: true,
    }
}

fn root_path() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    }
}

/// Open file of the test file systems.
struct Reader<R> {
    inner: R,
    meta: FileMeta,
}

impl<R: Read> Read for Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Reader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl<R: Read + Seek + Send + Sync> ReadHandle for Reader<R> {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(self.meta.clone())
    }
}

enum MemKind {
    Dir,
    File(Vec<u8>),
    Link(PathBuf),
    Special,
}

struct MemNode {
    kind: MemKind,
    id: u64,
    denied: bool,
}

/// In-memory tree rooted at `/` (`C:\` on Windows).
///
/// Paths in the builder methods are relative to the root with `/`
/// separators. Directories are listed in reverse byte order of names, so
/// tests detect callers that rely on the listing order.
pub struct MemFs {
    nodes: HashMap<PathBuf, MemNode>,
    children: HashMap<PathBuf, BTreeSet<OsString>>,
    next_id: u64,
    report_kinds: bool,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    /// Empty tree with a root directory.
    pub fn new() -> Self {
        let mut fs = Self {
            nodes: HashMap::new(),
            children: HashMap::new(),
            next_id: 1,
            report_kinds: true,
        };
        fs.insert(root_path(), MemKind::Dir);
        fs
    }

    /// Native path of `relative` below the root.
    pub fn native(relative: &str) -> PathBuf {
        let mut path = root_path();
        for part in relative.split('/').filter(|part| !part.is_empty()) {
            path.push(part);
        }
        path
    }

    /// Absolute path of `relative` below the root.
    pub fn path(relative: &str) -> AbsPath {
        AbsPath::new(Self::native(relative)).expect("test path is absolute")
    }

    /// Makes listings omit entry kinds, so that callers must read metadata.
    pub fn without_kinds(mut self) -> Self {
        self.report_kinds = false;
        self
    }

    /// Adds a directory and its missing parents.
    pub fn dir(&mut self, relative: &str) -> &mut Self {
        let path = Self::native(relative);
        self.ensure_dir(&path);
        self
    }

    /// Adds a file with `content`, creating missing parents.
    pub fn file(&mut self, relative: &str, content: &str) -> &mut Self {
        self.add(
            Self::native(relative),
            MemKind::File(content.as_bytes().to_vec()),
        );
        self
    }

    /// Adds a file with a raw name in the directory `parent`.
    pub fn file_named(&mut self, parent: &str, name: OsString) -> &mut Self {
        let path = Self::native(parent).join(name);
        self.add(path, MemKind::File(Vec::new()));
        self
    }

    /// Adds a symbolic link; a relative `target` is resolved against the
    /// directory of the link.
    pub fn link(&mut self, relative: &str, target: &str) -> &mut Self {
        let target = if target.starts_with('/') {
            Self::native(target)
        } else {
            target.split('/').collect()
        };
        self.add(Self::native(relative), MemKind::Link(target));
        self
    }

    /// Adds a device-like entry.
    pub fn special(&mut self, relative: &str) -> &mut Self {
        self.add(Self::native(relative), MemKind::Special);
        self
    }

    /// Refuses reading the directory or file at `relative`; its metadata
    /// stays readable, as for a directory without read permission.
    pub fn deny(&mut self, relative: &str) -> &mut Self {
        let path = Self::native(relative);
        let node = self.nodes.get_mut(&path).expect("the path to deny exists");
        node.denied = true;
        self
    }

    fn add(&mut self, path: PathBuf, kind: MemKind) {
        if let Some(parent) = path.parent() {
            self.ensure_dir(parent);
        }
        self.insert(path, kind);
    }

    fn ensure_dir(&mut self, path: &Path) {
        if self.nodes.contains_key(path) {
            return;
        }
        if let Some(parent) = path.parent() {
            self.ensure_dir(parent);
        }
        self.insert(path.to_path_buf(), MemKind::Dir);
    }

    fn insert(&mut self, path: PathBuf, kind: MemKind) {
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            self.children
                .entry(parent.to_path_buf())
                .or_default()
                .insert(name.to_os_string());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(
            path,
            MemNode {
                kind,
                id,
                denied: false,
            },
        );
    }

    /// Resolves links in every component and, with `follow_last`, in the
    /// last one; returns the path of the node reached.
    fn resolve(&self, path: &Path, follow_last: bool, hops: &mut u32) -> FsResult<PathBuf> {
        let components: Vec<Component<'_>> = path.components().collect();
        let mut current = PathBuf::new();
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::CurDir => continue,
                Component::ParentDir => {
                    current.pop();
                    continue;
                }
                _ => current.push(component.as_os_str()),
            }
            if matches!(component, Component::Prefix(_) | Component::RootDir) {
                continue;
            }
            let last = index + 1 == components.len();
            let node = self
                .nodes
                .get(&current)
                .ok_or_else(|| error(FsErrorKind::NotFound, "resolve", path))?;
            match &node.kind {
                MemKind::Link(target) if !last || follow_last => {
                    *hops += 1;
                    if *hops > MAX_LINK_HOPS {
                        return Err(error(FsErrorKind::Loop, "resolve", path));
                    }
                    let base = current
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(root_path);
                    current = self.resolve(&base.join(target), true, hops)?;
                }
                MemKind::File(_) | MemKind::Special if !last => {
                    return Err(error(FsErrorKind::NotADirectory, "resolve", path));
                }
                _ => {}
            }
            if !last
                && !matches!(
                    self.nodes.get(&current).map(|node| &node.kind),
                    Some(MemKind::Dir)
                )
            {
                return Err(error(FsErrorKind::NotADirectory, "resolve", path));
            }
        }
        Ok(current)
    }

    fn node(&self, path: &AbsPath, follow: Follow) -> FsResult<(PathBuf, &MemNode)> {
        let mut hops = 0;
        let resolved = self.resolve(path.as_path(), follow == Follow::Follow, &mut hops)?;
        let node = self
            .nodes
            .get(&resolved)
            .ok_or_else(|| error(FsErrorKind::NotFound, "lookup", path.as_path()))?;
        Ok((resolved, node))
    }

    fn meta_of(node: &MemNode) -> FileMeta {
        let (kind, size, mode) = match &node.kind {
            MemKind::Dir => (FileKind::Directory, 0, 0o755),
            MemKind::File(content) => (FileKind::File, content.len() as u64, 0o644),
            MemKind::Link(target) => (FileKind::Symlink, target.as_os_str().len() as u64, 0o777),
            MemKind::Special => (FileKind::Special, 0, 0o600),
        };
        FileMeta {
            kind,
            size,
            mode,
            uid: None,
            gid: None,
            links: 1,
            modified_ns: 0,
            accessed_ns: None,
            created_ns: None,
            changed_ns: None,
            file_id: FileId {
                volume: MEM_VOLUME,
                low: node.id,
                high: 0,
            },
            windows_attributes: 0,
        }
    }
}

impl Fs for MemFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        self.node(path, follow).map(|(_, node)| Self::meta_of(node))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        let (_, node) = self.node(path, follow)?;
        let link_target = match &node.kind {
            MemKind::Link(target) => Some(target.clone()),
            _ => None,
        };
        Ok(ExtendedMeta {
            basic: Self::meta_of(node),
            xattrs: Vec::new(),
            acl: None,
            account_sid: None,
            flags: None,
            streams: Vec::new(),
            link_target,
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let (_, node) = self.node(path, Follow::Follow)?;
        if node.denied {
            return Err(error(FsErrorKind::PermissionDenied, "open", path.as_path()));
        }
        match &node.kind {
            MemKind::File(content) => Ok(Box::new(Reader {
                inner: Cursor::new(content.clone()),
                meta: Self::meta_of(node),
            })),
            MemKind::Dir => Err(error(FsErrorKind::IsADirectory, "open", path.as_path())),
            _ => Err(error(FsErrorKind::Unsupported, "open", path.as_path())),
        }
    }

    fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        Err(unsupported("open_stream", path.as_path()))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let (resolved, node) = self.node(path, Follow::Follow)?;
        if !matches!(node.kind, MemKind::Dir) {
            return Err(error(
                FsErrorKind::NotADirectory,
                "read_dir",
                path.as_path(),
            ));
        }
        if node.denied {
            return Err(error(
                FsErrorKind::PermissionDenied,
                "read_dir",
                path.as_path(),
            ));
        }
        let names = self.children.get(&resolved).cloned().unwrap_or_default();
        let entries: Vec<FsResult<DirEntry>> = names
            .into_iter()
            .rev()
            .map(|name| {
                let kind = self
                    .nodes
                    .get(&resolved.join(&name))
                    .map(|child| Self::meta_of(child).kind)
                    .filter(|_| self.report_kinds);
                Ok(DirEntry { name, kind })
            })
            .collect();
        Ok(Box::new(entries.into_iter()))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        match self.node(path, Follow::NoFollow)?.1 {
            MemNode {
                kind: MemKind::Link(target),
                ..
            } => Ok(target.clone()),
            _ => Err(error(FsErrorKind::InvalidPath, "read_link", path.as_path())),
        }
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let (resolved, _) = self.node(path, Follow::Follow)?;
        AbsPath::new(resolved)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        Err(unsupported("content_hash", path.as_path()))
    }

    fn volume(&self, _path: &AbsPath) -> FsResult<VolumeInfo> {
        Ok(volume_info(Self::path(""), MEM_VOLUME))
    }

    fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
        Err(unsupported("space", &root_path()))
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        Ok(PathKey(
            path.as_path().as_os_str().as_encoded_bytes().to_vec(),
        ))
    }

    fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
        Ok(Vec::new())
    }

    fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
        Ok(false)
    }
}

/// Read-only view of the real file system.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiskFs;

fn disk_error(source: &io::Error, op: &str, path: &Path) -> FsError {
    let kind = match source.kind() {
        io::ErrorKind::NotFound => FsErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        io::ErrorKind::NotADirectory => FsErrorKind::NotADirectory,
        io::ErrorKind::IsADirectory => FsErrorKind::IsADirectory,
        other if format!("{other:?}") == "FilesystemLoop" => FsErrorKind::Loop,
        _ => FsErrorKind::Io,
    };
    let mut failure = error(kind, op, path);
    failure.os_code = source.raw_os_error();
    failure
}

fn disk_kind(file_type: fs::FileType) -> FileKind {
    if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Directory
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Special
    }
}

fn disk_meta(meta: &fs::Metadata) -> FileMeta {
    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
        .unwrap_or(0);
    #[cfg(unix)]
    let (mode, uid, gid, links, file_id) = {
        use std::os::unix::fs::MetadataExt;
        (
            meta.mode() & 0o7777,
            Some(meta.uid()),
            Some(meta.gid()),
            meta.nlink(),
            FileId {
                volume: meta.dev(),
                low: meta.ino(),
                high: 0,
            },
        )
    };
    #[cfg(not(unix))]
    let (mode, uid, gid, links, file_id) = (
        if meta.permissions().readonly() {
            0o444
        } else {
            0o644
        },
        None,
        None,
        1,
        FileId {
            volume: 0,
            low: 0,
            high: 0,
        },
    );
    #[cfg(windows)]
    let windows_attributes = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes()
    };
    #[cfg(not(windows))]
    let windows_attributes = 0;
    FileMeta {
        kind: disk_kind(meta.file_type()),
        size: meta.len(),
        mode,
        uid,
        gid,
        links,
        modified_ns,
        accessed_ns: None,
        created_ns: None,
        changed_ns: None,
        file_id,
        windows_attributes,
    }
}

impl Fs for DiskFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        let result = match follow {
            Follow::Follow => fs::metadata(path),
            Follow::NoFollow => fs::symlink_metadata(path),
        };
        result
            .map(|meta| disk_meta(&meta))
            .map_err(|source| disk_error(&source, "metadata", path.as_path()))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        let basic = self.metadata(path, follow)?;
        let link_target = (basic.kind == FileKind::Symlink)
            .then(|| fs::read_link(path).ok())
            .flatten();
        Ok(ExtendedMeta {
            basic,
            xattrs: Vec::new(),
            acl: None,
            account_sid: None,
            flags: None,
            streams: Vec::new(),
            link_target,
        })
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let file =
            fs::File::open(path).map_err(|source| disk_error(&source, "open", path.as_path()))?;
        let meta = file
            .metadata()
            .map_err(|source| disk_error(&source, "open", path.as_path()))?;
        if meta.is_dir() {
            return Err(error(FsErrorKind::IsADirectory, "open", path.as_path()));
        }
        Ok(Box::new(Reader {
            inner: file,
            meta: disk_meta(&meta),
        }))
    }

    fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        Err(unsupported("open_stream", path.as_path()))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let owned = path.as_path().to_path_buf();
        let entries =
            fs::read_dir(path).map_err(|source| disk_error(&source, "read_dir", &owned))?;
        Ok(Box::new(entries.map(move |item| {
            item.map(|entry| DirEntry {
                kind: entry.file_type().ok().map(disk_kind),
                name: entry.file_name(),
            })
            .map_err(|source| disk_error(&source, "read_dir", &owned))
        })))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        fs::read_link(path).map_err(|source| disk_error(&source, "read_link", path.as_path()))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        let real = fs::canonicalize(path)
            .map_err(|source| disk_error(&source, "real_path", path.as_path()))?;
        AbsPath::new(real)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        Err(unsupported("content_hash", path.as_path()))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        let dev = self.metadata(path, Follow::NoFollow)?.file_id.volume;
        let mount = AbsPath::new(root_path())?;
        Ok(volume_info(mount, dev))
    }

    fn space(&self, _volume: &VolumeId) -> FsResult<SpaceInfo> {
        Err(unsupported("space", &root_path()))
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        Ok(PathKey(
            path.as_path().as_os_str().as_encoded_bytes().to_vec(),
        ))
    }

    fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
        Ok(Vec::new())
    }

    fn can_write_dir(&self, _dir: &AbsPath) -> FsResult<bool> {
        Ok(false)
    }
}

/// Name that is not valid Unicode: the byte `0xFF` on Unix, an unpaired
/// surrogate on Windows, followed by `suffix`.
pub fn non_unicode_name(suffix: &str) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = vec![0xFF];
        bytes.extend_from_slice(suffix.as_bytes());
        OsString::from_vec(bytes)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut wide = vec![0xD800];
        wide.extend(suffix.encode_utf16());
        OsString::from_wide(&wide)
    }
}

/// Relative path of `path` below `root` in language form.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Language form of an [`OsStr`], for assertions.
pub fn lossy(name: &OsStr) -> String {
    name.to_string_lossy().into_owned()
}
