// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/golden.rs
// Responsibility:   End-to-end execution of bound programs from the golden documents.
// Dependencies:     common, nurt_contracts, nurt_plan, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! End-to-end execution of bound programs from the golden documents: the
//! document is planned with the catalog and executed on a virtual file
//! system with mock sources and effects next to the record stages.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{Action, FileEffect, Launcher, Source, World, error_of};
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{ExecStep, StageImpl};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};
use nurt_exec::{Executor, Outcome};
use nurt_plan::Plan;
use nurt_testkit::journal::TxState;

const MEGABYTE: u64 = 1_000_000;
const NANOS_PER_SECOND: i64 = 1_000_000_000;
const TEN_MINUTES: Duration = Duration::from_secs(600);

fn golden(name: &str) -> BoundDocument {
    let path = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/bound")
    .join(format!("{name}.json"));
    let text = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn planned(
    world: &World,
    name: &str,
    stages: Vec<Arc<dyn StageImpl>>,
) -> (Plan, nurt_exec::ExecEnv) {
    let env = world.env(&[], stages);
    let plan = nurt_plan::plan(&golden(name), &env.registry).unwrap();
    (plan, env)
}

fn recent(world: &World) -> Instant {
    let now = Instant::saturating_from_wall(nurt_contracts::env::Clock::now(world.clock.as_ref()));
    let age = i64::try_from(TEN_MINUTES.as_secs()).unwrap() * NANOS_PER_SECOND;
    Instant::from_unix_nanos(now.as_unix_nanos() - age)
}

fn sized_entries(world: &World, count: u64) -> Vec<Record> {
    let modified = recent(world);
    (0..count)
        .map(|n| {
            Record::new(RecordKind::Entry)
                .with(
                    "path",
                    Value::Path(nurt_testkit::paths::abs(&format!("/srv/app/f{n}.go"))),
                )
                .with(
                    "size",
                    Value::Size(nurt_contracts::value::Size::from_bytes(n * MEGABYTE / 10)),
                )
                .with("modified", Value::Instant(modified))
        })
        .collect()
}

fn run(plan: &Plan, env: &nurt_exec::ExecEnv) -> Outcome {
    Executor::new().run(plan, env)
}

#[test]
fn lists_the_largest_files() {
    let world = World::new();
    let source = Source::new("find", sized_entries(&world, 40));
    let (plan, env) = planned(&world, "task-01-largest-go-files", vec![Arc::new(source)]);
    let outcome = run(&plan, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let records = &outcome.last().unwrap().result.records;
    assert_eq!(records.len(), 20);
    let sizes: Vec<u64> = records
        .iter()
        .filter_map(|record| match record.get("size") {
            Some(Value::Size(size)) => Some(size.bytes()),
            _ => None,
        })
        .collect();
    assert!(sizes.windows(2).all(|pair| pair[0] >= pair[1]));
    assert_eq!(sizes[0], 39 * MEGABYTE / 10);
    assert!(sizes.iter().all(|size| *size > MEGABYTE));
}

#[test]
fn computes_a_named_result_once() {
    let world = World::new();
    let source = Source::new("find", sized_entries(&world, 7));
    let pulled = Arc::clone(&source.pulled);
    let (plan, env) = planned(&world, "task-06-named-result-count", vec![Arc::new(source)]);
    let outcome = run(&plan, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.lets, ["recent"]);
    assert_eq!(outcome.last().unwrap().result.value, Some(Value::Int(7)));
    assert_eq!(pulled.load(Ordering::SeqCst), 7);
}

fn temporary_files(world: &World) -> Vec<Record> {
    ["a.tmp", "b.tmp", "c.tmp"]
        .into_iter()
        .map(|name| {
            Record::new(RecordKind::Entry).with(
                "path",
                Value::Path(nurt_testkit::paths::abs(&world.file(name))),
            )
        })
        .collect()
}

#[test]
fn removes_the_listed_files_in_one_operation() {
    let world = World::new();
    let stages: Vec<Arc<dyn StageImpl>> = vec![
        Arc::new(Source::new("find", temporary_files(&world))),
        Arc::new(FileEffect::new("remove", RecordKind::Entry, Action::Remove)),
    ];
    let (plan, env) = planned(&world, "task-03-remove-temporary-files", stages);
    let outcome = run(&plan, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(world.begins(), 1);
    assert_eq!(world.journal.transactions()[0].state, TxState::Committed);
    for name in ["a.tmp", "b.tmp", "c.tmp"] {
        assert!(!world.exists(&format!("/data/{name}")));
    }
    let effect = outcome.last().unwrap().effect.as_ref().unwrap();
    assert!(effect.applied && effect.bulk);
}

#[test]
fn previews_the_removal_without_changing_files() {
    let world = World::new();
    let stages: Vec<Arc<dyn StageImpl>> = vec![
        Arc::new(Source::new("find", temporary_files(&world))),
        Arc::new(FileEffect::new("remove", RecordKind::Entry, Action::Remove)),
    ];
    let options = RequestOptions {
        preview: Some(true),
        ..RequestOptions::default()
    };
    let env = world.env(&[], stages).with_options(Arc::new(options));
    let plan = nurt_plan::plan(&golden("task-03-remove-temporary-files"), &env.registry).unwrap();
    let outcome = run(&plan, &env);
    assert_eq!(outcome.status, ReportStatus::Planned);
    assert_eq!(world.begins(), 0);
    assert!(world.exists("/data/a.tmp"));
    let effect = outcome.last().unwrap().effect.as_ref().unwrap();
    assert_eq!(effect.changes.len(), 3);
}

fn committed_transactions(world: &World) -> usize {
    world
        .journal
        .transactions()
        .iter()
        .filter(|transaction| transaction.state == TxState::Committed)
        .count()
}

fn launcher(exit_code: i64, detached: bool) -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Launcher {
        command: "run".to_owned(),
        exit_code,
        detached,
    })]
}

#[test]
fn passes_the_assertion_on_the_exit_code() {
    let world = World::new();
    let (plan, env) = planned(&world, "task-07-tests-exit-code", launcher(0, false));
    let outcome = run(&plan, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.status, ReportStatus::Ok);
    assert_eq!(committed_transactions(&world), 1);
}

#[test]
fn keeps_the_effect_when_the_assertion_on_the_exit_code_fails() {
    let world = World::new();
    let (plan, env) = planned(&world, "task-07-tests-exit-code", launcher(2, false));
    let outcome = run(&plan, &env);
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(error.stage(), Some(ExecStep::After));
    assert!(error.effect_applied);
    assert_eq!(committed_transactions(&world), 1);
    let operation = outcome.last().unwrap();
    assert_eq!(operation.result.records[0].kind, RecordKind::Exec);
}

#[test]
fn reports_a_background_job_as_running() {
    let world = World::new();
    let (plan, env) = planned(&world, "task-14-background-job", launcher(0, true));
    let outcome = run(&plan, &env);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.status, ReportStatus::Running);
    assert!(outcome.last().unwrap().effect.as_ref().unwrap().detached);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
