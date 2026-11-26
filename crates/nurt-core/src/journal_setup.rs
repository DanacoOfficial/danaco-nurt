// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Assembly of the journal of the running process.
//!
//! The journal is built over the system file system with space admission
//! control, the volume directories of the account and the retention limits of
//! the configuration layers. The space service runs inside the process: the
//! journal starts its module, which recovers the journal and writes the
//! heartbeats that admission requires before it preserves data.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, TryLockError, Weak};
use std::time::Duration;

use nurt_contracts::config::ConfigLayer;
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileLock, Fs, FsWrite, VolumeId, VolumeInfo};
use nurt_contracts::journal::{
    Eligibility, ExpiredEntry, ExpiryReason, JournalStore, OpId, RecoveryReport, ServiceLiveness,
    SweepReport, VolumeState,
};
use nurt_fs::failpoint::NoFailpoints;
use nurt_journal::store::{EntryCatalog, EntrySummary, HeldItem, ReclaimPaths};
use nurt_journal::volumes::{JournalDirectories, Registry};
use nurt_journal::{ConfiguredLimits, Journal, JournalBuilder};
use nurt_space::admission::{ServiceLink, SpaceAdmission};
use nurt_space::service::{
    JournalMaintenance, RegistryLink, ServiceProbe, SpaceModule, SpaceService, SpaceWorker,
};
use nurt_sys::registry::RegistryProvider;

use crate::config::LoadedConfig;
use crate::error::EngineError;

/// File of the user limits of the journal in the state directory.
const USER_LIMITS_FILE: &str = "user-config.toml";

/// Space module hosted by this process.
type ProcessModule = Mutex<SpaceModule<LateCatalog>>;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Transport to the space module that runs inside this process.
struct ProcessProbe {
    module: Arc<ProcessModule>,
}

impl ServiceProbe for ProcessProbe {
    fn process_alive(&self, pid: u32) -> bool {
        pid == std::process::id()
    }

    fn ping(&self, _timeout: Duration) -> bool {
        match self.module.try_lock() {
            Ok(module) => module.ping() == ServiceLiveness::Alive,
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().ping() == ServiceLiveness::Alive
            }
            Err(TryLockError::WouldBlock) => true,
        }
    }

    fn wake(&self, _volume: &VolumeId, _state: VolumeState) {}
}

/// Link to the space module of this process. Admission asks for the liveness
/// before the journal directory of a volume is recorded, so the link answers
/// from the module itself after it refreshes the heartbeats.
struct ProcessLink {
    inner: RegistryLink,
    module: Arc<ProcessModule>,
}

impl ServiceLink for ProcessLink {
    fn eligibility(&self, volume: &VolumeInfo) -> Eligibility {
        self.inner.eligibility(volume)
    }

    fn liveness(&self) -> ServiceLiveness {
        match self.module.try_lock() {
            Ok(mut module) => {
                module.refresh();
                module.ping()
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut module = poisoned.into_inner();
                module.refresh();
                module.ping()
            }
            Err(TryLockError::WouldBlock) => ServiceLiveness::Alive,
        }
    }

    fn volumes(&self) -> Vec<VolumeInfo> {
        self.inner.volumes()
    }

    fn wake(&self, volume: &VolumeId, state: VolumeState) {
        self.inner.wake(volume, state);
    }
}

/// Reclaim worker bound once the admission control is built.
#[derive(Default)]
struct LateWorker {
    admission: OnceLock<Weak<SpaceAdmission>>,
}

impl LateWorker {
    fn admission(&self) -> Result<Arc<SpaceAdmission>> {
        self.admission
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Error::new(ErrorCode::ServiceNotRunning, "the journal is closed"))
    }
}

impl SpaceWorker for LateWorker {
    fn calibrate(&self, volume: &VolumeInfo, scratch: &AbsPath) -> Result<()> {
        SpaceWorker::calibrate(&*self.admission()?, volume, scratch)
    }

    fn state(&self, volume: &VolumeInfo) -> VolumeState {
        self.admission().map_or(VolumeState::Normal, |admission| {
            SpaceWorker::state(&*admission, volume)
        })
    }

    fn reclaim(&self, volume: &VolumeId, budget: Duration) -> Result<()> {
        SpaceWorker::reclaim(&*self.admission()?, volume, budget)
    }
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

impl JournalMaintenance for LateCatalog {
    fn recover(&self) -> Result<RecoveryReport> {
        JournalStore::recover(&*self.journal()?)
    }

    fn sweep(&self) -> Result<SweepReport> {
        JournalStore::sweep(&*self.journal()?)
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
    registry: Option<Arc<RegistryProvider>>,
) -> std::result::Result<Arc<dyn JournalStore>, EngineError> {
    build(layers, loaded, project_trusted, clock, state_root, registry)
        .map(|journal| journal as Arc<dyn JournalStore>)
        .map_err(|error| EngineError::journal(&error))
}

fn with_providers(
    builder: JournalBuilder,
    registry: Option<Arc<RegistryProvider>>,
) -> JournalBuilder {
    match registry {
        Some(registry) => builder.provider(registry),
        None => builder,
    }
}

fn build(
    layers: &[ConfigLayer],
    loaded: &LoadedConfig,
    project_trusted: bool,
    clock: Arc<dyn Clock>,
    state_root: &Path,
    registry: Option<Arc<RegistryProvider>>,
) -> Result<Arc<Journal>> {
    let state_dir = AbsPath::new(state_root.to_path_buf()).map_err(Error::from)?;
    let fs: Arc<dyn FsWrite> = nurt_fs::system_fs();
    let reader: Arc<dyn Fs> = fs.clone();
    let settings = loaded.config();
    let catalog = Arc::new(LateCatalog::default());
    let worker = Arc::new(LateWorker::default());
    let module: Arc<ProcessModule> = Arc::new(Mutex::new(SpaceModule::new(
        fs.clone(),
        clock.clone(),
        catalog.clone(),
        worker.clone(),
        Registry::new(state_dir.clone()),
        settings.space.clone(),
    )));
    let link = Arc::new(ProcessLink {
        inner: RegistryLink::new(
            reader.clone(),
            clock.clone(),
            settings.journal.clone(),
            settings.space.clone(),
            Registry::new(state_dir.clone()),
            Arc::new(ProcessProbe {
                module: module.clone(),
            }),
        ),
        module: module.clone(),
    });
    let admission_handle = Arc::new(SpaceAdmission::new(
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
    let builder = JournalBuilder::new(
        fs,
        admission_handle.clone(),
        clock,
        Arc::new(journals),
        Arc::new(limits),
        state_dir,
    )
    .user_config(user_limits)
    .space_reporter(admission_handle.clone());
    let journal = Arc::new(with_providers(builder, registry).build()?);
    catalog.bind(&journal);
    let _ = worker.admission.set(Arc::downgrade(&admission_handle));
    locked(&module).start()?;
    Ok(journal)
}
