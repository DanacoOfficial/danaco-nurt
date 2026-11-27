// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/harness/volumes.rs
// Responsibility:   Volumes of a fixture and the changes other programs make to them.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs, tempfile
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Volumes of a fixture and the changes other programs make to them.
//!
//! Volumes are simulated in memory by default. With a crash seed they are
//! simulated with power loss, and the journal hits seeded failpoints. Mounted
//! directories use the platform file system. Every volume gets its own root;
//! the state directory of the account lies outside them, so that snapshots
//! of a volume see user data and the journal directory only.

use std::io::{Read, Write};
use std::sync::Arc;

use nurt_contracts::env::Clock;
use nurt_contracts::fs::{
    AbsPath, Durability, FileKind, Follow, FsWrite, ScopePurpose, TxScope, WriteHandle,
};
use nurt_contracts::journal::OpId;
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec as CrashVolume};
use nurt_fs::failpoint::{FailPlan, RandomFaults, SeededFailpoints};
use nurt_fs::mem::{MemFs, VolumeSpec as MemVolume};
use tempfile::TempDir;

use super::fixture::FixtureOptions;

/// File system type of the simulated volumes: a local journaled one, on
/// which the journal is active.
const LOCAL_FS_TYPE: &str = "ext4";
/// Entries that fit on a volume simulated with power loss.
const CRASH_VOLUME_ENTRIES: u64 = 1 << 22;
/// Chance of a crash at each failpoint hit of the journal, per million.
const CRASH_PER_MILLION: u32 = 20_000;
/// Mode of directories created by other programs.
const DIR_MODE: u32 = 0o755;
/// Directory of the account state below the host root.
const STATE_DIR: &str = "state";

/// Scope of changes made by other programs. The file systems below the
/// journal do not check scopes, and these changes bypass the mutation audit.
#[expect(
    clippy::disallowed_methods,
    reason = "the fixture acts as another program that changes files without the journal"
)]
fn outside() -> TxScope {
    TxScope::new_for_journal(OpId(0), ScopePurpose::Maintenance)
}

fn host_root() -> AbsPath {
    AbsPath::new(if cfg!(windows) { r"C:\" } else { "/" }).expect("host root is absolute")
}

enum Backing {
    Memory(Arc<MemFs>),
    Crash {
        fs: CrashFs,
        failpoints: Arc<SeededFailpoints>,
    },
    Mounted,
}

/// Volumes of a fixture with the file system below the journal.
pub(crate) struct Volumes {
    backing: Backing,
    fs: Arc<dyn FsWrite>,
    roots: Vec<AbsPath>,
    state_dir: AbsPath,
    lower_layer: bool,
    _dirs: Vec<TempDir>,
}

impl Volumes {
    /// Creates the volumes of `options`: mounted directories when the
    /// options name any, volumes simulated with power loss when they carry a
    /// crash seed, in-memory volumes otherwise.
    pub(crate) fn open(options: &FixtureOptions, clock: Arc<dyn Clock>) -> Self {
        if !options.mounted.is_empty() {
            return Self::mounted(options);
        }
        let host = host_root();
        let roots: Vec<AbsPath> = (0..options.volumes.len())
            .map(|index| {
                host.join_name(&format!("volume-{index}"))
                    .expect("volume name is a single component")
            })
            .collect();
        let state_dir = host
            .join_name(STATE_DIR)
            .expect("state directory name is a single component");
        match options.crash_seed {
            Some(seed) => Self::crashing(options, seed, &host, roots, state_dir),
            None => Self::memory(options, clock, &host, roots, state_dir),
        }
    }

    fn memory(
        options: &FixtureOptions,
        clock: Arc<dyn Clock>,
        host: &AbsPath,
        roots: Vec<AbsPath>,
        state_dir: AbsPath,
    ) -> Self {
        let mut builder = MemFs::builder()
            .clock(clock)
            .volume(MemVolume::posix(host.clone()).with_fs_type(LOCAL_FS_TYPE));
        for (index, (root, spec)) in roots.iter().zip(&options.volumes).enumerate() {
            let volume = if index == 0 && options.lower_layer {
                MemVolume::overlay(root.clone())
            } else {
                MemVolume::posix(root.clone()).with_fs_type(LOCAL_FS_TYPE)
            };
            builder = builder.volume(volume.with_capacity(spec.capacity));
        }
        let mem = Arc::new(builder.build().expect("in-memory volumes are created"));
        for (root, spec) in roots.iter().zip(&options.volumes) {
            mem.set_available(root, spec.capacity.saturating_sub(spec.used))
                .expect("free space of the volume is set");
        }
        mem.seed_dir(&state_dir)
            .expect("state directory is created");
        Self {
            fs: mem.clone(),
            backing: Backing::Memory(mem),
            roots,
            state_dir,
            lower_layer: options.lower_layer,
            _dirs: Vec::new(),
        }
    }

    fn crashing(
        options: &FixtureOptions,
        seed: u64,
        host: &AbsPath,
        roots: Vec<AbsPath>,
        state_dir: AbsPath,
    ) -> Self {
        let mut specs = vec![CrashVolume::new(host.clone())];
        specs.extend(roots.iter().zip(&options.volumes).map(|(root, spec)| {
            CrashVolume::new(root.clone()).with_capacity(
                spec.capacity.saturating_sub(spec.used),
                CRASH_VOLUME_ENTRIES,
            )
        }));
        let crash = CrashFs::new(CrashProfile::PosixWorstCase, seed, specs)
            .expect("volumes with power loss are created");
        let failpoints = Arc::new(SeededFailpoints::new(FailPlan {
            seed,
            rules: Vec::new(),
            random: RandomFaults {
                crash_per_million: CRASH_PER_MILLION,
                ..RandomFaults::default()
            },
        }));
        crash.attach_failpoints(failpoints.clone());
        let volumes = Self {
            fs: Arc::new(crash.clone()),
            backing: Backing::Crash {
                fs: crash,
                failpoints,
            },
            roots,
            state_dir,
            lower_layer: false,
            _dirs: Vec::new(),
        };
        volumes.create_dir_all(&volumes.state_dir);
        volumes.settle();
        volumes
    }

    fn mounted(options: &FixtureOptions) -> Self {
        let mut dirs: Vec<TempDir> = options
            .mounted
            .iter()
            .map(|mount| tempfile::tempdir_in(mount).expect("directory is created on the volume"))
            .collect();
        let roots = dirs
            .iter()
            .map(|dir| AbsPath::new(dir.path()).expect("temporary directory is absolute"))
            .collect();
        let state = tempfile::tempdir().expect("state directory is created");
        let state_dir = AbsPath::new(state.path()).expect("temporary directory is absolute");
        dirs.push(state);
        Self {
            backing: Backing::Mounted,
            fs: nurt_fs::system_fs(),
            roots,
            state_dir,
            lower_layer: false,
            _dirs: dirs,
        }
    }

    /// File system below the journal.
    pub(crate) fn fs(&self) -> Arc<dyn FsWrite> {
        self.fs.clone()
    }

    /// Roots of the volumes; the first one is the working volume.
    pub(crate) fn roots(&self) -> &[AbsPath] {
        &self.roots
    }

    /// State directory of the account.
    pub(crate) fn state_dir(&self) -> &AbsPath {
        &self.state_dir
    }

    /// Failpoint registry of a run with power loss, shared by the journal and
    /// the file system.
    pub(crate) fn failpoints(&self) -> Option<Arc<SeededFailpoints>> {
        match &self.backing {
            Backing::Crash { failpoints, .. } => Some(failpoints.clone()),
            Backing::Memory(_) | Backing::Mounted => None,
        }
    }

    /// Bytes copied by the file system, when it counts them.
    pub(crate) fn bytes_copied(&self) -> Option<u64> {
        match &self.backing {
            Backing::Memory(mem) => Some(mem.counters().bytes_copied),
            Backing::Crash { fs, .. } => Some(fs.stats().bytes_copied),
            Backing::Mounted => None,
        }
    }

    /// Cuts the power of volumes simulated with power loss: state that is
    /// not durable is lost, and the failpoints of the interrupted run no
    /// longer apply. Other volumes keep their state.
    pub(crate) fn power_loss(&self) {
        if let Backing::Crash { fs, .. } = &self.backing {
            fs.crash();
        }
    }

    /// Makes changes of other programs durable on volumes simulated with
    /// power loss.
    fn settle(&self) {
        if let Backing::Crash { fs, .. } = &self.backing {
            fs.sync_all();
        }
    }

    /// Moves a seeded entry into the lower layer of the overlay working
    /// volume, when the options ask for one.
    pub(crate) fn seeded(&self, path: &AbsPath) {
        if let (true, Backing::Memory(mem)) = (self.lower_layer, &self.backing) {
            mem.seed_lower(path)
                .expect("seeded entry is moved to the lower layer");
        }
    }

    /// Whether an entry exists at `path`, without following links.
    pub(crate) fn exists(&self, path: &AbsPath) -> bool {
        self.fs.metadata(path, Follow::NoFollow).is_ok()
    }

    /// Creates a directory and its missing parents.
    pub(crate) fn create_dir_all(&self, path: &AbsPath) {
        if let Ok(meta) = self.fs.metadata(path, Follow::NoFollow) {
            assert_eq!(meta.kind, FileKind::Directory, "{path} is not a directory");
            return;
        }
        if let Some(parent) = path.parent() {
            self.create_dir_all(&parent);
        }
        self.fs
            .create_dir(&outside(), path, DIR_MODE)
            .unwrap_or_else(|error| panic!("cannot create {path}: {error}"));
        self.settle();
    }

    fn create_parent(&self, path: &AbsPath) {
        if let Some(parent) = path.parent() {
            self.create_dir_all(&parent);
        }
    }

    fn open_for_write(&self, path: &AbsPath) -> Box<dyn WriteHandle> {
        self.create_parent(path);
        let scope = outside();
        if self.exists(path) {
            let mut handle = self
                .fs
                .open_in_place(&scope, path)
                .unwrap_or_else(|error| panic!("cannot open {path}: {error}"));
            handle
                .set_length(0)
                .unwrap_or_else(|error| panic!("cannot truncate {path}: {error}"));
            handle
        } else {
            self.fs
                .create_new(&scope, path)
                .unwrap_or_else(|error| panic!("cannot create {path}: {error}"))
        }
    }

    /// Writes `content` as the whole content of a file, as another program
    /// does: in place when the file exists, creating it and its parent
    /// directories otherwise.
    pub(crate) fn write(&self, path: &AbsPath, content: &[u8]) {
        let mut handle = self.open_for_write(path);
        handle
            .write_all(content)
            .unwrap_or_else(|error| panic!("cannot write {path}: {error}"));
        handle
            .sync(Durability::Full)
            .unwrap_or_else(|error| panic!("cannot flush {path}: {error}"));
        drop(handle);
        self.settle();
    }

    /// Makes a file `length` bytes long with zero content; the length takes
    /// no space on the volume.
    pub(crate) fn write_sparse(&self, path: &AbsPath, length: u64) {
        let mut handle = self.open_for_write(path);
        handle
            .set_length(length)
            .unwrap_or_else(|error| panic!("cannot extend {path}: {error}"));
        handle
            .sync(Durability::Full)
            .unwrap_or_else(|error| panic!("cannot flush {path}: {error}"));
        drop(handle);
        self.settle();
    }

    /// Whole content of a file.
    pub(crate) fn read(&self, path: &AbsPath) -> Vec<u8> {
        let mut handle = self
            .fs
            .open_read(path)
            .unwrap_or_else(|error| panic!("cannot open {path}: {error}"));
        let mut content = Vec::new();
        handle
            .read_to_end(&mut content)
            .unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
        content
    }

    /// Adds the name `new` to the file `existing`.
    pub(crate) fn hard_link(&self, existing: &AbsPath, new: &AbsPath) {
        self.fs
            .hard_link(&outside(), existing, new)
            .unwrap_or_else(|error| panic!("cannot link {new} to {existing}: {error}"));
        self.settle();
    }

    /// Removes the name of a file.
    pub(crate) fn remove_file(&self, path: &AbsPath) {
        self.fs
            .remove_file(&outside(), path)
            .unwrap_or_else(|error| panic!("cannot remove {path}: {error}"));
        self.settle();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
