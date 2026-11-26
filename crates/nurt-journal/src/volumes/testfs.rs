// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-memory file system with simulated volumes, used by the volume tests.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, Etag, ExtendedMeta, FileId, FileKind, FileLock,
    FileMeta, Follow, FsError, FsErrorKind, FsFeatures, FsResult, FsWrite, Holder, LinkKind,
    MetaChange, MetaTransferReport, PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo,
    TxScope, VolumeId, VolumeInfo, VolumeKind, WriteHandle,
};

/// Windows attribute bit reported for entries marked hidden.
const HIDDEN_ATTRIBUTE: u32 = 0x2;
/// Account id reported for every entry.
const TEST_UID: u32 = 1000;
/// Device of the root volume, which holds every path outside the attached
/// volumes.
const ROOT_DEV: u64 = 0;
/// Space figures of the root volume; no test measures it.
const ROOT_SPACE: SpaceInfo = SpaceInfo {
    total: 0,
    available: 0,
    quota_remaining: None,
    quota_limit: None,
    purgeable_system: None,
};

#[derive(Debug, Clone)]
enum Node {
    Dir {
        mode: u32,
        hidden: bool,
        lower: bool,
    },
    File {
        data: Vec<u8>,
        mode: u32,
        hidden: bool,
    },
    Symlink {
        target: PathBuf,
    },
}

#[derive(Debug, Clone)]
struct TestVolume {
    root: PathBuf,
    info: VolumeInfo,
    space: SpaceInfo,
}

#[derive(Default)]
struct State {
    nodes: BTreeMap<PathBuf, (u64, Node)>,
    volumes: Vec<TestVolume>,
    read_only_dirs: BTreeSet<PathBuf>,
    failures: Vec<(String, PathBuf, FsErrorKind)>,
    locks: BTreeSet<PathBuf>,
    next_id: u64,
    calls: Vec<String>,
}

impl State {
    fn volume_of(&self, path: &Path) -> Option<&TestVolume> {
        self.volumes
            .iter()
            .filter(|volume| path.starts_with(&volume.root))
            .max_by_key(|volume| volume.root.components().count())
    }

    fn insert(&mut self, path: PathBuf, node: Node) {
        self.next_id += 1;
        self.nodes.insert(path, (self.next_id, node));
    }

    fn node(&self, path: &Path) -> Option<&Node> {
        self.nodes.get(path).map(|(_, node)| node)
    }

    fn is_dir(&self, path: &Path) -> bool {
        matches!(self.node(path), Some(Node::Dir { .. }))
    }

    fn check(&self, op: &str, path: &Path) -> FsResult<()> {
        let failure = self
            .failures
            .iter()
            .find(|(name, at, _)| name == op && path.starts_with(at));
        match failure {
            Some((_, _, kind)) => Err(FsError::new(*kind, op, Some(path))),
            None => Ok(()),
        }
    }

    fn writable(&self, op: &str, dir: &Path) -> FsResult<()> {
        if !self.is_dir(dir) {
            return Err(FsError::new(FsErrorKind::NotFound, op, Some(dir)));
        }
        if self
            .volume_of(dir)
            .is_some_and(|volume| volume.info.read_only)
        {
            return Err(FsError::new(FsErrorKind::ReadOnly, op, Some(dir)));
        }
        if self.read_only_dirs.contains(dir) {
            return Err(FsError::new(FsErrorKind::PermissionDenied, op, Some(dir)));
        }
        Ok(())
    }

    fn parent_writable(&self, op: &str, path: &Path) -> FsResult<()> {
        match path.parent() {
            Some(parent) => self.writable(op, parent),
            None => Err(FsError::new(FsErrorKind::PermissionDenied, op, Some(path))),
        }
    }

    fn subtree(&self, root: &Path) -> Vec<PathBuf> {
        self.nodes
            .keys()
            .filter(|path| path.starts_with(root))
            .cloned()
            .collect()
    }
}

/// In-memory file system. Volumes are attached at root directories; a path
/// belongs to the volume with the longest matching root, and among volumes
/// attached at the same root to the one attached last. As on a real system,
/// a root volume at `/` holds every path outside the attached volumes.
#[derive(Clone)]
pub(crate) struct TestFs {
    state: Arc<Mutex<State>>,
    released: Arc<Condvar>,
}

/// Volume description with durable metadata and the given class.
pub(crate) fn volume_info(dev: u64, mount: &str, fs_type: &str, kind: VolumeKind) -> VolumeInfo {
    VolumeInfo {
        id: VolumeId {
            dev,
            uuid: Some(format!("uuid-{dev}")),
            subvolume: 0,
        },
        mount: abs(mount),
        fs_type: fs_type.to_owned(),
        kind,
        features: FsFeatures {
            durable_metadata: true,
            case_sensitive: true,
            ..FsFeatures::default()
        },
        rotational: false,
        read_only: false,
    }
}

/// Absolute path from a literal.
pub(crate) fn abs(path: &str) -> AbsPath {
    AbsPath::for_test(path)
}

impl TestFs {
    pub(crate) fn new() -> Self {
        let fs = Self {
            state: Arc::new(Mutex::new(State::default())),
            released: Arc::new(Condvar::new()),
        };
        fs.add_volume(
            "/",
            volume_info(ROOT_DEV, "/", "rootfs", VolumeKind::Fixed),
            ROOT_SPACE,
        );
        fs
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Attaches a volume whose entries start at `root` (usually its mount
    /// point; for a nested subvolume, the subvolume directory).
    pub(crate) fn add_volume(&self, root: &str, info: VolumeInfo, space: SpaceInfo) {
        let mut state = self.state();
        state.volumes.push(TestVolume {
            root: PathBuf::from(root),
            info,
            space,
        });
        drop(state);
        self.mkdir_p(root);
    }

    /// Creates a directory and its missing ancestors with mode `0755`.
    pub(crate) fn mkdir_p(&self, path: &str) {
        let mut state = self.state();
        let mut current = PathBuf::new();
        for component in Path::new(path).components() {
            current.push(component);
            if state.node(&current).is_none() {
                state.insert(
                    current.clone(),
                    Node::Dir {
                        mode: 0o755,
                        hidden: false,
                        lower: false,
                    },
                );
            }
        }
    }

    pub(crate) fn write_file(&self, path: &str, data: &[u8]) {
        self.state().insert(
            PathBuf::from(path),
            Node::File {
                data: data.to_vec(),
                mode: 0o644,
                hidden: false,
            },
        );
    }

    pub(crate) fn symlink(&self, path: &str, target: &str) {
        self.state().insert(
            PathBuf::from(path),
            Node::Symlink {
                target: PathBuf::from(target),
            },
        );
    }

    /// Marks a directory as belonging to the lower layer of an overlay:
    /// renaming it fails with a cross-device error.
    pub(crate) fn mark_lower(&self, path: &str) {
        if let Some((_, Node::Dir { lower, .. })) = self.state().nodes.get_mut(Path::new(path)) {
            *lower = true;
        }
    }

    /// Denies creation of entries in a directory.
    pub(crate) fn deny_write(&self, path: &str) {
        self.state().read_only_dirs.insert(PathBuf::from(path));
    }

    /// Makes the operation `op` on `path` and on every entry below it fail
    /// with `kind`.
    pub(crate) fn fail(&self, op: &str, path: &str, kind: FsErrorKind) {
        self.state()
            .failures
            .push((op.to_owned(), PathBuf::from(path), kind));
    }

    pub(crate) fn clear_failures(&self) {
        self.state().failures.clear();
    }

    pub(crate) fn read(&self, path: &str) -> Option<Vec<u8>> {
        match self.state().node(Path::new(path)) {
            Some(Node::File { data, .. }) => Some(data.clone()),
            _ => None,
        }
    }

    pub(crate) fn exists(&self, path: &str) -> bool {
        self.state().node(Path::new(path)).is_some()
    }

    pub(crate) fn mode(&self, path: &str) -> Option<u32> {
        match self.state().node(Path::new(path)) {
            Some(Node::Dir { mode, .. } | Node::File { mode, .. }) => Some(*mode),
            _ => None,
        }
    }

    pub(crate) fn is_hidden(&self, path: &str) -> bool {
        matches!(
            self.state().node(Path::new(path)),
            Some(Node::Dir { hidden: true, .. } | Node::File { hidden: true, .. })
        )
    }

    pub(crate) fn remove_all(&self, path: &str) {
        let mut state = self.state();
        for key in state.subtree(Path::new(path)) {
            state.nodes.remove(&key);
        }
    }

    /// Names of entries directly inside a directory.
    pub(crate) fn children(&self, path: &str) -> Vec<String> {
        let state = self.state();
        state
            .nodes
            .keys()
            .filter(|key| key.parent() == Some(Path::new(path)))
            .filter_map(|key| key.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }

    /// Mutating calls in order, as `op path`.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }

    fn record(&self, op: &str, path: &AbsPath) {
        self.state().calls.push(format!("{op} {path}"));
    }

    fn meta_of(state: &State, path: &Path) -> FsResult<FileMeta> {
        let (id, node) = state
            .nodes
            .get(path)
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "metadata", Some(path)))?;
        let (kind, size, mode, hidden) = match node {
            Node::Dir { mode, hidden, .. } => (FileKind::Directory, 0, *mode, *hidden),
            Node::File {
                data, mode, hidden, ..
            } => (FileKind::File, data.len() as u64, *mode, *hidden),
            Node::Symlink { .. } => (FileKind::Symlink, 0, 0o777, false),
        };
        let dev = state.volume_of(path).map_or(0, |volume| volume.info.id.dev);
        Ok(FileMeta {
            kind,
            size,
            mode,
            uid: Some(TEST_UID),
            gid: Some(TEST_UID),
            links: 1,
            modified_ns: 0,
            accessed_ns: None,
            created_ns: None,
            changed_ns: None,
            file_id: FileId {
                volume: dev,
                low: *id,
                high: 0,
            },
            windows_attributes: if hidden { HIDDEN_ATTRIBUTE } else { 0 },
        })
    }
}

fn unsupported<T>(op: &str, path: &AbsPath) -> FsResult<T> {
    Err(FsError::new(
        FsErrorKind::Unsupported,
        op,
        Some(path.as_path()),
    ))
}

struct TestReader {
    cursor: Cursor<Vec<u8>>,
    meta: FileMeta,
}

impl Read for TestReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.cursor.read(buf)
    }
}

impl Seek for TestReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(pos)
    }
}

impl ReadHandle for TestReader {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(self.meta.clone())
    }
}

struct TestWriter {
    path: AbsPath,
    state: Arc<Mutex<State>>,
    position: u64,
}

impl Write for TestWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.lock().unwrap();
        if state.check("write", self.path.as_path()).is_err() {
            return Err(io::Error::other("injected write failure"));
        }
        let Some((_, Node::File { data, .. })) = state.nodes.get_mut(self.path.as_path()) else {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        let start = usize::try_from(self.position).unwrap();
        if data.len() < start + buf.len() {
            data.resize(start + buf.len(), 0);
        }
        data[start..start + buf.len()].copy_from_slice(buf);
        self.position += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for TestWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if let SeekFrom::Start(offset) = pos {
            self.position = offset;
        }
        Ok(self.position)
    }
}

impl WriteHandle for TestWriter {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, _durability: Durability) -> FsResult<()> {
        self.state
            .lock()
            .unwrap()
            .check("sync", self.path.as_path())
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        let mut state = self.state.lock().unwrap();
        if let Some((_, Node::File { data, .. })) = state.nodes.get_mut(self.path.as_path()) {
            data.resize(usize::try_from(length).unwrap(), 0);
        }
        Ok(())
    }

    fn apply_meta(&mut self, _meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        Ok(MetaTransferReport::default())
    }
}

struct TestLock {
    path: AbsPath,
    state: Arc<Mutex<State>>,
    released: Arc<Condvar>,
}

impl FileLock for TestLock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

impl Drop for TestLock {
    fn drop(&mut self) {
        self.state.lock().unwrap().locks.remove(self.path.as_path());
        self.released.notify_all();
    }
}

impl nurt_contracts::fs::Fs for TestFs {
    fn metadata(&self, path: &AbsPath, _follow: Follow) -> FsResult<FileMeta> {
        let state = self.state();
        state.check("metadata", path.as_path())?;
        Self::meta_of(&state, path.as_path())
    }

    fn extended_metadata(&self, path: &AbsPath, _follow: Follow) -> FsResult<ExtendedMeta> {
        unsupported("extended_metadata", path)
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let state = self.state();
        state.check("open_read", path.as_path())?;
        let meta = Self::meta_of(&state, path.as_path())?;
        match state.node(path.as_path()) {
            Some(Node::File { data, .. }) => Ok(Box::new(TestReader {
                cursor: Cursor::new(data.clone()),
                meta,
            })),
            _ => Err(FsError::new(
                FsErrorKind::IsADirectory,
                "open_read",
                Some(path.as_path()),
            )),
        }
    }

    fn open_stream_read(&self, path: &AbsPath, _stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        unsupported("open_stream_read", path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        let entries: Vec<FsResult<DirEntry>> = self
            .children(&path.to_language_string())
            .into_iter()
            .map(|name| {
                Ok(DirEntry {
                    name: name.into(),
                    kind: None,
                })
            })
            .collect();
        Ok(Box::new(entries.into_iter()))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        match self.state().nodes.get(path.as_path()) {
            Some((_, Node::Symlink { target })) => Ok(target.clone()),
            Some(_) => Err(FsError::new(
                FsErrorKind::InvalidPath,
                "read_link",
                Some(path.as_path()),
            )),
            None => Err(FsError::new(
                FsErrorKind::NotFound,
                "read_link",
                Some(path.as_path()),
            )),
        }
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        Ok(path.clone())
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        unsupported("content_hash", path)
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        let state = self.state();
        state.check("volume", path.as_path())?;
        state
            .volume_of(path.as_path())
            .map(|volume| volume.info.clone())
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "volume", Some(path.as_path())))
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        self.state()
            .volumes
            .iter()
            .find(|candidate| candidate.info.id == *volume)
            .map(|candidate| candidate.space)
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "space", None))
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        Ok(PathKey(path.to_language_string().into_bytes()))
    }

    fn holders(&self, _path: &AbsPath) -> FsResult<Vec<Holder>> {
        Ok(Vec::new())
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        let state = self.state();
        state.check("can_write_dir", dir.as_path())?;
        match state.node(dir.as_path()) {
            None => Err(FsError::new(
                FsErrorKind::NotFound,
                "can_write_dir",
                Some(dir.as_path()),
            )),
            Some(Node::Dir { .. }) => Ok(state.writable("can_write_dir", dir.as_path()).is_ok()),
            Some(_) => Err(FsError::new(
                FsErrorKind::NotADirectory,
                "can_write_dir",
                Some(dir.as_path()),
            )),
        }
    }

    fn etag(&self, path: &AbsPath, follow: Follow) -> FsResult<Etag> {
        self.metadata(path, follow).map(|meta| meta.etag())
    }
}

impl FsWrite for TestFs {
    fn create_temp_in(
        &self,
        _scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let mut state = self.state();
        state.check("create_temp_in", dir.as_path())?;
        state.writable("create_temp_in", dir.as_path())?;
        let name = format!(".{name_hint}.{}.tmp", state.next_id + 1);
        let path = dir.join_name(&name)?;
        state.insert(
            path.as_path().to_path_buf(),
            Node::File {
                data: Vec::new(),
                mode: 0o600,
                hidden: false,
            },
        );
        drop(state);
        self.record("create_temp_in", &path);
        Ok(Box::new(TestWriter {
            path,
            state: Arc::clone(&self.state),
            position: 0,
        }))
    }

    fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let mut state = self.state();
        state.check("create_new", path.as_path())?;
        state.parent_writable("create_new", path.as_path())?;
        if state.node(path.as_path()).is_some() {
            return Err(FsError::new(
                FsErrorKind::Exists,
                "create_new",
                Some(path.as_path()),
            ));
        }
        state.insert(
            path.as_path().to_path_buf(),
            Node::File {
                data: Vec::new(),
                mode: 0o600,
                hidden: false,
            },
        );
        Ok(Box::new(TestWriter {
            path: path.clone(),
            state: Arc::clone(&self.state),
            position: 0,
        }))
    }

    fn open_in_place(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        unsupported("open_in_place", path)
    }

    fn open_stream_write(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        unsupported("open_stream_write", path)
    }

    fn rename(
        &self,
        _scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        let mut state = self.state();
        state.check("rename", from.as_path())?;
        let Some(node) = state.node(from.as_path()).cloned() else {
            return Err(FsError::new(
                FsErrorKind::NotFound,
                "rename",
                Some(from.as_path()),
            ));
        };
        let source = state.volume_of(from.as_path()).map(|v| v.info.id.clone());
        let destination = state.volume_of(to.as_path()).map(|v| v.info.id.clone());
        let lower = matches!(node, Node::Dir { lower: true, .. });
        if source != destination || lower {
            return Err(FsError::new(
                FsErrorKind::CrossDevice,
                "rename",
                Some(from.as_path()),
            ));
        }
        state.parent_writable("rename", to.as_path())?;
        if state.node(to.as_path()).is_some() {
            if mode == RenameMode::NoReplace {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "rename",
                    Some(to.as_path()),
                ));
            }
            for key in state.subtree(to.as_path()) {
                state.nodes.remove(&key);
            }
        }
        for key in state.subtree(from.as_path()) {
            if let Some(entry) = state.nodes.remove(&key) {
                let suffix = key.strip_prefix(from.as_path()).unwrap().to_path_buf();
                let moved = if suffix.as_os_str().is_empty() {
                    to.as_path().to_path_buf()
                } else {
                    to.as_path().join(suffix)
                };
                state.nodes.insert(moved, entry);
            }
        }
        drop(state);
        self.record("rename", to);
        Ok(())
    }

    fn exchange(&self, _scope: &TxScope, a: &AbsPath, _b: &AbsPath) -> FsResult<()> {
        unsupported("exchange", a)
    }

    fn replace_with_backup(
        &self,
        _scope: &TxScope,
        target: &AbsPath,
        _replacement: &AbsPath,
        _backup: &AbsPath,
    ) -> FsResult<()> {
        unsupported("replace_with_backup", target)
    }

    fn hard_link(&self, _scope: &TxScope, existing: &AbsPath, _new: &AbsPath) -> FsResult<()> {
        unsupported("hard_link", existing)
    }

    fn clone_file(&self, _scope: &TxScope, src: &AbsPath, _dst: &AbsPath) -> FsResult<()> {
        unsupported("clone_file", src)
    }

    fn copy_file(&self, _scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        let mut state = self.state();
        let Some(Node::File { data, mode, .. }) = state.node(src.as_path()).cloned() else {
            return Err(FsError::new(
                FsErrorKind::NotFound,
                "copy_file",
                Some(src.as_path()),
            ));
        };
        state.parent_writable("copy_file", dst.as_path())?;
        let size = data.len() as u64;
        state.insert(
            dst.as_path().to_path_buf(),
            Node::File {
                data,
                mode,
                hidden: false,
            },
        );
        Ok(size)
    }

    fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.check("remove_file", path.as_path())?;
        match state.node(path.as_path()) {
            Some(Node::Dir { .. }) => Err(FsError::new(
                FsErrorKind::IsADirectory,
                "remove_file",
                Some(path.as_path()),
            )),
            Some(_) => {
                state.nodes.remove(path.as_path());
                Ok(())
            }
            None => Err(FsError::new(
                FsErrorKind::NotFound,
                "remove_file",
                Some(path.as_path()),
            )),
        }
    }

    fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        if state.subtree(path.as_path()).len() > 1 {
            return Err(FsError::new(
                FsErrorKind::NotEmpty,
                "remove_dir",
                Some(path.as_path()),
            ));
        }
        state.nodes.remove(path.as_path());
        Ok(())
    }

    fn remove_tree(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _parallelism: usize,
    ) -> FsResult<RemoveStats> {
        let mut state = self.state();
        let mut removed = RemoveStats::default();
        for key in state.subtree(path.as_path()) {
            match state.nodes.remove(&key) {
                Some((_, Node::Dir { .. })) => removed.directories += 1,
                Some(_) => removed.files += 1,
                None => {}
            }
        }
        Ok(removed)
    }

    fn create_dir(&self, _scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        let mut state = self.state();
        state.check("create_dir", path.as_path())?;
        if state.node(path.as_path()).is_some() {
            return Err(FsError::new(
                FsErrorKind::Exists,
                "create_dir",
                Some(path.as_path()),
            ));
        }
        state.parent_writable("create_dir", path.as_path())?;
        state.insert(
            path.as_path().to_path_buf(),
            Node::Dir {
                mode,
                hidden: false,
                lower: false,
            },
        );
        drop(state);
        self.record("create_dir", path);
        Ok(())
    }

    fn create_link(
        &self,
        _scope: &TxScope,
        _target: &Path,
        link: &AbsPath,
        _kind: LinkKind,
    ) -> FsResult<()> {
        unsupported("create_link", link)
    }

    fn set_meta(&self, _scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        let mut state = self.state();
        if let (Some(new_mode), Some((_, Node::Dir { mode, .. } | Node::File { mode, .. }))) =
            (change.mode, state.nodes.get_mut(path.as_path()))
        {
            *mode = new_mode;
        }
        Ok(())
    }

    fn restore_meta(
        &self,
        _scope: &TxScope,
        path: &AbsPath,
        _meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        unsupported("restore_meta", path)
    }

    fn truncate(&self, _scope: &TxScope, path: &AbsPath, _length: u64) -> FsResult<()> {
        unsupported("truncate", path)
    }

    fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        let state = self.state();
        state.check("sync_dir", dir.as_path())?;
        if state.is_dir(dir.as_path()) {
            Ok(())
        } else {
            Err(FsError::new(
                FsErrorKind::NotFound,
                "sync_dir",
                Some(dir.as_path()),
            ))
        }
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state();
        state.check("lock_exclusive", path.as_path())?;
        while state.locks.contains(path.as_path()) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(FsError::new(
                    FsErrorKind::InUse,
                    "lock_exclusive",
                    Some(path.as_path()),
                ));
            }
            state = self.released.wait_timeout(state, left).unwrap().0;
        }
        state.locks.insert(path.as_path().to_path_buf());
        if state.node(path.as_path()).is_none() {
            state.insert(
                path.as_path().to_path_buf(),
                Node::File {
                    data: Vec::new(),
                    mode: 0o600,
                    hidden: false,
                },
            );
        }
        Ok(Box::new(TestLock {
            path: path.clone(),
            state: Arc::clone(&self.state),
            released: Arc::clone(&self.released),
        }))
    }

    fn mark_hidden(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        let mut state = self.state();
        state.check("mark_hidden", path.as_path())?;
        match state.nodes.get_mut(path.as_path()) {
            Some((_, Node::Dir { hidden, .. } | Node::File { hidden, .. })) => {
                *hidden = true;
                Ok(())
            }
            _ => Err(FsError::new(
                FsErrorKind::NotFound,
                "mark_hidden",
                Some(path.as_path()),
            )),
        }
    }
}
