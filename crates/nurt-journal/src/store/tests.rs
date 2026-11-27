// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::store::tests
// Responsibility:   Journal store on the in-memory file system, driven through the journal.
// Dependencies:     nurt_contracts, nurt_fs, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Journal store on the in-memory file system, driven through the journal
//! interface and the entry catalog.
//!
//! The file system has the volumes `/` and `/home`; only `/home` has a
//! journal, in `/home/<journal directory>`, and the account state directory
//! lies on it as well.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::config::{ConfigLayer, ConfigLevel, StepLimit};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{AbsPath, Fs as _, FsWrite as _, RenameMode, ScopePurpose, VolumeInfo};
use nurt_contracts::journal::{
    BeginRequest, EntryState, ExpiredEntry, ExpiryReason, FileOp, HistoryQuery, InactiveReason,
    JournalStore, LimitsRequest, LimitsScope, OnConflict, OpHandle, OpId, OpKind, OpState,
    PinRequest, PurgeRequest, PurgeSelector, RedoRequest, SessionId, UndoRequest, UndoSelector,
    apply_ops,
};
use nurt_fs::clock::VirtualClock;
use nurt_fs::failpoint::{FailAction, FailPlan, FailPoint, FailRule, SeededFailpoints};
use nurt_fs::mem::{MemFs, VolumeSpec};
use serde_json::{Value, json};

use super::test_support::TestAdmission;
use super::{EntryCatalog, Journal, JournalBuilder, JournalLocation, VolumeJournals, scope};
use crate::ConfiguredLimits;
use crate::layout::{self, JournalLayout, StateLayout};
use crate::manifest::{self, IndexRow, StateRecord};

const HOME: &str = "/home";
const STATE_DIR: &str = "/home/user/.local/state/nurt";
const OTHER_STATE_DIR: &str = "/home/other/.local/state/nurt";
const OTHER_FILE: &str = "/home/user/docs/other.txt";
const NOTES: &str = "/home/user/docs/notes.txt";
const USER_CONFIG: &str = "/home/user/.config/nurt/config.toml";
/// Just over the built-in age limit of 30 minutes.
const PAST_AGE_LIMIT: Duration = Duration::from_secs(31 * 60);
/// Short lock wait, so that a held entry lock fails a second taker at once.
const LOCK_TIMEOUT: Duration = Duration::from_millis(20);

fn path(text: &str) -> AbsPath {
    AbsPath::for_test(text)
}

fn journal_root() -> AbsPath {
    path(HOME).join_name(layout::default_dir_name()).unwrap()
}

fn journal_layout() -> JournalLayout {
    JournalLayout::new(journal_root())
}

/// Journal directory of the volume `/home` only.
struct HomeJournal;

impl VolumeJournals for HomeJournal {
    fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
        if volume.mount == path(HOME) {
            JournalLocation::Active(journal_root())
        } else {
            JournalLocation::Inactive(InactiveReason::NoAnchor)
        }
    }

    fn active(&self) -> Vec<AbsPath> {
        vec![journal_root()]
    }
}

fn program() -> BoundDocument {
    BoundDocument::new(
        "catalog",
        "corrections",
        BoundProgram {
            statements: Vec::new(),
        },
    )
}

fn request(command: &str) -> BeginRequest {
    BeginRequest {
        kind: OpKind::command(command),
        session: None,
        program: program(),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        preserve_budget: None,
    }
}

fn undo_last() -> UndoRequest {
    UndoRequest {
        selector: UndoSelector::Last,
        on_conflict: OnConflict::Report,
        session: None,
        program: program(),
    }
}

fn steps(count: u32, preview: bool) -> LimitsRequest {
    LimitsRequest {
        steps: Some(StepLimit::Limited(count)),
        age: None,
        size: None,
        scope: LimitsScope::Session(SessionId("s1".to_owned())),
        volume: None,
        preview,
    }
}

fn user_steps(count: u32) -> LimitsRequest {
    LimitsRequest {
        scope: LimitsScope::User,
        ..steps(count, false)
    }
}

fn ids(list: &[OpId]) -> Vec<u64> {
    list.iter().map(|op| op.0).collect()
}

/// In-memory volumes, a virtual clock and the user configuration layer
/// `{ journal: <settings> }`.
struct Env {
    fs: Arc<MemFs>,
    clock: Arc<VirtualClock>,
    admission: Arc<TestAdmission>,
    layers: Vec<ConfigLayer>,
}

impl Env {
    fn new(settings: &Value) -> Self {
        let clock = Arc::new(VirtualClock::default());
        let fs = MemFs::builder()
            .clock(clock.clone())
            .volume(VolumeSpec::posix(path("/")))
            .volume(VolumeSpec::posix(path(HOME)))
            .build()
            .unwrap();
        for dir in journal_layout().structure().unwrap() {
            fs.seed_dir(&dir).unwrap();
        }
        fs.seed_dir(&path(STATE_DIR)).unwrap();
        fs.seed_dir(&path("/home/user/docs")).unwrap();
        let values = json!({ "journal": settings });
        Self {
            fs: Arc::new(fs),
            clock,
            admission: Arc::new(TestAdmission::default()),
            layers: vec![ConfigLayer::new(
                ConfigLevel::User,
                "user.toml",
                values.as_object().cloned().unwrap_or_default(),
            )],
        }
    }

    fn builder(&self) -> JournalBuilder {
        self.builder_in(STATE_DIR)
    }

    /// Builder of the journal of the account whose state directory is
    /// `state_dir`; the journal directory of the volume is shared.
    fn builder_in(&self, state_dir: &str) -> JournalBuilder {
        let limits =
            ConfiguredLimits::new(&self.layers, false, self.fs.clone(), &path(state_dir)).unwrap();
        JournalBuilder::new(
            self.fs.clone(),
            self.admission.clone(),
            self.clock.clone(),
            Arc::new(HomeJournal),
            Arc::new(limits),
            path(state_dir),
        )
        .lock_timeout(LOCK_TIMEOUT)
    }

    /// Journal of a second account that works on the same volume.
    fn other_account(&self) -> Journal {
        self.fs.seed_dir(&path(OTHER_STATE_DIR)).unwrap();
        self.builder_in(OTHER_STATE_DIR).build().unwrap()
    }

    fn journal(&self) -> Journal {
        self.builder().build().unwrap()
    }

    /// Journal whose process stops at the first pass of `point`.
    fn crashing_at(&self, point: FailPoint) -> Journal {
        self.crashing_at_target(point, None)
    }

    /// Journal that crashes when `point` is reached for the target `n`.
    fn crashing_at_target(&self, point: FailPoint, n: Option<u32>) -> Journal {
        let rule = FailRule {
            point,
            n,
            occurrence: 0,
            action: FailAction::Crash,
        };
        let mut builder = self.builder();
        builder.ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(0, rule)));
        builder.build().unwrap()
    }

    fn seed(&self, target: &str, content: &[u8]) -> AbsPath {
        let target = path(target);
        self.fs.seed_file(&target, content).unwrap();
        target
    }

    fn read(&self, target: &AbsPath) -> Vec<u8> {
        manifest::read_bytes(self.fs.as_ref(), target).unwrap()
    }

    fn exists(&self, target: &AbsPath) -> bool {
        self.fs
            .metadata(target, nurt_contracts::fs::Follow::NoFollow)
            .is_ok()
    }

    fn state(&self, op: u64) -> EntryState {
        let entry = journal_layout().entry(OpId(op)).unwrap();
        manifest::read_state(self.fs.as_ref(), &entry.state().unwrap())
            .unwrap()
            .state
    }

    fn set_state(&self, op: u64, state: EntryState) {
        let entry = journal_layout().entry(OpId(op)).unwrap();
        manifest::write_state(
            self.fs.as_ref(),
            &scope(OpId(op), ScopePurpose::Maintenance),
            entry.dir(),
            &entry.state().unwrap(),
            StateRecord::new(state),
        )
        .unwrap();
    }

    fn home(&self) -> nurt_contracts::fs::VolumeId {
        self.fs.volume(&path(HOME)).unwrap().id
    }

    /// Journal that writes the user configuration file [`USER_CONFIG`].
    fn journal_with_user_config(&self) -> Journal {
        self.builder()
            .user_config(path(USER_CONFIG))
            .build()
            .unwrap()
    }

    /// Row of the operation index for entry `op`.
    fn index_row(&self, op: OpId) -> IndexRow {
        let index = StateLayout::new(path(STATE_DIR)).index().unwrap();
        manifest::read_index(self.fs.as_ref(), &index)
            .unwrap()
            .into_iter()
            .find(|row| row.op == op)
            .unwrap()
    }

    /// Makes the operation of `journal` crash after the switch of `target`,
    /// leaving an interrupted entry.
    fn interrupt(&self, target: &AbsPath) {
        let crashed = self.crashing_at(FailPoint::ApplySwitched);
        let mut tx = crashed.begin(request("write"));
        assert!(
            apply_ops(
                tx.as_mut(),
                vec![FileOp::write(target.clone(), b"lost".to_vec())]
            )
            .is_err()
        );
    }
}

fn write(journal: &Journal, target: &AbsPath, content: &[u8]) -> OpHandle {
    let mut tx = journal.begin(request("write"));
    apply_ops(
        tx.as_mut(),
        vec![FileOp::write(target.clone(), content.to_vec())],
    )
    .unwrap();
    tx.commit().unwrap()
}

#[test]
fn build_fails_without_the_state_directory() {
    let env = Env::new(&json!({}));
    let limits =
        ConfiguredLimits::new(&env.layers, false, env.fs.clone(), &path(STATE_DIR)).unwrap();
    let error = JournalBuilder::new(
        env.fs.clone(),
        env.admission.clone(),
        env.clock.clone(),
        Arc::new(HomeJournal),
        Arc::new(limits),
        path("/home/user/missing"),
    )
    .build()
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn an_incremental_transaction_preserves_after_its_first_change_and_is_one_entry() {
    let env = Env::new(&json!({}));
    let first = env.seed(NOTES, b"one");
    let second = env.seed(OTHER_FILE, b"two");
    let journal = env.journal();

    let mut tx = journal.begin(request("steps"));
    tx.allow_incremental();
    apply_ops(
        tx.as_mut(),
        vec![FileOp::write(first.clone(), b"ONE".to_vec())],
    )
    .unwrap();
    apply_ops(
        tx.as_mut(),
        vec![FileOp::write(second.clone(), b"TWO".to_vec())],
    )
    .unwrap();
    let handle = tx.commit().unwrap();
    assert_eq!(handle.op, Some(OpId(1)));
    assert_eq!(env.read(&first), b"ONE");
    assert_eq!(env.read(&second), b"TWO");
    assert_eq!(handle.targets.len(), 2);
    assert_eq!(journal.history(&HistoryQuery::default()).unwrap().len(), 1);

    journal.undo(&undo_last()).unwrap();
    assert_eq!(env.read(&first), b"one");
    assert_eq!(env.read(&second), b"two");
}

#[test]
fn rolling_back_an_incremental_transaction_restores_every_target() {
    let env = Env::new(&json!({}));
    let first = env.seed(NOTES, b"one");
    let second = env.seed(OTHER_FILE, b"two");
    let journal = env.journal();

    let mut tx = journal.begin(request("steps"));
    tx.allow_incremental();
    apply_ops(
        tx.as_mut(),
        vec![FileOp::write(first.clone(), b"ONE".to_vec())],
    )
    .unwrap();
    apply_ops(
        tx.as_mut(),
        vec![FileOp::write(second.clone(), b"TWO".to_vec())],
    )
    .unwrap();
    let report = tx
        .rollback(nurt_contracts::journal::RollbackReason::TargetFailed)
        .unwrap();
    assert!(report.failed.is_empty());
    assert_eq!(env.read(&first), b"one");
    assert_eq!(env.read(&second), b"two");
    assert!(
        journal
            .history(&HistoryQuery::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_transaction_that_is_not_incremental_refuses_to_preserve_after_a_change() {
    let env = Env::new(&json!({}));
    let first = env.seed(NOTES, b"one");
    let second = env.seed(OTHER_FILE, b"two");
    let journal = env.journal();

    let mut tx = journal.begin(request("write"));
    apply_ops(
        tx.as_mut(),
        vec![FileOp::write(first.clone(), b"ONE".to_vec())],
    )
    .unwrap();
    let refused = apply_ops(
        tx.as_mut(),
        vec![FileOp::write(second.clone(), b"TWO".to_vec())],
    );
    assert!(refused.is_err());
    assert_eq!(env.read(&first), b"one");
    assert_eq!(env.read(&second), b"two");
}

#[test]
fn an_incremental_transaction_interrupted_at_each_point_is_rolled_back_by_recovery() {
    for (point, occurrence) in [
        (FailPoint::PreserveDurable, Some(1)),
        (FailPoint::ApplySwitched, Some(0)),
        (FailPoint::ApplySwitched, Some(1)),
        (FailPoint::CommitWritten, None),
    ] {
        let env = Env::new(&json!({}));
        let first = env.seed(NOTES, b"one");
        let second = env.seed(OTHER_FILE, b"two");
        let crashed = env.crashing_at_target(point, occurrence);
        let mut tx = crashed.begin(request("steps"));
        tx.allow_incremental();
        let outcome = apply_ops(
            tx.as_mut(),
            vec![FileOp::write(first.clone(), b"ONE".to_vec())],
        )
        .and_then(|_| {
            apply_ops(
                tx.as_mut(),
                vec![FileOp::write(second.clone(), b"TWO".to_vec())],
            )
        })
        .and_then(|_| tx.commit().map(drop));
        assert!(outcome.is_err(), "{point:?} {occurrence:?}");
        drop(crashed);

        let journal = env.journal();
        journal.recover().unwrap();
        assert_eq!(env.read(&first), b"one", "{point:?} {occurrence:?}");
        assert_eq!(env.read(&second), b"two", "{point:?} {occurrence:?}");
        assert!(
            layout::pending_entries(env.fs.as_ref(), &journal_layout())
                .unwrap()
                .is_empty(),
            "{point:?} {occurrence:?}"
        );
    }
}

/// Writes `content` to `target` in the open transaction.
fn put(tx: &mut dyn nurt_contracts::journal::MutationTx, target: &AbsPath, content: &[u8]) {
    apply_ops(tx, vec![FileOp::write(target.clone(), content.to_vec())]).unwrap();
}

#[test]
fn savepoints_nest_and_each_rollback_restores_only_its_own_changes() {
    let env = Env::new(&json!({}));
    let a = env.seed(NOTES, b"a0");
    let b = env.seed(OTHER_FILE, b"b0");
    let c = env.seed("/home/user/docs/third.txt", b"c0");
    let journal = env.journal();

    let mut tx = journal.begin(request("steps"));
    tx.allow_incremental();
    put(tx.as_mut(), &a, b"a1");
    tx.savepoint();
    put(tx.as_mut(), &b, b"b1");
    tx.savepoint();
    put(tx.as_mut(), &c, b"c1");
    tx.rollback_to_savepoint().unwrap();
    assert_eq!(env.read(&c), b"c0");
    assert_eq!(env.read(&b), b"b1");
    tx.rollback_to_savepoint().unwrap();
    assert_eq!(env.read(&b), b"b0");
    assert_eq!(env.read(&a), b"a1");
    let handle = tx.commit().unwrap();
    assert_eq!(handle.op, Some(OpId(1)));
    journal.undo(&undo_last()).unwrap();
    assert_eq!(env.read(&a), b"a0");
    assert_eq!(env.read(&b), b"b0");
    assert_eq!(env.read(&c), b"c0");
}

#[test]
fn releasing_an_inner_savepoint_keeps_the_outer_one() {
    let env = Env::new(&json!({}));
    let a = env.seed(NOTES, b"a0");
    let b = env.seed(OTHER_FILE, b"b0");
    let journal = env.journal();

    let mut tx = journal.begin(request("steps"));
    tx.allow_incremental();
    tx.savepoint();
    put(tx.as_mut(), &a, b"a1");
    tx.savepoint();
    put(tx.as_mut(), &b, b"b1");
    tx.release_savepoint();
    tx.rollback_to_savepoint().unwrap();
    assert_eq!(env.read(&a), b"a0");
    assert_eq!(env.read(&b), b"b0");
}

#[test]
fn a_rolled_back_rename_is_not_restored_twice() {
    let env = Env::new(&json!({}));
    let a = env.seed(NOTES, b"a0");
    let b = env.seed(OTHER_FILE, b"b0");
    let moved = path("/home/user/docs/moved.txt");
    let journal = env.journal();

    let mut tx = journal.begin(request("steps"));
    tx.allow_incremental();
    put(tx.as_mut(), &a, b"a1");
    tx.savepoint();
    apply_ops(
        tx.as_mut(),
        vec![FileOp::rename(b.clone(), moved.clone(), false)],
    )
    .unwrap();
    assert!(env.exists(&moved));
    tx.rollback_to_savepoint().unwrap();
    assert_eq!(env.read(&b), b"b0");
    assert!(!env.exists(&moved));
    // A later rollback of the whole transaction does not touch it again.
    let report = tx
        .rollback(nurt_contracts::journal::RollbackReason::TargetFailed)
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(env.read(&a), b"a0");
    assert_eq!(env.read(&b), b"b0");
}

#[test]
fn a_committed_entry_with_a_rolled_back_rename_undoes_cleanly() {
    let env = Env::new(&json!({}));
    let a = env.seed(NOTES, b"a0");
    let b = env.seed(OTHER_FILE, b"b0");
    let moved = path("/home/user/docs/moved.txt");
    let journal = env.journal();

    let mut tx = journal.begin(request("steps"));
    tx.allow_incremental();
    put(tx.as_mut(), &a, b"a1");
    tx.savepoint();
    apply_ops(
        tx.as_mut(),
        vec![FileOp::rename(b.clone(), moved.clone(), false)],
    )
    .unwrap();
    tx.rollback_to_savepoint().unwrap();
    tx.commit().unwrap();
    journal.undo(&undo_last()).unwrap();
    assert_eq!(env.read(&a), b"a0");
    assert_eq!(env.read(&b), b"b0");
    assert!(!env.exists(&moved));
}

#[test]
fn write_is_listed_undone_and_redone() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let journal = env.journal();

    let handle = write(&journal, &notes, b"final");
    assert_eq!(handle.op, Some(OpId(1)));
    assert_eq!(env.read(&notes), b"final");
    let history = journal.history(&HistoryQuery::default()).unwrap();
    assert_eq!(
        history.iter().map(|record| record.id).collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(journal.details(OpId(1)).unwrap().items.len(), 1);

    let undone = journal.undo(&undo_last()).unwrap();
    assert_eq!(undone.sources, [OpId(1)]);
    assert_eq!(env.read(&notes), b"draft");
    let source = HistoryQuery {
        id: Some(OpId(1)),
        ..HistoryQuery::default()
    };
    assert_eq!(journal.history(&source).unwrap()[0].state, OpState::Undone);

    let redone = journal
        .redo(&RedoRequest {
            undo_entry: Some(undone.entry),
            session: None,
            program: program(),
        })
        .unwrap();
    assert!(redone.entry > undone.entry);
    assert_eq!(env.read(&notes), b"final");
    assert_eq!(journal.history(&HistoryQuery::default()).unwrap().len(), 3);
}

#[test]
fn recover_rolls_back_an_operation_interrupted_by_a_crash() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let crashed = env.crashing_at(FailPoint::ApplySwitched);
    let mut tx = crashed.begin(request("write"));
    let applied = apply_ops(
        tx.as_mut(),
        vec![FileOp::write(notes.clone(), b"final".to_vec())],
    );
    assert!(applied.is_err());
    drop(tx);
    drop(crashed);

    let journal = env.journal();
    let report = journal.recover().unwrap();
    assert_eq!(report.rolled_back, [OpId(1)]);
    assert_eq!(env.read(&notes), b"draft");
    assert!(
        layout::pending_entries(env.fs.as_ref(), &journal_layout())
            .unwrap()
            .is_empty()
    );
    assert!(write(&journal, &notes, b"again").op.is_some());
    assert_eq!(env.read(&notes), b"again");
}

#[test]
fn first_transaction_recovers_interrupted_entries() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let crashed = env.crashing_at(FailPoint::ApplySwitched);
    let mut tx = crashed.begin(request("write"));
    assert!(
        apply_ops(
            tx.as_mut(),
            vec![FileOp::write(notes.clone(), b"lost".to_vec())]
        )
        .is_err()
    );
    drop(tx);
    drop(crashed);

    let journal = env.journal();
    let other = env.seed("/home/user/docs/other.txt", b"other");
    assert!(write(&journal, &other, b"changed").op.is_some());
    assert_eq!(env.read(&notes), b"draft");
    assert_eq!(env.state(1), EntryState::Undone);
}

#[test]
fn recover_completes_the_source_update_of_an_interrupted_undo() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let journal = env.journal();
    write(&journal, &notes, b"final");
    let undone = journal.undo(&undo_last()).unwrap();
    assert_eq!(env.state(1), EntryState::Undone);
    drop(journal);

    env.set_state(1, EntryState::Committed);
    let marker = journal_layout().pending(undone.entry).unwrap();
    env.fs.seed_file(&marker, b"").unwrap();

    let journal = env.journal();
    journal.recover().unwrap();
    assert_eq!(env.state(1), EntryState::Undone);
    assert!(!env.exists(&marker));
    assert_eq!(env.read(&notes), b"draft");
}

/// Journal with the operations 1, 2 and 3, each writing its own file.
fn three_operations(env: &Env) -> Journal {
    let journal = env.journal();
    for (n, name) in ["a", "b", "c"].into_iter().enumerate() {
        let target = env.seed(&format!("/home/user/docs/{name}.txt"), name.as_bytes());
        assert_eq!(
            write(&journal, &target, b"new").op,
            Some(OpId(u64::try_from(n).unwrap() + 1))
        );
    }
    journal
}

#[test]
fn status_and_limits_follow_the_retention_limits() {
    let env = Env::new(&json!({}));
    let journal = three_operations(&env);
    let state = journal.status(None).unwrap().remove(0);
    assert_eq!(state.volume, path(HOME));
    assert_eq!((state.entries_total, state.entries_available), (3, 3));
    let unused = journal.status(Some(&path("/srv"))).unwrap();
    assert_eq!(unused.len(), 1);
    assert_eq!(unused[0].volume, path("/"));
    assert_eq!(
        (unused[0].entries_total, unused[0].entries_available),
        (0, 0)
    );

    let preview = journal.limits(&steps(2, true)).unwrap();
    assert!(!preview.applied);
    assert_eq!(ids(&preview.expiring), [1]);
    assert_eq!(journal.status(None).unwrap()[0].entries_available, 3);
    let lowered = journal.limits(&steps(2, false)).unwrap();
    assert!(lowered.applied);
    assert_eq!(ids(&lowered.expiring), [1]);
    assert_eq!(journal.status(None).unwrap()[0].entries_available, 2);
}

#[test]
fn purge_releases_selected_entries_and_sweep_finds_nothing_more() {
    let env = Env::new(&json!({}));
    let journal = three_operations(&env);
    let purged = journal
        .purge(&PurgeRequest {
            volume: None,
            selector: PurgeSelector::Ids(vec![OpId(2)]),
        })
        .unwrap();
    assert_eq!(ids(&purged.released), [2]);
    assert!(journal.sweep().unwrap().expired.is_empty());
    let state = journal.status(None).unwrap().remove(0);
    assert_eq!(state.entries_available, 2);
    assert_eq!(state.oldest.map(|oldest| oldest.op), Some(OpId(1)));
}

#[test]
fn pin_reports_changed_and_unavailable_entries() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let journal = env.journal();
    write(&journal, &notes, b"final");
    let pinned = journal
        .pin(&PinRequest {
            ids: vec![OpId(1), OpId(9)],
            release: false,
            until: None,
        })
        .unwrap();
    assert_eq!(
        (ids(&pinned.changed), ids(&pinned.not_available)),
        (vec![1], vec![9])
    );
    let record = journal
        .history(&HistoryQuery {
            id: Some(OpId(1)),
            ..HistoryQuery::default()
        })
        .unwrap()
        .remove(0);
    assert!(record.pinned);
    let released = journal
        .pin(&PinRequest {
            ids: vec![OpId(1)],
            release: true,
            until: None,
        })
        .unwrap();
    assert_eq!(ids(&released.changed), [1]);
}

#[test]
fn space_requires_a_space_reporter() {
    let env = Env::new(&json!({}));
    let error = env.journal().space(None).unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceNotRunning);
}

#[test]
fn catalog_lists_entries_with_held_bytes_and_overdue_limits() {
    let env = Env::new(&json!({ "max_ops": 1 }));
    let journal = env.journal();
    let a = env.seed("/home/user/docs/a.txt", b"aaaa");
    let b = env.seed("/home/user/docs/b.txt", b"bbbbbb");
    write(&journal, &a, b"A");
    write(&journal, &b, b"B");

    let mut entries = journal.entries(&env.home()).unwrap();
    entries.sort_by_key(|entry| entry.op);
    assert_eq!(
        entries.iter().map(|entry| entry.op.0).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry.state == EntryState::Committed)
    );
    assert!(entries.iter().all(|entry| entry.bytes > 0 && !entry.pinned));
    assert_eq!(entries[0].overdue, Some(ExpiryReason::Ops));
    assert_eq!(entries[1].overdue, None);

    let items = journal.held_items(&env.home(), OpId(1)).unwrap();
    assert_eq!(items.len(), 1);
    assert!(env.exists(&items[0].data));
    let error = journal.held_items(&env.home(), OpId(7)).unwrap_err();
    assert_eq!(error.code, ErrorCode::JournalNoop);
    let error = journal
        .entries(&env.fs.volume(&path("/")).unwrap().id)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn catalog_releases_items_under_the_entry_lock() {
    let env = Env::new(&json!({}));
    let journal = env.journal();
    let a = env.seed("/home/user/docs/a.txt", b"aaaa");
    write(&journal, &a, b"A");
    let home = env.home();

    let lock = journal.lock_entry(&home, OpId(1)).unwrap();
    let error = journal.lock_entry(&home, OpId(1)).err().unwrap();
    assert_eq!(error.code, ErrorCode::InUse);
    let item = journal.held_items(&home, OpId(1)).unwrap().remove(0);
    journal
        .mark_released(&home, OpId(1), &[item.index])
        .unwrap();
    let marker = journal_layout().pending(OpId(1)).unwrap();
    assert!(env.exists(&marker));
    assert_eq!(env.state(1), EntryState::Partial);
    env.fs
        .remove_file(&scope(OpId(1), ScopePurpose::Reclaim), &item.data)
        .unwrap();
    journal.finish_release(&home, OpId(1)).unwrap();
    assert!(!env.exists(&marker));
    drop(lock);

    let entry = journal.entries(&home).unwrap().remove(0);
    assert_eq!((entry.state, entry.bytes), (EntryState::Partial, 0));
    assert!(journal.held_items(&home, OpId(1)).unwrap().is_empty());
    journal.recover().unwrap();
    assert_eq!(env.state(1), EntryState::Partial);
}

#[test]
fn catalog_reclaims_a_whole_entry_and_leaves_a_tombstone() {
    let env = Env::new(&json!({}));
    let journal = env.journal();
    let a = env.seed("/home/user/docs/a.txt", b"aaaa");
    write(&journal, &a, b"A");
    let home = env.home();

    let lock = journal.lock_entry(&home, OpId(1)).unwrap();
    let paths = journal
        .begin_reclaim(&home, OpId(1), ExpiryReason::Space)
        .unwrap();
    assert_eq!(paths.entry, *journal_layout().entry(OpId(1)).unwrap().dir());
    assert!(!env.exists(&paths.reap));
    let reclaim = scope(OpId(1), ScopePurpose::Reclaim);
    env.fs
        .rename(&reclaim, &paths.entry, &paths.reap, RenameMode::NoReplace)
        .unwrap();
    let reaped = journal.entries(&home).unwrap().remove(0);
    assert_eq!(reaped.state, EntryState::Expiring);
    env.fs.remove_tree(&reclaim, &paths.reap, 1).unwrap();
    let expired = journal.complete_reclaim(&home, OpId(1)).unwrap();
    drop(lock);

    assert_eq!(
        expired,
        ExpiredEntry {
            op: OpId(1),
            reason: ExpiryReason::Space
        }
    );
    assert!(env.exists(&journal_layout().tomb(OpId(1)).unwrap()));
    assert!(journal.entries(&home).unwrap().is_empty());
    assert_eq!(journal.complete_reclaim(&home, OpId(1)).unwrap(), expired);
    let error = journal
        .begin_reclaim(&home, OpId(1), ExpiryReason::Space)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::JournalNoop);
    let record = journal.history(&HistoryQuery::default()).unwrap().remove(0);
    assert_eq!(record.state, OpState::Expired);
}

#[test]
fn recover_expires_overdue_entries_after_the_recovery() {
    let env = Env::new(&json!({}));
    three_operations(&env);
    env.clock.advance(PAST_AGE_LIMIT);
    let notes = env.seed(NOTES, b"draft");
    env.interrupt(&notes);

    let journal = env.journal();
    let report = journal.recover().unwrap();
    assert_eq!(report.rolled_back, [OpId(4)]);
    assert_eq!(ids(&report.expired), [1, 2, 3]);
    assert_eq!(env.read(&notes), b"draft");
    for op in 1..=3 {
        assert!(env.exists(&journal_layout().tomb(OpId(op)).unwrap()));
    }
    let again = journal.recover().unwrap();
    assert!(again.rolled_back.is_empty() && again.expired.is_empty());
}

#[test]
fn first_transaction_recovers_without_expiring_overdue_entries() {
    let env = Env::new(&json!({}));
    let a = env.seed("/home/user/docs/a.txt", b"a");
    write(&env.journal(), &a, b"A");
    env.clock.advance(PAST_AGE_LIMIT);
    let notes = env.seed(NOTES, b"draft");
    env.interrupt(&notes);

    let journal = env.journal();
    let b = env.seed("/home/user/docs/b.txt", b"b");
    assert_eq!(write(&journal, &b, b"B").op, Some(OpId(3)));
    assert_eq!(env.read(&notes), b"draft");
    assert_eq!(env.state(1), EntryState::Committed);
    assert_eq!(ids(&journal.recover().unwrap().expired), [1]);
}

#[test]
fn recover_reports_the_recovery_when_the_expiry_pass_fails() {
    let env = Env::new(&json!({}));
    let journal = env.journal();
    let a = env.seed("/home/user/docs/a.txt", b"a");
    write(&journal, &a, b"A");
    env.clock.advance(PAST_AGE_LIMIT);

    let lock = journal.lock_entry(&env.home(), OpId(1)).unwrap();
    let error = journal.recover().unwrap_err();
    assert!(error.details.contains_key("recovered"), "{error:?}");
    assert_eq!(env.state(1), EntryState::Committed);
    drop(lock);
    assert_eq!(ids(&journal.recover().unwrap().expired), [1]);
    let b = env.seed("/home/user/docs/b.txt", b"b");
    assert_eq!(write(&journal, &b, b"B").op, Some(OpId(2)));
}

#[test]
fn user_scope_writes_the_configuration_file_in_an_undoable_entry() {
    let env = Env::new(&json!({}));
    let original = "# account settings\n[journal]\nmax_age = \"1h\"\n";
    let config = env.seed(USER_CONFIG, original.as_bytes());
    let journal = env.journal_with_user_config();
    for name in ["a", "b", "c"] {
        let target = env.seed(&format!("/home/user/docs/{name}.txt"), name.as_bytes());
        write(&journal, &target, b"new");
    }

    let outcome = journal.limits(&user_steps(2)).unwrap();
    assert!(outcome.applied);
    assert_eq!(ids(&outcome.expiring), [1]);
    let limits = &outcome.effective[0].limits;
    assert_eq!(limits.max_ops.value, StepLimit::Limited(2));
    assert_eq!(limits.max_ops.source, ConfigLevel::User);
    let written = String::from_utf8(env.read(&config)).unwrap();
    assert!(
        written.starts_with("# account settings\n[journal]\n"),
        "{written}"
    );
    assert!(written.contains("max_age = \"1h\""), "{written}");
    assert!(written.contains("max_ops = 2"), "{written}");

    let row = env.index_row(OpId(4));
    assert_eq!(row.kind, OpKind::command("journal.limits"));
    assert!(!row.counts_as_step);
    let undone = journal
        .undo(&UndoRequest {
            selector: UndoSelector::Op(OpId(4)),
            ..undo_last()
        })
        .unwrap();
    assert_eq!(undone.sources, [OpId(4)]);
    assert_eq!(env.read(&config), original.as_bytes());
}

#[test]
fn user_scope_creates_the_missing_configuration_directories() {
    let env = Env::new(&json!({}));
    let journal = env.journal_with_user_config();
    let outcome = journal.limits(&user_steps(5)).unwrap();
    assert!(outcome.applied && outcome.expiring.is_empty());
    let written = String::from_utf8(env.read(&path(USER_CONFIG))).unwrap();
    assert!(written.contains("[journal]"), "{written}");
    assert!(written.contains("max_ops = 5"), "{written}");
    assert!(!env.index_row(OpId(1)).counts_as_step);

    journal.undo(&undo_last()).unwrap();
    assert!(!env.exists(&path(USER_CONFIG)));
    assert!(!env.exists(&path("/home/user/.config")));
}

#[test]
fn user_scope_without_a_configuration_file_is_unsupported() {
    let env = Env::new(&json!({}));
    let journal = three_operations(&env);
    for request in [
        user_steps(1),
        LimitsRequest {
            preview: true,
            ..user_steps(1)
        },
    ] {
        let error = journal.limits(&request).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(
            error.message,
            "the journal has no user configuration file; configure its path or use scope: session"
        );
    }
    assert_eq!(journal.status(None).unwrap()[0].entries_available, 3);
    let report = journal
        .limits(&LimitsRequest {
            steps: None,
            ..user_steps(1)
        })
        .unwrap();
    assert!(!report.applied);
    assert_eq!(
        report.effective[0].limits.max_ops.value,
        StepLimit::Limited(30)
    );
}

#[test]
fn undo_of_another_session_finds_nothing_to_undo() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let journal = env.journal();
    write(&journal, &notes, b"final");
    let error = journal
        .undo(&UndoRequest {
            session: Some(SessionId("other".to_owned())),
            ..undo_last()
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::JournalNoop);
    assert_eq!(env.read(&notes), b"final");
    assert_eq!(env.state(1), EntryState::Committed);
}

#[test]
fn undo_is_redone_by_another_process_of_the_account() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let first = env.journal();
    write(&first, &notes, b"final");
    let undone = first.undo(&undo_last()).unwrap();
    assert_eq!(env.read(&notes), b"draft");
    drop(first);

    let second = env.journal();
    let redone = second
        .redo(&RedoRequest {
            undo_entry: None,
            session: None,
            program: program(),
        })
        .unwrap();
    assert_eq!(redone.sources, [undone.entry]);
    assert_eq!(env.read(&notes), b"final");
    assert_eq!(env.state(undone.entry.0), EntryState::Undone);
}

#[test]
fn undo_interrupted_at_each_point_is_rolled_back_by_recovery_and_can_be_repeated() {
    for point in [
        FailPoint::PreserveDurable,
        FailPoint::ApplySwitched,
        FailPoint::ApplyDone,
    ] {
        let env = Env::new(&json!({}));
        let notes = env.seed(NOTES, b"draft");
        let first = env.journal();
        write(&first, &notes, b"final");
        drop(first);

        let crashed = env.crashing_at(point);
        assert!(crashed.undo(&undo_last()).is_err(), "{point:?}");
        drop(crashed);

        let journal = env.journal();
        journal.recover().unwrap();
        assert_eq!(env.read(&notes), b"final", "{point:?}");
        assert_eq!(env.state(1), EntryState::Committed, "{point:?}");
        let interrupted = journal_layout().entry(OpId(2)).unwrap();
        if env.exists(&interrupted.state().unwrap()) {
            assert_eq!(env.state(2), EntryState::Undone, "{point:?}");
        }
        let undone = journal.undo(&undo_last()).unwrap();
        assert_eq!(undone.sources, [OpId(1)], "{point:?}");
        assert_eq!(env.read(&notes), b"draft", "{point:?}");
    }
}

#[test]
fn undo_of_an_entry_with_a_damaged_manifest_reports_corruption_and_changes_nothing() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let journal = env.journal();
    write(&journal, &notes, b"final");
    let entry = journal_layout().entry(OpId(1)).unwrap();
    manifest::write_atomic(
        env.fs.as_ref(),
        &scope(OpId(1), ScopePurpose::Maintenance),
        entry.dir(),
        &entry.manifest().unwrap(),
        b"damaged",
    )
    .unwrap();
    let error = journal
        .undo(&UndoRequest {
            selector: UndoSelector::Op(OpId(1)),
            ..undo_last()
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::JournalCorrupt);
    assert_eq!(env.read(&notes), b"final");
}

#[test]
fn undo_without_a_number_leaves_the_entries_of_another_account_alone() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let other_file = env.seed(OTHER_FILE, b"theirs");
    let mine = env.journal();
    let theirs = env.other_account();
    write(&mine, &notes, b"final");
    let foreign = write(&theirs, &other_file, b"changed").op.unwrap();
    assert!(foreign > OpId(1));

    let undone = mine.undo(&undo_last()).unwrap();
    assert_eq!(undone.sources, [OpId(1)]);
    assert_eq!(env.read(&notes), b"draft");
    assert_eq!(env.read(&other_file), b"changed");
    let error = mine.undo(&undo_last()).unwrap_err();
    assert_eq!(error.code, ErrorCode::JournalNoop);
    assert_eq!(env.read(&other_file), b"changed");
    assert_eq!(env.state(foreign.0), EntryState::Committed);
}

#[test]
fn history_and_undo_by_number_see_the_entries_of_the_account_only() {
    let env = Env::new(&json!({}));
    let notes = env.seed(NOTES, b"draft");
    let other_file = env.seed(OTHER_FILE, b"theirs");
    let mine = env.journal();
    let theirs = env.other_account();
    write(&mine, &notes, b"final");
    let foreign = write(&theirs, &other_file, b"changed").op.unwrap();

    let listed = |journal: &Journal| {
        journal
            .history(&HistoryQuery::default())
            .unwrap()
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(&mine), [1]);
    assert_eq!(listed(&theirs), [foreign.0]);
    let by_number = UndoRequest {
        selector: UndoSelector::Op(foreign),
        ..undo_last()
    };
    assert!(mine.undo(&by_number).is_err());
    assert_eq!(env.read(&other_file), b"changed");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
