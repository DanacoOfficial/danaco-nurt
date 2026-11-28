// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::track::watch_events
// Responsibility:   File system events (inotify, FSEvents, `ReadDirectoryChangesW`) that narrow the scan after a run to the touched paths.
// Dependencies:     notify, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! File system events (inotify, FSEvents, `ReadDirectoryChangesW`) that
//! narrow the scan after a run to the touched paths.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use nurt_contracts::fs::AbsPath;

/// Time without new events after which delivery is considered complete.
const SETTLE_QUIET: Duration = Duration::from_millis(50);

/// Longest wait for event delivery to settle.
const SETTLE_MAX: Duration = Duration::from_millis(500);

/// Longest wait for the barrier file to be reported. FSEvents hands events
/// over after a delay that grows on a loaded machine; when the barrier does
/// not arrive in this time the events are treated as lost and the run is
/// scanned, so a slow delivery costs time and never a change.
const BARRIER_MAX: Duration = Duration::from_secs(3);

/// Interval at which the log is checked for the barrier file.
const BARRIER_POLL: Duration = Duration::from_millis(2);

/// Prefix of the name of the barrier directory.
const BARRIER_PREFIX: &str = "nurt-track-barrier";

/// Distinguishes the barrier directories of one process.
static BARRIER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Events received so far.
#[derive(Debug, Default)]
struct EventLog {
    touched: BTreeSet<PathBuf>,
    received: u64,
    lost: bool,
    /// Name of the barrier file; events of this path are not changes.
    barrier: OsString,
    /// Whether an event of the barrier file arrived.
    barrier_seen: bool,
}

impl EventLog {
    fn record(&mut self, event: notify::Result<notify::Event>) {
        self.received += 1;
        match event {
            Ok(event) if event.need_rescan() => self.lost = true,
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
            Ok(event) => {
                for path in event.paths {
                    if path.file_name() == Some(self.barrier.as_os_str()) {
                        self.barrier_seen = true;
                    } else {
                        self.touched.insert(path);
                    }
                }
            }
            Err(_) => self.lost = true,
        }
    }
}

/// Creates `path`, readable by the owner only where the system has modes.
#[cfg(unix)]
fn create_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

/// Creates `path`; the temporary directory of an account is private.
#[cfg(not(unix))]
fn create_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

/// Private directory in which the barrier file is written; removed with the
/// value.
#[derive(Debug)]
struct BarrierDir {
    path: PathBuf,
    file: OsString,
}

impl BarrierDir {
    /// Creates a new directory below the temporary directory of the system.
    fn create() -> Option<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |after| after.as_nanos());
        let unique = format!(
            "{BARRIER_PREFIX}-{}-{}-{nanos}",
            std::process::id(),
            BARRIER_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        );
        let path = std::env::temp_dir().join(&unique);
        create_private_dir(&path).ok()?;
        Some(Self {
            path,
            file: OsString::from(format!("{unique}.mark")),
        })
    }

    /// Writes the barrier file, which the watcher reports after every event
    /// that preceded the write.
    fn mark(&self) -> bool {
        fs::write(self.path.join(&self.file), b"barrier").is_ok()
    }
}

impl Drop for BarrierDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Watcher of the watched directories during a run.
pub(super) struct EventWatch {
    watcher: RecommendedWatcher,
    log: Arc<Mutex<EventLog>>,
    barrier: BarrierDir,
}

impl std::fmt::Debug for EventWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventWatch").finish_non_exhaustive()
    }
}

impl EventWatch {
    /// Starts watching every root recursively; `None` when the system
    /// refuses a watch (for example the inotify watch limit), in which case
    /// the run is scanned.
    ///
    /// A private directory is watched as well: the file written to it when
    /// the run ends is reported after the events that came earlier, which
    /// tells [`EventWatch::finish`] that delivery is complete.
    pub(super) fn start(roots: &[AbsPath]) -> Option<Self> {
        let barrier = BarrierDir::create()?;
        let log = Arc::new(Mutex::new(EventLog {
            barrier: barrier.file.clone(),
            ..EventLog::default()
        }));
        let sink = Arc::clone(&log);
        let mut watcher = notify::recommended_watcher(move |event| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .record(event);
        })
        .ok()?;
        for root in roots {
            watcher
                .watch(root.as_path(), RecursiveMode::Recursive)
                .ok()?;
        }
        if !roots.iter().any(|root| barrier.path.starts_with(root)) {
            watcher
                .watch(&barrier.path, RecursiveMode::NonRecursive)
                .ok()?;
        }
        Some(Self {
            watcher,
            log,
            barrier,
        })
    }

    /// Marks events as lost, as an overflow of the event queue does.
    #[cfg(test)]
    #[cfg(unix)]
    pub(super) fn lose_events(&self) {
        self.lock().lost = true;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EventLog> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the barrier file and waits until it is reported; `false` when
    /// it cannot be written or does not arrive in [`BARRIER_MAX`].
    fn wait_for_barrier(&self) -> bool {
        if !self.barrier.mark() {
            return false;
        }
        let started = Instant::now();
        while !self.lock().barrier_seen {
            if started.elapsed() >= BARRIER_MAX {
                return false;
            }
            thread::sleep(BARRIER_POLL);
        }
        true
    }

    /// Waits until event delivery is complete, stops watching and returns
    /// the touched paths; `None` when events were lost or their delivery
    /// could not be confirmed and a full scan is needed.
    pub(super) fn finish(self) -> Option<BTreeSet<PathBuf>> {
        let confirmed = self.wait_for_barrier();
        let started = Instant::now();
        let mut seen = self.lock().received;
        loop {
            thread::sleep(SETTLE_QUIET);
            let received = self.lock().received;
            if received == seen || started.elapsed() >= SETTLE_MAX {
                break;
            }
            seen = received;
        }
        drop(self.watcher);
        let log = std::mem::take(&mut *self.log.lock().unwrap_or_else(PoisonError::into_inner));
        (confirmed && !log.lost).then_some(log.touched)
    }

    /// Directory of the barrier file.
    #[cfg(test)]
    fn barrier_dir(&self) -> &std::path::Path {
        &self.barrier.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(dir: &tempfile::TempDir) -> AbsPath {
        AbsPath::new(dir.path().to_owned()).expect("an absolute path")
    }

    #[test]
    fn delivery_is_confirmed_and_the_barrier_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let watch = EventWatch::start(&[root(&dir)]).expect("the watch starts");
        let barrier = watch.barrier_dir().to_owned();
        assert!(barrier.is_dir());
        fs::write(dir.path().join("changed.txt"), "x").unwrap();
        let touched = watch.finish().expect("events are delivered");
        assert!(touched.iter().any(|path| path.ends_with("changed.txt")));
        assert!(
            !touched
                .iter()
                .any(|path| path.to_string_lossy().contains(BARRIER_PREFIX))
        );
        assert!(!barrier.exists());
    }

    #[test]
    fn the_barrier_directory_is_removed_when_the_watch_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let watch = EventWatch::start(&[root(&dir)]).expect("the watch starts");
        let barrier = watch.barrier_dir().to_owned();
        drop(watch);
        assert!(!barrier.exists());
    }

    #[test]
    fn events_without_the_barrier_are_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let watch = EventWatch::start(&[root(&dir)]).expect("the watch starts");
        fs::remove_dir_all(watch.barrier_dir()).unwrap();
        assert_eq!(watch.finish(), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
