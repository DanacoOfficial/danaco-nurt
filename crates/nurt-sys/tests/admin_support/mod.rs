// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sys/tests/admin_support/mod.rs
// Responsibility:   Provides the process host, transaction and request helpers shared by the administration tests against the real system.
// Dependencies:     nurt-contracts, nurt-testkit, which
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Harness shared by the tests of the administration commands against the real system.
// A shared harness: not every test uses every helper; test code asserts with unwrap, expect and panic.
#![allow(dead_code, clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetIndex, TargetSpec,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::services::{
    ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelToken, EffectStage, ExecCtx, StageData, StageInput,
};
use nurt_contracts::value::Value;
use nurt_testkit::ctx::TestContext;

pub(crate) struct StdProcess;

impl ProcessHost for StdProcess {
    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        let mut child = Command::new(&spec.program)
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| Error::new(ErrorCode::ProgramNotFound, error.to_string()))?;
        if let ProgramInput::Bytes(bytes) = &spec.stdin {
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(bytes)
                .expect("write");
        }
        let output = child.wait_with_output().expect("output");
        Ok(ProgramOutcome {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            ..ProgramOutcome::default()
        })
    }

    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        which::which(program)
            .map_err(|error| Error::new(ErrorCode::ProgramNotFound, error.to_string()))
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(ErrorCode::Spawn, "not available"))
    }
}

#[derive(Default)]
pub(crate) struct Tx {
    pub(crate) markers: Vec<IrreversibleEntry>,
}

fn unavailable<T>() -> Result<T> {
    Err(Error::new(ErrorCode::StageFailed, "not used"))
}

impl MutationTx for Tx {
    fn op(&self) -> Option<OpId> {
        None
    }
    fn preserve(&mut self, _t: &[TargetSpec]) -> Result<Vec<Preserved>> {
        unavailable()
    }
    fn stage(&mut self, _t: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        unavailable()
    }
    fn apply(&mut self, _t: TargetIndex, _m: Mutation) -> Result<Applied> {
        unavailable()
    }
    fn observe_external(&mut self, _c: &[ExternalChange]) -> Result<Vec<Applied>> {
        unavailable()
    }
    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.markers.push(marker);
    }
    fn record_exempt(&mut self, _w: ExemptWrite) {}
    fn commit(self: Box<Self>) -> Result<OpHandle> {
        unavailable()
    }
    fn rollback(self: Box<Self>, _r: RollbackReason) -> Result<RollbackReport> {
        unavailable()
    }
}

pub(crate) fn context() -> ExecCtx {
    TestContext::new()
        .with_process_host(Arc::new(StdProcess))
        .ctx()
}

pub(crate) fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

pub(crate) fn word(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

pub(crate) fn shell(program: &str, arguments: &[&str]) -> String {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .expect("program starts");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Changes reported by a stage.
pub(crate) struct Done {
    pub(crate) changes: Vec<Change>,
}

pub(crate) fn run(stage: &dyn EffectStage, request: &BoundArgs, ctx: &ExecCtx) -> Done {
    let input = StageInput {
        data: StageData::None,
        header: None,
        order: None,
        render: None,
    };
    let plan = stage.plan(request, input, ctx).expect("plan");
    let mut tx = Tx::default();
    let outcome = stage.apply(request, plan, &mut tx, ctx).expect("apply");
    assert!(!tx.markers.is_empty() || outcome.changes.is_empty());
    Done {
        changes: outcome.changes,
    }
}

/// Input of a stage that reads nothing from a pipeline.
pub(crate) fn no_input() -> StageInput<'static> {
    StageInput {
        data: StageData::None,
        header: None,
        order: None,
        render: None,
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
