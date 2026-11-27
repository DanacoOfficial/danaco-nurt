// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-core/tests/engine_continuation.rs
// Responsibility:   The continuation of a query that has its own cursor, end to end through.
// Dependencies:     nurt_contracts, nurt_testkit, serde_json, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The continuation of a query that has its own cursor, end to end through
//! the engine: the first response cuts the result at the page size, the
//! stage cuts it again at its own limit, and events arrive between the parts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::{Arc, Mutex};

use nurt_contracts::continuation::{
    ContinuationRequest, ContinuationSink, ContinuationSource, ContinuationUnit, ResumeOverride,
};
use nurt_contracts::envelope::Request;
use nurt_contracts::error::Result;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::vfs::VirtualFs;
use serde_json::json;
use support::{Programs, find_stage, program};

/// Events, newest first, read by the stage `find` page by page after a cursor.
struct Events(Arc<Mutex<Vec<i64>>>);

impl StageImpl for Events {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "find".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Events {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let resume = ctx.extensions().get::<ResumeOverride>();
        let before = resume
            .as_ref()
            .map_or(i64::MAX, |resume| resume.cursor.parse().unwrap());
        let limit = resume
            .as_ref()
            .and_then(|resume| resume.count)
            .map_or(100, |count| usize::try_from(count).unwrap());
        let older: Vec<i64> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|id| *id < before)
            .collect();
        let part: Vec<i64> = older.iter().copied().take(limit).collect();
        if older.len() > part.len()
            && let Some(sink) = ctx.extensions().get::<ContinuationSink>()
        {
            sink.push(ContinuationRequest {
                id: None,
                source: ContinuationSource::Resume {
                    cursor: part.last().unwrap().to_string(),
                },
                unit: ContinuationUnit::Records,
                shown: u64::try_from(part.len()).unwrap(),
                remaining: -1,
                cut_lines: 0,
            });
        }
        let records = part
            .into_iter()
            .map(|id| Record::new(RecordKind::Row).with("id", Value::Int(id)))
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

fn ids(response: &nurt_contracts::envelope::Response) -> Vec<i64> {
    response
        .report
        .records
        .as_ref()
        .map(|page| {
            page.rows
                .iter()
                .map(|row| match row.get("id") {
                    Some(Value::Int(id)) => *id,
                    other => panic!("not an event: {other:?}"),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn parts_of_a_query_cut_twice_join_without_gap_or_overlap_while_events_arrive() {
    let events = Arc::new(Mutex::new((1..=230).rev().collect::<Vec<i64>>()));
    let next = serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{"t": "pipeline",
            "head": {"t": "stage", "name": "next", "effect": "read", "phase": "source", "args": []},
            "stages": []}]}
    }))
    .unwrap();
    let frontend = Programs::default()
        .with("find /data", program(find_stage(), Vec::new()))
        .with("next", next);
    let clock = Arc::new(VirtualClock::new());
    let engine = Engine::open(
        EngineConfig::new(clock.clone(), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_fs(Arc::new(VirtualFs::new(clock.clone())))
            .with_frontend(Arc::new(frontend))
            .with_stage(Arc::new(Events(Arc::clone(&events)))),
    )
    .unwrap();
    let first = engine.run(&Request::program("find /data"));
    assert_eq!(first.report.error, None);
    let continuation = first.report.continuation.clone().expect("the page is cut");
    assert_eq!(continuation.id, "c1");
    assert_eq!(continuation.shown, 50);
    assert_eq!(continuation.remaining, -1);
    let mut seen = ids(&first);
    assert_eq!(seen.len(), 50);
    let mut arrived = 230;
    for _ in 0..40 {
        arrived += 1;
        events.lock().unwrap().insert(0, arrived);
        let part = engine.run(&Request::program("next"));
        assert_eq!(part.report.error, None);
        seen.extend(ids(&part));
        let more = part.report.continuation.as_ref().expect("a continuation");
        if more.remaining == 0 {
            break;
        }
    }
    assert_eq!(seen, (1..=230).rev().collect::<Vec<_>>());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
