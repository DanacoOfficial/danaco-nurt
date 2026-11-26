// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Scan of the watched directories: one entry per path with the state that
//! reveals a change (kind, size, modification time, file identity, mode,
//! link target and, on request, the content hash).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs, FsErrorKind};
use nurt_contracts::stage::CancelToken;
use serde::{Deserialize, Serialize};

/// Largest number of threads of one scan.
const MAX_SCAN_THREADS: usize = 8;

/// Number of scanned entries between two checks of the clock and of the
/// cancellation token inside one directory.
const CHECK_INTERVAL: usize = 64;

/// Why a bounded scan ended before it listed everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    /// The execution was cancelled.
    Cancelled,
    /// The time limit of the run ended.
    TimeLimit,
    /// The time budget of automatic detection was used up.
    Budget,
    /// The scope holds more entries than automatic detection accepts.
    Entries,
}

/// Bounds of a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScanLimits {
    /// Largest number of entries; a scan that finds more stops.
    pub(super) max_entries: usize,
    /// Time budget of one scan; `None` is bounded only by the time limit of
    /// the run.
    pub(super) budget: Option<Duration>,
    /// Directories on another volume than their root are listed as entries
    /// but not entered.
    pub(super) one_volume: bool,
}

impl ScanLimits {
    /// Bound only by the time limit of the run and by cancellation.
    pub(super) const UNLIMITED: Self = Self {
        max_entries: usize::MAX,
        budget: None,
        one_volume: false,
    };
}

/// State of the bounds while one scan runs.
pub(super) struct Bound {
    limits: ScanLimits,
    deadline: Option<Instant>,
    budget_end: Option<Instant>,
    cancel: CancelToken,
    entries: AtomicUsize,
    foreign: AtomicU64,
    stop: Mutex<Option<Stop>>,
}

impl fmt::Debug for Bound {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Bound")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Bound {
    /// Bounds of one scan starting now, which ends at `deadline` at the
    /// latest and when `cancel` fires.
    pub(super) fn new(limits: ScanLimits, deadline: Option<Instant>, cancel: CancelToken) -> Self {
        Self {
            limits,
            deadline,
            budget_end: limits
                .budget
                .and_then(|budget| Instant::now().checked_add(budget)),
            cancel,
            entries: AtomicUsize::new(0),
            foreign: AtomicU64::new(0),
            stop: Mutex::new(None),
        }
    }

    /// Reason the scan stopped, if it did.
    pub(super) fn stopped(&self) -> Option<Stop> {
        *self.stop.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Number of directories on other volumes that were not entered.
    pub(super) fn foreign(&self) -> u64 {
        self.foreign.load(Ordering::Relaxed)
    }

    fn halt(&self, reason: Stop) {
        self.stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(reason);
    }

    /// Checks cancellation and time; true when the scan has to stop.
    fn check(&self) -> bool {
        if self.stopped().is_some() {
            return true;
        }
        if self.cancel.is_cancelled() {
            self.halt(Stop::Cancelled);
        } else if self.deadline.is_some_and(|end| Instant::now() >= end) {
            self.halt(Stop::TimeLimit);
        } else if self.budget_end.is_some_and(|end| Instant::now() >= end) {
            self.halt(Stop::Budget);
        }
        self.stopped().is_some()
    }

    /// Counts one entry; true when the scan has to stop.
    fn count(&self) -> bool {
        let count = self.entries.fetch_add(1, Ordering::Relaxed) + 1;
        if count > self.limits.max_entries {
            self.halt(Stop::Entries);
            return true;
        }
        count % CHECK_INTERVAL == 0 && self.check()
    }
}

/// Kind of a scanned entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link or junction; never followed.
    Link,
    /// Device, pipe or socket.
    Special,
}

/// State of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct EntryState {
    /// Kind.
    pub(super) kind: Kind,
    /// Size in bytes.
    pub(super) size: u64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub(super) modified_ns: i64,
    /// File identity: volume, low and high part.
    pub(super) file_id: [u64; 3],
    /// Birth time, nanoseconds since the Unix epoch, when the system
    /// reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) created_ns: Option<i64>,
    /// Permission bits.
    pub(super) mode: u32,
    /// Target of a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) link: Option<PathBuf>,
    /// Content hash of a file, `sha256:…`, when requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hash: Option<String>,
}

/// Entries by absolute path.
pub(super) type Snapshot = BTreeMap<PathBuf, EntryState>;

/// What is scanned.
#[derive(Debug, Clone, Copy)]
pub(super) struct Scope<'a> {
    /// Scanned directories and files.
    pub(super) roots: &'a [AbsPath],
    /// Names excluded at any depth.
    pub(super) ignore: &'a [String],
    /// Content hashes of files are computed.
    pub(super) hash: bool,
    /// Bounds of the scan; `None` scans everything without a limit.
    pub(super) bound: Option<&'a Bound>,
}

impl Scope<'_> {
    /// Whether `path` lies under a root and below an ignored name.
    pub(super) fn is_ignored(&self, path: &Path) -> bool {
        self.root_of(path).is_some_and(|root| {
            path.strip_prefix(root)
                .is_ok_and(|relative| has_name(relative, self.ignore))
        })
    }

    /// Root that contains `path`.
    pub(super) fn root_of(&self, path: &Path) -> Option<&Path> {
        self.roots
            .iter()
            .map(AbsPath::as_path)
            .find(|root| path.starts_with(root))
    }
}

/// Whether a component of `relative` equals one of `names`.
pub(super) fn has_name(relative: &Path, names: &[String]) -> bool {
    relative.components().any(|component| {
        let component = component.as_os_str();
        names.iter().any(|name| component == name.as_str())
    })
}

/// Result of a scan.
#[derive(Debug, Default)]
pub(super) struct Scanned {
    /// Entries found.
    pub(super) entries: Snapshot,
    /// Directories that could not be listed for lack of permission.
    pub(super) denied: u64,
    /// Why the scan ended before it listed everything; `entries` is then
    /// incomplete.
    pub(super) stop: Option<Stop>,
    /// Directories on other volumes that were not entered.
    pub(super) foreign: u64,
}

impl Scanned {
    fn merge(&mut self, other: Self) {
        self.entries.extend(other.entries);
        self.denied += other.denied;
    }
}

/// Reads the state of one entry without following links; `None` when it does
/// not exist or cannot be read.
pub(super) fn stat(fs: &dyn Fs, path: &AbsPath, hash: bool) -> Option<EntryState> {
    let meta = fs.metadata(path, Follow::NoFollow).ok()?;
    let kind = match meta.kind {
        FileKind::File => Kind::File,
        FileKind::Directory => Kind::Directory,
        FileKind::Symlink | FileKind::Junction => Kind::Link,
        _ => Kind::Special,
    };
    let link = (kind == Kind::Link)
        .then(|| fs.read_link(path).ok())
        .flatten();
    let hash = (hash && kind == Kind::File)
        .then(|| fs.content_hash(path).ok().map(|digest| digest.to_string()))
        .flatten();
    Some(EntryState {
        kind,
        size: meta.size,
        modified_ns: meta.modified_ns,
        file_id: [meta.file_id.volume, meta.file_id.low, meta.file_id.high],
        created_ns: meta.created_ns,
        mode: meta.mode,
        link,
        hash,
    })
}

/// Directories waiting to be listed, each with the volume of its root, and
/// the number being listed.
#[derive(Default)]
struct Queue {
    pending: Vec<(AbsPath, u64)>,
    active: usize,
}

struct Work<'a> {
    fs: &'a dyn Fs,
    scope: Scope<'a>,
    queue: Mutex<Queue>,
    ready: Condvar,
}

impl Work<'_> {
    fn next(&self) -> Option<(AbsPath, u64)> {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(dir) = queue.pending.pop() {
                queue.active += 1;
                return Some(dir);
            }
            if queue.active == 0 {
                self.ready.notify_all();
                return None;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn done(&self, found: Vec<(AbsPath, u64)>) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.pending.extend(found);
        queue.active -= 1;
        self.ready.notify_all();
    }

    fn run(&self) -> Scanned {
        let mut scanned = Scanned::default();
        while let Some((dir, volume)) = self.next() {
            let found = self.list(&dir, volume, &mut scanned);
            self.done(found);
        }
        scanned
    }

    /// Lists one directory into `scanned`; returns its subdirectories.
    fn list(&self, dir: &AbsPath, volume: u64, scanned: &mut Scanned) -> Vec<(AbsPath, u64)> {
        let bound = self.scope.bound;
        if bound.is_some_and(Bound::check) {
            return Vec::new();
        }
        let entries = match self.fs.read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                if error.kind == FsErrorKind::PermissionDenied {
                    scanned.denied += 1;
                }
                return Vec::new();
            }
        };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            if self
                .scope
                .ignore
                .iter()
                .any(|name| entry.name == name.as_str())
            {
                continue;
            }
            let Ok(path) = AbsPath::new(dir.as_path().join(&entry.name)) else {
                continue;
            };
            let Some(state) = stat(self.fs, &path, self.scope.hash) else {
                continue;
            };
            if state.kind == Kind::Directory {
                let foreign = bound.is_some_and(|bound| bound.limits.one_volume)
                    && state.file_id[0] != volume;
                if foreign {
                    if let Some(bound) = bound {
                        bound.foreign.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    subdirs.push((path.clone(), state.file_id[0]));
                }
            }
            scanned.entries.insert(path.as_path().to_path_buf(), state);
            if bound.is_some_and(Bound::count) {
                break;
            }
        }
        subdirs
    }
}

/// Scans every root and everything below it, in parallel.
pub(super) fn scan(fs: &dyn Fs, scope: Scope<'_>) -> Scanned {
    let mut scanned = Scanned::default();
    let mut pending = Vec::new();
    for root in scope.roots {
        if let Some(state) = stat(fs, root, scope.hash) {
            if state.kind == Kind::Directory {
                pending.push((root.clone(), state.file_id[0]));
            }
            scanned.entries.insert(root.as_path().to_path_buf(), state);
        }
    }
    let work = Work {
        fs,
        scope,
        queue: Mutex::new(Queue { pending, active: 0 }),
        ready: Condvar::new(),
    };
    let threads = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(MAX_SCAN_THREADS);
    thread::scope(|threads_scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| threads_scope.spawn(|| work.run()))
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(part) => scanned.merge(part),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
    });
    if let Some(bound) = scope.bound {
        scanned.stop = bound.stopped();
        scanned.foreign = bound.foreign();
    }
    scanned
}

/// Removes `path` and every entry below it.
fn remove_tree(snapshot: &mut Snapshot, path: &Path) {
    let below: Vec<PathBuf> = snapshot
        .range(path.to_path_buf()..)
        .map(|(key, _)| key)
        .take_while(|key| key.starts_with(path))
        .cloned()
        .collect();
    for key in below {
        snapshot.remove(&key);
    }
}

/// Updates `snapshot` for the paths reported by file system events: every
/// touched path is read again, and a directory that appeared is scanned
/// completely. Returns the number of touched paths below ignored names.
pub(super) fn refresh(
    fs: &dyn Fs,
    scope: Scope<'_>,
    snapshot: &mut Snapshot,
    touched: &BTreeSet<PathBuf>,
) -> u64 {
    let mut ignored = 0;
    for path in touched {
        if scope.bound.is_some_and(Bound::check) {
            break;
        }
        if scope.root_of(path).is_none() {
            continue;
        }
        if scope.is_ignored(path) {
            ignored += 1;
            continue;
        }
        let Ok(abs) = AbsPath::new(path) else {
            continue;
        };
        let was_dir = snapshot
            .get(path)
            .is_some_and(|state| state.kind == Kind::Directory);
        match stat(fs, &abs, scope.hash) {
            None => remove_tree(snapshot, path),
            Some(state) if state.kind == Kind::Directory && !was_dir => {
                remove_tree(snapshot, path);
                let roots = [abs];
                let subtree = scan(
                    fs,
                    Scope {
                        roots: &roots,
                        ..scope
                    },
                );
                snapshot.extend(subtree.entries);
            }
            Some(state) => {
                if was_dir && state.kind != Kind::Directory {
                    remove_tree(snapshot, path);
                }
                snapshot.insert(path.clone(), state);
            }
        }
    }
    ignored
}
