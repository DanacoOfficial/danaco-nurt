// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Free space measurement.
//!
//! Free space `F` is what ordinary programs see: the bytes available to the
//! account after the journal took its share (`statvfs` `f_bavail × f_frsize`,
//! `volumeAvailableCapacityKey`, `lpFreeBytesAvailableToCaller`). The safety
//! condition is `F ≥ R`, never `F + J ≥ R`, because installers check free space
//! before they write. With quota awareness the remaining account quota bounds
//! `F` and the size of the quota bounds the capacity `T`, from which the
//! reserve of a quota-limited account `R_q` is derived. Purgeable space
//! reported by macOS is not counted: the system gives no public way to make
//! journal data purgeable.

use std::time::Duration;

use nurt_contracts::config::SpaceConfig;
use nurt_contracts::env::Clock;
use nurt_contracts::fs::{Fs, FsResult, SpaceInfo, VolumeId};
use nurt_contracts::journal::VolumeState;
use nurt_contracts::stage::CancelToken;

/// One reading of the space figures of a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Measurement {
    /// Free bytes `F` available to the account.
    pub(crate) free: u64,
    /// Capacity `T`, or the account quota when smaller.
    pub(crate) total: u64,
    /// An account quota applies to the volume.
    pub(crate) quota_limited: bool,
}

impl Measurement {
    /// Reading derived from the figures reported by the file system.
    ///
    /// Back ends report the remaining quota and the quota size together; a
    /// back end that reports only one of them still marks the account as
    /// quota-limited.
    pub(crate) fn from_space(space: &SpaceInfo, quota_aware: bool) -> Self {
        let remaining = space.quota_remaining.filter(|_| quota_aware);
        let limit = space.quota_limit.filter(|_| quota_aware);
        Self {
            free: remaining.map_or(space.available, |remaining| space.available.min(remaining)),
            total: limit.map_or(space.total, |limit| space.total.min(limit)),
            quota_limited: remaining.is_some() || limit.is_some(),
        }
    }
}

/// Measures a volume with one `statvfs` or `GetDiskFreeSpaceExW` call.
pub(crate) fn measure(fs: &dyn Fs, volume: &VolumeId, quota_aware: bool) -> FsResult<Measurement> {
    fs.space(volume)
        .map(|space| Measurement::from_space(&space, quota_aware))
}

/// Shortest period between two iterations of the service, so that a period
/// of zero in the configuration cannot make the service spin.
pub(crate) const MIN_POLL_PERIOD: Duration = Duration::from_millis(10);

/// Measurement period of the service: the fast period at or below the soft
/// threshold, the normal period above it, never shorter than
/// [`MIN_POLL_PERIOD`].
pub(crate) fn poll_period(state: VolumeState, config: &SpaceConfig) -> Duration {
    let millis = match state {
        VolumeState::Normal => config.poll_ms,
        VolumeState::Soft | VolumeState::Hard | VolumeState::Emergency => config.poll_fast_ms,
    };
    Duration::from_millis(u64::from(millis)).max(MIN_POLL_PERIOD)
}

/// Waits until free space reaches `target` or `window` elapses, measuring
/// every `poll`, and returns the last reading.
///
/// File systems release blocks of removed files with a delay (ext4 at the
/// next journal commit, btrfs and APFS asynchronously, XFS after the log
/// commit), so the effect of a reclaim is judged by the growth of `F` within
/// the settle window, not by the sizes of the removed files.
pub(crate) fn settle(
    fs: &dyn Fs,
    clock: &dyn Clock,
    volume: &VolumeId,
    quota_aware: bool,
    target: u64,
    window: Duration,
    poll: Duration,
) -> FsResult<Measurement> {
    let deadline = clock.monotonic().saturating_add(window);
    let never = CancelToken::new();
    loop {
        let reading = measure(fs, volume, quota_aware)?;
        let now = clock.monotonic();
        if reading.free >= target || now >= deadline {
            return Ok(reading);
        }
        let pause = poll.min(deadline.saturating_sub(now));
        if pause.is_zero() || clock.sleep(pause, &never).is_err() {
            return Ok(reading);
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Volume with simulated capacity, a virtual clock and fault injection
    //! for tests of measurement, admission and reclaim.

    use std::collections::BTreeMap;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::Duration;

    use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
    use nurt_contracts::fs::{
        AbsPath, ContentHash, DirEntry, ExtendedMeta, FileLock, FileMeta, Follow, Fs, FsError,
        FsErrorKind, FsFeatures, FsResult, FsWrite, Holder, LinkKind, MetaChange,
        MetaTransferReport, PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, TxScope,
        VolumeId, VolumeInfo, VolumeKind, WriteHandle,
    };
    use nurt_contracts::stage::{CancelReason, CancelToken};
    use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints};

    /// Clock whose time moves only when a test or a sleep advances it.
    #[derive(Debug, Default)]
    pub(crate) struct TestClock {
        nanos: AtomicU64,
    }

    impl TestClock {
        pub(crate) fn advance(&self, by: Duration) {
            let nanos = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
            self.nanos.fetch_add(nanos, Ordering::SeqCst);
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> WallTime {
            WallTime(i128::from(self.nanos.load(Ordering::SeqCst)))
        }

        fn monotonic(&self) -> Duration {
            Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: self.now(),
                boot: BootId("test".to_owned()),
                since_boot_with_suspend: self.monotonic(),
                since_boot_active: self.monotonic(),
            }
        }

        fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
            if let Some(reason) = cancel.reason() {
                return Err(reason);
            }
            self.advance(duration);
            Ok(())
        }
    }

    /// Clock backed by the real monotonic clock, for timing measurements.
    #[derive(Debug)]
    pub(crate) struct RealClock {
        origin: std::time::Instant,
    }

    impl RealClock {
        pub(crate) fn new() -> Self {
            Self {
                origin: std::time::Instant::now(),
            }
        }
    }

    impl Clock for RealClock {
        fn now(&self) -> WallTime {
            WallTime(i128::try_from(self.origin.elapsed().as_nanos()).unwrap_or(i128::MAX))
        }

        fn monotonic(&self) -> Duration {
            self.origin.elapsed()
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: self.now(),
                boot: BootId("test".to_owned()),
                since_boot_with_suspend: self.monotonic(),
                since_boot_active: self.monotonic(),
            }
        }

        fn sleep(&self, duration: Duration, _cancel: &CancelToken) -> Result<(), CancelReason> {
            std::thread::sleep(duration);
            Ok(())
        }
    }

    /// Fault registry that applies one action at one point.
    #[derive(Debug)]
    pub(crate) struct OneFault {
        pub(crate) point: FailPoint,
        pub(crate) action: FailAction,
    }

    impl Failpoints for OneFault {
        fn hit(&self, point: FailPoint, _n: Option<u32>) -> FailAction {
            if point == self.point {
                self.action
            } else {
                FailAction::Continue
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NodeKind {
        File,
        Dir,
    }

    #[derive(Debug, Clone, Copy)]
    struct Node {
        kind: NodeKind,
        size: u64,
    }

    #[derive(Debug, Default)]
    struct State {
        nodes: BTreeMap<PathBuf, Node>,
        foreign: u64,
        pending: Vec<(Duration, u64)>,
        log: Vec<String>,
        tree_parallelism: Vec<usize>,
    }

    /// Single in-memory volume with a fixed capacity.
    ///
    /// Bytes of removed files become visible as free space after
    /// `release_delay`, as on file systems that free blocks lazily. Removing a
    /// file advances the clock by `per_file`, so that time budgets can be
    /// exercised with the virtual clock.
    pub(crate) struct TestVolume {
        pub(crate) clock: Arc<TestClock>,
        pub(crate) info: VolumeInfo,
        capacity: u64,
        quota_remaining: Mutex<Option<u64>>,
        quota_limit: Mutex<Option<u64>>,
        release_delay: Mutex<Duration>,
        per_file: Mutex<Duration>,
        fail_space: AtomicBool,
        state: Arc<Mutex<State>>,
    }

    fn unsupported(op: &str) -> FsError {
        FsError::new(FsErrorKind::Unsupported, op, None)
    }

    fn not_found(op: &str, path: &Path) -> FsError {
        FsError::new(FsErrorKind::NotFound, op, Some(path))
    }

    pub(crate) fn path(text: &str) -> AbsPath {
        AbsPath::for_test(&format!("/{text}"))
    }

    pub(crate) fn volume_id() -> VolumeId {
        VolumeId {
            dev: 7,
            uuid: Some("test-volume".to_owned()),
            subvolume: 0,
        }
    }

    pub(crate) fn volume_info(rotational: bool) -> VolumeInfo {
        VolumeInfo {
            id: volume_id(),
            mount: path("vol"),
            fs_type: "ext4".to_owned(),
            kind: VolumeKind::Fixed,
            features: FsFeatures::default(),
            rotational,
            read_only: false,
        }
    }

    impl TestVolume {
        pub(crate) fn new(capacity: u64) -> Self {
            Self {
                clock: Arc::new(TestClock::default()),
                info: volume_info(false),
                capacity,
                quota_remaining: Mutex::new(None),
                quota_limit: Mutex::new(None),
                release_delay: Mutex::new(Duration::ZERO),
                per_file: Mutex::new(Duration::ZERO),
                fail_space: AtomicBool::new(false),
                state: Arc::new(Mutex::new(State::default())),
            }
        }

        fn state(&self) -> MutexGuard<'_, State> {
            self.state.lock().expect("test volume state")
        }

        /// Sets the bytes used by other programs.
        pub(crate) fn set_foreign(&self, bytes: u64) {
            self.state().foreign = bytes;
        }

        /// Writes `bytes` for another program when they fit, as one atomic
        /// step; returns whether the write succeeded.
        pub(crate) fn try_add_foreign(&self, bytes: u64) -> bool {
            let mut state = self.state();
            if self.capacity.saturating_sub(self.used(&state)) < bytes {
                return false;
            }
            state.foreign = state.foreign.saturating_add(bytes);
            true
        }

        /// Creates a file of `size` bytes when it fits, as one atomic step;
        /// returns whether the file was created.
        pub(crate) fn try_put_file(&self, at: &AbsPath, size: u64) -> bool {
            let mut state = self.state();
            let fits = self.capacity.saturating_sub(self.used(&state)) >= size;
            if fits {
                Self::insert_file(&mut state, at, size);
            }
            fits
        }

        pub(crate) fn set_quota_remaining(&self, remaining: Option<u64>) {
            *self.quota_remaining.lock().expect("quota") = remaining;
        }

        pub(crate) fn set_quota_limit(&self, limit: Option<u64>) {
            *self.quota_limit.lock().expect("quota") = limit;
        }

        pub(crate) fn set_release_delay(&self, delay: Duration) {
            *self.release_delay.lock().expect("delay") = delay;
        }

        pub(crate) fn set_per_file(&self, cost: Duration) {
            *self.per_file.lock().expect("per file") = cost;
        }

        pub(crate) fn fail_space(&self, fail: bool) {
            self.fail_space.store(fail, Ordering::SeqCst);
        }

        /// Creates a file of `size` bytes with its parent directories.
        pub(crate) fn put_file(&self, at: &AbsPath, size: u64) {
            Self::insert_file(&mut self.state(), at, size);
        }

        fn insert_file(state: &mut State, at: &AbsPath, size: u64) {
            for ancestor in at.as_path().ancestors().skip(1) {
                state.nodes.entry(ancestor.to_path_buf()).or_insert(Node {
                    kind: NodeKind::Dir,
                    size: 0,
                });
            }
            state.nodes.insert(
                at.as_path().to_path_buf(),
                Node {
                    kind: NodeKind::File,
                    size,
                },
            );
        }

        pub(crate) fn exists(&self, at: &AbsPath) -> bool {
            self.state().nodes.contains_key(at.as_path())
        }

        pub(crate) fn log(&self) -> Vec<String> {
            self.state().log.clone()
        }

        pub(crate) fn tree_parallelism(&self) -> Vec<usize> {
            self.state().tree_parallelism.clone()
        }

        pub(crate) fn free(&self) -> u64 {
            self.space(&volume_id()).map_or(0, |space| space.available)
        }

        fn release(&self, state: &mut State, bytes: u64) {
            let delay = *self.release_delay.lock().expect("delay");
            let due = self.clock.monotonic().saturating_add(delay);
            state.pending.push((due, bytes));
        }

        fn charge_file(&self) {
            let cost = *self.per_file.lock().expect("per file");
            self.clock.advance(cost);
        }

        fn used(&self, state: &State) -> u64 {
            let now = self.clock.monotonic();
            let files: u64 = state.nodes.values().map(|node| node.size).sum();
            let unreleased: u64 = state
                .pending
                .iter()
                .filter(|(due, _)| *due > now)
                .map(|(_, bytes)| bytes)
                .sum();
            files
                .saturating_add(unreleased)
                .saturating_add(state.foreign)
        }

        fn subtree(state: &State, root: &Path) -> Vec<PathBuf> {
            state
                .nodes
                .keys()
                .filter(|key| key.starts_with(root))
                .cloned()
                .collect()
        }
    }

    struct TestHandle {
        path: AbsPath,
        state: Arc<Mutex<State>>,
    }

    impl io::Write for TestHandle {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut state = self.state.lock().expect("test volume state");
            if let Some(node) = state.nodes.get_mut(self.path.as_path()) {
                node.size = node.size.saturating_add(buf.len() as u64);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl io::Seek for TestHandle {
        fn seek(&mut self, _pos: io::SeekFrom) -> io::Result<u64> {
            Ok(0)
        }
    }

    impl WriteHandle for TestHandle {
        fn path(&self) -> &AbsPath {
            &self.path
        }

        fn sync(&mut self, _durability: nurt_contracts::fs::Durability) -> FsResult<()> {
            Ok(())
        }

        fn set_length(&mut self, _length: u64) -> FsResult<()> {
            Err(unsupported("set_length"))
        }

        fn apply_meta(&mut self, _meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
            Err(unsupported("apply_meta"))
        }
    }

    impl Fs for TestVolume {
        fn metadata(&self, _path: &AbsPath, _follow: Follow) -> FsResult<FileMeta> {
            Err(unsupported("metadata"))
        }

        fn extended_metadata(&self, _path: &AbsPath, _follow: Follow) -> FsResult<ExtendedMeta> {
            Err(unsupported("extended_metadata"))
        }

        fn open_read(&self, _path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
            Err(unsupported("open_read"))
        }

        fn open_stream_read(
            &self,
            _path: &AbsPath,
            _stream: &str,
        ) -> FsResult<Box<dyn ReadHandle>> {
            Err(unsupported("open_stream_read"))
        }

        fn read_dir<'a>(
            &'a self,
            _path: &AbsPath,
        ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
            Err(unsupported("read_dir"))
        }

        fn read_link(&self, _path: &AbsPath) -> FsResult<PathBuf> {
            Err(unsupported("read_link"))
        }

        fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
            Ok(path.clone())
        }

        fn content_hash(&self, _path: &AbsPath) -> FsResult<ContentHash> {
            Err(unsupported("content_hash"))
        }

        fn volume(&self, _path: &AbsPath) -> FsResult<VolumeInfo> {
            Ok(self.info.clone())
        }

        fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
            if self.fail_space.load(Ordering::SeqCst) {
                return Err(FsError::new(FsErrorKind::Io, "space", None));
            }
            if *volume != self.info.id {
                return Err(FsError::new(FsErrorKind::NotFound, "space", None));
            }
            let used = self.used(&self.state());
            Ok(SpaceInfo {
                total: self.capacity,
                available: self.capacity.saturating_sub(used),
                quota_remaining: *self.quota_remaining.lock().expect("quota"),
                quota_limit: *self.quota_limit.lock().expect("quota"),
                purgeable_system: None,
            })
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

    impl FsWrite for TestVolume {
        fn create_temp_in(
            &self,
            _scope: &TxScope,
            _dir: &AbsPath,
            _name_hint: &str,
        ) -> FsResult<Box<dyn WriteHandle>> {
            Err(unsupported("create_temp_in"))
        }

        fn create_new(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
            let mut state = self.state();
            let parent = path.as_path().parent().map(Path::to_path_buf);
            if parent.is_some_and(|parent| !state.nodes.contains_key(&parent)) {
                return Err(not_found("create_new", path.as_path()));
            }
            if state.nodes.contains_key(path.as_path()) {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "create_new",
                    Some(path.as_path()),
                ));
            }
            state.nodes.insert(
                path.as_path().to_path_buf(),
                Node {
                    kind: NodeKind::File,
                    size: 0,
                },
            );
            Ok(Box::new(TestHandle {
                path: path.clone(),
                state: Arc::clone(&self.state),
            }))
        }

        fn open_in_place(
            &self,
            _scope: &TxScope,
            _path: &AbsPath,
        ) -> FsResult<Box<dyn WriteHandle>> {
            Err(unsupported("open_in_place"))
        }

        fn open_stream_write(
            &self,
            _scope: &TxScope,
            _path: &AbsPath,
            _stream: &str,
        ) -> FsResult<Box<dyn WriteHandle>> {
            Err(unsupported("open_stream_write"))
        }

        fn rename(
            &self,
            _scope: &TxScope,
            from: &AbsPath,
            to: &AbsPath,
            mode: RenameMode,
        ) -> FsResult<()> {
            let mut state = self.state();
            if !state.nodes.contains_key(from.as_path()) {
                return Err(not_found("rename", from.as_path()));
            }
            if mode == RenameMode::NoReplace && state.nodes.contains_key(to.as_path()) {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "rename",
                    Some(to.as_path()),
                ));
            }
            for old in Self::subtree(&state, from.as_path()) {
                if let (Some(node), Ok(rest)) =
                    (state.nodes.remove(&old), old.strip_prefix(from.as_path()))
                {
                    let new = if rest.as_os_str().is_empty() {
                        to.as_path().to_path_buf()
                    } else {
                        to.as_path().join(rest)
                    };
                    state.nodes.insert(new, node);
                }
            }
            state.log.push(format!(
                "rename {} {}",
                from.as_path().display(),
                to.as_path().display()
            ));
            Ok(())
        }

        fn exchange(&self, _scope: &TxScope, _a: &AbsPath, _b: &AbsPath) -> FsResult<()> {
            Err(unsupported("exchange"))
        }

        fn replace_with_backup(
            &self,
            _scope: &TxScope,
            _target: &AbsPath,
            _replacement: &AbsPath,
            _backup: &AbsPath,
        ) -> FsResult<()> {
            Err(unsupported("replace_with_backup"))
        }

        fn hard_link(&self, _scope: &TxScope, _existing: &AbsPath, _new: &AbsPath) -> FsResult<()> {
            Err(unsupported("hard_link"))
        }

        fn clone_file(&self, _scope: &TxScope, _src: &AbsPath, _dst: &AbsPath) -> FsResult<()> {
            Err(unsupported("clone_file"))
        }

        fn copy_file(&self, _scope: &TxScope, _src: &AbsPath, _dst: &AbsPath) -> FsResult<u64> {
            Err(unsupported("copy_file"))
        }

        fn remove_file(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            let mut state = self.state();
            match state.nodes.get(path.as_path()).copied() {
                None => Err(not_found("remove_file", path.as_path())),
                Some(node) if node.kind == NodeKind::Dir => Err(FsError::new(
                    FsErrorKind::IsADirectory,
                    "remove_file",
                    Some(path.as_path()),
                )),
                Some(node) => {
                    state.nodes.remove(path.as_path());
                    self.release(&mut state, node.size);
                    state
                        .log
                        .push(format!("remove_file {}", path.as_path().display()));
                    drop(state);
                    self.charge_file();
                    Ok(())
                }
            }
        }

        fn remove_dir(&self, _scope: &TxScope, path: &AbsPath) -> FsResult<()> {
            let mut state = self.state();
            if Self::subtree(&state, path.as_path()).len() > 1 {
                return Err(FsError::new(
                    FsErrorKind::NotEmpty,
                    "remove_dir",
                    Some(path.as_path()),
                ));
            }
            state
                .nodes
                .remove(path.as_path())
                .map(|_| ())
                .ok_or_else(|| not_found("remove_dir", path.as_path()))
        }

        fn remove_tree(
            &self,
            _scope: &TxScope,
            path: &AbsPath,
            parallelism: usize,
        ) -> FsResult<RemoveStats> {
            let mut state = self.state();
            let doomed = Self::subtree(&state, path.as_path());
            if doomed.is_empty() {
                return Err(not_found("remove_tree", path.as_path()));
            }
            let mut removed = RemoveStats::default();
            for key in doomed {
                if let Some(node) = state.nodes.remove(&key) {
                    match node.kind {
                        NodeKind::Dir => removed.directories += 1,
                        NodeKind::File => removed.files += 1,
                    }
                    removed.bytes += node.size;
                    self.release(&mut state, node.size);
                }
            }
            state.tree_parallelism.push(parallelism);
            state
                .log
                .push(format!("remove_tree {}", path.as_path().display()));
            drop(state);
            for _ in 0..removed.files {
                self.charge_file();
            }
            Ok(removed)
        }

        fn create_dir(&self, _scope: &TxScope, path: &AbsPath, _mode: u32) -> FsResult<()> {
            let mut state = self.state();
            if state.nodes.contains_key(path.as_path()) {
                return Err(FsError::new(
                    FsErrorKind::Exists,
                    "create_dir",
                    Some(path.as_path()),
                ));
            }
            state.nodes.insert(
                path.as_path().to_path_buf(),
                Node {
                    kind: NodeKind::Dir,
                    size: 0,
                },
            );
            Ok(())
        }

        fn create_link(
            &self,
            _scope: &TxScope,
            _target: &Path,
            _link: &AbsPath,
            _kind: LinkKind,
        ) -> FsResult<()> {
            Err(unsupported("create_link"))
        }

        fn set_meta(
            &self,
            _scope: &TxScope,
            _path: &AbsPath,
            _change: &MetaChange,
        ) -> FsResult<()> {
            Err(unsupported("set_meta"))
        }

        fn restore_meta(
            &self,
            _scope: &TxScope,
            _path: &AbsPath,
            _meta: &ExtendedMeta,
        ) -> FsResult<MetaTransferReport> {
            Err(unsupported("restore_meta"))
        }

        fn truncate(&self, _scope: &TxScope, _path: &AbsPath, _length: u64) -> FsResult<()> {
            Err(unsupported("truncate"))
        }

        fn sync_dir(&self, _scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
            self.state()
                .log
                .push(format!("sync_dir {}", dir.as_path().display()));
            Ok(())
        }

        fn lock_exclusive(
            &self,
            _path: &AbsPath,
            _timeout: Duration,
        ) -> FsResult<Box<dyn FileLock>> {
            Err(unsupported("lock_exclusive"))
        }

        fn mark_hidden(&self, _scope: &TxScope, _path: &AbsPath) -> FsResult<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::config::SpaceConfig;
    use nurt_contracts::fs::SpaceInfo;
    use nurt_contracts::journal::VolumeState;

    use super::testing::{TestVolume, path, volume_id};
    use super::*;

    fn space(total: u64, available: u64, quota_remaining: Option<u64>) -> SpaceInfo {
        SpaceInfo {
            total,
            available,
            quota_remaining,
            quota_limit: None,
            purgeable_system: Some(1_000),
        }
    }

    fn quota_space(total: u64, available: u64, remaining: u64, limit: u64) -> SpaceInfo {
        SpaceInfo {
            quota_limit: Some(limit),
            ..space(total, available, Some(remaining))
        }
    }

    #[test]
    fn free_space_is_available_bytes_without_quota() {
        let reading = Measurement::from_space(&space(1_000, 400, None), true);
        assert_eq!(reading.free, 400);
        assert_eq!(reading.total, 1_000);
        assert!(!reading.quota_limited);
    }

    #[test]
    fn quota_bounds_free_space_when_quota_aware() {
        let reading = Measurement::from_space(&space(1_000, 400, Some(150)), true);
        assert_eq!(reading.free, 150);
        assert!(reading.quota_limited);
        let larger_quota = Measurement::from_space(&space(1_000, 400, Some(900)), true);
        assert_eq!(larger_quota.free, 400);
    }

    #[test]
    fn quota_size_bounds_capacity_when_quota_aware() {
        let reading = Measurement::from_space(&quota_space(1_000_000, 4_000, 150, 1_000), true);
        assert_eq!(reading.total, 1_000);
        assert_eq!(reading.free, 150);
        assert!(reading.quota_limited);
        let larger_quota = Measurement::from_space(&quota_space(1_000, 400, 300, 1_000_000), true);
        assert_eq!(larger_quota.total, 1_000);
        let unaware = Measurement::from_space(&quota_space(1_000_000, 4_000, 150, 1_000), false);
        assert_eq!(unaware.total, 1_000_000);
        assert_eq!(unaware.free, 4_000);
        assert!(!unaware.quota_limited);
    }

    #[test]
    fn quota_size_alone_marks_the_account_quota_limited() {
        let only_limit = SpaceInfo {
            quota_limit: Some(500),
            ..space(1_000, 400, None)
        };
        let reading = Measurement::from_space(&only_limit, true);
        assert_eq!(reading.total, 500);
        assert_eq!(reading.free, 400);
        assert!(reading.quota_limited);
    }

    #[test]
    fn quota_is_ignored_when_not_quota_aware() {
        let reading = Measurement::from_space(&space(1_000, 400, Some(150)), false);
        assert_eq!(reading.free, 400);
        assert!(!reading.quota_limited);
    }

    #[test]
    fn purgeable_space_is_not_counted_as_free() {
        let reading = Measurement::from_space(&space(1_000, 0, None), true);
        assert_eq!(reading.free, 0);
    }

    #[test]
    fn measure_reads_the_volume() {
        let volume = TestVolume::new(1_000);
        volume.set_foreign(300);
        volume.set_quota_remaining(Some(500));
        volume.set_quota_limit(Some(800));
        let reading = measure(&volume, &volume_id(), true).expect("measure");
        assert_eq!(reading.free, 500);
        assert_eq!(reading.total, 800);
        let reading = measure(&volume, &volume_id(), false).expect("measure");
        assert_eq!(reading.free, 700);
        volume.fail_space(true);
        assert!(measure(&volume, &volume_id(), true).is_err());
    }

    #[test]
    fn poll_period_is_fast_at_or_below_soft_threshold() {
        let config = SpaceConfig::default();
        assert_eq!(
            poll_period(VolumeState::Normal, &config),
            Duration::from_millis(250)
        );
        for state in [VolumeState::Soft, VolumeState::Hard, VolumeState::Emergency] {
            assert_eq!(poll_period(state, &config), Duration::from_millis(50));
        }
    }

    #[test]
    fn poll_period_never_falls_below_the_minimum() {
        let mut config = SpaceConfig::default();
        config.poll_ms = 0;
        config.poll_fast_ms = 3;
        assert_eq!(poll_period(VolumeState::Normal, &config), MIN_POLL_PERIOD);
        assert_eq!(poll_period(VolumeState::Soft, &config), MIN_POLL_PERIOD);
        config.poll_fast_ms = 40;
        assert_eq!(
            poll_period(VolumeState::Hard, &config),
            Duration::from_millis(40)
        );
    }

    #[test]
    fn extreme_figures_of_the_file_system_do_not_overflow() {
        let reading = Measurement::from_space(&space(u64::MAX, u64::MAX, Some(0)), true);
        assert_eq!(reading.free, 0);
        assert_eq!(reading.total, u64::MAX);
        let empty = Measurement::from_space(&space(0, 0, None), true);
        assert_eq!((empty.free, empty.total), (0, 0));
    }

    #[test]
    fn settle_waits_for_delayed_release() {
        let volume = TestVolume::new(1_000);
        volume.set_release_delay(Duration::from_millis(300));
        volume.put_file(&path("vol/a"), 400);
        let scope = crate::reclaim::reclaim_scope(nurt_contracts::journal::OpId(1));
        nurt_contracts::fs::FsWrite::remove_file(&volume, &scope, &path("vol/a")).expect("remove");
        assert_eq!(volume.free(), 600);
        let reading = settle(
            &volume,
            volume.clock.as_ref(),
            &volume_id(),
            true,
            1_000,
            Duration::from_secs(1),
            Duration::from_millis(50),
        )
        .expect("settle");
        assert_eq!(reading.free, 1_000);
        assert!(volume.clock.monotonic() >= Duration::from_millis(300));
        assert!(volume.clock.monotonic() < Duration::from_secs(1));
    }

    #[test]
    fn settle_gives_up_after_window() {
        let volume = TestVolume::new(1_000);
        volume.set_foreign(500);
        let reading = settle(
            &volume,
            volume.clock.as_ref(),
            &volume_id(),
            true,
            900,
            Duration::from_millis(200),
            Duration::from_millis(50),
        )
        .expect("settle");
        assert_eq!(reading.free, 500);
        assert_eq!(volume.clock.monotonic(), Duration::from_millis(200));
    }

    #[test]
    fn settle_reports_measurement_errors() {
        let volume = TestVolume::new(1_000);
        volume.fail_space(true);
        let result = settle(
            &volume,
            volume.clock.as_ref(),
            &volume_id(),
            true,
            1,
            Duration::from_millis(200),
            Duration::from_millis(50),
        );
        assert!(result.is_err());
    }
}
