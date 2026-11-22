// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `test.run` command.

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput,
};

use super::args::{self, check_targets};
use super::launch::{self, Prepared, Request, Shape};
use crate::test::Framework;

/// Default time limit of a test run.
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Default number of failed tests shown in detail.
pub(super) const DEFAULT_FAILURES: usize = 10;

/// Default number of message lines per failed test.
pub(super) const DEFAULT_MESSAGE_LINES: usize = 5;

/// Value of `framework` that detects the framework.
const AUTO: &str = "auto";

pub(super) struct TestRun;

impl StageImpl for TestRun {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "test.run".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

pub(super) fn shape(args: &BoundArgs, default_slowest: usize) -> Result<Shape> {
    Ok(Shape {
        failures: args::count(args, "failures", DEFAULT_FAILURES)?,
        message_lines: args::count(args, "message_lines", DEFAULT_MESSAGE_LINES)?,
        slowest: args::count(args, "slowest", default_slowest)?,
    })
}

fn framework(args: &BoundArgs) -> Result<Option<Framework>> {
    match args::text(args, "framework")?.as_deref() {
        None | Some(AUTO) => Ok(None),
        Some(name) => Framework::from_name(name).map(Some).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownValue,
                format!("unknown test framework `{name}`"),
            )
            .with_fix("framework: auto")
        }),
    }
}

fn request(ctx: &ExecCtx, args: &BoundArgs) -> Result<Request> {
    let targets = args::list(args, "targets")?;
    check_targets(&targets)?;
    let command = args::list(args, "command")?;
    Ok(Request {
        dir: args::project_dir(ctx, args)?,
        framework: framework(args)?,
        targets,
        filter: args::text(args, "filter")?,
        bail: args::flag(args, "bail")?,
        only: Vec::new(),
        command: (!command.is_empty()).then_some(command),
        coverage: args::flag(args, "coverage")?,
        coverage_file: args::path(args, "coverage_file")?,
        timeout: args::duration(args, "timeout")?.unwrap_or(DEFAULT_TIMEOUT),
        wait: args::duration(args, "wait")?,
        shape: shape(args, 0)?,
        retry_of: None,
    })
}

impl EffectStage for TestRun {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let prepared = launch::prepare(request(ctx, args)?)?;
        Ok(launch::preview(ctx, &prepared))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let prepared = plan.take_state::<Prepared>().ok_or_else(|| {
            Error::new(ErrorCode::Internal, "the test run has no prepared state")
        })?;
        launch::execute(ctx, prepared)
    }
}
