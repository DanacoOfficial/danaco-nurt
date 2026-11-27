// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Space module of the service process.
//!
//! The service process hosts the space module next to other modules. The
//! host owns the process, the single-instance lock and the transport that
//! answers pings, and drives the module through [`SpaceService`]: one
//! [`start`](SpaceService::start), then [`tick`](SpaceService::tick) after
//! each returned delay, and [`stop`](SpaceService::stop). The module never
//! panics and never ends the process. A failed step of one iteration is kept
//! in [`SpaceModule::last_error`] and the next iteration runs normally.
//!
//! Every heartbeat round expires entries over their limits before it writes
//! the heartbeat. After a suspend the heartbeat is stale, callers keep no new
//! data, and the first round expires overdue entries before a fresh heartbeat
//! lets callers keep data again.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::config::{JournalConfig, SpaceConfig};
use nurt_contracts::env::{BootId, Clock};
use nurt_contracts::error::{Error, Result};
use nurt_contracts::fs::{AbsPath, Fs, FsErrorKind, FsWrite, VolumeId, VolumeInfo};
use nurt_contracts::journal::{
    Admission, Eligibility, JournalStore, RecoveryReport, ServiceLiveness, SweepReport, VolumeState,
};
use nurt_contracts::stage::CancelToken;
use nurt_journal::layout::TMP_SUFFIX;
use nurt_journal::volumes::{Registry, VolumeRecord, evaluate_eligibility};

pub use crate::heartbeat::{PING_TIMEOUT, ServiceProbe, liveness};

use crate::admission::{ServiceLink, SpaceAdmission};
use crate::heartbeat::{self, Heartbeat, RunState};
use crate::measure::MIN_POLL_PERIOD;

/// Name of the scratch directory of cost model calibration, inside the
/// journal directory of a volume.
const CALIBRATION_DIR: &str = "calibration";

/// Space module hosted by the service process.
pub trait SpaceService: Send {
    /// Starts the module. It runs crash recovery with the synchronous expiry
    /// review, calibrates the reclaim cost model of every volume and writes
    /// the first heartbeat. Calling it on a started module does nothing.
    ///
    /// # Errors
    ///
    /// Fails with the error of journal recovery. The module then stays
    /// stopped and writes no heartbeat, so callers keep no new data.
    fn start(&mut self) -> Result<()>;

    /// Runs one iteration. The iteration measures and reclaims every volume,
    /// and in every heartbeat round it expires entries by limits before it
    /// writes the heartbeat. The returned delay until the next iteration is
    /// the setting `space.poll_ms` when every volume is in the normal state,
    /// and the setting `space.poll_fast_ms` otherwise.
    fn tick(&mut self) -> Duration;

    /// Answers a ping. The module is alive when it is started and its last
    /// heartbeat round is not older than the setting
    /// `space.heartbeat_max_age_ms`.
    fn ping(&self) -> ServiceLiveness;

    /// Stops the module and writes a heartbeat marked stopped. A reclaim step
    /// never spans iterations, so no step is in progress when the host calls
    /// this method.
    fn stop(&mut self);
}

/// Journal operations the space module drives. Every [`JournalStore`]
/// implements them.
pub trait JournalMaintenance: Send + Sync {
    /// Runs recovery with the synchronous expiry review, as the method
    /// [`JournalStore::recover`] does.
    ///
    /// # Errors
    ///
    /// Fails as the method [`JournalStore::recover`] fails.
    fn recover(&self) -> Result<RecoveryReport>;

    /// Expires entries by limits, as the method [`JournalStore::sweep`] does.
    ///
    /// # Errors
    ///
    /// Fails as the method [`JournalStore::sweep`] fails.
    fn sweep(&self) -> Result<SweepReport>;
}

impl<T: JournalStore + ?Sized> JournalMaintenance for T {
    fn recover(&self) -> Result<RecoveryReport> {
        JournalStore::recover(self)
    }

    fn sweep(&self) -> Result<SweepReport> {
        JournalStore::sweep(self)
    }
}

/// Measurement, calibration and reclaim of one volume, as the space module
/// needs them. The admission control [`SpaceAdmission`] implements them.
pub trait SpaceWorker: Send + Sync {
    /// Calibrates the reclaim cost model of `volume` in the scratch
    /// directory `scratch`, which does not exist yet.
    ///
    /// # Errors
    ///
    /// Fails with the file system error of the calibration.
    fn calibrate(&self, volume: &VolumeInfo, scratch: &AbsPath) -> Result<()>;

    /// Current state of `volume` with respect to its thresholds.
    fn state(&self, volume: &VolumeInfo) -> VolumeState;

    /// Reclaims on the volume towards the target of its state within the budget.
    ///
    /// # Errors
    ///
    /// Fails with the error of measurement or of the journal index.
    fn reclaim(&self, volume: &VolumeId, budget: Duration) -> Result<()>;
}

impl SpaceWorker for SpaceAdmission {
    fn calibrate(&self, volume: &VolumeInfo, scratch: &AbsPath) -> Result<()> {
        SpaceAdmission::calibrate(self, volume, scratch).map(|_| ())
    }

    fn state(&self, volume: &VolumeInfo) -> VolumeState {
        match Admission::eligibility(self, volume) {
            Eligibility::Eligible => Admission::volume_status(self, &volume.id).state,
            Eligibility::Inactive(_) => VolumeState::Normal,
        }
    }

    fn reclaim(&self, volume: &VolumeId, budget: Duration) -> Result<()> {
        SpaceAdmission::reclaim(self, volume, budget).map(|_| ())
    }
}

/// Space module: drives recovery, expiry, reclaim and heartbeats of the
/// journal volumes recorded in the registry of the account.
pub struct SpaceModule<J: ?Sized> {
    fs: Arc<dyn FsWrite>,
    clock: Arc<dyn Clock>,
    journal: Arc<J>,
    worker: Arc<dyn SpaceWorker>,
    registry: Registry,
    config: SpaceConfig,
    pid: u32,
    seq: u64,
    running: bool,
    last_round: Option<(BootId, Duration)>,
    last_error: Option<Error>,
}

impl<J: JournalMaintenance + ?Sized> SpaceModule<J> {
    /// Space module over a file system, a clock, the journal, the
    /// measurement and reclaim worker, the registry of the account and the
    /// `[space]` configuration.
    #[must_use]
    pub fn new(
        fs: Arc<dyn FsWrite>,
        clock: Arc<dyn Clock>,
        journal: Arc<J>,
        worker: Arc<dyn SpaceWorker>,
        registry: Registry,
        config: SpaceConfig,
    ) -> Self {
        Self {
            fs,
            clock,
            journal,
            worker,
            registry,
            config,
            pid: std::process::id(),
            seq: 0,
            running: false,
            last_round: None,
            last_error: None,
        }
    }

    /// Error of the latest failed step, kept for diagnostics of the host.
    #[must_use]
    pub fn last_error(&self) -> Option<&Error> {
        self.last_error.as_ref()
    }

    /// Writes a heartbeat when a recorded volume has none or the heartbeat
    /// round is due, without recovery, expiry or reclaim. A host that runs
    /// the module inside a command process calls it before it asks for the
    /// liveness, because volumes are recorded after the module started.
    pub fn refresh(&mut self) {
        if !self.running {
            return;
        }
        let volumes = self.volumes();
        let missing = volumes
            .iter()
            .any(|(_, dir)| heartbeat::read(self.fs.as_ref(), dir).is_none());
        if missing || self.heartbeat_due() {
            self.beat(&volumes, RunState::Running);
        }
    }

    fn millis(value: u32) -> Duration {
        Duration::from_millis(u64::from(value))
    }

    /// Recorded volumes that are mounted where recorded, with their journal
    /// directories.
    fn volumes(&mut self) -> Vec<(VolumeInfo, AbsPath)> {
        let records = match self.registry.load(self.fs.as_ref()) {
            Ok(records) => records,
            Err(error) => {
                self.last_error = Some(error.into());
                return Vec::new();
            }
        };
        records
            .into_iter()
            .filter_map(|record| mounted(self.fs.as_ref(), record))
            .collect()
    }

    fn heartbeat_due(&self) -> bool {
        let stamp = self.clock.stamp();
        let period = Self::millis(self.config.heartbeat_ms);
        self.last_round.as_ref().is_none_or(|(boot, at)| {
            *boot != stamp.boot || stamp.since_boot_with_suspend.saturating_sub(*at) >= period
        })
    }

    fn sweep(&mut self) {
        if let Err(error) = self.journal.sweep() {
            self.last_error = Some(error);
        }
    }

    fn beat(&mut self, volumes: &[(VolumeInfo, AbsPath)], state: RunState) {
        let stamp = self.clock.stamp();
        self.seq += 1;
        let heartbeat = Heartbeat::new(self.pid, self.seq, &stamp, state);
        self.last_round = Some((stamp.boot, stamp.since_boot_with_suspend));
        for (_, dir) in volumes {
            match heartbeat::write(self.fs.as_ref(), dir, &heartbeat) {
                Err(error) if error.kind != FsErrorKind::NotFound => {
                    self.last_error = Some(error.into());
                }
                _ => {}
            }
        }
    }

    fn calibrate(&mut self, volumes: &[(VolumeInfo, AbsPath)]) {
        for (volume, dir) in volumes {
            let scratch = match dir.join_name(&format!("{CALIBRATION_DIR}{TMP_SUFFIX}")) {
                Ok(scratch) => scratch,
                Err(error) => {
                    self.last_error = Some(error.into());
                    continue;
                }
            };
            if let Err(error) = self.worker.calibrate(volume, &scratch) {
                self.last_error = Some(error);
            }
        }
    }
}

/// Volume of a record when it is still mounted at the recorded place.
fn mounted(fs: &dyn Fs, record: VolumeRecord) -> Option<(VolumeInfo, AbsPath)> {
    let volume = fs.volume(&record.mount).ok()?;
    (volume.mount == record.mount && volume.id.subvolume == record.id.subvolume)
        .then_some((volume, record.journal))
}

impl<J: JournalMaintenance + ?Sized> SpaceService for SpaceModule<J> {
    fn start(&mut self) -> Result<()> {
        if self.running {
            return Ok(());
        }
        self.journal.recover()?;
        let volumes = self.volumes();
        self.calibrate(&volumes);
        self.running = true;
        self.beat(&volumes, RunState::Running);
        Ok(())
    }

    fn tick(&mut self) -> Duration {
        if !self.running {
            return Self::millis(self.config.poll_ms).max(MIN_POLL_PERIOD);
        }
        let volumes = self.volumes();
        let due = self.heartbeat_due();
        if due {
            self.sweep();
        }
        let budget = Self::millis(self.config.heartbeat_ms);
        let mut pressed = false;
        for (volume, _) in &volumes {
            if self.worker.state(volume) == VolumeState::Normal {
                continue;
            }
            pressed = true;
            if let Err(error) = self.worker.reclaim(&volume.id, budget) {
                self.last_error = Some(error);
            }
        }
        if due {
            self.beat(&volumes, RunState::Running);
        }
        let period = if pressed {
            Self::millis(self.config.poll_fast_ms)
        } else {
            Self::millis(self.config.poll_ms)
        };
        period.max(MIN_POLL_PERIOD)
    }

    fn ping(&self) -> ServiceLiveness {
        let stamp = self.clock.stamp();
        let max_age = Self::millis(self.config.heartbeat_max_age_ms);
        let fresh = self.last_round.as_ref().is_some_and(|(boot, at)| {
            *boot == stamp.boot && stamp.since_boot_with_suspend.saturating_sub(*at) <= max_age
        });
        if self.running && fresh {
            ServiceLiveness::Alive
        } else {
            ServiceLiveness::Down
        }
    }

    fn stop(&mut self) {
        if !self.running {
            return;
        }
        let volumes = self.volumes();
        self.beat(&volumes, RunState::Stopped);
        self.running = false;
    }
}

/// Runs a space module on the calling thread until the token is cancelled.
/// The loop starts the module, runs iterations separated by the delay each
/// one returns, and stops the module.
///
/// With a virtual clock the waits complete at once, so tests run the module
/// in process.
///
/// # Errors
///
/// Fails with the error of the method [`SpaceService::start`]. No iteration
/// runs then.
pub fn run(service: &mut dyn SpaceService, clock: &dyn Clock, cancel: &CancelToken) -> Result<()> {
    service.start()?;
    while !cancel.is_cancelled() {
        let delay = service.tick();
        if clock.sleep(delay, cancel).is_err() {
            break;
        }
    }
    service.stop();
    Ok(())
}

/// Facts about the space service and the volume registry for admission,
/// read from the registry and the heartbeats. Command processes and the
/// service process use it as their [`ServiceLink`].
pub struct RegistryLink {
    fs: Arc<dyn Fs>,
    clock: Arc<dyn Clock>,
    journal: JournalConfig,
    space: SpaceConfig,
    registry: Registry,
    probe: Arc<dyn ServiceProbe>,
}

impl RegistryLink {
    /// Link over a file system, a clock, the `[journal]` and `[space]`
    /// configuration, the registry of the account and the transport to the
    /// service.
    #[must_use]
    pub fn new(
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
        journal: JournalConfig,
        space: SpaceConfig,
        registry: Registry,
        probe: Arc<dyn ServiceProbe>,
    ) -> Self {
        Self {
            fs,
            clock,
            journal,
            space,
            registry,
            probe,
        }
    }

    fn records(&self) -> Vec<VolumeRecord> {
        self.registry.load(self.fs.as_ref()).unwrap_or_default()
    }
}

impl ServiceLink for RegistryLink {
    fn eligibility(&self, volume: &VolumeInfo) -> Eligibility {
        evaluate_eligibility(volume, &self.journal)
    }

    fn liveness(&self) -> ServiceLiveness {
        let dirs: Vec<AbsPath> = self
            .records()
            .into_iter()
            .map(|record| record.journal)
            .collect();
        liveness(
            self.fs.as_ref(),
            self.clock.as_ref(),
            &dirs,
            &self.space,
            self.probe.as_ref(),
        )
    }

    fn volumes(&self) -> Vec<VolumeInfo> {
        self.records()
            .into_iter()
            .filter_map(|record| mounted(self.fs.as_ref(), record))
            .map(|(volume, _)| volume)
            .collect()
    }

    fn wake(&self, volume: &VolumeId, state: VolumeState) {
        self.probe.wake(volume, state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nurt_contracts::config::{JournalConfig, SpaceConfig};
    use nurt_contracts::env::Clock;
    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::fs::{AbsPath, FsErrorKind, FsWrite, VolumeId, VolumeInfo};
    use nurt_contracts::journal::{
        Eligibility, InactiveReason, RecoveryReport, ServiceLiveness, SweepReport, VolumeState,
    };
    use nurt_contracts::stage::{CancelReason, CancelToken};
    use nurt_fs::clock::VirtualClock;
    use nurt_fs::mem::{MemFault, MemFs, VolumeSpec};
    use nurt_journal::volumes::{JournalPlace, Registry, resolve};

    use super::{
        JournalMaintenance, RegistryLink, SpaceModule, SpaceService, SpaceWorker, liveness, run,
    };
    use crate::admission::ServiceLink;
    use crate::heartbeat::{self, RunState, testing::TestProbe};
    use crate::measure::MIN_POLL_PERIOD;

    type Events = Arc<Mutex<Vec<String>>>;

    fn abs(path: &str) -> AbsPath {
        AbsPath::for_test(path)
    }

    /// Two volumes (`/home`, `/mnt/data`) with journal directories recorded
    /// in the registry of `/home/user/.local/state/nurt`, a virtual clock and
    /// the journal directories.
    struct World {
        fs: MemFs,
        clock: Arc<VirtualClock>,
        registry: Registry,
        dirs: Vec<AbsPath>,
        probe: Arc<TestProbe>,
    }

    impl World {
        fn new() -> Self {
            let clock = Arc::new(VirtualClock::default());
            let fs = MemFs::builder()
                .clock(clock.clone())
                .volume(VolumeSpec::posix(abs("/home")).with_uuid("home"))
                .volume(VolumeSpec::posix(abs("/mnt/data")).with_uuid("data"))
                .build()
                .unwrap();
            fs.seed_dir(&abs("/home/user")).unwrap();
            let registry = Registry::new(abs("/home/user/.local/state/nurt"));
            let config = JournalConfig::default();
            let dirs = ["/home/user/notes.txt", "/mnt/data/file"]
                .into_iter()
                .map(|target| {
                    match resolve(&fs, &config, &registry, &abs(target))
                        .unwrap()
                        .place
                    {
                        JournalPlace::Active { dir, .. } => dir,
                        JournalPlace::Inactive(reason) => panic!("{}", reason.as_str()),
                    }
                })
                .collect();
            Self {
                fs,
                clock,
                registry,
                dirs,
                probe: Arc::new(TestProbe::alive()),
            }
        }

        fn caller_view(&self) -> ServiceLiveness {
            liveness(
                &self.fs,
                &*self.clock,
                &self.dirs,
                &SpaceConfig::default(),
                &*self.probe,
            )
        }

        fn module(
            &self,
            journal: Arc<TestJournal>,
            worker: Arc<TestWorker>,
        ) -> SpaceModule<TestJournal> {
            SpaceModule::new(
                Arc::new(self.fs.clone()),
                self.clock.clone(),
                journal,
                worker,
                self.registry.clone(),
                SpaceConfig::default(),
            )
        }

        fn seq(&self) -> Option<u64> {
            heartbeat::read(&self.fs, &self.dirs[0]).map(|heartbeat| heartbeat.seq)
        }
    }

    /// Journal that records its calls together with what a caller saw at
    /// that moment.
    struct TestJournal {
        events: Events,
        view: Box<dyn Fn() -> ServiceLiveness + Send + Sync>,
        fail_recover: AtomicBool,
        fail_sweep: AtomicBool,
    }

    impl TestJournal {
        fn new(world: &World, events: &Events) -> Arc<Self> {
            let fs = world.fs.clone();
            let clock = world.clock.clone();
            let dirs = world.dirs.clone();
            let probe = world.probe.clone();
            Arc::new(Self {
                events: events.clone(),
                view: Box::new(move || {
                    liveness(&fs, &*clock, &dirs, &SpaceConfig::default(), &*probe)
                }),
                fail_recover: AtomicBool::new(false),
                fail_sweep: AtomicBool::new(false),
            })
        }

        fn note(&self, call: &str) {
            let view = match (self.view)() {
                ServiceLiveness::Alive => "alive",
                ServiceLiveness::Down => "down",
            };
            self.events.lock().unwrap().push(format!("{call} {view}"));
        }
    }

    impl JournalMaintenance for TestJournal {
        fn recover(&self) -> Result<RecoveryReport> {
            self.note("recover");
            if self.fail_recover.load(Ordering::SeqCst) {
                return Err(Error::new(
                    ErrorCode::Io,
                    "journal directory cannot be read",
                ));
            }
            Ok(RecoveryReport::default())
        }

        fn sweep(&self) -> Result<SweepReport> {
            self.note("sweep");
            if self.fail_sweep.load(Ordering::SeqCst) {
                return Err(Error::new(ErrorCode::Io, "journal index cannot be read"));
            }
            Ok(SweepReport::default())
        }
    }

    /// Worker with scripted volume states that records its calls; cancels
    /// `cancel` after `limit` state queries.
    struct TestWorker {
        events: Events,
        states: Mutex<VecDeque<VolumeState>>,
        fail: AtomicBool,
        cancel: Option<(CancelToken, usize)>,
        queries: Mutex<usize>,
    }

    impl TestWorker {
        fn new(events: &Events, states: &[VolumeState]) -> Arc<Self> {
            Arc::new(Self {
                events: events.clone(),
                states: Mutex::new(states.iter().copied().collect()),
                fail: AtomicBool::new(false),
                cancel: None,
                queries: Mutex::new(0),
            })
        }

        fn push(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl SpaceWorker for TestWorker {
        fn calibrate(&self, volume: &VolumeInfo, scratch: &AbsPath) -> Result<()> {
            self.push(format!("calibrate {} {scratch}", volume.mount));
            if self.fail.load(Ordering::SeqCst) {
                return Err(Error::new(ErrorCode::Io, "calibration failed"));
            }
            Ok(())
        }

        fn state(&self, _volume: &VolumeInfo) -> VolumeState {
            let mut queries = self.queries.lock().unwrap();
            *queries += 1;
            if let Some((token, limit)) = &self.cancel
                && *queries >= *limit
            {
                token.cancel(CancelReason::Shutdown);
            }
            self.states
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(VolumeState::Normal)
        }

        fn reclaim(&self, volume: &VolumeId, budget: Duration) -> Result<()> {
            self.push(format!("reclaim {} {}ms", volume.dev, budget.as_millis()));
            if self.fail.load(Ordering::SeqCst) {
                return Err(Error::new(ErrorCode::Io, "reclaim failed"));
            }
            Ok(())
        }
    }

    fn events_of(events: &Events) -> Vec<String> {
        std::mem::take(&mut *events.lock().unwrap())
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn start_recovers_and_calibrates_before_the_first_heartbeat() {
        let world = World::new();
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        assert_eq!(module.ping(), ServiceLiveness::Down);
        module.start().unwrap();
        let recorded = events_of(&events);
        assert_eq!(recorded[0], "recover down");
        assert!(recorded[1].starts_with("calibrate /home "));
        assert!(recorded[1].ends_with("/calibration.tmp"));
        assert_eq!(recorded.len(), 3);
        assert_eq!(world.caller_view(), ServiceLiveness::Alive);
        assert_eq!(module.ping(), ServiceLiveness::Alive);
        assert_eq!(world.seq(), Some(1));

        module.start().unwrap();
        assert!(events_of(&events).is_empty());
        assert_eq!(world.seq(), Some(1));
    }

    #[test]
    fn failed_recovery_keeps_the_module_stopped_without_heartbeat() {
        let world = World::new();
        let events = Events::default();
        let journal = TestJournal::new(&world, &events);
        journal.fail_recover.store(true, Ordering::SeqCst);
        let mut module = world.module(journal, TestWorker::new(&events, &[]));
        assert_eq!(module.start().unwrap_err().code(), ErrorCode::Io);
        assert_eq!(world.seq(), None);
        assert_eq!(module.ping(), ServiceLiveness::Down);
        assert_eq!(module.tick(), Duration::from_millis(250));
        assert_eq!(events_of(&events), ["recover down"]);
        module.stop();
        assert_eq!(world.seq(), None);
    }

    #[test]
    fn restart_after_forty_minutes_expires_before_new_data_is_kept() {
        let world = World::new();
        let events = Events::default();
        let mut first = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        first.start().unwrap();
        drop(first);
        world.clock.advance(Duration::from_mins(40));
        assert_eq!(world.caller_view(), ServiceLiveness::Down);
        events_of(&events);

        let mut second = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        second.start().unwrap();
        assert_eq!(events_of(&events)[0], "recover down");
        assert_eq!(world.caller_view(), ServiceLiveness::Alive);
    }

    #[test]
    fn heartbeat_round_runs_every_heartbeat_period_and_sweeps_first() {
        let world = World::new();
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        module.start().unwrap();
        events_of(&events);
        let mut sequence = Vec::new();
        for _ in 0..4 {
            world.clock.advance(Duration::from_millis(250));
            assert_eq!(module.tick(), Duration::from_millis(250));
            sequence.push(world.seq().unwrap());
        }
        assert_eq!(sequence, [1, 2, 2, 3]);
        assert_eq!(events_of(&events), ["sweep alive", "sweep alive"]);
        assert_eq!(world.caller_view(), ServiceLiveness::Alive);
    }

    #[test]
    fn tick_interval_follows_the_state_of_the_volumes() {
        let world = World::new();
        let events = Events::default();
        let states = [
            VolumeState::Normal,
            VolumeState::Normal,
            VolumeState::Soft,
            VolumeState::Normal,
            VolumeState::Normal,
            VolumeState::Hard,
            VolumeState::Emergency,
            VolumeState::Normal,
        ];
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &states),
        );
        module.start().unwrap();
        events_of(&events);
        let delays: Vec<Duration> = (0..4).map(|_| module.tick()).collect();
        assert_eq!(delays, [250, 50, 50, 50].map(Duration::from_millis));
        let reclaims: Vec<String> = events_of(&events)
            .into_iter()
            .filter(|event| event.starts_with("reclaim"))
            .collect();
        assert_eq!(
            reclaims,
            ["reclaim 1 500ms", "reclaim 2 500ms", "reclaim 1 500ms"]
        );
    }

    #[test]
    fn tick_delay_is_never_shorter_than_the_minimum_period() {
        let world = World::new();
        let events = Events::default();
        let mut config = SpaceConfig::default();
        config.poll_ms = 0;
        config.poll_fast_ms = 0;
        let mut module = SpaceModule::new(
            Arc::new(world.fs.clone()),
            world.clock.clone(),
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[VolumeState::Hard]),
            world.registry.clone(),
            config,
        );
        assert_eq!(module.tick(), MIN_POLL_PERIOD);
        module.start().unwrap();
        assert_eq!(module.tick(), MIN_POLL_PERIOD);
        assert_eq!(module.tick(), MIN_POLL_PERIOD);
    }

    #[test]
    fn resume_after_two_hours_of_suspend_expires_before_the_fresh_heartbeat() {
        let world = World::new();
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        module.start().unwrap();
        events_of(&events);
        world.clock.suspend(Duration::from_hours(2));
        assert_eq!(world.caller_view(), ServiceLiveness::Down);
        assert_eq!(module.ping(), ServiceLiveness::Down);
        module.tick();
        assert_eq!(events_of(&events), ["sweep down"]);
        assert_eq!(world.caller_view(), ServiceLiveness::Alive);
        assert_eq!(module.ping(), ServiceLiveness::Alive);
    }

    #[test]
    fn heartbeat_round_starts_again_after_reboot() {
        let world = World::new();
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        module.start().unwrap();
        world.clock.reboot(Duration::from_secs(5));
        assert_eq!(module.ping(), ServiceLiveness::Down);
        module.tick();
        assert_eq!(module.ping(), ServiceLiveness::Alive);
        assert_eq!(world.seq(), Some(2));
    }

    #[test]
    fn stop_runs_after_the_reclaim_step_and_marks_the_heartbeat_stopped() {
        let world = World::new();
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[VolumeState::Hard]),
        );
        module.start().unwrap();
        events_of(&events);
        world.clock.advance(Duration::from_millis(500));
        module.tick();
        module.stop();
        assert_eq!(events_of(&events), ["sweep alive", "reclaim 1 500ms"]);
        let last = heartbeat::read(&world.fs, &world.dirs[1]).unwrap();
        assert_eq!(last.state, RunState::Stopped);
        assert_eq!(world.caller_view(), ServiceLiveness::Down);
        assert_eq!(module.ping(), ServiceLiveness::Down);
        module.stop();
        assert_eq!(world.seq(), Some(last.seq));
    }

    #[test]
    fn keeps_errors_of_steps_without_stopping() {
        let world = World::new();
        let events = Events::default();
        let journal = TestJournal::new(&world, &events);
        let worker = TestWorker::new(&events, &[VolumeState::Soft]);
        worker.fail.store(true, Ordering::SeqCst);
        journal.fail_sweep.store(true, Ordering::SeqCst);
        let mut module = world.module(journal, worker);
        module.start().unwrap();
        assert_eq!(module.last_error().unwrap().message, "calibration failed");
        module.tick();
        assert_eq!(module.last_error().unwrap().message, "reclaim failed");
        assert_eq!(module.ping(), ServiceLiveness::Alive);

        world.clock.advance(Duration::from_secs(1));
        world
            .fs
            .inject(MemFault::persistent(
                world.dirs[0].clone(),
                Some("create_temp_in"),
                FsErrorKind::Io,
            ))
            .unwrap();
        module.tick();
        assert_eq!(module.last_error().unwrap().code(), ErrorCode::Io);
        assert!(
            module
                .last_error()
                .unwrap()
                .message
                .contains("create_temp_in")
        );
        assert_eq!(world.caller_view(), ServiceLiveness::Alive);
    }

    #[test]
    fn ignores_journal_directories_of_volumes_that_are_gone() {
        let world = World::new();
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        world
            .fs
            .inject(MemFault::persistent(
                world.dirs[1].clone(),
                Some("create_temp_in"),
                FsErrorKind::NotFound,
            ))
            .unwrap();
        module.start().unwrap();
        assert!(module.last_error().is_none());

        world
            .fs
            .inject(MemFault::persistent(
                abs("/home/user/.local/state/nurt/volumes.json"),
                Some("open_read"),
                FsErrorKind::Io,
            ))
            .unwrap();
        world.clock.advance(Duration::from_secs(1));
        module.tick();
        assert_eq!(module.last_error().unwrap().code(), ErrorCode::Io);
    }

    #[test]
    fn host_loop_runs_until_cancelled_and_stops_the_module() {
        let world = World::new();
        let events = Events::default();
        let cancel = CancelToken::new();
        let worker = Arc::new(TestWorker {
            events: events.clone(),
            states: Mutex::new(VecDeque::new()),
            fail: AtomicBool::new(false),
            cancel: Some((cancel.clone(), 8)),
            queries: Mutex::new(0),
        });
        let mut module = world.module(TestJournal::new(&world, &events), worker);
        let started = world.clock.monotonic();
        run(&mut module, &*world.clock, &cancel).unwrap();
        assert_eq!(
            world.clock.monotonic().saturating_sub(started),
            Duration::from_millis(3 * 250)
        );
        let last = heartbeat::read(&world.fs, &world.dirs[0]).unwrap();
        assert_eq!(last.state, RunState::Stopped);
        assert_eq!(module.ping(), ServiceLiveness::Down);

        let failing = TestJournal::new(&world, &events);
        failing.fail_recover.store(true, Ordering::SeqCst);
        let mut module = world.module(failing, TestWorker::new(&events, &[]));
        assert!(run(&mut module, &*world.clock, &CancelToken::new()).is_err());
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn registry_link_reports_volumes_eligibility_liveness_and_wakes() {
        let world = World::new();
        let link = RegistryLink::new(
            Arc::new(world.fs.clone()),
            world.clock.clone(),
            JournalConfig::default(),
            SpaceConfig::default(),
            world.registry.clone(),
            world.probe.clone(),
        );
        let volumes = link.volumes();
        let mounts: Vec<String> = volumes
            .iter()
            .map(|volume| volume.mount.to_string())
            .collect();
        assert_eq!(mounts, ["/home", "/mnt/data"]);
        assert_eq!(link.eligibility(&volumes[0]), Eligibility::Eligible);
        let mut memory = volumes[0].clone();
        memory.fs_type = "tmpfs".to_owned();
        assert_eq!(
            link.eligibility(&memory),
            Eligibility::Inactive(InactiveReason::MemoryVolume)
        );
        assert_eq!(link.liveness(), ServiceLiveness::Down);

        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        module.start().unwrap();
        assert_eq!(link.liveness(), ServiceLiveness::Alive);

        link.wake(&volumes[1].id, VolumeState::Hard);
        assert_eq!(
            world.probe.wakes.lock().unwrap().as_slice(),
            [(volumes[1].id.clone(), VolumeState::Hard)]
        );
    }

    #[test]
    fn liveness_down_means_no_preservation_through_the_link() {
        let world = World::new();
        let link = RegistryLink::new(
            Arc::new(world.fs.clone()),
            world.clock.clone(),
            JournalConfig::default(),
            SpaceConfig::default(),
            world.registry.clone(),
            world.probe.clone(),
        );
        let fs: Arc<dyn FsWrite> = Arc::new(world.fs.clone());
        assert!(fs.can_write_dir(&abs("/home/user")).unwrap());
        assert_eq!(link.liveness(), ServiceLiveness::Down);
        world.probe.answers.store(false, Ordering::SeqCst);
        let events = Events::default();
        let mut module = world.module(
            TestJournal::new(&world, &events),
            TestWorker::new(&events, &[]),
        );
        module.start().unwrap();
        assert_eq!(link.liveness(), ServiceLiveness::Down);
        world.probe.answers.store(true, Ordering::SeqCst);
        assert_eq!(link.liveness(), ServiceLiveness::Alive);
    }
}
