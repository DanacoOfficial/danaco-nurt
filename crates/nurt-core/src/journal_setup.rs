// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Assembly of the journal of the running process.
//!
//! The journal is built over the system file system with space admission
//! control, the volume directories of the account and the retention limits of
//! the configuration layers. No space service runs inside the process, so the
//! link to it reports the service as not running and admission decides on its
//! own.

use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use nurt_contracts::config::ConfigLayer;
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileLock, Fs, FsWrite, VolumeId};
use nurt_contracts::journal::{ExpiredEntry, ExpiryReason, JournalStore, OpId, VolumeState};
use nurt_fs::failpoint::NoFailpoints;
use nurt_journal::store::{EntryCatalog, EntrySummary, HeldItem, ReclaimPaths};
use nurt_journal::volumes::{JournalDirectories, Registry};
use nurt_journal::{ConfiguredLimits, Journal, JournalBuilder};
use nurt_space::admission::SpaceAdmission;
use nurt_space::service::{RegistryLink, ServiceProbe};

use crate::config::LoadedConfig;
use crate::error::EngineError;

/// File of the user limits of the journal in the state directory.
const USER_LIMITS_FILE: &str = "user-config.toml";

/// Probe of a space service that does not run in this process.
#[derive(Debug, Clone, Copy)]
struct NoService;

impl ServiceProbe for NoService {
    fn process_alive(&self, _pid: u32) -> bool {
        false
    }

    fn ping(&self, _timeout: Duration) -> bool {
        false
    }

    fn wake(&self, _volume: &VolumeId, _state: VolumeState) {}
}

/// Catalog of the entries of the journal, bound once the journal is built
/// because admission control and the journal refer to each other.
#[derive(Default)]
struct LateCatalog {
    journal: OnceLock<Weak<Journal>>,
}

impl LateCatalog {
    fn bind(&self, journal: &Arc<Journal>) {
        self.journal.get_or_init(|| Arc::downgrade(journal));
    }

    fn journal(&self) -> Result<Arc<Journal>> {
        self.journal
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Error::new(ErrorCode::ServiceNotRunning, "the journal is closed"))
    }
}

impl EntryCatalog for LateCatalog {
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

/// Builds the journal over the state directory `state_root`.
///
/// # Errors
///
/// Returns the error of the journal when the state directory is not usable or
/// a configuration value of the retention limits is invalid.
pub(crate) fn open_journal(
    layers: &[ConfigLayer],
    loaded: &LoadedConfig,
    project_trusted: bool,
    clock: Arc<dyn Clock>,
    state_root: &Path,
) -> std::result::Result<Arc<dyn JournalStore>, EngineError> {
    build(layers, loaded, project_trusted, clock, state_root)
        .map(|journal| journal as Arc<dyn JournalStore>)
        .map_err(|error| EngineError::journal(&error))
}

fn build(
    layers: &[ConfigLayer],
    loaded: &LoadedConfig,
    project_trusted: bool,
    clock: Arc<dyn Clock>,
    state_root: &Path,
) -> Result<Arc<Journal>> {
    let state_dir = AbsPath::new(state_root.to_path_buf()).map_err(Error::from)?;
    let fs: Arc<dyn FsWrite> = nurt_fs::system_fs();
    let reader: Arc<dyn Fs> = fs.clone();
    let settings = loaded.config();
    let catalog = Arc::new(LateCatalog::default());
    let link = Arc::new(RegistryLink::new(
        reader.clone(),
        clock.clone(),
        settings.journal.clone(),
        settings.space.clone(),
        Registry::new(state_dir.clone()),
        Arc::new(NoService),
    ));
    let admission = Arc::new(SpaceAdmission::new(
        fs.clone(),
        clock.clone(),
        Arc::new(NoFailpoints),
        link,
        catalog.clone(),
        settings.space.clone(),
    ));
    let limits = ConfiguredLimits::new(layers, project_trusted, reader, &state_dir)?;
    let journals = JournalDirectories::new(
        fs.clone(),
        settings.journal.clone(),
        Registry::new(state_dir.clone()),
        vec![state_dir.clone()],
    );
    let user_limits = state_dir.join_name(USER_LIMITS_FILE).map_err(Error::from)?;
    let journal = Arc::new(
        JournalBuilder::new(
            fs,
            admission.clone(),
            clock,
            Arc::new(journals),
            Arc::new(limits),
            state_dir,
        )
        .user_config(user_limits)
        .space_reporter(admission)
        .build()?,
    );
    catalog.bind(&journal);
    Ok(journal)
}
