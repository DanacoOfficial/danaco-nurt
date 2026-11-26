// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Crash corpus of undo and redo.
//!
//! Each case commits its prior operations while the initial state is built,
//! then performs one `undo` or `redo` through `JournalStore` with the seeded
//! failpoint registry attached to the journal and to the file system. After
//! a crash at any protocol point or file system call and a recovery in a new
//! journal instance, the tree is in the state before the call or in the
//! state after it, never a mixture, and the content of both states exists on
//! the volume at every moment. The history agrees with the tree: the source
//! entries are `undone` (or `partial`) and the new entry is available exactly
//! when the tree is in the state after the call. A call that returned
//! survives a power loss that follows, recovery included.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult, FsWrite};
use nurt_contracts::journal::{
    FileOp, JournalStore, OnConflict, OpId, OpState, RedoRequest, TargetPath, UndoOutcome,
    UndoRequest, UndoSelector,
};
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
use nurt_crashfs::oracle::{
    Change, Expected, Matched, OpOutcome, TreeSnapshot, Violation, check_complete, check_explicit,
    check_no_leftovers, check_preserved,
};
use nurt_crashfs::points::{CrashPoint, Enumeration, Harness, Phase, Scenario, Trigger};
use nurt_fs::failpoint::{FailPlan, FailPoint, SeededFailpoints};
use nurt_journal::store::Journal;

use crate::corpus::assert_clean;
use crate::journal::{NEW, absolute, before, setup};
use crate::journal_prior::{
    NEWER, Prior, applied, entry_path, instrumented, outcome_of, perform_prior, program, records,
    recover, relative, remove, user_tree, view, volume_tree, write,
};
use crate::support::{scope, volume_root};

const UNDO_SEED: u64 = 0x756E_646F_0000;
/// Random points per case and profile in an ordinary test run.
const RANDOM_POINTS_PER_CASE: u64 = 6;
/// Random points of the many-file case, every trigger kind together.
const MANY_RANDOM_POINTS: u64 = 32;
/// Random points of the long corpus, all cases together.
const LONG_POINTS: u64 = 100_000;
/// Files overwritten by the many-file case; above the number of targets from
/// which the journal keeps small previous content in the manifest.
const MANY_FILES: usize = 72;
const MANY_DIR: &str = "data/many";
const DIR_MODE: u32 = 0o755;
/// Directory holding every target of the cases; previous content below it
/// must exist on the volume.
const DATA: &str = "data";

/// Undo or redo performed by a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoCase {
    /// `undo` of the last operation, an overwrite.
    Last,
    /// `undo N` of the removal of a directory tree.
    Tree,
    /// `undo` of a bulk operation: overwrite, removal and creation.
    Bulk,
    /// `undo N paths:` of one item of the bulk operation.
    Paths,
    /// `undo N on_conflict: partial` of the bulk operation after the space
    /// service released its largest item, the preserved `data/b`.
    Partial,
    /// `undo N on_conflict: include_later` of an overwrite followed by a
    /// later overwrite of the same file.
    IncludeLater,
    /// `undo to: N` over the two operations that follow entry `N`.
    To,
    /// `redo` of the undo of the bulk operation.
    Redo,
    /// `undo` of one operation that overwrote many files.
    Many,
}

/// Source entry of a case with its history state before and after the call.
struct Source {
    op: u64,
    before: OpState,
    after: OpState,
}

const fn source(op: u64, before: OpState, after: OpState) -> Source {
    Source { op, before, after }
}

/// Journal call of a case.
enum Call {
    Undo(Box<UndoRequest>),
    Redo,
}

fn undo(selector: UndoSelector, on_conflict: OnConflict) -> UndoRequest {
    UndoRequest {
        selector,
        on_conflict,
        session: None,
        program: program(),
    }
}

fn bulk_ops() -> Vec<FileOp> {
    vec![
        FileOp::write(absolute("data/a"), NEW.to_vec()),
        FileOp::remove(absolute("data/b")),
        FileOp::create(absolute("data/new"), NEW.to_vec()),
    ]
}

fn bulk_changes() -> Vec<Change> {
    vec![
        write("data/a", NEW),
        remove("data/b"),
        write("data/new", NEW),
    ]
}

fn many_path(index: usize) -> String {
    format!("{MANY_DIR}/f{index:02}")
}

fn many_old(index: usize) -> Vec<u8> {
    format!("file {index} before the operation").into_bytes()
}

fn many_new(index: usize) -> Vec<u8> {
    format!("file {index} after the operation").into_bytes()
}

impl UndoCase {
    /// Cases small enough to run at every trigger.
    pub const SMALL: [UndoCase; 8] = [
        Self::Last,
        Self::Tree,
        Self::Bulk,
        Self::Paths,
        Self::Partial,
        Self::IncludeLater,
        Self::To,
        Self::Redo,
    ];

    /// Every case.
    pub const ALL: [UndoCase; 9] = [
        Self::Last,
        Self::Tree,
        Self::Bulk,
        Self::Paths,
        Self::Partial,
        Self::IncludeLater,
        Self::To,
        Self::Redo,
        Self::Many,
    ];

    /// Stable name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Last => "undo_last",
            Self::Tree => "undo_tree",
            Self::Bulk => "undo_bulk",
            Self::Paths => "undo_paths",
            Self::Partial => "undo_partial",
            Self::IncludeLater => "undo_include_later",
            Self::To => "undo_to",
            Self::Redo => "redo_bulk",
            Self::Many => "undo_many",
        }
    }

    fn prior(self) -> Vec<Prior> {
        let bulk = || Prior::Operation("write", bulk_ops());
        let write_a = |content: &[u8]| {
            Prior::Operation(
                "write",
                vec![FileOp::write(absolute("data/a"), content.to_vec())],
            )
        };
        match self {
            Self::Last => vec![write_a(NEW)],
            Self::Tree => vec![Prior::Operation(
                "remove",
                vec![FileOp::remove(absolute("data/tree"))],
            )],
            Self::Bulk | Self::Paths => vec![bulk()],
            Self::Partial => vec![bulk(), Prior::Release(OpId(1))],
            Self::IncludeLater => vec![write_a(NEW), write_a(NEWER)],
            Self::To => vec![
                Prior::Operation(
                    "write",
                    vec![FileOp::create(absolute("data/new"), NEW.to_vec())],
                ),
                write_a(NEW),
                Prior::Operation("remove", vec![FileOp::remove(absolute("data/b"))]),
            ],
            Self::Redo => vec![
                bulk(),
                Prior::Undo(Box::new(undo(UndoSelector::Last, OnConflict::Report))),
            ],
            Self::Many => vec![Prior::Operation(
                "write",
                (0..MANY_FILES)
                    .map(|index| FileOp::write(absolute(&many_path(index)), many_new(index)))
                    .collect(),
            )],
        }
    }

    fn call(self) -> Call {
        let paths = vec![TargetPath::Fs(absolute("data/a"))];
        Call::Undo(Box::new(match self {
            Self::Last | Self::Bulk | Self::Many => undo(UndoSelector::Last, OnConflict::Report),
            Self::Tree => undo(UndoSelector::Op(OpId(1)), OnConflict::Report),
            Self::Paths => undo(
                UndoSelector::Paths { op: OpId(1), paths },
                OnConflict::Report,
            ),
            Self::Partial => undo(UndoSelector::Op(OpId(1)), OnConflict::Partial),
            Self::IncludeLater => undo(UndoSelector::Op(OpId(1)), OnConflict::IncludeLater),
            Self::To => undo(
                UndoSelector::To {
                    after: OpId(1),
                    all_sessions: false,
                },
                OnConflict::Report,
            ),
            Self::Redo => return Call::Redo,
        }))
    }

    /// Tree before the prior operations.
    fn initial(self) -> TreeSnapshot {
        if self != Self::Many {
            return before();
        }
        let dir = Change::CreateDir {
            path: relative(MANY_DIR),
            mode: DIR_MODE,
        };
        let files: Vec<Change> = (0..MANY_FILES)
            .map(|index| write(&many_path(index), &many_old(index)))
            .collect();
        applied(before().with(&dir), &files)
    }

    /// Tree before the call: the prior operations applied.
    fn state_before(self) -> TreeSnapshot {
        let changes = match self {
            Self::Last => vec![write("data/a", NEW)],
            Self::Tree => vec![remove("data/tree")],
            Self::Bulk | Self::Paths | Self::Partial => bulk_changes(),
            Self::IncludeLater => vec![write("data/a", NEWER)],
            Self::To => vec![
                write("data/new", NEW),
                write("data/a", NEW),
                remove("data/b"),
            ],
            Self::Redo => Vec::new(),
            Self::Many => (0..MANY_FILES)
                .map(|index| write(&many_path(index), &many_new(index)))
                .collect(),
        };
        applied(self.initial(), &changes)
    }

    /// Tree after the call.
    fn state_after(self) -> TreeSnapshot {
        let changes = match self {
            Self::Last | Self::Tree | Self::Bulk | Self::IncludeLater | Self::Many => Vec::new(),
            Self::Paths => vec![remove("data/b"), write("data/new", NEW)],
            Self::Partial => vec![remove("data/b")],
            Self::To => vec![write("data/new", NEW)],
            Self::Redo => bulk_changes(),
        };
        applied(self.initial(), &changes)
    }

    fn sources(self) -> Vec<Source> {
        use nurt_contracts::record::OperationState::{Available, Partial, Undone};
        match self {
            Self::Last | Self::Tree | Self::Bulk | Self::Many => {
                vec![source(1, Available, Undone)]
            }
            Self::Paths => vec![source(1, Available, Partial)],
            Self::Partial => vec![source(1, Partial, Undone)],
            Self::IncludeLater => vec![source(1, Available, Undone), source(2, Available, Undone)],
            Self::To => vec![
                source(1, Available, Available),
                source(2, Available, Undone),
                source(3, Available, Undone),
            ],
            Self::Redo => vec![source(1, Undone, Undone), source(2, Available, Undone)],
        }
    }

    /// Number of the entry created by the call.
    fn created(self) -> u64 {
        let prior = self
            .prior()
            .iter()
            .filter(|step| step.creates_entry())
            .count();
        u64::try_from(prior).unwrap() + 1
    }

    fn setup(self, fs: &CrashFs) -> FsResult<()> {
        setup(fs)?;
        if self == Self::Many {
            fs.create_dir(&scope(), &absolute(MANY_DIR), DIR_MODE)?;
            for index in 0..MANY_FILES {
                let mut file = fs.create_new(&scope(), &absolute(&many_path(index)))?;
                file.write_all(&many_old(index))
                    .map_err(|_| FsError::new(FsErrorKind::Io, "write", None))?;
            }
        }
        perform_prior(fs, self.prior())
    }

    fn perform(self, journal: &Journal) -> OpOutcome {
        let irreversible = |outcome: &UndoOutcome| !outcome.irreversible.is_empty();
        match self.call() {
            Call::Undo(request) => outcome_of(&journal.undo(&request), irreversible),
            Call::Redo => {
                let request = RedoRequest {
                    undo_entry: None,
                    session: None,
                    program: program(),
                };
                outcome_of(&journal.redo(&request), irreversible)
            }
        }
    }
}

/// Checks that the history agrees with the state the tree matched.
fn check_sources(case: UndoCase, journal: &Journal, matched: Matched) -> Result<(), Violation> {
    let records = records(journal)?;
    let state = |op: u64| records.get(&op).map(|record| record.state);
    let mut wrong: Vec<PathBuf> = case
        .sources()
        .into_iter()
        .filter(|source| {
            let want = match matched {
                Matched::Before => source.before,
                Matched::After => source.after,
            };
            state(source.op) != Some(want)
        })
        .map(|source| entry_path(OpId(source.op)))
        .collect();
    let created = state(case.created());
    let created_agrees = match matched {
        Matched::After => created == Some(OpState::Available),
        Matched::Before => created.is_none_or(|state| state == OpState::Undone),
    };
    if !created_agrees {
        wrong.push(entry_path(OpId(case.created())));
    }
    if wrong.is_empty() {
        return Ok(());
    }
    Err(match matched {
        Matched::Before => Violation::IntermediateState {
            from_before: wrong,
            from_after: Vec::new(),
        },
        Matched::After => Violation::IntermediateState {
            from_before: Vec::new(),
            from_after: wrong,
        },
    })
}

/// Oracle of the undo cases.
///
/// # Errors
///
/// Fails with the first violated property.
pub fn check(
    case: UndoCase,
    fs: &CrashFs,
    phase: Phase,
    outcome: OpOutcome,
) -> Result<(), Violation> {
    check_explicit(outcome)?;
    let volume = volume_tree(fs)?;
    let tree = user_tree(fs)?;
    let (start, end) = (case.state_before(), case.state_after());
    let targets = [relative(DATA)];
    if outcome != (OpOutcome::Committed { irreversible: true }) {
        check_preserved(&start, &targets, &volume)?;
    }
    check_preserved(&end, &targets, &volume)?;
    match (phase, outcome) {
        (Phase::AfterCrash, OpOutcome::Committed { .. }) => {
            check_complete(&tree.without_temporary(), &start, &end, Expected::After)?;
            let journal = view(fs);
            recover(&journal)?;
            let tree = user_tree(fs)?;
            check_no_leftovers(&tree)?;
            check_complete(&tree, &start, &end, Expected::After)?;
            check_sources(case, &journal, Matched::After)
        }
        (Phase::AfterCrash, _) => Ok(()),
        (Phase::AfterRecovery, _) => {
            let expected = match outcome {
                OpOutcome::Committed { .. } => Expected::After,
                OpOutcome::RolledBack => Expected::Before,
                _ => Expected::Either,
            };
            check_no_leftovers(&tree)?;
            let matched = check_complete(&tree, &start, &end, expected)?;
            check_sources(case, &view(fs), matched)
        }
    }
}

/// Undo case as a crash scenario.
#[derive(Clone, Copy, Debug)]
pub struct UndoScenario(pub UndoCase);

impl Scenario for UndoScenario {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn volumes(&self) -> Vec<VolumeSpec> {
        vec![VolumeSpec::new(volume_root())]
    }

    fn setup(&self, fs: &CrashFs) -> FsResult<()> {
        self.0.setup(fs)
    }

    fn run(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> OpOutcome {
        self.0.perform(&instrumented(fs, failpoints))
    }

    fn recover(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> FsResult<()> {
        recover(&instrumented(fs, failpoints))
    }

    fn check(&self, fs: &CrashFs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
        check(self.0, fs, phase, outcome)
    }
}

fn hits(enumeration: &Enumeration, point: FailPoint) -> usize {
    enumeration
        .protocol
        .iter()
        .filter(|trigger| matches!(trigger, Trigger::Protocol { point: hit, .. } if *hit == point))
        .count()
}

#[test]
fn undo_and_redo_cases_commit_without_faults_and_survive_power_loss() {
    for case in UndoCase::ALL {
        let scenario = UndoScenario(case);
        let fs = CrashFs::new(CrashProfile::PosixWorstCase, UNDO_SEED, scenario.volumes()).unwrap();
        scenario.setup(&fs).unwrap();
        fs.sync_all();
        let failpoints = Arc::new(SeededFailpoints::new(FailPlan::observe(UNDO_SEED)));
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
fn undo_and_redo_crash_at_every_protocol_point_and_call_end_in_a_complete_state() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    for case in UndoCase::SMALL {
        let scenario = UndoScenario(case);
        let enumeration = harness.enumerate(&scenario, UNDO_SEED).unwrap();
        assert!(
            hits(&enumeration, FailPoint::UndoChecked) > 0,
            "{}: the call hit no undo check",
            case.name()
        );
        let report = harness.run_all(&scenario, &enumeration, UNDO_SEED);
        assert_clean(&report);
        assert_eq!(report.points, enumeration.triggers().len() as u64);
    }
}

#[test]
fn bulk_undo_crash_at_every_restored_item_ends_in_a_complete_state() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let scenario = UndoScenario(UndoCase::Many);
    let enumeration = harness.enumerate(&scenario, UNDO_SEED).unwrap();
    assert!(hits(&enumeration, FailPoint::UndoRestored) >= MANY_FILES);
    let protocol = Enumeration {
        protocol: enumeration.protocol.clone(),
        fs_calls: 0,
    };
    let mut report = harness.run_all(&scenario, &protocol, UNDO_SEED);
    report.merge(harness.run_random(&scenario, &enumeration, UNDO_SEED, MANY_RANDOM_POINTS));
    assert_clean(&report);
}

#[test]
fn undo_crash_during_recovery_and_again_ends_in_a_complete_state() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let run = Trigger::Protocol {
        point: FailPoint::ApplyDone,
        n: None,
        occurrence: 0,
    };
    let recovery_points = [
        Trigger::Protocol {
            point: FailPoint::RecoveryStarted,
            n: None,
            occurrence: 0,
        },
        Trigger::Protocol {
            point: FailPoint::RecoveryEntry,
            n: None,
            occurrence: 0,
        },
        Trigger::FsCall { call: 1 },
        Trigger::FsCall { call: 2 },
    ];
    for case in [UndoCase::Bulk, UndoCase::Partial, UndoCase::Redo] {
        let scenario = UndoScenario(case);
        for (seed, recovery) in (UNDO_SEED..).zip(recovery_points) {
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
fn undo_crash_at_random_points_on_every_profile_ends_in_a_complete_state() {
    for profile in CrashProfile::ALL {
        let harness = Harness::new(profile);
        for case in UndoCase::SMALL {
            let scenario = UndoScenario(case);
            let enumeration = harness.enumerate(&scenario, UNDO_SEED).unwrap();
            let report =
                harness.run_random(&scenario, &enumeration, UNDO_SEED, RANDOM_POINTS_PER_CASE);
            assert_clean(&report);
        }
    }
}

#[test]
#[ignore = "long crash corpus; run: cargo test -p nurt-crashfs --release --test crash -- --ignored undo_crash_corpus_long"]
fn undo_crash_corpus_long_random_points() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let per_case = LONG_POINTS.div_ceil(UndoCase::ALL.len() as u64);
    let mut points = 0;
    for case in UndoCase::ALL {
        let scenario = UndoScenario(case);
        let enumeration = harness.enumerate(&scenario, UNDO_SEED).unwrap();
        let report = harness.run_random(&scenario, &enumeration, UNDO_SEED, per_case);
        assert_clean(&report);
        points += report.points;
    }
    assert!(points >= LONG_POINTS);
}

#[test]
fn undo_last_recovery_crash_on_xfs_keeps_the_entry_restorable() {
    let harness = Harness::new(CrashProfile::Xfs);
    let scenario = UndoScenario(UndoCase::Last);
    let point = CrashPoint {
        run: Trigger::FsCall { call: 37 },
        recovery: Some(Trigger::FsCall { call: 4 }),
    };
    let report = harness
        .run_point(&scenario, 9_833_594_160_106_553_642, point)
        .unwrap_or_else(|failure| panic!("{failure:?}"));
    assert_eq!(report.crashes, 2);
}
