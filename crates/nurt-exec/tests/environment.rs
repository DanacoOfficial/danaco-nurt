// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/environment.rs
// Responsibility:   Environments of an execution: without a journal and with extensions of.
// Dependencies:     common, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Environments of an execution: without a journal and with extensions of
//! the stage contexts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{
    Action, FileEffect, Source, World, count_stage, entry, error_of, pipeline, single, stage,
};
use nurt_contracts::bound::Phase;
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, ExecStep, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{RecordKind, Value};
use nurt_exec::{ExecEnv, Executor};
use nurt_testkit::host::TestHost;

const LIST: &str = "mock.list";
const WRITE: &str = "mock.write";
const CACHE: &str = "mock.cache";

fn read_only(world: &World, mocks: &[&str], stages: Vec<Arc<dyn StageImpl>>) -> ExecEnv {
    ExecEnv::read_only(
        common::registry(mocks, stages),
        world.vfs.clone(),
        world.clock.clone(),
        Arc::new(TestHost::linux()),
    )
}

#[test]
fn runs_a_program_that_only_reads_without_a_journal() {
    let world = World::new();
    let source = Source::new(LIST, vec![entry("/data/a", 1), entry("/data/b", 2)]);
    let env = read_only(&world, &[LIST], vec![Arc::new(source)]);
    let outcome = Executor::new().run(
        &single(pipeline(
            None,
            vec![stage(LIST, Phase::Source, Vec::new()), count_stage()],
        )),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.last().unwrap().result.value, Some(Value::Int(2)));
}

#[test]
fn refuses_an_effect_without_a_journal() {
    let world = World::new();
    let file = world.file("a.txt");
    let stages: Vec<Arc<dyn StageImpl>> = vec![
        Arc::new(Source::new(LIST, vec![entry(&file, 1)])),
        Arc::new(FileEffect::new(WRITE, RecordKind::Entry, Action::Write)),
    ];
    let env = read_only(&world, &[LIST, WRITE], stages);
    let outcome = Executor::new().run(
        &single(pipeline(
            None,
            vec![
                stage(LIST, Phase::Source, Vec::new()),
                stage(WRITE, Phase::Mutate, Vec::new()),
            ],
        )),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert_eq!(error.stage(), Some(ExecStep::Apply));
    assert_eq!(world.content(&file), common::ORIGINAL);
}

#[test]
fn previews_an_effect_without_a_journal() {
    let world = World::new();
    let file = world.file("a.txt");
    let stages: Vec<Arc<dyn StageImpl>> = vec![
        Arc::new(Source::new(LIST, vec![entry(&file, 1)])),
        Arc::new(FileEffect::new(WRITE, RecordKind::Entry, Action::Write)),
    ];
    let options = RequestOptions {
        preview: Some(true),
        ..RequestOptions::default()
    };
    let env = read_only(&world, &[LIST, WRITE], stages).with_options(Arc::new(options));
    let outcome = Executor::new().run(
        &single(pipeline(
            None,
            vec![
                stage(LIST, Phase::Source, Vec::new()),
                stage(WRITE, Phase::Mutate, Vec::new()),
            ],
        )),
        &env,
    );
    assert_eq!(outcome.status, ReportStatus::Planned);
}

/// Cache shared with the stages through the extensions of their contexts.
#[derive(Default)]
struct ReadCache {
    lookups: AtomicUsize,
}

struct CacheReader;

impl StageImpl for CacheReader {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: CACHE.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CacheReader {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let seen = ctx.extensions().get::<ReadCache>().map(|cache| {
            cache.lookups.fetch_add(1, Ordering::SeqCst);
        });
        let record = entry("/data/cached", i64::from(seen.is_some()));
        Ok(StageOutput::records(RecordStream::once(record)))
    }
}

#[test]
fn passes_extensions_to_the_stage_contexts() {
    let world = World::new();
    let cache = Arc::new(ReadCache::default());
    let env = world
        .env(&[CACHE], vec![Arc::new(CacheReader)])
        .with_extension(Arc::clone(&cache));
    let outcome = Executor::new().run(
        &single(pipeline(
            None,
            vec![stage(CACHE, Phase::Source, Vec::new())],
        )),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(cache.lookups.load(Ordering::SeqCst), 1);
    assert_eq!(
        outcome.last().unwrap().result.records[0].get("size"),
        Some(&Value::Int(1))
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
