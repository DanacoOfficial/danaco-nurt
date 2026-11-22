// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Seeded generator of crash points and corpus driver.
//!
//! A [`Scenario`] performs one operation on a [`CrashFs`] and recovers after
//! a crash. [`Harness::enumerate`] runs it once without faults and lists every
//! protocol hit and every file system call as a possible crash trigger.
//! [`Harness::run_point`] repeats the scenario from the same initial state,
//! crashes at the chosen trigger, optionally crashes again during recovery,
//! and checks the result with the scenario oracle before recovery, after
//! recovery and after a second recovery. Every run is determined by its
//! seed and point, which a [`Failure`] records for replay.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use nurt_contracts::fs::{AbsPath, FsResult};
use nurt_fs::failpoint::{FailAction, FailPlan, FailPoint, FailRule, SeededFailpoints, SplitMix64};
use serde::{Deserialize, Serialize};

use crate::crash::{CrashFs, CrashProfile, VolumeSpec};
use crate::oracle::{OpOutcome, TreeSnapshot, Violation, check_idempotent};

/// Maximum number of recovery calls considered for a crash during recovery.
const RECOVERY_CALL_WINDOW: u64 = 64;
/// Maximum occurrence of [`FailPoint::RecoveryEntry`] drawn for a crash
/// during recovery.
const RECOVERY_ENTRY_WINDOW: u64 = 8;
/// Probability, per million, that a random point also crashes recovery.
const RECOVERY_CRASH_PER_MILLION: u32 = 250_000;
const SEED_STRIDE: u64 = 0xD1B5_4A32_D192_ED03;

/// Event at which a crash happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum Trigger {
    /// A hit of a protocol point.
    Protocol {
        /// Point hit.
        point: FailPoint,
        /// Target or item index.
        n: Option<u32>,
        /// Zero-based number of the hit with the same point and index.
        occurrence: u32,
    },
    /// The start of file system call number `call`, counted from one.
    FsCall {
        /// Call number.
        call: u64,
    },
}

impl Trigger {
    fn plan(self, seed: u64) -> FailPlan {
        match self {
            Self::Protocol {
                point,
                n,
                occurrence,
            } => FailPlan::at(
                seed,
                FailRule {
                    point,
                    n,
                    occurrence,
                    action: FailAction::Crash,
                },
            ),
            Self::FsCall { .. } => FailPlan::observe(seed),
        }
    }

    fn arm(self, fs: &CrashFs, seed: u64) -> Arc<SeededFailpoints> {
        let failpoints = Arc::new(SeededFailpoints::new(self.plan(seed)));
        fs.attach_failpoints(Arc::clone(&failpoints));
        fs.reset_stats();
        if let Self::FsCall { call } = self {
            fs.arm_crash_at_call(call);
        }
        failpoints
    }
}

/// Crash point: a trigger in the operation and, optionally, a second trigger
/// in the recovery that follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CrashPoint {
    /// Trigger during the operation.
    pub run: Trigger,
    /// Trigger during the first recovery.
    pub recovery: Option<Trigger>,
}

/// Moment of an oracle check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// On the image left by the crash, before recovery.
    AfterCrash,
    /// After a complete recovery.
    AfterRecovery,
}

/// Operation under test together with its recovery and oracle.
pub trait Scenario {
    /// Stable name used in reports.
    fn name(&self) -> &str;

    /// Volumes of the file system.
    fn volumes(&self) -> Vec<VolumeSpec>;

    /// Creates the initial state; the harness makes it durable.
    ///
    /// # Errors
    ///
    /// Fails with the error of the file system call that failed.
    fn setup(&self, fs: &CrashFs) -> FsResult<()>;

    /// Performs the operation with `failpoints` attached to the protocol.
    fn run(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> OpOutcome;

    /// Recovers after a crash, as a new process would.
    ///
    /// # Errors
    ///
    /// Fails when recovery itself was interrupted or failed.
    fn recover(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> FsResult<()>;

    /// Checks the state in `phase`; `outcome` is the result of the operation.
    ///
    /// # Errors
    ///
    /// Fails with the first violated property.
    fn check(&self, fs: &CrashFs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation>;
}

/// Possible crash triggers of a scenario.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enumeration {
    /// Protocol hits of a run without faults.
    pub protocol: Vec<Trigger>,
    /// Number of file system calls of that run.
    pub fs_calls: u64,
}

impl Enumeration {
    /// Every trigger: each protocol hit and each file system call.
    pub fn triggers(&self) -> Vec<Trigger> {
        let calls = (1..=self.fs_calls).map(|call| Trigger::FsCall { call });
        self.protocol.iter().copied().chain(calls).collect()
    }
}

/// Oracle failure with what is needed to replay it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// Scenario name.
    pub scenario: String,
    /// Persistence profile.
    pub profile: CrashProfile,
    /// Seed of the run.
    pub seed: u64,
    /// Crash point.
    pub point: CrashPoint,
    /// Phase of the failed check.
    pub phase: Phase,
    /// Violated property.
    pub violation: Violation,
}

/// Result of one point that passed the oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointReport {
    /// Crashes performed: zero when the trigger was not reached, two when
    /// recovery crashed as well.
    pub crashes: u32,
    /// Result of the operation.
    pub outcome: OpOutcome,
}

/// Totals of a corpus run, written as the result file of the run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusReport {
    /// Scenario name.
    pub scenario: String,
    /// Persistence profile.
    pub profile: CrashProfile,
    /// Operating system of the run.
    pub os: String,
    /// Base seed of the run.
    pub seed: u64,
    /// Points executed.
    pub points: u64,
    /// Crashes performed, including crashes during recovery.
    pub crashes: u64,
    /// Oracle failures.
    pub failures: Vec<Failure>,
    /// Wall time of the run in milliseconds.
    pub elapsed_ms: u64,
}

impl CorpusReport {
    fn new(scenario: &str, profile: CrashProfile, seed: u64) -> Self {
        Self {
            scenario: scenario.to_owned(),
            profile,
            os: std::env::consts::OS.to_owned(),
            seed,
            points: 0,
            crashes: 0,
            failures: Vec::new(),
            elapsed_ms: 0,
        }
    }

    fn record(&mut self, result: Result<PointReport, Failure>) {
        self.points += 1;
        match result {
            Ok(report) => self.crashes += u64::from(report.crashes),
            Err(failure) => {
                self.crashes += 1;
                self.failures.push(failure);
            }
        }
    }

    /// Adds the totals of `other`, keeping the scenario and seed of `self`.
    pub fn merge(&mut self, other: CorpusReport) {
        self.points += other.points;
        self.crashes += other.crashes;
        self.failures.extend(other.failures);
        self.elapsed_ms += other.elapsed_ms;
    }

    /// Writes the report as JSON to `path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be created or written.
    pub fn write_json(&self, path: &Path) -> std::io::Result<()> {
        let file = BufWriter::new(File::create(path)?);
        serde_json::to_writer_pretty(file, self).map_err(std::io::Error::other)
    }
}

/// Corpus driver for one persistence profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Harness {
    /// Persistence profile.
    pub profile: CrashProfile,
}

impl Harness {
    /// Creates a driver for `profile`.
    pub fn new(profile: CrashProfile) -> Self {
        Self { profile }
    }

    fn prepare(&self, scenario: &dyn Scenario, seed: u64) -> FsResult<CrashFs> {
        let fs = CrashFs::new(self.profile, seed, scenario.volumes())?;
        scenario.setup(&fs)?;
        fs.sync_all();
        fs.reset_stats();
        Ok(fs)
    }

    /// Runs `scenario` without faults and lists its crash triggers.
    ///
    /// # Errors
    ///
    /// Fails when the initial state cannot be created.
    pub fn enumerate(&self, scenario: &dyn Scenario, seed: u64) -> FsResult<Enumeration> {
        let fs = self.prepare(scenario, seed)?;
        let failpoints = Arc::new(SeededFailpoints::new(FailPlan::observe(seed)));
        fs.attach_failpoints(Arc::clone(&failpoints));
        scenario.run(&fs, &failpoints);
        let protocol = failpoints
            .record()
            .hits
            .iter()
            .map(|hit| Trigger::Protocol {
                point: hit.point,
                n: hit.n,
                occurrence: hit.occurrence,
            })
            .collect();
        Ok(Enumeration {
            protocol,
            fs_calls: fs.stats().calls,
        })
    }

    /// Runs one crash point and checks it.
    ///
    /// # Errors
    ///
    /// Fails with the first oracle violation, recorded for replay.
    pub fn run_point(
        &self,
        scenario: &dyn Scenario,
        seed: u64,
        point: CrashPoint,
    ) -> Result<PointReport, Failure> {
        let fail = |phase: Phase, violation: Violation| Failure {
            scenario: scenario.name().to_owned(),
            profile: self.profile,
            seed,
            point,
            phase,
            violation,
        };
        let fs = self
            .prepare(scenario, seed)
            .map_err(|err| fail(Phase::AfterCrash, err.into()))?;
        let failpoints = point.run.arm(&fs, seed);
        let outcome = scenario.run(&fs, &failpoints);
        if !fs.is_halted() {
            fs.detach_failpoints();
            scenario
                .check(&fs, Phase::AfterRecovery, outcome)
                .map_err(|violation| fail(Phase::AfterRecovery, violation))?;
            return Ok(PointReport {
                crashes: 0,
                outcome,
            });
        }
        fs.crash();
        scenario
            .check(&fs, Phase::AfterCrash, OpOutcome::Crashed)
            .map_err(|violation| fail(Phase::AfterCrash, violation))?;
        let mut crashes = 1;
        if let Some(trigger) = point.recovery {
            let failpoints = trigger.arm(&fs, seed);
            if scenario.recover(&fs, &failpoints).is_err() && fs.is_halted() {
                fs.crash();
                crashes += 1;
                scenario
                    .check(&fs, Phase::AfterCrash, OpOutcome::Crashed)
                    .map_err(|violation| fail(Phase::AfterCrash, violation))?;
            }
        }
        self.recover_and_check(scenario, &fs, seed)
            .map_err(|violation| fail(Phase::AfterRecovery, violation))?;
        Ok(PointReport {
            crashes,
            outcome: OpOutcome::Crashed,
        })
    }

    fn recover_and_check(
        &self,
        scenario: &dyn Scenario,
        fs: &CrashFs,
        seed: u64,
    ) -> Result<(), Violation> {
        let observe = || {
            let failpoints = Arc::new(SeededFailpoints::new(FailPlan::observe(seed)));
            fs.attach_failpoints(Arc::clone(&failpoints));
            failpoints
        };
        scenario.recover(fs, &observe())?;
        scenario.check(fs, Phase::AfterRecovery, OpOutcome::Crashed)?;
        let first = Self::volume_snapshot(scenario, fs)?;
        scenario.recover(fs, &observe())?;
        let second = Self::volume_snapshot(scenario, fs)?;
        fs.detach_failpoints();
        check_idempotent(&first, &second)
    }

    fn volume_snapshot(scenario: &dyn Scenario, fs: &CrashFs) -> FsResult<TreeSnapshot> {
        let mounts: Vec<AbsPath> = scenario
            .volumes()
            .into_iter()
            .map(|volume| volume.mount)
            .collect();
        TreeSnapshot::capture_all(fs, &mounts)
    }

    /// Runs every trigger of `enumeration` once, without crashes during
    /// recovery.
    pub fn run_all(
        &self,
        scenario: &dyn Scenario,
        enumeration: &Enumeration,
        seed: u64,
    ) -> CorpusReport {
        let started = Instant::now();
        let mut report = CorpusReport::new(scenario.name(), self.profile, seed);
        for (index, run) in enumeration.triggers().into_iter().enumerate() {
            let point_seed = derive_seed(seed, index as u64);
            let point = CrashPoint {
                run,
                recovery: None,
            };
            report.record(self.run_point(scenario, point_seed, point));
        }
        report.elapsed_ms = elapsed_ms(started);
        report
    }

    /// Runs `count` points drawn from `enumeration` with seeds derived from
    /// `seed`; a share of them crashes recovery as well.
    pub fn run_random(
        &self,
        scenario: &dyn Scenario,
        enumeration: &Enumeration,
        seed: u64,
        count: u64,
    ) -> CorpusReport {
        let started = Instant::now();
        let mut report = CorpusReport::new(scenario.name(), self.profile, seed);
        let triggers = enumeration.triggers();
        for index in 0..count {
            let point_seed = derive_seed(seed, index);
            let point = random_point(&triggers, point_seed);
            report.record(self.run_point(scenario, point_seed, point));
        }
        report.elapsed_ms = elapsed_ms(started);
        report
    }

    /// Repeats the run recorded in `failure`.
    ///
    /// # Errors
    ///
    /// Fails with the violation found again.
    pub fn replay(
        &self,
        scenario: &dyn Scenario,
        failure: &Failure,
    ) -> Result<PointReport, Failure> {
        self.run_point(scenario, failure.seed, failure.point)
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Seed of point `index` of a run with base seed `seed`.
pub fn derive_seed(seed: u64, index: u64) -> u64 {
    SplitMix64::new(seed ^ index.wrapping_mul(SEED_STRIDE)).next_u64()
}

/// Draws a crash point from `triggers`; with a fixed probability the point
/// crashes recovery as well.
pub fn random_point(triggers: &[Trigger], seed: u64) -> CrashPoint {
    let mut rng = SplitMix64::new(seed);
    let index = usize::try_from(rng.below(triggers.len() as u64)).unwrap_or(0);
    let run = triggers
        .get(index)
        .copied()
        .unwrap_or(Trigger::FsCall { call: 1 });
    let recovery = rng
        .chance(RECOVERY_CRASH_PER_MILLION)
        .then(|| recovery_trigger(&mut rng));
    CrashPoint { run, recovery }
}

fn recovery_trigger(rng: &mut SplitMix64) -> Trigger {
    match rng.below(3) {
        0 => Trigger::Protocol {
            point: FailPoint::RecoveryStarted,
            n: None,
            occurrence: 0,
        },
        1 => Trigger::Protocol {
            point: FailPoint::RecoveryEntry,
            n: None,
            occurrence: u32::try_from(rng.below(RECOVERY_ENTRY_WINDOW)).unwrap_or(0),
        },
        _ => Trigger::FsCall {
            call: 1 + rng.below(RECOVERY_CALL_WINDOW),
        },
    }
}
