// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixture of the journal under test: volume directories, the journal bound
//! to them, a read view of the file system and a virtual clock.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::config::{ConfigLevel, JournalConfig, SpaceConfig};
use nurt_contracts::env::Clock;
use nurt_contracts::fs::{AbsPath, Fs};
use nurt_contracts::journal::{HistoryQuery, JournalState, JournalStore, OpId, OperationRecord};
use tempfile::TempDir;

use super::clock::VirtualClock;
use super::snapshot::TreeSnapshot;

/// Bytes in one gibibyte.
pub(crate) const GIB: u64 = 1 << 30;

/// Bytes in one gigabyte, as volume and file sizes are stated.
pub(crate) const GB: u64 = 1_000_000_000;

/// Volume simulated by the file system view of the fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VolumeSpec {
    /// Capacity reported for the volume.
    pub(crate) capacity: u64,
    /// Bytes used by other data, outside the journal.
    pub(crate) used: u64,
}

impl VolumeSpec {
    /// Volume of 500 GB with 100 GB used, the size of the retention
    /// scenarios.
    pub(crate) const STANDARD: Self = Self {
        capacity: 500 * GB,
        used: 100 * GB,
    };
}

/// Journal setting given at one configuration level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setting {
    /// Level of the setting.
    pub(crate) level: ConfigLevel,
    /// Key, for example `journal.max_ops`.
    pub(crate) key: &'static str,
    /// Value in the configuration file syntax.
    pub(crate) value: String,
}

/// Options of a fixture.
#[derive(Debug, Clone)]
pub(crate) struct FixtureOptions {
    /// Journal settings of the user level. Values equal to the built-in
    /// defaults are not written, so the built-in level stays their source.
    pub(crate) journal: JournalConfig,
    /// Space settings.
    pub(crate) space: SpaceConfig,
    /// Simulated volumes, one directory each; the first one is the working
    /// volume.
    pub(crate) volumes: Vec<VolumeSpec>,
    /// Mounted directories used as the volumes instead of simulated ones; the
    /// fixture works in a new temporary directory inside each of them.
    pub(crate) mounted: Vec<PathBuf>,
    /// The space service runs.
    pub(crate) service_alive: bool,
    /// Settings of other levels, applied over `journal`.
    pub(crate) settings: Vec<Setting>,
    /// The working volume is an overlay mount and entries seeded before the
    /// first operation lie in its lower layer.
    pub(crate) lower_layer: bool,
    /// The project of the working volume is trusted by the user
    /// configuration; settings of the project level apply only then.
    pub(crate) project_trusted: bool,
    /// Seed of the fault injection: when set, the file system runs through
    /// the crash layer and the journal hits seeded failpoints; a crash point
    /// ends the current call with an error and discards unsynchronised
    /// state.
    pub(crate) crash_seed: Option<u64>,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            journal: JournalConfig::default(),
            space: SpaceConfig::default(),
            volumes: vec![VolumeSpec::STANDARD],
            mounted: Vec::new(),
            service_alive: true,
            settings: Vec::new(),
            lower_layer: false,
            project_trusted: true,
            crash_seed: None,
        }
    }
}

/// File system operations counted since the fixture was opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OpCounters {
    /// Renames and exchanges of names.
    pub(crate) renames: u64,
    /// Bytes copied by copy operations.
    pub(crate) bytes_copied: u64,
    /// Entries removed.
    pub(crate) removals: u64,
}

/// Journal under test with the read view of its file system.
pub(crate) struct Bound {
    /// Journal store.
    pub(crate) journal: Box<dyn JournalStore>,
    /// Read view of the volumes, with the simulated capacity.
    pub(crate) fs: Arc<dyn Fs>,
    /// Counters of the file system layer below the journal.
    pub(crate) counters: Box<dyn Fn() -> OpCounters + Send + Sync>,
}

/// Binds a new journal instance to the volume directories.
///
/// The binding builds the journal store over the platform file system of the
/// directories. The file system is wrapped by the mutation audit when the
/// feature `mutation-audit` of the file system crate is enabled. Each
/// simulated volume reports its capacity. The space service runs when the
/// options say so. A writable provider with the scheme `test` serves value
/// targets. Settings of the options apply at their levels.
///
/// # Panics
///
/// Always. The journal crate exposes no public constructor of a journal
/// store, and every test that needs one is ignored.
fn bind(_options: &FixtureOptions, _volumes: &[AbsPath], _clock: Arc<dyn Clock>) -> Bound {
    panic!("no journal store implementation is bound to the test harness")
}

/// Result channel produced outside the journal store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channel {
    /// Events of the space service, including expired entries.
    Events,
    /// Audit export of the journal.
    AuditExport,
    /// History returned by the C interface.
    CInterfaceHistory,
}

impl Channel {
    /// Name used in messages.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::AuditExport => "audit export",
            Self::CInterfaceHistory => "C interface history",
        }
    }
}

/// Volumes with the journal under test.
pub(crate) struct Fixture {
    /// Journal store.
    pub(crate) journal: Box<dyn JournalStore>,
    /// Read view of the volumes.
    pub(crate) fs: Arc<dyn Fs>,
    /// Virtual clock of the journal.
    pub(crate) clock: Arc<VirtualClock>,
    counters: Box<dyn Fn() -> OpCounters + Send + Sync>,
    options: FixtureOptions,
    volumes: Vec<AbsPath>,
    _dirs: Vec<TempDir>,
}

impl Fixture {
    /// Creates the volume directories and binds a journal to them.
    pub(crate) fn open(options: FixtureOptions) -> Self {
        let dirs: Vec<TempDir> = if options.mounted.is_empty() {
            options
                .volumes
                .iter()
                .map(|_| tempfile::tempdir().expect("temporary directory is created"))
                .collect()
        } else {
            options
                .mounted
                .iter()
                .map(|mount| {
                    tempfile::tempdir_in(mount).expect("directory is created on the volume")
                })
                .collect()
        };
        let volumes: Vec<AbsPath> = dirs
            .iter()
            .map(|dir| AbsPath::new(dir.path()).expect("temporary directory is absolute"))
            .collect();
        let clock = Arc::new(VirtualClock::new());
        let bound = bind(&options, &volumes, clock.clone());
        Self {
            journal: bound.journal,
            fs: bound.fs,
            clock,
            counters: bound.counters,
            options,
            volumes,
            _dirs: dirs,
        }
    }

    /// Opens a fixture with default options.
    pub(crate) fn standard() -> Self {
        Self::open(FixtureOptions::default())
    }

    /// Replaces the journal by a new instance over the same volumes, as a new
    /// process after a restart; the clock keeps its state.
    pub(crate) fn restart(&mut self) {
        let bound = bind(&self.options, &self.volumes, self.clock.clone());
        self.journal = bound.journal;
        self.fs = bound.fs;
        self.counters = bound.counters;
    }

    /// File system operations counted so far.
    pub(crate) fn counters(&self) -> OpCounters {
        (self.counters)()
    }

    /// Everything the channel produced since the fixture was opened, as JSON.
    ///
    /// # Panics
    ///
    /// Always. The channels are produced by components outside the journal
    /// crate, and every test that reads one is ignored.
    pub(crate) fn channel(&self, channel: Channel) -> serde_json::Value {
        panic!(
            "the {} channel is not bound to the test harness",
            channel.name()
        )
    }

    /// Root of volume `index`.
    pub(crate) fn volume(&self, index: usize) -> &AbsPath {
        &self.volumes[index]
    }

    /// Path below the root of the working volume; `relative` uses `/`.
    pub(crate) fn path(&self, relative: &str) -> AbsPath {
        path_below(self.volume(0), relative)
    }

    /// Writes a file outside the journal, creating parent directories; used to
    /// prepare trees before an operation.
    pub(crate) fn seed_file(&self, relative: &str, content: &[u8]) -> AbsPath {
        let path = self.path(relative);
        create_parent(path.as_path());
        fs::write(path.as_path(), content).expect("seed file is written");
        path
    }

    /// Creates a sparse file of `length` bytes outside the journal; large
    /// sizes take no space on the volume.
    pub(crate) fn seed_sparse(&self, relative: &str, length: u64) -> AbsPath {
        let path = self.path(relative);
        create_parent(path.as_path());
        let file = fs::File::create(path.as_path()).expect("sparse file is created");
        file.set_len(length).expect("sparse file length is set");
        path
    }

    /// Creates a directory with `count` small files outside the journal.
    pub(crate) fn seed_tree(&self, relative: &str, count: usize) -> AbsPath {
        let dir = self.path(relative);
        fs::create_dir_all(dir.as_path()).expect("seed directory is created");
        for index in 0..count {
            fs::write(
                dir.as_path().join(format!("f{index:07}")),
                index.to_le_bytes(),
            )
            .expect("seed file is written");
        }
        dir
    }

    /// Snapshot of the working volume without the journal directory.
    pub(crate) fn snapshot(&self) -> TreeSnapshot {
        self.snapshot_of(0)
    }

    /// Snapshot of volume `index` without the journal directory.
    pub(crate) fn snapshot_of(&self, index: usize) -> TreeSnapshot {
        let journal_dir = self.options.journal.dir_name.as_str();
        TreeSnapshot::take(self.fs.as_ref(), self.volume(index), &[journal_dir])
    }

    /// Every entry of `history`, newest first.
    pub(crate) fn history(&self) -> Vec<OperationRecord> {
        self.journal
            .history(&HistoryQuery::default())
            .expect("history is readable")
    }

    /// The `history` record of one entry.
    pub(crate) fn record(&self, op: OpId) -> OperationRecord {
        self.journal
            .history(&HistoryQuery {
                id: Some(op),
                ..HistoryQuery::default()
            })
            .expect("history is readable")
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("entry {op:?} is listed by history"))
    }

    /// `journal.status` of volume `index`.
    pub(crate) fn status_of(&self, index: usize) -> JournalState {
        let mut states = self
            .journal
            .status(Some(self.volume(index)))
            .expect("journal status is readable");
        assert_eq!(states.len(), 1, "status of one volume has one record");
        states.remove(0)
    }

    /// `journal.status` of the working volume.
    pub(crate) fn status(&self) -> JournalState {
        self.status_of(0)
    }
}

/// Path below `root`; `relative` uses `/` on every system.
pub(crate) fn path_below(root: &AbsPath, relative: &str) -> AbsPath {
    relative.split('/').fold(root.clone(), |path, name| {
        path.join_name(name).expect("relative path has valid names")
    })
}

/// Directory named by a test environment variable.
///
/// # Panics
///
/// Panics with the name of the variable when it is not set or does not name
/// a directory.
pub(crate) fn mounted_dir(variable: &str) -> PathBuf {
    let dir = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the mount point of the prepared volume"));
    assert!(
        dir.is_dir(),
        "{variable} does not name a directory: {}",
        dir.display()
    );
    dir
}

fn create_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory is created");
    }
}
