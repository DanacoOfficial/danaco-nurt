// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/journal_rows.rs
// Responsibility:   Coverage cases of the journal commands and of the parameters common to.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Coverage cases of the journal commands and of the parameters common to
//! every command, run against the journal implementation itself.

use nurt_contracts::config::{SizeOrPercent, SizeSetting, StepLimit};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{AbsPath, Follow};
use nurt_contracts::journal::{
    Expect, HistoryQuery, LimitsRequest, LimitsScope, Mutation, MutationTx, OnConflict, OpHandle,
    OpId, OpKind, OpState, PinRequest, PurgeRequest, PurgeSelector, RollbackReason, SessionId,
    TargetAction, TargetIndex, TargetPath, TargetSpec, UndoScope, UndoSelector,
};

use crate::cases::Case;
use crate::harness::fixture::{Fixture, FixtureOptions, VolumeSpec};
use crate::harness::request::{self, OTHER_SESSION, SESSION};

/// Content of seeded files before an operation.
const BEFORE: &[u8] = b"before\n";
/// Content written by an operation.
const AFTER: &[u8] = b"after\n";
/// Content written by a program outside the console.
const FOREIGN: &[u8] = b"changed by another program\n";

fn spec(path: &AbsPath, action: TargetAction, expect: Expect) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect,
        size_hint: None,
    }
}

/// Overwrites the given files in one operation of the given session.
fn overwrite_in(fixture: &Fixture, session: &str, paths: &[&AbsPath], content: &[u8]) -> OpHandle {
    let mut begin = request::begin_in("write", Some("overwrite"), session);
    begin.bulk = paths.len() > 1;
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(begin);
    let specs: Vec<TargetSpec> = paths
        .iter()
        .map(|path| spec(path, TargetAction::Modify, Expect::Unchecked))
        .collect();
    tx.preserve(&specs).expect("targets are preserved");
    for index in 0..specs.len() {
        let index = TargetIndex(u32::try_from(index).expect("target index fits"));
        tx.apply(index, Mutation::WriteBytes(content.to_vec()))
            .expect("change is applied");
    }
    tx.commit().expect("transaction commits")
}

fn overwrite(fixture: &Fixture, paths: &[&AbsPath]) -> OpId {
    op_of(&overwrite_in(fixture, SESSION, paths, AFTER))
}

fn op_of(handle: &OpHandle) -> OpId {
    handle.op.expect("a committed change has a number")
}

fn state(fixture: &Fixture, op: OpId) -> OpState {
    fixture.record(op).state
}

fn assert_error(
    result: Result<impl std::fmt::Debug, nurt_contracts::error::Error>,
    code: ErrorCode,
) {
    match result {
        Ok(value) => panic!("expected {}, got {value:?}", code.as_str()),
        Err(error) => assert_eq!(error.code(), code, "{error}"),
    }
}

fn limits_options(steps: u32) -> FixtureOptions {
    let mut options = FixtureOptions::default();
    options.journal.max_ops = StepLimit::Limited(steps);
    options
}

/// A failed check after commit undoes the entry as an entry of kind `undo`.
pub(crate) fn verify_failure_undoes_entry_as_undo_entry(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let before = fixture.snapshot();
    let op = overwrite(&fixture, &[&file]);
    let after = fixture.snapshot();
    let outcome = fixture
        .journal
        .rollback_committed(op, RollbackReason::VerifyFailed)
        .expect("entry is rolled back");
    assert_eq!(outcome.sources, vec![op]);
    before.assert_same(&fixture.snapshot(), "after the failed check");
    assert_eq!(state(&fixture, op), OpState::Undone);
    let undo_entry = fixture.record(outcome.entry);
    assert_eq!(undo_entry.kind, OpKind::undo().command);
    assert_eq!(undo_entry.undo_of, vec![op.0]);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("rollback is redone");
    after.assert_same(&fixture.snapshot(), "after redo of the rollback");
}

fn reads_change_nothing(fixture: &Fixture, read: impl FnOnce(&Fixture)) {
    let file = fixture.seed_file("work/a.txt", BEFORE);
    overwrite(fixture, &[&file]);
    let before = fixture.snapshot();
    let entries = fixture.history();
    read(fixture);
    assert_eq!(fixture.history(), entries, "a read records no entry");
    before.assert_same(&fixture.snapshot(), "after a read");
}

/// `history` and `history details` change nothing.
pub(crate) fn history_changes_nothing(_case: &Case) {
    reads_change_nothing(&Fixture::standard(), |fixture| {
        let newest = OpId(fixture.history()[0].id);
        fixture
            .journal
            .details(newest)
            .expect("details are readable");
    });
}

/// `journal.space` changes nothing.
pub(crate) fn journal_space_changes_nothing(_case: &Case) {
    reads_change_nothing(&Fixture::standard(), |fixture| {
        fixture.journal.space(None).expect("space is reported");
    });
}

/// `journal.status` changes nothing.
pub(crate) fn journal_status_changes_nothing(_case: &Case) {
    reads_change_nothing(&Fixture::standard(), |fixture| {
        fixture.journal.status(None).expect("status is reported");
    });
}

fn limits_request(steps: Option<u32>, scope: LimitsScope, preview: bool) -> LimitsRequest {
    LimitsRequest {
        steps: steps.map(StepLimit::Limited),
        age: None,
        size: None,
        scope,
        volume: None,
        preview,
    }
}

fn session_scope() -> LimitsScope {
    LimitsScope::Session(SessionId(SESSION.to_owned()))
}

/// `journal.limits` without values and with `preview` changes nothing.
pub(crate) fn journal_limits_changes_nothing(_case: &Case) {
    reads_change_nothing(&Fixture::standard(), |fixture| {
        let shown = fixture
            .journal
            .limits(&limits_request(None, session_scope(), false))
            .expect("limits are reported");
        assert!(!shown.applied);
        let previewed = fixture
            .journal
            .limits(&limits_request(Some(1), session_scope(), true))
            .expect("limits are previewed");
        assert!(!previewed.applied);
    });
}

/// Undo of the last entry restores the tree; redo applies it again.
pub(crate) fn undo_last_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let first = fixture.seed_file("work/a.txt", BEFORE);
    let second = fixture.seed_file("work/b.txt", BEFORE);
    overwrite(&fixture, &[&first]);
    let before = fixture.snapshot();
    let op = overwrite(&fixture, &[&second]);
    let after = fixture.snapshot();
    let outcome = fixture
        .journal
        .undo(&request::undo_last())
        .expect("last entry is undone");
    assert_eq!(outcome.sources, vec![op]);
    assert_eq!(outcome.scope, UndoScope::Op);
    before.assert_same(&fixture.snapshot(), "after undo");
    fixture
        .journal
        .redo(&request::redo(None))
        .expect("undo is redone");
    after.assert_same(&fixture.snapshot(), "after redo");
}

/// `undo N` restores one entry and leaves later entries in place.
pub(crate) fn undo_id_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let first = fixture.seed_file("work/a.txt", BEFORE);
    let second = fixture.seed_file("work/b.txt", BEFORE);
    let op = overwrite(&fixture, &[&first]);
    overwrite(&fixture, &[&second]);
    let after = fixture.snapshot();
    let outcome = fixture
        .journal
        .undo(&request::undo_op(op))
        .expect("entry is undone");
    assert_eq!(outcome.sources, vec![op]);
    assert_eq!(fixture.read(&first), BEFORE);
    assert_eq!(fixture.read(&second), AFTER);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("undo is redone");
    after.assert_same(&fixture.snapshot(), "after redo");
}

/// `undo to:` restores every newer entry as one undo entry.
pub(crate) fn undo_to_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let files: Vec<AbsPath> = ["a", "b", "c"]
        .iter()
        .map(|name| fixture.seed_file(&format!("work/{name}.txt"), BEFORE))
        .collect();
    let ops: Vec<OpId> = files
        .iter()
        .map(|file| overwrite(&fixture, &[file]))
        .collect();
    let after = fixture.snapshot();
    let selector = UndoSelector::To {
        after: ops[0],
        all_sessions: false,
    };
    let outcome = fixture
        .journal
        .undo(&request::undo(selector, OnConflict::Report))
        .expect("range is undone");
    assert_eq!(outcome.sources, vec![ops[2], ops[1]], "newest first");
    assert_eq!(
        fixture.record(outcome.entry).undo_scope,
        Some(UndoScope::To)
    );
    assert_eq!(fixture.read(&files[0]), AFTER);
    assert_eq!(fixture.read(&files[1]), BEFORE);
    assert_eq!(fixture.read(&files[2]), BEFORE);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("range undo is redone");
    after.assert_same(&fixture.snapshot(), "after redo");
}

/// `undo to:` skips entries of other sessions unless `all_sessions` holds.
pub(crate) fn undo_to_sessions_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let base = fixture.seed_file("work/base.txt", BEFORE);
    let own = fixture.seed_file("work/own.txt", BEFORE);
    let other = fixture.seed_file("work/other.txt", BEFORE);
    let first = overwrite(&fixture, &[&base]);
    let foreign = op_of(&overwrite_in(&fixture, OTHER_SESSION, &[&other], AFTER));
    let mine = overwrite(&fixture, &[&own]);
    let selector = UndoSelector::To {
        after: first,
        all_sessions: false,
    };
    let outcome = fixture
        .journal
        .undo(&request::undo(selector, OnConflict::Report))
        .expect("own entries are undone");
    assert_eq!(outcome.sources, vec![mine]);
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].op, foreign);
    assert_eq!(fixture.read(&other), AFTER);
    let selector = UndoSelector::To {
        after: first,
        all_sessions: true,
    };
    let outcome = fixture
        .journal
        .undo(&request::undo(selector, OnConflict::Report))
        .expect("entries of all sessions are undone");
    assert_eq!(outcome.sources, vec![foreign]);
    assert_eq!(fixture.read(&other), BEFORE);
}

/// `undo file:` restores one file across entries; the entries become
/// partial.
pub(crate) fn undo_file_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let other = fixture.seed_file("work/b.txt", BEFORE);
    overwrite(&fixture, &[&file]);
    let op = op_of(&overwrite_in(
        &fixture,
        SESSION,
        &[&file, &other],
        b"second\n",
    ));
    let selector = UndoSelector::File {
        path: file.clone(),
        since: None,
    };
    let outcome = fixture
        .journal
        .undo(&request::undo(selector, OnConflict::Report))
        .expect("file is undone");
    assert_eq!(outcome.scope, UndoScope::File);
    assert_eq!(fixture.read(&file), BEFORE);
    assert_eq!(fixture.read(&other), b"second\n");
    assert_eq!(state(&fixture, op), OpState::Partial);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("file undo is redone");
    assert_eq!(fixture.read(&file), b"second\n");
}

/// `undo N paths:` restores selected items; the entry becomes partial.
pub(crate) fn undo_paths_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let first = fixture.seed_file("work/a.txt", BEFORE);
    let second = fixture.seed_file("work/b.txt", BEFORE);
    let op = overwrite(&fixture, &[&first, &second]);
    let selector = UndoSelector::Paths {
        op,
        paths: vec![TargetPath::Fs(first.clone())],
    };
    let outcome = fixture
        .journal
        .undo(&request::undo(selector, OnConflict::Report))
        .expect("item is undone");
    assert_eq!(outcome.scope, UndoScope::Paths);
    assert_eq!(fixture.read(&first), BEFORE);
    assert_eq!(fixture.read(&second), AFTER);
    assert_eq!(state(&fixture, op), OpState::Partial);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("item undo is redone");
    assert_eq!(fixture.read(&first), AFTER);
}

/// A conflict changes nothing; `force` undoes and keeps the diverging
/// content in the undo entry.
pub(crate) fn undo_force_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let op = overwrite(&fixture, &[&file]);
    fixture.write_outside(&file, FOREIGN);
    let diverged = fixture.snapshot();
    assert_error(
        fixture.journal.undo(&request::undo_op(op)),
        ErrorCode::JournalConflict,
    );
    diverged.assert_same(&fixture.snapshot(), "after a reported conflict");
    let outcome = fixture
        .journal
        .undo(&request::undo(UndoSelector::Op(op), OnConflict::Force))
        .expect("entry is undone by force");
    assert!(
        !outcome.forced.is_empty(),
        "overridden conflicts are reported"
    );
    assert_eq!(fixture.read(&file), BEFORE);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("forced undo is redone");
    diverged.assert_same(&fixture.snapshot(), "after redo of a forced undo");
}

/// `include_later` also undoes later entries touching the same items.
pub(crate) fn undo_include_later_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let first = overwrite(&fixture, &[&file]);
    let later = op_of(&overwrite_in(&fixture, SESSION, &[&file], b"later\n"));
    let after = fixture.snapshot();
    assert_error(
        fixture.journal.undo(&request::undo_op(first)),
        ErrorCode::JournalConflict,
    );
    let outcome = fixture
        .journal
        .undo(&request::undo(
            UndoSelector::Op(first),
            OnConflict::IncludeLater,
        ))
        .expect("entry and later entry are undone");
    assert_eq!(outcome.sources, vec![later, first]);
    assert_eq!(fixture.read(&file), BEFORE);
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("undo is redone");
    after.assert_same(&fixture.snapshot(), "after redo");
}

/// Items released for space are unavailable; the remaining items of the
/// partial entry are undone with `on_conflict: partial`.
pub(crate) fn undo_partial_is_undone_and_redone(_case: &Case) {
    const FILE_SIZE: u64 = 16 << 20;
    const FILLER_SIZE: usize = 33 << 20;
    let mut options = FixtureOptions {
        volumes: vec![VolumeSpec {
            capacity: 64 << 20,
            used: 8 << 20,
        }],
        ..FixtureOptions::default()
    };
    options.space.reserve = SizeOrPercent::Size(SizeSetting::from_mib(8));
    options.space.soft = Some(SizeOrPercent::Size(SizeSetting::from_mib(12)));
    let fixture = Fixture::open(options);
    let small = fixture.seed_file("work/small.txt", BEFORE);
    let large = fixture.seed_file("work/large.bin", &vec![7; FILE_SIZE as usize]);
    let op = overwrite(&fixture, &[&small, &large]);
    fixture.seed_file("work/filler.bin", &vec![9; FILLER_SIZE]);
    let other = fixture.seed_file("work/other.txt", BEFORE);
    overwrite(&fixture, &[&other]);
    assert_eq!(
        state(&fixture, op),
        OpState::Partial,
        "space pressure released part of the entry"
    );
    assert_error(
        fixture.journal.undo(&request::undo_op(op)),
        ErrorCode::JournalPartial,
    );
    let outcome = fixture
        .journal
        .undo(&request::undo(UndoSelector::Op(op), OnConflict::Partial))
        .expect("remaining items are undone");
    assert!(
        !outcome.unavailable.is_empty(),
        "released items are reported"
    );
    assert_eq!(fixture.read(&small), BEFORE);
}

/// `redo` reapplies an undone entry; undo of the redo restores the tree.
pub(crate) fn redo_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let before = fixture.snapshot();
    overwrite(&fixture, &[&file]);
    let after = fixture.snapshot();
    let undone = fixture
        .journal
        .undo(&request::undo_last())
        .expect("entry is undone");
    let redone = fixture
        .journal
        .redo(&request::redo(None))
        .expect("undo is redone");
    assert_eq!(redone.sources, vec![undone.entry]);
    assert_eq!(fixture.record(redone.entry).redo_of, Some(undone.entry.0));
    after.assert_same(&fixture.snapshot(), "after redo");
    fixture
        .journal
        .undo(&request::undo_op(redone.entry))
        .expect("redo is undone");
    before.assert_same(&fixture.snapshot(), "after undo of redo");
}

/// `redo` of an undo of selected items reapplies those items only.
pub(crate) fn redo_paths_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let first = fixture.seed_file("work/a.txt", BEFORE);
    let second = fixture.seed_file("work/b.txt", BEFORE);
    let op = overwrite(&fixture, &[&first, &second]);
    let after = fixture.snapshot();
    let selector = UndoSelector::Paths {
        op,
        paths: vec![TargetPath::Fs(first.clone())],
    };
    let undone = fixture
        .journal
        .undo(&request::undo(selector, OnConflict::Report))
        .expect("item is undone");
    fixture
        .journal
        .redo(&request::redo(Some(undone.entry)))
        .expect("item undo is redone");
    after.assert_same(&fixture.snapshot(), "after redo of items");
}

fn purge(fixture: &Fixture, selector: PurgeSelector) -> Vec<OpId> {
    let before = fixture.snapshot();
    let outcome = fixture
        .journal
        .purge(&PurgeRequest {
            volume: None,
            selector,
        })
        .expect("entries are purged");
    before.assert_same(&fixture.snapshot(), "purge changes no user data");
    outcome.released
}

fn three_entries(fixture: &Fixture) -> Vec<OpId> {
    ["a", "b", "c"]
        .iter()
        .map(|name| {
            let file = fixture.seed_file(&format!("work/{name}.txt"), BEFORE);
            overwrite(fixture, &[&file])
        })
        .collect()
}

/// `journal.purge up_to:` releases entries up to a number; they cannot be
/// undone.
pub(crate) fn journal_purge_up_to_is_marked_irreversible(_case: &Case) {
    let fixture = Fixture::standard();
    let ops = three_entries(&fixture);
    let mut released = purge(&fixture, PurgeSelector::UpTo(ops[1]));
    released.sort();
    assert_eq!(released, ops[..2].to_vec());
    assert_eq!(state(&fixture, ops[0]), OpState::Expired);
    assert_eq!(state(&fixture, ops[2]), OpState::Available);
    assert_error(
        fixture.journal.undo(&request::undo_op(ops[0])),
        ErrorCode::JournalExpired,
    );
}

/// `journal.purge` of all entries releases every entry.
pub(crate) fn journal_purge_all_is_marked_irreversible(_case: &Case) {
    let fixture = Fixture::standard();
    let ops = three_entries(&fixture);
    let mut released = purge(&fixture, PurgeSelector::All);
    released.sort();
    assert_eq!(released, ops);
    assert_eq!(fixture.status().entries_available, 0);
}

/// `journal.purge id:` releases the named entries without older ones.
pub(crate) fn journal_purge_id_is_marked_irreversible(_case: &Case) {
    let fixture = Fixture::standard();
    let ops = three_entries(&fixture);
    assert_eq!(
        purge(&fixture, PurgeSelector::Ids(vec![ops[1]])),
        vec![ops[1]]
    );
    assert_eq!(state(&fixture, ops[0]), OpState::Available);
    assert_eq!(state(&fixture, ops[1]), OpState::Expired);
}

fn pin(fixture: &Fixture, ids: Vec<OpId>, release: bool) {
    let outcome = fixture
        .journal
        .pin(&PinRequest {
            ids: ids.clone(),
            release,
            until: None,
        })
        .expect("pin state changes");
    assert_eq!(outcome.changed, ids);
}

/// `journal.pin` sets the pin marker without a new entry; release clears it.
pub(crate) fn journal_pin_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let ops = three_entries(&fixture);
    let entries = fixture.history().len();
    pin(&fixture, vec![ops[0]], false);
    assert!(fixture.record(ops[0]).pinned);
    pin(&fixture, vec![ops[0]], true);
    assert!(!fixture.record(ops[0]).pinned);
    assert_eq!(fixture.history().len(), entries, "pinning records no entry");
}

/// Releasing a pin expires an entry over the step limit at once.
pub(crate) fn journal_pin_release_is_undone_or_marked_irreversible(_case: &Case) {
    let fixture = Fixture::open(limits_options(2));
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let first = overwrite(&fixture, &[&file]);
    pin(&fixture, vec![first], false);
    let others = fixture.seed_file("work/b.txt", BEFORE);
    overwrite(&fixture, &[&others]);
    overwrite(&fixture, &[&others]);
    assert_eq!(
        state(&fixture, first),
        OpState::Available,
        "a pinned entry outlives the step limit"
    );
    pin(&fixture, vec![first], true);
    assert_eq!(state(&fixture, first), OpState::Expired);
}

/// Lowering the session limit expires entries at once without a new entry;
/// raising it does not restore them.
pub(crate) fn journal_limits_session_is_undone_or_marked_irreversible(_case: &Case) {
    let fixture = Fixture::standard();
    let ops: Vec<OpId> = (0..5)
        .map(|index| {
            let file = fixture.seed_file(&format!("work/{index}.txt"), BEFORE);
            overwrite(&fixture, &[&file])
        })
        .collect();
    let entries = fixture.history().len();
    let lowered = fixture
        .journal
        .limits(&limits_request(Some(2), session_scope(), false))
        .expect("limit is lowered");
    assert!(lowered.applied);
    let mut expiring = lowered.expiring.clone();
    expiring.sort();
    assert_eq!(expiring, ops[..3].to_vec());
    assert_eq!(
        fixture.history().len(),
        entries,
        "a session limit records no entry"
    );
    fixture
        .journal
        .limits(&limits_request(Some(30), session_scope(), false))
        .expect("limit is raised");
    assert_eq!(
        state(&fixture, ops[0]),
        OpState::Expired,
        "raising does not restore"
    );
}

/// A user limit is written to the configuration file by a change that does
/// not count as a step.
pub(crate) fn journal_limits_user_is_undone_or_marked_irreversible(_case: &Case) {
    let fixture = Fixture::open(limits_options(3));
    let ops = three_entries(&fixture);
    let outcome = fixture
        .journal
        .limits(&limits_request(Some(5), LimitsScope::User, false))
        .expect("user limit is written");
    assert!(outcome.applied);
    assert!(outcome.expiring.is_empty());
    fixture.journal.sweep().expect("retention pass runs");
    for op in &ops {
        assert_eq!(
            state(&fixture, *op),
            OpState::Available,
            "the limit write is not a step"
        );
    }
    let written = fixture
        .history()
        .into_iter()
        .find(|record| record.kind == "journal.limits")
        .expect("the limit write is recorded");
    let undone = fixture
        .journal
        .undo(&request::undo_op(OpId(written.id)))
        .expect("the limit write is undone");
    assert_eq!(undone.sources, vec![OpId(written.id)]);
}

/// Writes a result file as `save_to` of a read command: a new entry with one
/// created or overwritten target.
fn save_result(fixture: &Fixture, path: &AbsPath, action: TargetAction) -> OpHandle {
    let mut tx: Box<dyn MutationTx + '_> = fixture
        .journal
        .begin(request::begin("list", Some("save_to")));
    tx.preserve(&[spec(path, action, Expect::Unchecked)])
        .expect("result file is preserved");
    tx.apply(TargetIndex(0), Mutation::WriteBytes(AFTER.to_vec()))
        .expect("result is written");
    tx.commit().expect("transaction commits")
}

fn save_round_trip(fixture: &Fixture, path: &AbsPath, action: TargetAction) {
    let before = fixture.snapshot();
    save_result(fixture, path, action);
    let after = fixture.snapshot();
    let outcome = fixture
        .journal
        .undo(&request::undo_last())
        .expect("result is undone");
    before.assert_same(&fixture.snapshot(), "after undo of save_to");
    fixture
        .journal
        .redo(&request::redo(Some(outcome.entry)))
        .expect("save_to is redone");
    after.assert_same(&fixture.snapshot(), "after redo of save_to");
}

/// `save_to` to a new file is recorded as created.
pub(crate) fn save_to_new_file_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    fixture.seed_file("work/input.txt", BEFORE);
    let path = fixture.path("work/result.json");
    save_round_trip(&fixture, &path, TargetAction::Create);
}

/// `save_to` to an existing file preserves its content.
pub(crate) fn save_to_existing_file_is_undone_and_redone(_case: &Case) {
    let fixture = Fixture::standard();
    let path = fixture.seed_file("work/result.json", BEFORE);
    save_round_trip(&fixture, &path, TargetAction::Modify);
}

/// The result file of a changing command is a target of the same entry.
pub(crate) fn save_to_of_changing_command_joins_its_entry(_case: &Case) {
    let fixture = Fixture::standard();
    let data = fixture.seed_file("work/data.txt", BEFORE);
    let result = fixture.path("work/result.json");
    let before = fixture.snapshot();
    let mut begin = request::begin("replace", None);
    begin.bulk = true;
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(begin);
    tx.preserve(&[
        spec(&data, TargetAction::Modify, Expect::Unchecked),
        spec(&result, TargetAction::Create, Expect::Unchecked),
    ])
    .expect("targets are preserved");
    tx.apply(TargetIndex(0), Mutation::WriteBytes(AFTER.to_vec()))
        .expect("data is written");
    tx.apply(TargetIndex(1), Mutation::WriteBytes(AFTER.to_vec()))
        .expect("result is written");
    let handle = tx.commit().expect("transaction commits");
    assert_eq!(handle.targets.len(), 2, "one entry holds both targets");
    fixture
        .journal
        .undo(&request::undo_last())
        .expect("entry is undone");
    before.assert_same(&fixture.snapshot(), "after undo");
}

/// A preview of a limit change reports its effect and changes nothing.
pub(crate) fn preview_changes_nothing(_case: &Case) {
    let fixture = Fixture::standard();
    let ops = three_entries(&fixture);
    let before = fixture.snapshot();
    let status = fixture.status();
    let previewed = fixture
        .journal
        .limits(&limits_request(Some(1), session_scope(), true))
        .expect("limit change is previewed");
    assert!(!previewed.applied);
    assert_eq!(
        previewed.expiring.len(),
        2,
        "the preview counts the entries that would expire"
    );
    assert_eq!(state(&fixture, ops[0]), OpState::Available);
    assert_eq!(fixture.status(), status);
    before.assert_same(&fixture.snapshot(), "after a preview");
}

/// Every pipeline of a script is a separate entry with its own number.
pub(crate) fn script_records_one_entry_per_pipeline(_case: &Case) {
    let fixture = Fixture::standard();
    let first = fixture.seed_file("work/a.txt", BEFORE);
    let second = fixture.seed_file("work/b.txt", BEFORE);
    let one = overwrite(&fixture, &[&first]);
    let two = overwrite(&fixture, &[&second]);
    assert!(two > one, "numbers increase");
    fixture
        .journal
        .undo(&request::undo_last())
        .expect("last pipeline is undone");
    assert_eq!(fixture.read(&first), AFTER);
    assert_eq!(fixture.read(&second), BEFORE);
}

/// Reading further pages of history creates no entry and changes nothing.
pub(crate) fn cursor_page_does_not_rerun_changes(_case: &Case) {
    let fixture = Fixture::standard();
    let ops = three_entries(&fixture);
    let before = fixture.snapshot();
    let page = |before_op: Option<OpId>| {
        fixture
            .journal
            .history(&HistoryQuery {
                limit: Some(1),
                before: before_op,
                ..HistoryQuery::default()
            })
            .expect("page is readable")
    };
    assert_eq!(page(None)[0].id, ops[2].0);
    assert_eq!(page(Some(ops[2]))[0].id, ops[1].0);
    assert_eq!(fixture.history().len(), ops.len());
    before.assert_same(&fixture.snapshot(), "after reading pages");
}

/// Rollback before commit restores the targets and leaves no undoable entry.
pub(crate) fn validation_failure_rolls_back_without_new_entry(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let before = fixture.snapshot();
    let entries = fixture.history().len();
    let mut tx: Box<dyn MutationTx + '_> = fixture
        .journal
        .begin(request::begin("write", Some("overwrite")));
    tx.preserve(&[spec(&file, TargetAction::Modify, Expect::Unchecked)])
        .expect("target is preserved");
    tx.apply(TargetIndex(0), Mutation::WriteBytes(AFTER.to_vec()))
        .expect("change is applied");
    let report = tx
        .rollback(RollbackReason::ValidateFailed)
        .expect("transaction rolls back");
    assert!(report.failed.is_empty());
    before.assert_same(&fixture.snapshot(), "after rollback");
    let available = fixture
        .history()
        .into_iter()
        .filter(|record| record.state == OpState::Available)
        .count();
    assert_eq!(
        available, entries,
        "a rolled back change leaves nothing to undo"
    );
}

/// A target changed since it was read fails with `E_STALE` and is left as
/// found.
pub(crate) fn if_match_mismatch_rolls_back_without_new_entry(_case: &Case) {
    let fixture = Fixture::standard();
    let file = fixture.seed_file("work/a.txt", BEFORE);
    let observed = fixture
        .fs
        .etag(&file, Follow::NoFollow)
        .expect("entity tag is readable");
    fixture.write_outside(&file, FOREIGN);
    let found = fixture.snapshot();
    let entries = fixture.history().len();
    let mut tx: Box<dyn MutationTx + '_> = fixture
        .journal
        .begin(request::begin("write", Some("overwrite")));
    let declared = tx.preserve(&[spec(&file, TargetAction::Modify, Expect::Etag(observed))]);
    match declared {
        Ok(_) => assert_error(
            tx.apply(TargetIndex(0), Mutation::WriteBytes(AFTER.to_vec())),
            ErrorCode::Stale,
        ),
        Err(error) => assert_eq!(error.code(), ErrorCode::Stale, "{error}"),
    }
    found.assert_same(&fixture.snapshot(), "after a stale target");
    let available = fixture
        .history()
        .into_iter()
        .filter(|record| record.state == OpState::Available)
        .count();
    assert_eq!(available, entries, "a stale change leaves nothing to undo");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
