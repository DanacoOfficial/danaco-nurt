// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/harness/fixture.rs
// Responsibility:   Fixture of the journal under test: volumes, the journal bound to them, a read view of the file system and a virtual clock.
// Dependencies:     nurt_contracts, nurt_fs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fixture of the journal under test: volumes, the journal bound to them, a
//! read view of the file system and a virtual clock.

use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::config::{ConfigLevel, JournalConfig, SpaceConfig};
use nurt_contracts::fs::{AbsPath, Fs, FsWrite};
use nurt_contracts::journal::{
    Expect, HistoryQuery, JournalState, JournalStore, Mutation, OpId, OpState, OperationRecord,
    TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_fs::audit::AuditFs;

use super::clock::VirtualClock;
use super::services::{self, ValueProvider};
use super::snapshot::TreeSnapshot;
use super::volumes::Volumes;

/// File name of the retention audit log in the state directory.
const AUDIT_LOG: &str = "journal-audit.jsonl";

/// Bytes in one gibibyte.
pub(crate) const GIB: u64 = 1 << 30;

/// Bytes in one gigabyte, as volume and file sizes are stated.
pub(crate) const GB: u64 = 1_000_000_000;

/// Volume simulated by the fixture.
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
    /// Value as JSON, or as text when it is not valid JSON (`40m`).
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
    /// Simulated volumes; the first one is the working volume.
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
    /// Seed of the fault injection: when set, the volumes are simulated with
    /// power loss and the journal hits seeded failpoints; a crash ends the
    /// current call with an error, and [`Fixture::restart`] discards the
    /// state that was not durable.
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

/// File system operations on user data counted since the journal instance
/// was bound; operations inside the journal and state directories are not
/// counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OpCounters {
    /// Renames of user data.
    pub(crate) renames: u64,
    /// Bytes copied by the file system, journal copies included; `None` on
    /// mounted volumes, whose file system does not count them.
    pub(crate) bytes_copied: Option<u64>,
    /// Removals of user data.
    pub(crate) removals: u64,
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
    pub(crate) journal: Arc<dyn JournalStore>,
    /// Read view of the volumes.
    pub(crate) fs: Arc<dyn Fs>,
    /// Virtual clock of the journal and of the simulated volumes.
    pub(crate) clock: Arc<VirtualClock>,
    audit: Arc<AuditFs<dyn FsWrite>>,
    values: Arc<ValueProvider>,
    options: FixtureOptions,
    volumes: Volumes,
}

impl Fixture {
    /// Creates the volumes and binds a journal to them.
    pub(crate) fn open(options: FixtureOptions) -> Self {
        let clock = Arc::new(VirtualClock::new());
        let volumes = Volumes::open(&options, clock.clone());
        let values = Arc::new(ValueProvider::new());
        let bound = services::bind(&options, &volumes, clock.clone(), values.clone(), true);
        Self {
            journal: bound.journal,
            fs: volumes.fs(),
            clock,
            audit: bound.audit,
            values,
            options,
            volumes,
        }
    }

    /// Opens a fixture with default options.
    pub(crate) fn standard() -> Self {
        Self::open(FixtureOptions::default())
    }

    /// Replaces the journal by a new instance over the same volumes, as a new
    /// process after a restart; the clock keeps its state. Volumes simulated
    /// with power loss lose the state that was not durable.
    pub(crate) fn restart(&mut self) {
        self.volumes.power_loss();
        let bound = services::bind(
            &self.options,
            &self.volumes,
            self.clock.clone(),
            self.values.clone(),
            false,
        );
        self.journal = bound.journal;
        self.audit = bound.audit;
    }

    /// File system operations on user data counted so far.
    pub(crate) fn counters(&self) -> OpCounters {
        let (renames, removals) = services::user_data_calls(&self.audit);
        OpCounters {
            renames,
            bytes_copied: self.volumes.bytes_copied(),
            removals,
        }
    }

    /// Records one small change, so that the journal of the working volume
    /// exists: the journal reports a volume only after its first use.
    pub(crate) fn use_journal(&self) -> OpId {
        let path = self.seed_file("work/first-use.txt", b"before\n");
        let mut tx = self
            .journal
            .begin(super::request::begin("write", Some("overwrite")));
        tx.preserve(&[TargetSpec {
            path: TargetPath::Fs(path),
            action: TargetAction::Modify,
            expect: Expect::Unchecked,
            size_hint: None,
        }])
        .expect("target is preserved");
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"after\n".to_vec()))
            .expect("change is applied");
        tx.commit()
            .expect("transaction commits")
            .op
            .expect("a committed change has a number")
    }

    /// Everything the channel produced since the fixture was opened, as JSON.
    ///
    /// # Panics
    ///
    /// When the journal or the state directory cannot be read.
    pub(crate) fn channel(&self, channel: Channel) -> serde_json::Value {
        match channel {
            Channel::Events => self.events(),
            Channel::AuditExport => self.audit_export(),
            Channel::CInterfaceHistory => self.c_interface_history(),
        }
    }

    /// Events of the space service: its report of every volume, and for every
    /// expired entry the answer the journal gives about it.
    fn events(&self) -> serde_json::Value {
        let space = self.journal.space(None).expect("space is reported");
        let expired: Vec<serde_json::Value> = self
            .history()
            .iter()
            .filter(|record| record.state == OpState::Expired)
            .map(|record| match self.journal.details(OpId(record.id)) {
                Ok(details) => serde_json::json!({ "op": record.id, "details": details }),
                Err(error) => serde_json::json!({ "op": record.id, "error": error.to_string() }),
            })
            .collect();
        serde_json::json!({ "space": space, "expired": expired })
    }

    /// Audit log of retention in the state directory, one JSON value per
    /// line; an absent log is an empty export.
    fn audit_export(&self) -> serde_json::Value {
        let path = self
            .volumes
            .state_dir()
            .join_name(AUDIT_LOG)
            .expect("audit log name is a single component");
        if !self.exists(&path) {
            return serde_json::Value::Array(Vec::new());
        }
        let text = String::from_utf8(self.read(&path)).expect("audit log is text");
        serde_json::Value::Array(
            text.lines()
                .map(|line| serde_json::from_str(line).expect("audit record is JSON"))
                .collect(),
        )
    }

    /// History as the C interface returns it: the object `operations` with
    /// the records the journal lists for the default query.
    fn c_interface_history(&self) -> serde_json::Value {
        serde_json::json!({ "operations": self.history() })
    }

    /// Root of volume `index`.
    pub(crate) fn volume(&self, index: usize) -> &AbsPath {
        &self.volumes.roots()[index]
    }

    /// Path below the root of the working volume; `relative` uses `/`.
    pub(crate) fn path(&self, relative: &str) -> AbsPath {
        path_below(self.volume(0), relative)
    }

    /// Writes a file outside the journal, creating parent directories; used to
    /// prepare trees before an operation.
    pub(crate) fn seed_file(&self, relative: &str, content: &[u8]) -> AbsPath {
        let path = self.path(relative);
        self.volumes.write(&path, content);
        self.volumes.seeded(&path);
        path
    }

    /// Creates a sparse file of `length` bytes outside the journal; large
    /// sizes take no space on the volume.
    pub(crate) fn seed_sparse(&self, relative: &str, length: u64) -> AbsPath {
        self.seed_sparse_on(0, relative, length)
    }

    /// Creates a sparse file of `length` bytes below the root of volume
    /// `index`.
    pub(crate) fn seed_sparse_on(&self, index: usize, relative: &str, length: u64) -> AbsPath {
        let path = path_below(self.volume(index), relative);
        self.volumes.write_sparse(&path, length);
        self.volumes.seeded(&path);
        path
    }

    /// Creates a directory with `count` small files outside the journal.
    pub(crate) fn seed_tree(&self, relative: &str, count: usize) -> AbsPath {
        let dir = self.path(relative);
        self.volumes.create_dir_all(&dir);
        for index in 0..count {
            let file = dir
                .join_name(&format!("f{index:07}"))
                .expect("file name is a single component");
            self.volumes.write(&file, &index.to_le_bytes());
        }
        self.volumes.seeded(&dir);
        dir
    }

    /// Replaces the content of a file as another program does, without the
    /// journal.
    pub(crate) fn write_outside(&self, path: &AbsPath, content: &[u8]) {
        self.volumes.write(path, content);
    }

    /// Adds the name `new` to the file `existing` without the journal.
    pub(crate) fn hard_link(&self, existing: &AbsPath, new: &AbsPath) {
        self.volumes.hard_link(existing, new);
    }

    /// Removes the name of a file without the journal.
    pub(crate) fn remove_outside(&self, path: &AbsPath) {
        self.volumes.remove_file(path);
    }

    /// Whole content of a file.
    pub(crate) fn read(&self, path: &AbsPath) -> Vec<u8> {
        self.volumes.read(path)
    }

    /// Whether an entry exists at `path`, without following links.
    pub(crate) fn exists(&self, path: &AbsPath) -> bool {
        self.volumes.exists(path)
    }

    /// Snapshot of the working volume without the journal directory.
    pub(crate) fn snapshot(&self) -> TreeSnapshot {
        self.snapshot_of(0)
    }

    /// Snapshot of volume `index` without the journal directory.
    pub(crate) fn snapshot_of(&self, index: usize) -> TreeSnapshot {
        let journal_dir = services::journal_dir_name(&self.options.journal);
        TreeSnapshot::take(
            self.fs.as_ref(),
            self.volume(index),
            &[journal_dir.as_str()],
        )
    }

    /// Every entry of `history`, newest first.
    pub(crate) fn history(&self) -> Vec<OperationRecord> {
        self.journal
            .history(&HistoryQuery {
                limit: Some(u32::MAX),
                ..HistoryQuery::default()
            })
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

    /// Status of volume `index`, as `journal.status` reports it.
    pub(crate) fn status_of(&self, index: usize) -> JournalState {
        let mut states = self
            .journal
            .status(Some(self.volume(index)))
            .expect("journal status is readable");
        assert_eq!(states.len(), 1, "status of one volume has one record");
        states.remove(0)
    }

    /// Status of the working volume, as `journal.status` reports it.
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
