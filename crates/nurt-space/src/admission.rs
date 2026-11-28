// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-space::admission
// Responsibility:   Admission control of journal data.
// Dependencies:     nurt_contracts, nurt_fs, nurt_journal, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Admission control of journal data.
//!
//! User space has priority over the journal. For each target the journal
//! keeps previous content only when the volume is eligible, the space service
//! is alive, the operation fits the single-operation limit and the size
//! budget, and after the write free space `F` stays at or above reserve plus
//! margin: `F − extra ≥ R + m` and `bytes_held + extra ≤ L_size`. Otherwise
//! the oldest entries are released within the synchronous budget, and when
//! that is not enough the target is changed without preservation and marked
//! irreversible. Admission returns a decision on every path, so the journal
//! never causes a user operation to fail.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::config::SpaceConfig;
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FsWrite, VolumeId, VolumeInfo};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, AdmissionRequest, Eligibility, InactiveReason, OpId,
    ServiceLiveness, Thresholds, VolumeSpace, VolumeState, VolumeStatus,
};
use nurt_contracts::report::Irreversible;
use nurt_fs::failpoint::Failpoints;
use nurt_journal::store::{EntryCatalog, EntrySummary, SpaceReporter};
use serde::{Deserialize, Serialize};

use crate::cost;
use crate::measure::{self, Measurement};
use crate::reclaim::{self, Reclaimer, Released, SpaceGoal};
use crate::thresholds;

/// Facts that admission needs from the space service and its volume
/// registry.
pub trait ServiceLink: Send + Sync {
    /// Eligibility of a volume by its class, configuration and anchor
    /// directory, without regard to the service or free space.
    fn eligibility(&self, volume: &VolumeInfo) -> Eligibility;

    /// Liveness of the space service (fresh heartbeat and answered probe).
    fn liveness(&self) -> ServiceLiveness;

    /// Volumes that hold a journal or may hold one.
    fn volumes(&self) -> Vec<VolumeInfo>;

    /// Wakes the service because free space on a volume fell below the
    /// reserve.
    fn wake(&self, volume: &VolumeId, state: VolumeState);
}

/// Reclaim cost model of a volume: `cost = files · per_file + directories ·
/// per_dir + bytes / bytes_per_second`, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    /// Seconds to remove one file.
    pub per_file: f64,
    /// Seconds to remove one directory.
    pub per_dir: f64,
    /// Bytes released per second of removal, beyond the per-file cost.
    pub bytes_per_second: f64,
}

/// Result of a background reclaim pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReclaimReport {
    /// Entries released completely.
    pub released: Vec<OpId>,
    /// Entries released in part (hard and emergency states only).
    pub partial: Vec<OpId>,
    /// Logical bytes released.
    pub bytes: u64,
    /// Free bytes before the pass.
    pub free_before: u64,
    /// Free bytes measured after the pass and the settle window.
    pub free_after: u64,
    /// Free space reached the target of the state (`S + h` soft, `R + m`
    /// hard and emergency).
    pub reached: bool,
}

#[derive(Debug, Clone)]
struct Known {
    info: VolumeInfo,
    class: Eligibility,
}

/// Admission control and reclaim of the journal on the volumes of one
/// account.
///
/// One instance serves all threads of a process; reclaim within a process is
/// serialized, and the [`EntryCatalog`] coordinates with other processes.
/// The journal must establish [`Admission::eligibility`] of a volume before
/// it asks [`Admission::admit`] for targets on it; targets on a volume not
/// seen before are changed without preservation (`journal_inactive`).
///
/// As the [`SpaceReporter`] of the journal it produces the `journal.space`
/// records: free space, free space without the journal, reclaimable bytes,
/// reserve, soft threshold, journal bytes and the reason the journal is off.
pub struct SpaceAdmission {
    fs: Arc<dyn FsWrite>,
    clock: Arc<dyn Clock>,
    failpoints: Arc<dyn Failpoints>,
    link: Arc<dyn ServiceLink>,
    catalog: Arc<dyn EntryCatalog>,
    config: SpaceConfig,
    known: Mutex<HashMap<VolumeId, Known>>,
    models: Mutex<HashMap<VolumeId, CostModel>>,
    reclaiming: Mutex<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn skip(reason: Irreversible) -> AdmissionDecision {
    AdmissionDecision::Skip { reason }
}

fn too_large(request: &AdmissionRequest) -> bool {
    let over_limit = request
        .op_size_limit
        .is_some_and(|limit| request.op_size > limit);
    let over_budget = request
        .size_budget
        .is_some_and(|budget| request.op_size > budget);
    over_limit || (over_budget && !request.oversize_keep)
}

fn within_budget(bytes_held: u64, extra: u64, budget: Option<u64>) -> bool {
    budget.is_none_or(|budget| u128::from(bytes_held) + u128::from(extra) <= u128::from(budget))
}

fn cores() -> NonZeroUsize {
    std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
}

fn hard(state: VolumeState) -> bool {
    matches!(state, VolumeState::Hard | VolumeState::Emergency)
}

/// `journal.space` record of a volume.
fn space_record(
    info: &VolumeInfo,
    reading: &Measurement,
    thresholds: &Thresholds,
    entries: &[EntrySummary],
    eligibility: Eligibility,
) -> VolumeSpace {
    let journal_bytes = entries
        .iter()
        .fold(0_u64, |sum, entry| sum.saturating_add(entry.bytes));
    let reclaimable = entries
        .iter()
        .filter(|entry| reclaim::releasable(entry))
        .fold(0_u64, |sum, entry| sum.saturating_add(entry.bytes));
    let inactive = match eligibility {
        Eligibility::Inactive(reason) => Some(reason),
        Eligibility::Eligible => hard(thresholds::state(reading.free, thresholds))
            .then_some(InactiveReason::BelowReserve),
    };
    VolumeSpace {
        mount: info.mount.clone(),
        filesystem: info.fs_type.clone(),
        total: reading.total,
        free: reading.free,
        free_without_journal: reading.free.saturating_add(journal_bytes),
        reclaimable,
        reserve: thresholds.reserve,
        soft: thresholds.soft,
        journal_bytes,
        inactive,
    }
}

impl SpaceAdmission {
    /// Creates admission control over a file system, a clock, a fault
    /// registry (`NoFailpoints` in production), the service link, the journal
    /// catalog and the `[space]` configuration.
    pub fn new(
        fs: Arc<dyn FsWrite>,
        clock: Arc<dyn Clock>,
        failpoints: Arc<dyn Failpoints>,
        link: Arc<dyn ServiceLink>,
        catalog: Arc<dyn EntryCatalog>,
        config: SpaceConfig,
    ) -> Self {
        Self {
            fs,
            clock,
            failpoints,
            link,
            catalog,
            config,
            known: Mutex::new(HashMap::new()),
            models: Mutex::new(HashMap::new()),
            reclaiming: Mutex::new(()),
        }
    }

    fn measure(&self, volume: &VolumeId) -> Result<Measurement> {
        measure::measure(self.fs.as_ref(), volume, self.config.quota_aware).map_err(Error::from)
    }

    fn thresholds(&self, reading: &Measurement) -> Thresholds {
        thresholds::resolve(&self.config, reading.total, reading.quota_limited)
    }

    fn known(&self, volume: &VolumeId) -> Option<Known> {
        lock(&self.known).get(volume).cloned()
    }

    fn remember(&self, info: &VolumeInfo) -> Eligibility {
        let class = self.link.eligibility(info);
        lock(&self.known).insert(
            info.id.clone(),
            Known {
                info: info.clone(),
                class,
            },
        );
        class
    }

    fn duration(millis: u32) -> Duration {
        Duration::from_millis(u64::from(millis))
    }

    fn reclaimer<'a>(&'a self, info: &'a VolumeInfo, workers: usize) -> Reclaimer<'a> {
        Reclaimer {
            fs: self.fs.as_ref(),
            clock: self.clock.as_ref(),
            failpoints: self.failpoints.as_ref(),
            catalog: self.catalog.as_ref(),
            volume: info,
            quota_aware: self.config.quota_aware,
            workers,
            poll: Self::duration(self.config.poll_fast_ms),
            settle: Self::duration(self.config.settle_ms),
        }
    }

    /// Cost model of a volume: the calibrated one, or the initial estimate
    /// for its medium.
    #[must_use]
    pub fn cost_model(&self, volume: &VolumeInfo) -> CostModel {
        lock(&self.models)
            .get(&volume.id)
            .copied()
            .unwrap_or_else(|| CostModel::initial(volume.rotational))
    }

    /// Restores a cost model kept in the service state, for example after a
    /// restart of the service.
    pub fn restore_cost_model(&self, volume: &VolumeId, model: CostModel) {
        lock(&self.models).insert(volume.clone(), model);
    }

    /// Calibrates the cost model of a volume by creating and removing a small
    /// tree under `scratch`, a directory that must not exist and must lie in
    /// the journal directory of the volume, and returns the smoothed model.
    /// The calibration takes no more than about 150 ms.
    ///
    /// # Errors
    ///
    /// Fails with the file system error of the first step that fails; the
    /// model is then unchanged.
    pub fn calibrate(&self, volume: &VolumeInfo, scratch: &AbsPath) -> Result<CostModel> {
        let scope = reclaim::maintenance_scope();
        let sample = cost::calibrate(self.fs.as_ref(), self.clock.as_ref(), &scope, scratch)?;
        let model = self.cost_model(volume).smoothed(&sample);
        self.restore_cost_model(&volume.id, model);
        Ok(model)
    }

    /// Measurement period of the service for a volume: 250 ms above the soft
    /// threshold, 50 ms at or below it (configurable).
    #[must_use]
    pub fn poll_period(&self, volume: &VolumeId) -> Duration {
        measure::poll_period(self.volume_status(volume).state, &self.config)
    }

    /// Background reclaim of a volume towards the target of its state: in the
    /// soft state up to `S + h`, in the hard state up to `R + m` with partial
    /// release of large entries, in the emergency state as hard on every
    /// core. Stops after `budget`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` for a volume whose eligibility was never
    /// established, and with `E_IO` when free space or the journal index
    /// cannot be read.
    pub fn reclaim(&self, volume: &VolumeId, budget: Duration) -> Result<ReclaimReport> {
        let known = self.known(volume).ok_or_else(|| {
            Error::new(
                ErrorCode::NotFound,
                "volume has no journal; establish eligibility first",
            )
        })?;
        let reading = self.measure(volume)?;
        let thresholds = self.thresholds(&reading);
        let state = thresholds::state(reading.free, &thresholds);
        let mut report = ReclaimReport {
            free_before: reading.free,
            free_after: reading.free,
            reached: true,
            ..ReclaimReport::default()
        };
        let Some(target) = thresholds::reclaim_target(state, &thresholds) else {
            return Ok(report);
        };
        let _serial = lock(&self.reclaiming);
        let entries = self.catalog.entries(volume)?;
        let goal = SpaceGoal {
            target,
            deadline: self.clock.monotonic().saturating_add(budget),
            partial: hard(state),
        };
        let workers = reclaim::parallelism(known.info.rotational, state, cores());
        let model = self.cost_model(&known.info);
        let (released, after) = self.reclaimer(&known.info, workers).for_space(
            &entries,
            &model,
            &goal,
            &Released::default(),
        )?;
        report.released = released.whole;
        report.partial = released.partial;
        report.bytes = released.bytes;
        report.free_after = after.free;
        report.reached = after.free >= target;
        Ok(report)
    }

    /// Steps 1–3 of the decision: eligibility, liveness, size limits.
    fn precheck(&self, request: &AdmissionRequest) -> Option<Irreversible> {
        let Some(known) = self.known(&request.volume) else {
            return Some(Irreversible::JournalInactive);
        };
        if let Eligibility::Inactive(reason) = known.class {
            return Some(reason.irreversible());
        }
        if self.link.liveness() == ServiceLiveness::Down {
            return Some(Irreversible::JournalInactive);
        }
        too_large(request).then_some(Irreversible::TooLargeForJournal)
    }

    /// Synchronous reclaim within the budget, then the final decision.
    fn reclaim_for(
        &self,
        request: &AdmissionRequest,
        thresholds: &Thresholds,
        known: &Known,
    ) -> AdmissionDecision {
        let deadline = self
            .clock
            .monotonic()
            .saturating_add(Self::duration(self.config.sync_budget_ms));
        let _serial = lock(&self.reclaiming);
        let Ok(entries) = self.catalog.entries(&request.volume) else {
            return skip(Irreversible::NoSpaceForCopy);
        };
        let workers = reclaim::parallelism(known.info.rotational, VolumeState::Soft, cores());
        let reclaimer = self.reclaimer(&known.info, workers);
        let model = self.cost_model(&known.info);
        let mut released = Released::default();
        if !Self::meet_budget(
            request,
            &reclaimer,
            &entries,
            &model,
            deadline,
            &mut released,
        ) {
            return skip(Irreversible::NoSpaceForCopy);
        }
        let goal = SpaceGoal {
            target: request
                .extra
                .saturating_add(thresholds.reserve)
                .saturating_add(thresholds.margin),
            deadline,
            partial: false,
        };
        let space_met = match reclaimer.for_space(&entries, &model, &goal, &released) {
            Ok((more, reading)) => {
                released.absorb(more);
                thresholds::leaves_reserve(reading.free, request.extra, thresholds)
            }
            Err(_) => false,
        };
        match (space_met, released.whole.is_empty()) {
            (false, _) => skip(Irreversible::NoSpaceForCopy),
            (true, true) => AdmissionDecision::Keep,
            (true, false) => AdmissionDecision::KeepAfterReclaim {
                released: released.whole,
            },
        }
    }

    /// Expires the oldest entries until the size budget holds. With oversize
    /// handling `keep` the budget also counts as met once no other entry is
    /// left to expire.
    fn meet_budget(
        request: &AdmissionRequest,
        reclaimer: &Reclaimer<'_>,
        entries: &[EntrySummary],
        model: &CostModel,
        deadline: Duration,
        released: &mut Released,
    ) -> bool {
        let Some(budget) = request.size_budget else {
            return true;
        };
        if within_budget(request.bytes_held, request.extra, Some(budget)) {
            return true;
        }
        let need = request
            .bytes_held
            .saturating_add(request.extra)
            .saturating_sub(budget);
        released.absorb(reclaimer.for_budget(entries, model, need, deadline));
        let held = request.bytes_held.saturating_sub(released.bytes);
        let exhausted = reclaim::budget_order(entries)
            .iter()
            .all(|entry| released.contains(entry.op));
        within_budget(held, request.extra, Some(budget)) || (request.oversize_keep && exhausted)
    }
}

impl SpaceReporter for SpaceAdmission {
    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>> {
        let volumes = match volume {
            Some(path) => vec![self.fs.volume(path)?],
            None => self.link.volumes(),
        };
        volumes
            .iter()
            .map(|info| {
                let eligibility = self.eligibility(info);
                let reading = self.measure(&info.id)?;
                let entries = self.catalog.entries(&info.id)?;
                Ok(space_record(
                    info,
                    &reading,
                    &self.thresholds(&reading),
                    &entries,
                    eligibility,
                ))
            })
            .collect()
    }
}

impl Admission for SpaceAdmission {
    fn eligibility(&self, volume: &VolumeInfo) -> Eligibility {
        match self.remember(volume) {
            Eligibility::Eligible if self.link.liveness() == ServiceLiveness::Down => {
                Eligibility::Inactive(InactiveReason::ServiceDown)
            }
            class => class,
        }
    }

    fn admit(&self, request: &AdmissionRequest) -> AdmissionDecision {
        if let Some(reason) = self.precheck(request) {
            return skip(reason);
        }
        let Some(known) = self.known(&request.volume) else {
            return skip(Irreversible::JournalInactive);
        };
        let Ok(reading) = self.measure(&request.volume) else {
            return skip(Irreversible::JournalInactive);
        };
        let thresholds = self.thresholds(&reading);
        let state = thresholds::state(reading.free, &thresholds);
        if hard(state) {
            self.link.wake(&request.volume, state);
            return skip(Irreversible::NoSpaceForCopy);
        }
        let space_met = thresholds::leaves_reserve(reading.free, request.extra, &thresholds);
        if space_met && within_budget(request.bytes_held, request.extra, request.size_budget) {
            return AdmissionDecision::Keep;
        }
        self.reclaim_for(request, &thresholds, &known)
    }

    fn after_operation(&self, volume: &VolumeId) {
        let status = self.volume_status(volume);
        if hard(status.state) {
            self.link.wake(volume, status.state);
        }
    }

    fn volume_status(&self, volume: &VolumeId) -> VolumeStatus {
        match self.measure(volume) {
            Ok(reading) => {
                let thresholds = self.thresholds(&reading);
                VolumeStatus {
                    state: thresholds::state(reading.free, &thresholds),
                    thresholds,
                }
            }
            Err(_) => VolumeStatus {
                state: VolumeState::Hard,
                thresholds: thresholds::resolve(&self.config, 0, false),
            },
        }
    }

    fn liveness(&self) -> ServiceLiveness {
        self.link.liveness()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nurt_contracts::config::{SizeOrPercent, SizeSetting, SpaceConfig};
    use nurt_contracts::env::Clock;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::journal::{
        Admission, AdmissionDecision, AdmissionRequest, Eligibility, EntryState, ExpiryReason,
        InactiveReason, OpId, ServiceLiveness, VolumeState,
    };
    use nurt_contracts::report::Irreversible;
    use nurt_fs::failpoint::NoFailpoints;

    use super::*;
    use crate::measure::testing::{TestVolume, path, volume_id, volume_info};
    use crate::reclaim::testing::TestCatalog;

    /// Volume of 10 000 bytes with `R` = 1000, `S` = 2000, `m` = 100 and
    /// `h` = 100 (1 % of the capacity).
    const CAPACITY: u64 = 10_000;
    const RESERVE: u64 = 1_000;
    const SOFT: u64 = 2_000;
    const MARGIN: u64 = 100;
    const HYSTERESIS: u64 = 100;

    struct TestLink {
        class: Mutex<Eligibility>,
        alive: AtomicBool,
        woken: Mutex<Vec<VolumeState>>,
    }

    impl ServiceLink for TestLink {
        fn eligibility(&self, _volume: &VolumeInfo) -> Eligibility {
            *self.class.lock().expect("class")
        }

        fn liveness(&self) -> ServiceLiveness {
            if self.alive.load(Ordering::SeqCst) {
                ServiceLiveness::Alive
            } else {
                ServiceLiveness::Down
            }
        }

        fn volumes(&self) -> Vec<VolumeInfo> {
            vec![volume_info(false)]
        }

        fn wake(&self, _volume: &VolumeId, state: VolumeState) {
            self.woken.lock().expect("woken").push(state);
        }
    }

    fn config() -> SpaceConfig {
        let mut config = SpaceConfig::default();
        config.reserve = SizeOrPercent::Size(SizeSetting::from_bytes(RESERVE));
        config.soft = Some(SizeOrPercent::Size(SizeSetting::from_bytes(SOFT)));
        config.margin = Some(SizeOrPercent::Size(SizeSetting::from_bytes(MARGIN)));
        config
    }

    struct Setup {
        volume: Arc<TestVolume>,
        catalog: Arc<TestCatalog>,
        link: Arc<TestLink>,
        admission: SpaceAdmission,
    }

    impl Setup {
        fn new(foreign: u64) -> Self {
            let setup = Self::unregistered(foreign);
            let _ = setup.admission.eligibility(&volume_info(false));
            setup
        }

        fn unregistered(foreign: u64) -> Self {
            let volume = Arc::new(TestVolume::new(CAPACITY));
            volume.set_foreign(foreign);
            let catalog = Arc::new(TestCatalog::default());
            let link = Arc::new(TestLink {
                class: Mutex::new(Eligibility::Eligible),
                alive: AtomicBool::new(true),
                woken: Mutex::new(Vec::new()),
            });
            let admission = SpaceAdmission::new(
                volume.clone(),
                volume.clock.clone(),
                Arc::new(NoFailpoints),
                link.clone(),
                catalog.clone(),
                config(),
            );
            Self {
                volume,
                catalog,
                link,
                admission,
            }
        }

        fn woken(&self) -> Vec<VolumeState> {
            self.link.woken.lock().expect("woken").clone()
        }
    }

    fn request(extra: u64) -> AdmissionRequest {
        AdmissionRequest {
            volume: volume_id(),
            extra,
            op_size: extra,
            bytes_held: 0,
            size_budget: None,
            oversize_keep: false,
            op_size_limit: None,
        }
    }

    fn skipped(reason: Irreversible) -> AdmissionDecision {
        AdmissionDecision::Skip { reason }
    }

    fn after_reclaim(ops: &[u64]) -> AdmissionDecision {
        AdmissionDecision::KeepAfterReclaim {
            released: ops.iter().copied().map(OpId).collect(),
        }
    }

    #[test]
    fn admit_conditions_space_boundary_is_reserve_plus_margin() {
        let setup = Setup::new(5_000);
        assert_eq!(
            setup.admission.admit(&request(3_900)),
            AdmissionDecision::Keep
        );
        assert_eq!(
            setup.admission.admit(&request(3_901)),
            skipped(Irreversible::NoSpaceForCopy)
        );
    }

    #[test]
    fn admit_conditions_space_releases_entries_by_score() {
        let setup = Setup::new(6_000);
        setup.catalog.add(&setup.volume, 1, &[500]);
        setup.catalog.add(&setup.volume, 2, &[1_500]);
        assert_eq!(setup.admission.admit(&request(1_500)), after_reclaim(&[2]));
        assert_eq!(setup.catalog.tombs(), vec![(OpId(2), ExpiryReason::Space)]);
    }

    #[test]
    fn admit_conditions_budget_alone() {
        let setup = Setup::new(0);
        setup.catalog.add(&setup.volume, 1, &[300]);
        setup.catalog.add(&setup.volume, 2, &[300]);
        let mut fits = request(400);
        fits.bytes_held = 600;
        fits.size_budget = Some(1_000);
        assert_eq!(setup.admission.admit(&fits), AdmissionDecision::Keep);
        let mut over = fits.clone();
        over.extra = 401;
        assert_eq!(setup.admission.admit(&over), after_reclaim(&[1]));
        assert_eq!(setup.catalog.tombs(), vec![(OpId(1), ExpiryReason::Size)]);
    }

    #[test]
    fn admit_conditions_both_unmet_are_met_by_one_reclaim() {
        let setup = Setup::new(6_000);
        setup.catalog.add(&setup.volume, 1, &[1_000]);
        setup.catalog.add(&setup.volume, 2, &[1_000]);
        let mut both = request(1_500);
        both.bytes_held = 2_000;
        both.size_budget = Some(3_000);
        assert_eq!(setup.admission.admit(&both), after_reclaim(&[1]));
    }

    #[test]
    fn admit_conditions_budget_met_but_space_short_skips() {
        let setup = Setup::new(8_000);
        setup.catalog.add(&setup.volume, 1, &[100]);
        let mut both = request(1_000);
        both.bytes_held = 100;
        both.size_budget = Some(1_050);
        assert_eq!(
            setup.admission.admit(&both),
            skipped(Irreversible::NoSpaceForCopy)
        );
    }

    #[test]
    fn admit_conditions_op_size_limit() {
        let setup = Setup::new(0);
        let mut limited = request(100);
        limited.op_size_limit = Some(100);
        assert_eq!(setup.admission.admit(&limited), AdmissionDecision::Keep);
        limited.op_size = 101;
        assert_eq!(
            setup.admission.admit(&limited),
            skipped(Irreversible::TooLargeForJournal)
        );
    }

    #[test]
    fn admit_conditions_operation_above_budget_skips_without_expiring_entries() {
        let setup = Setup::new(0);
        setup.catalog.add(&setup.volume, 1, &[300]);
        let mut large = request(1_001);
        large.size_budget = Some(1_000);
        large.bytes_held = 300;
        assert_eq!(
            setup.admission.admit(&large),
            skipped(Irreversible::TooLargeForJournal)
        );
        assert!(setup.catalog.tombs().is_empty());
    }

    #[test]
    fn admit_conditions_oversize_keep_expires_other_entries_and_keeps() {
        let setup = Setup::new(0);
        setup.catalog.add(&setup.volume, 1, &[300]);
        setup.catalog.add(&setup.volume, 2, &[200]);
        let pinned = setup.catalog.add(&setup.volume, 3, &[100]);
        setup.catalog.update(pinned, |entry| entry.pinned = true);
        let mut large = request(1_500);
        large.size_budget = Some(1_000);
        large.bytes_held = 600;
        large.oversize_keep = true;
        assert_eq!(setup.admission.admit(&large), after_reclaim(&[1, 2]));
        assert_eq!(setup.catalog.state(pinned), Some(EntryState::Committed));
    }

    #[test]
    fn admit_conditions_unreachable_budget_skips() {
        let setup = Setup::new(0);
        let pinned = setup.catalog.add(&setup.volume, 1, &[800]);
        setup.catalog.update(pinned, |entry| entry.pinned = true);
        let mut blocked = request(900);
        blocked.size_budget = Some(1_000);
        blocked.bytes_held = 800;
        assert_eq!(
            setup.admission.admit(&blocked),
            skipped(Irreversible::NoSpaceForCopy)
        );
    }

    #[test]
    fn admit_inactive_volume_never_seen_by_eligibility() {
        let setup = Setup::unregistered(0);
        assert_eq!(
            setup.admission.admit(&request(1)),
            skipped(Irreversible::JournalInactive)
        );
    }

    #[test]
    fn admit_inactive_classes_map_to_irreversible_reasons() {
        let cases = [
            (InactiveReason::NetworkVolume, Irreversible::NetworkVolume),
            (
                InactiveReason::RemovableVolume,
                Irreversible::RemovableVolume,
            ),
            (InactiveReason::BelowReserve, Irreversible::NoSpaceForCopy),
            (InactiveReason::MemoryVolume, Irreversible::JournalInactive),
            (InactiveReason::NoAnchor, Irreversible::JournalInactive),
            (InactiveReason::Disabled, Irreversible::JournalInactive),
            (InactiveReason::NotDurable, Irreversible::JournalInactive),
            (InactiveReason::ServiceDown, Irreversible::JournalInactive),
        ];
        for (class, expected) in cases {
            let setup = Setup::unregistered(0);
            *setup.link.class.lock().expect("class") = Eligibility::Inactive(class);
            assert_eq!(
                setup.admission.eligibility(&volume_info(false)),
                Eligibility::Inactive(class)
            );
            assert_eq!(setup.admission.admit(&request(1)), skipped(expected));
        }
    }

    #[test]
    fn quota_size_sets_the_capacity_of_the_thresholds() {
        let setup = Setup::new(0);
        setup.volume.set_quota_limit(Some(4_000));
        setup.volume.set_quota_remaining(Some(3_000));
        let status = setup.admission.volume_status(&volume_id());
        assert_eq!(status.thresholds.reserve, 600);
        assert_eq!(status.thresholds.hysteresis, 40);
        assert_eq!(status.state, VolumeState::Normal);
        let space = setup.admission.space(None).expect("space");
        assert_eq!(space[0].total, 4_000);
        assert_eq!(space[0].free, 3_000);
        assert_eq!(space[0].reserve, 600);
        assert_eq!(
            setup.admission.admit(&request(2_300)),
            AdmissionDecision::Keep
        );
        assert_eq!(
            setup.admission.admit(&request(2_301)),
            skipped(Irreversible::NoSpaceForCopy)
        );
    }

    #[test]
    fn admit_inactive_service_down_keeps_no_new_data() {
        let setup = Setup::new(0);
        setup.link.alive.store(false, Ordering::SeqCst);
        assert_eq!(setup.admission.liveness(), ServiceLiveness::Down);
        assert_eq!(
            setup.admission.eligibility(&volume_info(false)),
            Eligibility::Inactive(InactiveReason::ServiceDown)
        );
        assert_eq!(
            setup.admission.admit(&request(1)),
            skipped(Irreversible::JournalInactive)
        );
    }

    #[test]
    fn admit_hard_state_skips_without_waiting_and_wakes_service() {
        let setup = Setup::new(9_000);
        setup.catalog.add(&setup.volume, 1, &[500]);
        assert_eq!(
            setup.admission.admit(&request(1)),
            skipped(Irreversible::NoSpaceForCopy)
        );
        assert_eq!(setup.woken(), vec![VolumeState::Hard]);
        assert!(setup.catalog.tombs().is_empty());
    }

    #[test]
    fn admit_emergency_state_skips_and_wakes_service() {
        let setup = Setup::new(9_600);
        assert_eq!(
            setup.admission.admit(&request(1)),
            skipped(Irreversible::NoSpaceForCopy)
        );
        assert_eq!(setup.woken(), vec![VolumeState::Emergency]);
    }

    #[test]
    fn admit_unmeasurable_volume_keeps_nothing() {
        let setup = Setup::new(0);
        setup.volume.fail_space(true);
        assert_eq!(
            setup.admission.admit(&request(1)),
            skipped(Irreversible::JournalInactive)
        );
        let status = setup.admission.volume_status(&volume_id());
        assert_eq!(status.state, VolumeState::Hard);
    }

    #[test]
    fn admit_with_unreadable_catalog_skips() {
        let setup = Setup::new(5_000);
        setup.catalog.fail_listing();
        assert_eq!(
            setup.admission.admit(&request(4_000)),
            skipped(Irreversible::NoSpaceForCopy)
        );
    }

    fn timed_setup(foreign: u64) -> Setup {
        let setup = Setup::new(foreign);
        setup.volume.set_per_file(Duration::from_millis(400));
        setup.admission.restore_cost_model(
            &volume_id(),
            CostModel {
                per_file: 0.4,
                per_dir: 0.0,
                bytes_per_second: 1e12,
            },
        );
        for op in 1..=5 {
            setup.catalog.add(&setup.volume, op, &[1_000]);
        }
        setup
    }

    #[test]
    fn admit_sync_reclaim_budget() {
        let setup = timed_setup(4_000);
        assert_eq!(
            setup.admission.admit(&request(1_000)),
            after_reclaim(&[1, 2])
        );
        assert!(setup.volume.clock.monotonic() <= Duration::from_millis(1_500));

        let setup = timed_setup(4_000);
        assert_eq!(
            setup.admission.admit(&request(4_500)),
            skipped(Irreversible::NoSpaceForCopy)
        );
        assert!(setup.volume.clock.monotonic() <= Duration::from_millis(1_500));
        assert_eq!(setup.catalog.tombs().len(), 3);
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(300))]

        /// Every admission ends in a decision, and data is kept only when
        /// reserve plus margin stays free after the write.
        #[test]
        fn admit_never_fails_and_journal_never_below_reserve(
            foreign in 0..CAPACITY,
            alive in proptest::bool::weighted(0.9),
            sizes in proptest::collection::vec(0..1_500_u64, 0..4),
            extra in 0..4_000_u64,
            slack in 0..500_u64,
            size_budget in proptest::option::of(0..5_000_u64),
            oversize_keep in proptest::bool::ANY,
            op_size_limit in proptest::option::of(0..5_000_u64),
        ) {
            let setup = Setup::new(foreign);
            setup.link.alive.store(alive, Ordering::SeqCst);
            for (op, size) in (1_u64..).zip(&sizes) {
                setup.catalog.add(&setup.volume, op, &[*size]);
            }
            let request = AdmissionRequest {
                volume: volume_id(),
                extra,
                op_size: extra + slack,
                bytes_held: sizes.iter().sum(),
                size_budget,
                oversize_keep,
                op_size_limit,
            };
            let decision = setup.admission.admit(&request);
            if !matches!(decision, AdmissionDecision::Skip { .. }) {
                proptest::prop_assert!(alive);
                proptest::prop_assert!(setup.volume.free() >= extra + RESERVE + MARGIN);
                setup.volume.put_file(&path("vol/j/new"), extra);
                proptest::prop_assert!(setup.volume.free() >= RESERVE + MARGIN);
            }
        }
    }

    #[test]
    fn write_within_free_succeeds() {
        let setup = Arc::new(Setup::new(0));
        let writer = {
            let setup = Arc::clone(&setup);
            std::thread::spawn(move || {
                let mut failures = 0;
                for _ in 0..200 {
                    let free = setup.volume.free();
                    if free < RESERVE {
                        break;
                    }
                    if !setup.volume.try_add_foreign(free.min(RESERVE) / 4) {
                        failures += 1;
                    }
                }
                failures
            })
        };
        for round in 0..200 {
            if let AdmissionDecision::Keep | AdmissionDecision::KeepAfterReclaim { .. } =
                setup.admission.admit(&request(150))
            {
                let _ = setup
                    .volume
                    .try_put_file(&path(&format!("vol/j/new/{round}")), 150);
            }
        }
        assert_eq!(writer.join().expect("writer"), 0);
    }

    #[test]
    fn after_operation_wakes_service_below_reserve() {
        let setup = Setup::new(0);
        setup.admission.after_operation(&volume_id());
        assert!(setup.woken().is_empty());
        setup.volume.set_foreign(9_200);
        setup.admission.after_operation(&volume_id());
        assert_eq!(setup.woken(), vec![VolumeState::Hard]);
    }

    #[test]
    fn volume_status_and_poll_period_follow_free_space() {
        let setup = Setup::new(0);
        let status = setup.admission.volume_status(&volume_id());
        assert_eq!(status.state, VolumeState::Normal);
        assert_eq!(status.thresholds.reserve, RESERVE);
        assert_eq!(status.thresholds.soft, SOFT);
        assert_eq!(status.thresholds.margin, MARGIN);
        assert_eq!(status.thresholds.hysteresis, HYSTERESIS);
        assert_eq!(
            setup.admission.poll_period(&volume_id()),
            Duration::from_millis(250)
        );
        setup.volume.set_foreign(8_500);
        assert_eq!(
            setup.admission.volume_status(&volume_id()).state,
            VolumeState::Soft
        );
        assert_eq!(
            setup.admission.poll_period(&volume_id()),
            Duration::from_millis(50)
        );
    }

    fn entry(op: u64, state: EntryState, bytes: u64) -> EntrySummary {
        EntrySummary {
            op: OpId(op),
            state,
            bytes,
            files: 1,
            directories: 1,
            pinned: false,
            overdue: None,
        }
    }

    #[test]
    fn space_record_fields() {
        let info = volume_info(false);
        let thresholds = Thresholds {
            reserve: RESERVE,
            soft: SOFT,
            margin: MARGIN,
            hysteresis: HYSTERESIS,
        };
        let entries = [
            entry(1, EntryState::Committed, 300),
            entry(2, EntryState::Partial, 200),
            entry(3, EntryState::Expiring, 400),
            entry(4, EntryState::Corrupt, 50),
        ];
        let reading = Measurement {
            free: 5_000,
            total: CAPACITY,
            quota_limited: false,
        };
        let record = space_record(
            &info,
            &reading,
            &thresholds,
            &entries,
            Eligibility::Eligible,
        );
        assert_eq!(record.mount, info.mount);
        assert_eq!(record.filesystem, "ext4");
        assert_eq!(record.total, CAPACITY);
        assert_eq!(record.free, 5_000);
        assert_eq!(record.journal_bytes, 950);
        assert_eq!(record.reclaimable, 500);
        assert_eq!(record.free_without_journal, 5_950);
        assert_eq!(record.reserve, RESERVE);
        assert_eq!(record.soft, SOFT);
        assert_eq!(record.inactive, None);
        let low = Measurement {
            free: 999,
            ..reading
        };
        let record = space_record(&info, &low, &thresholds, &[], Eligibility::Eligible);
        assert_eq!(record.inactive, Some(InactiveReason::BelowReserve));
        for reason in [
            InactiveReason::ServiceDown,
            InactiveReason::NetworkVolume,
            InactiveReason::RemovableVolume,
            InactiveReason::MemoryVolume,
            InactiveReason::BelowReserve,
            InactiveReason::NoAnchor,
            InactiveReason::Disabled,
            InactiveReason::NotDurable,
        ] {
            let record = space_record(
                &info,
                &reading,
                &thresholds,
                &entries,
                Eligibility::Inactive(reason),
            );
            assert_eq!(record.inactive, Some(reason));
            assert_eq!(record.journal_bytes, 950);
        }
    }

    #[test]
    fn space_reports_one_volume_or_all() {
        let setup = Setup::new(1_000);
        setup.catalog.add(&setup.volume, 1, &[700]);
        let one = setup.admission.space(Some(&path("vol/a"))).expect("space");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].free, 8_300);
        assert_eq!(one[0].journal_bytes, 700);
        assert_eq!(one[0].inactive, None);
        let all = setup.admission.space(None).expect("space");
        assert_eq!(all, one);
        setup.link.alive.store(false, Ordering::SeqCst);
        let down = setup.admission.space(None).expect("space");
        assert_eq!(down[0].inactive, Some(InactiveReason::ServiceDown));
        setup.catalog.fail_listing();
        assert!(setup.admission.space(None).is_err());
    }

    #[test]
    fn background_reclaim_soft_targets_soft_plus_hysteresis() {
        let setup = Setup::new(7_300);
        for op in 1..=3 {
            setup.catalog.add(&setup.volume, op, &[400]);
        }
        let report = setup
            .admission
            .reclaim(&volume_id(), Duration::from_secs(5))
            .expect("reclaim");
        assert_eq!(report.released, vec![OpId(1), OpId(2)]);
        assert!(report.partial.is_empty());
        assert_eq!(report.free_before, 1_500);
        assert_eq!(report.free_after, 2_300);
        assert!(report.reached);
    }

    #[test]
    fn background_reclaim_hard_releases_largest_items_of_an_entry() {
        let setup = Setup::new(8_500);
        let op = setup.catalog.add(&setup.volume, 1, &[100, 700, 200]);
        let report = setup
            .admission
            .reclaim(&volume_id(), Duration::from_secs(5))
            .expect("reclaim");
        assert_eq!(report.partial, vec![op]);
        assert!(report.released.is_empty());
        assert_eq!(report.free_after, 1_200);
        assert!(report.reached);
        assert_eq!(setup.catalog.released_items(op), vec![1]);
        assert_eq!(setup.catalog.state(op), Some(EntryState::Partial));
    }

    #[test]
    fn background_reclaim_in_normal_state_releases_nothing() {
        let setup = Setup::new(0);
        setup.catalog.add(&setup.volume, 1, &[400]);
        let report = setup
            .admission
            .reclaim(&volume_id(), Duration::from_secs(5))
            .expect("reclaim");
        assert_eq!(report.bytes, 0);
        assert!(report.reached);
        assert!(setup.catalog.tombs().is_empty());
    }

    #[test]
    fn background_reclaim_requires_known_volume() {
        let setup = Setup::unregistered(0);
        let error = setup
            .admission
            .reclaim(&volume_id(), Duration::from_secs(5))
            .expect_err("unknown volume");
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn calibration_smooths_the_cost_model() {
        let setup = Setup::new(0);
        setup.volume.set_per_file(Duration::from_micros(10));
        setup.volume.put_file(&path("vol/j/reap/keep"), 0);
        let info = volume_info(false);
        let initial = setup.admission.cost_model(&info);
        let model = setup
            .admission
            .calibrate(&info, &path("vol/j/reap/calibration"))
            .expect("calibrate");
        assert!(model.per_file < initial.per_file);
        assert!(model.per_file > 10e-6);
        assert_eq!(setup.admission.cost_model(&info), model);
        assert!(
            setup
                .admission
                .calibrate(&info, &path("vol/j/reap"))
                .is_err()
        );
        assert_eq!(setup.admission.cost_model(&info), model);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
