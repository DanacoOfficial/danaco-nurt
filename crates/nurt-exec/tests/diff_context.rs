// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/diff_context.rs
// Responsibility:   Context lines of the hunks of an effect: the executor limits them to the.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Context lines of the hunks of an effect: the executor limits them to the
//! request option `diff_context`, falling back to its default.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::sync::Arc;

use common::{Action, FileEffect, Source, World, entry, pipeline, single, stage};
use nurt_contracts::bound::Phase;
use nurt_contracts::envelope::{DEFAULT_DIFF_CONTEXT, RequestOptions};
use nurt_contracts::error::Result;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::Hunk;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, StageDescriptor, StageImpl,
    StageInput,
};
use nurt_contracts::value::{Path, RecordKind};
use nurt_exec::{ExecEnv, Executor, Outcome};

const LIST: &str = "mock.list";
const WRITE: &str = "mock.write";
const SURPLUS: usize = 6;

/// Effect that plans every change with a hunk carrying `SURPLUS` lines of
/// context on both sides of one replaced line, whatever the request asks.
struct Wide(FileEffect);

impl StageImpl for Wide {
    fn descriptor(&self) -> StageDescriptor {
        self.0.descriptor()
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Wide {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let mut plan = self.0.plan(args, input, ctx)?;
        for change in &mut plan.changes {
            let mut lines = vec![" before".to_owned(); SURPLUS];
            lines.push("-old".to_owned());
            lines.push("+new".to_owned());
            lines.extend(vec![" after".to_owned(); SURPLUS]);
            let count = (2 * SURPLUS + 1) as u64;
            change.diff = Some(vec![Hunk {
                path: Path::new(change.path.as_str()),
                old_start: 1,
                old_len: count,
                new_start: 1,
                new_len: count,
                lines,
                ..Hunk::default()
            }]);
        }
        Ok(plan)
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        self.0.apply(args, plan, tx, ctx)
    }
}

fn run(world: &World, options: RequestOptions) -> Outcome {
    let stages: Vec<Arc<dyn StageImpl>> = vec![
        Arc::new(Wide(FileEffect::new(
            WRITE,
            RecordKind::Entry,
            Action::Write,
        ))),
        Arc::new(Source::new(LIST, vec![entry(&world.file("a.txt"), 1)])),
    ];
    let env: ExecEnv = world
        .env(&[LIST, WRITE], stages)
        .with_options(Arc::new(options));
    Executor::new().run(
        &single(pipeline(
            None,
            vec![
                stage(LIST, Phase::Source, Vec::new()),
                stage(WRITE, Phase::Mutate, Vec::new()),
            ],
        )),
        &env,
    )
}

fn hunk_of(outcome: &Outcome) -> Hunk {
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let effect = outcome.last().unwrap().effect.as_ref().unwrap();
    effect.changes[0].change.diff.clone().unwrap().remove(0)
}

fn context_lines(hunk: &Hunk) -> (usize, usize) {
    let before = hunk.lines.iter().take_while(|l| l.starts_with(' ')).count();
    let after = hunk
        .lines
        .iter()
        .rev()
        .take_while(|l| l.starts_with(' '))
        .count();
    (before, after)
}

#[test]
fn applied_hunks_keep_the_requested_context() {
    let world = World::new();
    let options = RequestOptions {
        diff_context: Some(2),
        ..RequestOptions::default()
    };
    let hunk = hunk_of(&run(&world, options));
    assert_eq!(context_lines(&hunk), (2, 2));
    assert_eq!((hunk.old_start, hunk.old_len), (5, 5));
    assert_eq!((hunk.new_start, hunk.new_len), (5, 5));
}

#[test]
fn planned_hunks_of_a_preview_keep_the_requested_context() {
    let world = World::new();
    let options = RequestOptions {
        diff_context: Some(1),
        preview: Some(true),
        ..RequestOptions::default()
    };
    let hunk = hunk_of(&run(&world, options));
    assert_eq!(context_lines(&hunk), (1, 1));
    assert_eq!(world.begins(), 0);
}

#[test]
fn context_defaults_to_the_contract_value() {
    let world = World::new();
    let hunk = hunk_of(&run(&world, RequestOptions::default()));
    let keep = DEFAULT_DIFF_CONTEXT as usize;
    assert_eq!(context_lines(&hunk), (keep, keep));
}

#[test]
fn zero_context_leaves_only_the_changed_lines() {
    let world = World::new();
    let options = RequestOptions {
        diff_context: Some(0),
        ..RequestOptions::default()
    };
    let hunk = hunk_of(&run(&world, options));
    assert_eq!(hunk.lines, ["-old", "+new"].map(String::from));
}

#[test]
fn context_is_not_added_when_the_stage_gave_less() {
    let world = World::new();
    let options = RequestOptions {
        diff_context: Some(100),
        ..RequestOptions::default()
    };
    let hunk = hunk_of(&run(&world, options));
    assert_eq!(context_lines(&hunk), (SURPLUS, SURPLUS));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
