// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of pipelines over streams of mixed record kinds, early stop of
//! sources and the records read before a failed stage.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::{
    Action, FailingSource, FileEffect, ORIGINAL, SABOTAGED, Source, Tagger, World, count_stage,
    entry, error_of, finding, first, path_of, pipeline, pipeline_with_limits, single, stage,
};
use nurt_contracts::bound::Phase;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::ExecStep;
use nurt_contracts::value::{RecordKind, Value};
use nurt_exec::Executor;
use nurt_testkit::journal::{JournalCall, TxState};

const LIST: &str = "mock.list";
const WRITE: &str = "mock.write";
const TAG: &str = "mock.tag";

fn mixed_files(world: &World) -> Vec<nurt_contracts::value::Record> {
    vec![
        entry(&world.file("a.txt"), 1),
        finding(&world.file("b.txt")),
        entry(&world.file("c.txt"), 3),
        finding(&world.file("d.txt")),
    ]
}

fn writers() -> Vec<Arc<dyn nurt_contracts::stage::StageImpl>> {
    vec![
        Arc::new(FileEffect::new(WRITE, RecordKind::Entry, Action::Write)),
        Arc::new(FileEffect::new(WRITE, RecordKind::Finding, Action::Write)),
    ]
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn applies_a_mixed_stream_in_one_transaction() {
    let world = World::new();
    let records = mixed_files(&world);
    let mut stages = writers();
    stages.push(Arc::new(Source::new(LIST, records)));
    let env = world.env(&[LIST, WRITE], stages);
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
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(world.begins(), 1);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].state, TxState::Committed);
    assert_eq!(transactions[0].applied.len(), 4);
    assert_eq!(world.journal.committed().len(), 1);
    for file in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        let path = format!("/data/{file}");
        assert_eq!(world.content(&path), format!("written {path}").into_bytes());
    }
    let operation = outcome.last().unwrap();
    let effect = operation.effect.as_ref().unwrap();
    assert!(effect.applied && effect.bulk);
    let paths: Vec<&str> = effect
        .changes
        .iter()
        .map(|facts| facts.change.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["/data/a.txt", "/data/c.txt", "/data/b.txt", "/data/d.txt"]
    );
    assert_eq!(operation.result.records.len(), 2);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn preserves_the_targets_of_every_group_before_the_first_change() {
    let world = World::new();
    let records = mixed_files(&world);
    let mut stages = writers();
    stages.push(Arc::new(Source::new(LIST, records)));
    let env = world.env(&[LIST, WRITE], stages);
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
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let calls = world.journal.calls();
    let preserved: usize = calls
        .iter()
        .map(|call| match call {
            JournalCall::Preserve { targets, .. } => targets.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(preserved, 4);
    let last_preserve = calls
        .iter()
        .rposition(|call| matches!(call, JournalCall::Preserve { .. }))
        .unwrap();
    let first_apply = calls
        .iter()
        .position(|call| matches!(call, JournalCall::Apply { .. }))
        .unwrap();
    assert!(last_preserve < first_apply, "{calls:?}");
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn rolls_back_the_groups_already_applied_when_a_later_group_fails() {
    let world = World::new();
    let records = mixed_files(&world);
    let mut sabotaging = FileEffect::new(WRITE, RecordKind::Finding, Action::Write);
    sabotaging.sabotage = Some(world.vfs.clone());
    let stages: Vec<Arc<dyn nurt_contracts::stage::StageImpl>> = vec![
        Arc::new(FileEffect::new(WRITE, RecordKind::Entry, Action::Write)),
        Arc::new(sabotaging),
        Arc::new(Source::new(LIST, records)),
    ];
    let env = world.env(&[LIST, WRITE], stages);
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
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(outcome.status, ReportStatus::Failed);
    let transactions = world.journal.transactions();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].state, TxState::RolledBack);
    assert_eq!(world.content("/data/a.txt"), ORIGINAL);
    assert_eq!(world.content("/data/c.txt"), ORIGINAL);
    assert_eq!(world.content("/data/d.txt"), SABOTAGED);
}

#[test]
fn rejects_a_kind_without_implementation_before_changing_anything() {
    let world = World::new();
    let records = vec![
        entry(&world.file("a.txt"), 1),
        nurt_contracts::value::Record::new(RecordKind::Exec),
    ];
    let mut stages = writers();
    stages.push(Arc::new(Source::new(LIST, records)));
    let env = world.env(&[LIST, WRITE], stages);
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
    assert_eq!(error.code(), ErrorCode::Type);
    assert_eq!(error.stage(), Some(ExecStep::Plan));
    assert_eq!(world.begins(), 0);
    assert_eq!(world.content("/data/a.txt"), ORIGINAL);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn previews_a_mixed_stream_without_a_transaction() {
    let world = World::new();
    let records = mixed_files(&world);
    let mut stages = writers();
    stages.push(Arc::new(Source::new(LIST, records)));
    let options = nurt_contracts::envelope::RequestOptions {
        preview: Some(true),
        ..nurt_contracts::envelope::RequestOptions::default()
    };
    let env = world
        .env(&[LIST, WRITE], stages)
        .with_options(Arc::new(options));
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
    let effect = outcome.last().unwrap().effect.as_ref().unwrap();
    assert_eq!(effect.changes.len(), 4);
    assert!(!effect.applied);
    assert_eq!(world.begins(), 0);
    assert_eq!(world.content("/data/b.txt"), ORIGINAL);
}

#[test]
fn joins_the_outputs_of_a_read_stage_by_kind() {
    let world = World::new();
    let records = vec![
        entry("/data/a", 1),
        finding("/data/b"),
        entry("/data/c", 3),
        finding("/data/d"),
    ];
    let stages: Vec<Arc<dyn nurt_contracts::stage::StageImpl>> = vec![
        Arc::new(Source::new(LIST, records)),
        Arc::new(Tagger {
            command: TAG.to_owned(),
            kind: RecordKind::Entry,
            tag: "entry",
        }),
        Arc::new(Tagger {
            command: TAG.to_owned(),
            kind: RecordKind::Finding,
            tag: "finding",
        }),
    ];
    let env = world.env(&[LIST, TAG], stages);
    let outcome = Executor::new().run(
        &single(pipeline(
            None,
            vec![
                stage(LIST, Phase::Source, Vec::new()),
                stage(TAG, Phase::Transform, Vec::new()),
            ],
        )),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let seen: Vec<(String, Option<&str>)> = outcome
        .last()
        .unwrap()
        .result
        .records
        .iter()
        .map(|record| (path_of(record), record.get("tag").and_then(Value::as_str)))
        .collect();
    assert_eq!(
        seen,
        [
            ("/data/a".to_owned(), Some("entry")),
            ("/data/c".to_owned(), Some("entry")),
            ("/data/b".to_owned(), Some("finding")),
            ("/data/d".to_owned(), Some("finding")),
        ]
    );
}

#[test]
fn reads_only_the_demanded_records_of_a_mixed_stream() {
    let world = World::new();
    let records: Vec<_> = (0..20)
        .map(|n| {
            if n % 2 == 0 {
                entry(&format!("/data/e{n}"), n)
            } else {
                finding(&format!("/data/f{n}"))
            }
        })
        .collect();
    let source = Source::new(LIST, records);
    let pulled = Arc::clone(&source.pulled);
    let stages: Vec<Arc<dyn nurt_contracts::stage::StageImpl>> = vec![
        Arc::new(source),
        Arc::new(Tagger {
            command: TAG.to_owned(),
            kind: RecordKind::Entry,
            tag: "entry",
        }),
        Arc::new(Tagger {
            command: TAG.to_owned(),
            kind: RecordKind::Finding,
            tag: "finding",
        }),
    ];
    let env = world.env(&[LIST, TAG], stages);
    let outcome = Executor::new().run(
        &single(pipeline_with_limits(
            None,
            vec![
                stage(LIST, Phase::Source, Vec::new()),
                stage(TAG, Phase::Transform, Vec::new()),
            ],
            &[Some(5), None],
        )),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(pulled.load(Ordering::SeqCst), 5);
    assert_eq!(outcome.last().unwrap().result.records.len(), 5);
}

#[test]
fn stops_the_source_after_first() {
    let world = World::new();
    let records: Vec<_> = (0..50).map(|n| entry(&format!("/data/f{n}"), n)).collect();
    let source = Source::new(LIST, records);
    let pulled = Arc::clone(&source.pulled);
    let env = world.env(&[LIST], vec![Arc::new(source)]);
    let outcome = Executor::new().run(
        &single(pipeline(
            None,
            vec![
                stage(LIST, Phase::Source, Vec::new()),
                first(3),
                count_stage(),
            ],
        )),
        &env,
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.last().unwrap().result.value, Some(Value::Int(3)));
    assert!(pulled.load(Ordering::SeqCst) <= 3);
}

#[test]
fn reports_the_records_read_before_a_failed_stage() {
    let world = World::new();
    let records: Vec<_> = (0..3).map(|n| entry(&format!("/data/f{n}"), n)).collect();
    let env = world.env(
        &[LIST],
        vec![Arc::new(FailingSource {
            command: LIST.to_owned(),
            records,
        })],
    );
    let outcome = Executor::new().run(
        &single(pipeline(None, vec![stage(LIST, Phase::Source, Vec::new())])),
        &env,
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert_eq!(error.stage(), Some(ExecStep::Read));
    let partial = error.details.get("partial_read").unwrap();
    assert_eq!(partial["count"], 3);
    assert_eq!(partial["records"].as_array().unwrap().len(), 3);
    assert!(outcome.last().unwrap().result.records.is_empty());
}
