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

use nurt_contracts::config::ConfigLayer;
use nurt_contracts::env::ClockStamp;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Durability, FileLock, Follow, FsError, FsErrorKind, FsResult, FsWrite, RenameMode,
    ScopePurpose,
};
use nurt_contracts::journal::{
    EntryState, ExpiredEntry, ExpiryReason, InactiveReason, JournalState, LimitsOutcome,
    LimitsRequest, OpId, PreserveMode, PurgeOutcome, PurgeRequest, ServiceLiveness, SweepReport,
};
use nurt_fs::failpoint::FailPoint;

use super::age::{AGE_STATE_FILE, AgeState, EntryTimeBasis};
use super::expiry::{complete_reap_store, purge_store, sweep_store};
use super::limits::{limits_store, status_store};
use super::port::{Area, EntryFacts, RetentionStore, StepKind, StepMark, VolumeFacts};
use super::tomb;
use crate::layout::{self, EntryLayout, JournalLayout};
use crate::manifest::{self, Manifest, ReadError, StateRecord};
use crate::pin::PinSet;
use crate::store::{Fault, JournalCtx, scope};

/// File in the entry directory that records why the entry expires.
pub(crate) const EXPIRY_FILE: &str = "EXPIRY";

/// Audit records of retention (`journal.purge`) in the state directory.
pub(crate) const AUDIT_FILE: &str = "journal-audit.jsonl";

/// Permission bits of directories created for retention files.
const DIR_MODE: u32 = 0o700;

/// Workers removing the data of one reclaimed entry; reclaim of large
/// entries in parallel belongs to the space service.
const REMOVE_PARALLELISM: usize = 1;

/// Operation number of scopes that maintain files outside any entry.
const MAINTENANCE_OP: OpId = OpId(0);

/// Kinds of the undo and redo entries, which count as steps only with
/// `journal.count_undo_as_op`.
const UNDO_KINDS: [&str; 2] = ["undo", "redo"];

/// Expires entries over any limit (steps, age, size).
///
/// # Errors
///
/// Returns `E_CONFIG` for an invalid configuration and `E_IO` when a journal
/// directory cannot be read or an entry cannot be reclaimed.
pub(crate) fn sweep(ctx: &JournalCtx) -> Result<SweepReport> {
    sweep_store(&CtxStore(ctx))
}

/// Completes the reclaim of an entry moved to `reap/<N>`, under the lock of
/// the entry; idempotent.
///
/// # Errors
///
/// Returns `E_JOURNAL_NOOP` when the entry is neither being reclaimed nor
/// expired, `E_IN_USE` when another process holds the entry lock and `E_IO`
/// when the reclaim cannot be completed.
pub(crate) fn complete_reap(ctx: &JournalCtx, op: OpId) -> Result<ExpiredEntry> {
    complete_reap_store(&CtxStore(ctx), op)
}

/// Releases entries immediately (`journal.purge`).
///
/// # Errors
///
/// Returns `E_NOT_FOUND` for an unknown volume and `E_IO` when an entry
/// cannot be reclaimed.
pub(crate) fn purge(ctx: &JournalCtx, request: &PurgeRequest) -> Result<PurgeOutcome> {
    purge_store(&CtxStore(ctx), request)
}

/// Retention state per volume (`journal.status`).
///
/// # Errors
///
/// Returns `E_NOT_FOUND` for an unknown volume and `E_IO` when a journal
/// directory cannot be read.
pub(crate) fn status(ctx: &JournalCtx, volume: Option<&AbsPath>) -> Result<Vec<JournalState>> {
    status_store(&CtxStore(ctx), volume)
}

/// Shows or changes limits (`journal.limits`).
///
/// # Errors
///
/// As [`limits_store`]; `scope: user` fails with `E_UNSUPPORTED` while the
/// store has no access to the user configuration file.
pub(crate) fn limits(ctx: &JournalCtx, request: &LimitsRequest) -> Result<LimitsOutcome> {
    limits_store(&CtxStore(ctx), request)
}

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
        .chain(manifest.targets.iter().filter_map(|target| target.irreversible))
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
fn facts_of_tombstone(part: &PartState<'_>, tombstone: &tomb::Tombstone, now: &ClockStamp) -> EntryFacts {
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

fn reason_text(reason: ExpiryReason) -> String {
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
                    free_with_journal: space.available,
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
            entries.push(facts_of(&part, &manifest, basis_of(&manifest.time_basis, &age, &now)));
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
        fs.sync_dir(&scope, &reap_dir)?;
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
        Ok(manifest::write_atomic(self.fs(), &scope, &parent, &path, content)?)
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
        Err(user_config_unavailable())
    }

    fn write_user_config(&self, _content: &str) -> Result<OpId> {
        Err(user_config_unavailable())
    }
}

fn user_config_unavailable() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "the journal store has no access to the user configuration file; use scope: session",
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::StepLimit;
    use nurt_contracts::fs::ContentHash;
    use nurt_contracts::journal::{OpKind, PurgeSelector};

    use super::*;
    use crate::manifest::{IndexRow, Integrity};
    use crate::store::test_support::Fixture;

    /// Writes committed entry `op` of `bytes` held on volume `index` and its
    /// index row.
    fn commit(fixture: &Fixture, index: usize, op: u64, bytes: u64) {
        let journal = JournalLayout::new(fixture.journal_dir(index));
        for dir in journal.structure().unwrap() {
            std::fs::create_dir_all(dir.as_path()).unwrap();
        }
        let entry = journal.entry(OpId(op)).unwrap();
        std::fs::create_dir_all(entry.dir().as_path()).unwrap();
        let stamp = fixture.clock.stamp();
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
            bytes_held: bytes,
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
        fixture.write(&entry.manifest().unwrap(), &manifest.encode().unwrap());
        fixture.write(
            &entry.state().unwrap(),
            &StateRecord::new(EntryState::Committed).encode(),
        );
        let row = IndexRow {
            op: OpId(op),
            time: stamp.wall,
            kind: OpKind::command("write"),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: Vec::new(),
        };
        let index = fixture.ctx().state.index().unwrap();
        let mut rows = if fixture.exists(&index) {
            fixture.read(&index)
        } else {
            Vec::new()
        };
        rows.extend(row.encode().unwrap());
        fixture.write(&index, &rows);
    }

    #[test]
    fn sweep_reclaims_the_entry_over_the_step_limit() {
        let fixture = Fixture::new();
        for op in 1..=31 {
            commit(&fixture, 0, op, 10);
        }
        let ctx = fixture.ctx();
        let report = sweep(&ctx).unwrap();
        assert_eq!(
            report.expired,
            [ExpiredEntry {
                op: OpId(1),
                reason: ExpiryReason::Ops
            }]
        );
        assert_eq!(report.bytes, 10);
        let journal = JournalLayout::new(fixture.journal_dir(0));
        assert!(!fixture.exists(journal.entry(OpId(1)).unwrap().dir()));
        assert!(!fixture.exists(journal.reaped(OpId(1)).unwrap().dir()));
        assert!(fixture.exists(&journal.tomb(OpId(1)).unwrap()));
        assert!(fixture.exists(journal.entry(OpId(2)).unwrap().dir()));
        assert!(fixture.exists(&fixture.ctx().state.root().join_name(AGE_STATE_FILE).unwrap()));
        assert!(sweep(&ctx).unwrap().expired.is_empty());
    }

    #[test]
    fn complete_reap_finishes_a_moved_entry_once() {
        let fixture = Fixture::new();
        commit(&fixture, 0, 5, 7);
        let ctx = fixture.ctx();
        let store = CtxStore(&ctx);
        let volume = fixture.fs.volume_at(0).mount;
        store.begin_reap(&volume, OpId(5), ExpiryReason::Time).unwrap();
        let journal = JournalLayout::new(fixture.journal_dir(0));
        assert!(fixture.exists(journal.reaped(OpId(5)).unwrap().dir()));

        let first = complete_reap(&ctx, OpId(5)).unwrap();
        assert_eq!(
            first,
            ExpiredEntry {
                op: OpId(5),
                reason: ExpiryReason::Time
            }
        );
        assert!(!fixture.exists(journal.reaped(OpId(5)).unwrap().dir()));
        let tombstone = tomb::read(&store, &volume, OpId(5)).unwrap().unwrap();
        assert_eq!(tombstone.bytes, 7);
        assert_eq!(complete_reap(&ctx, OpId(5)).unwrap(), first);
        assert_eq!(
            complete_reap(&ctx, OpId(6)).unwrap_err().code(),
            ErrorCode::JournalNoop
        );
    }

    #[test]
    fn status_and_purge_work_on_the_journal_directories() {
        let fixture = Fixture::new();
        for op in 1..=3 {
            commit(&fixture, 0, op, 4);
        }
        commit(&fixture, 1, 4, 4);
        let ctx = fixture.ctx();
        let states = status(&ctx, None).unwrap();
        assert_eq!(states.len(), 2);
        let first = states
            .iter()
            .find(|state| state.volume == fixture.fs.volume_at(0).mount)
            .unwrap();
        assert_eq!(first.entries_available, 3);
        assert_eq!(first.bytes_held, 12);
        assert_eq!(first.limits.max_ops.value, StepLimit::Limited(30));

        let outcome = purge(
            &ctx,
            &PurgeRequest {
                volume: Some(fixture.fs.volume_at(0).mount),
                selector: PurgeSelector::UpTo(OpId(2)),
            },
        )
        .unwrap();
        assert_eq!(outcome.released, [OpId(1), OpId(2)]);
        assert!(fixture.exists(&ctx.state.root().join_name(AUDIT_FILE).unwrap()));
        let error = limits(
            &ctx,
            &LimitsRequest {
                steps: Some(StepLimit::Limited(10)),
                age: None,
                size: None,
                scope: nurt_contracts::journal::LimitsScope::User,
                volume: None,
                preview: false,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
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
