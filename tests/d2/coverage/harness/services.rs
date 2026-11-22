// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Services of the journal under test and their assembly.
//!
//! The journal store is built by `JournalBuilder` over the file system of the
//! volumes wrapped by the mutation audit, with the journal directories of the
//! account (`JournalDirectories`), the retention limits of the configuration
//! levels (`ConfiguredLimits`) and admission control of the space service
//! (`SpaceAdmission`), which also reports `journal.space`. The space service
//! of a fixture runs in the process of the test: a wake reclaims at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use nurt_contracts::config::{ConfigLayer, ConfigLevel, JournalConfig};
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, FileLock, Fs, FsError, FsErrorKind, FsResult, FsWrite, Provider, ProviderCaps,
    ProviderChild, ProviderKey, ProviderScheme, ProviderSnapshot, ProviderValue, ProviderWrite,
    TxScope, VolumeId, VolumeInfo,
};
use nurt_contracts::journal::{
    Eligibility, ExpiredEntry, ExpiryReason, JournalStore, OpId, ServiceLiveness, VolumeState,
};
use nurt_fs::audit::{AuditFs, StateOracle, WriteClass};
use nurt_fs::failpoint::NoFailpoints;
use nurt_journal::layout::JournalLayout;
use nurt_journal::manifest::read_state;
use nurt_journal::store::{EntryCatalog, EntrySummary, HeldItem, ReclaimPaths};
use nurt_journal::volumes::{JournalDirectories, Registry, evaluate_eligibility};
use nurt_journal::{ConfiguredLimits, Journal, JournalBuilder};
use nurt_space::admission::{ServiceLink, SpaceAdmission};
use serde_json::{Map, Value};

use super::fixture::FixtureOptions;
use super::volumes::Volumes;

/// Scheme of the writable provider that serves value targets.
pub(crate) const TEST_SCHEME: &str = "test";

/// Name of the user configuration file in the state directory.
const USER_CONFIG_FILE: &str = "user-config.toml";

/// Longest reclaim pass of the space service after a wake.
const RECLAIM_BUDGET: Duration = Duration::from_secs(5);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Journal store bound to the volumes of a fixture.
pub(crate) struct Bound {
    /// Journal store.
    pub(crate) journal: Arc<dyn JournalStore>,
    /// Mutation audit of the file system below the journal.
    pub(crate) audit: Arc<AuditFs<dyn FsWrite>>,
}

/// File system operations on user data recorded by the mutation audit:
/// renames and removals outside the journal and state directories.
pub(crate) fn user_data_calls(audit: &AuditFs<dyn FsWrite>) -> (u64, u64) {
    let records = audit.recorded();
    let user = records
        .iter()
        .filter(|record| record.class != WriteClass::Internal);
    let mut renames = 0;
    let mut removals = 0;
    for record in user {
        match record.method {
            "rename" => renames += 1,
            "remove_file" | "remove_dir" | "remove_tree" => removals += 1,
            _ => {}
        }
    }
    (renames, removals)
}

/// Builds a journal over `volumes`. Failpoints of a run with power loss
/// apply to the first instance only: the instance after a restart recovers
/// without faults.
pub(crate) fn bind(
    options: &FixtureOptions,
    volumes: &Volumes,
    clock: Arc<dyn Clock>,
    values: Arc<ValueProvider>,
    first: bool,
) -> Bound {
    let raw = volumes.fs();
    let mut anchored = volumes.roots().to_vec();
    anchored.push(volumes.state_dir().clone());
    let journal_dirs = journal_dirs(raw.as_ref(), &anchored, &options.journal);
    let audit = Arc::new(AuditFs::new(
        raw.clone(),
        Arc::new(EntryStates {
            fs: raw.clone(),
            dirs: journal_dirs.clone(),
        }),
    ));
    for dir in journal_dirs {
        audit.add_internal_root(dir);
    }
    audit.add_internal_root(volumes.state_dir().clone());
    let fs: Arc<dyn FsWrite> = audit.clone();

    let catalog = Arc::new(JournalCatalog::default());
    let link = Arc::new(FixtureLink {
        fs: raw.clone(),
        journal: options.journal.clone(),
        volumes: volumes.roots().to_vec(),
        alive: options.service_alive,
        service: OnceLock::new(),
    });
    let admission = Arc::new(SpaceAdmission::new(
        fs.clone(),
        clock.clone(),
        Arc::new(NoFailpoints),
        link.clone(),
        catalog.clone(),
        options.space.clone(),
    ));
    link.serve(&admission);

    let limits = ConfiguredLimits::new(
        &layers(options),
        options.project_trusted,
        raw.clone(),
        volumes.state_dir(),
    )
    .expect("retention settings of the fixture are valid");
    let journals = JournalDirectories::new(
        fs.clone(),
        options.journal.clone(),
        Registry::new(volumes.state_dir().clone()),
        Vec::new(),
    );
    let user_config = volumes
        .state_dir()
        .join_name(USER_CONFIG_FILE)
        .expect("user configuration file name is a single component");
    let mut builder = JournalBuilder::new(
        fs,
        admission.clone(),
        clock,
        Arc::new(journals),
        Arc::new(limits),
        volumes.state_dir().clone(),
    )
    .overwrite(options.journal.overwrite)
    .user_config(user_config)
    .space_reporter(admission)
    .provider(values);
    if first && let Some(failpoints) = volumes.failpoints() {
        builder = builder.failpoints(failpoints);
    }
    let journal = Arc::new(builder.build().expect("journal store is built"));
    catalog.bind(&journal);
    Bound { journal, audit }
}

/// Journal directory of every volume: the directory of the configured name
/// at the mount point, where the journal anchors it.
fn journal_dirs(fs: &dyn Fs, roots: &[AbsPath], config: &JournalConfig) -> Vec<AbsPath> {
    roots
        .iter()
        .map(|root| {
            fs.volume(root)
                .expect("volume of a fixture root is known")
                .mount
                .join_name(&config.dir_name)
                .expect("journal directory name is a single component")
        })
        .collect()
}

/// Durable state of an entry, read from its `STATE` file in any journal
/// directory of the fixture.
struct EntryStates {
    fs: Arc<dyn FsWrite>,
    dirs: Vec<AbsPath>,
}

impl StateOracle for EntryStates {
    fn state(&self, op: OpId) -> Option<nurt_contracts::journal::EntryState> {
        self.dirs.iter().find_map(|dir| {
            let entry = JournalLayout::new(dir.clone()).entry(op).ok()?;
            read_state(self.fs.as_ref(), &entry.state().ok()?)
                .ok()
                .map(|record| record.state)
        })
    }
}

/// Configuration layers of the fixture: the journal settings of the options
/// at the user level, then every setting at its level.
fn layers(options: &FixtureOptions) -> Vec<ConfigLayer> {
    let mut maps: Vec<Map<String, Value>> = ConfigLevel::ALL.iter().map(|_| Map::new()).collect();
    let user = changed_journal_settings(&options.journal);
    if !user.is_empty() {
        maps[level_index(ConfigLevel::User)].insert("journal".to_owned(), Value::Object(user));
    }
    for setting in &options.settings {
        insert_key(
            &mut maps[level_index(setting.level)],
            setting.key,
            setting_value(&setting.value),
        );
    }
    ConfigLevel::ALL
        .iter()
        .zip(maps)
        .filter(|(level, map)| **level != ConfigLevel::Builtin && !map.is_empty())
        .map(|(level, map)| ConfigLayer::new(*level, format!("{}.toml", level.as_str()), map))
        .collect()
}

fn level_index(level: ConfigLevel) -> usize {
    ConfigLevel::ALL
        .iter()
        .position(|candidate| *candidate == level)
        .expect("every level is listed")
}

/// Journal settings that differ from the built-in defaults, so that the
/// built-in level stays the source of the others.
fn changed_journal_settings(journal: &JournalConfig) -> Map<String, Value> {
    let actual = serde_json::to_value(journal).expect("journal settings serialize");
    let builtin =
        serde_json::to_value(JournalConfig::default()).expect("journal settings serialize");
    let (Value::Object(actual), Value::Object(builtin)) = (actual, builtin) else {
        panic!("journal settings serialize as a table");
    };
    actual
        .into_iter()
        .filter(|(key, value)| builtin.get(key) != Some(value))
        .collect()
}

/// Value of a setting: a JSON value, or the text itself when it is not one
/// (`40m`, `21gib`).
fn setting_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

/// Inserts `value` under a dotted key such as `journal.floor.max_ops`.
fn insert_key(map: &mut Map<String, Value>, key: &str, value: Value) {
    match key.split_once('.') {
        None => {
            map.insert(key.to_owned(), value);
        }
        Some((table, rest)) => {
            let inner = map
                .entry(table.to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            let Value::Object(inner) = inner else {
                panic!("setting {table} is not a table");
            };
            insert_key(inner, rest, value);
        }
    }
}

/// Entry catalog of the journal for admission control, bound once the
/// journal exists; admission is built before the journal it serves.
#[derive(Default)]
struct JournalCatalog {
    journal: OnceLock<Weak<Journal>>,
}

impl JournalCatalog {
    fn bind(&self, journal: &Arc<Journal>) {
        self.journal.get_or_init(|| Arc::downgrade(journal));
    }

    fn journal(&self) -> Result<Arc<Journal>> {
        self.journal.get().and_then(Weak::upgrade).ok_or_else(|| {
            Error::new(
                ErrorCode::ServiceNotRunning,
                "the journal of the fixture is closed",
            )
        })
    }
}

impl EntryCatalog for JournalCatalog {
    fn lock_entry(&self, volume: &VolumeId, op: OpId) -> Result<Box<dyn FileLock>> {
        self.journal()?.lock_entry(volume, op)
    }

    fn entries(&self, volume: &VolumeId) -> Result<Vec<EntrySummary>> {
        self.journal()?.entries(volume)
    }

    fn held_items(&self, volume: &VolumeId, op: OpId) -> Result<Vec<HeldItem>> {
        self.journal()?.held_items(volume, op)
    }

    fn begin_reclaim(
        &self,
        volume: &VolumeId,
        op: OpId,
        reason: ExpiryReason,
    ) -> Result<ReclaimPaths> {
        self.journal()?.begin_reclaim(volume, op, reason)
    }

    fn complete_reclaim(&self, volume: &VolumeId, op: OpId) -> Result<ExpiredEntry> {
        self.journal()?.complete_reclaim(volume, op)
    }

    fn mark_released(&self, volume: &VolumeId, op: OpId, items: &[u32]) -> Result<()> {
        self.journal()?.mark_released(volume, op, items)
    }

    fn finish_release(&self, volume: &VolumeId, op: OpId) -> Result<()> {
        self.journal()?.finish_release(volume, op)
    }
}

/// Space service of the fixture as seen by admission control: the volumes
/// of the fixture, the eligibility rules of the journal settings, liveness
/// from the options, and a reclaim pass at every wake while it runs.
struct FixtureLink {
    fs: Arc<dyn FsWrite>,
    journal: JournalConfig,
    volumes: Vec<AbsPath>,
    alive: bool,
    service: OnceLock<Weak<SpaceAdmission>>,
}

impl FixtureLink {
    fn serve(&self, admission: &Arc<SpaceAdmission>) {
        self.service.get_or_init(|| Arc::downgrade(admission));
    }
}

impl ServiceLink for FixtureLink {
    fn eligibility(&self, volume: &VolumeInfo) -> Eligibility {
        evaluate_eligibility(volume, &self.journal)
    }

    fn liveness(&self) -> ServiceLiveness {
        if self.alive {
            ServiceLiveness::Alive
        } else {
            ServiceLiveness::Down
        }
    }

    fn volumes(&self) -> Vec<VolumeInfo> {
        self.volumes
            .iter()
            .filter_map(|root| self.fs.volume(root).ok())
            .collect()
    }

    fn wake(&self, volume: &VolumeId, _state: VolumeState) {
        if !self.alive {
            return;
        }
        if let Some(service) = self.service.get().and_then(Weak::upgrade) {
            // A failed pass leaves the volume to the next wake, as the
            // service retries at its next tick.
            let _ = service.reclaim(volume, RECLAIM_BUDGET);
        }
    }
}

/// Writable provider of values under the scheme [`TEST_SCHEME`], kept in
/// memory across restarts of the journal.
pub(crate) struct ValueProvider {
    values: Mutex<HashMap<ProviderKey, ProviderValue>>,
}

impl ValueProvider {
    /// Provider without values.
    pub(crate) fn new() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
        }
    }
}

impl Provider for ValueProvider {
    fn scheme(&self) -> ProviderScheme {
        ProviderScheme(TEST_SCHEME.to_owned())
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            writable: true,
            hierarchical: false,
            persistent: true,
        }
    }

    fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>> {
        Ok(lock(&self.values).get(key).cloned())
    }

    fn list(&self, _key: &ProviderKey) -> FsResult<Vec<ProviderChild>> {
        Ok(Vec::new())
    }

    fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot> {
        let value = lock(&self.values).get(key).cloned();
        Ok(ProviderSnapshot {
            key: key.clone(),
            existed: value.is_some(),
            value,
            subtree: None,
        })
    }
}

impl ProviderWrite for ValueProvider {
    fn set(&self, _scope: &TxScope, key: &ProviderKey, value: &ProviderValue) -> FsResult<()> {
        lock(&self.values).insert(key.clone(), value.clone());
        Ok(())
    }

    fn remove(&self, _scope: &TxScope, key: &ProviderKey) -> FsResult<()> {
        lock(&self.values)
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "remove", None))
    }

    fn restore(&self, _scope: &TxScope, snapshot: &ProviderSnapshot) -> FsResult<()> {
        let mut values = lock(&self.values);
        match &snapshot.value {
            Some(value) => values.insert(snapshot.key.clone(), value.clone()),
            None => values.remove(&snapshot.key),
        };
        Ok(())
    }
}
