// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Programs run end to end through the engine on a virtual file system and a
//! recording journal. The commands `find` and `remove` are test stages that
//! work on the virtual file system; the record stages are those of the build.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::{Arc, Mutex};

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::envelope::{Request, RequestOptions};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::ChangeAction;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::value::{RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::events::{DeliveredEvent, EventListener};
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::vfs::{VirtualFs, test_path};
use serde_json::json;
use support::{
    FILES, Find, PROGRAM_HASH, Programs, Remove, count_stage, exists, filter_stage, find_stage,
    program, remove_stage, run, world, world_with,
};

#[derive(Default)]
struct Collect(Mutex<Vec<DeliveredEvent>>);

impl EventListener for Collect {
    fn deliver(&self, event: DeliveredEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn counting_filtered_entries_returns_the_count() {
    let world = world();
    let response = run(&world, "find /data |> filter size > 150 |> count", false);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Ok, "{:?}", report.error);
    assert_eq!(report.kind, "find");
    let page = report.records.as_ref().unwrap();
    assert_eq!(page.record_type, RecordKind::Row);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].get("value"), Some(&Value::from(2_u64)));
    assert!(report.changes.is_empty());
    assert!(report.timing.is_some());
    assert!(exists(&world, "/data/a.log"));
}

#[test]
fn script_with_two_pipelines_reports_each_pipeline_with_its_own_kind() {
    const SCRIPT: &str = "find /data |> count\nfind /data |> filter size > 150 |> remove";
    let bound: BoundDocument = serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [
            {"t": "pipeline", "head": find_stage(), "stages": [count_stage()]},
            {"t": "pipeline", "head": find_stage(), "stages": [filter_stage(), remove_stage()]}
        ]}
    }))
    .unwrap();
    let world = world_with(Programs::default().with(SCRIPT, bound));
    let options = RequestOptions {
        script: Some(true),
        ..RequestOptions::default()
    };
    let response = world
        .engine
        .run(&Request::program(SCRIPT).with_options(options));
    response.validate().unwrap();
    assert_eq!(response.report.status, ReportStatus::Ok, "{:?}", response.report.error);
    assert!(
        response.report.records.is_none(),
        "the records belong to the report of their own pipeline"
    );
    assert_eq!(response.reports.len(), 2);

    let counted = &response.reports[0];
    assert_eq!(counted.kind, "find");
    let page = counted.records.as_ref().unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].get("value"), Some(&Value::from(3_u64)));
    assert!(counted.changes.is_empty());

    let removed = &response.reports[1];
    assert_eq!(removed.kind, "remove");
    assert_eq!(removed.changes.len(), 2);
    assert!(removed.records.is_none());
    assert!(!exists(&world, "/data/b.log"));
    assert!(!exists(&world, "/data/c.log"));
    assert!(exists(&world, "/data/a.log"));
}

#[test]
fn program_with_one_pipeline_has_no_pipeline_reports() {
    let world = world();
    let response = run(&world, "find /data", false);
    assert!(response.reports.is_empty());
}

#[test]
fn reading_returns_the_records_of_the_source() {
    let world = world();
    let response = run(&world, "find /data", false);
    let page = response.report.records.as_ref().unwrap();
    assert_eq!(page.record_type, RecordKind::Entry);
    assert_eq!(page.rows.len(), 3);
    assert_eq!(page.matched, Some(3));
}

#[test]
fn removing_found_entries_deletes_them_and_reports_the_changes() {
    let world = world();
    let response = run(&world, "find /data |> remove", false);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Ok, "{:?}", report.error);
    assert_eq!(report.kind, "remove");
    assert_eq!(report.changes.len(), 3);
    assert!(
        report
            .changes
            .iter()
            .all(|change| change.action == ChangeAction::Delete)
    );
    assert_eq!(report.targets.as_ref().unwrap().count, 3);
    assert_eq!(report.summary.as_ref().unwrap().files_deleted, Some(3));
    assert!(report.op.is_some());
    for (path, _) in FILES {
        assert!(!exists(&world, path), "{path}");
    }
    assert_eq!(world.journal.committed().len(), 1);
}

#[test]
fn removing_filtered_entries_keeps_the_others() {
    let world = world();
    let response = run(&world, "find /data |> filter size > 150 |> remove", false);
    assert_eq!(response.report.status, ReportStatus::Ok);
    assert_eq!(response.report.changes.len(), 2);
    assert!(exists(&world, "/data/a.log"));
    assert!(!exists(&world, "/data/b.log"));
    assert!(!exists(&world, "/data/c.log"));
}

#[test]
fn preview_plans_the_changes_without_applying_them() {
    let world = world();
    let response = run(&world, "find /data |> remove", true);
    let report = &response.report;
    assert_eq!(report.status, ReportStatus::Planned, "{:?}", report.error);
    assert_eq!(report.changes.len(), 3);
    assert!(report.op.is_none());
    for (path, _) in FILES {
        assert!(exists(&world, path), "{path}");
    }
    assert!(world.journal.committed().is_empty());
}

fn engine_without_journal() -> Engine {
    let clock = Arc::new(VirtualClock::new());
    let vfs = Arc::new(VirtualFs::new(clock.clone()));
    vfs.create_dir_all(&test_path("/data").unwrap()).unwrap();
    vfs.write_file(&test_path("/data/a.log").unwrap(), b"x")
        .unwrap();
    let frontend = Programs::default()
        .with("find /data", program(find_stage(), Vec::new()))
        .with(
            "find /data |> remove",
            program(find_stage(), vec![remove_stage()]),
        );
    Engine::open(
        EngineConfig::new(clock, Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_fs(vfs)
            .with_frontend(Arc::new(frontend))
            .with_stage(Arc::new(Find))
            .with_stage(Arc::new(Remove)),
    )
    .unwrap()
}

#[test]
fn read_only_program_runs_without_journal() {
    let response = engine_without_journal().run(&Request::program("find /data"));
    response.validate().unwrap();
    assert!(response.report.error.is_none());
    assert!(response.report.records.is_some());
    assert_eq!(response.report.ast_hash.as_deref(), Some(PROGRAM_HASH));
}

#[test]
fn program_with_effects_fails_without_journal() {
    let response = engine_without_journal().run(&Request::program("find /data |> remove"));
    response.validate().unwrap();
    let error = response.report.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::StageFailed);
}

#[test]
fn started_requests_deliver_the_events_of_the_executor_once() {
    let world = world();
    let events = Arc::new(Collect::default());
    let handle = world
        .engine
        .start(&Request::program("find /data |> remove"), events.clone());
    let response = handle.wait(None).unwrap();
    assert_eq!(response.report.status, ReportStatus::Ok);
    let delivered = events.0.lock().unwrap().clone();
    let kinds: Vec<String> = delivered
        .iter()
        .map(|event| {
            serde_json::to_value(event).unwrap()["t"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds.first().map(String::as_str), Some("exec.started"));
    assert_eq!(kinds.last().map(String::as_str), Some("exec.finished"));
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "exec.started").count(),
        1
    );
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "exec.finished").count(),
        1
    );
    let seqs: Vec<u64> = delivered.iter().map(|event| event.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
}

#[test]
fn cancelled_execution_reports_the_cancellation() {
    let world = world();
    let events = Arc::new(Collect::default());
    let handle = world
        .engine
        .start(&Request::program("find /data |> remove"), events);
    handle.cancel();
    let response = handle.wait(None).unwrap();
    response.validate().unwrap();
    match response.report.status {
        ReportStatus::Ok => assert!(!exists(&world, "/data/a.log")),
        _ => {
            assert_eq!(
                response.report.error.as_ref().unwrap().code,
                ErrorCode::Cancelled
            );
            assert!(exists(&world, "/data/a.log"));
        }
    }
}
