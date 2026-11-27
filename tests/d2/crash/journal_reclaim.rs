// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/journal_reclaim.rs
// Responsibility:   Crash corpus of reclaim and `journal.purge`.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs, nurt_journal
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Crash corpus of reclaim and `journal.purge`.
//!
//! Each case commits two operations while the initial state is built: a bulk
//! operation (entry 1) and an overwrite of `data/c` (entry 2). The call
//! releases journal data with the failpoint registry attached: a whole entry
//! or the largest item of an entry with the steps of the space service
//! through `EntryCatalog`, or entries through `journal.purge`. Reclaim never
//! changes the tree. After a crash at any point and a recovery in a new
//! journal instance, no entry is left in the reclaim area or expiring, every
//! entry has the blocks of the items it counts, no block is orphaned, and
//! each entry is either kept with its previous content or released; a call
//! that returned has released its entries, also after a power loss that
//! follows.

use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult};
use nurt_contracts::journal::{
    ExpiryReason, FileOp, JournalStore, OpId, OpState, PurgeRequest, PurgeSelector,
};
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
use nurt_crashfs::oracle::{
    Expected, OpOutcome, TreeSnapshot, Violation, check_complete, check_explicit, check_preserved,
};
use nurt_crashfs::points::{CrashPoint, Enumeration, Harness, Phase, Scenario, Trigger};
use nurt_fs::failpoint::{FailPlan, FailPoint, SeededFailpoints};
use nurt_journal::store::Journal;

use crate::corpus::assert_clean;
use crate::journal::{NEW, absolute, before, setup};
use crate::journal_prior::{
    NEWER, Prior, applied, check_blocks, entry_path, instrumented, outcome_of, perform_prior,
    records, recover, relative, release_entry, release_items, remove, resume_reclaims, user_tree,
    view, volume_tree, write,
};
use crate::support::volume_root;

const RECLAIM_SEED: u64 = 0x7265_6170_0000;
/// Random points per case and profile in an ordinary test run.
const RANDOM_POINTS_PER_CASE: u64 = 6;
/// Random points of the long corpus, all cases together.
const LONG_POINTS: u64 = 100_000;
/// Bytes a partial release needs; the largest item covers them.
const PARTIAL_NEED: u64 = 1;

/// Release performed by a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimCase {
    /// Whole entry 1 released for space.
    Whole,
    /// Largest item of entry 1 released for space.
    Partial,
    /// `journal.purge ids: [1]`.
    PurgeOne,
    /// `journal.purge all: true`.
    PurgeAll,
}

/// Entry of the initial state with its history states.
struct Fate {
    op: u64,
    /// State while the entry is kept.
    kept: OpState,
    /// State once the call has released it.
    released: OpState,
    /// Targets whose previous content the entry keeps while it is available.
    targets: &'static [&'static str],
}

const BULK_TARGETS: &[&str] = &["data/a", "data/b"];
const OVERWRITE_TARGETS: &[&str] = &["data/c"];

impl ReclaimCase {
    /// Every case.
    pub const ALL: [ReclaimCase; 4] = [Self::Whole, Self::Partial, Self::PurgeOne, Self::PurgeAll];

    /// Stable name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Whole => "reclaim_whole_entry",
            Self::Partial => "reclaim_partial_entry",
            Self::PurgeOne => "purge_one_entry",
            Self::PurgeAll => "purge_all_entries",
        }
    }

    fn prior() -> Vec<Prior> {
        vec![
            Prior::Operation(
                "write",
                vec![
                    FileOp::write(absolute("data/a"), NEW.to_vec()),
                    FileOp::remove(absolute("data/b")),
                    FileOp::create(absolute("data/new"), NEW.to_vec()),
                ],
            ),
            Prior::Operation(
                "write",
                vec![FileOp::write(absolute("data/c"), NEWER.to_vec())],
            ),
        ]
    }

    /// Tree of every phase: the prior operations applied.
    fn tree() -> TreeSnapshot {
        applied(
            before(),
            &[
                write("data/a", NEW),
                remove("data/b"),
                write("data/new", NEW),
                write("data/c", NEWER),
            ],
        )
    }

    fn fates(self) -> [Fate; 2] {
        let first = |released| Fate {
            op: 1,
            kept: OpState::Available,
            released,
            targets: BULK_TARGETS,
        };
        let second = |released| Fate {
            op: 2,
            kept: OpState::Available,
            released,
            targets: OVERWRITE_TARGETS,
        };
        match self {
            Self::Whole | Self::PurgeOne => [first(OpState::Expired), second(OpState::Available)],
            Self::Partial => [first(OpState::Partial), second(OpState::Available)],
            Self::PurgeAll => [first(OpState::Expired), second(OpState::Expired)],
        }
    }

    fn perform(self, journal: &Journal, fs: &CrashFs, failpoints: &SeededFailpoints) -> OpOutcome {
        let purge = |selector| {
            journal
                .purge(&PurgeRequest {
                    volume: None,
                    selector,
                })
                .map(|_| ())
        };
        let result = match self {
            Self::Whole => {
                release_entry(journal, fs, Some(failpoints), OpId(1), ExpiryReason::Space)
            }
            Self::Partial => release_items(journal, fs, Some(failpoints), OpId(1), PARTIAL_NEED),
            Self::PurgeOne => purge(PurgeSelector::Ids(vec![OpId(1)])),
            Self::PurgeAll => purge(PurgeSelector::All),
        };
        outcome_of(&result, |_| false)
    }

    /// Protocol point the call passes after it changed the journal.
    fn reclaim_point(self) -> FailPoint {
        match self {
            Self::Partial => FailPoint::ReclaimPartial,
            Self::Whole | Self::PurgeOne | Self::PurgeAll => FailPoint::ReclaimMoved,
        }
    }
}

/// Recovers the journal, then completes the releases a crash interrupted.
fn recover_and_resume(journal: &Journal, fs: &CrashFs) -> FsResult<()> {
    recover(journal)?;
    resume_reclaims(journal, fs)
        .map_err(|error| FsError::new(FsErrorKind::Io, &format!("resume reclaim: {error}"), None))
}

/// Checks the blocks of the journal and the state of each entry: kept with
/// its previous content, released, or either when `released` is `None`.
fn check_journal(
    case: ReclaimCase,
    journal: &Journal,
    fs: &CrashFs,
    released: Option<bool>,
) -> Result<(), Violation> {
    check_blocks(journal, fs)?;
    let records = records(journal)?;
    let volume = volume_tree(fs)?;
    let mut wrong: Vec<PathBuf> = Vec::new();
    for fate in case.fates() {
        let state = records.get(&fate.op).map(|record| record.state);
        let agrees = match released {
            Some(true) => state == Some(fate.released),
            Some(false) => state == Some(fate.kept),
            None => state == Some(fate.kept) || state == Some(fate.released),
        };
        if !agrees {
            wrong.push(entry_path(OpId(fate.op)));
        }
        if state == Some(OpState::Available) {
            let targets: Vec<PathBuf> = fate.targets.iter().map(|path| relative(path)).collect();
            check_preserved(&before(), &targets, &volume)?;
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(Violation::IntermediateState {
            from_before: wrong.clone(),
            from_after: wrong,
        })
    }
}

/// Oracle of the reclaim cases.
///
/// # Errors
///
/// Fails with the first violated property.
pub fn check(
    case: ReclaimCase,
    fs: &CrashFs,
    phase: Phase,
    outcome: OpOutcome,
) -> Result<(), Violation> {
    check_explicit(outcome)?;
    let tree = user_tree(fs)?;
    let visible = match phase {
        Phase::AfterCrash => tree.without_temporary(),
        Phase::AfterRecovery => tree,
    };
    let expected = ReclaimCase::tree();
    check_complete(&visible, &expected, &expected, Expected::After)?;
    match (phase, outcome) {
        (Phase::AfterCrash, OpOutcome::Committed { .. }) => {
            let journal = view(fs);
            recover_and_resume(&journal, fs)?;
            check_journal(case, &journal, fs, Some(true))
        }
        (Phase::AfterCrash, _) => Ok(()),
        (Phase::AfterRecovery, OpOutcome::Committed { .. }) => {
            check_journal(case, &view(fs), fs, Some(true))
        }
        (Phase::AfterRecovery, _) => check_journal(case, &view(fs), fs, None),
    }
}

/// Reclaim case as a crash scenario.
#[derive(Clone, Copy, Debug)]
pub struct ReclaimScenario(pub ReclaimCase);

impl Scenario for ReclaimScenario {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn volumes(&self) -> Vec<VolumeSpec> {
        vec![VolumeSpec::new(volume_root())]
    }

    fn setup(&self, fs: &CrashFs) -> FsResult<()> {
        setup(fs)?;
        perform_prior(fs, ReclaimCase::prior())
    }

    fn run(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> OpOutcome {
        self.0
            .perform(&instrumented(fs, failpoints), fs, failpoints)
    }

    fn recover(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> FsResult<()> {
        recover_and_resume(&instrumented(fs, failpoints), fs)
    }

    fn check(&self, fs: &CrashFs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
        check(self.0, fs, phase, outcome)
    }
}

fn first_hit(enumeration: &Enumeration, point: FailPoint) -> Option<Trigger> {
    enumeration
        .protocol
        .iter()
        .copied()
        .find(|trigger| matches!(trigger, Trigger::Protocol { point: hit, .. } if *hit == point))
}

#[test]
fn reclaim_cases_release_without_faults_and_survive_power_loss() {
    for case in ReclaimCase::ALL {
        let scenario = ReclaimScenario(case);
        let fs = CrashFs::new(
            CrashProfile::PosixWorstCase,
            RECLAIM_SEED,
            scenario.volumes(),
        )
        .unwrap();
        scenario.setup(&fs).unwrap();
        fs.sync_all();
        let failpoints = Arc::new(SeededFailpoints::new(FailPlan::observe(RECLAIM_SEED)));
        fs.attach_failpoints(Arc::clone(&failpoints));
        let outcome = scenario.run(&fs, &failpoints);
        fs.detach_failpoints();
        assert_eq!(
            outcome,
            OpOutcome::Committed {
                irreversible: false
            },
            "{}",
            case.name()
        );
        scenario
            .check(&fs, Phase::AfterRecovery, outcome)
            .unwrap_or_else(|violation| panic!("{}: {violation}", case.name()));
        fs.crash();
        scenario
            .check(&fs, Phase::AfterCrash, outcome)
            .unwrap_or_else(|violation| panic!("{} after power loss: {violation}", case.name()));
    }
}

#[test]
fn reclaim_crash_at_every_point_leaves_no_orphaned_or_missing_blocks() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    for case in ReclaimCase::ALL {
        let scenario = ReclaimScenario(case);
        let enumeration = harness.enumerate(&scenario, RECLAIM_SEED).unwrap();
        assert!(
            first_hit(&enumeration, case.reclaim_point()).is_some(),
            "{}: the call did not pass {:?}",
            case.name(),
            case.reclaim_point()
        );
        let report = harness.run_all(&scenario, &enumeration, RECLAIM_SEED);
        assert_clean(&report);
        assert_eq!(report.points, enumeration.triggers().len() as u64);
    }
}

#[test]
fn reclaim_crash_during_recovery_and_again_leaves_no_orphaned_or_missing_blocks() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let recovery_points = [
        Trigger::Protocol {
            point: FailPoint::RecoveryStarted,
            n: None,
            occurrence: 0,
        },
        Trigger::FsCall { call: 1 },
        Trigger::FsCall { call: 2 },
    ];
    for case in [
        ReclaimCase::Whole,
        ReclaimCase::Partial,
        ReclaimCase::PurgeOne,
    ] {
        let scenario = ReclaimScenario(case);
        let enumeration = harness.enumerate(&scenario, RECLAIM_SEED).unwrap();
        let run = first_hit(&enumeration, case.reclaim_point()).unwrap();
        for (seed, recovery) in (RECLAIM_SEED..).zip(recovery_points) {
            let point = CrashPoint {
                run,
                recovery: Some(recovery),
            };
            let report = harness
                .run_point(&scenario, seed, point)
                .unwrap_or_else(|failure| panic!("{}: {failure:?}", case.name()));
            assert_eq!(report.crashes, 2, "{}: {recovery:?}", case.name());
        }
    }
}

#[test]
fn reclaim_crash_at_random_points_on_every_profile_leaves_no_orphaned_blocks() {
    for profile in CrashProfile::ALL {
        let harness = Harness::new(profile);
        for case in ReclaimCase::ALL {
            let scenario = ReclaimScenario(case);
            let enumeration = harness.enumerate(&scenario, RECLAIM_SEED).unwrap();
            let report = harness.run_random(
                &scenario,
                &enumeration,
                RECLAIM_SEED,
                RANDOM_POINTS_PER_CASE,
            );
            assert_clean(&report);
        }
    }
}

#[test]
#[ignore = "long crash corpus; run: cargo test -p nurt-crashfs --release --test crash -- --ignored reclaim_crash_corpus_long"]
fn reclaim_crash_corpus_long_random_points() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let per_case = LONG_POINTS.div_ceil(ReclaimCase::ALL.len() as u64);
    let mut points = 0;
    for case in ReclaimCase::ALL {
        let scenario = ReclaimScenario(case);
        let enumeration = harness.enumerate(&scenario, RECLAIM_SEED).unwrap();
        let report = harness.run_random(&scenario, &enumeration, RECLAIM_SEED, per_case);
        assert_clean(&report);
        points += report.points;
    }
    assert!(points >= LONG_POINTS);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
