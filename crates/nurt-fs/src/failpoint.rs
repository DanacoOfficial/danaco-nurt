// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::failpoint
// Responsibility:   Named fault injection points of the file system and journal protocol.
// Dependencies:     nurt_contracts, seeded, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Named fault injection points of the file system and journal protocol.
//!
//! The journal calls [`Failpoints::hit`] at every step of its protocol.
//! Production builds pass [`NoFailpoints`], which always continues. With the
//! `failpoints` feature, [`SeededFailpoints`] decides per hit whether to
//! continue, fail the current system call or simulate a crash, following a
//! [`FailPlan`] and a seed, and records every decision so that a run can be
//! replayed from its [`RunRecord`].

use nurt_contracts::fs::FsErrorKind;
use serde::{Deserialize, Serialize};

/// Injection point of the journal protocol.
///
/// Names are stable: they identify crash points in recorded runs. Points that
/// concern one target or item receive its index as the `n` argument of
/// [`Failpoints::hit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailPoint {
    /// Before the sequence number is incremented.
    SeqBefore,
    /// After the sequence file was written, before its flush.
    SeqWritten,
    /// After admission, before the manifest is written.
    PlanDecided,
    /// Manifest written to the temporary directory, not flushed.
    ManifestWritten,
    /// Manifest flushed, temporary directory not renamed.
    ManifestSynced,
    /// Entry directory renamed to its final name, parent not flushed.
    PreparedRenamed,
    /// Entry in the `prepared` state is durable.
    PreparedDurable,
    /// Before preserving target `n`.
    PreserveBefore,
    /// Previous content moved or exchanged, metadata not yet written.
    PreserveMoved,
    /// Previous content and metadata preserved and flushed.
    PreserveDurable,
    /// Staged file written, not flushed.
    StageWritten,
    /// Staged file flushed, not switched in.
    StageSynced,
    /// New content switched in, directory not flushed.
    ApplySwitched,
    /// All targets applied, state file not changed.
    ApplyDone,
    /// Committed state written to a temporary file.
    CommitWritten,
    /// Committed state renamed, not flushed.
    CommitRenamed,
    /// Entry committed and durable; index not appended.
    CommitDurable,
    /// Rollback started (`rolling_back` written).
    RollbackStarted,
    /// Rollback restored target `n`.
    RollbackRestored,
    /// Undo check finished, before the first restore.
    UndoChecked,
    /// Undo restored item `n`.
    UndoRestored,
    /// Entry renamed to the reclaim area.
    ReclaimMoved,
    /// Reclaim removed part of an entry.
    ReclaimPartial,
    /// Recovery started.
    RecoveryStarted,
    /// Recovery finished one entry.
    RecoveryEntry,
}

impl FailPoint {
    /// Every point in protocol order.
    pub const ALL: [FailPoint; 25] = [
        Self::SeqBefore,
        Self::SeqWritten,
        Self::PlanDecided,
        Self::ManifestWritten,
        Self::ManifestSynced,
        Self::PreparedRenamed,
        Self::PreparedDurable,
        Self::PreserveBefore,
        Self::PreserveMoved,
        Self::PreserveDurable,
        Self::StageWritten,
        Self::StageSynced,
        Self::ApplySwitched,
        Self::ApplyDone,
        Self::CommitWritten,
        Self::CommitRenamed,
        Self::CommitDurable,
        Self::RollbackStarted,
        Self::RollbackRestored,
        Self::UndoChecked,
        Self::UndoRestored,
        Self::ReclaimMoved,
        Self::ReclaimPartial,
        Self::RecoveryStarted,
        Self::RecoveryEntry,
    ];

    /// Stable snake-case name, equal to the serialized form.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::failpoint::FailPoint;
    ///
    /// assert_eq!(FailPoint::PreserveBefore.as_str(), "preserve_before");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SeqBefore => "seq_before",
            Self::SeqWritten => "seq_written",
            Self::PlanDecided => "plan_decided",
            Self::ManifestWritten => "manifest_written",
            Self::ManifestSynced => "manifest_synced",
            Self::PreparedRenamed => "prepared_renamed",
            Self::PreparedDurable => "prepared_durable",
            Self::PreserveBefore => "preserve_before",
            Self::PreserveMoved => "preserve_moved",
            Self::PreserveDurable => "preserve_durable",
            Self::StageWritten => "stage_written",
            Self::StageSynced => "stage_synced",
            Self::ApplySwitched => "apply_switched",
            Self::ApplyDone => "apply_done",
            Self::CommitWritten => "commit_written",
            Self::CommitRenamed => "commit_renamed",
            Self::CommitDurable => "commit_durable",
            Self::RollbackStarted => "rollback_started",
            Self::RollbackRestored => "rollback_restored",
            Self::UndoChecked => "undo_checked",
            Self::UndoRestored => "undo_restored",
            Self::ReclaimMoved => "reclaim_moved",
            Self::ReclaimPartial => "reclaim_partial",
            Self::RecoveryStarted => "recovery_started",
            Self::RecoveryEntry => "recovery_entry",
        }
    }

    /// Whether the point is hit once per target or item and receives its
    /// index.
    #[must_use]
    pub fn takes_index(self) -> bool {
        matches!(
            self,
            Self::PreserveBefore
                | Self::PreserveMoved
                | Self::PreserveDurable
                | Self::StageWritten
                | Self::StageSynced
                | Self::ApplySwitched
                | Self::RollbackRestored
                | Self::UndoRestored
                | Self::RecoveryEntry
        )
    }
}

/// Action taken at a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailAction {
    /// Continue normally.
    Continue,
    /// Fail the current step with the given error, as a system call would.
    Error(FsErrorKind),
    /// Simulate a crash: the caller stops immediately, without cleanup, and
    /// the harness discards unsynchronised file system state.
    Crash,
}

/// Registry of active injection points.
pub trait Failpoints: Send + Sync {
    /// Decides the action for a hit; `n` is the target or item index for
    /// points where [`FailPoint::takes_index`] holds, `None` otherwise.
    fn hit(&self, point: FailPoint, n: Option<u32>) -> FailAction;
}

/// Registry with all points disabled; used by production builds.
///
/// # Examples
///
/// ```
/// use nurt_fs::failpoint::{FailAction, FailPoint, Failpoints, NoFailpoints};
///
/// assert_eq!(NoFailpoints.hit(FailPoint::SeqBefore, None), FailAction::Continue);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoFailpoints;

impl Failpoints for NoFailpoints {
    fn hit(&self, _point: FailPoint, _n: Option<u32>) -> FailAction {
        FailAction::Continue
    }
}

#[cfg(feature = "failpoints")]
pub use seeded::{
    FailPlan, FailRule, HitRecord, RandomFaults, RunRecord, SeededFailpoints, SplitMix64,
};

#[cfg(feature = "failpoints")]
mod seeded {
    use std::collections::HashMap;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use nurt_contracts::fs::FsErrorKind;
    use serde::{Deserialize, Serialize};

    use super::{FailAction, FailPoint, Failpoints};

    /// Probabilities are expressed per million hits.
    const PER_MILLION: u64 = 1_000_000;

    /// Deterministic pseudo-random generator (`SplitMix64`).
    ///
    /// Used where results must depend only on a seed and must not change with
    /// the version of an external generator.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::failpoint::SplitMix64;
    ///
    /// let mut a = SplitMix64::new(7);
    /// let mut b = SplitMix64::new(7);
    /// assert_eq!(a.next_u64(), b.next_u64());
    /// ```
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SplitMix64 {
        state: u64,
    }

    impl SplitMix64 {
        const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
        const MIX_1: u64 = 0xBF58_476D_1CE4_E5B9;
        const MIX_2: u64 = 0x94D0_49BB_1331_11EB;

        /// Creates a generator from a seed.
        #[must_use]
        pub fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        /// Next 64-bit value.
        pub fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_add(Self::GAMMA);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(Self::MIX_1);
            z = (z ^ (z >> 27)).wrapping_mul(Self::MIX_2);
            z ^ (z >> 31)
        }

        /// Value in `0..bound`; zero when `bound` is zero.
        pub fn below(&mut self, bound: u64) -> u64 {
            if bound == 0 {
                return 0;
            }
            let wide = u128::from(self.next_u64()) * u128::from(bound);
            u64::try_from(wide >> 64).unwrap_or(0)
        }

        /// True with probability `per_million / 1_000_000`.
        pub fn chance(&mut self, per_million: u32) -> bool {
            self.below(PER_MILLION) < u64::from(per_million)
        }
    }

    /// Rule that fires at one hit.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FailRule {
        /// Point the rule applies to.
        pub point: FailPoint,
        /// Required index; `None` matches any index.
        pub n: Option<u32>,
        /// Zero-based number of the matching hit at which the rule fires.
        pub occurrence: u32,
        /// Action returned when the rule fires.
        pub action: FailAction,
    }

    /// Random faults applied to hits that no rule matches.
    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RandomFaults {
        /// Probability of a crash per hit, per million.
        pub crash_per_million: u32,
        /// Probability of an error per hit, per million.
        pub error_per_million: u32,
        /// Error kinds drawn uniformly when an error is injected; an empty
        /// list disables random errors.
        pub error_kinds: Vec<FsErrorKind>,
    }

    /// Plan of a run: seed, explicit rules and random faults.
    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FailPlan {
        /// Seed of the random decisions.
        pub seed: u64,
        /// Rules checked in order; the first rule that fires decides.
        pub rules: Vec<FailRule>,
        /// Random faults for hits without a firing rule.
        pub random: RandomFaults,
    }

    impl FailPlan {
        /// Plan that continues at every hit; used to enumerate the hits of a
        /// scenario.
        #[must_use]
        pub fn observe(seed: u64) -> Self {
            Self {
                seed,
                ..Self::default()
            }
        }

        /// Plan with a single rule.
        #[must_use]
        pub fn at(seed: u64, rule: FailRule) -> Self {
            Self {
                seed,
                rules: vec![rule],
                random: RandomFaults::default(),
            }
        }
    }

    /// One recorded decision.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct HitRecord {
        /// Zero-based position of the hit in the run.
        pub seq: u64,
        /// Point hit.
        pub point: FailPoint,
        /// Index passed with the hit.
        pub n: Option<u32>,
        /// Zero-based number of earlier hits with the same point and index.
        pub occurrence: u32,
        /// Decision taken.
        pub action: FailAction,
    }

    /// Complete record of a run, sufficient to replay it.
    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RunRecord {
        /// Plan the run followed.
        pub plan: FailPlan,
        /// Decisions in hit order.
        pub hits: Vec<HitRecord>,
    }

    impl RunRecord {
        /// First hit that injected a fault, if any.
        #[must_use]
        pub fn first_fault(&self) -> Option<&HitRecord> {
            self.hits
                .iter()
                .find(|hit| hit.action != FailAction::Continue)
        }

        /// Whether the run ended in a crash.
        #[must_use]
        pub fn crashed(&self) -> bool {
            self.hits.iter().any(|hit| hit.action == FailAction::Crash)
        }
    }

    #[derive(Debug)]
    struct State {
        rng: SplitMix64,
        counts: HashMap<(FailPoint, Option<u32>), u32>,
        record: RunRecord,
        crashed: bool,
    }

    /// Seeded registry that follows a [`FailPlan`] and records every hit.
    ///
    /// After a crash, either decided by the plan or signalled through
    /// [`SeededFailpoints::mark_crashed`], every later hit returns
    /// [`FailAction::Crash`]: the simulated process no longer runs. Decisions
    /// depend only on the plan and on the sequence of hits.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::failpoint::{
    ///     FailAction, FailPlan, FailPoint, FailRule, Failpoints, SeededFailpoints,
    /// };
    ///
    /// let rule = FailRule {
    ///     point: FailPoint::PreserveBefore,
    ///     n: Some(1),
    ///     occurrence: 0,
    ///     action: FailAction::Crash,
    /// };
    /// let registry = SeededFailpoints::new(FailPlan::at(42, rule));
    /// assert_eq!(registry.hit(FailPoint::PreserveBefore, Some(0)), FailAction::Continue);
    /// assert_eq!(registry.hit(FailPoint::PreserveBefore, Some(1)), FailAction::Crash);
    /// assert_eq!(registry.hit(FailPoint::ApplyDone, None), FailAction::Crash);
    /// assert!(registry.is_crashed());
    /// ```
    #[derive(Debug)]
    pub struct SeededFailpoints {
        state: Mutex<State>,
    }

    impl SeededFailpoints {
        /// Creates a registry following `plan`.
        #[must_use]
        pub fn new(plan: FailPlan) -> Self {
            let rng = SplitMix64::new(plan.seed);
            Self {
                state: Mutex::new(State {
                    rng,
                    counts: HashMap::new(),
                    record: RunRecord {
                        plan,
                        hits: Vec::new(),
                    },
                    crashed: false,
                }),
            }
        }

        /// Creates a registry that repeats the run described by `record`.
        ///
        /// The replay uses the recorded plan; with the same sequence of hits
        /// it yields the same decisions.
        #[must_use]
        pub fn replay(record: &RunRecord) -> Self {
            Self::new(record.plan.clone())
        }

        /// Copy of the record of the run so far.
        pub fn record(&self) -> RunRecord {
            self.lock().record.clone()
        }

        /// Number of hits so far.
        pub fn hit_count(&self) -> u64 {
            u64::try_from(self.lock().record.hits.len()).unwrap_or(u64::MAX)
        }

        /// Whether the simulated process crashed.
        pub fn is_crashed(&self) -> bool {
            self.lock().crashed
        }

        /// Marks the simulated process as crashed; used when a crash is
        /// triggered outside the protocol points, for example by a file
        /// system call.
        pub fn mark_crashed(&self) {
            self.lock().crashed = true;
        }

        fn lock(&self) -> MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    impl State {
        fn decide(&mut self, point: FailPoint, n: Option<u32>, occurrence: u32) -> FailAction {
            if self.crashed {
                return FailAction::Crash;
            }
            let ruled = self.record.plan.rules.iter().find(|rule| {
                rule.point == point
                    && rule.occurrence == occurrence
                    && rule.n.is_none_or(|wanted| Some(wanted) == n)
            });
            if let Some(rule) = ruled {
                return rule.action;
            }
            self.random_action()
        }

        fn random_action(&mut self) -> FailAction {
            let random = &self.record.plan.random;
            let (crash, error) = (random.crash_per_million, random.error_per_million);
            if crash == 0 && (error == 0 || random.error_kinds.is_empty()) {
                return FailAction::Continue;
            }
            if self.rng.chance(crash) {
                return FailAction::Crash;
            }
            let kinds = &self.record.plan.random.error_kinds;
            if kinds.is_empty() || !self.rng.chance(error) {
                return FailAction::Continue;
            }
            let len = u64::try_from(kinds.len()).unwrap_or(u64::MAX);
            let index = usize::try_from(self.rng.below(len)).unwrap_or(0);
            kinds
                .get(index)
                .map_or(FailAction::Continue, |kind| FailAction::Error(*kind))
        }
    }

    impl Failpoints for SeededFailpoints {
        fn hit(&self, point: FailPoint, n: Option<u32>) -> FailAction {
            let mut state = self.lock();
            let count = state.counts.entry((point, n)).or_insert(0);
            let occurrence = *count;
            *count = count.saturating_add(1);
            let action = state.decide(point, n, occurrence);
            if action == FailAction::Crash {
                state.crashed = true;
            }
            let seq = u64::try_from(state.record.hits.len()).unwrap_or(u64::MAX);
            state.record.hits.push(HitRecord {
                seq,
                point,
                n,
                occurrence,
                action,
            });
            action
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_point_has_a_distinct_stable_name() {
        let mut names: Vec<&str> = FailPoint::ALL.iter().map(|point| point.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FailPoint::ALL.len());
    }

    #[test]
    fn disabled_registry_always_continues() {
        for point in FailPoint::ALL {
            assert_eq!(NoFailpoints.hit(point, Some(3)), FailAction::Continue);
        }
    }

    #[cfg(feature = "failpoints")]
    mod seeded {
        use super::super::*;

        fn crash_at(point: FailPoint, n: Option<u32>, occurrence: u32) -> FailPlan {
            FailPlan::at(
                1,
                FailRule {
                    point,
                    n,
                    occurrence,
                    action: FailAction::Crash,
                },
            )
        }

        #[test]
        fn rule_fires_at_requested_occurrence_only() {
            let registry = SeededFailpoints::new(crash_at(FailPoint::RecoveryEntry, None, 2));
            assert_eq!(
                registry.hit(FailPoint::RecoveryEntry, None),
                FailAction::Continue
            );
            assert_eq!(
                registry.hit(FailPoint::RecoveryEntry, None),
                FailAction::Continue
            );
            assert_eq!(
                registry.hit(FailPoint::RecoveryEntry, None),
                FailAction::Crash
            );
        }

        #[test]
        fn every_hit_after_a_crash_is_a_crash() {
            let registry = SeededFailpoints::new(crash_at(FailPoint::SeqBefore, None, 0));
            assert_eq!(registry.hit(FailPoint::SeqBefore, None), FailAction::Crash);
            assert_eq!(registry.hit(FailPoint::SeqWritten, None), FailAction::Crash);
            assert!(registry.record().crashed());
        }

        #[test]
        fn external_crash_signal_stops_later_hits() {
            let registry = SeededFailpoints::new(FailPlan::observe(5));
            registry.mark_crashed();
            assert_eq!(registry.hit(FailPoint::ApplyDone, None), FailAction::Crash);
        }

        #[test]
        fn error_rule_does_not_stop_the_run() {
            let plan = FailPlan::at(
                3,
                FailRule {
                    point: FailPoint::StageWritten,
                    n: Some(0),
                    occurrence: 0,
                    action: FailAction::Error(FsErrorKind::NoSpace),
                },
            );
            let registry = SeededFailpoints::new(plan);
            assert_eq!(
                registry.hit(FailPoint::StageWritten, Some(0)),
                FailAction::Error(FsErrorKind::NoSpace)
            );
            assert_eq!(
                registry.hit(FailPoint::StageWritten, Some(0)),
                FailAction::Continue
            );
            assert!(!registry.is_crashed());
            assert_eq!(registry.record().first_fault().map(|hit| hit.seq), Some(0));
        }

        fn random_plan(seed: u64) -> FailPlan {
            FailPlan {
                seed,
                rules: Vec::new(),
                random: RandomFaults {
                    crash_per_million: 20_000,
                    error_per_million: 200_000,
                    error_kinds: vec![FsErrorKind::Io, FsErrorKind::NoSpace],
                },
            }
        }

        fn drive(registry: &SeededFailpoints) {
            for round in 0..200_u32 {
                for point in FailPoint::ALL {
                    let n = point.takes_index().then_some(round % 3);
                    registry.hit(point, n);
                }
            }
        }

        #[test]
        fn same_seed_replays_same_decisions() {
            let first = SeededFailpoints::new(random_plan(77));
            drive(&first);
            let record = first.record();
            assert!(record.first_fault().is_some());
            let second = SeededFailpoints::replay(&record);
            drive(&second);
            assert_eq!(second.record(), record);
        }

        #[test]
        fn different_seeds_give_different_runs() {
            let first = SeededFailpoints::new(random_plan(1));
            let second = SeededFailpoints::new(random_plan(2));
            drive(&first);
            drive(&second);
            assert_ne!(first.record().hits, second.record().hits);
        }

        #[test]
        fn observe_plan_records_every_hit_without_faults() {
            let registry = SeededFailpoints::new(FailPlan::observe(9));
            drive(&registry);
            assert_eq!(registry.hit_count(), 200 * 25);
            assert!(registry.record().first_fault().is_none());
        }

        #[test]
        fn generator_bound_is_respected() {
            let mut rng = SplitMix64::new(11);
            assert_eq!(rng.below(0), 0);
            for _ in 0..1_000 {
                assert!(rng.below(7) < 7);
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
