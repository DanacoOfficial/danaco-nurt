// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Property tests of the journal protocol.
//!
//! Random sequences of writes, undos, limit changes, clock steps,
//! suspensions, restarts and injected crashes are run against the journal
//! and compared with a reference model. Failing cases are not persisted to
//! files; a failing seed is turned into an explicit test case.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions report failures by panicking"
)]

#[allow(
    dead_code,
    reason = "the harness is shared by several test targets, each of which uses a subset"
)]
#[path = "../coverage/harness/mod.rs"]
mod harness;
mod model;

use std::collections::BTreeSet;
use std::time::Duration;

use nurt_contracts::config::{AgeLimit, StepLimit};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, LimitsRequest, LimitsScope, Mutation, MutationTx, OpId, OpState, SessionId,
    TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use proptest::prelude::*;
use proptest::test_runner::Config;

use harness::fixture::{Fixture, FixtureOptions};
use harness::request;
use model::{Action, Model};

/// Step limit of the sequences; small so that sequences reach it.
const STEPS: u32 = 8;
/// Age limit of the sequences.
const AGE: Duration = Duration::from_secs(30 * 60);
/// Files written by the sequences.
const FILES: u8 = 4;
/// Longest sequence.
const MAX_ACTIONS: usize = 40;
/// Cases per property.
const CASES: u32 = 32;
const MINUTE: Duration = Duration::from_secs(60);

fn config() -> Config {
    Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    }
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        6 => (0..FILES).prop_map(Action::Write),
        2 => Just(Action::UndoLast),
        1 => (1..=STEPS + 2).prop_map(Action::Limit),
        2 => (1u16..20).prop_map(Action::Advance),
        1 => (1u16..120).prop_map(Action::Suspend),
        1 => (1u16..60).prop_map(Action::StepBack),
        1 => (1u16..90).prop_map(Action::StepForward),
        1 => (1u16..60).prop_map(Action::Restart),
    ]
}

fn actions() -> impl Strategy<Value = Vec<Action>> {
    prop::collection::vec(action(), 1..MAX_ACTIONS)
}

fn options() -> FixtureOptions {
    let mut options = FixtureOptions::default();
    options.journal.max_ops = StepLimit::Limited(STEPS);
    options.journal.max_age = "30m".parse::<AgeLimit>().expect("age limit is valid");
    options
}

fn file(fixture: &Fixture, index: u8) -> AbsPath {
    fixture.path(&format!("work/{index}.txt"))
}

fn seed(fixture: &Fixture) {
    for index in 0..FILES {
        fixture.seed_file(&format!("work/{index}.txt"), b"initial\n");
    }
}

fn write(fixture: &Fixture, index: u8, round: usize) -> OpId {
    let path = file(fixture, index);
    let mut tx: Box<dyn MutationTx + '_> = fixture
        .journal
        .begin(request::begin("write", Some("overwrite")));
    tx.preserve(&[TargetSpec {
        path: TargetPath::Fs(path),
        action: TargetAction::Modify,
        expect: Expect::Unchecked,
        size_hint: None,
    }])
    .expect("the journal never fails a user operation");
    tx.apply(
        TargetIndex(0),
        Mutation::WriteBytes(format!("round {round}\n").into_bytes()),
    )
    .expect("the journal never fails a user operation");
    tx.commit()
        .expect("the journal never fails a user operation")
        .op
        .expect("a committed change has a number")
}

/// Entries created so far by a sequence, in commit order, and the entries
/// that are undo entries.
#[derive(Default)]
struct Trace {
    ops: Vec<OpId>,
    undo_entries: Vec<usize>,
}

/// Runs the actions on the journal and the model; returns the entry numbers
/// in commit order.
fn run(fixture: &mut Fixture, model: &mut Model, actions: &[Action]) -> Vec<OpId> {
    let mut trace = Trace::default();
    for (round, action) in actions.iter().enumerate() {
        step(fixture, model, &mut trace, round, *action);
    }
    trace.ops
}

/// Runs one action as the `round`-th of its sequence.
fn step(fixture: &mut Fixture, model: &mut Model, trace: &mut Trace, round: usize, action: Action) {
    let Trace { ops, undo_entries } = trace;
    {
        match action {
            Action::Write(index) => ops.push(write(fixture, index, round)),
            Action::UndoLast => {
                if let Some(index) = model.newest_available(undo_entries) {
                    let outcome = fixture
                        .journal
                        .undo(&request::undo_op(ops[index]))
                        .expect("an available entry is undone");
                    ops.push(outcome.entry);
                }
            }
            Action::Limit(steps) => {
                fixture
                    .journal
                    .limits(&LimitsRequest {
                        steps: Some(StepLimit::Limited(steps)),
                        age: None,
                        size: None,
                        scope: LimitsScope::Session(SessionId(request::SESSION.to_owned())),
                        volume: None,
                        preview: false,
                    })
                    .expect("limit change is accepted");
            }
            Action::Advance(minutes) => fixture.clock.advance(MINUTE * u32::from(minutes)),
            Action::Suspend(minutes) => fixture.clock.suspend(MINUTE * u32::from(minutes)),
            Action::StepBack(minutes) => fixture.clock.step_wall_back(MINUTE * u32::from(minutes)),
            Action::StepForward(minutes) => {
                fixture.clock.step_wall_forward(MINUTE * u32::from(minutes));
            }
            Action::Restart(minutes) => {
                fixture.clock.reboot(MINUTE * u32::from(minutes));
                fixture.restart();
                fixture.journal.recover().expect("recovery runs");
            }
        }
        model.apply(action, undo_entries);
        fixture.journal.sweep().expect("sweep runs");
    }
}

fn states(fixture: &Fixture, ops: &[OpId]) -> Vec<OpState> {
    ops.iter().map(|op| fixture.record(*op).state).collect()
}

/// An entry committed after the wall clock was moved back starts from the
/// effective wall time, so its age is not inflated by the step.
#[test]
fn entry_committed_after_wall_clock_step_back_keeps_its_age() {
    let mut fixture = Fixture::open(options());
    seed(&fixture);
    let mut model = Model::new(STEPS, AGE);
    let actions = [Action::Write(0), Action::StepBack(30), Action::Write(0)];
    let ops = run(&mut fixture, &mut model, &actions);
    assert_eq!(fixture.record(ops[1]).state, OpState::Available);
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn journal_states_match_reference_model(actions in actions()) {
        let mut fixture = Fixture::open(options());
        seed(&fixture);
        let mut model = Model::new(STEPS, AGE);
        let ops = run(&mut fixture, &mut model, &actions);
        for (entry, op) in model.entries.iter().zip(&ops) {
            let state = fixture.record(*op).state;
            prop_assert_eq!(state == OpState::Expired, entry.expired, "entry {:?}", op);
            if !entry.expired {
                prop_assert_eq!(state == OpState::Undone, entry.undone, "entry {:?}", op);
            }
        }
    }

    #[test]
    fn numbers_are_unique_and_increase_with_commit_order(actions in actions()) {
        let mut fixture = Fixture::open(options());
        seed(&fixture);
        let mut model = Model::new(STEPS, AGE);
        let ops = run(&mut fixture, &mut model, &actions);
        let unique: BTreeSet<OpId> = ops.iter().copied().collect();
        prop_assert_eq!(unique.len(), ops.len());
        prop_assert!(ops.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn oldest_available_entry_never_moves_back(actions in actions()) {
        let mut fixture = Fixture::open(options());
        seed(&fixture);
        let mut model = Model::new(STEPS, AGE);
        let mut oldest: Option<OpId> = None;
        let mut trace = Trace::default();
        for (round, action) in actions.iter().enumerate() {
            step(&mut fixture, &mut model, &mut trace, round, *action);
            let current = fixture
                .journal
                .status(Some(fixture.volume(0)))
                .ok()
                .and_then(|mut states| states.pop())
                .and_then(|state| state.oldest)
                .map(|entry| entry.op);
            if let (Some(before), Some(now)) = (oldest, current) {
                prop_assert!(now >= before, "oldest entry moved from {:?} to {:?}", before, now);
            }
            oldest = current.or(oldest);
        }
    }

    #[test]
    fn expiry_is_deterministic(actions in actions()) {
        let mut first = Fixture::open(options());
        seed(&first);
        let ops = run(&mut first, &mut Model::new(STEPS, AGE), &actions);
        let mut second = Fixture::open(options());
        seed(&second);
        let again = run(&mut second, &mut Model::new(STEPS, AGE), &actions);
        prop_assert_eq!(states(&first, &ops), states(&second, &again));
    }

    #[test]
    fn undo_of_committed_entry_restores_previous_tree(writes in prop::collection::vec(0..FILES, 1..12)) {
        let fixture = Fixture::open(options());
        seed(&fixture);
        for (round, index) in writes.iter().enumerate() {
            let before = fixture.snapshot();
            write(&fixture, *index, round);
            fixture.journal.undo(&request::undo_last()).expect("entry is undone");
            before.assert_same(&fixture.snapshot(), "after undo");
            fixture.journal.redo(&request::redo(None)).expect("undo is redone");
        }
    }

    #[test]
    fn conflict_report_changes_nothing(index in 0..FILES, foreign in "[a-z]{1,16}") {
        let fixture = Fixture::open(options());
        seed(&fixture);
        let op = write(&fixture, index, 0);
        fixture.write_outside(&file(&fixture, index), foreign.as_bytes());
        let before = fixture.snapshot();
        let entries = fixture.history();
        let error = fixture.journal.undo(&request::undo_op(op)).expect_err("the undo conflicts");
        prop_assert_eq!(error.code(), ErrorCode::JournalConflict);
        before.assert_same(&fixture.snapshot(), "after a conflict report");
        prop_assert_eq!(fixture.history(), entries);
    }

    #[test]
    fn recovery_after_crash_leaves_every_file_old_or_new(seed_value in any::<u64>(), writes in prop::collection::vec(0..FILES, 1..10)) {
        let mut options = options();
        options.crash_seed = Some(seed_value);
        let mut fixture = Fixture::open(options);
        seed(&fixture);
        fn try_write(fixture: &Fixture, index: u8, round: usize) -> nurt_contracts::error::Result<()> {
            let mut tx: Box<dyn MutationTx + '_> =
                fixture.journal.begin(request::begin("write", Some("overwrite")));
            tx.preserve(&[TargetSpec {
                path: TargetPath::Fs(file(fixture, index)),
                action: TargetAction::Modify,
                expect: Expect::Unchecked,
                size_hint: None,
            }])?;
            tx.apply(TargetIndex(0), Mutation::WriteBytes(format!("round {round}\n").into_bytes()))?;
            tx.commit().map(|_| ())
        }

        let mut committed: Vec<(u8, usize)> = Vec::new();
        let mut interrupted = None;
        for (round, index) in writes.iter().enumerate() {
            if try_write(&fixture, *index, round).is_err() {
                interrupted = Some((*index, round));
                break;
            }
            committed.push((*index, round));
        }
        fixture.restart();
        fixture.journal.recover().expect("recovery completes");
        for index in 0..FILES {
            let content = fixture.read(&file(&fixture, index));
            let previous = committed
                .iter()
                .rev()
                .find(|(file_index, _)| *file_index == index)
                .map_or_else(|| b"initial\n".to_vec(), |(_, round)| format!("round {round}\n").into_bytes());
            let mut allowed = vec![previous];
            if let Some((file_index, round)) = interrupted
                && file_index == index
            {
                allowed.push(format!("round {round}\n").into_bytes());
            }
            prop_assert!(allowed.contains(&content), "file {} is neither old nor new after recovery", index);
        }
    }
}
