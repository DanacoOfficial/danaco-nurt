// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Retention over the shared context of the journal store.
//!
//! [`CtxStore`] gives retention access to the journal directories and the
//! state directory of the account through the layout and file formats of the
//! store. An entry moved to the reclaim area carries the reason of its expiry
//! in the file [`EXPIRY_FILE`], written into the entry directory before the
//! rename, so that the move and the reason become durable together.

use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom};

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::config::ConfigLayer;
use nurt_contracts::env::ClockStamp;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Durability, FileLock, Follow, FsError, FsErrorKind, FsResult, FsWrite, RenameMode,
    ScopePurpose,
};
use nurt_contracts::journal::{
    BeginRequest, EntryState, ExpiryReason, FileOp, InactiveReason, OpId, OpKind, PreserveMode,
    ServiceLiveness, apply_ops,
};
use nurt_fs::failpoint::FailPoint;

use super::age::{AGE_STATE_FILE, AgeState, EntryTimeBasis};
use super::port::{Area, EntryFacts, RetentionStore, StepKind, StepMark, VolumeFacts};
use super::tomb;
use crate::layout::{self, EntryLayout, JournalLayout};
use crate::manifest::{self, Manifest, ReadError, StateRecord};
use crate::pin::PinSet;
use crate::store::{Fault, JournalCtx, scope};
use crate::{recovery, tx};

/// File in the entry directory that records why the entry expires.
pub(crate) const EXPIRY_FILE: &str = "EXPIRY";

/// Audit records of retention (`journal.purge`) in the state directory.
pub(crate) const AUDIT_FILE: &str = "journal-audit.jsonl";

/// Permission bits of directories created for retention files and for the
/// user configuration file.
const DIR_MODE: u32 = 0o700;

/// Kind of the entry that records a change of the user configuration file.
const LIMITS_KIND: &str = "journal.limits";

/// Catalog and correction revision recorded for the program of a journal
/// maintenance entry, which is not bound from a catalog.
const NO_REVISION: &str = "";

/// Workers removing the data of one reclaimed entry; reclaim of large
/// entries in parallel belongs to the space service.
const REMOVE_PARALLELISM: usize = 1;

/// Operation number of scopes that maintain files outside any entry.
const MAINTENANCE_OP: OpId = OpId(0);

/// Kinds of the undo and redo entries, which count as steps only with
/// `journal.count_undo_as_op`.
const UNDO_KINDS: [&str; 2] = ["undo", "redo"];

/// Journal store context seen through [`RetentionStore`].
pub(crate) struct CtxStore<'a>(pub(crate) &'a JournalCtx);

impl CtxStore<'_> {
    fn fs(&self) -> &dyn FsWrite {
        self.0.fs.as_ref()
    }

    fn journal(&self, volume: &AbsPath) -> Result<JournalLayout> {
        for layout in self.0.active_journals() {
            if &self.0.fs.volume(layout.root())?.mount == volume {
                return Ok(layout);
            }
        }
        Err(Error::new(
            ErrorCode::NotFound,
            format!("the volume {volume} has no active journal"),
        ))
    }

    fn path(&self, area: Area<'_>, name: &str) -> Result<AbsPath> {
        let mut path = match area {
            Area::Journal(volume) => self.journal(volume)?.root().clone(),
            Area::State => self.0.state.root().clone(),
        };
        for segment in name.split('/') {
            path = path.join_name(segment)?;
        }
        Ok(path)
    }

    fn ensure_dir(&self, dir: &AbsPath) -> Result<()> {
        let scope = scope(MAINTENANCE_OP, ScopePurpose::Maintenance);
        match self.fs().create_dir(&scope, dir, DIR_MODE) {
            Ok(()) => Ok(()),
            Err(error) if error.kind == FsErrorKind::Exists => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn age_state(&self) -> Result<AgeState> {
        Ok(match self.read_file(Area::State, AGE_STATE_FILE)? {
            Some(content) => AgeState::decode(&content).unwrap_or_default(),
            None => AgeState::default(),
        })
    }

    fn pins(&self, layout: &JournalLayout) -> Result<PinSet> {
        match manifest::read_bytes(self.fs(), &layout.pins()?) {
            Ok(content) => Ok(PinSet::parse(&content).unwrap_or_default()),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(PinSet::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn reaped_ops(&self, layout: &JournalLayout) -> Result<Vec<OpId>> {
        let mut ops: Vec<OpId> = list_names(self.fs(), &layout.reap_dir()?)?
            .iter()
            .filter_map(|name| layout::parse_entry_name(name))
            .collect();
        ops.sort_unstable();
        Ok(ops)
    }

    fn reason(&self, entry: &EntryLayout) -> Result<Option<ExpiryReason>> {
        match manifest::read_bytes(self.fs(), &entry.dir().join_name(EXPIRY_FILE)?) {
            Ok(content) => Ok(parse_reason(&content)),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn user_config(&self) -> Result<AbsPath> {
        self.0
            .limits
            .user_config()
            .ok_or_else(user_config_unavailable)
    }

    /// Directories above `path` that do not exist, outermost first.
    fn missing_parents(&self, path: &AbsPath) -> Result<Vec<AbsPath>> {
        let mut missing = Vec::new();
        let mut current = path.parent();
        while let Some(dir) = current {
            if exists(self.fs(), &dir)? {
                break;
            }
            current = dir.parent();
            missing.push(dir);
        }
        missing.reverse();
        Ok(missing)
    }
}

/// What the store knows about one entry part besides its manifest.
struct PartState<'a> {
    volume: &'a AbsPath,
    op: OpId,
    state: EntryState,
    reason: Option<ExpiryReason>,
    pinned: bool,
}

fn facts_of(part: &PartState<'_>, manifest: &Manifest, basis: EntryTimeBasis) -> EntryFacts {
    let mut irreversible: Vec<_> = manifest
        .irreversible
        .iter()
        .map(|entry| entry.reason)
        .chain(
            manifest
                .targets
                .iter()
                .filter_map(|target| target.irreversible),
        )
        .collect();
    irreversible.dedup();
    EntryFacts {
        op: part.op,
        volume: part.volume.clone(),
        state: part.state,
        reason: part.reason,
        kind: manifest.kind.clone(),
        time: manifest.time,
        basis,
        files: u64::try_from(manifest.targets.len()).unwrap_or(u64::MAX),
        bytes_held: manifest.bytes_held,
        bytes_physical: manifest
            .targets
            .iter()
            .filter(|target| target.before.store != PreserveMode::Clone)
            .map(|target| target.bytes_held)
            .sum(),
        irreversible,
        pinned: part.pinned,
    }
}

/// Facts of a reclaimed part whose manifest is already removed, from its
/// tombstone.
fn facts_of_tombstone(
    part: &PartState<'_>,
    tombstone: &tomb::Tombstone,
    now: &ClockStamp,
) -> EntryFacts {
    EntryFacts {
        op: part.op,
        volume: part.volume.clone(),
        state: part.state,
        reason: part.reason.or(Some(tombstone.reason)),
        kind: tombstone.kind.clone(),
        time: tombstone.time,
        basis: EntryTimeBasis {
            committed: now.clone(),
            effective_wall: tombstone.time,
            active_at_commit: now.since_boot_active,
        },
        files: tombstone.files,
        bytes_held: tombstone.bytes,
        bytes_physical: 0,
        irreversible: tombstone.irreversible.clone(),
        pinned: false,
    }
}

/// Time basis of an entry from the clock readings recorded at commit.
///
/// The running time accumulated at commit is derived from the persisted age
/// state: exactly for an entry of the current boot, and from the start of
/// the current boot otherwise, so that an entry measured by running time
/// never expires earlier than its real running age.
fn basis_of(committed: &ClockStamp, age: &AgeState, now: &ClockStamp) -> EntryTimeBasis {
    let total = age.active_total(now);
    let since_commit = if committed.boot == now.boot {
        now.since_boot_active
            .saturating_sub(committed.since_boot_active)
    } else {
        now.since_boot_active
    };
    EntryTimeBasis {
        committed: committed.clone(),
        effective_wall: committed.wall,
        active_at_commit: total.saturating_sub(since_commit),
    }
}

/// Content of the file [`EXPIRY_FILE`] for `reason`.
pub(crate) fn reason_text(reason: ExpiryReason) -> String {
    match serde_json::to_value(reason) {
        Ok(serde_json::Value::String(name)) => format!("{name}\n"),
        _ => String::new(),
    }
}

fn parse_reason(content: &[u8]) -> Option<ExpiryReason> {
    let text = std::str::from_utf8(content).ok()?.trim_end();
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}

fn list_names(fs: &dyn FsWrite, dir: &AbsPath) -> Result<Vec<String>> {
    let entries = match fs.read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        names.push(entry?.name.to_string_lossy().into_owned());
    }
    Ok(names)
}

fn exists(fs: &dyn FsWrite, path: &AbsPath) -> FsResult<bool> {
    match fs.metadata(path, Follow::NoFollow) {
        Ok(_) => Ok(true),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn read_error(error: ReadError) -> Option<Error> {
    match error {
        ReadError::Fs(error) if error.kind == FsErrorKind::NotFound => None,
        ReadError::Format(_) => None,
        ReadError::Fs(error) => Some(error.into()),
    }
}

impl RetentionStore for CtxStore<'_> {
    fn config_layers(&self) -> Vec<ConfigLayer> {
        self.0.limits.layers()
    }

    fn project_trusted(&self) -> bool {
        self.0.limits.project_trusted()
    }

    fn clock(&self) -> ClockStamp {
        self.0.clock.stamp()
    }

    fn volumes(&self) -> Result<Vec<VolumeFacts>> {
        let inactive = (self.0.admission.liveness() == ServiceLiveness::Down)
            .then_some(InactiveReason::ServiceDown);
        let mut volumes: BTreeMap<AbsPath, VolumeFacts> = BTreeMap::new();
        for layout in self.0.active_journals() {
            let info = self.0.fs.volume(layout.root())?;
            let space = self.0.fs.space(&info.id)?;
            volumes.insert(
                info.mount.clone(),
                VolumeFacts {
                    mount: info.mount,
                    capacity: space.total,
                    free: space.available,
                    inactive,
                },
            );
        }
        Ok(volumes.into_values().collect())
    }

    fn entries(&self, volume: &AbsPath) -> Result<Vec<EntryFacts>> {
        let layout = self.journal(volume)?;
        let now = self.0.clock.stamp();
        let age = self.age_state()?;
        let pins = self.pins(&layout)?;
        let fs = self.fs();
        let mut entries = Vec::new();
        for op in layout::entries(fs, &layout)? {
            let entry = layout.entry(op)?;
            let state = match manifest::read_state(fs, &entry.state()?) {
                Ok(record) => record.state,
                Err(ReadError::Format(_)) => EntryState::Corrupt,
                Err(error) => match read_error(error) {
                    Some(error) => return Err(error),
                    None => continue,
                },
            };
            let manifest = match manifest::read_manifest(fs, &entry.manifest()?) {
                Ok(manifest) => manifest,
                Err(error) => match read_error(error) {
                    Some(error) => return Err(error),
                    None => continue,
                },
            };
            let part = PartState {
                volume,
                op,
                state,
                reason: None,
                pinned: pins.is_pinned(op, now.wall),
            };
            entries.push(facts_of(
                &part,
                &manifest,
                basis_of(&manifest.time_basis, &age, &now),
            ));
        }
        for op in self.reaped_ops(&layout)? {
            let entry = layout.reaped(op)?;
            let part = PartState {
                volume,
                op,
                state: EntryState::Expiring,
                reason: self.reason(&entry)?,
                pinned: false,
            };
            match manifest::read_manifest(fs, &entry.manifest()?) {
                Ok(manifest) => {
                    let basis = basis_of(&manifest.time_basis, &age, &now);
                    entries.push(facts_of(&part, &manifest, basis));
                }
                Err(error) => {
                    if let Some(error) = read_error(error) {
                        return Err(error);
                    }
                    if let Some(tombstone) = tomb::read(self, volume, op)? {
                        entries.push(facts_of_tombstone(&part, &tombstone, &now));
                    }
                }
            }
        }
        Ok(entries)
    }

    fn steps(&self) -> Result<Vec<StepMark>> {
        let rows = manifest::read_index(self.fs(), &self.0.state.index()?)?;
        Ok(rows
            .into_iter()
            .map(|row| StepMark {
                op: row.op,
                kind: if !row.counts_as_step {
                    StepKind::NotCounted
                } else if UNDO_KINDS.contains(&row.kind.command.as_str()) {
                    StepKind::Undo
                } else {
                    StepKind::Operation
                },
            })
            .collect())
    }

    fn lock_entry(&self, volume: &AbsPath, op: OpId) -> Result<Box<dyn FileLock>> {
        let layout = self.journal(volume)?;
        self.ensure_dir(&layout.lock_dir()?)?;
        Ok(self
            .fs()
            .lock_exclusive(&layout.entry_lock(op)?, self.0.settings.lock_timeout)?)
    }

    fn begin_reap(&self, volume: &AbsPath, op: OpId, reason: ExpiryReason) -> Result<()> {
        let layout = self.journal(volume)?;
        let entry = layout.entry(op)?;
        let reaped = layout.reaped(op)?;
        let fs = self.fs();
        let scope = scope(op, ScopePurpose::Reclaim);
        if !exists(fs, entry.dir())? {
            if exists(fs, reaped.dir())? {
                return Ok(());
            }
            return Err(Error::new(
                ErrorCode::JournalNoop,
                format!("entry {} does not exist on {volume}", op.0),
            ));
        }
        manifest::write_atomic(
            fs,
            &scope,
            entry.dir(),
            &entry.dir().join_name(EXPIRY_FILE)?,
            reason_text(reason).as_bytes(),
        )?;
        let reap_dir = layout.reap_dir()?;
        self.ensure_dir(&reap_dir)?;
        fs.rename(&scope, entry.dir(), reaped.dir(), RenameMode::NoReplace)?;
        crate::blob::sync_parents(fs, &scope, &[entry.dir(), reaped.dir(), &reap_dir])?;
        manifest::write_state(
            fs,
            &scope,
            reaped.dir(),
            &reaped.state()?,
            StateRecord::new(EntryState::Expiring),
        )?;
        Ok(())
    }

    fn remove_reaped(&self, volume: &AbsPath, op: OpId) -> Result<()> {
        let layout = self.journal(volume)?;
        let reaped = layout.reaped(op)?;
        let scope = scope(op, ScopePurpose::Reclaim);
        match self
            .fs()
            .remove_tree(&scope, reaped.dir(), REMOVE_PARALLELISM)
        {
            Ok(_) => {}
            Err(error) if error.kind == FsErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        Ok(self.fs().sync_dir(&scope, &layout.reap_dir()?)?)
    }

    fn hit(&self, point: FailPoint) -> Result<()> {
        self.0.hit(point, None).map_err(|fault| match fault {
            Fault::Fs(error) => error.into(),
            Fault::Halt => Error::new(
                ErrorCode::Internal,
                format!("stopped at the simulated crash point {}", point.as_str()),
            ),
        })
    }

    fn read_file(&self, area: Area<'_>, name: &str) -> Result<Option<Vec<u8>>> {
        match manifest::read_bytes(self.fs(), &self.path(area, name)?) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write_file(&self, area: Area<'_>, name: &str, content: &[u8]) -> Result<()> {
        let path = self.path(area, name)?;
        let parent = path.parent().ok_or_else(|| {
            Error::from(FsError::new(
                FsErrorKind::InvalidPath,
                "write_file",
                Some(path.as_path()),
            ))
        })?;
        self.ensure_dir(&parent)?;
        let scope = scope(MAINTENANCE_OP, ScopePurpose::Maintenance);
        Ok(manifest::write_atomic(
            self.fs(),
            &scope,
            &parent,
            &path,
            content,
        )?)
    }

    fn remove_file(&self, area: Area<'_>, name: &str) -> Result<()> {
        let scope = scope(MAINTENANCE_OP, ScopePurpose::Maintenance);
        match self.fs().remove_file(&scope, &self.path(area, name)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn list_dir(&self, area: Area<'_>, dir: &str) -> Result<Vec<String>> {
        list_names(self.fs(), &self.path(area, dir)?)
    }

    fn audit(&self, record: &serde_json::Value) -> Result<()> {
        let path = self.0.state.root().join_name(AUDIT_FILE)?;
        let mut line = serde_json::to_vec(record).map_err(|error| {
            Error::new(
                ErrorCode::Internal,
                format!("cannot serialize the audit record: {error}"),
            )
        })?;
        line.push(b'\n');
        let fs = self.fs();
        let scope = scope(MAINTENANCE_OP, ScopePurpose::Maintenance);
        let mut handle = match fs.open_in_place(&scope, &path) {
            Ok(handle) => handle,
            Err(error) if error.kind == FsErrorKind::NotFound => fs.create_new(&scope, &path)?,
            Err(error) => return Err(error.into()),
        };
        handle
            .seek(SeekFrom::End(0))
            .map_err(|_| FsError::new(FsErrorKind::Io, "seek", Some(path.as_path())))?;
        manifest::write_all(handle.as_mut(), &line)?;
        Ok(handle.sync(Durability::Data)?)
    }

    fn read_user_config(&self) -> Result<Option<String>> {
        let path = self.user_config()?;
        let content = match manifest::read_bytes(self.fs(), &path) {
            Ok(content) => content,
            Err(error) if error.kind == FsErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        String::from_utf8(content).map(Some).map_err(|_| {
            Error::new(
                ErrorCode::Config,
                format!("the user configuration file {path} is not valid UTF-8; correct it first"),
            )
        })
    }

    /// Writes the user configuration file in a transaction of kind
    /// `journal.limits` that does not count as a step; missing parent
    /// directories are created by the same transaction, so undo of the entry
    /// restores the previous state completely.
    fn write_user_config(&self, content: &str) -> Result<OpId> {
        let path = self.user_config()?;
        let mut ops: Vec<FileOp> = self
            .missing_parents(&path)?
            .into_iter()
            .map(|dir| FileOp::create_dir(dir, DIR_MODE))
            .collect();
        ops.push(FileOp::write(path.clone(), content.as_bytes().to_vec()));
        // A failed recovery is recorded in the context; the transaction then
        // keeps no previous content and reports `journal_error`.
        let _ = recovery::ensure_recovered(self.0);
        let mut transaction = tx::begin(
            self.0,
            BeginRequest {
                kind: OpKind::command(LIMITS_KIND),
                session: None,
                program: BoundDocument::new(
                    NO_REVISION,
                    NO_REVISION,
                    BoundProgram {
                        statements: Vec::new(),
                    },
                ),
                corrections: Vec::new(),
                bulk: ops.len() > 1,
                counts_as_step: false,
                preserve_budget: None,
            },
        );
        apply_ops(transaction.as_mut(), ops)?;
        transaction.commit()?.op.ok_or_else(|| {
            Error::new(
                ErrorCode::Internal,
                format!("the change of {path} created no journal entry"),
            )
        })
    }
}

fn user_config_unavailable() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "the journal has no user configuration file; configure its path or use scope: session",
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use nurt_contracts::config::{ConfigLevel, OverwriteStrategy, StepLimit};
    use nurt_contracts::env::{Clock as _, WallTime};
    use nurt_contracts::fs::{ContentHash, Fs as _, VolumeInfo};
    use nurt_contracts::journal::{
        ExpiredEntry, LimitsRequest, LimitsScope, OpKind, PurgeRequest, PurgeSelector, SessionId,
    };
    use nurt_fs::clock::VirtualClock;
    use nurt_fs::failpoint::{
        FailAction, FailPlan, FailRule, Failpoints, NoFailpoints, SeededFailpoints,
    };
    use nurt_fs::mem::{MemFs, VolumeSpec};
    use serde_json::{Value, json};

    use super::super::expiry::{BudgetAdmission, admit_size, complete_reap, purge, sweep};
    use super::super::limits::{SESSION_DIR, limits, status};
    use super::super::resolve::ConfiguredLimits;
    use super::*;
    use crate::layout::StateLayout;
    use crate::manifest::{IndexRow, Integrity};
    use crate::store::test_support::TestAdmission;
    use crate::store::{JournalLocation, Settings, VolumeJournals};

    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;
    const MINUTE: u64 = 60;
    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    /// Volume of the home directory, which also holds the state directory.
    const HOME: &str = "/home";
    const HOME_CAPACITY: u64 = 500 * GIB;
    /// Second volume with a journal.
    const DATA: &str = "/mnt/data";
    const DATA_CAPACITY: u64 = 100 * GIB;
    const STATE_DIR: &str = "/home/user/.local/state/nurt";
    /// Short lock wait, so that a held entry lock fails the call at once.
    const LOCK_TIMEOUT: Duration = Duration::from_millis(20);

    fn mount(path: &str) -> AbsPath {
        AbsPath::new(path).unwrap()
    }

    fn state_dir() -> AbsPath {
        mount(STATE_DIR)
    }

    fn journal_of(volume: &str) -> JournalLayout {
        JournalLayout::new(mount(volume).join_name(layout::default_dir_name()).unwrap())
    }

    fn expired(op: u64, reason: ExpiryReason) -> ExpiredEntry {
        ExpiredEntry {
            op: OpId(op),
            reason,
        }
    }

    fn ops(list: &[OpId]) -> Vec<u64> {
        list.iter().map(|op| op.0).collect()
    }

    fn steps(count: u32, scope: LimitsScope, preview: bool) -> LimitsRequest {
        LimitsRequest {
            steps: Some(StepLimit::Limited(count)),
            age: None,
            size: None,
            scope,
            volume: None,
            preview,
        }
    }

    fn session() -> LimitsScope {
        LimitsScope::Session(SessionId("s1".to_owned()))
    }

    /// Journal directories of both volumes, always active.
    struct Journals(Vec<AbsPath>);

    impl VolumeJournals for Journals {
        fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
            self.0
                .iter()
                .find(|dir| dir.parent().as_ref() == Some(&volume.mount))
                .map_or(JournalLocation::Inactive(InactiveReason::NoAnchor), |dir| {
                    JournalLocation::Active(dir.clone())
                })
        }

        fn active(&self) -> Vec<AbsPath> {
            self.0.clone()
        }
    }

    /// In-memory file system with the volumes `/`, `/home` (500 GiB) and
    /// `/mnt/data` (100 GiB), a virtual clock and the user configuration
    /// layer `{ journal: <settings> }`.
    struct Env {
        fs: Arc<MemFs>,
        clock: Arc<VirtualClock>,
        admission: Arc<TestAdmission>,
        layers: Vec<ConfigLayer>,
        index: RefCell<Vec<u8>>,
    }

    impl Env {
        fn new(settings: Value) -> Self {
            let clock = Arc::new(VirtualClock::default());
            let fs = MemFs::builder()
                .clock(clock.clone())
                .volume(VolumeSpec::posix(mount("/")))
                .volume(VolumeSpec::posix(mount(HOME)).with_capacity(HOME_CAPACITY))
                .volume(VolumeSpec::posix(mount(DATA)).with_capacity(DATA_CAPACITY))
                .build()
                .unwrap();
            for volume in [HOME, DATA] {
                for dir in journal_of(volume).structure().unwrap() {
                    fs.seed_dir(&dir).unwrap();
                }
            }
            fs.seed_dir(&state_dir()).unwrap();
            let values = json!({ "journal": settings });
            Self {
                fs: Arc::new(fs),
                clock,
                admission: Arc::new(TestAdmission::default()),
                layers: vec![ConfigLayer::new(
                    ConfigLevel::User,
                    "user.toml",
                    values.as_object().cloned().unwrap_or_default(),
                )],
                index: RefCell::new(Vec::new()),
            }
        }

        fn ctx(&self) -> JournalCtx {
            self.ctx_with(Arc::new(NoFailpoints))
        }

        fn crashing_at(&self, point: FailPoint) -> JournalCtx {
            let rule = FailRule {
                point,
                n: None,
                occurrence: 0,
                action: FailAction::Crash,
            };
            self.ctx_with(Arc::new(SeededFailpoints::new(FailPlan::at(0, rule))))
        }

        fn ctx_with(&self, failpoints: Arc<dyn Failpoints>) -> JournalCtx {
            let limits =
                ConfiguredLimits::new(&self.layers, false, self.fs.clone(), &state_dir()).unwrap();
            JournalCtx {
                fs: self.fs.clone(),
                providers: Vec::new(),
                admission: self.admission.clone(),
                limits: Arc::new(limits),
                clock: self.clock.clone(),
                journals: Arc::new(Journals(vec![
                    journal_of(HOME).root().clone(),
                    journal_of(DATA).root().clone(),
                ])),
                state: StateLayout::new(state_dir()),
                failpoints,
                settings: Settings {
                    overwrite: OverwriteStrategy::default(),
                    lock_timeout: LOCK_TIMEOUT,
                    reference_schemes: Vec::new(),
                },
                space: None,
                startup: OnceLock::new(),
            }
        }

        /// Commits entry `op` with one part of the given logical size per
        /// volume and appends its row to the operation index.
        fn commit(&self, op: u64, parts: &[(&str, u64)]) {
            let stamp = self.clock.stamp();
            for (volume, bytes) in parts {
                let entry = journal_of(volume).entry(OpId(op)).unwrap();
                self.fs.seed_dir(entry.dir()).unwrap();
                let manifest = Manifest {
                    v: manifest::FORMAT_VERSION,
                    op: OpId(op),
                    uuid: uuid::Uuid::nil(),
                    time: stamp.wall,
                    time_basis: stamp.clone(),
                    kind: OpKind::command("write"),
                    session: None,
                    ast_hash: ContentHash([0; 32]),
                    ast: serde_json::Value::Null,
                    corrections: Vec::new(),
                    bulk: false,
                    counts_as_step: true,
                    targets: Vec::new(),
                    parts: Vec::new(),
                    bytes_held: *bytes,
                    irreversible: Vec::new(),
                    notable: Vec::new(),
                    exempt: Vec::new(),
                    expires: None,
                    undo_of: Vec::new(),
                    redo_of: None,
                    scope: None,
                    items: Vec::new(),
                    integrity: Integrity::default(),
                };
                self.fs
                    .seed_file(&entry.manifest().unwrap(), &manifest.encode().unwrap())
                    .unwrap();
                self.fs
                    .seed_file(
                        &entry.state().unwrap(),
                        &StateRecord::new(EntryState::Committed).encode(),
                    )
                    .unwrap();
            }
            let row = IndexRow {
                op: OpId(op),
                time: stamp.wall,
                kind: OpKind::command("write"),
                session: None,
                bulk: false,
                counts_as_step: true,
                parts: Vec::new(),
            };
            let mut index = self.index.borrow_mut();
            index.extend(row.encode().unwrap());
            let state = StateLayout::new(state_dir());
            self.replace(state.root(), &state.index().unwrap(), &index);
        }

        fn commit_all(&self, volume: &str, ops: std::ops::RangeInclusive<u64>) {
            for op in ops {
                self.commit(op, &[(volume, MIB)]);
            }
        }

        fn replace(&self, dir: &AbsPath, path: &AbsPath, content: &[u8]) {
            let scope = scope(MAINTENANCE_OP, ScopePurpose::Maintenance);
            manifest::write_atomic(self.fs.as_ref(), &scope, dir, path, content).unwrap();
        }

        fn corrupt_state(&self, volume: &str, op: u64) {
            let entry = journal_of(volume).entry(OpId(op)).unwrap();
            self.replace(entry.dir(), &entry.state().unwrap(), b"not a state record");
        }

        fn pin(&self, volume: &str, op: u64, until: WallTime) {
            let mut pins = PinSet::default();
            pins.pin(OpId(op), until);
            self.fs
                .seed_file(
                    &journal_of(volume).pins().unwrap(),
                    &pins.to_bytes().unwrap(),
                )
                .unwrap();
        }

        fn live(&self, volume: &str) -> Vec<u64> {
            let mut live: Vec<u64> = layout::entries(self.fs.as_ref(), &journal_of(volume))
                .unwrap()
                .into_iter()
                .map(|op| op.0)
                .collect();
            live.sort_unstable();
            live
        }

        fn exists(&self, path: &AbsPath) -> bool {
            exists(self.fs.as_ref(), path).unwrap()
        }

        fn names(&self, dir: &AbsPath) -> Vec<String> {
            list_names(self.fs.as_ref(), dir).unwrap()
        }

        fn volume(&self, path: &str) -> VolumeInfo {
            self.fs.volume(&mount(path)).unwrap()
        }
    }

    #[test]
    fn sweep_expires_the_entry_over_the_step_limit() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=31);
        let ctx = env.ctx();
        let report = sweep(&ctx).unwrap();
        assert_eq!(report.expired, [expired(1, ExpiryReason::Ops)]);
        assert_eq!(report.bytes, MIB);
        assert_eq!(env.live(HOME), (2..=31).collect::<Vec<_>>());
        let journal = journal_of(HOME);
        assert!(!env.exists(journal.reaped(OpId(1)).unwrap().dir()));
        assert!(env.exists(&journal.tomb(OpId(1)).unwrap()));
        assert!(env.exists(&state_dir().join_name(AGE_STATE_FILE).unwrap()));
        assert!(sweep(&ctx).unwrap().expired.is_empty());
    }

    #[test]
    fn sweep_measures_age_by_the_wall_clock_including_suspend() {
        let env = Env::new(json!({}));
        env.commit(1, &[(HOME, MIB)]);
        let ctx = env.ctx();
        env.clock.advance(Duration::from_secs(30 * MINUTE - 1));
        assert!(sweep(&ctx).unwrap().expired.is_empty());
        env.clock.advance(Duration::from_secs(1));
        assert_eq!(
            sweep(&ctx).unwrap().expired,
            [expired(1, ExpiryReason::Time)]
        );

        env.commit(2, &[(HOME, MIB)]);
        env.clock.suspend(Duration::from_secs(2 * HOUR));
        assert_eq!(
            sweep(&ctx).unwrap().expired,
            [expired(2, ExpiryReason::Time)]
        );
    }

    #[test]
    fn sweep_measures_age_by_running_time_with_the_active_clock() {
        let env = Env::new(json!({ "age_clock": "active" }));
        env.commit(1, &[(HOME, MIB)]);
        let ctx = env.ctx();
        env.clock.suspend(Duration::from_secs(2 * HOUR));
        assert!(sweep(&ctx).unwrap().expired.is_empty());
        env.clock.advance(Duration::from_secs(30 * MINUTE));
        assert_eq!(
            sweep(&ctx).unwrap().expired,
            [expired(1, ExpiryReason::Time)]
        );
    }

    #[test]
    fn sweep_keeps_a_pinned_entry_over_the_step_limit() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=31);
        env.pin(
            HOME,
            1,
            env.clock.now().saturating_add(Duration::from_secs(DAY)),
        );
        let ctx = env.ctx();
        assert!(sweep(&ctx).unwrap().expired.is_empty());
        assert_eq!(env.live(HOME).first(), Some(&1));
    }

    #[test]
    fn sweep_fails_without_change_while_the_entry_lock_is_held() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=31);
        let journal = journal_of(HOME);
        env.fs.seed_dir(&journal.lock_dir().unwrap()).unwrap();
        let lock = env
            .fs
            .lock_exclusive(&journal.entry_lock(OpId(1)).unwrap(), LOCK_TIMEOUT)
            .unwrap();
        let ctx = env.ctx();
        assert!(sweep(&ctx).is_err());
        assert_eq!(env.live(HOME).len(), 31);
        assert!(!env.exists(journal.reaped(OpId(1)).unwrap().dir()));
        drop(lock);
        assert_eq!(
            sweep(&ctx).unwrap().expired,
            [expired(1, ExpiryReason::Ops)]
        );
    }

    #[test]
    fn corrupt_entries_are_counted_but_never_expired() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=31);
        env.corrupt_state(HOME, 1);
        let ctx = env.ctx();
        assert!(sweep(&ctx).unwrap().expired.is_empty());
        let state = status(&ctx, Some(&mount(HOME))).unwrap().remove(0);
        assert_eq!((state.entries_total, state.entries_available), (31, 30));
        assert_eq!(state.oldest.map(|oldest| oldest.op), Some(OpId(2)));
    }

    #[test]
    fn session_limits_expire_entries_and_reach_the_store_limits() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=25);
        let ctx = env.ctx();
        let lowered = limits(&ctx, &steps(10, session(), false)).unwrap();
        assert!(lowered.applied);
        assert_eq!(ops(&lowered.expiring), (1..=15).collect::<Vec<_>>());
        assert_eq!(lowered.expiring_bytes, 15 * MIB);
        assert_eq!(env.live(HOME), (16..=25).collect::<Vec<_>>());
        assert_eq!(
            env.names(&state_dir().join_name(SESSION_DIR).unwrap())
                .len(),
            1
        );
        let effective = ctx.limits.effective(&env.volume(HOME));
        assert_eq!(effective.max_ops.value, StepLimit::Limited(10));
        assert_eq!(effective.max_ops.source, ConfigLevel::Runtime);

        let raised = limits(&ctx, &steps(30, session(), false)).unwrap();
        assert!(raised.expiring.is_empty());
        assert_eq!(env.live(HOME), (16..=25).collect::<Vec<_>>());
        assert!(env.exists(&journal_of(HOME).tomb(OpId(15)).unwrap()));
    }

    #[test]
    fn limits_preview_changes_nothing() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=25);
        let ctx = env.ctx();
        let outcome = limits(&ctx, &steps(10, session(), true)).unwrap();
        assert!(!outcome.applied);
        assert_eq!(outcome.expiring.len(), 15);
        assert_eq!(outcome.expiring_bytes, 15 * MIB);
        assert_eq!(env.live(HOME).len(), 25);
        assert!(
            env.names(&state_dir().join_name(SESSION_DIR).unwrap())
                .is_empty()
        );
    }

    #[test]
    fn user_scope_is_unsupported_without_the_user_configuration_file() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=12);
        let ctx = env.ctx();
        let error = limits(&ctx, &steps(10, LimitsScope::User, false)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert_eq!(
            error.message,
            "the journal has no user configuration file; configure its path or use scope: session"
        );
        assert_eq!(env.live(HOME).len(), 12);
        let error = limits(&ctx, &steps(10, LimitsScope::User, true)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
        let mut report = steps(10, LimitsScope::User, false);
        report.steps = None;
        assert!(!limits(&ctx, &report).unwrap().applied);
    }

    #[test]
    fn status_reports_every_volume_and_the_volume_of_a_path() {
        let env = Env::new(json!({}));
        for op in 1..=3 {
            env.commit(op, &[(HOME, 4 * MIB)]);
        }
        env.commit(4, &[(DATA, MIB)]);
        let ctx = env.ctx();
        let states = status(&ctx, None).unwrap();
        let volumes: Vec<AbsPath> = states.iter().map(|state| state.volume.clone()).collect();
        assert_eq!(volumes, [mount(HOME), mount(DATA)]);
        let home = &states[0];
        assert_eq!(
            (
                home.entries_total,
                home.entries_available,
                home.entries_expired
            ),
            (3, 3, 0)
        );
        assert_eq!(home.bytes_held, 12 * MIB);
        assert_eq!(home.oldest.as_ref().map(|oldest| oldest.op), Some(OpId(1)));
        assert_eq!(home.limits.max_ops.value, StepLimit::Limited(30));
        assert_eq!(home.limits.max_ops.source, ConfigLevel::Builtin);
        assert_eq!((home.inactive, home.over_limit), (None, 0));

        let data = status(&ctx, Some(&mount("/mnt/data/project"))).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].volume, mount(DATA));
        let error = status(&ctx, Some(&mount("/srv"))).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);

        *env.admission.liveness.lock().unwrap() = ServiceLiveness::Down;
        assert!(
            status(&ctx, None)
                .unwrap()
                .iter()
                .all(|state| state.inactive == Some(InactiveReason::ServiceDown))
        );
    }

    #[test]
    fn share_of_free_space_counts_the_bytes_held_by_the_journal() {
        let env = Env::new(json!({ "max_size": "10%free" }));
        env.commit(1, &[(DATA, GIB)]);
        let ctx = env.ctx();
        let state = status(&ctx, Some(&mount(DATA))).unwrap().remove(0);
        let free = env.fs.space(&env.volume(DATA).id).unwrap().available;
        assert_eq!(state.limits.max_size_bytes, Some((free + GIB) / 10));
    }

    fn home_limits(env: &Env) -> nurt_contracts::journal::EffectiveRetention {
        let ctx = env.ctx();
        status(&ctx, Some(&mount(HOME))).unwrap().remove(0).limits
    }

    #[test]
    fn admission_expires_the_oldest_entries_that_make_room_for_the_operation() {
        let env = Env::new(json!({ "max_size": "4mib" }));
        env.commit_all(HOME, 1..=4);
        let ctx = env.ctx();
        let limits = home_limits(&env);
        let admission = admit_size(&ctx, &mount(HOME), 2 * MIB, 2 * MIB, &limits).unwrap();
        assert_eq!(
            admission,
            BudgetAdmission::Fits {
                victims: vec![OpId(1), OpId(2)]
            }
        );
        assert_eq!(env.live(HOME), [3, 4]);
        assert!(env.exists(&journal_of(HOME).tomb(OpId(1)).unwrap()));
    }

    #[test]
    fn admission_of_an_operation_over_the_budget_expires_nothing() {
        let env = Env::new(json!({ "max_size": "4mib" }));
        env.commit_all(HOME, 1..=2);
        let ctx = env.ctx();
        let limits = home_limits(&env);
        let admission = admit_size(&ctx, &mount(HOME), 5 * MIB, 5 * MIB, &limits).unwrap();
        assert_eq!(
            admission,
            BudgetAdmission::TooLarge {
                size: 5 * MIB,
                limit: 4 * MIB
            }
        );
        assert_eq!(env.live(HOME), [1, 2]);
    }

    #[test]
    fn admission_without_a_budget_leaves_the_entries_alone() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=3);
        let ctx = env.ctx();
        let limits = home_limits(&env);
        let admission = admit_size(&ctx, &mount(HOME), GIB, GIB, &limits).unwrap();
        assert_eq!(
            admission,
            BudgetAdmission::Fits {
                victims: Vec::new()
            }
        );
        assert_eq!(env.live(HOME), [1, 2, 3]);
    }

    #[test]
    fn purge_releases_selected_entries_and_appends_an_audit_record() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=5);
        env.commit(6, &[(DATA, MIB)]);
        env.pin(
            HOME,
            2,
            env.clock.now().saturating_add(Duration::from_secs(DAY)),
        );
        let ctx = env.ctx();
        let request = |volume: Option<&str>, selector| PurgeRequest {
            volume: volume.map(mount),
            selector,
        };
        let first = purge(&ctx, &request(Some(HOME), PurgeSelector::UpTo(OpId(2)))).unwrap();
        assert_eq!((ops(&first.released), first.bytes), (vec![1, 2], 2 * MIB));
        let second = purge(
            &ctx,
            &request(None, PurgeSelector::Ids(vec![OpId(4), OpId(9)])),
        )
        .unwrap();
        assert_eq!(ops(&second.released), [4]);
        let third = purge(&ctx, &request(None, PurgeSelector::All)).unwrap();
        assert_eq!(ops(&third.released), [3, 5, 6]);
        assert!(env.live(HOME).is_empty() && env.live(DATA).is_empty());

        let store = CtxStore(&ctx);
        let pinned = tomb::read(&store, &mount(HOME), OpId(2)).unwrap().unwrap();
        assert_eq!(pinned.reason, ExpiryReason::Purge);
        assert_eq!(store.steps().unwrap().len(), 6);
        let audit =
            manifest::read_bytes(env.fs.as_ref(), &state_dir().join_name(AUDIT_FILE).unwrap())
                .unwrap();
        let records: Vec<Value> = audit
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["event"], "journal.purge");
        assert_eq!(records[0]["released"], json!([1, 2]));
    }

    #[test]
    fn purge_releases_an_entry_on_every_volume() {
        let env = Env::new(json!({}));
        env.commit(1, &[(HOME, MIB), (DATA, 2 * MIB)]);
        env.commit(2, &[(DATA, MIB)]);
        let ctx = env.ctx();
        let outcome = purge(
            &ctx,
            &PurgeRequest {
                volume: Some(mount(DATA)),
                selector: PurgeSelector::All,
            },
        )
        .unwrap();
        assert_eq!(
            (ops(&outcome.released), outcome.bytes),
            (vec![1, 2], 4 * MIB)
        );
        assert!(env.live(HOME).is_empty());
        for volume in [HOME, DATA] {
            assert!(env.exists(&journal_of(volume).tomb(OpId(1)).unwrap()));
        }
    }

    #[test]
    fn complete_reap_finishes_an_entry_left_in_the_reclaim_area() {
        let env = Env::new(json!({}));
        env.commit_all(HOME, 1..=31);
        let error = sweep(&env.crashing_at(FailPoint::ReclaimMoved)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
        let journal = journal_of(HOME);
        let reaped = journal.reaped(OpId(1)).unwrap();
        assert!(env.exists(reaped.dir()));

        let ctx = env.ctx();
        assert_eq!(status(&ctx, Some(&mount(HOME))).unwrap()[0].over_limit, 1);
        let first = complete_reap(&ctx, OpId(1)).unwrap();
        assert_eq!(first, expired(1, ExpiryReason::Ops));
        assert!(!env.exists(reaped.dir()));
        assert!(env.exists(&journal.tomb(OpId(1)).unwrap()));
        assert_eq!(complete_reap(&ctx, OpId(1)).unwrap(), first);
        let error = complete_reap(&ctx, OpId(2)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::JournalNoop);
    }

    #[test]
    fn complete_reap_uses_the_tombstone_when_the_manifest_is_gone() {
        let env = Env::new(json!({}));
        env.commit(5, &[(HOME, 7)]);
        let ctx = env.ctx();
        let store = CtxStore(&ctx);
        let volume = mount(HOME);
        store
            .begin_reap(&volume, OpId(5), ExpiryReason::Time)
            .unwrap();
        let part = store.entries(&volume).unwrap().remove(0);
        assert_eq!(
            (part.state, part.reason),
            (EntryState::Expiring, Some(ExpiryReason::Time))
        );
        tomb::write(
            &store,
            &volume,
            &tomb::Tombstone::of(&part, ExpiryReason::Time),
        )
        .unwrap();
        let reaped = journal_of(HOME).reaped(OpId(5)).unwrap();
        env.fs
            .remove_file(
                &scope(OpId(5), ScopePurpose::Reclaim),
                &reaped.manifest().unwrap(),
            )
            .unwrap();
        let remaining = store.entries(&volume).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].bytes_held, 7);

        assert_eq!(
            complete_reap(&ctx, OpId(5)).unwrap(),
            expired(5, ExpiryReason::Time)
        );
        assert!(!env.exists(reaped.dir()));
    }

    #[test]
    fn expiry_reasons_round_trip_through_the_reason_file() {
        for reason in [
            ExpiryReason::Ops,
            ExpiryReason::Time,
            ExpiryReason::Size,
            ExpiryReason::Space,
            ExpiryReason::Purge,
        ] {
            assert_eq!(parse_reason(reason_text(reason).as_bytes()), Some(reason));
        }
        assert_eq!(parse_reason(b"later"), None);
    }
}

#[cfg(test)]
mod durability_tests {
    use nurt_contracts::bound::{BoundDocument, BoundProgram};
    use nurt_contracts::journal::{
        BeginRequest, Expect, ExpiryReason, Mutation, OpKind, TargetAction, TargetIndex,
        TargetPath, TargetSpec,
    };

    use super::CtxStore;
    use crate::retention::port::RetentionStore as _;
    use crate::store::test_support::{Fixture, unflushed};
    use crate::tx;
    use nurt_contracts::fs::AbsPath;

    fn request() -> BeginRequest {
        BeginRequest {
            kind: OpKind::command("write"),
            session: None,
            program: BoundDocument::new(
                "cat-1",
                "corr-1",
                BoundProgram {
                    statements: Vec::new(),
                },
            ),
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            preserve_budget: None,
        }
    }

    fn spec(path: &AbsPath, action: TargetAction) -> TargetSpec {
        TargetSpec {
            path: TargetPath::Fs(path.clone()),
            action,
            expect: Expect::Unchecked,
            size_hint: None,
        }
    }

    #[test]
    fn begin_reap_flushes_both_directories_of_the_rename() {
        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        let path = fixture.fs.path("vol-a/a.txt");
        fixture.write(&path, b"a");
        let mut transaction = tx::begin(&ctx, request());
        transaction
            .preserve(&[spec(&path, TargetAction::Delete)])
            .unwrap();
        transaction.apply(TargetIndex(0), Mutation::Remove).unwrap();
        let handle = transaction.commit().unwrap();
        let op = handle.op.unwrap();
        let volume = fixture.fs.path("vol-a");
        fixture.fs.clear_log();
        CtxStore(&ctx)
            .begin_reap(&volume, op, ExpiryReason::Time)
            .unwrap();
        let log = fixture.fs.log();
        let pending = unflushed(&log, log.len());
        assert!(pending.is_empty(), "{pending:#?}\n{log:#?}");
    }

    #[test]
    fn complete_reclaim_of_a_purged_entry_keeps_its_tombstone() {
        use nurt_contracts::fs::Fs as _;

        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        let path = fixture.fs.path("vol-a/a.txt");
        fixture.write(&path, b"a");
        let mut transaction = tx::begin(&ctx, request());
        transaction
            .preserve(&[spec(&path, TargetAction::Delete)])
            .unwrap();
        transaction.apply(TargetIndex(0), Mutation::Remove).unwrap();
        let op = transaction.commit().unwrap().op.unwrap();
        let volume = fixture.fs.path("vol-a");
        CtxStore(&ctx)
            .begin_reap(&volume, op, ExpiryReason::Purge)
            .unwrap();
        let id = fixture.fs.volume(&volume).unwrap().id;
        let expired = crate::catalog::complete_reclaim(&ctx, &id, op).unwrap();
        assert_eq!(expired.reason, ExpiryReason::Purge);
        assert!(CtxStore(&ctx).entries(&volume).unwrap().is_empty());
    }
}
