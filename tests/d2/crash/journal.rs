// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/journal.rs
// Responsibility:   Crash corpus of the journal transaction protocol.
// Dependencies:     nurt_contracts, nurt_crashfs, nurt_fs, nurt_journal
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Crash corpus of the journal transaction protocol.
//!
//! Each case performs one operation through `Journal::begin`, `apply_ops` and
//! `MutationTx::commit` on a `CrashFs` volume, with the seeded failpoint
//! registry attached to the journal and to the file system. A crash at any
//! protocol point or file system call must leave the previous content of
//! every target on the volume, in the tree or in the journal. Recovery runs
//! through `JournalStore::recover` in a new journal instance on the image
//! left by the crash, as a restarted process does; afterwards the tree is in
//! the state before or after the operation, without staged files, and a
//! second recovery changes nothing. An operation that returned must match
//! its result and survive a power loss that follows.
//!
//! The totals of each run are written as JSON to the directory named by the
//! `NURT_CRASH_REPORT_DIR` environment variable when it is set.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::config::OverwriteStrategy;
use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{
    AbsPath, Fs, FsError, FsErrorKind, FsResult, FsWrite, MetaChange, VolumeId, VolumeInfo,
};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, AdmissionRequest, BeginRequest, Eligibility, FileOp,
    InactiveReason, JournalStore, OpKind, ServiceLiveness, Thresholds, VolumeState, VolumeStatus,
    apply_ops,
};
use nurt_contracts::report::Irreversible;
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};
use nurt_crashfs::oracle::{
    Change, Expected, OpOutcome, TreeSnapshot, Violation, check_complete, check_explicit,
    check_no_leftovers, check_preserved,
};
use nurt_crashfs::points::{CrashPoint, Harness, Phase, Scenario, Trigger};
use nurt_fs::clock::VirtualClock;
use nurt_fs::failpoint::{FailPoint, Failpoints, SeededFailpoints};
use nurt_journal::ConfiguredLimits;
use nurt_journal::layout::DIR_NAME;
use nurt_journal::store::{Journal, JournalBuilder, JournalLocation, VolumeJournals};

use crate::corpus::assert_clean;
use crate::support::{scope, volume_root};

/// Account state directory of the journal, on the volume under test.
pub const STATE_DIR: &str = ".nurt-state";
/// Content written by the operations.
pub const NEW: &[u8] = b"content written by the operation, longer than any previous one";

/// Room left for journal metadata on a full volume: enough for the entry
/// files, less than the content of `data/large`.
pub const METADATA_ROOM: u64 = 8 * 1024;

const DIR_MODE: u32 = 0o755;
/// Permission bits of the state and journal directories, created by the
/// volume registry before the first operation.
const PRIVATE_MODE: u32 = 0o700;
/// Size of `data/large`, the file preserved by copy.
const LARGE_SIZE: usize = 16 * 1024;
const LARGE: &[u8] = &[b'l'; LARGE_SIZE];
const MADE_DIR_MODE: u32 = 0o750;
const NARROW_MODE: u32 = 0o600;
/// Start of the virtual clock: 2026-01-01T00:00:00Z.
const CLOCK_START_NS: i128 = 1_767_225_600_000_000_000;
const CORPUS_SEED: u64 = 0x6A6F_7572_6E61_6C00;
/// Random points per case and profile in an ordinary test run.
const RANDOM_POINTS_PER_CASE: u64 = 8;
/// Random points of the long corpus on the worst-case profile, all cases
/// together.
const LONG_POINTS: u64 = 100_000;

const OLD_A: &[u8] = b"first file before the operation";
const OLD_B: &[u8] = b"second file before the operation";
const OLD_C: &[u8] = b"destination file before the operation";
const LOG: &[u8] = b"line 1\n";
const APPENDED: &[u8] = b"line 2\n";
const TREE_X: &[u8] = b"file in the tree";
const TREE_Y: &[u8] = b"file in a subdirectory of the tree";

/// Directories of the initial state, parents first.
const DIRS: [&str; 3] = ["data", "data/tree", "data/tree/sub"];
/// Files of the initial state: three small files, a file larger than
/// [`METADATA_ROOM`], an append log and a tree with a nested directory.
const FILES: [(&str, &[u8]); 7] = [
    ("data/a", OLD_A),
    ("data/large", LARGE),
    ("data/b", OLD_B),
    ("data/c", OLD_C),
    ("data/log", LOG),
    ("data/tree/x", TREE_X),
    ("data/tree/sub/y", TREE_Y),
];

fn relative(path: &str) -> PathBuf {
    path.split('/').collect()
}

/// Absolute path of `path`, written with `/`, in the volume root.
pub fn absolute(path: &str) -> AbsPath {
    let mut full = volume_root().as_path().to_path_buf();
    full.extend(path.split('/'));
    AbsPath::new(full).unwrap()
}

/// Journal directory of the volume under test.
pub fn journal_dir() -> AbsPath {
    absolute(DIR_NAME)
}

/// Account state directory.
pub fn state_dir() -> AbsPath {
    absolute(STATE_DIR)
}

/// Bytes of file content in the initial state.
pub fn initial_bytes() -> u64 {
    FILES.iter().map(|(_, content)| content.len() as u64).sum()
}

/// Operation performed by a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Overwrite of a file; the old file is kept by exchange of names.
    OverwriteByExchange,
    /// Overwrite on a volume without exchange; the old file is kept by a hard
    /// link.
    OverwriteByHardLink,
    /// Overwrite with the clone strategy on a volume with block cloning.
    OverwriteByClone,
    /// Overwrite with the clone strategy on a volume without block cloning.
    OverwriteByCopy,
    /// Append to a file.
    Append,
    /// Removal of a file.
    RemoveFile,
    /// Removal of a directory tree.
    RemoveTree,
    /// Rename over an existing destination.
    RenameOverDestination,
    /// Creation of a file.
    CreateFile,
    /// Creation of a directory.
    CreateDirectory,
    /// Change of permission bits.
    ChangeMode,
    /// Bulk operation: overwrite, removal and creation.
    Bulk,
}

impl Case {
    /// Every case.
    pub const ALL: [Case; 12] = [
        Self::OverwriteByExchange,
        Self::OverwriteByHardLink,
        Self::OverwriteByClone,
        Self::OverwriteByCopy,
        Self::Append,
        Self::RemoveFile,
        Self::RemoveTree,
        Self::RenameOverDestination,
        Self::CreateFile,
        Self::CreateDirectory,
        Self::ChangeMode,
        Self::Bulk,
    ];

    /// Stable name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::OverwriteByExchange => "journal_overwrite_by_exchange",
            Self::OverwriteByHardLink => "journal_overwrite_by_hard_link",
            Self::OverwriteByClone => "journal_overwrite_by_clone",
            Self::OverwriteByCopy => "journal_overwrite_by_copy",
            Self::Append => "journal_append",
            Self::RemoveFile => "journal_remove_file",
            Self::RemoveTree => "journal_remove_tree",
            Self::RenameOverDestination => "journal_rename_over_destination",
            Self::CreateFile => "journal_create_file",
            Self::CreateDirectory => "journal_create_directory",
            Self::ChangeMode => "journal_change_mode",
            Self::Bulk => "journal_bulk",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::OverwriteByExchange
            | Self::OverwriteByHardLink
            | Self::OverwriteByClone
            | Self::OverwriteByCopy
            | Self::CreateFile
            | Self::Bulk => "write",
            Self::Append => "append",
            Self::RemoveFile | Self::RemoveTree => "remove",
            Self::RenameOverDestination => "move",
            Self::CreateDirectory => "mkdir",
            Self::ChangeMode => "chmod",
        }
    }

    /// Strategy of keeping overwritten files.
    pub fn strategy(self) -> OverwriteStrategy {
        match self {
            Self::OverwriteByClone | Self::OverwriteByCopy => OverwriteStrategy::CloneOrCopy,
            _ => OverwriteStrategy::Exchange,
        }
    }

    /// Volume of the case.
    pub fn volume(self) -> VolumeSpec {
        let spec = VolumeSpec::new(volume_root());
        let mut features = spec.features;
        match self {
            Self::OverwriteByHardLink => features.exchange = false,
            Self::OverwriteByCopy => features.reflink = false,
            _ => {}
        }
        spec.with_features(features)
    }

    fn ops(self) -> Vec<FileOp> {
        let new = || NEW.to_vec();
        match self {
            Self::OverwriteByExchange | Self::OverwriteByHardLink | Self::OverwriteByClone => {
                vec![FileOp::write(absolute("data/a"), new())]
            }
            Self::OverwriteByCopy => vec![FileOp::write(absolute("data/large"), new())],
            Self::Append => vec![FileOp::append(absolute("data/log"), APPENDED.to_vec())],
            Self::RemoveFile => vec![FileOp::remove(absolute("data/b"))],
            Self::RemoveTree => vec![FileOp::remove(absolute("data/tree"))],
            Self::RenameOverDestination => {
                vec![FileOp::rename(absolute("data/a"), absolute("data/c"), true)]
            }
            Self::CreateFile => vec![FileOp::create(absolute("data/new"), new())],
            Self::CreateDirectory => vec![FileOp::create_dir(absolute("data/made"), MADE_DIR_MODE)],
            Self::ChangeMode => vec![FileOp::set_meta(
                absolute("data/a"),
                MetaChange {
                    mode: Some(NARROW_MODE),
                    ..MetaChange::default()
                },
            )],
            Self::Bulk => vec![
                FileOp::write(absolute("data/a"), new()),
                FileOp::remove(absolute("data/b")),
                FileOp::create(absolute("data/new"), new()),
            ],
        }
    }

    /// Changes of the tree made by the operation, independent of the
    /// journal.
    fn changes(self) -> Vec<Change> {
        let write = |path: &str, content: &[u8]| Change::Write {
            path: relative(path),
            content: content.to_vec(),
        };
        let remove = |path: &str| Change::Remove {
            path: relative(path),
        };
        match self {
            Self::OverwriteByExchange | Self::OverwriteByHardLink | Self::OverwriteByClone => {
                vec![write("data/a", NEW)]
            }
            Self::OverwriteByCopy => vec![write("data/large", NEW)],
            Self::Append => vec![write("data/log", &[LOG, APPENDED].concat())],
            Self::RemoveFile => vec![remove("data/b")],
            Self::RemoveTree => vec![remove("data/tree")],
            Self::RenameOverDestination => vec![Change::Rename {
                from: relative("data/a"),
                to: relative("data/c"),
            }],
            Self::CreateFile => vec![write("data/new", NEW)],
            Self::CreateDirectory => vec![Change::CreateDir {
                path: relative("data/made"),
                mode: MADE_DIR_MODE,
            }],
            Self::ChangeMode => vec![Change::SetMode {
                path: relative("data/a"),
                mode: NARROW_MODE,
            }],
            Self::Bulk => vec![
                write("data/a", NEW),
                remove("data/b"),
                write("data/new", NEW),
            ],
        }
    }

    /// Targets whose previous content must exist on the volume at every
    /// moment. An appended file keeps its previous content as a prefix,
    /// checked separately.
    fn targets(self) -> Vec<PathBuf> {
        let paths: &[&str] = match self {
            Self::OverwriteByExchange
            | Self::OverwriteByHardLink
            | Self::OverwriteByClone
            | Self::ChangeMode => &["data/a"],
            Self::OverwriteByCopy => &["data/large"],
            Self::Append | Self::CreateFile | Self::CreateDirectory => &[],
            Self::RemoveFile => &["data/b"],
            Self::RemoveTree => &["data/tree"],
            Self::RenameOverDestination => &["data/a", "data/c"],
            Self::Bulk => &["data/a", "data/b"],
        };
        paths.iter().map(|path| relative(path)).collect()
    }

    /// Expected tree after the operation.
    pub fn after(self) -> TreeSnapshot {
        self.changes()
            .iter()
            .fold(before(), |tree, change| tree.with(change))
    }
}

/// Creates the initial state shared by every case.
///
/// # Errors
///
/// Fails with the error of the file system call that failed.
pub fn setup(fs: &dyn FsWrite) -> FsResult<()> {
    fs.create_dir(&scope(), &state_dir(), PRIVATE_MODE)?;
    fs.create_dir(&scope(), &journal_dir(), PRIVATE_MODE)?;
    for dir in DIRS {
        fs.create_dir(&scope(), &absolute(dir), DIR_MODE)?;
    }
    for (path, content) in FILES {
        let mut file = fs.create_new(&scope(), &absolute(path))?;
        file.write_all(content)
            .map_err(|_| FsError::new(FsErrorKind::Io, "write", None))?;
    }
    Ok(())
}

/// Expected tree before the operation, the journal and state directories
/// excluded.
pub fn before() -> TreeSnapshot {
    let dirs = DIRS.iter().map(|dir| Change::CreateDir {
        path: relative(dir),
        mode: DIR_MODE,
    });
    let files = FILES.iter().map(|(path, content)| Change::Write {
        path: relative(path),
        content: content.to_vec(),
    });
    dirs.chain(files)
        .fold(TreeSnapshot::default(), |tree, change| tree.with(&change))
}

/// Admission that keeps every previous content.
struct KeepAdmission;

impl Admission for KeepAdmission {
    fn eligibility(&self, _volume: &VolumeInfo) -> Eligibility {
        Eligibility::Eligible
    }

    fn admit(&self, _request: &AdmissionRequest) -> AdmissionDecision {
        AdmissionDecision::Keep
    }

    fn after_operation(&self, _volume: &VolumeId) {}

    fn volume_status(&self, _volume: &VolumeId) -> VolumeStatus {
        VolumeStatus {
            state: VolumeState::Normal,
            thresholds: Thresholds {
                reserve: 0,
                soft: 0,
                margin: 0,
                hysteresis: 0,
            },
        }
    }

    fn liveness(&self) -> ServiceLiveness {
        ServiceLiveness::Alive
    }
}

/// Journal of the volume under test, in its root directory.
struct RootJournal;

impl VolumeJournals for RootJournal {
    fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
        if volume.mount == volume_root() {
            JournalLocation::Active(journal_dir())
        } else {
            JournalLocation::Inactive(InactiveReason::NoAnchor)
        }
    }

    fn active(&self) -> Vec<AbsPath> {
        vec![journal_dir()]
    }
}

/// Journal working on `fs`; `model` serves the reads of the retention
/// limits.
pub fn journal(
    fs: Arc<dyn FsWrite>,
    model: &CrashFs,
    failpoints: Option<Arc<dyn Failpoints>>,
    strategy: OverwriteStrategy,
) -> Journal {
    let limits = ConfiguredLimits::new(&[], false, Arc::new(model.clone()), &state_dir()).unwrap();
    let clock = VirtualClock::new(WallTime::from_unix_nanos(CLOCK_START_NS));
    let builder = JournalBuilder::new(
        fs,
        Arc::new(KeepAdmission),
        Arc::new(clock),
        Arc::new(RootJournal),
        Arc::new(limits),
        state_dir(),
    )
    .overwrite(strategy);
    match failpoints {
        Some(registry) => builder.failpoints(registry).build(),
        None => builder.build(),
    }
    .unwrap()
}

fn request(case: Case, bulk: bool) -> BeginRequest {
    BeginRequest {
        kind: OpKind::command(case.command()),
        session: None,
        program: BoundDocument::new(
            "catalog",
            "corrections",
            BoundProgram {
                statements: Vec::new(),
            },
        ),
        corrections: Vec::new(),
        bulk,
        counts_as_step: true,
        preserve_budget: None,
    }
}

/// Result of one operation as seen by its caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Performed {
    /// Outcome for the oracle.
    pub outcome: OpOutcome,
    /// Irreversible reasons reported by the transaction.
    pub reasons: Vec<Irreversible>,
    /// Code of the error that ended the operation.
    pub code: Option<ErrorCode>,
}

impl Performed {
    /// An injected crash stops the transaction with `E_INTERNAL`; every
    /// other error has rolled the transaction back before it is returned.
    fn failed(error: &Error) -> Self {
        let code = error.code();
        let outcome = if code == ErrorCode::Internal {
            OpOutcome::Crashed
        } else {
            OpOutcome::RolledBack
        };
        Self {
            outcome,
            reasons: Vec::new(),
            code: Some(code),
        }
    }
}

/// Performs the operation of `case` as one transaction.
pub fn perform(journal: &Journal, case: Case) -> Performed {
    let ops = case.ops();
    let mut tx = journal.begin(request(case, ops.len() > 1));
    let outcomes = match apply_ops(tx.as_mut(), ops) {
        Ok(outcomes) => outcomes,
        Err(error) => return Performed::failed(&error),
    };
    match tx.commit() {
        Ok(handle) => {
            let mut reasons: Vec<Irreversible> = outcomes
                .iter()
                .filter_map(|outcome| outcome.preserved.irreversible)
                .collect();
            reasons.extend(
                handle
                    .targets
                    .iter()
                    .filter_map(|target| target.irreversible),
            );
            reasons.extend(handle.irreversible.iter().map(|entry| entry.reason));
            Performed {
                outcome: OpOutcome::Committed {
                    irreversible: !reasons.is_empty(),
                },
                reasons,
                code: None,
            }
        }
        Err(error) => Performed::failed(&error),
    }
}

fn check_appended_prefix(case: Case, fs: &dyn Fs) -> Result<(), Violation> {
    if case != Case::Append {
        return Ok(());
    }
    let mut content = Vec::new();
    fs.open_read(&absolute("data/log"))?
        .read_to_end(&mut content)
        .map_err(|err| Violation::Unreadable {
            detail: err.to_string(),
        })?;
    if content.starts_with(LOG) {
        Ok(())
    } else {
        Err(Violation::PreviousContentLost {
            path: relative("data/log"),
        })
    }
}

/// Oracle of the journal cases.
///
/// Previous content is required on the volume unless the result reports an
/// irreversible target. A returned operation must leave the tree in the state
/// it reports, without staged files; after a power loss that follows a
/// commit, the tree must still be in the state after the operation, staged
/// files aside, since recovery removes them. After the recovery of a crashed
/// operation the tree is in either complete state.
///
/// # Errors
///
/// Fails with the first violated property.
pub fn check(case: Case, fs: &dyn Fs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
    check_explicit(outcome)?;
    let volume = TreeSnapshot::capture(fs, &volume_root(), &[])?;
    let tree = TreeSnapshot::capture(
        fs,
        &volume_root(),
        &[Path::new(DIR_NAME), Path::new(STATE_DIR)],
    )?;
    if outcome != (OpOutcome::Committed { irreversible: true }) {
        check_preserved(&before(), &case.targets(), &volume)?;
        check_appended_prefix(case, fs)?;
    }
    let expected = match (phase, outcome) {
        (Phase::AfterRecovery, OpOutcome::Committed { .. }) => Expected::After,
        (Phase::AfterRecovery, OpOutcome::RolledBack) => Expected::Before,
        (Phase::AfterRecovery, _) => Expected::Either,
        (Phase::AfterCrash, OpOutcome::Committed { .. }) => {
            return check_complete(
                &tree.without_temporary(),
                &before(),
                &case.after(),
                Expected::After,
            )
            .map(|_| ());
        }
        _ => return Ok(()),
    };
    check_no_leftovers(&tree)?;
    check_complete(&tree, &before(), &case.after(), expected).map(|_| ())
}

/// Journal case as a crash scenario.
#[derive(Clone, Copy, Debug)]
pub struct JournalCase(pub Case);

impl Scenario for JournalCase {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn volumes(&self) -> Vec<VolumeSpec> {
        vec![self.0.volume()]
    }

    fn setup(&self, fs: &CrashFs) -> FsResult<()> {
        setup(fs)
    }

    fn run(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> OpOutcome {
        let registry: Arc<dyn Failpoints> = Arc::clone(failpoints) as Arc<dyn Failpoints>;
        let journal = journal(Arc::new(fs.clone()), fs, Some(registry), self.0.strategy());
        perform(&journal, self.0).outcome
    }

    fn recover(&self, fs: &CrashFs, failpoints: &Arc<SeededFailpoints>) -> FsResult<()> {
        let registry: Arc<dyn Failpoints> = Arc::clone(failpoints) as Arc<dyn Failpoints>;
        let journal = journal(Arc::new(fs.clone()), fs, Some(registry), self.0.strategy());
        journal
            .recover()
            .map(|_| ())
            .map_err(|error| FsError::new(FsErrorKind::Io, &format!("recover: {error}"), None))
    }

    fn check(&self, fs: &CrashFs, phase: Phase, outcome: OpOutcome) -> Result<(), Violation> {
        check(self.0, fs, phase, outcome)
    }
}

fn prepared(case: Case) -> CrashFs {
    let fs = CrashFs::new(
        CrashProfile::PosixWorstCase,
        CORPUS_SEED,
        vec![case.volume()],
    )
    .unwrap();
    setup(&fs).unwrap();
    fs.sync_all();
    fs
}

#[test]
fn journal_cases_commit_and_keep_previous_content_without_faults() {
    for case in Case::ALL {
        let fs = prepared(case);
        let journal = journal(Arc::new(fs.clone()), &fs, None, case.strategy());
        let performed = perform(&journal, case);
        assert_eq!(
            performed.outcome,
            OpOutcome::Committed {
                irreversible: false
            },
            "{}: {performed:?}",
            case.name()
        );
        check(case, &fs, Phase::AfterRecovery, performed.outcome)
            .unwrap_or_else(|violation| panic!("{}: {violation}", case.name()));
        fs.crash();
        check(case, &fs, Phase::AfterCrash, performed.outcome)
            .unwrap_or_else(|violation| panic!("{} after power loss: {violation}", case.name()));
    }
}

#[test]
fn journal_crash_at_every_protocol_point_and_call_keeps_previous_content() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    for case in Case::ALL {
        let scenario = JournalCase(case);
        let enumeration = harness.enumerate(&scenario, CORPUS_SEED).unwrap();
        assert!(
            !enumeration.protocol.is_empty(),
            "{}: the operation hit no protocol point",
            case.name()
        );
        let report = harness.run_all(&scenario, &enumeration, CORPUS_SEED);
        assert_clean(&report);
        assert_eq!(report.points, enumeration.triggers().len() as u64);
    }
}

#[test]
fn journal_crash_during_recovery_and_again_ends_in_a_complete_state() {
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
    for case in [Case::OverwriteByExchange, Case::RemoveTree, Case::Bulk] {
        let scenario = JournalCase(case);
        for (seed, recovery) in (CORPUS_SEED..).zip(recovery_points) {
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
fn journal_crash_at_random_points_on_every_profile_keeps_previous_content() {
    for profile in CrashProfile::ALL {
        let harness = Harness::new(profile);
        for case in Case::ALL {
            let scenario = JournalCase(case);
            let enumeration = harness.enumerate(&scenario, CORPUS_SEED).unwrap();
            let report =
                harness.run_random(&scenario, &enumeration, CORPUS_SEED, RANDOM_POINTS_PER_CASE);
            assert_clean(&report);
        }
    }
}

#[test]
#[ignore = "long crash corpus; run: cargo test -p nurt-crashfs --release --test crash -- --ignored journal_crash_corpus_long"]
fn journal_crash_corpus_long_random_points() {
    let harness = Harness::new(CrashProfile::PosixWorstCase);
    let per_case = LONG_POINTS.div_ceil(Case::ALL.len() as u64);
    let mut points = 0;
    for case in Case::ALL {
        let scenario = JournalCase(case);
        let enumeration = harness.enumerate(&scenario, CORPUS_SEED).unwrap();
        let report = harness.run_random(&scenario, &enumeration, CORPUS_SEED, per_case);
        assert_clean(&report);
        points += report.points;
    }
    assert!(points >= LONG_POINTS);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
