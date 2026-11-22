// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File system events (inotify, FSEvents, `ReadDirectoryChangesW`) that
//! narrow the scan after a run to the touched paths.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use nurt_contracts::fs::AbsPath;

/// Time without new events after which delivery is considered complete.
const SETTLE_QUIET: Duration = Duration::from_millis(50);

/// Longest wait for event delivery to settle.
const SETTLE_MAX: Duration = Duration::from_millis(500);

/// Events received so far.
#[derive(Debug, Default)]
struct EventLog {
    touched: BTreeSet<PathBuf>,
    received: u64,
    lost: bool,
}

impl EventLog {
    fn record(&mut self, event: notify::Result<notify::Event>) {
        self.received += 1;
        match event {
            Ok(event) if event.need_rescan() => self.lost = true,
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
            Ok(event) => self.touched.extend(event.paths),
            Err(_) => self.lost = true,
        }
    }
}

/// Watcher of the watched directories during a run.
pub(super) struct EventWatch {
    watcher: RecommendedWatcher,
    log: Arc<Mutex<EventLog>>,
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
    pub(super) fn start(roots: &[AbsPath]) -> Option<Self> {
        let log = Arc::new(Mutex::new(EventLog::default()));
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
        Some(Self { watcher, log })
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

    /// Waits until event delivery settles, stops watching and returns the
    /// touched paths; `None` when events were lost and a full scan is needed.
    pub(super) fn finish(self) -> Option<BTreeSet<PathBuf>> {
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
        (!log.lost).then_some(log.touched)
    }
}
