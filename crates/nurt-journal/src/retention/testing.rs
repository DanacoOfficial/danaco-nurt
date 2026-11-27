// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-memory journal store and virtual clock for retention tests.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use nurt_contracts::config::{ConfigLayer, ConfigLevel};
use nurt_contracts::env::{BootId, ClockStamp, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileLock};
use nurt_contracts::journal::{EntryState, ExpiryReason, OpId, OpKind};
use nurt_fs::failpoint::FailPoint;
use serde_json::Value;

use super::age::EntryTimeBasis;
use super::port::{Area, EntryFacts, RetentionStore, StepKind, StepMark, VolumeFacts};

/// Wall time of operation 0: 2026-01-01T00:00:00Z, in seconds.
pub(crate) const START: i64 = 1_767_225_600;
/// One gibibyte.
pub(crate) const GIB: u64 = 1 << 30;
/// One mebibyte.
pub(crate) const MIB: u64 = 1 << 20;
const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// Absolute path from a literal.
pub(crate) fn mount(path: &str) -> AbsPath {
    AbsPath::for_test(path)
}

/// Clock readings at `secs` seconds after [`START`] in boot `boot-a`, with
/// both monotonic clocks equal to `secs`.
pub(crate) fn stamp_at(secs: u64) -> ClockStamp {
    ClockStamp {
        wall: WallTime::from_unix_nanos((i128::from(START) + i128::from(secs)) * NANOS_PER_SECOND),
        boot: BootId("boot-a".to_owned()),
        since_boot_with_suspend: Duration::from_secs(secs),
        since_boot_active: Duration::from_secs(secs),
    }
}

/// Committed entry `op` of one item of 1 MiB on `volume`, committed `op`
/// seconds after [`START`].
pub(crate) fn entry(op: u64, volume: &str) -> EntryFacts {
    let stamp = stamp_at(op);
    EntryFacts {
        op: OpId(op),
        volume: mount(volume),
        state: EntryState::Committed,
        reason: None,
        account: None,
        kind: OpKind::command("write"),
        time: stamp.wall,
        basis: EntryTimeBasis {
            effective_wall: stamp.wall,
            active_at_commit: stamp.since_boot_active,
            committed: stamp,
        },
        files: 1,
        bytes_held: MIB,
        bytes_physical: MIB,
        irreversible: Vec::new(),
        pinned: false,
        rolled_back: false,
    }
}

#[derive(Default)]
struct Inner {
    layers: Vec<ConfigLayer>,
    trusted: bool,
    clock_secs: u64,
    volumes: Vec<VolumeFacts>,
    entries: BTreeMap<(AbsPath, OpId), EntryFacts>,
    steps: Vec<StepMark>,
    files: BTreeMap<(Option<AbsPath>, String), Vec<u8>>,
    audit: Vec<Value>,
    user_config: Option<String>,
    user_writes: Vec<OpId>,
    fail_at: Option<FailPoint>,
    next_op: u64,
    locks: usize,
    account: Option<String>,
}

/// Journal store in memory.
pub(crate) struct MemStore {
    inner: Mutex<Inner>,
}

impl MemStore {
    /// Empty store without volumes, at [`START`].
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Sets the account that runs the retention call.
    pub(crate) fn set_account(&self, account: &str) {
        self.lock().account = Some(account.to_owned());
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// Adds a volume of `capacity` bytes with `free` bytes free including
    /// the journal.
    pub(crate) fn volume(&self, path: &str, capacity: u64, free: u64) {
        let mut inner = self.lock();
        inner.volumes.push(VolumeFacts {
            mount: mount(path),
            capacity,
            free,
            inactive: None,
        });
        inner.volumes.sort_by(|a, b| a.mount.cmp(&b.mount));
    }

    /// Adds a configuration layer.
    pub(crate) fn layer(&self, level: ConfigLevel, values: Value) {
        let values = values.as_object().cloned().unwrap_or_default();
        self.lock()
            .layers
            .push(ConfigLayer::new(level, format!("{level}.toml"), values));
    }

    /// Adds an entry part and its step mark.
    pub(crate) fn add(&self, facts: EntryFacts, kind: StepKind) {
        let mut inner = self.lock();
        let op = facts.op;
        if !inner.steps.iter().any(|mark| mark.op == op) {
            inner.steps.push(StepMark { op, kind });
            inner.steps.sort_by_key(|mark| mark.op);
        }
        inner.next_op = inner.next_op.max(op.0);
        inner.entries.insert((facts.volume.clone(), op), facts);
    }

    /// Adds committed operations `ops` of 1 MiB each on `volume`.
    pub(crate) fn commit(&self, volume: &str, ops: impl IntoIterator<Item = u64>) {
        for op in ops {
            self.add(entry(op, volume), StepKind::Operation);
        }
    }

    /// Sets the virtual clock to `secs` seconds after [`START`].
    pub(crate) fn set_clock(&self, secs: u64) {
        self.lock().clock_secs = secs;
    }

    /// Makes the given fault injection point fail.
    pub(crate) fn fail_at(&self, point: FailPoint) {
        self.lock().fail_at = Some(point);
    }

    /// Numbers of the live entries (not expiring) of `volume`.
    pub(crate) fn live(&self, volume: &str) -> Vec<u64> {
        let volume = mount(volume);
        self.lock()
            .entries
            .values()
            .filter(|facts| facts.volume == volume && facts.state != EntryState::Expiring)
            .map(|facts| facts.op.0)
            .collect()
    }

    /// State of an entry part.
    pub(crate) fn state(&self, volume: &str, op: u64) -> Option<EntryState> {
        self.lock()
            .entries
            .get(&(mount(volume), OpId(op)))
            .map(|facts| facts.state)
    }

    /// Content of a file; panics when it does not exist.
    pub(crate) fn file(&self, area: Area<'_>, name: &str) -> Vec<u8> {
        self.read_file(area, name).unwrap().unwrap()
    }

    /// Number of entry locks taken so far.
    pub(crate) fn locks_taken(&self) -> usize {
        self.lock().locks
    }

    /// Audit records appended so far.
    pub(crate) fn audit_records(&self) -> Vec<Value> {
        self.lock().audit.clone()
    }

    /// Sets the content of the user configuration file.
    pub(crate) fn set_user_config(&self, content: &str) {
        self.lock().user_config = Some(content.to_owned());
    }

    /// Content of the user configuration file.
    pub(crate) fn user_config(&self) -> Option<String> {
        self.lock().user_config.clone()
    }

    /// Operations that wrote the user configuration file.
    pub(crate) fn user_writes(&self) -> Vec<OpId> {
        self.lock().user_writes.clone()
    }

    fn key(area: Area<'_>, name: &str) -> (Option<AbsPath>, String) {
        match area {
            Area::Journal(volume) => (Some(volume.clone()), name.to_owned()),
            Area::State => (None, name.to_owned()),
        }
    }
}

/// Lock of an entry held by the in-memory store.
struct MemLock(AbsPath);

impl FileLock for MemLock {
    fn path(&self) -> &AbsPath {
        &self.0
    }
}

impl RetentionStore for MemStore {
    fn account_id(&self) -> Option<String> {
        self.lock().account.clone()
    }

    fn lock_entry(&self, volume: &AbsPath, _op: OpId) -> Result<Box<dyn FileLock>> {
        self.lock().locks += 1;
        Ok(Box::new(MemLock(volume.clone())))
    }

    fn config_layers(&self) -> Vec<ConfigLayer> {
        self.lock().layers.clone()
    }

    fn project_trusted(&self) -> bool {
        self.lock().trusted
    }

    fn clock(&self) -> ClockStamp {
        stamp_at(self.lock().clock_secs)
    }

    fn volumes(&self) -> Result<Vec<VolumeFacts>> {
        Ok(self.lock().volumes.clone())
    }

    fn entries(&self, volume: &AbsPath) -> Result<Vec<EntryFacts>> {
        Ok(self
            .lock()
            .entries
            .values()
            .filter(|facts| &facts.volume == volume)
            .cloned()
            .collect())
    }

    fn steps(&self) -> Result<Vec<StepMark>> {
        Ok(self.lock().steps.clone())
    }

    fn begin_reap(&self, volume: &AbsPath, op: OpId, reason: ExpiryReason) -> Result<()> {
        let mut inner = self.lock();
        let facts = inner
            .entries
            .get_mut(&(volume.clone(), op))
            .ok_or_else(|| Error::new(ErrorCode::Io, format!("no entry {} to reap", op.0)))?;
        facts.state = EntryState::Expiring;
        facts.reason = Some(reason);
        Ok(())
    }

    fn remove_reaped(&self, volume: &AbsPath, op: OpId) -> Result<()> {
        let mut inner = self.lock();
        let key = (volume.clone(), op);
        if inner
            .entries
            .get(&key)
            .is_some_and(|facts| facts.state == EntryState::Expiring)
        {
            inner.entries.remove(&key);
        }
        Ok(())
    }

    fn hit(&self, point: FailPoint) -> Result<()> {
        if self.lock().fail_at == Some(point) {
            return Err(Error::new(
                ErrorCode::Io,
                format!("injected at {}", point.as_str()),
            ));
        }
        Ok(())
    }

    fn read_file(&self, area: Area<'_>, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.lock().files.get(&Self::key(area, name)).cloned())
    }

    fn write_file(&self, area: Area<'_>, name: &str, content: &[u8]) -> Result<()> {
        self.lock()
            .files
            .insert(Self::key(area, name), content.to_vec());
        Ok(())
    }

    fn remove_file(&self, area: Area<'_>, name: &str) -> Result<()> {
        self.lock().files.remove(&Self::key(area, name));
        Ok(())
    }

    fn list_dir(&self, area: Area<'_>, dir: &str) -> Result<Vec<String>> {
        let (volume, _) = Self::key(area, dir);
        let prefix = format!("{dir}/");
        Ok(self
            .lock()
            .files
            .keys()
            .filter(|(owner, _)| *owner == volume)
            .filter_map(|(_, name)| name.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_owned)
            .collect())
    }

    fn audit(&self, record: &Value) -> Result<()> {
        self.lock().audit.push(record.clone());
        Ok(())
    }

    fn read_user_config(&self) -> Result<Option<String>> {
        Ok(self.lock().user_config.clone())
    }

    fn write_user_config(&self, content: &str) -> Result<OpId> {
        let mut inner = self.lock();
        inner.next_op += 1;
        let op = OpId(inner.next_op);
        inner.user_config = Some(content.to_owned());
        inner.user_writes.push(op);
        inner.steps.push(StepMark {
            op,
            kind: StepKind::NotCounted,
        });
        Ok(op)
    }
}
