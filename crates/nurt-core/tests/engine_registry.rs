// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-core/tests/engine_registry.rs
// Responsibility:   Access to the registry in the context of stages that run through the.
// Dependencies:     nurt_contracts, nurt_sys, nurt_testkit, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Access to the registry in the context of stages that run through the
//! engine: the context built for execution, not only the one returned by
//! `Engine::context`, carries it when the engine has a registry.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;

use nurt_contracts::envelope::Request;
use nurt_contracts::error::Result;
use nurt_contracts::fs::RegistryAccess;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig};
use nurt_sys::registry::RegistryProvider;
use nurt_sys::registry::mem::MemRegistry;
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::vfs::VirtualFs;
use support::standard_programs;

/// `find`: one record that tells whether the stage context gives access to
/// the registry.
struct Probe;

impl StageImpl for Probe {
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

impl ReadStage for Probe {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let present = ctx.extensions().get::<dyn RegistryAccess>().is_some();
        let record = Record::new(RecordKind::Entry)
            .with("path", Value::Path(nurt_testkit::paths::abs("/data/a.log")))
            .with("registry", Value::Bool(present));
        Ok(StageOutput::records(RecordStream::from_records(vec![
            record,
        ])))
    }
}

fn engine_reports_registry(registry: Option<Arc<RegistryProvider>>) -> bool {
    let clock = Arc::new(VirtualClock::new());
    let vfs = Arc::new(VirtualFs::new(clock.clone()));
    let mut config = EngineConfig::new(clock, Arc::new(TestHost::windows()))
        .with_sources(ConfigSources::isolated())
        .without_state_dir()
        .with_fs(vfs)
        .with_frontend(Arc::new(standard_programs()))
        .with_stage(Arc::new(Probe));
    if let Some(registry) = registry {
        config = config.with_registry(registry);
    }
    let engine = Engine::open(config).unwrap();
    let response = engine.run(&Request::program("find /data"));
    assert_eq!(
        response.report.status,
        ReportStatus::Ok,
        "{:?}",
        response.report.error
    );
    let page = response.report.records.as_ref().unwrap();
    page.rows[0].get("registry") == Some(&Value::Bool(true))
}

#[test]
fn a_stage_that_runs_through_the_engine_reaches_the_registry_of_the_engine() {
    let registry = Arc::new(RegistryProvider::new(Arc::new(MemRegistry::new())));
    assert!(engine_reports_registry(Some(registry)));
}

#[cfg(not(windows))]
#[test]
fn an_engine_without_a_registry_gives_stages_no_access_to_it() {
    assert!(!engine_reports_registry(None));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
